//! 表格网格的虚拟化渲染(基于 `iced_widget::canvas`)。
//!
//! 只画落进当前可视窗口的单元格、列头、行号与网格线——单帧开销只与可视
//! 窗口大小相关,不随数据总量增长。交互只有滚轮滚动(`Action::Scroll`,按
//! 行列步进),滚动量转成整型行列步进后回写 `TabularView.scroll_*`,由
//! `TabularView::apply` 钳位。列头/行号冻结(不随数据区滚动),行高/列宽由
//! 当前字号与 `Sheet.col_widths`(等宽字符格)换算。
//!
//! 用 `canvas::Program` 而非手写 `Widget`:canvas 的 `Program::update` 会
//! 逐事件回调(含滚轮),`Frame::fill_text` 画文本,免去手写 `Tree`/`diff`/
//! `fill_text` 渲染器绑定那套样板。文本经 `Frame::fill_text` 恒画在矩形
//! 之上,正好满足「文字盖在单元格底色上」的层级。

use iced_widget::canvas::{self, Frame, Text as CanvasText};
use iced_widget::core::alignment;
use iced_widget::core::event::Event;
use iced_widget::core::mouse;
use iced_widget::core::text::{Alignment, LineHeight, Shaping};
use iced_widget::core::{Color, Element, Font, Length, Pixels, Point, Rectangle, Size};

use super::TabularView;

/// 网格交互动作。`Scroll` 的 `dx`/`dy` 是**整型行列步进**(正=向右/向下)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Scroll { dx: i32, dy: i32 },
    SelectSheet(usize),
}

/// 网格渲染几何(由当前字号/scale 派生,每帧现算)。
struct Metrics {
    font_size: f32,
    row_height: f32,
    char_width: f32,
    header_h: f32,
    gutter_w: f32,
}

impl Metrics {
    fn new() -> Self {
        let font_size = crate::theme::terminal_font::size() * byteui::theme::icon_size::scale();
        let row_height = font_size * ROW_HEIGHT_FACTOR;
        let char_width = font_size * CHAR_WIDTH_FACTOR;
        Self {
            font_size,
            row_height,
            char_width,
            header_h: row_height,
            gutter_w: char_width * 6.0,
        }
    }
}

/// 行高 = 字号 × 该因子(与终端行距同源的观感)。
const ROW_HEIGHT_FACTOR: f32 = 1.5;
/// 等宽字符推进宽度 = 字号 × 该因子(JetBrains Mono advance ≈ 0.6em)。
const CHAR_WIDTH_FACTOR: f32 = 0.6;

/// 一个只读的虚拟化表格网格。`view` 每帧读当前 scroll 与激活 sheet。
pub struct TabularGrid<'a> {
    view: &'a TabularView,
}

impl<'a> TabularGrid<'a> {
    pub fn new(view: &'a TabularView) -> Self {
        Self { view }
    }
}

fn column_letter(mut idx: usize) -> String {
    let mut s = String::new();
    idx += 1;
    while idx > 0 {
        let rem = (idx - 1) % 26;
        s.insert(0, (b'A' + rem as u8) as char);
        idx = (idx - 1) / 26;
    }
    s
}

/// 折叠换行并按「等宽字符格」(`unicode` 显示宽,中文占 2 格)截断到最多
/// `max_cells` 格,供单元格单行展示(超长省略)。按显示宽而非字符数截断,
/// 中文这类宽字符不会因按字符计数而溢出列宽。
fn truncate_to(s: &str, max_cells: usize) -> String {
    if max_cells == 0 {
        return String::new();
    }
    let mut width = 0usize;
    let mut out = String::new();
    for ch in s.chars() {
        let ch = if ch == '\n' || ch == '\r' { ' ' } else { ch };
        let w = unicode_width::UnicodeWidthChar::width(ch)
            .unwrap_or(0)
            .max(1);
        if width + w > max_cells {
            break;
        }
        width += w;
        out.push(ch);
    }
    out
}

fn cell_text(
    content: String,
    position: Point,
    max_width: f32,
    color: Color,
    m: &Metrics,
) -> CanvasText {
    CanvasText {
        content,
        position,
        max_width,
        color,
        size: Pixels(m.font_size),
        line_height: LineHeight::Relative(1.0),
        // 系统默认字体(非代码场景不用 JetBrains Mono,见 CLAUDE.md 字体统一
        // 裁决);`Shaping::Advanced` 做字体回退——中文等非 ASCII 字形回退到
        // 系统 CJK 字体。`Basic` 明确不做回退(见 iced 文档),会导致中文变
        // 方块/空白。
        font: Font::default(),
        align_x: Alignment::Left,
        align_y: alignment::Vertical::Top,
        shaping: Shaping::Advanced,
    }
}

impl<'a> canvas::Program<Action> for TabularGrid<'a> {
    type State = ();

    fn update(
        &self,
        _state: &mut Self::State,
        event: &Event,
        _bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Action>> {
        let Event::Mouse(mouse::Event::WheelScrolled { delta }) = event else {
            return None;
        };
        let m = Metrics::new();
        let (dx, dy): (i32, i32) = match delta {
            mouse::ScrollDelta::Lines { x, y } => {
                ((*x * 3.0).round() as i32, (*y * 3.0).round() as i32)
            }
            mouse::ScrollDelta::Pixels { x, y } => (
                (*x / (m.char_width * 8.0)).round() as i32,
                (*y / m.row_height).round() as i32,
            ),
        };
        if dx == 0 && dy == 0 {
            return None;
        }
        Some(canvas::Action::publish(Action::Scroll { dx, dy }))
    }

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced_renderer::Renderer,
        _theme: &iced_widget::Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let m = Metrics::new();
        let colors = byteui::theme::color::current();
        let mut frame = Frame::new(renderer, bounds.size());
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), colors.panel);
        // `view.rs` 只在 sheet 已加载时才装配这个 Canvas(未加载时改画
        // loading_hint),这里是防御性兜底,不应该真的走到。
        let Some(sheet) = self.view.active_sheet() else {
            return vec![frame.into_geometry()];
        };

        let data_w = (bounds.width - m.gutter_w).max(0.0);
        let data_h = (bounds.height - m.header_h).max(0.0);

        let col_w: Vec<f32> = sheet.col_widths.iter().map(|c| c * m.char_width).collect();

        // 可视行窗口(viewport-aware 钳位,滚到底显示最后一屏)。
        let rows_visible = ((data_h / m.row_height).ceil() as usize).max(1);
        let max_row = sheet.total_rows.saturating_sub(rows_visible);
        let scroll_row = self.view.scroll_row.min(max_row);
        let row_end = (scroll_row + rows_visible).min(sheet.total_rows);

        // 可视列窗口。
        let scroll_col = self.view.scroll_col.min(sheet.col_count.saturating_sub(1));
        let mut col_end = scroll_col;
        let mut x = m.gutter_w;
        for (idx, w) in col_w.iter().enumerate().skip(scroll_col) {
            if x - m.gutter_w >= data_w {
                break;
            }
            x += w;
            col_end = idx + 1;
        }

        // 1. 列头行 + 左上角转角(整体背景已在上面提前画过)。
        frame.fill_rectangle(
            Point::ORIGIN,
            Size::new(bounds.width, m.header_h),
            colors.tab_active_bg,
        );
        frame.fill_rectangle(
            Point::ORIGIN,
            Size::new(m.gutter_w, m.header_h),
            colors.card,
        );

        // 2. 列头字母。
        let mut hx = m.gutter_w;
        for (col, w) in col_w
            .iter()
            .enumerate()
            .skip(scroll_col)
            .take(col_end - scroll_col)
        {
            let w = *w;
            frame.fill_text(cell_text(
                column_letter(col),
                Point::new(hx + 4.0, (m.header_h - m.font_size) / 2.0),
                w - 8.0,
                colors.cream,
                &m,
            ));
            hx += w;
        }

        // 3. 行号 gutter 背景 + 行号。
        frame.fill_rectangle(
            Point::new(0.0, m.header_h),
            Size::new(m.gutter_w, data_h),
            colors.tab_active_bg,
        );
        for row in scroll_row..row_end {
            let y = m.header_h + (row - scroll_row) as f32 * m.row_height;
            let mut rt = cell_text(
                (row + 1).to_string(),
                Point::new(0.0, y + (m.row_height - m.font_size) / 2.0),
                m.gutter_w - 6.0,
                colors.dim,
                &m,
            );
            rt.align_x = Alignment::Right;
            frame.fill_text(rt);
        }

        // 4. 数据单元格 + 网格线。
        for row in scroll_row..row_end {
            let y = m.header_h + (row - scroll_row) as f32 * m.row_height;
            let cells = sheet.rows.get(row);
            let mut cx = m.gutter_w;
            for (col, w) in col_w
                .iter()
                .enumerate()
                .skip(scroll_col)
                .take(col_end - scroll_col)
            {
                let w = *w;
                let raw = cells
                    .and_then(|r| r.get(col))
                    .map(|s| s.as_str())
                    .unwrap_or("");
                // 超长/多行文本折叠成单行并截断到列宽,避免 `Word` 换行把单元格
                // 撑破(渲染层裁剪,不改底层数据)。
                let text = truncate_to(raw, (w / m.char_width).floor() as usize);
                if !text.is_empty() {
                    frame.fill_text(cell_text(
                        text,
                        Point::new(cx + 4.0, y + (m.row_height - m.font_size) / 2.0),
                        w - 8.0,
                        colors.body,
                        &m,
                    ));
                }
                // 右/下网格线。
                frame.fill_rectangle(
                    Point::new(cx + w - 1.0, y),
                    Size::new(1.0, m.row_height),
                    colors.border,
                );
                frame.fill_rectangle(
                    Point::new(cx, y + m.row_height - 1.0),
                    Size::new(w, 1.0),
                    colors.border,
                );
                cx += w;
            }
        }

        // 5. 列头与数据区之间的横线 + gutter 竖线。
        frame.fill_rectangle(
            Point::new(0.0, m.header_h),
            Size::new(bounds.width, 1.0),
            colors.border,
        );
        frame.fill_rectangle(
            Point::new(m.gutter_w, 0.0),
            Size::new(1.0, bounds.height),
            colors.border,
        );

        vec![frame.into_geometry()]
    }
}

/// 把网格包成 `Canvas` widget(`Message` 泛型由调用方 `.map` 收敛)。
pub fn view<'a>(
    view: &'a TabularView,
) -> Element<'a, Action, iced_widget::Theme, iced_renderer::Renderer> {
    iced_widget::canvas::Canvas::new(TabularGrid::new(view))
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_to_counts_cjk_by_display_width() {
        // 4 格预算:两个中文(各 2 格)放得下,第三个中文(超预算)被截掉。
        assert_eq!(truncate_to("你好世界", 4), "你好");
        // 混排:a(1) + 中(2) + b(1) = 4 格,正好放满。
        assert_eq!(truncate_to("a中b", 4), "a中b");
        // 换行折叠成空格。
        assert_eq!(truncate_to("a\nb", 4), "a b");
        // 0 格直接空。
        assert_eq!(truncate_to("abc", 0), "");
    }

    #[test]
    fn column_letter_is_excel_style() {
        assert_eq!(column_letter(0), "A");
        assert_eq!(column_letter(25), "Z");
        assert_eq!(column_letter(26), "AA");
        assert_eq!(column_letter(27), "AB");
    }
}
