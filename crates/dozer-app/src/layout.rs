//! 四栏宽度布局的本地持久化——应用级偏好，不属于任何项目/窗口。
//! `load()`/`save()` 是真实调用方用的入口（固定读写
//! `dozer_core::paths::config_dir()/layout.json`）；`load_from`/`save_to`
//! 接收显式路径，供单测指向临时文件，不碰用户真实配置目录。

use crate::workspace::PanelLayout;
use std::path::{Path, PathBuf};

pub fn default_path() -> PathBuf {
    dozer_core::paths::config_dir().join("layout.json")
}

pub fn load() -> PanelLayout {
    load_from(&default_path())
}

pub fn save(layout: &PanelLayout) -> std::io::Result<()> {
    save_to(&default_path(), layout)
}

fn load_from(path: &Path) -> PanelLayout {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_to(path: &Path, layout: &PanelLayout) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let s = serde_json::to_string_pretty(layout).unwrap_or_default();
    std::fs::write(path, s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_from_missing_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.json");
        assert_eq!(load_from(&path), PanelLayout::default());
    }

    #[test]
    fn load_from_corrupt_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("layout.json");
        std::fs::write(&path, "not valid json").unwrap();
        assert_eq!(load_from(&path), PanelLayout::default());
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("layout.json");
        let layout = PanelLayout {
            project_col_width: 300.0,
            ai_col_width: 260.0,
            preview_ratio: 0.42,
        };
        save_to(&path, &layout).unwrap();
        assert_eq!(load_from(&path), layout);
    }
}
