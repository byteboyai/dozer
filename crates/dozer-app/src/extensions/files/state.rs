//! 文件树面板状态已随代码迁入 `dozer-panel-files`(S5b-1)。这里只重新导出,
//! 并保留产品与库之间的类型映射(库不认识 `PanelKind`)。
//!
//! 焦点捕获 `CaptureSearchFocus` / `CaptureTreeEditFocus` 与 `static` 桥接
//! 也随状态一起迁进库——它们只依赖 `WorkspaceState` 自己的 widget id,
//! 与产品无关。

pub(crate) use dozer_panel_files::state::*;
pub use dozer_panel_files::state::DRAG_HOVER_EXPAND_DELAY;
pub use dozer_panel_files::state::{
    CaptureSearchFocus, CaptureTreeEditFocus, move_name_field_id, take_search_focused,
    take_tree_edit_focused, tree_edit_field_id,
};

/// 库的中性预览来源 ↔ Dozer 的 `PanelKind`(S5b-1:库不认识 `PanelKind`)。
pub(crate) fn preview_kind_of(panel: crate::app::PanelKind) -> dozer_panel_files::adapter::PreviewKind {
    match panel {
        crate::app::PanelKind::Project => dozer_panel_files::adapter::PreviewKind::Project,
        _ => dozer_panel_files::adapter::PreviewKind::Files,
    }
}

pub(crate) fn panel_kind_of(kind: dozer_panel_files::adapter::PreviewKind) -> crate::app::PanelKind {
    match kind {
        dozer_panel_files::adapter::PreviewKind::Project => crate::app::PanelKind::Project,
        dozer_panel_files::adapter::PreviewKind::Files => crate::app::PanelKind::Files,
    }
}
