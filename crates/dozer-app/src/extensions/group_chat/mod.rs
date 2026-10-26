//! Group-chat panel host adapter. The state machine, protocol, resources and
//! views live in `dozer-panel-group-chat`; this module keeps only Dozer-facing
//! re-exports plus the original effect tests (which exercise the shared code
//! through Dozer's fake services).
pub use dozer_panel_group_chat::consts::{
    CREATABLE_AGENTS, POLL_BACKOFF, POLL_INTERVAL, POLL_LIMIT,
};
pub use dozer_panel_group_chat::i18n::{keyed, keyed_args, t, t_args};
pub use dozer_panel_group_chat::view::{ShellMessage, content_pane, list_pane};
pub use dozer_panel_group_chat::{
    Command, Effect, GroupChatWebviewEvent, Message, WebviewPushState, WorkspaceState,
    current_view_payload, encode_group_chat_push, host, i18n, on_activate, parse_group_chat_event,
    poll_if_due, route_event, run_effect, select_group, spawn_command, spawn_fetch_messages,
    spawn_load_groups, update,
};

dozer_core::scope!(pub(crate) LOG, panel, "group_chat");

/// Protocol re-exports for tests and existing host call sites.
pub mod protocol {
    pub use dozer_panel_group_chat::protocol::*;
}

#[cfg(test)]
mod tests;
