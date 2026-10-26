//! Host-facing webview logic: event routing, push script assembly and the
//! "should the host schedule a poll" predicate. The product keeps the webview
//! id, lifecycle and `dispatch_script` wrapper.
use crate::Command;
use crate::protocol::{
    GroupChatWebviewEvent, WebviewPushState, current_view_payload, encode_group_chat_push,
    route_event,
};
use crate::state::WorkspaceState;
use crate::update::{Message, select_group, spawn_command};
use dozer_model::{LocaleStamp, Stamped};
use dozer_panel_host::PanelIo;
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostAction {
    None,
    ShowTodoPanel,
}

pub fn handle_webview_event(
    state: &mut WorkspaceState,
    push: &mut WebviewPushState,
    project_id: i64,
    event: GroupChatWebviewEvent,
    io: &PanelIo<Message>,
) -> HostAction {
    use GroupChatWebviewEvent as Ev;
    match &event {
        Ev::Ready => {
            push.set_ready(true);
            return HostAction::None;
        }
        Ev::Failed { reason } => {
            crate::log::warn("群聊内容页渲染失败", reason);
            push.set_failed(dozer_model::LocalizedText::literal(
                crate::i18n::HOST_OWNER,
                reason.clone(),
            ));
            return HostAction::None;
        }
        Ev::SetDraft { group_id, text } => {
            state.set_composer_draft(*group_id, text);
            return HostAction::None;
        }
        _ => {}
    }
    let Some(cmd) = route_event(state, event) else {
        return HostAction::None;
    };
    match cmd {
        Command::SelectGroup { group_id } => {
            select_group(state, project_id, group_id, io);
            HostAction::None
        }
        Command::OpenTodo { .. } => HostAction::ShowTodoPanel,
        other => {
            spawn_command(state.requests(), project_id, other, io);
            HostAction::None
        }
    }
}

/// Scripts to inject after the webview has been created / hidden / restored.
/// Draft restore is one-shot; the payload uses the declarative push state.
pub fn take_content_script(
    state: &WorkspaceState,
    push: &mut WebviewPushState,
    project_id: i64,
    webview_available: bool,
    now: Instant,
    stamp: LocaleStamp,
    locale: &str,
) -> Vec<String> {
    push.observe_availability(webview_available, now);
    if !webview_available || push.failed().is_some() {
        return Vec::new();
    }
    let mut scripts = Vec::new();
    let drafts_json = state.composer_drafts_restore_json();
    let desired = Stamped::new(stamp, current_view_payload(state, project_id, locale));
    if push.take_needs_draft_restore() {
        scripts.push(format!(
            "window.__gcDrafts={drafts_json};window.__gcApplyDrafts&&window.__gcApplyDrafts()"
        ));
    }
    if let Some(payload) = push.pending_push(&desired) {
        let revision = push.mark_sent(payload.clone());
        let (locale, payload) = payload.into_parts();
        let envelope = encode_group_chat_push(revision, Some(locale), payload);
        scripts.push(dispatch_script(&envelope));
    }
    scripts
}

pub fn poll_wanted(state: &WorkspaceState, panel_visible: bool) -> bool {
    panel_visible && (state.load_pending() || state.has_active_turn())
}

fn dispatch_script(json: &str) -> String {
    format!(
        "window.__dozer?.dispatch({});",
        serde_json::to_string(json).unwrap_or_else(|_| "\"null\"".into())
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::{Message, update};
    use dozer_model::{GroupInfo, LocalizedText};
    use std::time::{Duration, Instant};

    fn empty_state() -> WorkspaceState {
        let mut state = WorkspaceState::default();
        update(
            &mut state,
            Message::GroupsLoaded(
                7,
                Ok(vec![GroupInfo {
                    id: 1,
                    project_id: 7,
                    topic: "t".into(),
                    created_ms: 0,
                    members: vec![],
                }]),
            ),
        );
        state
    }

    #[test]
    fn poll_wanted_only_when_visible_and_loading_or_active_turn() {
        let state = WorkspaceState::default();
        assert!(!poll_wanted(&state, false));
        assert!(poll_wanted(&state, true), "fresh state needs a load");
        let state = empty_state();
        assert!(!poll_wanted(&state, false));
        assert!(!poll_wanted(&state, true), "loaded, no active turn");
    }

    #[test]
    fn content_script_is_empty_when_unavailable_or_failed() {
        let state = empty_state();
        let mut push = WebviewPushState::default();
        let stamp = dozer_model::LocaleStamp {
            ui_locale: dozer_model::Locale::default_locale(),
            locale_revision: 0,
        };
        assert!(
            take_content_script(
                &state,
                &mut push,
                7,
                false,
                Instant::now(),
                stamp.clone(),
                "en"
            )
            .is_empty()
        );
        push.set_failed(LocalizedText::literal(crate::i18n::HOST_OWNER, "boom"));
        assert!(
            take_content_script(&state, &mut push, 7, true, Instant::now(), stamp, "en").is_empty()
        );
    }

    #[test]
    fn failed_state_is_not_cleared_by_a_late_ready_event() {
        let mut push = WebviewPushState::default();
        push.set_failed(LocalizedText::literal(crate::i18n::HOST_OWNER, "boom"));
        push.set_ready(true);
        assert!(push.failed().is_some(), "late ready must not undo failure");
    }

    #[test]
    fn draft_restore_and_push_are_one_shot() {
        let mut state = empty_state();
        state.set_composer_draft(1, "draft");
        let mut push = WebviewPushState::default();
        push.set_ready(true);
        let stamp = dozer_model::LocaleStamp {
            ui_locale: dozer_model::Locale::default_locale(),
            locale_revision: 0,
        };
        let first = take_content_script(
            &state,
            &mut push,
            7,
            true,
            Instant::now(),
            stamp.clone(),
            "en",
        );
        assert_eq!(first.len(), 2, "draft + payload");
        let second = take_content_script(&state, &mut push, 7, true, Instant::now(), stamp, "en");
        assert!(second.is_empty(), "same state is not re-pushed");
        let _ = Duration::ZERO;
    }
}
