//! 终端条:页签栏、终端网格、溢出菜单与空态(S6c-2,从 `dozer-app` 的
//! `term/terminal.rs` 搬入;Dozer 专属的上下文条经 [`TerminalSlots`] 注入)。

use crate::display::{
    AgentHover, AgentPane, El, OverflowEntry, PanelDisplay, TabArgs, TerminalSlots,
};
use crate::i18n;
use crate::state::AgentPanelState;
use crate::style::{agent_state_label, dot_color};
use crate::tabs::{tab_display_width, tab_title};
use crate::update::Message;
use iced_widget::core::{Border, Element, Length, Padding};
use iced_widget::{MouseArea, space};
use iced_widget::{Theme, column, container, row, text};

/// 终端栏:页签栏 + 非零退出码提示 + 终端网格 + 产品插槽。
pub fn terminal_pane<'a, M: Clone + 'static>(
    state: &'a AgentPanelState,
    host: &impl PanelDisplay<'a, M>,
    slots: TerminalSlots<'a, M>,
    width: Length,
    outer: Border,
) -> Element<'a, M, Theme, iced_renderer::Renderer> {
    let region = host.pane_style(AgentPane::Terminal);
    let mut content = column![tab_bar(state, host)].spacing(region.gap);

    if let Some(code) = state.tabs.get(state.active).and_then(|t| t.last_exit)
        && code != 0
    {
        content = content.push(
            text(format!("exit {code}"))
                .size(byteui::theme::font::label())
                .color(byteui::theme::color::current().red),
        );
    }

    let term: Element<'a, M, Theme, iced_renderer::Renderer> = active_tab_view(state, host);
    let term = match slots.floating {
        Some(panel) => iced_widget::stack![
            term,
            container(column![
                space::vertical(),
                container(panel).padding(Padding {
                    bottom: region.padding.bottom + 34.0 + region.gap,
                    ..Default::default()
                }),
            ])
            .width(Length::Fill)
            .height(Length::Fill),
        ]
        .into(),
        None => term,
    };
    content = content.push(term);
    if let Some(footer) = slots.footer {
        content = content.push(footer);
    }

    let body = container(content.spacing(region.gap).padding(region.padding))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_theme: &Theme| container::Style {
            background: region.background.map(Into::into),
            border: outer,
            ..container::Style::default()
        });
    container(body).width(width).height(Length::Fill).into()
}

fn tab_bar<'a, M: Clone + 'static>(
    state: &'a AgentPanelState,
    host: &impl PanelDisplay<'a, M>,
) -> El<'a, M> {
    let widths: Vec<f32> = state
        .tabs
        .iter()
        .map(|t| tab_display_width(&tab_title(t.agent, t.cwd.as_deref(), &t.info.name)))
        .collect();
    let window = byteui::interaction::tab_strip::tab_window(
        &widths,
        4.0,
        host.tab_bar_avail_px(),
        state.term_tab_first,
    );
    let items: Vec<El<'a, M>> = state
        .tabs
        .iter()
        .enumerate()
        .filter(|(idx, _)| (window.first..window.visible_end).contains(idx))
        .map(|(idx, tab)| {
            let title = tab_title(tab.agent, tab.cwd.as_deref(), &tab.info.name);
            let content = host.panel_tab(TabArgs {
                title,
                active: idx == state.active,
                dot: dot_color(tab.agent_state, tab.alive),
                item: AgentHover::TabItem(idx),
                close: AgentHover::TabClose(idx),
                on_select: host.wrap(Message::SelectTab(idx)),
                on_close: host.wrap(Message::CloseTab(idx)),
            });
            host.drag_surface(content, idx)
        })
        .collect();

    let tabs_row = row(items).spacing(4);
    let clipped = container(tabs_row).width(Length::Fill).clip(true);
    let overflow = host.overflow_button(
        state.tabs.len(),
        host.wrap(Message::OverflowToggle { at: None }),
    );

    let mut tab_row = row![]
        .spacing(4)
        .align_y(iced_widget::core::Alignment::Center);
    if let Some(btn) = overflow {
        tab_row = tab_row.push(btn);
    }
    tab_row = tab_row.push(clipped);
    let locale = host.locale();
    tab_row = tab_row.push(host.list_collapse_button(
        i18n::text("panel.collapse_list", "Collapse list", locale),
        i18n::text("panel.expand_list", "Expand list", locale),
    ));

    column![tab_row, host.tab_divider()].spacing(4).into()
}

fn active_tab_view<'a, M: Clone + 'static>(
    state: &'a AgentPanelState,
    host: &impl PanelDisplay<'a, M>,
) -> Element<'a, M, Theme, iced_renderer::Renderer> {
    match state.tabs.get(state.active) {
        Some(tab) => {
            let focused = host.terminal_focused();
            let preedit = focused.then(|| host.ime_preedit()).flatten();
            let map = host.term_event_fn();
            dozer_panel_term::view::view(
                &tab.model,
                focused,
                (),
                preedit,
                host.term_style(),
                move |_t: (), e| map(e),
            )
        }
        None => container(
            text(i18n::text(
                "term.no_sessions",
                "No sessions yet — click “+” in the Agent panel",
                host.locale(),
            ))
            .size(byteui::theme::font::subtitle())
            .color(byteui::theme::color::current().dim),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .into(),
    }
}

/// 终端 tab 栏溢出下拉浮层(由产品在顶层 `stack![..]` 里拼)。
pub fn overflow_popup<'a, M: Clone + 'static>(
    state: &'a AgentPanelState,
    host: &impl PanelDisplay<'a, M>,
) -> Option<El<'a, M>> {
    let anchor = state.term_tab_overflow_anchor?;
    if state.tabs.is_empty() {
        return None;
    }
    let entries: Vec<OverflowEntry> = state
        .tabs
        .iter()
        .enumerate()
        .map(|(idx, tab)| OverflowEntry {
            index: idx,
            title: tab_title(tab.agent, tab.cwd.as_deref(), &tab.info.name),
            active: idx == state.active,
            dot: dot_color(tab.agent_state, tab.alive),
            row: AgentHover::OverflowRow(idx),
        })
        .collect();
    Some(host.overflow_menu(
        entries,
        anchor,
        Message::SelectTab,
        Message::CloseTab,
        Message::OverflowDismiss,
    ))
}

/// 供测试:非零退出码提示只在 active tab 显示。
pub fn exit_code_line(state: &AgentPanelState) -> Option<String> {
    state
        .tabs
        .get(state.active)
        .and_then(|t| t.last_exit)
        .filter(|c| *c != 0)
        .map(|c| format!("exit {c}"))
}

/// 供测试:空态文案 key。
pub fn empty_label_key() -> &'static str {
    "term.no_sessions"
}

/// 供测试:状态文案(与页签/卡片同一套)。
pub fn state_label(state: dozer_model::AgentState, locale: &str) -> String {
    agent_state_label(state, locale)
}

/// `MouseArea` 包装的拖拽感应层(产品实现可复用)。
pub fn drag_area<'a, M: Clone + 'static>(content: El<'a, M>, on_move: M) -> El<'a, M> {
    MouseArea::new(content)
        .on_move(move |_| on_move.clone())
        .into()
}
