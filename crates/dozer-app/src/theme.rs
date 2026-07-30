// crates/dozer-app/src/theme.rs
//! ByteBoy2077 主题常量。这 14 个颜色值来自设计规格，禁止改动——
//! 改动请回到规格文档 (Global Constraints 表) 重新核对再同步。
//!
//! 本任务（P1c T3）的四栏骨架只用到其中几个常量；其余（CARD/BODY/
//! DIM/GOLD/CYAN/GREEN/PURPLE/RED）是给终端渲染、AI 面板等后续任务
//! （T4-T6）用的，先整体建好接口，因此这里放行 dead_code。
#![allow(dead_code)]
use iced_widget::core::Color;

const fn c(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgb8(r, g, b)
}

pub const BG: Color = c(0x0a, 0x0e, 0x16);
pub const PANEL: Color = c(0x0a, 0x0e, 0x16);
pub const TERM_BG: Color = c(0x08, 0x14, 0x1d);
pub const CARD: Color = c(0x12, 0x20, 0x2a);
pub const BORDER: Color = c(0x1c, 0x34, 0x40);
pub const CREAM: Color = c(0xFF, 0xE5, 0xB4);
pub const BODY: Color = c(0x9A, 0xB4, 0xC4);
pub const DIM: Color = c(0x6B, 0x7F, 0x8F);
pub const GOLD: Color = c(0xF2, 0xD9, 0x4E);
pub const CYAN: Color = c(0x47, 0xDE, 0xF0);
pub const GREEN: Color = c(0x1A, 0xD5, 0x85);
pub const PURPLE: Color = c(0x95, 0x80, 0xFF);
pub const RED: Color = c(0xFF, 0x6E, 0x6E);

#[cfg(test)]
mod tests {
    use super::*;

    /// 防漂移锚：BG 是四栏骨架里最基础的背景色，直接核对 rgb8 -> f32
    /// 换算结果，避免后续有人"顺手"改动常量表而没人发现。
    #[test]
    fn bg_matches_rgb8_conversion() {
        assert_eq!(BG.r, 0x0a as f32 / 255.0);
        assert_eq!(BG.g, 0x0e as f32 / 255.0);
        assert_eq!(BG.b, 0x16 as f32 / 255.0);
        assert_eq!(BG.a, 1.0);
    }

    #[test]
    fn cream_matches_rgb8_conversion() {
        assert_eq!(CREAM.r, 0xFF as f32 / 255.0);
        assert_eq!(CREAM.g, 0xE5 as f32 / 255.0);
        assert_eq!(CREAM.b, 0xB4 as f32 / 255.0);
        assert_eq!(CREAM.a, 1.0);
    }
}
