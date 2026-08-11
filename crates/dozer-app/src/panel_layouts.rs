//! 每个项目各自的面板布局(左右视图选择 + 收起态)的本地持久化——按项目 id
//! 记到 `config_dir()/panel_layouts.json`,与全局几何 `layout.json` 分离。这样
//! 切换项目时只换当前项目的面板状态、不会盖掉别的项目(见切换项目 bug)。
//!
//! `load()`/`save()` 是真实调用方用的入口(固定读写 `panel_layouts.json`);
//! `load_from`/`save_to` 接收显式路径,供单测指向临时文件,不碰用户真实配置
//! 目录。

use crate::app::PanelLayout;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub fn default_path() -> PathBuf {
    dozer_core::paths::config_dir().join("panel_layouts.json")
}

pub fn load() -> HashMap<i64, PanelLayout> {
    load_from(&default_path())
}

pub fn save(map: &HashMap<i64, PanelLayout>) -> std::io::Result<()> {
    save_to(&default_path(), map)
}

fn load_from(path: &Path) -> HashMap<i64, PanelLayout> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_to(path: &Path, map: &HashMap<i64, PanelLayout>) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let s = serde_json::to_string_pretty(map).unwrap_or_default();
    std::fs::write(path, s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{LeftView, RightView};

    #[test]
    fn load_from_missing_file_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.json");
        assert!(load_from(&path).is_empty());
    }

    #[test]
    fn load_from_corrupt_file_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("panel_layouts.json");
        std::fs::write(&path, "not valid json").unwrap();
        assert!(load_from(&path).is_empty());
    }

    #[test]
    fn save_then_load_round_trips_per_project() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("panel_layouts.json");
        let mut map = HashMap::new();
        map.insert(
            1,
            PanelLayout {
                left_view: LeftView::Project,
                right_view: RightView::Acceptance,
                left_collapsed: true,
                right_collapsed: false,
            },
        );
        map.insert(
            2,
            PanelLayout {
                left_view: LeftView::Files,
                right_view: RightView::Agent,
                left_collapsed: false,
                right_collapsed: true,
            },
        );
        save_to(&path, &map).unwrap();
        assert_eq!(load_from(&path), map);
    }

    #[test]
    fn per_project_layouts_are_independent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("panel_layouts.json");
        let mut map = HashMap::new();
        map.insert(
            1,
            PanelLayout {
                left_view: LeftView::Project,
                right_view: RightView::Acceptance,
                left_collapsed: true,
                right_collapsed: false,
            },
        );
        save_to(&path, &map).unwrap();
        // 只存了项目 1,项目 2 不存在(读回应是 None),互不影响。
        let loaded = load_from(&path);
        assert!(loaded.contains_key(&1));
        assert!(!loaded.contains_key(&2));
    }
}
