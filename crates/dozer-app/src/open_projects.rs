//! 并行项目页签集合的本地持久化——记"当前开了哪些项目、什么顺序、哪个
//! 在前台"，项目自己的路径/名字元数据不重复存，权威数据在 dozerd。

use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct OpenProjectsState {
    pub project_ids: Vec<i64>,
    pub active_project_id: Option<i64>,
}

fn file_path() -> PathBuf {
    dozer_core::paths::config_dir().join("open_projects.json")
}

pub fn load() -> OpenProjectsState {
    load_from(&file_path())
}

pub fn save(state: &OpenProjectsState) -> io::Result<()> {
    save_to(&file_path(), state)
}

fn load_from(path: &Path) -> OpenProjectsState {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_to(path: &Path, state: &OpenProjectsState) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(state).expect("OpenProjectsState 总能序列化");
    std::fs::write(path, json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_from_missing_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nope.json");
        assert_eq!(load_from(&path), OpenProjectsState::default());
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("open_projects.json");
        let state = OpenProjectsState {
            project_ids: vec![3, 1, 2],
            active_project_id: Some(1),
        };
        save_to(&path, &state).unwrap();
        assert_eq!(load_from(&path), state);
    }

    #[test]
    fn load_from_corrupt_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.json");
        std::fs::write(&path, "not json").unwrap();
        assert_eq!(load_from(&path), OpenProjectsState::default());
    }
}
