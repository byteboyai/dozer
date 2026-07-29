//! 四栏宽度布局的本地持久化——应用级偏好，不属于任何项目/窗口。
//! `load()`/`save()` 是真实调用方用的入口（固定读写
//! `dozer_core::paths::config_dir()/layout.json`）；`load_from`/`save_to`
//! 接收显式路径，供单测指向临时文件，不碰用户真实配置目录。

use crate::workspace::ShellLayout;
use std::path::{Path, PathBuf};

pub fn default_path() -> PathBuf {
    dozer_core::paths::config_dir().join("layout.json")
}

pub fn load() -> ShellLayout {
    load_from(&default_path())
}

pub fn save(layout: &ShellLayout) -> std::io::Result<()> {
    save_to(&default_path(), layout)
}

fn load_from(path: &Path) -> ShellLayout {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_to(path: &Path, layout: &ShellLayout) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let s = serde_json::to_string_pretty(layout).unwrap_or_default();
    std::fs::write(path, s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::{LeftView, RightView};

    #[test]
    fn load_from_missing_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.json");
        assert_eq!(load_from(&path), ShellLayout::default());
    }

    #[test]
    fn load_from_corrupt_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("layout.json");
        std::fs::write(&path, "not valid json").unwrap();
        assert_eq!(load_from(&path), ShellLayout::default());
    }

    #[test]
    fn shell_layout_default_has_sane_values() {
        let l = ShellLayout::default();
        assert!(l.left_width > 0.0);
        assert!((0.0..=1.0).contains(&l.files_split));
        assert!((0.0..=1.0).contains(&l.agent_split));
        assert!((0.0..=1.0).contains(&l.conversations_split));
    }

    #[test]
    fn shell_layout_save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("shell_layout.json");
        let layout = ShellLayout {
            left_width: 500.0,
            files_split: 0.4,
            agent_split: 0.35,
            conversations_split: 0.45,
            ..ShellLayout::default()
        };
        save_to(&path, &layout).unwrap();
        assert_eq!(load_from(&path), layout);
    }

    #[test]
    fn shell_layout_persists_view_selection_and_collapse() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shell_layout.json");
        let layout = ShellLayout {
            left_width: 500.0,
            files_split: 0.4,
            agent_split: 0.35,
            conversations_split: 0.45,
            left_view: LeftView::Web,
            right_view: RightView::Conversations,
            left_collapsed: true,
            right_collapsed: false,
        };
        save_to(&path, &layout).unwrap();
        assert_eq!(load_from(&path), layout);
    }
}
