//! 有界同步执行池；future 构造不做 IO，取消的排队工作不会执行。
//! 使用独立线程和 oneshot，可由非 Tokio executor 驱动。
use crate::{BoxFuture, ServiceError, ServiceErrorKind};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static QUEUED: AtomicUsize = AtomicUsize::new(0);
/// (currently executing, admitted but not started), without request content.
pub fn occupancy() -> (usize, usize) {
    (ACTIVE.load(Ordering::SeqCst), QUEUED.load(Ordering::SeqCst))
}
struct Active;
impl Drop for Active {
    fn drop(&mut self) {
        ACTIVE.fetch_sub(1, Ordering::SeqCst);
    }
}
type Job = Box<dyn FnOnce() + Send>;
fn queue() -> &'static mpsc::SyncSender<Job> {
    static QUEUE: OnceLock<mpsc::SyncSender<Job>> = OnceLock::new();
    QUEUE.get_or_init(|| {
        let (tx, rx) = mpsc::sync_channel::<Job>(64);
        let rx = Arc::new(Mutex::new(rx));
        for index in 0..4 {
            let rx = rx.clone();
            std::thread::Builder::new()
                .name(format!("dozer-io-{index}"))
                .spawn(move || {
                    loop {
                        let job = rx.lock().expect("blocking queue").recv();
                        match job {
                            Ok(job) => job(),
                            Err(_) => break,
                        }
                    }
                })
                .expect("start blocking worker");
        }
        tx
    })
}
pub fn run<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, ServiceError> + Send + 'static,
) -> BoxFuture<'static, Result<T, ServiceError>> {
    Box::pin(async move {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let job = Box::new(move || {
            QUEUED.fetch_sub(1, Ordering::SeqCst);
            ACTIVE.fetch_add(1, Ordering::SeqCst);
            let _active = Active;
            if tx.is_closed() {
                return;
            }
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
                .unwrap_or_else(|_| {
                    Err(ServiceError::new(
                        ServiceErrorKind::Unavailable,
                        "blocking worker panicked",
                    ))
                });
            let _ = tx.send(result);
        });
        QUEUED.fetch_add(1, Ordering::SeqCst);
        queue().try_send(job).map_err(|_| {
            QUEUED.fetch_sub(1, Ordering::SeqCst);
            ServiceError::new(ServiceErrorKind::Rejected, "Busy: blocking queue is full")
        })?;
        rx.await.map_err(|_| {
            ServiceError::new(ServiceErrorKind::Unavailable, "blocking worker stopped")
        })?
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn constructing_and_dropping_future_does_not_run_work() {
        let ran = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let check = ran.clone();
        drop(run(move || {
            check.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }));
        assert!(!ran.load(std::sync::atomic::Ordering::SeqCst));
    }
    #[tokio::test]
    async fn panic_is_an_error_and_worker_remains_usable() {
        assert!(run::<()>(|| panic!("fixture")).await.is_err());
        assert_eq!(run(|| Ok(42)).await.unwrap(), 42);
    }
    #[tokio::test(flavor = "current_thread")]
    async fn blocked_io_does_not_block_the_async_executor() {
        let (started, ready) = tokio::sync::oneshot::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let work = tokio::spawn(run(move || {
            let _ = started.send(());
            wait.recv_timeout(std::time::Duration::from_secs(2))
                .map_err(|_| {
                    ServiceError::new(
                        ServiceErrorKind::Unavailable,
                        "executor could not release worker",
                    )
                })?;
            Ok(42)
        }));
        ready.await.unwrap();
        tokio::task::yield_now().await;
        assert!(!work.is_finished());
        release.send(()).unwrap();
        assert_eq!(work.await.unwrap().unwrap(), 42);
    }
}
