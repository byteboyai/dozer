//! Group-chat constants shared by the panel state machine and native views.
use dozer_model::AgentKind;
use std::time::Duration;

pub const GROUP_CHAT_OWNER: &str = crate::GROUP_CHAT_OWNER;
pub const CREATABLE_AGENTS: &[AgentKind] = &[
    AgentKind::Claude,
    AgentKind::Codex,
    AgentKind::Codebuddy,
    AgentKind::Opencode,
    AgentKind::Qodercn,
];
pub const POLL_INTERVAL: Duration = Duration::from_millis(500);
pub const POLL_BACKOFF: Duration = Duration::from_secs(5);
pub const POLL_LIMIT: u32 = 200;
