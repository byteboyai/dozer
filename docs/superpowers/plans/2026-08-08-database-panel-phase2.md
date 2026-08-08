# 数据库面板 · 阶段 2(schema 树浏览)Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 给阶段 1 已落地的数据库面板加 **schema 树浏览**:数据源卡片上"浏览结构" → 面板切换为该源的表/视图树(Postgres 多一层 schema 节点),展开表逐表懒加载列(名字/类型/可空性),顶部"← 返回"回卡片列表、"刷新"重拉并对账,加载失败有错误态与重试路径。设计口径唯一真相源:`docs/superpowers/specs/2026-08-08-database-panel-phase2-design.md`(下文引用"设计文档"即此文件)。数据网格浏览/SQL 执行/MongoDB 集合浏览不在本阶段(分别是阶段 3/4/5)。

**Architecture:** 全部改动集中在 `crates/dozer-app/src/extensions/database.rs`(追加类型/消息/状态机/视图,不另拆文件——延续阶段 1 口径)+ `crates/dozer-app/src/workspace.rs` 的 `App::update` 里加**两个**异步结果特化 match 臂(结构逐行照抄现有 `TestConnectionResult` 臂,排在通配 `Message::Database(msg)` 之前)。`view` 函数签名、`LeftView`、`App`/`Workspace` struct 字段、`left_panel_area` 分发**全部不动**;`view` 内部按 `browsing` 自行分发卡片列表/schema 树。新增状态(`browsing`/`schemas`)全挂 `WorkspaceState`(per-project),**纯内存不持久化**。

**Tech Stack:** Rust;sqlx 0.8(`Any` 驱动,阶段 1 已引入 `any`/`postgres`/`mysql`/`sqlite` features + `runtime-tokio`)。**零新增依赖**,只新增两个 Lucide SVG 图标(`table`/`eye`)。已核实的关键事实(设计文档"背景"节):`Any` 驱动**不重写占位符**(PG 必须 `$1`/`$2`,MySQL/SQLite 用 `?`);`Any` 只解码 9 种基础类型(introspection SQL 只投影 TEXT/INTEGER);表清单查询可全部免绑定参数,列查询按驱动分三套 SQL。

## Global Constraints

- **分支基线**:从 `main` 当前 tip(`62713a3`,已含阶段 1 合并点 `d8fcecb` 与 SSH 面板合并)切 `feature/database-panel-phase2`。完成后提请审阅合并。
- **并行分支**:`feature/app-workspace-split`(App/Workspace 文件拆分)尚未合并;若实现期间它先合并,`App::update`/`loaded_workspace_mut` 等符号会挪进新文件——**按符号名 grep 定位,不要假设行号/文件位置**。本阶段在 `App::update` 里只追加两个特化臂,与该分支是"相邻位置各改各的"级别冲突。
- **本阶段对数据库只读**:introspection 只有 `SELECT`(`information_schema`/`sqlite_master`/`pragma_table_info`),不写任何 DDL/DML(集成测试建表只发生在 tempfile 临时库)。连接串复用阶段 1 `build_sql_url(source, password)`;密码只经 `keyring::Entry`(service `"dozer"`,key `{project_id}:{source_id}`)存取,不进日志/持久化,测试不写真实密码(草稿密码留空即跳过 Keychain)。
- **不做**(设计文档"非目标"):数据行浏览(阶段 3)、SQL 执行(阶段 4)、MongoDB 集合浏览(阶段 5)、表名搜索框、行数统计、主键/索引/默认值/注释、列类型归一化、池化复用、展开状态/树内容持久化。都不要顺手加。
- 每个任务结束:`cargo build -p dozer-app`、`cargo test -p dozer-app`、`cargo clippy -p dozer-app --all-targets`、`cargo fmt --check` 全绿。Task 1-3 结束时新符号尚未被视图调用,会出现 `dead_code`/unused 警告——**预期过渡态,不要加 `#[allow(dead_code)]`**,Task 4 接入后自然清零。
- 每个任务一个 commit;全部完成后跑 Task 6 的人工验收清单再提请合并。

---

### Task 1: 图标资源(table / eye)

**Files:**
- Create: `crates/dozer-app/assets/icons/table.svg`
- Create: `crates/dozer-app/assets/icons/eye.svg`
- Modify: `crates/dozer-app/src/icons.rs`

**Interfaces:**
- Produces:`icons::IconKind::Table` / `icons::IconKind::Eye`(Task 4 树视图消费)。

- [ ] **Step 1: 新增两个 Lucide 图标资源**

创建 `crates/dozer-app/assets/icons/table.svg`(Lucide `table` 图标,去掉原文件的 license 注释与 `class` 属性,属性格式对齐阶段 1 的 `database.svg`):

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
  <path d="M12 3v18" />
  <rect width="18" height="18" x="3" y="3" rx="2" />
  <path d="M3 9h18" />
  <path d="M3 15h18" />
</svg>
```

创建 `crates/dozer-app/assets/icons/eye.svg`(Lucide `eye`,同口径):

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
  <path d="M2 12s3-7 10-7 10 7 10 7-3 7-10 7-10-7-10-7Z" />
  <circle cx="12" cy="12" r="3" />
</svg>
```

- [ ] **Step 2: `IconKind` 加两个变体**

`crates/dozer-app/src/icons.rs`:`IconKind` 枚举在阶段 1 加的 `Database` 变体后追加:

```rust
    /// schema 树表节点图标(Lucide table)。
    Table,
    /// schema 树视图节点图标(Lucide eye)。
    Eye,
```

`bytes()` 的 `match` 里 `IconKind::Database => ...` 臂后追加:

```rust
            IconKind::Table => include_bytes!("../assets/icons/table.svg"),
            IconKind::Eye => include_bytes!("../assets/icons/eye.svg"),
```

- [ ] **Step 3: 编译验证**

```bash
cargo build -p dozer-app
cargo clippy -p dozer-app --all-targets
cargo fmt --check
```

预期:`Table`/`Eye` 无调用点,dead_code 警告属预期(Task 4 清掉)。

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/assets/icons/table.svg crates/dozer-app/assets/icons/eye.svg crates/dozer-app/src/icons.rs
git commit -m "feat(dozer-app): add table and eye icons for database schema tree"
```

---

### Task 2: 数据模型 + `tree_rows` 摊平 + 状态字段

**Files:**
- Modify: `crates/dozer-app/src/extensions/database.rs`

**Interfaces:**
- Produces(Task 3/4 消费):
  - `pub struct TableRef { pub schema: Option<String>, pub name: String, pub is_view: bool }`
  - `pub struct ColumnInfo { pub name: String, pub type_name: String, pub nullable: bool }`
  - `pub enum ColumnLoad { Loading, Loaded(Vec<ColumnInfo>), Failed(String) }`
  - `pub struct SchemaState`(字段私有 + 只读访问器 `loading_tables()`/`tables_error()`/`tables()`/`column_load(..)`)
  - `pub enum SchemaRowKind<'a>` / `pub struct SchemaRow<'a>` / `pub fn tree_rows(..)`(设计文档 §1 的 API 形状)
  - `WorkspaceState` 新字段 `browsing`/`schemas` + 访问器 `browsing_source()`

- [ ] **Step 1: import 扩充**

`database.rs` 顶部 `use std::collections::HashMap;` 改为:

```rust
use std::collections::{BTreeMap, HashMap, HashSet};
```

(`iced_widget` 行的 `scrollable` 与 `use crate::icons;` 放 Task 4 再加,避免本任务 unused import。)

- [ ] **Step 2: 数据模型类型**

加在 `TestStatus` 定义之后、`fn drivers_path()` 之前(类型形状逐字照设计文档 §1):

```rust
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
```

- [ ] **Step 3: 摊平纯函数 `tree_rows`**

接在 `SchemaState` impl 之后(API 形状照设计文档 §1):

```rust
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
```

- [ ] **Step 4: `WorkspaceState` 加字段与访问器**

`WorkspaceState` struct 改为(阶段 1 三字段后追加两个,doc 注释同步更新):

```rust
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
```

impl 块里 `test_status(..)` 之后加:

```rust
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
```

- [ ] **Step 5: 摊平函数单测**

加在文件尾部现有 `mod tests` 里(该 mod 已 `use super::*`,测试在同模块内可直接构造私有字段):

```rust
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
        st.columns.insert((None, "b".into()), ColumnLoad::Failed("nope".into()));
        let rows = tree_rows(&st, DriverKind::MySQL);
        assert!(rows.iter().any(|r| matches!(r.kind, SchemaRowKind::ColumnsLoading) && r.depth == 1));
        assert!(rows.iter().any(|r| matches!(r.kind, SchemaRowKind::ColumnsFailed(e) if e == "nope")));
    }
```

- [ ] **Step 6: 编译/测试/lint/格式**

```bash
cargo test -p dozer-app database::
cargo build -p dozer-app
cargo clippy -p dozer-app --all-targets
cargo fmt --check
```

预期:新增 3 个摊平测试 PASS;新类型未被视图/状态机引用,dead_code 警告属预期。

- [ ] **Step 7: Commit**

```bash
git add crates/dozer-app/src/extensions/database.rs
git commit -m "feat(dozer-app): add database schema tree data model and flatten"
```

---

### Task 3: 消息、状态机、异步加载、内核接线

**Files:**
- Modify: `crates/dozer-app/src/extensions/database.rs`
- Modify: `crates/dozer-app/src/workspace.rs`

**Interfaces:**
- `database::Message` 新增 6 个变体(名字/形状逐字照设计文档 §2 表格):`BrowseSchema(String)`、`SchemaBack`、`SchemaRefresh(String)`、`ToggleSchema(String)`、`ToggleTable { source_id, schema, table }`、`TablesLoaded(i64, String, Result<Vec<TableRef>, String>)`、`ColumnsLoaded { project_id, source_id, schema, table, result }`(后两个带 `project_id`,理由同阶段 1 `TestConnectionResult`)。
- `database::update` 的 `emit` 参数签名追加 `Clone` 约束(刷新对账时多个重加载任务要各持一份 emit;`workspace.rs` 传入的闭包捕获 `proxy`,天然 `Clone`,调用点**无需改动**)。
- `workspace.rs` `App::update` 新增两个特化 match 臂(结构逐行照抄 `TestConnectionResult` 臂,排在通配臂之前)。
- `DeleteSource` / `DraftSave`(编辑已有源)追加 `schemas` 条目清理与 `browsing` 清除(设计文档 §2 过期防线)。
- Produces(Task 4 消费):`tables_sql(..)`、`columns_sql(..)`、`load_tables(..)`、`load_columns(..)` 及各消息处理分支。

**状态机口径(设计文档 §2,实现时逐条对照):**

| 消息 | 行为 |
|------|------|
| `BrowseSchema(id)` | `browsing = Some(id)`;**仅当该源从未加载过**(表空 ∧ 不在加载 ∧ 无错误)才发起表加载(有缓存/有错误都保留现状,错误态由用户点"重试"触发) |
| `SchemaBack` | `browsing = None`;树状态**保留**(返回再进入不重拉) |
| `SchemaRefresh(id)` | 先核对 `browsing == Some(id)`(防旧视图残留按钮);`loading_tables=true`、`tables_error=None`,**旧表快照保留不闪空**,重拉表列表 |
| `ToggleSchema(name)` | 经 `browsing` 路由到当前源的 `SchemaState`,反转 `expanded_schemas`(纯同步,不触发加载——表列表一次查全) |
| `ToggleTable { source_id, schema, table }` | 先核对 `browsing == Some(source_id)`;反转 `expanded_tables`;收起时保留缓存;展开且列缓存缺失或 `Failed` → 置 `Loading` 并发起列加载 |
| `TablesLoaded(id, src, r)` | 过期防线:`loading_tables != true` → 丢弃。`Err` → 记 `tables_error`,**保留旧快照**。`Ok` → 写表 + 对账(见下) |
| `ColumnsLoaded { .. }` | 过期防线:对应 key 不是 `ColumnLoad::Loading` → 丢弃;否则写结果 |

**`TablesLoaded` Ok 分支的对账(设计文档目标第 7 条)**:清理新表列表里不存在的 `expanded_tables`/`columns` 条目 → Postgres 若全表同属一个 schema 自动展开它 → **仍然展开的表全部重新拉列**(置 `Loading` + spawn),用户无感。

- [ ] **Step 1: `Message` 枚举扩展**

`database.rs` 的 `pub enum Message`,在 `TestConnectionResult` 变体后追加:

```rust
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
```

- [ ] **Step 2: introspection SQL 构造(按驱动分派,只读)**

加在 `tree_rows` 相关代码之后。SQL 文本逐字照设计文档 §3(表清单免绑定参数;列清单 PG 用 `$1`/`$2`、MySQL/SQLite 用 `?`——`Any` 驱动不重写占位符):

```rust
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
        DriverKind::MongoDB => unreachable!("tables_sql 不处理 MongoDB(UI 无入口 + load_tables 双保险)"),
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
```

- [ ] **Step 3: 异步加载器(5 秒超时 + 池用完即关)**

接在 SQL 构造之后。口径:每次加载新建 `AnyPool`、查完 `pool.close().await`(补阶段 1 没关池的欠账,避免 SQLite 文件句柄残留);整体套 5 秒 `tokio::time::timeout`(同阶段 1);MongoDB 直接错误文案(双保险);注意**解码 tuple 形状按驱动分别写**(PG 表清单 3 列带 TEXT table_type;MySQL/SQLite 表清单 2 列;PG/MySQL 列清单 is_nullable 是 TEXT,SQLite notnull 是 INTEGER——`Any` 只解码基础类型,tuple 必须与 SELECT 列一一对应):

```rust
async fn load_tables(kind: DriverKind, url: &str) -> Result<Vec<TableRef>, String> {
    if kind == DriverKind::MongoDB {
        return Err("MongoDB 集合浏览将在后续阶段支持".to_string());
    }
    let work = async {
        let pool = sqlx::AnyPool::connect(url).await.map_err(|e| e.to_string())?;
        let out = async {
            let sql = tables_sql(kind);
            if kind == DriverKind::Postgres {
                let rows: Vec<(String, String, String)> =
                    sqlx::query_as(sql).fetch_all(&pool).await.map_err(|e| e.to_string())?;
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
                let rows: Vec<(String, String)> =
                    sqlx::query_as(sql).fetch_all(&pool).await.map_err(|e| e.to_string())?;
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
        let pool = sqlx::AnyPool::connect(url).await.map_err(|e| e.to_string())?;
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
```

两个 spawn 辅助(emit 按值传入,`handle.spawn` 里 move;超时已在 loader 内,spawn 处不再套):

```rust
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
    ws_state.sources.iter().find(|s| s.id == source_id).map(|s| s.driver)
}
```

- [ ] **Step 4: `update` 签名与六个新消息分支**

`update(..)` 的 `emit` 参数追加 `Clone` 约束(其余签名不动):

```rust
pub fn update(
    ws_state: &mut WorkspaceState,
    app_state: &mut AppState,
    msg: Message,
    project_id: i64,
    repo_path: &Path,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + Clone + 'static,
) {
```

`match msg` 追加六个分支(放在 `TestConnectionResult` 臂后;密码读取口径与 `TestConnection` 相同——Keychain 读不到按无密码处理,不 panic):

```rust
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
            let Some(source) = ws_state
                .sources
                .iter()
                .find(|s| s.id == source_id)
                .cloned()
            else {
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
            let Some(source) = ws_state
                .sources
                .iter()
                .find(|s| s.id == source_id)
                .cloned()
            else {
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
                    let Some(source) = ws_state
                        .sources
                        .iter()
                        .find(|s| s.id == source_id)
                        .cloned()
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
```

- [ ] **Step 5: `DeleteSource` / `DraftSave` 的 schema 状态清理**

`DeleteSource` 臂开头(`ws_state.sources.retain(..)` 之后、`test_status.remove` 旁)追加:

```rust
            ws_state.schemas.remove(&id);
            if ws_state.browsing.as_deref() == Some(id.as_str()) {
                ws_state.browsing = None;
            }
```

`DraftSave` 臂里 `let id = draft.id.clone().unwrap_or_else(..)` 之后追加(编辑已有源才算;连接信息可能变了,旧结构快照不作数——设计文档 §2):

```rust
            if draft.id.is_some() {
                ws_state.schemas.remove(&id);
                if ws_state.browsing.as_deref() == Some(id.as_str()) {
                    ws_state.browsing = None;
                }
            }
```

- [ ] **Step 6: `workspace.rs` — `App::update` 两个特化 match 臂**

在 `App::update` 中现有 `Message::Database(database::Message::TestConnectionResult(..))` 特化臂**之后、**通配 `Message::Database(msg)` 臂**之前**插入两个臂——逐行照抄 `TestConnectionResult` 臂的结构(`loaded_workspace_mut` → `repo_path` → `handle`/`proxy` clone → `emit` 闭包 → `database::update`),只改消息构造(grep `TestConnectionResult` 定位;若期间 app-workspace-split 已合并,按符号名重新定位):

```rust
            // schema 树两个异步结果同 `TestConnectionResult` 口径:自带 project_id,
            // 按自带 id 路由,不能用当前聚焦项目。特化分支必须排在通配
            // `Message::Database(msg)` 之前,否则永远匹配不到。
            Message::Database(database::Message::TablesLoaded(project_id, source_id, result)) => {
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
                    database::Message::TablesLoaded(project_id, source_id, result),
                    project_id,
                    &repo_path,
                    &handle,
                    emit,
                );
            }
            Message::Database(database::Message::ColumnsLoaded {
                project_id,
                source_id,
                schema,
                table,
                result,
            }) => {
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
                    database::Message::ColumnsLoaded {
                        project_id,
                        source_id,
                        schema,
                        table,
                        result,
                    },
                    project_id,
                    &repo_path,
                    &handle,
                    emit,
                );
            }
```

> `emit` 的 `Clone` 约束:`workspace.rs` 传入的闭包只捕获 `proxy`(Clone),自动满足;`Message::Database(msg)` 通配臂与 `TestConnectionResult` 臂无需改动。

- [ ] **Step 7: 状态机单测**

加在 `mod tests` 尾部。测试直接调 `update(..)`(阶段 1 测试只测了 load/save/url,这是第一次真正跑状态机)。辅助:`update_with` 起一个 current_thread runtime,`block_on` 里同步执行 `update`;spawn 出去的加载任务在 `block_on` 返回后不会执行(runtime 随即 drop),不会产生真实连接副作用。**被测源一律用 Postgres 驱动 + 不可达端口**(避免 SQLite 相对路径被后台任务意外建文件):

```rust
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
        assert!(tree_rows(ws.schema_state("s1").unwrap(), DriverKind::Postgres)
            .iter()
            .any(|r| matches!(r.kind, SchemaRowKind::Table(_))));
        update_with(&mut ws, Message::ToggleSchema("public".into()));
        assert!(!tree_rows(ws.schema_state("s1").unwrap(), DriverKind::Postgres)
            .iter()
            .any(|r| matches!(r.kind, SchemaRowKind::Table(_))));
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
        assert!(tree_rows(ws.schema_state("s1").unwrap(), DriverKind::Postgres)
            .iter()
            .any(|r| matches!(r.kind, SchemaRowKind::Column(c) if c.name == "id")));
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
            Message::ToggleTable { source_id: "s1".into(), schema: key.0.clone(), table: key.1.clone() },
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
        assert!(tree_rows(ws.schema_state("s1").unwrap(), DriverKind::Postgres)
            .iter()
            .any(|r| matches!(r.kind, SchemaRowKind::ColumnsFailed("boom"))));
        // 收起再展开 → 回到 Loading(重试语义)
        update_with(
            &mut ws,
            Message::ToggleTable { source_id: "s1".into(), schema: key.0.clone(), table: key.1.clone() },
        );
        update_with(
            &mut ws,
            Message::ToggleTable { source_id: "s1".into(), schema: key.0.clone(), table: key.1.clone() },
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
            st.expanded_tables.insert((Some("public".into()), "users".into()));
            st.columns.insert(
                (Some("public".into()), "users".into()),
                ColumnLoad::Failed("旧错误".into()),
            );
            st.expanded_tables.insert((Some("public".into()), "gone".into()));
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
        assert!(!rows.iter().any(|r| matches!(r.kind, SchemaRowKind::Table(t) if t.name == "gone")));
        // 对账二:单 schema public 自动展开(users 直接可见)
        assert!(rows.iter().any(|r| matches!(r.kind, SchemaRowKind::Table(t) if t.name == "users")));
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
            Message::TablesLoaded(42, "s1".into(), Ok(vec![tree_table(Some("public"), "new", false)])),
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
```

- [ ] **Step 8: 编译/测试/lint/格式**

```bash
cargo test -p dozer-app database::
cargo build -p dozer-app
cargo clippy -p dozer-app --all-targets
cargo fmt --check
```

预期:Task 3 新增 10 个状态机测试全 PASS;`workspace.rs` 两个特化臂编译通过;`BrowseSchema` 等消息尚无生产者,variant dead_code 警告属预期(Task 4 清)。

- [ ] **Step 9: Commit**

```bash
git add crates/dozer-app/src/extensions/database.rs crates/dozer-app/src/workspace.rs
git commit -m "feat(dozer-app): add database schema tree state machine and loaders"
```

---

### Task 4: 视图——卡片入口 + schema 树渲染

**Files:**
- Modify: `crates/dozer-app/src/extensions/database.rs`

**Interfaces:**
- `view(..)` 签名**不变**(`app_state, ws_state, width, outer`),内部按 `browsing_source()` 分发:有 → `schema_tree_view`,无 → 阶段 1 卡片列表原样。
- 新增私有视图函数:`schema_tree_view(..)`、`schema_tree_row(source_id, row)`。
- `source_card` 按钮行加"浏览结构"(排在"测试连接"**之后**,"编辑"/"删除"之前;**非 MongoDB 源才出现**)。
- 视觉口径(设计文档"UI 与视觉"节):树行照搬 `extensions::files` 文件树惯例——`"  ".repeat(depth)` 缩进文本 + chevron + 图标 + 名称,字号 `crate::workspace::tree_row_font_size()`;整树 `scrollable` + `crate::scrollbar::scrollbar()`/`scrollbar_style()`;schema 节点用现有 `Folder`/`FolderOpen`,表用新 `IconKind::Table`,视图用新 `IconKind::Eye`,**列节点不放图标**(等宽 Space 占位对齐,同 files.rs 文件行);列名颜色表达可空性——**非空列 CREAM、可空列 BODY**,不加 "NOT NULL" 文本;主题只用现有色 token,不新增。

- [ ] **Step 1: import 扩充**

`database.rs` 顶部:

```rust
use crate::icons;
use iced_widget::core::{Border, Element, Length};
use iced_widget::{button, column, container, row, scrollable, text, text_input};
```

- [ ] **Step 2: `source_card` 加"浏览结构"按钮**

阶段 1 的按钮行是内联 `row![button(测试连接), button(编辑), button(删除)]`(在 `column![..]` 宏里)。改为先构造可变 `row` 再插入卡片列:把该 `row![..]` 表达式替换为——

```rust
            {
                let mut btns = row![button(text("测试连接"))
                    .on_press(Message::TestConnection(source.id.clone()))]
                .spacing(8);
                if source.driver != DriverKind::MongoDB {
                    // MongoDB 集合浏览是阶段 5;本阶段无入口
                    btns = btns.push(
                        button(text("浏览结构"))
                            .on_press(Message::BrowseSchema(source.id.clone())),
                    );
                }
                btns.push(button(text("编辑")).on_press(Message::EditSourceStart(source.id.clone())))
                    .push(button(text("删除")).on_press(Message::DeleteSource(source.id.clone())))
            },
```

- [ ] **Step 3: `view` 分发**

`view` 函数开头(阶段 1 既有 `let mut col = column![..]` 之前)插入:

```rust
    // 正在浏览某数据源的 schema 树 → 树视图;否则阶段 1 卡片列表(以下原样)。
    if let Some((source, st)) = ws_state.browsing_source() {
        return schema_tree_view(source, st, width, outer);
    }
```

- [ ] **Step 4: `schema_tree_view`**

加在 `view` 函数之后。头部行:`←` 返回图标按钮 + 源名(subtitle/CREAM)+ 驱动标签(caption_sm/DIM)+ 右侧撑满后"刷新"按钮;加载/错误/空态按"有无旧快照"分两路渲染(旧快照在手不闪空,加提示行而非替换整树):

```rust
/// schema 树浏览视图(阶段 2)。drill-down:从卡片列表进入,`SchemaBack` 返回。
fn schema_tree_view<'a>(
    source: &'a DataSource,
    st: &'a SchemaState,
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    let header = row![
        button(icons::view(
            icons::IconKind::ChevronLeft,
            crate::theme::icon_size::row(),
            crate::theme::color::CREAM,
        ))
        .on_press(Message::SchemaBack),
        text(source.name.clone())
            .size(crate::theme::font::subtitle())
            .color(crate::theme::color::CREAM),
        text(source.driver.label())
            .size(crate::theme::font::caption_sm())
            .color(crate::theme::color::DIM),
        iced_widget::space::horizontal(),
        button(text("刷新")).on_press(Message::SchemaRefresh(source.id.clone())),
    ]
    .spacing(8)
    .align_y(iced_widget::core::Alignment::Center);

    let mut col = column![header].spacing(8);

    // 旧快照在手 + 正在刷新 → 一行"刷新中…"提示,旧树照常(同 git_log 不闪空惯例)
    if st.loading_tables() && !st.tables().is_empty() {
        col = col.push(
            text("刷新中…")
                .size(crate::theme::font::caption_sm())
                .color(crate::theme::color::DIM),
        );
    }
    if let Some(e) = st.tables_error() {
        if !st.tables().is_empty() {
            // 有旧快照:树保留,一行红字说明刷新失败
            col = col.push(
                text(format!("刷新失败:{e}"))
                    .size(crate::theme::font::caption_sm())
                    .color(crate::theme::color::RED),
            );
        }
    }

    if st.loading_tables() && st.tables().is_empty() {
        col = col.push(
            text("加载中…")
                .size(crate::theme::font::body())
                .color(crate::theme::color::DIM),
        );
    } else if st.tables().is_empty() {
        if let Some(e) = st.tables_error() {
            col = col.push(
                text(format!("✗ {e}"))
                    .size(crate::theme::font::body())
                    .color(crate::theme::color::RED),
            );
            col = col.push(
                button(text("重试")).on_press(Message::SchemaRefresh(source.id.clone())),
            );
        } else {
            col = col.push(
                text("该库没有表或视图")
                    .size(crate::theme::font::body())
                    .color(crate::theme::color::DIM),
            );
        }
    } else {
        let mut tree = column![].spacing(2);
        for r in tree_rows(st, source.driver) {
            tree = tree.push(schema_tree_row(&source.id, r));
        }
        col = col.push(
            scrollable(tree)
                .direction(scrollable::Direction::Vertical(crate::scrollbar::scrollbar()))
                .style(crate::scrollbar::scrollbar_style()),
        );
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
```

- [ ] **Step 5: `schema_tree_row` 行渲染**

紧随 `schema_tree_view` 之后。缩进/图标/字号/按钮样式**逐项对齐 `extensions::files` 的树行实现**(对照 `files.rs` `view` 里 `row_icon`/`line`/`row_btn` 的写法,含按钮透明背景样式);列节点与 Loading/Failed 占位行用等宽 Space 占住 chevron+图标位:

```rust
/// 单行渲染。source_id 用于构造 `ToggleTable`。
fn schema_tree_row<'a>(
    source_id: &str,
    r: SchemaRow<'a>,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
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
                    icons::view(chevron, crate::theme::icon_size::chevron(), crate::theme::color::DIM),
                    icons::view(folder, crate::theme::icon_size::row(), crate::theme::color::DIM),
                    text(name.to_string())
                        .size(crate::workspace::tree_row_font_size())
                        .color(crate::theme::color::CREAM),
                ]
                .spacing(crate::theme::icon_size::tree_row_gap())
                .align_y(iced_widget::core::Alignment::Center),
            )
            .on_press(Message::ToggleSchema(name.to_string()))
            .width(Length::Fill)
            .into()
        }
        SchemaRowKind::Table(t) => {
            let chevron = if r.expanded {
                icons::IconKind::ChevronDown
            } else {
                icons::IconKind::ChevronRight
            };
            let icon = if t.is_view {
                icons::IconKind::Eye
            } else {
                icons::IconKind::Table
            };
            button(
                row![
                    indent,
                    icons::view(chevron, crate::theme::icon_size::chevron(), crate::theme::color::DIM),
                    icons::view(icon, crate::theme::icon_size::row(), crate::theme::color::DIM),
                    text(t.name.clone())
                        .size(crate::workspace::tree_row_font_size())
                        .color(crate::theme::color::CREAM),
                ]
                .spacing(crate::theme::icon_size::tree_row_gap())
                .align_y(iced_widget::core::Alignment::Center),
            )
            .on_press(Message::ToggleTable {
                source_id: source_id.to_string(),
                schema: t.schema.clone(),
                table: t.name.clone(),
            })
            .width(Length::Fill)
            .into()
        }
        SchemaRowKind::Column(c) => {
            // 可空性用颜色深浅表达:非空 CREAM、可空 BODY(不加 "NOT NULL" 文本)
            let name_color = if c.nullable {
                crate::theme::color::BODY
            } else {
                crate::theme::color::CREAM
            };
            row![
                indent,
                iced_widget::space::Space::new()
                    .width(Length::Fixed(
                        crate::theme::icon_size::chevron()
                            + crate::theme::icon_size::tree_row_gap()
                            + crate::theme::icon_size::row()
                            + crate::theme::icon_size::tree_row_gap(),
                    ))
                    .height(Length::Shrink),
                text(c.name.clone())
                    .size(crate::workspace::tree_row_font_size())
                    .color(name_color),
                text(c.type_name.clone())
                    .size(crate::theme::font::caption_sm())
                    .color(crate::theme::color::DIM),
            ]
            .spacing(6)
            .align_y(iced_widget::core::Alignment::Center)
            .into()
        }
        SchemaRowKind::ColumnsLoading => row![
            indent,
            iced_widget::space::Space::new()
                .width(Length::Fixed(
                    crate::theme::icon_size::chevron()
                        + crate::theme::icon_size::tree_row_gap()
                        + crate::theme::icon_size::row()
                        + crate::theme::icon_size::tree_row_gap(),
                ))
                .height(Length::Shrink),
            text("加载列中…")
                .size(crate::workspace::tree_row_font_size())
                .color(crate::theme::color::DIM),
        ]
        .spacing(6)
        .align_y(iced_widget::core::Alignment::Center)
        .into(),
        SchemaRowKind::ColumnsFailed(e) => row![
            indent,
            iced_widget::space::Space::new()
                .width(Length::Fixed(
                    crate::theme::icon_size::chevron()
                        + crate::theme::icon_size::tree_row_gap()
                        + crate::theme::icon_size::row()
                        + crate::theme::icon_size::tree_row_gap(),
                ))
                .height(Length::Shrink),
            text(format!("列加载失败:{e}"))
                .size(crate::workspace::tree_row_font_size())
                .color(crate::theme::color::RED),
            text("(收起再展开可重试)")
                .size(crate::theme::font::caption_sm())
                .color(crate::theme::color::DIM),
        ]
        .spacing(6)
        .align_y(iced_widget::core::Alignment::Center)
        .into(),
    }
}
```

> 按钮样式:`Schema`/`Table` 两行的 `button(..)` 需要与 `files.rs` 树行按钮一致的透明底样式(files.rs 里 `row_btn` 的 `.style(..)` 写法)——实现时照抄那段样式链,不要自创新样式。

- [ ] **Step 6: 编译/测试/lint/格式**

```bash
cargo test -p dozer-app database::
cargo build -p dozer-app
cargo clippy -p dozer-app --all-targets
cargo fmt --check
```

预期:全绿;Task 1-3 的 dead_code 警告至此清零(Task 1 图标经 `IconKind::Table/Eye` 被 tree_rows 视图引用,消息变体全部有生产者/消费者)。

- [ ] **Step 7: Commit**

```bash
git add crates/dozer-app/src/extensions/database.rs
git commit -m "feat(dozer-app): add database schema tree view"
```

---

### Task 5: SQLite 集成测试(真库跑 introspection)

**Files:**
- Modify: `crates/dozer-app/src/extensions/database.rs`

**Interfaces:**
- 验证 `load_tables`/`load_columns` + 三套 SQL 中 SQLite 那套 + `Any` 解码形状端到端正确(设计文档"测试策略":tempfile 建库 → DDL 建表/视图 → 断言)。Postgres/MySQL 需要真实服务,**不在单测范围**,Task 6 人工验收覆盖。

- [ ] **Step 1: 集成测试模块**

加在 `database.rs` 尾部(独立 `#[cfg(test)] mod`,`install_default_drivers` 用 `Once` 防重复注册——app 级注册在 `main.rs` 启动时已做过,测试二进制是另一个进程):

```rust
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
        let url = format!("sqlite://{}/phase2.sqlite", dir.path().display());
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
        let tables = load_tables(DriverKind::Sqlite, &url).await.unwrap();
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
        // UI 已无入口的双保险:直接错误文案,不 panic/不尝试连接
        let err = load_tables(DriverKind::MongoDB, "mongodb://x").await.unwrap_err();
        assert!(err.contains("MongoDB"));
        let err = load_columns(DriverKind::MongoDB, "mongodb://x", None, "c")
            .await
            .unwrap_err();
        assert!(err.contains("MongoDB"));
    }
}
```

> 说明:`mongodb` loader 测试不需要 `install_drivers`(MongoDB 分支在连接前就返回)。`sqlite://` URL 拼的是绝对路径,sqlx 会在 tempdir 里建文件——`TempDir` drop 自动清理,不污染仓库。

- [ ] **Step 2: 编译/测试/lint/格式**

```bash
cargo test -p dozer-app database::
cargo clippy -p dozer-app --all-targets
cargo fmt --check
```

预期:4 个集成测试 PASS(首次跑若 sqlx 驱动注册顺序有问题,按报错调 `Once` 包裹范围)。

- [ ] **Step 3: Commit**

```bash
git add crates/dozer-app/src/extensions/database.rs
git commit -m "test(dozer-app): add sqlite introspection integration tests"
```

---

### Task 6: 全量验证 + 人工验收

**Files:** 无代码改动(验证任务)。

- [ ] **Step 1: 全量机器检查**

```bash
cargo build
cargo test -p dozer-app
cargo clippy --all-targets
cargo fmt --check
```

预期:全 workspace 编译过、dozer-app 测试全绿、clippy 无警告、fmt 干净。

- [ ] **Step 2: 人工验收清单(`cargo run -p dozer-app`,按设计文档"测试策略")**

打开数据库面板,逐项过:

1. **SQLite**:新增一个 SQLite 数据源(指向任一现存 `.sqlite` 文件,没有就 `sqlite3 /tmp/dozer_phase2.sqlite 'CREATE TABLE t(a INT); CREATE VIEW v AS SELECT * FROM t;'` 建一个),测试连接通过 → 卡片出现"浏览结构"按钮(在"测试连接"之后)。
2. 点"浏览结构" → 面板切成树,表/视图可见;展开表 → 先"加载列中…",随后列名/类型出现;非空列与可空列颜色深浅有别(对照 `NOT NULL` 定义)。
3. "← 返回" → 回卡片列表;**再进同一源不重拉**(秒开树)。
4. **Postgres**(有真实服务时):配置一个多 schema 的库 → 树出现 schema 节点(Folder 图标);单 schema 库 → 自动展开、直接见表行。无真实服务则跳过此项并在 PR 注明。
5. **错误态**:把数据源 host 改成不可达 → 树显示错误 + "重试";有旧快照时"刷新"host 失效 → 旧树保留 + 一行"刷新失败:…"。
6. **列失败重试**:断网/改错库名后展开表 → 红色"列加载失败";收起再展开 → 重新加载成功。
7. **刷新对账**:在 app 外给库加/删表 → 点"刷新" → 新表出现、被删表的展开态消失;"刷新中…"提示行短暂出现且不闪空旧树。
8. **MongoDB 卡片无"浏览结构"按钮**。
9. **删除/编辑联动**:删除正在浏览的源 → 回卡片列表;编辑保存已有源后再浏览 → 重新加载(不留旧快照)。
10. **切项目路由**:加载期间切到别的项目页签再切回 → 结果落到正确项目,不串态、不 panic。

- [ ] **Step 3: 整理提交历史并提请合并**

```bash
git log --oneline main..HEAD   # 检查:约 5 个语义清晰的 commit
```

确认无杂物 commit 后,将 `feature/database-panel-phase2` 提请合并到 `main`(PR 描述附人工验收结果,Postgres/MySQL 未覆盖项如实注明)。

---

## 完成定义(DoD)

- `cargo build` / `cargo test -p dozer-app` / `cargo clippy --all-targets` / `cargo fmt --check` 全绿,无 dead_code/unused 警告。
- 设计文档 §2 状态机表格逐条落地且被单测覆盖(摊平 3 例 + 状态机 10 例 + SQLite 集成 4 例)。
- 人工验收清单(Task 6 Step 2)通过;Postgres/MySQL 实测缺项在 PR 注明。
- 未引入任何新依赖;未触碰 `view` 签名/`LeftView`/struct 字段/`left_panel_area`;未做非目标清单里的任何事。


