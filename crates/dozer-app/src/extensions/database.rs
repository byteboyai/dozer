//! 数据库面板 · 阶段 1:驱动管理 + 每项目数据源 CRUD + 连接测试。结构照搬
//! `extensions::todo` 的双状态模式——`AppState` 挂 `App`(全局:哪些驱动
//! 启用),`WorkspaceState` 挂每个 `Workspace`(当前项目的数据源列表)。
//! 密码不进这个文件的任何持久化结构,单独走 macOS Keychain(见
//! `keyring_key`)。schema 树/数据浏览/SQL 执行/MongoDB 集合浏览留后续
//! 阶段,见
//! `docs/superpowers/specs/2026-08-08-database-panel-phase1-design.md`。

use byteui::interaction::icons;
use iced_widget::core::{Border, Element, Length};
use iced_widget::{MouseArea, Scrollable, button, column, container, row, scrollable, text};
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

/// 新增/编辑数据源表单里"数据源类型"select 的一个选项:驱动 + 预先算好的
/// 展示文案(可能带"(已禁用)"后缀,见 `source_form`)。
#[derive(Debug, Clone, PartialEq)]
struct DriverOption {
    driver: DriverKind,
    label: String,
}

impl std::fmt::Display for DriverOption {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.label)
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
    /// 可选连接 URI(Postgres/MySQL,设计文档"可整串粘贴")。持久化时密码会被
    /// 抽取到 Keychain、URI 里不留明文;连接时若缺密码再从 Keychain 补回。
    pub uri: Option<String>,
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
    #[cfg(test)]
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
            // information_schema.tables 列是 sql_identifier/character_data(sqlx Any 驱动
            // 只认 text/varchar),必须 ::text 转成 text 才不会被 Any 拒绝解码。
            "SELECT table_schema::text, table_name::text, table_type::text \
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
            // 同 tables:@Any 不认 sql_identifier/character_data,列首三列 ::text 转 text
            "SELECT column_name::text, data_type::text, is_nullable::text \
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
        // pragma_table_info 表值函数:常规 SELECT,`Any` 驱动下比 PRAGMA 语句稳;
        // notnull 是保留字,须用 "notnull" 引号包住(SQLite 3.x 实库验证)
        DriverKind::Sqlite => {
            "SELECT name, type, \"notnull\" FROM pragma_table_info(?) ORDER BY cid"
        }
        DriverKind::MongoDB => unreachable!("columns_sql 不处理 MongoDB"),
    }
}

async fn load_tables(
    kind: DriverKind,
    url: &str,
    mongo_db: Option<&str>,
) -> Result<Vec<TableRef>, String> {
    if kind == DriverKind::MongoDB {
        let Some(db_name) = mongo_db.filter(|s| !s.is_empty()) else {
            return Err("请在数据源配置里填写数据库名后再浏览集合".to_string());
        };
        return load_mongo_collections(url, db_name).await;
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

/// MongoDB 集合清单,伪装成 `TableRef`(`schema: None, is_view: false`)——
/// 复用阶段 2 现成的树渲染/展开数据结构,不新开 `SchemaRowKind` 变体
/// (设计文档写作时预留了 `SchemaRowKind::Collection` 的可能性,写计划时
/// 发现集合的"形状"和 MySQL/SQLite 的表完全一致,真正的差异只在**点击行为**
/// ——这个差异在 Task 7 的 `schema_tree_row` 里按 `driver` 参数处理,不需要
/// 数据模型层面的新类型)。
async fn load_mongo_collections(url: &str, db_name: &str) -> Result<Vec<TableRef>, String> {
    let work = async {
        let client = mongodb::Client::with_uri_str(url)
            .await
            .map_err(|e| e.to_string())?;
        let names = client
            .database(db_name)
            .list_collection_names()
            .await
            .map_err(|e| e.to_string())?;
        let mut refs: Vec<TableRef> = names
            .into_iter()
            .map(|name| TableRef {
                schema: None,
                name,
                is_view: false,
            })
            .collect();
        refs.sort_by(|a, b| a.name.cmp(&b.name));
        Ok::<_, String>(refs)
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
            // 真实表必然有 ≥1 列;0 行 → 表不存在(Postgres/MySQL 走 information_schema,
            // SQLite 走 pragma_table_info,缺表一律返回空集而非错误)
            if out.is_empty() {
                return Err(format!("表不存在或无可见列:{table}"));
            }
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
    mongo_db: Option<String>,
    emit: impl Fn(Message) + Send + 'static,
) {
    handle.spawn(async move {
        let result = load_tables(kind, &url, mongo_db.as_deref()).await;
        emit(Message::TablesLoaded(project_id, source_id, result));
    });
}

#[allow(clippy::too_many_arguments)] // 计划给定签名:加载器游标 + 结果回传 emit 共 8 参,边界合理
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
    /// 可选连接 URI 输入(Postgres/MySQL)。保存时优先用它,并抽取密码进 Keychain。
    pub uri: String,
}

/// 挂在每个 `Workspace` 上:当前项目配置的数据源列表 + 编辑态 + 每条数据源
/// 的连接测试状态 + schema 树浏览态(阶段 2,纯内存)。
#[derive(Default)]
pub struct WorkspaceState {
    sources: Vec<DataSource>,
    editing: Option<DataSourceDraft>,
    test_status: HashMap<String, TestStatus>,
    /// 数据源树里当前展开(显示 schema/表)的数据源 id 集合——允许多个
    /// 根节点同时展开,不再是单选的"进入/返回"整页切换(设计文档回顾里
    /// 的卡片列表已改成内联树,见 `view`)。
    expanded_sources: HashSet<String>,
    /// 每个数据源 id 一份 schema 树状态(阶段 2,纯内存)。
    schemas: HashMap<String, SchemaState>,
    /// 右侧内容窗格状态(表/集合/查询 tab)。纯内存,不持久化——同
    /// `schemas`(阶段 2),重启后 tab 全部关闭,不留痕迹。
    content: DatabaseContentState,
    /// 新增/编辑表单**任意一个字段**是否持有 iced 真实焦点——main.rs 每帧用
    /// `CaptureFormFocus`/`take_form_focused` 查回来写进这里(同 Files 搜索框
    /// `search_focused` 的既有手法)。`App::database_form_open` 键盘路由用它
    /// 判断要不要放行给标准 iced 管线,不再只看"表单是否打开"(2026-09 用户
    /// 反馈:数据库面板与 Agent 终端分栏同屏时,表单开着但用户点进的是终端
    /// 输入框,旧信号仍卡真导致终端打不进字)。
    form_focused: bool,
}

impl std::fmt::Debug for WorkspaceState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkspaceState")
            .field("sources", &self.sources)
            .field("editing", &self.editing)
            .field("test_status", &self.test_status)
            .field("expanded_sources", &self.expanded_sources)
            .field("schemas", &self.schemas)
            .field("content_tab_count", &self.content.tabs().len())
            .finish()
    }
}

impl WorkspaceState {
    pub fn sources(&self) -> &[DataSource] {
        &self.sources
    }

    pub fn editing(&self) -> Option<&DataSourceDraft> {
        self.editing.as_ref()
    }

    /// 新增/编辑表单任意字段是否持有真实焦点(main.rs 键盘路由用)。
    pub fn form_focused(&self) -> bool {
        self.form_focused
    }

    /// 每帧渲染循环用 `take_form_focused` 查回来的真实焦点态写进这里。
    pub fn set_form_focused(&mut self, focused: bool) {
        self.form_focused = focused;
    }

    pub fn test_status(&self, source_id: &str) -> &TestStatus {
        self.test_status.get(source_id).unwrap_or(&TestStatus::Idle)
    }

    /// 该数据源在树里是否已展开(header 行 chevron 状态 + 右键菜单"刷新"
    /// 是否可用都靠它判断)。
    pub fn is_expanded(&self, source_id: &str) -> bool {
        self.expanded_sources.contains(source_id)
    }

    /// 已展开数据源的 schema 树状态只读视图(`None` = 尚未加载/已被删除,
    /// 调用方按"加载中"渲染即可,不专门清理,同设计文档 §2 过期防线)。
    pub fn schema_state(&self, source_id: &str) -> Option<&SchemaState> {
        self.schemas.get(source_id)
    }

    /// 右侧内容窗格状态只读视图(视图层 Task 7/8 消费)。
    pub fn content(&self) -> &DatabaseContentState {
        &self.content
    }

    /// 右侧内容窗格状态的可变视图。仅供 `app.rs` 拦截 `TabOverflowToggle`/
    /// `TabOverflowDismiss`(这两个消息需要 `App::last_cursor`,不进
    /// `database::update`)时对溢出锚点做切换时使用。
    pub(crate) fn content_mut(&mut self) -> &mut DatabaseContentState {
        &mut self.content
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

/// 内容窗格 tab 栏里某个可悬停部件的身份;配合 `Message::TabHover` 由内核
/// 转发到 `HoverId::DatabaseTabItem/DatabaseTabClose`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatabaseTabHoverTarget {
    Title,
    Close,
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
    /// 表单里的"连接 URI"输入变更(Postgres/MySQL 便捷粘贴)。
    DraftUriChanged(String),
    /// 任意数据源输入框/SQL 编辑器被右键:内核拦截,不进 `update`——转发成
    /// 顶层 `Message::TextInputMenuOpen` 弹出通用输入框右键菜单(见 app.rs)。
    TextInputMenuOpen(crate::app::TextInputTarget),
    /// 内容侧"收起/展开列表列"按钮:内核拦截,不进 `update`——转发成顶层
    /// `Message::TogglePanelListCollapse(PanelKind::Database)`(见 app.rs)。
    ToggleListCollapse,
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
    /// 数据源树 header 行左键点(chevron/名字):展开/收起该源的 schema 树。
    /// 首次展开且从未加载过才自动发起表加载(有缓存/有错误保留现状,
    /// 错误态由右键菜单"刷新"触发)。允许多个源同时展开。
    ToggleSourceExpanded(String),
    /// 数据源树 header 行右键:内核拦截,不进 `update`——转发成顶层
    /// `App::database_source_context_menu` 弹出测试连接/编辑/删除/刷新
    /// 菜单(见 app.rs)。
    SourceContextMenu(String),
    /// 右键菜单"刷新":重拉表列表(旧快照保留不闪空,成功后对账)。处理前
    /// 先核对该源当前确实展开着,防菜单残留动作作用到已收起的源。
    SchemaRefresh(String),
    /// 任意顶部 `HoverId` 的悬停进入/离开(内容侧收起按钮等)。内核拦截转发
    /// 给顶层 `App::set_hover`,本面板 `update` 保 no-op 分支维持 match 穷尽。
    Hover(crate::app::HoverId, bool),
    /// 内容窗格 tab 栏某个 tab 的悬停进入/离开;纯转发动机,`update()` 里
    /// 保 no-op 分支维持 match 穷尽,真正接线在 `app.rs` 的特化臂。
    TabHover(DatabaseTabHoverTarget, usize, bool),
    /// Postgres schema 节点展开/收起(纯同步,不触发加载)。带 `source_id`——
    /// 允许多个源同时展开后,不能再靠单一"当前浏览源"隐式定位。
    ToggleSchema(String, String),
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

    // ---- 右侧内容窗格:表/集合/查询 tab 的开关 ----
    /// schema 树点一张表/视图的行内文字 → 开(或聚焦已开的)浏览 tab。
    OpenTableTab {
        source_id: String,
        schema: Option<String>,
        table: String,
    },
    /// schema 树点一个 MongoDB 集合 → 开(或聚焦已开的)浏览 tab。
    OpenCollectionTab {
        source_id: String,
        name: String,
    },
    /// schema 树头部"+ 新查询" → 永远新开一个查询 tab。
    OpenQueryTab(String),
    /// tab 栏点某个 tab → 切换 active(索引)。
    SelectTab(usize),
    /// tab 栏点最前面那个固定的"空白"占位 tab(不对应 `tabs` 里任何一条
    /// 记录,`content.active_idx() == None` 即代表它处于选中态,参考
    /// `extensions/ssh.rs::Message::SelectBlankTab` 同款设计)。
    SelectBlankTab,
    /// tab 栏点 × → 关闭(索引)。
    CloseTab(usize),
    /// tab 栏溢出下拉开关,语义同顶层 `Message::TermTabOverflowToggle`。由
    /// `App::update` 拦截处理(需要 `App::last_cursor`),不进 `database::update`。
    TabOverflowToggle,
    /// tab 栏溢出下拉:点击外部关闭。同样由 `App::update` 拦截。
    TabOverflowDismiss,

    // ---- 浏览页(WHERE/ORDER BY/分页) ----
    BrowseWhereChanged(usize, String),
    BrowseOrderByChanged(usize, String),
    BrowsePageSizeChanged(usize, u32),
    BrowsePrev(usize),
    BrowseNext(usize),
    /// 显式"运行"(WHERE 框回车/翻页/改页大小/改排序 统一走这个,见 Task 7)。
    BrowseRun(usize),
    /// 异步结果,带 `project_id` + tab 稳定 id(路由口径同
    /// `TablesLoaded`/`ColumnsLoaded`)。
    /// 第三个字段是发起这轮请求时 `BrowseState::begin_run()` 返回的
    /// `run_seq`——落地前必须核对它仍是当前值(`BrowseState::is_current_run`),
    /// 只查 `loading` 布尔标志分辨不出"哪一轮"结果,连续两次改 WHERE 会让
    /// 旧结果覆盖新结果(设计文档"架构与数据流 §7"的过期防线,数字比对是
    /// 必须的,不能简化成布尔)。
    BrowseResult(i64, usize, u64, Result<BrowsePage, String>),

    // ---- SQL 查询控制台 ----
    /// 编辑器动作(`text_editor::Action`),`update()` 只管
    /// `content.sql.perform(action)`,同 `todo.rs::AddEdit` 的既有用法。
    QueryTextAction(usize, iced_widget::text_editor::Action),
    QueryRun(usize),
    /// 第三个字段同 `BrowseResult`,是 `QueryState::begin_run()` 的 `run_seq`。
    QueryResult(i64, usize, u64, Result<QueryOutcome, String>),
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
        // 这两个溢出开关由 `app.rs` 主级 `update` 拦截(需要 `App::last_cursor`),
        // 正常不会走到这个子级 `database::update`——保留空 arm 只为满足穷尽。
        Message::TabOverflowToggle | Message::TabOverflowDismiss => {}
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
                    uri: src.uri.clone().unwrap_or_default(),
                });
            }
        }
        Message::DraftNameChanged(v) => set_draft(ws_state, |d| d.name = v),
        Message::TextInputMenuOpen(_) => {
            unreachable!("由内核拦截处理,见 database::Message::TextInputMenuOpen 文档")
        }
        Message::ToggleListCollapse => {
            unreachable!("由内核拦截处理,见 database::Message::ToggleListCollapse 文档")
        }
        Message::Hover(_, _) => {
            unreachable!("由内核拦截处理,见 database::Message::Hover 文档")
        }
        Message::SourceContextMenu(_) => {
            unreachable!("由内核拦截处理,见 database::Message::SourceContextMenu 文档")
        }
        Message::DraftDriverChanged(v) => set_draft(ws_state, |d| d.driver = v),
        Message::DraftHostChanged(v) => set_draft(ws_state, |d| d.host = v),
        Message::DraftPortChanged(v) => set_draft(ws_state, |d| d.port = v),
        Message::DraftDatabaseChanged(v) => set_draft(ws_state, |d| d.database = v),
        Message::DraftUsernameChanged(v) => set_draft(ws_state, |d| d.username = v),
        Message::DraftPasswordChanged(v) => set_draft(ws_state, |d| d.password = v),
        Message::DraftUriChanged(v) => set_draft(ws_state, |d| d.uri = v),
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

            // 连接 URI(可选):存在则以此为准填字段,并从 URI 抽出密码进 Keychain。
            let mut uri: Option<String> = None;
            let mut uri_pw: Option<String> = None;
            let mut host = non_empty(&draft.host);
            let mut port = draft.port.parse().ok();
            let mut database = non_empty(&draft.database);
            let mut username = non_empty(&draft.username);
            // URI 无效(非 postgres/mysql 或解析失败):忽略 URI,退化为字段式。
            if !draft.uri.trim().is_empty()
                && let Some(p) = parse_connection_uri(draft.uri.trim())
            {
                uri = Some(p.uri);
                uri_pw = p.password;
                host = p.host;
                port = p.port;
                database = p.database;
                username = p.username;
            }
            let source = DataSource {
                id: id.clone(),
                name: draft.name.clone(),
                driver: draft.driver,
                host,
                port,
                database,
                username,
                uri,
            };
            if let Some(pos) = ws_state.sources.iter().position(|s| s.id == id) {
                ws_state.sources[pos] = source;
            } else {
                ws_state.sources.push(source);
            }
            // 密码:URI 内嵌优先,其次表单 password 字段。
            let pw_to_save = uri_pw.or_else(|| {
                if draft.password.is_empty() {
                    None
                } else {
                    Some(draft.password.clone())
                }
            });
            if let Some(p) = pw_to_save
                && let Ok(entry) = keyring_entry(project_id, &id)
            {
                let _ = entry.set_password(&p);
            }
            // 编辑已有源:连接信息可能变了,旧结构快照不作数(设计文档 §2)。
            if draft.id.is_some() {
                ws_state.schemas.remove(&id);
                ws_state.content.close_by_source(&id);
                ws_state.expanded_sources.remove(&id);
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
            ws_state.content.close_by_source(&id);
            ws_state.expanded_sources.remove(&id);
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
        Message::ToggleSourceExpanded(id) => {
            if !ws_state.expanded_sources.remove(&id) {
                ws_state.expanded_sources.insert(id.clone());
            } else {
                return; // 收起:保留缓存,不取消飞行中的加载
            }
            let Some(source) = ws_state.sources.iter().find(|s| s.id == id).cloned() else {
                return;
            };
            let st = ws_state.schemas.entry(id.clone()).or_default();
            // 从未加载过才拉:有缓存 → 直接显示;有错误 → 等用户点右键菜单"刷新"
            let need_load = st.tables.is_empty() && !st.loading_tables && st.tables_error.is_none();
            if !need_load {
                return;
            }
            st.loading_tables = true;
            let password = keyring_entry(project_id, &id)
                .ok()
                .and_then(|e| e.get_password().ok());
            let url = if source.driver == DriverKind::MongoDB {
                build_mongo_url(&source, password.as_deref())
            } else {
                build_sql_url(&source, password.as_deref())
            };
            spawn_tables_load(
                handle,
                project_id,
                id,
                source.driver,
                url,
                source.database.clone(),
                emit,
            );
        }
        Message::SchemaRefresh(source_id) => {
            if !ws_state.expanded_sources.contains(&source_id) {
                return; // 菜单残留防线:源已被收起
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
            let url = if source.driver == DriverKind::MongoDB {
                build_mongo_url(&source, password.as_deref())
            } else {
                build_sql_url(&source, password.as_deref())
            };
            spawn_tables_load(
                handle,
                project_id,
                source_id,
                source.driver,
                url,
                source.database.clone(),
                emit,
            );
        }
        Message::ToggleSchema(source_id, name) => {
            let Some(st) = ws_state.schemas.get_mut(&source_id) else {
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
            if !ws_state.expanded_sources.contains(&source_id) {
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
                        if distinct.len() == 1
                            && let Some(only) = distinct.into_iter().next()
                        {
                            st.expanded_schemas.insert(only.to_string());
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
        Message::OpenTableTab {
            source_id,
            schema,
            table,
        } => {
            ws_state.content.open_table(source_id, schema, table);
        }
        Message::OpenCollectionTab { source_id, name } => {
            ws_state.content.open_collection(source_id, name);
        }
        Message::OpenQueryTab(source_id) => {
            ws_state.content.open_query(source_id);
        }
        Message::SelectTab(idx) => {
            ws_state.content.select(idx);
            reveal_tab_widths(ws_state, idx + 1);
        }
        Message::SelectBlankTab => {
            ws_state.content.select_blank();
            reveal_tab_widths(ws_state, 0);
        }
        Message::CloseTab(idx) => ws_state.content.close(idx),
        Message::BrowseWhereChanged(tab_id, v) => {
            if let Some(TabContent::Browse(b)) = ws_state.content.content_mut(tab_id) {
                b.where_clause = v;
            }
        }
        Message::BrowseOrderByChanged(tab_id, v) => {
            if let Some(TabContent::Browse(b)) = ws_state.content.content_mut(tab_id) {
                b.order_by = v;
            }
        }
        Message::BrowsePageSizeChanged(tab_id, size) => {
            if let Some(TabContent::Browse(b)) = ws_state.content.content_mut(tab_id) {
                b.page_size = size;
                b.page = 0; // 换页大小回第一页,避免 offset 算错位置
            }
        }
        Message::BrowsePrev(tab_id) => {
            if let Some(TabContent::Browse(b)) = ws_state.content.content_mut(tab_id)
                && b.page > 0
            {
                b.page -= 1;
            }
            dispatch_browse_run(
                ws_state, app_state, tab_id, project_id, repo_path, handle, emit,
            );
        }
        Message::BrowseNext(tab_id) => {
            if let Some(TabContent::Browse(b)) = ws_state.content.content_mut(tab_id)
                && b.has_more
            {
                b.page += 1;
            }
            dispatch_browse_run(
                ws_state, app_state, tab_id, project_id, repo_path, handle, emit,
            );
        }
        Message::BrowseRun(tab_id) => {
            dispatch_browse_run(
                ws_state, app_state, tab_id, project_id, repo_path, handle, emit,
            );
        }
        Message::BrowseResult(_project_id, tab_id, seq, result) => {
            let Some(TabContent::Browse(b)) = ws_state.content.content_mut(tab_id) else {
                return; // tab 已关闭
            };
            if !b.is_current_run(seq) {
                return; // 过期结果:tab 内又发起了更新的一轮请求,这轮作废
            }
            apply_browse_result(b, result);
        }
        Message::QueryTextAction(tab_id, action) => {
            if let Some(TabContent::Query(q)) = ws_state.content.content_mut(tab_id) {
                q.sql.perform(action);
            }
        }
        Message::QueryRun(tab_id) => {
            let Some(kind) = ws_state
                .content
                .tabs()
                .iter()
                .find(|t| t.id == tab_id)
                .and_then(|t| match &t.kind {
                    DatabaseTabKind::Query { source_id, .. } => driver_of(ws_state, source_id),
                    _ => None,
                })
            else {
                return;
            };
            let Some(source) = ws_state
                .content
                .tabs()
                .iter()
                .find(|t| t.id == tab_id)
                .and_then(|t| match &t.kind {
                    DatabaseTabKind::Query { source_id, .. } => ws_state
                        .sources
                        .iter()
                        .find(|s| &s.id == source_id)
                        .cloned(),
                    _ => None,
                })
            else {
                return;
            };
            let sql = match ws_state.content.content_mut(tab_id) {
                Some(TabContent::Query(q)) => {
                    let seq = q.begin_run();
                    let text = q.sql.text();
                    (seq, text)
                }
                _ => return,
            };
            let (seq, sql_text) = sql;
            let password = keyring_entry(project_id, &source.id)
                .ok()
                .and_then(|e| e.get_password().ok());
            let url = build_sql_url(&source, password.as_deref());
            let emit = emit.clone();
            handle.spawn(async move {
                let result = run_query(kind, &url, &sql_text).await;
                emit(Message::QueryResult(project_id, tab_id, seq, result));
            });
        }
        Message::QueryResult(_project_id, tab_id, seq, result) => {
            let Some(TabContent::Query(q)) = ws_state.content.content_mut(tab_id) else {
                return;
            };
            if !q.is_current_run(seq) {
                return; // 过期结果:同一 tab 内已经又执行了一次更新的查询
            }
            apply_query_result(q, result);
        }
        Message::TabHover(..) => {
            // 内容窗格 tab 悬停:由内核 `App::update` 的特化臂转发到
            // `HoverId::DatabaseTabItem/DatabaseTabClose`,吃不到这里。
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn dispatch_browse_run(
    ws_state: &mut WorkspaceState,
    _app_state: &AppState,
    tab_id: usize,
    project_id: i64,
    repo_path: &Path,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + Clone + 'static,
) {
    let Some(tab) = ws_state.content.tabs().iter().find(|t| t.id == tab_id) else {
        return;
    };
    let (source_id, mongo_name, table_schema, table_name) = match &tab.kind {
        DatabaseTabKind::Table {
            source_id,
            schema,
            table,
        } => (source_id.clone(), None, schema.clone(), Some(table.clone())),
        DatabaseTabKind::Collection { source_id, name } => {
            (source_id.clone(), Some(name.clone()), None, None)
        }
        DatabaseTabKind::Query { .. } => return, // 查询 tab 不走这条路径
    };
    let Some(source) = ws_state.sources.iter().find(|s| s.id == source_id).cloned() else {
        return;
    };
    let Some(TabContent::Browse(b)) = ws_state.content.content_mut(tab_id) else {
        return;
    };
    let seq = b.begin_run();
    b.error = None;
    let (where_clause, order_by, page, page_size) = (
        b.where_clause.clone(),
        b.order_by.clone(),
        b.page,
        b.page_size,
    );
    let password = keyring_entry(project_id, &source.id)
        .ok()
        .and_then(|e| e.get_password().ok());
    let _ = repo_path; // 本函数不需要仓库路径,保留参数只为和 update() 里其它 dispatch 签名一致
    let emit2 = emit.clone();
    if source.driver == DriverKind::MongoDB {
        let url = build_mongo_url(&source, password.as_deref());
        let db_name = source.database.clone().unwrap_or_default();
        let name = mongo_name.unwrap_or_default();
        handle.spawn(async move {
            let result = browse_collection(&url, &db_name, &name, page, page_size).await;
            emit2(Message::BrowseResult(project_id, tab_id, seq, result));
        });
    } else {
        let url = build_sql_url(&source, password.as_deref());
        let kind = source.driver;
        let table = table_name.unwrap_or_default();
        handle.spawn(async move {
            let result = browse_table(BrowseTableParams {
                kind,
                url: &url,
                schema: table_schema.as_deref(),
                table: &table,
                where_clause: &where_clause,
                order_by: &order_by,
                page,
                page_size,
            })
            .await;
            emit2(Message::BrowseResult(project_id, tab_id, seq, result));
        });
    }
}

/// `BrowseResult` 落地:过期结果的核对(`run_seq` 对不上)在 `update()` 的
/// `Message::BrowseResult` 分支里做完才会调到这个函数,这里只管把结果写
/// 进状态。模块私有——`app.rs`(Task 6)不直接调它,而是把消息重新塞回
/// `database::update()`,由 `update()` 内部调这个函数。
fn apply_browse_result(b: &mut BrowseState, result: Result<BrowsePage, String>) {
    b.loading = false;
    match result {
        Ok(page) => {
            b.error = None;
            b.result = Some(page.result);
            b.has_more = page.has_more;
        }
        Err(e) => b.error = Some(e),
    }
}

/// `QueryResult` 落地,同上——模块私有,理由同 `apply_browse_result`。
fn apply_query_result(q: &mut QueryState, result: Result<QueryOutcome, String>) {
    q.running = false;
    match result {
        Ok(outcome) => {
            q.error = None;
            q.result = Some(outcome);
        }
        Err(e) => q.error = Some(e),
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
    // 连接 URI 优先(用户整串粘贴);密码留 Keychain,用时补回。
    if let Some(u) = source.uri.as_deref().filter(|u| !u.trim().is_empty()) {
        let u = u.trim();
        if u.contains("://") {
            return inject_password_into_uri(u, password);
        }
        return u.to_string();
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

/// 解析用户粘贴的连接 URI(仅 postgres/postgresql/mysql)。返回:脱敏后的
/// 一次 `parse_connection_uri` 的解析结果。
struct ParsedUri {
    /// 脱敏后的 URI(密码已抽走,保留用户名/host/port/库名)。
    uri: String,
    host: Option<String>,
    port: Option<u16>,
    database: Option<String>,
    username: Option<String>,
    /// 从 URI 抽出的密码,交调用方写入 Keychain。
    password: Option<String>,
}

/// 解析用户粘贴的连接 URI(仅 postgres/postgresql/mysql)。脱敏 = 密码抽走
/// (交调用方进 Keychain),URI 里不再留明文,避免写进 `database.json`。
/// 解析失败或 scheme 不受支持返回 None。
fn parse_connection_uri(uri: &str) -> Option<ParsedUri> {
    let u = url::Url::parse(uri).ok()?;
    let scheme = u.scheme();
    if scheme != "postgres" && scheme != "postgresql" && scheme != "mysql" {
        return None;
    }
    let host = u.host_str().map(|h| h.to_string());
    let host = host.filter(|h| !h.is_empty())?;
    let port = u.port();
    let database = u
        .path_segments()
        .and_then(|mut s| s.next())
        .map(|s| s.to_string());
    let username = if u.username().is_empty() {
        None
    } else {
        Some(u.username().to_string())
    };
    let password = u.password().map(|p| p.to_string());
    let mut redacted = u;
    let _ = redacted.set_password(None);
    Some(ParsedUri {
        uri: redacted.to_string(),
        host: Some(host),
        port,
        database,
        username,
        password,
    })
}

/// 连接时把 Keychain 密码补回 URI(URI 若已带密码则保留原样)。
fn inject_password_into_uri(uri: &str, password: Option<&str>) -> String {
    let Ok(mut u) = url::Url::parse(uri) else {
        return uri.to_string();
    };
    if u.password().is_none()
        && let Some(p) = password
        && !p.is_empty()
    {
        let _ = u.set_password(Some(p));
    }
    u.to_string()
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
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    // 首行是分区标题(纯文字、不套按钮),随后每行一个 checkbox 前置位的菜单
    // 项(启用的打 ✓)。单项/外壳统一走 `crate::menu`,表面即文件树右键菜单。
    let mut items: Vec<Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>> = vec![
        text("已启用的驱动")
            .size(byteui::theme::font::caption())
            .color(byteui::theme::color::current().dim)
            .into(),
    ];
    for driver in DriverKind::ALL {
        let enabled = app_state.is_enabled(driver);
        let checkbox: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
            text(if enabled { "✓" } else { " " })
                .size(byteui::theme::font::body())
                .into();
        items.push(crate::menu::item_row_fill(
            Some(checkbox),
            driver.label(),
            byteui::theme::color::current().body,
            Some(Message::ToggleDriver(driver)),
        ));
    }
    crate::menu::shell_frosted(items, iced_widget::core::Length::Fixed(220.0))
}

/// 数据源根节点图标:关系型驱动统一用 `Database`(圆柱),MongoDB 用
/// `Leaf`。现有图标库没有贴切的各厂商品牌图标,退而求其次按"关系型/
/// 文档型"两类区分形状——颜色仍走主题染色(dim),不引入固定品牌色,
/// 跟本仓库其它 icon 的处理口径一致。
fn driver_icon(driver: DriverKind) -> icons::IconKind {
    match driver {
        DriverKind::MongoDB => icons::IconKind::Leaf,
        DriverKind::Postgres | DriverKind::MySQL | DriverKind::Sqlite => icons::IconKind::Database,
    }
}

/// 树行左侧缩进宽度,对齐 header 行的 chevron + 图标列(子行没有自己的
/// chevron 时用这个占位,行内文字信息用这个当左边距)。
fn source_tree_indent() -> Length {
    Length::Fixed(byteui::theme::icon_size::chevron() + byteui::theme::icon_size::tree_row_gap())
}

fn indented_line<'a>(
    text_str: impl Into<String>,
    color: iced_widget::core::Color,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    row![
        iced_widget::space::Space::new().width(source_tree_indent()),
        text(text_str.into())
            .size(byteui::theme::font::caption_sm())
            .color(color),
    ]
    .into()
}

/// 数据源树的一个根节点:header 行(chevron + 驱动图标 + 名字,左键展开/
/// 收起、右键弹测试连接/编辑/删除/刷新菜单,见 `app.rs` 的
/// `database_source_context_menu_popup`)+ 展开时的 schema/表子树。子树
/// 结构复用阶段 2 现成的 `SchemaState`/`tree_rows`/`schema_tree_row`——此前
/// 是切到独立整页 `schema_tree_view`,这次改成内联挂在同一个可滚动列表里。
fn source_tree_node<'a>(
    source: &'a DataSource,
    status: &'a TestStatus,
    expanded: bool,
    st: Option<&'a SchemaState>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let chevron = if expanded {
        icons::IconKind::ChevronDown
    } else {
        icons::IconKind::ChevronRight
    };
    let header = MouseArea::new(
        button(
            row![
                icons::view(
                    chevron,
                    byteui::theme::icon_size::chevron(),
                    byteui::theme::color::current().dim
                ),
                icons::view(
                    driver_icon(source.driver),
                    byteui::theme::icon_size::row(),
                    byteui::theme::color::current().dim
                ),
                text(source.name.clone())
                    .size(byteui::theme::font::body())
                    .color(byteui::theme::color::current().cream),
            ]
            .spacing(byteui::theme::icon_size::tree_row_gap())
            .align_y(iced_widget::core::Alignment::Center),
        )
        .on_press(Message::ToggleSourceExpanded(source.id.clone()))
        .width(Length::Fill)
        .style(|_t: &iced_widget::Theme, _s| iced_widget::button::Style {
            background: None,
            ..iced_widget::button::Style::default()
        }),
    )
    .on_right_press(Message::SourceContextMenu(source.id.clone()));

    let mut col = column![header].spacing(2);

    let status_line = match status {
        TestStatus::Idle => None,
        TestStatus::Testing => Some(("测试中…".to_string(), byteui::theme::color::current().dim)),
        TestStatus::Ok => Some((
            "✓ 连接成功".to_string(),
            byteui::theme::color::current().green,
        )),
        TestStatus::Err(e) => Some((format!("✗ {e}"), byteui::theme::color::current().red)),
    };
    if let Some((line, color)) = status_line {
        col = col.push(indented_line(line, color));
    }

    if !expanded {
        return col.into();
    }

    if source.driver != DriverKind::MongoDB {
        col = col.push(row![
            iced_widget::space::Space::new().width(source_tree_indent()),
            button(
                text("+ 新查询")
                    .size(byteui::theme::font::caption_sm())
                    .color(byteui::theme::color::current().dim)
            )
            .on_press(Message::OpenQueryTab(source.id.clone()))
            .style(|_t: &iced_widget::Theme, _s| iced_widget::button::Style {
                background: None,
                ..iced_widget::button::Style::default()
            }),
        ]);
    }

    let dim = byteui::theme::color::current().dim;
    let Some(st) = st else {
        return col.push(indented_line("加载中…", dim)).into();
    };

    // 旧快照在手 + 正在刷新 → 一行"刷新中…"提示,旧树照常(同 git_log 不闪空惯例)
    if st.loading_tables() && !st.tables().is_empty() {
        col = col.push(indented_line("刷新中…", dim));
    }
    if let Some(e) = st.tables_error()
        && !st.tables().is_empty()
    {
        col = col.push(indented_line(
            format!("刷新失败:{e}"),
            byteui::theme::color::current().red,
        ));
    }

    if st.loading_tables() && st.tables().is_empty() {
        col.push(indented_line("加载中…", dim)).into()
    } else if st.tables().is_empty() {
        if let Some(e) = st.tables_error() {
            col.push(indented_line(
                format!("✗ {e}(右键菜单可重试)"),
                byteui::theme::color::current().red,
            ))
            .into()
        } else {
            col.push(indented_line("该库没有表或视图", dim)).into()
        }
    } else {
        for r in tree_rows(st, source.driver) {
            col = col.push(row![
                iced_widget::space::Space::new().width(source_tree_indent()),
                schema_tree_row(&source.id, source.driver, r),
            ]);
        }
        col.into()
    }
}

/// 新增/编辑数据源表单。SQLite 只留"文件路径"一栏,其它驱动列出 host/port/
/// database/username/password。
fn source_form<'a>(
    draft: &'a DataSourceDraft,
    app_state: &'a AppState,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let mut driver_options: Vec<DriverOption> = Vec::new();
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
        driver_options.push(DriverOption { driver, label });
    }
    let selected_driver_option = driver_options
        .iter()
        .find(|o| o.driver == draft.driver)
        .cloned();
    let driver_select = byteui::form::select::view(
        driver_options,
        selected_driver_option,
        |opt: DriverOption| Message::DraftDriverChanged(opt.driver),
    );

    let mut col = column![driver_select].spacing(8);
    col = col.push(wrap_form_input(
        byteui::form::input_text::view_on_bg(
            "名字",
            &draft.name,
            false,
            Some(form_field_id("name")),
            false,
            None,
            false,
            Message::DraftNameChanged,
        ),
        form_field_id("name"),
        false,
    ));
    if draft.driver == DriverKind::Sqlite {
        col = col.push(wrap_form_input(
            byteui::form::input_text::view_on_bg(
                "文件路径",
                &draft.database,
                false,
                Some(form_field_id("sqlite-database")),
                false,
                None,
                false,
                Message::DraftDatabaseChanged,
            ),
            form_field_id("sqlite-database"),
            false,
        ));
    } else {
        col = col.push(wrap_form_input(
            byteui::form::input_text::view_on_bg(
                "连接 URI(可选,填了则忽略下面各项,例如 postgres://user:pw@host:5432/db)",
                &draft.uri,
                false,
                Some(form_field_id("uri")),
                false,
                None,
                false,
                Message::DraftUriChanged,
            ),
            form_field_id("uri"),
            false,
        ));
        col = col.push(wrap_form_input(
            byteui::form::input_text::view_on_bg(
                "host",
                &draft.host,
                false,
                Some(form_field_id("host")),
                false,
                None,
                false,
                Message::DraftHostChanged,
            ),
            form_field_id("host"),
            false,
        ));
        col = col.push(wrap_form_input(
            byteui::form::input_text::view_on_bg(
                "port",
                &draft.port,
                false,
                Some(form_field_id("port")),
                false,
                None,
                false,
                Message::DraftPortChanged,
            ),
            form_field_id("port"),
            false,
        ));
        col = col.push(wrap_form_input(
            byteui::form::input_text::view_on_bg(
                "database",
                &draft.database,
                false,
                Some(form_field_id("database")),
                false,
                None,
                false,
                Message::DraftDatabaseChanged,
            ),
            form_field_id("database"),
            false,
        ));
        col = col.push(wrap_form_input(
            byteui::form::input_text::view_on_bg(
                "username",
                &draft.username,
                false,
                Some(form_field_id("username")),
                false,
                None,
                false,
                Message::DraftUsernameChanged,
            ),
            form_field_id("username"),
            false,
        ));
        col = col.push(wrap_form_input(
            byteui::form::input_text::view_on_bg(
                "password(留空则不修改)",
                &draft.password,
                true,
                Some(form_field_id("password")),
                false,
                None,
                false,
                Message::DraftPasswordChanged,
            ),
            form_field_id("password"),
            true,
        ));
    }
    // 按钮样式照抄 `ssh.rs::host_form` 的 `text_btn`:透明背景 + 1px
    // 描边(描边色=文字色),不再用纯色填充按钮,跟主机表单保持同一产品
    // 语言(设计文档回顾)。
    let text_btn = |label: &'a str, color: iced_widget::core::Color, msg: Message| {
        button(text(label).size(byteui::theme::font::label()).color(color))
            .on_press(msg)
            .padding([6, 12])
            .style(
                move |_t: &iced_widget::Theme, _s| iced_widget::button::Style {
                    background: Some(byteui::theme::color::current().bg.into()),
                    border: iced_widget::core::Border {
                        color,
                        width: 1.0,
                        radius: 4.0.into(),
                    },
                    text_color: color,
                    ..iced_widget::button::Style::default()
                },
            )
    };
    col = col.push(
        row![
            text_btn(
                "保存",
                byteui::theme::color::current().cream,
                Message::DraftSave
            ),
            text_btn(
                "取消",
                byteui::theme::color::current().dim,
                Message::DraftCancel
            ),
        ]
        .spacing(6),
    );

    // 边框/底色统一成原生预览"文件内搜索"风格(`find_field_shell`/`find_rows`
    // 外层组合的既有配色):底色 card、边框普通态 `colors.border`(不再恒描
    // 金)——2026-09-11 需求,数据库/主机新增表单跟文件内搜索输入框对齐。
    container(col)
        .padding(12)
        .width(iced_widget::core::Length::Fill)
        .style(|_t: &iced_widget::Theme| iced_widget::container::Style {
            background: Some(byteui::theme::color::current().card.into()),
            border: iced_widget::core::Border {
                color: byteui::theme::color::current().border,
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
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    // 顶部面板标题固定、中间可滚动、底部「管理驱动/新增数据源」footer-bar
    // 固定在面板最下方——镜像 `ssh.rs::view`/`ssh_footer_bar` 的既有布局
    // (原先两个按钮跟标题挤在同一行,不随内容滚动分区,验收反馈参照主机
    // 面板"添加主机"统一到 footer-bar)。
    let head = container(crate::homespace::home_panel_head(
        icons::IconKind::Database,
        "数据库",
    ))
    .padding(crate::theme::region::project_pane().padding);

    let mut list = column![].spacing(12).padding([0, 20]);

    if app_state.drivers_popup_open() {
        list = list.push(drivers_popup(app_state));
    }

    if ws_state.sources().is_empty() {
        list = list.push(
            text("还没有数据源")
                .size(byteui::theme::font::body())
                .color(byteui::theme::color::current().dim),
        );
    } else {
        for source in ws_state.sources() {
            let expanded = ws_state.is_expanded(&source.id);
            list = list.push(source_tree_node(
                source,
                ws_state.test_status(&source.id),
                expanded,
                ws_state.schema_state(&source.id),
            ));
        }
    }

    // 表单排在数据源树下方,不是上方——跟主机面板 `ssh.rs::view` 的既有
    // 顺序一致(先列表后表单),新增/编辑不会把树往下挤。
    if let Some(draft) = ws_state.editing() {
        list = list.push(source_form(draft, app_state));
    }

    let scroll = Scrollable::new(list)
        .width(Length::Fill)
        .height(Length::Fill)
        .direction(scrollable::Direction::Vertical(
            byteui::interaction::scrollbar::scrollbar(),
        ))
        .style(|_t, _s| byteui::interaction::scrollbar::scrollbar_style());

    let body = column![head, scroll, database_footer_bar()].spacing(0);

    container(body)
        .width(width)
        .height(iced_widget::core::Length::Fill)
        .style(
            move |_t: &iced_widget::Theme| iced_widget::container::Style {
                background: Some(byteui::theme::color::current().bg.into()),
                border: outer,
                ..iced_widget::container::Style::default()
            },
        )
        .into()
}

/// 数据库面板底部 footer-bar:1px `BORDER` 分隔线 + `padding([6, 8])` 容器,
/// 结构照抄 `ssh.rs::ssh_footer_bar`(同一产品语言——"管理驱动"+"新增数据源"
/// 两个按钮固定在面板最下方,不随数据源列表滚动)。
fn database_footer_bar<'a>() -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let btn = |icon: icons::IconKind, label: &'static str, on_press: Message| {
        button(
            row![
                icons::view(
                    icon,
                    byteui::theme::icon_size::row(),
                    byteui::theme::color::current().cream,
                ),
                text(label)
                    .size(byteui::theme::font::label())
                    .color(byteui::theme::color::current().cream),
            ]
            .spacing(6)
            .align_y(iced_widget::core::Alignment::Center),
        )
        .on_press(on_press)
        .padding([4, 8])
        .style(|_t: &iced_widget::Theme, _s| button::Style {
            background: Some(byteui::theme::color::current().bg.into()),
            border: iced_widget::core::Border {
                color: byteui::theme::color::current().border,
                width: 1.0,
                radius: 4.0.into(),
            },
            text_color: byteui::theme::color::current().cream,
            ..button::Style::default()
        })
    };

    let bar = row![
        iced_widget::space::horizontal(),
        btn(
            icons::IconKind::Settings,
            "管理驱动",
            Message::DriversPopupToggle,
        ),
        btn(
            icons::IconKind::SquarePlus,
            "新增数据源",
            Message::AddSourceStart
        ),
    ]
    .spacing(6)
    .align_y(iced_widget::core::Alignment::Center);

    let top_line = container(iced_widget::Space::new())
        .width(iced_widget::core::Length::Fill)
        .height(iced_widget::core::Length::Fixed(1.0))
        .style(|_t: &iced_widget::Theme| iced_widget::container::Style {
            background: Some(byteui::theme::color::current().border.into()),
            ..iced_widget::container::Style::default()
        });

    let pp = crate::theme::region::project_pane().padding;
    container(column![top_line, bar].spacing(4))
        .width(iced_widget::core::Length::Fill)
        .padding(iced_widget::core::Padding {
            top: 6.0,
            right: pp.right,
            bottom: 6.0,
            left: pp.left,
        })
        .style(|_t: &iced_widget::Theme| iced_widget::container::Style {
            background: None,
            ..iced_widget::container::Style::default()
        })
        .into()
}

/// 右侧内容窗格:tab 栏(表/集合/查询)+ 当前激活 tab 的内容。骨架照抄
/// `workspace.rs::preview_pane_for`,**不**带拖拽换位/右键菜单(设计文档
/// 非目标)。
pub fn content_pane<'a>(
    app: &'a crate::app::App,
    ws_state: &'a WorkspaceState,
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let content = ws_state.content();
    // "空白"占位 tab 固定打头,不对应 `content.tabs()` 里任何一条记录,
    // 选中态即 `content.active_idx() == None`——参考 SSH 面板
    // `app.rs::ssh_tab_bar` 同款设计(见 `Message::SelectBlankTab`)。
    // hover key 用 `usize::MAX`,真实 tab 下标不可能到这个值。
    const BLANK_HOVER_KEY: usize = usize::MAX;
    let blank_active = content.active_idx().is_none();
    let mut entries: Vec<(
        f32,
        Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>,
    )> = vec![(
        tab_title_display_width("空白"),
        crate::tab_widget::panel_tab(crate::tab_widget::PanelTabArgs {
            title: "空白".to_string(),
            active: blank_active,
            hover_t: app.hover_progress(crate::app::HoverId::DatabaseTabItem(BLANK_HOVER_KEY)),
            close_hover_t: app
                .hover_progress(crate::app::HoverId::DatabaseTabClose(BLANK_HOVER_KEY)),
            prefix: None,
            suffix: None,
            on_select: Message::SelectBlankTab,
            on_close: Message::SelectBlankTab,
            show_tooltip: app
                .hover_tooltip_ready(crate::app::HoverId::DatabaseTabItem(BLANK_HOVER_KEY)),
            title_hover: move |h| {
                Message::TabHover(DatabaseTabHoverTarget::Title, BLANK_HOVER_KEY, h)
            },
            close_hover: move |h| {
                Message::TabHover(DatabaseTabHoverTarget::Close, BLANK_HOVER_KEY, h)
            },
        }),
    )];
    entries.extend(content.tabs().iter().enumerate().map(|(idx, tab)| {
        let active = Some(idx) == content.active_idx();
        let title_hover_t = app.hover_progress(crate::app::HoverId::DatabaseTabItem(idx));
        let close_hover_t = app.hover_progress(crate::app::HoverId::DatabaseTabClose(idx));
        let title = tab_title(tab, ws_state);
        (
            tab_title_display_width(&title),
            crate::tab_widget::panel_tab(crate::tab_widget::PanelTabArgs {
                title,
                active,
                hover_t: title_hover_t,
                close_hover_t,
                prefix: None,
                suffix: None,
                on_select: Message::SelectTab(idx),
                on_close: Message::CloseTab(idx),
                show_tooltip: app.hover_tooltip_ready(crate::app::HoverId::DatabaseTabItem(idx)),
                title_hover: move |h| Message::TabHover(DatabaseTabHoverTarget::Title, idx, h),
                close_hover: move |h| Message::TabHover(DatabaseTabHoverTarget::Close, idx, h),
            }),
        )
    }));
    let widths: Vec<f32> = entries.iter().map(|(w, _)| *w).collect();
    let window = crate::tab_widget::tab_window(
        &widths,
        4.0,
        byteui::theme::geometry::tab_bar_avail_px(),
        content.tab_scroll_first(),
    );
    let items: Vec<Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>> = entries
        .into_iter()
        .enumerate()
        .filter(|(idx, _)| (window.first..window.visible_end).contains(idx))
        .map(|(_, (_, el))| el)
        .collect();
    let tabs_row = row(items).spacing(4);
    let clipped = container(tabs_row).width(Length::Fill).clip(true);
    // V 只数**真实** tab,不算"空白"占位——下拉本就不列空白(见
    // `tab_overflow_popup`),只剩空白页时 V 本身也不该显示(验收反馈)。
    let db_tab_total = content.tabs().len();
    let overflow_button = crate::tab_widget::tab_overflow_button(
        db_tab_total,
        app.hover_progress(crate::app::HoverId::DatabaseTabOverflow),
        Message::TabOverflowToggle,
        move |hovered| Message::Hover(crate::app::HoverId::DatabaseTabOverflow, hovered),
    );
    // 内容侧"收起/展开列表列"按钮(收起左列 schema 树后仍在此可见以便恢复)。
    // 消息为本地 `Message::ToggleListCollapse`,由内核 `App::update` 拦截。
    let collapse = app.list_collapse_button(
        crate::app::PanelKind::Database,
        app.list_collapsed(crate::app::PanelKind::Database),
        crate::app::HoverId::DatabaseListCollapse,
        "收起列表",
        "展开列表",
        Message::ToggleListCollapse,
        move |hovered| Message::Hover(crate::app::HoverId::DatabaseListCollapse, hovered),
    );
    let mut tab_bar_row = row![]
        .spacing(4)
        .align_y(iced_widget::core::Alignment::Center);
    if let Some(btn) = overflow_button {
        tab_bar_row = tab_bar_row.push(btn);
    }
    let tab_bar = tab_bar_row.push(clipped).push(collapse);

    // tab 栏下方 1px 分割线,同 SSH/预览面板的 `tab_divider()`(此前漏加,
    // 验收反馈 tab 下方少了一根横线)。外层 padding/tab 栏间距改用
    // `terminal_pane` region(同 `app.rs::ssh_terminal_pane` 的既有取值:
    // padding 8、gap 4)——之前硬编码 16/8 是 SSH 面板的两倍,tab 栏看起来
    // 比主机面板厚一圈(验收反馈"右侧 tab 高度太高")。
    let region = crate::theme::region::terminal_pane();
    let mut col = column![tab_bar, crate::app::tab_divider()].spacing(region.gap);

    match content.active_tab() {
        None => {
            col = col.push(
                container(
                    text("在左侧 schema 树点一张表/视图/集合,或点 + 新查询")
                        .size(byteui::theme::font::subtitle())
                        .color(byteui::theme::color::current().dim),
                )
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(iced_widget::core::alignment::Horizontal::Center)
                .align_y(iced_widget::core::alignment::Vertical::Center),
            );
        }
        Some(tab) => {
            let tab_id = tab.id;
            match content.content(tab_id) {
                Some(TabContent::Browse(b)) => col = col.push(browse_view(tab_id, b)),
                Some(TabContent::Query(q)) => col = col.push(query_view(tab_id, q)),
                None => {}
            }
        }
    }

    let outer_container = container(col.padding(region.padding))
        .width(width)
        .height(iced_widget::core::Length::Fill)
        .style(
            move |_t: &iced_widget::Theme| iced_widget::container::Style {
                background: Some(byteui::theme::color::current().bg.into()),
                border: outer,
                ..iced_widget::container::Style::default()
            },
        );
    outer_container.into()
}

/// Database 面板 tab 栏"溢出下拉"浮层。**必须**在 `App::view` 顶层
/// `stack![base, ...]` 里拼(同 `terminal::term_tab_overflow_popup` 文档
/// 解释的理由——`anchor`/`window_size` 是全窗口坐标系,嵌在 `content_pane`
/// 自己的局部布局里换算位置会跟真实点击位置对不上)。
pub fn tab_overflow_popup<'a>(
    app: &'a crate::app::App,
    ws_state: &'a WorkspaceState,
) -> Option<Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>> {
    let content = ws_state.content();
    let anchor = content.tab_overflow_anchor()?;
    // 下拉列出该面板内**真实** tab,方便一览/直接跳到任一项,而不是只列
    // 当前被挤出可见区的子集。"空白"占位不进列表(点开也没什么可跳的);
    // 下标沿用调用方 `on_select`/`on_close`(`idx - 1` 换算)既有的 1 起步
    // 方案(0 留给空白,虽然它现在不会出现在列表里,翻译逻辑不用跟着改)。
    let mut overflow_entries: Vec<crate::tab_widget::TabOverflowEntry<'_, Message>> = Vec::new();
    for (i, tab) in content.tabs().iter().enumerate() {
        let idx = i + 1;
        overflow_entries.push(crate::tab_widget::TabOverflowEntry {
            index: idx,
            prefix: None,
            title: tab_title(tab, ws_state),
            active: Some(i) == content.active_idx(),
            closable: true,
            hover_t: app.hover_progress(crate::app::HoverId::TabOverflowRow(idx)),
        });
    }
    Some(crate::tab_widget::tab_overflow_menu(
        crate::tab_widget::TabOverflowMenuArgs {
            entries: overflow_entries,
            anchor,
            window_size: app.window_size,
            on_select: |idx| {
                if idx == 0 {
                    Message::SelectBlankTab
                } else {
                    Message::SelectTab(idx - 1)
                }
            },
            on_close: |idx| Message::CloseTab(idx - 1),
            on_dismiss: Message::TabOverflowDismiss,
            on_row_hover: move |idx, hovered| {
                Message::Hover(crate::app::HoverId::TabOverflowRow(idx), hovered)
            },
        },
    ))
}

fn tab_title(tab: &DatabaseTab, ws_state: &WorkspaceState) -> String {
    let source_name = |id: &str| {
        ws_state
            .sources()
            .iter()
            .find(|s| s.id == id)
            .map(|s| s.name.clone())
            .unwrap_or_else(|| id.to_string())
    };
    match &tab.kind {
        DatabaseTabKind::Table {
            source_id,
            schema,
            table,
        } => match schema {
            Some(s) => format!("{table}@{}.{s}", source_name(source_id)),
            None => format!("{table}@{}", source_name(source_id)),
        },
        DatabaseTabKind::Collection { source_id, name } => {
            format!("{name}@{}", source_name(source_id))
        }
        DatabaseTabKind::Query {
            source_id,
            console_seq,
        } => format!("查询 {console_seq}@{}", source_name(source_id)),
    }
}

fn tab_title_display_width(title: &str) -> f32 {
    // 同 `workspace.rs::preview_tab_display_width` 的估算思路:字符数 *
    // 单字宽 + tab 内边距/关闭按钮的固定开销,不做真实文本测量(tab_window
    // 只需要一个足够准的相对宽度做窗口裁剪)。
    title.chars().count() as f32 * 8.0 + 56.0
}

/// 按"空白占位 + 各数据库 tab"的真实估算宽重建宽度向量并调用
/// `DatabaseContentState::reveal_tab` 把选中 tab 带入可见区。用实际内容宽
/// (而非早年的统一上限 `PANEL_TAB_MAX_W`)喂给翻页窗口数学,见渲染侧
/// `content_pane` 同款 `tab_title_display_width` 口径。
fn reveal_tab_widths(ws_state: &mut WorkspaceState, target: usize) {
    let widths = {
        let content = ws_state.content();
        let mut w = vec![tab_title_display_width("空白")];
        w.extend(
            content
                .tabs()
                .iter()
                .map(|tab| tab_title_display_width(&tab_title(tab, ws_state))),
        );
        w
    };
    ws_state.content_mut().reveal_tab(&widths, target);
}

// 各输入框/SQL 编辑器的稳定 `widget::Id`,供右键菜单把焦点移到被右键的
// 输入(复制/粘贴作用于它,见 main.rs 的合成键盘事件)。表单字段是单例、
// 固定 id;浏览 WHERE/ORDER BY 与查询 SQL 编辑器按 `tab_id` 区分(同个 tab
// 内是单例,多个 tab 互不抢焦点)。
fn form_field_id(component: &'static str) -> iced_widget::core::widget::Id {
    match component {
        "name" => iced_widget::core::widget::Id::new("db-form-name"),
        "sqlite-database" => iced_widget::core::widget::Id::new("db-form-sqlite-database"),
        "uri" => iced_widget::core::widget::Id::new("db-form-uri"),
        "host" => iced_widget::core::widget::Id::new("db-form-host"),
        "port" => iced_widget::core::widget::Id::new("db-form-port"),
        "database" => iced_widget::core::widget::Id::new("db-form-database"),
        "username" => iced_widget::core::widget::Id::new("db-form-username"),
        "password" => iced_widget::core::widget::Id::new("db-form-password"),
        _ => iced_widget::core::widget::Id::new("db-form-unknown"),
    }
}

/// `form_field_id` 全部合法 component 名字,`CaptureFormFocus` 用它逐个比对
/// 当前遍历到的 focusable id 是不是新增/编辑表单的某个字段——不用挨个字段
/// 单独开一个 `HoverId` 式的 static,一次遍历顺带查完这张表单所有字段。
const DB_FORM_FIELD_COMPONENTS: &[&str] = &[
    "name",
    "sqlite-database",
    "uri",
    "host",
    "port",
    "database",
    "username",
    "password",
];

static DB_FORM_FOCUSED: std::sync::LazyLock<std::sync::Mutex<bool>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(false));

/// 读走并复位(消费式)数据库新增/编辑表单**任意一个字段**上一帧是否持有
/// iced 内部真实焦点——同 `extensions::files::take_search_focused` 的既有
/// 桥接手法。main.rs 键盘路由据此放行给标准 iced 管线,不再像旧版那样只看
/// "表单是否打开"这个粗粒度信号(2026-09 用户反馈根因:数据库/主机面板与
/// Agent 终端分栏同屏显示时,表单开着但用户实际点进的是右侧终端输入框,
/// 旧版信号仍卡真,导致终端收不到任何按键——面板"可不可见"跟字段"是否真
/// 聚焦"是两回事,必须查后者)。
pub(crate) fn take_form_focused() -> bool {
    std::mem::replace(&mut *DB_FORM_FOCUSED.lock().unwrap(), false)
}

/// 每帧 `interface.operate()` 跑一遍:命中 `DB_FORM_FIELD_COMPONENTS` 里任一
/// 字段且该字段真聚焦,就把 `DB_FORM_FOCUSED` 置真。只在找到"真聚焦"时才
/// 写 `true`,不在遇到未聚焦的匹配字段时写回 `false`——由 `take_form_focused`
/// 消费式复位负责清零(同一帧内至多一个 widget 持有真焦点,不会有两个匹配
/// 字段互相覆盖出错误结果)。`traverse` 必须调用传入的 `operate` 闭包才能
/// 继续递归子节点,道理同 `extensions::files::CaptureSearchFocus` 的既有文档。
pub(crate) struct CaptureFormFocus;
impl iced_widget::core::widget::Operation<()> for CaptureFormFocus {
    fn focusable(
        &mut self,
        id: Option<&iced_widget::core::widget::Id>,
        _bounds: iced_widget::core::Rectangle,
        state: &mut dyn iced_widget::core::widget::operation::Focusable,
    ) {
        let Some(id) = id else { return };
        for component in DB_FORM_FIELD_COMPONENTS {
            if *id == form_field_id(component) && state.is_focused() {
                *DB_FORM_FOCUSED.lock().unwrap() = true;
                return;
            }
        }
    }

    fn traverse(
        &mut self,
        operate: &mut dyn for<'a> FnMut(
            &'a mut (dyn iced_widget::core::widget::Operation<()> + 'a),
        ),
    ) {
        operate(self);
    }
}
fn new_tab_field_id(component: &'static str, _tab_id: usize) -> iced_widget::core::widget::Id {
    // 内容窗格同一时刻只渲染激活 tab 的控件,不同 tab 用同一个静态 id 不冲突
    // (激活 tab 的编辑器是唯一在 widget 树里的那个)。
    match component {
        "browse-where" => iced_widget::core::widget::Id::new("db-browse-where"),
        "browse-order" => iced_widget::core::widget::Id::new("db-browse-order"),
        "query" => iced_widget::core::widget::Id::new("db-query"),
        _ => iced_widget::core::widget::Id::new("db-unknown"),
    }
}

/// 给数据库表单/浏览/编辑器等输入框套上统一右键菜单封装:外包
/// `MouseArea::on_right_press`,右键下发 `TextInputMenuOpen`(见
/// `byteui::interaction::context_menu`)。`secure` 输入(密码)禁用
/// 复制/剪切(菜单项灰掉),见 `text_input_menu_popup`。
fn wrap_form_input<'a, I>(
    input: I,
    id: iced_widget::core::widget::Id,
    secure: bool,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>
where
    I: Into<Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>>,
{
    byteui::interaction::context_menu::wrap(
        input.into(),
        Some(Message::TextInputMenuOpen(crate::app::TextInputTarget {
            id,
            secure,
        })),
    )
}

fn browse_view<'a>(
    tab_id: usize,
    b: &'a BrowseState,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let toolbar = row![
        wrap_form_input(
            byteui::form::input_text::view(
                "WHERE(原始 SQL 片段,例如 id > 100)",
                &b.where_clause,
                false,
                Some(new_tab_field_id("browse-where", tab_id)),
                false,
                Some(Message::BrowseRun(tab_id)),
                false,
                move |v| Message::BrowseWhereChanged(tab_id, v),
            ),
            new_tab_field_id("browse-where", tab_id),
            false,
        ),
        wrap_form_input(
            byteui::form::input_text::view(
                "ORDER BY(原始 SQL 片段,例如 title DESC)",
                &b.order_by,
                false,
                Some(new_tab_field_id("browse-order", tab_id)),
                false,
                Some(Message::BrowseRun(tab_id)),
                false,
                move |v| Message::BrowseOrderByChanged(tab_id, v),
            ),
            new_tab_field_id("browse-order", tab_id),
            false,
        ),
        byteui::form::select::view(&PAGE_SIZES[..], Some(&b.page_size), move |v| {
            Message::BrowsePageSizeChanged(tab_id, v)
        }),
        button(text("上一页")).on_press_maybe((b.page > 0).then_some(Message::BrowsePrev(tab_id))),
        button(text("下一页")).on_press_maybe(b.has_more.then_some(Message::BrowseNext(tab_id))),
    ]
    .spacing(8)
    .align_y(iced_widget::core::Alignment::Center);

    let mut col = column![toolbar].spacing(8);

    if b.loading {
        col = col.push(
            text("加载中…")
                .size(byteui::theme::font::caption_sm())
                .color(byteui::theme::color::current().dim),
        );
    }
    if let Some(e) = &b.error {
        col = col.push(
            text(format!("✗ {e}"))
                .size(byteui::theme::font::caption_sm())
                .color(byteui::theme::color::current().red),
        );
    }

    match &b.result {
        None => {
            if !b.loading && b.error.is_none() {
                col = col.push(
                    text("暂无数据")
                        .size(byteui::theme::font::body())
                        .color(byteui::theme::color::current().dim),
                );
            }
        }
        Some(result) if result.rows.is_empty() => {
            col = col.push(
                text("该表当前没有数据(或筛选条件不匹配任何行)")
                    .size(byteui::theme::font::body())
                    .color(byteui::theme::color::current().dim),
            );
        }
        Some(result) => {
            col = col.push(result_table(tab_id, result, true));
        }
    }

    col.into()
}

/// 结果网格。`sortable` 为 `true` 时(浏览页)列头可点写排序;查询控制台
/// 的结果(`sortable=false`)纯展示,不接排序点击(设计文档"架构与数据流
/// §6":查询结果不支持再排序)。
fn result_table<'a>(
    tab_id: usize,
    result: &'a QueryResult,
    sortable: bool,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    // 点列头 = 把 `"{列名} ASC"` 写进 ORDER BY 框(设计文档"架构与数据流
    // §5":同列再点一次不做两态切换,DESC 由用户在框里手动追加——原始片段
    // 输入框的既定口径下,点击只是个"快速起手")。
    let header = row(result
        .columns
        .iter()
        .map(|c| {
            let label = text(c.clone())
                .size(byteui::theme::font::caption_sm())
                .color(byteui::theme::color::current().cream);
            if sortable {
                let asc = format!("{c} ASC");
                button(label)
                    .on_press(Message::BrowseOrderByChanged(tab_id, asc))
                    .style(|_t: &iced_widget::Theme, _s| iced_widget::button::Style {
                        background: None,
                        ..iced_widget::button::Style::default()
                    })
                    .into()
            } else {
                container(label).into()
            }
        })
        .collect::<Vec<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>>>())
    .spacing(12);

    let rows_col = column(
        result
            .rows
            .iter()
            .map(|r| {
                row(r
                    .iter()
                    .map(|cell| match cell {
                        CellValue::Text(s) => text(s.clone())
                            .size(byteui::theme::font::caption_sm())
                            .color(byteui::theme::color::current().body)
                            .into(),
                        CellValue::Null => text("NULL")
                            .size(byteui::theme::font::caption_sm())
                            .color(byteui::theme::color::current().dim)
                            .into(),
                    })
                    .collect::<Vec<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>>>())
                .spacing(12)
                .into()
            })
            .collect::<Vec<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>>>(),
    )
    .spacing(4);

    scrollable(column![header, rows_col].spacing(6))
        .direction(scrollable::Direction::Both {
            vertical: byteui::interaction::scrollbar::scrollbar(),
            horizontal: byteui::interaction::scrollbar::scrollbar(),
        })
        .style(|_t, _s| byteui::interaction::scrollbar::scrollbar_style())
        .into()
}

/// SQL 查询控制台 tab。`text_editor` 多行输入 + "执行"按钮 + 结果区;
/// `QueryOutcome` 三态(行集 / 受影响行数 / DDL)分别渲染。
fn query_view<'a>(
    tab_id: usize,
    q: &'a QueryState,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let toolbar = row![
        button(text(if q.running { "执行中…" } else { "执行" }))
            .on_press_maybe((!q.running).then_some(Message::QueryRun(tab_id))),
        text("Cmd+Enter 快捷执行")
            .size(byteui::theme::font::caption_sm())
            .color(byteui::theme::color::current().dim),
    ]
    .spacing(8)
    .align_y(iced_widget::core::Alignment::Center);

    let editor = wrap_form_input(
        byteui::form::text_area::view(
            &q.sql,
            "SELECT * FROM ...",
            Some(new_tab_field_id("query", tab_id)),
            false,
            Some(160.0),
            move |action| Message::QueryTextAction(tab_id, action),
        ),
        new_tab_field_id("query", tab_id),
        false,
    );

    let mut col = column![toolbar, editor].spacing(8);

    if let Some(e) = &q.error {
        col = col.push(
            text(format!("✗ {e}"))
                .size(byteui::theme::font::caption_sm())
                .color(byteui::theme::color::current().red),
        );
    }

    match &q.result {
        None => {}
        Some(QueryOutcome::Rows(result)) if result.rows.is_empty() => {
            col = col.push(
                text("查询未返回任何行")
                    .size(byteui::theme::font::body())
                    .color(byteui::theme::color::current().dim),
            );
        }
        Some(QueryOutcome::Rows(result)) => {
            col = col.push(result_table(tab_id, result, false));
        }
        Some(QueryOutcome::Affected(n)) => {
            col = col.push(
                text(format!("{n} 行受影响"))
                    .size(byteui::theme::font::body())
                    .color(byteui::theme::color::current().green),
            );
        }
        Some(QueryOutcome::Ddl) => {
            col = col.push(
                text("执行成功")
                    .size(byteui::theme::font::body())
                    .color(byteui::theme::color::current().green),
            );
        }
    }

    col.into()
}

/// 单行渲染。source_id 用于构造 `ToggleTable`/`OpenTableTab`/`OpenCollectionTab`,driver
/// 决定 MongoDB 集合行(无 chevron、点文字开集合 tab)与关系型表/视图行的差异。
fn schema_tree_row<'a>(
    source_id: &str,
    driver: DriverKind,
    r: SchemaRow<'a>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let indent = text("  ".repeat(r.depth)).size(crate::workspace::tree_row_font_size());
    match r.kind {
        SchemaRowKind::Schema(name) => {
            let chevron = if r.expanded {
                icons::IconKind::ChevronDown
            } else {
                icons::IconKind::ChevronRight
            };
            let folder = if r.expanded {
                icons::IconKind::FolderOpen
            } else {
                icons::IconKind::Folder
            };
            button(
                row![
                    indent,
                    icons::view(
                        chevron,
                        byteui::theme::icon_size::chevron(),
                        byteui::theme::color::current().dim
                    ),
                    icons::view(
                        folder,
                        byteui::theme::icon_size::row(),
                        byteui::theme::color::current().dim
                    ),
                    text(name.to_string())
                        .size(crate::workspace::tree_row_font_size())
                        .color(byteui::theme::color::current().cream),
                ]
                .spacing(byteui::theme::icon_size::tree_row_gap())
                .align_y(iced_widget::core::Alignment::Center),
            )
            .on_press(Message::ToggleSchema(
                source_id.to_string(),
                name.to_string(),
            ))
            .width(Length::Fill)
            .style(|_t: &iced_widget::Theme, _s| iced_widget::button::Style {
                background: None,
                ..iced_widget::button::Style::default()
            })
            .into()
        }
        SchemaRowKind::Table(t) => {
            let icon_kind = if t.is_view {
                icons::IconKind::Eye
            } else {
                icons::IconKind::Table // MongoDB 集合复用这个图标,视觉上够用(设计文档)
            };
            let icon = icons::view(
                icon_kind,
                byteui::theme::icon_size::row(),
                byteui::theme::color::current().dim,
            );
            let label = text(t.name.clone())
                .size(crate::workspace::tree_row_font_size())
                .color(byteui::theme::color::current().cream);
            let open_msg = if driver == DriverKind::MongoDB {
                Message::OpenCollectionTab {
                    source_id: source_id.to_string(),
                    name: t.name.clone(),
                }
            } else {
                Message::OpenTableTab {
                    source_id: source_id.to_string(),
                    schema: t.schema.clone(),
                    table: t.name.clone(),
                }
            };
            let label_btn = button(
                row![icon, label]
                    .spacing(byteui::theme::icon_size::tree_row_gap())
                    .align_y(iced_widget::core::Alignment::Center),
            )
            .on_press(open_msg)
            .width(Length::Fill)
            .style(|_t: &iced_widget::Theme, _s| iced_widget::button::Style {
                background: None,
                ..iced_widget::button::Style::default()
            });

            let mut row_el = row![indent].spacing(byteui::theme::icon_size::tree_row_gap());
            if driver == DriverKind::MongoDB {
                // MongoDB 集合无列层可展开,不画 chevron,留同宽空位对齐。
                row_el = row_el.push(
                    iced_widget::space::Space::new()
                        .width(Length::Fixed(byteui::theme::icon_size::chevron())),
                );
            } else {
                let chevron_icon = if r.expanded {
                    icons::IconKind::ChevronDown
                } else {
                    icons::IconKind::ChevronRight
                };
                let chevron_btn = button(icons::view(
                    chevron_icon,
                    byteui::theme::icon_size::chevron(),
                    byteui::theme::color::current().dim,
                ))
                .on_press(Message::ToggleTable {
                    source_id: source_id.to_string(),
                    schema: t.schema.clone(),
                    table: t.name.clone(),
                })
                .style(|_t: &iced_widget::Theme, _s| iced_widget::button::Style {
                    background: None,
                    ..iced_widget::button::Style::default()
                });
                row_el = row_el.push(chevron_btn);
            }
            row_el
                .push(label_btn)
                .align_y(iced_widget::core::Alignment::Center)
                .into()
        }
        SchemaRowKind::Column(c) => {
            // 可空性用颜色深浅表达:非空 CREAM、可空 BODY(不加 "NOT NULL" 文本)
            let name_color = if c.nullable {
                byteui::theme::color::current().body
            } else {
                byteui::theme::color::current().cream
            };
            row![
                indent,
                iced_widget::space::Space::new()
                    .width(Length::Fixed(
                        byteui::theme::icon_size::chevron()
                            + byteui::theme::icon_size::tree_row_gap()
                            + byteui::theme::icon_size::row()
                            + byteui::theme::icon_size::tree_row_gap(),
                    ))
                    .height(Length::Shrink),
                text(c.name.clone())
                    .size(crate::workspace::tree_row_font_size())
                    .color(name_color),
                text(c.type_name.clone())
                    .size(byteui::theme::font::caption_sm())
                    .color(byteui::theme::color::current().dim),
            ]
            .spacing(6)
            .align_y(iced_widget::core::Alignment::Center)
            .into()
        }
        SchemaRowKind::ColumnsLoading => row![
            indent,
            iced_widget::space::Space::new()
                .width(Length::Fixed(
                    byteui::theme::icon_size::chevron()
                        + byteui::theme::icon_size::tree_row_gap()
                        + byteui::theme::icon_size::row()
                        + byteui::theme::icon_size::tree_row_gap(),
                ))
                .height(Length::Shrink),
            text("加载列中…")
                .size(crate::workspace::tree_row_font_size())
                .color(byteui::theme::color::current().dim),
        ]
        .spacing(6)
        .align_y(iced_widget::core::Alignment::Center)
        .into(),
        SchemaRowKind::ColumnsFailed(e) => row![
            indent,
            iced_widget::space::Space::new()
                .width(Length::Fixed(
                    byteui::theme::icon_size::chevron()
                        + byteui::theme::icon_size::tree_row_gap()
                        + byteui::theme::icon_size::row()
                        + byteui::theme::icon_size::tree_row_gap(),
                ))
                .height(Length::Shrink),
            text(format!("列加载失败:{e}"))
                .size(crate::workspace::tree_row_font_size())
                .color(byteui::theme::color::current().red),
            text("(收起再展开可重试)")
                .size(byteui::theme::font::caption_sm())
                .color(byteui::theme::color::current().dim),
        ]
        .spacing(6)
        .align_y(iced_widget::core::Alignment::Center)
        .into(),
    }
}

/// 右侧内容窗格里的一个 tab。`id` 是跨重排/关闭都稳定的标识(消息/异步
/// 结果按它路由,不用索引——索引会随关闭其它 tab 而漂移)。
pub struct DatabaseTab {
    pub id: usize,
    pub kind: DatabaseTabKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DatabaseTabKind {
    Table {
        source_id: String,
        schema: Option<String>,
        table: String,
    },
    Collection {
        source_id: String,
        name: String,
    },
    /// `console_seq` 只用来生成默认标题("查询 1"/"查询 2"),不参与去重
    /// 比较——`open_query` 永远新开,同一数据源可以有多个查询 tab。
    Query {
        source_id: String,
        console_seq: u32,
    },
}

impl DatabaseTabKind {
    fn source_id(&self) -> &str {
        match self {
            DatabaseTabKind::Table { source_id, .. }
            | DatabaseTabKind::Collection { source_id, .. }
            | DatabaseTabKind::Query { source_id, .. } => source_id,
        }
    }
}

/// 表格/集合浏览页的页大小可选项。
pub const PAGE_SIZES: [u32; 3] = [50, 100, 500];

/// 表格/集合浏览 tab 的状态。`run_seq` 是过期结果防线:每次发起查询
/// `+1` 并带进异步闭包,结果落地时核对是否仍是发出时那个值(设计文档
/// "异步路由与过期防线"一节)。
pub struct BrowseState {
    pub where_clause: String,
    pub order_by: String,
    pub page: u32,
    pub page_size: u32,
    pub has_more: bool,
    pub loading: bool,
    pub error: Option<String>,
    pub result: Option<QueryResult>,
    run_seq: u64,
}

impl Default for BrowseState {
    fn default() -> Self {
        Self {
            where_clause: String::new(),
            order_by: String::new(),
            page: 0,
            page_size: PAGE_SIZES[0],
            has_more: false,
            loading: false,
            error: None,
            result: None,
            run_seq: 0,
        }
    }
}

impl BrowseState {
    /// 发起一次新请求前调用:`run_seq +1` 并置 loading,返回新 seq 供
    /// 异步闭包携带。
    pub fn begin_run(&mut self) -> u64 {
        self.run_seq += 1;
        self.loading = true;
        self.run_seq
    }

    /// 结果落地时核对:seq 不是当前这轮 → 过期,调用方应丢弃不落地。
    pub fn is_current_run(&self, seq: u64) -> bool {
        self.loading && self.run_seq == seq
    }
}

/// SQL 查询控制台 tab 的状态。`sql` 用 `text_editor::Content`(同
/// `todo.rs` 任务内容多行编辑框的既有用法),不是纯 `String`——
/// `byteui::form::text_area::view` 要求这个类型。
pub struct QueryState {
    pub sql: iced_widget::text_editor::Content,
    pub running: bool,
    pub error: Option<String>,
    pub result: Option<QueryOutcome>,
    run_seq: u64,
}

impl Default for QueryState {
    fn default() -> Self {
        Self {
            sql: iced_widget::text_editor::Content::new(),
            running: false,
            error: None,
            result: None,
            run_seq: 0,
        }
    }
}

impl QueryState {
    pub fn begin_run(&mut self) -> u64 {
        self.run_seq += 1;
        self.running = true;
        self.run_seq
    }

    pub fn is_current_run(&self, seq: u64) -> bool {
        self.running && self.run_seq == seq
    }
}

pub enum TabContent {
    Browse(BrowseState),
    Query(QueryState),
}

#[derive(Debug, Clone, PartialEq)]
pub enum CellValue {
    Text(String),
    Null,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<CellValue>>,
}

#[derive(Debug, Clone)]
pub enum QueryOutcome {
    Rows(QueryResult),
    Affected(u64),
    Ddl,
}

/// 右侧内容窗格:多个表/集合/查询 tab,`active` 是**索引**(同
/// `PreviewPane::active_idx()` 的约定,tab 栏渲染/hover 状态按索引找)。
/// `None` = 当前选中的是 tab 栏最前面固定的"空白"占位 tab(不在
/// `tabs` 里,参考 `extensions/ssh.rs::ssh_active` 同款 `Option` 设计)。
/// `contents` 按**稳定 id**存(消息/异步结果按 id 路由,索引会随关闭
/// 漂移)。
#[derive(Default)]
pub struct DatabaseContentState {
    tabs: Vec<DatabaseTab>,
    contents: std::collections::HashMap<usize, TabContent>,
    active: Option<usize>,
    next_id: usize,
    next_console_seq: u32,
    /// tab 栏箭头翻页的窗口起点(同 `Workspace::preview_tab_first` 的用法),
    /// 每次渲染都交给 `tab_window` 钳到合法范围,这里存的只是"用户上次翻到
    /// 哪"的粗略意图。
    tab_scroll_first: usize,
    /// tab 栏溢出下拉的悬浮锚点,语义同 `Workspace::term_tab_overflow_anchor`。
    tab_overflow_anchor: Option<(f32, f32)>,
}

impl DatabaseContentState {
    pub fn tabs(&self) -> &[DatabaseTab] {
        &self.tabs
    }

    pub fn active_idx(&self) -> Option<usize> {
        self.active
    }

    pub fn tab_scroll_first(&self) -> usize {
        self.tab_scroll_first
    }

    pub fn tab_overflow_anchor(&self) -> Option<(f32, f32)> {
        self.tab_overflow_anchor
    }

    pub fn toggle_tab_overflow(&mut self, cursor: (f32, f32)) {
        self.tab_overflow_anchor = if self.tab_overflow_anchor.is_some() {
            None
        } else {
            Some(cursor)
        };
    }

    pub fn dismiss_tab_overflow(&mut self) {
        self.tab_overflow_anchor = None;
    }

    /// 选中 `target`(扁平下标,0 留给空白占位 tab)后,若它当前隐藏,重新
    /// 钳出包含它的窗口;已可见则不动。`widths` 由渲染侧按 `tab_bar_avail_px`
    /// 同一套口径传入(含开头的空白占位 tab 宽度)。
    pub fn reveal_tab(&mut self, widths: &[f32], target: usize) {
        self.tab_scroll_first = crate::tab_widget::tab_window_reveal(
            widths,
            4.0,
            byteui::theme::geometry::tab_bar_avail_px(),
            self.tab_scroll_first,
            target,
        );
    }

    pub fn active_tab(&self) -> Option<&DatabaseTab> {
        self.tabs.get(self.active?)
    }

    pub fn content(&self, id: usize) -> Option<&TabContent> {
        self.contents.get(&id)
    }

    pub fn content_mut(&mut self, id: usize) -> Option<&mut TabContent> {
        self.contents.get_mut(&id)
    }

    fn push(&mut self, kind: DatabaseTabKind, content: TabContent) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        self.tabs.push(DatabaseTab { id, kind });
        self.contents.insert(id, content);
        self.active = Some(self.tabs.len() - 1);
        id
    }

    /// 已开同一张表的 tab → 聚焦(内容/游标不重置);否则新开一个空浏览态。
    pub fn open_table(
        &mut self,
        source_id: String,
        schema: Option<String>,
        table: String,
    ) -> usize {
        if let Some((idx, tab)) = self.tabs.iter().enumerate().find(|(_, t)| {
            matches!(&t.kind, DatabaseTabKind::Table { source_id: s, schema: sc, table: tb }
                if *s == source_id && *sc == schema && *tb == table)
        }) {
            self.active = Some(idx);
            return tab.id;
        }
        self.push(
            DatabaseTabKind::Table {
                source_id,
                schema,
                table,
            },
            TabContent::Browse(BrowseState::default()),
        )
    }

    /// 已开同一个集合的 tab → 聚焦;否则新开。
    pub fn open_collection(&mut self, source_id: String, name: String) -> usize {
        if let Some((idx, tab)) = self.tabs.iter().enumerate().find(|(_, t)| {
            matches!(&t.kind, DatabaseTabKind::Collection { source_id: s, name: n }
                if *s == source_id && *n == name)
        }) {
            self.active = Some(idx);
            return tab.id;
        }
        self.push(
            DatabaseTabKind::Collection { source_id, name },
            TabContent::Browse(BrowseState::default()),
        )
    }

    /// 查询 tab 永不去重,`console_seq` 递增当默认标题的编号来源。
    pub fn open_query(&mut self, source_id: String) -> usize {
        self.next_console_seq += 1;
        let seq = self.next_console_seq;
        self.push(
            DatabaseTabKind::Query {
                source_id,
                console_seq: seq,
            },
            TabContent::Query(QueryState::default()),
        )
    }

    pub fn select(&mut self, idx: usize) {
        if idx < self.tabs.len() {
            self.active = Some(idx);
        }
        self.tab_overflow_anchor = None;
    }

    /// 点固定的"空白"占位 tab:不对应 `tabs` 里任何一条记录,选中态就是
    /// `active == None`(同 `ssh::update` 里 `Message::SelectBlankTab` 的
    /// 处理)。
    pub fn select_blank(&mut self) {
        self.active = None;
        self.tab_overflow_anchor = None;
    }

    /// 关闭指定索引的 tab。`active` 调整规则同浏览器标签页惯例:关掉
    /// active 之前的 tab → active 索引减 1(仍指向原 tab);关掉 active
    /// 自己且不是最后一个 → active 索引不变(自然落到后一个 tab 上);
    /// 关掉最后一个 tab 且它正是 active → active 收缩到新的最后一个。
    /// `active == None`(正显示"空白"占位 tab)时关掉某个后台 tab 不改变
    /// 选中态,仍留在空白 tab 上。
    pub fn close(&mut self, idx: usize) {
        if idx >= self.tabs.len() {
            return;
        }
        let id = self.tabs[idx].id;
        self.tabs.remove(idx);
        self.contents.remove(&id);
        if self.tabs.is_empty() {
            self.active = None;
            return;
        }
        match self.active {
            None => {}
            Some(a) if a >= self.tabs.len() => self.active = Some(self.tabs.len() - 1),
            Some(a) if idx < a => self.active = Some(a - 1),
            Some(_) => {}
        }
    }

    /// 数据源被删除/编辑保存(连接信息可能变了)时,关掉所有关联 tab。
    pub fn close_by_source(&mut self, source_id: &str) {
        while let Some(idx) = self
            .tabs
            .iter()
            .position(|t| t.kind.source_id() == source_id)
        {
            self.close(idx);
        }
    }
}

/// 数据浏览/查询执行用的驱动原生连接池(区别于阶段 1/2 introspection
/// 专用的 `sqlx::AnyPool`——原生池才能正确解码真实列类型,设计文档
/// "架构与数据流 §4"一节)。
enum NativePool {
    Pg(sqlx::PgPool),
    MySql(sqlx::MySqlPool),
    Sqlite(sqlx::SqlitePool),
}

async fn connect_native(kind: DriverKind, url: &str) -> Result<NativePool, String> {
    match kind {
        DriverKind::Postgres => sqlx::PgPool::connect(url)
            .await
            .map(NativePool::Pg)
            .map_err(|e| e.to_string()),
        DriverKind::MySQL => sqlx::MySqlPool::connect(url)
            .await
            .map(NativePool::MySql)
            .map_err(|e| e.to_string()),
        DriverKind::Sqlite => sqlx::SqlitePool::connect(url)
            .await
            .map(NativePool::Sqlite)
            .map_err(|e| e.to_string()),
        DriverKind::MongoDB => unreachable!("connect_native 不处理 MongoDB"),
    }
}

async fn close_native(pool: NativePool) {
    match pool {
        NativePool::Pg(p) => p.close().await,
        NativePool::MySql(p) => p.close().await,
        NativePool::Sqlite(p) => p.close().await,
    }
}

/// 行 → 结果集:表头来自首行的列元信息(0 行结果时没有列头——已知限制,
/// 见设计文档"错误处理"一节的补充说明,浏览页对 0 行走友好提示而不是
/// 空表头)。三种 `Row` 类型共享这一个泛型函数。
fn rows_to_result<R: sqlx::Row>(rows: &[R], stringify: impl Fn(&R) -> Vec<CellValue>) -> QueryResult
where
    <R::Database as sqlx::Database>::Column: sqlx::Column,
{
    use sqlx::Column;
    let columns = rows
        .first()
        .map(|r| r.columns().iter().map(|c| c.name().to_string()).collect())
        .unwrap_or_default();
    let rows = rows.iter().map(&stringify).collect();
    QueryResult { columns, rows }
}

/// Postgres 行转字符串。常见标量类型直接 match `TypeInfo::name()`;
/// UUID/时间/JSON(B)/NUMERIC 走 Step 1 新加的 sqlx feature 解码;解不出的
/// 生僻类型(数组、自定义枚举、复合类型…)显示占位,不 panic、不让整行
/// 失败(设计文档"架构与数据流 §4")。
fn stringify_pg_row(row: &sqlx::postgres::PgRow) -> Vec<CellValue> {
    use sqlx::{Row, TypeInfo, ValueRef};
    let mut out = Vec::with_capacity(row.len());
    for i in 0..row.len() {
        let Ok(raw) = row.try_get_raw(i) else {
            out.push(CellValue::Null);
            continue;
        };
        if raw.is_null() {
            out.push(CellValue::Null);
            continue;
        }
        let type_name = raw.type_info().name().to_ascii_uppercase();
        let decoded: Option<String> = match type_name.as_str() {
            "BOOL" => row.try_get::<bool, _>(i).ok().map(|v| v.to_string()),
            "INT2" => row.try_get::<i16, _>(i).ok().map(|v| v.to_string()),
            "INT4" => row.try_get::<i32, _>(i).ok().map(|v| v.to_string()),
            "INT8" => row.try_get::<i64, _>(i).ok().map(|v| v.to_string()),
            "FLOAT4" => row.try_get::<f32, _>(i).ok().map(|v| v.to_string()),
            "FLOAT8" => row.try_get::<f64, _>(i).ok().map(|v| v.to_string()),
            "TEXT" | "VARCHAR" | "BPCHAR" | "NAME" | "CITEXT" => row.try_get::<String, _>(i).ok(),
            "BYTEA" => row
                .try_get::<Vec<u8>, _>(i)
                .ok()
                .map(|b| format!("<{} bytes>", b.len())),
            "UUID" => row
                .try_get::<sqlx::types::Uuid, _>(i)
                .ok()
                .map(|v| v.to_string()),
            "TIMESTAMP" => row
                .try_get::<sqlx::types::chrono::NaiveDateTime, _>(i)
                .ok()
                .map(|v| v.to_string()),
            "TIMESTAMPTZ" => row
                .try_get::<sqlx::types::chrono::DateTime<sqlx::types::chrono::Utc>, _>(i)
                .ok()
                .map(|v| v.to_string()),
            "DATE" => row
                .try_get::<sqlx::types::chrono::NaiveDate, _>(i)
                .ok()
                .map(|v| v.to_string()),
            "JSON" | "JSONB" => row
                .try_get::<serde_json::Value, _>(i)
                .ok()
                .map(|v| v.to_string()),
            "NUMERIC" => row
                .try_get::<sqlx::types::Decimal, _>(i)
                .ok()
                .map(|v| v.to_string()),
            _ => None,
        };
        out.push(match decoded {
            Some(s) => CellValue::Text(s),
            None => CellValue::Text(format!("<不支持的类型: {type_name}>")),
        });
    }
    out
}

/// MySQL 行转字符串,类型名集合参照 `information_schema.columns.data_type`
/// 在 MySQL 里的常见取值(大写)。
fn stringify_mysql_row(row: &sqlx::mysql::MySqlRow) -> Vec<CellValue> {
    use sqlx::{Row, TypeInfo, ValueRef};
    let mut out = Vec::with_capacity(row.len());
    for i in 0..row.len() {
        let Ok(raw) = row.try_get_raw(i) else {
            out.push(CellValue::Null);
            continue;
        };
        if raw.is_null() {
            out.push(CellValue::Null);
            continue;
        }
        let type_name = raw.type_info().name().to_ascii_uppercase();
        let decoded: Option<String> = match type_name.as_str() {
            "TINYINT" | "SMALLINT" | "MEDIUMINT" | "INT" | "INTEGER" => {
                row.try_get::<i64, _>(i).ok().map(|v| v.to_string())
            }
            "BIGINT" => row.try_get::<i64, _>(i).ok().map(|v| v.to_string()),
            "FLOAT" => row.try_get::<f32, _>(i).ok().map(|v| v.to_string()),
            "DOUBLE" => row.try_get::<f64, _>(i).ok().map(|v| v.to_string()),
            "VARCHAR" | "CHAR" | "TEXT" | "ENUM" => row.try_get::<String, _>(i).ok(),
            "BLOB" | "VARBINARY" | "BINARY" => row
                .try_get::<Vec<u8>, _>(i)
                .ok()
                .map(|b| format!("<{} bytes>", b.len())),
            "DATE" => row
                .try_get::<sqlx::types::chrono::NaiveDate, _>(i)
                .ok()
                .map(|v| v.to_string()),
            "DATETIME" | "TIMESTAMP" => row
                .try_get::<sqlx::types::chrono::NaiveDateTime, _>(i)
                .ok()
                .map(|v| v.to_string()),
            "JSON" => row
                .try_get::<serde_json::Value, _>(i)
                .ok()
                .map(|v| v.to_string()),
            "DECIMAL" => row
                .try_get::<sqlx::types::Decimal, _>(i)
                .ok()
                .map(|v| v.to_string()),
            _ => None,
        };
        out.push(match decoded {
            Some(s) => CellValue::Text(s),
            None => CellValue::Text(format!("<不支持的类型: {type_name}>")),
        });
    }
    out
}

/// SQLite 行转字符串。SQLite 只有 5 种存储类型(含 NULL),`Any` 驱动
/// 原本也能应付——这里用原生池只是为了和 Postgres/MySQL 走同一套
/// `NativePool`/`rows_to_result` 代码路径,不是因为 SQLite 真的需要。
fn stringify_sqlite_row(row: &sqlx::sqlite::SqliteRow) -> Vec<CellValue> {
    use sqlx::{Row, TypeInfo, ValueRef};
    let mut out = Vec::with_capacity(row.len());
    for i in 0..row.len() {
        let Ok(raw) = row.try_get_raw(i) else {
            out.push(CellValue::Null);
            continue;
        };
        if raw.is_null() {
            out.push(CellValue::Null);
            continue;
        }
        let type_name = raw.type_info().name().to_ascii_uppercase();
        let decoded: Option<String> = match type_name.as_str() {
            "INTEGER" | "BOOLEAN" => row.try_get::<i64, _>(i).ok().map(|v| v.to_string()),
            "REAL" => row.try_get::<f64, _>(i).ok().map(|v| v.to_string()),
            "TEXT" => row.try_get::<String, _>(i).ok(),
            "BLOB" => row
                .try_get::<Vec<u8>, _>(i)
                .ok()
                .map(|b| format!("<{} bytes>", b.len())),
            _ => row.try_get::<String, _>(i).ok(), // SQLite 动态类型,兜底当文本试一次
        };
        out.push(match decoded {
            Some(s) => CellValue::Text(s),
            None => CellValue::Text(format!("<不支持的类型: {type_name}>")),
        });
    }
    out
}

fn quote_table(kind: DriverKind, schema: Option<&str>, table: &str) -> String {
    match kind {
        DriverKind::Postgres => match schema {
            Some(s) => format!("\"{s}\".\"{table}\""),
            None => format!("\"{table}\""),
        },
        DriverKind::MySQL | DriverKind::Sqlite => format!("`{table}`"),
        DriverKind::MongoDB => unreachable!("quote_table 不处理 MongoDB"),
    }
}

/// 浏览页 SQL 生成:`LIMIT page_size+1` 用来判断"是否有下一页"(多出的
/// 第 page_size+1 行渲染前丢弃),不做 `COUNT(*)`(设计文档"架构与数据流
/// §5")。`where_clause`/`order_by` 原样拼接,不转义——原始片段输入框的
/// 既定口径,用户对拼错/注入自担。
fn build_browse_sql(
    kind: DriverKind,
    schema: Option<&str>,
    table: &str,
    where_clause: &str,
    order_by: &str,
    page: u32,
    page_size: u32,
) -> String {
    let mut sql = format!("SELECT * FROM {}", quote_table(kind, schema, table));
    let w = where_clause.trim();
    if !w.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(w);
    }
    let o = order_by.trim();
    if !o.is_empty() {
        sql.push_str(" ORDER BY ");
        sql.push_str(o);
    }
    sql.push_str(&format!(
        " LIMIT {} OFFSET {}",
        page_size as u64 + 1,
        page as u64 * page_size as u64
    ));
    sql
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StatementKind {
    /// `fetch_all` 走结果集渲染。
    Rows,
    /// `execute` 走"N 行受影响"/DDL 文案。
    Execute,
}

/// 按 SQL 文本首个关键字(大小写不敏感、忽略前导空白)分流。设计文档
/// "架构与数据流 §6":不识别 `RETURNING` 子句,`INSERT`/`UPDATE`/`DELETE`
/// 一律走 `Execute`。
fn classify_statement(sql: &str) -> StatementKind {
    let first_word: String = sql
        .trim_start()
        .chars()
        .take_while(|c| c.is_alphabetic())
        .collect::<String>()
        .to_ascii_uppercase();
    match first_word.as_str() {
        "SELECT" | "WITH" | "SHOW" | "EXPLAIN" | "PRAGMA" => StatementKind::Rows,
        _ => StatementKind::Execute,
    }
}

/// `Execute` 分支里进一步区分"DDL(执行成功,不显示行数)" vs "DML(显示
/// 受影响行数)"。
fn is_ddl_keyword(sql: &str) -> bool {
    let first_word: String = sql
        .trim_start()
        .chars()
        .take_while(|c| c.is_alphabetic())
        .collect::<String>()
        .to_ascii_uppercase();
    matches!(
        first_word.as_str(),
        "CREATE" | "ALTER" | "DROP" | "TRUNCATE"
    )
}

/// 浏览页一次查询的结果:`has_more` 由"多取一行"判断(设计文档"架构与
/// 数据流 §5"),渲染前已把多出的那行丢弃。
#[derive(Debug, Clone)]
pub struct BrowsePage {
    pub result: QueryResult,
    pub has_more: bool,
}

/// `browse_table` 的参数对象:8 个位置参数里 `schema`/`table`/
/// `where_clause`/`order_by` 四个 `Option<&str>`/`&str` 挨在一起,顺序传错
/// 编译器发现不了(Rust Design Patterns:Builder,用具名字段替代同类型
/// 位置参数)。
struct BrowseTableParams<'a> {
    kind: DriverKind,
    url: &'a str,
    schema: Option<&'a str>,
    table: &'a str,
    where_clause: &'a str,
    order_by: &'a str,
    page: u32,
    page_size: u32,
}

async fn browse_table(params: BrowseTableParams<'_>) -> Result<BrowsePage, String> {
    let BrowseTableParams {
        kind,
        url,
        schema,
        table,
        where_clause,
        order_by,
        page,
        page_size,
    } = params;
    let sql = build_browse_sql(kind, schema, table, where_clause, order_by, page, page_size);
    let work = async {
        let pool = connect_native(kind, url).await?;
        let mut result = match &pool {
            NativePool::Pg(p) => {
                let rows = sqlx::query(&sql)
                    .fetch_all(p)
                    .await
                    .map_err(|e| e.to_string())?;
                rows_to_result(&rows, stringify_pg_row)
            }
            NativePool::MySql(p) => {
                let rows = sqlx::query(&sql)
                    .fetch_all(p)
                    .await
                    .map_err(|e| e.to_string())?;
                rows_to_result(&rows, stringify_mysql_row)
            }
            NativePool::Sqlite(p) => {
                let rows = sqlx::query(&sql)
                    .fetch_all(p)
                    .await
                    .map_err(|e| e.to_string())?;
                rows_to_result(&rows, stringify_sqlite_row)
            }
        };
        close_native(pool).await;
        let has_more = result.rows.len() > page_size as usize;
        if has_more {
            result.rows.truncate(page_size as usize);
        }
        Ok::<_, String>(BrowsePage { result, has_more })
    };
    match tokio::time::timeout(std::time::Duration::from_secs(15), work).await {
        Ok(r) => r,
        Err(_) => Err("查询超时(15秒)".to_string()),
    }
}

/// MongoDB 集合浏览:游标式拉取,`limit(page_size+1)` 判"是否有下一页"
/// (同浏览页其余路径,不做 `COUNT(*)`)。单列 `document`,整份 JSON 文本
/// (设计文档"架构与数据流 §4")。
async fn browse_collection(
    url: &str,
    db_name: &str,
    name: &str,
    page: u32,
    page_size: u32,
) -> Result<BrowsePage, String> {
    use futures::stream::TryStreamExt;
    let work = async {
        let client = mongodb::Client::with_uri_str(url)
            .await
            .map_err(|e| e.to_string())?;
        let coll: mongodb::Collection<mongodb::bson::Document> =
            client.database(db_name).collection(name);
        let mut cursor = coll
            .find(mongodb::bson::doc! {})
            .skip(page as u64 * page_size as u64)
            .limit(page_size as i64 + 1)
            .await
            .map_err(|e| e.to_string())?;
        let mut docs = Vec::new();
        while let Some(doc) = cursor.try_next().await.map_err(|e| e.to_string())? {
            docs.push(doc);
        }
        let has_more = docs.len() > page_size as usize;
        docs.truncate(page_size as usize);
        let rows = docs
            .into_iter()
            .map(|d| {
                let json = mongodb::bson::Bson::Document(d).into_relaxed_extjson();
                vec![CellValue::Text(
                    serde_json::to_string_pretty(&json).unwrap_or_default(),
                )]
            })
            .collect();
        Ok::<_, String>(BrowsePage {
            result: QueryResult {
                columns: vec!["document".to_string()],
                rows,
            },
            has_more,
        })
    };
    match tokio::time::timeout(std::time::Duration::from_secs(15), work).await {
        Ok(r) => r,
        Err(_) => Err("查询超时(15秒)".to_string()),
    }
}

/// 任意 SQL 执行入口。语句分流见 `classify_statement`(设计文档"架构与
/// 数据流 §6");超时比被动加载的 5 秒更宽(30秒)——用户主动点"执行"、
/// 愿意等,且任意 SQL 可能是有意的慢查询。
async fn run_query(kind: DriverKind, url: &str, sql: &str) -> Result<QueryOutcome, String> {
    let work = async {
        let pool = connect_native(kind, url).await?;
        let outcome = match classify_statement(sql) {
            StatementKind::Rows => {
                let result = match &pool {
                    NativePool::Pg(p) => rows_to_result(
                        &sqlx::query(sql)
                            .fetch_all(p)
                            .await
                            .map_err(|e| e.to_string())?,
                        stringify_pg_row,
                    ),
                    NativePool::MySql(p) => rows_to_result(
                        &sqlx::query(sql)
                            .fetch_all(p)
                            .await
                            .map_err(|e| e.to_string())?,
                        stringify_mysql_row,
                    ),
                    NativePool::Sqlite(p) => rows_to_result(
                        &sqlx::query(sql)
                            .fetch_all(p)
                            .await
                            .map_err(|e| e.to_string())?,
                        stringify_sqlite_row,
                    ),
                };
                QueryOutcome::Rows(result)
            }
            StatementKind::Execute => {
                let affected = match &pool {
                    NativePool::Pg(p) => sqlx::query(sql)
                        .execute(p)
                        .await
                        .map_err(|e| e.to_string())?
                        .rows_affected(),
                    NativePool::MySql(p) => sqlx::query(sql)
                        .execute(p)
                        .await
                        .map_err(|e| e.to_string())?
                        .rows_affected(),
                    NativePool::Sqlite(p) => sqlx::query(sql)
                        .execute(p)
                        .await
                        .map_err(|e| e.to_string())?
                        .rows_affected(),
                };
                if is_ddl_keyword(sql) {
                    QueryOutcome::Ddl
                } else {
                    QueryOutcome::Affected(affected)
                }
            }
        };
        close_native(pool).await;
        Ok::<_, String>(outcome)
    };
    match tokio::time::timeout(std::time::Duration::from_secs(30), work).await {
        Ok(r) => r,
        Err(_) => Err("查询超时(30秒)".to_string()),
    }
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
            uri: None,
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

    #[test]
    fn parse_uri_extracts_password_and_redacts() {
        let p =
            parse_connection_uri("postgresql://alice:SuperSecret@db.internal:5433/shop").unwrap();
        assert_eq!(p.host.as_deref(), Some("db.internal"));
        assert_eq!(p.port, Some(5433));
        assert_eq!(p.database.as_deref(), Some("shop"));
        assert_eq!(p.username.as_deref(), Some("alice"));
        assert_eq!(p.password.as_deref(), Some("SuperSecret"));
        // 脱敏:密码已抽走,不留在 uri 里。
        assert!(!p.uri.contains("SuperSecret"));
        assert!(p.uri.contains("@db.internal:5433/shop"));
    }

    #[test]
    fn parse_uri_mysql_without_password() {
        let p = parse_connection_uri("mysql://root@db/shop").unwrap();
        assert_eq!(p.host.as_deref(), Some("db"));
        assert_eq!(p.database.as_deref(), Some("shop"));
        assert_eq!(p.username.as_deref(), Some("root"));
        assert!(p.password.is_none());
        assert_eq!(p.uri, "mysql://root@db/shop");
    }

    #[test]
    fn parse_uri_rejects_non_db_schemes() {
        assert!(parse_connection_uri("https://db/shop").is_none());
        assert!(parse_connection_uri("not-a-uri").is_none());
    }

    #[test]
    fn uri_takes_precedence_in_build_sql_url() {
        let mut s = base(DriverKind::Postgres);
        s.uri = Some("postgresql://alice@db:5433/shop".into());
        // 无 keychain 密码:直接抄 URI。
        assert_eq!(build_sql_url(&s, None), "postgresql://alice@db:5433/shop");
        // 有 keychain 密码且 URI 无密码:补回。
        assert_eq!(
            build_sql_url(&s, Some("secret")),
            "postgresql://alice:secret@db:5433/shop"
        );
    }

    #[test]
    fn inject_password_keeps_existing_uri_password() {
        assert_eq!(
            inject_password_into_uri("postgresql://alice:pw@db/shop", Some("other")),
            "postgresql://alice:pw@db/shop"
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
            uri: None,
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
        let mut st = SchemaState {
            tables: vec![
                tree_table(None, "users", false),
                tree_table(None, "orders", true),
            ],
            ..Default::default()
        };
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
        let mut st = SchemaState {
            tables: vec![
                tree_table(Some("public"), "users", false),
                tree_table(Some("audit"), "events", false),
            ],
            ..Default::default()
        };
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
        let mut st = SchemaState {
            tables: vec![tree_table(None, "a", false), tree_table(None, "b", false)],
            ..Default::default()
        };
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
            uri: None,
        }
    }

    fn seeded_ws(source: DataSource) -> WorkspaceState {
        let mut ws = WorkspaceState {
            expanded_sources: HashSet::from([source.id.clone()]),
            ..Default::default()
        };
        ws.sources.push(source.clone());
        ws.schemas.insert(source.id.clone(), SchemaState::default());
        ws
    }

    #[test]
    fn browse_schema_first_time_starts_loading() {
        let src = pg_source("s1");
        let mut ws = WorkspaceState::default();
        ws.sources.push(src.clone());
        update_with(&mut ws, Message::ToggleSourceExpanded("s1".into()));
        assert!(ws.is_expanded("s1"));
        let st = ws.schema_state("s1").unwrap_or_else(|| panic!());
        assert!(st.loading_tables());
        assert!(st.tables().is_empty());
        assert!(st.tables_error().is_none());
    }

    #[test]
    fn browse_schema_second_time_uses_cache() {
        let mut ws = seeded_ws(pg_source("s1"));
        ws.schemas.get_mut("s1").unwrap().tables = vec![tree_table(Some("public"), "users", false)];
        // 模拟已加载完成:收起再展开
        update_with(&mut ws, Message::ToggleSourceExpanded("s1".into()));
        update_with(&mut ws, Message::ToggleSourceExpanded("s1".into()));
        let st = ws.schema_state("s1").unwrap();
        assert!(!st.loading_tables()); // 有缓存,不重拉
        assert_eq!(st.tables().len(), 1);
    }

    #[test]
    fn schema_back_clears_browsing_keeps_cache() {
        let mut ws = seeded_ws(pg_source("s1"));
        ws.schemas.get_mut("s1").unwrap().tables = vec![tree_table(Some("public"), "users", false)];
        update_with(&mut ws, Message::ToggleSourceExpanded("s1".into()));
        assert!(!ws.is_expanded("s1"));
        assert_eq!(ws.schema_state("s1").unwrap().tables().len(), 1);
    }

    #[test]
    fn schema_refresh_requires_browsing_match() {
        let mut ws = seeded_ws(pg_source("s1"));
        update_with(&mut ws, Message::ToggleSourceExpanded("s1".into())); // 收起
        update_with(&mut ws, Message::SchemaRefresh("s1".into()));
        assert!(!ws.schema_state("s1").unwrap().loading_tables()); // 旧菜单残留防线
    }

    #[test]
    fn toggle_schema_flips_expansion() {
        let mut ws = seeded_ws(pg_source("s1"));
        let st = ws.schemas.get_mut("s1").unwrap();
        st.tables = vec![tree_table(Some("public"), "users", false)];
        update_with(&mut ws, Message::ToggleSchema("s1".into(), "public".into()));
        assert!(
            tree_rows(ws.schema_state("s1").unwrap(), DriverKind::Postgres)
                .iter()
                .any(|r| matches!(r.kind, SchemaRowKind::Table(_)))
        );
        update_with(&mut ws, Message::ToggleSchema("s1".into(), "public".into()));
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
        update_with(&mut ws, Message::ToggleSchema("s1".into(), "public".into()));
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
        update_with(&mut ws, Message::ToggleSchema("s1".into(), "public".into()));
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
        assert!(!ws.is_expanded("s1"));

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
            uri: String::new(),
            password: String::new(), // 留空 → 跳过 Keychain,不写真实密码
        });
        update_with(&mut ws, Message::DraftSave);
        assert!(ws.schema_state("s1").is_none());
        assert!(!ws.is_expanded("s1"));
    }

    #[test]
    fn draft_save_legal_uri_redacts_and_backfills_fields() {
        // 无密码 URI:不触 Keychain(本仓库测试不写真实钥匙串)。
        let draft = DataSourceDraft {
            id: None, // 新建
            name: "cloud".into(),
            driver: DriverKind::Postgres,
            host: "wrong-field-host".into(), // 字段填错,应为 URI 覆盖
            port: String::new(),
            database: String::new(),
            username: String::new(),
            uri: "postgresql://alice@db:5433/shop".into(),
            password: String::new(),
        };
        let mut ws = WorkspaceState {
            editing: Some(draft),
            ..Default::default()
        };
        update_with(&mut ws, Message::DraftSave);
        let src = ws.sources.iter().find(|s| s.name == "cloud").unwrap();
        assert_eq!(src.uri.as_deref(), Some("postgresql://alice@db:5433/shop"));
        assert_eq!(src.host.as_deref(), Some("db")); // URI 优先,覆盖错误字段
        assert_eq!(src.port, Some(5433));
        assert_eq!(src.database.as_deref(), Some("shop"));
        assert_eq!(src.username.as_deref(), Some("alice"));
        // 无密码 → 不产生 Keychain 写入,uri 无明文密码(来源本就没有)。
        assert_eq!(src.uri.as_deref(), Some("postgresql://alice@db:5433/shop"));
    }

    #[test]
    fn draft_save_invalid_uri_falls_back_to_fields() {
        let draft = DataSourceDraft {
            id: None,
            name: "legacy".into(),
            driver: DriverKind::Postgres,
            host: "plain.example".into(),
            port: "6543".into(),
            database: "mydb".into(),
            username: "user".into(),
            uri: "https://not-a-db-scheme".into(), // 非法 → 退化为字段式
            password: String::new(),
        };
        let mut ws = WorkspaceState {
            editing: Some(draft),
            ..Default::default()
        };
        update_with(&mut ws, Message::DraftSave);
        let src = ws.sources.iter().find(|s| s.name == "legacy").unwrap();
        assert_eq!(src.uri, None); // 非法 URI 不落盘
        assert_eq!(src.host.as_deref(), Some("plain.example"));
        assert_eq!(src.port, Some(6543));
        assert_eq!(src.database.as_deref(), Some("mydb"));
        assert_eq!(src.username.as_deref(), Some("user"));
    }
}

#[cfg(test)]
mod sqlite_introspection {
    use super::*;
    use std::sync::Once;

    static INSTALL: Once = Once::new();

    fn install_drivers() {
        INSTALL.call_once(sqlx::any::install_default_drivers);
    }

    async fn setup_db() -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        // mode=rwc:文件不存在时创建(默认 create_if_missing=false,新库必须显式造文件)
        let url = format!("sqlite://{}/phase2.sqlite?mode=rwc", dir.path().display());
        let pool = sqlx::AnyPool::connect(&url).await.unwrap();
        sqlx::query("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("CREATE VIEW v_users AS SELECT id FROM users")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
        (dir, url)
    }

    #[tokio::test]
    async fn load_tables_lists_tables_and_views_in_order() {
        install_drivers();
        let (_dir, url) = setup_db().await;
        let tables = load_tables(DriverKind::Sqlite, &url, None).await.unwrap();
        assert_eq!(tables.len(), 2);
        // SQLite: name ORDER BY → users 在 v_users 前
        assert_eq!(tables[0].name, "users");
        assert!(!tables[0].is_view);
        assert_eq!(tables[1].name, "v_users");
        assert!(tables[1].is_view);
        assert!(tables.iter().all(|t| t.schema.is_none()));
    }

    #[tokio::test]
    async fn load_columns_reports_name_type_nullability() {
        install_drivers();
        let (_dir, url) = setup_db().await;
        let cols = load_columns(DriverKind::Sqlite, &url, None, "users")
            .await
            .unwrap();
        assert_eq!(cols.len(), 2);
        // pragma_table_info 按 cid 序:id 在前
        assert_eq!(cols[0].name, "id");
        let name = &cols[1];
        assert_eq!(name.name, "name");
        assert_eq!(name.type_name.to_uppercase(), "TEXT");
        assert!(!name.nullable); // NOT NULL 列
    }

    #[tokio::test]
    async fn load_columns_missing_table_is_error() {
        install_drivers();
        let (_dir, url) = setup_db().await;
        let err = load_columns(DriverKind::Sqlite, &url, None, "nope")
            .await
            .unwrap_err();
        assert!(!err.is_empty());
    }

    #[tokio::test]
    async fn mongodb_loaders_refuse_with_friendly_error() {
        // 缺数据库名 → 友好文案,不 panic/不尝试连接
        let err = load_tables(DriverKind::MongoDB, "mongodb://x", None)
            .await
            .unwrap_err();
        assert!(err.contains("数据库名"));
        let err = load_columns(DriverKind::MongoDB, "mongodb://x", None, "c")
            .await
            .unwrap_err();
        assert!(err.contains("MongoDB"));
    }
}

#[cfg(test)]
mod content_tests {
    use super::*;

    #[test]
    fn open_table_dedups_and_focuses_existing() {
        let mut st = DatabaseContentState::default();
        let id1 = st.open_table("s1".into(), Some("public".into()), "users".into());
        st.open_table("s1".into(), Some("public".into()), "orders".into());
        assert_eq!(st.tabs().len(), 2);
        let id_again = st.open_table("s1".into(), Some("public".into()), "users".into());
        assert_eq!(id1, id_again);
        assert_eq!(st.tabs().len(), 2); // 没有新开
        assert_eq!(st.active_idx(), Some(0)); // 聚焦回第一个 tab
    }

    #[test]
    fn open_table_reopen_preserves_content() {
        let mut st = DatabaseContentState::default();
        let id = st.open_table("s1".into(), None, "users".into());
        if let Some(TabContent::Browse(b)) = st.content_mut(id) {
            b.where_clause = "id > 10".into();
        }
        st.open_collection("s1".into(), "other".into()); // 切走
        st.open_table("s1".into(), None, "users".into()); // 再开同一张表
        let TabContent::Browse(b) = st.content(id).unwrap() else {
            panic!("应为 Browse");
        };
        assert_eq!(b.where_clause, "id > 10"); // 内容没被重置
    }

    #[test]
    fn open_collection_dedups() {
        let mut st = DatabaseContentState::default();
        let id1 = st.open_collection("s1".into(), "logs".into());
        let id2 = st.open_collection("s1".into(), "logs".into());
        assert_eq!(id1, id2);
        assert_eq!(st.tabs().len(), 1);
    }

    #[test]
    fn open_query_never_dedups_and_increments_seq() {
        let mut st = DatabaseContentState::default();
        st.open_query("s1".into());
        st.open_query("s1".into());
        assert_eq!(st.tabs().len(), 2);
        let seqs: Vec<u32> = st
            .tabs()
            .iter()
            .map(|t| match &t.kind {
                DatabaseTabKind::Query { console_seq, .. } => *console_seq,
                _ => panic!("应为 Query"),
            })
            .collect();
        assert_eq!(seqs, vec![1, 2]);
    }

    #[test]
    fn close_before_active_shifts_active_index_down() {
        let mut st = DatabaseContentState::default();
        st.open_table("s1".into(), None, "a".into());
        st.open_table("s1".into(), None, "b".into());
        st.select(1); // active = b(索引1)
        st.close(0); // 关掉 a(在 active 之前)
        assert_eq!(st.tabs().len(), 1);
        assert_eq!(st.active_idx(), Some(0)); // 仍指向 b,现在挪到索引0
        assert_eq!(
            st.tabs()[0].kind,
            DatabaseTabKind::Table {
                source_id: "s1".into(),
                schema: None,
                table: "b".into()
            }
        );
    }

    #[test]
    fn close_active_last_tab_shrinks_active() {
        let mut st = DatabaseContentState::default();
        st.open_table("s1".into(), None, "a".into());
        st.open_table("s1".into(), None, "b".into());
        // active 目前是索引1(b,刚开的)
        st.close(1);
        assert_eq!(st.tabs().len(), 1);
        assert_eq!(st.active_idx(), Some(0));
    }

    #[test]
    fn close_only_tab_empties_state() {
        let mut st = DatabaseContentState::default();
        st.open_table("s1".into(), None, "a".into());
        st.close(0);
        assert!(st.tabs().is_empty());
        assert!(st.active_tab().is_none());
    }

    #[test]
    fn close_by_source_removes_all_kinds_for_that_source() {
        let mut st = DatabaseContentState::default();
        st.open_table("s1".into(), None, "a".into());
        st.open_collection("s1".into(), "c".into());
        st.open_query("s1".into());
        st.open_table("s2".into(), None, "keep".into());
        st.close_by_source("s1");
        assert_eq!(st.tabs().len(), 1);
        assert_eq!(
            st.tabs()[0].kind,
            DatabaseTabKind::Table {
                source_id: "s2".into(),
                schema: None,
                table: "keep".into()
            }
        );
    }
}

#[cfg(test)]
mod query_gen_tests {
    use super::*;

    #[test]
    fn quote_table_postgres_with_schema() {
        assert_eq!(
            quote_table(DriverKind::Postgres, Some("public"), "users"),
            "\"public\".\"users\""
        );
    }

    #[test]
    fn quote_table_mysql_and_sqlite_use_backticks() {
        assert_eq!(quote_table(DriverKind::MySQL, None, "users"), "`users`");
        assert_eq!(quote_table(DriverKind::Sqlite, None, "users"), "`users`");
    }

    #[test]
    fn build_browse_sql_omits_empty_where_and_order_by() {
        let sql = build_browse_sql(DriverKind::Sqlite, None, "users", "", "", 0, 50);
        assert_eq!(sql, "SELECT * FROM `users` LIMIT 51 OFFSET 0");
    }

    #[test]
    fn build_browse_sql_includes_where_and_order_by_and_pages() {
        let sql = build_browse_sql(
            DriverKind::Postgres,
            Some("public"),
            "users",
            "id > 10",
            "id DESC",
            2,
            50,
        );
        assert_eq!(
            sql,
            "SELECT * FROM \"public\".\"users\" WHERE id > 10 ORDER BY id DESC LIMIT 51 OFFSET 100"
        );
    }

    #[test]
    fn classify_statement_recognizes_row_producing_keywords() {
        for sql in [
            "select 1",
            "  SELECT * FROM t",
            "with x as (select 1) select * from x",
            "SHOW TABLES",
            "explain select 1",
            "pragma table_info(t)",
        ] {
            assert_eq!(classify_statement(sql), StatementKind::Rows, "sql={sql}");
        }
    }

    #[test]
    fn classify_statement_treats_dml_ddl_as_execute() {
        for sql in [
            "insert into t values (1)",
            "UPDATE t SET x=1",
            "delete from t",
            "CREATE TABLE t (id int)",
        ] {
            assert_eq!(classify_statement(sql), StatementKind::Execute, "sql={sql}");
        }
    }

    #[test]
    fn is_ddl_keyword_matches_only_ddl() {
        assert!(is_ddl_keyword("CREATE TABLE t (id int)"));
        assert!(is_ddl_keyword("  drop table t"));
        assert!(!is_ddl_keyword("insert into t values (1)"));
        assert!(!is_ddl_keyword("update t set x=1"));
    }
}

#[cfg(test)]
mod sqlite_browse_and_query {
    use super::*;

    async fn setup_db_with_rows() -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}/browse.sqlite?mode=rwc", dir.path().display());
        let pool = sqlx::SqlitePool::connect(&url).await.unwrap();
        sqlx::query("CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT NOT NULL, note TEXT)")
            .execute(&pool)
            .await
            .unwrap();
        for i in 1..=5 {
            sqlx::query("INSERT INTO items (id, name, note) VALUES (?, ?, ?)")
                .bind(i)
                .bind(format!("item-{i}"))
                .bind(if i == 3 {
                    None::<String>
                } else {
                    Some(format!("note-{i}"))
                })
                .execute(&pool)
                .await
                .unwrap();
        }
        pool.close().await;
        (dir, url)
    }

    #[tokio::test]
    async fn browse_table_paginates_and_reports_has_more() {
        let (_dir, url) = setup_db_with_rows().await;
        let page0 = browse_table(BrowseTableParams {
            kind: DriverKind::Sqlite,
            url: &url,
            schema: None,
            table: "items",
            where_clause: "",
            order_by: "id ASC",
            page: 0,
            page_size: 2,
        })
        .await
        .unwrap();
        assert_eq!(page0.result.columns, vec!["id", "name", "note"]);
        assert_eq!(page0.result.rows.len(), 2);
        assert!(page0.has_more);

        let page2 = browse_table(BrowseTableParams {
            kind: DriverKind::Sqlite,
            url: &url,
            schema: None,
            table: "items",
            where_clause: "",
            order_by: "id ASC",
            page: 2,
            page_size: 2,
        })
        .await
        .unwrap();
        assert_eq!(page2.result.rows.len(), 1); // 第5条,最后一页
        assert!(!page2.has_more);
    }

    #[tokio::test]
    async fn browse_table_where_clause_filters_and_null_renders_as_null() {
        let (_dir, url) = setup_db_with_rows().await;
        let page = browse_table(BrowseTableParams {
            kind: DriverKind::Sqlite,
            url: &url,
            schema: None,
            table: "items",
            where_clause: "id = 3",
            order_by: "",
            page: 0,
            page_size: 50,
        })
        .await
        .unwrap();
        assert_eq!(page.result.rows.len(), 1);
        let note_idx = page
            .result
            .columns
            .iter()
            .position(|c| c == "note")
            .unwrap();
        assert_eq!(page.result.rows[0][note_idx], CellValue::Null);
    }

    #[tokio::test]
    async fn run_query_select_returns_rows() {
        let (_dir, url) = setup_db_with_rows().await;
        let outcome = run_query(
            DriverKind::Sqlite,
            &url,
            "SELECT id, name FROM items WHERE id <= 2 ORDER BY id",
        )
        .await
        .unwrap();
        let QueryOutcome::Rows(result) = outcome else {
            panic!("应为 Rows");
        };
        assert_eq!(result.rows.len(), 2);
        assert_eq!(result.columns, vec!["id", "name"]);
    }

    #[tokio::test]
    async fn run_query_update_returns_affected_count() {
        let (_dir, url) = setup_db_with_rows().await;
        let outcome = run_query(
            DriverKind::Sqlite,
            &url,
            "UPDATE items SET note = 'x' WHERE id <= 2",
        )
        .await
        .unwrap();
        assert!(matches!(outcome, QueryOutcome::Affected(2)));
    }

    #[tokio::test]
    async fn run_query_delete_returns_affected_count() {
        let (_dir, url) = setup_db_with_rows().await;
        let outcome = run_query(DriverKind::Sqlite, &url, "DELETE FROM items WHERE id >= 4")
            .await
            .unwrap();
        assert!(matches!(outcome, QueryOutcome::Affected(2)));
    }

    #[tokio::test]
    async fn browse_real_and_datetime_values_render_as_text() {
        let dir = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}/types.sqlite?mode=rwc", dir.path().display());
        let pool = sqlx::SqlitePool::connect(&url).await.unwrap();
        sqlx::query("CREATE TABLE t (score REAL, seen DATETIME, note TEXT)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO t (score, seen, note) VALUES (3.25, '2026-01-02 03:04:05', 'x')")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;

        let page = browse_table(BrowseTableParams {
            kind: DriverKind::Sqlite,
            url: &url,
            schema: None,
            table: "t",
            where_clause: "",
            order_by: "",
            page: 0,
            page_size: 50,
        })
        .await
        .unwrap();
        // 真实数值/时间不是"不支持的类型":SQLite 路径统一被 stringify 成文本。
        // 行序尽力按数值/文本值摆列,这里只校验两个非 NULL 单元格写出了
        // 非空、非降级标记的文本(具体格式交给 sqlite 的 ToSql/stringify)。
        for cell in &page.result.rows[0] {
            match cell {
                CellValue::Text(s) => assert!(!s.is_empty()),
                CellValue::Null => panic!("行里所有列都插了值,不应有 NULL"),
            }
        }
        assert_eq!(page.result.rows[0].len(), 3);
    }

    #[tokio::test]
    async fn run_query_create_table_returns_ddl_outcome() {
        let (_dir, url) = setup_db_with_rows().await;
        let outcome = run_query(DriverKind::Sqlite, &url, "CREATE TABLE extra (id INTEGER)")
            .await
            .unwrap();
        assert!(matches!(outcome, QueryOutcome::Ddl));
    }

    #[tokio::test]
    async fn run_query_syntax_error_is_reported() {
        let (_dir, url) = setup_db_with_rows().await;
        let err = run_query(DriverKind::Sqlite, &url, "SELEKT * FROM items")
            .await
            .unwrap_err();
        assert!(!err.is_empty());
    }
}

#[cfg(test)]
mod mongo_collection_tests {
    use super::*;

    #[tokio::test]
    async fn load_tables_mongo_without_db_name_gives_friendly_error() {
        let err = load_tables(DriverKind::MongoDB, "mongodb://x", None)
            .await
            .unwrap_err();
        assert!(err.contains("数据库名"));
    }

    #[tokio::test]
    async fn load_tables_mongo_with_empty_db_name_also_errors() {
        let err = load_tables(DriverKind::MongoDB, "mongodb://x", Some(""))
            .await
            .unwrap_err();
        assert!(err.contains("数据库名"));
    }
}

#[cfg(test)]
mod content_message_tests {
    use super::*;

    fn sqlite_source(id: &str) -> DataSource {
        DataSource {
            id: id.into(),
            name: format!("test-{id}"),
            driver: DriverKind::Sqlite,
            host: None,
            port: None,
            database: Some("/nonexistent/does-not-matter.sqlite".into()),
            username: None,
            uri: None,
        }
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

    #[test]
    fn open_table_tab_message_creates_tab() {
        let mut ws = WorkspaceState::default();
        update_with(
            &mut ws,
            Message::OpenTableTab {
                source_id: "s1".into(),
                schema: None,
                table: "users".into(),
            },
        );
        assert_eq!(ws.content().tabs().len(), 1);
    }

    #[test]
    fn browse_where_changed_updates_state() {
        let mut ws = WorkspaceState::default();
        update_with(
            &mut ws,
            Message::OpenTableTab {
                source_id: "s1".into(),
                schema: None,
                table: "users".into(),
            },
        );
        let tab_id = ws.content().tabs()[0].id;
        update_with(
            &mut ws,
            Message::BrowseWhereChanged(tab_id, "id > 1".into()),
        );
        let TabContent::Browse(b) = ws.content().content(tab_id).unwrap() else {
            panic!("应为 Browse");
        };
        assert_eq!(b.where_clause, "id > 1");
    }

    #[test]
    fn close_tab_message_removes_it() {
        let mut ws = WorkspaceState::default();
        update_with(
            &mut ws,
            Message::OpenTableTab {
                source_id: "s1".into(),
                schema: None,
                table: "users".into(),
            },
        );
        update_with(&mut ws, Message::CloseTab(0));
        assert!(ws.content().tabs().is_empty());
    }

    #[test]
    fn delete_source_closes_its_tabs() {
        let mut ws = WorkspaceState::default();
        ws.sources.push(sqlite_source("s1"));
        update_with(
            &mut ws,
            Message::OpenTableTab {
                source_id: "s1".into(),
                schema: None,
                table: "users".into(),
            },
        );
        assert_eq!(ws.content().tabs().len(), 1);
        update_with(&mut ws, Message::DeleteSource("s1".into()));
        assert!(ws.content().tabs().is_empty());
    }

    #[test]
    fn apply_browse_result_ok_and_err_paths() {
        let mut b = BrowseState::default();
        b.begin_run();
        apply_browse_result(
            &mut b,
            Ok(BrowsePage {
                result: QueryResult {
                    columns: vec!["id".into()],
                    rows: vec![vec![CellValue::Text("1".into())]],
                },
                has_more: true,
            }),
        );
        assert!(!b.loading);
        assert!(b.has_more);
        assert_eq!(b.result.as_ref().unwrap().rows.len(), 1);

        let mut b2 = BrowseState::default();
        b2.begin_run();
        apply_browse_result(&mut b2, Err("boom".into()));
        assert_eq!(b2.error.as_deref(), Some("boom"));
    }

    /// 过期的浏览结果(seq 不匹配当前轮)在 `update()` 的 `BrowseResult` 臂里
    /// 被丢弃,不落地——对应人工验收里"连续快速改 WHERE 各回车一次,最终停
    /// 在最后一次结果"的竞态防线(`run_seq`)。
    #[test]
    fn browse_result_stale_or_unloading_seq_is_dropped() {
        let mut ws = WorkspaceState::default();
        let tab = Message::OpenTableTab {
            source_id: "s1".into(),
            schema: None,
            table: "users".into(),
        };
        update_with(&mut ws, tab);
        let tab_id = ws.content().tabs()[0].id;
        // tab 打开后还没有任何 run:`loading=false`,seq 进来对不上 → 弃。
        let page = BrowsePage {
            result: QueryResult {
                columns: vec!["id".into()],
                rows: vec![vec![CellValue::Text("1".into())]],
            },
            has_more: false,
        };
        update_with(&mut ws, Message::BrowseResult(42, tab_id, 7, Ok(page)));
        let TabContent::Browse(b) = ws.content().content(tab_id).unwrap() else {
            panic!("应为 Browse");
        };
        assert!(b.result.is_none(), "过期/未运行的结果不应落地");
        assert!(!b.loading);
    }
}
