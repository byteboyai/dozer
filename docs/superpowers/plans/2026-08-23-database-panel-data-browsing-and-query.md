# 数据库面板 · 数据浏览与 SQL 查询 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 给数据库面板加右侧内容窗格:点 schema 树里的表/视图/MongoDB 集合 → 分页数据网格(只读,原始 WHERE/ORDER BY 片段筛选);"+ 新查询" → SQL 控制台(任意语句,含 UPDATE/DELETE/DDL)。取代阶段 2 设计文档"非目标"里留白的③数据浏览④SQL执行⑤MongoDB集合浏览(仅浏览,不含 filter/sort)。设计口径唯一真相源:`docs/superpowers/specs/2026-08-23-database-panel-data-browsing-and-query-design.md`(下文引用"设计文档"即此文件)。

**Architecture:** 沿用阶段 1/2 的单文件口径,全部数据模型/状态机/异步执行逻辑追加进 `crates/dozer-app/src/extensions/database.rs`。右侧内容窗格照抄 `crates/dozer-app/src/preview.rs::PreviewPane` 的 tab 管理范式(`tabs: Vec<_>` + `active: usize` 索引 + 稳定 `id: usize`),tab 栏渲染复用 `crates/dozer-app/src/tab_widget.rs` 的 `panel_tab`/`tab_arrow_button`/`tab_window`(**不**复用 `PreviewPane` 的拖拽换位/右键菜单——设计文档非目标里没提,属于范围外,别顺手加)。内核接线仿照 `PanelKind::Ssh`/`PanelKind::Project` 在 `app.rs::panel_body()` 里的"列表/树 + `Divider` + 内容窗格"三件套结构。异步结果路由沿用阶段 1/2 的显式 `project_id` 特化 match 臂(排在通配 `Message::Database(msg)` 之前),额外加一层 `run_seq` 防线处理"同一 tab 内连续两次触发查询"这个阶段 1/2 没有的新场景。

**Tech Stack:** Rust;sqlx 0.8(`postgres`/`mysql`/`sqlite` 三个原生 feature 阶段 1 已启用,本计划额外加 `uuid`/`chrono`/`json`/`rust_decimal` 四个 feature 才能正确解码真实表数据里的常见列类型——**这是对设计文档"依赖变更:无新增依赖"一节的修正**,见下方 Global Constraints 详细说明)。`iced_widget::table`(仓库已有依赖,首次使用)。`mongodb`/`serde_json`(阶段 1 已引入)。

**Spec:** `docs/superpowers/specs/2026-08-23-database-panel-data-browsing-and-query-design.md`

## Global Constraints

- **分支基线**:从 `main` 当前 tip(`b552467`,即本设计文档的提交点)切 `feature/database-panel-data-browsing`。完成后提请审阅合并,不直接推 `main`(同仓库既有 SDD 惯例)。
- **对设计文档"依赖变更"一节的修正**(brainstorming 阶段未核实到这一层细节,写计划时发现):数据浏览/SQL 查询需要正确显示真实表里的 `uuid`/`timestamp`/`jsonb`/`numeric` 等常见列类型,`sqlx::Any` 驱动做不到(只认 9 种基础类型),原生池(`PgPool`/`MySqlPool`/`SqlitePool`)本身也需要额外 sqlx feature 才能 `Decode` 这些类型。Task 2 会把 `uuid`/`chrono`/`json`/`rust_decimal` 四个 feature 加进 `crates/dozer-app/Cargo.toml` 的 `sqlx` 依赖行——都是 `sqlx` 一个 crate 内的 feature 开关,不是引入全新的第三方库,但确实会多拉几个小体积传递依赖(`uuid`/`chrono`/`rust_decimal` 三个 crate;`serde_json` 已经是既有依赖)。这个修正范围明确、影响可控,不需要回头改设计文档,这里记录一下即可。
- **MongoDB 数据库名**:阶段 1 的 `DataSource.database` 字段对 MongoDB 源同样会被 `source_form` 收集(非 SQLite 分支的通用字段),但阶段 1/2 从未真正用过它连接 MongoDB(`load_tables`/`load_columns` 对 MongoDB 分支此前恒返回"暂不支持"错误文案,从未连接)。本计划 Task 4 是第一次真正需要它——`browse_collection`/集合清单查询都要先 `client.database(db_name)` 选库。**这不是新发现的 bug,是阶段 2 范围里从未触达的路径**,现在才需要处理。
- **不做**(设计文档"非目标",照单复述,任务里不要顺手加):单元格编辑/删行/插入行、`COUNT(*)` 精确总行数、结构化 WHERE/ORDER BY 构造器、MongoDB 的 filter/sort、查询历史/收藏、取消运行中的查询、结果导出、多语句批量执行、列宽拖拽持久化、`RETURNING` 子句识别、tab 拖拽换位、tab 右键菜单。
- **测试要求**:纯函数(`build_browse_sql`/`classify_statement`/`quote_table`/`DatabaseContentState` 的去重与关闭逻辑)必须有单测。`stringify_pg_row`/`stringify_mysql_row` 无法脱离真实服务单测,不强求;`stringify_sqlite_row`/`browse_table`(SQLite 路径)/`run_query`(SQLite 路径)走 `#[tokio::test]` 真实 SQLite 临时库集成测试(同阶段 2 `sqlite_introspection` 模块的写法)。Postgres/MySQL/MongoDB 路径全部留 Task 9 人工验收。
- **每个任务结束**:`cargo build -p dozer-app`、`cargo test -p dozer-app`、`cargo clippy -p dozer-app --all-targets`、`cargo fmt --check` 全绿。Task 1-5 结束时部分新符号还没被 `app.rs`/视图接线消费,会出现 `dead_code`/unused 警告——**预期过渡态,不要加 `#[allow(dead_code)]`**,Task 6-8 接线后自然清零。
- **每个任务一个 commit**;全部完成后跑 Task 9 的人工验收清单再提请合并。
- **并行分支**:执行计划时先跑 `git branch -a` 核对有没有新的并行分支(写这份计划时只有 `main` 一条)。如果发现别的分支在动 `app.rs`/`database.rs`,按符号名 grep 重新定位,不要假设行号。

---

### Task 1: 纯数据模型——tab/内容状态类型 + `DatabaseContentState`

**Files:**
- Modify: `crates/dozer-app/src/extensions/database.rs`(在阶段 2 代码之后、`#[cfg(test)] mod url_tests` 之前追加新代码;测试追加进新的 `#[cfg(test)] mod content_tests` 模块)

**Interfaces:**
- Produces:`DatabaseTabKind`、`DatabaseTab`、`TabContent`、`BrowseState`、`QueryState`、`QueryOutcome`、`QueryResult`、`CellValue`、`PAGE_SIZES`、`DatabaseContentState`(含 `tabs()`/`active_idx()`/`active_tab()`/`content()`/`content_mut()`/`open_table()`/`open_collection()`/`open_query()`/`select()`/`close()`/`close_by_source()`)——Task 2/3/4/5/7/8 消费。

- [ ] **Step 1: 追加类型定义**

在 `database.rs` 里,阶段 2 的 `schema_tree_row` 函数(现有代码结尾,约第 1759 行)之后追加:

```rust
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

pub enum QueryOutcome {
    Rows(QueryResult),
    Affected(u64),
    Ddl,
}

/// 右侧内容窗格:多个表/集合/查询 tab,`active` 是**索引**(同
/// `PreviewPane::active_idx()` 的约定,tab 栏渲染/hover 状态按索引找)。
/// `contents` 按**稳定 id**存(消息/异步结果按 id 路由,索引会随关闭
/// 漂移)。
#[derive(Default)]
pub struct DatabaseContentState {
    tabs: Vec<DatabaseTab>,
    contents: std::collections::HashMap<usize, TabContent>,
    active: usize,
    next_id: usize,
    next_console_seq: u32,
    /// tab 栏箭头翻页的窗口起点(同 `Workspace::preview_tab_first` 的用法),
    /// 每次渲染都交给 `tab_window` 钳到合法范围,这里存的只是"用户上次翻到
    /// 哪"的粗略意图。
    tab_scroll_first: usize,
}

impl DatabaseContentState {
    pub fn tabs(&self) -> &[DatabaseTab] {
        &self.tabs
    }

    pub fn active_idx(&self) -> usize {
        self.active
    }

    pub fn tab_scroll_first(&self) -> usize {
        self.tab_scroll_first
    }

    /// tab 栏箭头翻页,`right=true` 右翻、`false` 左翻。步进量(2)和越界
    /// 钳制逻辑照抄 `app.rs::Message::PreviewTabScroll` 的既有实现——越界
    /// 不在这里防,`tab_window` 渲染时会自动钳回合法范围。
    pub fn scroll_tabs(&mut self, right: bool) {
        if right {
            self.tab_scroll_first = self.tab_scroll_first.saturating_add(2);
        } else {
            self.tab_scroll_first = self.tab_scroll_first.saturating_sub(2);
        }
    }

    pub fn active_tab(&self) -> Option<&DatabaseTab> {
        self.tabs.get(self.active)
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
        self.active = self.tabs.len() - 1;
        id
    }

    /// 已开同一张表的 tab → 聚焦(内容/游标不重置);否则新开一个空浏览态。
    pub fn open_table(&mut self, source_id: String, schema: Option<String>, table: String) -> usize {
        if let Some((idx, tab)) = self.tabs.iter().enumerate().find(|(_, t)| {
            matches!(&t.kind, DatabaseTabKind::Table { source_id: s, schema: sc, table: tb }
                if *s == source_id && *sc == schema && *tb == table)
        }) {
            self.active = idx;
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
            self.active = idx;
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
            self.active = idx;
        }
    }

    /// 关闭指定索引的 tab。`active` 调整规则同浏览器标签页惯例:关掉
    /// active 之前的 tab → active 索引减 1(仍指向原 tab);关掉 active
    /// 自己且不是最后一个 → active 索引不变(自然落到后一个 tab 上);
    /// 关掉最后一个 tab 且它正是 active → active 收缩到新的最后一个。
    pub fn close(&mut self, idx: usize) {
        if idx >= self.tabs.len() {
            return;
        }
        let id = self.tabs[idx].id;
        self.tabs.remove(idx);
        self.contents.remove(&id);
        if self.tabs.is_empty() {
            self.active = 0;
        } else if self.active >= self.tabs.len() {
            self.active = self.tabs.len() - 1;
        } else if idx < self.active {
            self.active -= 1;
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
```

- [ ] **Step 2: `WorkspaceState` 挂新字段**

找到 `WorkspaceState` 定义(阶段 2 已有,约第 540 行),加一个字段:

```rust
#[derive(Debug, Default)]
pub struct WorkspaceState {
    sources: Vec<DataSource>,
    editing: Option<DataSourceDraft>,
    test_status: HashMap<String, TestStatus>,
    browsing: Option<String>,
    schemas: HashMap<String, SchemaState>,
    /// 右侧内容窗格状态(表/集合/查询 tab)。纯内存,不持久化——同
    /// `schemas`(阶段 2),重启后 tab 全部关闭,不留痕迹。
    content: DatabaseContentState,
}
```

`WorkspaceState` 当前是 `#[derive(Debug, Default)]`,但 `DatabaseContentState` 没有 derive `Debug`(`TabContent`/`QueryState` 内含 `text_editor::Content`,和 `preview.rs::PreviewTab` 因为含 `CodeEditor` 摘掉 `Debug`/`Clone`/`PartialEq` 是同一原因)。给 `WorkspaceState` 手写 `Debug`(照抄 `preview.rs::PreviewTab` 的手写 `Debug` 思路,只是这里是外层结构体):

```rust
impl std::fmt::Debug for WorkspaceState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkspaceState")
            .field("sources", &self.sources)
            .field("editing", &self.editing)
            .field("test_status", &self.test_status)
            .field("browsing", &self.browsing)
            .field("schemas", &self.schemas)
            .field("content_tab_count", &self.content.tabs().len())
            .finish()
    }
}
```

同时把 `#[derive(Debug, Default)]` 改成只 `#[derive(Default)]`(`DatabaseContentState` 已经 `#[derive(Default)]`,`WorkspaceState` 的 `Default` 派生不受影响)。加一个只读访问器给 Task 7/8 视图用(状态变更全部经 `Message` 走 `update()`,`update()` 本身在同一个模块里,直接摸 `ws_state.content` 私有字段即可,不需要额外的 `pub` 可变访问器——同阶段 1/2 `sources`/`schemas`/`browsing` 几个字段的既有口径:模块内直接改,模块外只读):

```rust
impl WorkspaceState {
    // ...(阶段 2 已有的 sources()/editing()/test_status()/browsing_source()/schema_state() 不动)

    pub fn content(&self) -> &DatabaseContentState {
        &self.content
    }
}
```

- [ ] **Step 3: 单测**

在文件末尾新增一个测试模块(和现有 `#[cfg(test)] mod tests`/`mod url_tests`/`mod sqlite_introspection` 平级):

```rust
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
        assert_eq!(st.active_idx(), 0); // 聚焦回第一个 tab
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
        assert_eq!(st.active_idx(), 0); // 仍指向 b,现在挪到索引0
        assert_eq!(st.tabs()[0].kind, DatabaseTabKind::Table { source_id: "s1".into(), schema: None, table: "b".into() });
    }

    #[test]
    fn close_active_last_tab_shrinks_active() {
        let mut st = DatabaseContentState::default();
        st.open_table("s1".into(), None, "a".into());
        st.open_table("s1".into(), None, "b".into());
        // active 目前是索引1(b,刚开的)
        st.close(1);
        assert_eq!(st.tabs().len(), 1);
        assert_eq!(st.active_idx(), 0);
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
```

- [ ] **Step 4: 验证**

```bash
cargo test -p dozer-app content_tests
cargo build -p dozer-app
cargo clippy -p dozer-app --all-targets
cargo fmt --check
```

Expected: `content_tests` 全部 PASS;`cargo build` 会有 `dead_code` 警告(新类型还没被消费)——预期过渡态。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/extensions/database.rs
git commit -m "feat(dozer-app): add database content-pane tab data model"
```

---

### Task 2: 值解码 + SQL 生成/语句分流纯函数

**Files:**
- Modify: `crates/dozer-app/Cargo.toml`
- Modify: `crates/dozer-app/src/extensions/database.rs`

**Interfaces:**
- Consumes:`CellValue`/`QueryResult`(Task 1)。
- Produces:`NativePool`、`connect_native()`、`close_native()`、`stringify_pg_row()`/`stringify_mysql_row()`/`stringify_sqlite_row()`、`rows_to_result()`、`quote_table()`、`build_browse_sql()`、`StatementKind`、`classify_statement()`、`is_ddl_keyword()`——Task 3 消费。

- [ ] **Step 1: 加 sqlx feature**

`crates/dozer-app/Cargo.toml` 找到现有这行(约第 52 行):

```toml
sqlx = { version = "0.8", default-features = false, features = ["runtime-tokio", "tls-rustls", "any", "postgres", "mysql", "sqlite"] }
```

改成:

```toml
sqlx = { version = "0.8", default-features = false, features = ["runtime-tokio", "tls-rustls", "any", "postgres", "mysql", "sqlite", "uuid", "chrono", "json", "rust_decimal"] }
```

跑 `cargo build -p dozer-app` 确认能拉到依赖、不报 feature 不存在(sqlx 0.8 系列这几个 feature 名字是标准约定,如果本仓库锁定的具体 patch 版本有出入,以编译器报错为准调整)。

- [ ] **Step 2: 值解码类型与函数**

在 `database.rs` 里 Task 1 追加的代码之后继续追加:

```rust
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
    use sqlx::{Column, Row};
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
```

如果 Step 1 加的 feature 名字或 `sqlx::types::Uuid`/`sqlx::types::chrono::*`/`sqlx::types::Decimal` 这几个路径和本仓库锁定的 sqlx 0.8.6 版本对不上(`cargo build` 报"找不到该类型"/"没有实现 `Decode`"),用 `cargo doc -p sqlx --open` 或直接翻 `~/.cargo/registry/src/*/sqlx-postgres-0.8.*/src/types/` 源码核对真实导出路径,按报错调整 `use`/类型路径——上面给的是 sqlx 0.8 系列的标准约定,不是编出来的占位。

- [ ] **Step 3: SQL 生成 + 语句分流纯函数**

```rust
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
    matches!(first_word.as_str(), "CREATE" | "ALTER" | "DROP" | "TRUNCATE")
}
```

- [ ] **Step 4: 单测**

```rust
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
        for sql in ["select 1", "  SELECT * FROM t", "with x as (select 1) select * from x", "SHOW TABLES", "explain select 1", "pragma table_info(t)"] {
            assert_eq!(classify_statement(sql), StatementKind::Rows, "sql={sql}");
        }
    }

    #[test]
    fn classify_statement_treats_dml_ddl_as_execute() {
        for sql in ["insert into t values (1)", "UPDATE t SET x=1", "delete from t", "CREATE TABLE t (id int)"] {
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
```

- [ ] **Step 5: 验证**

```bash
cargo test -p dozer-app query_gen_tests
cargo build -p dozer-app
cargo clippy -p dozer-app --all-targets
cargo fmt --check
```

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/Cargo.toml crates/dozer-app/src/extensions/database.rs
git commit -m "feat(dozer-app): add native-pool value decoding and browse SQL generation"
```

---

### Task 3: 关系型数据浏览与查询执行 + SQLite 集成测试

**Files:**
- Modify: `crates/dozer-app/src/extensions/database.rs`

**Interfaces:**
- Consumes:`NativePool`/`connect_native`/`close_native`/`stringify_*_row`/`rows_to_result`/`build_browse_sql`/`classify_statement`/`is_ddl_keyword`(Task 2)、`QueryResult`/`QueryOutcome`(Task 1)。
- Produces:`BrowsePage`、`browse_table()`、`run_query()`——Task 5 消费。

- [ ] **Step 1: `browse_table`/`run_query`**

```rust
/// 浏览页一次查询的结果:`has_more` 由"多取一行"判断(设计文档"架构与
/// 数据流 §5"),渲染前已把多出的那行丢弃。
pub struct BrowsePage {
    pub result: QueryResult,
    pub has_more: bool,
}

async fn browse_table(
    kind: DriverKind,
    url: &str,
    schema: Option<&str>,
    table: &str,
    where_clause: &str,
    order_by: &str,
    page: u32,
    page_size: u32,
) -> Result<BrowsePage, String> {
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
```

- [ ] **Step 2: SQLite 集成测试**

追加到已有的 `#[cfg(test)] mod sqlite_introspection` 模块里(复用它的 `setup_db()`/`install_drivers()`),或新开一个平级模块(推荐新开,避免这个模块名字和职责对不上):

```rust
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
                .bind(if i == 3 { None::<String> } else { Some(format!("note-{i}")) })
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
        let page0 = browse_table(DriverKind::Sqlite, &url, None, "items", "", "id ASC", 0, 2)
            .await
            .unwrap();
        assert_eq!(page0.result.columns, vec!["id", "name", "note"]);
        assert_eq!(page0.result.rows.len(), 2);
        assert!(page0.has_more);

        let page2 = browse_table(DriverKind::Sqlite, &url, None, "items", "", "id ASC", 2, 2)
            .await
            .unwrap();
        assert_eq!(page2.result.rows.len(), 1); // 第5条,最后一页
        assert!(!page2.has_more);
    }

    #[tokio::test]
    async fn browse_table_where_clause_filters_and_null_renders_as_null() {
        let (_dir, url) = setup_db_with_rows().await;
        let page = browse_table(
            DriverKind::Sqlite,
            &url,
            None,
            "items",
            "id = 3",
            "",
            0,
            50,
        )
        .await
        .unwrap();
        assert_eq!(page.result.rows.len(), 1);
        let note_idx = page.result.columns.iter().position(|c| c == "note").unwrap();
        assert_eq!(page.result.rows[0][note_idx], CellValue::Null);
    }

    #[tokio::test]
    async fn run_query_select_returns_rows() {
        let (_dir, url) = setup_db_with_rows().await;
        let outcome = run_query(DriverKind::Sqlite, &url, "SELECT id, name FROM items WHERE id <= 2 ORDER BY id")
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
        let outcome = run_query(DriverKind::Sqlite, &url, "UPDATE items SET note = 'x' WHERE id <= 2")
            .await
            .unwrap();
        assert!(matches!(outcome, QueryOutcome::Affected(2)));
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
```

- [ ] **Step 3: 验证**

```bash
cargo test -p dozer-app sqlite_browse_and_query
cargo build -p dozer-app
cargo clippy -p dozer-app --all-targets
cargo fmt --check
```

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/src/extensions/database.rs
git commit -m "feat(dozer-app): add relational table browsing and arbitrary SQL execution"
```

---

### Task 4: MongoDB 集合浏览

**Files:**
- Modify: `crates/dozer-app/src/extensions/database.rs`

**Interfaces:**
- Consumes:`BrowsePage`/`QueryResult`/`CellValue`(Task 1/3)、既有 `load_tables`/`spawn_tables_load`/`build_mongo_url`(阶段 1/2)。
- Produces:`load_mongo_collections()`、`browse_collection()`;修改 `load_tables()` 签名(新增 `mongo_db: Option<&str>` 参数)与 `spawn_tables_load()` 签名(新增 `mongo_db: Option<String>` 参数)——Task 5 消费改过的签名。

- [ ] **Step 1: `load_tables` 加 MongoDB 分支,签名多一个参数**

阶段 2 的 `load_tables` 现在对 MongoDB 恒返回错误文案(从未真正连接)。改成:

```rust
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
    // 以下关系型分支原样不动(阶段 2 现有代码,只是缩进后现在多了一层
    // "if kind == MongoDB { ... }" 在它前面)。
    let work = async {
        let pool = sqlx::AnyPool::connect(url)
            .await
            .map_err(|e| e.to_string())?;
        // ...(阶段 2 原有实现不动)
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
```

- [ ] **Step 2: `spawn_tables_load` 签名加一个参数,两个调用点修正 URL 构造**

```rust
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
```

`Message::BrowseSchema` 和 `Message::SchemaRefresh` 两个 `update()` 分支目前都无条件调 `build_sql_url(&source, password)`——这个函数对 `DriverKind::MongoDB` 是 `unreachable!()`。之前 MongoDB 源永远走不到这两个分支(`source_card` 的"浏览结构"按钮对 MongoDB 不渲染),Task 5/7 会去掉这个按钮排除,所以这里必须先把 URL 构造改成按驱动分流,不然一点"浏览结构"就 panic:

```rust
// Message::BrowseSchema(id) 分支里,原来的:
//   spawn_tables_load(handle, project_id, id, source.driver, build_sql_url(&source, password.as_deref()), emit);
// 改成:
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
```

`Message::SchemaRefresh(source_id)` 分支做同样的改动(找到里面调用 `spawn_tables_load` 的那一行,同样先按驱动分流构造 `url`,再多传 `source.database.clone()`)。

- [ ] **Step 3: `browse_collection`**

```rust
async fn browse_collection(
    url: &str,
    db_name: &str,
    name: &str,
    page: u32,
    page_size: u32,
) -> Result<BrowsePage, String> {
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
        // 单列 "document",值是整份 JSON 文本(设计文档"架构与数据流 §4":
        // BSON → serde_json::Value → 文本)。
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
```

需要 `use futures_util::stream::TryStreamExt;`(拿 `cursor.try_next()`)——检查 `crates/dozer-app/Cargo.toml` 是否已有 `futures-util`(`mongodb` crate 自己依赖它,大概率已经在依赖树里但没被 `dozer-app` 直接列为依赖;如果 `cargo build` 报找不到这个 crate,在 `Cargo.toml` 加一行 `futures-util = "0.3"`)。

- [ ] **Step 4: `source_card` 去掉 MongoDB 排除**

找到 `source_card` 函数里这段(阶段 2 现有代码):

```rust
if source.driver != DriverKind::MongoDB {
    // MongoDB 集合浏览是阶段 5;本阶段无入口
    btns = btns.push(
        button(text("浏览结构")).on_press(Message::BrowseSchema(source.id.clone())),
    );
}
```

改成无条件推入(去掉 `if`/注释,MongoDB 现在也能浏览):

```rust
btns = btns.push(button(text("浏览结构")).on_press(Message::BrowseSchema(source.id.clone())));
```

- [ ] **Step 5: 修复现有测试调用点的签名变更**

Task 4 改了 `load_tables`/`spawn_tables_load` 的签名,阶段 2 遗留的测试(`sqlite_introspection` 模块里 `load_tables(DriverKind::Sqlite, &url)`、`load_tables(DriverKind::MongoDB, "mongodb://x")` 等调用)要跟着补第三个参数:

```rust
// load_tables_lists_tables_and_views_in_order / mongodb_loaders_refuse_with_friendly_error 等测试里:
let tables = load_tables(DriverKind::Sqlite, &url, None).await.unwrap();
// ...
let err = load_tables(DriverKind::MongoDB, "mongodb://x", None).await.unwrap_err();
assert!(err.contains("数据库名")); // 断言文案也要跟着改——不再是"MongoDB 集合浏览将在后续阶段支持"
```

- [ ] **Step 6: 单测(MongoDB 无本地服务,只测能测的部分)**

```rust
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
```

(真实连接 MongoDB 服务器的路径——`load_mongo_collections`/`browse_collection` 实际拉到集合/文档——留 Task 9 人工验收,同设计文档测试策略一节口径。)

- [ ] **Step 7: 验证**

```bash
cargo test -p dozer-app
cargo build -p dozer-app
cargo clippy -p dozer-app --all-targets
cargo fmt --check
```

Expected: 全绿,阶段 2 遗留测试因为 Step 5 的签名修补继续通过。

- [ ] **Step 8: Commit**

```bash
git add crates/dozer-app/src/extensions/database.rs crates/dozer-app/Cargo.toml
git commit -m "feat(dozer-app): add MongoDB collection browsing"
```

---

### Task 5: `Message` 扩展 + `database::update()` 状态机

**Files:**
- Modify: `crates/dozer-app/src/extensions/database.rs`

**Interfaces:**
- Consumes:Task 1-4 全部类型/函数。
- Produces:`Message` 新增 14 个变体(`OpenTableTab`/`OpenCollectionTab`/`OpenQueryTab`/`SelectTab`/`CloseTab`/`BrowseWhereChanged`/`BrowseOrderByChanged`/`BrowsePageSizeChanged`/`BrowsePrev`/`BrowseNext`/`BrowseRun`/`BrowseResult`/`QueryTextAction`/`QueryRun`/`QueryResult`)——Task 6(app.rs 路由)、Task 7/8(视图)消费。

- [ ] **Step 1: `Message` 枚举追加变体**

找到 `Message` 枚举定义(阶段 2 已有,约第 620 行),在 `ColumnsLoaded` 变体之后追加:

```rust
    // ---- 右侧内容窗格:表/集合/查询 tab 的开关 ----
    /// schema 树点一张表/视图的行内文字 → 开(或聚焦已开的)浏览 tab。
    OpenTableTab {
        source_id: String,
        schema: Option<String>,
        table: String,
    },
    /// schema 树点一个 MongoDB 集合 → 开(或聚焦已开的)浏览 tab。
    OpenCollectionTab { source_id: String, name: String },
    /// schema 树头部"+ 新查询" → 永远新开一个查询 tab。
    OpenQueryTab(String),
    /// tab 栏点某个 tab → 切换 active(索引)。
    SelectTab(usize),
    /// tab 栏点 × → 关闭(索引)。
    CloseTab(usize),
    /// tab 栏箭头翻页(tab 溢出可视宽度时),`true`=右翻、`false`=左翻。
    TabScroll(bool),

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
```

- [ ] **Step 2: `update()` 里加对应分支**

找到 `update()` 函数(阶段 2 已有,约第 689 行),在 `Message::ToolbarHover(..) => { ... }` 分支之前追加(保持"过期结果防线"排在其它同步分支之后、和阶段 2 现有顺序风格一致即可,不要求特定位置):

```rust
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
        Message::SelectTab(idx) => ws_state.content.select(idx),
        Message::CloseTab(idx) => ws_state.content.close(idx),
        Message::TabScroll(right) => ws_state.content.scroll_tabs(right),
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
            dispatch_browse_run(ws_state, app_state, tab_id, project_id, repo_path, handle, emit);
            return;
        }
        Message::BrowseNext(tab_id) => {
            if let Some(TabContent::Browse(b)) = ws_state.content.content_mut(tab_id)
                && b.has_more
            {
                b.page += 1;
            }
            dispatch_browse_run(ws_state, app_state, tab_id, project_id, repo_path, handle, emit);
            return;
        }
        Message::BrowseRun(tab_id) => {
            dispatch_browse_run(ws_state, app_state, tab_id, project_id, repo_path, handle, emit);
            return;
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
                    DatabaseTabKind::Query { source_id, .. } => {
                        ws_state.sources.iter().find(|s| &s.id == source_id).cloned()
                    }
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
```

`dispatch_browse_run` 是个小 helper,`BrowsePrev`/`BrowseNext`/`BrowseRun` 三处共用(找 tab 对应的表/集合定位信息、发起异步查询、更新 `run_seq`):

```rust
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
    let (where_clause, order_by, page, page_size) =
        (b.where_clause.clone(), b.order_by.clone(), b.page, b.page_size);
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
            let result = browse_table(
                kind,
                &url,
                table_schema.as_deref(),
                &table,
                &where_clause,
                &order_by,
                page,
                page_size,
            )
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
```

**关于 `run_seq` 防线怎么落地**:`BrowseState::begin_run()`/`is_current_run()`(Task 1)已经把判断逻辑封装好了。上面 `dispatch_browse_run`/`Message::QueryRun` 分支拿到 `seq` 后**没有**在本函数里立刻用来过滤——真正的核对发生在 Task 6 的 `database_browse_result`/`database_query_result`(app.rs 特化 handler,那里能拿到 `project_id` 路由到正确 workspace 之后,再读一次 `BrowseState`/`QueryState` 当前的 `run_seq` 跟结果消息里带的对比)。这里的 `let _ = seq;` 只是占位提醒"这个值真正被消费的地方在 Task 6",不是遗漏——**Task 6 必须记得用它**,否则整条过期防线就是摆设。

- [ ] **Step 3: 源删除/编辑联动关闭 tab**

找到 `Message::DeleteSource(id)` 分支(阶段 1 已有),在清 `ws_state.schemas`/`browsing` 那几行之后加一行:

```rust
Message::DeleteSource(id) => {
    ws_state.sources.retain(|s| s.id != id);
    ws_state.test_status.remove(&id);
    ws_state.schemas.remove(&id);
    ws_state.content.close_by_source(&id); // 新增
    if ws_state.browsing.as_deref() == Some(id.as_str()) {
        ws_state.browsing = None;
    }
    // ...(阶段 1 原有 keyring 删除/save_sources 不动)
}
```

找到 `Message::DraftSave` 分支里"编辑已有源"那段(阶段 2 已有 `if draft.id.is_some() { ws_state.schemas.remove(&id); ... }`),同样加一行:

```rust
if draft.id.is_some() {
    ws_state.schemas.remove(&id);
    ws_state.content.close_by_source(&id); // 新增
    if ws_state.browsing.as_deref() == Some(id.as_str()) {
        ws_state.browsing = None;
    }
}
```

- [ ] **Step 4: 单测**

```rust
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
        update_with(&mut ws, Message::BrowseWhereChanged(tab_id, "id > 1".into()));
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
}
```

- [ ] **Step 5: 验证**

```bash
cargo test -p dozer-app
cargo build -p dozer-app
cargo clippy -p dozer-app --all-targets
cargo fmt --check
```

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/extensions/database.rs
git commit -m "feat(dozer-app): add database content-pane message handling and state machine"
```

---

### Task 6: `app.rs` 内核接线

**Files:**
- Modify: `crates/dozer-app/src/app.rs`

**Interfaces:**
- Consumes:Task 1-5 全部(`database::Message::BrowseResult`/`QueryResult`、`database::update()`、`loaded_workspace_mut`、`PanelDims`、`Divider`、`apply_column_drag`、`split_portions`)。
- Produces:`PanelDims::database_split`、`Divider::DatabaseSplit`、`HoverId::DatabaseTabItem(usize)`/`DatabaseTabClose(usize)`、`panel_body()` 里 `PanelKind::Database` 的三栏布局、`database_browse_result()`/`database_query_result()` 两个 handler——Task 7/8 的视图函数被这里调用。

- [ ] **Step 1: `PanelDims` 加字段**

`PanelDims` struct 定义(约第 372 行)追加字段:

```rust
    /// 数据库面板配对:schema 树占左面板区宽度的比例,右侧内容窗格(表/
    /// 集合/查询 tab)拿剩下的。
    pub database_split: f32,
```

`default_panel_dims()`(约第 404 行)追加:

```rust
        database_split: byteui::theme::geometry::default_split_ratio(),
```

`sanitize_panel_dims()`(约第 486 行)追加:

```rust
        database_split: clamp_split(d.database_split),
```

- [ ] **Step 2: `Divider` 加变体 + `apply_column_drag` 分支**

`Divider` 枚举(约第 518 行)追加:

```rust
    /// 数据库面板内部的分隔线:左边 schema 树,右边表/集合/查询内容窗格。
    DatabaseSplit,
```

`apply_column_drag()` 里,照抄 `Divider::SshSplit` 分支(约第 846 行)的结构加一条:

```rust
        Divider::DatabaseSplit => {
            let side = state.layout.rail_layout.side_of(PanelKind::Database);
            let (x0, pair_w) = pair_x0_and_width(side, window_width, &state);
            if pair_w <= 0.0 {
                return state.dims;
            }
            let raw_ratio = ((logical_x - x0) / pair_w).clamp(
                byteui::theme::geometry::min_split_ratio(),
                byteui::theme::geometry::max_split_ratio(),
            );
            let mirrored = side != PanelKind::Database.default_side();
            let ratio = if list_rendered_first(true, mirrored) {
                raw_ratio
            } else {
                1.0 - raw_ratio
            };
            PanelDims {
                database_split: ratio,
                ..state.dims
            }
        }
```

- [ ] **Step 3: `HoverId` 加两个变体**

`HoverId` 枚举(约第 140 行)追加,照抄 `PreviewTabItem`/`PreviewTabClose` 的写法:

```rust
    /// 数据库内容窗格 tab 栏:某个 tab 本体的悬停(按索引区分,同
    /// `PreviewTabItem`)。
    DatabaseTabItem(usize),
    /// 数据库内容窗格 tab 栏:某个 tab 关闭按钮 `×` 的悬停。
    DatabaseTabClose(usize),
```

- [ ] **Step 4: `panel_body()` 里 `PanelKind::Database` 改三栏布局**

找到现有分支(约第 6909 行):

```rust
PanelKind::Database => {
    if ws.project.is_none() {
        return column![].into();
    }
    database::view(
        &app.database,
        &ws.database,
        Length::Fill,
        zone_pane_border(zone, ac),
        app.hover_progress(HoverId::DatabaseSchemaBack),
    )
    .map(Message::Database)
}
```

改成(结构照抄 `PanelKind::Ssh` 分支,约第 6924 行,含 mirrored 支持):

```rust
PanelKind::Database => {
    if ws.project.is_none() {
        return column![].into();
    }
    let (list_portion, content_portion) = split_portions(app.dims.database_split);
    let list_pane = database::view(
        &app.database,
        &ws.database,
        Length::FillPortion(list_portion),
        zone_pane_border(zone, lc),
        app.hover_progress(HoverId::DatabaseSchemaBack),
    )
    .map(Message::Database);
    let content_pane = database::content_pane(
        app,
        &ws.database,
        Length::FillPortion(content_portion),
        zone_pane_border(zone, rc),
    )
    .map(Message::Database);
    let list_bg = theme::region::project_pane()
        .background
        .unwrap_or(byteui::theme::color::current().bg);
    let content_bg = theme::region::preview_pane()
        .background
        .unwrap_or(byteui::theme::color::current().bg);
    if app.panel_mirrored(PanelKind::Database) {
        row![
            content_pane,
            divider_bar(
                Divider::DatabaseSplit,
                content_bg,
                list_bg,
                Message::ColumnDragStart(Divider::DatabaseSplit),
            ),
            list_pane,
        ]
        .width(Length::Fill)
        .into()
    } else {
        row![
            list_pane,
            divider_bar(
                Divider::DatabaseSplit,
                list_bg,
                content_bg,
                Message::ColumnDragStart(Divider::DatabaseSplit),
            ),
            content_pane,
        ]
        .width(Length::Fill)
        .into()
    }
}
```

`lc`/`rc`/`ac`/`zone` 几个局部变量沿用这个 `match` 分支作用域里已有的绑定(同 `PanelKind::Ssh`/`PanelKind::Project` 分支一样直接用,不用自己重新算)。`database::content_pane` 是 Task 7 才会定义的函数——这一步先按上面的签名写调用点,Task 7 落地函数体后才能编译通过(这是这份计划里唯一一处"调用点先于定义"的情况,和阶段 2 的排期备注一致:两个任务之间会有短暂编译不过的中间态,属于同一个 PR 内的正常过程,不单独验证这一步)。

- [ ] **Step 5: 两个 `project_id` 路由特化 handler + `App::update` 特化臂**

照抄 `database_test_connection_result`(约第 5090 行)的结构——**不是**直接调 `database::apply_browse_result`,而是把收到的消息原样重新塞回 `database::update()`,让 Task 5 里写好的 `Message::BrowseResult`/`QueryResult` 分支(`run_seq` 核对 + 落地都已经在那两个分支里)去处理,和 `TestConnectionResult`/`TablesLoaded`/`ColumnsLoaded` 三个既有 handler 的做法完全一致。在同一个 `impl App` 块里追加:

```rust
    fn database_browse_result(
        &mut self,
        project_id: i64,
        tab_id: usize,
        run_seq: u64,
        result: Result<database::BrowsePage, String>,
    ) {
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
            database::Message::BrowseResult(project_id, tab_id, run_seq, result),
            project_id,
            &repo_path,
            &handle,
            emit,
        );
    }

    fn database_query_result(
        &mut self,
        project_id: i64,
        tab_id: usize,
        run_seq: u64,
        result: Result<database::QueryOutcome, String>,
    ) {
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
            database::Message::QueryResult(project_id, tab_id, run_seq, result),
            project_id,
            &repo_path,
            &handle,
            emit,
        );
    }
```

`App::update` 里,找到阶段 1/2 现有的 `Message::Database(database::Message::TestConnectionResult(...))`/`TablesLoaded(...)`/`ColumnsLoaded(...)` 三条特化臂(约第 3980-4000 行),紧接着追加两条,**必须继续排在通配 `Message::Database(msg) => self.database_message(msg)` 之前**:

```rust
            Message::Database(database::Message::BrowseResult(project_id, tab_id, run_seq, result)) => {
                self.database_browse_result(project_id, tab_id, run_seq, result)
            }
            Message::Database(database::Message::QueryResult(project_id, tab_id, run_seq, result)) => {
                self.database_query_result(project_id, tab_id, run_seq, result)
            }
```

（`database::apply_browse_result`/`apply_query_result` 这两个函数本身仍按 Task 5 写的样子保留在 `database.rs` 里、供 `update()` 内部的 `Message::BrowseResult`/`QueryResult` 分支调用——不需要 `pub(crate)`,因为 `app.rs` 这一步并不直接调它们,只是重新调用 `database::update()`。）

- [ ] **Step 6: 验证**

```bash
cargo build -p dozer-app
```

Expected: 这一步结束时还编译不过(`database::content_pane` 函数体还没写,Task 7 才补)——**这是预期的**,先跑 `cargo build -p dozer-app 2>&1 | grep "content_pane"` 确认唯一的报错就是这个函数不存在,不要有其它意外报错。`cargo clippy`/`cargo fmt --check` 这一步跳过(编译都过不了跑不了),留到 Task 7 结束后一起补。

- [ ] **Step 7: Commit**

```bash
git add crates/dozer-app/src/app.rs crates/dozer-app/src/extensions/database.rs
git commit -m "feat(dozer-app): wire database content pane into kernel (panel_body/Divider/HoverId/async routing)"
```

（这个 commit 之后仓库处于短暂编译不过的中间态,下一个任务紧接着修复——同阶段 2 计划"排期"一节对相邻任务边界的既有口径,不是异常。）

---

### Task 7: 视图——内容窗格骨架 + 浏览页 + schema 树点击行为改造

**Files:**
- Modify: `crates/dozer-app/src/extensions/database.rs`

**Interfaces:**
- Consumes:Task 1-6 全部类型/消息;`crate::tab_widget::{panel_tab, tab_arrow_button, tab_window}`;`byteui::form::select::view`(页大小下拉);`byteui::form::input_text::view`(WHERE/ORDER BY 框)。
- Produces:`content_pane()`(Task 6 的 `panel_body` 调用点消费)。

- [ ] **Step 1: `content_pane` 骨架 + tab 栏**

```rust
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
    let widths: Vec<f32> = content
        .tabs()
        .iter()
        .map(|t| crate::tab_widget::PANEL_TAB_MAX_W.min(tab_title_display_width(&tab_title(t, ws_state))))
        .collect();
    let (first, can_left, can_right) = crate::tab_widget::tab_window(
        &widths,
        4.0,
        byteui::theme::geometry::tab_bar_avail_px(),
        content.tab_scroll_first(),
    );

    let items: Vec<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>> = content
        .tabs()
        .iter()
        .enumerate()
        .filter(|(idx, _)| *idx >= first)
        .map(|(idx, tab)| {
            let active = idx == content.active_idx();
            let title_hover_t = app.hover_progress(crate::app::HoverId::DatabaseTabItem(idx));
            let close_hover_t = app.hover_progress(crate::app::HoverId::DatabaseTabClose(idx));
            crate::tab_widget::panel_tab(
                tab_title(tab, ws_state),
                active,
                title_hover_t,
                close_hover_t,
                None,
                None,
                Message::SelectTab(idx),
                Message::CloseTab(idx),
                app.hover_tooltip_ready(crate::app::HoverId::DatabaseTabItem(idx)),
                move |h| Message::Hover(crate::app::HoverId::DatabaseTabItem(idx), h),
                move |h| Message::Hover(crate::app::HoverId::DatabaseTabClose(idx), h),
            )
        })
        .collect();
    let tabs_row = row(items).spacing(4);
    let clipped = container(tabs_row).width(Length::Fill).clip(true);
    let left_arrow =
        crate::tab_widget::tab_arrow_button(icons::IconKind::ChevronLeft, can_left, Message::TabScroll(false));
    let right_arrow =
        crate::tab_widget::tab_arrow_button(icons::IconKind::ChevronRight, can_right, Message::TabScroll(true));
    let tab_bar = row![left_arrow, right_arrow, clipped]
        .spacing(4)
        .align_y(iced_widget::core::Alignment::Center);

    let mut col = column![tab_bar].spacing(8);

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

    container(col.padding(16))
        .width(width)
        .height(iced_widget::core::Length::Fill)
        .style(move |_t: &iced_widget::Theme| iced_widget::container::Style {
            background: Some(byteui::theme::color::current().bg.into()),
            border: outer,
            ..iced_widget::container::Style::default()
        })
        .into()
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
```

`tab_bar_avail_px()`、`PANEL_TAB_MAX_W` 已确认是既有导出(`byteui::theme::geometry::tab_bar_avail_px()`、`crate::tab_widget::PANEL_TAB_MAX_W`)。如果 `PANEL_TAB_MAX_W` 当前是 `pub(crate)` 但不在 `tab_widget` 模块外可见到 `extensions::database` 子模块(同一个 crate 内 `pub(crate)` 应该没问题,`extensions` 是 `dozer-app` crate 内部子模块)——`cargo build` 报可见性错误的话,把 `tab_widget.rs` 里对应项从 `pub(crate)` 改成 `pub(crate)` 已经是 crate 内可见,不需要改;如果确实报错,检查是不是函数名/路径打错,不是可见性问题。

- [ ] **Step 2: `browse_view`(WHERE/ORDER BY/分页/结果网格)**

```rust
fn browse_view<'a>(
    tab_id: usize,
    b: &'a BrowseState,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let toolbar = row![
        byteui::form::input_text::view(
            "WHERE(原始 SQL 片段,例如 id > 100)",
            &b.where_clause,
            false,
            None,
            false,
            Some(Message::BrowseRun(tab_id)),
            false,
            move |v| Message::BrowseWhereChanged(tab_id, v),
        ),
        byteui::form::input_text::view(
            "ORDER BY(原始 SQL 片段,例如 title DESC)",
            &b.order_by,
            false,
            None,
            false,
            Some(Message::BrowseRun(tab_id)),
            false,
            move |v| Message::BrowseOrderByChanged(tab_id, v),
        ),
        byteui::form::select::view(&PAGE_SIZES, Some(&b.page_size), move |v| {
            Message::BrowsePageSizeChanged(tab_id, v)
        }),
        button(text("上一页"))
            .on_press_maybe((b.page > 0).then_some(Message::BrowsePrev(tab_id))),
        button(text("下一页"))
            .on_press_maybe(b.has_more.then_some(Message::BrowseNext(tab_id))),
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
```

- [ ] **Step 3: schema 树点击行为改造**

`schema_tree_view`(阶段 2 已有)调 `schema_tree_row` 的地方(约"let mut tree = column![]..."那段)加一个参数:

```rust
for r in tree_rows(st, source.driver) {
    tree = tree.push(schema_tree_row(&source.id, source.driver, r));
}
```

`schema_tree_row` 签名加 `driver: DriverKind` 参数,`SchemaRowKind::Table(t)` 分支从"整行一个 button"拆成"chevron 按钮(收展列)+ 文字区按钮(开 tab)"两个可点区域;MongoDB 驱动不画 chevron、点文字区直接开集合 tab:

```rust
fn schema_tree_row<'a>(
    source_id: &str,
    driver: DriverKind,
    r: SchemaRow<'a>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let indent = text("  ".repeat(r.depth)).size(crate::workspace::tree_row_font_size());
    match r.kind {
        SchemaRowKind::Schema(name) => {
            // ...(阶段 2 原有实现不动)
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
                row_el = row_el.push(iced_widget::space::Space::new().width(Length::Fixed(
                    byteui::theme::icon_size::chevron(),
                )));
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
            row_el.push(label_btn).align_y(iced_widget::core::Alignment::Center).into()
        }
        SchemaRowKind::Column(c) => {
            // ...(阶段 2 原有实现不动)
        }
        SchemaRowKind::ColumnsLoading => {
            // ...(阶段 2 原有实现不动)
        }
        SchemaRowKind::ColumnsFailed(e) => {
            // ...(阶段 2 原有实现不动)
        }
    }
}
```

`schema_tree_view` 头部工具行(阶段 2 已有,`back_button`/源名/驱动标签/`Space`/"刷新"按钮那一行)加"+ 新查询"按钮,MongoDB 源不显示:

```rust
let mut header = row![
    back_button,
    text(source.name.clone())
        .size(byteui::theme::font::subtitle())
        .color(byteui::theme::color::current().cream),
    text(source.driver.label())
        .size(byteui::theme::font::caption_sm())
        .color(byteui::theme::color::current().dim),
    iced_widget::space::horizontal(),
]
.spacing(8)
.align_y(iced_widget::core::Alignment::Center);
if source.driver != DriverKind::MongoDB {
    header = header.push(
        button(text("+ 新查询")).on_press(Message::OpenQueryTab(source.id.clone())),
    );
}
header = header.push(button(text("刷新")).on_press(Message::SchemaRefresh(source.id.clone())));
```

（把阶段 2 原有的 `let header = row![...]` 那句替换成上面这段;`header` 从 `let` 改成 `let mut`。）

- [ ] **Step 4: 验证**

```bash
cargo build -p dozer-app
cargo test -p dozer-app
cargo clippy -p dozer-app --all-targets
cargo fmt --check
```

Expected: 这一步之后 Task 6 遗留的编译错误应该清零(`content_pane` 已定义)。如果 clippy 对 `result_table`/`browse_view` 里的闭包类型标注啰嗦报 warning(常见于 iced 视图代码的 `Vec<Element<...>>` collect),按 clippy 建议加类型标注或 `#[allow]` 精确到那一行,不要整个函数级 `#[allow]`。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/extensions/database.rs
git commit -m "feat(dozer-app): add database content-pane shell and table/collection browse view"
```

---

### Task 8: 视图——SQL 查询控制台

**Files:**
- Modify: `crates/dozer-app/src/extensions/database.rs`

**Interfaces:**
- Consumes:`QueryState`(Task 1)、`byteui::form::text_area::view`。
- Produces:`query_view()`(Task 7 的 `content_pane` 消费)。

- [ ] **Step 1: `query_view`**

```rust
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

    let editor = byteui::form::text_area::view(
        &q.sql,
        "SELECT * FROM ...",
        None,
        false,
        Some(160.0),
        move |action| Message::QueryTextAction(tab_id, action),
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
```

**Cmd+Enter 快捷执行**:这一步先只做按钮(`Message::QueryRun(tab_id)`),键盘快捷键需要接内核的全局按键分发(同 `⌘K` 之类快捷键的既有接线位置)。搜索 `app.rs` 里现有的 `key_pressed`/`on_key_press`/`Modifiers` 相关处理入口(和"打开命令面板"同一处),加一条:当前聚焦是数据库面板、且激活 tab 是查询 tab、且按下 Cmd+Enter → 派发 `Message::Database(database::Message::QueryRun(tab_id))`。这处内核接线的具体函数名/位置在写这份计划时没有专门确认(不在设计文档必做范围,属于体验优化),**如果时间/优先级不够,先跳过、只留按钮**,不影响功能完整性——按钮本身已经覆盖设计文档"目标"里的执行入口要求。

- [ ] **Step 2: 验证**

```bash
cargo build -p dozer-app
cargo test -p dozer-app
cargo clippy -p dozer-app --all-targets
cargo fmt --check
```

- [ ] **Step 3: Commit**

```bash
git add crates/dozer-app/src/extensions/database.rs
git commit -m "feat(dozer-app): add SQL query console view"
```

---

### Task 9: 全量验证 + 人工验收

**Files:** 无代码改动(除人工验收过程中发现问题需要回头小修)。

- [ ] **Step 1: 全量自动化验证**

```bash
cargo build --workspace
cargo test -p dozer-app
cargo clippy --all-targets
cargo fmt --check
```

Expected: 全绿,零 warning(Task 1-5 阶段允许的 `dead_code` 过渡态到这里应该已经清零——如果还有,说明某个 Task 7/8 该接的调用点漏接了,回头补)。

- [ ] **Step 2: 人工验收清单**

对照设计文档"测试策略"一节的人工验收范围,逐条走一遍(需要本机能连到至少一个 Postgres、一个 MySQL、一个 MongoDB 实例;SQLite 用任意本地文件):

- [ ] SQLite/Postgres/MySQL 各配一个数据源,"浏览结构" → 点一张有数据的表 → 右侧开 tab、显示分页数据网格。
- [ ] 翻页(上一页/下一页)行为正确,最后一页"下一页"变灰。
- [ ] WHERE 框输入条件回车 → 结果按条件过滤;ORDER BY 框同理。
- [ ] 点列头 → ORDER BY 框写入 `"{列名} ASC"` 且结果重新排序。
- [ ] 同一张表再点一次(树里或已开的 tab)→ 聚焦已开 tab,不重复开。
- [ ] MongoDB 源:填好数据库名后"浏览结构" → 平铺集合列表(无 schema/列层)→ 点集合 → 右侧分页显示文档 JSON。
- [ ] MongoDB 源数据库名留空时点集合浏览 → 看到"请在数据源配置里填写数据库名"提示,不崩溃。
- [ ] "+ 新查询" → 开查询 tab,执行 `SELECT * FROM <某表> LIMIT 5` → 结果网格正确显示,含 `NULL` 列显示为 `NULL`(非空字符串)。
- [ ] 查询控制台执行 `UPDATE`/`DELETE` → 显示"N 行受影响";执行 `CREATE TABLE`/`DROP TABLE` → 显示"执行成功"。
- [ ] 查询控制台执行语法错误的 SQL → 显示驱动原始报错,不崩溃、不清空编辑器内容。
- [ ] 同一数据源开两个查询 tab,标题分别是"查询 1@..."/"查询 2@..."。
- [ ] 关闭一个 tab(点 ×)不影响其它已开 tab 的状态。
- [ ] 删除/编辑保存一个正被浏览的数据源 → 该源关联的所有 tab(浏览+查询)一并关闭。
- [ ] 拖拽数据库面板内部的分隔线(schema 树 ↔ 内容窗格)宽度可调,刷新/切项目后记住上次比例。
- [ ] 真实类型覆盖:Postgres 建一张含 `uuid`/`timestamp`/`jsonb`/`numeric` 列的表,浏览/查询都能正常显示这些列的值(不是清一色"不支持的类型")。
- [ ] 连续快速两次改 WHERE 框内容并各回车一次(制造竞态)→ 最终显示的是最后一次查询的结果,不被更早那次的结果覆盖(`run_seq` 防线生效;人工测试可以故意先输入一个会命中大表全扫描、执行较慢的条件,再立刻改成一个简单条件,观察最终停在哪个结果上)。
- [ ] 加载期间切换项目页签,结果不会跑到别的项目的 tab 上(`project_id` 路由生效)。

- [ ] **Step 3: 收尾**

人工验收全部通过后,按 `superpowers:finishing-a-development-branch` 的既定流程处理 `feature/database-panel-data-browsing` 分支(提请审阅、合并)。不在这份计划里展开——那是另一个技能的职责范围。

---

## 自查记录(写计划时的 fresh-eyes 复查)

- **spec 覆盖**:设计文档"目标"1-5 条分别对应 Task 7(表格浏览)、Task 4+7(MongoDB 浏览)、Task 3+8(SQL 控制台)、Task 7(tab 骨架复用)、Task 2-3(值解码)——逐条有落点。"非目标"清单已整段搬进 Global Constraints。
- **占位符扫描**:Task 7 初稿里 tab 栏箭头翻页错误地复用了 `SelectTab(0)`/`SelectTab(len-1)`(选中首/末 tab,不是翻页),且 `result_table` 列头闭包写出过一行不合法的 `.into().into().let _ = name;`。两处都已直接改正:`DatabaseContentState` 补了 `tab_scroll_first` 字段 + `scroll_tabs()`(Task 1)、`Message::TabScroll(bool)`(Task 5)、箭头按钮改接 `Message::TabScroll`(Task 7);`result_table` 换成干净版本。计划正文里已经是修正后的最终版本,不是"先给错的再给对的"。
- **类型一致性**:`DatabaseTabKind`/`TabContent`/`BrowseState`/`QueryState`/`QueryResult`/`CellValue`/`BrowsePage`/`QueryOutcome` 从 Task 1 定义后,Task 2-8 全程复用同一套名字,没有出现"Task 3 叫 `BrowsePage` 但 Task 7 叫 `BrowsePageResult`"这类漂移。
- **正确性缺口(写计划过程中发现并修正)**:Task 1 给 `BrowseState`/`QueryState` 设计了 `run_seq`/`is_current_run()` 防"同一 tab 连续两次触发查询"的竞态,但最初写 Task 6 时图省事,改用了"落地前查一下 `loading`/`running` 布尔标志"——这个简化实际上**不能防住**设计要防的那个场景(两个请求都在飞时,布尔标志分辨不出结果该配对给哪一次请求,后落地的会覆盖先落地的,不管谁新谁旧)。已改正:`Message::BrowseResult`/`QueryResult` 加了 `run_seq: u64` 字段,`database::update()` 里的两个分支用 `is_current_run(seq)` 真正做数字比对(Task 5);顺带发现 `app.rs` 的异步结果 handler 不该直接调 `apply_browse_result`/`apply_query_result`,而要照抄 `database_test_connection_result` 的既有模式——把消息整个重新塞回 `database::update()`,让 Task 5 写的分支去处理(Task 6,连带去掉了一个多余的 `WorkspaceState::content_mut()` 访问器,YAGNI)。
