//! 代码健康度面板原生部分:右侧分类导航 + 内容列原生壳(webview 加载失败时
//! 的占位)。内容渲染全部在 `dozer://codehealth-content` webview 里
//! (见 `web/codehealth-content/` 与 `protocol.rs`)。

use super::{CodeHealthCategory, Message, WorkspaceState};
use byteui::interaction::icons;
use iced_widget::core::{Border, Color, Element, Length};
use iced_widget::{button, column, container, row, space, text};

fn category_icon(category: CodeHealthCategory) -> icons::IconKind {
    match category {
        CodeHealthCategory::Overview => icons::IconKind::BarChart3,
        CodeHealthCategory::Structure => icons::IconKind::FileCode,
        CodeHealthCategory::UiConsistency => icons::IconKind::LayoutList,
        CodeHealthCategory::ScanScope => icons::IconKind::Search,
    }
}

fn category_button(
    category: CodeHealthCategory,
    current: CodeHealthCategory,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let active = category == current;
    let c = byteui::theme::color::current();
    let fg = if active { c.cream } else { c.dim };
    let icon_color = if active { c.gold } else { c.dim };
    button(
        row![
            icons::view(
                category_icon(category),
                byteui::theme::icon_size::row(),
                icon_color
            ),
            text(category.label())
                .size(byteui::theme::font::body())
                .color(fg),
            space::Space::new()
                .width(Length::Fill)
                .height(Length::Shrink),
        ]
        .spacing(8)
        .align_y(iced_widget::core::Alignment::Center),
    )
    .on_press(Message::CategorySet(category))
    .width(Length::Fill)
    .padding([8, 10])
    .style(move |_t: &iced_widget::Theme, _s| button::Style {
        background: if active {
            Some(byteui::theme::color::current().card.into())
        } else {
            None
        },
        text_color: fg,
        border: Border {
            color: if active {
                byteui::theme::color::current().gold
            } else {
                Color::TRANSPARENT
            },
            width: if active { 1.0 } else { 0.0 },
            radius: 6.0.into(),
        },
        ..button::Style::default()
    })
    .into()
}

pub fn list_pane(
    ws_state: &WorkspaceState,
    width: Length,
    outer: Border,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let header = container(crate::chrome::homespace::home_panel_head(
        icons::IconKind::SquareActivity,
        "代码健康度",
    ))
    .padding(iced_widget::core::Padding {
        top: 12.0,
        right: 12.0,
        bottom: 8.0,
        left: 12.0,
    });

    let current = ws_state.category();
    let mut nav = column![].spacing(4).padding(iced_widget::core::Padding {
        top: 0.0,
        right: 8.0,
        bottom: 12.0,
        left: 8.0,
    });
    for category in CodeHealthCategory::all() {
        nav = nav.push(category_button(category, current));
    }

    container(column![header, nav])
        .width(width)
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(byteui::theme::color::current().bg.into()),
            border: outer,
            ..container::Style::default()
        })
        .into()
}

/// 内容列原生壳:webview 盖在这块 `container` 之上(同 Files/Usage 现状),
/// 它只负责面板背景/边框;`failed` 为 `Some` 时 webview 不挂载,这里显示
/// 失败原因与"重试"。
pub fn content_pane(
    failed: Option<&str>,
    width: Length,
    outer: Border,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let tokens = byteui::theme::color::current();
    let body: Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> = match failed
    {
        Some(reason) => column![
            text("代码健康度页面加载失败").size(14).color(tokens.body),
            text(reason.to_string()).size(12).color(tokens.dim),
            button(text("重试").size(13)).on_press(Message::ContentRetry),
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
    fn category_labels_and_all_cover_four_categories() {
        assert_eq!(CodeHealthCategory::all().len(), 4);
        assert_eq!(CodeHealthCategory::Overview.label(), "总览");
        assert_eq!(CodeHealthCategory::Structure.label(), "结构复杂度");
        assert_eq!(CodeHealthCategory::UiConsistency.label(), "UI 一致性");
        assert_eq!(CodeHealthCategory::ScanScope.label(), "扫描范围");
    }
}
