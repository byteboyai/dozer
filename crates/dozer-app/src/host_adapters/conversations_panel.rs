//! Dozer presentation and native menu adapter for the neutral conversations panel.
use crate::app::{App, HoverId, Message as AppMessage, PanelKind};
use crate::workspace::{agent_dot_color, agent_icon, relative_time_text};
use byteui::interaction::icons::{self, IconKind};
use dozer_model::AgentKind;
use dozer_panel_conversations::{Message, adapter::*, update::UpdateCtx};
use iced_widget::core::{Color, Element, Length};
use std::path::Path;
impl PanelDisplay<AppMessage> for App {
    fn wrap(&self, message: Message) -> AppMessage {
        AppMessage::Conversations(message)
    }
    fn locale(&self) -> &str {
        self.locale.active_locale()
    }
    fn hover_progress(&self, hover: ConvHover) -> f32 {
        self.hover_progress(hover_id(hover))
    }
    fn panel_head<'a>(
        &self,
        icon: IconKind,
        title: String,
        actions: Option<Element<'a, AppMessage, iced_widget::Theme, iced_renderer::Renderer>>,
    ) -> Element<'a, AppMessage, iced_widget::Theme, iced_renderer::Renderer> {
        crate::chrome::homespace::home_panel_head_with_actions(icon, title, actions)
    }
    fn list_collapse_button<'a>(
        &self,
        collapse: String,
        expand: String,
    ) -> Element<'a, AppMessage, iced_widget::Theme, iced_renderer::Renderer> {
        self.list_collapse_button(
            PanelKind::Conversations,
            self.list_collapsed(PanelKind::Conversations),
            HoverId::list_collapse(PanelKind::Conversations),
            collapse,
            expand,
            AppMessage::TogglePanelListCollapse(PanelKind::Conversations),
            |h| AppMessage::Hover(HoverId::list_collapse(PanelKind::Conversations), h),
        )
    }
    fn tab_divider<'a>(
        &self,
    ) -> Element<'a, AppMessage, iced_widget::Theme, iced_renderer::Renderer> {
        byteui::layout::divider::horizontal()
    }
    fn pane_style(&self, pane: ConversationsPane) -> PaneStyle {
        let region = match pane {
            ConversationsPane::List => crate::theme::region::conversation_list_pane(),
            ConversationsPane::Detail => crate::theme::region::review_content_pane(),
        };
        PaneStyle {
            padding: region.padding,
            gap: region.gap,
            background: region.background,
            header_padding: crate::theme::region::project_pane().padding,
            line_height: crate::theme::terminal_font::line_height_factor(),
        }
    }
    fn agent_icon(
        &self,
        agent: AgentKind,
        color: Color,
    ) -> Element<'static, AppMessage, iced_widget::Theme, iced_renderer::Renderer> {
        icons::view(agent_icon(agent), byteui::theme::icon_size::row(), color)
    }
    fn agent_icon_color(&self, agent: AgentKind) -> Color {
        agent_dot_color(agent)
    }
    fn relative_time(&self, last: u64, now: u64) -> String {
        relative_time_text(last, now, self.locale.active_locale())
    }
    fn cursor_position(&self) -> (f32, f32) {
        self.last_cursor
    }
    fn menu_item<'a>(
        &self,
        leading: Option<Element<'a, AppMessage, iced_widget::Theme, iced_renderer::Renderer>>,
        label: String,
        color: Color,
        message: Option<AppMessage>,
    ) -> Element<'a, AppMessage, iced_widget::Theme, iced_renderer::Renderer> {
        crate::chrome::menu::item_row_fill(leading, label, color, message)
    }
    fn menu_shell<'a>(
        &self,
        items: Vec<Element<'a, AppMessage, iced_widget::Theme, iced_renderer::Renderer>>,
    ) -> Element<'a, AppMessage, iced_widget::Theme, iced_renderer::Renderer> {
        crate::chrome::menu::shell_frosted(items, Length::Fill)
    }
}
pub(crate) fn hover_id(hover: ConvHover) -> HoverId {
    match hover {
        ConvHover::SearchSubmit => HoverId::search_submit(PanelKind::Conversations),
        ConvHover::More => HoverId::more(PanelKind::Conversations),
    }
}
pub(crate) struct WorkspaceTodos<'a>(pub &'a crate::workspace::Workspace);
impl LinkedTodos for WorkspaceTodos<'_> {
    fn linked_text(&self, cid: &str) -> Option<String> {
        let task_id = self
            .0
            .conversations
            .sessions()?
            .iter()
            .find(|r| r.conversation_id == cid)?
            .task_id?;
        self.0
            .todo
            .items()
            .iter()
            .find(|item| item.id == task_id)
            .map(|item| item.text.clone())
    }
}
pub(crate) fn update_ctx<'a>(
    project_id: i64,
    path: &'a Path,
    services: &'a dozer_services::Services,
    handle: &'a tokio::runtime::Handle,
    locale: &'a str,
) -> UpdateCtx<'a> {
    UpdateCtx {
        project_id,
        project_path: path,
        conversation: &services.conversation,
        summary: &services.summary,
        handle,
        caps: ConversationsCapabilities::all(),
        locale,
        now_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
        relative_time: &relative_time_text,
    }
}
impl App {
    pub(crate) fn conversations_message(&mut self, project_id: Option<i64>, msg: Message) {
        match msg {
            Message::Hover(hover, h) => {
                self.set_hover(hover_id(hover), h);
                return;
            }
            Message::TextInputMenuOpen(target) => {
                self.update(AppMessage::TextInputMenuOpen(crate::app::TextInputTarget {
                    id: target.id,
                    secure: target.secure,
                }));
                return;
            }
            _ => {}
        }
        let locale = self.locale.active_locale().to_owned();
        let target = project_id.or_else(|| msg.result_project());
        let apply = move |ws: &mut crate::workspace::Workspace, io: &crate::workspace::ShellIo| {
            let Some(project) = ws.project.as_ref() else {
                return;
            };
            let services = crate::host_adapters::services::services_for(&io.client);
            let ctx = update_ctx(
                project.id,
                Path::new(&project.path),
                &services,
                &io.handle,
                &locale,
            );
            let proxy = io.proxy.clone();
            dozer_panel_conversations::update::update(
                &mut ws.conversations,
                msg,
                &ctx,
                move |message| {
                    let _ = proxy.send_event(AppMessage::Conversations(message));
                },
            );
        };
        if let Some(id) = target {
            self.with_project(id, apply);
        } else {
            self.with_focused_project(apply);
        }
        self.drain_conversations_requests();
    }
    fn drain_conversations_requests(&mut self) {
        let Some(ws) = self.active_workspace_mut() else {
            return;
        };
        let requests = ws.conversations.take_requests();
        for request in requests {
            let ConversationsRequest::ShowAgentFilter {
                at,
                current,
                present,
            } = request;
            #[cfg(target_os = "macos")]
            {
                let items = agent_picker_items_for(current, &present, self.locale.active_locale());
                if let Some(msg) = crate::chrome::native_menu::show(items, at) {
                    self.update(AppMessage::Conversations(msg));
                }
            }
            #[cfg(not(target_os = "macos"))]
            {
                let _ = (at, current, present);
                if let Some(ws) = self.active_workspace_mut() {
                    ws.conversations.set_agent_picker_open(true);
                }
            }
        }
    }
}
#[cfg(target_os = "macos")]
fn agent_picker_items_for(
    current: Option<AgentKind>,
    agents: &[AgentKind],
    locale: &str,
) -> Vec<crate::chrome::native_menu::Item<Message>> {
    use crate::chrome::native_menu::Item;
    let gold = byteui::theme::color::current().gold;
    let body = byteui::theme::color::current().body;
    let mut items: Vec<Item<Message>> = Vec::new();
    let is_all_current = current.is_none();
    items.push(Item::Entry {
        icon: None,
        icon_color: None,
        label: dozer_panel_conversations::i18n::t("filter.all", "All", locale),
        color: if is_all_current { gold } else { body },
        enabled: !is_all_current,
        msg: Message::AgentFilterSelect(None),
    });
    for &agent in agents {
        let is_current = current == Some(agent);
        items.push(Item::Entry {
            icon: Some(agent_icon(agent)),
            icon_color: Some(agent_dot_color(agent)),
            label: agent.label().into(),
            color: if is_current { gold } else { body },
            enabled: !is_current,
            msg: Message::AgentFilterSelect(Some(agent)),
        });
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "macos")]
    #[test]
    fn agent_picker_items_all_entry_enabled_only_when_a_filter_is_set() {
        let no_filter = None;
        let all_enabled = |current: &Option<AgentKind>| match &agent_picker_items_for(
            *current,
            &[AgentKind::Claude],
            "en",
        )[0]
        {
            crate::chrome::native_menu::Item::Entry { enabled, .. } => *enabled,
            crate::chrome::native_menu::Item::Separator => panic!("expected entry"),
        };
        assert!(!all_enabled(&no_filter), "已经是'全部'时该项自身该锁定");

        let with_filter = Some(AgentKind::Claude);
        assert!(all_enabled(&with_filter), "筛了某个 agent 时'全部'项该可点");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn agent_picker_items_locks_currently_selected_agent() {
        let items = agent_picker_items_for(
            Some(AgentKind::Claude),
            &[AgentKind::Claude, AgentKind::Opencode],
            "en",
        );
        // items[0] = "全部",之后按 conversation_agents_present 的稳定顺序
        // (Claude 先于 Opencode)排列。
        let enabled_at = |i: usize| match &items[i] {
            crate::chrome::native_menu::Item::Entry { enabled, .. } => *enabled,
            crate::chrome::native_menu::Item::Separator => panic!("expected entry"),
        };
        assert!(!enabled_at(1), "当前选中的 Claude 该锁定");
        assert!(enabled_at(2), "未选中的 Opencode 该可点");
    }
}
