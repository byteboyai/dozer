use crate::{adapter::ConversationsRequest, snapshot::ReviewView};
use dozer_model::{AgentKind, SessionRow};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};
#[derive(Default)]
pub struct WorkspaceState {
    /// Current detail; the snapshot is published only for accepted current-source results.
    pub review: Option<ReviewView>,
    /// Workspace-wide wrapping watermark, independent of the selected source.
    pub review_nonce: u64,
    pub(crate) review_snapshot: Arc<Mutex<Option<String>>>,
    pub(crate) host_requests: Vec<ConversationsRequest>,
    pub requests: dozer_panel_host::RequestTrackers,
    pub(crate) sessions: Option<Vec<SessionRow>>,
    pub(crate) pages: usize,
    pub(crate) search: String,
    pub(crate) search_draft: String,
    pub(crate) search_focused: bool,
    pub(crate) agent_filter: Option<AgentKind>,
    pub(crate) agent_picker_open: bool,
    pub(crate) summary_generating: HashSet<String>,
    pub(crate) summary_errors: HashMap<String, dozer_model::LocalizedText>,
    /// 待发提示;`App::drain_outboxes` 排空成 Toast。
    pub(crate) outbox: dozer_panel_host::NoticeOutbox,
}

impl WorkspaceState {
    pub fn take_outbox(&mut self) -> Vec<dozer_panel_host::Notice> {
        self.outbox.take()
    }

    pub fn summary_generating(&self, conversation_id: &str) -> bool {
        self.summary_generating.contains(conversation_id)
    }

    pub fn summary_error(&self, conversation_id: &str) -> Option<&dozer_model::LocalizedText> {
        self.summary_errors.get(conversation_id)
    }

    pub fn start_summary_generation(&mut self, conversation_id: String) -> bool {
        self.summary_errors.remove(&conversation_id);
        self.summary_generating.insert(conversation_id)
    }

    pub fn finish_summary_generation(
        &mut self,
        conversation_id: String,
        result: Result<(), dozer_model::LocalizedText>,
    ) {
        self.summary_generating.remove(&conversation_id);
        if let Err(error) = result {
            self.summary_errors.insert(conversation_id, error);
        } else {
            self.summary_errors.remove(&conversation_id);
        }
    }

    /// Latest list rows, including the summary fields used when opening a detail.
    pub fn sessions(&self) -> Option<&[SessionRow]> {
        self.sessions.as_deref()
    }

    /// 会话列表搜索框是否持有 iced 真实焦点(main.rs 键盘路由用)。
    pub fn search_focused(&self) -> bool {
        self.search_focused
    }

    /// Whether the host's iced agent-filter fallback is open. Native menus never set this flag.
    pub fn agent_picker_open(&self) -> bool {
        self.agent_picker_open
    }

    /// 每帧渲染循环读走 `CaptureConversationSearchFocus` 查到的真实焦点态后
    /// 写回这里。
    pub fn set_search_focused(&mut self, focused: bool) {
        self.search_focused = focused;
    }
}

impl WorkspaceState {
    pub fn take_requests(&mut self) -> Vec<ConversationsRequest> {
        std::mem::take(&mut self.host_requests)
    }
    pub fn review_snapshot(&self) -> Arc<Mutex<Option<String>>> {
        Arc::clone(&self.review_snapshot)
    }
    pub fn current_conversation_id(&self) -> Option<&str> {
        self.review.as_ref().map(|rv| {
            let crate::snapshot::ReviewSource::Conversation(cid) = &rv.source;
            cid.as_str()
        })
    }
    pub fn agent_filter(&self) -> Option<AgentKind> {
        self.agent_filter
    }
    /// Hosts with an iced fallback may show their overlay after consuming the request.
    pub fn set_agent_picker_open(&mut self, open: bool) {
        self.agent_picker_open = open;
    }
}
