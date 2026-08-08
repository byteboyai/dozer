//! 数据库面板 · 阶段 1:驱动管理 + 每项目数据源 CRUD + 连接测试。结构照搬
//! `extensions::todo` 的双状态模式——`AppState` 挂 `App`(全局:哪些驱动
//! 启用),`WorkspaceState` 挂每个 `Workspace`(当前项目的数据源列表)。
//! 密码不进这个文件的任何持久化结构,单独走 macOS Keychain(见
//! `keyring_key`)。schema 树/数据浏览/SQL 执行/MongoDB 集合浏览留后续
//! 阶段,见
//! `docs/superpowers/specs/2026-08-08-database-panel-phase1-design.md`。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// 数据库面板支持的驱动类型。穷举枚举,不做插件机制(见设计文档"非目标")。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum DriverKind {
    #[default]
    Postgres,
    MySQL,
    Sqlite,
    MongoDB,
}

impl DriverKind {
    pub const ALL: [DriverKind; 4] = [
        DriverKind::Postgres,
        DriverKind::MySQL,
        DriverKind::Sqlite,
        DriverKind::MongoDB,
    ];

    /// 面板/表单里展示的中文名。
    pub fn label(self) -> &'static str {
        match self {
            DriverKind::Postgres => "PostgreSQL",
            DriverKind::MySQL => "MySQL",
            DriverKind::Sqlite => "SQLite",
            DriverKind::MongoDB => "MongoDB",
        }
    }
}

/// 一个数据源(不含密码)。`.dozer/database.json` 存 `Vec<DataSource>`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DataSource {
    pub id: String,
    pub name: String,
    pub driver: DriverKind,
    pub host: Option<String>,
    pub port: Option<u16>,
    /// SQLite 这里存文件路径,其它驱动存库名。
    pub database: Option<String>,
    pub username: Option<String>,
}

/// 某条数据源当前的连接测试状态,画在卡片上。
#[derive(Debug, Clone, PartialEq, Default)]
pub enum TestStatus {
    #[default]
    Idle,
    Testing,
    Ok,
    Err(String),
}

fn drivers_path() -> PathBuf {
    dozer_core::paths::config_dir().join("database_drivers.json")
}

#[derive(Debug, Serialize, Deserialize)]
struct EnabledDriversFile {
    enabled: Vec<DriverKind>,
}

/// 挂在 `App` 上:哪些驱动类型在"新增数据源"下拉里可选。默认全部启用。
#[derive(Debug)]
pub struct AppState {
    enabled: std::collections::HashSet<DriverKind>,
    /// 驱动管理弹层的开关态(非持久化 UI 态,不参与 `save()`)。
    drivers_popup_open: bool,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            enabled: DriverKind::ALL.into_iter().collect(),
            drivers_popup_open: false,
        }
    }
}

impl AppState {
    pub fn load() -> Self {
        let path = drivers_path();
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        let Ok(file) = serde_json::from_str::<EnabledDriversFile>(&text) else {
            return Self::default();
        };
        Self {
            enabled: file.enabled.into_iter().collect(),
            drivers_popup_open: false,
        }
    }

    fn save(&self) {
        let file = EnabledDriversFile {
            enabled: self.enabled.iter().copied().collect(),
        };
        let Ok(json) = serde_json::to_string_pretty(&file) else {
            return;
        };
        if let Err(e) = std::fs::write(drivers_path(), json) {
            tracing::warn!("写入 database_drivers.json 失败: {e}");
        }
    }

    pub fn is_enabled(&self, driver: DriverKind) -> bool {
        self.enabled.contains(&driver)
    }

    pub fn toggle(&mut self, driver: DriverKind) {
        if !self.enabled.remove(&driver) {
            self.enabled.insert(driver);
        }
        self.save();
    }

    /// 驱动管理弹层的开关态(非持久化 UI 态)。
    pub fn drivers_popup_open(&self) -> bool {
        self.drivers_popup_open
    }

    pub fn set_drivers_popup_open(&mut self, open: bool) {
        self.drivers_popup_open = open;
    }
}

/// 新增/编辑数据源表单的草稿态。`id` 为 `None` = 新增,`Some(..)` = 编辑
/// 已有数据源(保留原 id 不变)。
#[derive(Debug, Clone, Default)]
pub struct DataSourceDraft {
    pub id: Option<String>,
    pub name: String,
    pub driver: DriverKind,
    pub host: String,
    pub port: String,
    pub database: String,
    pub username: String,
    pub password: String,
}

/// 挂在每个 `Workspace` 上:当前项目配置的数据源列表 + 编辑态 + 每条数据源
/// 的连接测试状态。
#[derive(Debug, Default)]
pub struct WorkspaceState {
    sources: Vec<DataSource>,
    editing: Option<DataSourceDraft>,
    test_status: HashMap<String, TestStatus>,
}

impl WorkspaceState {
    pub fn sources(&self) -> &[DataSource] {
        &self.sources
    }

    pub fn editing(&self) -> Option<&DataSourceDraft> {
        self.editing.as_ref()
    }

    pub fn test_status(&self, source_id: &str) -> &TestStatus {
        self.test_status.get(source_id).unwrap_or(&TestStatus::Idle)
    }
}

pub fn sources_path(repo: &Path) -> PathBuf {
    repo.join(".dozer").join("database.json")
}

/// 从 `.dozer/database.json` 重读数据源列表。文件不存在/格式损坏→空列表,
/// 不 panic(同 `todo`/`goal` 现有容错口径)。**不**清空 `test_status`/
/// `editing`——重读只影响 `sources` 本身,进行中的编辑/测试状态原样保留。
pub fn reload_from_disk(ws_state: &mut WorkspaceState, repo: &Path) {
    let path = sources_path(repo);
    ws_state.sources = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
}

fn save_sources(repo: &Path, sources: &[DataSource]) -> std::io::Result<()> {
    let dir = repo.join(".dozer");
    std::fs::create_dir_all(&dir)?;
    let json = serde_json::to_string_pretty(sources).expect("Vec<DataSource> 总能序列化");
    std::fs::write(sources_path(repo), json)
}

/// Keychain 条目 key:`{project_id}:{source_id}`,同一把钥匙串条目按项目+
/// 数据源双重区分。
fn keyring_entry(project_id: i64, source_id: &str) -> Result<keyring::Entry, keyring::Error> {
    keyring::Entry::new("dozer", &format!("{project_id}:{source_id}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ds(id: &str, driver: DriverKind) -> DataSource {
        DataSource {
            id: id.into(),
            name: format!("test-{id}"),
            driver,
            host: Some("localhost".into()),
            port: Some(5432),
            database: Some("mydb".into()),
            username: Some("user".into()),
        }
    }

    #[test]
    fn app_state_defaults_to_all_drivers_enabled() {
        let state = AppState::default();
        for d in DriverKind::ALL {
            assert!(state.is_enabled(d));
        }
    }

    #[test]
    fn app_state_load_missing_file_returns_default() {
        // config_dir() 指向真实用户目录,这里只验证"文件不存在"分支不 panic
        // 且落到全启用默认值——不清真实用户配置,不需要临时目录隔离(同
        // `icon_size.rs` 现有测试对 `config_dir()` 的处理口径:该文件本就
        // 可能不存在,读不到就是默认值,这条本身不依赖文件真的不存在)。
        let path = drivers_path();
        if !path.exists() {
            let state = AppState::load();
            assert!(state.is_enabled(DriverKind::Postgres));
        }
    }

    #[test]
    fn toggle_flips_membership() {
        let mut state = AppState::default();
        state.toggle(DriverKind::MongoDB);
        assert!(!state.is_enabled(DriverKind::MongoDB));
        state.toggle(DriverKind::MongoDB);
        assert!(state.is_enabled(DriverKind::MongoDB));
    }

    #[test]
    fn reload_from_disk_missing_file_yields_empty_sources() {
        let dir = tempfile::tempdir().unwrap();
        let mut ws_state = WorkspaceState::default();
        reload_from_disk(&mut ws_state, dir.path());
        assert!(ws_state.sources().is_empty());
    }

    #[test]
    fn save_then_reload_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let sources = vec![ds("a", DriverKind::Postgres), ds("b", DriverKind::Sqlite)];
        save_sources(dir.path(), &sources).unwrap();
        let mut ws_state = WorkspaceState::default();
        reload_from_disk(&mut ws_state, dir.path());
        assert_eq!(ws_state.sources(), sources.as_slice());
    }

    #[test]
    fn reload_from_disk_corrupt_file_yields_empty_sources() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".dozer")).unwrap();
        std::fs::write(sources_path(dir.path()), "not json").unwrap();
        let mut ws_state = WorkspaceState::default();
        reload_from_disk(&mut ws_state, dir.path());
        assert!(ws_state.sources().is_empty());
    }
}
