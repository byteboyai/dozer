//! 终端视图(S6b 抽库后的薄适配层)。
//!
//! 模型与 iced canvas 控件都在共享库 `dozer-panel-term`;本文件只做两件事:
//! 用 Dozer 的字体/字号/行高构造 [`TermStyle`],把库事件映射回内核的
//! [`Message`]。对外签名(`view` / `grid_size`)与抽库前一致,所以
//! `app.rs` / `app/view.rs` / `terminal.rs` 的调用点不需要改动。

use super::term_model::TerminalModel;
use super::terminal::TermTarget;
use crate::app::Message;
use crate::assets::fonts;
use crate::theme::terminal_font;
use dozer_panel_term::style::TermStyle;
use dozer_panel_term::view::{self as lib_view, TermEvent};
use iced_widget::core::Element;

/// Dozer 的终端样式:等宽字体、设计基准字号与行高倍数。
fn style() -> &'static TermStyle {
    static STYLE: std::sync::OnceLock<TermStyle> = std::sync::OnceLock::new();
    STYLE.get_or_init(|| TermStyle {
        font: fonts::code_font(),
        font_size: terminal_font::size(),
        line_height_factor: terminal_font::line_height_factor(),
    })
}

/// 当前终端样式的一份拷贝。字号/行高来自编译期内嵌的 `terminal.json`
/// (运行期不变);全局 UI scale 由 `TermStyle` 的方法在每次换算时读取。
pub fn terminal_style() -> TermStyle {
    style().clone()
}

/// 终端样式的 `'static` 引用(视图适配层用,避免每次构造临时值)。
pub(crate) fn terminal_style_ref() -> &'static TermStyle {
    style()
}

/// 库事件 → 内核消息。载荷一一对应,不丢字段。
pub(crate) fn map_event(target: TermTarget, event: TermEvent) -> Message {
    match event {
        TermEvent::Input(bytes) => Message::TermInput(target, bytes),
        TermEvent::Scroll(lines) => Message::TermScroll(target, lines),
        TermEvent::SelStart {
            col,
            row,
            right_half,
        } => Message::TermSelStart {
            target,
            col,
            row,
            right: right_half,
        },
        TermEvent::SelUpdate {
            col,
            row,
            right_half,
        } => Message::TermSelUpdate {
            target,
            col,
            row,
            right: right_half,
        },
    }
}

/// 渲染整块终端网格。`preedit` 只在 `focused` 的那个 pane 传 `Some`。
pub fn view<'a>(
    model: &'a TerminalModel,
    focused: bool,
    target: TermTarget,
    preedit: Option<&'a str>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    lib_view::view(model, focused, target, preedit, style(), map_event)
}

/// 终端 pane 的像素尺寸 → 网格尺寸 `(cols, rows)`。签名与抽库前一致。
pub fn grid_size(width_px: f32, height_px: f32) -> (usize, usize) {
    terminal_style().grid_size(width_px, height_px)
}
