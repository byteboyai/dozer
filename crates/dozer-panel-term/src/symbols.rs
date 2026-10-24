//! 终端里私有区字符(Nerd Font 图标)的专用字体。
//!
//! `ls --icons`、提示符工具等用 Nerd Font 的图标字符,它们在 Unicode 私有区。主字体
//! (JetBrains Mono)没有这些字形,交给系统回退时,macOS 的中文字体(Songti 等)在
//! U+E000 附近放着 GB18030 自定义字,会抢先命中,把图标画成不相关的汉字;有没有装
//! Nerd Font 也因机器而异。所以私有区字符固定用这里内置的只含符号的字体画(见
//! `view::layout_runs`),不靠回退顺序。
//!
//! 主字体仍由调用方注册并传入(`TermStyle::font`);这个符号字体按字符区段限定用途,
//! 由库内置,调用方只需在启动时把 [`SYMBOLS_FONT_DATA`] 注册进字体库。

use iced_widget::core::Font;

/// 内置的 Nerd Font 符号字体(MIT,见 `assets/SymbolsNerdFontMono-LICENSE.txt`),
/// 字族名与字体文件内声明的一致。等宽版:每个字符占一格。
pub const SYMBOLS_FONT_FAMILY: &str = "Symbols Nerd Font Mono";

/// 字体文件字节,由产品在第一次排版前注册进自己的字体库。
pub const SYMBOLS_FONT_DATA: &[u8] = include_bytes!("../assets/SymbolsNerdFontMono-Regular.ttf");

/// 符号字体的 iced 描述符。
pub fn font() -> Font {
    Font::with_name(SYMBOLS_FONT_FAMILY)
}

/// 是否私有区字符:BMP 私有区、补充私有区 A(Nerd Font v3 的 Material Design 图标)与 B。
pub fn is_private_use(c: char) -> bool {
    matches!(
        c as u32,
        0xE000..=0xF8FF | 0xF0000..=0xFFFFD | 0x100000..=0x10FFFD
    )
}

/// 符号字体每个字形的横向宽度(单位 em):等宽版的字形宽就是 1 em,比终端格(约 0.6 em)宽。
pub const SYMBOLS_ADVANCE_EM: f32 = 1.0;

/// 把符号字形均匀缩小到恰好占一个终端格的比例;格比字形宽时不放大(上限 1)。
pub fn scale_for_cell(cell_width: f32, font_size: f32) -> f32 {
    (cell_width / (font_size * SYMBOLS_ADVANCE_EM)).clamp(0.0, 1.0)
}
