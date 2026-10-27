//! Dozer presentation adapter for the shared group-chat panel.
use crate::app::App;
use crate::workspace::{agent_dot_color, agent_icon};
use byteui::interaction::icons::IconKind;
use dozer_model::{AgentKind, LocalizedText};
use dozer_panel_group_chat::Message;
use dozer_panel_group_chat::adapter::{GroupChatPane, PaneStyle};
use iced_widget::core::{Color, Element, Padding};

impl dozer_panel_group_chat::adapter::PanelDisplay<Message> for App {
    fn wrap(&self, message: Message) -> Message {
        message
    }

    fn locale(&self) -> &str {
        self.locale.active_locale()
    }

    fn panel_head<'a>(
        &self,
        icon: IconKind,
        title: String,
        actions: Option<Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>>,
    ) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
        crate::chrome::homespace::home_panel_head_with_actions(icon, title, actions)
    }

    fn pane_style(&self, pane: GroupChatPane) -> PaneStyle {
        match pane {
            GroupChatPane::List => {
                let r = crate::theme::region::group_chat_list_pane();
                PaneStyle {
                    padding: r.padding,
                    gap: r.gap,
                    background: r.background,
                    header_padding: r.padding,
                }
            }
            GroupChatPane::Content => PaneStyle {
                padding: Padding::from(0),
                gap: 0.0,
                background: Some(byteui::theme::color::current().panel),
                header_padding: Padding::from(0),
            },
        }
    }

    fn agent_icon(
        &self,
        agent: AgentKind,
        color: Color,
    ) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
        iced_widget::container(byteui::interaction::icons::view(
            agent_icon(agent),
            byteui::theme::icon_size::row(),
            color,
        ))
        .into()
    }

    fn agent_icon_color(&self, agent: AgentKind) -> Color {
        agent_dot_color(agent)
    }

    fn render_localized(&self, text: &LocalizedText) -> String {
        crate::i18n::host_render(text, self.locale.active_locale())
    }
}
