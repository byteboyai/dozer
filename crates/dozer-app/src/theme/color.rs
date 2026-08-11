// crates/dozer-app/src/theme.rs
//! ByteBoy2077 主题常量。这 14 个颜色值来自设计规格，禁止改动——
//! 改动请回到规格文档 (Global Constraints 表) 重新核对再同步。
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

/// 文件树被 `.gitignore` 忽略的条目的名称颜色(`#6B7F8F`,与 `DIM` 同值)。
/// 中性的低饱和灰,比正文 `BODY` 更弱一号,示意"存在但不在版本控制视线
/// 内"。不在设计规格锁死的 14 色之内,作为增量具名令牌加在此,不动上面
/// 锁定的颜色。
pub const IGNORED: Color = c(0x6B, 0x7F, 0x8F);

/// Agent-identity 扩展色（Codex/Qoder/Kilo/V8agent 的 picker 圆点/图标底色）。
/// ByteBoy2077 核心 5 色（bg/金/奶油/青/绿）之外，`PURPLE` 等已是先例。
pub const ORANGE: Color = c(0xFF, 0x9B, 0x4D);
pub const MAGENTA: Color = c(0xFF, 0x6E, 0xC7);
pub const BLUE: Color = c(0x4D, 0x8C, 0xFF);
pub const LIME: Color = c(0xA3, 0xE6, 0x35);

/// 两色按 `t`(0..=1)线性插值,返回中间色。iced 0.14 的 `Color` 没有自带的
/// `lerp`/`mix`,这里补一个给悬停动画等需要平滑过渡颜色的地方用。
/// `t` 超出 [0,1] 不外夹,调用方保证区间。
pub fn mix(a: Color, b: Color, t: f32) -> Color {
    Color {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a + (b.a - a.a) * t,
    }
}

/// 放大态浮层的变暗遮罩色(半透明黑)。不在设计规格锁死的 14 色之内——
/// 之前是 `maximize_overlay` 函数里的游离字面量,这里给它转正成具名令牌,
/// 值不变,纯增量,不改动上面 14 个锁定颜色。
pub const SCRIM: Color = Color {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 0.55,
};

/// 顶栏选中项目页签的边框色(`#dcc9a3`)。不在设计规格锁死的 14 色之内——
/// 是顶栏页签"当前选中"态的专属描边,作为增量具名令牌加在此,不动上面
/// 的锁定颜色。
pub const TAB_ACTIVE_BORDER: Color = c(0xDC, 0xC9, 0xA3);

/// 顶栏选中项目页签的实底背景色(`#152630`)。与 `TAB_ACTIVE_BORDER` 同属
/// 顶栏页签专属增量令牌,不动上面锁死的 14 色。
pub const TAB_ACTIVE_BG: Color = c(0x15, 0x26, 0x30);

/// 顶栏未选中项目页签 hover 态的背景色(`#152630`)。与
/// `TAB_ACTIVE_BORDER` 同属顶栏页签专属增量令牌,不动上面锁死的 14 色。
pub const TAB_HOVER: Color = c(0x15, 0x26, 0x30);

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

    #[test]
    fn scrim_is_half_transparent_black() {
        assert_eq!(SCRIM.r, 0.0);
        assert_eq!(SCRIM.g, 0.0);
        assert_eq!(SCRIM.b, 0.0);
        assert_eq!(SCRIM.a, 0.55);
    }
}
