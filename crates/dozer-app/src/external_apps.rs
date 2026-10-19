//! "用外部软件打开"配置表:实现已搬到共享库 `dozer-panel-files`(S5b-1),
//! 这里只保留 `crate::external_apps::*` 的路径转调,并把配置目录定在本产品。
pub use dozer_panel_files::external_apps::{
    ExternalAppsConfig, OpenError, open_failure_message, run_open_command,
};

use dozer_panel_files::external_apps as lib;

fn config_dir() -> std::path::PathBuf {
    dozer_core::paths::config_dir()
}

/// 启动时读一次进 `App::external_apps`(不在每帧 `view()` 里读盘)。
pub fn load() -> ExternalAppsConfig {
    lib::load(&config_dir())
}

#[allow(dead_code)] // 本期无管理 UI,`save` 暂无调用方;保留给下一期。
pub fn save(config: &ExternalAppsConfig) -> std::io::Result<()> {
    lib::save(&config_dir(), config)
}
