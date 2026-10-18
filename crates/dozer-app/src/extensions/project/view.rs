//! 项目面板的 view:实现与渲染已搬进共享库 `dozer-panel-project`;
//! 这里只做重导出,Dozer 的调用点(`app/view.rs`、两个 overlay)不变。
//! 悬停进度的产品取值见 `crate::host_adapters::project_panel::pane_hover`。

pub use dozer_panel_project::view::*;
