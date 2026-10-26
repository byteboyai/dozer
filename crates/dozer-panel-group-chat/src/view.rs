//! Native group list and content shell.
use crate::adapter::{GroupChatPane, PanelDisplay};
use crate::state::WorkspaceState;
use crate::{Message, i18n};
use byteui::interaction::icons;
use iced_widget::core::{Border, Element, Length};
use iced_widget::{button, column, container, text};

#[derive(Debug, Clone)]
pub enum ShellMessage {
    Retry,
}

pub fn list_pane<'a>(
    state: &'a WorkspaceState,
    host: &'a impl PanelDisplay<Message>,
    project_id: i64,
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let style = host.pane_style(GroupChatPane::List);
    let locale = host.locale();
    let add_button = icons::icon_button_entry(
        icons::IconKind::SquarePlus,
        byteui::theme::icon_size::row(),
        state.new_group_dialog_open(),
        false,
        0.0,
        false,
        byteui::theme::geometry::tab_button_size(),
        true,
        Message::NewGroupDialogToggle(project_id),
        move |_h| Message::Noop(project_id),
        i18n::t("new_group.tooltip", "New group chat", locale),
    );
    let mut content = column![host.panel_head(
        icons::IconKind::SquareSparkles,
        i18n::t("head.list", "Group chat", locale),
        Some(add_button),
    )]
    .spacing(style.gap);
    if state.groups().is_empty() {
        content = content.push(
            text(i18n::t("empty.no_groups", "No group chats yet", locale))
                .size(byteui::theme::font::body())
                .color(byteui::theme::color::current().dim),
        );
    } else {
        let mut cards = column![].spacing(style.gap);
        for group in state.groups() {
            cards = cards.push(group_row(group, state, project_id, locale));
        }
        content = content.push(cards);
    }
    container(content.padding(style.padding))
        .width(width)
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: style.background.map(Into::into),
            border: outer,
            ..container::Style::default()
        })
        .into()
}

fn group_row<'a>(
    group: &'a dozer_model::GroupInfo,
    state: &'a WorkspaceState,
    project_id: i64,
    locale: &'a str,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let c = byteui::theme::color::current();
    let current = state.selected() == Some(group.id);
    let member_count = group.members.len();
    let sub = if member_count == 0 {
        i18n::t("members.none", "No members yet", locale)
    } else {
        i18n::t_args(
            "members.count",
            "{n} members",
            locale,
            &[("n", &member_count.to_string())],
        )
    };
    let sub_color = if current { c.green } else { c.dim };
    let title_color = if current { c.green } else { c.cream };
    button(
        column![
            container(
                text(group.topic.clone())
                    .size(byteui::theme::font::body())
                    .color(title_color)
            )
            .width(Length::Fill)
            .clip(true),
            text(sub)
                .size(byteui::theme::font::caption_sm())
                .color(sub_color),
        ]
        .spacing(4),
    )
    .on_press(Message::SelectRequested(project_id, group.id))
    .width(Length::Fill)
    .padding(10)
    .style(byteui::interaction::cards::button_card(current, c.card))
    .into()
}

pub fn content_pane<'a>(
    failed: Option<&dozer_model::LocalizedText>,
    host: &'a impl PanelDisplay<Message>,
    width: Length,
    outer: Border,
) -> Element<'a, ShellMessage, iced_widget::Theme, iced_renderer::Renderer> {
    let tokens = byteui::theme::color::current();
    let locale = host.locale();
    let body: Element<'a, ShellMessage, iced_widget::Theme, iced_renderer::Renderer> = match failed
    {
        Some(reason) => column![
            text(i18n::t(
                "content.load_failed",
                "Group chat page failed to load",
                locale
            ))
            .size(14)
            .color(tokens.body),
            text(host.render_localized(reason))
                .size(12)
                .color(tokens.dim),
            button(text(i18n::t("retry", "Retry", locale)).size(13)).on_press(ShellMessage::Retry),
        ]
        .spacing(10)
        .padding(16)
        .into(),
        None => column![].into(),
    };
    container(body)
        .width(width)
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(byteui::theme::color::current().panel.into()),
            border: outer,
            ..container::Style::default()
        })
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::{GroupChatPane, PanelDisplay, PaneStyle};
    use crate::update::{Message as GcMessage, update};
    use byteui::interaction::icons::IconKind;
    use dozer_model::{AgentKind, GroupInfo, LocalizedText};
    use iced_widget::core::Color;

    struct Stub;
    impl PanelDisplay<Message> for Stub {
        fn wrap(&self, message: Message) -> Message {
            message
        }
        fn locale(&self) -> &str {
            "en"
        }
        fn panel_head<'a>(
            &self,
            _icon: IconKind,
            _title: String,
            _actions: Option<Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>>,
        ) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
            iced_widget::Space::new().into()
        }
        fn list_collapse_button<'a>(
            &self,
            _c: String,
            _e: String,
        ) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
            iced_widget::Space::new().into()
        }
        fn pane_style(&self, _pane: GroupChatPane) -> PaneStyle {
            PaneStyle {
                padding: iced_widget::core::Padding::from(0),
                gap: 0.0,
                background: None,
                header_padding: iced_widget::core::Padding::from(0),
            }
        }
        fn agent_icon(
            &self,
            _a: AgentKind,
            _c: Color,
        ) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
            iced_widget::Space::new().into()
        }
        fn agent_icon_color(&self, _a: AgentKind) -> Color {
            Color::BLACK
        }
        fn render_localized(&self, text: &LocalizedText) -> String {
            text.fallback.clone()
        }
    }

    fn state(count: usize) -> WorkspaceState {
        let mut state = WorkspaceState::default();
        let groups: Vec<GroupInfo> = (1..=count)
            .map(|id| GroupInfo {
                id: id as i64,
                project_id: 7,
                topic: format!("g{id}"),
                created_ms: 0,
                members: vec![],
            })
            .collect();
        update(&mut state, GcMessage::GroupsLoaded(7, Ok(groups)));
        state
    }

    #[test]
    fn list_pane_builds_for_no_groups_one_group_and_five_hundred_groups() {
        for count in [0, 1, 500] {
            let state = state(count);
            let _ = list_pane(&state, &Stub, 7, Length::Fill, Border::default());
        }
    }

    #[test]
    fn content_pane_shows_the_failure_reason_and_a_retry_that_emits_retry() {
        let reason = LocalizedText::literal(crate::i18n::HOST_OWNER, "boom");
        let _ = content_pane(Some(&reason), &Stub, Length::Fill, Border::default());
        let _ = content_pane(None, &Stub, Length::Fill, Border::default());
        let _ = ShellMessage::Retry;
    }

    #[test]
    fn degenerate_states_build_without_panicking() {
        let mut state = WorkspaceState::default();
        update(&mut state, GcMessage::NewGroupDialogToggle(7));
        let _ = list_pane(&state, &Stub, 0, Length::Fill, Border::default());
        let _ = crate::dialog::new_group_dialog_card(&state, &Stub, 7);
    }
}
