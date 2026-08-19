//! 设计 token 系统入口——区域样式(`region`)、UI 字号
//! (`font`)、终端字号(`terminal_font`)、几何常量(`geometry`)。`region`/
//! `font`/`geometry` 三个子模块编译期内嵌同一份
//! `assets/theme/workspace.json`,各自只解析自己关心的顶层字段;
//! `terminal_font` 内嵌 `assets/theme/terminal.json`。颜色 token 已迁移到
//! `byteui::theme::color`,图标尺寸 token 已迁移到 `byteui::theme::icon_size`,
//! 本地不再有 `color`/`icon_size` 子模块。

use std::path::PathBuf;

/// `byteui::theme::icon_size` 的 `init_scale`/`persist_scale`/`reset_scale`
/// 需要调用方传入落盘路径(`byteui` 不内置 Dozer 专属路径约定)。
pub(crate) fn ui_scale_path() -> PathBuf {
    dozer_core::paths::config_dir().join("ui_scale.json")
}

pub mod font;
pub mod geometry;
pub mod homespace_color;
pub mod homespace_font;
pub mod region;
pub mod terminal_font;
