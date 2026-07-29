//! 预览 tab(仅文件类)按项目本地持久化——重启/切项目后认回上次打开的
//! 文件。`load`/`save` 是真实调用方入口(固定读写
//! `dozer_core::paths::config_dir()/preview_state/<project_id>.json`);
//! `load_from`/`save_to` 接收显式路径,供单测指向临时文件。
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PreviewState {
    pub paths: Vec<PathBuf>,
    pub active_path: Option<PathBuf>,
}

fn state_path(project_id: i64) -> PathBuf {
    dozer_core::paths::config_dir()
        .join("preview_state")
        .join(format!("{project_id}.json"))
}

pub fn load(project_id: i64) -> PreviewState {
    load_from(&state_path(project_id))
}

pub fn save(project_id: i64, state: &PreviewState) -> std::io::Result<()> {
    save_to(&state_path(project_id), state)
}

fn load_from(path: &Path) -> PreviewState {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_to(path: &Path, state: &PreviewState) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let s = serde_json::to_string_pretty(state).unwrap_or_default();
    std::fs::write(path, s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_from_missing_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.json");
        assert_eq!(load_from(&path), PreviewState::default());
    }

    #[test]
    fn load_from_corrupt_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preview_state.json");
        std::fs::write(&path, "not valid json").unwrap();
        assert_eq!(load_from(&path), PreviewState::default());
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("preview_state.json");
        let state = PreviewState {
            paths: vec![
                PathBuf::from("/repo/src/main.rs"),
                PathBuf::from("/repo/README.md"),
            ],
            active_path: Some(PathBuf::from("/repo/README.md")),
        };
        save_to(&path, &state).unwrap();
        assert_eq!(load_from(&path), state);
    }

    #[test]
    fn save_then_load_round_trips_no_active() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preview_state.json");
        let state = PreviewState {
            paths: vec![PathBuf::from("/repo/src/main.rs")],
            active_path: None,
        };
        save_to(&path, &state).unwrap();
        assert_eq!(load_from(&path), state);
    }
}
