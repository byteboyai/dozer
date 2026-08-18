//! 设计 token 系统入口——区域样式(`region`)、UI 字号
//! (`font`)、终端字号(`terminal_font`)、图标尺寸(`icon_size`)、几何常量
//! (`geometry`)。`region`/`font`/`icon_size`/`geometry` 四个子模块编译期
//! 内嵌同一份 `assets/theme/workspace.json`,各自只解析自己关心的顶层
//! 字段;`terminal_font` 内嵌 `assets/theme/terminal.json`。颜色 token 已
//! 迁移到 `byteui::theme::color`,本地不再有 `color` 子模块。

pub mod font;
pub mod geometry;
pub mod homespace_color;
pub mod homespace_font;
pub mod icon_size;
pub mod region;
pub mod terminal_font;
