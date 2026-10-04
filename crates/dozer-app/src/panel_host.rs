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

/// 面板内一个可悬停元素的**槽位**——词汇通用,不含任何面板名(规格 E2:宿主公开类型里不出现业务类型)。
/// 与面板(`PanelKind`)一起构成 `HoverId::Panel(panel, slot)`。新增槽位种类前先看能不能用 `Named`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HoverSlot {
    /// 列表列折叠按钮。
    ListCollapse,
    /// 搜索框提交按钮。
    SearchSubmit,
    /// "更多…"翻页按钮。
    More,
    /// 页签标题(下标或稳定 id)。
    TabItem(u64),
    /// 页签关闭按钮。
    TabClose(u64),
    /// 页签溢出菜单按钮。
    TabOverflow,
    /// 列表行/卡片。
    Row(u64),
    /// 一组互斥选项里的一项。
    Choice(u64),
    /// 面板内一次性的具名按钮(同一面板内唯一)。
    Named(&'static str),
}

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
    // ---- H2:HoverId 的面板作用域键 ----

    use crate::extensions::git_log::FileFilter;

    #[test]
    fn same_slot_in_different_panels_is_a_different_key() {
        assert_ne!(
            HoverId::tab_item(PanelKind::Files, 1),
            HoverId::tab_item(PanelKind::Project, 1)
        );
        assert_ne!(
            HoverId::named(PanelKind::Files, "find_prev"),
            HoverId::named(PanelKind::Project, "find_prev")
        );
        assert_ne!(
            HoverId::list_collapse(PanelKind::Todo),
            HoverId::list_collapse(PanelKind::Usage)
        );
    }

    #[test]
    fn different_slots_and_keys_in_one_panel_are_different_keys() {
        let p = PanelKind::Database;
        let all = [
            HoverId::list_collapse(p),
            HoverId::search_submit(p),
            HoverId::more(p),
            HoverId::tab_overflow(p),
            HoverId::tab_item(p, 0),
            HoverId::tab_item(p, 1),
            HoverId::tab_close(p, 0),
            HoverId::row(p, 0),
            HoverId::choice(p, 0),
            HoverId::named(p, "a"),
            HoverId::named(p, "b"),
        ];
        let set: std::collections::HashSet<_> = all.iter().copied().collect();
        assert_eq!(set.len(), all.len());
    }

    #[test]
    fn git_file_filters_map_to_distinct_choice_keys() {
        let keys: std::collections::HashSet<_> = [
            FileFilter::All,
            FileFilter::Modified,
            FileFilter::Added,
            FileFilter::Deleted,
            FileFilter::Renamed,
        ]
        .into_iter()
        .map(|f| HoverId::choice(PanelKind::GitLog, f as u64))
        .collect();
        assert_eq!(keys.len(), 5);
    }
}
