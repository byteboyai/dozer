//! `PanelHost` 的最小假实现(从 `panel_host.rs` 测试里的 `FakeHost` 演化而来,那份是私有的)。

use crate::app::{HoverId, PanelKind};
use crate::panel_host::PanelHost;
use iced_widget::core::Element;
use std::cell::Cell;

#[derive(Default)]
pub(crate) struct FakePanelHost {
    collapse_buttons_asked: Cell<u32>,
}

impl FakePanelHost {
    pub(crate) fn collapse_buttons_asked(&self) -> u32 {
        self.collapse_buttons_asked.get()
    }
}

impl PanelHost for FakePanelHost {
    fn hover_progress(&self, _id: HoverId) -> f32 {
        0.0
    }
    fn hover_tooltip_ready(&self, _id: HoverId) -> bool {
        false
    }
    fn list_collapsed(&self, _kind: PanelKind) -> bool {
        false
    }
    fn list_collapse_button<'a, M: Clone + 'a>(
        &self,
        _kind: PanelKind,
        _collapsed: bool,
        _hover_id: HoverId,
        _tooltip_collapse: &'a str,
        _tooltip_expand: &'a str,
        _on_select: M,
        _on_hover: impl Fn(bool) -> M + 'a,
    ) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer> {
        self.collapse_buttons_asked
            .set(self.collapse_buttons_asked.get() + 1);
        iced_widget::Space::new().into()
    }
    fn window_size(&self) -> (f32, f32) {
        (800.0, 600.0)
    }
    fn last_cursor(&self) -> (f32, f32) {
        (0.0, 0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::usage::{self, WorkspaceState};
    use iced_widget::core::{Border, Length};

    #[test]
    fn usage_views_build_against_the_fake_host() {
        let host = FakePanelHost::default();
        let ws = WorkspaceState::default();
        let _content = usage::content_pane(&host, &ws, Length::Fill, Border::default());
        let _list = usage::list_pane(&ws, Length::Fill, Border::default());
        assert_eq!(host.collapse_buttons_asked(), 1);
    }
}
