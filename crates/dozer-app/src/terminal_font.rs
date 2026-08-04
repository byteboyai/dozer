//! 终端字号 / 行高 token 化：原先硬编码在 `term_view.rs` 顶部的
//! `FONT_SIZE` / `LINE_HEIGHT_FACTOR` 两个常量，收敛成编译期内嵌
//! `assets/theme/terminal.json` 的具名配置，启动时解析一次。
//!
//! 与 `workspace_font.rs`（管 UI 控件文字字号，整数字号档位）和
//! `chrome_style.rs`（管区域外层容器，`regions.json`）职责分离——这里
//! 专门管终端 pane 的等宽字体：字号是浮点（逻辑像素），行高用相对字号
//! 的倍数，二者都从 JSON 读，方便对齐外部编辑器（如 RustRover 的
//! 14px / 行距 1.0）。解析失败（格式错误、缺字段）直接 panic：开发期
//! 配置错误，不是需要优雅降级的运行时数据（同 `chrome_style.rs` /
//! `workspace_font.rs` 的定位）。
use serde::Deserialize;
use std::sync::LazyLock;

const RAW: &str = include_str!("../assets/theme/terminal.json");

#[derive(Deserialize)]
struct TerminalFont {
    /// 终端字号（逻辑像素）。
    size: f32,
    /// 行高倍数（相对字号），1.0 = 无额外行距。
    line_height_factor: f32,
}

fn load(raw: &str) -> TerminalFont {
    serde_json::from_str(raw).expect("terminal.json 格式错误(解析失败)")
}

static FONT: LazyLock<TerminalFont> = LazyLock::new(|| load(RAW));

/// 终端字号（逻辑像素）。
pub fn size() -> f32 {
    FONT.size
}

/// 行高倍数（相对字号）。
pub fn line_height_factor() -> f32 {
    FONT.line_height_factor
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 防漂移锚：终端字号必须和 RustRover 编辑器对齐——JetBrains Mono
    /// 14px、行距 1.0。改前是硬编码 15.0 / 1.4。
    #[test]
    fn matches_rustrover_editor_font() {
        assert_eq!(size(), 14.0);
        assert_eq!(line_height_factor(), 1.0);
    }

    #[test]
    #[should_panic(expected = "terminal.json 格式错误")]
    fn malformed_json_panics() {
        load(r#"{"size": 14.0}"#);
    }
}
