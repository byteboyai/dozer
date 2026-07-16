use crate::ring::{RingBuffer, SCROLLBACK_CAP};
use anyhow::{Context, Result};
use dozer_core::protocol::SessionInfo;
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

#[derive(Debug, Clone)]
pub enum SessionEvent {
    Output { data: Vec<u8>, offset: u64 },
    Exited { code: Option<i32> },
}

#[derive(Debug, Clone)]
pub struct SessionSpec {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub cols: u16,
    pub rows: u16,
}

pub struct Session {
    id: String,
    spec: SessionSpec,
    created_ms: u64,
    alive: Arc<AtomicBool>,
    buffer: Arc<Mutex<RingBuffer>>,
    tx: broadcast::Sender<SessionEvent>,
    writer: Mutex<Box<dyn Write + Send>>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    child: Mutex<Box<dyn Child + Send + Sync>>,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock before epoch")
        .as_millis() as u64
}

impl Session {
    pub fn spawn(spec: SessionSpec) -> Result<Self> {
        let pty = native_pty_system();
        let pair = pty
            .openpty(PtySize { rows: spec.rows, cols: spec.cols, pixel_width: 0, pixel_height: 0 })
            .context("openpty")?;
        let mut cmd = CommandBuilder::new(&spec.command);
        cmd.args(&spec.args);
        cmd.cwd(&spec.cwd);
        let child = pair.slave.spawn_command(cmd).context("spawn_command")?;
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().context("clone reader")?;
        let writer = pair.master.take_writer().context("take writer")?;

        let alive = Arc::new(AtomicBool::new(true));
        let buffer = Arc::new(Mutex::new(RingBuffer::new(SCROLLBACK_CAP)));
        let (tx, _) = broadcast::channel::<SessionEvent>(1024);

        // 读线程：PTY 是阻塞 IO，用 std::thread 泵到 ring + broadcast
        {
            let alive = alive.clone();
            let buffer = buffer.clone();
            let tx = tx.clone();
            std::thread::spawn(move || {
                let mut chunk = [0u8; 8192];
                loop {
                    match reader.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            let data = chunk[..n].to_vec();
                            let offset = buffer.lock().expect("ring lock").push(&data);
                            let _ = tx.send(SessionEvent::Output { data, offset });
                        }
                    }
                }
                alive.store(false, Ordering::SeqCst);
                let _ = tx.send(SessionEvent::Exited { code: None });
            });
        }

        Ok(Self {
            id: uuid::Uuid::new_v4().to_string(),
            created_ms: now_ms(),
            alive,
            buffer,
            tx,
            writer: Mutex::new(writer),
            master: Mutex::new(pair.master),
            child: Mutex::new(child),
            spec,
        })
    }

    pub fn info(&self) -> SessionInfo {
        SessionInfo {
            id: self.id.clone(),
            name: self.spec.name.clone(),
            command: self.spec.command.clone(),
            cwd: self.spec.cwd.clone(),
            alive: self.alive.load(Ordering::SeqCst),
            created_ms: self.created_ms,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn write(&self, data: &[u8]) -> Result<()> {
        let mut w = self.writer.lock().expect("writer lock");
        w.write_all(data)?;
        w.flush()?;
        Ok(())
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        self.master
            .lock()
            .expect("master lock")
            .resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })?;
        Ok(())
    }

    pub fn kill(&self) -> Result<()> {
        self.child.lock().expect("child lock").kill()?;
        Ok(())
    }

    pub fn snapshot(&self) -> (Vec<u8>, u64) {
        self.buffer.lock().expect("ring lock").snapshot()
    }

    pub fn read_from(&self, offset: u64) -> Option<Vec<u8>> {
        self.buffer.lock().expect("ring lock").read_from(offset)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<SessionEvent> {
        self.tx.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn spec(cmd: &str) -> SessionSpec {
        SessionSpec {
            name: "t".into(),
            command: "/bin/sh".into(),
            args: vec!["-c".into(), cmd.into()],
            cwd: std::env::temp_dir().to_string_lossy().into_owned(),
            cols: 80,
            rows: 24,
        }
    }

    /// 等待直到缓冲包含期望内容或超时（PTY 输出是异步的）
    async fn wait_contains(s: &Session, needle: &[u8]) -> bool {
        for _ in 0..100 {
            let (data, _) = s.snapshot();
            if data.windows(needle.len()).any(|w| w == needle) {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
        false
    }

    #[tokio::test]
    async fn output_lands_in_buffer_and_broadcast() {
        let s = Session::spawn(spec("printf marker123; sleep 5")).unwrap();
        let mut rx = s.subscribe();
        assert!(wait_contains(&s, b"marker123").await, "buffer should contain output");
        // broadcast 也应收到含 marker 的事件（可能分片，收多次拼接）
        let mut got = Vec::new();
        while let Ok(Ok(ev)) =
            tokio::time::timeout(Duration::from_millis(500), rx.recv()).await
        {
            if let SessionEvent::Output { data, .. } = ev {
                got.extend(data);
                if got.windows(9).any(|w| w == b"marker123") {
                    break;
                }
            }
        }
        assert!(got.windows(9).any(|w| w == b"marker123"));
        s.kill().unwrap();
    }

    #[tokio::test]
    async fn write_reaches_child_stdin() {
        let s = Session::spawn(spec("cat")).unwrap();
        s.write(b"pingpong\n").unwrap();
        // cat 回显（经 PTY，含回显本身）
        assert!(wait_contains(&s, b"pingpong").await);
        s.kill().unwrap();
    }

    #[tokio::test]
    async fn natural_exit_broadcasts_exited_and_marks_dead() {
        let s = Session::spawn(spec("printf done")).unwrap();
        let mut rx = s.subscribe();
        let mut exited = false;
        for _ in 0..100 {
            match tokio::time::timeout(Duration::from_millis(100), rx.recv()).await {
                Ok(Ok(SessionEvent::Exited { .. })) => {
                    exited = true;
                    break;
                }
                Ok(Ok(_)) => continue,
                _ => continue,
            }
        }
        assert!(exited, "should broadcast Exited");
        assert!(!s.info().alive);
    }
}
