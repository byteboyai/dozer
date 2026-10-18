//! 预览 tab 持久化的数据类型与读写已搬进 `dozer-panel-preview::persistence`;
//! 这里只绑定 Dozer 的配置目录。
pub use dozer_panel_preview::{
    PREVIEW_STATE_VERSION, PersistedPreviewTab, PersistedTabular, PreviewPersistence, PreviewState,
};

/// Dozer 的落盘实现:`<config_dir>/preview_state/<project_id>.json`。
pub fn persistence() -> dozer_panel_preview::FilePersistence {
    dozer_panel_preview::FilePersistence {
        config_dir: dozer_core::paths::config_dir(),
    }
}

pub fn load(project_id: i64) -> PreviewState {
    persistence().load(project_id)
}

pub fn save(project_id: i64, state: &PreviewState) -> std::io::Result<()> {
    persistence().save(project_id, state)
}
