//! "用外部软件打开"配置表:文件扩展名 -> 外部 App 名字(如 `"Microsoft
//! Excel"`),供预览窗口工具栏的外部打开按钮查询。本期没有管理 UI,用户
//! 手工编辑 `config_dir()/external_apps.json`,启动时读一次进
//! `App::external_apps`,不在每帧 `view()` 里读盘。
//!
//! `load()`/`save()` 是真实调用方用的入口(固定读写
//! `external_apps.json`);`load_from`/`save_to` 接收显式路径,供单测指向
//! 临时文件,不碰用户真实配置目录。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ExternalAppsConfig {
    /// key 是小写扩展名(不带前导 `.`,如 `"xlsx"`),value 是要传给 macOS
    /// `open -a <value>` 的 App 名字。
    pub by_extension: HashMap<String, String>,
}

impl ExternalAppsConfig {
    /// 按文件路径的扩展名查配置的外部 App 名字,扩展名大小写不敏感。没有
    /// 扩展名或配置表里没有对应项都返回 `None`——调用方(预览工具栏)据此
    /// 决定"外部打开"按钮要不要出现。
    pub fn lookup_for_path(&self, path: &Path) -> Option<&str> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        self.by_extension.get(&ext).map(String::as_str)
    }
}

fn file_path() -> PathBuf {
    dozer_core::paths::config_dir().join("external_apps.json")
}

pub fn load() -> ExternalAppsConfig {
    load_from(&file_path())
}

#[allow(dead_code)] // 本期无管理 UI,`save` 暂无调用方;保留给下一期。
pub fn save(config: &ExternalAppsConfig) -> io::Result<()> {
    save_to(&file_path(), config)
}

fn load_from(path: &Path) -> ExternalAppsConfig {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_to(path: &Path, config: &ExternalAppsConfig) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(config).expect("ExternalAppsConfig 总能序列化");
    std::fs::write(path, json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_from_missing_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nope.json");
        assert_eq!(load_from(&path), ExternalAppsConfig::default());
    }

    #[test]
    fn load_from_corrupt_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.json");
        std::fs::write(&path, "not json").unwrap();
        assert_eq!(load_from(&path), ExternalAppsConfig::default());
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("external_apps.json");
        let mut config = ExternalAppsConfig::default();
        config
            .by_extension
            .insert("xlsx".to_string(), "Microsoft Excel".to_string());
        config
            .by_extension
            .insert("docx".to_string(), "Microsoft Word".to_string());
        save_to(&path, &config).unwrap();
        assert_eq!(load_from(&path), config);
    }

    #[test]
    fn lookup_for_path_is_case_insensitive_on_extension() {
        let mut config = ExternalAppsConfig::default();
        config
            .by_extension
            .insert("xlsx".to_string(), "Microsoft Excel".to_string());
        assert_eq!(
            config.lookup_for_path(Path::new("/tmp/report.XLSX")),
            Some("Microsoft Excel")
        );
        assert_eq!(
            config.lookup_for_path(Path::new("/tmp/report.xlsx")),
            Some("Microsoft Excel")
        );
    }

    #[test]
    fn lookup_for_path_returns_none_when_unconfigured_or_no_extension() {
        let config = ExternalAppsConfig::default();
        assert_eq!(config.lookup_for_path(Path::new("/tmp/report.csv")), None);
        assert_eq!(config.lookup_for_path(Path::new("/tmp/README")), None);
    }
}
