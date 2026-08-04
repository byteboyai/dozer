//! 字体 token 化：把散落在各处的字号字面量收敛成具名 token，编译期内嵌
//! `assets/theme/fonts.json`，启动时解析一次。
//!
//! JSON 分两个节点：
//! - `workspace`：UI 控件内部文字的整数字号档位（`dot_xs`..`title`，8–15），
//!   供 `workspace.rs` 里 `text(...).size(N)` 调用点使用；
//! - `terminal`：终端 pane 的等宽字体（浮点 `size` + 相对行高
//!   `line_height_factor`），供 `term_view.rs` 使用，对齐外部编辑器
//!   （如 RustRover 的 14px / 行距 1.0）。
//!
//! 与 `chrome_style.rs`（管区域外层容器的背景/边框/间距，`regions.json`）
//! 职责严格分离——这里只管"字体"，不越界。解析失败（格式错误、缺字段）
//! 直接 panic：开发期配置错误，不是需要优雅降级的运行时数据（同
//! `chrome_style.rs` 的定位）。
use serde::Deserialize;
use std::sync::LazyLock;

const RAW: &str = include_str!("../assets/theme/fonts.json");

#[derive(Deserialize)]
struct WorkspaceFonts {
    dot_xs: u32,
    dot_sm: u32,
    caption_sm: u32,
    caption: u32,
    label: u32,
    body: u32,
    subtitle: u32,
    title: u32,
}

#[derive(Deserialize)]
struct TerminalFont {
    /// 终端字号（逻辑像素）。
    size: f32,
    /// 行高倍数（相对字号），1.0 = 无额外行距。
    line_height_factor: f32,
}

#[derive(Deserialize)]
struct Fonts {
    workspace: WorkspaceFonts,
    terminal: TerminalFont,
}

fn load(raw: &str) -> Fonts {
    serde_json::from_str(raw).expect("fonts.json 格式错误(解析失败)")
}

static FONTS: LazyLock<Fonts> = LazyLock::new(|| load(RAW));

// ---- workspace：UI 控件文字字号（整数字号档位） ----

pub fn dot_xs() -> u32 {
    FONTS.workspace.dot_xs
}
pub fn dot_sm() -> u32 {
    FONTS.workspace.dot_sm
}
pub fn caption_sm() -> u32 {
    FONTS.workspace.caption_sm
}
pub fn caption() -> u32 {
    FONTS.workspace.caption
}
pub fn label() -> u32 {
    FONTS.workspace.label
}
pub fn body() -> u32 {
    FONTS.workspace.body
}
pub fn subtitle() -> u32 {
    FONTS.workspace.subtitle
}
pub fn title() -> u32 {
    FONTS.workspace.title
}

// ---- terminal：终端等宽字体（浮点字号 + 相对行高） ----

/// 终端字号（逻辑像素）。
pub fn terminal_size() -> f32 {
    FONTS.terminal.size
}

/// 终端行高倍数（相对字号）。
pub fn terminal_line_height_factor() -> f32 {
    FONTS.terminal.line_height_factor
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 防漂移锚:8 个 UI token 的解析结果必须和改动前 workspace.rs 里的
    /// 字面量完全一致——纯代码搬家,数值不该变。
    #[test]
    fn workspace_tokens_match_pre_migration_literals() {
        assert_eq!(dot_xs(), 8);
        assert_eq!(dot_sm(), 9);
        assert_eq!(caption_sm(), 10);
        assert_eq!(caption(), 11);
        assert_eq!(label(), 12);
        assert_eq!(body(), 13);
        assert_eq!(subtitle(), 14);
        assert_eq!(title(), 15);
    }

    /// 防漂移锚:终端字号必须和 RustRover 编辑器对齐——JetBrains Mono
    /// 14px、行距 1.0。改前是硬编码 15.0 / 1.4。
    #[test]
    fn terminal_matches_rustrover_editor_font() {
        assert_eq!(terminal_size(), 14.0);
        assert_eq!(terminal_line_height_factor(), 1.0);
    }

    #[test]
    #[should_panic(expected = "fonts.json 格式错误")]
    fn malformed_json_panics() {
        load(r#"{"workspace": {"dot_xs": 8}}"#);
    }
}
