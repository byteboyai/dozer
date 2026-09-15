//! 设计 token 系统入口——区域样式(`region`)、终端字号(`terminal_font`)、
//! 几何常量(`geometry`)。`region`/`geometry` 两个子模块编译期内嵌同一份
//! `assets/theme/workspace.json`,各自只解析自己关心的顶层字段;
//! `terminal_font` 内嵌 `assets/theme/terminal.json`。颜色/字号/图标尺寸/基础
//! 几何 token 由 `byteui::theme` 持有,本模块用 `init()` 在启动时把这份
//! `assets/theme/workspace.json`(唯一真相源)灌进 `byteui` 三个 token 模块,
//! 取代它们编译期内置的 ByteBoy2077 默认值(本地只留 3 个文件树专用的
//! `geometry::tree_*` 函数)。

use std::path::PathBuf;

/// `byteui::theme::icon_size` 的 `init_scale`/`persist_scale`/`reset_scale`
/// 需要调用方传入落盘路径(`byteui` 不内置 Dozer 专属路径约定)。
pub(crate) fn ui_scale_path() -> PathBuf {
    dozer_core::paths::config_dir().join("ui_scale.json")
}

/// `byteui::theme::color` 的 `init_scheme`/`persist_scheme` 需要调用方传入
/// 落盘路径,同 `ui_scale_path` 的定位。
pub(crate) fn color_theme_path() -> PathBuf {
    dozer_core::paths::config_dir().join("color_theme.json")
}

const WORKSPACE_JSON: &str = include_str!("../assets/theme/workspace.json");

#[derive(serde::Deserialize)]
struct RawWorkspaceFile {
    font_sizes: byteui::theme::font::FontTokens,
    geometry: byteui::theme::geometry::GeometryTokens,
    icon_sizes: byteui::theme::icon_size::IconSizeTokens,
}

/// 启动时把 dozer-app 自己的 `workspace.json` 灌进 byteui 三个 token
/// 模块,取代它们编译期内置的 ByteBoy2077 默认值。必须在建窗、任何
/// 渲染逻辑跑之前调用一次。解析失败(格式错误、缺字段)直接 panic——
/// 开发期配置错误,不是需要优雅降级的运行时数据(同 `region.rs`/
/// `terminal_font.rs` 一贯的定位)。
pub(crate) fn init() {
    let raw: RawWorkspaceFile =
        serde_json::from_str(WORKSPACE_JSON).expect("workspace.json 格式错误(解析失败)");
    byteui::theme::font::set_theme(raw.font_sizes);
    byteui::theme::geometry::set_theme(raw.geometry);
    byteui::theme::icon_size::set_theme(raw.icon_sizes);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_parses_and_applies_real_workspace_json() {
        init();
        assert_eq!(byteui::theme::font::current().body, 14);
        assert_eq!(byteui::theme::geometry::current().icon_rail_width, 44.0);
        assert_eq!(byteui::theme::icon_size::current().rail, 16.0);
    }

    #[test]
    #[should_panic(expected = "workspace.json 格式错误")]
    fn init_panics_on_malformed_json() {
        let raw = "{not valid json";
        let _: RawWorkspaceFile =
            serde_json::from_str(raw).expect("workspace.json 格式错误(解析失败)");
    }
}

pub mod geometry;
pub mod homespace_color;
pub mod homespace_font;
pub mod region;
pub mod terminal_font;
