//! 统一的"卡片"视觉样式：把 Agent 卡片的"一般 / hover / 选中"三态抽象成
//! 单一来源,供全应用各类列表卡(Agent / 项目 / 最近 / todo 任务 / git
//! commit / 主机 / 对话)复用,确保"选中=金边、hover=金边+背景、一般=描边"
//! 的观感一致。
//!
//! 三态语义(与 Agent 卡片对齐):
//! - 一般态:无背景,`BORDER` 描边;
//! - hover:填充 `hover_bg`(默认 `CARD`),`GOLD` 描边;
//! - 选中:无背景,`GOLD` 描边(跟"一般态"的区别只在边框色)。
//!
//! 按钮型卡片用 `button_card`(靠 `button::Status` 自动拿 hover);容器型卡片
//! 用 `container_card`,由调用方用 `HoverId` + `app.hover_progress` 提供
//! `hovered`(非按钮卡没有 `button::Status`,得自己接悬停机制)。
use iced_widget::core::Background;
use iced_widget::core::{Border, Color};
use iced_widget::{button, container};

use crate::theme::color;

/// 卡片统一圆角(与 Agent 卡片对齐)。
pub const CARD_RADIUS: f32 = 8.0;

/// 三态统一的边框色:选中或 hover → 金;否则 `BORDER`。
pub fn card_border_color(selected: bool, hovered: bool) -> Color {
    if selected || hovered {
        color::GOLD
    } else {
        color::BORDER
    }
}

/// 三态统一的背景:仅 hover 时填充 `hover_bg`(默认 `CARD`);选中/一般态不填,
/// 与 Agent 卡片"选中不填背景、hover 才填背景"的语义一致。
pub fn card_background(hovered: bool, hover_bg: Color) -> Option<Background> {
    hovered.then_some(hover_bg.into())
}

/// 按钮型卡片样式闭包:靠 `button::Status::Hovered` 自动拿 hover,无需调用方
/// 自己接 `HoverId`。选中态由 `selected` 传入。
pub fn button_card(
    selected: bool,
    hover_bg: Color,
) -> impl Fn(&iced_widget::Theme, button::Status) -> button::Style {
    move |_t, s| {
        let hovered = matches!(s, button::Status::Hovered);
        button::Style {
            background: card_background(hovered, hover_bg),
            border: Border {
                color: card_border_color(selected, hovered),
                width: 1.0,
                radius: CARD_RADIUS.into(),
            },
            text_color: color::CREAM,
            ..button::Style::default()
        }
    }
}

/// 容器型卡片样式:调用方负责用 `HoverId` + `app.hover_progress` 提供
/// `hovered`(非按钮卡没有 `button::Status`)。选中态由 `selected` 传入。
pub fn container_card(selected: bool, hovered: bool, hover_bg: Color) -> container::Style {
    container::Style {
        background: card_background(hovered, hover_bg),
        border: Border {
            color: card_border_color(selected, hovered),
            width: 1.0,
            radius: CARD_RADIUS.into(),
        },
        ..container::Style::default()
    }
}
