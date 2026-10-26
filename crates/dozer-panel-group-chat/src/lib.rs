//! Shared group-chat panel: workspace state, pure update/effects, protocol,
//! embedded web resources, list/content views and the new-group dialog card.
//! Desktop hosts inject presentation and execute the returned effects.
pub const GROUP_CHAT_OWNER: &str = "dozer.group_chat";
pub mod adapter;
pub mod assets;
pub mod consts;
pub mod dialog;
pub mod effects;
pub mod host;
pub mod i18n;
pub mod protocol;
pub mod state;
pub mod update;
pub mod view;

pub use protocol::{
    Command, GroupChatWebviewEvent, WebviewPushState, current_view_payload, encode_group_chat_push,
    parse_group_chat_event, route_event,
};
pub use state::WorkspaceState;
pub use update::{
    Effect, Message, on_activate, poll_if_due, run_effect, select_group, spawn_command,
    spawn_fetch_messages, spawn_load_groups, update,
};

/// Neutral panel logging, with the existing group-chat target.
pub mod log {
    pub fn warn(message: &str, detail: &str) {
        tracing::warn!(target:"dozer::panel::group_chat","{message}: {detail}");
    }
}
