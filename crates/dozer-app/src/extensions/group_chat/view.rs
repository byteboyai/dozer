//! 群聊面板原生部分:右侧群列表列 + 左侧聊天详情原生壳(webview 盖在其上)。
//! 群列表(选择群 / 新建群)完全原生;删除群走 webview 内容区菜单。成员条、
//! 消息流、输入框与成员/转待办对话框在 `dozer://group-chat-content` webview 里
//! (见 `web/group-chat-content/` 与 `protocol.rs`)。原生壳在 webview 加载失败时
//! 显示失败原因与"重试"。

use super::{Message, WorkspaceState};
use crate::chrome::homespace::home_panel_head_with_actions;
use crate::ui_text::text;
use byteui::interaction::icons;
use dozer_model::GroupInfo;
use iced_widget::core::{Border, Element, Length};
use iced_widget::{button, column, container};
/// 原生壳发出的消息:只有"重试"。
#[derive(Debug, Clone)]
pub enum ShellMessage {
    Retry,
}

/// 右侧群列表列:群标题卡片列表(当前选中高亮)+ 头部"新建群聊"弹窗。
/// 删除入口已移除,删除走 webview 内容区。
pub fn list_pane<'a>(
    ws_state: &'a WorkspaceState,
    project_id: i64,
    locale: &'a str,
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let region = crate::theme::region::group_chat_list_pane();
    let add_button = icons::icon_button_entry(
        icons::IconKind::SquarePlus,
        byteui::theme::icon_size::row(),
        ws_state.new_group_dialog_open(),
        false,
        0.0,
        false,
        byteui::theme::geometry::tab_button_size(),
        true,
        Message::NewGroupDialogToggle(project_id),
        move |_h| Message::Noop(project_id),
        super::t("new_group.tooltip", "New group chat", locale),
    );
    let mut content = column![home_panel_head_with_actions(
        icons::IconKind::SquareSparkles,
        super::t("head.list", "Group chat", locale),
        Some(add_button),
    )]
    .spacing(region.gap);

    if ws_state.groups().is_empty() {
        content = content.push(
            text(super::t("empty.no_groups", "No group chats yet", locale))
                .size(byteui::theme::font::body())
                .color(byteui::theme::color::current().dim),
        );
    } else {
        let mut cards = column![].spacing(region.gap);
        for group in ws_state.groups() {
            cards = cards.push(group_row(group, ws_state, project_id, locale));
        }
        content = content.push(cards);
    }

    container(content.padding(region.padding))
        .width(width)
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: outer,
            ..container::Style::default()
        })
        .into()
}

/// 单行群卡片:选中高亮;整行可点。
fn group_row<'a>(
    group: &'a GroupInfo,
    ws_state: &'a WorkspaceState,
    project_id: i64,
    locale: &'a str,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let c = byteui::theme::color::current();
    let current = ws_state.selected() == Some(group.id);
    let member_count = group.members.len();
    let sub = if member_count == 0 {
        super::t("members.none", "No members yet", locale)
    } else {
        super::t_args(
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
            iced_widget::container(
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
    // 项目 id 供消息路由,群 id 供 `select` 选中该行——不能只传项目 id。
    .on_press(Message::SelectRequested(project_id, group.id))
    .width(Length::Fill)
    .padding(10)
    .style(byteui::interaction::cards::button_card(current, c.card))
    .into()
}

pub fn content_pane(
    failed: Option<&dozer_model::LocalizedText>,
    locale: &str,
    width: Length,
    outer: Border,
) -> Element<'static, ShellMessage, iced_widget::Theme, iced_renderer::Renderer> {
    let tokens = byteui::theme::color::current();
    let body: Element<'static, ShellMessage, iced_widget::Theme, iced_renderer::Renderer> =
        match failed {
            Some(reason) => column![
                text(super::t(
                    "content.load_failed",
                    "Group chat page failed to load",
                    locale
                ))
                .size(14)
                .color(tokens.body),
                text(crate::i18n::host_render(reason, locale))
                    .size(12)
                    .color(tokens.dim),
                button(text(super::t("retry", "Retry", locale)).size(13))
                    .on_press(ShellMessage::Retry),
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
