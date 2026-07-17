// crates/dozer-app/src/term_view.rs
//! 终端渲染：把 `TerminalModel::visible_lines()` 快照画成逐行 `rich_text`
//! （P1c T5）。等宽字体 `Font::MONOSPACE`，13px 字号，1.4 倍行高；同一行内
//! 连续同风格（fg/bg/bold）的格子合并成一个 span，减少 span 数量。
//!
//! 光标格反色：`focused` 时实心 CREAM 底 + TERM_BG 字（iced 0.14 的
//! `text::Span` 支持 `background`/`border`，见 `iced_core::text::Span`，
//! 不需要退化成"实心块字符"画法）；未聚焦时透明底 + CREAM 描边
//! （`Span::border`）。
//!
//! T4 交接注意：`TerminalModel::cursor()` 把 `Term` 内部 `Line`（`i32`，
//! 可能为负）直接 cast 成 `usize` 作为 row，若真遇到负值会环绕成一个巨大
//! 数字。这里不额外做 `usize` 层面的裁剪——因为下面按 `row_idx == cursor_row`
//! 逐行比对，`row_idx` 永远在 `0..lines.len()` 内，环绕出的巨大值不可能命中
//! 任何一行，因此“光标只在可视行内绘制”这条防御是结构性自动满足的，无需
//! 额外 `if row_idx < len` 判断（该判断已经隐含在 for 循环边界里）。
//!
//! `grid_size` 已在 T6 接线：`main.rs` 用 pane 像素尺寸算出的 `(cols,
//! rows)` 驱动 `TerminalModel::resize`，不再是固定 80x24。
use crate::term_model::{Cell, TerminalModel};
use crate::theme;
use crate::workspace::Message;
use iced_widget::core::font::Weight;
use iced_widget::core::text::Span;
use iced_widget::core::{Border, Color, Element, Font, Length};
use iced_widget::{column, container, rich_text};

/// 字号（逻辑像素）。
const FONT_SIZE: f32 = 13.0;
/// 等宽字体单元格宽度 ≈ 0.6em。
const CELL_WIDTH: f32 = FONT_SIZE * 0.6;
/// 行高倍数（相对字号）。
const LINE_HEIGHT_FACTOR: f32 = 1.4;
/// 行高（逻辑像素），供 `grid_size` 换算用。
const LINE_HEIGHT_PX: f32 = FONT_SIZE * LINE_HEIGHT_FACTOR;

/// 终端 pane 的像素尺寸 → 网格尺寸 `(cols, rows)`，向下取整（不足一格的
/// 余量丢弃，避免半个字符溢出边界）。
pub fn grid_size(width_px: f32, height_px: f32) -> (usize, usize) {
    let cols = (width_px / CELL_WIDTH).floor().max(0.0) as usize;
    let rows = (height_px / LINE_HEIGHT_PX).floor().max(0.0) as usize;
    (cols, rows)
}

fn rgb(c: (u8, u8, u8)) -> Color {
    Color::from_rgb8(c.0, c.1, c.2)
}

fn cell_font(bold: bool) -> Font {
    if bold {
        Font {
            weight: Weight::Bold,
            ..Font::MONOSPACE
        }
    } else {
        Font::MONOSPACE
    }
}

/// 光标格反色 span。
fn cursor_span(cell: &Cell, focused: bool) -> Span<'static, (), Font> {
    let span = Span::new(cell.ch.to_string()).font(cell_font(cell.bold));
    if focused {
        span.color(theme::TERM_BG).background(theme::CREAM)
    } else {
        span.color(rgb(cell.fg)).border(Border {
            color: theme::CREAM,
            width: 1.0,
            radius: 0.0.into(),
        })
    }
}

fn plain_span(
    text: String,
    fg: (u8, u8, u8),
    bg: Option<(u8, u8, u8)>,
    bold: bool,
) -> Span<'static, (), Font> {
    Span::new(text)
        .font(cell_font(bold))
        .color(rgb(fg))
        .background_maybe(bg.map(rgb))
}

/// 单行 cell 序列 → span 序列：光标格单独切出反色；其余按
/// `(fg, bg, bold)` 相同与否合并成连续 span，减少渲染层的 span 数量。
fn line_spans(
    row: &[Cell],
    cursor_col: Option<usize>,
    focused: bool,
) -> Vec<Span<'static, (), Font>> {
    let mut spans = Vec::new();
    let mut idx = 0;
    while idx < row.len() {
        if cursor_col == Some(idx) {
            spans.push(cursor_span(&row[idx], focused));
            idx += 1;
            continue;
        }
        let style = (row[idx].fg, row[idx].bg, row[idx].bold);
        let mut text = String::new();
        while idx < row.len()
            && cursor_col != Some(idx)
            && (row[idx].fg, row[idx].bg, row[idx].bold) == style
        {
            text.push(row[idx].ch);
            idx += 1;
        }
        spans.push(plain_span(text, style.0, style.1, style.2));
    }
    if spans.is_empty() {
        // 空行占位，避免零 span 导致行高塌陷。
        spans.push(Span::new(" ".to_string()));
    }
    spans
}

/// 渲染整块终端网格：逐行 `rich_text`，堆叠进一个 `column`。
pub fn view(
    model: &TerminalModel,
    focused: bool,
) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let (cursor_col, cursor_row) = model.cursor();
    let lines = model.visible_lines();

    let rows: Vec<Element<'_, Message, iced_widget::Theme, iced_widget::Renderer>> = lines
        .iter()
        .enumerate()
        .map(|(row_idx, row)| {
            let is_cursor_row = row_idx == cursor_row && cursor_col < row.len();
            let spans = line_spans(row, is_cursor_row.then_some(cursor_col), focused);
            rich_text(spans)
                .font(Font::MONOSPACE)
                .size(FONT_SIZE)
                .line_height(LINE_HEIGHT_FACTOR)
                .into()
        })
        .collect();

    container(column(rows))
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_size_from_pixels() {
        // 字号 13px 等宽：单元格宽 ≈ 7.8px（0.6em），行高 ≈ 18.2px（1.4）
        let (cols, rows) = grid_size(780.0, 546.0);
        assert!((95..=105).contains(&cols), "cols={cols}");
        assert!((28..=32).contains(&rows), "rows={rows}");
    }
}
