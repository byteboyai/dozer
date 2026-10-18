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

/// 库消息 → 今天的顶层消息:Files → `Message::Preview*`,Project → `Message::ProjectPreview*`;
/// 没有 Project 专属变体的,走原有的带 `PanelKind` 的变体。库里只描述「对这个面板做什么」的 UI
/// 消息有映射;后台任务结果(`Profiled` 等)由产品带路由信息自行构造,返回 `None`。
pub(crate) fn legacy_message(panel: ViewerPanel, message: LibMessage) -> Option<Message> {
    let project = panel == ViewerPanel::Project;
    let kind = kind_of(panel);
    Some(match message {
        LibMessage::OpenPath { path, .. } => {
            if project {
                Message::ProjectPreviewOpenPath(path)
            } else {
                Message::PreviewOpenPath(path)
            }
        }
        LibMessage::SelectTab(i) => {
            if project {
                Message::ProjectPreviewSelectTab(i)
            } else {
                Message::PreviewSelectTab(i)
            }
        }
        LibMessage::CloseTab(i) => {
            if project {
                Message::ProjectPreviewCloseTab(i)
            } else {
                Message::PreviewCloseTab(i)
            }
        }
        LibMessage::ToggleRenderMode(i) => {
            if project {
                Message::ProjectPreviewToggleRenderMode(i)
            } else {
                Message::PreviewToggleRenderMode(i)
            }
        }
        LibMessage::TabOverflowToggle { .. } => {
            if project {
                Message::ProjectPreviewTabOverflowToggle
            } else {
                Message::PreviewTabOverflowToggle
            }
        }
        LibMessage::TabOverflowDismiss => {
            if project {
                Message::ProjectPreviewTabOverflowDismiss
            } else {
                Message::PreviewTabOverflowDismiss
            }
        }
        LibMessage::OpenExternal(t) => Message::PreviewOpenExternal(kind, t),
        LibMessage::Retry(t) => Message::PreviewRetry(kind, t),
        LibMessage::PlainTextOpen(t) => Message::PreviewPlainTextOpen(kind, t),
        LibMessage::ConflictKeep(t) => Message::PreviewConflictKeep(kind, t),
        LibMessage::ConflictReload(t) => Message::PreviewConflictReload(kind, t),
        LibMessage::TabularTextModeToggle(t) => Message::PreviewTabularTextModeToggle(kind, t),
        LibMessage::JsonModeToggle(t) => Message::PreviewJsonModeToggle(kind, t),
        LibMessage::SaveActive => Message::PreviewSaveActive(kind),
        LibMessage::InsertTab => Message::PreviewTabInsertTab(kind),
        LibMessage::UndoActive => Message::PreviewUndoActive(kind),
        LibMessage::RedoActive => Message::PreviewRedoActive(kind),
        LibMessage::FindOpen => Message::PreviewFindOpen(kind),
        LibMessage::FindOpenWithReplace => Message::PreviewFindOpenWithReplace(kind),
        LibMessage::FindReplaceToggle => Message::PreviewFindReplaceToggle(kind),
        LibMessage::FindClose => Message::PreviewFindClose(kind),
        LibMessage::FindText(q) => Message::PreviewFindText(kind, q),
        LibMessage::FindGo(next) => Message::PreviewFindGo(kind, next),
        LibMessage::FindCase(c) => Message::PreviewFindCase(kind, c),
        LibMessage::FindReplacement(r) => Message::PreviewFindReplacement(kind, r),
        LibMessage::FindReplaceCurrent => Message::PreviewFindReplaceCurrent(kind),
        LibMessage::FindReplaceAll => Message::PreviewFindReplaceAll(kind),
        LibMessage::FindWebviewState { current, total } => {
            Message::PreviewFindWebviewState(kind, current, total)
        }
        LibMessage::LargeFileSearchClose => Message::PreviewLargeFileSearchClose(kind),
        LibMessage::LargeFileSearchSubmit { tab_id, query } => {
            Message::PreviewLargeFileSearchSubmit(kind, tab_id, query)
        }
        LibMessage::LargeFileSearchGo(forward) => Message::PreviewLargeFileSearchGo(kind, forward),
        _ => return None,
    })
}

impl PanelDisplay<Message> for App {
    fn wrap(&self, panel: ViewerPanel, message: LibMessage) -> Message {
        legacy_message(panel, message)
            .expect("视图只发 UI 类消息;后台任务结果由产品带路由信息自行构造")
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
                    kind: kind_of(panel),
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
