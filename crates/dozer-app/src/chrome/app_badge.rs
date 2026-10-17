//! 已安装应用在图标栏里的首字母徽标(验收:所有应用入口都是同一个通用列表图标,分不清谁是谁)。
//!
//! 产品策略在这里(纯函数,好测):取哪个字符、用哪种底色。画法在 `byteui::interaction::icons`
//! (`glyph_button_entry`,与普通图标按钮共用同一个外壳)。

use byteui::theme::color::ColorTokens;
use iced_widget::core::Color;

/// 徽标上的字符:标题里第一个非空白字符;ASCII 字母转大写(`py-notes` → `P`),中文取第一个字;
/// 空标题回落到 `?`。
pub(crate) fn badge_glyph(title: &str) -> char {
    title
        .chars()
        .find(|c| !c.is_whitespace())
        .map(|c| c.to_ascii_uppercase())
        .unwrap_or('?')
}

/// FNV-1a(32 位):稳定、与平台无关的字符串哈希。`std` 的 `DefaultHasher` 不保证跨版本稳定,
/// 而徽标底色必须"同一个应用永远同一种颜色"。
fn fnv1a(s: &str) -> u32 {
    let mut h = 0x811c_9dc5u32;
    for b in s.bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

/// 徽标底色:按应用 id 的哈希在主题的 8 个强调色里选一个。不用 `gold`(留给选中框/甲方动作)和
/// `red`(错误语义)。
pub(crate) fn badge_fill(tokens: &ColorTokens, app_id: &str) -> Color {
    let palette = [
        tokens.cyan,
        tokens.green,
        tokens.purple,
        tokens.orange,
        tokens.magenta,
        tokens.blue,
        tokens.lime,
        tokens.teal,
    ];
    palette[fnv1a(app_id) as usize % palette.len()]
}

/// 底色上的字符色:在主题的主文字色(`cream`)与背景色(`bg`)里选对比度更高的。
pub(crate) fn badge_ink(tokens: &ColorTokens, fill: Color) -> Color {
    byteui::interaction::icons::readable_ink(fill, tokens.cream, tokens.bg)
}

#[cfg(test)]
mod tests {
    use super::{badge_fill, badge_glyph, badge_ink};
    use byteui::theme::color::ColorTokens;
    use iced_widget::core::Color;

    fn luminance(c: Color) -> f32 {
        let lin = |v: f32| {
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
    }

    fn contrast(a: Color, b: Color) -> f32 {
        let (la, lb) = (luminance(a), luminance(b));
        (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
    }

    #[test]
    fn glyph_rules() {
        assert_eq!(badge_glyph("Py Notes"), 'P');
        assert_eq!(badge_glyph("py-notes"), 'P', "英文首字母大写");
        assert_eq!(badge_glyph("双语实时"), '双', "中文取第一个字");
        assert_eq!(badge_glyph("  node"), 'N', "跳过前导空白");
        assert_eq!(badge_glyph("12 apps"), '1');
        assert_eq!(badge_glyph(""), '?');
        assert_eq!(badge_glyph("   "), '?', "全是空白也回落到 ?");
    }

    #[test]
    fn the_same_app_id_always_gets_the_same_fill() {
        let t = ColorTokens::byteboy2077();
        assert_eq!(badge_fill(&t, "py-notes"), badge_fill(&t, "py-notes"));
    }

    #[test]
    fn different_ids_spread_over_the_palette() {
        let t = ColorTokens::byteboy2077();
        let ids = [
            "py-notes",
            "py-crashy",
            "py-ver",
            "py-badver",
            "py-chatty",
            "py-zip",
            "node-notes",
            "node-badlock",
            "bilingual-live",
            "excalidraw",
        ];
        let mut fills: Vec<[u32; 3]> = ids
            .iter()
            .map(|id| {
                let c = badge_fill(&t, id);
                [
                    (c.r * 255.0) as u32,
                    (c.g * 255.0) as u32,
                    (c.b * 255.0) as u32,
                ]
            })
            .collect();
        fills.sort_unstable();
        fills.dedup();
        assert!(fills.len() >= 3, "10 个 id 只落在 {} 种底色上", fills.len());
    }

    #[test]
    fn ink_is_readable_on_every_fill_in_both_themes() {
        for (name, t) in [
            ("dark", ColorTokens::byteboy2077()),
            ("light", ColorTokens::byteboy2077_light()),
        ] {
            for i in 0..200 {
                let fill = badge_fill(&t, &format!("app-{i}"));
                let ratio = contrast(fill, badge_ink(&t, fill));
                assert!(ratio >= 3.0, "{name}: app-{i} 对比度 {ratio:.2} < 3.0");
            }
        }
    }
}
