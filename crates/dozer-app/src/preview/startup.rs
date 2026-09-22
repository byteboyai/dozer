//! 安全启动与失败计数(文件预览重构 Phase C Task 7)。
//!
//! - **启动进行/完成标记**:启动写 `in_progress`,正常启动完成后写 `done`。
//!   下次启动若读到 `in_progress`,说明上次启动没走完(崩溃/强退),进入
//!   "安全启动":只恢复 tab 壳,不自动加载问题文件,避免启动死循环。
//! - **单文件连续失败计数**:加载失败按路径累计;成功即清零。达到阈值后 UI
//!   应给纯文本只读/Windowed/外部打开等降级,而不是无限自动重试。
//!
//! 只写状态与计数,不写任何文件内容/路径以外的敏感信息。

// 安全启动已接入 runtime/restore;失败计数的 UI 降级入口(Task 7 后续)接入
// 前部分 API 暂未被非测试代码调用,显式允许。
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// 启动标记状态。
pub const STATUS_IN_PROGRESS: &str = "in_progress";
pub const STATUS_DONE: &str = "done";

/// 连续失败达到该次数后停止自动重试 / 提示降级。
pub const FAILURE_THRESHOLD: u32 = 3;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartupMarker {
    pub status: String,
    pub started_ms: u64,
}

/// 默认标记文件:`<config_dir>/preview_startup.json`。
pub fn marker_path() -> PathBuf {
    dozer_core::paths::config_dir().join("preview_startup.json")
}

/// 默认失败计数文件:`<config_dir>/preview_failures.json`。
pub fn failures_path() -> PathBuf {
    dozer_core::paths::config_dir().join("preview_failures.json")
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 原子写启动标记。
pub fn write_status_to(path: &Path, status: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let marker = StartupMarker {
        status: status.to_string(),
        started_ms: now_ms(),
    };
    let json = serde_json::to_vec(&marker).map_err(std::io::Error::other)?;
    atomic_write(path, &json)
}

/// 读启动标记状态;缺失/损坏返回 `None`。
pub fn read_status_from(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    let marker: StartupMarker = serde_json::from_slice(&bytes).ok()?;
    Some(marker.status)
}

/// 上次启动是否没走完(本次应进入安全启动)。
pub fn was_interrupted_from(path: &Path) -> bool {
    read_status_from(path).as_deref() == Some(STATUS_IN_PROGRESS)
}

/// 读某路径的连续失败次数(缺失/损坏为 0)。
pub fn failure_count_from(path: &Path, file: &Path) -> u32 {
    load_failures(path)
        .get(&file.to_string_lossy().into_owned())
        .copied()
        .unwrap_or(0)
}

/// 累加并持久化某路径的失败次数,返回累加后的值。
pub fn bump_failure_to(path: &Path, file: &Path) -> u32 {
    let mut map = load_failures(path);
    let key = file.to_string_lossy().into_owned();
    let count = map.entry(key).or_insert(0);
    *count = count.saturating_add(1);
    let value = *count;
    let _ = save_failures(path, &map);
    value
}

/// 加载成功后清零某路径的失败计数。
pub fn reset_failure_to(path: &Path, file: &Path) {
    let mut map = load_failures(path);
    if map.remove(&file.to_string_lossy().into_owned()).is_some() {
        let _ = save_failures(path, &map);
    }
}

fn load_failures(path: &Path) -> std::collections::HashMap<String, u32> {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save_failures(path: &Path, map: &std::collections::HashMap<String, u32>) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_vec(map).map_err(std::io::Error::other)?;
    atomic_write(path, &json)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.flush()?;
        f.sync_all()?;
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_marker_round_trip_and_interrupted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("startup.json");
        assert!(!was_interrupted_from(&path), "缺失不算中断");
        write_status_to(&path, STATUS_IN_PROGRESS).unwrap();
        assert!(was_interrupted_from(&path));
        write_status_to(&path, STATUS_DONE).unwrap();
        assert!(!was_interrupted_from(&path));
        assert_eq!(read_status_from(&path).as_deref(), Some(STATUS_DONE));
    }

    #[test]
    fn corrupt_marker_is_none_not_interrupted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("startup.json");
        std::fs::write(&path, b"not json").unwrap();
        assert_eq!(read_status_from(&path), None);
        assert!(!was_interrupted_from(&path));
    }

    #[test]
    fn failure_count_bumps_and_resets() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("failures.json");
        let file = Path::new("/repo/a.rs");
        assert_eq!(failure_count_from(&f, file), 0);
        assert_eq!(bump_failure_to(&f, file), 1);
        assert_eq!(bump_failure_to(&f, file), 2);
        assert_eq!(failure_count_from(&f, file), 2);
        reset_failure_to(&f, file);
        assert_eq!(failure_count_from(&f, file), 0);

        // 不同文件互不影响。
        bump_failure_to(&f, Path::new("/repo/b.rs"));
        assert_eq!(failure_count_from(&f, Path::new("/repo/a.rs")), 0);
    }

    #[test]
    fn corrupt_failures_treated_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("failures.json");
        std::fs::write(&f, b"{broken").unwrap();
        assert_eq!(failure_count_from(&f, Path::new("/x")), 0);
    }
}
