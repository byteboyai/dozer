//! 工作区（UI 控件）字号 token 化：`ByteBoy2077` 是编译期默认值，
//! `set_theme` 可在运行时整体替换成另一份产品的取值（同 `theme::color`
//! 的模式）——组件内部一律读 `current()`，不直接引用 `byteboy2077()`。
//!
//! 每个 accessor 返回的字号都乘过 `icon_size::scale()`（全局缩放因子），
//! 因此改 `scale` 即整体缩放全部控件文字，与图标尺寸同步。
use serde::Deserialize;
use std::sync::RwLock;

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct FontTokens {
    pub dot_sm: u32,
    pub caption_sm: u32,
    pub caption: u32,
    pub label: u32,
    pub body: u32,
    pub subtitle: u32,
    pub title: u32,
}

impl FontTokens {
    /// 逐一对应 `dozer-app` 当前 `assets/theme/workspace.json` 的
    /// `font_sizes` 节点，仅作未显式 `set_theme()` 时的兜底默认值。
    pub const fn byteboy2077() -> Self {
        Self {
            dot_sm: 10,
            caption_sm: 11,
            caption: 12,
            label: 13,
            body: 14,
            subtitle: 15,
            title: 16,
        }
    }
}

static CURRENT: RwLock<FontTokens> = RwLock::new(FontTokens::byteboy2077());

/// 当前生效的字号 token（默认 ByteBoy2077）。
pub fn current() -> FontTokens {
    *CURRENT.read().expect("byteui font RwLock poisoned")
}

/// 整体替换当前字号 token——供调用方（如 `dozer-app::theme::init()`）在
/// 启动时用自己的 `workspace.json` 覆盖默认值。
pub fn set_theme(tokens: FontTokens) {
    *CURRENT.write().expect("byteui font RwLock poisoned") = tokens;
}

pub fn dot_sm() -> u32 {
    scale(current().dot_sm)
}
pub fn caption_sm() -> u32 {
    scale(current().caption_sm)
}
pub fn caption() -> u32 {
    scale(current().caption)
}
pub fn label() -> u32 {
    scale(current().label)
}
pub fn body() -> u32 {
    scale(current().body)
}
pub fn subtitle() -> u32 {
    scale(current().subtitle)
}
pub fn title() -> u32 {
    scale(current().title)
}

/// 把设计基准字号按全局 scale 折算成实际像素字号（四舍五入）。
fn scale(base: u32) -> u32 {
    ((base as f32) * super::icon_size::scale()).round() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 防漂移锚：`byteboy2077()` 的每个字段值必须和 `dozer-app` 当前
    /// `assets/theme/workspace.json` 的 `font_sizes` 字面量一致。
    #[test]
    fn byteboy2077_matches_dozer_app_baseline() {
        let t = FontTokens::byteboy2077();
        assert_eq!(t.dot_sm, 10);
        assert_eq!(t.caption_sm, 11);
        assert_eq!(t.caption, 12);
        assert_eq!(t.label, 13);
        assert_eq!(t.body, 14);
        assert_eq!(t.subtitle, 15);
        assert_eq!(t.title, 16);
    }

    #[test]
    fn current_defaults_to_byteboy2077() {
        let c = current();
        assert_eq!(c.body, FontTokens::byteboy2077().body);
    }

    #[test]
    fn set_theme_replaces_current_and_is_visible_globally() {
        let mut custom = FontTokens::byteboy2077();
        custom.body = 99;
        set_theme(custom);
        assert_eq!(current().body, 99);
        // 复原，避免污染同进程里跑在本测试之后的其它测试。
        set_theme(FontTokens::byteboy2077());
    }
}
