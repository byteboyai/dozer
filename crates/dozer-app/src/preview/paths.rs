//! 预览落盘位置的 Dozer 绑定:核心 `dozer-preview-core` 只认显式传入的 `config_dir`,
//! 这里用 Dozer 的 `dozer_core::paths::config_dir()` 给出与改造前完全相同的默认路径。

use std::path::PathBuf;

/// 默认启动标记文件:`<config_dir>/preview_startup.json`。
pub(crate) fn marker_path() -> PathBuf {
    dozer_preview_core::startup::marker_path_in(&dozer_core::paths::config_dir())
}

/// 默认失败计数文件:`<config_dir>/preview_failures.json`。
pub(crate) fn failures_path() -> PathBuf {
    dozer_preview_core::startup::failures_path_in(&dozer_core::paths::config_dir())
}

/// 默认恢复目录:`<config_dir>/preview_recovery/`。
pub(crate) fn recovery_dir() -> PathBuf {
    dozer_preview_core::recovery::recovery_dir_in(&dozer_core::paths::config_dir())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 绑定后的默认路径与改造前逐字相同。
    #[test]
    fn default_paths_are_the_legacy_ones() {
        let cfg = dozer_core::paths::config_dir();
        assert_eq!(marker_path(), cfg.join("preview_startup.json"));
        assert_eq!(failures_path(), cfg.join("preview_failures.json"));
        assert_eq!(recovery_dir(), cfg.join("preview_recovery"));
    }
}
