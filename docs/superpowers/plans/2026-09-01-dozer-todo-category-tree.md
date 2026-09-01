# Todo 任务分类树 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 给 Todo 面板加一棵用户自定义的树状分类结构:单项目内、单一归属、左侧树导航 + 右侧过滤列表,分类的增删改/reparent/排序全走右键菜单和点击选择器(不做拖拽),任务行加一个可点击的分类标签。

**Architecture:** `dozerd` 新增独立的 `CategoryStore`(与既有 `TodoStore` 平级,共享 `dozer.db`),`todos` 表加一列 `category_id`;协议层加一组 `Category*` `Request`/`Reply`;`dozer-client` 加对称的七个薄方法;`dozer-app` 侧在 `extensions/todo.rs` 已有的"左栏 sidebar_pane(header+分类导航+底部计数栏)/右栏 content_pane(任务列表)"两栏结构里,把分类树接到左栏既有状态过滤导航下方,任务行新增分类 chip;树的内存表示照抄 `project.rs::FileTree`/`TreeRow` 的"扁平存储+展开集+拍平成行"模式;右键菜单/分类选择器浮层照抄 `app.rs::ProjectLinkMenu`(`crate::menu::shell`/`item` + `stack![base, dismiss, popup]`)的既有惯例,状态挂在 `App` 上而不是 `WorkspaceState` 上。

**Tech Stack:** Rust, rusqlite(SQLite), tokio, iced 0.14(`iced_widget`/`iced_renderer`), `byteui` 共享组件库。

**Spec:** `docs/superpowers/specs/2026-09-01-dozer-todo-category-tree-design.md`

## Global Constraints

- 在独立分支 `feature/todo-category-tree` 上开发,**不要直接提交到 main**;全部任务完成、自测通过后提请审阅,审阅通过后再合并回 main(参考已有先例:`git worktree add ../dozer-todo-category-tree feature/todo-category-tree` 或本地 `git checkout -b`,二选一均可,不强制用 worktree)。
- 每个任务完成后运行 `cargo build`(改动涉及的 crate 至少要过,收尾任务再跑全 workspace)、`cargo test -p <改动 crate>`、`cargo clippy --all-targets -- -D warnings`、`cargo fmt`,四项全绿才算任务完成。
- 分类树本期**不做拖拽**、**不给 dozer-mcp 开放写工具**、**不支持多分类标签**、**不跨项目共享**——这四条是本次范围的硬边界,任何任务都不应该悄悄引入这些能力(详见 spec 非目标一节)。
- 新增 icon 按钮复用 `byteui::interaction::icons::icon_button_entry`;右键菜单复用 `crate::menu::shell`/`crate::menu::item`;不重新手写 `MouseArea` + 自绘菜单。
- `TodoInfo`/`CategoryInfo` 等协议结构体新增字段一律带 `#[serde(default)]`(对齐 `SessionInfo` 现有的 `project_id`/`agent_state` 等字段的前例),保证协议帧的前向兼容。

---

## Task 1: 协议层——`CategoryInfo`/`CategoryMoveDirection` + 七组 `Request`/`Reply`

**Files:**
- Modify: `crates/dozer-core/src/protocol.rs`

**Interfaces:**
- Produces: `pub struct CategoryInfo { id: i64, project_id: i64, parent_id: Option<i64>, name: String, rank: i64, created_ms: u64 }`;`pub enum CategoryMoveDirection { Up, Down }`;`Request::{ListCategories,AddCategory,RenameCategory,DeleteCategory,ReparentCategory,MoveCategorySibling,SetTodoCategory}`;`Reply::{Categories{categories},Category{category}}`(`SetTodoCategory` 复用既有 `Reply::Todo`,`DeleteCategory` 复用既有 `Reply::Ok`);`TodoInfo.category_id: Option<i64>`。这些类型名/字段名是后续全部任务(dozerd/dozer-client/dozer-app)的唯一依据。

- [ ] **Step 1: 在 `TodoInfo` 后面加 `CategoryInfo`/`CategoryMoveDirection`,给 `TodoInfo` 加 `category_id` 字段**

在 `crates/dozer-core/src/protocol.rs` 里 `TodoInfo` 结构体定义(现有第 200-218 行)之后插入:

```rust
/// 一个分类树节点(`dozerd` 的 `todo_categories` 表一行)。`parent_id ==
/// None` 表示顶层节点。作用域按 `project_id` 隔离,不跨项目共享
/// (2026-09-01 分类树设计)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CategoryInfo {
    pub id: i64,
    pub project_id: i64,
    pub parent_id: Option<i64>,
    pub name: String,
    /// 同一 `parent_id` 下的兄弟排序键,升序展示。
    pub rank: i64,
    pub created_ms: u64,
}

/// `MoveCategorySibling` 的方向:与同一 `parent_id` 下相邻的前一个/
/// 后一个兄弟节点交换 `rank`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CategoryMoveDirection {
    Up,
    Down,
}
```

然后给 `TodoInfo` 加一个字段(紧跟在现有 `dispatch_at_ms: Option<u64>,` 之后,`}` 之前):

```rust
    /// 所属分类节点 id,`None` = 未分类(2026-09-01 分类树设计新增)。
    #[serde(default)]
    pub category_id: Option<i64>,
```

- [ ] **Step 2: 在 `Request` 枚举里加七个变体**

在现有 `Request` 枚举的 `RecordTodoDispatch { .. }` 变体(现有第 450-453 行)之后、枚举收尾 `}` 之前插入:

```rust
    /// 列出某项目全部分类节点,扁平返回(不分页,数据量小)。
    ListCategories {
        project_id: i64,
    },
    /// 新增分类,追加到 `parent_id` 下兄弟节点末尾。
    AddCategory {
        project_id: i64,
        parent_id: Option<i64>,
        name: String,
    },
    /// 重命名。`id` 不存在 → `Reply::Error`。
    RenameCategory {
        id: i64,
        name: String,
    },
    /// 删除该节点及其全部子孙节点;原本挂在这棵子树下的任务全部降级为
    /// 未分类(`category_id = NULL`),任务本身不删除。`id` 不存在 →
    /// `Reply::Error`。
    DeleteCategory {
        id: i64,
    },
    /// 重新挂到 `new_parent_id` 下(`None` = 顶层),追加到新父节点子级
    /// 末尾。目标是自己或自己的子孙时 → `Reply::Error`(防止成环)。
    ReparentCategory {
        id: i64,
        new_parent_id: Option<i64>,
    },
    /// 与同一 `parent_id` 下相邻的前一个/后一个兄弟节点交换 `rank`。已经
    /// 在最前/最后时对应方向是 no-op(仍返回 `Reply::Category`,不报错)。
    MoveCategorySibling {
        id: i64,
        direction: CategoryMoveDirection,
    },
    /// 挂/摘任务的分类。`category_id: None` = 摘掉分类,变回未分类。
    SetTodoCategory {
        id: i64,
        category_id: Option<i64>,
    },
```

- [ ] **Step 3: 在 `Reply` 枚举里加两个变体**

在现有 `Reply` 枚举的 `Todo { todo: TodoInfo }`(现有第 557-559 行)之后、枚举收尾 `}` 之前插入:

```rust
    Categories {
        categories: Vec<CategoryInfo>,
    },
    Category {
        category: CategoryInfo,
    },
```

- [ ] **Step 4: 写协议往返测试**

在文件末尾 `#[cfg(test)] mod tests` 块(现有 572 行起)里追加:

```rust
    #[test]
    fn category_protocol_types_roundtrip() {
        let req = Request::AddCategory {
            project_id: 1,
            parent_id: Some(2),
            name: "前端".into(),
        };
        let line = encode_line(&req);
        let decoded: Request = decode_line(&line).unwrap();
        assert_eq!(req, decoded);

        let category = CategoryInfo {
            id: 10,
            project_id: 1,
            parent_id: Some(2),
            name: "前端".into(),
            rank: 0,
            created_ms: 1_700_000_000_000,
        };
        let reply = Reply::Category {
            category: category.clone(),
        };
        let line = encode_line(&reply);
        let decoded: Reply = decode_line(&line).unwrap();
        assert_eq!(reply, decoded);

        let list_reply = Reply::Categories {
            categories: vec![category],
        };
        let line = encode_line(&list_reply);
        let decoded: Reply = decode_line(&line).unwrap();
        assert_eq!(list_reply, decoded);
    }

    #[test]
    fn todo_info_category_id_defaults_to_none_when_absent_from_json() {
        // 老协议帧没有 category_id 字段,新增字段要能优雅缺省,不报错。
        let json = r#"{"id":1,"project_id":1,"text":"任务","done":false,"rank":0,
            "created_ms":0,"completed_at_ms":null,"plan_date":null,
            "dispatch_session_id":null,"dispatch_at_ms":null}"#;
        let todo: TodoInfo = serde_json::from_str(json).unwrap();
        assert_eq!(todo.category_id, None);
    }
```

- [ ] **Step 5: 编译 + 跑测试**

Run: `cargo test -p dozer-core`
Expected: 全部通过,包括新增的 `category_protocol_types_roundtrip`/
`todo_info_category_id_defaults_to_none_when_absent_from_json`。

Run: `cargo build` (全 workspace)
Expected: **编译失败**——`dozerd`/`dozer-client` 里 match `Request`/`Reply`
的地方现在不是穷尽的(缺新变体的处理分支)。这是预期的中间状态,后续
任务会逐一补上;确认失败信息只指向这些遗漏分支,不是别的错误。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-core/src/protocol.rs
git commit -m "feat(dozer-core): 协议层新增分类树 Request/Reply/CategoryInfo"
```

---

## Task 2: `dozerd` — `todos` 表加 `category_id` 列 + `TodoStore::set_category`

**Files:**
- Modify: `crates/dozerd/src/todo.rs`

**Interfaces:**
- Consumes: `dozer_core::protocol::TodoInfo`(Task 1 新增的 `category_id` 字段)。
- Produces: `TodoStore::set_category(&self, id: i64, category_id: Option<i64>) -> Result<TodoInfo>`——后续 Task 4(server.rs 接线)依赖这个方法名和签名。

- [ ] **Step 1: 给 `TODO_COLUMNS`/`row_to_todo` 加 `category_id`,`CREATE TABLE`/迁移逻辑加列**

修改 `TODO_COLUMNS` 常量(现有第 31-32 行):

```rust
const TODO_COLUMNS: &str = "id, project_id, text, done, rank, created_ms, \
    completed_at_ms, plan_date, dispatch_session_id, dispatch_at_ms, category_id";
```

修改 `row_to_todo`(现有第 34-47 行),在 `dispatch_at_ms` 字段后加一行:

```rust
fn row_to_todo(row: &rusqlite::Row) -> rusqlite::Result<TodoInfo> {
    Ok(TodoInfo {
        id: row.get(0)?,
        project_id: row.get(1)?,
        text: row.get(2)?,
        done: row.get(3)?,
        rank: row.get(4)?,
        created_ms: row.get::<_, i64>(5)? as u64,
        completed_at_ms: row.get::<_, Option<i64>>(6)?.map(|v| v as u64),
        plan_date: row.get(7)?,
        dispatch_session_id: row.get(8)?,
        dispatch_at_ms: row.get::<_, Option<i64>>(9)?.map(|v| v as u64),
        category_id: row.get(10)?,
    })
}
```

修改 `TodoStore::new`(现有第 50-75 行),在 `execute_batch` 建表调用之后、
`Ok(Self { .. })` 之前,补上老库迁移探测(镜像
`transcripts/mod.rs::open_on_pre_existing_db_without_is_error_column_adds_it`
同款 `pragma_table_info` 先例):

```rust
        // 老库(建表时还没有 category_id 列)迁移:CREATE TABLE IF NOT
        // EXISTS 对已存在的表不生效,新列需要单独补。SQLite 的 ALTER
        // TABLE ADD COLUMN 没有 IF NOT EXISTS 语法,靠 PRAGMA table_info
        // 先查有没有再决定要不要补(同 transcripts.rs 的 is_error 列前例)。
        let has_category_id: bool = conn
            .prepare("SELECT 1 FROM pragma_table_info('todos') WHERE name = 'category_id'")?
            .exists([])?;
        if !has_category_id {
            conn.execute("ALTER TABLE todos ADD COLUMN category_id INTEGER", [])
                .context("迁移 category_id 列")?;
        }
```

- [ ] **Step 2: 给 `add`/`toggle`/`edit_text`/`set_plan_date`/`record_dispatch`/`reorder` 的 `RETURNING`/`SELECT` 语句换成新的 `TODO_COLUMNS`**

这些方法都用 `format!("... RETURNING {TODO_COLUMNS}")` 或
`format!("SELECT {TODO_COLUMNS} FROM todos ...")` 拼 SQL(现有第 90-205
行),已经引用常量,**不需要改动方法体本身**——只要 Step 1 改了
`TODO_COLUMNS` 常量,这些方法自动跟着变。这一步只是确认:跑一遍
`cargo build -p dozerd`,不应有这几个方法的编译错误。

Run: `cargo build -p dozerd`
Expected: 编译错误只出现在 `server.rs`(Task 1 Step 5 已预告的
match 不穷尽),`todo.rs` 自身应该干净编译。

- [ ] **Step 3: 写失败测试(先写测试,后写 `set_category`)**

在 `#[cfg(test)] mod tests`(现有第 208 行起)里,`tasks_are_isolated_per_project`
测试之后追加:

```rust
    #[test]
    fn set_category_assigns_and_clears() {
        let (_dir, store) = store();
        let t = store.add(1, "任务").unwrap();
        assert_eq!(t.category_id, None);

        let categorized = store.set_category(t.id, Some(42)).unwrap();
        assert_eq!(categorized.category_id, Some(42));

        let cleared = store.set_category(t.id, None).unwrap();
        assert_eq!(cleared.category_id, None);
    }

    #[test]
    fn set_category_unknown_id_errors() {
        let (_dir, store) = store();
        assert!(store.set_category(999, Some(1)).is_err());
    }
```

- [ ] **Step 4: 跑测试确认失败**

Run: `cargo test -p dozerd set_category`
Expected: FAIL,`error[E0599]: no method named `set_category` found`。

- [ ] **Step 5: 实现 `TodoStore::set_category`**

在 `impl TodoStore` 块里,`record_dispatch`(现有第 142-154 行)之后、
`reorder` 之前插入:

```rust
    /// 挂/摘任务的分类。`category_id: None` 摘掉分类(变回未分类)。
    /// **不校验** `category_id` 指向的分类是否存在——那是跨 store 的
    /// 校验,`TodoStore` 不知道 `CategoryStore` 的存在,交给
    /// `server.rs` 的请求处理器在调用前做(见 Task 4)。`id` 不存在
    /// 返回 `Err`。
    pub fn set_category(&self, id: i64, category_id: Option<i64>) -> Result<TodoInfo> {
        let conn = self.conn.lock().expect("db lock");
        let sql =
            format!("UPDATE todos SET category_id = ?1 WHERE id = ?2 RETURNING {TODO_COLUMNS}");
        conn.query_row(&sql, params![category_id, id], row_to_todo)
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => id_not_found(id),
                e => e.into(),
            })
    }
```

- [ ] **Step 6: 跑测试确认通过**

Run: `cargo test -p dozerd`
Expected: 全部通过(含新增两个测试),`todo.rs` 里既有测试(如
`add_list_and_toggle`)不受影响——它们不断言 `category_id`,而
`TodoInfo` 上新字段默认 `None` 不影响既有断言。

- [ ] **Step 7: Commit**

```bash
git add crates/dozerd/src/todo.rs
git commit -m "feat(dozerd): todos 表加 category_id 列 + TodoStore::set_category"
```

---

## Task 3: `dozerd` — 新增 `CategoryStore`(`todo_category.rs`)

**Files:**
- Create: `crates/dozerd/src/todo_category.rs`
- Modify: `crates/dozerd/src/lib.rs`

**Interfaces:**
- Consumes: `dozer_core::protocol::{CategoryInfo, CategoryMoveDirection}`(Task 1)。
- Produces: `pub struct CategoryStore`;`CategoryStore::new(path: &Path) -> Result<Self>`;`list(&self, project_id: i64) -> Result<Vec<CategoryInfo>>`;`add(&self, project_id: i64, parent_id: Option<i64>, name: &str) -> Result<CategoryInfo>`;`rename(&self, id: i64, name: &str) -> Result<CategoryInfo>`;`delete(&self, id: i64) -> Result<()>`;`reparent(&self, id: i64, new_parent_id: Option<i64>) -> Result<CategoryInfo>`;`move_sibling(&self, id: i64, direction: CategoryMoveDirection) -> Result<CategoryInfo>`。这些方法名/签名是 Task 4(server.rs 接线)的唯一依据。

- [ ] **Step 1: 建表 + `list`/`add`,先写测试**

创建 `crates/dozerd/src/todo_category.rs`:

```rust
//! 分类树存储:与 `TodoStore` 平级、共享同一个 `dozer.db`。一行一个
//! 分类节点,`parent_id` 表达树形结构;删除/reparent 的级联逻辑全部在
//! Rust 层手动实现(不用 SQLite 外键 `ON DELETE` 系列——级联规则本身
//! 是业务语义,见 2026-09-01 分类树设计)。

use anyhow::{Context, Result};
use dozer_core::protocol::{CategoryInfo, CategoryMoveDirection};
use rusqlite::{Connection, params};
use std::path::Path;
use std::sync::Mutex;

pub struct CategoryStore {
    conn: Mutex<Connection>,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn id_not_found(id: i64) -> anyhow::Error {
    anyhow::anyhow!("分类不存在: id={id}")
}

const CATEGORY_COLUMNS: &str = "id, project_id, parent_id, name, rank, created_ms";

fn row_to_category(row: &rusqlite::Row) -> rusqlite::Result<CategoryInfo> {
    Ok(CategoryInfo {
        id: row.get(0)?,
        project_id: row.get(1)?,
        parent_id: row.get(2)?,
        name: row.get(3)?,
        rank: row.get(4)?,
        created_ms: row.get::<_, i64>(5)? as u64,
    })
}

impl CategoryStore {
    pub fn new(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("建库目录")?;
        }
        let conn = Connection::open(path).context("打开 SQLite")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS todo_categories (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                project_id INTEGER NOT NULL,
                parent_id INTEGER,
                name TEXT NOT NULL,
                rank INTEGER NOT NULL,
                created_ms INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_todo_categories_project_parent
                ON todo_categories(project_id, parent_id, rank);",
        )
        .context("建表")?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// 某项目全部分类节点,扁平返回(树形结构由调用方按 `parent_id` 自己
    /// 拼)。数据量小,不做懒加载/分页。
    pub fn list(&self, project_id: i64) -> Result<Vec<CategoryInfo>> {
        let conn = self.conn.lock().expect("db lock");
        let sql = format!(
            "SELECT {CATEGORY_COLUMNS} FROM todo_categories WHERE project_id = ?1 ORDER BY parent_id ASC, rank ASC"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([project_id], row_to_category)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 同一 `parent_id` 下下一个 `rank`(当前最大 + 1,该 `parent_id` 下
    /// 还没有节点则从 0 开始)。`add`/`reparent` 共用。
    fn next_sibling_rank(
        conn: &Connection,
        project_id: i64,
        parent_id: Option<i64>,
    ) -> rusqlite::Result<i64> {
        let max_rank: Option<i64> = conn.query_row(
            "SELECT MAX(rank) FROM todo_categories WHERE project_id = ?1
             AND parent_id IS ?2",
            params![project_id, parent_id],
            |r| r.get(0),
        )?;
        Ok(max_rank.map(|r| r + 1).unwrap_or(0))
    }

    /// 新增分类,追加到 `parent_id` 下兄弟节点末尾。
    pub fn add(&self, project_id: i64, parent_id: Option<i64>, name: &str) -> Result<CategoryInfo> {
        let conn = self.conn.lock().expect("db lock");
        let rank = Self::next_sibling_rank(&conn, project_id, parent_id)?;
        let now = now_ms() as i64;
        let sql = format!(
            "INSERT INTO todo_categories (project_id, parent_id, name, rank, created_ms)
             VALUES (?1, ?2, ?3, ?4, ?5) RETURNING {CATEGORY_COLUMNS}"
        );
        conn.query_row(
            &sql,
            params![project_id, parent_id, name, rank, now],
            row_to_category,
        )
        .map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, CategoryStore) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.db");
        let store = CategoryStore::new(&db).unwrap();
        (dir, store)
    }

    #[test]
    fn add_and_list_top_level() {
        let (_dir, store) = store();
        let a = store.add(1, None, "前端").unwrap();
        let b = store.add(1, None, "后端").unwrap();
        assert_eq!(a.rank, 0);
        assert_eq!(b.rank, 1, "追加到末尾,不是置顶");
        let listed = store.list(1).unwrap();
        assert_eq!(listed.len(), 2);
    }

    #[test]
    fn add_child_ranks_independent_of_siblings_at_other_levels() {
        let (_dir, store) = store();
        let parent = store.add(1, None, "前端").unwrap();
        let child1 = store.add(1, Some(parent.id), "UI").unwrap();
        let child2 = store.add(1, Some(parent.id), "性能").unwrap();
        assert_eq!(child1.rank, 0);
        assert_eq!(child2.rank, 1);
        // 顶层再加一个,rank 从顶层自己的序列算,不受子级影响。
        let sibling = store.add(1, None, "后端").unwrap();
        assert_eq!(sibling.rank, 1);
    }

    #[test]
    fn categories_are_isolated_per_project() {
        let (_dir, store) = store();
        store.add(1, None, "项目1分类").unwrap();
        store.add(2, None, "项目2分类").unwrap();
        assert_eq!(store.list(1).unwrap().len(), 1);
        assert_eq!(store.list(2).unwrap().len(), 1);
    }
}
```

- [ ] **Step 2: 跑测试**

Run: `cargo test -p dozerd todo_category`
Expected: 需要先在 `lib.rs` 注册模块才能编译——先做下一步。

在 `crates/dozerd/src/lib.rs` 里,现有 `pub mod todo;` 那一行(第 15 行)
之后插入:

```rust
pub mod todo_category;
```

再跑一次:

Run: `cargo test -p dozerd todo_category`
Expected: PASS(3 个测试全过)。

- [ ] **Step 3: 加 `rename`/`delete`(含级联降未分类)——先写失败测试**

在 `tests` 模块里追加:

```rust
    #[test]
    fn rename_updates_name_and_unknown_id_errors() {
        let (_dir, store) = store();
        let a = store.add(1, None, "旧名字").unwrap();
        let renamed = store.rename(a.id, "新名字").unwrap();
        assert_eq!(renamed.name, "新名字");
        assert!(store.rename(999, "x").is_err());
    }

    #[test]
    fn delete_removes_node_and_descendants() {
        let (_dir, store) = store();
        let parent = store.add(1, None, "前端").unwrap();
        let child = store.add(1, Some(parent.id), "UI").unwrap();
        let grandchild = store.add(1, Some(child.id), "组件库").unwrap();
        store.delete(parent.id).unwrap();
        let listed = store.list(1).unwrap();
        assert!(listed.is_empty(), "父子孙三级都应该被删掉: {listed:?}");
        let _ = grandchild; // 仅用于构造场景
    }

    #[test]
    fn delete_unknown_id_errors() {
        let (_dir, store) = store();
        assert!(store.delete(999).is_err());
    }
```

- [ ] **Step 4: 跑测试确认失败**

Run: `cargo test -p dozerd rename_updates,delete_removes,delete_unknown`
Expected: FAIL(`no method named `rename`/`delete``)。

- [ ] **Step 5: 实现 `rename`/`delete`**

在 `impl CategoryStore` 块里,`add` 方法之后追加:

```rust
    pub fn rename(&self, id: i64, name: &str) -> Result<CategoryInfo> {
        let conn = self.conn.lock().expect("db lock");
        let sql =
            format!("UPDATE todo_categories SET name = ?1 WHERE id = ?2 RETURNING {CATEGORY_COLUMNS}");
        conn.query_row(&sql, params![name, id], row_to_category)
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => id_not_found(id),
                e => e.into(),
            })
    }

    /// 递归收集 `root` 自身及其全部子孙分类 id(树深度不大,BFS 就够,不
    /// 需要 SQL 递归 CTE)。
    fn collect_subtree_ids(conn: &Connection, root: i64) -> rusqlite::Result<Vec<i64>> {
        let mut ids = vec![root];
        let mut frontier = vec![root];
        while !frontier.is_empty() {
            let mut next = Vec::new();
            for parent in frontier {
                let mut stmt =
                    conn.prepare("SELECT id FROM todo_categories WHERE parent_id = ?1")?;
                let children: Vec<i64> =
                    stmt.query_map([parent], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
                next.extend(children);
            }
            ids.extend(&next);
            frontier = next;
        }
        Ok(ids)
    }

    /// 删除该节点及其全部子孙节点;这棵子树下原本挂靠的任务全部降级为
    /// 未分类(`category_id = NULL`),任务本身不删除。`id` 不存在返回
    /// `Err`(不是静默 no-op,对齐 `ProjectStore::rename`/`remove` 风格)。
    pub fn delete(&self, id: i64) -> Result<()> {
        let mut conn = self.conn.lock().expect("db lock");
        let exists: bool = conn
            .query_row(
                "SELECT 1 FROM todo_categories WHERE id = ?1",
                [id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !exists {
            return Err(id_not_found(id));
        }
        let subtree_ids = Self::collect_subtree_ids(&conn, id)?;
        let tx = conn.transaction()?;
        {
            let placeholders = subtree_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let update_sql =
                format!("UPDATE todos SET category_id = NULL WHERE category_id IN ({placeholders})");
            let params: Vec<&dyn rusqlite::ToSql> =
                subtree_ids.iter().map(|id| id as &dyn rusqlite::ToSql).collect();
            tx.execute(&update_sql, params.as_slice())?;
            let delete_sql =
                format!("DELETE FROM todo_categories WHERE id IN ({placeholders})");
            tx.execute(&delete_sql, params.as_slice())?;
        }
        tx.commit()?;
        Ok(())
    }
```

在文件顶部 `use` 里补上 `OptionalExtension`(`delete` 用到了
`.optional()?`):

```rust
use rusqlite::{Connection, OptionalExtension, params};
```

- [ ] **Step 6: 跑测试确认通过**

Run: `cargo test -p dozerd todo_category`
Expected: PASS(7 个测试全过)。

- [ ] **Step 7: 加 `reparent`(含成环校验)——先写失败测试**

追加测试:

```rust
    #[test]
    fn reparent_moves_to_new_parent_appended_to_end() {
        let (_dir, store) = store();
        let a = store.add(1, None, "A").unwrap();
        let b = store.add(1, None, "B").unwrap();
        let c = store.add(1, None, "C").unwrap(); // 挪去当 B 的子节点
        let moved = store.reparent(c.id, Some(b.id)).unwrap();
        assert_eq!(moved.parent_id, Some(b.id));
        assert_eq!(moved.rank, 0, "B 下还没有子节点,从 0 开始");
        let _ = a;
    }

    #[test]
    fn reparent_to_self_is_rejected() {
        let (_dir, store) = store();
        let a = store.add(1, None, "A").unwrap();
        assert!(store.reparent(a.id, Some(a.id)).is_err());
    }

    #[test]
    fn reparent_to_own_descendant_is_rejected() {
        let (_dir, store) = store();
        let parent = store.add(1, None, "前端").unwrap();
        let child = store.add(1, Some(parent.id), "UI").unwrap();
        let grandchild = store.add(1, Some(child.id), "组件库").unwrap();
        // 想把「前端」挪到自己的孙节点「组件库」下面——必须拒绝,否则成环。
        assert!(store.reparent(parent.id, Some(grandchild.id)).is_err());
        // 直接子节点同理。
        assert!(store.reparent(parent.id, Some(child.id)).is_err());
    }

    #[test]
    fn reparent_to_top_level_with_none() {
        let (_dir, store) = store();
        let parent = store.add(1, None, "前端").unwrap();
        let child = store.add(1, Some(parent.id), "UI").unwrap();
        let moved = store.reparent(child.id, None).unwrap();
        assert_eq!(moved.parent_id, None);
    }

    #[test]
    fn reparent_unknown_id_errors() {
        let (_dir, store) = store();
        assert!(store.reparent(999, None).is_err());
    }
```

- [ ] **Step 8: 跑测试确认失败**

Run: `cargo test -p dozerd reparent`
Expected: FAIL(`no method named `reparent``)。

- [ ] **Step 9: 实现 `reparent`**

在 `impl CategoryStore` 块里,`delete` 方法之后追加:

```rust
    /// 沿 `parent_id` 链从 `start` 往根走,判断路径上是否经过 `target`——
    /// 用来判断"把 `id` 挪到 `candidate` 下面会不会成环":如果从
    /// `candidate` 往上走能走到 `id`,说明 `candidate` 是 `id` 的子孙,
    /// 不能把 `id` 挂到自己的子孙下面。
    fn is_ancestor_or_self(conn: &Connection, start: i64, target: i64) -> rusqlite::Result<bool> {
        let mut cur = Some(start);
        while let Some(node) = cur {
            if node == target {
                return Ok(true);
            }
            cur = conn
                .query_row(
                    "SELECT parent_id FROM todo_categories WHERE id = ?1",
                    [node],
                    |r| r.get::<_, Option<i64>>(0),
                )
                .optional()?
                .flatten();
        }
        Ok(false)
    }

    /// 重新挂到 `new_parent_id` 下(`None` = 顶层),追加到新父节点子级
    /// 末尾。目标是自己或自己的子孙时拒绝(防止成环)。`id` 不存在返回
    /// `Err`。
    pub fn reparent(&self, id: i64, new_parent_id: Option<i64>) -> Result<CategoryInfo> {
        let conn = self.conn.lock().expect("db lock");
        let project_id: i64 = conn
            .query_row(
                "SELECT project_id FROM todo_categories WHERE id = ?1",
                [id],
                |r| r.get(0),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => id_not_found(id),
                e => e.into(),
            })?;
        if let Some(target) = new_parent_id {
            if Self::is_ancestor_or_self(&conn, target, id)? {
                anyhow::bail!("不能把分类移动到自己或自己的子分类下面");
            }
        }
        let rank = Self::next_sibling_rank(&conn, project_id, new_parent_id)?;
        let sql = format!(
            "UPDATE todo_categories SET parent_id = ?1, rank = ?2 WHERE id = ?3
             RETURNING {CATEGORY_COLUMNS}"
        );
        conn.query_row(&sql, params![new_parent_id, rank, id], row_to_category)
            .map_err(Into::into)
    }
```

- [ ] **Step 10: 跑测试确认通过**

Run: `cargo test -p dozerd todo_category`
Expected: PASS(12 个测试全过)。

- [ ] **Step 11: 加 `move_sibling`(上移/下移)——先写失败测试**

追加测试:

```rust
    #[test]
    fn move_sibling_swaps_rank_with_neighbor() {
        let (_dir, store) = store();
        let a = store.add(1, None, "A").unwrap(); // rank 0
        let b = store.add(1, None, "B").unwrap(); // rank 1
        let c = store.add(1, None, "C").unwrap(); // rank 2
        // 把 B 上移:B、A 交换 rank → 顺序变 B, A, C。
        store
            .move_sibling(b.id, CategoryMoveDirection::Up)
            .unwrap();
        let listed = store.list(1).unwrap();
        assert_eq!(
            listed.iter().map(|c| c.id).collect::<Vec<_>>(),
            vec![b.id, a.id, c.id]
        );
    }

    #[test]
    fn move_sibling_up_at_front_is_noop() {
        let (_dir, store) = store();
        let a = store.add(1, None, "A").unwrap();
        let b = store.add(1, None, "B").unwrap();
        let result = store
            .move_sibling(a.id, CategoryMoveDirection::Up)
            .unwrap();
        assert_eq!(result.rank, 0, "已经在最前,不报错也不改变");
        let listed = store.list(1).unwrap();
        assert_eq!(listed[0].id, a.id);
        assert_eq!(listed[1].id, b.id);
    }

    #[test]
    fn move_sibling_down_at_back_is_noop() {
        let (_dir, store) = store();
        let a = store.add(1, None, "A").unwrap();
        let b = store.add(1, None, "B").unwrap();
        store
            .move_sibling(b.id, CategoryMoveDirection::Down)
            .unwrap();
        let listed = store.list(1).unwrap();
        assert_eq!(listed[0].id, a.id);
        assert_eq!(listed[1].id, b.id, "已经在最后,顺序不变");
    }

    #[test]
    fn move_sibling_only_affects_same_parent() {
        let (_dir, store) = store();
        let parent = store.add(1, None, "前端").unwrap();
        let child = store.add(1, Some(parent.id), "UI").unwrap();
        let top_level = store.add(1, None, "后端").unwrap();
        // child 只有一个同级(它自己),上移应该是 no-op,不会跟顶层的
        // top_level 混到一起交换。
        let result = store
            .move_sibling(child.id, CategoryMoveDirection::Up)
            .unwrap();
        assert_eq!(result.parent_id, Some(parent.id));
        let _ = top_level;
    }
```

- [ ] **Step 12: 跑测试确认失败**

Run: `cargo test -p dozerd move_sibling`
Expected: FAIL(`no method named `move_sibling``,`CategoryMoveDirection`
需要在测试文件里 `use super::*` 已经带进来,来自
`dozer_core::protocol` 的 re-export)。

- [ ] **Step 13: 实现 `move_sibling`**

在 `impl CategoryStore` 块里,`reparent` 方法之后追加:

```rust
    /// 与同一 `parent_id` 下相邻的前一个/后一个兄弟节点交换 `rank`。
    /// 已经在最前/最后时对应方向是 no-op(返回当前状态,不报错)。`id`
    /// 不存在返回 `Err`。
    pub fn move_sibling(&self, id: i64, direction: CategoryMoveDirection) -> Result<CategoryInfo> {
        let mut conn = self.conn.lock().expect("db lock");
        let (project_id, parent_id, rank): (i64, Option<i64>, i64) = conn
            .query_row(
                "SELECT project_id, parent_id, rank FROM todo_categories WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => id_not_found(id),
                e => e.into(),
            })?;
        let neighbor: Option<(i64, i64)> = match direction {
            CategoryMoveDirection::Up => conn
                .query_row(
                    "SELECT id, rank FROM todo_categories WHERE project_id = ?1
                     AND parent_id IS ?2 AND rank < ?3 ORDER BY rank DESC LIMIT 1",
                    params![project_id, parent_id, rank],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?,
            CategoryMoveDirection::Down => conn
                .query_row(
                    "SELECT id, rank FROM todo_categories WHERE project_id = ?1
                     AND parent_id IS ?2 AND rank > ?3 ORDER BY rank ASC LIMIT 1",
                    params![project_id, parent_id, rank],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?,
        };
        if let Some((neighbor_id, neighbor_rank)) = neighbor {
            let tx = conn.transaction()?;
            tx.execute(
                "UPDATE todo_categories SET rank = ?1 WHERE id = ?2",
                params![neighbor_rank, id],
            )?;
            tx.execute(
                "UPDATE todo_categories SET rank = ?1 WHERE id = ?2",
                params![rank, neighbor_id],
            )?;
            tx.commit()?;
        }
        let sql = format!("SELECT {CATEGORY_COLUMNS} FROM todo_categories WHERE id = ?1");
        conn.query_row(&sql, [id], row_to_category)
            .map_err(Into::into)
    }
```

- [ ] **Step 14: 跑测试 + clippy + fmt**

Run: `cargo test -p dozerd`
Expected: PASS(`todo_category` 模块 16 个测试 + 既有 `todo`/其它模块
测试全过)。

Run: `cargo clippy -p dozerd --all-targets -- -D warnings`
Expected: 干净,无警告。

Run: `cargo fmt`

- [ ] **Step 15: Commit**

```bash
git add crates/dozerd/src/todo_category.rs crates/dozerd/src/lib.rs
git commit -m "feat(dozerd): 新增 CategoryStore(增删改/reparent/上移下移)"
```

---

## Task 4: `dozerd` — 接线(`main.rs`/`server.rs`)

**Files:**
- Modify: `crates/dozerd/src/main.rs`
- Modify: `crates/dozerd/src/server.rs`
- Modify: `crates/dozer-client/tests/against_real_daemon.rs`(`start_daemon` 辅助函数需要同步加新参数,否则编译不过)

**Interfaces:**
- Consumes: `crate::todo_category::CategoryStore`(Task 3)、`Request::{ListCategories,AddCategory,RenameCategory,DeleteCategory,ReparentCategory,MoveCategorySibling,SetTodoCategory}`(Task 1)。
- Produces: `dozerd::server::serve`/`handle_conn` 新增 `categories: Arc<crate::todo_category::CategoryStore>` 参数(位置在末尾,紧跟现有 `todos` 参数之后)——Task 5(dozer-client 测试)依赖这个新参数顺序。

- [ ] **Step 1: `main.rs` 构造 `CategoryStore` 并传入 `serve`**

修改 `crates/dozerd/src/main.rs`,在现有 `let todos = Arc::new(...)`
(第 95-97 行)之后插入:

```rust
    let categories = Arc::new(dozerd::todo_category::CategoryStore::new(
        &dozer_core::paths::state_dir().join("dozer.db"),
    )?);
```

修改 `dozerd::server::serve(...)` 调用(第 103-113 行),在 `todos,` 之后加一行:

```rust
    let serve = dozerd::server::serve(
        &socket,
        registry,
        store,
        projects,
        bookmarks,
        transcripts,
        session_summaries,
        backfill_registry,
        todos,
        categories,
    );
```

- [ ] **Step 2: `server.rs` 的 `serve`/`handle_conn` 签名加 `categories` 参数**

修改 `crates/dozerd/src/server.rs` 的 `serve` 函数签名(现有第 15-25
行),在 `todos: Arc<crate::todo::TodoStore>,` 之后加一行:

```rust
    categories: Arc<crate::todo_category::CategoryStore>,
```

在 `serve` 函数体内,现有 `let todos = todos.clone();`(第 51 行)
之后加:

```rust
        let categories = categories.clone();
```

在 `handle_conn(...)` 调用处(现有第 53-64 行),`todos.clone(),` 之后加:

```rust
                categories.clone(),
```

修改 `handle_conn` 函数签名(现有第 233-245 行),在
`todos: Arc<crate::todo::TodoStore>,` 之后加一行:

```rust
    categories: Arc<crate::todo_category::CategoryStore>,
```

- [ ] **Step 3: 给 `TodoStore` 加 `get(id) -> Result<TodoInfo>`**

`SetTodoCategory` 落库前要校验"`category_id` 指向的分类存在且属于任务
所在的 project"(防止 GUI 传错 id 把任务挂到别的项目的分类下面),这
需要先拿到任务自己的 `project_id`,而 `TodoStore` 目前没有"查单条任务
详情(不 list 全部)"的方法,先补上。

回到 `crates/dozerd/src/todo.rs`,在 `impl TodoStore` 块里 `list`
方法(现有第 78-86 行)之后插入:

```rust
    /// 查单条任务(`SetTodoCategory` 校验分类归属用)。`id` 不存在返回
    /// `Err`。
    pub fn get(&self, id: i64) -> Result<TodoInfo> {
        let conn = self.conn.lock().expect("db lock");
        let sql = format!("SELECT {TODO_COLUMNS} FROM todos WHERE id = ?1");
        conn.query_row(&sql, [id], row_to_todo)
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => id_not_found(id),
                e => e.into(),
            })
    }
```

追加一个单测(在 `tasks_are_isolated_per_project` 之后):

```rust
    #[test]
    fn get_returns_task_and_unknown_id_errors() {
        let (_dir, store) = store();
        let t = store.add(1, "任务").unwrap();
        let fetched = store.get(t.id).unwrap();
        assert_eq!(fetched.id, t.id);
        assert!(store.get(999).is_err());
    }
```

Run: `cargo test -p dozerd get_returns_task`
Expected: PASS。

- [ ] **Step 4: 补 `Request`/`Reply` match 分支**

在 `handle_conn` 内的大 `match req { .. }` 里,现有
`Request::RecordTodoDispatch { .. }` 分支(第 459-466 行)之后,
`Request::UpdatePreviewContext { .. }` 之前插入:

```rust
                        Request::ListCategories { project_id } => {
                            match categories.list(project_id) {
                                Ok(categories) => Reply::Categories { categories },
                                Err(e) => Reply::Error {
                                    message: format!("列分类失败: {e}"),
                                },
                            }
                        }
                        Request::AddCategory { project_id, parent_id, name } => {
                            match categories.add(project_id, parent_id, &name) {
                                Ok(category) => Reply::Category { category },
                                Err(e) => Reply::Error {
                                    message: format!("新增分类失败: {e}"),
                                },
                            }
                        }
                        Request::RenameCategory { id, name } => {
                            match categories.rename(id, &name) {
                                Ok(category) => Reply::Category { category },
                                Err(e) => Reply::Error {
                                    message: format!("重命名分类失败: {e}"),
                                },
                            }
                        }
                        Request::DeleteCategory { id } => match categories.delete(id) {
                            Ok(()) => Reply::Ok,
                            Err(e) => Reply::Error {
                                message: format!("删除分类失败: {e}"),
                            },
                        },
                        Request::ReparentCategory { id, new_parent_id } => {
                            match categories.reparent(id, new_parent_id) {
                                Ok(category) => Reply::Category { category },
                                Err(e) => Reply::Error {
                                    message: format!("移动分类失败: {e}"),
                                },
                            }
                        }
                        Request::MoveCategorySibling { id, direction } => {
                            match categories.move_sibling(id, direction) {
                                Ok(category) => Reply::Category { category },
                                Err(e) => Reply::Error {
                                    message: format!("调整分类顺序失败: {e}"),
                                },
                            }
                        }
                        Request::SetTodoCategory { id, category_id } => {
                            // 挂真实分类前先校验它存在且属于同一 project——
                            // 防止 GUI 传错 id 把任务挂到别的项目的分类下面。
                            // `TodoStore` 自己不知道 `CategoryStore` 的存在,
                            // 这层校验只能在这里(两个 store 的交汇点)做。
                            let validation = match category_id {
                                None => Ok(()),
                                Some(cat_id) => todos.get(id).and_then(|todo| {
                                    let same_project = categories
                                        .list(todo.project_id)?
                                        .iter()
                                        .any(|c| c.id == cat_id);
                                    if same_project {
                                        Ok(())
                                    } else {
                                        Err(anyhow::anyhow!(
                                            "分类 id={cat_id} 不存在或不属于该项目"
                                        ))
                                    }
                                }),
                            };
                            match validation.and_then(|()| todos.set_category(id, category_id)) {
                                Ok(todo) => Reply::Todo { todo },
                                Err(e) => Reply::Error {
                                    message: format!("设置任务分类失败: {e}"),
                                },
                            }
                        }
```

- [ ] **Step 5: 修 `against_real_daemon.rs` 的 `start_daemon` 辅助函数**

`crates/dozer-client/tests/against_real_daemon.rs` 的 `start_daemon`
(现有第 13-48 行)直接调用 `dozerd::server::serve(...)`,新增的
`categories` 参数会让这里编译失败。在 `let todos = ...`(第 26 行)
之后插入:

```rust
    let categories = Arc::new(dozerd::todo_category::CategoryStore::new(&db).unwrap());
```

在 `serve(...)` 调用里 `todos,`(第 37 行)之后加:

```rust
            categories,
```

- [ ] **Step 6: 编译 + 跑全部相关测试**

Run: `cargo build` (全 workspace)
Expected: 编译通过(`dozer-app`/`dozer-mcp` 还没碰过 `Request`/`Reply`
的穷尽 match,如果它们也 match 了这两个枚举需要确认没有遗漏分支——
下一步验证)。

Run: `cargo test -p dozerd -p dozer-client -p dozer-core`
Expected: 全部通过。

Run: `cargo clippy --all-targets -- -D warnings`
Expected: 干净。

Run: `cargo fmt`

- [ ] **Step 7: Commit**

```bash
git add crates/dozerd/src/main.rs crates/dozerd/src/server.rs \
  crates/dozerd/src/todo.rs crates/dozer-client/tests/against_real_daemon.rs
git commit -m "feat(dozerd): 接入 CategoryStore,server.rs 处理全部分类请求"
```

---

## Task 5: `dozer-client` — 七个客户端方法 + roundtrip 测试

**Files:**
- Modify: `crates/dozer-client/src/lib.rs`
- Modify: `crates/dozer-client/tests/against_real_daemon.rs`

**Interfaces:**
- Consumes: `Request`/`Reply` 的 `Category*`/`SetTodoCategory` 变体(Task 1)。
- Produces: `Client::{list_categories(project_id) -> Result<Vec<CategoryInfo>>, add_category(project_id, parent_id, name) -> Result<CategoryInfo>, rename_category(id, name) -> Result<CategoryInfo>, delete_category(id) -> Result<()>, reparent_category(id, new_parent_id) -> Result<CategoryInfo>, move_category_sibling(id, direction) -> Result<CategoryInfo>, set_todo_category(id, category_id) -> Result<TodoInfo>}`——Task 6/7(dozer-app)依赖这七个方法名和签名。

- [ ] **Step 1: 加七个方法**

在 `crates/dozer-client/src/lib.rs` 里,现有 `record_todo_dispatch`
方法(第 345-357 行)之后插入:

```rust
    pub async fn list_categories(&self, project_id: i64) -> Result<Vec<CategoryInfo>> {
        match self
            .roundtrip(&Request::ListCategories { project_id })
            .await?
        {
            Reply::Categories { categories } => Ok(categories),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn add_category(
        &self,
        project_id: i64,
        parent_id: Option<i64>,
        name: &str,
    ) -> Result<CategoryInfo> {
        match self
            .roundtrip(&Request::AddCategory {
                project_id,
                parent_id,
                name: name.into(),
            })
            .await?
        {
            Reply::Category { category } => Ok(category),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn rename_category(&self, id: i64, name: &str) -> Result<CategoryInfo> {
        match self
            .roundtrip(&Request::RenameCategory {
                id,
                name: name.into(),
            })
            .await?
        {
            Reply::Category { category } => Ok(category),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn delete_category(&self, id: i64) -> Result<()> {
        match self.roundtrip(&Request::DeleteCategory { id }).await? {
            Reply::Ok => Ok(()),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn reparent_category(
        &self,
        id: i64,
        new_parent_id: Option<i64>,
    ) -> Result<CategoryInfo> {
        match self
            .roundtrip(&Request::ReparentCategory { id, new_parent_id })
            .await?
        {
            Reply::Category { category } => Ok(category),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn move_category_sibling(
        &self,
        id: i64,
        direction: CategoryMoveDirection,
    ) -> Result<CategoryInfo> {
        match self
            .roundtrip(&Request::MoveCategorySibling { id, direction })
            .await?
        {
            Reply::Category { category } => Ok(category),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn set_todo_category(
        &self,
        id: i64,
        category_id: Option<i64>,
    ) -> Result<TodoInfo> {
        match self
            .roundtrip(&Request::SetTodoCategory { id, category_id })
            .await?
        {
            Reply::Todo { todo } => Ok(todo),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }
```

确认文件顶部 `use dozer_core::protocol::{..}` 的 import 列表里已经带
上 `CategoryInfo`/`CategoryMoveDirection`(没有的话补上——具体现有
import 写法先 `grep -n "use dozer_core::protocol" crates/dozer-client/src/lib.rs`
确认后再改,通常是一整块 `use dozer_core::protocol::{TodoInfo, ...};`
的花括号列表,加两个名字进去即可)。

- [ ] **Step 2: 写 roundtrip 集成测试**

在 `crates/dozer-client/tests/against_real_daemon.rs` 末尾追加:

```rust
#[tokio::test]
async fn category_crud_and_reorder_roundtrip() {
    let (sock, _registry, _guard) = start_daemon().await;
    let client = Client::new(sock);

    let frontend = client.add_category(1, None, "前端").await.unwrap();
    let backend = client.add_category(1, None, "后端").await.unwrap();
    let ui = client
        .add_category(1, Some(frontend.id), "UI")
        .await
        .unwrap();

    let listed = client.list_categories(1).await.unwrap();
    assert_eq!(listed.len(), 3);

    let renamed = client.rename_category(ui.id, "界面").await.unwrap();
    assert_eq!(renamed.name, "界面");

    let reparented = client
        .reparent_category(ui.id, Some(backend.id))
        .await
        .unwrap();
    assert_eq!(reparented.parent_id, Some(backend.id));

    client.delete_category(ui.id).await.unwrap();
    let listed = client.list_categories(1).await.unwrap();
    assert_eq!(listed.len(), 2);

    let moved = client
        .move_category_sibling(backend.id, dozer_core::protocol::CategoryMoveDirection::Up)
        .await
        .unwrap();
    assert_eq!(moved.id, backend.id);
}

#[tokio::test]
async fn set_todo_category_roundtrip() {
    let (sock, _registry, _guard) = start_daemon().await;
    let client = Client::new(sock);

    let category = client.add_category(1, None, "分类").await.unwrap();
    let todo = client.add_todo(1, "任务").await.unwrap();
    assert_eq!(todo.category_id, None);

    let categorized = client
        .set_todo_category(todo.id, Some(category.id))
        .await
        .unwrap();
    assert_eq!(categorized.category_id, Some(category.id));

    let cleared = client.set_todo_category(todo.id, None).await.unwrap();
    assert_eq!(cleared.category_id, None);
}
```

- [ ] **Step 3: 跑测试**

Run: `cargo test -p dozer-client`
Expected: 全部通过,包括新增的
`category_crud_and_reorder_roundtrip`/`set_todo_category_roundtrip`。

Run: `cargo clippy -p dozer-client --all-targets -- -D warnings`
Run: `cargo fmt`

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-client/src/lib.rs crates/dozer-client/tests/against_real_daemon.rs
git commit -m "feat(dozer-client): 分类树七个客户端方法 + roundtrip 测试"
```

---

## Task 6: `dozer-app` — 纯函数(`CategoryFilter`/树拍平/子孙汇总/过滤)

**Files:**
- Modify: `crates/dozer-app/src/extensions/todo.rs`

**Interfaces:**
- Consumes: `dozer_core::protocol::CategoryInfo`(Task 1)。
- Produces: `pub enum CategoryFilter { All, Uncategorized, Node(i64) }`;`pub struct CategoryTreeRow { pub id: i64, pub name: String, pub depth: usize, pub has_children: bool, pub expanded: bool }`;`pub fn visible_category_rows(categories: &[CategoryInfo], expanded: &HashSet<i64>) -> Vec<CategoryTreeRow>`;`pub fn category_descendants(categories: &[CategoryInfo], root: i64) -> HashSet<i64>`;`pub fn filter_todos_by_category(todos: &[TodoInfo], categories: &[CategoryInfo], filter: CategoryFilter) -> HashSet<i64>`(返回**任务 id 集合**而不是下标——避免和现有 `filter_todos` 返回的下标语义混淆,调用方在 Task 7/8 里按 `todo.id` 是否在集合里跟 `filter_todos` 的下标结果取交集)。这些类型/函数名是 Task 7(状态接线)和 Task 8(视图渲染)的唯一依据。

- [ ] **Step 1: 写失败测试**

在 `crates/dozer-app/src/extensions/todo.rs` 现有 `#[cfg(test)] mod
tests`(用 `grep -n "mod tests" crates/dozer-app/src/extensions/todo.rs`
先确认起始行号,现有测试都在文件尾部,`filter_todos` 相关测试在第
2164-2195 行附近)里追加:

```rust
    fn cat(id: i64, parent_id: Option<i64>, name: &str) -> CategoryInfo {
        CategoryInfo {
            id,
            project_id: 1,
            parent_id,
            name: name.into(),
            rank: 0,
            created_ms: 0,
        }
    }

    #[test]
    fn visible_category_rows_flattens_by_expanded_state() {
        // 前端(展开) -> UI, 性能(未展开无子节点)
        //   UI(未展开) -> 组件库(不可见,父未展开)
        // 后端(未展开) -> 数据库(不可见)
        let categories = vec![
            cat(1, None, "前端"),
            cat(2, Some(1), "UI"),
            cat(3, Some(1), "性能"),
            cat(4, Some(2), "组件库"),
            cat(5, None, "后端"),
            cat(6, Some(5), "数据库"),
        ];
        let mut expanded = std::collections::HashSet::new();
        expanded.insert(1);
        let rows = visible_category_rows(&categories, &expanded);
        let visible_ids: Vec<i64> = rows.iter().map(|r| r.id).collect();
        assert_eq!(visible_ids, vec![1, 2, 3, 5], "只展开了前端,UI/后端的子节点都不可见");
        let front = rows.iter().find(|r| r.id == 1).unwrap();
        assert_eq!(front.depth, 0);
        assert!(front.has_children);
        assert!(front.expanded);
        let ui = rows.iter().find(|r| r.id == 2).unwrap();
        assert_eq!(ui.depth, 1);
        assert!(ui.has_children, "UI 有子节点组件库,即使未展开也要标记有子节点");
        assert!(!ui.expanded);
        let perf = rows.iter().find(|r| r.id == 3).unwrap();
        assert!(!perf.has_children);
    }

    #[test]
    fn category_descendants_covers_multi_level_and_excludes_root() {
        let categories = vec![
            cat(1, None, "前端"),
            cat(2, Some(1), "UI"),
            cat(3, Some(2), "组件库"),
            cat(4, None, "后端"),
        ];
        let descendants = category_descendants(&categories, 1);
        assert_eq!(
            descendants,
            std::collections::HashSet::from([2, 3]),
            "根节点自己不算子孙,后端这条不相关分支不应该出现"
        );
        assert!(category_descendants(&categories, 3).is_empty(), "叶子节点没有子孙");
    }

    #[test]
    fn filter_todos_by_category_all_returns_everything() {
        let todos = vec![
            todo_with_category(1, Some(10)),
            todo_with_category(2, None),
        ];
        let categories = vec![cat(10, None, "分类")];
        let ids = filter_todos_by_category(&todos, &categories, CategoryFilter::All);
        assert_eq!(ids, std::collections::HashSet::from([1, 2]));
    }

    #[test]
    fn filter_todos_by_category_uncategorized_only() {
        let todos = vec![
            todo_with_category(1, Some(10)),
            todo_with_category(2, None),
        ];
        let ids = filter_todos_by_category(&todos, &[], CategoryFilter::Uncategorized);
        assert_eq!(ids, std::collections::HashSet::from([2]));
    }

    #[test]
    fn filter_todos_by_category_node_includes_descendant_tasks() {
        // 前端(id 10) -> UI(id 11);任务 1 挂前端,任务 2 挂 UI,任务 3 未分类。
        // 选中"前端"应该看到任务 1 和任务 2(子孙汇总)。
        let todos = vec![
            todo_with_category(1, Some(10)),
            todo_with_category(2, Some(11)),
            todo_with_category(3, None),
        ];
        let categories = vec![cat(10, None, "前端"), cat(11, Some(10), "UI")];
        let ids = filter_todos_by_category(&todos, &categories, CategoryFilter::Node(10));
        assert_eq!(ids, std::collections::HashSet::from([1, 2]));
        // 选中叶子节点"UI"只看到任务 2。
        let ids = filter_todos_by_category(&todos, &categories, CategoryFilter::Node(11));
        assert_eq!(ids, std::collections::HashSet::from([2]));
    }

    fn todo_with_category(id: i64, category_id: Option<i64>) -> TodoInfo {
        TodoInfo {
            id,
            project_id: 1,
            text: format!("任务{id}"),
            done: false,
            rank: 0,
            created_ms: 0,
            completed_at_ms: None,
            plan_date: None,
            dispatch_session_id: None,
            dispatch_at_ms: None,
            category_id,
        }
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app visible_category_rows,category_descendants,filter_todos_by_category`
Expected: FAIL(编译错误——`CategoryFilter`/`CategoryTreeRow`/三个函数
都还不存在)。

- [ ] **Step 3: 实现**

在 `crates/dozer-app/src/extensions/todo.rs` 里,现有 `TodoFilter`
枚举定义(第 45-52 行)之后插入:

```rust
/// 分类树的当前过滤选中态。`All`/`Uncategorized` 是钉在树顶的两个伪
/// 节点(不对应真实 `CategoryInfo` 行),`Node(id)` 才是用户自建的真实
/// 分类节点。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CategoryFilter {
    #[default]
    All,
    Uncategorized,
    Node(i64),
}

/// 分类树左侧面板一行的拍平展示(镜像 `project.rs::TreeRow` 的"扁平
/// 存储 + 展开集 → 拍平成行"模式,只是节点数据源从文件系统换成
/// `CategoryInfo`)。
#[derive(Debug, Clone, PartialEq)]
pub struct CategoryTreeRow {
    pub id: i64,
    pub name: String,
    pub depth: usize,
    pub has_children: bool,
    pub expanded: bool,
}
```

在 `filter_todos` 函数(现有第 94-116 行)之后插入三个纯函数:

```rust
/// 按 `parent_id` 把 `categories` 拼成树,按 `expanded` 展开态深度优先
/// 拍平成可见行(未展开节点的子孙不出现在结果里,但节点自身若有子节点
/// 仍会标 `has_children = true`,供左侧渲染箭头)。同级顺序按
/// `CategoryInfo.rank` 升序。
pub fn visible_category_rows(
    categories: &[CategoryInfo],
    expanded: &std::collections::HashSet<i64>,
) -> Vec<CategoryTreeRow> {
    let mut children_of: std::collections::HashMap<Option<i64>, Vec<&CategoryInfo>> =
        std::collections::HashMap::new();
    for c in categories {
        children_of.entry(c.parent_id).or_default().push(c);
    }
    for siblings in children_of.values_mut() {
        siblings.sort_by_key(|c| c.rank);
    }
    let mut rows = Vec::new();
    fn walk(
        parent: Option<i64>,
        depth: usize,
        children_of: &std::collections::HashMap<Option<i64>, Vec<&CategoryInfo>>,
        expanded: &std::collections::HashSet<i64>,
        rows: &mut Vec<CategoryTreeRow>,
    ) {
        let Some(siblings) = children_of.get(&parent) else {
            return;
        };
        for c in siblings {
            let has_children = children_of
                .get(&Some(c.id))
                .map(|v| !v.is_empty())
                .unwrap_or(false);
            let is_expanded = expanded.contains(&c.id);
            rows.push(CategoryTreeRow {
                id: c.id,
                name: c.name.clone(),
                depth,
                has_children,
                expanded: is_expanded,
            });
            if is_expanded {
                walk(Some(c.id), depth + 1, children_of, expanded, rows);
            }
        }
    }
    walk(None, 0, &children_of, expanded, &mut rows);
    rows
}

/// `root` 的全部子孙节点 id(不含 `root` 自己)。给"选中父节点汇总子孙
/// 任务"和"reparent 目标合法性校验"(GUI 侧提前拦截,dozerd 侧
/// `CategoryStore::reparent` 仍会再校验一次,双保险)复用。
pub fn category_descendants(
    categories: &[CategoryInfo],
    root: i64,
) -> std::collections::HashSet<i64> {
    let mut children_of: std::collections::HashMap<i64, Vec<i64>> = std::collections::HashMap::new();
    for c in categories {
        if let Some(p) = c.parent_id {
            children_of.entry(p).or_default().push(c.id);
        }
    }
    let mut out = std::collections::HashSet::new();
    let mut frontier = vec![root];
    while let Some(node) = frontier.pop() {
        if let Some(children) = children_of.get(&node) {
            for &child in children {
                if out.insert(child) {
                    frontier.push(child);
                }
            }
        }
    }
    out
}

/// 按当前分类过滤选中态,返回符合条件的任务 id 集合(不是下标——调用方
/// 若需要按下标跟既有 `filter_todos` 的结果取交集,自己按 `todos[i].id`
/// 是否在这个集合里判断)。`Node(id)` 汇总该节点及其全部子孙下的任务。
pub fn filter_todos_by_category(
    todos: &[TodoInfo],
    categories: &[CategoryInfo],
    filter: CategoryFilter,
) -> std::collections::HashSet<i64> {
    match filter {
        CategoryFilter::All => todos.iter().map(|t| t.id).collect(),
        CategoryFilter::Uncategorized => todos
            .iter()
            .filter(|t| t.category_id.is_none())
            .map(|t| t.id)
            .collect(),
        CategoryFilter::Node(root) => {
            let mut allowed = category_descendants(categories, root);
            allowed.insert(root);
            todos
                .iter()
                .filter(|t| t.category_id.is_some_and(|cid| allowed.contains(&cid)))
                .map(|t| t.id)
                .collect()
        }
    }
}
```

在文件顶部 `use dozer_core::protocol::{AgentKind, TodoInfo};`(现有第
13 行)改成:

```rust
use dozer_core::protocol::{AgentKind, CategoryInfo, TodoInfo};
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app visible_category_rows,category_descendants,filter_todos_by_category`
Expected: PASS(7 个新测试全过)。

Run: `cargo test -p dozer-app` (全量,确认没有破坏既有测试)
Run: `cargo clippy -p dozer-app --all-targets -- -D warnings`
Run: `cargo fmt`

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/extensions/todo.rs
git commit -m "feat(dozer-app): 分类树纯函数(拍平/子孙汇总/按分类过滤)"
```

---

## Task 7: `dozer-app` — 状态接线(`WorkspaceState`/`Message`/`update`/轮询)

**Files:**
- Modify: `crates/dozer-app/src/extensions/todo.rs`
- Modify: `crates/dozer-app/src/app.rs`

**Interfaces:**
- Consumes: `CategoryFilter`/`visible_category_rows`/`category_descendants`/`filter_todos_by_category`(Task 6)、`Client::{list_categories,add_category,rename_category,delete_category,reparent_category,move_category_sibling,set_todo_category}`(Task 5)。
- Produces: `WorkspaceState::categories() -> &[CategoryInfo]`;`WorkspaceState::category_expanded() -> &HashSet<i64>`;`WorkspaceState::category_selected() -> CategoryFilter`;`Message::{CategoriesLoaded(Vec<CategoryInfo>), CategoryMutated(Result<(),String>), CategoryToggleExpand(i64), CategorySelect(CategoryFilter)}`——Task 8/9/10(视图层)依赖这些。

- [ ] **Step 1: `WorkspaceState` 加字段**

在 `crates/dozer-app/src/extensions/todo.rs` 的 `WorkspaceState`
结构体(现有第 130-200 行)里,`drag: Option<TodoDrag>,` 字段之后加:

```rust
    /// 当前项目全部分类节点,随 `ListTodos` 同一轮轮询一并拉取
    /// (`request_categories_refresh`)。
    categories: Vec<CategoryInfo>,
    /// 展开的分类节点 id,纯 UI 态,不落盘(对齐 `FileTree::expanded`
    /// 同样"只在内存里"的处理)。
    category_expanded: std::collections::HashSet<i64>,
    /// 当前选中的分类过滤节点,默认"全部"。
    category_selected: CategoryFilter,
```

在 `impl WorkspaceState` 块里(现有第 202-391 行),`clear_search`
方法之后追加访问器:

```rust
    /// 当前项目全部分类节点(左侧树渲染 + 过滤计算用)。
    pub fn categories(&self) -> &[CategoryInfo] {
        &self.categories
    }

    /// 展开的分类节点 id 集合(左侧树渲染用)。
    pub fn category_expanded(&self) -> &std::collections::HashSet<i64> {
        &self.category_expanded
    }

    /// 当前选中的分类过滤节点。
    pub fn category_selected(&self) -> CategoryFilter {
        self.category_selected
    }

    /// 展开/收起某个分类节点(点左侧树箭头)。
    fn toggle_category_expanded(&mut self, id: i64) {
        if !self.category_expanded.remove(&id) {
            self.category_expanded.insert(id);
        }
    }
```

- [ ] **Step 2: `Message` 加变体**

在 `Message` 枚举(现有第 429-501 行)里,`Mutated(Result<(), String>),`
之后加:

```rust
    /// 拉取分类树的异步结果(轮询、或任一分类写操作成功后的刷新都落
    /// 这里),与 `Loaded` 同构。
    CategoriesLoaded(Vec<CategoryInfo>),
    /// 分类写操作(增/改/删/reparent/上移下移/挂任务分类)的异步确认;
    /// 不管成功失败都触发一次 `CategoriesLoaded` + `Loaded` 双刷新——
    /// `SetTodoCategory` 改的是任务的 `category_id`,单刷分类树看不到
    /// 任务列表那边的变化,所以两份列表一起刷,与 `Mutated` 只刷
    /// `Loaded` 的原因不同(那边改的字段只影响任务本身)。
    CategoryMutated(Result<(), String>),
    /// 点左侧树箭头,展开/收起该节点。
    CategoryToggleExpand(i64),
    /// 点左侧树某一行(或"全部"/"未分类"伪节点),切换当前过滤。
    CategorySelect(CategoryFilter),
```

- [ ] **Step 3: `request_categories_refresh` + `update` 分支**

在 `request_todos_refresh` 函数(现有第 577-588 行)之后插入:

```rust
/// `request_todos_refresh` 的分类树版本:异步拉取某项目的全部分类节点。
pub fn request_categories_refresh(
    project_id: i64,
    client: &Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    let client = client.clone();
    handle.spawn(async move {
        let categories = client.list_categories(project_id).await.unwrap_or_default();
        emit(Message::CategoriesLoaded(categories));
    });
}
```

在 `update` 函数的 `match msg { .. }` 里,现有 `Message::Mutated(res)
=> { .. }` 分支(第 667-672 行)之后插入。`CategoryMutated` 需要同时
触发分类树刷新(`request_categories_refresh`)和任务列表刷新(内联一次
`list_todos`——因为 `SetTodoCategory` 改的是任务的 `category_id` 字段,
只刷分类树看不到任务那边的变化),而 `emit: impl Fn(Message) + Send +
'static` 是按值传入、只能用一次,所以这里用 `Arc` 包一层供两处闭包
各自克隆一份:

```rust
        Message::CategoriesLoaded(categories) => ws_state.categories = categories,
        Message::CategoryMutated(res) => {
            if let Err(e) = res {
                tracing::warn!("分类写操作失败: {e}");
            }
            let client1 = client.clone();
            let client2 = client.clone();
            let handle1 = handle.clone();
            let handle2 = handle.clone();
            let emit = std::sync::Arc::new(emit);
            let emit1 = emit.clone();
            let emit2 = emit;
            request_categories_refresh(project_id, &client1, &handle1, move |m| emit1(m));
            let _ = handle2.spawn(async move {
                let todos = client2.list_todos(project_id).await.unwrap_or_default();
                emit2(Message::Loaded(todos));
            });
        }
        Message::CategoryToggleExpand(id) => ws_state.toggle_category_expanded(id),
        Message::CategorySelect(filter) => ws_state.category_selected = filter,
```

- [ ] **Step 4: 轮询接线(`app.rs`)**

`grep -n "poll_todo_if_visible\|TODO_POLL_INTERVAL" crates/dozer-app/src/app.rs`
找到现有轮询函数,在它调用 `todo::request_todos_refresh(...)` 的同一个
地方(通常紧邻)追加一次
`crates/dozer-app/src/app.rs` 现有 `poll_todo_if_visible` 方法(第
2672-2691 行)完整实现是:

```rust
    pub fn poll_todo_if_visible(&mut self) {
        if !self.todo_panel_visible() {
            return;
        }
        let now = std::time::Instant::now();
        if now.duration_since(self.last_todo_poll_at) < crate::TODO_POLL_INTERVAL {
            return;
        }
        self.last_todo_poll_at = now;
        let Some(project_id) = self.active_project_id else {
            return;
        };
        let client = self.client.clone();
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let emit = move |m: todo::Message| {
            let _ = proxy.send_event(Message::Todo(m));
        };
        todo::request_todos_refresh(project_id, &client, &handle, emit);
    }
```

把最后一行 `todo::request_todos_refresh(project_id, &client, &handle,
emit);` 替换成(`emit` 同样只能用一次,这里跟 Step 3 的
`CategoryMutated` 分支一样需要拆成两份):

```rust
        let emit_todos = emit.clone();
        todo::request_todos_refresh(project_id, &client, &handle, move |m| emit_todos(m));
        todo::request_categories_refresh(project_id, &client, &handle, move |m| emit(m));
    }
```

**这里 `emit` 定义要从 `let emit = move |m: todo::Message| { .. };`
改成可以 `Clone` 的形式**——闭包本身若只捕获 `proxy`(实现了 `Clone`)
且闭包体只是读取捕获值不做消耗性操作,闭包自动是 `Clone` 的(`proxy:
EventLoopProxy<Message>` 实现 `Clone`,闭包捕获它按值存一份,`Clone`
派生对闭包类型是编译器自动推导的,不需要手动加任何 derive)。这一步
按上面的替换直接写、`cargo build` 验证即可,不需要额外改动 `emit` 的
定义。

同样,在写操作成功后触发刷新的地方(`grep -n
"Message::Todo(todo::Message::Mutated" crates/dozer-app/src/app.rs`
能找到几处 `proxy.send_event(Message::Todo(todo::Message::Mutated(res)))`),
**这些地方本任务不用动**——它们是任务本身字段(text/done/rank/plan_date/
dispatch)的写操作确认,不涉及分类,继续只刷 `Loaded` 即可。分类相关的
写操作确认(Task 9/10 会加)会自己发 `Message::Todo(todo::Message::
CategoryMutated(res))`,已经在 Step 3 里处理好双刷新。

- [ ] **Step 5: 编译验证**

Run: `cargo build -p dozer-app`
Expected: 编译通过。

Run: `cargo test -p dozer-app`
Expected: 全部通过(本任务不新增测试——状态接线本身没有独立可测的纯
逻辑,行为在 Task 8/9/10 接上真实 UI 后靠真实 GUI 验证)。

Run: `cargo clippy -p dozer-app --all-targets -- -D warnings`
Run: `cargo fmt`

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/extensions/todo.rs crates/dozer-app/src/app.rs
git commit -m "feat(dozer-app): 分类树状态接线(WorkspaceState/Message/轮询刷新)"
```

---

## Task 8: `dozer-app` — 左侧树只读渲染 + 选中过滤(不含增删改)

**Files:**
- Modify: `crates/dozer-app/src/extensions/todo.rs`
- Modify: `crates/dozer-app/src/app.rs`(`HoverId` 加变体)

**Interfaces:**
- Consumes: `CategoryTreeRow`/`visible_category_rows`/`CategoryFilter`/`filter_todos_by_category`(Task 6)、`WorkspaceState::{categories,category_expanded,category_selected}`(Task 7)、`icons::icon_button_entry`(既有 `byteui` 组件)。
- Produces: 左侧 `sidebar_pane` 在既有状态过滤 `nav` 之下新增一段分类树区域;`todo_list_view` 的可见任务集合改为"既有状态/搜索过滤 ∩ 分类过滤"的交集。这一步**只做展示和选中过滤,不做新建/重命名/删除/reparent/上移下移**(那是 Task 9/10)。

- [ ] **Step 1: `HoverId` 加一个分类行的通用悬停 id**

`grep -n "TodoListCollapse" crates/dozer-app/src/app.rs` 找到 `HoverId`
枚举里的 `TodoListCollapse` 变体(现有第 230 行附近),之后加:

```rust
    /// Todo 面板分类树的某一行(展开箭头 + 行本身共用一个悬停态,按分类
    /// id 区分,同一时刻可能有多行渲染,不能用全局标识共用)。
    TodoCategoryRow(i64),
```

- [ ] **Step 2: 渲染分类树区域,接到左侧 `nav` 下方**

在 `crates/dozer-app/src/extensions/todo.rs` 的 `view` 函数里,现有
`let mut nav = column![].spacing(4).padding([12, 8]); for (filter,
count) in counts { nav = nav.push(todo_category_button(filter, count,
ws_state.filter)); }`(现有第 995-998 行)之后,`let sidebar_pane
= ...` 赋值语句(现有第 999 行)之前插入:

```rust
    // ---- 分类树导航:钉在状态过滤 nav 下方,同一块左栏滚动区域 ----
    let category_nav = category_tree_nav(app, ws_state);
```

把 `sidebar_pane` 的 `column![header, nav, space::Space::new()..., ...]`
(现有第 1001-1006 行)改成:

```rust
        container(
            column![
                header,
                nav,
                category_nav,
                space::Space::new().height(Length::Fill),
                todo_clear_footer_bar(ws_state),
            ]
            .height(Length::Fill),
        )
```

在文件里(建议紧邻 `todo_category_button` 函数,现有第 2031-2096 行,
放在它之后)新增：

```rust
/// 分类树导航区:钉顶的"全部"/"未分类"伪节点 + 用户自建分类节点(可
/// 展开/收起、点选切过滤)。本函数(Task 8)只做展示 + 选中,右键菜单/
/// 增删改在 Task 9/10 接入,这里先占好每行的右键落点消息
/// (`CategoryContextMenuOpen`,Task 9 才会真正处理,本任务先把消息
/// 发出来但 App 侧还没有对应分支——**这会导致编译通过但点右键没反应,
/// 是预期的中间状态,不是 bug**)。
fn category_tree_nav<'a>(
    app: &App,
    ws_state: &'a WorkspaceState,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let mut col = column![].spacing(2).padding([4, 8]);

    // 伪节点"全部"/"未分类",样式复用 todo_category_button 的选中态视觉
    // (金色边框+cream 文字选中,dim 未选中),但消息换成 CategorySelect。
    col = col.push(category_pseudo_row(
        icons::IconKind::CircleSmall,
        "全部",
        ws_state.category_selected() == CategoryFilter::All,
        Message::CategorySelect(CategoryFilter::All),
    ));
    col = col.push(category_pseudo_row(
        icons::IconKind::CircleSmall,
        "未分类",
        ws_state.category_selected() == CategoryFilter::Uncategorized,
        Message::CategorySelect(CategoryFilter::Uncategorized),
    ));

    let rows = visible_category_rows(ws_state.categories(), ws_state.category_expanded());
    for row in rows {
        let active = ws_state.category_selected() == CategoryFilter::Node(row.id);
        let fg = if active {
            byteui::theme::color::current().cream
        } else {
            byteui::theme::color::current().dim
        };
        let chevron: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
            if row.has_children {
                icons::icon_button_entry(
                    if row.expanded {
                        icons::IconKind::ChevronDown
                    } else {
                        icons::IconKind::ChevronRight
                    },
                    byteui::theme::icon_size::row(),
                    active,
                    !active,
                    app.hover_progress(HoverId::TodoCategoryRow(row.id)),
                    false,
                    byteui::theme::geometry::tab_button_size(),
                    true,
                    Message::CategoryToggleExpand(row.id),
                    move |hovered| Message::Hover(HoverId::TodoCategoryRow(row.id), hovered),
                    if row.expanded { "收起" } else { "展开" },
                )
            } else {
                space::Space::new()
                    .width(byteui::theme::geometry::tab_button_size())
                    .into()
            };
        let label = button(
            row![
                chevron,
                text(row.name.clone())
                    .size(byteui::theme::font::body())
                    .color(fg),
            ]
            .spacing(4)
            .align_y(iced_widget::core::alignment::Vertical::Center),
        )
        .on_press(Message::CategorySelect(CategoryFilter::Node(row.id)))
        .width(Length::Fill)
        .padding([6, 4 + (row.depth as u16) * 16])
        .style(move |_t: &iced_widget::Theme, _s| button::Style {
            background: if active {
                Some(byteui::theme::color::current().card.into())
            } else {
                None
            },
            text_color: fg,
            border: Border {
                color: if active {
                    byteui::theme::color::current().gold
                } else {
                    Color::TRANSPARENT
                },
                width: if active { 1.0 } else { 0.0 },
                radius: 6.0.into(),
            },
            ..button::Style::default()
        });
        col = col.push(label);
    }
    col.into()
}

/// "全部"/"未分类"两个不可删除/不可右键的伪节点行,视觉对齐真实分类
/// 节点但没有展开箭头。
fn category_pseudo_row<'a>(
    icon: icons::IconKind,
    label: &'a str,
    active: bool,
    on_press: Message,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let fg = if active {
        byteui::theme::color::current().cream
    } else {
        byteui::theme::color::current().dim
    };
    button(
        row![
            icons::view(
                icon,
                byteui::theme::icon_size::row(),
                if active {
                    byteui::theme::color::current().gold
                } else {
                    byteui::theme::color::current().dim
                }
            ),
            text(label).size(byteui::theme::font::body()).color(fg),
        ]
        .spacing(8)
        .align_y(iced_widget::core::alignment::Vertical::Center),
    )
    .on_press(on_press)
    .width(Length::Fill)
    .padding([6, 10])
    .style(move |_t: &iced_widget::Theme, _s| button::Style {
        background: if active {
            Some(byteui::theme::color::current().card.into())
        } else {
            None
        },
        text_color: fg,
        border: Border {
            color: if active {
                byteui::theme::color::current().gold
            } else {
                Color::TRANSPARENT
            },
            width: if active { 1.0 } else { 0.0 },
            radius: 6.0.into(),
        },
        ..button::Style::default()
    })
    .into()
}
```

**核对 `icons::IconKind` 是否已有 `ChevronDown`/`ChevronRight` 两个成员**
(`grep -n "ChevronDown\|ChevronRight" crates/byteui/src/interaction/icons.rs`)——
files.rs 的文件夹展开箭头大概率复用的就是这两个,若名字不同以实际
枚举成员为准替换上面代码里的名字。

- [ ] **Step 3: 把分类过滤接进 `todo_list_view`**

修改 `todo_list_view` 函数(现有第 1279-1284 行)开头:

```rust
fn todo_list_view<'a>(
    app: &App,
    ws_state: &'a WorkspaceState,
    states: &[TodoState],
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let status_visible_idx = filter_todos(&ws_state.items, states, ws_state.filter, &ws_state.search);
    let category_allowed_ids =
        filter_todos_by_category(&ws_state.items, ws_state.categories(), ws_state.category_selected());
    let visible_idx: Vec<usize> = status_visible_idx
        .into_iter()
        .filter(|&i| category_allowed_ids.contains(&ws_state.items[i].id))
        .collect();
```

原来第一行 `let visible_idx = filter_todos(...)` 整行替换成上面这段
(变量名 `visible_idx` 保持不变,后续代码不用改)。

- [ ] **Step 4: 编译 + 真实 GUI 验证**

Run: `cargo build -p dozer-app`
Expected: 编译通过。

Run: `cargo clippy -p dozer-app --all-targets -- -D warnings`
Run: `cargo fmt`

真实 GUI 验证(单测覆盖不到渲染):`cargo run -p dozer-app` 打开一个
项目,Todo 面板左栏状态过滤下方应该出现"全部"/"未分类"两行(此时
真实分类节点列表是空的,因为 Task 9 才接入新建);点"未分类"应该看到
现有全部任务(因为都还没有分类);点"全部"应该看到全部任务。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/extensions/todo.rs crates/dozer-app/src/app.rs
git commit -m "feat(dozer-app): 分类树左侧只读渲染 + 按分类过滤任务列表"
```

---

## Task 9: `dozer-app` — 右键菜单(新建/重命名/删除)+ 行内改名

**Files:**
- Modify: `crates/dozer-app/src/extensions/todo.rs`
- Modify: `crates/dozer-app/src/app.rs`

**Interfaces:**
- Consumes: `Client::{add_category,rename_category,delete_category}`(Task 5)、`crate::menu::{shell,item}`(既有共享组件)、`self.files.last_right_click()`(既有坐标捕获,`ProjectLinkMenu` 同款复用)。
- Produces: `todo::Message::{CategoryContextMenuOpen(i64), CategoryRenameStart(i64), CategoryRenameEdit(text_editor::Action), CategoryRenameSubmit, CategoryNewChild(i64), CategoryNewSibling(Option<i64>), CategoryDelete(i64)}`;`App` 新增字段 `category_context_menu: Option<CategoryContextMenu>`。这是 Task 10(上移下移/reparent/任务分类 chip)之前必须先落地的交互面(新建分类之后 Task 10 才有节点可挪)。

**注:行内改名沿用 `extensions::files` 项目树重命名的既有实现方式——
单行 `String` 草稿 + `byteui::form::input_text::view`(原生 `text_input`,
`on_submit` 直接回车提交,不需要像 `text_editor` 那样手动拦截
`Edit::Enter`)+ 一次性聚焦标记 + `App` 层"失焦边缘触发提交"(镜像
`files.rs::TreeEdit`/`App::set_tree_edit_focused`,现有
`crates/dozer-app/src/extensions/files.rs` 第 29-33、1302-1335 行,
`crates/dozer-app/src/app.rs` 第 3023-3040 行),不是凭空新发明,也
**不是**照抄本文件里任务内容编辑用的 `text_editor`(那个是多行,分类名
是单行,`input_text` 更贴切)。**

- [ ] **Step 1: `WorkspaceState` 加改名态 + `Message` 加变体**

在 `WorkspaceState` 结构体里,`category_selected: CategoryFilter,`
字段之后加:

```rust
    /// 分类树行内改名态(分类 id, 草稿字符串),镜像
    /// `files::TreeEdit`——单行文本,不用 `text_editor::Content`。
    category_renaming: Option<(i64, String)>,
    /// 改名框是否持有 iced 真实焦点,镜像 `files.rs::tree_edit_focused`。
    category_rename_focused: bool,
    /// 一次性聚焦标记,镜像 `files.rs::tree_edit_focus_pending`。
    category_rename_focus_pending: bool,
```

在 `impl WorkspaceState` 里加访问器(镜像
`files.rs` 对应的 `tree_edit_focused`/`take_tree_edit_focus_pending`/
`submit_tree_edit` 三件套):

```rust
    pub fn category_renaming(&self) -> Option<(i64, &str)> {
        self.category_renaming
            .as_ref()
            .map(|(id, draft)| (*id, draft.as_str()))
    }

    pub fn category_rename_focused(&self) -> bool {
        self.category_rename_focused
    }

    pub fn set_category_rename_focused_flag(&mut self, focused: bool) {
        self.category_rename_focused = focused;
    }

    pub fn take_category_rename_focus_pending(&mut self) -> bool {
        std::mem::take(&mut self.category_rename_focus_pending)
    }

    /// 草稿变化时调用(`on_input`)。
    fn set_category_rename_draft(&mut self, text: String) {
        if let Some((_, draft)) = self.category_renaming.as_mut() {
            *draft = text;
        }
    }

    /// 提交(回车/失焦边缘触发共用)。返回 `Some((id, new_name))` 表示有
    /// 改动需要落盘(空白/未变都视为无改动,直接丢弃草稿)。
    fn commit_category_rename(&mut self) -> Option<(i64, String)> {
        let (id, draft) = self.category_renaming.take()?;
        let new_name = draft.trim().to_string();
        if new_name.is_empty() {
            return None;
        }
        let current = self.categories.iter().find(|c| c.id == id)?;
        if current.name == new_name {
            return None;
        }
        Some((id, new_name))
    }
```

在 `Message` 枚举里,`CategorySelect(CategoryFilter),` 之后加:

```rust
    /// 右键某个分类节点(伪节点"全部"/"未分类"不触发这个消息)。内核
    /// 拦截转发成 `App::todo_category_context_menu`(同 `ProjectLinkMenu`
    /// 的既有接线方式),不进 `todo::update`。
    CategoryContextMenuOpen(i64),
    /// 右键菜单"新建子分类":先用默认名新建(RPC 返回真实 id),再立刻
    /// 进入该节点的改名态,让用户直接输入真实名字。
    CategoryNewChild(i64),
    /// 右键菜单"新建同级分类":`parent_id` 是被右键节点的父节点(`None`
    /// 表示新建一个顶层分类,对应"未分类"/空白处右键 → 全局"新建分类"
    /// 入口,Task 10 接入)。
    CategoryNewSibling(Option<i64>),
    /// 右键菜单"删除"。
    CategoryDelete(i64),
    /// 右键菜单"重命名"/新建后自动触发:进入行内改名态。
    CategoryRenameStart(i64),
    /// 改名框草稿变化(`text_input::on_input`,给全量当前字符串,同
    /// `SearchInput` 同构)。
    CategoryRenameEdit(String),
    /// 改名框提交(回车 `on_submit`;失焦边缘触发走 `App::
    /// set_category_rename_focused`,不经过这条消息,同
    /// `set_tree_edit_focused` 的既有分工)。
    CategoryRenameSubmit,
```

- [ ] **Step 2: `update` 里的处理分支**

在 `update` 函数里,`Message::CategorySelect(filter) => ...`(Task 7
Step 3 加的)之后追加:

```rust
        Message::CategoryContextMenuOpen(_) => {} // 内核拦截,见 app.rs
        Message::CategoryNewChild(parent_id) => {
            let client = client.clone();
            let project_id_owned = project_id;
            handle.spawn(async move {
                let res = client
                    .add_category(project_id_owned, Some(parent_id), "新分类")
                    .await;
                match res {
                    Ok(category) => {
                        emit(Message::CategoryRenameStart(category.id));
                        emit(Message::CategoryMutated(Ok(())));
                    }
                    Err(e) => emit(Message::CategoryMutated(Err(e.to_string()))),
                }
            });
        }
        Message::CategoryNewSibling(parent_id) => {
            let client = client.clone();
            let project_id_owned = project_id;
            handle.spawn(async move {
                let res = client.add_category(project_id_owned, parent_id, "新分类").await;
                match res {
                    Ok(category) => {
                        emit(Message::CategoryRenameStart(category.id));
                        emit(Message::CategoryMutated(Ok(())));
                    }
                    Err(e) => emit(Message::CategoryMutated(Err(e.to_string()))),
                }
            });
        }
        Message::CategoryDelete(id) => {
            let client = client.clone();
            handle.spawn(async move {
                let res = client.delete_category(id).await.map_err(|e| e.to_string());
                emit(Message::CategoryMutated(res));
            });
        }
        Message::CategoryRenameStart(id) => {
            let Some(current) = ws_state.categories.iter().find(|c| c.id == id) else {
                return;
            };
            ws_state.category_renaming = Some((id, current.name.clone()));
            ws_state.category_rename_focus_pending = true;
        }
        Message::CategoryRenameEdit(text) => ws_state.set_category_rename_draft(text),
        Message::CategoryRenameSubmit => {
            if let Some((id, new_name)) = ws_state.commit_category_rename() {
                let client = client.clone();
                handle.spawn(async move {
                    let res = client
                        .rename_category(id, &new_name)
                        .await
                        .map(|_| ())
                        .map_err(|e| e.to_string());
                    emit(Message::CategoryMutated(res));
                });
            }
        }
```

- [ ] **Step 3: `app.rs` 内核拦截 + 右键菜单浮层状态**

`grep -n "struct ProjectLinkMenu" crates/dozer-app/src/app.rs` 找到
（Task 3 前面已探过在第 1958-1963 行）,之后加一个新结构体:

```rust
/// Todo 分类树节点右键菜单浮层状态,镜像 `ProjectLinkMenu`。
struct CategoryContextMenu {
    x: f32,
    y: f32,
    /// 被右键的分类节点 id。
    id: i64,
}
```

`grep -n "project_link_menu: Option<ProjectLinkMenu>" crates/dozer-app/src/app.rs`
找到 `App` 结构体里的字段(现有第 2105 行),之后加:

```rust
    category_context_menu: Option<CategoryContextMenu>,
```

`grep -n "Message::ProjectLinkContextMenuClose =>" crates/dozer-app/src/app.rs`
找到对应的关闭消息处理分支(现有第 4861 行附近),先在**顶层 `Message`
枚举**(不是 `todo::Message`,是 `app.rs` 自己的 `Message`)里加一个
`CategoryContextMenuClose` 变体(紧邻 `ProjectLinkContextMenuClose`
定义处,现有第 1860 行),再在 `update` 里加处理:

```rust
            Message::CategoryContextMenuClose => {
                self.category_context_menu = None;
            }
```

在 `Message::Todo(msg) => match msg { .. }`(Task 7 之前已经存在,
现有第 4410-4419 行)里,`todo::Message::ToggleListCollapse => { .. }`
之后加一条拦截:

```rust
                todo::Message::CategoryContextMenuOpen(id) => {
                    self.todo_category_context_menu(id);
                }
```

在 `App` 的 `impl` 块里(`fn project_link_context_menu`,现有第 3795
行附近)之后加对应方法:

```rust
    /// 打开分类树节点的右键菜单。坐标复用 `files.last_right_click()`
    /// (同 `project_link_context_menu` 的既有接线方式)。
    fn todo_category_context_menu(&mut self, id: i64) {
        let (x, y) = self.files.last_right_click();
        self.files.close_context_menu();
        self.category_context_menu = Some(CategoryContextMenu { x, y, id });
    }
```

在渲染右键菜单的大 `if-else` 链里(`grep -n
"self.project_link_menu.is_some()" crates/dozer-app/src/app.rs`,现有
第 7361-7371 行)之后加一段(位置任意,只要在 `else if self.
text_input_menu.is_some() { .. }` 之前或之后均可,建议紧邻
`project_link_menu` 那一段):

```rust
        } else if self.category_context_menu.is_some() {
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::CategoryContextMenuClose);
            stack![base, dismiss, self.category_context_menu_popup()]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
```

**这一段是 `if-else` 链中间的一个分支,不是独立语句——插入时要接续前一
个分支的 `} else if ... {` 结构,具体缩进/花括号配对以现有代码的
`project_link_menu`/`text_input_menu` 两段为准比照写,不要破坏链式
结构。**

在 `fn project_link_context_menu_popup` 方法(现有第 7030-7059 行)
之后加对应的菜单内容方法:

```rust
    fn category_context_menu_popup<'a>(
        &self,
    ) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
        let menu = match &self.category_context_menu {
            Some(m) => m,
            None => return column![].into(),
        };
        let id = menu.id;
        // 被右键节点的 parent_id,给"新建同级分类"用(同级 = 挂在同一个
        // parent_id 下)。`active_workspace()` 是 `App` 上现成的只读访问器
        // (`app.rs:2499`),取不到(没有聚焦项目,理论不可能发生在右键
        // 菜单已经打开的前提下)就退化成顶层。
        let sibling_parent_id = self
            .active_workspace()
            .and_then(|ws| ws.todo.categories().iter().find(|c| c.id == id).and_then(|c| c.parent_id));
        let items: Vec<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>> = vec![
            crate::menu::item::<Message>(
                Some(icons::IconKind::SquarePlus),
                "新建子分类",
                Message::Todo(todo::Message::CategoryNewChild(id)),
            ),
            crate::menu::item::<Message>(
                Some(icons::IconKind::SquarePlus),
                "新建同级分类",
                Message::Todo(todo::Message::CategoryNewSibling(sibling_parent_id)),
            ),
            crate::menu::item::<Message>(
                Some(icons::IconKind::Rename),
                "重命名",
                Message::Todo(todo::Message::CategoryRenameStart(id)),
            ),
            crate::menu::item::<Message>(
                Some(icons::IconKind::Trash),
                "删除",
                Message::Todo(todo::Message::CategoryDelete(id)),
            ),
        ];
        let list: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
            crate::menu::shell(items, Length::Shrink);
        container(list)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(Padding {
                top: menu.y,
                left: menu.x,
                right: 0.0,
                bottom: 0.0,
            })
            .into()
    }
```

- [ ] **Step 4: 行内改名的渲染 + 焦点捕获**

镜像 `content_field_id`/`CaptureContentEditFocus`/
`take_content_edit_focused`(现有第 511-541 行),在同一段之后加:

```rust
pub fn category_rename_field_id() -> Id {
    Id::new("todo-category-rename-field")
}

static CATEGORY_RENAME_FOCUSED: std::sync::LazyLock<std::sync::Mutex<bool>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(false));

pub fn take_category_rename_focused() -> bool {
    std::mem::replace(&mut *CATEGORY_RENAME_FOCUSED.lock().unwrap(), false)
}

pub struct CaptureCategoryRenameFocus;
impl Operation<()> for CaptureCategoryRenameFocus {
    fn focusable(&mut self, id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Focusable) {
        if id == Some(&category_rename_field_id()) {
            *CATEGORY_RENAME_FOCUSED.lock().unwrap() = state.is_focused();
        }
    }

    fn traverse(&mut self, operate: &mut dyn for<'a> FnMut(&'a mut (dyn Operation<()> + 'a))) {
        operate(self);
    }
}

/// 分类树一行的行内改名输入框,镜像 `files.rs::tree_edit_row`(同款
/// `byteui::form::input_text::view` 单行 `text_input`,`on_submit` 直接
/// 回车提交,缩进用外层 `Padding::left` 换算,不拼进文本内容)。
fn category_rename_row<'a>(depth: usize, draft: &'a str) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let indent_px = 4.0 + depth as f32 * 16.0; // 对齐 category_tree_nav 里真实行的缩进算法
    let field = container(byteui::form::input_text::view(
        "",
        draft,
        false,
        Some(category_rename_field_id()),
        false,
        Some(Message::CategoryRenameSubmit),
        false,
        Message::CategoryRenameEdit,
    ))
    .width(Length::Fill)
    .padding(Padding {
        left: indent_px,
        ..Padding::default()
    });
    byteui::interaction::context_menu::wrap(
        field.into(),
        Some(Message::TextInputMenuOpen(crate::app::TextInputTarget {
            id: category_rename_field_id(),
            secure: false,
        })),
    )
}
```

把 `category_tree_nav` 里 Task 8 Step 2 写的 `for row in rows { .. }`
整段替换成(唯一的改动是循环体开头加了一个 `if let` 分支,分支内部
是新代码,分支之后的原有代码原样保留):

```rust
    for row in rows {
        if let Some((renaming_id, draft)) = ws_state.category_renaming() {
            if renaming_id == row.id {
                col = col.push(category_rename_row(row.depth, draft));
                continue;
            }
        }
        let active = ws_state.category_selected() == CategoryFilter::Node(row.id);
        let fg = if active {
            byteui::theme::color::current().cream
        } else {
            byteui::theme::color::current().dim
        };
        let chevron: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
            if row.has_children {
                icons::icon_button_entry(
                    if row.expanded {
                        icons::IconKind::ChevronDown
                    } else {
                        icons::IconKind::ChevronRight
                    },
                    byteui::theme::icon_size::row(),
                    active,
                    !active,
                    app.hover_progress(HoverId::TodoCategoryRow(row.id)),
                    false,
                    byteui::theme::geometry::tab_button_size(),
                    true,
                    Message::CategoryToggleExpand(row.id),
                    move |hovered| Message::Hover(HoverId::TodoCategoryRow(row.id), hovered),
                    if row.expanded { "收起" } else { "展开" },
                )
            } else {
                space::Space::new()
                    .width(byteui::theme::geometry::tab_button_size())
                    .into()
            };
        let label = button(
            row![
                chevron,
                text(row.name.clone())
                    .size(byteui::theme::font::body())
                    .color(fg),
            ]
            .spacing(4)
            .align_y(iced_widget::core::alignment::Vertical::Center),
        )
        .on_press(Message::CategorySelect(CategoryFilter::Node(row.id)))
        .width(Length::Fill)
        .padding([6, 4 + (row.depth as u16) * 16])
        .style(move |_t: &iced_widget::Theme, _s| button::Style {
            background: if active {
                Some(byteui::theme::color::current().card.into())
            } else {
                None
            },
            text_color: fg,
            border: Border {
                color: if active {
                    byteui::theme::color::current().gold
                } else {
                    Color::TRANSPARENT
                },
                width: if active { 1.0 } else { 0.0 },
                radius: 6.0.into(),
            },
            ..button::Style::default()
        });
        col = col.push(label);
    }
```

- [ ] **Step 5: `App::set_category_rename_focused`(失焦落盘,镜像 `set_todo_content_focused`)**

在 `crates/dozer-app/src/app.rs` 里,`fn set_todo_content_focused`
(现有第 3213-3242 行)之后插入:

```rust
    /// 分类改名框真实焦点态每帧写回;失焦边缘(`was_focused && !focused`)
    /// 触发一次提交(镜像 `set_todo_content_focused`,只是落盘方法换成
    /// `rename_category`,成功/失败都触发 `CategoryMutated` 刷新)。
    pub fn set_category_rename_focused(&mut self, focused: bool) {
        let Some(ws) = self.active_workspace_mut() else {
            return;
        };
        let was_focused = ws.todo.category_rename_focused();
        let pending_commit = if was_focused && !focused {
            ws.todo.commit_category_rename_for_blur()
        } else {
            None
        };
        ws.todo.set_category_rename_focused_flag(focused);
        if let Some((id, new_name)) = pending_commit {
            let client = self.client.clone();
            let handle = self.handle.clone();
            let proxy = self.proxy.clone();
            handle.spawn(async move {
                let res = client
                    .rename_category(id, &new_name)
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string());
                let _ = proxy.send_event(Message::Todo(todo::Message::CategoryMutated(res)));
            });
        }
    }
```

`commit_category_rename` 是 `todo.rs` 里的私有方法(Task 9 Step 1),
`app.rs` 在别的模块里调不到——在 `crates/dozer-app/src/extensions/
todo.rs` 的 `impl WorkspaceState` 里给 `commit_category_rename` 加一个
`pub(crate)` 包装(紧邻它自己,不要把 `commit_category_rename` 本身
从 `fn` 改成 `pub`,理由同 `commit_content_edit` 就是直接 `pub`——两者
不一致是因为 `commit_content_edit` 现有就是 `pub fn`,这里为了不改
既有方法的可见性,新加一层同名转发):

```rust
    /// `app.rs::set_category_rename_focused` 失焦边缘触发用的公开入口,
    /// 转发到 `commit_category_rename`。
    pub(crate) fn commit_category_rename_for_blur(&mut self) -> Option<(i64, String)> {
        self.commit_category_rename()
    }
```

- [ ] **Step 6: `main.rs` 每帧焦点捕获接线**

在 `crates/dozer-app/src/main.rs` 里,现有 `let content_edit_focused =
if matches!(app.left_view(), crate::app::PanelKind::Todo) { .. } else {
false };`(现有第 2417-2427 行)之后插入:

```rust
                                // Todo 分类树行内改名框:同款每帧查真实
                                // 焦点态,只在 Todo 左栏可见时跑。
                                let category_rename_focused =
                                    if matches!(app.left_view(), crate::app::PanelKind::Todo) {
                                        run_operate(
                                            &mut interface,
                                            renderer,
                                            &mut extensions::todo::CaptureCategoryRenameFocus,
                                        );
                                        extensions::todo::take_category_rename_focused()
                                    } else {
                                        false
                                    };
```

在现有 `app.set_todo_content_focused(content_edit_focused);`(现有第
2699 行)之后加:

```rust
                                app.set_category_rename_focused(category_rename_focused);
```

- [ ] **Step 7: 编译 + clippy + fmt**

Run: `cargo build -p dozer-app`
Expected: 编译通过。

Run: `cargo clippy -p dozer-app --all-targets -- -D warnings`
Run: `cargo fmt`

- [ ] **Step 8: 真实 GUI 验证**

`cargo run -p dozer-app`:右键"全部"/"未分类"应该没反应(伪节点不可
右键,本任务没有给它们接右键消息);右键一个空白处应该暂时也没有
入口(全局"新建分类"入口留给 Task 10 的 footer 按钮)。为了能测试,
临时可以在 `category_pseudo_row("未分类", ...)` 那一行手动改成也发
`CategoryContextMenuOpen`来触发"新建同级分类"验证增删改流程,验证完
后**改回来**(未分类伪节点不该被右键删除/重命名,这行改动只是为了
手工验证,不要提交进最终 diff)。更稳妥的验证路径是等 Task 10 加上
真正的"新建分类"入口按钮后再一并验证——如果赶时间,这一步的真实 GUI
验证可以推迟到 Task 10 完成后一次性做,但 `cargo build`/`clippy`/`fmt`
三项本任务必须现在就过。

- [ ] **Step 9: Commit**

```bash
git add crates/dozer-app/src/extensions/todo.rs crates/dozer-app/src/app.rs \
  crates/dozer-app/src/main.rs
git commit -m "feat(dozer-app): 分类树右键菜单(新建/重命名/删除)+ 行内改名"
```

---

## Task 10: `dozer-app` — 上移/下移/移动到...(reparent)+ 任务行分类 chip

**Files:**
- Modify: `crates/dozer-app/src/extensions/todo.rs`
- Modify: `crates/dozer-app/src/app.rs`

**Interfaces:**
- Consumes: `Client::{move_category_sibling,reparent_category,set_todo_category}`(Task 5)、`CategoryContextMenu`(Task 9)、任务卡片渲染现有代码(`todo_list_view`/`todo_card`,Task 8 已知位置)。
- Produces: 右键菜单补上"上移"/"下移"/"移动到...";一个可复用的分类选择器浮层(`CategoryPicker`,`App` 级状态,`target: CategoryPickerTarget { Todo(i64), Category(i64) }` 区分是"给任务挂分类"还是"给分类 reparent"两种用途);任务卡片新增分类标签 chip,点击打开选择器。这是本次分类树功能收口的最后一个任务。

- [ ] **Step 1: 右键菜单加"上移"/"下移"/"移动到..."**

在 `todo::Message` 里,`CategoryRenameSubmit,` 之后加:

```rust
    /// 与前一个/后一个同级节点交换顺序。
    CategoryMoveSibling(i64, dozer_core::protocol::CategoryMoveDirection),
    /// 打开"移动到..."选择器(内核拦截,转发到
    /// `App::todo_category_picker_open_for_category`)。
    CategoryReparentPickerOpen(i64),
```

在 `update` 里,`Message::CategoryRenameSubmit => { .. }` 之后加:

```rust
        Message::CategoryMoveSibling(id, direction) => {
            let client = client.clone();
            handle.spawn(async move {
                let res = client
                    .move_category_sibling(id, direction)
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string());
                emit(Message::CategoryMutated(res));
            });
        }
        Message::CategoryReparentPickerOpen(_) => {} // 内核拦截,见 app.rs
```

在 `crates/dozer-app/src/app.rs` 的 `Message::Todo(msg) => match msg {
.. }` 里,`todo::Message::CategoryContextMenuOpen(id) => { .. }`
(Task 9 Step 3 加的)之后加:

```rust
                todo::Message::CategoryReparentPickerOpen(id) => {
                    self.todo_category_picker_open(CategoryPickerTarget::Category(id));
                }
```

在 `category_context_menu_popup` 方法(Task 9 Step 3)的 `items`
`Vec` 里,`"删除"` 那一项之前插入三项:

```rust
            crate::menu::item::<Message>(
                Some(icons::IconKind::ChevronUp),
                "上移",
                Message::Todo(todo::Message::CategoryMoveSibling(
                    id,
                    dozer_core::protocol::CategoryMoveDirection::Up,
                )),
            ),
            crate::menu::item::<Message>(
                Some(icons::IconKind::ChevronDown),
                "下移",
                Message::Todo(todo::Message::CategoryMoveSibling(
                    id,
                    dozer_core::protocol::CategoryMoveDirection::Down,
                )),
            ),
            crate::menu::item::<Message>(
                Some(icons::IconKind::FolderOpen),
                "移动到...",
                Message::Todo(todo::Message::CategoryReparentPickerOpen(id)),
            ),
```

（`icons::IconKind::ChevronUp`/`FolderOpen` 若不存在,
`grep -n "pub enum IconKind" -A 80 crates/byteui/src/interaction/icons.rs`
核对实际可用的图标名后替换。）

- [ ] **Step 2: 通用分类选择器(`CategoryPicker`)——App 级浮层状态**

在 `app.rs` 里,`struct CategoryContextMenu { .. }`(Task 9 Step 3)
之后加:

```rust
/// 分类选择器要挂靠的目标:给任务挂分类,还是给分类节点 reparent。
/// 两种场景共用同一份"点树选一个节点"的浮层交互,只是选中后调用的
/// `Client` 方法不同(`set_todo_category` vs `reparent_category`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CategoryPickerTarget {
    Todo(i64),
    Category(i64),
}

/// 分类选择器浮层状态:定位坐标 + 目标。渲染内容复用
/// `category_tree_nav` 同一份树数据(只读展示,不接展开/右键,选中即
/// 关闭并提交)。
struct CategoryPicker {
    x: f32,
    y: f32,
    target: CategoryPickerTarget,
}
```

`App` 结构体里,`category_context_menu: Option<CategoryContextMenu>,`
之后加:

```rust
    category_picker: Option<CategoryPicker>,
```

顶层 `Message` 枚举里(`Message::CategoryContextMenuClose` 旁边)加:

```rust
    CategoryPickerClose,
    /// 选择器里点了某一项:`None` = "未分类"(仅 `Todo` target 下有效,
    /// `Category` target 选"未分类"表示挪到顶层)。
    CategoryPickerSelect(Option<i64>),
```

`update` 里加:

```rust
            Message::CategoryPickerClose => {
                self.category_picker = None;
            }
            Message::CategoryPickerSelect(chosen) => {
                let Some(picker) = self.category_picker.take() else {
                    return;
                };
                match picker.target {
                    CategoryPickerTarget::Todo(todo_id) => {
                        let client = self.client.clone();
                        let handle = self.handle.clone();
                        let proxy = self.proxy.clone();
                        handle.spawn(async move {
                            let res = client
                                .set_todo_category(todo_id, chosen)
                                .await
                                .map(|_| ())
                                .map_err(|e| e.to_string());
                            let _ = proxy.send_event(Message::Todo(todo::Message::CategoryMutated(res)));
                        });
                    }
                    CategoryPickerTarget::Category(category_id) => {
                        let client = self.client.clone();
                        let handle = self.handle.clone();
                        let proxy = self.proxy.clone();
                        handle.spawn(async move {
                            let res = client
                                .reparent_category(category_id, chosen)
                                .await
                                .map(|_| ())
                                .map_err(|e| e.to_string());
                            let _ = proxy.send_event(Message::Todo(todo::Message::CategoryMutated(res)));
                        });
                    }
                }
            }
```

给 `App` 加两个打开方法(紧邻 `todo_category_context_menu`):

```rust
    fn todo_category_picker_open(&mut self, target: CategoryPickerTarget) {
        let (x, y) = self.last_cursor;
        self.category_picker = Some(CategoryPicker { x, y, target });
    }
```

（`self.last_cursor` 的具体字段名/类型以 `todo_message` 方法里已经
用过的 `let last_cursor = self.last_cursor;`,Task 7 探过是
`(f32, f32)`,直接复用即可。）

- [ ] **Step 3: 渲染选择器浮层**

在渲染右键菜单的 `if-else` 链里,`self.category_context_menu.is_some()`
分支(Task 9 Step 3)之后加一段:

```rust
        } else if self.category_picker.is_some() {
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::CategoryPickerClose);
            stack![base, dismiss, self.category_picker_popup()]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
```

加对应的内容渲染方法(紧邻 `category_context_menu_popup`):

```rust
    fn category_picker_popup<'a>(
        &self,
    ) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
        let Some(picker) = &self.category_picker else {
            return column![].into();
        };
        let categories = self
            .active_workspace()
            .map(|ws| ws.todo.categories().to_vec())
            .unwrap_or_default();
        let mut list = column![].spacing(2);
        list = list.push(
            button(text("未分类").size(byteui::theme::font::body()))
                .on_press(Message::CategoryPickerSelect(None))
                .width(Length::Fill)
                .padding([6, 10]),
        );
        // 复用一份没有展开态(全展开)的拍平——选择器只做单次选择,不需要
        // 折叠交互,直接把整棵树按 depth 缩进平铺出来最简单。
        let all_expanded: std::collections::HashSet<i64> =
            categories.iter().map(|c| c.id).collect();
        for row in todo::visible_category_rows(&categories, &all_expanded) {
            list = list.push(
                button(
                    text(format!("{}{}", "  ".repeat(row.depth), row.name))
                        .size(byteui::theme::font::body()),
                )
                .on_press(Message::CategoryPickerSelect(Some(row.id)))
                .width(Length::Fill)
                .padding([6, 10]),
            );
        }
        container(list)
            .width(Length::Fixed(220.0))
            .padding(Padding {
                top: picker.y,
                left: picker.x,
                right: 0.0,
                bottom: 0.0,
            })
            .style(move |_t: &iced_widget::Theme| container::Style {
                background: Some(byteui::theme::color::current().card.into()),
                border: Border {
                    color: byteui::theme::color::current().border,
                    width: 1.0,
                    radius: 6.0.into(),
                },
                ..container::Style::default()
            })
            .into()
    }
```

- [ ] **Step 4: 任务卡片加分类 chip**

`grep -n "fn todo_card" crates/dozer-app/src/extensions/todo.rs` 找到
单张任务卡片的渲染函数,在卡片内容行(通常是任务文字/状态徽章那一
`row![...]`)里追加一个 chip 元素:

```rust
let category_label = ws_state
    .categories()
    .iter()
    .find(|c| Some(c.id) == item.category_id)
    .map(|c| c.name.clone())
    .unwrap_or_else(|| "未分类".to_string());
let chip = button(
    text(category_label)
        .size(byteui::theme::font::caption())
        .color(byteui::theme::color::current().dim),
)
.on_press(Message::CategoryPickerOpenForTodo(item.id))
.padding([2, 8])
.style(|_t: &iced_widget::Theme, _s| button::Style {
    background: Some(byteui::theme::color::current().card.into()),
    border: Border {
        color: byteui::theme::color::current().border,
        width: 1.0,
        radius: 10.0.into(),
    },
    ..button::Style::default()
});
```

**把 `chip` 塞进现有卡片的某个 `row![...]` 里**(挨着计划日期徽章/
派发按钮那一组最自然,具体位置以 `todo_card` 函数实际读到的布局为
准,追加到已有 `row!` 宏的元素列表末尾)。

在 `todo::Message` 里加:

```rust
    /// 点任务卡片的分类 chip,打开分类选择器(内核拦截,转发到
    /// `App::todo_category_picker_open`)。
    CategoryPickerOpenForTodo(i64),
```

在 `update` 里(`Message::CategoryReparentPickerOpen(_) => {}` 之后)加:

```rust
        Message::CategoryPickerOpenForTodo(_) => {} // 内核拦截,见 app.rs
```

在 `app.rs` 的 `Message::Todo(msg) => match msg { .. }` 里加:

```rust
                todo::Message::CategoryPickerOpenForTodo(todo_id) => {
                    self.todo_category_picker_open(CategoryPickerTarget::Todo(todo_id));
                }
```

- [ ] **Step 5: 编译 + clippy + fmt**

Run: `cargo build -p dozer-app`
Run: `cargo clippy -p dozer-app --all-targets -- -D warnings`
Run: `cargo fmt`

- [ ] **Step 6: 真实 GUI 端到端验证(单测覆盖不到)**

`cargo run -p dozer-app`,在一个测试项目里:

1. 右键"未分类"伪节点旁边空白区域进不了菜单是预期的(伪节点不可右键);
   通过右键任意已建好的真实分类节点触发"新建子分类"/"新建同级分类",
   确认新分类立即出现且自动进入改名态,输入名字回车生效。
2. 建两三级嵌套分类,确认展开/收起箭头正确、选中父节点时右侧任务列表
   汇总显示子孙分类下的任务。
3. 右键删除一个有子孙和任务的分类,确认子孙节点消失、原本挂靠的任务
   变回"未分类"、任务本身没有丢。
4. 右键"移动到..."把一个分类挪到另一个分类下,确认树结构正确更新;
   尝试把父节点挪到自己的子孙下面,确认被拒绝(弹层里点了之后没有
   生效、树结构不变——校验失败目前只记 `tracing::warn`,不弹错误
   提示,这是 spec 里"UDS 调用失败按现状降级方式处理"的既定设计,
   不是遗漏)。
5. 右键上移/下移,确认同级顺序正确调整,且首位/末位时点了没反应
   (no-op)。
6. 点任务卡片的分类 chip,选择器弹出,选中某个分类后 chip 文字更新;
   再次点开选"未分类"能摘掉。
7. 重启 `dozerd`(`pkill dozerd` 后重新 `cargo run -p dozer-app` 会
   自动拉起,或参照 CLAUDE.md 里 `cargo run -p dozer-app` 的既有拉起
   机制),确认分类树和任务归属都还在(SQLite 落盘生效)。
8. 用一个没有 `category_id` 列的老 `dozer.db`(可以从当前机器上的
   `~/Library/Application Support/ai.byteboy.dozer/dozer.db` 备份一份
   旧版本,或者干脆临时手写一个不带该列的库文件,同 Task 2 的迁移
   测试思路)验证 dozerd 启动时能自动补列,不报错、不丢历史任务。

- [ ] **Step 7: 全量收尾检查**

Run: `cargo build` (全 workspace)
Run: `cargo test -p dozerd -p dozer-app -p dozer-core -p dozer-client`
Run: `cargo clippy --all-targets -- -D warnings`
Run: `cargo fmt -- --check`

Expected: 四项全绿。

- [ ] **Step 8: Commit**

```bash
git add crates/dozer-app/src/extensions/todo.rs crates/dozer-app/src/app.rs
git commit -m "feat(dozer-app): 分类上移/下移/移动到... + 任务行分类标签选择器"
```

---

## 收尾:提请审阅

十个任务全部完成、Task 10 Step 7 的全量检查通过后,在
`feature/todo-category-tree` 分支上提请代码审阅(`superpowers:
requesting-code-review`),审阅通过后再合并回 `main`——不要自行合并。
