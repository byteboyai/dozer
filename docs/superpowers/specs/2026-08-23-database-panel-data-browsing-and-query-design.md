# 数据库面板 · 数据浏览与 SQL 查询

**状态:草案(brainstorming 完成,待用户批准)**

## 背景

数据库面板此前已完成两个阶段并合并进 `main`:

- **阶段 1**(commit `d8fcecb`):驱动管理 + 每项目数据源 CRUD + 连接测试。见
  `docs/superpowers/specs/2026-08-08-database-panel-phase1-design.md`。
- **阶段 2**(commit `4ca6427`):schema 树浏览(drill-down 进数据源看
  库/表/视图/列),纯只读 introspection。见
  `docs/superpowers/specs/2026-08-08-database-panel-phase2-design.md`。

阶段 2 设计文档"非目标"一节明确留了三个坑给后续:③ 数据表格浏览、
④ SQL 脚本执行、⑤ MongoDB 集合浏览。用户拿一张 DataGrip 风格的草图
(`blog_post@coral_node.public` 打开一张表的截图)提出两条要求:①右侧要能
看表/集合数据,②右侧要有 SQL 查询工具。

这份文档把③④⑤(⑤ 仅取"集合浏览",不含 filter/sort)合并成一份设计,因为
三者共享同一套新基建——右侧内容窗格、tab 管理、结果集渲染、值转字符串的
类型解码层——拆成三份文档会把架构一节重复三遍。**取代**阶段 2 文档里对
③④⑤ 的占位描述;阶段 5 的 Mongo filter/sort 部分仍留待更后面。

调研核实(brainstorming 阶段已用 Explore 子代理核实,详见下方各节引用):

- `iced_widget::table`(仓库 `iced_widget = "0.14"` 依赖自带,`postgres`/
  `mysql`/`sqlite` 三个 sqlx 原生 feature 也已在 `Cargo.toml` 启用,见
  `crates/dozer-app/Cargo.toml:52`)——本设计**零新增依赖**,只是**首次使用**
  这两类既有能力。
- 主要内容区已有可复用的 tab 管理范式:`crates/dozer-app/src/preview.rs::
  PreviewPane`(`tabs: Vec<PreviewTab>` + `active: usize` + `next_id: usize`,
  `open_path` 做"已开则聚焦、否则新开"的去重),配 `workspace.rs::
  preview_pane_for` 的 tab 栏渲染(`tabs::tab_core`/`tab_widget::panel_tab`/
  `tab_arrow_button`/`tab_window`)。本设计的 `DatabaseContentState`/
  `DatabaseTab` 照抄这个结构。
- `PanelKind::Database` 目前在 `app.rs:6909` 只渲染 `database::view(...)`
  单栏、无右侧内容窗格、无 `Divider`——这正是本设计要填的空。
- `AppState`/`WorkspaceState` 的 `project_id` 显式路由口径(阶段 1/2 已用于
  `TestConnectionResult`/`TablesLoaded`/`ColumnsLoaded`)本设计延用,并加一层
  `tab_id`/`run_seq` 做 tab 级别的过期防线(见"异步路由"一节——这是本设计
  相对阶段 1/2 唯一新增的防线层,原因是同一 tab 可能被用户改完 WHERE 又点
  一次"运行",需要区分新旧两次请求)。

## 目标 / 非目标

**目标**:

1. **表格/视图浏览**(Postgres/MySQL/SQLite):schema 树点一张表 → 右侧内容
   窗格开一个新 tab(已开则聚焦),显示分页数据网格,带原始 WHERE/ORDER BY
   文本框、翻页控件、列头点击写排序。
2. **MongoDB 集合浏览**:schema 树(此驱动下平铺集合列表,无 schema/列两层)
   点一个集合 → 右侧开 tab,分页显示文档(BSON→JSON 文本渲染),不带
   filter/sort 输入(阶段 5 留白范围)。
3. **SQL 查询控制台**(仅关系型三家):tab 栏"+ 新查询"开一个新 tab(不去重,
   可开多个),多行 SQL 编辑器 + 执行按钮,支持任意语句(`SELECT`/`INSERT`/
   `UPDATE`/`DELETE`/DDL 均放行),结果按语句类型分流渲染。
4. 三种 tab 共用同一套右侧内容窗格骨架(tab 栏 + 关闭 + 切换)与同一个结果集
   渲染组件(`iced_widget::table`)。
5. 值解码从 introspection 专用的 `sqlx::AnyPool` 换成**驱动原生池**
   (`PgPool`/`MySqlPool`/`SqlitePool`),因为真实表数据/任意查询结果集里的
   列类型(`timestamp`/`numeric`/`jsonb`/`uuid`/数组…)无法像阶段 2
   introspection 查询那样提前转成 `Any` 认识的 9 种基础类型。

**非目标**(本轮明确不做):

- 不做单元格编辑/删行/插入行——只读浏览(草图定位是"看数据",改数据走 SQL
  查询工具执行 `UPDATE`/`DELETE`)。
- 不做 `COUNT(*)` 精确总行数(延续阶段 2 的成本顾虑,用"多取一行"判断是否
  有下一页,不显示总页数)。
- 不做结构化 WHERE/ORDER BY 构造器(下拉选列名+操作符),用原始文本片段。
- 不做 MongoDB 的 filter/sort 输入(仅分页浏览全部文档)。
- 不做查询历史/收藏查询。
- 不做取消运行中的查询(靠超时兜底)。
- 不做结果导出(CSV/JSON 落盘)。
- 不做多语句批量执行(一个查询 tab 的文本框整体当一条语句扔给驱动)。
- 不做列宽拖拽/持久化、不做结果集内再筛选/再排序。
- 不识别 `INSERT ... RETURNING`/`UPDATE ... RETURNING` 的返回行——按 SQL
  首关键字分流(见下),`INSERT`/`UPDATE`/`DELETE` 一律走"N 行受影响"文案,
  不尝试探测 `RETURNING` 子句。

## 架构与数据流

### 1. 右侧内容窗格骨架(仿 `preview.rs::PreviewPane`)

新增文件内类型(`extensions/database.rs` 追加,沿用阶段 1/2 单文件模式):

```rust
/// 右侧内容窗格里的一个 tab。
pub struct DatabaseTab {
    pub id: usize,
    pub kind: DatabaseTabKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DatabaseTabKind {
    Table { source_id: String, schema: Option<String>, table: String },
    Collection { source_id: String, name: String },
    /// 查询控制台不去重,`console_seq` 只用来生成默认标题("查询 1"/"查询 2"),
    /// 不参与去重比较。
    Query { source_id: String, console_seq: u32 },
}

/// tab 各自的内容态,和 `kind` 一一对应但分开存(浏览/查询字段差异大,
/// 拆成两个子结构比一个大 struct 塞两套字段可空清楚)。
pub enum TabContent {
    Browse(BrowseState),
    Query(QueryState),
}

pub struct BrowseState {
    pub where_clause: String,
    pub order_by: String,
    pub page: u32,
    pub page_size: u32,     // 取值来自 PAGE_SIZES
    pub has_more: bool,
    pub loading: bool,
    pub error: Option<String>,
    pub result: Option<QueryResult>, // 旧结果保留不闪空(阶段 2 惯例)
}

pub struct QueryState {
    pub sql: String,
    pub running: bool,
    pub error: Option<String>,
    pub result: Option<QueryOutcome>,
}

pub enum QueryOutcome {
    Rows(QueryResult),
    Affected(u64),
    Ddl,
}

/// 结果集统一表示:列名 + 逐行逐格已转字符串的值(渲染侧不再关心原始类型)。
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<CellValue>>,
}

pub enum CellValue {
    Text(String),
    Null,
}

pub const PAGE_SIZES: [u32; 3] = [50, 100, 500];
```

窗格状态容器(挂在 `WorkspaceState` 上,新增字段 `content:
DatabaseContentState`,和阶段 2 的 `browsing`/`schemas` 平级):

```rust
#[derive(Default)]
pub struct DatabaseContentState {
    tabs: Vec<DatabaseTab>,
    contents: HashMap<usize, TabContent>,
    active: usize,
    next_id: usize,
    next_console_seq: u32,
}

impl DatabaseContentState {
    /// 已开对应表的 tab → 聚焦;否则新开并置 Loading，照抄
    /// `PreviewPane::open_path` 的去重判断。
    pub fn open_table(&mut self, source_id: String, schema: Option<String>, table: String) -> usize;
    pub fn open_collection(&mut self, source_id: String, name: String) -> usize;
    /// 查询 tab 永远新开，不去重。
    pub fn open_query(&mut self, source_id: String) -> usize;
    pub fn close(&mut self, id: usize);
    pub fn select(&mut self, id: usize);
    pub fn tabs(&self) -> &[DatabaseTab];
    pub fn active_id(&self) -> Option<usize>;
    pub fn content(&self, id: usize) -> Option<&TabContent>;
    pub fn content_mut(&mut self, id: usize) -> Option<&mut TabContent>;
}
```

### 2. `app.rs` 内核接线

`panel_body()` 里 `PanelKind::Database` 分支（`app.rs:6909`）从单栏改成
"schema 树 + Divider + 内容窗格" 三件套,结构照抄 `Files` 分支
(`files::view` + `preview_pane()`)：

```rust
PanelKind::Database => {
    if ws.project.is_none() { return column![].into(); }
    let (list_portion, content_portion) = split_portions(app.dims.database_split);
    let list_pane = database::view(&app.database, &ws.database, ...);
    let content_pane = database::content_pane(&app.database, &ws.database, ...)
        .map(Message::Database);
    row![
        list_pane, // Length::FillPortion(list_portion)
        divider_bar(Divider::DatabaseSplit, ..., Message::ColumnDragStart(Divider::DatabaseSplit)),
        content_pane, // Length::FillPortion(content_portion)
    ].into()
}
```

新增 `AppDims::database_split: f32`(同 `files_split`/`ssh_split` 等字段
惯例,初始化 `f32::NAN` 走默认 50/50,`clamp_split` 复用现有逻辑)、
`Divider::DatabaseSplit` 变体、`HoverId` 追加 tab 关闭按钮所需的变体(如
`DatabaseTabClose(usize)`,同 `ProjectTabClose(i64)` 的按 id 区分惯例)。

`content_pane` 内部 tab 栏渲染复用 `tab_widget::panel_tab`/`tab_core`/
`tab_arrow_button`/`tab_window`(阶段 2 文档"UI 与视觉"一节已确认整套
组件对本项目其它面板通用,不重新手写 `MouseArea`)。

### 3. schema 树的改动(阶段 2 结构上追加,不推翻)

- `source_card`:去掉"MongoDB 不出现浏览结构按钮"的排除——MongoDB 源现在
  也能点"浏览结构"，进去看到的是**平铺集合列表**（无 schema 层、无列展开，
  `tree_rows` 对 MongoDB 走一条新分支：`SchemaRowKind::Collection(&str)`，
  单击直接触发 `OpenCollectionTab`，没有 chevron/展开箭头）。
- `TablesLoaded` 对 MongoDB 源改走 `list_collection_names()`（`mongodb` 驱动
  已有方法，替换阶段 2 里 `load_tables` 对 Mongo 直接返回错误文案的分支）。
- 关系型表/视图行新增点击行为：单击 = `Message::OpenTableTab { source_id,
  schema, table }`（阶段 2 单击只做展开/收起列，不冲突——列的展开箭头
  chevron 和"打开数据"分成两个可点区域，chevron 收展列、行内文字部分开
  tab，同 Files 文件树"点文件夹展开 vs 点文件打开"的既有分工）。
- schema 树头部工具行新增"+ 新查询"图标按钮（仅当前浏览的数据源非
  MongoDB 时可点），点击 = `Message::OpenQueryTab(source_id)`。

### 4. 值解码：从 `AnyPool` 切到驱动原生池

阶段 2 的 `load_tables`/`load_columns` 继续用 `sqlx::AnyPool`（不动，
introspection 查询本来就手工把每一列转成 `::text`，问题不大）。本设计新增
的数据浏览/查询执行路径改用原生池：

```rust
enum NativePool {
    Pg(sqlx::PgPool),
    MySql(sqlx::MySqlPool),
    Sqlite(sqlx::SqlitePool),
}

async fn connect_native(kind: DriverKind, url: &str) -> Result<NativePool, String>;

/// 各驱动一份"行 → 显示字符串"映射。常见标量（bool/int/float/text/bytes/
/// null）统一处理；驱动专属类型（jsonb/uuid/timestamp/array/decimal 等）
/// 按需 match 对应 Rust 类型转字符串；解不出的生僻类型显示占位
/// `<不支持的类型: {type_name}>`，不 panic、不让整行/整查询失败。
fn stringify_pg_row(row: &sqlx::postgres::PgRow) -> Vec<CellValue>;
fn stringify_mysql_row(row: &sqlx::mysql::MySqlRow) -> Vec<CellValue>;
fn stringify_sqlite_row(row: &sqlx::sqlite::SqliteRow) -> Vec<CellValue>;
```

`postgres`/`mysql`/`sqlite` 三个 sqlx feature 已在 `Cargo.toml:52` 启用
（`Any` 驱动本来就是靠它们做后端实现），零新增依赖。

MongoDB 走 `mongodb` 驱动的 `bson::Document`，`serde_json::to_string_pretty`
（`serde_json` 已是既有依赖）转 JSON 文本，天然不存在这个解码问题。

### 5. 表格/集合浏览查询

```rust
async fn browse_table(
    kind: DriverKind, url: &str,
    schema: Option<&str>, table: &str,
    where_clause: &str, order_by: &str,
    page: u32, page_size: u32,
) -> Result<BrowsePage, String>;

async fn browse_collection(
    url: &str, name: &str, page: u32, page_size: u32,
) -> Result<BrowsePage, String>;

pub struct BrowsePage {
    pub result: QueryResult,
    pub has_more: bool,
}
```

SQL 生成（关系型三家各自拼表名/分页语法，`where_clause`/`order_by` 非空时
原样拼进对应子句，不做转义/校验——用户对拼错/注入自担，符合"查询工具允许
任意 SQL"的同一口径）：

```sql
SELECT * FROM {table} {WHERE where_clause} {ORDER BY order_by} LIMIT {page_size+1} OFFSET {page*page_size}
```

`page_size+1` 之所以多取一行：拿到的行数 `> page_size` 就说明还有下一页
（`has_more = true`，渲染时丢弃多出的第 `page_size+1` 行），不做 `COUNT(*)`。

MongoDB 分页：`collection.find().skip(page*page_size).limit(page_size+1)`，
`has_more` 判定同上。

列头 ↑↓ 点击（草图里每列都有）：把 `"{列名} ASC"`/`"{列名} DESC"` 写入/
切换 `order_by` 文本框（同列再点一次在 ASC↔DESC 间切换，点别的列整体替换，
不支持多列排序组合——保持和"原始文本框"这一决定一致，用户想要多列排序
自己在框里写逗号分隔），随后自动重新发起查询（同"翻页"触发方式）。

翻页/改 WHERE/改 ORDER BY/切页大小都统一走 `Message::BrowseRun(tab_id)`：
写完文本框不是按键就查（避免半个 SQL 片段触发一堆失败请求），而是显式
"运行"按钮/回车确认 WHERE 框、下拉即触发 ORDER BY 与页大小、翻页按钮
直接触发。

### 6. SQL 查询控制台

```rust
async fn run_query(kind: DriverKind, url: &str, sql: &str) -> Result<QueryOutcome, String>;
```

**语句类型分流**：按 SQL 文本 trim 后首个关键字（大小写不敏感）判断——
`SELECT`/`WITH`/`SHOW`/`EXPLAIN`/`PRAGMA` 前缀 → `fetch_all` 走
`QueryOutcome::Rows`；其余（`INSERT`/`UPDATE`/`DELETE`/`CREATE`/`ALTER`/
`DROP`/…）→ `execute` 走 `QueryOutcome::Affected(rows_affected)`；`execute`
返回但驱动认定这是 DDL（没有 `rows_affected` 语义，如 `CREATE TABLE`）时
统一显示 `QueryOutcome::Ddl`（"执行成功"文案，不显示行数）——三种关系型
驱动的 `execute()` 返回值本身就带 `rows_affected()`，DDL 语句该值通常是 0，
用"关键字是否属于 DML 集合"而非返回值判断走哪个文案分支。

编辑器：`byteui::form::text_area::view`（多行自增高，`todo.rs` 已用于内容
编辑，同一组件复用），`bare: false`（这里要看得出是独立输入框，不是内联
紧凑模式）。执行入口：tab 内"执行"按钮 + Cmd+Enter 快捷键。

超时：区别于阶段 1/2 被动加载的 5 秒，这里是用户主动点"执行"、愿意等
（且任意 SQL 可能是有意的慢查询），超时改成 **30 秒**。不做取消运行中查询
的按钮（超时兜底）。

### 7. 异步路由与过期防线

延用阶段 1/2 的 `project_id` 显式路由（`App::update` 里特化 match 臂排在
通配 `Message::Database(msg)` 之前，同 `TestConnectionResult`/`TablesLoaded`
既有位置约定）。新增消息统一带 `(project_id, tab_id)`：

```rust
Message::BrowseResult(i64 /* project_id */, usize /* tab_id */, Result<BrowsePage, String>),
Message::QueryResult(i64 /* project_id */, usize /* tab_id */, Result<QueryOutcome, String>),
```

比阶段 1/2 多一层防线：**同一 tab 可能被重复触发**（用户改完 WHERE 又点
一次运行、或查询 tab 连续点两次执行）。`BrowseState`/`QueryState` 各加一个
`run_seq: u64` 字段，发请求前 `+1` 并把当前值带进异步闭包；结果落地时先比
对 `run_seq` 是否仍是发出时那个值，不是则丢弃（防"旧请求结果覆盖新请求
结果"，`ColumnsLoaded`/`TablesLoaded` 的过期防线只处理"源被删/tab 被关"，
没处理"同 tab 内连续两次请求"这个新场景，因为阶段 1/2 的操作都不可能在
一个尚未返回的加载期间被同一 tab 重新触发）。

关闭 tab（`Message::CloseTab(id)`）：直接从 `tabs`/`contents` 移除，飞行中
的请求靠 `contents.get_mut(&tab_id)` 落地时找不到 entry 自然丢弃，不用额外
取消机制。

## UI 与视觉

- 内容窗格 tab 栏、tab 标题格式：`{table}@{source.name}.{schema}`（Postgres
  带 schema 段，MySQL/SQLite 省略）、`{collection}@{source.name}`（Mongo）、
  `查询 {console_seq}@{source.name}`。全部沿用 `panel_tab` 现有截断+
  tooltip-on-long-hover。
- 数据网格首次使用 `iced_widget::table`：表头 = 列名（+ 浏览页的 ↑↓ 排序
  点击），单元格纯文本渲染，`CellValue::Null` 用 DIM 色 "NULL" 字样区分
  真实空字符串。不做拖拽调宽（列头文字过长走省略号 + tooltip 兜底）。
- 浏览页工具行（草图对应区域）：WHERE 文本框 + ORDER BY 文本框（都是
  `byteui::form::input_text`，placeholder 提示"原始 SQL 片段"）+ 页大小
  下拉（50/100/500）+ 上一页/下一页按钮（下一页按 `has_more` 置灰）。
- 查询控制台：顶部工具行"执行"按钮（Cmd+Enter 提示 tooltip）+ 多行编辑器
  + 下方结果区（`Rows` → 数据网格；`Affected(n)` → 绿字"N 行受影响"；
  `Ddl` → 绿字"执行成功"；错误 → 红字原样显示驱动报错）。
- 主题令牌全部用现有 14 色，不新增色值；Divider/split 交互照抄
  `files_split`/`ssh_split` 现有拖拽手感。
- Mongo 集合树行：复用阶段 2 的表行样式（图标换成新增 `IconKind::
  Collection` 或直接复用 `IconKind::Table`——视觉上够用不必新切图标，
  留实现阶段判断），不显示 chevron（无可展开的列层）。

## 错误处理

- 浏览/查询失败（连接失败、SQL 语法错误、超时）→ tab 内红字显示原始驱动
  报错，旧结果（如有）保留不闪空（阶段 2 惯例延续）。
- Keychain 读不到密码 → 视为无密码传连接串，认证失败由驱动报出具体错误
  （同阶段 1/2 口径）。
- 生僻类型解码失败 → 单元格显示占位文案，不让整行/整查询失败（"架构"
  一节已述）。
- tab 关闭/项目切走后结果晚到 → 按 `project_id` + `tab_id` + `run_seq`
  三层防线静默丢弃。
- 源被删除/编辑保存（连接信息变了）→ 该源关联的所有已开 tab（浏览/查询）
  一并关闭（同阶段 2 对 `schemas` 缓存的处理口径，避免 tab 里残留指向已失效
  连接的状态）。

## 测试策略

- **纯函数单测**：`DatabaseContentState::open_table`/`open_collection` 的
  去重逻辑；`open_query` 不去重且 `console_seq` 递增；`stringify_*_row` 对
  常见类型（null/bool/int/float/text/bytes）与至少一个"生僻类型走占位"的
  用例；SQL 关键字分流函数（`SELECT`/`WITH`/`INSERT`/`CREATE` 等边界大小写
  与前导空白）。
- **SQLite 集成测试**（`#[tokio::test]`，同阶段 2 用 tempfile 建临时库）：
  `browse_table` 分页（`has_more` 正确性）、WHERE/ORDER BY 片段生效、
  `run_query` 对 `SELECT`/`INSERT`/`CREATE TABLE` 三类分流结果正确。
  Postgres/MySQL/MongoDB 需要真实服务，不在单测范围，人工验收覆盖。
- **人工验收**：SQLite/Postgres/MySQL 各开一张表浏览、翻页、WHERE 生效、
  列头排序；MongoDB 集合分页浏览；查询控制台跑 SELECT/UPDATE/CREATE TABLE
  三类语句；查询语法错误显示报错；连续两次改 WHERE 触发的过期结果不覆盖
  最新结果（可用人为加延时的方式验证 `run_seq` 防线）；关闭 tab 后飞行中
  请求安全丢弃（不崩溃）；删除/编辑数据源后关联 tab 被关闭；加载期间切
  项目页签结果路由正确。

## 依赖变更

**无新增依赖**。`iced_widget::table`、sqlx 的 `postgres`/`mysql`/`sqlite`
原生 feature、`mongodb`/`serde_json` 均已是现有依赖，本设计只是**首次
启用/使用**这些既有能力。

## 排期

- 基线：`main`（当前 `HEAD`，`8bd0083`，无进行中的并行分支与本设计改动的
  文件冲突——`git branch -a` 核实过，只有 `main` 一条分支）。
- 建议按"架构骨架 → 表格浏览 → 查询控制台 → Mongo 集合浏览"的顺序拆
  Task，交给 `writing-plans` 技能细化。
- 实现计划：`docs/superpowers/plans/2026-08-23-database-panel-data-browsing-and-query.md`（待写）。
