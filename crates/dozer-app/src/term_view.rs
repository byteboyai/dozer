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
//! `(col * cell_width(), row * line_height_px())`：
//! - ASCII 连续同风格格子合并成一个 run——等宽字体保证 run 内部对齐；
//! - 宽字符（CJK）独立成 run、占 2 格绘制盒——字形 advance 与网格假设的
//!   偏差被"每个宽字符重新定位"吞掉，不会累积；
//! - 宽字符 spacer 格与无背景空白格不产生字形（空白格同时切断 run，
//!   保证 run 内文本列数与网格列数一致）。
//!
//! 光标最后画：先补一块实心格（focused：CREAM 底 + TERM_BG 字；未聚焦：
//! CREAM 描边），覆盖在 run 字形之上，天然处理"光标落在任意 run 中间"。
use crate::app::Message;
use crate::term_model::{Cell, TerminalModel};
use crate::theme;
use crate::theme::icon_size;
use crate::theme::terminal_font;
use iced_widget::canvas::{self, Canvas};
use iced_widget::core::font::Weight;
use iced_widget::core::mouse::{self, ScrollDelta};
use iced_widget::core::text::LineHeight;
use iced_widget::core::{
    Color, Element, Event, Font, Length, Pixels, Point, Rectangle, Size, Vector,
};

/// 终端字号（逻辑像素），已乘全局 UI scale——Ctrl + / Ctrl - 缩放时
/// 终端字符与图标/控件一起放大，而不是卡在设计基准 14px。grid 换算与
/// 实际绘制都走这里，二者始终一致。
fn font_size() -> f32 {
    terminal_font::size() * icon_size::scale()
}
/// 等宽字体单元格宽度 ≈ 0.6em（`font_size()` 驱动，含 scale）。
fn cell_width() -> f32 {
    font_size() * 0.6
}
/// 行高（逻辑像素），供 `grid_size` 换算用（`font_size()` 驱动，含 scale）。
fn line_height_px() -> f32 {
    font_size() * terminal_font::line_height_factor()
}

/// 终端 pane 的像素尺寸 → 网格尺寸 `(cols, rows)`，向下取整（不足一格的
/// 余量丢弃，避免半个字符溢出边界）。
pub fn grid_size(width_px: f32, height_px: f32) -> (usize, usize) {
    let cols = (width_px / cell_width()).floor().max(0.0) as usize;
    let rows = (height_px / line_height_px()).floor().max(0.0) as usize;
    (cols, rows)
}

fn rgb(c: (u8, u8, u8)) -> Color {
    Color::from_rgb8(c.0, c.1, c.2)
}

fn cell_font(bold: bool) -> Font {
    if bold {
        Font {
            weight: Weight::Bold,
            ..crate::fonts::code_font()
        }
    } else {
        crate::fonts::code_font()
    }
}

/// 画一格/一个 run 的字形，定位在 `(x, y)`（已包过 `with_save`，不影响后续
/// 绘制的坐标系）。
///
/// 曾经在这里给宽字符（CJK）加过水平方向的补偿缩放，想把比 2 格绘制盒窄的
/// 回退字形拉伸填满——结果是只放大宽度、不动高度的非均匀缩放，把方块字
/// 的字形拉扁了（宽高比失真，比原来的"偏松散"更难看，见验收反馈）。改为
/// 均匀缩放又会让字形连带长高，终端行距没有为此预留余量，容易跟下一行
/// 撞在一起。两条路都比"字形原样、只是没填满 2 格盒子右侧"更糟，所以
/// 干脆不缩放——`layout_runs` 对宽字符的逐格重新定位已经解决了原本的
/// 整行漂移问题，字形本身留白就留白，不再用缩放去凑。
fn fill_cell_text(
    frame: &mut canvas::Frame,
    content: String,
    x: f32,
    y: f32,
    color: Color,
    font: Font,
) {
    frame.with_save(|frame| {
        frame.translate(Vector::new(x, y));
        frame.fill_text(canvas::Text {
            content,
            position: Point::ORIGIN,
            color,
            size: Pixels(font_size()),
            line_height: LineHeight::Absolute(Pixels(line_height_px())),
            font,
            ..canvas::Text::default()
        });
    });
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
    selected: bool,
    /// 宽字符（CJK 等）本体 run：`cells == 2` 不能当宽字符判据——两个窄
    /// 字符合并的 run 也可能 `cells == 2`（如 "ab"）,必须显式记录。
    wide: bool,
}

/// 单行 cell 序列 → 绘制 run 序列。切分规则见模块注释；选区内的空白格
/// 不跳过（要画选区底色），`selected` 变化处切断 run。
fn layout_runs(row: &[Cell]) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    let mut col = 0;
    while col < row.len() {
        let cell = &row[col];
        // spacer 由宽字符本体的 2 格绘制盒覆盖；选区外的无背景空白格画
        // 不出任何东西——两者都跳过（后者顺带切断了 run 的连续性）。
        if cell.spacer || (cell.ch == ' ' && cell.bg.is_none() && !cell.selected) {
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
                selected: cell.selected,
                wide: true,
            });
            col += 2; // 本体 + spacer
            continue;
        }
        let style = (cell.fg, cell.bg, cell.bold, cell.selected);
        let start = col;
        let mut text = String::new();
        while col < row.len() {
            let c = &row[col];
            let blank = c.ch == ' ' && c.bg.is_none() && !c.selected;
            if c.wide || c.spacer || blank || (c.fg, c.bg, c.bold, c.selected) != style {
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
            selected: style.3,
            wide: false,
        });
    }
    runs
}

/// 滚轮事件 → 滚动行数。行式滚轮直接取整；像素式（触控板）除以行高，
/// 不足一行的余量经 `residual` 跨事件累积，避免细腻滑动永远凑不满一行。
/// 返回 `(整行数, 新的 residual)`；正数 = 向历史方向（上翻）。
fn wheel_to_lines(delta: ScrollDelta, residual: f32) -> (i32, f32) {
    let lines = match delta {
        ScrollDelta::Lines { y, .. } => y,
        ScrollDelta::Pixels { y, .. } => y / line_height_px(),
    };
    let total = residual + lines;
    let whole = total.trunc();
    (whole as i32, total - whole)
}

/// 一行滚轮 → xterm 鼠标上报字节。`up`=true 对应滚轮上滑（按钮码 64,
/// xterm 协议为滚轮预留的专用码位，不是常规左中右键 0/1/2）；`col`/`row`
/// 是 0-based 网格坐标，协议要求 1-based。
///
/// SGR 扩展格式（`CSI ?1006h`）坐标无字节上限，用 `ESC[<Cb;Px;PyM`；
/// 未开启则退回 legacy X10 单字节编码（`ESC[M` + 三个 `值+32` 字节），
/// 列/行封顶 223 避免单字节溢出。
fn encode_wheel_report(up: bool, col: usize, row: usize, sgr: bool) -> Vec<u8> {
    let button = if up { 64u32 } else { 65 };
    let px = col + 1;
    let py = row + 1;
    if sgr {
        format!("\x1b[<{button};{px};{py}M").into_bytes()
    } else {
        let byte = |v: usize| (v.min(223) + 32) as u8;
        vec![0x1b, b'[', b'M', (button + 32) as u8, byte(px), byte(py)]
    }
}

/// canvas 绘制程序：持有当前 tab 的 `TerminalModel` 快照引用逐帧重画。
/// 网格规模（百列 × 数十行）下 run 数量有限，不做 `canvas::Cache`——
/// 终端输出本来就是高频失效场景，缓存收益低。
struct TermCanvas<'a> {
    model: &'a TerminalModel,
    focused: bool,
    target: crate::app::TermTarget,
}

/// canvas 内部交互状态：滚轮余量累积 + 拖选进行中标记。
#[derive(Default)]
struct InteractionState {
    /// 跨滚轮事件累积的不足一行余量（触控板像素滚动）。
    residual: f32,
    /// 左键按下且未松开（拖选进行中）。
    dragging: bool,
}

/// 画布内像素坐标 → 网格格坐标 `(col, row, right_half)`，钳制在
/// `(cols, rows)` 网格内（拖出边界时选区停在边缘格）。
fn cell_at(pos: Point, cols: usize, rows: usize) -> (usize, usize, bool) {
    let col_f = (pos.x / cell_width()).max(0.0);
    let col = (col_f as usize).min(cols.saturating_sub(1));
    let row = ((pos.y / line_height_px()).max(0.0) as usize).min(rows.saturating_sub(1));
    let right_half = col_f.fract() > 0.5;
    (col, row, right_half)
}

impl canvas::Program<Message, iced_widget::Theme, iced_renderer::Renderer> for TermCanvas<'_> {
    type State = InteractionState;

    fn update(
        &self,
        state: &mut Self::State,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let Event::Mouse(mouse_event) = event else {
            return None;
        };
        match mouse_event {
            mouse::Event::WheelScrolled { delta } => {
                if !cursor.is_over(bounds) {
                    return None;
                }
                let (lines, residual) = wheel_to_lines(*delta, state.residual);
                state.residual = residual;
                if lines == 0 {
                    return None;
                }
                // 前台程序自己开了鼠标上报(典型如 claude 进 alt screen 后
                // 用它实现内部滚动——alt screen 没有真正的 scrollback，
                // 本地 `TermScroll` 在那儿是滚不动的死路):把滚轮编码成
                // 鼠标转义序列转发给它，而不是走本地 scrollback。
                if self.model.mouse_report_mode() {
                    let pos = cursor.position_in(bounds)?;
                    let (cols, rows) = self.model.grid_dims();
                    let (col, row, _) = cell_at(pos, cols, rows);
                    let sgr = self.model.sgr_mouse();
                    let up = lines > 0;
                    let mut bytes = Vec::new();
                    for _ in 0..lines.unsigned_abs() {
                        bytes.extend(encode_wheel_report(up, col, row, sgr));
                    }
                    return Some(
                        canvas::Action::publish(Message::TermInput(self.target, bytes))
                            .and_capture(),
                    );
                }
                Some(canvas::Action::publish(Message::TermScroll(self.target, lines)).and_capture())
            }
            mouse::Event::ButtonPressed(mouse::Button::Left) => {
                let pos = cursor.position_in(bounds)?;
                state.dragging = true;
                let (cols, rows) = self.model.grid_dims();
                let (col, row, right) = cell_at(pos, cols, rows);
                Some(
                    canvas::Action::publish(Message::TermSelStart {
                        target: self.target,
                        col,
                        row,
                        right,
                    })
                    .and_capture(),
                )
            }
            mouse::Event::CursorMoved { .. } if state.dragging => {
                // 拖拽中允许移出画布：用全局位置减 bounds 原点，交给
                // `cell_at` 钳到边缘格。
                let pos = cursor.position()?;
                let rel = Point::new(pos.x - bounds.x, pos.y - bounds.y);
                let (cols, rows) = self.model.grid_dims();
                let (col, row, right) = cell_at(rel, cols, rows);
                Some(canvas::Action::publish(Message::TermSelUpdate {
                    target: self.target,
                    col,
                    row,
                    right,
                }))
            }
            mouse::Event::ButtonReleased(mouse::Button::Left) if state.dragging => {
                state.dragging = false;
                None
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced_renderer::Renderer,
        _theme: &iced_widget::Theme,
        bounds: Rectangle,
        _cursor: iced_widget::core::mouse::Cursor,
    ) -> Vec<canvas::Geometry<iced_renderer::Renderer>> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let lines = self.model.visible_lines();
        let (cursor_col, cursor_row) = self.model.cursor();

        for (row_idx, row) in lines.iter().enumerate() {
            let y = row_idx as f32 * line_height_px();
            for run in layout_runs(row) {
                let x = run.col as f32 * cell_width();
                let run_size = Size::new(run.cells as f32 * cell_width(), line_height_px());
                if let Some(bg) = run.bg {
                    frame.fill_rectangle(Point::new(x, y), run_size, rgb(bg));
                }
                if run.selected {
                    // 选区底色：CYAN 25% 叠加（不动锁定的主题色表）。
                    frame.fill_rectangle(
                        Point::new(x, y),
                        run_size,
                        Color {
                            a: 0.25,
                            ..theme::color::CYAN
                        },
                    );
                }
                fill_cell_text(&mut frame, run.text, x, y, rgb(run.fg), cell_font(run.bold));
            }
        }

        // 光标覆盖层（画在文本之上）。回看历史（display_offset > 0）时
        // 光标在视口之外，不画；程序自己关掉了真实光标（DECTCEM `?25l`）
        // 也不画——全屏重绘型 TUI（实测 CodeBuddy CLI）常年不发 `?25h`，
        // 自己在文本里用反相画一个假光标，这时 `cursor()` 读到的坐标只是
        // 一堆重绘期间相对移动/清行序列扫过后随便落下的陈旧位置，跟视觉
        // 上的假光标毫无关系，画出来就是一个游离在别处的多余色块（Claude
        // Code 每轮重绘都以 `?25h` 收尾，光标坐标随时有效，不受影响）。
        // `cursor()` 直接取自 `Term` 网格，坐标是活动区视口坐标；防御性
        // 判界只为杜绝 cast 环绕的极端值。
        let offset = self.model.display_offset();
        if offset == 0
            && self.model.cursor_visible()
            && let Some(cell) = lines.get(cursor_row).and_then(|r| r.get(cursor_col))
        {
            let x = cursor_col as f32 * cell_width();
            let y = cursor_row as f32 * line_height_px();
            let box_w = if cell.wide { 2.0 } else { 1.0 } * cell_width();
            if self.focused {
                frame.fill_rectangle(
                    Point::new(x, y),
                    Size::new(box_w, line_height_px()),
                    theme::color::CREAM,
                );
                if cell.ch != ' ' {
                    fill_cell_text(
                        &mut frame,
                        cell.ch.to_string(),
                        x,
                        y,
                        theme::color::TERM_BG,
                        cell_font(cell.bold),
                    );
                }
            } else {
                frame.stroke(
                    &canvas::Path::rectangle(Point::new(x, y), Size::new(box_w, line_height_px())),
                    canvas::Stroke::default()
                        .with_color(theme::color::CREAM)
                        .with_width(1.0),
                );
            }
        }

        // 滚动指示条：仅回看历史时出现在右缘（实时跟随输出时不占视觉）。
        // 内容总量 = 历史 + 视口；thumb 位置/高度按视口在总量中的窗口映射。
        if offset > 0 {
            let history = self.model.history_len() as f32;
            let rows = lines.len() as f32;
            let total = history + rows;
            let h = bounds.height;
            // 复用全应用统一滚动条配置(见 `byteui::interaction::scrollbar`):轨道宽度
            // `scrollbar_width`,滑块(thumb)宽度 `scrollbar_thumb_width` 并居
            // 中,滑块颜色甲方金 `#dcc9a3`(`theme::color::TAB_ACTIVE_BORDER`)。
            let bar_w = theme::geometry::scrollbar_width();
            let thumb_w = theme::geometry::scrollbar_thumb_width();
            let track_x = bounds.width - bar_w;
            let thumb_h = (rows / total * h).max(12.0);
            let thumb_top = ((history - offset as f32) / total * h).min(h - thumb_h);
            frame.fill_rectangle(
                Point::new(track_x, 0.0),
                Size::new(bar_w, h),
                theme::color::BORDER,
            );
            frame.fill_rectangle(
                Point::new(track_x + (bar_w - thumb_w) / 2.0, thumb_top),
                Size::new(thumb_w, thumb_h),
                theme::color::TAB_ACTIVE_BORDER,
            );
        }

        vec![frame.into_geometry()]
    }
}

/// 渲染整块终端网格。
pub fn view(
    model: &TerminalModel,
    focused: bool,
    target: crate::app::TermTarget,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    Canvas::new(TermCanvas {
        model,
        focused,
        target,
    })
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row_of(input: &[u8], cols: u16) -> Vec<Cell> {
        let mut t = TerminalModel::new(cols, 4);
        let _ = t.feed(input);
        t.visible_lines().remove(0)
    }

    #[test]
    fn selection_splits_runs_and_flags_them() {
        let mut t = TerminalModel::new(20, 4);
        let _ = t.feed(b"abcdef");
        t.selection_start(1, 0, false);
        t.selection_update(3, 0, true); // 选中 bcd
        let row = t.visible_lines().remove(0);
        let runs = layout_runs(&row);
        let shape: Vec<_> = runs
            .iter()
            .map(|r| (r.col, r.text.as_str(), r.selected))
            .collect();
        assert_eq!(
            shape,
            vec![(0, "a", false), (1, "bcd", true), (4, "ef", false)]
        );
    }

    #[test]
    fn selected_blank_cells_are_kept_for_highlight() {
        let mut t = TerminalModel::new(20, 4);
        let _ = t.feed(b"a b");
        t.selection_start(0, 0, false);
        t.selection_update(2, 0, true); // 选中 "a b"，中间空格也要高亮
        let row = t.visible_lines().remove(0);
        let runs = layout_runs(&row);
        assert_eq!(runs.len(), 1);
        assert_eq!(
            (runs[0].col, runs[0].text.as_str(), runs[0].selected),
            (0, "a b", true)
        );
    }

    #[test]
    fn wheel_report_sgr_encoding() {
        // 列/行 0-based → 协议 1-based；上滑=按钮 64。
        assert_eq!(
            encode_wheel_report(true, 3, 7, true),
            b"\x1b[<64;4;8M".to_vec()
        );
        // 下滑=按钮 65。
        assert_eq!(
            encode_wheel_report(false, 3, 7, true),
            b"\x1b[<65;4;8M".to_vec()
        );
    }

    #[test]
    fn wheel_report_legacy_encoding_clamps_to_single_byte() {
        // legacy X10:三个 值+32 字节，列/行封顶 223 避免超宽终端溢出。
        assert_eq!(
            encode_wheel_report(true, 3, 7, false),
            vec![0x1b, b'[', b'M', 64 + 32, 4 + 32, 8 + 32]
        );
        let huge = encode_wheel_report(false, 300, 300, false);
        assert_eq!(huge, vec![0x1b, b'[', b'M', 65 + 32, 223 + 32, 223 + 32]);
    }

    #[test]
    fn wheel_lines_accumulate_with_residual() {
        use iced_widget::core::mouse::ScrollDelta;
        // 行式滚轮：整行直接进位
        let (n, r) = wheel_to_lines(ScrollDelta::Lines { x: 0.0, y: 2.0 }, 0.0);
        assert_eq!((n, r), (2, 0.0));
        // 像素式（触控板）：不足一行的余量留在 residual 里跨事件累积。
        // 用行高的比例表达,与字号无关（0.6 行:单次不足 1 行,两次 > 1 行）。
        let half = line_height_px() * 0.6;
        let (n, r) = wheel_to_lines(ScrollDelta::Pixels { x: 0.0, y: half }, 0.0);
        assert_eq!(n, 0);
        assert!(r > 0.0);
        let (n2, _) = wheel_to_lines(ScrollDelta::Pixels { x: 0.0, y: half }, r);
        assert_eq!(n2, 1, "两次 0.6 行的像素量应累积出 1 行");
        // 反方向：整整两行的像素量
        let (n, _) = wheel_to_lines(
            ScrollDelta::Pixels {
                x: 0.0,
                y: -line_height_px() * 2.0,
            },
            0.0,
        );
        assert_eq!(n, -2);
    }

    #[test]
    fn wide_flag_distinguishes_cjk_run_from_same_cell_count_ascii_run() {
        // "ab" 是两个窄字符合并的 run，cells 也是 2——不能靠 cells==2 判断
        // 宽字符（见 `Run::wide` 字段注释），必须显式 flag。
        let ascii = &layout_runs(&row_of(b"ab", 40))[0];
        assert_eq!((ascii.cells, ascii.wide), (2, false));
        let cjk = &layout_runs(&row_of("你".as_bytes(), 40))[0];
        assert_eq!((cjk.cells, cjk.wide), (2, true));
    }

    #[test]
    fn grid_size_from_pixels() {
        // 字号 14px 等宽：单元格宽 ≈ 8.4px（0.6em），行高 ≈ 16.8px（1.2，对齐 RustRover）
        let (cols, rows) = grid_size(780.0, 546.0);
        assert!((88..=96).contains(&cols), "cols={cols}");
        assert!((28..=38).contains(&rows), "rows={rows}");
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
