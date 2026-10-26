use crate::{
    Message,
    adapter::*,
    filter::*,
    i18n::{t, t_args},
    state::WorkspaceState,
};
use byteui::interaction::icons::{self, IconKind};
use dozer_model::AgentKind;
use iced_widget::core::{Border, Element, Length};
use iced_widget::{MouseArea, Scrollable, button, column, container, row, scrollable};
pub fn conversation_search_field_id() -> iced_widget::core::widget::Id {
    iced_widget::core::widget::Id::new("conversation-search-box")
}
pub(crate) fn lh<'a>(
    factor: f32,
    t: iced_widget::text::Text<'a, iced_widget::Theme, iced_renderer::Renderer>,
) -> iced_widget::text::Text<'a, iced_widget::Theme, iced_renderer::Renderer> {
    t.line_height(iced_widget::core::text::LineHeight::Relative(factor))
}
fn loading_hint_localized<'a>(
    curve: byteui::feedback::math_curve::Curve,
    key: &str,
    fallback: &str,
    locale: &str,
    size: f32,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    container(
        column![
            byteui::feedback::math_curve::view_animated(curve, size),
            text(t(key, fallback, locale))
                .size(byteui::theme::font::body())
                .color(byteui::theme::color::current().dim),
        ]
        .spacing(12)
        .align_x(iced_widget::core::alignment::Horizontal::Center),
    )
    .center_x(Length::Fill)
    .center_y(Length::Fill)
    .into()
}

fn footer_bar<'a, M: Clone + 'a>(
    ws_state: &WorkspaceState,
    agents: &[AgentKind],
    app: &'a impl PanelDisplay<M>,
) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer> {
    let locale = app.locale();
    if agents.is_empty() {
        return column![].into();
    }

    let current_label = ws_state
        .agent_filter
        .map(|a| a.label().to_string())
        .unwrap_or_else(|| t("filter.all", "All", locale));
    let switch = button(icons::view(
        if ws_state.agent_picker_open {
            IconKind::ChevronUp
        } else {
            IconKind::ChevronDown
        },
        byteui::theme::icon_size::row(),
        byteui::theme::color::current().cream,
    ))
    .on_press(if ws_state.agent_picker_open {
        app.wrap(Message::AgentFilterClose)
    } else {
        app.wrap(Message::AgentFilterOpen {
            at: app.cursor_position(),
        })
    })
    .padding(6)
    .style(|_t: &iced_widget::Theme, _s| button::Style {
        background: None,
        text_color: byteui::theme::color::current().cream,
        border: iced_widget::core::Border {
            color: byteui::theme::color::current().border,
            width: 0.0,
            radius: 4.0.into(),
        },
        ..button::Style::default()
    });

    let bar = row![
        icons::view(
            IconKind::Bot,
            byteui::theme::icon_size::row(),
            byteui::theme::color::current().cream,
        ),
        text(current_label)
            .size(byteui::theme::font::label())
            .color(byteui::theme::color::current().cream),
        iced_widget::space::horizontal(),
        switch,
    ]
    .spacing(6)
    .align_y(iced_widget::core::Alignment::Center);

    let top_line = container(iced_widget::Space::new())
        .width(Length::Fill)
        .height(Length::Fixed(1.0))
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(byteui::theme::color::current().border.into()),
            ..container::Style::default()
        });

    container(column![top_line, bar].spacing(4))
        .width(Length::Fill)
        .padding([6, 0])
        .style(|_t: &iced_widget::Theme| container::Style {
            background: None,
            ..container::Style::default()
        })
        .into()
}

fn agent_picker_view<'a, M: Clone + 'a>(
    ws_state: &WorkspaceState,
    agents: &[AgentKind],
    app: &'a impl PanelDisplay<M>,
) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer> {
    let locale = app.locale();
    if !ws_state.agent_picker_open {
        return iced_widget::Space::new().into();
    }
    let is_all_current = ws_state.agent_filter.is_none();
    let mut items: Vec<Element<'_, M, iced_widget::Theme, iced_renderer::Renderer>> = Vec::new();
    items.push(app.menu_item(
        None,
        t("filter.all", "All", locale),
        if is_all_current {
            byteui::theme::color::current().gold
        } else {
            byteui::theme::color::current().body
        },
        (!is_all_current).then_some(app.wrap(Message::AgentFilterSelect(None))),
    ));
    for &agent in agents {
        let is_current = ws_state.agent_filter == Some(agent);
        let color = if is_current {
            byteui::theme::color::current().gold
        } else {
            byteui::theme::color::current().body
        };
        let leading = app.agent_icon(agent, app.agent_icon_color(agent));
        items.push(app.menu_item(
            Some(leading),
            agent.label().into(),
            color,
            (!is_current).then_some(app.wrap(Message::AgentFilterSelect(Some(agent)))),
        ));
    }
    let panel: Element<'_, M, iced_widget::Theme, iced_renderer::Renderer> = app.menu_shell(items);

    let dismiss = MouseArea::new(
        iced_widget::Space::new()
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .on_press(app.wrap(Message::AgentFilterClose));
    let positioned = column![iced_widget::Space::new().height(Length::Fill), panel]
        .width(Length::Fill)
        .height(Length::Fill);
    iced_widget::stack![dismiss, positioned]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

pub fn conversations_list<'a, M: Clone + 'a>(
    app: &'a impl PanelDisplay<M>,
    ws_state: &'a WorkspaceState,
    todos: &dyn LinkedTodos,
    caps: ConversationsCapabilities,
    width: Length,
    outer: Border,
) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer> {
    let locale = app.locale();
    let current_review_id = ws_state.current_conversation_id();
    let region = app.pane_style(ConversationsPane::List);
    let mut content = column![app.panel_head(
        IconKind::BotMessageSquare,
        t("head.title", "Conversations", locale),
        None,
    )]
    .spacing(region.gap);

    let search_active = ws_state.search_focused || !ws_state.search.is_empty();
    let search_placeholder = t(
        "search.placeholder",
        "Search conversation title/summary…",
        locale,
    );
    let search_box = byteui::form::search_box::view(
        &search_placeholder,
        &ws_state.search_draft,
        Some(conversation_search_field_id()),
        search_active,
        |s| app.wrap(Message::SearchInput(s)),
        app.wrap(Message::SearchSubmit),
        app.hover_progress(ConvHover::SearchSubmit),
        |hovered| app.wrap(Message::Hover(ConvHover::SearchSubmit, hovered)),
    );
    content = content.push(byteui::interaction::context_menu::wrap(
        search_box,
        Some(app.wrap(Message::TextInputMenuOpen(TextInputTarget {
            id: conversation_search_field_id(),
            secure: false,
        }))),
    ));

    let Some(rows) = ws_state.sessions.as_ref() else {
        content = content.push(
            loading_hint_localized(
                byteui::feedback::math_curve::Curve::RoseThree,
                "loading",
                "Loading…",
                locale,
                48.0,
            )
            .map(|msg| app.wrap(msg)),
        );
        return container(content.padding(region.padding))
            .width(width)
            .height(Length::Fill)
            .style(move |_t: &iced_widget::Theme| container::Style {
                background: region.background.map(Into::into),
                border: outer,
                ..container::Style::default()
            })
            .into();
    };

    let agents_present = conversation_agents_present(rows);
    let filtered = filter_sessions(rows, &ws_state.search, ws_state.agent_filter);
    if rows.is_empty() {
        content = content.push(lh(
            region.line_height,
            text(t("empty.no_sessions", "No conversations yet", locale))
                .size(byteui::theme::font::body())
                .color(byteui::theme::color::current().dim),
        ));
    } else if filtered.is_empty() {
        content = content.push(lh(
            region.line_height,
            text(t("empty.no_match", "No matching results", locale))
                .size(byteui::theme::font::body())
                .color(byteui::theme::color::current().dim),
        ));
    }
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let mut cards = column![].spacing(region.gap);
    let visible = conversation_visible_count(ws_state.pages);
    for g in filtered.iter().take(visible) {
        let current = current_review_id == Some(g.conversation_id.as_str());
        let agent_label = g.agent.label();
        let task_suffix = caps
            .linked_todos
            .then(|| todos.linked_text(&g.conversation_id))
            .flatten()
            .map(|text| {
                let truncated: String = text.chars().take(30).collect();
                if text.chars().count() > 30 {
                    t_args(
                        "row.task_suffix_truncated",
                        " · Linked todo: {text}…",
                        locale,
                        &[("text", truncated.as_str())],
                    )
                } else {
                    t_args(
                        "row.task_suffix",
                        " · Linked todo: {text}",
                        locale,
                        &[("text", truncated.as_str())],
                    )
                }
            })
            .unwrap_or_default();
        let time = app.relative_time(g.last_ts, now_ms);
        let sub = if current {
            t_args(
                "row.current",
                "● Current · {agent} · {time}{task}",
                locale,
                &[
                    ("agent", agent_label),
                    ("time", time.as_str()),
                    ("task", task_suffix.as_str()),
                ],
            )
        } else {
            t_args(
                "row.normal",
                "{agent} · {time}{task}",
                locale,
                &[
                    ("agent", agent_label),
                    ("time", time.as_str()),
                    ("task", task_suffix.as_str()),
                ],
            )
        };
        let sub_color = if current {
            byteui::theme::color::current().green
        } else {
            byteui::theme::color::current().dim
        };
        let row_btn = button(
            row![
                app.agent_icon(g.agent, app.agent_icon_color(g.agent)),
                column![
                    lh(
                        region.line_height,
                        text(g.display_title.clone())
                            .size(byteui::theme::font::body())
                            .color(byteui::theme::color::current().cream)
                    ),
                    lh(
                        region.line_height,
                        text(sub)
                            .size(byteui::theme::font::caption_sm())
                            .color(sub_color)
                    ),
                ]
                .spacing(4),
            ]
            .spacing(8)
            .align_y(iced_widget::core::Alignment::Center),
        )
        .on_press(app.wrap(Message::SessionOpen(g.conversation_id.clone(), g.agent)))
        .width(Length::Fill)
        .padding(10)
        .style(byteui::interaction::cards::button_card(
            current,
            byteui::theme::color::current().card,
        ));
        cards = cards.push(row_btn);
    }
    if filtered.len() > visible {
        let more_button = icons::icon_button_entry(
            icons::IconKind::Ellipsis,
            byteui::theme::icon_size::row(),
            false,
            false,
            app.hover_progress(ConvHover::More),
            false,
            byteui::theme::geometry::tab_button_size(),
            true,
            app.wrap(Message::ListMore),
            |hovered| app.wrap(Message::Hover(ConvHover::More, hovered)),
            t("more", "More", locale),
        );
        cards = cards.push(
            container(more_button)
                .width(Length::Fill)
                .align_x(iced_widget::core::alignment::Horizontal::Center),
        );
    }
    content = content.push(
        Scrollable::new(cards)
            .width(Length::Fill)
            .height(Length::Fill)
            .direction(scrollable::Direction::Vertical(
                byteui::interaction::scrollbar::scrollbar(),
            ))
            .style(|_t, _s| byteui::interaction::scrollbar::scrollbar_style()),
    );
    content = content.push(footer_bar(ws_state, &agents_present, app));

    let panel = container(content.padding(region.padding))
        .width(width)
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: outer,
            ..container::Style::default()
        });

    iced_widget::stack![panel, agent_picker_view(ws_state, &agents_present, app)]
        .width(width)
        .height(Length::Fill)
        .into()
}

use crate::snapshot::{ReviewPaneState, ReviewSource, review_pane_state};
pub fn review_pane<'a, M: Clone + 'a>(
    state: &'a WorkspaceState,
    app: &'a impl PanelDisplay<M>,
    caps: ConversationsCapabilities,
    width: Length,
    outer: Border,
) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer> {
    let region = app.pane_style(ConversationsPane::Detail);
    let locale = app.locale();
    // 内容侧标题栏:左侧名字信息面板(会话/agent 态) + 右侧"生成总结"按钮
    // (会话详情手动生成/重试,spec 2026-09-26 第 8 节)+ "收起/展开列表列"
    // 按钮(与 Todo/Database/SSH/Agent 内容侧统一)。
    let collapse_button = app.list_collapse_button(
        t("panel.collapse_list", "Collapse list", locale),
        t("panel.expand_list", "Expand list", locale),
    );
    let actions: Element<'_, M, iced_widget::Theme, iced_renderer::Renderer> =
        match state.review.as_ref().map(|rv| (&rv.source, rv.agent)) {
            Some((ReviewSource::Conversation(cid), agent)) if caps.summaries => {
                let generating = state.summary_generating(cid);
                let generate = button(
                    text(if generating {
                        t("generating", "Generating…", locale)
                    } else {
                        t("generate_summary", "Generate Summary", locale)
                    })
                    .size(byteui::theme::font::label())
                    .color(byteui::theme::color::current().cream),
                )
                .padding([4, 8]);
                let generate = if generating {
                    generate
                } else {
                    generate.on_press(app.wrap(Message::SummaryGenerate(cid.clone(), agent)))
                };
                row![generate, collapse_button].spacing(6).into()
            }
            _ => collapse_button,
        };
    let header = container(
        row![app.panel_head(
            IconKind::BotMessageSquare,
            t("pane_title", "Conversations", locale),
            Some(actions),
        )]
        .width(Length::Fill),
    )
    .padding(region.header_padding);
    let mut content = column![header].spacing(region.gap);

    if let Some(error) = state.review.as_ref().and_then(|rv| match &rv.source {
        ReviewSource::Conversation(cid) => state.summary_error(cid),
    }) {
        content = content.push(
            text(crate::i18n::render(error, locale))
                .size(byteui::theme::font::body())
                .color(byteui::theme::color::current().red),
        );
    }

    match review_pane_state(state.review.as_ref()) {
        ReviewPaneState::Empty => {
            content = content.push(
                container(lh(region.line_height,text(t(
                    "no_review_content",
                    "No review content—click a conversation in the list on the left to start reviewing",
                    locale,
                ))
                    .size(byteui::theme::font::subtitle())
                    .color(byteui::theme::color::current().dim)))
                .width(Length::Fill)
                .height(Length::Fill),
            );
        }
        ReviewPaneState::Loading => {
            // 与 Empty 同结构(独占内容区、不渲染 Scrollable),只是文案换成
            // "加载中",让 webview 隐藏期间这块区域有意义的提示而不是空的可
            // 滚动区域(同用量面板"统计中…"分支的写法,视觉基调统一)。
            content = content.push(
                container(
                    iced_widget::column![
                        byteui::feedback::math_curve::view_animated::<M>(
                            byteui::feedback::math_curve::Curve::RoseThree,
                            64.0,
                        ),
                        iced_widget::text(t("common.loading", "Loading…", locale,))
                            .size(byteui::theme::font::body())
                            .color(byteui::theme::color::current().dim),
                    ]
                    .spacing(12)
                    .align_x(iced_widget::core::alignment::Horizontal::Center),
                )
                .center_x(Length::Fill)
                .center_y(Length::Fill),
            );
        }
        ReviewPaneState::Ready => {
            let body = crate::detail::review_fallback(column![].spacing(region.gap), state, app);
            content = content.push(
                Scrollable::new(body)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .direction(scrollable::Direction::Vertical(
                        byteui::interaction::scrollbar::scrollbar(),
                    ))
                    .style(|_t, _s| byteui::interaction::scrollbar::scrollbar_style()),
            );
        }
    }

    container(content.padding(region.padding))
        .width(width)
        .height(Length::Fill)
        .style(move |_theme: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: outer,
            ..container::Style::default()
        })
        .into()
}

// Match the original host's free-flowing text wrapping for long paths/titles.
pub(crate) fn text<'a>(
    content: impl iced_widget::core::text::IntoFragment<'a>,
) -> iced_widget::Text<'a, iced_widget::Theme, iced_renderer::Renderer> {
    iced_widget::Text::new(content).wrapping(iced_widget::core::text::Wrapping::WordOrGlyph)
}
