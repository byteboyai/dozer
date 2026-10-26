//! Product-owned presentation seams for the group-chat panel.
use crate::Message;
use byteui::interaction::icons::IconKind;
use dozer_model::{AgentKind, LocalizedText};
use iced_widget::core::{Color, Element, Padding};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupChatPane {
    List,
    Content,
}

#[derive(Debug, Clone, Copy)]
pub struct PaneStyle {
    pub padding: Padding,
    pub gap: f32,
    pub background: Option<Color>,
    pub header_padding: Padding,
}

pub trait PanelDisplay<M> {
    fn wrap(&self, message: Message) -> M;
    fn locale(&self) -> &str;
    fn panel_head<'a>(
        &self,
        icon: IconKind,
        title: String,
        actions: Option<Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>>,
    ) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>;
    fn list_collapse_button<'a>(
        &self,
        tooltip_collapse: String,
        tooltip_expand: String,
    ) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>;
    fn pane_style(&self, pane: GroupChatPane) -> PaneStyle;
    fn agent_icon(
        &self,
        agent: AgentKind,
        color: Color,
    ) -> Element<'static, M, iced_widget::Theme, iced_renderer::Renderer>;
    fn agent_icon_color(&self, agent: AgentKind) -> Color;
    /// Resolve a host-owned `LocalizedText` (e.g. the webview timeout message).
    fn render_localized(&self, text: &LocalizedText) -> String;
}
