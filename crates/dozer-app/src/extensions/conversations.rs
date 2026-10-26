//! Shared panel exports and Dozer keyboard focus capture.
pub use dozer_panel_conversations::update::spawn_refresh;
pub use dozer_panel_conversations::{Message, WorkspaceState};
use iced_widget::core::{
    Rectangle,
    widget::{Id, Operation, operation::Focusable},
};
use std::sync::Mutex;
dozer_core::scope!(pub(crate) LOG, panel, "conversations");
/// 会话列表搜索框(iced 原生 `text_input`)的 `widget::Id`:main.rs 每帧
/// `interface.operate` 用 `CaptureConversationSearchFocus` 问真实焦点态。
pub use dozer_panel_conversations::view::conversation_search_field_id;

static CONVERSATION_SEARCH_FOCUSED: std::sync::LazyLock<Mutex<bool>> =
    std::sync::LazyLock::new(|| Mutex::new(false));

/// 读走并复位(消费式),避免搜索框不可见的帧卡死上一次 `true` 永久堵死终端
/// 键盘转发。
pub fn take_conversation_search_focused() -> bool {
    std::mem::replace(&mut *CONVERSATION_SEARCH_FOCUSED.lock().unwrap(), false)
}

/// 每帧 `interface.operate()` 跑一遍。`traverse` 必须调用传入闭包(见
/// [[dozer-operation-traverse-noop-bug]])。
pub struct CaptureConversationSearchFocus;
impl Operation<()> for CaptureConversationSearchFocus {
    fn focusable(&mut self, id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Focusable) {
        if id == Some(&conversation_search_field_id()) {
            *CONVERSATION_SEARCH_FOCUSED.lock().unwrap() = state.is_focused();
        }
    }

    fn traverse(&mut self, operate: &mut dyn for<'a> FnMut(&'a mut (dyn Operation<()> + 'a))) {
        operate(self);
    }
}

pub fn on_activate(state: &WorkspaceState, ctx: &crate::panel_host::ActivationCtx<Message>) {
    if let Some(path) = ctx.project_path.as_deref() {
        let services = ctx.io.services();
        let update_ctx = crate::host_adapters::conversations_panel::update_ctx(
            ctx.project_id,
            path,
            services,
            ctx.io.handle(),
            "en",
        );
        dozer_panel_conversations::update::on_activate(state, &update_ctx, ctx.io.emitter());
    }
}

#[cfg(test)]
mod activation_tests {
    use super::*;
    trait Payload {
        fn result_payload(self) -> Self;
    }
    impl Payload for Message {
        fn result_payload(self) -> Self {
            match self {
                Self::AsyncResult(_, inner) => inner.result_payload(),
                other => other,
            }
        }
    }

    use crate::panel_host::testing::{TIMEOUT, offline_ctx, runtime};

    #[test]
    fn on_activate_refreshes_the_session_list_of_that_project() {
        let rt = runtime();
        let (ctx, rx) = offline_ctx::<Message>(&rt, 7, Some(std::env::temp_dir()));
        on_activate(&WorkspaceState::default(), &ctx);
        assert!(matches!(
            rx.recv_timeout(TIMEOUT).unwrap().result_payload(),
            Message::SessionsRefreshed(7, Err(_))
        ));
    }

    #[test]
    fn on_activate_without_a_project_path_does_nothing() {
        let rt = runtime();
        let (ctx, rx) = offline_ctx::<Message>(&rt, 7, None);
        on_activate(&WorkspaceState::default(), &ctx);
        assert!(
            rx.recv_timeout(std::time::Duration::from_millis(200))
                .is_err()
        );
    }
}
