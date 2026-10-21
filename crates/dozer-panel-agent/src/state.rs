//! Agent 面板状态:会话/页签集合 + 交互态 + 发件箱(S6c-1)。

use crate::adapter::{AgentHostEvent, AgentRequest};
use crate::tabs::SessionTab;
use dozer_panel_host::{Notice, NoticeOutbox};
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
}

impl Default for AgentPanelState {
    fn default() -> Self {
        Self::new()
    }
}
