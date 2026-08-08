# 数据库面板 · 阶段 2:schema 树浏览

**状态:草案(按阶段 1 设计/计划口径由 agent 起草,待用户批准)**

## 背景

阶段 1(驱动管理 + 数据源 CRUD + 连接测试)已合并进 `main`(commit
`d8fcecb`,见 `docs/superpowers/specs/2026-08-08-database-panel-phase1-design.md`
与 `docs/superpowers/plans/2026-08-08-database-panel-phase1.md`)。阶段 1 把数据库
面板的功能面切成 5 块:① 驱动管理 + 数据源 CRUD + 连接测试(已完成)、
② **schema 树浏览(本文档范围)**、③ 数据表格浏览、④ SQL 脚本执行、
⑤ MongoDB 集合浏览。①-④ 是关系型数据库(Postgres/MySQL/SQLite)的完整闭环,
⑤ 单独成阶段(MongoDB 没有"执行 SQL 脚本"这个概念)。

本阶段的目标:让用户能看到一个已配置数据源里**有什么**——库里的
schema(Postgres)/ 表 / 视图 / 每张表的列(名字、类型、可空性)。这是后续
"点一张表看数据"(阶段 3)的导航基础。

调研核实(均写代码前已验证,避免阶段 3 才踩坑):

- **sqlx `Any` 驱动不做占位符重写**——SQL 文本原样透传给后端驱动
  (核对过 `sqlx-core 0.8.6` 源码 `any/connection/executor.rs`,
  `prepare_with(sql, ..)` 直接传原始 SQL)。所以带绑定参数的查询**必须按驱动
  分别写**(Postgres `$1`/MySQL `?`/SQLite `?`),不存在"一条 SQL 三种后端
  通用"的写法。表列表查询可以完全不用绑定参数(见下),列查询不行。
- **`Any` 只能解码 9 种基础类型**(核对 `sqlx-core 0.8.6` 源码
  `any/type_info.rs`:Null/Bool/SmallInt/Integer/BigInt/Real/Double/Text/
  Blob)——introspection SQL 只投影 TEXT/INTEGER 列即可,不要 SELECT
  timestamp/numeric 之类。
- 仓库锁定的 sqlx 0.8.6 自带 `sqlite`/`postgres`/`mysql` 驱动与
  `runtime-tokio`,**本阶段零新增依赖**(图标是资源文件,不是依赖)。

## 目标 / 非目标

**目标**:

1. 交互模型采用**drill-down**:数据源卡片列表(阶段 1 已有视图)→ 点卡片上
   新增的"浏览结构"按钮 → 面板切换为该数据源的 schema 树,顶部"← 返回"
   回到卡片列表。不采用"所有数据源平铺成多根树"的方案——左面板窄,多根树
   会把卡片列表(增删改/测试连接入口)挤没;drill-down 与阶段 3"点表看
   数据"天然衔接(树 → 点表 → 再进一层)。
2. 树形态(按驱动分三种,前两种平铺、第三种多一层 schema 节点):
   - **MySQL/SQLite**:表/视图(depth 0)→ 列(depth 1),无中间层。
   - **Postgres**:schema 节点(depth 0,来自 `information_schema.tables` 的
     `table_schema`,过滤掉 `pg_catalog`/`information_schema` 两个系统
     schema)→ 表/视图(depth 1)→ 列(depth 2)。**加载后若整库只有一个
     schema,自动展开它**(绝大多数项目都是单 `public`,少一次点击)。
     schema 节点不做异步加载(表列表一次查全,客户端按 `table_schema`
     分组),避免多一层异步状态机。
   - **MongoDB:本阶段无入口**——"浏览结构"按钮不在 MongoDB 卡片上出现,
     集合浏览是阶段 5 的事。
3. `WorkspaceState` 新增(全部**纯内存、不持久化**——schema 是即时快照,
   重启后重新拉,展开状态不值得落盘):
   - `browsing: Option<String>`(正在浏览 schema 树的数据源 id;`None` =
     卡片列表视图)。
   - `schemas: HashMap<String, SchemaState>`(每个数据源一份树状态:表列表
     + 加载态 + 展开集合 + 每表列缓存)。
4. 加载策略:**表列表进入时拉一次(懒),列按表展开时逐表懒加载**。
   表列表一条 SQL(`information_schema.tables`/`sqlite_master`)成本固定;列
   在 Postgres/MySQL 也是逐表一条 SQL(`information_schema.columns` 按表
   过滤)——全库一次拉所有列在大库里会生成成千上万行,不做。SQLite 的列
   走 `pragma_table_info(?)` 表值函数(常规 SELECT,`Any` 驱动下比
   `PRAGMA table_info(...)` 语句形式更稳)。
5. 异步路由沿用阶段 1 口径:**所有异步结果消息带 `project_id`**
   (`TablesLoaded`/`ColumnsLoaded`),`App::update` 按消息自带 id 路由到
   正确的 `Workspace`(用户可能在加载期间切走项目页签)——内核需要两个新
   特化 match 臂,结构照抄 `TestConnectionResult`(排在通配
   `Message::Database(msg)` 之前)。
6. 超时沿用阶段 1 口径:`load_tables`/`load_columns` 各套 5 秒
   `tokio::time::timeout`,网络不可达不能让 UI 一直转圈。
7. 刷新:schema 树顶部"刷新"按钮重拉表列表;成功后做**缓存对账**——展开
   集/列缓存里已不存在的表清掉,仍然展开的表自动重新拉列,用户无感。

**非目标**(留给阶段 3-5 或明确不做):

- 不做数据浏览(点表看行数据——阶段 3,`iced_widget::table` 集成留那里)。
- 不做 SQL 执行、不做 schema 变更 DDL(本阶段对数据库**只读**:只有
  `SELECT` introspection 查询,连接用阶段 1 同一套连接串构造)。
- 不做 MongoDB 集合浏览(阶段 5)。
- 不做表名搜索/过滤框、不做行数统计(`COUNT(*)` 每表一次太贵)、不显示
  主键/索引/默认值/注释——列信息只有名字/类型/可空性三样,够用且三种
  后端取法都便宜;主键标注等真有需要再单独开工作。
- 不做列类型映射归一化——`data_type`(PG/MySQL)与 `type`(SQLite)原文
  显示(`character varying` 就显示 `character varying`,不翻译成 `varchar`)。
- 不做连接复用/池缓存——每次加载新建 `AnyPool`、用完 `close()`,同阶段 1
  "测试连接"一次一池的口径(schema 浏览是低频操作,池化的复杂度不值)。
- 不做展开状态/树内容持久化;切项目、重开 app 一律从头拉。
- 不碰 `view` 函数签名/`left_panel_area` 分发——内核改动只有两个特化
  match 臂(见"架构"节)。

## 架构与数据流

### 1. 数据模型(全部在 `extensions::database.rs` 现有文件内追加)

```rust
/// schema 树里的一个表/视图。
#[derive(Debug, Clone, PartialEq)]
pub struct TableRef {
    /// 仅 Postgres 有值(table_schema);MySQL/SQLite 恒 None。
    pub schema: Option<String>,
    pub name: String,
    pub is_view: bool,
}

/// 列元信息。主键/索引/默认值不在本阶段范围(见"非目标")。
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnInfo {
    pub name: String,
    pub type_name: String,
    pub nullable: bool,
}

/// 某张表的列加载进度。
#[derive(Debug, Clone)]
pub enum ColumnLoad {
    Loading,
    Loaded(Vec<ColumnInfo>),
    Failed(String),
}

/// 单个数据源的 schema 树状态(纯内存,不持久化)。
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
```

`WorkspaceState` 加两个字段:`browsing: Option<String>`、
`schemas: HashMap<String, SchemaState>`。

渲染用的**摊平**是纯函数(照搬 `project::FileTree::visible_rows` 的
DFS-摊平思路):

```rust
pub enum SchemaRowKind<'a> {
    Schema(&'a str),
    Table(&'a TableRef),
    Column(&'a ColumnInfo),
    ColumnsLoading,       // 列加载中的占位行
    ColumnsFailed(&'a str), // 列加载失败的错误行(收起再展开=重试)
}

pub struct SchemaRow<'a> {
    pub kind: SchemaRowKind<'a>,
    pub depth: usize,
    pub expanded: bool,
}

pub fn tree_rows(state: &SchemaState, driver: DriverKind) -> Vec<SchemaRow<'_>>;
```

### 2. 消息与状态机

`database::Message` 新增 6 个变体:

| 变体 | 语义 |
|------|------|
| `BrowseSchema(String)` | 点"浏览结构":`browsing = Some(id)`;该源从未加载过(无缓存、无错误、不在加载)则自动发起表加载 |
| `SchemaBack` | 顶部"← 返回":`browsing = None`(树状态保留,返回再进入不重拉) |
| `SchemaRefresh(String)` | 顶部"刷新":重拉表列表(旧快照保留不闪空,成功后对账) |
| `ToggleSchema(String)` | Postgres schema 节点展开/收起(纯同步) |
| `ToggleTable { source_id, schema, table }` | 表节点展开/收起;展开时列缓存缺失或曾失败 → 置 `Loading` 并发起列加载(收起保留缓存) |
| `TablesLoaded(i64, String, Result<Vec<TableRef>, String>)` / `ColumnsLoaded { project_id, source_id, schema, table, result }` | 异步结果,**带 `project_id`**(理由同阶段 1 `TestConnectionResult`) |

过期结果防线(照搬 `git_log` 的 pending 核对思路,按场景简化):

- `TablesLoaded`:落地时 `SchemaState.loading_tables` 不为 `true` → 丢弃
  (该源已删除/已被新一轮加载覆盖)。
- `ColumnsLoaded`:落地时对应 key 不是 `ColumnLoad::Loading` → 丢弃。
- `ToggleTable`/`SchemaRefresh` 一律先核对 `browsing == Some(source_id)`,
  防旧视图残留按钮。
- 源被删除(`DeleteSource`)或被编辑保存(`DraftSave` 且是编辑已有源):
  删其 `schemas` 条目并清 `browsing`。连接信息可能变了,旧结构快照不作数。
- `reload_from_disk`(切进面板重读 `database.json`)后 `browsing` 可能指向
  磁盘上已不存在的源——`view` 用 `browsing().and_then(find)` 取源,取不到
  自然回退卡片列表,不专门清。

### 3. introspection SQL(按驱动分派,只读)

表列表(无绑定参数,`Any` 下可一条 SQL 一个后端各写各的):

```sql
-- Postgres
SELECT table_schema, table_name, table_type
FROM information_schema.tables
WHERE table_schema NOT IN ('pg_catalog', 'information_schema')
ORDER BY table_schema, table_name;

-- MySQL(DATABASE() 服务端函数,免绑定)
SELECT table_name, table_type
FROM information_schema.tables
WHERE table_schema = DATABASE()
ORDER BY table_name;

-- SQLite(排除 sqlite_ 内部表)
SELECT name, type FROM sqlite_master
WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite_%'
ORDER BY name;
```

`is_view` = `table_type == 'VIEW'`(PG/MySQL)/ `type == 'view'`(SQLite)。

列(需要绑定参数 → 占位符风格按后端,三套):

```sql
-- Postgres(schema 缺失兜底 'public')
SELECT column_name, data_type, is_nullable
FROM information_schema.columns
WHERE table_schema = $1 AND table_name = $2
ORDER BY ordinal_position;

-- MySQL
SELECT column_name, data_type, is_nullable
FROM information_schema.columns
WHERE table_schema = DATABASE() AND table_name = ?
ORDER BY ordinal_position;

-- SQLite(pragma_table_info 表值函数;notnull 列 0/1 → nullable)
SELECT name, type, notnull FROM pragma_table_info(?) ORDER BY cid;
```

连接/关闭:`build_sql_url`(阶段 1 已有)拼串 → `AnyPool::connect` →
查询 → **`pool.close().await`**(阶段 1 测试连接一池一用没关,本阶段补上,
避免 SQLite 文件句柄残留)。MongoDB 源走到这两个函数直接返回错误文案
("MongoDB 集合浏览将在后续阶段支持")——双保险,正常路径 UI 已无入口。

### 4. 内核接线(workspace.rs)

只有两处新增,都在 `App::update`:

```rust
Message::Database(database::Message::TablesLoaded(project_id, source_id, result)) => { /* 照抄 TestConnectionResult 臂结构 */ }
Message::Database(database::Message::ColumnsLoaded { project_id, source_id, schema, table, result }) => { /* 同上 */ }
```

排在通配 `Message::Database(msg)` 之前(Rust match 按序匹配;同
`TestConnectionResult` 臂的既有顺序要求)。`view` 签名、`LeftView`、
struct 字段、`left_panel_area` 全部不动——`view` 内部按 `browsing` 自行
分发卡片列表/schema 树。

## UI 与视觉

- schema 树渲染**照搬 `extensions::files` 文件树惯例**:
  `"  ".repeat(depth)` 缩进文本 + chevron(`ChevronDown`/`ChevronRight`)+
  图标 + 名称;文件名用 `crate::workspace::tree_row_font_size()`;整树套
  `Scrollable` + `crate::scrollbar::scrollbar()`/`scrollbar_style()` 统一
  滚动条。
- 图标:Licene 表/视图/列节点各就各位——schema 节点复用已有
  `Folder`/`FolderOpen`;表节点新增 `IconKind::Table`(Lucide `table`);
  视图节点新增 `IconKind::Eye`(Lucide `eye`);列节点**不放图标**,用
  等宽 Space 占位对齐 chevron 位(同 files.rs 文件行处理),避免几百列的
  图标噪音。
- 列行:列名 + 类型小字(`caption_sm`/DIM);**非空列名用 CREAM、可空列名
  用 BODY**——用颜色深浅表达 nullable,不加 "NOT NULL" 文本后缀,树保持
  干净。
- 头部行:`[←ChevronLeft] 源名(subtitle/CREAM) 驱动类型
  (caption_sm/DIM) … 右侧"刷新"按钮`;旧快照在手又点了刷新 → 加一行
  "刷新中…"(DIM),同 `git_log`"旧图不闪空"惯例。
- 卡片区变化:`source_card` 按钮行加"浏览结构"(非 MongoDB 源才出现),
  排在"测试连接"之后。
- 主题令牌全部用现有 14 色,不新增色值。

## 错误处理

- 表加载失败/超时 → `tables_error`;无旧快照时面板显示错误 + "重试"按钮
  (重试 = `SchemaRefresh`);有旧快照时旧树保留 + 一行红字"刷新失败:…"。
- 列加载失败/超时 → 该表下红色错误行"列加载失败:{e}",**收起再展开该表
  即重试**(`ColumnLoad::Failed` 在再展开时重新触发加载)。
- Keychain 读不到密码 → 视为无密码传连接串,认证失败会由驱动报出具体
  错误,不 panic(同阶段 1)。
- 加载期间源被删/项目被切走 → 结果按上面的过期防线丢弃,不产生悬挂状态。

## 测试策略

- **纯函数单测**(Task 2):`tree_rows` 摊平——SQLite/MySQL 平铺形态、
  Postgres schema 分组与折叠/展开、Loading/Failed 占位行、视图标记。
- **SQLite 集成测试**(Task 3,`#[tokio::test]`):tempfile 建临时库 →
  AnyPool DDL 建两列一视图 → `load_tables` 断言表/视图成员与排序无关性 →
  `load_columns` 断言名字/类型/可空性。不需要外部服务;`sqlx::any::
  install_default_drivers()` 用 `std::sync::Once` 包一层防重复注册。
  Postgres/MySQL 需要真实服务,不在单测范围,人工验收覆盖。
- **人工验收**(Task 6):SQLite 源浏览/展开列;Postgres 源 schema 节点;
  不可达 host 的错误态与"重试";刷新对账(库外改表后刷新);MongoDB 卡片
  无"浏览结构"按钮;删除/编辑源后再浏览需重新加载;加载期间切项目页签
  结果路由正确。

## 依赖变更

**无**。sqlx/postgres/mysql/sqlite/any/keyring/mongodb 全部阶段 1 已引入;
新增的只有两个 Lucide SVG 资源(`table`/`eye`,LICENSE 沿用
`assets/icons/LICENSE` 既有约定)。

## 排期

- 基线:`main`(commit `d8fcecb`,即阶段 1 合并点)。开发分支
  `feature/database-panel-phase2`。
- 并行分支:`feature/ssh-panel-phase1`(SSH 面板,进行中,同样动
  `workspace.rs` 的 `App::update` 分发与 `LeftView`——与本阶段改的
  Database 特化臂/卡片视图是不同符号,冲突最多是"相邻位置各加一臂"级别)、
  `feature/app-workspace-split`(尚未合并;若实现期间它先合并,
  `workspace.rs` 符号位置按符号名 grep 重新定位即可)。
- 实现计划:`docs/superpowers/plans/2026-08-08-database-panel-phase2.md`。
