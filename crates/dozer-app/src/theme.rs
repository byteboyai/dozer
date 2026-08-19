//! 设计 token 系统入口——区域样式(`region`)、终端字号(`terminal_font`)、
//! 几何常量(`geometry`)。`region`/`geometry` 两个子模块编译期内嵌同一份
//! `assets/theme/workspace.json`,各自只解析自己关心的顶层字段;
//! `terminal_font` 内嵌 `assets/theme/terminal.json`。颜色/字号/图标尺寸
//! token 已分别迁移到 `byteui::theme::color`/`font`/`icon_size`(几何的 32
//! 个基础 accessor 也已迁到 `byteui::theme::geometry`,本地只留 3 个文件树
//! 专用的 `tree_*` 函数)。

use std::path::PathBuf;

/// `byteui::theme::icon_size` 的 `init_scale`/`persist_scale`/`reset_scale`
/// 需要调用方传入落盘路径(`byteui` 不内置 Dozer 专属路径约定)。
pub(crate) fn ui_scale_path() -> PathBuf {
    dozer_core::paths::config_dir().join("ui_scale.json")
}

pub mod geometry;
pub mod homespace_color;
pub mod homespace_font;
pub mod region;
pub mod terminal_font;
