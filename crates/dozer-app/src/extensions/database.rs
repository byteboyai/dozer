//! 数据库面板 · 阶段 1:驱动管理 + 每项目数据源 CRUD + 连接测试。结构照搬
//! `extensions::todo` 的双状态模式——`AppState` 挂 `App`(全局:哪些驱动
//! 启用),`WorkspaceState` 挂每个 `Workspace`(当前项目的数据源列表)。
//! 密码不进这个文件的任何持久化结构,单独走 macOS Keychain(见
//! `keyring_key`)。schema 树/数据浏览/SQL 执行/MongoDB 集合浏览留后续
//! 阶段,见
//! `docs/superpowers/specs/2026-08-08-database-panel-phase1-design.md`。

use iced_widget::core::{Border, Element, Length};
use iced_widget::{button, column, container, row, text, text_input};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
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

/// schema 树里的一个表/视图(阶段 2)。
#[derive(Debug, Clone, PartialEq)]
pub struct TableRef {
    /// 仅 Postgres 有值(table_schema);MySQL/SQLite 恒 None。
    pub schema: Option<String>,
    pub name: String,
    pub is_view: bool,
}

/// 列元信息(阶段 2)。主键/索引/默认值不在本阶段范围(设计文档"非目标")。
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnInfo {
    pub name: String,
    pub type_name: String,
    pub nullable: bool,
}

/// 某张表的列加载进度(阶段 2)。
#[derive(Debug, Clone)]
pub enum ColumnLoad {
    Loading,
    Loaded(Vec<ColumnInfo>),
    Failed(String),
}

/// 单个数据源的 schema 树状态(**纯内存,不持久化**——schema 是即时快照,
/// 重启后重新拉,展开状态不值得落盘)。
#[derive(Debug, Default)]
pub struct SchemaState {
    loading_tables: bool,
    tables_error: Option<String>,
    /// 按 (schema, name) 有序(SQL 层 ORDER BY,客户端不再排序)。
    tables: Vec<TableRef>,
    /// Postgres schema 节点展开集。
    expanded_schemas: HashSet<String>,
    /// 表节点展开集,key = (schema, name)。
    expanded_tables: HashSet<(Option<String>, String)>,
    /// 每表列缓存,key = (schema, name)。
    columns: HashMap<(Option<String>, String), ColumnLoad>,
}

impl SchemaState {
    pub fn loading_tables(&self) -> bool {
        self.loading_tables
    }

    pub fn tables_error(&self) -> Option<&str> {
        self.tables_error.as_deref()
    }

    pub fn tables(&self) -> &[TableRef] {
        &self.tables
    }

    /// 指定表的列加载态(单测断言与过期防线用)。
    pub fn column_load(&self, key: &(Option<String>, String)) -> Option<&ColumnLoad> {
        self.columns.get(key)
    }
}

/// schema 树可见行(摊平结果)。
pub struct SchemaRow<'a> {
    pub kind: SchemaRowKind<'a>,
    pub depth: usize,
    pub expanded: bool,
}

/// schema 树行类型。`ColumnsLoading`/`ColumnsFailed` 是展开表之后的占位行。
pub enum SchemaRowKind<'a> {
    Schema(&'a str),
    Table(&'a TableRef),
    Column(&'a ColumnInfo),
    ColumnsLoading,
    ColumnsFailed(&'a str),
}

/// 把 `SchemaState` 摊平成可见行(纯函数,单测友好;渲染侧单层循环)。
/// Postgres 比 MySQL/SQLite 多一层 schema 节点(schema 节点不做异步加载,
/// 表列表一次查全后客户端按 `table_schema` 分组)。
pub fn tree_rows(state: &SchemaState, driver: DriverKind) -> Vec<SchemaRow<'_>> {
    let mut out: Vec<SchemaRow<'_>> = Vec::new();
    if driver == DriverKind::Postgres {
        let mut by_schema: BTreeMap<&str, Vec<&TableRef>> = BTreeMap::new();
        for t in &state.tables {
            by_schema
                .entry(t.schema.as_deref().unwrap_or("public"))
                .or_default()
                .push(t);
        }
        for (schema, tables) in by_schema {
            let expanded = state.expanded_schemas.contains(schema);
            out.push(SchemaRow {
                kind: SchemaRowKind::Schema(schema),
                depth: 0,
                expanded,
            });
            if expanded {
                push_table_rows(&mut out, state, &tables, 1);
            }
        }
    } else {
        let tables: Vec<&TableRef> = state.tables.iter().collect();
        push_table_rows(&mut out, state, &tables, 0);
    }
    out
}

fn push_table_rows<'a>(
    out: &mut Vec<SchemaRow<'a>>,
    state: &'a SchemaState,
    tables: &[&'a TableRef],
    depth: usize,
) {
    for t in tables {
        let key = (t.schema.clone(), t.name.clone());
        let expanded = state.expanded_tables.contains(&key);
        out.push(SchemaRow {
            kind: SchemaRowKind::Table(t),
            depth,
            expanded,
        });
        if !expanded {
            continue;
        }
        match state.columns.get(&key) {
            // 展开动作总会伴随加载触发,正常到不了这里;防御性忽略。
            None => {}
            Some(ColumnLoad::Loading) => out.push(SchemaRow {
                kind: SchemaRowKind::ColumnsLoading,
                depth: depth + 1,
                expanded: false,
            }),
            Some(ColumnLoad::Failed(e)) => out.push(SchemaRow {
                kind: SchemaRowKind::ColumnsFailed(e),
                depth: depth + 1,
                expanded: false,
            }),
            Some(ColumnLoad::Loaded(cols)) => {
                for c in cols {
                    out.push(SchemaRow {
                        kind: SchemaRowKind::Column(c),
                        depth: depth + 1,
                        expanded: false,
                    });
                }
            }
        }
    }
}

/// 表/视图清单 SQL(免绑定参数;三种后端各写各的,is_view 在 Rust 侧判断)。
fn tables_sql(driver: DriverKind) -> &'static str {
    match driver {
        DriverKind::Postgres => {
            "SELECT table_schema, table_name, table_type \
             FROM information_schema.tables \
             WHERE table_schema NOT IN ('pg_catalog', 'information_schema') \
             ORDER BY table_schema, table_name"
        }
        DriverKind::MySQL => {
            "SELECT table_name, table_type \
             FROM information_schema.tables \
             WHERE table_schema = DATABASE() \
             ORDER BY table_name"
        }
        DriverKind::Sqlite => {
            "SELECT name, type FROM sqlite_master \
             WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite_%' \
             ORDER BY name"
        }
        DriverKind::MongoDB => {
            unreachable!("tables_sql 不处理 MongoDB(UI 无入口 + load_tables 双保险)")
        }
    }
}

/// 列清单 SQL(需绑定参数 → 占位符风格按后端)。
fn columns_sql(driver: DriverKind) -> &'static str {
    match driver {
        DriverKind::Postgres => {
            "SELECT column_name, data_type, is_nullable \
             FROM information_schema.columns \
             WHERE table_schema = $1 AND table_name = $2 \
             ORDER BY ordinal_position"
        }
        DriverKind::MySQL => {
            "SELECT column_name, data_type, is_nullable \
             FROM information_schema.columns \
             WHERE table_schema = DATABASE() AND table_name = ? \
             ORDER BY ordinal_position"
        }
        // pragma_table_info 表值函数:常规 SELECT,`Any` 驱动下比 PRAGMA 语句稳
        DriverKind::Sqlite => "SELECT name, type, notnull FROM pragma_table_info(?) ORDER BY cid",
        DriverKind::MongoDB => unreachable!("columns_sql 不处理 MongoDB"),
    }
}

async fn load_tables(kind: DriverKind, url: &str) -> Result<Vec<TableRef>, String> {
    if kind == DriverKind::MongoDB {
        return Err("MongoDB 集合浏览将在后续阶段支持".to_string());
    }
    let work = async {
        let pool = sqlx::AnyPool::connect(url)
            .await
            .map_err(|e| e.to_string())?;
        let out = async {
            let sql = tables_sql(kind);
            if kind == DriverKind::Postgres {
                let rows: Vec<(String, String, String)> = sqlx::query_as(sql)
                    .fetch_all(&pool)
                    .await
                    .map_err(|e| e.to_string())?;
                Ok::<_, String>(
                    rows.into_iter()
                        .map(|(schema, name, table_type)| TableRef {
                            schema: Some(schema),
                            name,
                            is_view: table_type == "VIEW",
                        })
                        .collect(),
                )
            } else {
                let rows: Vec<(String, String)> = sqlx::query_as(sql)
                    .fetch_all(&pool)
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(rows
                    .into_iter()
                    .map(|(name, ty)| TableRef {
                        schema: None,
                        name,
                        // MySQL: information_schema.tables.table_type == 'VIEW';
                        // SQLite: sqlite_master.type == 'view'
                        is_view: if kind == DriverKind::MySQL {
                            ty == "VIEW"
                        } else {
                            ty == "view"
                        },
                    })
                    .collect())
            }
        }
        .await;
        pool.close().await;
        out
    };
    match tokio::time::timeout(std::time::Duration::from_secs(5), work).await {
        Ok(r) => r,
        Err(_) => Err("加载超时(5秒)".to_string()),
    }
}

async fn load_columns(
    kind: DriverKind,
    url: &str,
    schema: Option<&str>,
    table: &str,
) -> Result<Vec<ColumnInfo>, String> {
    if kind == DriverKind::MongoDB {
        return Err("MongoDB 集合浏览将在后续阶段支持".to_string());
    }
    let work = async {
        let pool = sqlx::AnyPool::connect(url)
            .await
            .map_err(|e| e.to_string())?;
        let out = async {
            let out: Vec<ColumnInfo> = match kind {
                DriverKind::Postgres => {
                    let rows: Vec<(String, String, String)> = sqlx::query_as(columns_sql(kind))
                        .bind(schema.unwrap_or("public"))
                        .bind(table)
                        .fetch_all(&pool)
                        .await
                        .map_err(|e| e.to_string())?;
                    rows.into_iter()
                        .map(|(name, dt, nullable)| ColumnInfo {
                            name,
                            type_name: dt,
                            nullable: nullable == "YES",
                        })
                        .collect()
                }
                DriverKind::MySQL => {
                    let rows: Vec<(String, String, String)> = sqlx::query_as(columns_sql(kind))
                        .bind(table)
                        .fetch_all(&pool)
                        .await
                        .map_err(|e| e.to_string())?;
                    rows.into_iter()
                        .map(|(name, dt, nullable)| ColumnInfo {
                            name,
                            type_name: dt,
                            nullable: nullable == "YES",
                        })
                        .collect()
                }
                DriverKind::Sqlite => {
                    let rows: Vec<(String, String, i64)> = sqlx::query_as(columns_sql(kind))
                        .bind(table)
                        .fetch_all(&pool)
                        .await
                        .map_err(|e| e.to_string())?;
                    rows.into_iter()
                        .map(|(name, ty, notnull)| ColumnInfo {
                            name,
                            type_name: ty,
                            nullable: notnull == 0,
                        })
                        .collect()
                }
                DriverKind::MongoDB => unreachable!("load_columns 已在入口拦下 MongoDB"),
            };
            Ok::<_, String>(out)
        }
        .await;
        pool.close().await;
        out
    };
    match tokio::time::timeout(std::time::Duration::from_secs(5), work).await {
        Ok(r) => r,
        Err(_) => Err("加载超时(5秒)".to_string()),
    }
}

fn spawn_tables_load(
    handle: &tokio::runtime::Handle,
    project_id: i64,
    source_id: String,
    kind: DriverKind,
    url: String,
    emit: impl Fn(Message) + Send + 'static,
) {
    handle.spawn(async move {
        let result = load_tables(kind, &url).await;
        emit(Message::TablesLoaded(project_id, source_id, result));
    });
}

fn spawn_columns_load(
    handle: &tokio::runtime::Handle,
    project_id: i64,
    source_id: String,
    kind: DriverKind,
    url: String,
    schema: Option<String>,
    table: String,
    emit: impl Fn(Message) + Send + 'static,
) {
    handle.spawn(async move {
        let result = load_columns(kind, &url, schema.as_deref(), &table).await;
        emit(Message::ColumnsLoaded {
            project_id,
            source_id,
            schema,
            table,
            result,
        });
    });
}

fn driver_of(ws_state: &WorkspaceState, source_id: &str) -> Option<DriverKind> {
    ws_state
        .sources
        .iter()
        .find(|s| s.id == source_id)
        .map(|s| s.driver)
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
/// 的连接测试状态 + schema 树浏览态(阶段 2,纯内存)。
#[derive(Debug, Default)]
pub struct WorkspaceState {
    sources: Vec<DataSource>,
    editing: Option<DataSourceDraft>,
    test_status: HashMap<String, TestStatus>,
    /// 正在浏览 schema 树的数据源 id;`None` = 卡片列表视图(阶段 2)。
    browsing: Option<String>,
    /// 每个数据源 id 一份 schema 树状态(阶段 2,纯内存)。
    schemas: HashMap<String, SchemaState>,
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

    /// 当前正在浏览的数据源及其 schema 树状态(阶段 2 树视图用)。
    /// `reload_from_disk` 后 `browsing` 可能指向磁盘已不存在的源——取不到
    /// 返回 `None`,调用方回退卡片列表即可,不专门清理(设计文档 §2 过期防线)。
    pub fn browsing_source(&self) -> Option<(&DataSource, &SchemaState)> {
        let id = self.browsing.as_deref()?;
        let source = self.sources.iter().find(|s| s.id == id)?;
        let st = self.schemas.get(id)?;
        Some((source, st))
    }

    /// 指定数据源的 schema 树状态只读视图(状态机单测断言用)。
    pub fn schema_state(&self, source_id: &str) -> Option<&SchemaState> {
        self.schemas.get(source_id)
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

/// 连接测试 / 表单交互的统一消息。`TestConnectionResult` 特化携带
/// `project_id`——异步结果可能晚于用户切换项目才回来,必须按这个项目 id
/// 而不是"当前聚焦项目"路由回正确的 `WorkspaceState`。
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
    /// 点"浏览结构":`browsing = Some(id)`;该源从未加载过则自动发起表加载
    /// (有缓存/有错误保留现状,错误态由"重试"触发)。
    BrowseSchema(String),
    /// schema 树顶部"← 返回":回到卡片列表(树状态保留,再进入不重拉)。
    SchemaBack,
    /// schema 树顶部"刷新":重拉表列表(旧快照保留不闪空,成功后对账)。
    /// 处理前先核对 `browsing == Some(id)`,防旧视图残留按钮。
    SchemaRefresh(String),
    /// Postgres schema 节点展开/收起(纯同步,不触发加载)。
    ToggleSchema(String),
    /// 表节点展开/收起;展开时列缓存缺失或曾失败 → 置 `Loading` 并发起列加载。
    ToggleTable {
        source_id: String,
        schema: Option<String>,
        table: String,
    },
    /// 表清单异步结果。**带 `project_id`**——结果可能晚于用户切走项目页签,
    /// 必须按自带 id 路由(同 `TestConnectionResult` 的口径)。
    TablesLoaded(i64, String, Result<Vec<TableRef>, String>),
    /// 列加载异步结果,带 `project_id`/`source_id` 双路由。
    ColumnsLoaded {
        project_id: i64,
        source_id: String,
        schema: Option<String>,
        table: String,
        result: Result<Vec<ColumnInfo>, String>,
    },
}

pub fn update(
    ws_state: &mut WorkspaceState,
    app_state: &mut AppState,
    msg: Message,
    project_id: i64,
    repo_path: &Path,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + Clone + 'static,
) {
    match msg {
        Message::ToggleDriver(driver) => app_state.toggle(driver),
        Message::DriversPopupToggle => {
            app_state.drivers_popup_open = !app_state.drivers_popup_open;
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
            let id = draft
                .id
                .clone()
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
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
            // 编辑已有源:连接信息可能变了,旧结构快照不作数(设计文档 §2)。
            if draft.id.is_some() {
                ws_state.schemas.remove(&id);
                if ws_state.browsing.as_deref() == Some(id.as_str()) {
                    ws_state.browsing = None;
                }
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
            ws_state.schemas.remove(&id);
            if ws_state.browsing.as_deref() == Some(id.as_str()) {
                ws_state.browsing = None;
            }
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
        Message::BrowseSchema(id) => {
            let Some(source) = ws_state.sources.iter().find(|s| s.id == id).cloned() else {
                return;
            };
            ws_state.browsing = Some(id.clone());
            let st = ws_state.schemas.entry(id.clone()).or_default();
            // 从未加载过才拉:有缓存 → 直接显示;有错误 → 等用户点"重试"
            let need_load = st.tables.is_empty() && !st.loading_tables && st.tables_error.is_none();
            if !need_load {
                return;
            }
            st.loading_tables = true;
            let password = keyring_entry(project_id, &id)
                .ok()
                .and_then(|e| e.get_password().ok());
            spawn_tables_load(
                handle,
                project_id,
                id,
                source.driver,
                build_sql_url(&source, password.as_deref()),
                emit,
            );
        }
        Message::SchemaBack => {
            ws_state.browsing = None;
        }
        Message::SchemaRefresh(source_id) => {
            if ws_state.browsing.as_deref() != Some(source_id.as_str()) {
                return; // 旧视图残留按钮防线
            }
            let Some(source) = ws_state.sources.iter().find(|s| s.id == source_id).cloned() else {
                return;
            };
            let st = ws_state.schemas.entry(source_id.clone()).or_default();
            st.loading_tables = true;
            st.tables_error = None;
            // 旧表快照保留不闪空(设计文档 UI 节)
            let password = keyring_entry(project_id, &source_id)
                .ok()
                .and_then(|e| e.get_password().ok());
            spawn_tables_load(
                handle,
                project_id,
                source_id,
                source.driver,
                build_sql_url(&source, password.as_deref()),
                emit,
            );
        }
        Message::ToggleSchema(name) => {
            let Some(browsing) = ws_state.browsing.clone() else {
                return;
            };
            let Some(st) = ws_state.schemas.get_mut(&browsing) else {
                return;
            };
            if !st.expanded_schemas.remove(&name) {
                st.expanded_schemas.insert(name);
            }
        }
        Message::ToggleTable {
            source_id,
            schema,
            table,
        } => {
            if ws_state.browsing.as_deref() != Some(source_id.as_str()) {
                return; // 旧视图残留按钮防线
            }
            let Some(source) = ws_state.sources.iter().find(|s| s.id == source_id).cloned() else {
                return;
            };
            let key = (schema, table);
            let Some(st) = ws_state.schemas.get_mut(&source_id) else {
                return;
            };
            if st.expanded_tables.remove(&key) {
                return; // 收起:保留缓存,不取消飞行中的加载(回来照常写缓存)
            }
            st.expanded_tables.insert(key.clone());
            if matches!(
                st.columns.get(&key),
                Some(ColumnLoad::Loaded(_)) | Some(ColumnLoad::Loading)
            ) {
                return;
            }
            // 缺失或曾失败 → (重新)加载
            st.columns.insert(key.clone(), ColumnLoad::Loading);
            let password = keyring_entry(project_id, &source_id)
                .ok()
                .and_then(|e| e.get_password().ok());
            spawn_columns_load(
                handle,
                project_id,
                source.id.clone(),
                source.driver,
                build_sql_url(&source, password.as_deref()),
                key.0,
                key.1,
                emit,
            );
        }
        Message::TablesLoaded(_project_id, source_id, result) => {
            // 借用顺序:先只读查驱动,再拿 schemas 可变借用(st 存活期间
            // 不能再回借 ws_state.sources,见下 Ok 分支尾部)
            let is_pg = driver_of(ws_state, &source_id) == Some(DriverKind::Postgres);
            let Some(st) = ws_state.schemas.get_mut(&source_id) else {
                return;
            };
            if !st.loading_tables {
                return; // 过期防线:源已删/被新一轮加载覆盖
            }
            st.loading_tables = false;
            match result {
                Err(e) => {
                    st.tables_error = Some(e); // 旧快照保留,视图按有无快照分别渲染
                }
                Ok(tables) => {
                    st.tables_error = None;
                    st.tables = tables;
                    // 对账一:新表列表里不存在的展开项/列缓存清掉
                    let keys: HashSet<(Option<String>, String)> = st
                        .tables
                        .iter()
                        .map(|t| (t.schema.clone(), t.name.clone()))
                        .collect();
                    st.expanded_tables.retain(|k| keys.contains(k));
                    st.columns.retain(|k, _| keys.contains(k));
                    // 对账二:Postgres 全表同属一个 schema → 自动展开(MVP 少一次点击)
                    if is_pg {
                        let distinct: HashSet<&str> = st
                            .tables
                            .iter()
                            .filter_map(|t| t.schema.as_deref())
                            .collect();
                        if distinct.len() == 1 {
                            if let Some(only) = distinct.into_iter().next() {
                                st.expanded_schemas.insert(only.to_string());
                            }
                        }
                    }
                    // 对账三:仍然展开的表全部重新拉列(置 Loading + spawn)
                    let reload: Vec<(Option<String>, String)> =
                        st.expanded_tables.iter().cloned().collect();
                    for key in &reload {
                        st.columns.insert(key.clone(), ColumnLoad::Loading);
                    }
                    let Some(source) = ws_state.sources.iter().find(|s| s.id == source_id).cloned()
                    else {
                        return;
                    };
                    for key in reload {
                        let password = keyring_entry(project_id, &source.id)
                            .ok()
                            .and_then(|e| e.get_password().ok());
                        spawn_columns_load(
                            handle,
                            project_id,
                            source.id.clone(),
                            source.driver,
                            build_sql_url(&source, password.as_deref()),
                            key.0,
                            key.1,
                            emit.clone(),
                        );
                    }
                }
            }
        }
        Message::ColumnsLoaded {
            project_id: _project_id,
            source_id,
            schema,
            table,
            result,
        } => {
            let Some(st) = ws_state.schemas.get_mut(&source_id) else {
                return;
            };
            let key = (schema, table);
            if !matches!(st.columns.get(&key), Some(ColumnLoad::Loading)) {
                return; // 过期防线:只接收仍在等的加载结果
            }
            st.columns.insert(
                key,
                match result {
                    Ok(cols) => ColumnLoad::Loaded(cols),
                    Err(e) => ColumnLoad::Failed(e),
                },
            );
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
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

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

async fn test_connection(source: DataSource, password: Option<String>) -> Result<(), String> {
    match source.driver {
        DriverKind::Postgres | DriverKind::MySQL | DriverKind::Sqlite => {
            let url = build_sql_url(&source, password.as_deref());
            let pool = sqlx::AnyPool::connect(&url)
                .await
                .map_err(|e| e.to_string())?;
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

/// 驱动管理弹层:列出全部驱动类型,点按切换启用/禁用。
fn drivers_popup<'a>(
    app_state: &'a AppState,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    let mut col = column![
        text("已启用的驱动")
            .size(crate::theme::font::caption())
            .color(crate::theme::color::DIM)
    ]
    .spacing(8);
    for driver in DriverKind::ALL {
        let enabled = app_state.is_enabled(driver);
        col = col.push(
            button(
                row![
                    text(if enabled { "✓" } else { " " }).size(crate::theme::font::body()),
                    text(driver.label())
                        .size(crate::theme::font::body())
                        .color(crate::theme::color::CREAM),
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

/// 单条数据源卡片:名称 + 驱动 + 连接摘要 + 测试/编辑/删除按钮 + 测试状态。
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
                text(source.name.clone())
                    .size(crate::theme::font::body())
                    .color(crate::theme::color::CREAM),
                text(source.driver.label())
                    .size(crate::theme::font::caption_sm())
                    .color(crate::theme::color::DIM),
            ]
            .spacing(8),
            text(summary)
                .size(crate::theme::font::caption_sm())
                .color(crate::theme::color::DIM),
            row![
                button(text("测试连接")).on_press(Message::TestConnection(source.id.clone())),
                button(text("编辑")).on_press(Message::EditSourceStart(source.id.clone())),
                button(text("删除")).on_press(Message::DeleteSource(source.id.clone())),
            ]
            .spacing(8),
            text(status_text)
                .size(crate::theme::font::caption_sm())
                .color(status_color),
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

/// 新增/编辑数据源表单。SQLite 只留"文件路径"一栏,其它驱动列出 host/port/
/// database/username/password。
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
                .style(
                    move |_t: &iced_widget::Theme, _s| iced_widget::button::Style {
                        background: Some(
                            if is_current {
                                crate::theme::color::GOLD
                            } else {
                                crate::theme::color::CARD
                            }
                            .into(),
                        ),
                        ..iced_widget::button::Style::default()
                    },
                ),
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

/// 面板主视图:驱动管理弹层 + 新增/编辑表单 + 数据源卡片列表。
pub fn view<'a>(
    app_state: &'a AppState,
    ws_state: &'a WorkspaceState,
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    let mut col = column![
        row![
            text("数据源")
                .size(crate::theme::font::subtitle())
                .color(crate::theme::color::CREAM),
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
            text("还没有数据源")
                .size(crate::theme::font::body())
                .color(crate::theme::color::DIM),
        );
    } else {
        for source in ws_state.sources() {
            col = col.push(source_card(source, ws_state.test_status(&source.id)));
        }
    }

    container(col.padding(16))
        .width(width)
        .height(iced_widget::core::Length::Fill)
        .style(
            move |_t: &iced_widget::Theme| iced_widget::container::Style {
                background: Some(crate::theme::color::BG.into()),
                border: outer,
                ..iced_widget::container::Style::default()
            },
        )
        .into()
}

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

    fn tree_table(schema: Option<&str>, name: &str, is_view: bool) -> TableRef {
        TableRef {
            schema: schema.map(|s| s.to_string()),
            name: name.into(),
            is_view,
        }
    }

    #[test]
    fn tree_rows_flat_for_sqlite() {
        let mut st = SchemaState::default();
        st.tables = vec![
            tree_table(None, "users", false),
            tree_table(None, "orders", true),
        ];
        st.expanded_tables.insert((None, "users".into()));
        st.columns.insert(
            (None, "users".into()),
            ColumnLoad::Loaded(vec![ColumnInfo {
                name: "id".into(),
                type_name: "INTEGER".into(),
                nullable: false,
            }]),
        );
        let rows = tree_rows(&st, DriverKind::Sqlite);
        assert_eq!(rows.len(), 3);
        assert!(matches!(rows[0].kind, SchemaRowKind::Table(t) if t.name == "users"));
        assert_eq!(rows[0].depth, 0);
        assert!(rows[0].expanded);
        assert!(matches!(rows[1].kind, SchemaRowKind::Column(c) if c.name == "id" && !c.nullable));
        assert_eq!(rows[1].depth, 1);
        assert!(matches!(rows[2].kind, SchemaRowKind::Table(t) if t.name == "orders" && t.is_view));
    }

    #[test]
    fn tree_rows_postgres_groups_by_schema_and_folds() {
        let mut st = SchemaState::default();
        st.tables = vec![
            tree_table(Some("public"), "users", false),
            tree_table(Some("audit"), "events", false),
        ];
        // 未展开:只有两个 schema 行(BTreeMap 序 audit < public)
        let rows = tree_rows(&st, DriverKind::Postgres);
        assert_eq!(rows.len(), 2);
        assert!(matches!(rows[0].kind, SchemaRowKind::Schema("audit")));
        assert!(matches!(rows[1].kind, SchemaRowKind::Schema("public")));
        assert!(!rows[1].expanded);

        // 展开 public:表行挂在 depth 1
        st.expanded_schemas.insert("public".into());
        let rows = tree_rows(&st, DriverKind::Postgres);
        assert_eq!(rows.len(), 3);
        assert!(matches!(rows[2].kind, SchemaRowKind::Table(t) if t.name == "users"));
        assert_eq!(rows[2].depth, 1);
    }

    #[test]
    fn tree_rows_emits_loading_and_failed_placeholders() {
        let mut st = SchemaState::default();
        st.tables = vec![tree_table(None, "a", false), tree_table(None, "b", false)];
        st.expanded_tables.insert((None, "a".into()));
        st.expanded_tables.insert((None, "b".into()));
        st.columns.insert((None, "a".into()), ColumnLoad::Loading);
        st.columns
            .insert((None, "b".into()), ColumnLoad::Failed("nope".into()));
        let rows = tree_rows(&st, DriverKind::MySQL);
        assert!(
            rows.iter()
                .any(|r| matches!(r.kind, SchemaRowKind::ColumnsLoading) && r.depth == 1)
        );
        assert!(
            rows.iter()
                .any(|r| matches!(r.kind, SchemaRowKind::ColumnsFailed(e) if e == "nope"))
        );
    }

    fn update_with(ws: &mut WorkspaceState, msg: Message) {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let mut app_state = AppState::default();
        let dir = tempfile::tempdir().unwrap();
        rt.block_on(async {
            update(ws, &mut app_state, msg, 42, dir.path(), rt.handle(), |_| ());
        });
    }

    /// Postgres 源 + 不可达端口:BrowseSchema/ToggleTable 触发的后台任务
    /// 要么不执行(单测 runtime),要么秒拒连,且不会有 SQLite 建文件副作用。
    fn pg_source(id: &str) -> DataSource {
        DataSource {
            id: id.into(),
            name: format!("test-{id}"),
            driver: DriverKind::Postgres,
            host: Some("127.0.0.1".into()),
            port: Some(1),
            database: Some("mydb".into()),
            username: Some("user".into()),
        }
    }

    fn seeded_ws(source: DataSource) -> WorkspaceState {
        let mut ws = WorkspaceState::default();
        ws.browsing = Some(source.id.clone());
        ws.sources.push(source.clone());
        ws.schemas.insert(source.id.clone(), SchemaState::default());
        ws
    }

    #[test]
    fn browse_schema_first_time_starts_loading() {
        let src = pg_source("s1");
        let mut ws = WorkspaceState::default();
        ws.sources.push(src.clone());
        update_with(&mut ws, Message::BrowseSchema("s1".into()));
        assert_eq!(ws.browsing.as_deref(), Some("s1"));
        let st = ws.schema_state("s1").unwrap_or_else(|| panic!());
        assert!(st.loading_tables());
        assert!(st.tables().is_empty());
        assert!(st.tables_error().is_none());
    }

    #[test]
    fn browse_schema_second_time_uses_cache() {
        let mut ws = seeded_ws(pg_source("s1"));
        ws.schemas.get_mut("s1").unwrap().tables = vec![tree_table(Some("public"), "users", false)];
        // 模拟已加载完成
        update_with(&mut ws, Message::SchemaBack);
        update_with(&mut ws, Message::BrowseSchema("s1".into()));
        let st = ws.schema_state("s1").unwrap();
        assert!(!st.loading_tables()); // 有缓存,不重拉
        assert_eq!(st.tables().len(), 1);
    }

    #[test]
    fn schema_back_clears_browsing_keeps_cache() {
        let mut ws = seeded_ws(pg_source("s1"));
        ws.schemas.get_mut("s1").unwrap().tables = vec![tree_table(Some("public"), "users", false)];
        update_with(&mut ws, Message::SchemaBack);
        assert_eq!(ws.browsing, None);
        assert_eq!(ws.schema_state("s1").unwrap().tables().len(), 1);
    }

    #[test]
    fn schema_refresh_requires_browsing_match() {
        let mut ws = seeded_ws(pg_source("s1"));
        update_with(&mut ws, Message::SchemaBack); // browsing = None
        update_with(&mut ws, Message::SchemaRefresh("s1".into()));
        assert!(!ws.schema_state("s1").unwrap().loading_tables()); // 旧按钮防线
    }

    #[test]
    fn toggle_schema_flips_expansion() {
        let mut ws = seeded_ws(pg_source("s1"));
        let st = ws.schemas.get_mut("s1").unwrap();
        st.tables = vec![tree_table(Some("public"), "users", false)];
        update_with(&mut ws, Message::ToggleSchema("public".into()));
        assert!(
            tree_rows(ws.schema_state("s1").unwrap(), DriverKind::Postgres)
                .iter()
                .any(|r| matches!(r.kind, SchemaRowKind::Table(_)))
        );
        update_with(&mut ws, Message::ToggleSchema("public".into()));
        assert!(
            !tree_rows(ws.schema_state("s1").unwrap(), DriverKind::Postgres)
                .iter()
                .any(|r| matches!(r.kind, SchemaRowKind::Table(_)))
        );
    }

    #[test]
    fn toggle_table_expands_to_loading_then_loaded() {
        let mut ws = seeded_ws(pg_source("s1"));
        ws.schemas.get_mut("s1").unwrap().tables = vec![tree_table(Some("public"), "users", false)];
        // 展开 schema 节点,表行/列行才会进 tree_rows(Postgres 多一层)
        update_with(&mut ws, Message::ToggleSchema("public".into()));
        // 首次展开 → Loading(spawn 的任务不会在本测试 runtime 里跑完)
        update_with(
            &mut ws,
            Message::ToggleTable {
                source_id: "s1".into(),
                schema: Some("public".into()),
                table: "users".into(),
            },
        );
        let key = (Some("public".to_string()), "users".to_string());
        assert!(matches!(
            ws.schema_state("s1").unwrap().column_load(&key),
            Some(ColumnLoad::Loading)
        ));
        // 结果落地 → Loaded,树里出现列行
        update_with(
            &mut ws,
            Message::ColumnsLoaded {
                project_id: 42,
                source_id: "s1".into(),
                schema: Some("public".into()),
                table: "users".into(),
                result: Ok(vec![ColumnInfo {
                    name: "id".into(),
                    type_name: "integer".into(),
                    nullable: false,
                }]),
            },
        );
        assert!(
            tree_rows(ws.schema_state("s1").unwrap(), DriverKind::Postgres)
                .iter()
                .any(|r| matches!(r.kind, SchemaRowKind::Column(c) if c.name == "id"))
        );
        // 过期结果(已不是 Loading)→ 丢弃,仍是旧 Loaded
        update_with(
            &mut ws,
            Message::ColumnsLoaded {
                project_id: 42,
                source_id: "s1".into(),
                schema: Some("public".into()),
                table: "users".into(),
                result: Ok(vec![]),
            },
        );
        assert!(matches!(
            ws.schema_state("s1").unwrap().column_load(&key),
            Some(ColumnLoad::Loaded(cols)) if cols.len() == 1
        ));
    }

    #[test]
    fn failed_columns_retry_by_recollapse_expand() {
        let mut ws = seeded_ws(pg_source("s1"));
        ws.schemas.get_mut("s1").unwrap().tables = vec![tree_table(Some("public"), "users", false)];
        update_with(&mut ws, Message::ToggleSchema("public".into()));
        let key = (Some("public".to_string()), "users".to_string());
        update_with(
            &mut ws,
            Message::ToggleTable {
                source_id: "s1".into(),
                schema: key.0.clone(),
                table: key.1.clone(),
            },
        );
        update_with(
            &mut ws,
            Message::ColumnsLoaded {
                project_id: 42,
                source_id: "s1".into(),
                schema: key.0.clone(),
                table: key.1.clone(),
                result: Err("boom".into()),
            },
        );
        assert!(
            tree_rows(ws.schema_state("s1").unwrap(), DriverKind::Postgres)
                .iter()
                .any(|r| matches!(r.kind, SchemaRowKind::ColumnsFailed("boom")))
        );
        // 收起再展开 → 回到 Loading(重试语义)
        update_with(
            &mut ws,
            Message::ToggleTable {
                source_id: "s1".into(),
                schema: key.0.clone(),
                table: key.1.clone(),
            },
        );
        update_with(
            &mut ws,
            Message::ToggleTable {
                source_id: "s1".into(),
                schema: key.0.clone(),
                table: key.1.clone(),
            },
        );
        assert!(matches!(
            ws.schema_state("s1").unwrap().column_load(&key),
            Some(ColumnLoad::Loading)
        ));
    }

    #[test]
    fn tables_loaded_ok_prunes_autoexpands_and_reloads_expanded() {
        let mut ws = seeded_ws(pg_source("s1"));
        {
            let st = ws.schemas.get_mut("s1").unwrap();
            st.loading_tables = true;
            st.tables = vec![
                tree_table(Some("public"), "gone", false),
                tree_table(Some("public"), "users", false),
            ];
            st.expanded_tables
                .insert((Some("public".into()), "users".into()));
            st.columns.insert(
                (Some("public".into()), "users".into()),
                ColumnLoad::Failed("旧错误".into()),
            );
            st.expanded_tables
                .insert((Some("public".into()), "gone".into()));
        }
        update_with(
            &mut ws,
            Message::TablesLoaded(
                42,
                "s1".into(),
                Ok(vec![tree_table(Some("public"), "users", false)]),
            ),
        );
        let st = ws.schema_state("s1").unwrap();
        assert!(!st.loading_tables());
        assert_eq!(st.tables().len(), 1);
        // 对账一:gone 的展开项被清
        let rows = tree_rows(st, DriverKind::Postgres);
        assert!(
            !rows
                .iter()
                .any(|r| matches!(r.kind, SchemaRowKind::Table(t) if t.name == "gone"))
        );
        // 对账二:单 schema public 自动展开(users 直接可见)
        assert!(
            rows.iter()
                .any(|r| matches!(r.kind, SchemaRowKind::Table(t) if t.name == "users"))
        );
        // 对账三:仍展开的 users 重新拉列(Failed → Loading)
        assert!(matches!(
            st.column_load(&(Some("public".into()), "users".into())),
            Some(ColumnLoad::Loading)
        ));
    }

    #[test]
    fn tables_loaded_stale_or_err() {
        // 过期:loading_tables=false → 丢弃
        let mut ws = seeded_ws(pg_source("s1"));
        ws.schemas.get_mut("s1").unwrap().tables = vec![tree_table(Some("public"), "keep", false)];
        update_with(
            &mut ws,
            Message::TablesLoaded(
                42,
                "s1".into(),
                Ok(vec![tree_table(Some("public"), "new", false)]),
            ),
        );
        assert_eq!(ws.schema_state("s1").unwrap().tables()[0].name, "keep");

        // 错误:记 tables_error,旧快照保留
        let mut ws = seeded_ws(pg_source("s2"));
        let st = ws.schemas.get_mut("s2").unwrap();
        st.loading_tables = true;
        st.tables = vec![tree_table(Some("public"), "old", false)];
        update_with(
            &mut ws,
            Message::TablesLoaded(42, "s2".into(), Err("net down".into())),
        );
        let st = ws.schema_state("s2").unwrap();
        assert_eq!(st.tables_error(), Some("net down"));
        assert_eq!(st.tables()[0].name, "old"); // 旧快照不闪空
    }

    #[test]
    fn delete_source_and_draft_save_clear_schema_state() {
        // DeleteSource:正在浏览的源被删 → schemas 条目与 browsing 一并清掉
        let mut ws = seeded_ws(pg_source("s1"));
        update_with(&mut ws, Message::DeleteSource("s1".into()));
        assert!(ws.schema_state("s1").is_none());
        assert_eq!(ws.browsing, None);

        // DraftSave 编辑已有源 → 同样清理(连接信息可能变,旧快照不作数)
        let mut ws = seeded_ws(pg_source("s1"));
        ws.editing = Some(DataSourceDraft {
            id: Some("s1".into()),
            name: "renamed".into(),
            driver: DriverKind::Postgres,
            host: "127.0.0.1".into(),
            port: "1".into(),
            database: "mydb".into(),
            username: "user".into(),
            password: String::new(), // 留空 → 跳过 Keychain,不写真实密码
        });
        update_with(&mut ws, Message::DraftSave);
        assert!(ws.schema_state("s1").is_none());
        assert_eq!(ws.browsing, None);
    }
}
