//! 项目信息(Project)面板:项目名、git 分支/脏标、验收次数。阶段 1 扩展化
//! 重构项目,设计见 `docs/superpowers/specs/2026-08-13-project-info-pane-v2-design.md`。
//!
//! 状态(`WorkspaceState`)与更新逻辑(`update`、异步发起函数)已搬进共享库
//! `dozer-panel-project`;这里只做重导出,原有 `project::X` 调用点不变。

pub mod delete;
pub mod view;

pub use dozer_panel_project::update::*;
pub use dozer_panel_project::{
    BackfillStepState, CaptureDescriptionEditFocus, CaptureNameEditFocus, MemoryDraft, Message,
    ProjectToolbarTarget, ScaffoldStepState, WorkspaceState, description_field_id, name_field_id,
    take_description_edit_focused, take_name_edit_focused,
};
pub use view::*;

/// 纯逻辑(链接发现、脚手架步骤、描述文件、尺寸统计)已搬进共享库
/// `dozer-panel-project`;这里重新导出,原有 `project::links::…` 调用点不变。
pub use dozer_panel_project::disk::{DISK_USAGE_EXCLUDE, compute_blank_info, dir_size_excluding};
pub use dozer_panel_project::i18n::{t, t_args};
pub use dozer_panel_project::links;
pub use dozer_panel_project::scaffold;

dozer_core::scope!(pub(crate) LOG, panel, "project");
