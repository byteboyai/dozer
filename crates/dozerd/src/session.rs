use crate::ring::{RingBuffer, SCROLLBACK_CAP};
use anyhow::{Context, Result};
use dozer_core::protocol::{AgentKind, AgentState, SessionInfo};
use portable_pty::{Child, CommandBuilder, PtySize, native_pty_system};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

dozer_core::scope!(LOG, module, "session");

/// 每会话待写 PTY 正文预算。超过明确失败,不静默丢键。
const MAX_INPUT_BYTES: usize = 64 * 1024;
/// 队列消息数上限;字节预算之外的第二道边界。
const MAX_INPUT_MESSAGES: usize = 256;

enum InputCommand {
    Write(Vec<u8>),
    Resize { cols: u16, rows: u16 },
}

#[derive(Debug, Clone)]
pub enum SessionEvent {
    Output {
        data: Vec<u8>,
        offset: u64,
    },
    Exited {
        code: Option<i32>,
    },
    /// hook 事件驱动的 agent 状态变更（P1e）。
    Agent {
        agent: AgentKind,
        state: AgentState,
        event: String,
        ts_ms: u64,
        transcript_path: Option<String>,
    },
}

#[derive(Debug, Clone)]
pub struct SessionSpec {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub cols: u16,
    pub rows: u16,
    pub project_id: i64,
}

/// 快照、水位和终端流完成状态必须在同一把锁内读取。
pub(crate) struct AttachSnapshot {
    pub bytes: Vec<u8>,
    pub next_offset: u64,
    pub exit_status: Option<Option<i32>>,
}

struct OutputState {
    ring: RingBuffer,
    exit_status: Option<Option<i32>>,
    truncated: bool,
}

impl OutputState {
    fn new() -> Self {
        Self {
            ring: RingBuffer::new(SCROLLBACK_CAP),
            exit_status: None,
            truncated: false,
        }
    }

    fn append(&mut self, data: Vec<u8>, tx: &broadcast::Sender<SessionEvent>) {
        // 排空期限后已封闭的流不再产生迟到 Output。
        if self.exit_status.is_none() {
            let offset = self.ring.push(&data);
            let _ = tx.send(SessionEvent::Output { data, offset });
        }
    }

    fn finish(&mut self, code: Option<i32>, tx: &broadcast::Sender<SessionEvent>) {
        if self.exit_status.is_none() {
            self.exit_status = Some(code);
            let _ = tx.send(SessionEvent::Exited { code });
        }
    }

    fn attach_snapshot(&self, from_offset: u64) -> AttachSnapshot {
        let (bytes, next_offset) = if from_offset > 0 {
            self.ring
                .read_from(from_offset)
                .map(|bytes| (bytes, self.ring.total_written()))
                .unwrap_or_else(|| self.ring.snapshot())
        } else {
            self.ring.snapshot()
        };
        AttachSnapshot {
            bytes,
            next_offset,
            exit_status: self.exit_status,
        }
    }
}

struct RuntimeHandle(Arc<AtomicUsize>);
impl Drop for RuntimeHandle {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

struct AgentMetadata {
    agent: AgentKind,
    state: AgentState,
    transcript_path: Option<String>,
    revision: u64,
}

pub struct Session {
    id: String,
    spec: SessionSpec,
    created_ms: u64,
    alive: Arc<AtomicBool>,
    exit_code: Arc<Mutex<Option<Option<i32>>>>,
    exited_ms: Arc<Mutex<Option<u64>>>,
    history_evicted: Arc<AtomicBool>,
    agent_metadata: Mutex<AgentMetadata>,
    buffer: Arc<Mutex<OutputState>>,
    tx: broadcast::Sender<SessionEvent>,
    input_tx: Arc<Mutex<Option<std::sync::mpsc::SyncSender<InputCommand>>>>,
    input_bytes: Arc<AtomicUsize>,
    runtime_handles: Arc<AtomicUsize>,
    child: Arc<Mutex<Option<Box<dyn Child + Send + Sync>>>>,
    lifecycle: tokio::sync::watch::Sender<Option<Option<i32>>>,
    summary_claimed: AtomicBool,
    summary_completed: AtomicBool,
}

pub(crate) struct SummaryClaim<'a> {
    state: &'a AtomicBool,
    completed: &'a AtomicBool,
    committed: bool,
}
impl SummaryClaim<'_> {
    pub fn commit(mut self) {
        self.committed = true;
        self.completed.store(true, Ordering::SeqCst);
    }
}
impl Drop for SummaryClaim<'_> {
    fn drop(&mut self) {
        if !self.committed {
            self.state.store(false, Ordering::SeqCst);
        }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock before epoch")
        .as_millis() as u64
}

fn wait_for_output_drain(drained: &AtomicBool, budget: std::time::Duration) -> bool {
    let deadline = std::time::Instant::now() + budget;
    while !drained.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
        std::thread::sleep(
            std::time::Duration::from_millis(10)
                .min(deadline.saturating_duration_since(std::time::Instant::now())),
        );
    }
    drained.load(Ordering::SeqCst)
}

fn finish_output(
    buffer: &Mutex<OutputState>,
    drained: &AtomicBool,
    tx: &broadcast::Sender<SessionEvent>,
    code: Option<i32>,
) {
    finish_output_with_budget(buffer, drained, tx, code, std::time::Duration::from_secs(1));
}
fn finish_output_with_budget(
    buffer: &Mutex<OutputState>,
    drained: &AtomicBool,
    tx: &broadcast::Sender<SessionEvent>,
    code: Option<i32>,
    budget: std::time::Duration,
) {
    let truncated = !wait_for_output_drain(drained, budget);
    if truncated {
        dozer_core::log_warn!(LOG, "PTY 输出排空超时，关闭终端流，尾部输出可能不完整");
    }
    // 与 append/attach_snapshot 共用锁，完成状态不会领先于快照水位。
    let mut output = buffer.lock().expect("ring lock");
    output.truncated = truncated;
    output.finish(code, tx);
}

impl Session {
    pub fn spawn(spec: SessionSpec) -> Result<Self> {
        let pty = native_pty_system();
        let pair = pty
            .openpty(PtySize {
                rows: spec.rows,
                cols: spec.cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("openpty")?;
        // id 提前生成：spawn 前注入 DOZER_SESSION_ID，hook 进程（claude
        // 在会话 shell 里手动跑也一样）经环境继承拿到归属（spec P1e D4）。
        let id = uuid::Uuid::new_v4().to_string();
        let mut cmd = CommandBuilder::new(&spec.command);
        cmd.args(&spec.args);
        cmd.cwd(&spec.cwd);
        // dozerd 自己常常是被外层 claude 会话（比如开发时在 Claude Code 里
        // `cargo run -p dozer-app`）拉起的子进程，这层 CLAUDE_CODE_* 身份标记会
        // 原样继承进 PTY。会话 shell 里手动跑的 claude 一旦看到
        // CLAUDE_CODE_CHILD_SESSION，就会把自己误认成外层会话的子会话直接关掉
        // transcript saving。dozer 的 PTY 会话本身就是独立顶层会话（归属靠上面
        // 的 DOZER_SESSION_ID），不该继承这层标记。
        for key in [
            "CLAUDECODE",
            "CLAUDE_CODE_CHILD_SESSION",
            "CLAUDE_CODE_ENTRYPOINT",
            "CLAUDE_CODE_EXECPATH",
            "CLAUDE_CODE_SESSION_ATTENDED",
            "CLAUDE_CODE_SESSION_ID",
            "CLAUDE_CODE_MESSAGING_SOCKET",
            "CLAUDE_CODE_MESSAGING_TOKEN",
        ] {
            cmd.env_remove(key);
        }
        cmd.env(dozer_core::paths::session_env(), &id);
        // zsh 会话注入 OSC 7/133 发射端（spec P1e D2；失败仅降级不阻断 spawn）
        if crate::shell_integration::should_inject(&spec.command) {
            match crate::shell_integration::ensure_zdotdir() {
                Ok(wrapper) => {
                    if let Ok(orig) = std::env::var("ZDOTDIR") {
                        cmd.env("DOZER_ORIG_ZDOTDIR", orig);
                    }
                    cmd.env("DOZER_ZDOTDIR_WRAPPER", &wrapper);
                    cmd.env("ZDOTDIR", &wrapper);
                }
                Err(e) => dozer_core::log_warn!(LOG, "shell 集成落盘失败，跳过注入: {e}"),
            }
        }
        // 没有 TERM 时很多 shell 行编辑器（readline/zle）退化成极简模式，
        // 方向键历史、颜色等一律不可用；COLORTERM=truecolor 让识别它的
        // 程序知道可以用 24-bit 真彩色而不是退化到 256 色。
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        // daemon 由 launchd/GUI 拉起时往往没有 LANG，子进程会落到 C locale，
        // 多字节输入（中文/emoji）在 readline/zle 里直接乱码。已有 UTF-8
        // locale 就沿用用户的语言偏好，否则兜底 en_US.UTF-8。
        let lang_is_utf8 = |v: &str| {
            let v = v.to_ascii_uppercase();
            v.contains("UTF-8") || v.contains("UTF8")
        };
        if !std::env::var("LANG").is_ok_and(|v| lang_is_utf8(&v)) {
            cmd.env("LANG", "en_US.UTF-8");
        }
        // A blocked PTY input must remain cancellable after the child exits.
        // Master clones share these flags; the reader also handles WouldBlock.
        #[cfg(unix)]
        {
            let fd = pair.master.as_raw_fd().context("PTY descriptor")?;
            let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
            if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
            {
                return Err(std::io::Error::last_os_error().into());
            }
        }
        let child = pair.slave.spawn_command(cmd).context("spawn_command")?;
        drop(pair.slave);

        #[cfg(unix)]
        let (mut reader, reader_fd) = {
            use std::os::fd::{AsRawFd, FromRawFd};
            let fd = unsafe {
                libc::fcntl(
                    pair.master.as_raw_fd().context("PTY descriptor")?,
                    libc::F_DUPFD_CLOEXEC,
                    0,
                )
            };
            if fd < 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            let file = unsafe { std::fs::File::from_raw_fd(fd) };
            let raw = file.as_raw_fd();
            (file, raw)
        };
        #[cfg(not(unix))]
        let mut reader = pair.master.try_clone_reader().context("clone reader")?;
        let writer = pair.master.take_writer().context("take writer")?;

        // 每会话单一 PTY 写入所有者:write/resize 进有界队列,由该线程按顺序
        // 执行;异步 handler 不再直接做可能阻塞的 PTY write。kill 走 child
        // 独立通道,不会被满载的输入队列挡住。
        let alive = Arc::new(AtomicBool::new(true));
        let (input_tx, input_rx) = std::sync::mpsc::sync_channel(MAX_INPUT_MESSAGES);
        let input_tx = Arc::new(Mutex::new(Some(input_tx)));
        let input_bytes = Arc::new(AtomicUsize::new(0));
        let runtime_handles = Arc::new(AtomicUsize::new(3));
        {
            let alive = alive.clone();
            let mut writer = writer;
            let master = pair.master;
            let input_bytes = input_bytes.clone();
            let runtime_handles = runtime_handles.clone();
            std::thread::spawn(move || {
                let _handle = RuntimeHandle(runtime_handles);
                while let Ok(command) = input_rx.recv() {
                    if !alive.load(Ordering::SeqCst) {
                        break;
                    }
                    match command {
                        InputCommand::Write(data) => {
                            let mut written = 0;
                            while written < data.len() && alive.load(Ordering::SeqCst) {
                                match writer.write(&data[written..]) {
                                    Ok(0) => break,
                                    Ok(n) => written += n,
                                    Err(error)
                                        if error.kind() == std::io::ErrorKind::Interrupted =>
                                    {
                                        continue;
                                    }
                                    Err(error)
                                        if error.kind() == std::io::ErrorKind::WouldBlock =>
                                    {
                                        #[cfg(unix)]
                                        if let Some(fd) = master.as_raw_fd() {
                                            let mut descriptor = libc::pollfd {
                                                fd,
                                                events: libc::POLLOUT,
                                                revents: 0,
                                            };
                                            unsafe { libc::poll(&mut descriptor, 1, 100) };
                                        }
                                        #[cfg(not(unix))]
                                        std::thread::sleep(std::time::Duration::from_millis(10));
                                    }
                                    Err(error) => {
                                        dozer_core::log_warn!(LOG,error=%error,"写 PTY 失败");
                                        break;
                                    }
                                }
                            }
                            input_bytes.fetch_sub(data.len(), Ordering::SeqCst);
                        }
                        InputCommand::Resize { cols, rows } => {
                            if let Err(error) = master.resize(PtySize {
                                rows,
                                cols,
                                pixel_width: 0,
                                pixel_height: 0,
                            }) {
                                dozer_core::log_warn!(LOG, error = %error, "调整 PTY 尺寸失败");
                            }
                        }
                    }
                }
                input_bytes.store(0, Ordering::SeqCst);
            });
        }

        let exit_code = Arc::new(Mutex::new(None));
        let exited_ms = Arc::new(Mutex::new(None));
        let history_evicted = Arc::new(AtomicBool::new(false));
        let output_drained = Arc::new(AtomicBool::new(false));
        let buffer = Arc::new(Mutex::new(OutputState::new()));
        let (tx, _) = broadcast::channel::<SessionEvent>(128);
        let child = Arc::new(Mutex::new(Some(child)));
        let (lifecycle, _) = tokio::sync::watch::channel(None);

        let stop_reader = Arc::new(AtomicBool::new(false));
        // 读线程：PTY 是阻塞 IO，用 std::thread 泵到 ring + broadcast。
        // PTY EOF 只代表数据面结束，不再等同于进程退出（退出码由独立 waiter
        // 通过 child.try_wait 回收）。
        {
            let buffer = buffer.clone();
            let tx = tx.clone();
            let output_drained = output_drained.clone();
            let stop_reader = stop_reader.clone();
            let runtime_handles = runtime_handles.clone();
            std::thread::spawn(move || {
                let _handle = RuntimeHandle(runtime_handles);
                let mut chunk = [0u8; 8192];
                loop {
                    if stop_reader.load(Ordering::SeqCst) {
                        break;
                    }
                    #[cfg(unix)]
                    {
                        let mut descriptor = libc::pollfd {
                            fd: reader_fd,
                            events: libc::POLLIN,
                            revents: 0,
                        };
                        let ready = unsafe { libc::poll(&mut descriptor, 1, 100) };
                        if ready == 0 {
                            continue;
                        }
                        if ready < 0 {
                            if std::io::Error::last_os_error().kind()
                                == std::io::ErrorKind::Interrupted
                            {
                                continue;
                            }
                            break;
                        }
                    }
                    match reader.read(&mut chunk) {
                        Err(error)
                            if matches!(
                                error.kind(),
                                std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
                            ) =>
                        {
                            continue;
                        }
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            let data = chunk[..n].to_vec();
                            buffer.lock().expect("ring lock").append(data, &tx);
                        }
                    }
                }
                output_drained.store(true, Ordering::SeqCst);
            });
        }

        // 独立 child waiter：记录真实退出码，且只广播一次 Exited。短命进程在
        // bridge/订阅者启动前退出时，迟到观察者通过 exit_status() 读取权威状态。
        {
            let alive = alive.clone();
            let exit_code = exit_code.clone();
            let exited_ms = exited_ms.clone();
            let output_drained = output_drained.clone();
            let buffer = buffer.clone();
            let child = child.clone();
            let input_tx = input_tx.clone();
            let lifecycle = lifecycle.clone();
            let tx = tx.clone();
            let runtime_handles = runtime_handles.clone();
            std::thread::spawn(move || {
                let _handle = RuntimeHandle(runtime_handles);
                loop {
                    let status = {
                        let mut child = child.lock().expect("child lock");
                        match child.as_mut() {
                            Some(child) => child.try_wait(),
                            None => break,
                        }
                    };
                    match status {
                        Ok(Some(status)) => {
                            let code = i32::try_from(status.exit_code()).ok();
                            alive.store(false, Ordering::SeqCst);
                            child.lock().expect("child lock").take();
                            input_tx.lock().expect("input lock").take();
                            *exit_code.lock().expect("exit lock") = Some(code);
                            lifecycle.send_replace(Some(code));
                            *exited_ms.lock().expect("exit time lock") = Some(now_ms());
                            alive.store(false, Ordering::SeqCst);
                            // 先记录退出状态,再给读线程最多 1s 排空 PTY;避免
                            // Exited 早于最后一段 Output 送达订阅者。
                            finish_output(&buffer, &output_drained, &tx, code);
                            stop_reader.store(true, Ordering::SeqCst);
                            break;
                        }
                        Ok(None) => std::thread::sleep(std::time::Duration::from_millis(20)),
                        Err(error) => {
                            dozer_core::log_warn!(LOG, error = %error, "等待子进程退出失败");
                            alive.store(false, Ordering::SeqCst);
                            child.lock().expect("child lock").take();
                            input_tx.lock().expect("input lock").take();
                            *exit_code.lock().expect("exit lock") = Some(None);
                            lifecycle.send_replace(Some(None));
                            *exited_ms.lock().expect("exit time lock") = Some(now_ms());
                            alive.store(false, Ordering::SeqCst);
                            finish_output(&buffer, &output_drained, &tx, None);
                            stop_reader.store(true, Ordering::SeqCst);
                            break;
                        }
                    }
                }
            });
        }

        Ok(Self {
            id,
            created_ms: now_ms(),
            alive,
            exit_code,
            exited_ms,
            history_evicted,
            agent_metadata: Mutex::new(AgentMetadata {
                agent: AgentKind::default(),
                state: AgentState::default(),
                transcript_path: None,
                revision: 1,
            }),
            buffer,
            tx,
            input_tx,
            input_bytes,
            runtime_handles,
            child,
            lifecycle,
            summary_claimed: AtomicBool::new(false),
            summary_completed: AtomicBool::new(false),
            spec,
        })
    }

    pub fn info(&self) -> SessionInfo {
        let metadata = self.agent_metadata.lock().expect("agent metadata");
        SessionInfo {
            id: self.id.clone(),
            name: self.spec.name.clone(),
            command: self.spec.command.clone(),
            cwd: self.spec.cwd.clone(),
            alive: self.alive.load(Ordering::SeqCst),
            created_ms: self.created_ms,
            agent: metadata.agent,
            agent_state: metadata.state,
            transcript_path: metadata.transcript_path.clone(),
            project_id: Some(self.spec.project_id),
        }
    }

    /// 记下会话的 transcript 路径（hook data 携带；覆盖旧值）。
    pub fn set_transcript_path(&self, path: &str) {
        let mut metadata = self.agent_metadata.lock().expect("agent metadata");
        if metadata.transcript_path.as_deref() != Some(path) {
            metadata.transcript_path = Some(path.into());
            metadata.revision += 1;
        }
    }

    /// 记下会话归属的 agent（首个 hook 事件到达时坐实；覆盖旧值）。
    ///
    /// 但拒绝“降级”：一个已经坐实为具体 agent（非 Unknown）的会话，不会被
    /// 后到的 `Unknown` 事件（外来 hook 误触发/竞态）冲回 Unknown——只有当
    /// 当前值本身就是 Unknown（首次坐实），或新值是另一个已知 agent（同一
    /// tab 里先跑 Claude 后来改跑 CodeBuddy 这种真实换 agent），才会覆盖。
    pub fn set_agent(&self, agent: AgentKind) {
        let mut metadata = self.agent_metadata.lock().expect("agent metadata");
        if (agent != AgentKind::Unknown || metadata.agent == AgentKind::Unknown)
            && agent != metadata.agent
        {
            metadata.agent = agent;
            metadata.revision += 1;
        }
    }

    /// hook 事件驱动的状态更新：记最新态 + 广播给本会话订阅者。
    pub fn set_agent_state(&self, state: AgentState, event: &str, ts_ms: u64) {
        let (agent, transcript_path) = {
            let mut metadata = self.agent_metadata.lock().expect("agent metadata");
            metadata.state = state;
            metadata.revision += 1;
            (metadata.agent, metadata.transcript_path.clone())
        };
        let _ = self.tx.send(SessionEvent::Agent {
            agent,
            state,
            event: event.to_string(),
            ts_ms,
            transcript_path,
        });
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn write(&self, data: &[u8]) -> Result<()> {
        if data.len() > MAX_INPUT_BYTES {
            anyhow::bail!("输入超过 {} 字节上限", MAX_INPUT_BYTES);
        }
        let previous = self.input_bytes.fetch_add(data.len(), Ordering::SeqCst);
        if previous + data.len() > MAX_INPUT_BYTES {
            self.input_bytes.fetch_sub(data.len(), Ordering::SeqCst);
            anyhow::bail!("输入队列已满（{} 字节预算）", MAX_INPUT_BYTES);
        }
        let sent = self
            .input_tx
            .lock()
            .expect("input lock")
            .as_ref()
            .is_some_and(|tx| tx.try_send(InputCommand::Write(data.to_vec())).is_ok());
        if !sent {
            self.input_bytes.fetch_sub(data.len(), Ordering::SeqCst);
            anyhow::bail!("输入队列已满或会话已经退出");
        }
        Ok(())
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        let queue = self.input_tx.lock().expect("input lock");
        let tx = queue
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("会话已经退出"))?;
        tx.try_send(InputCommand::Resize { cols, rows })
            .map_err(|_| anyhow::anyhow!("PTY 输入队列已满"))?;
        Ok(())
    }

    /// 当前排队未写出的正文字节数(测试/指标用)。
    #[cfg(test)]
    pub(crate) fn runtime_child_for_test(
        &self,
    ) -> Arc<Mutex<Option<Box<dyn Child + Send + Sync>>>> {
        self.child.clone()
    }
    pub fn runtime_handle_count(&self) -> usize {
        self.runtime_handles.load(Ordering::SeqCst)
    }
    pub fn input_queue_bytes(&self) -> usize {
        self.input_bytes.load(Ordering::SeqCst)
    }

    pub fn kill(&self) -> Result<()> {
        if self.exit_status().is_some() {
            return Ok(());
        }
        let mut child = self.child.lock().expect("child lock");
        let Some(child) = child.as_mut() else {
            return Ok(());
        };
        match child.kill() {
            Ok(()) => Ok(()),
            Err(error) if error.raw_os_error() == Some(libc::ESRCH) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    /// `Some(code)` 表示进程已退出；`code` 为 `None` 表示拿不到退出码。
    /// 独立于 PTY EOF；不能据此结束终端流，Attach 使用 `attach_snapshot`。
    pub fn exit_status(&self) -> Option<Option<i32>> {
        *self.exit_code.lock().expect("exit lock")
    }

    pub async fn wait_for_exit(&self) -> Option<i32> {
        let mut receiver = self.lifecycle.subscribe();
        loop {
            if let Some(code) = *receiver.borrow() {
                return code;
            }
            if receiver.changed().await.is_err() {
                return None;
            }
        }
    }

    pub(crate) fn retention_pinned(&self) -> bool {
        let output_open = self.buffer.lock().expect("ring lock").exit_status.is_none();
        output_open
            || (self
                .agent_metadata
                .lock()
                .expect("agent metadata")
                .transcript_path
                .is_some()
                && !self.summary_completed.load(Ordering::SeqCst))
    }
    pub(crate) fn claim_summary(&self) -> Option<SummaryClaim<'_>> {
        self.summary_claimed
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .ok()?;
        Some(SummaryClaim {
            state: &self.summary_claimed,
            completed: &self.summary_completed,
            committed: false,
        })
    }

    pub(crate) fn terminal_state(
        &self,
        output_exit: Option<Option<i32>>,
    ) -> dozer_core::protocol::TerminalState {
        let info = self.info();
        let output_truncated = self.buffer.lock().expect("ring lock").truncated;
        let metadata = self.agent_metadata.lock().expect("agent metadata");
        dozer_core::protocol::TerminalState {
            lifecycle_revision: if info.alive { 1 } else { 2 },
            agent_revision: metadata.revision,
            alive: info.alive,
            exit_code: self.exit_status().flatten(),
            output_complete: output_exit.is_some(),
            output_truncated,
            agent: metadata.agent,
            agent_state: metadata.state,
            transcript_path: metadata.transcript_path.clone(),
        }
    }

    pub fn exited_ms(&self) -> Option<u64> {
        *self.exited_ms.lock().expect("exit time lock")
    }

    pub fn history_len(&self) -> usize {
        self.buffer.lock().expect("ring lock").ring.buffered_len()
    }

    pub fn history_evicted(&self) -> bool {
        self.history_evicted.load(Ordering::SeqCst)
    }

    /// 只丢正文,保留会话元信息与 `next_offset` 坐标。只对已退出且无活动
    /// 订阅的会话调用(registry 负责判定)。
    pub fn evict_history(&self) {
        self.buffer.lock().expect("ring lock").ring.clear();
        self.history_evicted.store(true, Ordering::SeqCst);
    }

    pub fn snapshot(&self) -> (Vec<u8>, u64) {
        self.buffer.lock().expect("ring lock").ring.snapshot()
    }

    pub(crate) fn attach_snapshot(&self, from_offset: u64) -> AttachSnapshot {
        self.buffer
            .lock()
            .expect("ring lock")
            .attach_snapshot(from_offset)
    }

    pub fn read_from(&self, offset: u64) -> Option<Vec<u8>> {
        self.buffer
            .lock()
            .expect("ring lock")
            .ring
            .read_from(offset)
    }

    /// 单次加锁同时取数据与 next_offset，避免双锁间隙丢字节
    pub fn read_from_with_next(&self, offset: u64) -> Option<(Vec<u8>, u64)> {
        let buf = self.buffer.lock().expect("ring lock");
        buf.ring
            .read_from(offset)
            .map(|d| (d, buf.ring.total_written()))
    }

    pub fn subscribe(&self) -> broadcast::Receiver<SessionEvent> {
        self.tx.subscribe()
    }

    /// 当前存活的 broadcast 订阅数——每个连接的 `handle_conn` 在 attach
    /// 期间持有一个订阅，连接退出（EOF/关闭）时随 `sub` 一起被 drop。
    /// 主要用于测试：观察连接释放是否真的传导到了订阅计数归零。
    pub fn subscriber_count(&self) -> usize {
        self.tx.receiver_count()
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
            project_id: 1,
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
        assert!(
            wait_contains(&s, b"marker123").await,
            "buffer should contain output"
        );
        // broadcast 也应收到含 marker 的事件（可能分片，收多次拼接）
        let mut got = Vec::new();
        while let Ok(Ok(ev)) = tokio::time::timeout(Duration::from_millis(500), rx.recv()).await {
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
    async fn input_queue_rejects_when_budget_is_full_instead_of_dropping_keys() {
        // 子进程不读 stdin，PTY 内核缓冲满后 writer 线程阻塞，队列随之填满。
        let s = Session::spawn(spec("sleep 30")).unwrap();
        let mut rejected = false;
        for _ in 0..64 {
            match s.write(&vec![b'x'; 8192]) {
                Ok(()) => {}
                Err(error) => {
                    assert!(error.to_string().contains("输入队列已满"));
                    rejected = true;
                    break;
                }
            }
        }
        assert!(rejected, "队列满时必须明确失败");
        assert!(s.input_queue_bytes() <= MAX_INPUT_BYTES);
        s.kill().unwrap();
    }

    #[tokio::test]
    async fn spawned_child_has_utf8_locale_env() {
        let s = Session::spawn(spec("printf '%s' \"$LANG\"")).unwrap();
        assert!(
            wait_contains(&s, b"UTF-8").await,
            "buffer should contain a UTF-8 LANG value set by Session::spawn"
        );
        s.kill().unwrap();
    }

    #[tokio::test]
    async fn spawned_child_has_term_and_colorterm_env() {
        let s = Session::spawn(spec("printf '%s|%s' \"$TERM\" \"$COLORTERM\"")).unwrap();
        assert!(
            wait_contains(&s, b"xterm-256color|truecolor").await,
            "buffer should contain TERM/COLORTERM values set by Session::spawn"
        );
        s.kill().unwrap();
    }

    #[tokio::test]
    async fn zsh_session_gets_zdotdir_injected() {
        let s = Session::spawn(SessionSpec {
            name: "t".into(),
            command: "/bin/zsh".into(),
            args: vec!["-c".into(), "echo zd=$ZDOTDIR; sleep 5".into()],
            cwd: std::env::temp_dir().to_string_lossy().into_owned(),
            cols: 80,
            rows: 24,
            project_id: 1,
        })
        .unwrap();
        assert!(
            wait_contains(&s, b"zd=").await,
            "zsh 会话应有输出（zd= 行）"
        );
        let text = String::from_utf8_lossy(&s.snapshot().0).into_owned();
        assert!(text.contains("zdotdir"), "ZDOTDIR 应指向包装目录: {text}");
        let _ = s.kill();
    }

    #[tokio::test]
    async fn non_zsh_session_has_no_zdotdir() {
        let s = Session::spawn(spec("echo zd=[$ZDOTDIR]; sleep 5")).unwrap();
        assert!(wait_contains(&s, b"zd=[]").await, "非 zsh 不注入 ZDOTDIR");
        let _ = s.kill();
    }

    #[tokio::test]
    async fn spawn_strips_inherited_claude_code_child_session_marker() {
        // dozerd 若是被外层 claude 会话拉起的子进程,这个标记会原样出现在
        // dozerd 自己的进程 env 里；CommandBuilder 默认继承父进程 env，
        // 得显式 env_remove 掉，否则会话 shell 里手动跑的 claude 会把自己
        // 误认成外层会话的子会话，关掉 transcript saving。
        unsafe {
            std::env::set_var("CLAUDE_CODE_CHILD_SESSION", "1");
        }
        let spawned = Session::spawn(spec(
            "printf 'cc=%s' \"${CLAUDE_CODE_CHILD_SESSION:-unset}\"",
        ));
        unsafe {
            std::env::remove_var("CLAUDE_CODE_CHILD_SESSION");
        }
        let s = spawned.unwrap();
        assert!(
            wait_contains(&s, b"cc=unset").await,
            "spawn 应该把继承到的 CLAUDE_CODE_CHILD_SESSION 从子进程 env 里剔除"
        );
        s.kill().unwrap();
    }

    #[tokio::test]
    async fn spawn_injects_dozer_session_id_env() {
        // shell 里要展开的是环境变量本身,Rust 侧只负责把变量名拼进去
        let script = format!("echo id=${}; sleep 5", dozer_core::paths::session_env());
        let s = Session::spawn(spec(&script)).unwrap();
        let needle = format!("id={}", s.id());
        assert!(
            wait_contains(&s, needle.as_bytes()).await,
            "PTY 输出应含注入的会话 id"
        );
        let _ = s.kill();
    }

    #[tokio::test]
    async fn set_agent_state_updates_info_and_broadcasts() {
        let s = Session::spawn(spec("sleep 5")).unwrap();
        let mut rx = s.subscribe();
        s.set_agent_state(AgentState::Running, "UserPromptSubmit", 42);
        assert_eq!(s.info().agent_state, AgentState::Running);
        loop {
            match tokio::time::timeout(Duration::from_secs(5), rx.recv())
                .await
                .expect("agent 事件应在 5s 内到达")
                .unwrap()
            {
                SessionEvent::Agent {
                    state,
                    event,
                    ts_ms,
                    ..
                } => {
                    assert_eq!(state, AgentState::Running);
                    assert_eq!(event, "UserPromptSubmit");
                    assert_eq!(ts_ms, 42);
                    break;
                }
                _ => continue, // PTY 启动输出等无关事件
            }
        }
        let _ = s.kill();
    }

    #[tokio::test]
    async fn transcript_path_stored_and_in_info_and_broadcast() {
        let s = Session::spawn(spec("sleep 5")).unwrap();
        assert_eq!(s.info().transcript_path, None);
        let mut rx = s.subscribe();
        s.set_transcript_path("/t/conv.jsonl");
        s.set_agent_state(AgentState::Running, "UserPromptSubmit", 1);
        assert_eq!(s.info().transcript_path.as_deref(), Some("/t/conv.jsonl"));
        loop {
            match tokio::time::timeout(Duration::from_secs(5), rx.recv())
                .await
                .unwrap()
                .unwrap()
            {
                SessionEvent::Agent {
                    transcript_path, ..
                } => {
                    assert_eq!(transcript_path.as_deref(), Some("/t/conv.jsonl"));
                    break;
                }
                _ => continue,
            }
        }
        let _ = s.kill();
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

    #[tokio::test]
    async fn late_observer_can_read_real_exit_code() {
        let s = Session::spawn(spec("exit 42")).unwrap();
        for _ in 0..100 {
            if s.exit_status().is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(s.exit_status(), Some(Some(42)));
        assert!(!s.info().alive);
    }

    #[tokio::test]
    async fn exited_event_arrives_after_tail_output() {
        let s = Session::spawn(spec("printf FINAL; exit 0")).unwrap();
        let mut rx = s.subscribe();
        let mut saw_output = false;
        for _ in 0..100 {
            match tokio::time::timeout(Duration::from_millis(100), rx.recv()).await {
                Ok(Ok(SessionEvent::Output { data, .. })) => {
                    saw_output |= data.windows(5).any(|window| window == b"FINAL");
                }
                Ok(Ok(SessionEvent::Exited { .. })) => {
                    assert!(saw_output, "Exited 不能早于尾部输出到达");
                    return;
                }
                _ => continue,
            }
        }
        panic!("未收到 Exited");
    }

    #[tokio::test]
    async fn attach_during_drain_does_not_publish_process_exit_as_stream_exit() {
        let dir = tempfile::tempdir().unwrap();
        let mut session_spec = spec("while [ ! -f go ]; do sleep 0.01; done; printf FINAL");
        session_spec.cwd = dir.path().to_string_lossy().into_owned();
        let s = Session::spawn(session_spec).unwrap();
        let mut rx = s.subscribe();
        {
            // 锁住正文提交，允许 child 先写完/退出、waiter 先记录退出码。
            // 这是 snapshot 临界区可导致的合法调度，不改生产 reader。
            let output = s.buffer.lock().unwrap();
            std::fs::write(dir.path().join("go"), b"go").unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            while s.exit_status().is_none() && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(2));
            }
            assert_eq!(s.exit_status(), Some(Some(0)));
            let snapshot = output.attach_snapshot(0);
            assert!(snapshot.bytes.is_empty());
            assert_eq!(snapshot.next_offset, 0);
            assert_eq!(snapshot.exit_status, None, "进程退出不代表正文已经排空");
        }

        let mut bytes = Vec::new();
        loop {
            let event = tokio::time::timeout(Duration::from_secs(2), rx.recv())
                .await
                .unwrap()
                .unwrap();
            match event {
                SessionEvent::Output { data, .. } => bytes.extend(data),
                SessionEvent::Exited { code } => {
                    assert_eq!(code, Some(0));
                    break;
                }
                _ => {}
            }
        }
        assert_eq!(bytes, b"FINAL");
        let snapshot = s.attach_snapshot(0);
        assert_eq!(snapshot.bytes, b"FINAL");
        assert_eq!(snapshot.next_offset, 5);
        assert_eq!(snapshot.exit_status, Some(Some(0)));
        let resumed = s.attach_snapshot(3);
        assert_eq!(resumed.bytes, b"AL");
        assert_eq!(resumed.exit_status, Some(Some(0)));
    }

    #[test]
    fn finished_output_cannot_publish_bytes_after_exited() {
        let mut output = OutputState::new();
        let (tx, mut rx) = broadcast::channel(8);
        output.append(b"FINAL".to_vec(), &tx);
        output.finish(Some(0), &tx);
        output.append(b"too late".to_vec(), &tx);
        output.finish(Some(1), &tx);
        assert!(matches!(
            rx.try_recv(),
            Ok(SessionEvent::Output { offset: 5, .. })
        ));
        assert!(matches!(
            rx.try_recv(),
            Ok(SessionEvent::Exited { code: Some(0) })
        ));
        assert!(matches!(
            rx.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
        let snapshot = output.attach_snapshot(0);
        assert_eq!(snapshot.bytes, b"FINAL");
        assert_eq!(snapshot.exit_status, Some(Some(0)));
    }

    #[tokio::test]
    async fn set_agent_updates_info_and_is_included_in_broadcast() {
        let s = Session::spawn(spec("sleep 5")).unwrap();
        assert_eq!(s.info().agent, AgentKind::Unknown);
        let mut rx = s.subscribe();
        s.set_agent(AgentKind::Codebuddy);
        assert_eq!(s.info().agent, AgentKind::Codebuddy);
        s.set_agent_state(AgentState::Running, "PreToolUse", 1);
        loop {
            match tokio::time::timeout(Duration::from_secs(5), rx.recv())
                .await
                .unwrap()
                .unwrap()
            {
                SessionEvent::Agent { agent, .. } => {
                    assert_eq!(agent, AgentKind::Codebuddy);
                    break;
                }
                _ => continue,
            }
        }
        let _ = s.kill();
    }

    #[tokio::test]
    async fn set_agent_refuses_to_downgrade_known_agent_to_unknown() {
        let s = Session::spawn(spec("sleep 5")).unwrap();
        // 首次坐实：默认 Unknown → 真实 agent 应该生效。
        assert_eq!(s.info().agent, AgentKind::Unknown);
        s.set_agent(AgentKind::Codebuddy);
        assert_eq!(s.info().agent, AgentKind::Codebuddy);

        // 一个已坐实的 agent 不该被后到的 Unknown（外来 hook 误触发/竞态）冲回去。
        s.set_agent(AgentKind::Unknown);
        assert_eq!(
            s.info().agent,
            AgentKind::Codebuddy,
            "已知 agent 不该被 Unknown 降级"
        );

        // 换成另一个已知 agent（真的换 agent 跑）仍然允许覆盖。
        s.set_agent(AgentKind::Claude);
        assert_eq!(
            s.info().agent,
            AgentKind::Claude,
            "已知 agent 之间的切换应该仍然允许覆盖"
        );

        let _ = s.kill();
    }

    #[tokio::test]
    async fn info_carries_project_id_from_spec() {
        let s = Session::spawn(SessionSpec {
            project_id: 42,
            ..spec("sleep 5")
        })
        .unwrap();
        assert_eq!(s.info().project_id, Some(42));
        let _ = s.kill();
    }
    #[tokio::test]
    async fn exiting_child_releases_runtime_handles_even_with_unread_input() {
        let session = Session::spawn(spec("stty -icanon -echo; printf READY; sleep 30")).unwrap();
        let mut output = session.subscribe();
        while !session.snapshot().0.windows(5).any(|b| b == b"READY") {
            tokio::time::timeout(Duration::from_secs(3), output.recv())
                .await
                .unwrap()
                .unwrap();
        }
        session.write(&vec![b'x'; MAX_INPUT_BYTES]).unwrap();
        session.kill().unwrap();
        session.wait_for_exit().await;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        while session.runtime_handle_count() > 0 {
            assert!(
                tokio::time::Instant::now() < deadline,
                "runtime handles retained after exit"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(session.input_queue_bytes(), 0);
    }
    #[test]
    fn output_drain_deadline_discloses_truncation_and_seals_late_bytes() {
        let buffer = Mutex::new(OutputState::new());
        let drained = AtomicBool::new(false);
        let (tx, _) = broadcast::channel(4);
        buffer.lock().unwrap().append(b"known".to_vec(), &tx);
        finish_output_with_budget(&buffer, &drained, &tx, Some(7), Duration::ZERO);
        let mut output = buffer.lock().unwrap();
        assert!(output.truncated);
        output.append(b"late".to_vec(), &tx);
        assert_eq!(output.attach_snapshot(0).bytes, b"known");
        assert_eq!(output.exit_status, Some(Some(7)));
    }
}
