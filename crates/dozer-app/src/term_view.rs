// crates/dozer-app/src/term_view.rs
//! 终端渲染：canvas 逐格定位绘制（T7 验收反馈修复，取代 rich_text 流式排版）。
//!
//! ## 为什么不是 rich_text
//! rich_text 是流式排版：span 依次排开，后一个字形的 x 取决于前一个字形的
//! advance。实测（macOS，Menlo 13px + Advanced shaping）CJK 回退字形的
//! advance 是 1.661 个等宽格，不是终端网格假设的 2 格——只要行内出现中文，
//! 后续所有列全部漂移，`ls` 多列、vim/top 全屏界面必乱。终端正确性要求
//! **每个格子画在 `col × 格宽` 的绝对位置上**，这只有 canvas 能做到。
//!
//! ## 绘制模型
//! `TerminalModel::visible_lines()` 快照 → 每行切成 [`Run`]（见
//! [`layout_runs`]，headless 全测）→ 每个 run 一次 `fill_text`，定位在
//! `(col * CELL_WIDTH, row * LINE_HEIGHT_PX)`：
//! - ASCII 连续同风格格子合并成一个 run——等宽字体保证 run 内部对齐；
//! - 宽字符（CJK）独立成 run、占 2 格绘制盒——字形 advance 与网格假设的
//!   偏差被"每个宽字符重新定位"吞掉，不会累积；
//! - 宽字符 spacer 格与无背景空白格不产生字形（空白格同时切断 run，
//!   保证 run 内文本列数与网格列数一致）。
//!
//! 光标最后画：先补一块实心格（focused：CREAM 底 + TERM_BG 字；未聚焦：
//! CREAM 描边），覆盖在 run 字形之上，天然处理"光标落在任意 run 中间"。
use crate::term_model::{Cell, TerminalModel};
use crate::theme;
use crate::workspace::Message;
use iced_widget::canvas::{self, Canvas};
use iced_widget::core::font::Weight;
use iced_widget::core::text::LineHeight;
use iced_widget::core::{Color, Element, Font, Length, Pixels, Point, Rectangle, Size};

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

/// 一段可一次 `fill_text` 画完的连续格子：起始列、占据列数、文本与风格。
#[derive(Debug, Clone, PartialEq)]
struct Run {
    col: usize,
    cells: usize,
    text: String,
    fg: (u8, u8, u8),
    bg: Option<(u8, u8, u8)>,
    bold: bool,
}

/// 单行 cell 序列 → 绘制 run 序列。切分规则见模块注释。
fn layout_runs(row: &[Cell]) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    let mut col = 0;
    while col < row.len() {
        let cell = &row[col];
        // spacer 由宽字符本体的 2 格绘制盒覆盖；无背景空白格画不出任何
        // 东西——两者都跳过（后者顺带切断了 run 的连续性）。
        if cell.spacer || (cell.ch == ' ' && cell.bg.is_none()) {
            col += 1;
            continue;
        }
        if cell.wide {
            runs.push(Run {
                col,
                cells: 2,
                text: cell.ch.to_string(),
                fg: cell.fg,
                bg: cell.bg,
                bold: cell.bold,
            });
            col += 2; // 本体 + spacer
            continue;
        }
        let style = (cell.fg, cell.bg, cell.bold);
        let start = col;
        let mut text = String::new();
        while col < row.len() {
            let c = &row[col];
            let blank = c.ch == ' ' && c.bg.is_none();
            if c.wide || c.spacer || blank || (c.fg, c.bg, c.bold) != style {
                break;
            }
            text.push(c.ch);
            col += 1;
        }
        runs.push(Run {
            col: start,
            cells: col - start,
            text,
            fg: style.0,
            bg: style.1,
            bold: style.2,
        });
    }
    runs
}

/// canvas 绘制程序：持有当前 tab 的 `TerminalModel` 快照引用逐帧重画。
/// 网格规模（百列 × 数十行）下 run 数量有限，不做 `canvas::Cache`——
/// 终端输出本来就是高频失效场景，缓存收益低。
struct TermCanvas<'a> {
    model: &'a TerminalModel,
    focused: bool,
}

impl canvas::Program<Message, iced_widget::Theme, iced_widget::Renderer> for TermCanvas<'_> {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced_widget::Renderer,
        _theme: &iced_widget::Theme,
        bounds: Rectangle,
        _cursor: iced_widget::core::mouse::Cursor,
    ) -> Vec<canvas::Geometry<iced_widget::Renderer>> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let lines = self.model.visible_lines();
        let (cursor_col, cursor_row) = self.model.cursor();

        for (row_idx, row) in lines.iter().enumerate() {
            let y = row_idx as f32 * LINE_HEIGHT_PX;
            for run in layout_runs(row) {
                let x = run.col as f32 * CELL_WIDTH;
                if let Some(bg) = run.bg {
                    frame.fill_rectangle(
                        Point::new(x, y),
                        Size::new(run.cells as f32 * CELL_WIDTH, LINE_HEIGHT_PX),
                        rgb(bg),
                    );
                }
                frame.fill_text(canvas::Text {
                    content: run.text,
                    position: Point::new(x, y),
                    color: rgb(run.fg),
                    size: Pixels(FONT_SIZE),
                    line_height: LineHeight::Absolute(Pixels(LINE_HEIGHT_PX)),
                    font: cell_font(run.bold),
                    ..canvas::Text::default()
                });
            }
        }

        // 光标覆盖层（画在文本之上）。`cursor()` 直接取自 `Term` 网格，
        // 永远在可视范围内；防御性判界只为杜绝 cast 环绕的极端值。
        if let Some(cell) = lines.get(cursor_row).and_then(|r| r.get(cursor_col)) {
            let x = cursor_col as f32 * CELL_WIDTH;
            let y = cursor_row as f32 * LINE_HEIGHT_PX;
            let box_w = if cell.wide { 2.0 } else { 1.0 } * CELL_WIDTH;
            if self.focused {
                frame.fill_rectangle(
                    Point::new(x, y),
                    Size::new(box_w, LINE_HEIGHT_PX),
                    theme::CREAM,
                );
                if cell.ch != ' ' {
                    frame.fill_text(canvas::Text {
                        content: cell.ch.to_string(),
                        position: Point::new(x, y),
                        color: theme::TERM_BG,
                        size: Pixels(FONT_SIZE),
                        line_height: LineHeight::Absolute(Pixels(LINE_HEIGHT_PX)),
                        font: cell_font(cell.bold),
                        ..canvas::Text::default()
                    });
                }
            } else {
                frame.stroke(
                    &canvas::Path::rectangle(Point::new(x, y), Size::new(box_w, LINE_HEIGHT_PX)),
                    canvas::Stroke::default()
                        .with_color(theme::CREAM)
                        .with_width(1.0),
                );
            }
        }

        vec![frame.into_geometry()]
    }
}

/// 渲染整块终端网格。
pub fn view(
    model: &TerminalModel,
    focused: bool,
) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    Canvas::new(TermCanvas { model, focused })
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row_of(input: &[u8], cols: u16) -> Vec<Cell> {
        let mut t = TerminalModel::new(cols, 4);
        t.feed(input);
        t.visible_lines().remove(0)
    }

    #[test]
    fn grid_size_from_pixels() {
        // 字号 13px 等宽：单元格宽 ≈ 7.8px（0.6em），行高 ≈ 18.2px（1.4）
        let (cols, rows) = grid_size(780.0, 546.0);
        assert!((95..=105).contains(&cols), "cols={cols}");
        assert!((28..=32).contains(&rows), "rows={rows}");
    }

    #[test]
    fn ascii_same_style_merges_into_one_run() {
        let runs = layout_runs(&row_of(b"hello", 40));
        assert_eq!(runs.len(), 1);
        assert_eq!(
            (runs[0].col, runs[0].cells, runs[0].text.as_str()),
            (0, 5, "hello")
        );
    }

    #[test]
    fn wide_char_gets_own_two_cell_run_and_spacer_is_skipped() {
        let runs = layout_runs(&row_of("ab你cd".as_bytes(), 40));
        let shape: Vec<_> = runs
            .iter()
            .map(|r| (r.col, r.cells, r.text.as_str()))
            .collect();
        assert_eq!(shape, vec![(0, 2, "ab"), (2, 2, "你"), (4, 2, "cd")]);
    }

    #[test]
    fn style_change_splits_runs() {
        let runs = layout_runs(&row_of(b"a\x1b[31mb", 40));
        assert_eq!(runs.len(), 2);
        assert_eq!((runs[0].col, runs[0].text.as_str()), (0, "a"));
        assert_eq!((runs[1].col, runs[1].text.as_str()), (1, "b"));
        assert_eq!(runs[1].fg, (0xFF, 0x6E, 0x6E));
    }

    #[test]
    fn bare_blank_cells_are_skipped_and_break_runs() {
        let runs = layout_runs(&row_of(b"a  b", 40));
        let shape: Vec<_> = runs.iter().map(|r| (r.col, r.text.as_str())).collect();
        assert_eq!(shape, vec![(0, "a"), (3, "b")]);
    }

    #[test]
    fn blank_cells_with_background_are_kept() {
        let runs = layout_runs(&row_of(b"\x1b[41m x", 40));
        assert_eq!(runs.len(), 1);
        assert_eq!((runs[0].col, runs[0].text.as_str()), (0, " x"));
        assert!(runs[0].bg.is_some());
    }
}
