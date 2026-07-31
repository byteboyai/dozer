use crate::ring::{RingBuffer, SCROLLBACK_CAP};
use anyhow::{Context, Result};
use dozer_core::protocol::{AgentKind, AgentState, SessionInfo};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

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

pub struct Session {
    id: String,
    spec: SessionSpec,
    created_ms: u64,
    alive: Arc<AtomicBool>,
    agent_state: Mutex<AgentState>,
    agent: Mutex<AgentKind>,
    transcript_path: Mutex<Option<String>>,
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
        cmd.env("DOZER_SESSION_ID", &id);
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
                Err(e) => tracing::warn!("shell 集成落盘失败，跳过注入: {e}"),
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
            id,
            created_ms: now_ms(),
            alive,
            agent_state: Mutex::new(AgentState::default()),
            agent: Mutex::new(AgentKind::default()),
            transcript_path: Mutex::new(None),
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
            agent: *self.agent.lock().expect("agent lock"),
            agent_state: *self.agent_state.lock().expect("agent_state lock"),
            transcript_path: self.transcript_path.lock().expect("tp lock").clone(),
            project_id: Some(self.spec.project_id),
        }
    }

    /// 记下会话的 transcript 路径（hook data 携带；覆盖旧值）。
    pub fn set_transcript_path(&self, path: &str) {
        *self.transcript_path.lock().expect("tp lock") = Some(path.to_string());
    }

    /// 记下会话归属的 agent（首个 hook 事件到达时坐实；覆盖旧值）。
    ///
    /// 但拒绝“降级”：一个已经坐实为具体 agent（非 Unknown）的会话，不会被
    /// 后到的 `Unknown` 事件（外来 hook 误触发/竞态）冲回 Unknown——只有当
    /// 当前值本身就是 Unknown（首次坐实），或新值是另一个已知 agent（同一
    /// tab 里先跑 Claude 后来改跑 CodeBuddy 这种真实换 agent），才会覆盖。
    pub fn set_agent(&self, agent: AgentKind) {
        let mut current = self.agent.lock().expect("agent lock");
        if agent != AgentKind::Unknown || *current == AgentKind::Unknown {
            *current = agent;
        }
    }

    /// hook 事件驱动的状态更新：记最新态 + 广播给本会话订阅者。
    pub fn set_agent_state(&self, state: AgentState, event: &str, ts_ms: u64) {
        *self.agent_state.lock().expect("agent_state lock") = state;
        let transcript_path = self.transcript_path.lock().expect("tp lock").clone();
        let agent = *self.agent.lock().expect("agent lock");
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
        let mut w = self.writer.lock().expect("writer lock");
        w.write_all(data)?;
        w.flush()?;
        Ok(())
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        self.master.lock().expect("master lock").resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
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

    /// 单次加锁同时取数据与 next_offset，避免双锁间隙丢字节
    pub fn read_from_with_next(&self, offset: u64) -> Option<(Vec<u8>, u64)> {
        let buf = self.buffer.lock().expect("ring lock");
        buf.read_from(offset).map(|d| (d, buf.total_written()))
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
    async fn spawn_injects_dozer_session_id_env() {
        let s = Session::spawn(spec("echo id=$DOZER_SESSION_ID; sleep 5")).unwrap();
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
}
