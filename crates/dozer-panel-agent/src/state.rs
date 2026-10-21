//! Agent 面板状态:会话/页签集合 + 交互态 + 发件箱(S6c-1)。

use crate::adapter::{AgentHostEvent, AgentRequest};
use crate::restore::RestoredSession;
use crate::tabs::{SessionTab, TabBackend};
use crate::update::{Message, forward_events};
use dozer_model::AgentKind;
use dozer_panel_host::{Notice, NoticeOutbox};
use dozer_panel_term::model::TerminalModel;
use dozer_panel_term::osc::OscScanner;
use std::collections::HashMap;

/// 从 `Workspace` 搬出的 Agent 面板自有状态。
pub struct AgentPanelState {
    pub tabs: Vec<SessionTab>,
    /// 当前显示的 tab 在 `tabs` 中的位置(不是 `tab_id`)。
    pub active: usize,
    pub next_tab_id: usize,
    /// 发起新建会话(create+attach)期间的转发任务句柄暂存区,
    /// `Attached` 到达时取出、装进新建的 `SessionTab`。
    pub pending: HashMap<usize, tokio::task::JoinHandle<()>>,
    /// 终端 tab 栏当前最左可见 tab 序号。
    pub term_tab_first: usize,
    /// 终端 tab 栏溢出下拉的悬浮锚点。
    pub term_tab_overflow_anchor: Option<(f32, f32)>,
    pub agent_picker_open: bool,
    /// 等用户确认关闭的会话 id(按 id 而非下标记住)。
    pub pending_close_tab: Option<String>,
    pub picked_agent: Option<dozer_model::AgentKind>,
    /// 一次性提示(错误等)发件箱,由产品排空成 Toast。
    pub outbox: NoticeOutbox,
    /// 需要产品执行的请求(原生菜单等)。
    pub requests: Vec<AgentRequest>,
    /// 需要产品跟随处理的跨面板事件。
    pub host_events: Vec<AgentHostEvent>,
}

impl AgentPanelState {
    pub fn new() -> Self {
        Self {
            tabs: Vec::new(),
            active: 0,
            next_tab_id: 0,
            pending: HashMap::new(),
            term_tab_first: 0,
            term_tab_overflow_anchor: None,
            agent_picker_open: false,
            pending_close_tab: None,
            picked_agent: None,
            outbox: NoticeOutbox::default(),
            requests: Vec::new(),
            host_events: Vec::new(),
        }
    }

    pub fn take_outbox(&mut self) -> Vec<Notice> {
        self.outbox.take()
    }

    pub fn take_requests(&mut self) -> Vec<AgentRequest> {
        std::mem::take(&mut self.requests)
    }

    pub fn take_host_events(&mut self) -> Vec<AgentHostEvent> {
        std::mem::take(&mut self.host_events)
    }

    pub fn tab_by_id_mut(&mut self, id: usize) -> Option<&mut SessionTab> {
        self.tabs.iter_mut().find(|t| t.tab_id == id)
    }

    pub fn tab_by_id(&self, id: usize) -> Option<&SessionTab> {
        self.tabs.iter().find(|t| t.tab_id == id)
    }

    pub fn active_app_cursor_mode(&self) -> bool {
        self.tabs
            .get(self.active)
            .is_some_and(|t| t.model.app_cursor_mode())
    }

    pub fn active_selection_text(&self) -> Option<String> {
        self.tabs
            .get(self.active)
            .and_then(|t| t.model.selection_text())
    }

    /// 在 UI 线程把已 attach 的恢复素材装配成页签，并开始转发后续事件。
    pub fn adopt_restored(
        &mut self,
        sessions: Vec<RestoredSession>,
        handle: &tokio::runtime::Handle,
        emit: impl Fn(Message) + Clone + Send + 'static,
    ) {
        for RestoredSession {
            info,
            snapshot,
            events,
        } in sessions
        {
            let tab_id = self.next_tab_id;
            self.next_tab_id += 1;
            let mut model = TerminalModel::new(80, 24);
            model.set_answer_dynamic_color(info.agent == AgentKind::Opencode);
            let _ = model.feed(&snapshot);
            let forwarder = handle.spawn(forward_events(tab_id, events, emit.clone()));
            let mut tab = SessionTab {
                agent_state: info.agent_state,
                agent: info.agent,
                transcript_path: info.transcript_path.clone(),
                info,
                model,
                alive: true,
                tab_id,
                forwarder,
                osc: OscScanner::new(),
                cwd: None,
                last_exit: None,
                llm_model: None,
                permission_mode: None,
                last_activity: None,
                workspace_override: None,
                backend: TabBackend::Daemon,
            };
            tab.ingest_osc(&snapshot);
            self.tabs.push(tab);
        }
    }
}

impl Default for AgentPanelState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dozer_model::{AgentState, SessionEvent, SessionInfo};

    fn restored(
        id: &str,
        snapshot: &[u8],
    ) -> (
        RestoredSession,
        tokio::sync::mpsc::UnboundedSender<SessionEvent>,
    ) {
        let (events_tx, events) = tokio::sync::mpsc::unbounded_channel();
        (
            RestoredSession {
                info: SessionInfo {
                    id: id.into(),
                    name: id.into(),
                    command: "sh".into(),
                    cwd: "/tmp".into(),
                    alive: true,
                    created_ms: 0,
                    agent_state: AgentState::Idle,
                    transcript_path: None,
                    project_id: Some(7),
                    agent: AgentKind::Unknown,
                },
                snapshot: snapshot.to_vec(),
                events,
            },
            events_tx,
        )
    }

    #[tokio::test]
    async fn adopt_restored_builds_one_tab_per_session_with_distinct_ids_and_the_snapshot_fed() {
        let (one, _) = restored("one", b"hello");
        let (two, _) = restored("two", b"world");
        let mut state = AgentPanelState::new();
        state.adopt_restored(vec![one, two], &tokio::runtime::Handle::current(), |_| {});
        assert_eq!(state.tabs.len(), 2);
        assert_eq!(
            (
                state.tabs[0].tab_id,
                state.tabs[1].tab_id,
                state.next_tab_id
            ),
            (0, 1, 2)
        );
        assert_eq!(state.tabs[0].model.visible_lines()[0][0].ch, 'h');
        assert_eq!(state.tabs[1].model.visible_lines()[0][0].ch, 'w');
    }

    #[tokio::test]
    async fn adopt_restored_forwards_later_events_to_the_right_tab() {
        let (session, events) = restored("one", b"");
        let messages = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let received = std::sync::Arc::clone(&messages);
        let mut state = AgentPanelState::new();
        state.adopt_restored(
            vec![session],
            &tokio::runtime::Handle::current(),
            move |message| received.lock().unwrap().push(message),
        );
        events
            .send(SessionEvent::Output(b"later".to_vec()))
            .unwrap();
        tokio::task::yield_now().await;
        assert_eq!(
            messages.lock().unwrap().as_slice(),
            &[Message::Output {
                tab_id: 0,
                bytes: b"later".to_vec()
            }]
        );
    }

    #[tokio::test]
    async fn adopt_restored_with_no_sessions_leaves_the_state_empty() {
        let mut state = AgentPanelState::new();
        state.adopt_restored(Vec::new(), &tokio::runtime::Handle::current(), |_| {});
        assert!(state.tabs.is_empty());
        assert_eq!(state.next_tab_id, 0);
    }
}
