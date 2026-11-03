//! 受管子进程执行器。
//!
//! 统一期限、stdin/stdout/stderr 并行泵送、输出预算、取消、进程组终止、
//! 显式 wait/reap 与全局准入。Todo、总结、群聊等场景只负责构造命令和解释
//! 结果,不再各自维护一份较弱/不同的执行逻辑。

use std::process::{ExitStatus, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};
use tokio::sync::Semaphore;

tokio::task_local! { static CONTEXT_CANCEL:tokio::sync::watch::Receiver<bool>; }
/// A runtime owner can cancel nested adapters (including binary resolution)
/// without changing each provider's public trait.
pub async fn with_cancellation<F: std::future::Future>(
    cancel: tokio::sync::watch::Receiver<bool>,
    work: F,
) -> F::Output {
    CONTEXT_CANCEL.scope(cancel, work).await
}

/// 普通任务 stdout 初始预算。
pub const DEFAULT_STDOUT_LIMIT: usize = 8 * 1024 * 1024;
/// stderr 初始保留预算。
pub const DEFAULT_STDERR_LIMIT: usize = 64 * 1024;
/// 超时/取消后等待被终止进程回收的清理预算。
const CLEANUP_BUDGET: Duration = Duration::from_secs(3);
/// TERM 到 KILL 的宽限。
const TERM_GRACE: Duration = Duration::from_millis(500);
/// 全局同时执行上限。
const MAX_CONCURRENT: usize = 4;
/// 全局排队上限;超过直接 Busy,不无限创建 task。
const MAX_WAITERS: usize = 32;

/// 具名执行输入。`deadline` 覆盖程序解析之后的 stdin、输出与等待。
pub struct ExecutionSpec {
    pub command: Command,
    pub stdin: Option<Vec<u8>>,
    pub deadline: Instant,
    pub stdout_limit: usize,
    pub stderr_limit: usize,
    pub cancel: Option<tokio::sync::watch::Receiver<bool>>,
}

impl ExecutionSpec {
    pub fn new(command: Command, budget: Duration) -> Self {
        Self {
            command,
            stdin: None,
            deadline: Instant::now() + budget,
            stdout_limit: DEFAULT_STDOUT_LIMIT,
            stderr_limit: DEFAULT_STDERR_LIMIT,
            cancel: None,
        }
    }

    pub fn stdin(mut self, stdin: Option<Vec<u8>>) -> Self {
        self.stdin = stdin;
        self
    }

    pub fn limits(mut self, stdout: usize, stderr: usize) -> Self {
        self.stdout_limit = stdout;
        self.stderr_limit = stderr;
        self
    }

    pub fn cancel(mut self, cancel: tokio::sync::watch::Receiver<bool>) -> Self {
        self.cancel = Some(cancel);
        self
    }

    pub fn remaining(&self) -> Duration {
        self.deadline.saturating_duration_since(Instant::now())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Captured {
    pub text: String,
    pub truncated: bool,
}

#[derive(Debug)]
pub struct ExecutionOutcome {
    pub status: ExitStatus,
    pub stdout: Captured,
    pub stderr: Captured,
    /// 正常完成时清理组内残留/回收 leader 的诊断;为空表示没有观察到问题。
    pub cleanup_warnings: Vec<String>,
}

impl ExecutionOutcome {
    pub fn success(&self) -> bool {
        self.status.success()
    }

    pub fn exit_code(&self) -> Option<i32> {
        self.status.code()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionFailure {
    Spawn(String),
    Timeout,
    Cancelled,
    OutputLimit {
        stream: &'static str,
        limit: usize,
    },
    Read {
        stream: &'static str,
        reason: String,
    },
    Stdin(String),
    Wait(String),
    Busy,
    CleanupFailed(String),
}

impl std::fmt::Display for ExecutionFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecutionFailure::Spawn(reason) => write!(f, "启动子进程失败: {reason}"),
            ExecutionFailure::Timeout => write!(f, "执行超时"),
            ExecutionFailure::Cancelled => write!(f, "执行已取消"),
            ExecutionFailure::OutputLimit { stream, limit } => {
                write!(f, "{stream} 输出超过 {limit} 字节预算")
            }
            ExecutionFailure::Read { stream, reason } => {
                write!(f, "读取 {stream} 失败: {reason}")
            }
            ExecutionFailure::Stdin(reason) => write!(f, "写入 stdin 失败: {reason}"),
            ExecutionFailure::Wait(reason) => write!(f, "等待子进程失败: {reason}"),
            ExecutionFailure::Busy => write!(f, "执行队列已满"),
            ExecutionFailure::CleanupFailed(reason) => {
                write!(f, "子进程清理未完成: {reason}")
            }
        }
    }
}

impl std::error::Error for ExecutionFailure {}

/// 执行一个受管子进程。许可持有到进程组回收结束。
pub async fn run(mut spec: ExecutionSpec) -> Result<ExecutionOutcome, ExecutionFailure> {
    if spec.cancel.is_none() {
        spec.cancel = CONTEXT_CANCEL.try_with(Clone::clone).ok();
    }
    run_with_limiter(spec, limiter()).await
}

async fn run_with_limiter(
    spec: ExecutionSpec,
    admission: &Limiter,
) -> Result<ExecutionOutcome, ExecutionFailure> {
    // Portable budget below macOS ARG_MAX, including inherited environment.
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let command = spec.command.as_std();
        let mut env = std::env::vars_os().collect::<std::collections::HashMap<_, _>>();
        for (key, value) in command.get_envs() {
            if let Some(value) = value {
                env.insert(key.to_owned(), value.to_owned());
            } else {
                env.remove(key);
            }
        }
        let bytes = command
            .get_args()
            .map(|s| s.as_bytes().len() + 1)
            .sum::<usize>()
            + command.get_program().as_bytes().len()
            + 1
            + env
                .iter()
                .map(|(k, v)| k.as_bytes().len() + v.as_bytes().len() + 2)
                .sum::<usize>();
        if bytes > 128 * 1024 {
            return Err(ExecutionFailure::Spawn(
                "Limit: argv/environment exceeds 128KiB".into(),
            ));
        }
    }
    let remaining = spec.remaining();
    if remaining.is_zero() {
        return Err(ExecutionFailure::Timeout);
    }
    let cancel_future = wait_for_cancel(spec.cancel.clone());
    tokio::pin!(cancel_future);
    let _permit = tokio::select! {
        biased;
        _ = &mut cancel_future => return Err(ExecutionFailure::Cancelled),
        result = admission.acquire(remaining) => result?,
    };

    // 许可刚交付时取消/期限也可能已到达；不要先 spawn 再清理一个本不该启动的命令。
    if spec.cancel.as_ref().is_some_and(|rx| *rx.borrow()) {
        return Err(ExecutionFailure::Cancelled);
    }
    if spec.remaining().is_zero() {
        return Err(ExecutionFailure::Timeout);
    }

    let mut spec = spec;
    spec.command
        .stdin(if spec.stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    spec.command.process_group(0);

    let mut child = spec
        .command
        .spawn()
        .map_err(|error| ExecutionFailure::Spawn(error.to_string()))?;
    let group = ProcessGroup::new(child.id());
    if spec.remaining().is_zero() {
        return match cleanup_group(&mut child, &group).await {
            Ok(_) => Err(ExecutionFailure::Timeout),
            Err(w) => Err(ExecutionFailure::CleanupFailed(w.join("; "))),
        };
    }
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let pump = async {
        let write = write_stdin(stdin, spec.stdin);
        let read_out = read_capped(stdout, spec.stdout_limit);
        let read_err = read_capped(stderr, spec.stderr_limit);
        let (write, out, err) = tokio::join!(write, read_out, read_err);
        (write, out, err)
    };

    enum Step {
        Waited(Result<ExitStatus, std::io::Error>),
        Pumped(
            (
                Result<(), String>,
                (Captured, Option<String>),
                (Captured, Option<String>),
            ),
        ),
        Timeout,
        Cancelled,
    }

    let mut wait = Box::pin(child.wait());
    let mut io = Box::pin(pump);
    let step = tokio::select! {
        result = &mut wait => Step::Waited(result),
        result = &mut io => Step::Pumped(result),
        _ = tokio::time::sleep_until(tokio::time::Instant::from_std(spec.deadline)) => Step::Timeout,
        _ = &mut cancel_future => Step::Cancelled,
    };

    let mut cleanup_warnings = Vec::new();
    let deadline = tokio::time::Instant::from_std(spec.deadline);
    let ((write, out, err), status) = match step {
        Step::Waited(status) => {
            drop(wait);
            tokio::select! {
                io = io => (io, status),
                _ = tokio::time::sleep_until(deadline) => {
                    return match cleanup_group(&mut child, &group).await {
                        Ok(_) => Err(ExecutionFailure::Timeout),
                        Err(warnings) => Err(ExecutionFailure::CleanupFailed(warnings.join("; "))),
                    };
                }
                _ = &mut cancel_future => {
                    return match cleanup_group(&mut child, &group).await {
                        Ok(_) => Err(ExecutionFailure::Cancelled),
                        Err(warnings) => Err(ExecutionFailure::CleanupFailed(warnings.join("; "))),
                    };
                }
            }
        }
        Step::Pumped(io) => {
            tokio::select! {
                status = &mut wait => (io, status),
                _ = tokio::time::sleep_until(deadline) => {
                    drop(wait);
                    return match cleanup_group(&mut child, &group).await {
                        Ok(_) => Err(ExecutionFailure::Timeout),
                        Err(warnings) => Err(ExecutionFailure::CleanupFailed(warnings.join("; "))),
                    };
                }
                _ = &mut cancel_future => {
                    drop(wait);
                    return match cleanup_group(&mut child, &group).await {
                        Ok(_) => Err(ExecutionFailure::Cancelled),
                        Err(warnings) => Err(ExecutionFailure::CleanupFailed(warnings.join("; "))),
                    };
                }
            }
        }
        Step::Timeout => {
            drop(wait);
            drop(io);
            return match cleanup_group(&mut child, &group).await {
                Ok(_) => Err(ExecutionFailure::Timeout),
                Err(warnings) => Err(ExecutionFailure::CleanupFailed(warnings.join("; "))),
            };
        }
        Step::Cancelled => {
            drop(wait);
            drop(io);
            return match cleanup_group(&mut child, &group).await {
                Ok(_) => Err(ExecutionFailure::Cancelled),
                Err(warnings) => Err(ExecutionFailure::CleanupFailed(warnings.join("; "))),
            };
        }
    };
    let status = status.map_err(|error| ExecutionFailure::Wait(error.to_string()))?;
    let (out, out_error) = out;
    let (err, err_error) = err;
    if !out.truncated {
        // 正常完成也清理组内可能残留的后代(leader 已退出但它们仍持有管道时
        // 会先触发 deadline,不会走到这里)。
        #[cfg(unix)]
        group.signal(libc::SIGKILL);
    }
    if let Some(reason) = out_error {
        return Err(ExecutionFailure::Read {
            stream: "stdout",
            reason,
        });
    }
    if let Some(reason) = err_error {
        return Err(ExecutionFailure::Read {
            stream: "stderr",
            reason,
        });
    }
    if out.truncated {
        return Err(ExecutionFailure::OutputLimit {
            stream: "stdout",
            limit: spec.stdout_limit,
        });
    }
    write.map_err(ExecutionFailure::Stdin)?;
    if let Some(warning) = group.reap_warning() {
        cleanup_warnings.push(warning);
    }
    Ok(ExecutionOutcome {
        status,
        stdout: out,
        stderr: err,
        cleanup_warnings,
    })
}

async fn wait_for_cancel(mut cancel: Option<tokio::sync::watch::Receiver<bool>>) {
    match cancel.as_mut() {
        Some(rx) => loop {
            if *rx.borrow() {
                return;
            }
            if rx.changed().await.is_err() {
                std::future::pending::<()>().await;
            }
        },
        None => std::future::pending::<()>().await,
    }
}

async fn write_stdin(
    stdin: Option<tokio::process::ChildStdin>,
    bytes: Option<Vec<u8>>,
) -> Result<(), String> {
    let Some(mut stdin) = stdin else {
        return Ok(());
    };
    if let Some(bytes) = bytes {
        stdin.write_all(&bytes).await.map_err(|e| e.to_string())?;
    }
    // 显式关闭,向子进程发送 EOF;错误不能吞掉。
    stdin.shutdown().await.map_err(|e| e.to_string())
}

async fn read_capped<R>(reader: Option<R>, limit: usize) -> (Captured, Option<String>)
where
    R: AsyncRead + Unpin,
{
    let Some(mut reader) = reader else {
        return (
            Captured {
                text: String::new(),
                truncated: false,
            },
            None,
        );
    };
    let mut stored = Vec::new();
    let mut truncated = false;
    let mut block = [0u8; 8192];
    let mut read_error = None;
    loop {
        match reader.read(&mut block).await {
            Ok(0) => break,
            Ok(n) => {
                if stored.len() < limit {
                    let keep = n.min(limit - stored.len());
                    stored.extend_from_slice(&block[..keep]);
                    truncated |= keep < n;
                } else {
                    truncated = true;
                }
            }
            Err(error) => {
                read_error = Some(error.to_string());
                break;
            }
        }
    }
    (
        Captured {
            text: String::from_utf8_lossy(&stored).into_owned(),
            truncated,
        },
        read_error,
    )
}

#[cfg(unix)]
struct ProcessGroup {
    pgid: Option<u32>,
}

#[cfg(unix)]
impl ProcessGroup {
    fn new(pid: Option<u32>) -> Self {
        Self { pgid: pid }
    }

    fn signal(&self, signal: i32) {
        if let Some(pgid) = self.pgid {
            unsafe {
                libc::kill(-(pgid as i32), signal);
            }
        }
    }

    fn alive(&self) -> bool {
        self.pgid
            .map(|pgid| unsafe { libc::kill(-(pgid as i32), 0) } == 0)
            .unwrap_or(false)
    }

    fn reap_warning(&self) -> Option<String> {
        // leader 已 wait;这里只剩组内后代。短暂检查一次,不阻塞正常返回。
        if self.alive() {
            Some("进程组中仍有后代存活,已发送 SIGKILL".to_string())
        } else {
            None
        }
    }
}

#[cfg(unix)]
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        // task abort / runtime shutdown 的兜底。
        self.signal(libc::SIGKILL);
    }
}

#[cfg(not(unix))]
struct ProcessGroup {
    _private: (),
}

#[cfg(not(unix))]
impl ProcessGroup {
    fn new(_pid: Option<u32>) -> Self {
        Self { _private: () }
    }
    fn signal(&self, _signal: i32) {}
    fn reap_warning(&self) -> Option<String> {
        None
    }
}

/// TERM → 宽限 → KILL → 显式 wait/reap。返回清理 warning;`Err` 表示在清理
/// 预算内仍未完成回收。
#[cfg(unix)]
async fn cleanup_group(
    child: &mut Child,
    group: &ProcessGroup,
) -> Result<Vec<String>, Vec<String>> {
    let deadline = Instant::now() + CLEANUP_BUDGET;
    group.signal(libc::SIGTERM);
    let mut warnings = Vec::new();
    let leader_reaped = tokio::time::timeout(TERM_GRACE, child.wait()).await.is_ok();
    group.signal(libc::SIGKILL);
    if !leader_reaped {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match tokio::time::timeout(remaining, child.wait()).await {
            Ok(Ok(_)) => warnings.push("TERM 宽限内未退出,已升级 SIGKILL 回收".into()),
            Ok(Err(error)) => {
                return Err(vec![format!("等待被终止的 leader 失败: {error}")]);
            }
            Err(_) => {
                return Err(vec!["清理预算内未能回收 leader".to_string()]);
            }
        }
    }
    // 给组内后代时间响应 SIGKILL;预算与整体清理预算共用。
    while group.alive() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    if group.alive() {
        return Err(vec!["进程组中仍有后代未在清理预算内消失".into()]);
    }
    Ok(warnings)
}

#[cfg(not(unix))]
async fn cleanup_group(
    child: &mut Child,
    _group: &ProcessGroup,
) -> Result<Vec<String>, Vec<String>> {
    match tokio::time::timeout(CLEANUP_BUDGET, child.kill()).await {
        Ok(Ok(_)) => Ok(Vec::new()),
        Ok(Err(error)) => Err(vec![error.to_string()]),
        Err(_) => Err(vec!["清理预算内未能结束子进程".into()]),
    }
}

struct Limiter {
    semaphore: std::sync::Arc<Semaphore>,
    waiting: AtomicUsize,
}

/// 排队计数 RAII:future 被取消/abort/panic 时也会减计数,避免永久耗尽。
struct WaitingGuard<'a> {
    waiting: &'a AtomicUsize,
}

impl<'a> WaitingGuard<'a> {
    fn enter(waiting: &'a AtomicUsize, max: usize) -> Option<Self> {
        let previous = waiting.fetch_add(1, Ordering::SeqCst);
        if previous >= max {
            waiting.fetch_sub(1, Ordering::SeqCst);
            return None;
        }
        Some(Self { waiting })
    }
}

impl Drop for WaitingGuard<'_> {
    fn drop(&mut self) {
        self.waiting.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Limiter {
    async fn acquire(
        &self,
        remaining: Duration,
    ) -> Result<tokio::sync::OwnedSemaphorePermit, ExecutionFailure> {
        let Some(_waiting) = WaitingGuard::enter(&self.waiting, MAX_WAITERS) else {
            return Err(ExecutionFailure::Busy);
        };
        let acquired =
            tokio::time::timeout(remaining, self.semaphore.clone().acquire_owned()).await;
        match acquired {
            Ok(Ok(permit)) => Ok(permit),
            Ok(Err(_)) => Err(ExecutionFailure::Busy),
            Err(_) => Err(ExecutionFailure::Timeout),
        }
    }
}

fn limiter() -> &'static Limiter {
    static LIMITER: OnceLock<Limiter> = OnceLock::new();
    LIMITER.get_or_init(|| Limiter {
        semaphore: std::sync::Arc::new(Semaphore::new(MAX_CONCURRENT)),
        waiting: AtomicUsize::new(0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use tokio::sync::watch;

    fn shell(script: &str, budget: Duration) -> ExecutionSpec {
        let mut command = Command::new("sh");
        command.arg("-c").arg(script);
        ExecutionSpec::new(command, budget)
    }

    #[tokio::test]
    async fn captures_stdout_and_success() {
        let spec = shell("printf hello", Duration::from_secs(2));
        let outcome = run(spec).await.unwrap();
        assert!(outcome.success());
        assert_eq!(outcome.stdout.text, "hello");
    }

    #[tokio::test]
    async fn nonzero_exit_is_not_success() {
        let spec = shell("echo out; echo err >&2; exit 42", Duration::from_secs(2));
        let outcome = run(spec).await.unwrap();
        assert!(!outcome.success());
        assert_eq!(outcome.exit_code(), Some(42));
        assert_eq!(outcome.stdout.text.trim(), "out");
        assert_eq!(outcome.stderr.text.trim(), "err");
    }

    #[tokio::test]
    async fn stdout_over_budget_is_reported_not_truncated_success() {
        let spec = shell("printf 'xxxxxxxxxx'", Duration::from_secs(2)).limits(4, 1024);
        let error = run(spec).await.unwrap_err();
        assert!(matches!(
            error,
            ExecutionFailure::OutputLimit {
                stream: "stdout",
                limit: 4
            }
        ));
    }

    #[tokio::test]
    async fn stderr_over_budget_is_drained_but_does_not_fail_success() {
        let spec =
            shell("yes error | head -c 100000 >&2", Duration::from_secs(2)).limits(1024, 128);
        let outcome = run(spec).await.unwrap();
        assert!(outcome.success());
        assert!(outcome.stderr.truncated);
        assert!(outcome.stderr.text.len() <= 128);
    }

    #[tokio::test]
    async fn blocked_stdin_is_covered_by_the_same_deadline() {
        let spec = shell("sleep 30", Duration::from_millis(100)).stdin(Some(vec![b'x'; 1_000_000]));
        let started = Instant::now();
        let error = run(spec).await.unwrap_err();
        assert_eq!(error, ExecutionFailure::Timeout);
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn timeout_kills_leader_and_group_descendants() {
        let dir = tempfile::tempdir().unwrap();
        let leader_file = dir.path().join("leader.pid");
        let child_file = dir.path().join("child.pid");
        let script = format!(
            "echo $$ > '{}'; sleep 30 & echo $! > '{}'; wait",
            leader_file.display(),
            child_file.display()
        );
        let error = run(shell(&script, Duration::from_millis(150)))
            .await
            .unwrap_err();
        assert_eq!(error, ExecutionFailure::Timeout);
        let leader: i32 = std::fs::read_to_string(&leader_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let child: i32 = std::fs::read_to_string(&child_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(!process_alive(leader), "leader 应已回收");
        assert!(
            wait_process_gone(child, Duration::from_secs(3)),
            "组内后代应已被终止"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn leader_exit_with_descendant_holding_pipe_hits_deadline_then_cleans_group() {
        let dir = tempfile::tempdir().unwrap();
        let child_file = dir.path().join("child.pid");
        let script = format!("sleep 30 & echo $! > '{}'; exit 0", child_file.display());
        let error = run(shell(&script, Duration::from_millis(200)))
            .await
            .unwrap_err();
        assert_eq!(error, ExecutionFailure::Timeout);
        let child: i32 = std::fs::read_to_string(&child_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(wait_process_gone(child, Duration::from_secs(3)));
    }

    #[tokio::test]
    async fn cancel_terminates_execution() {
        let (tx, rx) = watch::channel(false);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let _ = tx.send(true);
        });
        let spec = shell("sleep 30", Duration::from_secs(10)).cancel(rx);
        let error = run(spec).await.unwrap_err();
        assert_eq!(error, ExecutionFailure::Cancelled);
    }

    #[tokio::test]
    async fn queued_cancellation_releases_admission_without_spawning() {
        use std::future::Future;
        use std::task::Poll;

        let admission = Limiter {
            semaphore: std::sync::Arc::new(Semaphore::new(1)),
            waiting: AtomicUsize::new(0),
        };
        let _occupied = admission.semaphore.clone().acquire_owned().await.unwrap();
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("must-not-start");
        let mut command = Command::new("touch");
        command.arg(&marker);
        let (tx, rx) = watch::channel(false);
        let spec = ExecutionSpec::new(command, Duration::from_secs(30)).cancel(rx);
        let mut running = Box::pin(run_with_limiter(spec, &admission));

        // Poll 到真正等待许可的位置，无需靠 sleep 猜测是否已经进入队列。
        std::future::poll_fn(|cx| {
            assert!(running.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        assert_eq!(admission.waiting.load(Ordering::SeqCst), 1);
        tx.send(true).unwrap();
        let error = tokio::time::timeout(Duration::from_secs(2), running)
            .await
            .expect("取消不能等待排队期限")
            .unwrap_err();
        assert_eq!(error, ExecutionFailure::Cancelled);
        assert_eq!(admission.waiting.load(Ordering::SeqCst), 0);
        assert!(!marker.exists(), "排队取消后不能启动命令");
    }

    #[tokio::test]
    async fn already_cancelled_command_is_never_spawned() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("must-not-start");
        let mut command = Command::new("touch");
        command.arg(&marker);
        let (_tx, rx) = watch::channel(true);
        let error = run(ExecutionSpec::new(command, Duration::from_secs(2)).cancel(rx))
            .await
            .unwrap_err();
        assert_eq!(error, ExecutionFailure::Cancelled);
        assert!(!marker.exists());
    }

    #[tokio::test]
    async fn cancel_after_leader_exit_still_terminates_descendants() {
        let (tx, rx) = watch::channel(false);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let _ = tx.send(true);
        });
        // leader 立即退出,后代仍持有管道;取消不能被“等待 IO”阶段吞掉。
        let spec = shell("sleep 30 & exit 0", Duration::from_secs(10)).cancel(rx);
        let started = Instant::now();
        let error = run(spec).await.unwrap_err();
        assert_eq!(error, ExecutionFailure::Cancelled);
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[tokio::test]
    async fn saturated_stdout_and_stderr_are_pumped_in_parallel() {
        let spec = shell(
            "yes out | head -c 100000; yes err | head -c 100000 >&2",
            Duration::from_secs(3),
        )
        .limits(200_000, 200_000);
        let outcome = run(spec).await.unwrap();
        assert_eq!(outcome.stdout.text.len(), 100_000);
        assert_eq!(outcome.stderr.text.len(), 100_000);
    }

    #[cfg(unix)]
    fn process_alive(pid: i32) -> bool {
        // 先收尸由执行器负责;这里只检查进程是否仍存在。
        let rc = unsafe { libc::kill(pid, 0) };
        rc == 0
    }

    #[cfg(unix)]
    fn wait_process_gone(pid: i32, budget: Duration) -> bool {
        let deadline = Instant::now() + budget;
        while process_alive(pid) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        !process_alive(pid)
    }

    #[tokio::test]
    async fn spawn_failure_is_classified() {
        let command = Command::new("this-binary-does-not-exist-xyz");
        let error = run(ExecutionSpec::new(command, Duration::from_secs(1)))
            .await
            .unwrap_err();
        assert!(matches!(error, ExecutionFailure::Spawn(_)));
    }

    #[tokio::test]
    async fn zero_deadline_does_not_spawn() {
        let mut command = Command::new("sh");
        command.arg("-c").arg("touch /tmp/dozer-should-not-exist");
        let mut spec = ExecutionSpec::new(command, Duration::from_secs(1));
        spec.deadline = Instant::now();
        let error = run(spec).await.unwrap_err();
        assert_eq!(error, ExecutionFailure::Timeout);
    }

    #[test]
    fn waiting_guard_releases_on_drop_and_enforces_limit() {
        let counter = AtomicUsize::new(0);
        {
            let _first = WaitingGuard::enter(&counter, 1).unwrap();
            assert!(WaitingGuard::enter(&counter, 1).is_none());
            assert_eq!(counter.load(Ordering::SeqCst), 1);
        }
        assert_eq!(counter.load(Ordering::SeqCst), 0);
        assert!(WaitingGuard::enter(&counter, 1).is_some());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn helper_can_be_found_on_disk() {
        assert!(Path::new("/bin/sh").exists());
    }
    #[tokio::test]
    async fn owner_cancellation_reaches_nested_adapters_before_spawn() {
        let (cancel, receiver) = tokio::sync::watch::channel(true);
        let mut command = Command::new("/bin/sh");
        command.arg("-c").arg("exit 0");
        let result = with_cancellation(
            receiver,
            run(ExecutionSpec::new(command, Duration::from_secs(3))),
        )
        .await;
        assert!(matches!(result, Err(ExecutionFailure::Cancelled)));
        drop(cancel);
    }
}
