//! Todo 面板 GUI 本地任务元数据（Todo 面板 design 第 3/8 节）：派发
//! 记录 + 计划时间 + 完成时间。跟 `open_projects.rs` 同一挂靠模式——
//! 纯运行时缓存，不进 git，不影响 `.dozer/todo.md` 本身的格式，读失败
//! （不存在/损坏）一律回落空 map，不 panic。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DispatchRecord {
    pub session_id: String,
    pub dispatched_at: SystemTime,
}

/// 三个字段互相独立——只设 `plan_date` 不影响 `dispatch`，反之亦然。
/// `#[serde(default)]` 让老文件缺字段时补 `None` 而不是整份反序列化
/// 失败（同 `ShellLayout` 的既有惯例）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct TodoTaskMeta {
    #[serde(default)]
    pub dispatch: Option<DispatchRecord>,
    #[serde(default)]
    pub plan_date: Option<String>,
    #[serde(default)]
    pub completed_at: Option<SystemTime>,
}

pub type TodoMetaState = HashMap<i64, HashMap<u64, TodoTaskMeta>>;

fn file_path() -> PathBuf {
    dozer_core::paths::config_dir().join("todo_meta.json")
}

pub fn load() -> TodoMetaState {
    load_from(&file_path())
}

pub fn save(state: &TodoMetaState) -> io::Result<()> {
    save_to(&file_path(), state)
}

fn load_from(path: &Path) -> TodoMetaState {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_to(path: &Path, state: &TodoMetaState) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(state).expect("TodoMetaState 总能序列化");
    std::fs::write(path, json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_from_missing_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nope.json");
        assert_eq!(load_from(&path), TodoMetaState::default());
    }

    #[test]
    fn load_from_corrupt_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.json");
        std::fs::write(&path, "not json").unwrap();
        assert_eq!(load_from(&path), TodoMetaState::default());
    }

    #[test]
    fn save_then_load_round_trips_and_fields_are_independent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("todo_meta.json");
        let mut state = TodoMetaState::new();
        let mut per_project = HashMap::new();
        // 一条只设了 plan_date，一条只设了 dispatch——验证字段互相独立。
        per_project.insert(
            1,
            TodoTaskMeta {
                plan_date: Some("2026-08-10".to_string()),
                ..Default::default()
            },
        );
        per_project.insert(
            2,
            TodoTaskMeta {
                dispatch: Some(DispatchRecord {
                    session_id: "sess-abc".to_string(),
                    dispatched_at: SystemTime::UNIX_EPOCH,
                }),
                ..Default::default()
            },
        );
        state.insert(42, per_project);
        save_to(&path, &state).unwrap();
        let loaded = load_from(&path);
        assert_eq!(loaded, state);
        assert!(loaded[&42][&1].dispatch.is_none());
        assert!(loaded[&42][&2].plan_date.is_none());
    }
}
