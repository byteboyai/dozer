//! ByteBoy2077 是编译期默认值,`set_theme` 可在运行时整体替换成另一份
//! 产品的取值——组件内部一律读 `current()`,不再有硬编码色值常量。

use iced_widget::core::Color;
use std::sync::RwLock;

const fn c(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgb8(r, g, b)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorTokens {
    pub bg: Color,
    pub panel: Color,
    pub term_bg: Color,
    pub card: Color,
    pub border: Color,
    pub cream: Color,
    pub body: Color,
    pub dim: Color,
    pub gold: Color,
    pub cyan: Color,
    pub green: Color,
    pub purple: Color,
    pub red: Color,
    pub ignored: Color,
    pub orange: Color,
    pub magenta: Color,
    pub blue: Color,
    pub lime: Color,
    pub scrim: Color,
    pub tab_active_border: Color,
    pub tab_active_bg: Color,
    pub tab_hover: Color,
    pub desc_bg: Color,
}

impl ColorTokens {
    /// 逐一对应 `crates/dozer-app/src/theme/color.rs` 的锁死值,禁止改动。
    pub const fn byteboy2077() -> Self {
        Self {
            bg: c(0x0a, 0x0e, 0x16),
            panel: c(0x0a, 0x0e, 0x16),
            term_bg: c(0x08, 0x14, 0x1d),
            card: c(0x12, 0x20, 0x2a),
            border: c(0x1c, 0x34, 0x40),
            cream: c(0xFF, 0xE5, 0xB4),
            body: c(0x9A, 0xB4, 0xC4),
            dim: c(0x6B, 0x7F, 0x8F),
            gold: c(0xF2, 0xD9, 0x4E),
            cyan: c(0x47, 0xDE, 0xF0),
            green: c(0x1A, 0xD5, 0x85),
            purple: c(0x95, 0x80, 0xFF),
            red: c(0xFF, 0x6E, 0x6E),
            ignored: c(0x6B, 0x7F, 0x8F),
            orange: c(0xFF, 0x9B, 0x4D),
            magenta: c(0xFF, 0x6E, 0xC7),
            blue: c(0x4D, 0x8C, 0xFF),
            lime: c(0xA3, 0xE6, 0x35),
            scrim: Color { r: 0.0, g: 0.0, b: 0.0, a: 0.55 },
            tab_active_border: c(0xDC, 0xC9, 0xA3),
            tab_active_bg: c(0x15, 0x26, 0x30),
            tab_hover: c(0x15, 0x26, 0x30),
            desc_bg: c(0x15, 0x26, 0x30),
        }
    }
}

static CURRENT: RwLock<ColorTokens> = RwLock::new(ColorTokens::byteboy2077());

/// 当前生效的颜色 token(默认 ByteBoy2077)。
pub fn current() -> ColorTokens {
    *CURRENT.read().expect("byteui color RwLock poisoned")
}

/// 整体替换当前颜色 token——供未来 ByteBoy 产品换主题用,组件代码不用改。
pub fn set_theme(tokens: ColorTokens) {
    *CURRENT.write().expect("byteui color RwLock poisoned") = tokens;
}

/// 两色按 `t`(0..=1)线性插值。`t` 超出 [0,1] 不外夹,调用方保证区间。
pub fn mix(a: Color, b: Color, t: f32) -> Color {
    Color {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a + (b.a - a.a) * t,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byteboy2077_bg_matches_hex() {
        let t = ColorTokens::byteboy2077();
        assert_eq!(t.bg.r, 0x0a as f32 / 255.0);
        assert_eq!(t.bg.g, 0x0e as f32 / 255.0);
        assert_eq!(t.bg.b, 0x16 as f32 / 255.0);
    }

    #[test]
    fn current_defaults_to_byteboy2077() {
        let c = current();
        assert_eq!(c.gold, ColorTokens::byteboy2077().gold);
    }

    #[test]
    fn set_theme_replaces_current_and_is_visible_globally() {
        let mut custom = ColorTokens::byteboy2077();
        custom.gold = Color::from_rgb8(0x00, 0x00, 0x00);
        set_theme(custom);
        assert_eq!(current().gold, Color::from_rgb8(0x00, 0x00, 0x00));
        // 复原,避免污染同进程里跑在本测试之后的其它测试。
        set_theme(ColorTokens::byteboy2077());
    }

    #[test]
    fn mix_at_zero_and_one_returns_endpoints() {
        let a = Color::from_rgb8(0, 0, 0);
        let b = Color::from_rgb8(255, 255, 255);
        assert_eq!(mix(a, b, 0.0), a);
        assert_eq!(mix(a, b, 1.0), b);
    }
}
