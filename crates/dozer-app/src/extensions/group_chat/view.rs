//! 群聊面板原生壳:webview 盖在这块 `container` 之上,它只负责面板背景/边框;
//! `failed` 为 `Some` 时 webview 不挂载,这里显示失败原因与"重试"。内容渲染
//! 全部在 `dozer://group-chat-content` webview 里(见 `web/group-chat-content/`)。

use iced_widget::core::{Border, Element, Length};
use iced_widget::{button, column, container, text};

/// 原生壳发出的消息:只有"重试加载"。
#[derive(Debug, Clone)]
pub enum ShellMessage {
    Retry,
}

pub fn content_pane(
    failed: Option<&str>,
    outer: Border,
) -> Element<'static, ShellMessage, iced_widget::Theme, iced_renderer::Renderer> {
    let tokens = byteui::theme::color::current();
    let body: Element<'static, ShellMessage, iced_widget::Theme, iced_renderer::Renderer> =
        match failed {
            Some(reason) => column![
                text("群聊页面加载失败").size(14).color(tokens.body),
                text(reason.to_string()).size(12).color(tokens.dim),
                button(text("重试").size(13)).on_press(ShellMessage::Retry),
            ]
            .spacing(10)
            .padding(16)
            .into(),
            None => column![].into(),
        };
    container(body)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(byteui::theme::color::current().panel.into()),
            border: outer,
            ..container::Style::default()
        })
        .into()
}
