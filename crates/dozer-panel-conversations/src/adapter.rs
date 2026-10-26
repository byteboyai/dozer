//! Product-owned rendering and native menu seams.
use crate::Message;
use byteui::interaction::icons::IconKind;
use dozer_model::AgentKind;
use iced_widget::core::{Color, Element, Padding};
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ConversationsCapabilities {
    pub summaries: bool,
    pub linked_todos: bool,
}
impl ConversationsCapabilities {
    pub const fn all() -> Self {
        Self {
            summaries: true,
            linked_todos: true,
        }
    }
}
pub trait LinkedTodos {
    fn linked_text(&self, conversation_id: &str) -> Option<String>;
}
#[derive(Debug, Clone, PartialEq)]
pub enum ConversationsRequest {
    ShowAgentFilter {
        at: (f32, f32),
        current: Option<AgentKind>,
        present: Vec<AgentKind>,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConvHover {
    SearchSubmit,
    More,
}
#[derive(Debug, Clone)]
pub struct TextInputTarget {
    pub id: iced_widget::core::widget::Id,
    pub secure: bool,
}
#[derive(Debug, Clone, Copy)]
pub enum ConversationsPane {
    List,
    Detail,
}
#[derive(Clone, Copy)]
pub struct PaneStyle {
    pub padding: Padding,
    pub gap: f32,
    pub background: Option<Color>,
    pub header_padding: Padding,
    pub line_height: f32,
}
pub trait PanelDisplay<M> {
    fn wrap(&self, message: Message) -> M;
    fn locale(&self) -> &str;
    fn hover_progress(&self, hover: ConvHover) -> f32;
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
    fn tab_divider<'a>(&self) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>;
    fn pane_style(&self, pane: ConversationsPane) -> PaneStyle;
    fn agent_icon(
        &self,
        agent: AgentKind,
        color: Color,
    ) -> Element<'static, M, iced_widget::Theme, iced_renderer::Renderer>;
    fn agent_icon_color(&self, agent: AgentKind) -> Color;
    fn relative_time(&self, last_ts_ms: u64, now_ms: u64) -> String;
    fn menu_item<'a>(
        &self,
        leading: Option<Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>>,
        label: String,
        color: Color,
        message: Option<M>,
    ) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>;
    fn menu_shell<'a>(
        &self,
        items: Vec<Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>>,
    ) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>;
    fn cursor_position(&self) -> (f32, f32) {
        (0.0, 0.0)
    }
}
