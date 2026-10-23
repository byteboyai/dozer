//! Agent 面板的 Dozer 适配层(S6c-1):hook/MCP 安装器,以及把宿主事件
//! 落回 Dozer 的跨面板联动。

use dozer_model::AgentKind;
use dozer_panel_agent::adapter::HookInstaller;
use std::path::PathBuf;

/// 转调 Dozer 既有 hook/MCP 安装实现(路径与安装逻辑留在产品侧)。
pub(crate) struct DozerHooks;

impl HookInstaller for DozerHooks {
    fn ensure_hook(&self, agent: AgentKind) {
        crate::workspace::ensure_hook_installed(agent);
    }

    fn ensure_mcp(&self, agent: AgentKind) {
        crate::workspace::ensure_mcp_installed(agent);
    }

    fn hook_exe(&self) -> String {
        crate::workspace::dozer_hook_binary_path(
            &std::env::current_exe().unwrap_or_else(|_| PathBuf::from("dozer")),
        )
        .to_string_lossy()
        .into_owned()
    }
}

// ---- PanelDisplay:库视图的 Dozer 展示适配(S6c-2) ----

use crate::app::{App, HoverId, Message, PanelKind, TabGroup};
use crate::chrome::tab_widget;
use crate::theme;
use dozer_panel_agent::display::{
    AgentHover, AgentPane, El, OverflowEntry, PaneStyle, PanelDisplay, TabArgs,
};
use dozer_panel_agent::update::Message as AgentMessage;
use dozer_panel_term::view::TermEvent;

/// 库的悬停槽位 → Dozer 的 `HoverId`(一一对应,互不碰撞)。
pub(crate) fn hover_id(slot: AgentHover) -> HoverId {
    match slot {
        AgentHover::PickerToggle => HoverId::named(PanelKind::Agent, "picker_toggle"),
        AgentHover::TabItem(i) => HoverId::TermTabItem(i),
        AgentHover::TabClose(i) => HoverId::TermTabClose(i),
        AgentHover::TabOverflow => HoverId::TermTabOverflow,
        AgentHover::OverflowRow(i) => HoverId::TabOverflowRow(i),
    }
}

pub(crate) struct DozerDisplay<'a> {
    pub(crate) app: &'a App,
    project_id: i64,
    style: &'a dozer_panel_term::style::TermStyle,
}

impl<'a> DozerDisplay<'a> {
    pub(crate) fn new(app: &'a App, project_id: i64) -> Self {
        Self {
            app,
            project_id,
            style: crate::term::term_view::terminal_style_ref(),
        }
    }

    fn hover(&self, slot: AgentHover) -> f32 {
        self.app.hover_progress(hover_id(slot))
    }
}

impl<'a> PanelDisplay<'a, Message> for DozerDisplay<'a> {
    fn wrap(&self, message: AgentMessage) -> Message {
        // 视图消息本身不经过这里到达 App;由调用方在 `Message::Agent` 包
        // 一层项目 id。视图里只需要库 `Message` → 顶层 `Message` 的恒等
        // 映射,项目路由在 `App::agent_panel_message` 里完成。
        Message::Agent(self.project_id, message)
    }
    fn locale(&self) -> &str {
        self.app.locale.active_locale()
    }
    fn hover_progress(&self, slot: AgentHover) -> f32 {
        self.app.hover_progress(hover_id(slot))
    }
    fn hover_message(&self, slot: AgentHover, hovered: bool) -> Message {
        Message::Hover(hover_id(slot), hovered)
    }
    fn hover_tooltip_ready(&self, slot: AgentHover) -> bool {
        self.app.hover_tooltip_ready(hover_id(slot))
    }
    fn pane_style(&self, pane: AgentPane) -> PaneStyle {
        let r = match pane {
            AgentPane::List => theme::region::agent_list_pane(),
            AgentPane::Terminal => theme::region::terminal_pane(),
        };
        PaneStyle {
            gap: r.gap,
            padding: r.padding,
            background: r.background,
        }
    }
    fn panel_head(
        &self,
        icon: byteui::interaction::icons::IconKind,
        title: &'a str,
        actions: Option<El<'a, Message>>,
    ) -> El<'a, Message> {
        crate::chrome::homespace::home_panel_head_with_actions(icon, title, actions)
    }
    fn list_collapse_button(&self, collapse: String, expand: String) -> El<'a, Message> {
        self.app.list_collapse_button(
            PanelKind::Agent,
            self.app.list_collapsed(PanelKind::Agent),
            HoverId::list_collapse(PanelKind::Agent),
            collapse,
            expand,
            Message::TogglePanelListCollapse(PanelKind::Agent),
            |hovered| Message::Hover(HoverId::list_collapse(PanelKind::Agent), hovered),
        )
    }
    fn tab_divider(&self) -> El<'a, Message> {
        crate::app::tab_divider()
    }
    fn panel_tab(&self, args: TabArgs<Message>) -> El<'a, Message> {
        tab_widget::panel_tab(tab_widget::PanelTabArgs {
            title: args.title,
            active: args.active,
            hover_t: self.hover(args.item),
            close_hover_t: self.hover(args.close),
            prefix: Some(byteui::feedback::status::dot(args.dot)),
            suffix: None,
            on_select: args.on_select,
            on_close: args.on_close,
            show_tooltip: self.hover_tooltip_ready(args.item),
            title_hover: {
                let id = hover_id(args.item);
                move |h| Message::Hover(id, h)
            },
            close_hover: {
                let id = hover_id(args.close);
                move |h| Message::Hover(id, h)
            },
        })
    }
    fn overflow_button(&self, tab_count: usize, on_press: Message) -> Option<El<'a, Message>> {
        tab_widget::tab_overflow_button(
            tab_count,
            self.hover(AgentHover::TabOverflow),
            on_press,
            |hovered| Message::Hover(hover_id(AgentHover::TabOverflow), hovered),
        )
    }
    fn overflow_menu(
        &self,
        entries: Vec<OverflowEntry>,
        anchor: (f32, f32),
        on_select: fn(usize) -> AgentMessage,
        on_close: fn(usize) -> AgentMessage,
        on_dismiss: AgentMessage,
    ) -> El<'a, Message> {
        let project_id = self.project_id;
        let entries: Vec<tab_widget::TabOverflowEntry<'static, Message>> = entries
            .into_iter()
            .map(|e| tab_widget::TabOverflowEntry {
                index: e.index,
                prefix: Some(byteui::feedback::status::dot(e.dot)),
                title: e.title,
                active: e.active,
                closable: true,
                hover_t: self.app.hover_progress(hover_id(e.row)),
            })
            .collect();
        tab_widget::tab_overflow_menu(tab_widget::TabOverflowMenuArgs {
            entries,
            anchor,
            window_size: self.app.window_size,
            on_select: move |i| Message::Agent(project_id, on_select(i)),
            on_close: move |i| Message::Agent(project_id, on_close(i)),
            on_dismiss: Message::Agent(project_id, on_dismiss),
            on_row_hover: move |idx, hovered| {
                Message::Hover(hover_id(AgentHover::OverflowRow(idx)), hovered)
            },
        })
    }
    fn drag_surface(&self, content: El<'a, Message>, index: usize) -> El<'a, Message> {
        // 换位必须经 App 的拖拽状态机(按下武装 + 位移阈值 + 组校验),
        // 否则光标划过页签就会触发拖动。
        crate::term::terminal::tab_drag_surface(
            content,
            TabGroup::Terminal,
            index,
            self.app.dragging_group(TabGroup::Terminal),
            Message::TabDragMove {
                group: TabGroup::Terminal,
                index,
            },
        )
    }
    fn term_style(&self) -> &'a dozer_panel_term::style::TermStyle {
        self.style
    }
    fn terminal_focused(&self) -> bool {
        crate::term::terminal::keyboard_term_target(self.app.left_view, self.app.active_zone)
            == crate::term::terminal::TermTarget::Shared
    }
    fn ime_preedit(&self) -> Option<&'a str> {
        self.app.term_ime_preedit()
    }
    fn term_event(&self, event: TermEvent) -> Message {
        crate::term::term_view::map_event(crate::term::terminal::TermTarget::Shared, event)
    }
    fn term_event_fn(&self) -> fn(TermEvent) -> Message {
        |event| crate::term::term_view::map_event(crate::term::terminal::TermTarget::Shared, event)
    }
    fn tab_bar_avail_px(&self) -> f32 {
        self.app.terminal_tab_bar_avail_px()
    }
}

impl dozer_panel_agent::adapter::TaskTitleSource for crate::workspace::Workspace {
    fn task_title_for_session(&self, session_id: &str) -> Option<String> {
        crate::extensions::todo::WorkspaceState::task_title_for_session(&self.todo, session_id)
            .map(str::to_string)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hover_slots_map_one_to_one_to_the_legacy_hover_ids() {
        assert_eq!(
            hover_id(AgentHover::PickerToggle),
            HoverId::named(PanelKind::Agent, "picker_toggle")
        );
        assert_eq!(hover_id(AgentHover::TabItem(3)), HoverId::TermTabItem(3));
        assert_eq!(hover_id(AgentHover::TabClose(3)), HoverId::TermTabClose(3));
        assert_eq!(hover_id(AgentHover::TabOverflow), HoverId::TermTabOverflow);
        assert_eq!(
            hover_id(AgentHover::OverflowRow(3)),
            HoverId::TabOverflowRow(3)
        );
        // 互不碰撞:同一数字在 item/close/row 三个槽位上映射到不同 HoverId。
        assert_ne!(
            hover_id(AgentHover::TabItem(3)),
            hover_id(AgentHover::TabClose(3))
        );
        assert_ne!(
            hover_id(AgentHover::TabClose(3)),
            hover_id(AgentHover::OverflowRow(3))
        );
    }
}
