//! 设置弹窗的「应用」页:状态机已搬到界面框架无关的 `bytehost_panel::install`(A7 task 5),
//! 这里只做 re-export 与 dozer 侧的适配(状态别名、iced 视图入口)。
//!
//! dozer 侧仍用 `State` 指代 `InstallFlowState`,调用点不动;视图在
//! [`settings_apps_view`](super::settings_apps_view)。

pub use bytehost_panel::install::*;

/// dozer 侧的惯用别名(调用点历史保留)。
pub type State = InstallFlowState;

pub use super::settings_apps_view::view;
