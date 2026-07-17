// crates/dozer-app/src/term_model.rs
//! TerminalModel：headless 终端状态机（P1c T4）。
//!
//! 包装 `alacritty_terminal` 的 VTE 解析器与 `Term` 网格状态，向渲染层
//! （T5）暴露一份纯数据快照（`Cell` 网格 + 光标位置），不依赖 iced，避免
//! 渲染 crate 反向渗入本模块。
//!
//! T5 消费：`Cell`、`TerminalModel` 及其全部方法目前尚无调用方（渲染层未
//! 接线），下方 `#![allow(dead_code)]` 是过渡期占位，等 T5 接上渲染后可去掉。
#![allow(dead_code)]

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::{Color as AnsiColor, NamedColor, Processor};

/// ANSI 16 色的主题 RGB 表，下标与 `NamedColor` 的判别值（0..=15）一致：
/// Black/Red/Green/Yellow/Blue/Magenta/Cyan/White，随后是对应的 Bright 变体。
const ANSI16: [(u8, u8, u8); 16] = [
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

/// 默认前景色（无显式 SGR 时的字符颜色）。
const DEFAULT_FG: (u8, u8, u8) = (0x9A, 0xB4, 0xC4);

/// 渲染层唯一数据源：一个终端网格格子。刻意只含原始值（`char`/`(u8,u8,u8)`），
/// 不引入 `iced::Color`，保持本模块可在无渲染依赖下单测。
#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    pub ch: char,
    pub fg: (u8, u8, u8),
    pub bg: Option<(u8, u8, u8)>,
    pub bold: bool,
}

/// `Term::new`/`Term::resize` 需要的最小尺寸描述。本模块不做历史回滚
/// （scrollback），因此 `total_lines` 等于可视行数。
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
    if idx < ANSI16.len() {
        return Some(ANSI16[idx]);
    }
    match name {
        NamedColor::Background => None,
        _ => Some(DEFAULT_FG),
    }
}

/// ANSI-256 索引色 → RGB：0..16 复用 16 色主题表；16..232 是标准 xterm
/// 6x6x6 色立方；232..256 是灰阶渐变。P1c 范围内的 ANSI 转义序列不会
/// 触发这一段，仅为完整性保留标准 xterm 映射。
fn indexed_to_rgb(idx: u8) -> (u8, u8, u8) {
    match idx {
        0..=15 => ANSI16[idx as usize],
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
        AnsiColor::Named(name) => named_color_rgb(name).unwrap_or(DEFAULT_FG),
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

/// headless 终端状态机：字节流喂给 VTE 解析器，驱动 `alacritty_terminal`
/// 的 `Term` 网格状态；对外只暴露只读快照（`visible_lines`/`cursor`）。
pub struct TerminalModel {
    term: Term<VoidListener>,
    parser: Processor,
}

impl TerminalModel {
    /// 新建一个 `cols` x `rows` 的终端，无 scrollback。
    pub fn new(cols: u16, rows: u16) -> Self {
        let size = TermSize {
            columns: cols as usize,
            screen_lines: rows as usize,
        };
        let term = Term::new(Config::default(), &size, VoidListener);
        Self {
            term,
            parser: Processor::new(),
        }
    }

    /// 把字节流喂给 VTE 解析器，解析结果直接落到 `Term` 的网格状态上。
    pub fn feed(&mut self, bytes: &[u8]) {
        // `Processor::advance` 需要同时可变借用 parser 与 term；先解构再
        // 分别取字段引用，避免对 `self` 的双重可变借用。
        let Self { term, parser } = self;
        parser.advance(term, bytes);
    }

    /// 调整终端尺寸，尽量保留既有内容（委托给 `Term::resize` 的重排逻辑）。
    pub fn resize(&mut self, cols: u16, rows: u16) {
        let size = TermSize {
            columns: cols as usize,
            screen_lines: rows as usize,
        };
        self.term.resize(size);
    }

    /// 可视网格快照，逐行逐格返回。宽字符的 spacer 格 `ch` 置为空格。
    pub fn visible_lines(&self) -> Vec<Vec<Cell>> {
        let grid = self.term.grid();
        let cols = grid.columns();
        let rows = grid.screen_lines();

        (0..rows)
            .map(|row| {
                let line = &grid[Line(row as i32)];
                (0..cols)
                    .map(|col| {
                        let cell = &line[Column(col)];
                        let ch = if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                            ' '
                        } else {
                            cell.c
                        };
                        Cell {
                            ch,
                            fg: fg_to_rgb(cell.fg),
                            bg: bg_to_rgb(cell.bg),
                            bold: cell.flags.contains(Flags::BOLD),
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

    /// 是否处于 application cursor mode（DECCKM，`CSI ?1h` 开启 /
    /// `CSI ?1l` 关闭）。shell 行编辑器（readline/zle 等）常用它来把方向
    /// 键从 CSI 序列（`\x1b[A`）切换成 SS3 序列（`\x1bOA`），
    /// `keymap::key_to_bytes` 据此决定发哪一种转义序列。
    pub fn app_cursor_mode(&self) -> bool {
        self.term.mode().contains(TermMode::APP_CURSOR)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line_text(cells: &[Cell]) -> String {
        cells
            .iter()
            .map(|c| c.ch)
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    #[test]
    fn plain_text_lands_on_first_row() {
        let mut t = TerminalModel::new(40, 10);
        t.feed(b"hello dozer");
        assert_eq!(line_text(&t.visible_lines()[0]), "hello dozer");
        assert_eq!(t.cursor(), (11, 0));
    }

    #[test]
    fn newline_and_cr_move_cursor() {
        let mut t = TerminalModel::new(40, 10);
        t.feed(b"one\r\ntwo");
        let lines = t.visible_lines();
        assert_eq!(line_text(&lines[0]), "one");
        assert_eq!(line_text(&lines[1]), "two");
        assert_eq!(t.cursor(), (3, 1));
    }

    #[test]
    fn sgr_red_foreground_is_mapped() {
        let mut t = TerminalModel::new(40, 10);
        t.feed(b"\x1b[31mred\x1b[0m");
        let cell = &t.visible_lines()[0][0];
        assert_eq!(cell.ch, 'r');
        assert_eq!(cell.fg, (0xFF, 0x6E, 0x6E)); // ANSI 红 → 主题 RED
    }

    #[test]
    fn resize_keeps_content() {
        let mut t = TerminalModel::new(40, 10);
        t.feed(b"keepme");
        t.resize(60, 20);
        assert_eq!(line_text(&t.visible_lines()[0]), "keepme");
        assert_eq!(t.visible_lines().len(), 20);
    }

    #[test]
    fn utf8_cjk_occupies_two_columns() {
        let mut t = TerminalModel::new(40, 10);
        t.feed("你好".as_bytes());
        let l = &t.visible_lines()[0];
        assert_eq!(l[0].ch, '你');
        assert_eq!(l[2].ch, '好'); // 宽字符占两格，第 1 格为 spacer
        assert_eq!(t.cursor(), (4, 0));
    }

    #[test]
    fn decckm_toggles_app_cursor_mode() {
        let mut t = TerminalModel::new(40, 10);
        assert!(!t.app_cursor_mode());
        t.feed(b"\x1b[?1h");
        assert!(t.app_cursor_mode());
        t.feed(b"\x1b[?1l");
        assert!(!t.app_cursor_mode());
    }
}
