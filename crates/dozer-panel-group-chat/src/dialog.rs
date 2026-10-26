//! New-group dialog card (window lifecycle stays in the desktop host).
use crate::adapter::PanelDisplay;
use crate::state::WorkspaceState;
use crate::{Message, consts::CREATABLE_AGENTS, i18n};
use iced_widget::core::{Element, Length};
use iced_widget::{button, column, container, row, text};

const TOPIC_AREA_HEIGHT: f32 = 72.0;

pub fn new_group_dialog_card<'a>(
    state: &'a WorkspaceState,
    host: &'a impl PanelDisplay<Message>,
    project_id: i64,
) -> Option<Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>> {
    if !state.new_group_dialog_open() {
        return None;
    }
    let c = byteui::theme::color::current();
    let locale = host.locale();
    let input = byteui::form::text_area::view(
        state.new_group_topic(),
        &i18n::t("new_group.placeholder", "New group topic…", locale),
        Some(iced_widget::core::widget::Id::new("group-chat-new-topic")),
        false,
        Some(TOPIC_AREA_HEIGHT),
        move |action| Message::NewGroupTopicAction(project_id, action),
    );
    let mut agent_rows: Vec<Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>> =
        Vec::with_capacity(CREATABLE_AGENTS.len());
    for agent in CREATABLE_AGENTS {
        let selected = state.new_group_selected_agents().contains(agent);
        let check = if selected {
            text("✓").size(byteui::theme::font::label()).color(c.green)
        } else {
            text("").size(byteui::theme::font::label()).color(c.dim)
        };
        let icon: Element<Message, iced_widget::Theme, iced_renderer::Renderer> =
            container(host.agent_icon(*agent, host.agent_icon_color(*agent))).into();
        let icon = icon.map(move |_| Message::Noop(project_id));
        let row = row![
            icon,
            text(agent.display_label())
                .size(byteui::theme::font::body())
                .color(if selected { c.cream } else { c.dim }),
            check,
        ]
        .align_y(iced_widget::core::alignment::Vertical::Center)
        .spacing(8);
        let btn = button(row)
            .on_press(Message::NewGroupAgentToggled(project_id, *agent))
            .width(Length::Fill)
            .padding(8)
            .style(byteui::interaction::cards::button_card(selected, c.card));
        agent_rows.push(btn.into());
    }
    let cancel = button(
        text(i18n::t("cancel.button", "Cancel", locale))
            .size(byteui::theme::font::label())
            .color(c.dim),
    )
    .on_press(Message::NewGroupDialogCancel(project_id))
    .padding([6, 12])
    .style(byteui::feedback::dialog::action_button_style(c.dim));
    let create = button(
        text(i18n::t("new_group.confirm", "Create", locale))
            .size(byteui::theme::font::label())
            .color(c.gold),
    )
    .on_press(Message::NewGroupSubmit(project_id))
    .padding([6, 12])
    .style(byteui::feedback::dialog::action_button_style(c.gold));
    let card = column![
        text(i18n::t("new_group.title", "New group chat", locale))
            .size(byteui::theme::font::subtitle())
            .color(c.cream),
        input,
        text(i18n::t("new_group.agents", "Participating agents", locale))
            .size(byteui::theme::font::caption_sm())
            .color(c.dim),
        iced_widget::Column::with_children(agent_rows).spacing(6),
        byteui::feedback::dialog::actions(row![cancel, create].spacing(8)),
    ]
    .spacing(12);
    Some(
        container(card)
            .padding(16)
            .style(byteui::feedback::dialog::card_style)
            .into(),
    )
}
