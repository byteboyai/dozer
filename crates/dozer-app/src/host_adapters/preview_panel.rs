//! 预览面板的**产品侧适配层**(S5b-2 P2)。
//!
//! 面板视图已抽到 `dozer-panel-preview`;库经 [`PanelDisplay`] 反过来要产品提供展示能力
//! (悬停表、页签部件、折叠按钮、配色区域、顶层消息类型)。这里把它们逐项转调 Dozer 现有的
//! chrome 函数,行为与抽库前逐项相同。Files 与 Project 两个面板共用同一个 `App` 实现,
//! 靠 `panel: ViewerPanel` 参数区分。

use dozer_panel_preview::display::{
    ActionButtonStyle, Element, OverflowMenuArgs, PaneStyle, PanelDisplay, PreviewHover, TabArgs,
};
use dozer_panel_preview::{Message as LibMessage, ViewerPanel};
use iced_widget::MouseArea;
use iced_widget::core::{Color, mouse};

use crate::app::{App, HoverId, Message, PanelKind, TabGroup};
use crate::chrome::tab_widget::{
    PanelTabArgs, TabOverflowEntry, TabOverflowMenuArgs, panel_tab, tab_json_mode_button,
    tab_overflow_button, tab_overflow_menu, tab_render_mode_button, tab_tabular_mode_button,
};

fn kind_of(panel: ViewerPanel) -> PanelKind {
    PanelKind::from(panel)
}

fn tab_group_of(panel: ViewerPanel) -> TabGroup {
    match panel {
        ViewerPanel::Project => TabGroup::ProjectPreview,
        _ => TabGroup::Preview,
    }
}

/// 库的悬停部件 → Dozer 悬停动画表的键。
pub(crate) fn hover_id_of(panel: ViewerPanel, hover: PreviewHover) -> HoverId {
    let kind = kind_of(panel);
    match hover {
        PreviewHover::TabItem(i) => HoverId::tab_item(kind, i as u64),
        PreviewHover::TabClose(i) => HoverId::tab_close(kind, i as u64),
        PreviewHover::TabOverflow => HoverId::tab_overflow(kind),
        PreviewHover::TabOverflowRow(i) => HoverId::TabOverflowRow(i),
        PreviewHover::RenderMode => HoverId::named(kind, "render_mode"),
        PreviewHover::TabularMode => HoverId::named(kind, "tabular_mode"),
        PreviewHover::JsonMode => HoverId::named(kind, "json_mode"),
        PreviewHover::FindNext => HoverId::named(kind, "find_next"),
        PreviewHover::FindPrev => HoverId::named(kind, "find_prev"),
        PreviewHover::FindReplaceToggle => HoverId::named(kind, "find_replace_toggle"),
        PreviewHover::FindReplaceCurrent => HoverId::named(kind, "find_replace_current"),
        PreviewHover::FindReplaceAll => HoverId::named(kind, "find_replace_all"),
    }
}

impl PanelDisplay<Message> for App {
    fn wrap(&self, panel: ViewerPanel, message: LibMessage) -> Message {
        Message::Preview(kind_of(panel), message)
    }

    fn locale(&self) -> &str {
        self.locale.active_locale()
    }

    fn product_name(&self) -> &str {
        "Dozer"
    }

    fn render_text(&self, text: &dozer_model::LocalizedText) -> String {
        crate::i18n::host_render(text, self.locale.active_locale())
    }

    fn pane_style(&self) -> PaneStyle {
        let region = crate::theme::region::preview_pane();
        PaneStyle {
            padding: region.padding,
            gap: region.gap,
            background: region.background,
        }
    }

    fn hover_progress(&self, panel: ViewerPanel, hover: PreviewHover) -> f32 {
        App::hover_progress(self, hover_id_of(panel, hover))
    }

    fn hover_tooltip_ready(&self, panel: ViewerPanel, hover: PreviewHover) -> bool {
        App::hover_tooltip_ready(self, hover_id_of(panel, hover))
    }

    fn hover_message(&self, panel: ViewerPanel, hover: PreviewHover, hovered: bool) -> Message {
        Message::Hover(hover_id_of(panel, hover), hovered)
    }

    fn tab_bar_avail_px(&self, panel: ViewerPanel) -> f32 {
        self.preview_tab_bar_avail_px(kind_of(panel))
    }

    fn tab_display_width(&self, title: &str) -> f32 {
        crate::workspace::preview_tab_display_width(title)
    }

    fn tab_divider<'a>(&self) -> Element<'a, Message> {
        crate::app::tab_divider()
    }

    fn panel_tab<'a>(&self, args: TabArgs<'a, Message>) -> Element<'a, Message> {
        let TabArgs {
            title,
            active,
            hover_t,
            close_hover_t,
            on_select,
            on_close,
            show_tooltip,
            title_hover,
            close_hover,
        } = args;
        panel_tab(PanelTabArgs {
            title,
            active,
            hover_t,
            close_hover_t,
            prefix: None,
            suffix: None,
            on_select,
            on_close,
            show_tooltip,
            title_hover,
            close_hover,
        })
    }

    fn tab_gestures<'a>(
        &self,
        panel: ViewerPanel,
        index: usize,
        tab: Element<'a, Message>,
        path: Option<std::path::PathBuf>,
    ) -> Element<'a, Message> {
        // 拖拽换位:按住页签(选中处理已把 `app.tab_drag` 置位)后光标扫过哪个页签,这个 `on_move`
        // 就按它发 `TabDragMove`,完成换位。
        let group = tab_group_of(panel);
        let mut area = MouseArea::new(tab).on_move(move |_| Message::TabDragMove { group, index });
        // 页签右键菜单:仅对真实文件 tab 生效。
        if let Some(path) = path {
            area = area.on_right_press(Message::Files(
                crate::extensions::files::Message::TabContextMenuOpen {
                    kind: crate::extensions::files::state::preview_kind_of(kind_of(panel)),
                    path,
                },
            ));
        }
        if self.dragging_group(group) {
            area = area.interaction(mouse::Interaction::Grabbing);
        }
        area.into()
    }

    fn overflow_button<'a>(
        &self,
        tab_count: usize,
        hover_t: f32,
        on_press: Message,
        on_hover: Box<dyn Fn(bool) -> Message + 'a>,
    ) -> Option<Element<'a, Message>> {
        tab_overflow_button(tab_count, hover_t, on_press, on_hover)
    }

    fn render_mode_button<'a>(
        &self,
        in_code_mode: bool,
        hover_t: f32,
        on_press: Message,
        on_hover: Box<dyn Fn(bool) -> Message + 'a>,
    ) -> Element<'a, Message> {
        tab_render_mode_button(in_code_mode, hover_t, on_press, on_hover)
    }

    fn tabular_mode_button<'a>(
        &self,
        in_text: bool,
        hover_t: f32,
        on_press: Message,
        on_hover: Box<dyn Fn(bool) -> Message + 'a>,
    ) -> Element<'a, Message> {
        tab_tabular_mode_button(
            in_text,
            hover_t,
            on_press,
            on_hover,
            self.locale.active_locale(),
        )
    }

    fn json_mode_button<'a>(
        &self,
        in_text: bool,
        hover_t: f32,
        on_press: Message,
        on_hover: Box<dyn Fn(bool) -> Message + 'a>,
    ) -> Element<'a, Message> {
        tab_json_mode_button(
            in_text,
            hover_t,
            on_press,
            on_hover,
            self.locale.active_locale(),
        )
    }

    fn collapse_button<'a>(&self, panel: ViewerPanel) -> Element<'a, Message> {
        let locale = self.locale.active_locale();
        match panel {
            ViewerPanel::Project => self.list_collapse_button(
                PanelKind::Project,
                self.list_collapsed(PanelKind::Project),
                HoverId::list_collapse(PanelKind::Project),
                crate::i18n::host_text("panel.collapse_list", "Collapse list", locale),
                crate::i18n::host_text("panel.expand_list", "Expand list", locale),
                Message::TogglePanelListCollapse(PanelKind::Project),
                move |hovered| Message::Hover(HoverId::list_collapse(PanelKind::Project), hovered),
            ),
            _ => self.list_collapse_button(
                PanelKind::Files,
                self.files_tree_collapsed(),
                HoverId::list_collapse(PanelKind::Files),
                crate::i18n::host_text("files.collapse_tree", "Collapse file tree", locale),
                crate::i18n::host_text("files.expand_tree", "Expand file tree", locale),
                Message::ToggleFileTreeCollapse,
                move |hovered| Message::Hover(HoverId::list_collapse(PanelKind::Files), hovered),
            ),
        }
    }

    fn overflow_menu<'a>(&self, args: OverflowMenuArgs<'a, Message>) -> Element<'a, Message> {
        let OverflowMenuArgs {
            entries,
            anchor,
            on_select,
            on_close,
            on_dismiss,
            on_row_hover,
        } = args;
        tab_overflow_menu(TabOverflowMenuArgs {
            entries: entries
                .into_iter()
                .map(|e| TabOverflowEntry {
                    index: e.index,
                    prefix: None,
                    title: e.title,
                    active: e.active,
                    closable: e.closable,
                    hover_t: e.hover_t,
                })
                .collect(),
            anchor,
            window_size: self.window_size,
            on_select,
            on_close,
            on_dismiss,
            on_row_hover,
        })
    }

    fn action_button_style(&self, text_color: Color) -> ActionButtonStyle {
        Box::new(crate::extensions::project_create::action_button_hover_style(text_color))
    }

    fn find_font_size(&self) -> f32 {
        crate::workspace::tree_row_font_size()
    }

    fn line_height_factor(&self) -> f32 {
        crate::theme::terminal_font::line_height_factor()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    const ALL_HOVERS: [PreviewHover; 12] = [
        PreviewHover::TabItem(3),
        PreviewHover::TabClose(3),
        PreviewHover::TabOverflow,
        PreviewHover::TabOverflowRow(3),
        PreviewHover::RenderMode,
        PreviewHover::TabularMode,
        PreviewHover::JsonMode,
        PreviewHover::FindNext,
        PreviewHover::FindPrev,
        PreviewHover::FindReplaceToggle,
        PreviewHover::FindReplaceCurrent,
        PreviewHover::FindReplaceAll,
    ];

    /// 穷举:`PreviewHover` 新增变体时这里编译即报错,提醒补 `hover_id_of`。
    fn assert_exhaustive(h: PreviewHover) {
        match h {
            PreviewHover::TabItem(_)
            | PreviewHover::TabClose(_)
            | PreviewHover::TabOverflow
            | PreviewHover::TabOverflowRow(_)
            | PreviewHover::RenderMode
            | PreviewHover::TabularMode
            | PreviewHover::JsonMode
            | PreviewHover::FindNext
            | PreviewHover::FindPrev
            | PreviewHover::FindReplaceToggle
            | PreviewHover::FindReplaceCurrent
            | PreviewHover::FindReplaceAll => {}
        }
    }

    #[test]
    fn hover_slots_map_one_to_one_to_the_legacy_hover_ids() {
        for h in ALL_HOVERS {
            assert_exhaustive(h);
        }
        // 与迁移前 `workspace/view.rs` 里逐个构造的 `HoverId` 一致。
        for (panel, kind) in [
            (ViewerPanel::Files, PanelKind::Files),
            (ViewerPanel::Project, PanelKind::Project),
        ] {
            assert_eq!(
                hover_id_of(panel, PreviewHover::TabItem(3)),
                HoverId::tab_item(kind, 3)
            );
            assert_eq!(
                hover_id_of(panel, PreviewHover::TabClose(3)),
                HoverId::tab_close(kind, 3)
            );
            assert_eq!(
                hover_id_of(panel, PreviewHover::TabOverflow),
                HoverId::tab_overflow(kind)
            );
            for (h, name) in [
                (PreviewHover::RenderMode, "render_mode"),
                (PreviewHover::TabularMode, "tabular_mode"),
                (PreviewHover::JsonMode, "json_mode"),
                (PreviewHover::FindNext, "find_next"),
                (PreviewHover::FindPrev, "find_prev"),
                (PreviewHover::FindReplaceToggle, "find_replace_toggle"),
                (PreviewHover::FindReplaceCurrent, "find_replace_current"),
                (PreviewHover::FindReplaceAll, "find_replace_all"),
            ] {
                assert_eq!(hover_id_of(panel, h), HoverId::named(kind, name), "{h:?}");
            }
            // 溢出下拉的行悬停是全局键(下拉同一时刻只属于一个面板)。
            assert_eq!(
                hover_id_of(panel, PreviewHover::TabOverflowRow(3)),
                HoverId::TabOverflowRow(3)
            );
        }
        // Files 与 Project 不碰撞(按面板区分的部件)。
        let mut seen = HashSet::new();
        for panel in [ViewerPanel::Files, ViewerPanel::Project] {
            for h in ALL_HOVERS {
                if matches!(h, PreviewHover::TabOverflowRow(_)) {
                    continue;
                }
                assert!(seen.insert(format!("{:?}", hover_id_of(panel, h))), "{h:?}");
            }
        }
    }

    #[test]
    fn panel_kind_mapping_is_total_for_preview_panels_and_falls_back_to_files_for_others() {
        assert_eq!(kind_of(ViewerPanel::Files), PanelKind::Files);
        assert_eq!(kind_of(ViewerPanel::Project), PanelKind::Project);
        assert_eq!(kind_of(ViewerPanel::GitLog), PanelKind::GitLog);
        assert_eq!(
            ViewerPanel::from(PanelKind::Conversations),
            ViewerPanel::Files
        );
    }

    #[test]
    fn tab_groups_follow_the_panel() {
        assert_eq!(tab_group_of(ViewerPanel::Files), TabGroup::Preview);
        assert_eq!(tab_group_of(ViewerPanel::Project), TabGroup::ProjectPreview);
    }
}
