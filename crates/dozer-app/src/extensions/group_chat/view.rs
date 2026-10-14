//! 群聊面板原生部分:右侧群列表列 + 左侧聊天详情原生壳(webview 盖在其上)。
//! 群列表(选择群 / 新建群 / 删除群)完全原生;成员条、消息流、输入框与成员/
//! 转待办对话框在 `dozer://group-chat-content` webview 里
//! (见 `web/group-chat-content/` 与 `protocol.rs`)。原生壳在 webview 加载失败时
//! 显示失败原因与"重试"。

use super::{Message, WorkspaceState};
use crate::chrome::homespace::home_panel_head;
use byteui::interaction::icons;
use dozer_model::GroupInfo;
use iced_widget::core::{Border, Element, Length};
use iced_widget::{button, column, container, row, text};
/// 原生壳发出的消息:只有"重试"。
#[derive(Debug, Clone)]
pub enum ShellMessage {
    Retry,
}

/// 原生群列表"新建群聊"内联输入框的 `widget::Id`(供焦点/右键菜单识别)。
pub fn new_group_topic_field_id() -> iced_widget::core::widget::Id {
    iced_widget::core::widget::Id::new("group-chat-new-topic")
}

/// 右侧群列表列:群标题卡片列表(当前选中高亮)+ 底部"新建群聊"内联输入。
/// 每行右侧有删除入口;处于删除确认态的群显示"删除/取消"两个按钮。
pub fn list_pane<'a>(
    ws_state: &'a WorkspaceState,
    project_id: i64,
    locale: &'a str,
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let region = crate::theme::region::group_chat_list_pane();
    let mut content = column![home_panel_head(
        icons::IconKind::SquareSparkles,
        super::t("head.list", "Group chat", locale)
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

    content = content.push(new_group_row(ws_state, project_id, locale));

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

/// 单行群卡片:选中高亮;删除确认态下换成"删除/取消"。
fn group_row<'a>(
    group: &'a GroupInfo,
    ws_state: &'a WorkspaceState,
    project_id: i64,
    locale: &'a str,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let c = byteui::theme::color::current();
    if ws_state.delete_confirm() == Some(group.id) {
        return row![
            text(super::t_args(
                "delete.confirm",
                "Delete \"{topic}\"?",
                locale,
                &[("topic", &group.topic)]
            ))
            .size(byteui::theme::font::body())
            .color(c.cream)
            .width(Length::Fill),
            button(
                text(super::t("delete.button", "Delete", locale))
                    .size(byteui::theme::font::caption_sm())
            )
            .on_press(Message::DeleteConfirmed(project_id))
            .padding([4, 8])
            .style(delete_confirm_button_style()),
            button(
                text(super::t("cancel.button", "Cancel", locale))
                    .size(byteui::theme::font::caption_sm())
            )
            .on_press(Message::DeleteCancelled(project_id))
            .padding([4, 8])
            .style(cancel_button_style()),
        ]
        .spacing(6)
        .align_y(iced_widget::core::Alignment::Center)
        .into();
    }

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

    let main = button(
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
    .on_press(Message::SelectRequested(project_id))
    .width(Length::Fill)
    .padding(10)
    .style(byteui::interaction::cards::button_card(current, c.card));

    let del = icons::icon_button_entry(
        icons::IconKind::Trash,
        byteui::theme::icon_size::row(),
        false,
        false,
        0.0,
        false,
        byteui::theme::geometry::tab_button_size(),
        true,
        Message::DeleteRequested(project_id, group.id),
        move |_h| Message::Noop(project_id),
        super::t("delete.tooltip", "Delete group chat", locale),
    );

    row![main, del]
        .spacing(4)
        .align_y(iced_widget::core::Alignment::Center)
        .into()
}

/// 底部"新建群聊"内联输入:placeholder + 回车/＋ 提交。
fn new_group_row<'a>(
    ws_state: &'a WorkspaceState,
    project_id: i64,
    locale: &'a str,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let input = byteui::form::input_text::view(
        &super::t("new_group.placeholder", "New group topic…", locale),
        ws_state.new_group_topic(),
        false,
        Some(new_group_topic_field_id()),
        false,
        Some(Message::NewGroupSubmit(project_id)),
        false,
        move |s| Message::NewGroupDraftChanged(project_id, s),
    );
    let add = icons::icon_button_entry(
        icons::IconKind::Plus,
        byteui::theme::icon_size::row(),
        false,
        false,
        0.0,
        false,
        byteui::theme::geometry::tab_button_size(),
        true,
        Message::NewGroupSubmit(project_id),
        move |_h| Message::Noop(project_id),
        super::t("new_group.tooltip", "New group chat", locale),
    );
    row![input, add]
        .spacing(6)
        .align_y(iced_widget::core::Alignment::Center)
        .into()
}

fn delete_confirm_button_style()
-> impl Fn(&iced_widget::Theme, button::Status) -> button::Style + 'static {
    |_t, _s| button::Style {
        background: Some(byteui::theme::color::current().red.into()),
        text_color: byteui::theme::color::current().bg,
        border: Border::default(),
        ..button::Style::default()
    }
}

fn cancel_button_style() -> impl Fn(&iced_widget::Theme, button::Status) -> button::Style + 'static
{
    |_t, _s| button::Style {
        background: None,
        text_color: byteui::theme::color::current().dim,
        border: Border::default(),
        ..button::Style::default()
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_group_topic_field_id_is_stable() {
        assert_eq!(
            new_group_topic_field_id(),
            iced_widget::core::widget::Id::new("group-chat-new-topic")
        );
    }
}
