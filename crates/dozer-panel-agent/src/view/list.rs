//! Agent 会话列表 + 卡片(S6c-2,从 `dozer-app` 的 `workspace/view.rs` 搬入)。

use crate::card::{CardCtx, card_model};
use crate::display::{AgentHover, AgentPane, El, PanelDisplay};
use crate::i18n;
use crate::state::AgentPanelState;
use crate::style::{agent_dot_color, agent_icon, group_tabs_by_agent};
use crate::update::Message;
use byteui::interaction::icons;
use iced_widget::core::text::Wrapping;
use iced_widget::core::{Border, Color, Length};
use iced_widget::{Theme, button, column, container, row, text};

/// Agent 列表面板:按 agent 分组展示会话,点击一行选中(NoDrag)。
pub fn agent_list<'a, M: Clone + 'static>(
    state: &'a AgentPanelState,
    host: &impl PanelDisplay<'a, M>,
    cards: &CardCtx<'a>,
    width: Length,
    outer: Border,
) -> iced_widget::core::Element<'a, M, Theme, iced_renderer::Renderer> {
    let region = host.pane_style(AgentPane::List);
    let locale = host.locale();
    let picker = picker_toggle(host);
    let mut content: iced_widget::Column<'a, M, Theme, iced_renderer::Renderer> =
        column![host.panel_head(icons::IconKind::Brain, "Agent", Some(picker),)]
            .spacing(region.gap);

    if state.tabs.is_empty() {
        content = content.push(
            text(i18n::text(
                "agent.no_conversations",
                "No conversations",
                locale,
            ))
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().dim),
        );
    } else {
        for (agent, idxs) in group_tabs_by_agent(&state.tabs) {
            content = content.push(
                row![
                    icons::view(
                        agent_icon(agent),
                        byteui::theme::icon_size::row(),
                        agent_dot_color(agent),
                    ),
                    text(format!("{}（{}）", agent.display_label(), idxs.len()))
                        .size(byteui::theme::font::caption())
                        .color(byteui::theme::color::current().dim),
                ]
                .align_y(iced_widget::core::alignment::Vertical::Center)
                .spacing(8),
            );
            for idx in idxs {
                content = content.push(card(
                    &state.tabs[idx],
                    idx == state.active,
                    idx,
                    host,
                    cards,
                    locale,
                ));
            }
        }
    }

    container(content.padding(region.padding))
        .width(width)
        .height(Length::Fill)
        .style(move |_theme: &Theme| container::Style {
            background: region.background.map(Into::into),
            border: outer,
            ..container::Style::default()
        })
        .into()
}

fn picker_toggle<'a, M: Clone + 'static>(host: &impl PanelDisplay<'a, M>) -> El<'a, M> {
    let slot = AgentHover::PickerToggle;
    icons::icon_button_entry(
        icons::IconKind::SquarePlus,
        byteui::theme::icon_size::row(),
        false,
        false,
        host.hover_progress(slot),
        false,
        byteui::theme::geometry::tab_button_size(),
        true,
        host.wrap(Message::PickerToggle),
        {
            let on = host.hover_message(slot, true);
            let off = host.hover_message(slot, false);
            move |hovered| {
                if hovered { on.clone() } else { off.clone() }
            }
        },
        i18n::text("agent.new_session", "New agent session", host.locale()),
    )
}

fn card<'a, M: Clone + 'static>(
    tab: &crate::tabs::SessionTab,
    active: bool,
    idx: usize,
    host: &impl PanelDisplay<'a, M>,
    ctx: &CardCtx<'_>,
    locale: &str,
) -> El<'a, M> {
    let model = card_model(tab, active, ctx, locale);
    let lines = column![
        text(model.title)
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().cream),
        text(model.work_line)
            .size(byteui::theme::font::caption())
            .color(byteui::theme::color::current().dim)
            .wrapping(Wrapping::WordOrGlyph),
        row![
            byteui::feedback::status::dot(model.dot),
            text(model.status)
                .size(byteui::theme::font::caption_sm())
                .color(byteui::theme::color::current().dim),
        ]
        .spacing(6)
        .align_y(iced_widget::core::Alignment::Center),
    ]
    .spacing(4);
    button(container(lines).padding(10))
        .on_press(host.wrap(Message::SelectTabNoDrag(idx)))
        .width(Length::Fill)
        .style(byteui::interaction::cards::button_card(
            active,
            byteui::theme::color::current().card,
        ))
        .into()
}

/// 供测试断言:卡片颜色跟随 `byteui::theme::color::current().card`。
pub fn card_background() -> Color {
    byteui::theme::color::current().card
}
