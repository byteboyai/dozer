//! 面板 view 对宿主的**只读视图契约**:面板需要从宿主问的东西(悬停动画进度、列表列折叠态、
//! 窗口/光标位置、折叠按钮)集中在这一个 trait 里,面板代码写 `app: &impl PanelHost`,
//! 不再 import 宿主的 `App`。
//!
//! 这是 bytehost 面板边界的第一块契约(见 `docs/superpowers/specs/2026-10-03-bytehost-boundary-design.md`
//! §2 的 E1/E2):trait 里不得出现任何面板的业务类型;`HoverId`/`PanelKind` 暂时仍是宿主类型,
//! 它们的命名空间化/注册制是后续切片(`docs/dozer-v2/bytehost-H0/00-summary.md` §4)。
//! 目前只有 `App` 一个实现;不为未来的第二个实现预先抽象更多方法——面板需要什么才加什么。

use crate::app::{App, HoverId, PanelKind};
use iced_widget::core::Element;

pub trait PanelHost {
    /// 某个按钮/页签的悬停动画进度 0.0..=1.0。
    fn hover_progress(&self, id: HoverId) -> f32;
    /// 悬停是否已持续满 tooltip 延迟(该弹标题全称 tooltip 了)。
    fn hover_tooltip_ready(&self, id: HoverId) -> bool;
    /// 两栏面板的列表列当前是否收起。
    fn list_collapsed(&self, kind: PanelKind) -> bool;
    /// 列表列的折叠/展开按钮(图标随面板所在栏位镜像)。
    #[allow(clippy::too_many_arguments)]
    fn list_collapse_button<'a, M: Clone + 'a>(
        &self,
        kind: PanelKind,
        collapsed: bool,
        hover_id: HoverId,
        tooltip_collapse: &'a str,
        tooltip_expand: &'a str,
        on_select: M,
        on_hover: impl Fn(bool) -> M + 'a,
    ) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>;
    /// 窗口逻辑尺寸 `(宽, 高)`。
    fn window_size(&self) -> (f32, f32);
    /// 最近一次光标位置 `(x, y)`(窗口坐标)。
    fn last_cursor(&self) -> (f32, f32);
}

impl PanelHost for App {
    fn hover_progress(&self, id: HoverId) -> f32 {
        App::hover_progress(self, id)
    }

    fn hover_tooltip_ready(&self, id: HoverId) -> bool {
        App::hover_tooltip_ready(self, id)
    }

    fn list_collapsed(&self, kind: PanelKind) -> bool {
        App::list_collapsed(self, kind)
    }

    fn list_collapse_button<'a, M: Clone + 'a>(
        &self,
        kind: PanelKind,
        collapsed: bool,
        hover_id: HoverId,
        tooltip_collapse: &'a str,
        tooltip_expand: &'a str,
        on_select: M,
        on_hover: impl Fn(bool) -> M + 'a,
    ) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer> {
        App::list_collapse_button(
            self,
            kind,
            collapsed,
            hover_id,
            tooltip_collapse,
            tooltip_expand,
            on_select,
            on_hover,
        )
    }

    fn window_size(&self) -> (f32, f32) {
        self.window_size
    }

    fn last_cursor(&self) -> (f32, f32) {
        self.last_cursor
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced_widget::core::{Border, Length};
    use std::cell::Cell;

    /// 不依赖 `App` 的假宿主:证明"一个面板 + 宿主契约"就能构造出 view(规格 §6 第 5 条的第一个探针)。
    #[derive(Default)]
    struct FakeHost {
        collapse_buttons_asked: Cell<u32>,
    }

    impl PanelHost for FakeHost {
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

    #[test]
    fn usage_content_pane_builds_against_a_fake_host_and_asks_it_for_the_collapse_button() {
        let host = FakeHost::default();
        let ws_state = crate::extensions::usage::WorkspaceState::default();
        let _view = crate::extensions::usage::content_pane(
            &host,
            &ws_state,
            Length::Fill,
            Border::default(),
        );
        assert_eq!(host.collapse_buttons_asked.get(), 1);
    }
}
