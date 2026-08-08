# 数据库面板 · 阶段 1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 新增 `extensions::database` 面板:App 级驱动启用/禁用(Postgres/
MySQL/SQLite/MongoDB)+ 每个项目自己的数据源列表(增删改)+ 异步连接测试。
密码进 macOS Keychain,不进配置文件。这是数据库面板 5 个阶段里的第一个,
schema 树/数据网格浏览/SQL 脚本执行/MongoDB 集合浏览都不在这次范围内。

**Architecture:** 新文件 `crates/dozer-app/src/extensions/database.rs`,结构
照搬 `extensions::todo` 的双状态模式(`AppState` 挂 `App`、`WorkspaceState`
挂每个 `Workspace`,`update` 需要同时可变借用两者,走 `loaded_workspace_mut`
直接借,不套 `with_focused_project` 闭包)。连接层用 `sqlx::AnyPool`
(Postgres/MySQL/SQLite 一套 API)+ `mongodb::Client`(MongoDB)。新增
`LeftView::Database` 接入工作区左侧图标栏/面板区,现有模式(`rail_icon_
button`/`zone_pane_border`/`left_panel_area` 分发)原样复用。

**Tech Stack:** Rust workspace;iced 0.14(`iced_widget` 0.14.2,已核实
`text_input(..).secure(bool)` 在这个版本可用);新增 `sqlx`(Any 驱动)、
`mongodb`、`keyring`;`uuid` 已经是 workspace 依赖(`dozer-app/Cargo.toml`
已有 `uuid.workspace = true`),`DataSource::id` 直接用,不算新增依赖。

## Global Constraints

- **在独立分支上开发**:建分支 `feature/database-panel-phase1`(或对应
  worktree),完成后提请审阅,通过再合并回 `main`。
- **这个计划基于 `main`(截至 commit `9562992`,已包含首页四栏化,不包含
  `App`/`Workspace` 文件拆分)分析——`App`/`Workspace` 拆分正由另一个 agent
  在 `feature/app-workspace-split` 分支并行推进,尚未合并**。本计划涉及的
  `LeftView`/`RailButton`/`App` struct/`Workspace` struct/`Message` 枚举/
  `App::update`/`left_icon_rail`/`left_panel_area` 这些符号,现状(本计划
  写作时)全部在 `workspace.rs` 一个文件里;**如果你开工时那个拆分已经
  合并**,这些符号会分散在 `app.rs`(`App`/`Message`/`impl App`/
  `left_icon_rail`/`left_panel_area` 等壳层符号)和 `workspace.rs`
  (`Workspace` struct 本身)两个文件——按符号名 `grep` 定位实际所在文件,
  不要假设都在 `workspace.rs`。两个分支改的是不同符号(这个计划新增
  `LeftView::Database` 这一个变体 + 少数几个匹配分支,不碰 `App`/
  `Workspace` 的内部结构),真发生合并冲突时是"两处都加了一行"级别的
  冲突,直接接受两边都要的改动即可。
- **密码绝不写入 `.dozer/database.json` 或任何日志**——只经
  `keyring::Entry` 存取。测试里不要用真实密码,用占位字符串。
- **这次不做** schema 树/数据网格浏览/SQL 脚本执行/MongoDB 集合浏览——
  `database::view` 只有"驱动管理 + 数据源卡片列表 + 增删改表单 + 测试
  连接"这一层,不要顺手加任何浏览/查询入口。
- 每个任务结束都要 `cargo build && cargo test && cargo clippy --all-targets
  && cargo fmt` 干净通过(全 workspace)。Task 2/3 结束时,新模块的类型/
  函数还没被 `App`/`Workspace`/`Message` 引用,会有 `dead_code`/`unused`
  警告——这是预期的过渡态(同 Task 5 收尾),不要为此加 `#[allow(dead_
  code)]`,Task 5 接入内核后应该自然清零。
- 设计文档:`docs/superpowers/specs/2026-08-08-database-panel-phase1-
  design.md`(有疑问以它为准)。

---

### Task 1: 新增依赖 + 图标资源

**Files:**
- Modify: `crates/dozer-app/Cargo.toml`
- Create: `crates/dozer-app/assets/icons/database.svg`
- Modify: `crates/dozer-app/src/icons.rs`

**Interfaces:**
- Produces:`icons::IconKind::Database` 变体(可渲染)。

- [ ] **Step 1: 加依赖**

`crates/dozer-app/Cargo.toml` 的 `[dependencies]` 块里,在 `uuid.workspace
= true` 之后加:

```toml
sqlx = { version = "0.8", default-features = false, features = ["runtime-tokio", "tls-rustls", "any", "postgres", "mysql", "sqlite"] }
mongodb = { version = "3", default-features = false, features = ["tokio-runtime"] }
keyring = "3"
```

（版本号写计划时按 crates.io 当前最新的 0.8.x/3.x/3.x 系列填,若实际
`cargo add` 解出的具体 patch 版本不同以 `cargo add` 结果为准,不用手改这里
的大版本号。`sqlx` 关掉 `default-features` 是因为默认 features 里有一部分
仓库不需要的运行时/TLS 组合,显式列出需要的四个 feature 避免拉重复依赖。）

- [ ] **Step 2: 装依赖,确认编译**

```bash
cargo build -p dozer-app 2>&1 | tail -30
```

预期:能拉到依赖并编译通过(此时还没有任何代码用到这几个新 crate,只是
把它们加进依赖图;如果 `sqlx`/`mongodb` 版本号解析冲突,按报错调整版本
号,不强求锁死本文档写的具体版本)。

- [ ] **Step 3: 新增图标资源**

创建 `crates/dozer-app/assets/icons/database.svg`(Lucide `database`,去掉
license 注释与 `class` 属性,格式对齐仓库其它已有图标):

```svg
<svg
  xmlns="http://www.w3.org/2000/svg"
  width="24"
  height="24"
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="2"
  stroke-linecap="round"
  stroke-linejoin="round"
>
  <ellipse cx="12" cy="5" rx="9" ry="3" />
  <path d="M3 5V19A9 3 0 0 0 21 19V5" />
  <path d="M3 12A9 3 0 0 0 21 12" />
</svg>
```

`crates/dozer-app/src/icons.rs` 的 `IconKind` 枚举,在 `BadgeCheck` 变体
之后加:

```rust
    /// 数据库面板 rail 图标(Lucide database)。
    Database,
```

`bytes()` 方法的 `match` 里,对应位置加:

```rust
            IconKind::Database => include_bytes!("../assets/icons/database.svg"),
```

- [ ] **Step 4: 编译/lint/格式确认**

```bash
cargo build -p dozer-app
cargo clippy -p dozer-app --all-targets
cargo fmt --check
```

预期:`IconKind::Database` 这个变体这一步还没被任何代码构造(没有调用点),
`cargo build`/`clippy` 会给一条 `dead_code`("variant is never constructed"）
警告——这是预期的,Task 5 接入 rail 按钮后会自然清掉,不要加
`#[allow(dead_code)]`。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/Cargo.toml Cargo.lock crates/dozer-app/assets/icons/database.svg crates/dozer-app/src/icons.rs
git commit -m "feat(dozer-app): add sqlx/mongodb/keyring deps and database icon"
```

---

### Task 2: 数据模型 + 持久化

**Files:**
- Create: `crates/dozer-app/src/extensions/database.rs`
- Modify: `crates/dozer-app/src/extensions.rs`(加 `pub mod database;`)

**Interfaces:**
- Produces:
  - `pub enum DriverKind { Postgres, MySQL, Sqlite, MongoDB }`
  - `pub struct DataSource { pub id: String, pub name: String, pub driver: DriverKind, pub host: Option<String>, pub port: Option<u16>, pub database: Option<String>, pub username: Option<String> }`
  - `pub enum TestStatus { Idle, Testing, Ok, Err(String) }`
  - `pub struct AppState { .. }`(`load()`/内部 `save()`,驱动启用集合)
  - `pub struct WorkspaceState { .. }`(`Default`,数据源列表 + 编辑草稿 +
    测试状态 map)
  - `pub fn sources_path(repo: &Path) -> PathBuf`
  - `pub fn reload_from_disk(ws_state: &mut WorkspaceState, repo: &Path)`
    (读 `.dozer/database.json`,失败/不存在→空列表,不 panic)
  - `fn save_sources(repo: &Path, sources: &[DataSource]) -> std::io::Result<()>`

- [ ] **Step 1: 写文件头 + 数据模型**

```rust
// crates/dozer-app/src/extensions/database.rs
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DriverKind {
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
```

- [ ] **Step 2: `AppState`(驱动启用集合)**

```rust
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
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            enabled: DriverKind::ALL.into_iter().collect(),
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
}
```

- [ ] **Step 3: `WorkspaceState`(每项目数据源列表)**

```rust
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

impl Default for DriverKind {
    fn default() -> Self {
        DriverKind::Postgres
    }
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
```

- [ ] **Step 4: 单测**

```rust
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
        let mut state = AppState {
            enabled: DriverKind::ALL.into_iter().collect(),
        };
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
        let sources = vec![
            ds("a", DriverKind::Postgres),
            ds("b", DriverKind::Sqlite),
        ];
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
```

- [ ] **Step 5: 注册模块**

`crates/dozer-app/src/extensions.rs` 的 `pub mod` 列表按字母序,在 `pub mod
browser;` 之后、`pub mod files;` 之前插入 `pub mod database;`。

- [ ] **Step 6: 编译/测试/lint/格式**

```bash
cargo test -p dozer-app database:: -- --test-threads=1
cargo build -p dozer-app
cargo clippy -p dozer-app --all-targets
cargo fmt --check
```

预期:新增的 6 个测试全部 PASS。`AppState`/`WorkspaceState`/`DataSource`
等还没被 `App`/`Workspace` struct 引用,会有 dead_code 警告——预期中,
Task 5 解决。

- [ ] **Step 7: Commit**

```bash
git add crates/dozer-app/src/extensions/database.rs crates/dozer-app/src/extensions.rs
git commit -m "feat(dozer-app): add database extension data model and persistence"
```

---

### Task 3: 连接测试逻辑 + `Message`/`update`

**Files:**
- Modify: `crates/dozer-app/src/extensions/database.rs`
- Modify: `crates/dozer-app/src/main.rs`(`sqlx::any::install_default_
  drivers()` 初始化调用)

**Interfaces:**
- Consumes:Task 2 的 `AppState`/`WorkspaceState`/`DataSource`/`DriverKind`/
  `TestStatus`/`DataSourceDraft`/`reload_from_disk`/`sources_path`。
- Produces:
  - `pub enum Message { .. }`
  - `pub fn update(ws_state: &mut WorkspaceState, app_state: &mut AppState, msg: Message, project_id: i64, repo_path: &Path, handle: &tokio::runtime::Handle, emit: impl Fn(Message) + Send + 'static)`
  - `fn build_sql_url(source: &DataSource, password: Option<&str>) -> String`
  - `fn build_mongo_url(source: &DataSource, password: Option<&str>) -> String`

- [ ] **Step 1: `main.rs` 注册 sqlx Any 驱动**

`sqlx::Any` 需要在用之前注册具体驱动实现,同字体注册(`fonts::load()`)
一类"启动时跑一次的初始化"。`main.rs` 里找到程序启动早期的初始化段(字体
加载/主题加载那一块附近),加一行:

```rust
sqlx::any::install_default_drivers();
```

（不需要 `unwrap`/错误处理——这个函数本身不返回 `Result`,只是注册进程内
静态表。放的位置只要求"在第一次 `sqlx::AnyPool::connect` 之前跑过一次"
即可,不要求精确到具体是哪一行,写计划时看 `main.rs` 现状初始化段的组织
方式,插进去风格一致的位置。）

- [ ] **Step 2: 连接串构造纯函数**

```rust
fn build_sql_url(source: &DataSource, password: Option<&str>) -> String {
    let scheme = match source.driver {
        DriverKind::Postgres => "postgres",
        DriverKind::MySQL => "mysql",
        DriverKind::Sqlite => "sqlite",
        DriverKind::MongoDB => unreachable!("build_sql_url 不处理 MongoDB"),
    };
    if source.driver == DriverKind::Sqlite {
        // SQLite: sqlite://<文件路径>,不带 host/port/用户名/密码。
        let path = source.database.as_deref().unwrap_or("");
        return format!("sqlite://{path}");
    }
    let host = source.host.as_deref().unwrap_or("localhost");
    let port = source.port.map(|p| format!(":{p}")).unwrap_or_default();
    let db = source.database.as_deref().unwrap_or("");
    let auth = match (source.username.as_deref(), password) {
        (Some(u), Some(p)) if !p.is_empty() => format!("{u}:{p}@"),
        (Some(u), _) => format!("{u}@"),
        (None, _) => String::new(),
    };
    format!("{scheme}://{auth}{host}{port}/{db}")
}

fn build_mongo_url(source: &DataSource, password: Option<&str>) -> String {
    let host = source.host.as_deref().unwrap_or("localhost");
    let port = source.port.map(|p| format!(":{p}")).unwrap_or_default();
    let auth = match (source.username.as_deref(), password) {
        (Some(u), Some(p)) if !p.is_empty() => format!("{u}:{p}@"),
        (Some(u), _) => format!("{u}@"),
        (None, _) => String::new(),
    };
    format!("mongodb://{auth}{host}{port}")
}
```

写测试(先写、跑失败、再确认上面的实现让它过——这两个是纯函数,可以严格
走 TDD):

```rust
#[cfg(test)]
mod url_tests {
    use super::*;

    fn base(driver: DriverKind) -> DataSource {
        DataSource {
            id: "x".into(),
            name: "n".into(),
            driver,
            host: Some("db.internal".into()),
            port: Some(5432),
            database: Some("mydb".into()),
            username: Some("alice".into()),
        }
    }

    #[test]
    fn postgres_url_with_password() {
        let s = base(DriverKind::Postgres);
        assert_eq!(
            build_sql_url(&s, Some("secret")),
            "postgres://alice:secret@db.internal:5432/mydb"
        );
    }

    #[test]
    fn postgres_url_without_password() {
        let s = base(DriverKind::Postgres);
        assert_eq!(
            build_sql_url(&s, None),
            "postgres://alice@db.internal:5432/mydb"
        );
    }

    #[test]
    fn sqlite_url_uses_database_as_path_ignores_host() {
        let mut s = base(DriverKind::Sqlite);
        s.database = Some("/tmp/my.sqlite".into());
        assert_eq!(build_sql_url(&s, None), "sqlite:///tmp/my.sqlite");
    }

    #[test]
    fn mongo_url_with_password() {
        let s = base(DriverKind::MongoDB);
        assert_eq!(
            build_mongo_url(&s, Some("secret")),
            "mongodb://alice:secret@db.internal:5432"
        );
    }
}
```

- [ ] **Step 3: 跑测试确认通过**

```bash
cargo test -p dozer-app url_tests -- --test-threads=1
```

- [ ] **Step 4: `Message` 枚举**

```rust
#[derive(Debug, Clone)]
pub enum Message {
    /// 驱动管理弹层:勾/取消勾某个驱动类型。
    ToggleDriver(DriverKind),
    /// 打开"驱动管理"弹层。
    DriversPopupToggle,
    /// 点"＋新增数据源"→ 打开空白草稿表单。
    AddSourceStart,
    /// 点某张卡的"编辑"→ 用该数据源现有字段(不含密码)预填草稿表单。
    EditSourceStart(String),
    /// 表单字段编辑(草稿态,未提交)。
    DraftNameChanged(String),
    DraftDriverChanged(DriverKind),
    DraftHostChanged(String),
    DraftPortChanged(String),
    DraftDatabaseChanged(String),
    DraftUsernameChanged(String),
    DraftPasswordChanged(String),
    /// 提交表单:新增或更新(视 `draft.id` 是否为 `None`),写盘 + 密码进
    /// Keychain,关闭表单。
    DraftSave,
    /// 取消表单,丢弃草稿。
    DraftCancel,
    /// 删除一条数据源:同时删 `.dozer/database.json` 里的记录和 Keychain
    /// 里的密码条目。
    DeleteSource(String),
    /// 点"测试连接":发起异步测试,`ws_state.test_status` 先置
    /// `Testing`。
    TestConnection(String),
    /// 异步测试结果。**带 `project_id`**——见设计文档"结果经 `emit`
    /// 回传"一节,不能只带 `source_id`,用户可能在等待期间切走了项目
    /// 页签,`App::update` 必须按这里的 `project_id` 而不是"当前聚焦
    /// 项目"路由。
    TestConnectionResult(i64, String, Result<(), String>),
}
```

- [ ] **Step 5: `update` 函数**

```rust
pub fn update(
    ws_state: &mut WorkspaceState,
    app_state: &mut AppState,
    msg: Message,
    project_id: i64,
    repo_path: &Path,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    match msg {
        Message::ToggleDriver(driver) => app_state.toggle(driver),
        Message::DriversPopupToggle => {
            // 弹层开关态如果放在 WorkspaceState 里会随项目切换而不一致
            // (驱动管理是全局概念),这一格开关态应该放 AppState——写代码
            // 时给 AppState 加一个 `drivers_popup_open: bool` 字段
            // (非持久化,不需要 `save()`),Step 4 补充这个字段到 AppState
            // 定义里(Task 2 Step 2 当时没加,这里补)。
        }
        Message::AddSourceStart => {
            ws_state.editing = Some(DataSourceDraft::default());
        }
        Message::EditSourceStart(id) => {
            if let Some(src) = ws_state.sources.iter().find(|s| s.id == id) {
                ws_state.editing = Some(DataSourceDraft {
                    id: Some(src.id.clone()),
                    name: src.name.clone(),
                    driver: src.driver,
                    host: src.host.clone().unwrap_or_default(),
                    port: src.port.map(|p| p.to_string()).unwrap_or_default(),
                    database: src.database.clone().unwrap_or_default(),
                    username: src.username.clone().unwrap_or_default(),
                    password: String::new(), // 不回显已存密码,留空=不改密码
                });
            }
        }
        Message::DraftNameChanged(v) => set_draft(ws_state, |d| d.name = v),
        Message::DraftDriverChanged(v) => set_draft(ws_state, |d| d.driver = v),
        Message::DraftHostChanged(v) => set_draft(ws_state, |d| d.host = v),
        Message::DraftPortChanged(v) => set_draft(ws_state, |d| d.port = v),
        Message::DraftDatabaseChanged(v) => set_draft(ws_state, |d| d.database = v),
        Message::DraftUsernameChanged(v) => set_draft(ws_state, |d| d.username = v),
        Message::DraftPasswordChanged(v) => set_draft(ws_state, |d| d.password = v),
        Message::DraftSave => {
            let Some(draft) = ws_state.editing.take() else {
                return;
            };
            if draft.name.trim().is_empty() {
                ws_state.editing = Some(draft); // 名字必填,打回表单
                return;
            }
            let id = draft.id.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            let source = DataSource {
                id: id.clone(),
                name: draft.name.clone(),
                driver: draft.driver,
                host: non_empty(&draft.host),
                port: draft.port.parse().ok(),
                database: non_empty(&draft.database),
                username: non_empty(&draft.username),
            };
            if let Some(pos) = ws_state.sources.iter().position(|s| s.id == id) {
                ws_state.sources[pos] = source;
            } else {
                ws_state.sources.push(source);
            }
            if !draft.password.is_empty()
                && let Ok(entry) = keyring_entry(project_id, &id)
            {
                let _ = entry.set_password(&draft.password);
            }
            if let Err(e) = save_sources(repo_path, &ws_state.sources) {
                tracing::warn!("写入 database.json 失败: {e}");
            }
        }
        Message::DraftCancel => {
            ws_state.editing = None;
        }
        Message::DeleteSource(id) => {
            ws_state.sources.retain(|s| s.id != id);
            ws_state.test_status.remove(&id);
            if let Ok(entry) = keyring_entry(project_id, &id) {
                let _ = entry.delete_credential();
            }
            if let Err(e) = save_sources(repo_path, &ws_state.sources) {
                tracing::warn!("写入 database.json 失败: {e}");
            }
        }
        Message::TestConnection(id) => {
            let Some(source) = ws_state.sources.iter().find(|s| s.id == id).cloned() else {
                return;
            };
            ws_state.test_status.insert(id.clone(), TestStatus::Testing);
            let password = keyring_entry(project_id, &id)
                .ok()
                .and_then(|e| e.get_password().ok());
            handle.spawn(async move {
                let result = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    test_connection(source, password),
                )
                .await;
                let result = match result {
                    Ok(r) => r,
                    Err(_) => Err("连接超时(5秒)".to_string()),
                };
                emit(Message::TestConnectionResult(project_id, id, result));
            });
        }
        Message::TestConnectionResult(_project_id, id, result) => {
            let status = match result {
                Ok(()) => TestStatus::Ok,
                Err(e) => TestStatus::Err(e),
            };
            ws_state.test_status.insert(id, status);
        }
    }
}

fn set_draft(ws_state: &mut WorkspaceState, f: impl FnOnce(&mut DataSourceDraft)) {
    if let Some(draft) = ws_state.editing.as_mut() {
        f(draft);
    }
}

fn non_empty(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() { None } else { Some(t.to_string()) }
}

async fn test_connection(source: DataSource, password: Option<String>) -> Result<(), String> {
    match source.driver {
        DriverKind::Postgres | DriverKind::MySQL | DriverKind::Sqlite => {
            let url = build_sql_url(&source, password.as_deref());
            let pool = sqlx::AnyPool::connect(&url).await.map_err(|e| e.to_string())?;
            sqlx::query("SELECT 1")
                .execute(&pool)
                .await
                .map_err(|e| e.to_string())?;
            Ok(())
        }
        DriverKind::MongoDB => {
            let url = build_mongo_url(&source, password.as_deref());
            let client = mongodb::Client::with_uri_str(&url)
                .await
                .map_err(|e| e.to_string())?;
            client
                .list_database_names()
                .await
                .map_err(|e| e.to_string())?;
            Ok(())
        }
    }
}
```

**Step 4 里提到的 `AppState.drivers_popup_open` 字段**:回到 Task 2 Step 2
写的 `AppState` 定义,补一个字段:

```rust
pub struct AppState {
    enabled: std::collections::HashSet<DriverKind>,
    drivers_popup_open: bool,
}
```

`Default`/`load()` 里对应初始化成 `false`,不参与 `save()`/序列化(纯 UI
开关态,不持久化)。`AppState` 加两个方法:

```rust
impl AppState {
    pub fn drivers_popup_open(&self) -> bool {
        self.drivers_popup_open
    }
    pub fn set_drivers_popup_open(&mut self, open: bool) {
        self.drivers_popup_open = open;
    }
}
```

`Message::DriversPopupToggle` 处理器改成:

```rust
Message::DriversPopupToggle => {
    app_state.drivers_popup_open = !app_state.drivers_popup_open;
}
```

- [ ] **Step 6: 编译/测试/lint/格式**

```bash
cargo build -p dozer-app
cargo test -p dozer-app database:: url_tests -- --test-threads=1
cargo clippy -p dozer-app --all-targets
cargo fmt --check
```

预期:全部通过。`Message`/`update`/`test_connection` 还没被 `App::update`
调用,继续有 dead_code 警告——Task 5 解决。

- [ ] **Step 7: Commit**

```bash
git add crates/dozer-app/src/extensions/database.rs crates/dozer-app/src/main.rs
git commit -m "feat(dozer-app): add database connection-test logic and update()"
```

---

### Task 4: `view()` 面板视图

**Files:**
- Modify: `crates/dozer-app/src/extensions/database.rs`

**Interfaces:**
- Consumes:Task 2/3 的全部类型 + `Message`。
- Produces:`pub fn view<'a>(app_state: &'a AppState, ws_state: &'a WorkspaceState, width: Length, outer: Border) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer>`
  ——签名对齐 `project::view`/`browser::view` 那一档"单 pane、带 `outer:
  Border` 参数"的既有惯例(不是 `todo::view` 那种额外带 `project_id`/
  `tabs` 参数的——数据库面板不需要那两样)。

- [ ] **Step 1: 驱动管理弹层**

```rust
fn drivers_popup<'a>(app_state: &'a AppState) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    let mut col = column![
        text("已启用的驱动").size(crate::theme::font::caption()).color(crate::theme::color::DIM)
    ]
    .spacing(8);
    for driver in DriverKind::ALL {
        let enabled = app_state.is_enabled(driver);
        col = col.push(
            button(
                row![
                    text(if enabled { "✓" } else { " " }).size(crate::theme::font::body()),
                    text(driver.label()).size(crate::theme::font::body()).color(crate::theme::color::CREAM),
                ]
                .spacing(8),
            )
            .on_press(Message::ToggleDriver(driver))
            .width(iced_widget::core::Length::Fill)
            .style(|_t: &iced_widget::Theme, _s| iced_widget::button::Style {
                background: Some(crate::theme::color::CARD.into()),
                ..iced_widget::button::Style::default()
            }),
        );
    }
    container(col)
        .padding(12)
        .width(iced_widget::core::Length::Fixed(220.0))
        .style(|_t: &iced_widget::Theme| iced_widget::container::Style {
            background: Some(crate::theme::color::CARD.into()),
            border: iced_widget::core::Border {
                color: crate::theme::color::BORDER,
                width: 1.0,
                radius: 8.0.into(),
            },
            ..iced_widget::container::Style::default()
        })
        .into()
}
```

- [ ] **Step 2: 数据源卡片**

```rust
fn source_card<'a>(
    source: &'a DataSource,
    status: &'a TestStatus,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    let status_text = match status {
        TestStatus::Idle => "".to_string(),
        TestStatus::Testing => "测试中…".to_string(),
        TestStatus::Ok => "✓ 连接成功".to_string(),
        TestStatus::Err(e) => format!("✗ {e}"),
    };
    let status_color = match status {
        TestStatus::Ok => crate::theme::color::GREEN,
        TestStatus::Err(_) => crate::theme::color::RED,
        _ => crate::theme::color::DIM,
    };
    let summary = match source.driver {
        DriverKind::Sqlite => source.database.clone().unwrap_or_default(),
        _ => format!(
            "{}:{}/{}",
            source.host.as_deref().unwrap_or("-"),
            source.port.map(|p| p.to_string()).unwrap_or_default(),
            source.database.as_deref().unwrap_or("-"),
        ),
    };
    container(
        column![
            row![
                text(source.name.clone()).size(crate::theme::font::body()).color(crate::theme::color::CREAM),
                text(source.driver.label()).size(crate::theme::font::caption_sm()).color(crate::theme::color::DIM),
            ]
            .spacing(8),
            text(summary).size(crate::theme::font::caption_sm()).color(crate::theme::color::DIM),
            row![
                button(text("测试连接")).on_press(Message::TestConnection(source.id.clone())),
                button(text("编辑")).on_press(Message::EditSourceStart(source.id.clone())),
                button(text("删除")).on_press(Message::DeleteSource(source.id.clone())),
            ]
            .spacing(8),
            text(status_text).size(crate::theme::font::caption_sm()).color(status_color),
        ]
        .spacing(6),
    )
    .padding(10)
    .width(iced_widget::core::Length::Fill)
    .style(|_t: &iced_widget::Theme| iced_widget::container::Style {
        background: Some(crate::theme::color::CARD.into()),
        border: iced_widget::core::Border {
            color: crate::theme::color::BORDER,
            width: 1.0,
            radius: 8.0.into(),
        },
        ..iced_widget::container::Style::default()
    })
    .into()
}
```

- [ ] **Step 3: 新增/编辑表单**

```rust
fn source_form<'a>(
    draft: &'a DataSourceDraft,
    app_state: &'a AppState,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    let mut driver_row = row![].spacing(8);
    for driver in DriverKind::ALL {
        // 只列已启用的驱动;若正在编辑的数据源本身用的驱动已被禁用,
        // 仍把它加回来并标注"已禁用"(设计文档"目标"第 4 条)——这里判断
        // "已禁用但是当前草稿正用着"这个特例。
        let is_current = draft.driver == driver;
        if !app_state.is_enabled(driver) && !is_current {
            continue;
        }
        let label = if app_state.is_enabled(driver) {
            driver.label().to_string()
        } else {
            format!("{}(已禁用)", driver.label())
        };
        driver_row = driver_row.push(
            button(text(label))
                .on_press(Message::DraftDriverChanged(driver))
                .style(move |_t: &iced_widget::Theme, _s| iced_widget::button::Style {
                    background: Some(
                        if is_current { crate::theme::color::GOLD } else { crate::theme::color::CARD }.into(),
                    ),
                    ..iced_widget::button::Style::default()
                }),
        );
    }

    let mut col = column![driver_row].spacing(8);
    col = col.push(
        text_input("名字", &draft.name)
            .on_input(Message::DraftNameChanged)
            .size(crate::theme::font::body()),
    );
    if draft.driver == DriverKind::Sqlite {
        col = col.push(
            text_input("文件路径", &draft.database)
                .on_input(Message::DraftDatabaseChanged)
                .size(crate::theme::font::body()),
        );
    } else {
        col = col.push(
            text_input("host", &draft.host)
                .on_input(Message::DraftHostChanged)
                .size(crate::theme::font::body()),
        );
        col = col.push(
            text_input("port", &draft.port)
                .on_input(Message::DraftPortChanged)
                .size(crate::theme::font::body()),
        );
        col = col.push(
            text_input("database", &draft.database)
                .on_input(Message::DraftDatabaseChanged)
                .size(crate::theme::font::body()),
        );
        col = col.push(
            text_input("username", &draft.username)
                .on_input(Message::DraftUsernameChanged)
                .size(crate::theme::font::body()),
        );
        col = col.push(
            text_input("password(留空则不修改)", &draft.password)
                .secure(true)
                .on_input(Message::DraftPasswordChanged)
                .size(crate::theme::font::body()),
        );
    }
    col = col.push(
        row![
            button(text("保存")).on_press(Message::DraftSave),
            button(text("取消")).on_press(Message::DraftCancel),
        ]
        .spacing(8),
    );

    container(col)
        .padding(12)
        .width(iced_widget::core::Length::Fill)
        .style(|_t: &iced_widget::Theme| iced_widget::container::Style {
            background: Some(crate::theme::color::CARD.into()),
            border: iced_widget::core::Border {
                color: crate::theme::color::GOLD,
                width: 1.0,
                radius: 8.0.into(),
            },
            ..iced_widget::container::Style::default()
        })
        .into()
}
```

- [ ] **Step 4: 整合 `view`**

```rust
pub fn view<'a>(
    app_state: &'a AppState,
    ws_state: &'a WorkspaceState,
    width: iced_widget::core::Length,
    outer: iced_widget::core::Border,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    let mut col = column![
        row![
            text("数据源").size(crate::theme::font::subtitle()).color(crate::theme::color::CREAM),
            button(text("管理驱动")).on_press(Message::DriversPopupToggle),
            button(text("＋新增数据源")).on_press(Message::AddSourceStart),
        ]
        .spacing(8)
        .align_y(iced_widget::core::Alignment::Center),
    ]
    .spacing(12);

    if app_state.drivers_popup_open() {
        col = col.push(drivers_popup(app_state));
    }

    if let Some(draft) = ws_state.editing() {
        col = col.push(source_form(draft, app_state));
    }

    if ws_state.sources().is_empty() {
        col = col.push(
            text("还没有数据源").size(crate::theme::font::body()).color(crate::theme::color::DIM),
        );
    } else {
        for source in ws_state.sources() {
            col = col.push(source_card(source, ws_state.test_status(&source.id)));
        }
    }

    container(col.padding(16))
        .width(width)
        .height(iced_widget::core::Length::Fill)
        .style(move |_t: &iced_widget::Theme| iced_widget::container::Style {
            background: Some(crate::theme::color::BG.into()),
            border: outer,
            ..iced_widget::container::Style::default()
        })
        .into()
}
```

（这一步顶部需要给文件加 `use iced_widget::{Element, button, column,
container, row, text, text_input};` 之类的组件导入——写代码时按现有
`extensions::todo.rs`/`extensions::project.rs` 顶部 `use` 块的写法照抄,
不在这里重复列一遍。`theme::color::GREEN` 需要核对确实存在这个令牌
——`CLAUDE.md` 列的核心色板里有"绿 #1AD585",大概率叫 `GREEN`,写代码时
`grep -n "pub const GREEN" crates/dozer-app/src/theme/color.rs` 核对一下
实际常量名。）

- [ ] **Step 5: 编译/lint/格式**

```bash
cargo build -p dozer-app
cargo clippy -p dozer-app --all-targets
cargo fmt --check
```

预期:`view` 函数本身编译通过;它还没被任何调用点引用,`dead_code` 警告
继续保留到 Task 5。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/extensions/database.rs
git commit -m "feat(dozer-app): add database panel view"
```

---

### Task 5: 接入内核(`LeftView`/rail 图标/面板区分发/`Message`/`App::update`)

**Files:**
- Modify:`workspace.rs`(若 `App`/`Workspace` 拆分已合并,按符号名分布到
  `app.rs`/`workspace.rs`——见 Global Constraints)。

**Interfaces:**
- Consumes:Task 2-4 的 `database::{AppState, WorkspaceState, Message,
  update, view, reload_from_disk}`。

- [ ] **Step 1: `LeftView` 加变体**

找到 `pub enum LeftView { Files, Web, GitLog, Todo, Project }`,加第 6 个:

```rust
pub enum LeftView {
    Files,
    Web,
    GitLog,
    Todo,
    Project,
    Database,
}
```

- [ ] **Step 2: 补全穷尽 `match state.left_view`/`match app.left_view`**

`grep -n "LeftView::Project =>" workspace.rs`(或拆分后对应文件)定位——
现状有 **5 处**穷尽 match 需要各加一个 `LeftView::Database` 分支:

1. `preview_content_bounds` 函数内,两处(一处提前 `return` 的 `match`,
   一处兜底 `match`)——`Database` 同 `GitLog`/`Todo`/`Project`,返回
   `(0.0, 0.0, 0.0, 0.0)`(数据库面板没有"预览内容列"这个概念)。
2. `is_in_preview_column` 函数内,同样两处——`Database` 返回 `false`。
3. `left_panel_area` 函数内的内容分发大 `match`(`LeftView::Project =>
   project::view(..)` 那个),加:

   ```rust
   LeftView::Database => {
       let Some(project_id) = ws.project.as_ref().map(|p| p.id) else {
           return column![].into();
       };
       database::view(
           &app.database,
           &ws.database,
           Length::Fill,
           zone_pane_border(zone, ac),
       )
       .map(Message::Database)
   }
   ```

- [ ] **Step 3: `RailButton` 加变体 + rail 图标按钮**

`pub enum RailButton { .. LeftProject, .. }` 里加 `LeftDatabase`。

`left_icon_rail` 函数里,在 `LeftProject` 那个 `MouseArea::new(rail_icon_
button(..))` 块之后加一份同结构的:

```rust
        // 数据库面板入口:数据源管理 + 连接测试。
        MouseArea::new(rail_icon_button(
            icons::IconKind::Database,
            app.left_view == LeftView::Database && left_open,
            app.hover_progress(HoverId::Rail(RailButton::LeftDatabase)),
            Message::LeftIconSelect(LeftView::Database),
        ))
        .on_enter(Message::Hover(HoverId::Rail(RailButton::LeftDatabase), true))
        .on_exit(Message::Hover(HoverId::Rail(RailButton::LeftDatabase), false)),
```

- [ ] **Step 4: `App`/`Workspace` struct 加字段**

`App` struct 里,`todo: todo::AppState,` 那一行附近加:

```rust
    database: database::AppState,
```

`Workspace` struct 里,`todo: todo::WorkspaceState,` 那一行附近加:

```rust
    database: database::WorkspaceState,
```

对应的两处初始化:`App::new_shell`(或 `bootstrap`,`todo:
todo::AppState::load(),` 那一行附近)加 `database:
database::AppState::load(),`;`Workspace::from_restore`(`todo:
todo::WorkspaceState::default(),` 那一行附近)加 `database:
database::WorkspaceState::default(),`。

- [ ] **Step 5: `Message` 加变体**

`Todo(todo::Message),` 那一行附近加:

```rust
    Database(database::Message),
```

- [ ] **Step 6: `App::update` 分发**

照抄 `Message::Todo(msg) => { .. }` 那段的结构(`loaded_workspace_mut` 直接
借 `&mut Workspace` + `&mut self.database`),但要拆成两个 match 臂——一个
处理带显式 `project_id` 的异步结果(`TestConnectionResult`),一个处理其余
所有走"当前聚焦项目"的同步交互消息(理由见设计文档"结果经 `emit` 回传"
一节——异步结果不能假设聚焦项目没变):

```rust
            Message::Database(database::Message::TestConnectionResult(
                project_id,
                source_id,
                result,
            )) => {
                let app_db = &mut self.database;
                let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
                    return;
                };
                let Some(project) = ws.project.as_ref() else {
                    return;
                };
                let repo_path = std::path::PathBuf::from(&project.path);
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let emit = move |m| {
                    let _ = proxy.send_event(Message::Database(m));
                };
                database::update(
                    &mut ws.database,
                    app_db,
                    database::Message::TestConnectionResult(project_id, source_id, result),
                    project_id,
                    &repo_path,
                    &handle,
                    emit,
                );
            }
            Message::Database(msg) => {
                let Some(project_id) = self.active_project_id else {
                    return;
                };
                let app_db = &mut self.database;
                let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
                    return;
                };
                let Some(project) = ws.project.as_ref() else {
                    return;
                };
                let repo_path = std::path::PathBuf::from(&project.path);
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let emit = move |m| {
                    let _ = proxy.send_event(Message::Database(m));
                };
                database::update(&mut ws.database, app_db, msg, project_id, &repo_path, &handle, emit);
            }
```

（这两个 match 臂要放在 `Message::Todo(msg) => { .. }` 附近,且
`TestConnectionResult` 那个特化分支必须排在通配的 `Message::Database(msg)
=> ..` **之前**——Rust `match` 按顺序匹配,通配分支在前会让特化分支永远
匹配不到,同 `browser.rs` 现有 `Message::Browser(browser::Message::
BookmarksLoaded(..)) => ..` 必须排在 `Message::Browser(msg) => ..` 之前
的既有顺序要求一致。）

- [ ] **Step 7: 进面板时重读磁盘**

`Message::LeftIconSelect(v)` 处理器里,`if self.left_view == LeftView::Todo
{ .. }` 那个分支之后,加一段同结构的:

```rust
                if self.left_view == LeftView::Database {
                    self.with_focused_project(|ws, _io| {
                        if let Some(project) = ws.project.as_ref() {
                            database::reload_from_disk(
                                &mut ws.database,
                                std::path::Path::new(&project.path),
                            );
                        }
                    });
                }
```

- [ ] **Step 8: 编译/测试/lint/格式收敛**

```bash
cargo build -p dozer-app 2>&1 | grep -E "^error"
```

反复跑,按报错补漏(常见:某处 `use database;`/`use crate::extensions::
database;` 没加;`use crate::extensions::database;` 应该已经在
`workspace.rs`/`app.rs` 顶部批量 `use crate::extensions::{acceptance,
browser, files, git_log, project, todo, usage};` 这一档 import 里补一个
`database`)。

```bash
cargo build -p dozer-app
cargo test -p dozer-app -- --test-threads=1
cargo clippy -p dozer-app --all-targets
cargo fmt --check
```

预期:全部干净通过,**零警告**——Task 1-4 遗留的 dead_code 应该在这一步
全部消失(`IconKind::Database`/`database::*` 现在都被 rail 按钮/`Message`/
`App::update`/`left_panel_area` 实际引用了)。

- [ ] **Step 9: Commit**

```bash
git add -u
git commit -m "feat(dozer-app): wire database panel into LeftView/kernel"
```

---

### Task 6: 全量验证 + 人工验收

**Files:** 无新改动,只跑验证命令。

- [ ] **Step 1: 全量构建/测试/lint/格式**

```bash
cargo build
cargo test
cargo clippy --all-targets
cargo fmt --check
```

预期:全部干净通过,零警告。

- [ ] **Step 2: 人工验收**

```bash
cargo run -p dozer-app
```

1. 打开一个项目,点左图标栏新出现的"数据库"图标,面板显示"还没有数据源"。
2. 点"管理驱动",能看到 4 个驱动的勾选状态,取消勾选 MongoDB 后关闭弹层,
   再点"＋新增数据源",驱动选项里 MongoDB 不出现;重新勾选后再出现。
3. 新增一条 SQLite 数据源,文件路径指向一个真实存在的本地 `.sqlite` 文件
   (没有的话先用 `sqlite3 /tmp/test.db "create table t(id int);"` 建一个),
   保存后卡片出现,点"测试连接"几秒内显示"✓ 连接成功"。
4. 新增一条 Postgres 数据源,host 填一个不存在的地址(如
   `nonexistent.invalid`),点"测试连接",5 秒左右显示超时错误,不是无限
   转圈。
5. 编辑刚才的 SQLite 数据源,改个名字保存,卡片标题更新。
6. 删除一条数据源,卡片消失。
7. 完全退出 app 再重新打开、回到同一个项目、进数据库面板:确认数据源列表
   还在(`.dozer/database.json` 生效),点"测试连接"密码类数据源(如果有
   配置密码的 Postgres/MySQL 数据源)仍能正常连接(Keychain 密码往返正常)。
8. 切到另一个项目,数据源列表应该是空的(或那个项目自己独立配置的)——
   确认数据源确实是按项目隔离,不是全局共享。

- [ ] **Step 3: 确认分支状态**

```bash
git status
git log --oneline main..HEAD
```

确认工作区干净、当前分支只比 `main` 多这次的提交。按 Global Constraints
提请代码审阅,审阅通过后再合并——不在这个计划里自动合并。
