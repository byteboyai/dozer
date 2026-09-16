// crates/dozer-app/src/term_model.rs
//! TerminalModel：headless 终端状态机（P1c T4）。
//!
//! 包装 `alacritty_terminal` 的 VTE 解析器与 `Term` 网格状态，向渲染层
//! （T5）暴露一份纯数据快照（`Cell` 网格 + 光标位置），不依赖 iced，避免
//! 渲染 crate 反向渗入本模块。

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::{Color as AnsiColor, NamedColor, Processor, Rgb};
use std::cell::RefCell;
use std::rc::Rc;

/// ANSI 16 色的深色主题 RGB 表，下标与 `NamedColor` 的判别值（0..=15）
/// 一致：Black/Red/Green/Yellow/Blue/Magenta/Cyan/White，随后是对应的
/// Bright 变体。
const ANSI16_DARK: [(u8, u8, u8); 16] = [
    (0x08, 0x14, 0x1d), // Black
    (0xFF, 0x6E, 0x6E), // Red
    (0x1A, 0xD5, 0x85), // Green
    (0xF2, 0xD9, 0x4E), // Yellow
    (0x95, 0x80, 0xFF), // Blue
    (0xFF, 0x3D, 0xCC), // Magenta
    (0x47, 0xDE, 0xF0), // Cyan
    (0xFF, 0xE5, 0xB4), // White
    (0x6B, 0x7F, 0x8F), // BrightBlack
    (0xFF, 0x8E, 0x8E), // BrightRed
    (0x4A, 0xE5, 0xA5), // BrightGreen
    (0xFF, 0xF3, 0xB0), // BrightYellow
    (0xB5, 0xA5, 0xFF), // BrightBlue
    (0xFF, 0x6D, 0xDC), // BrightMagenta
    (0x87, 0xEE, 0xF8), // BrightCyan
    (0xFF, 0xF5, 0xD4), // BrightWhite
];

/// ANSI 16 色的浅色主题 RGB 表，逐项对应 `design/浅色配色表.html` 的终端
/// 色板提案表。下标含义同 `ANSI16_DARK`。
const ANSI16_LIGHT: [(u8, u8, u8); 16] = [
    (0xfc, 0xfd, 0xfe), // Black ≈ term_bg
    (0xd1, 0x48, 0x3f), // Red
    (0x12, 0x8f, 0x5a), // Green
    (0xc9, 0xa2, 0x27), // Yellow(浅色版不再与 gold 同值)
    (0x6a, 0x4f, 0xdb), // Blue(实为紫色调)
    (0xc7, 0x1f, 0xa0), // Magenta
    (0x0e, 0x8a, 0x9e), // Cyan
    (0x16, 0x23, 0x2e), // White == ink
    (0x8b, 0x98, 0xa2), // BrightBlack
    (0xe5, 0x45, 0x3a), // BrightRed
    (0x0f, 0xa9, 0x68), // BrightGreen
    (0x8a, 0x7a, 0x12), // BrightYellow(刻意区别于 gold)
    (0x7c, 0x5c, 0xe8), // BrightBlue
    (0xd8, 0x2b, 0xb0), // BrightMagenta
    (0x0a, 0xa0, 0xb8), // BrightCyan
    (0x0b, 0x13, 0x1a), // BrightWhite
];

/// 当前生效的 ANSI16 表：随 `byteui::theme::color::current_scheme()`
/// 切换，不需要重开终端。
fn ansi16() -> &'static [(u8, u8, u8); 16] {
    match byteui::theme::color::current_scheme() {
        byteui::theme::color::ColorScheme::Dark => &ANSI16_DARK,
        byteui::theme::color::ColorScheme::Light => &ANSI16_LIGHT,
    }
}

/// 默认前景色（无显式 SGR 时的字符颜色），深色版数值。
const DEFAULT_FG_DARK: (u8, u8, u8) = (0x9A, 0xB4, 0xC4);
/// 默认前景色，浅色版数值（== 语义色板 `body`）。
const DEFAULT_FG_LIGHT: (u8, u8, u8) = (0x36, 0x42, 0x4e);

fn default_fg() -> (u8, u8, u8) {
    match byteui::theme::color::current_scheme() {
        byteui::theme::color::ColorScheme::Dark => DEFAULT_FG_DARK,
        byteui::theme::color::ColorScheme::Light => DEFAULT_FG_LIGHT,
    }
}

/// 终端默认前景色（无显式 SGR 时的字符颜色）。供预览编辑器把语法高亮
/// token 锚定到终端同款观感时取用（见 `preview::dozer_syntax_theme`）。
pub(crate) fn default_fg_rgb() -> (u8, u8, u8) {
    default_fg()
}

/// ANSI 16 色主题的第 `idx` 个 RGB（`0..16`，下标即 `NamedColor`）。供
/// 预览编辑器把语法高亮 token 锚定到终端色板时取用（见
/// `preview::dozer_syntax_theme`）。越界返回 `None`。
pub(crate) fn ansi16_color(idx: usize) -> Option<(u8, u8, u8)> {
    ansi16().get(idx).copied()
}

/// 渲染层唯一数据源：一个终端网格格子。刻意只含原始值（`char`/`(u8,u8,u8)`），
/// 不引入 `iced::Color`，保持本模块可在无渲染依赖下单测。
#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    pub ch: char,
    pub fg: (u8, u8, u8),
    pub bg: Option<(u8, u8, u8)>,
    pub bold: bool,
    /// 宽字符（CJK 等）本体格：占两列，渲染层应给它 2 格宽的绘制盒。
    pub wide: bool,
    /// 宽字符的第二格（占位符）：渲染层应跳过，不产生任何字形。
    pub spacer: bool,
    /// 处于鼠标选区内：渲染层画选区底色。
    pub selected: bool,
    /// 反相显示（`CSI 7m`/`27m`，DECSCNM 之外最常见的用法是全屏重绘型
    /// TUI 自己在文本里画"假光标"——见 `cursor_visible` 文档 CodeBuddy
    /// CLI 的例子）。`fg`/`bg` 这里仍是原始未交换的值,交没交换由渲染层
    /// 按这个标记决定,模型层不假设渲染层的默认背景色是什么。
    pub inverse: bool,
}

/// `Term::new`/`Term::resize` 需要的最小尺寸描述。滚屏历史（scrollback）
/// 不走这里——由 `Config::default()` 的 `scrolling_history`（10000 行）
/// 决定；`total_lines` 与 alacritty 自身的 `SizeInfo` 一致，等于可视行数。
#[derive(Clone, Copy)]
struct TermSize {
    columns: usize,
    screen_lines: usize,
}

impl Dimensions for TermSize {
    fn total_lines(&self) -> usize {
        self.screen_lines
    }

    fn screen_lines(&self) -> usize {
        self.screen_lines
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

/// ANSI 16 色（`NamedColor`）→ 主题 RGB。`Background` 按规格无主题色，
/// 返回 `None`（由背景色分支处理成"透明/继承面板底色"）；其余具名色
/// （Cursor/Dim*/BrightForeground 等）主题未定义，退回默认前景色。
fn named_color_rgb(name: NamedColor) -> Option<(u8, u8, u8)> {
    let idx = name as usize;
    let table = ansi16();
    if idx < table.len() {
        return Some(table[idx]);
    }
    match name {
        NamedColor::Background => None,
        _ => Some(default_fg()),
    }
}

/// ANSI-256 索引色 → RGB：0..16 复用 16 色主题表；16..232 是标准 xterm
/// 6x6x6 色立方；232..256 是灰阶渐变。P1c 范围内的 ANSI 转义序列不会
/// 触发这一段，仅为完整性保留标准 xterm 映射。
fn indexed_to_rgb(idx: u8) -> (u8, u8, u8) {
    match idx {
        0..=15 => ansi16()[idx as usize],
        16..=231 => {
            let i = idx - 16;
            let r = i / 36;
            let g = (i % 36) / 6;
            let b = i % 6;
            let scale = |c: u8| if c == 0 { 0 } else { 55 + c * 40 };
            (scale(r), scale(g), scale(b))
        }
        232..=255 => {
            let level = 8 + (idx - 232) * 10;
            (level, level, level)
        }
    }
}

fn fg_to_rgb(color: AnsiColor) -> (u8, u8, u8) {
    match color {
        AnsiColor::Named(name) => named_color_rgb(name).unwrap_or_else(default_fg),
        AnsiColor::Spec(rgb) => (rgb.r, rgb.g, rgb.b),
        AnsiColor::Indexed(idx) => indexed_to_rgb(idx),
    }
}

fn bg_to_rgb(color: AnsiColor) -> Option<(u8, u8, u8)> {
    match color {
        AnsiColor::Named(name) => named_color_rgb(name),
        AnsiColor::Spec(rgb) => Some((rgb.r, rgb.g, rgb.b)),
        AnsiColor::Indexed(idx) => Some(indexed_to_rgb(idx)),
    }
}

#[derive(Default)]
struct PtyResponsesInner {
    buf: Vec<u8>,
    /// opencode 靠 OSC 10/11 查询探测终端真实前景/背景色决定自己的
    /// "system" 主题——实测 PTY 抓包：无人应答时它固定退回硬编码深色
    /// 主题。默认关闭，仅在会话 `agent == AgentKind::Opencode` 时开启
    /// （见 `workspace.rs` 的 `TerminalModel::new` 调用点），避免对没有
    /// 这个问题的其它 agent 引入不必要的行为变化。
    answer_dynamic_color: bool,
}

/// 收集 `Term` 解析过程中生成的 PTY 回写应答（`Event::PtyWrite`）：
/// DSR 光标位置（CSI 6n）、DA 设备属性（CSI c）、DECRPM 模式查询等。
/// atuin/ink（claude）等 TUI 依赖这些应答；此前用 `VoidListener` 全部
/// 丢弃，导致这类程序探测超时/行为异常。
#[derive(Default, Clone)]
struct PtyResponses(Rc<RefCell<PtyResponsesInner>>);

/// `Event::ColorRequest` 里 alacritty 已经解析好的目标色号——只识别 OSC
/// 10（前景）/11（背景），其余（光标色 OSC 12、调色板 0-15 动态查询）
/// 不在 opencode 实测会用到的范围内，忽略。
fn dynamic_color_rgb(index: usize) -> Option<(u8, u8, u8)> {
    if index == NamedColor::Foreground as usize {
        Some(default_fg_rgb())
    } else if index == NamedColor::Background as usize {
        let term_bg = byteui::theme::color::current().term_bg;
        let [r, g, b, _] = term_bg.into_rgba8();
        Some((r, g, b))
    } else {
        None
    }
}

impl EventListener for PtyResponses {
    fn send_event(&self, event: Event) {
        let mut inner = self.0.borrow_mut();
        match event {
            Event::PtyWrite(text) => inner.buf.extend_from_slice(text.as_bytes()),
            Event::ColorRequest(index, formatter) if inner.answer_dynamic_color => {
                if let Some((r, g, b)) = dynamic_color_rgb(index) {
                    let reply = formatter(Rgb { r, g, b });
                    inner.buf.extend_from_slice(reply.as_bytes());
                }
            }
            _ => {}
        }
    }
}

/// headless 终端状态机：字节流喂给 VTE 解析器，驱动 `alacritty_terminal`
/// 的 `Term` 网格状态；对外只暴露只读快照（`visible_lines`/`cursor`）。
pub struct TerminalModel {
    term: Term<PtyResponses>,
    parser: Processor,
    /// 与 `term` 内 listener 共享同一块缓冲，`feed` 后取走。
    responses: PtyResponses,
}

impl TerminalModel {
    /// 新建一个 `cols` x `rows` 的终端。
    pub fn new(cols: u16, rows: u16) -> Self {
        let size = TermSize {
            columns: cols as usize,
            screen_lines: rows as usize,
        };
        let responses = PtyResponses::default();
        let term = Term::new(Config::default(), &size, responses.clone());
        Self {
            term,
            parser: Processor::new(),
            responses,
        }
    }

    /// 把字节流喂给 VTE 解析器，解析结果直接落到 `Term` 的网格状态上。
    ///
    /// 返回本次解析生成的 PTY 回写应答（设备查询的响应字节）。调用方
    /// 决定去向：实时输出 → 写回 daemon；快照回放 → 丢弃（重放历史里的
    /// 查询不能补发陈旧应答）。
    #[must_use = "PTY 应答字节需要显式处理：实时输出写回 daemon，快照回放丢弃"]
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<u8> {
        // `Processor::advance` 需要同时可变借用 parser 与 term；先解构再
        // 分别取字段引用，避免对 `self` 的双重可变借用。
        let Self { term, parser, .. } = self;
        parser.advance(term, bytes);
        std::mem::take(&mut self.responses.0.borrow_mut().buf)
    }

    /// 是否回应 OSC 10/11 前景/背景色查询（见 [`dynamic_color_rgb`]）。
    /// 只应该对 `agent == AgentKind::Opencode` 的会话开启。
    pub fn set_answer_dynamic_color(&mut self, enabled: bool) {
        self.responses.0.borrow_mut().answer_dynamic_color = enabled;
    }

    /// 调整终端尺寸，尽量保留既有内容（委托给 `Term::resize` 的重排逻辑）。
    pub fn resize(&mut self, cols: u16, rows: u16) {
        let size = TermSize {
            columns: cols as usize,
            screen_lines: rows as usize,
        };
        self.term.resize(size);
    }

    /// 向历史方向（正数）或活动区方向（负数）滚动视口 `delta` 行；
    /// 越界由 alacritty 自动钳制在 `[0, history_len]`。
    pub fn scroll_display(&mut self, delta: i32) {
        self.term.scroll_display(Scroll::Delta(delta));
    }

    /// 回到活动区底部（`display_offset` 归零）。
    pub fn scroll_to_bottom(&mut self) {
        self.term.scroll_display(Scroll::Bottom);
    }

    /// 当前视口相对活动区底部上移的行数；0 = 正在看实时输出。
    pub fn display_offset(&self) -> usize {
        self.term.grid().display_offset()
    }

    /// 当前网格尺寸 `(cols, rows)`（渲染层做像素 → 格坐标换算用）。
    pub fn grid_dims(&self) -> (usize, usize) {
        let grid = self.term.grid();
        (grid.columns(), grid.screen_lines())
    }

    /// 滚屏历史当前实际行数（随输出增长，上限 `scrolling_history`）。
    pub fn history_len(&self) -> usize {
        let grid = self.term.grid();
        grid.total_lines() - grid.screen_lines()
    }

    /// 视口坐标 `(col, row)` + display offset → 网格坐标。
    fn viewport_to_point(&self, col: usize, row: usize) -> Point {
        let offset = self.term.grid().display_offset() as i32;
        Point::new(Line(row as i32 - offset), Column(col))
    }

    /// 鼠标按下：在 `(col, row)`（视口坐标）起一个新的简单选区。
    /// `right_half` 表示按点落在格子的右半（决定选区端点贴哪一侧）。
    pub fn selection_start(&mut self, col: usize, row: usize, right_half: bool) {
        let point = self.viewport_to_point(col, row);
        let side = if right_half { Side::Right } else { Side::Left };
        self.term.selection = Some(Selection::new(SelectionType::Simple, point, side));
    }

    /// 鼠标拖拽：把选区末端拖到 `(col, row)`（视口坐标）。
    pub fn selection_update(&mut self, col: usize, row: usize, right_half: bool) {
        let point = self.viewport_to_point(col, row);
        let side = if right_half { Side::Right } else { Side::Left };
        if let Some(selection) = &mut self.term.selection {
            selection.update(point, side);
        }
    }

    /// 当前选区文本（跨行以 `\n` 连接）；无选区/空选区返回 `None`。
    pub fn selection_text(&self) -> Option<String> {
        self.term.selection_to_string().filter(|s| !s.is_empty())
    }

    /// 清除选区。
    pub fn selection_clear(&mut self) {
        self.term.selection = None;
    }

    /// 可视网格快照，逐行逐格返回（已计入 `display_offset`——回看历史时
    /// 返回的就是屏幕上应显示的行）。宽字符的 spacer 格 `ch` 置为空格。
    pub fn visible_lines(&self) -> Vec<Vec<Cell>> {
        let grid = self.term.grid();
        let cols = grid.columns();
        let rows = grid.screen_lines();
        let offset = grid.display_offset() as i32;
        let selection = self
            .term
            .selection
            .as_ref()
            .and_then(|s| s.to_range(&self.term));

        (0..rows)
            .map(|row| {
                // 视口第 `row` 行对应网格 `Line(row - offset)`：负值索引
                // 进入滚屏历史（alacritty 的 `Index<Line>` 原生支持）。
                let grid_line = Line(row as i32 - offset);
                let line = &grid[grid_line];
                (0..cols)
                    .map(|col| {
                        let cell = &line[Column(col)];
                        let spacer = cell.flags.contains(Flags::WIDE_CHAR_SPACER);
                        Cell {
                            ch: if spacer { ' ' } else { cell.c },
                            fg: fg_to_rgb(cell.fg),
                            bg: bg_to_rgb(cell.bg),
                            bold: cell.flags.contains(Flags::BOLD),
                            wide: cell.flags.contains(Flags::WIDE_CHAR),
                            spacer,
                            selected: selection
                                .is_some_and(|r| r.contains(Point::new(grid_line, Column(col)))),
                            inverse: cell.flags.contains(Flags::INVERSE),
                        }
                    })
                    .collect()
            })
            .collect()
    }

    /// 光标位置 `(col, row)`，row 为可视行号（无 scrollback，因此等于
    /// `Term` 内部 `Line` 的原始值）。
    pub fn cursor(&self) -> (usize, usize) {
        let point = self.term.grid().cursor.point;
        (point.column.0, point.line.0 as usize)
    }

    /// 真实终端光标是否可见（DECTCEM，`CSI ?25h` 开 / `CSI ?25l` 关）。
    /// 全屏重绘型 TUI（实测 CodeBuddy CLI 2.132.0）经常整段会话只在启动
    /// 时发一次 `?25l`、此后再也不发 `?25h`——它们自己在文本内容里用反相
    /// （`CSI 7m`/`CSI 27m`）画一个"假光标"字符，真实光标被晾在原地（每
    /// 帧一堆相对移动/清行序列扫过后落在的任意位置，跟视觉上的假光标毫
    /// 无关系）。`cursor()` 只读 `grid().cursor.point`、不看这个 mode，
    /// 于是会在这类 TUI 上把真实光标的陈旧坐标当成有效位置画出一个额外的
    /// 光标块——正是这次要修的 bug。对照实测：Claude Code 每轮重绘都以
    /// `?25h` 收尾，把真实光标移到正确位置后再显示，从不用反相假光标，
    /// 所以同样的绘制逻辑在它身上不出这个问题。
    pub fn cursor_visible(&self) -> bool {
        self.term.mode().contains(TermMode::SHOW_CURSOR)
    }

    /// 是否处于 application cursor mode（DECCKM，`CSI ?1h` 开启 /
    /// `CSI ?1l` 关闭）。shell 行编辑器（readline/zle 等）常用它来把方向
    /// 键从 CSI 序列（`\x1b[A`）切换成 SS3 序列（`\x1bOA`），
    /// `keymap::key_to_bytes` 据此决定发哪一种转义序列。
    pub fn app_cursor_mode(&self) -> bool {
        self.term.mode().contains(TermMode::APP_CURSOR)
    }

    /// 是否处于 bracketed paste mode（`CSI ?2004h/l`）。开启时粘贴文本
    /// 需用 `ESC[200~`/`ESC[201~` 包裹，vim/claude 等据此区分"粘贴"与
    /// "逐键输入"（避免自动缩进错乱、按键误触发）。
    pub fn bracketed_paste(&self) -> bool {
        self.term.mode().contains(TermMode::BRACKETED_PASTE)
    }

    /// 是否处于任意鼠标上报模式（`CSI ?1000/1002/1003h` 任一开启）。
    ///
    /// claude 等 TUI 进 alt screen 后没有真正的 scrollback 可回看（alt
    /// grid 的 history 容量恒为 0，这是 alacritty_terminal 的既定行为，
    /// 真实终端同样如此），于是自己开鼠标上报、靠接收滚轮转义序列在
    /// 应用内部实现"滚动"。此前本模块不查这个 mode，滚轮永远走本地
    /// `scroll_display`——在 alt screen 下这是滚不动的死路，表现为"无法
    /// 上下滚动"。开启时滚轮应编码成鼠标转义序列转发给前台程序，而不是
    /// 走本地 scrollback。
    pub fn mouse_report_mode(&self) -> bool {
        self.term.mode().intersects(TermMode::MOUSE_MODE)
    }

    /// 鼠标上报是否用 SGR 扩展格式（`CSI ?1006h`）。开启时坐标无单字节
    /// 数值上限；未开启则退回 legacy X10 编码（列/行数值需 +32 压进一个
    /// 字节，超宽终端会溢出，故封顶）。
    pub fn sgr_mouse(&self) -> bool {
        self.term.mode().contains(TermMode::SGR_MOUSE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// `byteui::theme::color` 的当前配色方案是进程级共享 static，见
    /// `byteui/src/theme/color.rs` 测试模块同名锁的注释。这里只有下面两个
    /// 主题相关测试会碰它，加锁避免它们互相插队。
    static SCHEME_TEST_LOCK: Mutex<()> = Mutex::new(());

    fn lock_scheme() -> std::sync::MutexGuard<'static, ()> {
        SCHEME_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn line_text(cells: &[Cell]) -> String {
        cells
            .iter()
            .map(|c| c.ch)
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    #[test]
    fn mouse_report_mode_off_by_default() {
        let t = TerminalModel::new(40, 10);
        assert!(!t.mouse_report_mode());
        assert!(!t.sgr_mouse());
    }

    #[test]
    fn mouse_report_mode_on_after_claude_startup_sequence() {
        // claude 启动时实测发出的模式序列(节选,足以复现):进 alt screen +
        // 开三档鼠标上报(1000/1002/1003)+ SGR 扩展坐标(1006)。
        let mut t = TerminalModel::new(80, 24);
        let _ = t.feed(b"\x1b[?1049h\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1006h");
        assert!(t.mouse_report_mode());
        assert!(t.sgr_mouse());
    }

    #[test]
    fn mouse_report_mode_reflects_disable() {
        let mut t = TerminalModel::new(40, 10);
        let _ = t.feed(b"\x1b[?1000h");
        assert!(t.mouse_report_mode());
        let _ = t.feed(b"\x1b[?1000l");
        assert!(!t.mouse_report_mode());
    }

    #[test]
    fn cursor_visible_on_by_default() {
        let t = TerminalModel::new(40, 10);
        assert!(t.cursor_visible());
    }

    #[test]
    fn cursor_visible_reflects_dectcem_hide_show() {
        let mut t = TerminalModel::new(40, 10);
        let _ = t.feed(b"\x1b[?25l");
        assert!(!t.cursor_visible());
        let _ = t.feed(b"\x1b[?25h");
        assert!(t.cursor_visible());
    }

    #[test]
    fn cursor_visible_stays_off_through_codebuddy_style_redraw() {
        // 实测 CodeBuddy CLI 2.132.0（pty 抓包）：启动时发一次 `?25l`，
        // 此后整段会话（含打字回显）再也不发 `?25h`——靠反相
        // （`CSI 7m`/`CSI 27m`）在文本里画自己的假光标。中间一大段相对
        // 光标移动 + 清行（`nA`/`2K`）是它的整帧重绘套路，节选自抓包，
        // 验证这些操作不会意外把 SHOW_CURSOR 拨回 true。
        let mut t = TerminalModel::new(100, 30);
        let _ = t.feed(b"\x1b[?25l\x1b[?25l");
        let _ = t.feed(
            b"\x1b[9A\r\x1b[2K\x1b[38;2;184;191;197m> \x1b[39mhello world\x1b[7m \x1b[27m\x1b[9B",
        );
        let _ = t.feed(
            b"\r\x1b[7A\r\x1b[2K  \x1b[38;2;184;191;197m\xe2\x86\x90 for agents\x1b[39m\x1b[7B",
        );
        assert!(!t.cursor_visible());
    }

    #[test]
    fn plain_text_lands_on_first_row() {
        let mut t = TerminalModel::new(40, 10);
        let _ = t.feed(b"hello dozer");
        assert_eq!(line_text(&t.visible_lines()[0]), "hello dozer");
        assert_eq!(t.cursor(), (11, 0));
    }

    #[test]
    fn newline_and_cr_move_cursor() {
        let mut t = TerminalModel::new(40, 10);
        let _ = t.feed(b"one\r\ntwo");
        let lines = t.visible_lines();
        assert_eq!(line_text(&lines[0]), "one");
        assert_eq!(line_text(&lines[1]), "two");
        assert_eq!(t.cursor(), (3, 1));
    }

    #[test]
    fn sgr_red_foreground_is_mapped() {
        let mut t = TerminalModel::new(40, 10);
        let _ = t.feed(b"\x1b[31mred\x1b[0m");
        let cell = &t.visible_lines()[0][0];
        assert_eq!(cell.ch, 'r');
        assert_eq!(cell.fg, (0xFF, 0x6E, 0x6E)); // ANSI 红 → 主题 RED
    }

    #[test]
    fn resize_keeps_content() {
        let mut t = TerminalModel::new(40, 10);
        let _ = t.feed(b"keepme");
        t.resize(60, 20);
        assert_eq!(line_text(&t.visible_lines()[0]), "keepme");
        assert_eq!(t.visible_lines().len(), 20);
    }

    #[test]
    fn utf8_cjk_occupies_two_columns() {
        let mut t = TerminalModel::new(40, 10);
        let _ = t.feed("你好".as_bytes());
        let l = &t.visible_lines()[0];
        assert_eq!(l[0].ch, '你');
        assert_eq!(l[2].ch, '好'); // 宽字符占两格，第 1 格为 spacer
        assert_eq!(t.cursor(), (4, 0));
    }

    #[test]
    fn drag_selection_extracts_text_and_marks_cells() {
        let mut t = TerminalModel::new(20, 5);
        let _ = t.feed(b"hello world");
        t.selection_start(0, 0, false);
        t.selection_update(4, 0, true); // 拖到第 5 格右半 → 含 'o'
        assert_eq!(t.selection_text().as_deref(), Some("hello"));

        let lines = t.visible_lines();
        assert!(lines[0][0].selected && lines[0][4].selected);
        assert!(!lines[0][5].selected, "空格在选区外");

        t.selection_clear();
        assert!(t.selection_text().is_none());
        assert!(!t.visible_lines()[0][0].selected);
    }

    #[test]
    fn click_without_drag_selects_nothing() {
        let mut t = TerminalModel::new(20, 5);
        let _ = t.feed(b"hello");
        t.selection_start(2, 0, false);
        assert!(t.selection_text().is_none(), "单击未拖拽不构成选区");
        assert!(!t.visible_lines()[0][2].selected);
    }

    #[test]
    fn selection_spans_multiple_rows() {
        let mut t = TerminalModel::new(10, 5);
        let _ = t.feed(b"aaa\r\nbbb");
        t.selection_start(0, 0, false);
        t.selection_update(2, 1, true);
        assert_eq!(t.selection_text().as_deref(), Some("aaa\nbbb"));
    }

    #[test]
    fn bracketed_paste_mode_toggles() {
        let mut t = TerminalModel::new(10, 5);
        assert!(!t.bracketed_paste());
        let _ = t.feed(b"\x1b[?2004h");
        assert!(t.bracketed_paste());
        let _ = t.feed(b"\x1b[?2004l");
        assert!(!t.bracketed_paste());
    }

    #[test]
    fn dsr_cursor_position_query_is_answered() {
        let mut t = TerminalModel::new(40, 10);
        let _ = t.feed(b"ab");
        // CSI 6n（DSR）：应用查询光标位置，终端必须应答 CSI row;col R
        // （1-based）。atuin/ink 等 TUI 靠它工作，无应答即超时报错。
        let resp = t.feed(b"\x1b[6n");
        assert_eq!(resp, b"\x1b[1;3R".to_vec());
    }

    #[test]
    fn plain_output_produces_no_pty_response() {
        let mut t = TerminalModel::new(40, 10);
        assert!(t.feed(b"hello").is_empty());
        assert!(t.feed("你好\r\n".as_bytes()).is_empty());
    }

    #[test]
    fn primary_device_attributes_query_is_answered() {
        let mut t = TerminalModel::new(40, 10);
        // CSI c（DA1）：很多 TUI 启动时探测终端型号，应答不能为空。
        let resp = t.feed(b"\x1b[c");
        assert!(
            resp.starts_with(b"\x1b[?"),
            "DA1 应答应为 CSI ? ... c，实际: {resp:?}"
        );
    }

    /// 造一个 10x5 终端并打印 l0..l19 共 20 行：屏幕剩 [l16..l19, 空]，
    /// 历史里应存下 l0..l15（16 行）。
    fn scrolled_term() -> TerminalModel {
        let mut t = TerminalModel::new(10, 5);
        for i in 0..20 {
            let _ = t.feed(format!("l{i}\r\n").as_bytes());
        }
        t
    }

    #[test]
    fn scroll_display_moves_viewport_into_history() {
        let mut t = scrolled_term();
        assert_eq!(t.display_offset(), 0);
        assert_eq!(line_text(&t.visible_lines()[0]), "l16");

        t.scroll_display(3);
        assert_eq!(t.display_offset(), 3);
        assert_eq!(line_text(&t.visible_lines()[0]), "l13");
        assert_eq!(t.visible_lines().len(), 5, "视口行数不因滚动改变");
    }

    #[test]
    fn scroll_display_clamps_to_history_len() {
        let mut t = scrolled_term();
        t.scroll_display(1000);
        assert_eq!(t.display_offset(), 16, "最多滚到历史顶部");
        assert_eq!(line_text(&t.visible_lines()[0]), "l0");
        assert_eq!(t.history_len(), 16);
    }

    #[test]
    fn scroll_to_bottom_resets_offset() {
        let mut t = scrolled_term();
        t.scroll_display(5);
        t.scroll_to_bottom();
        assert_eq!(t.display_offset(), 0);
        assert_eq!(line_text(&t.visible_lines()[0]), "l16");
    }

    #[test]
    fn new_output_keeps_scrolled_view_pinned() {
        let mut t = scrolled_term();
        t.scroll_display(3);
        let _ = t.feed(b"x\r\n");
        // alacritty 语义：回看时新输出把 offset 顶上去，视口内容不动。
        assert_eq!(t.display_offset(), 4);
        assert_eq!(line_text(&t.visible_lines()[0]), "l13");
    }

    #[test]
    fn cjk_cells_carry_wide_and_spacer_flags() {
        let mut t = TerminalModel::new(40, 10);
        let _ = t.feed("你a".as_bytes());
        let l = &t.visible_lines()[0];
        assert!(l[0].wide && !l[0].spacer, "宽字符本体格应标 wide");
        assert!(l[1].spacer, "宽字符第二格应标 spacer");
        assert_eq!(l[2].ch, 'a');
        assert!(!l[2].wide && !l[2].spacer, "普通格不应带宽字符标记");
    }

    #[test]
    fn decckm_toggles_app_cursor_mode() {
        let mut t = TerminalModel::new(40, 10);
        assert!(!t.app_cursor_mode());
        let _ = t.feed(b"\x1b[?1h");
        assert!(t.app_cursor_mode());
        let _ = t.feed(b"\x1b[?1l");
        assert!(!t.app_cursor_mode());
    }

    /// 防漂移锚：浅色 ANSI16 必须和 `design/浅色配色表.html` 的终端色板
    /// 提案表逐项一致，且切换随 `byteui::theme::color::current_scheme()`
    /// 立即生效，不需要重开终端。
    #[test]
    fn ansi16_color_reflects_light_scheme() {
        let _guard = lock_scheme();
        byteui::theme::color::set_scheme(byteui::theme::color::ColorScheme::Light);
        assert_eq!(ansi16_color(0), Some((0xfc, 0xfd, 0xfe))); // Black ≈ term_bg
        assert_eq!(ansi16_color(1), Some((0xd1, 0x48, 0x3f))); // Red
        assert_eq!(ansi16_color(3), Some((0xc9, 0xa2, 0x27))); // Yellow(浅色版不再与 gold 同值)
        byteui::theme::color::set_scheme(byteui::theme::color::ColorScheme::Dark);
        assert_eq!(ansi16_color(0), Some((0x08, 0x14, 0x1d)));
    }

    #[test]
    fn default_fg_reflects_light_scheme() {
        let _guard = lock_scheme();
        byteui::theme::color::set_scheme(byteui::theme::color::ColorScheme::Light);
        assert_eq!(default_fg_rgb(), (0x36, 0x42, 0x4e));
        byteui::theme::color::set_scheme(byteui::theme::color::ColorScheme::Dark);
        assert_eq!(default_fg_rgb(), (0x9A, 0xB4, 0xC4));
    }

    /// opencode 靠 OSC 10/11 查询探测终端真实前景/背景色来决定自己的
    /// "system" 主题(实测 PTY 抓包确认);dozer 默认不回应任何会话的这类
    /// 查询,除非显式开启——避免误伤没有这个问题的其它 agent。
    #[test]
    fn dynamic_color_query_ignored_by_default() {
        let mut t = TerminalModel::new(40, 10);
        let responses = t.feed(b"\x1b]11;?\x07");
        assert!(responses.is_empty());
    }

    #[test]
    fn dynamic_color_query_answers_background_when_enabled() {
        let _guard = lock_scheme();
        byteui::theme::color::set_scheme(byteui::theme::color::ColorScheme::Dark); // 与「默认即 Dark」的假设对齐,显式写一遍避免依赖执行顺序
        let mut t = TerminalModel::new(40, 10);
        t.set_answer_dynamic_color(true);
        let responses = t.feed(b"\x1b]11;?\x07");
        assert_eq!(responses, b"\x1b]11;rgb:0808/1414/1d1d\x07");
    }

    #[test]
    fn dynamic_color_query_answers_foreground_when_enabled() {
        let _guard = lock_scheme();
        byteui::theme::color::set_scheme(byteui::theme::color::ColorScheme::Dark);
        let mut t = TerminalModel::new(40, 10);
        t.set_answer_dynamic_color(true);
        let responses = t.feed(b"\x1b]10;?\x07");
        assert_eq!(responses, b"\x1b]10;rgb:9a9a/b4b4/c4c4\x07");
    }

    #[test]
    fn dynamic_color_query_reflects_light_scheme_when_enabled() {
        let _guard = lock_scheme();
        byteui::theme::color::set_scheme(byteui::theme::color::ColorScheme::Light);
        let mut t = TerminalModel::new(40, 10);
        t.set_answer_dynamic_color(true);
        let responses = t.feed(b"\x1b]11;?\x07");
        byteui::theme::color::set_scheme(byteui::theme::color::ColorScheme::Dark);
        assert_eq!(responses, b"\x1b]11;rgb:fcfc/fdfd/fefe\x07");
    }
}
