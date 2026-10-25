//! Agent 面板的消息与更新(S6c-1):新建、attach、输出摄取、状态变更、
//! 选择器、关闭与确认、尺寸同步、输入写入。

use crate::adapter::{AgentCapabilities, AgentHostEvent, AgentRequest, HookInstaller};
use crate::i18n;
use crate::launch::{
    PickerLaunch, attached_agent, picker_launch_command, should_answer_dynamic_color,
};
use crate::state::AgentPanelState;
use crate::tabs::{
    SessionTab, TabBackend, exited_marker, send_to_tab, should_summarize_on_close,
    tab_display_width, tab_title,
};
use dozer_model::{AgentKind, AgentState, ProjectInfo, SessionEvent, SessionInfo};
use dozer_panel_host::NoticeLevel;
use dozer_panel_term::model::TerminalModel;
use dozer_panel_term::osc::OscScanner;
use dozer_services::{SessionService, SessionSpec};
use std::sync::Arc;
use tokio::sync::mpsc;

/// 面板自有消息。
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    /// 键盘/IME 字节(已翻译)。
    Input(Vec<u8>),
    /// 粘贴文本(按 bracketed paste 模式包裹)。
    Paste(String),
    /// 终端滚轮，仅作用于活动页签。
    Scroll(i32),
    /// 开始更新活动页签的鼠标选区。
    SelStart {
        col: usize,
        row: usize,
        right: bool,
    },
    /// 继续更新活动页签的鼠标选区。
    SelUpdate {
        col: usize,
        row: usize,
        right: bool,
    },
    /// 同步所有存活页签的终端网格尺寸。
    Resize {
        cols: u16,
        rows: u16,
    },
    Output {
        tab_id: usize,
        bytes: Vec<u8>,
    },
    Exited {
        tab_id: usize,
    },
    AgentState {
        tab_id: usize,
        agent: AgentKind,
        state: AgentState,
        transcript_path: Option<String>,
    },
    SelectTab(usize),
    SelectTabNoDrag(usize),
    CloseTab(usize),
    CloseConfirm,
    CloseCancel,
    PickerToggle,
    PickerClose,
    PickerSelect(PickerLaunch),
    Attached {
        tab_id: usize,
        info: SessionInfo,
        snapshot: Vec<u8>,
        picked_agent: Option<AgentKind>,
    },
    OverflowToggle {
        at: Option<(f32, f32)>,
    },
    OverflowDismiss,
    /// 悬停状态变化(产品映射到自己的 HoverId)。
    Hover(crate::display::AgentHover, bool),
    /// 卡片刷新结果(库内 `card::spawn_refresh` 回灌)。
    CardRefreshed {
        tab_id: usize,
        llm_model: Option<String>,
        mode: Option<String>,
        activity: Option<String>,
        workspace: Option<crate::tabs::WorkspaceGitInfo>,
    },
    /// 异步失败:统一经 `outbox` 出一条带 key 的提示。
    Failed {
        key: &'static str,
        error: String,
    },
}

/// 更新所需的宿主上下文(全部按值/借用注入,库不持有产品状态)。
pub struct UpdateCtx<'a> {
    pub project: &'a ProjectInfo,
    pub svc: &'a Arc<dyn SessionService>,
    pub handle: &'a tokio::runtime::Handle,
    pub cols: u16,
    pub rows: u16,
    pub term_tab_bar_avail_px: f32,
    pub caps: AgentCapabilities,
    pub hooks: &'a Arc<dyn HookInstaller>,
    /// 退出前必须等待的任务(kill/总结)。Dozer 传 `ShellIo::track_exit_critical`。
    pub track_exit_critical: &'a dyn Fn(tokio::task::JoinHandle<()>),
}

const KEY_NEW_SESSION: &str = "new-session-failed";
const KEY_ATTACH_SESSION: &str = "attach-session-failed";
const KEY_INITIAL_WRITE: &str = "term-initial-write";
const KEY_DISPATCH_WRITE: &str = "term-dispatch-write";

fn failure_text(key: &str, error: &str) -> dozer_model::LocalizedText {
    match key {
        KEY_NEW_SESSION => i18n::new_session_failed(error),
        KEY_ATTACH_SESSION => i18n::attach_session_failed(error),
        KEY_INITIAL_WRITE => i18n::term_initial_write_failed(error),
        KEY_DISPATCH_WRITE => i18n::term_dispatch_write_failed(error),
        _ => i18n::new_session_failed(error),
    }
}

pub fn update(
    state: &mut AgentPanelState,
    msg: Message,
    ctx: &UpdateCtx<'_>,
    emit: impl Fn(Message) + Clone + Send + 'static,
) {
    match msg {
        Message::Input(bytes) => {
            let Some(tab) = state.tabs.get(state.active) else {
                return;
            };
            send_to_tab(tab, ctx.svc, ctx.handle, bytes);
        }
        Message::Paste(text) => {
            let Some(tab) = state.tabs.get_mut(state.active) else {
                return;
            };
            tab.model.scroll_to_bottom();
            let bytes = if tab.model.bracketed_paste() {
                let mut b = b"\x1b[200~".to_vec();
                b.extend_from_slice(text.as_bytes());
                b.extend_from_slice(b"\x1b[201~");
                b
            } else {
                text.into_bytes()
            };
            send_to_tab(tab, ctx.svc, ctx.handle, bytes);
        }
        Message::Scroll(delta) => {
            if let Some(tab) = state.tabs.get_mut(state.active) {
                tab.model.scroll_display(delta);
            }
        }
        Message::SelStart { col, row, right } => {
            if let Some(tab) = state.tabs.get_mut(state.active) {
                tab.model.selection_start(col, row, right);
            }
        }
        Message::SelUpdate { col, row, right } => {
            if let Some(tab) = state.tabs.get_mut(state.active) {
                tab.model.selection_update(col, row, right);
            }
        }
        Message::Resize { cols, rows } => {
            for tab in &mut state.tabs {
                crate::tabs::resize_tab(tab, ctx.svc, ctx.handle, cols, rows);
            }
        }
        Message::Output { tab_id, bytes } => {
            let Some(tab) = state.tab_by_id_mut(tab_id) else {
                return;
            };
            tab.ingest_osc(&bytes);
            let responses = tab.model.feed(&bytes);
            if responses.is_empty() || !tab.alive {
                return;
            }
            send_to_tab(tab, ctx.svc, ctx.handle, responses);
        }
        Message::Exited { tab_id } => {
            let Some(tab) = state.tab_by_id_mut(tab_id) else {
                return;
            };
            tab.alive = false;
            let _ = tab.model.feed(&exited_marker());
            state
                .host_events
                .push(AgentHostEvent::SessionExited { tab_id });
        }
        Message::AgentState {
            tab_id,
            agent,
            state: agent_state,
            transcript_path,
        } => {
            let Some(tab) = state.tab_by_id_mut(tab_id) else {
                return;
            };
            tab.agent_state = agent_state;
            tab.agent = agent;
            tab.model
                .set_answer_dynamic_color(agent == AgentKind::Opencode);
            if let Some(tp) = transcript_path.clone() {
                tab.transcript_path = Some(tp);
            }
            state.host_events.push(AgentHostEvent::StateChanged {
                tab_id,
                agent,
                state: agent_state,
                transcript_path,
            });
            if agent_state == AgentState::TurnEnded {
                state.host_events.push(AgentHostEvent::TurnEnded { tab_id });
            }
            let (transcript_path, cwd) = {
                let Some(tab) = state.tab_by_id(tab_id) else {
                    return;
                };
                (tab.transcript_path.clone(), tab.effective_cwd())
            };
            crate::card::spawn_refresh(
                ctx.handle,
                emit,
                tab_id,
                agent,
                transcript_path,
                cwd,
                Some(std::path::PathBuf::from(&ctx.project.path)),
            );
        }
        Message::SelectTab(idx) | Message::SelectTabNoDrag(idx) => {
            if idx < state.tabs.len() {
                state.active = idx;
                state.term_tab_first = reveal_tab(state, idx, ctx.term_tab_bar_avail_px);
                state.term_tab_overflow_anchor = None;
            }
        }
        Message::CloseTab(idx) => {
            let confirm = state.tabs.get(idx).is_some_and(|t| {
                matches!(
                    t.agent_state,
                    AgentState::Running | AgentState::AwaitingInput
                )
            });
            if confirm {
                state.pending_close_tab = state.tabs.get(idx).map(|t| t.info.id.clone());
            } else {
                close_tab(state, ctx, idx);
                ensure_project_terminal(state, ctx, emit);
            }
        }
        Message::CloseConfirm => {
            if let Some(id) = state.pending_close_tab.take()
                && let Some(idx) = state.tabs.iter().position(|t| t.info.id == id)
            {
                close_tab(state, ctx, idx);
                ensure_project_terminal(state, ctx, emit);
            }
        }
        Message::CloseCancel => state.pending_close_tab = None,
        Message::PickerToggle => {
            state.agent_picker_open = true;
            state.requests.push(AgentRequest::ShowPicker { at: None });
        }
        Message::PickerClose => state.agent_picker_open = false,
        Message::PickerSelect(launch) => {
            state.agent_picker_open = false;
            spawn_new_tab(state, ctx, emit, launch, None);
        }
        Message::Attached {
            tab_id,
            info,
            snapshot,
            picked_agent,
        } => {
            let Some(forwarder) = state.pending.remove(&tab_id) else {
                return;
            };
            let mut model = TerminalModel::new(ctx.cols, ctx.rows);
            model.set_answer_dynamic_color(should_answer_dynamic_color(picked_agent, info.agent));
            let _ = model.feed(&snapshot);
            let agent = attached_agent(picked_agent, info.agent);
            let mut tab = SessionTab {
                agent_state: info.agent_state,
                agent,
                transcript_path: info.transcript_path.clone(),
                info,
                model,
                alive: true,
                tab_id,
                forwarder,
                osc: OscScanner::new(),
                cwd: None,
                last_exit: None,
                // Codex 启动到第一条消息之间没有 hook:先用配置里的默认模型兜底。
                llm_model: crate::codex_config::initial_llm_model(
                    agent,
                    crate::codex_config::default_model,
                ),
                permission_mode: None,
                last_activity: None,
                workspace_override: None,
                backend: TabBackend::Daemon,
            };
            tab.ingest_osc(&snapshot);
            state.tabs.push(tab);
            state.active = state.tabs.len() - 1;
            state.term_tab_first = reveal_tab(state, state.active, ctx.term_tab_bar_avail_px);
            state.term_tab_overflow_anchor = None;
        }
        Message::OverflowToggle { at } => {
            state.term_tab_overflow_anchor = if state.term_tab_overflow_anchor.is_some() {
                None
            } else {
                Some(at.unwrap_or((0.0, 0.0)))
            };
        }
        Message::OverflowDismiss => state.term_tab_overflow_anchor = None,
        Message::Hover(_, _) => {}
        Message::CardRefreshed {
            tab_id,
            llm_model,
            mode,
            activity,
            workspace,
        } => {
            crate::card::apply_refresh(
                &mut state.tabs,
                tab_id,
                llm_model,
                mode,
                activity,
                workspace,
            );
        }
        Message::Failed { key, error } => {
            state
                .outbox
                .push_keyed(NoticeLevel::Error, failure_text(key, &error), key);
        }
    }
}

fn reveal_tab(state: &AgentPanelState, target: usize, avail_px: f32) -> usize {
    let widths: Vec<f32> = state
        .tabs
        .iter()
        .map(|t| tab_display_width(&tab_title(t.agent, t.cwd.as_deref(), &t.info.name)))
        .collect();
    byteui::interaction::tab_strip::tab_window_reveal(
        &widths,
        4.0,
        avail_px,
        state.term_tab_first,
        target,
    )
}

/// 把页签 `from` 挪到 `to`,并让 `active` 仍指向同一个会话。越界或 `from == to`
/// 返回 `false`、不改任何东西。
///
/// 这是纯换位,不含拖拽手势:什么时候算"在拖"(按下武装、位移阈值、组校验)
/// 是产品的拖拽状态机的事——库曾带过一个"消息一到就换位"的版本,会让光标划过
/// 页签就触发换位(2026-08-17 的真实 bug),已删除;换位入口只留这个显式的
/// `from`/`to` 版本,来源下标由调用方维护,不再借用 `active`。
pub fn reorder_term_tab(state: &mut AgentPanelState, from: usize, to: usize) -> bool {
    let len = state.tabs.len();
    if from == to || from >= len || to >= len {
        return false;
    }
    let tab = state.tabs.remove(from);
    state.tabs.insert(to, tab);
    state.active = if state.active == from {
        to
    } else if from < state.active && state.active <= to {
        state.active - 1
    } else if to <= state.active && state.active < from {
        state.active + 1
    } else {
        state.active
    };
    true
}

pub fn close_tab(state: &mut AgentPanelState, ctx: &UpdateCtx<'_>, idx: usize) {
    if idx >= state.tabs.len() {
        return;
    }
    let tab = state.tabs.remove(idx);
    tab.forwarder.abort();
    let summarize =
        ctx.caps.summary_on_close && should_summarize_on_close(tab.agent, tab.alive, &tab.backend);
    if summarize {
        let svc = Arc::clone(ctx.svc);
        let id = tab.info.id.clone();
        let task = ctx.handle.spawn(async move {
            if let Err(e) = svc.close_with_summary(id).await {
                tracing::warn!("关闭 tab 时触发总结失败: {e}");
            }
        });
        (ctx.track_exit_critical)(task);
    } else if tab.alive && matches!(tab.backend, TabBackend::Daemon) {
        let svc = Arc::clone(ctx.svc);
        let id = tab.info.id.clone();
        let task = ctx.handle.spawn(async move {
            if let Err(e) = svc.kill(id).await {
                tracing::warn!("关闭 tab 时结束会话失败: {e}");
            }
        });
        (ctx.track_exit_critical)(task);
    }
    if state.active >= state.tabs.len() {
        state.active = state.tabs.len().saturating_sub(1);
    } else if idx < state.active {
        state.active -= 1;
    }
    state.term_tab_first = 0;
}

pub fn ensure_project_terminal(
    state: &mut AgentPanelState,
    ctx: &UpdateCtx<'_>,
    emit: impl Fn(Message) + Clone + Send + 'static,
) {
    if state.tabs.is_empty() {
        spawn_new_tab(state, ctx, emit, PickerLaunch::Agent(None), None);
    }
}

/// 新建会话:先装 hook/MCP(仅已知 agent),再 create + attach,成功后
/// 发送 `Attached` 并启动事件转发。失败经 `Failed` 出提示,不产生页签。
pub fn spawn_new_tab(
    state: &mut AgentPanelState,
    ctx: &UpdateCtx<'_>,
    emit: impl Fn(Message) + Clone + Send + 'static,
    launch: PickerLaunch,
    follow_up: Option<String>,
) -> Option<usize> {
    let tab_id = state.next_tab_id;
    state.next_tab_id += 1;

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    let project_id = ctx.project.id;
    let cwd = ctx.project.path.clone();
    let (cols, rows) = (ctx.cols, ctx.rows);
    let svc = Arc::clone(ctx.svc);
    let hooks = Arc::clone(ctx.hooks);
    let caps = ctx.caps;
    let hook_agent = match launch {
        PickerLaunch::Agent(Some(agent)) => Some(agent),
        _ => None,
    };

    let emit_fail = emit.clone();
    let jh = ctx.handle.spawn(async move {
        if let Some(agent) = hook_agent {
            let hook_hooks = Arc::clone(&hooks);
            let _ = tokio::task::spawn_blocking(move || hook_hooks.ensure_hook(agent)).await;
            if caps.mcp_install {
                let mcp_hooks = Arc::clone(&hooks);
                let _ = tokio::task::spawn_blocking(move || mcp_hooks.ensure_mcp(agent)).await;
            }
        }
        let spec = SessionSpec {
            name: "shell".into(),
            command: shell,
            args: Vec::new(),
            cwd,
            cols,
            rows,
            project_id,
            agent: hook_agent.unwrap_or(AgentKind::Unknown),
        };
        let info = match svc.create(spec).await {
            Ok(info) => info,
            Err(e) => {
                emit_fail(Message::Failed {
                    key: KEY_NEW_SESSION,
                    error: e.to_string(),
                });
                return;
            }
        };
        match svc.attach(info.id.clone(), 0).await {
            Ok((snapshot, _next_offset, rx)) => {
                let session_id = info.id.clone();
                emit(Message::Attached {
                    tab_id,
                    info,
                    snapshot,
                    picked_agent: hook_agent,
                });
                if let Some(cmd) = picker_launch_command(launch, &hooks.hook_exe()) {
                    let bytes = format!("{cmd}\n").into_bytes();
                    if let Err(e) = svc.write(session_id.clone(), bytes).await {
                        emit(Message::Failed {
                            key: KEY_INITIAL_WRITE,
                            error: e.to_string(),
                        });
                    }
                }
                if let Some(text) = follow_up {
                    let bytes = format!("{text}\n").into_bytes();
                    if let Err(e) = svc.write(session_id.clone(), bytes).await {
                        emit(Message::Failed {
                            key: KEY_DISPATCH_WRITE,
                            error: e.to_string(),
                        });
                    }
                }
                forward_events(tab_id, rx, emit).await;
            }
            Err(e) => {
                // create 已在守护进程里建好会话,但没有 `Attached`,产品拿不到会话 id,
                // 没人能结束它。先尽力 kill,再上报 attach 的错误;kill 失败不覆盖它。
                let _ = svc.kill(info.id.clone()).await;
                emit(Message::Failed {
                    key: KEY_ATTACH_SESSION,
                    error: e.to_string(),
                });
            }
        }
    });

    state.pending.insert(tab_id, jh);
    Some(tab_id)
}

/// attach 事件 → 面板消息。`Lagged` 跳过,退出/断连后在送出一条
/// `Exited` 后结束转发。
pub async fn forward_events(
    tab_id: usize,
    mut rx: mpsc::UnboundedReceiver<SessionEvent>,
    emit: impl Fn(Message) + Send + 'static,
) {
    while let Some(event) = rx.recv().await {
        let (message, stop) = match event {
            SessionEvent::Output(bytes) => (Message::Output { tab_id, bytes }, false),
            SessionEvent::Exited(_) | SessionEvent::Disconnected => {
                (Message::Exited { tab_id }, true)
            }
            SessionEvent::Lagged => {
                tracing::warn!(tab_id, "终端事件滞后（lagged），可能丢失部分历史输出");
                continue;
            }
            SessionEvent::Agent {
                agent,
                state,
                transcript_path,
            } => (
                Message::AgentState {
                    tab_id,
                    agent,
                    state,
                    transcript_path,
                },
                false,
            ),
        };
        emit(message);
        if stop {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::AgentHostEvent;
    use crate::tabs::TabOut;
    use dozer_model::{AgentKind, AgentState, SessionInfo};
    use dozer_services::{BoxFuture, ServiceError, ServiceErrorKind};
    use std::sync::{Arc, Mutex};

    // ---- 测试替身 ----

    #[derive(Default, Clone)]
    struct FakeSvc {
        inner: Arc<Mutex<FakeState>>,
    }

    #[derive(Default)]
    struct FakeState {
        calls: Vec<String>,
        sessions: Vec<SessionInfo>,
        snapshot: Vec<u8>,
        events: Vec<SessionEvent>,
        create_err: Option<ServiceError>,
        attach_err: Option<ServiceError>,
        write_err: Option<ServiceError>,
        resize_err: Option<ServiceError>,
        kill_err: Option<ServiceError>,
        close_err: Option<ServiceError>,
    }

    impl FakeSvc {
        fn calls(&self) -> Vec<String> {
            self.inner.lock().unwrap().calls.clone()
        }
        fn with_create_err(self, e: ServiceError) -> Self {
            self.inner.lock().unwrap().create_err = Some(e);
            self
        }
        fn with_attach_err(self, e: ServiceError) -> Self {
            self.inner.lock().unwrap().attach_err = Some(e);
            self
        }
        fn with_kill_err(self, e: ServiceError) -> Self {
            self.inner.lock().unwrap().kill_err = Some(e);
            self
        }
        fn with_write_err(self, e: ServiceError) -> Self {
            self.inner.lock().unwrap().write_err = Some(e);
            self
        }
    }

    fn err() -> ServiceError {
        ServiceError::new(ServiceErrorKind::Rejected, "boom")
    }

    impl SessionService for FakeSvc {
        fn list_sessions(&self) -> BoxFuture<'_, Result<Vec<SessionInfo>, ServiceError>> {
            let r = self.inner.lock().unwrap().sessions.clone();
            Box::pin(async move { Ok(r) })
        }
        fn create(&self, spec: SessionSpec) -> BoxFuture<'_, Result<SessionInfo, ServiceError>> {
            let mut s = self.inner.lock().unwrap();
            s.calls.push(format!("create({})", spec.name));
            let r = match s.create_err.clone() {
                Some(e) => Err(e),
                None => Ok(SessionInfo {
                    id: "fake-session".into(),
                    name: spec.name,
                    command: spec.command,
                    cwd: spec.cwd,
                    alive: true,
                    created_ms: 0,
                    agent_state: AgentState::Idle,
                    transcript_path: None,
                    project_id: Some(spec.project_id),
                    agent: spec.agent,
                }),
            };
            Box::pin(async move { r })
        }
        fn attach(
            &self,
            id: String,
            from_offset: u64,
        ) -> BoxFuture<
            '_,
            Result<
                (
                    Vec<u8>,
                    u64,
                    tokio::sync::mpsc::UnboundedReceiver<SessionEvent>,
                ),
                ServiceError,
            >,
        > {
            let mut s = self.inner.lock().unwrap();
            s.calls.push(format!("attach({id},{from_offset})"));
            let attach_err = s.attach_err.clone();
            let snapshot = std::mem::take(&mut s.snapshot);
            let events = std::mem::take(&mut s.events);
            let next = snapshot.len() as u64;
            Box::pin(async move {
                if let Some(e) = attach_err {
                    return Err(e);
                }
                let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
                for ev in events {
                    let _ = tx.send(ev);
                }
                Ok((snapshot, next, rx))
            })
        }
        fn write(&self, id: String, data: Vec<u8>) -> BoxFuture<'_, Result<(), ServiceError>> {
            let mut s = self.inner.lock().unwrap();
            s.calls.push(format!("write({id},{}B)", data.len()));
            let r = match s.write_err.clone() {
                Some(e) => Err(e),
                None => Ok(()),
            };
            Box::pin(async move { r })
        }
        fn resize(
            &self,
            id: String,
            cols: u16,
            rows: u16,
        ) -> BoxFuture<'_, Result<(), ServiceError>> {
            let mut s = self.inner.lock().unwrap();
            s.calls.push(format!("resize({id},{cols},{rows})"));
            let r = match s.resize_err.clone() {
                Some(e) => Err(e),
                None => Ok(()),
            };
            Box::pin(async move { r })
        }
        fn kill(&self, id: String) -> BoxFuture<'_, Result<(), ServiceError>> {
            let mut s = self.inner.lock().unwrap();
            s.calls.push(format!("kill({id})"));
            let r = match s.kill_err.clone() {
                Some(e) => Err(e),
                None => Ok(()),
            };
            Box::pin(async move { r })
        }
        fn close_with_summary(&self, id: String) -> BoxFuture<'_, Result<(), ServiceError>> {
            let mut s = self.inner.lock().unwrap();
            s.calls.push(format!("close_with_summary({id})"));
            let r = match s.close_err.clone() {
                Some(e) => Err(e),
                None => Ok(()),
            };
            Box::pin(async move { r })
        }
    }

    #[derive(Default)]
    struct FakeHooks {
        calls: Mutex<Vec<String>>,
    }
    impl HookInstaller for FakeHooks {
        fn ensure_hook(&self, agent: AgentKind) {
            self.calls.lock().unwrap().push(format!("hook({agent:?})"));
        }
        fn ensure_mcp(&self, agent: AgentKind) {
            self.calls.lock().unwrap().push(format!("mcp({agent:?})"));
        }
        fn hook_exe(&self) -> String {
            "/x/dozer-hook".into()
        }
    }

    fn project() -> ProjectInfo {
        ProjectInfo {
            id: 7,
            name: "demo".into(),
            path: "/tmp/demo".into(),
            last_active_ms: 0,
            created_ms: 0,
            updated_ms: 0,
        }
    }

    fn info(id: &str, agent: AgentKind, state: AgentState) -> SessionInfo {
        SessionInfo {
            id: id.into(),
            name: "sess".into(),
            command: "sh".into(),
            cwd: "/tmp".into(),
            alive: true,
            created_ms: 0,
            agent_state: state,
            transcript_path: None,
            project_id: Some(7),
            agent,
        }
    }

    fn make_tab(rt: &tokio::runtime::Handle, tab_id: usize, agent: AgentKind) -> SessionTab {
        SessionTab {
            info: info(&format!("s{tab_id}"), agent, AgentState::Idle),
            model: TerminalModel::new(20, 4),
            alive: true,
            agent_state: AgentState::Idle,
            agent,
            transcript_path: None,
            osc: OscScanner::new(),
            cwd: None,
            last_exit: None,
            llm_model: None,
            permission_mode: None,
            last_activity: None,
            workspace_override: None,
            tab_id,
            forwarder: rt.spawn(async {}),
            backend: TabBackend::Daemon,
        }
    }

    #[tokio::test]
    async fn reorder_term_tab_moves_the_tab_and_keeps_active_on_the_same_session() {
        let handle = tokio::runtime::Handle::current();
        let order = |s: &AgentPanelState| s.tabs.iter().map(|t| t.tab_id).collect::<Vec<_>>();
        let fresh = |active: usize| {
            let mut state = AgentPanelState::new();
            state.tabs = (0..4)
                .map(|i| make_tab(&handle, i, AgentKind::Unknown))
                .collect();
            state.active = active;
            state
        };
        // 被拖的就是当前页签:active 跟着走。
        let mut s = fresh(0);
        assert!(reorder_term_tab(&mut s, 0, 2));
        assert_eq!((order(&s), s.active), (vec![1, 2, 0, 3], 2));
        // 当前页签夹在 from 与 to 之间:向后拖前面的页签,active 前移一位。
        let mut s = fresh(1);
        assert!(reorder_term_tab(&mut s, 0, 2));
        assert_eq!((order(&s), s.active), (vec![1, 2, 0, 3], 0));
        // 反方向:把后面的页签拖到前面,active 后移一位。
        let mut s = fresh(1);
        assert!(reorder_term_tab(&mut s, 3, 0));
        assert_eq!((order(&s), s.active), (vec![3, 0, 1, 2], 2));
        // 不在区间内的 active 不变,且每种情况下 active 指的仍是原来那个会话。
        let mut s = fresh(3);
        assert!(reorder_term_tab(&mut s, 0, 2));
        assert_eq!((order(&s), s.active), (vec![1, 2, 0, 3], 3));
    }

    #[tokio::test]
    async fn reorder_term_tab_ignores_out_of_range_and_noop_moves() {
        let handle = tokio::runtime::Handle::current();
        let mut state = AgentPanelState::new();
        state.tabs = (0..3)
            .map(|i| make_tab(&handle, i, AgentKind::Unknown))
            .collect();
        state.active = 1;
        for (from, to) in [(1, 1), (3, 0), (0, 3), (9, 9)] {
            assert!(!reorder_term_tab(&mut state, from, to), "{from}->{to}");
        }
        let ids: Vec<_> = state.tabs.iter().map(|t| t.tab_id).collect();
        assert_eq!((ids, state.active), (vec![0, 1, 2], 1));
        // 单个页签没有可换的位置。
        state.tabs.truncate(1);
        state.active = 0;
        assert!(!reorder_term_tab(&mut state, 0, 0));
    }

    #[tokio::test]
    async fn scroll_moves_the_display_of_the_active_tab_only() {
        let harness = Harness::new();
        let svc = harness.svc();
        let handle = tokio::runtime::Handle::current();
        let tracked = |_| {};
        let ctx = harness.ctx(&svc, &handle, &tracked);
        let mut state = AgentPanelState::new();
        let mut first = make_tab(&handle, 0, AgentKind::Unknown);
        let mut second = make_tab(&handle, 1, AgentKind::Unknown);
        let _ = first.model.feed(b"one\ntwo\nthree\nfour\nfive\n");
        let _ = second.model.feed(b"one\ntwo\nthree\nfour\nfive\n");
        state.tabs = vec![first, second];
        state.active = 1;
        update(&mut state, Message::Scroll(2), &ctx, |_| {});
        assert_eq!(state.tabs[0].model.display_offset(), 0);
        assert!(state.tabs[1].model.display_offset() > 0);
    }

    #[tokio::test]
    async fn selection_start_and_update_apply_to_the_active_tab_only() {
        let harness = Harness::new();
        let svc = harness.svc();
        let handle = tokio::runtime::Handle::current();
        let tracked = |_| {};
        let ctx = harness.ctx(&svc, &handle, &tracked);
        let mut state = AgentPanelState::new();
        let mut first = make_tab(&handle, 0, AgentKind::Unknown);
        let mut second = make_tab(&handle, 1, AgentKind::Unknown);
        let _ = first.model.feed(b"first");
        let _ = second.model.feed(b"second");
        state.tabs = vec![first, second];
        state.active = 1;
        update(
            &mut state,
            Message::SelStart {
                col: 0,
                row: 0,
                right: false,
            },
            &ctx,
            |_| {},
        );
        update(
            &mut state,
            Message::SelUpdate {
                col: 2,
                row: 0,
                right: true,
            },
            &ctx,
            |_| {},
        );
        assert_eq!(state.tabs[0].model.selection_text(), None);
        assert_eq!(state.active_selection_text().as_deref(), Some("sec"));
    }

    #[tokio::test]
    async fn scroll_and_selection_without_an_active_tab_are_ignored() {
        let harness = Harness::new();
        let svc = harness.svc();
        let handle = tokio::runtime::Handle::current();
        let tracked = |_| {};
        let ctx = harness.ctx(&svc, &handle, &tracked);
        let mut state = AgentPanelState::new();
        update(&mut state, Message::Scroll(1), &ctx, |_| {});
        update(
            &mut state,
            Message::SelStart {
                col: 0,
                row: 0,
                right: false,
            },
            &ctx,
            |_| {},
        );
        update(
            &mut state,
            Message::SelUpdate {
                col: 1,
                row: 0,
                right: false,
            },
            &ctx,
            |_| {},
        );
        assert!(state.tabs.is_empty());
    }

    #[tokio::test]
    async fn resize_message_resizes_every_live_tab_and_skips_exited_ones() {
        let harness = Harness::new();
        let svc = harness.svc();
        let handle = tokio::runtime::Handle::current();
        let tracked = |_| {};
        let ctx = harness.ctx(&svc, &handle, &tracked);
        let mut state = AgentPanelState::new();
        let live = make_tab(&handle, 0, AgentKind::Unknown);
        let mut exited = make_tab(&handle, 1, AgentKind::Unknown);
        exited.alive = false;
        state.tabs = vec![live, exited];
        update(
            &mut state,
            Message::Resize { cols: 33, rows: 9 },
            &ctx,
            |_| {},
        );
        assert_eq!(state.tabs[0].model.grid_dims(), (33, 9));
        assert_eq!(state.tabs[1].model.grid_dims(), (33, 9));
        tokio::task::yield_now().await;
        assert_eq!(harness.svc.calls(), vec!["resize(s0,33,9)"]);
    }

    struct Harness {
        svc: Arc<FakeSvc>,
        hooks: Arc<FakeHooks>,
        hooks_dyn: Arc<dyn HookInstaller>,
        project: ProjectInfo,
    }

    impl Harness {
        fn new() -> Self {
            let hooks = Arc::new(FakeHooks::default());
            Self {
                svc: Arc::new(FakeSvc::default()),
                hooks: hooks.clone(),
                hooks_dyn: hooks,
                project: project(),
            }
        }

        fn svc(&self) -> Arc<dyn SessionService> {
            self.svc.clone()
        }

        fn ctx<'a>(
            &'a self,
            svc: &'a Arc<dyn SessionService>,
            handle: &'a tokio::runtime::Handle,
            tracked: &'a dyn Fn(tokio::task::JoinHandle<()>),
        ) -> UpdateCtx<'a> {
            UpdateCtx {
                project: &self.project,
                svc,
                handle,
                cols: 40,
                rows: 8,
                term_tab_bar_avail_px: 400.0,
                caps: AgentCapabilities::all(),
                hooks: &self.hooks_dyn,
                track_exit_critical: tracked,
            }
        }
    }

    // ---- 会话字节流守恒 ----

    const SESSION_STREAM_GOLDEN: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../dozer-app/tests/fixtures/session_stream_golden.txt"
    );

    fn golden_char_token(ch: char) -> String {
        match ch {
            ' ' => "\\u{20}".to_string(),
            '!'..='~' if !matches!(ch, '/' | '|' | ',' | '\\') => ch.to_string(),
            _ => format!("\\u{{{:X}}}", ch as u32),
        }
    }

    fn golden_cell_token(cell: &dozer_panel_term::model::Cell) -> String {
        let mut flags = String::new();
        if cell.bold {
            flags.push('b');
        }
        if cell.wide {
            flags.push('w');
        }
        if cell.spacer {
            flags.push('s');
        }
        if cell.selected {
            flags.push('l');
        }
        if cell.inverse {
            flags.push('i');
        }
        if flags.is_empty() {
            flags.push('-');
        }
        let fg = format!("{:02x}{:02x}{:02x}", cell.fg.0, cell.fg.1, cell.fg.2);
        let bg = cell
            .bg
            .map(|(r, g, b)| format!("{r:02x}{g:02x}{b:02x}"))
            .unwrap_or_else(|| "-".to_string());
        format!("{fg}/{bg}/{flags}/{}", golden_char_token(cell.ch))
    }

    fn render_tab(tab: &SessionTab) -> String {
        use std::fmt::Write as _;
        let mut text = String::new();
        let (cols, rows) = tab.model.grid_dims();
        let (cursor_col, cursor_row) = tab.model.cursor();
        let _ = writeln!(
            text,
            "offset={} dims={cols}x{rows} cursor=({cursor_col},{cursor_row}) cwd={:?} last_exit={:?} alive={}",
            tab.model.display_offset(),
            tab.cwd,
            tab.last_exit,
            tab.alive
        );
        for (row, cells) in tab.model.visible_lines().iter().enumerate() {
            let _ = write!(text, "R{row:02}");
            for cell in cells {
                text.push('|');
                text.push_str(&golden_cell_token(cell));
            }
            text.push('\n');
        }
        text
    }

    #[tokio::test]
    async fn session_stream_through_the_service_matches_the_golden() {
        let h = Harness::new();
        let svc = h.svc();
        let handle = tokio::runtime::Handle::current();
        let tracked = |_t| {};
        let ctx = h.ctx(&svc, &handle, &tracked);
        let mut state = AgentPanelState::new();

        let snapshot = b"\x1b[31mred\x1b[0m \x1b[1mbold\x1b[0m\r\n$ ls\x1b[2;5H!\x1b]7;file://localhost/tmp/demo\x07\x1b]133;C\x07".to_vec();
        let info = info("sess-golden", AgentKind::Claude, AgentState::Running);
        state.pending.insert(1, handle.spawn(async {}));
        update(
            &mut state,
            Message::Attached {
                tab_id: 1,
                info,
                snapshot,
                picked_agent: Some(AgentKind::Claude),
            },
            &ctx,
            |_m| {},
        );

        let mut payload = Vec::new();
        payload.extend_from_slice(b"\x1b[8;1Hl00\r\nl01\x1b[1;5HX\x1b[2K\x1b[32mgreen\x1b[0m\r\n");
        payload.extend_from_slice(b"\x1b]7;file://localhost/tmp/demo/sub\x07");
        payload.extend_from_slice(b"\x1b]133;D;3\x07");
        payload.extend_from_slice("你好 😀 e\u{0301}\r\n".as_bytes());
        payload.extend_from_slice(b"tail-padding-xyz");
        payload.truncate(115);
        while payload.len() < 115 {
            payload.push(b'.');
        }
        let sizes = [
            1usize, 7, 3, 11, 2, 5, 13, 4, 9, 2, 6, 1, 8, 3, 12, 5, 2, 7, 4, 10,
        ];
        let mut pos = 0;
        for size in sizes {
            update(
                &mut state,
                Message::Output {
                    tab_id: 1,
                    bytes: payload[pos..pos + size].to_vec(),
                },
                &ctx,
                |_m| {},
            );
            pos += size;
        }
        update(&mut state, Message::Exited { tab_id: 1 }, &ctx, |_m| {});

        let tab = state.tabs.first().expect("tab");
        let mut actual = String::from("== bottom ==\n");
        actual.push_str(&render_tab(tab));
        // scroll to history top: render_tab reads display_offset, so mutate.
        state.tabs[0].model.scroll_display(1000);
        actual.push_str("== history top ==\n");
        actual.push_str(&render_tab(&state.tabs[0]));

        let expected = std::fs::read_to_string(SESSION_STREAM_GOLDEN).expect("golden");
        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn attach_events_are_forwarded_in_order_and_lagged_or_disconnected_do_not_panic() {
        let (tx, rx) = mpsc::unbounded_channel();
        tx.send(SessionEvent::Output(b"a".to_vec())).unwrap();
        tx.send(SessionEvent::Lagged).unwrap();
        tx.send(SessionEvent::Agent {
            agent: AgentKind::Claude,
            state: AgentState::Running,
            transcript_path: Some("/t".into()),
        })
        .unwrap();
        tx.send(SessionEvent::Output(b"b".to_vec())).unwrap();
        tx.send(SessionEvent::Disconnected).unwrap();
        tx.send(SessionEvent::Output(b"after".to_vec())).unwrap();
        drop(tx);

        let got = Arc::new(Mutex::new(Vec::new()));
        let sink = got.clone();
        forward_events(3, rx, move |m| sink.lock().unwrap().push(m)).await;
        let got = got.lock().unwrap();
        assert_eq!(
            got.as_slice(),
            &[
                Message::Output {
                    tab_id: 3,
                    bytes: b"a".to_vec()
                },
                Message::AgentState {
                    tab_id: 3,
                    agent: AgentKind::Claude,
                    state: AgentState::Running,
                    transcript_path: Some("/t".into())
                },
                Message::Output {
                    tab_id: 3,
                    bytes: b"b".to_vec()
                },
                Message::Exited { tab_id: 3 },
            ]
        );
    }

    #[tokio::test]
    async fn closing_a_tab_before_the_active_one_keeps_the_same_tab_active() {
        let h = Harness::new();
        let svc = h.svc();
        let handle = tokio::runtime::Handle::current();
        let ctx = h.ctx(&svc, &handle, &|_| {});
        let mut state = AgentPanelState::new();
        for id in 0..3 {
            state.tabs.push(make_tab(&handle, id, AgentKind::Claude));
        }
        state.active = 2;
        update(&mut state, Message::CloseTab(0), &ctx, |_| {});
        assert_eq!(state.tabs.len(), 2);
        assert_eq!(state.active, 1);
        assert_eq!(state.tabs[state.active].tab_id, 2);
        tokio::task::yield_now().await;
    }

    #[tokio::test]
    async fn closing_the_active_last_tab_selects_the_previous_one() {
        let h = Harness::new();
        let svc = h.svc();
        let handle = tokio::runtime::Handle::current();
        let ctx = h.ctx(&svc, &handle, &|_| {});
        let mut state = AgentPanelState::new();
        for id in 0..2 {
            state.tabs.push(make_tab(&handle, id, AgentKind::Claude));
        }
        state.active = 1;
        update(&mut state, Message::CloseTab(1), &ctx, |_| {});
        assert_eq!(state.tabs.len(), 1);
        assert_eq!(state.active, 0);
    }

    #[tokio::test]
    async fn closing_resets_term_tab_first() {
        let h = Harness::new();
        let svc = h.svc();
        let handle = tokio::runtime::Handle::current();
        let ctx = h.ctx(&svc, &handle, &|_| {});
        let mut state = AgentPanelState::new();
        for id in 0..3 {
            state.tabs.push(make_tab(&handle, id, AgentKind::Claude));
        }
        state.term_tab_first = 2;
        update(&mut state, Message::CloseTab(0), &ctx, |_| {});
        assert_eq!(state.term_tab_first, 0);
    }

    #[tokio::test]
    async fn close_with_summary_is_used_only_for_supported_live_daemon_tabs_when_the_capability_is_on_else_kill()
     {
        for caps_on in [true, false] {
            for alive in [true, false] {
                for custom in [false, true] {
                    let h = Harness::new();
                    let svc = h.svc();
                    let handle = tokio::runtime::Handle::current();
                    let mut ctx = h.ctx(&svc, &handle, &|_| {});
                    ctx.caps.summary_on_close = caps_on;
                    let mut state = AgentPanelState::new();
                    let mut tab = make_tab(&handle, 1, AgentKind::Claude);
                    tab.alive = alive;
                    if custom {
                        let (tx, _rx) = mpsc::unbounded_channel();
                        tab.backend = TabBackend::Custom(tx);
                    }
                    state.tabs.push(tab);
                    update(&mut state, Message::CloseTab(0), &ctx, |_| {});
                    tokio::task::yield_now().await;
                    let calls = h.svc.calls();
                    let want_close = caps_on && alive && !custom;
                    let want_kill = !want_close && alive && !custom;
                    let closes = calls
                        .iter()
                        .filter(|c| c.starts_with("close_with_summary"))
                        .count();
                    let kills = calls.iter().filter(|c| c.starts_with("kill")).count();
                    assert_eq!(
                        closes,
                        usize::from(want_close),
                        "caps={caps_on} alive={alive} custom={custom} {calls:?}"
                    );
                    assert_eq!(
                        kills,
                        usize::from(want_kill),
                        "caps={caps_on} alive={alive} custom={custom} {calls:?}"
                    );
                }
            }
        }
    }

    #[tokio::test]
    async fn close_confirm_resolves_by_session_id_not_by_index() {
        let h = Harness::new();
        let svc = h.svc();
        let handle = tokio::runtime::Handle::current();
        let ctx = h.ctx(&svc, &handle, &|_| {});
        let mut state = AgentPanelState::new();
        for id in 0..3 {
            let mut tab = make_tab(&handle, id, AgentKind::Claude);
            tab.agent_state = AgentState::Running;
            state.tabs.push(tab);
        }
        update(&mut state, Message::CloseTab(1), &ctx, |_| {});
        assert_eq!(state.pending_close_tab.as_deref(), Some("s1"));
        // 弹窗期间列表顺序变化(把 s1 移到末尾)。
        let tab = state.tabs.remove(1);
        state.tabs.push(tab);
        update(&mut state, Message::CloseConfirm, &ctx, |_| {});
        assert_eq!(state.tabs.len(), 2);
        assert!(state.tabs.iter().all(|t| t.info.id != "s1"));
    }

    #[tokio::test]
    async fn running_or_awaiting_input_tab_asks_for_confirmation_others_close_directly() {
        for (state_kind, confirm) in [
            (AgentState::Running, true),
            (AgentState::AwaitingInput, true),
            (AgentState::Idle, false),
            (AgentState::TurnEnded, false),
        ] {
            let h = Harness::new();
            let svc = h.svc();
            let handle = tokio::runtime::Handle::current();
            let ctx = h.ctx(&svc, &handle, &|_| {});
            let mut state = AgentPanelState::new();
            let mut tab = make_tab(&handle, 1, AgentKind::Claude);
            tab.agent_state = state_kind;
            state.tabs.push(tab);
            update(&mut state, Message::CloseTab(0), &ctx, |_| {});
            if confirm {
                assert!(state.pending_close_tab.is_some(), "{state_kind:?}");
                assert_eq!(state.tabs.len(), 1);
            } else {
                assert!(state.pending_close_tab.is_none(), "{state_kind:?}");
                assert!(state.tabs.is_empty(), "{state_kind:?}");
            }
        }
    }

    #[tokio::test]
    async fn late_messages_for_a_closed_tab_are_ignored() {
        let h = Harness::new();
        let svc = h.svc();
        let handle = tokio::runtime::Handle::current();
        let ctx = h.ctx(&svc, &handle, &|_| {});
        let mut state = AgentPanelState::new();
        state.tabs.push(make_tab(&handle, 1, AgentKind::Claude));
        update(&mut state, Message::CloseTab(0), &ctx, |_| {});

        update(
            &mut state,
            Message::Output {
                tab_id: 99,
                bytes: b"x".to_vec(),
            },
            &ctx,
            |_| {},
        );
        update(
            &mut state,
            Message::AgentState {
                tab_id: 99,
                agent: AgentKind::Claude,
                state: AgentState::Running,
                transcript_path: None,
            },
            &ctx,
            |_| {},
        );
        update(&mut state, Message::Exited { tab_id: 99 }, &ctx, |_| {});
        update(
            &mut state,
            Message::Attached {
                tab_id: 99,
                info: info("late", AgentKind::Claude, AgentState::Idle),
                snapshot: Vec::new(),
                picked_agent: None,
            },
            &ctx,
            |_| {},
        );
        assert!(state.tabs.is_empty());
        assert!(state.host_events.is_empty());
    }

    #[tokio::test]
    async fn create_failure_pushes_a_keyed_notice_and_adds_no_tab() {
        let mut h = Harness::new();
        h.svc = Arc::new(FakeSvc::default().with_create_err(err()));
        let svc = h.svc();
        let handle = tokio::runtime::Handle::current();
        let ctx = h.ctx(&svc, &handle, &|_| {});
        let mut state = AgentPanelState::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        spawn_new_tab(
            &mut state,
            &ctx,
            move |m| {
                let _ = tx.send(m);
            },
            PickerLaunch::Agent(Some(AgentKind::Claude)),
            None,
        );
        let msg = rx.recv().await.expect("message");
        update(&mut state, msg, &ctx, |_| {});
        let notices = state.take_outbox();
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].key.as_deref(), Some("new-session-failed"));
        assert!(state.tabs.is_empty());
    }

    #[tokio::test]
    async fn attach_failure_pushes_a_keyed_notice_and_clears_pending() {
        let mut h = Harness::new();
        h.svc = Arc::new(FakeSvc::default().with_attach_err(err()));
        let svc = h.svc();
        let handle = tokio::runtime::Handle::current();
        let ctx = h.ctx(&svc, &handle, &|_| {});
        let mut state = AgentPanelState::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        spawn_new_tab(
            &mut state,
            &ctx,
            move |m| {
                let _ = tx.send(m);
            },
            PickerLaunch::Agent(Some(AgentKind::Claude)),
            None,
        );
        let msg = rx.recv().await.expect("message");
        update(&mut state, msg, &ctx, |_| {});
        let notices = state.take_outbox();
        assert_eq!(notices[0].key.as_deref(), Some("attach-session-failed"));
        assert!(state.tabs.is_empty());
    }

    #[tokio::test]
    async fn attach_failure_kills_the_session_that_create_already_made() {
        // create 成功、attach 失败:守护进程里会话已经存在,产品拿不到 `Attached`
        // 也就拿不到会话 id,没人能 kill。库自己在 attach 失败时收尾,不留孤儿。
        let mut h = Harness::new();
        h.svc = Arc::new(FakeSvc::default().with_attach_err(err()));
        let svc = h.svc();
        let handle = tokio::runtime::Handle::current();
        let ctx = h.ctx(&svc, &handle, &|_| {});
        let mut state = AgentPanelState::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        spawn_new_tab(
            &mut state,
            &ctx,
            move |m| {
                let _ = tx.send(m);
            },
            PickerLaunch::Agent(Some(AgentKind::Claude)),
            None,
        );
        let msg = rx.recv().await.expect("failed message");
        assert!(matches!(msg, Message::Failed { .. }));
        let calls = h.svc.calls();
        assert!(
            calls.iter().any(|c| c.starts_with("kill(")),
            "attach 失败后必须 kill 已创建的会话,实际调用: {calls:?}"
        );
    }

    #[tokio::test]
    async fn a_failing_kill_after_attach_failure_still_reports_the_attach_error() {
        let mut h = Harness::new();
        h.svc = Arc::new(
            FakeSvc::default()
                .with_attach_err(err())
                .with_kill_err(err()),
        );
        let svc = h.svc();
        let handle = tokio::runtime::Handle::current();
        let ctx = h.ctx(&svc, &handle, &|_| {});
        let mut state = AgentPanelState::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        spawn_new_tab(
            &mut state,
            &ctx,
            move |m| {
                let _ = tx.send(m);
            },
            PickerLaunch::Agent(Some(AgentKind::Claude)),
            None,
        );
        let msg = rx.recv().await.expect("failed message");
        update(&mut state, msg, &ctx, |_| {});
        let notices = state.take_outbox();
        assert_eq!(notices[0].key.as_deref(), Some("attach-session-failed"));
        assert!(state.tabs.is_empty());
    }

    #[tokio::test]
    async fn initial_launch_write_failure_keeps_the_tab_and_notifies() {
        let mut h = Harness::new();
        h.svc = Arc::new(FakeSvc::default().with_write_err(err()));
        let svc = h.svc();
        let handle = tokio::runtime::Handle::current();
        let ctx = h.ctx(&svc, &handle, &|_| {});
        let mut state = AgentPanelState::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        spawn_new_tab(
            &mut state,
            &ctx,
            move |m| {
                let _ = tx.send(m);
            },
            PickerLaunch::Agent(Some(AgentKind::Claude)),
            None,
        );
        // Attached 先到,随后 Failed(initial write)。
        let attached = rx.recv().await.expect("attached");
        update(&mut state, attached, &ctx, |_| {});
        let failed = rx.recv().await.expect("failed");
        assert_eq!(
            failed,
            Message::Failed {
                key: "term-initial-write",
                error: "boom".into()
            }
        );
        update(&mut state, failed, &ctx, |_| {});
        assert_eq!(state.tabs.len(), 1);
        assert_eq!(
            state.take_outbox()[0].key.as_deref(),
            Some("term-initial-write")
        );
    }

    #[tokio::test]
    async fn follow_up_write_failure_notifies() {
        let mut h = Harness::new();
        h.svc = Arc::new(FakeSvc::default().with_write_err(err()));
        let svc = h.svc();
        let handle = tokio::runtime::Handle::current();
        let ctx = h.ctx(&svc, &handle, &|_| {});
        let mut state = AgentPanelState::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        // 纯 Shell 没有初始命令,只有 follow_up 写。
        spawn_new_tab(
            &mut state,
            &ctx,
            move |m| {
                let _ = tx.send(m);
            },
            PickerLaunch::Agent(None),
            Some("todo text".into()),
        );
        let attached = rx.recv().await.expect("attached");
        update(&mut state, attached, &ctx, |_| {});
        let failed = rx.recv().await.expect("failed");
        assert_eq!(
            failed,
            Message::Failed {
                key: "term-dispatch-write",
                error: "boom".into()
            }
        );
    }

    #[tokio::test]
    async fn hooks_are_installed_before_the_session_is_created_only_for_a_known_agent() {
        for (launch, want_hook) in [
            (PickerLaunch::Agent(Some(AgentKind::Claude)), true),
            (PickerLaunch::Agent(None), false),
            (PickerLaunch::Git, false),
        ] {
            let h = Harness::new();
            let svc = h.svc();
            let handle = tokio::runtime::Handle::current();
            let ctx = h.ctx(&svc, &handle, &|_| {});
            let mut state = AgentPanelState::new();
            spawn_new_tab(&mut state, &ctx, |_| {}, launch, None);
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            let hooks = h.hooks.calls.lock().unwrap().clone();
            let calls = h.svc.calls();
            if want_hook {
                assert!(hooks.iter().any(|c| c.starts_with("hook(")), "{hooks:?}");
                assert!(hooks.iter().any(|c| c.starts_with("mcp(")), "{hooks:?}");
                assert!(
                    hooks.len() >= 2,
                    "hook/MCP 必须先于 create: {hooks:?} {calls:?}"
                );
            } else {
                assert!(hooks.is_empty(), "{launch:?} 不该装 hook: {hooks:?}");
            }
        }
    }

    #[tokio::test]
    async fn resize_skips_dead_tabs_and_routes_by_backend() {
        let h = Harness::new();
        let svc = h.svc();
        let handle = tokio::runtime::Handle::current();

        let mut dead = make_tab(&handle, 1, AgentKind::Claude);
        dead.alive = false;
        crate::tabs::resize_tab(&mut dead, &svc, &handle, 10, 4);
        assert!(h.svc.calls().is_empty());

        let mut live = make_tab(&handle, 2, AgentKind::Claude);
        crate::tabs::resize_tab(&mut live, &svc, &handle, 1, 1);
        tokio::task::yield_now().await;
        assert_eq!(h.svc.calls(), vec!["resize(s2,1,1)"]);

        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut custom = make_tab(&handle, 3, AgentKind::Claude);
        custom.backend = TabBackend::Custom(tx);
        crate::tabs::resize_tab(&mut custom, &svc, &handle, 0, 0);
        match rx.recv().await {
            Some(TabOut::Resize { cols: 0, rows: 0 }) => {}
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn new_tab_is_scrolled_into_view_when_the_tab_bar_overflows() {
        let h = Harness::new();
        let svc = h.svc();
        let handle = tokio::runtime::Handle::current();
        let mut ctx = h.ctx(&svc, &handle, &|_| {});
        ctx.term_tab_bar_avail_px = 120.0;
        let mut state = AgentPanelState::new();
        for id in 0..8 {
            state.tabs.push(make_tab(&handle, id, AgentKind::Claude));
        }
        state.pending.insert(9, handle.spawn(async {}));
        update(
            &mut state,
            Message::Attached {
                tab_id: 9,
                info: info("new", AgentKind::Claude, AgentState::Idle),
                snapshot: Vec::new(),
                picked_agent: None,
            },
            &ctx,
            |_| {},
        );
        assert_eq!(state.active, state.tabs.len() - 1);
        assert!(state.term_tab_first > 0, "新 tab 应滚入可见窗口");
    }

    #[tokio::test]
    async fn agent_state_message_requests_a_card_refresh_for_an_existing_tab_only() {
        let h = Harness::new();
        let svc = h.svc();
        let handle = tokio::runtime::Handle::current();
        let ctx = h.ctx(&svc, &handle, &|_| {});
        let mut state = AgentPanelState::new();
        state.tabs.push(make_tab(&handle, 5, AgentKind::Unknown));
        let (tx, mut rx) = mpsc::unbounded_channel();
        update(
            &mut state,
            Message::AgentState {
                tab_id: 5,
                agent: AgentKind::Claude,
                state: AgentState::Running,
                transcript_path: None,
            },
            &ctx,
            move |m| {
                let _ = tx.send(m);
            },
        );
        let msg = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .expect("卡片刷新应在合理时间内回灌")
            .expect("消息");
        assert!(
            matches!(msg, Message::CardRefreshed { tab_id: 5, .. }),
            "{msg:?}"
        );

        // 不存在的 tab 不触发刷新任务。
        let (tx2, mut rx2) = mpsc::unbounded_channel();
        let keep_tx2 = tx2.clone();
        update(
            &mut state,
            Message::AgentState {
                tab_id: 999,
                agent: AgentKind::Claude,
                state: AgentState::Running,
                transcript_path: None,
            },
            &ctx,
            move |m| {
                let _ = tx2.send(m);
            },
        );
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), rx2.recv())
                .await
                .is_err(),
            "不存在的 tab 不应产生 CardRefreshed"
        );
        drop(keep_tx2);
    }

    #[tokio::test]
    async fn picker_toggle_pushes_a_show_picker_request_instead_of_opening_a_menu() {
        let h = Harness::new();
        let svc = h.svc();
        let handle = tokio::runtime::Handle::current();
        let ctx = h.ctx(&svc, &handle, &|_| {});
        let mut state = AgentPanelState::new();
        update(&mut state, Message::PickerToggle, &ctx, |_| {});
        assert!(state.agent_picker_open);
        let requests = state.take_requests();
        assert_eq!(requests, vec![AgentRequest::ShowPicker { at: None }]);
    }

    #[tokio::test]
    async fn state_change_and_turn_end_are_reported_to_the_host_not_handled_here() {
        let h = Harness::new();
        let svc = h.svc();
        let handle = tokio::runtime::Handle::current();
        let ctx = h.ctx(&svc, &handle, &|_| {});
        let mut state = AgentPanelState::new();
        state.tabs.push(make_tab(&handle, 5, AgentKind::Unknown));
        update(
            &mut state,
            Message::AgentState {
                tab_id: 5,
                agent: AgentKind::Claude,
                state: AgentState::TurnEnded,
                transcript_path: Some("/t.jsonl".into()),
            },
            &ctx,
            |_| {},
        );
        assert_eq!(state.tabs[0].agent, AgentKind::Claude);
        assert_eq!(state.tabs[0].agent_state, AgentState::TurnEnded);
        let events = state.take_host_events();
        assert!(events.contains(&AgentHostEvent::StateChanged {
            tab_id: 5,
            agent: AgentKind::Claude,
            state: AgentState::TurnEnded,
            transcript_path: Some("/t.jsonl".into())
        }));
        assert!(events.contains(&AgentHostEvent::TurnEnded { tab_id: 5 }));
    }

    #[tokio::test]
    async fn a_new_codex_tab_starts_with_the_configured_model_and_other_agents_do_not() {
        let h = Harness::new();
        let svc = h.svc();
        let handle = tokio::runtime::Handle::current();
        let tracked = |_t| {};
        let ctx = h.ctx(&svc, &handle, &tracked);
        let mut state = AgentPanelState::new();
        for (tab_id, agent) in [(1, AgentKind::Codex), (2, AgentKind::Claude)] {
            state.pending.insert(tab_id, handle.spawn(async {}));
            update(
                &mut state,
                Message::Attached {
                    tab_id,
                    info: info(&format!("s{tab_id}"), agent, AgentState::Idle),
                    snapshot: Vec::new(),
                    picked_agent: Some(agent),
                },
                &ctx,
                |_m| {},
            );
        }
        assert_eq!(
            state.tab_by_id(1).unwrap().llm_model,
            crate::codex_config::default_model()
        );
        assert_eq!(state.tab_by_id(2).unwrap().llm_model, None);
    }
}
