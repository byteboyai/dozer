# Todo 存储迁移到 SQLite Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 Todo 面板的存储从 `.dozer/todo.md` + `todo_meta.json` 两份本地文件,原样平移到 `dozerd` 侧 `dozer.db` 的 SQLite 表,行为不变,任务身份从"文本哈希"换成稳定整数 id,为下一阶段的 agent 指派/轮询/自动完成/评论搭地基。

**Architecture:** 新增 `dozerd::todo::TodoStore`(仿 `BookmarkStore` 模式)托管一张 `todos` 表;`dozer-core::protocol` 新增 `Todo*` 系列 `Request`/`Reply`;`dozer-client::Client` 加对应方法;`dozer-app` 的 `extensions/todo.rs` 从直接 `std::fs` 读写改造成走 UDS 的异步调用(仿 `extensions/browser.rs` 的 `client`/`handle`/`emit` 三件套模式);`dozer-mcp` 新增 4 个工具补齐外部 CLI agent 的操作入口;MARKDOWN 视图 tab 整体移除。

**Tech Stack:** Rust workspace;`rusqlite`(bundled feature,已是 `dozerd` 现有依赖);`iced 0.14`;`rmcp`(`dozer-mcp` 现有依赖);`tokio`。

**Spec:** `docs/superpowers/specs/2026-09-01-dozer-todo-sqlite-migration-design.md`

## Global Constraints

- **在独立分支上开发,不要直接提交到 `main`**(如 `feature/todo-sqlite-migration`)。全部 9 个 Task 完成、`cargo build/test/clippy/fmt` 全绿、Task 9 的手工 GUI 验收清单走完后,提请审阅;审阅通过后再合并回 `main`。这条独立于每个 Task 内部的逐步 commit——分支内可以正常按 Task 提交,只是不要把这些 commit 直接落在 `main` 上。
- SQLite 是任务数据(正文+元数据)的唯一权威源;`.dozer/todo.md`/`todo_meta.json` 迁移后不再被读写,原文件不自动删除。
- 不新增单条任务删除功能;footbar"清空列表"按钮保持现状 no-op。
- 不做推送式实时更新;轮询节奏沿用现有 `TODO_POLL_INTERVAL`(1 秒,自限速)。
- MARKDOWN 视图 tab 整体移除,不做只读降级。
- `dozer-mcp` 新增工具只暴露 `list_todos`/`add_todo`/`toggle_todo`/`edit_todo_text`,不暴露 `reorder`/`plan_date`/`dispatch`(纯 GUI 侧交互,外部 agent 原本碰不到)。
- 每个任务完成后运行 `cargo build`(涉及的 crate)+ 对应 `cargo test` + 提交一次 commit。全部任务完成后跑一次全 workspace 的 `cargo build && cargo clippy --all-targets && cargo fmt`。

---

## Task 1: 协议层新增 TodoInfo + Todo* Request/Reply

**Files:**
- Modify: `crates/dozer-core/src/protocol.rs`

**Interfaces:**
- Produces: `pub struct TodoInfo { id, project_id, text, done, rank, created_ms, completed_at_ms, plan_date, dispatch_session_id, dispatch_at_ms }`;`Request::{ListTodos, AddTodo, ToggleTodo, EditTodoText, ReorderTodo, SetTodoPlanDate, RecordTodoDispatch}`;`Reply::{Todos, Todo}`。这些类型名/字段名是后续所有任务(dozerd store、client、GUI)必须原样匹配的契约。

- [ ] **Step 1: 在 `BookmarkInfo` 定义之后新增 `TodoInfo` 结构体**

在 `crates/dozer-core/src/protocol.rs` 里 `BookmarkInfo` 结构体(现约在 187-194 行)之后插入:

```rust
/// 一条任务(`dozerd` 的 `todos` 表一行)。`id` 是稳定身份,取代 v1 时
/// 靠文本哈希关联元数据的做法(2026-09-01 SQLite 迁移)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TodoInfo {
    pub id: i64,
    pub project_id: i64,
    pub text: String,
    pub done: bool,
    /// 排序键:同一 project 下,展示查询固定 `ORDER BY done, rank`——待办
    /// 按 rank 升序在前,已完成按 rank 升序沉底,同一 done 分组内比较才
    /// 有意义,跨分组数值不保证可比。
    pub rank: i64,
    pub created_ms: u64,
    pub completed_at_ms: Option<u64>,
    /// "MM-DD" 格式,GUI 日历选择器写入。
    pub plan_date: Option<String>,
    pub dispatch_session_id: Option<String>,
    pub dispatch_at_ms: Option<u64>,
}
```

- [ ] **Step 2: 新增 `Request` 变体**

在 `Request` 枚举末尾(`GetSessionSummaryBackfillStatus { cwd: String }` 之后,闭合 `}` 之前)插入:

```rust
    /// 列出某项目全部任务,`ORDER BY done, rank` 排好序返回。
    ListTodos {
        project_id: i64,
    },
    /// 新增一条待办,置顶(rank 小于当前最小待办 rank)。
    AddTodo {
        project_id: i64,
        text: String,
    },
    /// 勾选/取消勾选;`done` 从假变真时服务端顺带写 `completed_at_ms`,
    /// 真变假时清空。`id` 不存在 → `Reply::Error`。
    ToggleTodo {
        id: i64,
        done: bool,
    },
    /// 改任务文字。`id` 不存在 → `Reply::Error`。
    EditTodoText {
        id: i64,
        text: String,
    },
    /// 把 `id` 挪到 `after_id` 之后(`None` = 待办块最前);只在待办子集内
    /// 生效,服务端对该 project 的待办子集做一次完整 rank 重编号。
    ReorderTodo {
        id: i64,
        after_id: Option<i64>,
    },
    /// 写/清计划日期,`None` 表示清空。
    SetTodoPlanDate {
        id: i64,
        plan_date: Option<String>,
    },
    /// 记录一次派发(指派到已有会话)。
    RecordTodoDispatch {
        id: i64,
        session_id: String,
    },
```

- [ ] **Step 3: 新增 `Reply` 变体**

在 `Reply` 枚举末尾(`BackfillStatus { total: u32, completed: u32 }` 之后,闭合 `}` 之前)插入:

```rust
    Todos {
        todos: Vec<TodoInfo>,
    },
    Todo {
        todo: TodoInfo,
    },
```

- [ ] **Step 4: 写序列化往返测试**

在 `protocol.rs` 底部 `#[cfg(test)] mod tests` 里追加(仿现有 `conversation_protocol_types_roundtrip`):

```rust
    #[test]
    fn todo_protocol_types_roundtrip() {
        let add_req = Request::AddTodo {
            project_id: 1,
            text: "写完 spec".into(),
        };
        let line = encode_line(&add_req);
        let back: Request = decode_line(&line).unwrap();
        assert_eq!(add_req, back);

        let reorder_req = Request::ReorderTodo {
            id: 5,
            after_id: Some(3),
        };
        let line = encode_line(&reorder_req);
        let back: Request = decode_line(&line).unwrap();
        assert_eq!(reorder_req, back);

        let todo = TodoInfo {
            id: 1,
            project_id: 1,
            text: "写完 spec".into(),
            done: false,
            rank: 0,
            created_ms: 1_700_000_000_000,
            completed_at_ms: None,
            plan_date: Some("08-10".into()),
            dispatch_session_id: None,
            dispatch_at_ms: None,
        };
        let reply = Reply::Todo { todo: todo.clone() };
        let line = encode_line(&reply);
        let back: Reply = decode_line(&line).unwrap();
        assert_eq!(reply, back);

        let list_reply = Reply::Todos { todos: vec![todo] };
        let line = encode_line(&list_reply);
        let back: Reply = decode_line(&line).unwrap();
        assert_eq!(list_reply, back);
    }
```

- [ ] **Step 5: 运行测试**

Run: `cargo test -p dozer-core todo_protocol_types_roundtrip`
Expected: PASS

- [ ] **Step 6: 编译检查 + 提交**

```bash
cargo build -p dozer-core
git add crates/dozer-core/src/protocol.rs
git commit -m "feat(dozer-core): 新增 TodoInfo 与 Todo* 协议消息"
```

---

## Task 2: dozerd TodoStore——CRUD + 排序

**Files:**
- Create: `crates/dozerd/src/todo.rs`
- Modify: `crates/dozerd/src/lib.rs`(注册模块)

**Interfaces:**
- Consumes: `dozer_core::protocol::TodoInfo`(Task 1)。
- Produces: `pub struct TodoStore { .. }`,方法 `new(path: &Path) -> Result<Self>`、`list(&self, project_id: i64) -> Result<Vec<TodoInfo>>`、`add(&self, project_id: i64, text: &str) -> Result<TodoInfo>`、`toggle(&self, id: i64, done: bool) -> Result<TodoInfo>`、`edit_text(&self, id: i64, text: &str) -> Result<TodoInfo>`、`reorder(&self, id: i64, after_id: Option<i64>) -> Result<TodoInfo>`、`set_plan_date(&self, id: i64, plan_date: Option<&str>) -> Result<TodoInfo>`、`record_dispatch(&self, id: i64, session_id: &str) -> Result<TodoInfo>`。这些签名是 Task 4(server.rs 接线)、Task 6/7(GUI 调用)直接依赖的契约。

- [ ] **Step 1: 在 `crates/dozerd/src/lib.rs` 注册新模块**

按字母序在 `pub mod session_summary_backfill;` 和 `pub mod shell_integration;` 之间插入:

```rust
pub mod todo;
```

（确认插入后模块声明仍按字母序排列,和文件里其它 `pub mod` 一致。）

- [ ] **Step 2: 创建 `crates/dozerd/src/todo.rs`,写建表+基础 CRUD**

```rust
//! Todo 存储:rusqlite,与 `ProjectStore`/`AcceptanceStore`/`BookmarkStore`
//! 共享同一个 `dozer.db`(见 `main.rs` 里 `new` 时传入同一路径)。任务
//! 正文+全部元数据(派发记录/计划日期/完成时间)合并成一张表、一行一条
//! 任务,`id` 是稳定身份,取代 v1(`.dozer/todo.md` + `todo_meta.json`)
//! 靠"文本哈希"关联元数据的做法(2026-09-01 SQLite 迁移设计
//! `docs/superpowers/specs/2026-09-01-dozer-todo-sqlite-migration-design.md`)。

use anyhow::{Context, Result};
use dozer_core::protocol::TodoInfo;
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;
use std::sync::Mutex;

pub struct TodoStore {
    conn: Mutex<Connection>,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn id_not_found(id: i64) -> anyhow::Error {
    anyhow::anyhow!("任务不存在: id={id}")
}

/// `RETURNING`/`SELECT` 都用这份列顺序,`query_row`/`query_map` 的行映射
/// 闭包与之一一对应。
const TODO_COLUMNS: &str = "id, project_id, text, done, rank, created_ms, \
    completed_at_ms, plan_date, dispatch_session_id, dispatch_at_ms";

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
    })
}

impl TodoStore {
    pub fn new(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("建库目录")?;
        }
        let conn = Connection::open(path).context("打开 SQLite")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS todos (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                project_id INTEGER NOT NULL,
                text TEXT NOT NULL,
                done INTEGER NOT NULL DEFAULT 0,
                rank INTEGER NOT NULL,
                created_ms INTEGER NOT NULL,
                completed_at_ms INTEGER,
                plan_date TEXT,
                dispatch_session_id TEXT,
                dispatch_at_ms INTEGER
             );
             CREATE INDEX IF NOT EXISTS idx_todos_project_order
                ON todos(project_id, done, rank);
             CREATE TABLE IF NOT EXISTS todo_legacy_imported (
                project_id INTEGER PRIMARY KEY,
                imported_ms INTEGER NOT NULL
             );",
        )
        .context("建表")?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// 某项目全部任务,待办按 rank 升序在前、已完成按 rank 升序沉底。
    pub fn list(&self, project_id: i64) -> Result<Vec<TodoInfo>> {
        let conn = self.conn.lock().expect("db lock");
        let sql = format!(
            "SELECT {TODO_COLUMNS} FROM todos WHERE project_id = ?1 ORDER BY done ASC, rank ASC"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([project_id], row_to_todo)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 新增即置顶:新 rank = 当前待办最小 rank - 1(表内该项目还没有待办
    /// 时从 0 开始)。
    pub fn add(&self, project_id: i64, text: &str) -> Result<TodoInfo> {
        let conn = self.conn.lock().expect("db lock");
        let min_rank: Option<i64> = conn.query_row(
            "SELECT MIN(rank) FROM todos WHERE project_id = ?1 AND done = 0",
            [project_id],
            |r| r.get(0),
        )?;
        let rank = min_rank.map(|r| r - 1).unwrap_or(0);
        let now = now_ms() as i64;
        let sql = format!(
            "INSERT INTO todos (project_id, text, done, rank, created_ms)
             VALUES (?1, ?2, 0, ?3, ?4) RETURNING {TODO_COLUMNS}"
        );
        conn.query_row(&sql, params![project_id, text, rank, now], row_to_todo)
            .map_err(Into::into)
    }

    /// `done` 从假变真时写 `completed_at_ms = now`,真变假时清空——与现状
    /// `completed_at_for_toggle` 纯函数行为等价,合并进这一次 UPDATE。
    pub fn toggle(&self, id: i64, done: bool) -> Result<TodoInfo> {
        let conn = self.conn.lock().expect("db lock");
        let completed_at_ms: Option<i64> = done.then(|| now_ms() as i64);
        let sql =
            format!("UPDATE todos SET done = ?1, completed_at_ms = ?2 WHERE id = ?3 RETURNING {TODO_COLUMNS}");
        conn.query_row(&sql, params![done, completed_at_ms, id], row_to_todo)
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => id_not_found(id),
                e => e.into(),
            })
    }

    pub fn edit_text(&self, id: i64, text: &str) -> Result<TodoInfo> {
        let conn = self.conn.lock().expect("db lock");
        let sql = format!("UPDATE todos SET text = ?1 WHERE id = ?2 RETURNING {TODO_COLUMNS}");
        conn.query_row(&sql, params![text, id], row_to_todo)
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => id_not_found(id),
                e => e.into(),
            })
    }

    pub fn set_plan_date(&self, id: i64, plan_date: Option<&str>) -> Result<TodoInfo> {
        let conn = self.conn.lock().expect("db lock");
        let sql =
            format!("UPDATE todos SET plan_date = ?1 WHERE id = ?2 RETURNING {TODO_COLUMNS}");
        conn.query_row(&sql, params![plan_date, id], row_to_todo)
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => id_not_found(id),
                e => e.into(),
            })
    }

    pub fn record_dispatch(&self, id: i64, session_id: &str) -> Result<TodoInfo> {
        let conn = self.conn.lock().expect("db lock");
        let now = now_ms() as i64;
        let sql = format!(
            "UPDATE todos SET dispatch_session_id = ?1, dispatch_at_ms = ?2
             WHERE id = ?3 RETURNING {TODO_COLUMNS}"
        );
        conn.query_row(&sql, params![session_id, now, id], row_to_todo)
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => id_not_found(id),
                e => e.into(),
            })
    }

    /// 把 `id` 挪到 `after_id` 之后(`None` = 待办块最前);只在待办子集内
    /// 生效。做法:取该 project 全部待办 id(按当前 rank 升序),摘出 `id`
    /// 插到目标位置,对整个待办子集重新编号(0..N-1)——待办条数通常几十
    /// 条以内,全量重编号比"两个相邻 rank 间插值"更简单且不存在整数间隙
    /// 耗尽的问题,一次事务内完成,防止中途崩溃留下部分重排的脏状态。
    /// `after_id` 不在当前待办里(比如已经是已完成,或调用方传了脏 id)
    /// 时退化为"挪到待办块末尾",不报错。
    pub fn reorder(&self, id: i64, after_id: Option<i64>) -> Result<TodoInfo> {
        let mut conn = self.conn.lock().expect("db lock");
        let project_id: i64 = conn
            .query_row(
                "SELECT project_id FROM todos WHERE id = ?1 AND done = 0",
                [id],
                |r| r.get(0),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => id_not_found(id),
                e => e.into(),
            })?;
        let mut pending_ids: Vec<i64> = {
            let mut stmt = conn.prepare(
                "SELECT id FROM todos WHERE project_id = ?1 AND done = 0 ORDER BY rank ASC",
            )?;
            stmt.query_map([project_id], |r| r.get(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        let Some(pos) = pending_ids.iter().position(|&x| x == id) else {
            return Err(id_not_found(id));
        };
        pending_ids.remove(pos);
        let insert_at = match after_id {
            None => 0,
            Some(aid) => pending_ids
                .iter()
                .position(|&x| x == aid)
                .map(|p| p + 1)
                .unwrap_or(pending_ids.len()),
        };
        pending_ids.insert(insert_at, id);
        let tx = conn.transaction()?;
        for (rank, task_id) in pending_ids.iter().enumerate() {
            tx.execute(
                "UPDATE todos SET rank = ?1 WHERE id = ?2",
                params![rank as i64, task_id],
            )?;
        }
        tx.commit()?;
        let sql = format!("SELECT {TODO_COLUMNS} FROM todos WHERE id = ?1");
        conn.query_row(&sql, [id], row_to_todo).map_err(Into::into)
    }
}
```

- [ ] **Step 3: 写基础 CRUD 单元测试**

在 `todo.rs` 底部追加:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, TodoStore) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.db");
        let store = TodoStore::new(&db).unwrap();
        (dir, store)
    }

    #[test]
    fn add_list_and_toggle() {
        let (_dir, store) = store();
        let t = store.add(1, "写 spec").unwrap();
        assert_eq!(t.text, "写 spec");
        assert!(!t.done);
        assert_eq!(t.completed_at_ms, None);

        let listed = store.list(1).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, t.id);

        let toggled = store.toggle(t.id, true).unwrap();
        assert!(toggled.done);
        assert!(toggled.completed_at_ms.is_some());

        let untoggled = store.toggle(t.id, false).unwrap();
        assert!(!untoggled.done);
        assert_eq!(untoggled.completed_at_ms, None);
    }

    #[test]
    fn toggle_unknown_id_errors() {
        let (_dir, store) = store();
        assert!(store.toggle(999, true).is_err());
    }

    #[test]
    fn add_prepends_new_task_before_existing_pending() {
        let (_dir, store) = store();
        let first = store.add(1, "第一条").unwrap();
        let second = store.add(1, "第二条").unwrap();
        let listed = store.list(1).unwrap();
        assert_eq!(listed[0].id, second.id, "后加的排最前");
        assert_eq!(listed[1].id, first.id);
    }

    #[test]
    fn done_tasks_sink_below_pending_regardless_of_rank() {
        let (_dir, store) = store();
        let a = store.add(1, "A").unwrap();
        let b = store.add(1, "B").unwrap();
        store.toggle(a.id, true).unwrap();
        let listed = store.list(1).unwrap();
        assert_eq!(listed[0].id, b.id, "待办在前");
        assert_eq!(listed[1].id, a.id, "已完成沉底");
    }

    #[test]
    fn edit_text_updates_and_unknown_id_errors() {
        let (_dir, store) = store();
        let t = store.add(1, "旧文字").unwrap();
        let edited = store.edit_text(t.id, "新文字").unwrap();
        assert_eq!(edited.text, "新文字");
        assert!(store.edit_text(999, "x").is_err());
    }

    #[test]
    fn set_plan_date_sets_and_clears() {
        let (_dir, store) = store();
        let t = store.add(1, "任务").unwrap();
        let with_date = store.set_plan_date(t.id, Some("08-10")).unwrap();
        assert_eq!(with_date.plan_date, Some("08-10".to_string()));
        let cleared = store.set_plan_date(t.id, None).unwrap();
        assert_eq!(cleared.plan_date, None);
    }

    #[test]
    fn record_dispatch_sets_session_and_timestamp() {
        let (_dir, store) = store();
        let t = store.add(1, "任务").unwrap();
        let dispatched = store.record_dispatch(t.id, "sess-1").unwrap();
        assert_eq!(dispatched.dispatch_session_id, Some("sess-1".to_string()));
        assert!(dispatched.dispatch_at_ms.is_some());
    }

    #[test]
    fn reorder_moves_task_after_target() {
        let (_dir, store) = store();
        let a = store.add(1, "A").unwrap();
        let b = store.add(1, "B").unwrap();
        let c = store.add(1, "C").unwrap();
        // 当前顺序(新加置顶): C, B, A。把 A 挪到 C 之后 → C, A, B。
        let moved = store.reorder(a.id, Some(c.id)).unwrap();
        assert_eq!(moved.id, a.id);
        let listed = store.list(1).unwrap();
        assert_eq!(
            listed.iter().map(|t| t.id).collect::<Vec<_>>(),
            vec![c.id, a.id, b.id]
        );
    }

    #[test]
    fn reorder_to_front_with_none() {
        let (_dir, store) = store();
        let a = store.add(1, "A").unwrap();
        let b = store.add(1, "B").unwrap();
        let moved = store.reorder(a.id, None).unwrap();
        assert_eq!(moved.rank, 0);
        let listed = store.list(1).unwrap();
        assert_eq!(listed[0].id, a.id);
        assert_eq!(listed[1].id, b.id);
    }

    #[test]
    fn reorder_unknown_id_errors() {
        let (_dir, store) = store();
        assert!(store.reorder(999, None).is_err());
    }

    #[test]
    fn tasks_are_isolated_per_project() {
        let (_dir, store) = store();
        store.add(1, "项目1的任务").unwrap();
        store.add(2, "项目2的任务").unwrap();
        assert_eq!(store.list(1).unwrap().len(), 1);
        assert_eq!(store.list(2).unwrap().len(), 1);
    }
}
```

- [ ] **Step 4: 确认 `tempfile` 是 `dozerd` 的 dev-dependency**

Run: `grep -n "tempfile" crates/dozerd/Cargo.toml`
Expected: 已存在(`bookmarks.rs`/`acceptance.rs` 现有测试已经在用);若没有,在 `[dev-dependencies]` 下加 `tempfile = "3"`(参考 workspace 其它 crate 里 `tempfile` 的版本号,保持一致)。

- [ ] **Step 5: 运行测试**

Run: `cargo test -p dozerd todo::`
Expected: 全部 PASS(11 个测试)

- [ ] **Step 6: 编译 + 提交**

```bash
cargo build -p dozerd
git add crates/dozerd/src/todo.rs crates/dozerd/src/lib.rs
git commit -m "feat(dozerd): 新增 TodoStore(CRUD + 排序)"
```

---

## Task 3: TodoStore 历史数据导入

**Files:**
- Modify: `crates/dozerd/src/todo.rs`

**Interfaces:**
- Consumes: Task 2 的 `TodoStore`(同文件,直接扩展 `impl TodoStore`)。
- Produces: `pub fn import_legacy_if_needed(&self, project_id: i64, cwd: &Path) -> Result<()>`——Task 4(`server.rs` 的 `OpenProject` 处理)直接调用。

- [ ] **Step 1: 新增可测试的内部实现 + 对外包装**

在 `todo.rs` 的 `impl TodoStore` 块内(`reorder` 方法之后)追加:

```rust
    /// 对外入口:算出真实磁盘路径后调 `import_legacy_if_needed_at`。
    /// `cwd` 是项目根目录绝对路径(与 `Request::OpenProject.path` 同一种
    /// 形状)。
    pub fn import_legacy_if_needed(&self, project_id: i64, cwd: &Path) -> Result<()> {
        let todo_md_path = cwd.join(".dozer").join("todo.md");
        let meta_json_path = dozer_core::paths::config_dir().join("todo_meta.json");
        self.import_legacy_if_needed_at(project_id, &todo_md_path, &meta_json_path)
    }

    /// 参数化的实现,便于单测用临时目录路径调用,不依赖真实的
    /// `dozer_core::paths::config_dir()`(那是机器级全局路径,测试不该碰)。
    /// 一次性:`todo_legacy_imported` 里已有该 `project_id` 记录就直接
    /// 返回,不重复导入(即使 `todo_md_path` 之后又变了)。读取/解析失败
    /// 按"这个项目没有历史任务"处理,仍然写迁移标记,避免每次 `OpenProject`
    /// 都重新尝试同一个读不了的文件。
    fn import_legacy_if_needed_at(
        &self,
        project_id: i64,
        todo_md_path: &Path,
        meta_json_path: &Path,
    ) -> Result<()> {
        let mut conn = self.conn.lock().expect("db lock");
        let already = conn
            .query_row(
                "SELECT 1 FROM todo_legacy_imported WHERE project_id = ?1",
                [project_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if already {
            return Ok(());
        }
        let md = std::fs::read_to_string(todo_md_path).unwrap_or_default();
        let items = parse_legacy_markdown(&md);
        let meta = read_legacy_meta_json(meta_json_path, project_id);
        let now = now_ms() as i64;
        let tx = conn.transaction()?;
        for (rank, (text, done)) in items.iter().enumerate() {
            let key = legacy_todo_line_key(text);
            let m = meta.get(&key);
            tx.execute(
                "INSERT INTO todos (project_id, text, done, rank, created_ms,
                    completed_at_ms, plan_date, dispatch_session_id, dispatch_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    project_id,
                    text,
                    *done,
                    rank as i64,
                    now,
                    m.and_then(|m| m.completed_at_ms),
                    m.and_then(|m| m.plan_date.clone()),
                    m.and_then(|m| m.dispatch_session_id.clone()),
                    m.and_then(|m| m.dispatch_at_ms),
                ],
            )?;
        }
        tx.execute(
            "INSERT INTO todo_legacy_imported (project_id, imported_ms) VALUES (?1, ?2)",
            params![project_id, now],
        )?;
        tx.commit()?;
        Ok(())
    }
```

- [ ] **Step 2: 新增旧格式解析 + 哈希函数(镜像 `dozer-app` 现有逻辑)**

在 `impl TodoStore` 块之外(文件顶层,靠近其它自由函数)追加。**这两个函数的逻辑必须与 `crates/dozer-app/src/extensions/todo.rs` 里的 `parse_todo`/`todo_line_key` 保持字节级一致**——`dozerd` 不能反向依赖 `dozer-app`(依赖方向错误),所以在这里维护一份独立副本;哈希算法或解析规则任何一边改了,历史数据关联就会失效,改动时两边要同步核对:

```rust
/// 镜像 `dozer-app::extensions::todo::parse_todo`:只认顶格(列 0)的
/// `- [ ]`/`- [x]` 一级列表项,按文件出现顺序返回 `(text, done)`。
fn parse_legacy_markdown(md: &str) -> Vec<(String, bool)> {
    let mut items = Vec::new();
    for line in md.lines() {
        if let Some(rest) = line.strip_prefix("- [ ]") {
            items.push((rest.trim().to_string(), false));
        } else if let Some(rest) = line.strip_prefix("- [x]") {
            items.push((rest.trim().to_string(), true));
        }
    }
    items
}

/// 镜像 `dozer-app::extensions::todo::todo_line_key`:trim 后文本的
/// `DefaultHasher` 哈希,用来跟旧 `todo_meta.json` 里的记录关联。
fn legacy_todo_line_key(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.trim().hash(&mut hasher);
    hasher.finish()
}

struct LegacyMeta {
    dispatch_session_id: Option<String>,
    dispatch_at_ms: Option<i64>,
    plan_date: Option<String>,
    completed_at_ms: Option<i64>,
}

/// `SystemTime` 经 serde 序列化成 `{"secs_since_epoch": u64,
/// "nanos_since_epoch": u32}`(serde 标准库支持,`dozer-app` 那边
/// `TodoTaskMeta`/`DispatchRecord` 的 `SystemTime` 字段就是这么落盘的),
/// 这里手动转换成毫秒,不需要在 `dozerd` 里重新定义匹配的 Rust 结构体。
fn system_time_json_to_ms(v: &serde_json::Value) -> Option<i64> {
    let secs = v.get("secs_since_epoch")?.as_i64()?;
    let nanos = v
        .get("nanos_since_epoch")
        .and_then(|n| n.as_i64())
        .unwrap_or(0);
    Some(secs * 1000 + nanos / 1_000_000)
}

/// 读旧 `todo_meta.json`(`TodoMetaState = HashMap<i64 project_id,
/// HashMap<u64 hash, TodoTaskMeta>>`,整数 key 经 serde_json 序列化成字符串
/// key),按 `project_id` 取子表。文件不存在/JSON 损坏/这个 project_id
/// 没有记录,一律回落空表,不报错(与 `dozer-app` 侧 `meta_load` 同样的
/// 宽松读取哲学)。
fn read_legacy_meta_json(
    path: &Path,
    project_id: i64,
) -> std::collections::HashMap<u64, LegacyMeta> {
    let mut out = std::collections::HashMap::new();
    let Ok(raw) = std::fs::read_to_string(path) else {
        return out;
    };
    let Ok(root) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return out;
    };
    let Some(project_map) = root
        .get(project_id.to_string().as_str())
        .and_then(|v| v.as_object())
    else {
        return out;
    };
    for (key, entry) in project_map {
        let Ok(hash_key) = key.parse::<u64>() else {
            continue;
        };
        let dispatch = entry.get("dispatch").filter(|d| !d.is_null());
        out.insert(
            hash_key,
            LegacyMeta {
                dispatch_session_id: dispatch
                    .and_then(|d| d.get("session_id"))
                    .and_then(|s| s.as_str())
                    .map(String::from),
                dispatch_at_ms: dispatch
                    .and_then(|d| d.get("dispatched_at"))
                    .and_then(system_time_json_to_ms),
                plan_date: entry
                    .get("plan_date")
                    .filter(|v| !v.is_null())
                    .and_then(|v| v.as_str())
                    .map(String::from),
                completed_at_ms: entry
                    .get("completed_at")
                    .filter(|v| !v.is_null())
                    .and_then(system_time_json_to_ms),
            },
        );
    }
    out
}
```

- [ ] **Step 3: 加 `serde_json` 依赖(若 `dozerd` 尚未直接依赖)**

Run: `grep -n "^serde_json" crates/dozerd/Cargo.toml`
Expected: 已存在(`session_summary.rs`/`transcripts` 大概率已经在用 JSON);若没有,在 `[dependencies]` 加 `serde_json = "1"`。

- [ ] **Step 4: 写导入测试(覆盖 spec 要求的四种场景)**

在 `todo.rs` 的 `mod tests` 里追加:

```rust
    #[test]
    fn import_skips_when_no_md_file() {
        let (dir, store) = store();
        let md_path = dir.path().join("nope").join("todo.md");
        let meta_path = dir.path().join("nope").join("todo_meta.json");
        store
            .import_legacy_if_needed_at(1, &md_path, &meta_path)
            .unwrap();
        assert!(store.list(1).unwrap().is_empty());
    }

    #[test]
    fn import_parses_markdown_without_meta_file() {
        let (dir, store) = store();
        let md_path = dir.path().join("todo.md");
        std::fs::write(&md_path, "- [ ] 任务一\n- [x] 任务二\n").unwrap();
        let meta_path = dir.path().join("todo_meta.json");
        store
            .import_legacy_if_needed_at(1, &md_path, &meta_path)
            .unwrap();
        let listed = store.list(1).unwrap();
        assert_eq!(listed.len(), 2);
        assert!(listed.iter().any(|t| t.text == "任务一" && !t.done));
        assert!(listed.iter().any(|t| t.text == "任务二" && t.done));
    }

    #[test]
    fn import_correlates_meta_by_legacy_hash() {
        let (dir, store) = store();
        let md_path = dir.path().join("todo.md");
        std::fs::write(&md_path, "- [ ] 派发过的任务\n").unwrap();
        let key = legacy_todo_line_key("派发过的任务");
        let meta_json = serde_json::json!({
            "1": {
                key.to_string(): {
                    "dispatch": {
                        "session_id": "sess-abc",
                        "dispatched_at": {"secs_since_epoch": 1_700_000_000u64, "nanos_since_epoch": 0},
                    },
                    "plan_date": "08-10",
                    "completed_at": null,
                }
            }
        });
        let meta_path = dir.path().join("todo_meta.json");
        std::fs::write(&meta_path, meta_json.to_string()).unwrap();
        store
            .import_legacy_if_needed_at(1, &md_path, &meta_path)
            .unwrap();
        let listed = store.list(1).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].dispatch_session_id, Some("sess-abc".to_string()));
        assert_eq!(listed[0].dispatch_at_ms, Some(1_700_000_000_000));
        assert_eq!(listed[0].plan_date, Some("08-10".to_string()));
    }

    #[test]
    fn import_is_not_repeated_on_second_call() {
        let (dir, store) = store();
        let md_path = dir.path().join("todo.md");
        std::fs::write(&md_path, "- [ ] 任务\n").unwrap();
        let meta_path = dir.path().join("todo_meta.json");
        store
            .import_legacy_if_needed_at(1, &md_path, &meta_path)
            .unwrap();
        // 追加内容后再导入一次,不应该产生新行(已标记导入过)。
        std::fs::write(&md_path, "- [ ] 任务\n- [ ] 后来加的\n").unwrap();
        store
            .import_legacy_if_needed_at(1, &md_path, &meta_path)
            .unwrap();
        assert_eq!(store.list(1).unwrap().len(), 1);
    }

    #[test]
    fn import_is_isolated_per_project() {
        let (dir, store) = store();
        let md_path = dir.path().join("todo.md");
        std::fs::write(&md_path, "- [ ] 任务\n").unwrap();
        let meta_path = dir.path().join("todo_meta.json");
        store
            .import_legacy_if_needed_at(1, &md_path, &meta_path)
            .unwrap();
        // project 2 没导入过,即使复用同一份 md 路径也应该正常导入。
        store
            .import_legacy_if_needed_at(2, &md_path, &meta_path)
            .unwrap();
        assert_eq!(store.list(1).unwrap().len(), 1);
        assert_eq!(store.list(2).unwrap().len(), 1);
    }
```

- [ ] **Step 5: 运行测试**

Run: `cargo test -p dozerd todo::`
Expected: 全部 PASS(新增 5 个导入测试 + 之前 11 个,共 16 个)

- [ ] **Step 6: 编译 + 提交**

```bash
cargo build -p dozerd
git add crates/dozerd/src/todo.rs crates/dozerd/Cargo.toml
git commit -m "feat(dozerd): TodoStore 支持一次性历史数据导入"
```

---

## Task 4: dozerd 接线(server.rs/main.rs)+ 修复全部 `serve()` 调用点

**Files:**
- Modify: `crates/dozerd/src/server.rs`
- Modify: `crates/dozerd/src/main.rs`
- Modify: `crates/dozerd/tests/hook_events.rs`
- Modify: `crates/dozerd/tests/session_survival.rs`
- Modify: `crates/dozer-client/tests/against_real_daemon.rs`
- Modify: `crates/dozer-mcp/tests/get_preview_context.rs`
- Modify: `crates/dozer-mcp/tests/submit_session_summary.rs`

**Interfaces:**
- Consumes: `dozerd::todo::TodoStore`(Task 2/3)、`Request`/`Reply` 的 `Todo*` 变体(Task 1)。
- Produces: `dozerd::server::serve(..., todos: Arc<crate::todo::TodoStore>)`——新增的最后一个参数,本任务负责修复它引入的全部编译错误。

`serve()` 现有 8 个参数(`socket, registry, store, projects, bookmarks, transcripts, session_summaries, backfill_registry`),本任务在**末尾**追加第 9 个参数 `todos`——选在末尾是为了让下面逐个调用点的修复统一变成"在参数列表最后加一行",不用去数现有参数的相对位置,降低改错风险。

- [ ] **Step 1: 修改 `server.rs` 的 `serve()` 签名 + 请求处理**

在 `crates/dozerd/src/server.rs` 的 `pub async fn serve(...)` 参数列表末尾加:

```rust
    todos: Arc<crate::todo::TodoStore>,
```

在函数体内、`Request::OpenProject { path } => match projects.open(&path) {...}` 分支里,`projects.open(&path)` 成功后插入一次性历史导入(导入失败只记日志,不影响 `OpenProject` 本身的成功返回——`import_legacy_if_needed` 内部已经把"读不到文件"当空列表处理,这里 `Err` 只可能是数据库层面的问题):

```rust
                        Request::OpenProject { path } => match projects.open(&path) {
                            Ok(p) => {
                                if let Err(e) =
                                    todos.import_legacy_if_needed(p.id, Path::new(&path))
                                {
                                    tracing::warn!(project_id = p.id, error = %e, "Todo 历史导入失败");
                                }
                                Reply::Project { project: Some(p) }
                            }
                            Err(e) => Reply::Error { message: format!("打开项目失败: {e}") },
                        },
```

在 `Request::ListBookmarks { .. } => { ... }` 分支之后(任意合适位置,建议紧跟在 bookmark 三件套之后)新增 Todo 请求处理:

```rust
                        Request::ListTodos { project_id } => match todos.list(project_id) {
                            Ok(todos) => Reply::Todos { todos },
                            Err(e) => Reply::Error {
                                message: format!("列任务失败: {e}"),
                            },
                        },
                        Request::AddTodo { project_id, text } => {
                            match todos.add(project_id, &text) {
                                Ok(todo) => Reply::Todo { todo },
                                Err(e) => Reply::Error {
                                    message: format!("新增任务失败: {e}"),
                                },
                            }
                        }
                        Request::ToggleTodo { id, done } => match todos.toggle(id, done) {
                            Ok(todo) => Reply::Todo { todo },
                            Err(e) => Reply::Error {
                                message: format!("切换完成态失败: {e}"),
                            },
                        },
                        Request::EditTodoText { id, text } => {
                            match todos.edit_text(id, &text) {
                                Ok(todo) => Reply::Todo { todo },
                                Err(e) => Reply::Error {
                                    message: format!("改任务文字失败: {e}"),
                                },
                            }
                        }
                        Request::ReorderTodo { id, after_id } => {
                            match todos.reorder(id, after_id) {
                                Ok(todo) => Reply::Todo { todo },
                                Err(e) => Reply::Error {
                                    message: format!("排序失败: {e}"),
                                },
                            }
                        }
                        Request::SetTodoPlanDate { id, plan_date } => {
                            match todos.set_plan_date(id, plan_date.as_deref()) {
                                Ok(todo) => Reply::Todo { todo },
                                Err(e) => Reply::Error {
                                    message: format!("设置计划日期失败: {e}"),
                                },
                            }
                        }
                        Request::RecordTodoDispatch { id, session_id } => {
                            match todos.record_dispatch(id, &session_id) {
                                Ok(todo) => Reply::Todo { todo },
                                Err(e) => Reply::Error {
                                    message: format!("记录派发失败: {e}"),
                                },
                            }
                        }
```

- [ ] **Step 2: 修复 `server.rs` 内部的签名检查测试函数**

`server.rs` 底部 `mod tests` 里的 `transcript_store_field_compiles_into_serve_signature`(现约 645-670 行)是个纯编译期签名检查,给它的内层 `_assert_signature` 函数追加参数并在调用处追加实参:

```rust
        fn _assert_signature(
            socket: &std::path::Path,
            registry: std::sync::Arc<crate::registry::SessionRegistry>,
            store: std::sync::Arc<crate::acceptance::AcceptanceStore>,
            projects: std::sync::Arc<crate::projects::ProjectStore>,
            bookmarks: std::sync::Arc<crate::bookmarks::BookmarkStore>,
            transcripts: std::sync::Arc<crate::transcripts::TranscriptStore>,
            session_summaries: std::sync::Arc<crate::session_summary::SessionSummaryStore>,
            todos: std::sync::Arc<crate::todo::TodoStore>,
        ) {
            let fut = crate::server::serve(
                socket,
                registry,
                store,
                projects,
                bookmarks,
                transcripts,
                session_summaries,
                std::sync::Arc::new(crate::session_summary_backfill::BackfillRegistry::new()),
                todos,
            );
            let _ = fut;
        }
```

（保持原来这个函数体最后一行 `let _ = fut;` 或等价写法不变——只加 `todos` 参数声明 + 调用处追加 `todos` 实参,其余不动。先用 `grep -n "fn _assert_signature" -A 25 crates/dozerd/src/server.rs` 确认原函数结尾的确切写法再改。）

- [ ] **Step 3: 修改 `main.rs`——构造 `TodoStore` + 传给 `serve`**

在 `crates/dozerd/src/main.rs` 里 `let session_summaries = ...` 之后插入:

```rust
    let todos = Arc::new(dozerd::todo::TodoStore::new(
        &dozer_core::paths::state_dir().join("dozer.db"),
    )?);
```

在 `let serve = dozerd::server::serve(...)` 调用的参数列表末尾追加:

```rust
        todos,
```

- [ ] **Step 4: 修复 `crates/dozerd/tests/hook_events.rs`**

在文件里已有的 `test_backfill_registry()` 之后新增同款 helper:

```rust
/// 每次调用建独立临时库的 Todo 存储(测试用；serve 需要)。
fn test_todos() -> std::sync::Arc<dozerd::todo::TodoStore> {
    let db = std::env::temp_dir().join(format!("dozerd-test-{}.db", uuid::Uuid::new_v4()));
    std::sync::Arc::new(dozerd::todo::TodoStore::new(&db).unwrap())
}
```

然后给文件里全部 6 处 `dozerd::server::serve(` 调用,在各自参数列表最后一个实参(`test_backfill_registry(),`)之后追加一行 `test_todos(),`。

Run: `grep -n "test_backfill_registry(),$" crates/dozerd/tests/hook_events.rs` 定位全部 6 处插入点,逐一确认改完。

- [ ] **Step 5: 修复 `crates/dozerd/tests/session_survival.rs`**

同 Step 4 手法:在已有 `test_backfill_registry()` helper 之后新增:

```rust
fn test_todos() -> std::sync::Arc<dozerd::todo::TodoStore> {
    let db = std::env::temp_dir().join(format!("dozerd-test-{}.db", uuid::Uuid::new_v4()));
    std::sync::Arc::new(dozerd::todo::TodoStore::new(&db).unwrap())
}
```

给全部 6 处 `dozerd::server::serve(` 调用追加 `test_todos(),`。

Run: `grep -n "test_backfill_registry(),$" crates/dozerd/tests/session_survival.rs` 定位。

- [ ] **Step 6: 修复 `crates/dozer-client/tests/against_real_daemon.rs`**

在 `start_daemon()` 里,`let backfill_registry = ...` 之后插入:

```rust
    let todos = Arc::new(dozerd::todo::TodoStore::new(&db).unwrap());
```

在 `dozerd::server::serve(...)` 调用的参数列表末尾(`backfill_registry,` 之后)追加 `todos,`。

- [ ] **Step 7: 修复 `crates/dozer-mcp/tests/get_preview_context.rs` 与 `crates/dozer-mcp/tests/submit_session_summary.rs`**

两个文件的 `start_daemon()` helper 结构相同,各自做 Step 6 同样的两处修改(`let todos = Arc::new(dozerd::todo::TodoStore::new(&db).unwrap());` + `serve(...)` 末尾追加 `todos,`)。

- [ ] **Step 8: 全量编译确认没有遗漏的调用点**

Run: `cargo build --workspace 2>&1 | grep -A3 "error\[" | head -100`
Expected: 无输出(全绿)。如果还有报错,说明有遗漏的 `server::serve(` 调用点未修复,按报错的文件路径回去补。

- [ ] **Step 9: 运行相关测试**

Run: `cargo test -p dozerd -p dozer-client -p dozer-mcp`
Expected: 全部 PASS(不应该因为签名改动导致既有测试行为变化,只是新增了参数)

- [ ] **Step 10: 提交**

```bash
git add crates/dozerd/src/server.rs crates/dozerd/src/main.rs \
  crates/dozerd/tests/hook_events.rs crates/dozerd/tests/session_survival.rs \
  crates/dozer-client/tests/against_real_daemon.rs \
  crates/dozer-mcp/tests/get_preview_context.rs \
  crates/dozer-mcp/tests/submit_session_summary.rs
git commit -m "feat(dozerd): 接入 TodoStore,OpenProject 触发一次性历史导入"
```

---

## Task 5: dozer-client Client 方法 + 往返集成测试

**Files:**
- Modify: `crates/dozer-client/src/lib.rs`
- Modify: `crates/dozer-client/tests/against_real_daemon.rs`

**Interfaces:**
- Consumes: `Request`/`Reply` 的 `Todo*` 变体(Task 1)、`dozerd::todo::TodoStore` 已接入 `serve()`(Task 4)。
- Produces: `Client::{list_todos, add_todo, toggle_todo, edit_todo_text, reorder_todo, set_todo_plan_date, record_todo_dispatch}`——Task 6/7(GUI)、Task 8(MCP)直接调用的方法。

- [ ] **Step 1: 在 `crates/dozer-client/src/lib.rs` 新增 7 个方法**

找到 `list_bookmarks` 方法(仿照它的写法),在其后插入:

```rust
    pub async fn list_todos(&self, project_id: i64) -> Result<Vec<TodoInfo>> {
        match self.roundtrip(&Request::ListTodos { project_id }).await? {
            Reply::Todos { todos } => Ok(todos),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn add_todo(&self, project_id: i64, text: &str) -> Result<TodoInfo> {
        match self
            .roundtrip(&Request::AddTodo {
                project_id,
                text: text.into(),
            })
            .await?
        {
            Reply::Todo { todo } => Ok(todo),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn toggle_todo(&self, id: i64, done: bool) -> Result<TodoInfo> {
        match self.roundtrip(&Request::ToggleTodo { id, done }).await? {
            Reply::Todo { todo } => Ok(todo),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn edit_todo_text(&self, id: i64, text: &str) -> Result<TodoInfo> {
        match self
            .roundtrip(&Request::EditTodoText {
                id,
                text: text.into(),
            })
            .await?
        {
            Reply::Todo { todo } => Ok(todo),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn reorder_todo(&self, id: i64, after_id: Option<i64>) -> Result<TodoInfo> {
        match self
            .roundtrip(&Request::ReorderTodo { id, after_id })
            .await?
        {
            Reply::Todo { todo } => Ok(todo),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn set_todo_plan_date(
        &self,
        id: i64,
        plan_date: Option<&str>,
    ) -> Result<TodoInfo> {
        match self
            .roundtrip(&Request::SetTodoPlanDate {
                id,
                plan_date: plan_date.map(String::from),
            })
            .await?
        {
            Reply::Todo { todo } => Ok(todo),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn record_todo_dispatch(&self, id: i64, session_id: &str) -> Result<TodoInfo> {
        match self
            .roundtrip(&Request::RecordTodoDispatch {
                id,
                session_id: session_id.into(),
            })
            .await?
        {
            Reply::Todo { todo } => Ok(todo),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }
```

确认文件顶部 `use dozer_core::protocol::{..., TodoInfo, ...};` 已经把 `TodoInfo` 引入(参照 `BookmarkInfo`/`BookmarkScope` 现有的 import 写法补上)。

- [ ] **Step 2: 编译检查**

Run: `cargo build -p dozer-client`
Expected: 编译通过

- [ ] **Step 3: 在 `against_real_daemon.rs` 写往返测试**

在文件末尾追加(复用已有的 `start_daemon()` helper,Task 4 已经给它接上了 `todos`):

```rust
#[tokio::test]
async fn add_toggle_and_list_todos_roundtrip() {
    let (sock, _registry, _guard) = start_daemon().await;
    let client = Client::new(sock);

    let t1 = client.add_todo(1, "第一条").await.unwrap();
    let t2 = client.add_todo(1, "第二条").await.unwrap();

    let listed = client.list_todos(1).await.unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].id, t2.id, "后加的排最前");
    assert_eq!(listed[1].id, t1.id);

    let toggled = client.toggle_todo(t1.id, true).await.unwrap();
    assert!(toggled.done);
    assert!(toggled.completed_at_ms.is_some());

    let listed = client.list_todos(1).await.unwrap();
    assert_eq!(listed[0].id, t2.id, "唯一待办排最前");
    assert_eq!(listed[1].id, t1.id, "已完成沉底");
}

#[tokio::test]
async fn edit_reorder_plan_date_and_dispatch_roundtrip() {
    let (sock, _registry, _guard) = start_daemon().await;
    let client = Client::new(sock);

    let t1 = client.add_todo(1, "旧文字").await.unwrap();
    let edited = client.edit_todo_text(t1.id, "新文字").await.unwrap();
    assert_eq!(edited.text, "新文字");

    let t2 = client.add_todo(1, "另一条").await.unwrap();
    let reordered = client.reorder_todo(t1.id, Some(t2.id)).await.unwrap();
    assert_eq!(reordered.id, t1.id);
    let listed = client.list_todos(1).await.unwrap();
    assert_eq!(listed[0].id, t2.id);
    assert_eq!(listed[1].id, t1.id);

    let with_date = client
        .set_todo_plan_date(t1.id, Some("08-10"))
        .await
        .unwrap();
    assert_eq!(with_date.plan_date, Some("08-10".to_string()));
    let cleared = client.set_todo_plan_date(t1.id, None).await.unwrap();
    assert_eq!(cleared.plan_date, None);

    let dispatched = client
        .record_todo_dispatch(t1.id, "sess-xyz")
        .await
        .unwrap();
    assert_eq!(dispatched.dispatch_session_id, Some("sess-xyz".to_string()));
}

#[tokio::test]
async fn toggle_unknown_todo_id_errors() {
    let (sock, _registry, _guard) = start_daemon().await;
    let client = Client::new(sock);
    let err = client.toggle_todo(999, true).await.unwrap_err();
    assert!(err.to_string().contains("任务不存在"));
}
```

- [ ] **Step 4: 运行测试**

Run: `cargo test -p dozer-client`
Expected: 全部 PASS

- [ ] **Step 5: 提交**

```bash
git add crates/dozer-client/src/lib.rs crates/dozer-client/tests/against_real_daemon.rs
git commit -m "feat(dozer-client): 新增 Todo CRUD 客户端方法"
```

---

## Task 6: dozer-app 数据层替换(核心)——TodoInfo 化 + 移除文件 I/O + 移除 MARKDOWN tab

这是整个迁移里改动面最大的任务。目标:`extensions/todo.rs` 从"直接读写 `.dozer/todo.md`/`todo_meta.json`"改造成"通过 `Client` 走 UDS",`WorkspaceState::items` 类型从 `Vec<TodoItem>` 换成 `Vec<TodoInfo>`,`todo::AppState`(GUI 本地元数据 sidecar)整体删除,MARKDOWN 视图 tab 连同它专属的键盘路由代码一并删除。List/Add/Toggle 三个操作在本任务里做完并可用;Edit/Reorder/PlanDate/Dispatch 留给 Task 7(依赖本任务打好的地基)。

**Files:**
- Modify: `crates/dozer-app/src/extensions/todo.rs`(大改)
- Modify: `crates/dozer-app/src/app.rs`
- Modify: `crates/dozer-app/src/main.rs`

**Interfaces:**
- Consumes: `Client::{list_todos, add_todo, toggle_todo}`(Task 5)、`dozer_core::protocol::TodoInfo`(Task 1)。
- Produces: `todo::update(ws_state: &mut WorkspaceState, msg: Message, project_id: i64, client: &Client, handle: &tokio::runtime::Handle, emit: impl Fn(Message) + Send + 'static)`——新签名,Task 7 继续在这个函数里加 match 分支。`pub fn request_todos_refresh(project_id: i64, client: &Client, handle: &tokio::runtime::Handle, emit: impl Fn(Message) + Send + 'static)`——轮询和写操作后刷新都调它。

- [ ] **Step 1: 读取现状确认改动边界**

Run:
```bash
grep -n "^pub fn \|^pub struct \|^pub enum \|^fn " crates/dozer-app/src/extensions/todo.rs
```
这会列出文件里全部顶层项(函数/结构体/枚举),对照下面 Step 2 的删除清单和 Step 3 的新增清单逐一核对,确保没有遗漏或误删跟 Todo 存储无关的东西(比如日历相关的纯函数 `today_ymd`/`days_in_month`/`first_weekday_of_month`/`parse_month_day`/`civil_from_days`/`days_from_civil` 全部保留不动,它们跟数据存储无关)。

- [ ] **Step 2: 删除文件 I/O 与 GUI 本地元数据相关的代码**

在 `crates/dozer-app/src/extensions/todo.rs` 里删除以下项(按现有行号定位,函数体本身不用逐行看,按函数名整体删除即可):

- `TodoItem` 结构体(约 22-26 行)—— 被 `dozer_core::protocol::TodoInfo` 取代。
- `todo_path` 函数(约 28-30 行)。
- `parse_todo` 函数(约 39-57 行)—— 逻辑已经在 Task 3 里以 `parse_legacy_markdown` 的形式搬进 `dozerd`,这里不再需要。
- `replace_todo_line` 函数(约 63-76 行)。
- `prepend_todo_item` 函数(约 83-105 行)。
- `move_pending_to` 函数(约 114-155 行)—— 排序逻辑已经在 Task 2 的 `TodoStore::reorder` 里用另一种(id 化)方式重新实现。
- `todo_line_key` 函数(约 163-167 行)。
- `TodoState`/`todo_display_state` **保留不动**(纯展示推导,不碰存储)。
- `DispatchRecord`/`TodoTaskMeta`/`TodoMetaState`/`meta_load`/`meta_save`/`load_from`/`save_to`/`file_path`(约 291-342 行整段)—— 全部被 `TodoInfo` 自带的字段取代。
- `AppState` 结构体及其 `impl`(`load`/`save`/`meta_for`/`record_dispatch`/`set_plan_date`/`set_completed_at`,约 691-752 行)—— 职责搬进了 `dozerd::todo::TodoStore`。
- `reload_from_disk` 函数(约 837-842 行)。
- `set_done` 函数(约 847-889 行)—— Step 5 里用新的异步版本取代。
- `commit_add_task` 函数(约 957-988 行)—— Step 5 里用新的异步版本取代。
- `commit_content_edit` 自由函数(约 990-1028 行)删除——**Task 7 负责重写等价逻辑**,`Message::ContentEdit` 里对它的调用改成本任务 Step 5 描述的临时处理(见下)。
- `impl WorkspaceState` 里委托给上面这个自由函数的同名方法(`pub fn commit_content_edit(&mut self, project_path: &Path)`,约 605-607 行)——自由函数删除后它编译不过,本任务先把它改成占位版本(**不是留空/TODO**,是一个行为明确的临时实现,Task 7 Step 1 会替换成正式版本):

  ```rust
      /// 临时版本(Task 6):失焦直接丢弃草稿,不落盘。Task 7 会改成返回
      /// `Option<(i64, String)>`,由调用方发起真正的 `edit_todo_text`。
      pub fn commit_content_edit(&mut self) {
          self.editing_content = None;
      }
  ```

  对应地,`app.rs` 里调用它的地方(`App::set_todo_content_focused`,约 3225-3234 行)本任务里把 `ws.todo.commit_content_edit(&path);` 改成 `ws.todo.commit_content_edit();`(去掉 `&path` 实参,方法暂时不需要路径)。

`TodoViewMode`/`markdown_editing`/`markdown_draft`/`ViewModeSet`/`MarkdownEditStart`/`MarkdownEvent`/`cancel_markdown_edit`/`todo_markdown_view`/`todo_view_tabs` 相关的删除在 Step 6 单独处理(MARKDOWN tab 移除是相对独立的一块,拆开做减少一次改动里出错的范围)。

- [ ] **Step 3: `WorkspaceState`/`Message` 改造**

`WorkspaceState` 结构体:
- `items: Vec<TodoItem>` → `items: Vec<TodoInfo>`。
- `mtime: Option<std::time::SystemTime>` 字段删除(不再有"文件 mtime"这个概念,轮询直接对比拉回来的列表内容——`TodoInfo` 已 `derive(PartialEq)`)。
- `pub fn mtime(&self) -> Option<...>` 访问器删除。
- `pub fn items(&self) -> &[TodoItem]` 改成 `pub fn items(&self) -> &[TodoInfo]`。
- 其余字段(`add_draft`/`add_focused`/`add_input_height`/`filter`/`selected_row`/`flash`/`scroll_to_top`/`search`/`search_draft`/`search_focused`/`dispatch_open`/`dispatch_anchor`/`calendar_open`/`calendar_view`/`calendar_anchor`/`editing_content`/`content_edit_focused`/`content_edit_focus_pending`/`drag`)**原样保留**——纯 UI 交互态,与存储无关。

在文件顶部 import 区加入(参照 `browser.rs` 顶部的 `use dozer_client::Client;`):

```rust
use dozer_client::Client;
use dozer_core::protocol::TodoInfo;
```

`Message` 枚举新增两个变体(紧跟在 `ClearList` 之后):

```rust
    /// 拉取列表的异步结果(轮询、或任一写操作成功后的刷新都落这里)。
    Loaded(Vec<TodoInfo>),
    /// 写操作(增/改/勾选/排序/计划日期/派发)的异步确认;不管成功失败都
    /// 触发一次 `Loaded` 刷新——成功时拿到权威的最新状态,失败时也借这次
    /// 刷新纠正掉之前的乐观更新。
    Mutated(Result<(), String>),
```

- [ ] **Step 4: 新增 `request_todos_refresh` + 乐观更新用的 sentinel id**

在文件里靠近 `todo_search_field_id`/`add_field_id` 一类的辅助函数处新增:

```rust
/// 乐观新增(`commit_add_task`)时,服务端真实 id 还没回来前的占位值。
/// 真实 id 从 1 起(`AUTOINCREMENT`),用 0 保证不会跟真实任务撞车。
/// `Mutated` 触发的刷新会用服务端权威列表整体替换掉带这个 id 的乐观行。
const OPTIMISTIC_TODO_ID: i64 = 0;

/// 现有 `Workspace::spawn_bookmarks_refresh`(见 `extensions::browser::
/// request_bookmarks_refresh`)同款手法的搬家版本:异步拉取某项目的任务
/// 列表。轮询(`App::poll_todo_if_visible`)和任一写操作成功后都调它。
pub fn request_todos_refresh(
    project_id: i64,
    client: &Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    let client = client.clone();
    handle.spawn(async move {
        let todos = client.list_todos(project_id).await.unwrap_or_default();
        emit(Message::Loaded(todos));
    });
}
```

- [ ] **Step 5: 重写 `update` 签名 + List/Add/Toggle 三个消息的处理**

`update` 函数签名改为:

```rust
pub fn update(
    ws_state: &mut WorkspaceState,
    msg: Message,
    project_id: i64,
    client: &Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
```

（不再接收 `app_state: &mut AppState`/`project_path: &Path`——`AppState` 已删除,文件路径不再需要。）

`match msg` 里新增/修改以下分支(其余未提到的分支——`Hover`/`ClearList`/`TextInputMenuOpen`/`ToggleListCollapse`/`AddEdit`/`AddResizeStart`/`FilterSet`/`RowSelect`/`SearchInput`/`SearchSubmit`/`DragMove`——原样保留不动,它们不碰存储):

```rust
        Message::Loaded(todos) => ws_state.items = todos,
        Message::Mutated(res) => {
            if let Err(e) = res {
                tracing::warn!("Todo 写操作失败: {e}");
            }
            request_todos_refresh(project_id, client, handle, emit);
        }
        Message::Toggle(idx) => {
            let Some(item) = ws_state.items.get(idx) else {
                return;
            };
            let id = item.id;
            let target = !item.done;
            // 乐观更新:本地立即翻转,给出与迁移前同等的零延迟反馈。
            if let Some(item) = ws_state.items.get_mut(idx) {
                item.done = target;
            }
            let client = client.clone();
            handle.spawn(async move {
                let res = client
                    .toggle_todo(id, target)
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string());
                emit(Message::Mutated(res));
            });
        }
        Message::AddSubmit => {
            let text = ws_state.add_draft.text();
            let text = text.trim().to_string();
            if text.is_empty() {
                return;
            }
            ws_state.add_draft = iced_widget::text_editor::Content::new();
            // 乐观置顶:插入一条临时 id 的条目,立即触发既有的"新增闪光+
            // 滚回顶部"效果,不等服务端往返。
            ws_state.items.insert(
                0,
                TodoInfo {
                    id: OPTIMISTIC_TODO_ID,
                    project_id,
                    text: text.clone(),
                    done: false,
                    rank: 0,
                    created_ms: 0,
                    completed_at_ms: None,
                    plan_date: None,
                    dispatch_session_id: None,
                    dispatch_at_ms: None,
                },
            );
            ws_state.start_flash(0);
            let client = client.clone();
            let project_id_owned = project_id;
            handle.spawn(async move {
                let res = client
                    .add_todo(project_id_owned, &text)
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string());
                emit(Message::Mutated(res));
            });
        }
```

`Message::ContentEdit(action)` 分支(现有约 1275-1284 行起)里对 `commit_content_edit` 的调用先保持函数签名不变但函数体placeholder——**不允许**;本任务里把这个分支临时改成只做本地草稿编辑、回车不提交(Task 7 会补上真正的提交逻辑):把原本 `if matches!(action, Edit::Enter) { commit_content_edit(...) }` 的分支体改成:

```rust
        Message::ContentEdit(action) => {
            if let Some((_, draft)) = ws_state.editing_content.as_mut() {
                draft.perform(action);
            }
        }
```

（即:先只做 `text_editor::Content::perform`,不做提交——这是 Task 7 明确要接手的部分,Step 5 到此为止,不要在本任务里尝试实现完整的 edit_text 调用,那属于 Task 7 的范围。）

`Message::DragEnd`/`Message::CalendarPick`/`Message::DispatchOpen`(popup 相关的 `dispatch_open`/`calendar_open` 状态切换保留)里任何调用了已删除函数(`move_pending_to`等)的部分,本任务先改成 no-op(不写盘,只保留状态切换),显式加注释说明:

```rust
        Message::DragEnd => {
            // 排序落盘逻辑由 Task 7 接入 `client.reorder_todo`,本任务只
            // 保留拖拽进行态清理,不重排。
            ws_state.drag = None;
        }
```

```rust
        Message::CalendarPick(idx, day) => {
            // 计划日期落盘逻辑由 Task 7 接入 `client.set_todo_plan_date`。
            let _ = (idx, day);
            ws_state.close_calendar_popup();
        }
```

- [ ] **Step 6: 移除 MARKDOWN 视图 tab**

删除:
- `TodoViewMode` 枚举定义(约 208-213 行)。
- `WorkspaceState::view_mode` 字段(约 363 行)。
- `WorkspaceState::markdown_draft` 字段(约 418 行)。
- `WorkspaceState::markdown_editing` 字段(约 414-415 行)。
- `Message::ViewModeSet(TodoViewMode)` 变体(约 777 行)。
- `Message::MarkdownEditStart` 变体(约 820 行)。
- `Message::MarkdownEvent(AddrEvent)` 变体(约 824 行)。
- `Message::ViewModeSet`/`MarkdownEditStart`/`MarkdownEvent` 在 `update` 里对应的 `match` 分支。
- `cancel_markdown_edit` 方法(约 617-628 行,`impl WorkspaceState` 内)。
- `markdown_editing()` 方法(约 610-612 行,`impl WorkspaceState` 内)。

Run: `grep -n "todo_view_tabs\|todo_markdown_view\|TodoViewMode::" crates/dozer-app/src/extensions/todo.rs`,读出这些引用点的上下文(建议 `sed -n '1420,1460p;1800,1870p;2580,2660p' crates/dozer-app/src/extensions/todo.rs`),据此删除:
- `todo_view_tabs` 函数(视图层的 List/MARKDOWN 切换 tab 按钮,约 2589-2650 行)。
- `todo_markdown_view` 函数(约 1807-1857 行)。
- 主视图函数里(`todo_view`/类似名字,约 1437-1448 行附近)对 `todo_view_tabs(...)` 的调用和 `match ws_state.view_mode { List => ..., Markdown => ... }` 的分支——改成直接、无条件调用 `todo_list_view(...)`,不再有 tab 切换。

- [ ] **Step 7: `app.rs`——移除 `todo::AppState` 字段与相关代码**

删除:
- `App` 结构体里的 `todo: todo::AppState` 字段(约 2161-2162 行)。
- `App::todo_meta(&self) -> &todo::AppState` 方法(约 2332-2338 行)。
- `App` 初始化里的 `todo: todo::AppState::load(),`(约 2482 行)。
- `App::todo_markdown_editing(&self) -> bool` 方法(约 3237-3241 行)——MARKDOWN tab 已移除,这个查询没有意义了。

`App::poll_todo_if_visible`(约 2679-2699 行)整体重写:

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
        self.with_focused_project(|_ws, io| {
            let client = io.client.clone();
            let handle = io.handle.clone();
            let proxy = io.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::Todo(m));
            };
            todo::request_todos_refresh(project_id, &client, &handle, emit);
        });
    }
```

`App::todo_message`(约 6049-6086 行)整体重写(`self.todo` 已删除,不再需要 `loaded_workspace_mut` 那种绕开借用冲突的写法,直接用 `with_focused_project`):

```rust
    fn todo_message(&mut self, msg: todo::Message) {
        if let todo::Message::AddResizeStart = msg {
            self.dragging_row = Some(RowDivider::TodoAddGrow);
            return;
        }
        let Some(project_id) = self.active_project_id else {
            return;
        };
        let last_cursor = self.last_cursor;
        self.with_focused_project(|ws, io| {
            if matches!(msg, todo::Message::CalendarOpen(_)) {
                ws.todo.set_calendar_anchor(last_cursor);
            }
            if matches!(msg, todo::Message::DispatchOpen(_)) {
                ws.todo.set_dispatch_anchor(last_cursor);
            }
            let client = io.client.clone();
            let handle = io.handle.clone();
            let proxy = io.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::Todo(m));
            };
            todo::update(&mut ws.todo, msg, project_id, &client, &handle, emit);
        });
    }
```

`Message::Todo(msg)` 顶层路由分支(约 4409-4416 行)加两条透传(`Loaded`/`Mutated` 都要真正进 `todo_message`,不能被内核拦截提前吃掉):确认这两个变体没有被现有的 `todo::Message::Hover`/`TextInputMenuOpen`/`ToggleListCollapse` 特判吞掉——它们已经落在 `other => self.todo_message(other),` 的通配分支里,不需要改动这段路由代码本身,只是确认一下(读一遍 4409-4416 行确认 `Loaded`/`Mutated` 会走到 `other` 分支)。

- [ ] **Step 8: `main.rs`——移除仅服务于 MARKDOWN 编辑的自绘输入路由 + 确认 `poll_todo_if_visible` 调用点**

`main.rs` 里有一整块键盘路由代码,存在的唯一理由就是给 Todo MARKDOWN 整文件编辑做"自绘输入"分发(注释原文:"文件树搜索框/项目树行内编辑框/浏览器地址栏/Todo 面板搜索框、新增任务框、任务内容编辑框/首页项目搜索框均已迁移 iced 原生控件……不再在此列"——MARKDOWN 是当时唯一剩下的自绘输入消费者)。MARKDOWN 删除后这块整体是死代码,必须一并删除,否则 `todo::Message::MarkdownEvent`(已在 Step 6 删除)会导致编译失败。

Run: `grep -n "to_todo_markdown\|to_self_drawn_input\|addr_message" crates/dozer-app/src/main.rs` 定位当前行号(现约 1203-1315 行),读一遍 `sed -n '1195,1320p' crates/dozer-app/src/main.rs` 确认边界后按下面三处改:

1. 删除 `let to_todo_markdown = app.todo_markdown_editing();`、`let to_self_drawn_input = to_todo_markdown;`、`let addr_message = |ev: workspace::AddrEvent| -> Message { Message::Todo(extensions::todo::Message::MarkdownEvent(ev)) };` 这三行定义及其上方的说明注释(约 1203-1219 行)。

2. ⌘V 粘贴分支(约 1243-1271 行)里,把:
   ```rust
   if to_self_drawn_input {
       app.update(addr_message(workspace::AddrEvent::Text(text)));
   } else {
       let target = app.keyboard_term_target();
       app.update(Message::TermPaste(target, text));
   }
   ```
   简化成(`to_self_drawn_input` 恒假,直接保留 `else` 分支的内容):
   ```rust
   let target = app.keyboard_term_target();
   app.update(Message::TermPaste(target, text));
   ```
   同一分支里紧接着的 `} else if !to_self_drawn_input && let Some(path) = clipboard_image::read_pasteboard_image_as_temp_file() {` 里的 `!to_self_drawn_input &&` 条件删除,简化成 `} else if let Some(path) = clipboard_image::read_pasteboard_image_as_temp_file() {`。

3. 删除整个 `if to_self_drawn_input { let addr_event = match event { .. }; if let Some(ev) = addr_event { app.update(addr_message(ev)); window.request_redraw(); } return; }` 代码块(约 1278-1315 行)——删除后,原本被这个 `return` 挡住的后续代码(IME 组字预览等终端相关处理,约 1317 行起)自然成为唯一的执行路径,不需要额外改动。

`app.poll_todo_if_visible()` 的调用点(约 1922/1953 行)本身不需要改动(签名没变,还是 `&mut self` 无参方法),只需确认编译通过。

- [ ] **Step 9: 重写受影响的单元测试**

`extensions/todo.rs` 底部 `mod tests` 里,原本直接调用旧版 `update(&mut ws_state, &mut app_state, Message::Toggle(0), 1, &root)` 这种签名的测试(现约 3035/3049/3260/3393/3516/3563/3592/3600 行附近,以及围绕 `TodoItem`/`AppState`/`meta_for` 构造测试 fixture 的辅助函数)需要改写。新增测试辅助:

```rust
    fn client_for_test() -> Client {
        Client::new(std::path::PathBuf::from(
            "/tmp/dozer-todo-test-nonexistent.sock",
        ))
    }
```

把测试里所有 `TodoItem { text, done }` 的构造替换成 `TodoInfo { id, project_id, text, done, rank: 0, created_ms: 0, completed_at_ms: None, plan_date: None, dispatch_session_id: None, dispatch_at_ms: None }`(按测试需要填 `id`/`project_id`/`completed_at_ms`/`plan_date`/`dispatch_session_id` 等字段)。

把直接调用 `update(&mut ws_state, &mut app_state, msg, project_id, &root)` 的测试改成:

```rust
        let handle = tokio::runtime::Handle::current();
        let client = client_for_test();
        update(&mut ws_state, msg, 1, &client, &handle, |_| {});
```

（这类测试需要标 `#[tokio::test]` 而不是 `#[test]`——`tokio::runtime::Handle::current()` 需要在 tokio runtime 里调用。）

对每个现有测试逐个过一遍,判断它测的是"纯本地状态变化"(比如 `Toggle` 的乐观更新、`AddSubmit` 的乐观插入)还是"文件落盘结果"(比如断言 `.dozer/todo.md` 的内容)——前者按上面的手法直接改写并保留断言意图(把断言目标从"文件内容"改成"`ws_state.items` 的乐观状态"),后者(旧版直接断言文件内容的测试,如果有的话)删除,因为落盘现在发生在 `dozerd` 侧、已经被 Task 2/3/5 的测试覆盖,不在这里重复测。

引用了已删除的 `AppState`/`meta_for`/`todo_line_key` 的测试(`task_title_for_session_*` 系列,约 3418-3453 行)本任务先删除——**Task 7** 会给 `task_title_for_session` 写新签名和新测试,这里不用保留旧版本占位。

- [ ] **Step 10: 编译 + 运行测试**

Run: `cargo build -p dozer-app 2>&1 | head -150`
Expected: 编译报错列表——按报错逐一修复(典型遗漏:某处还在用 `TodoItem`/`ws_state.mtime()`/`app_state`/`todo_path` 等已删除的类型或函数)。反复运行直到无报错。

Run: `cargo test -p dozer-app extensions::todo::`
Expected: 全部 PASS

- [ ] **Step 11: 全 workspace 编译确认**

Run: `cargo build --workspace`
Expected: 全绿(确认 `app.rs`/`main.rs` 的改动没有影响其它模块)

- [ ] **Step 12: 提交**

```bash
git add crates/dozer-app/src/extensions/todo.rs crates/dozer-app/src/app.rs
git commit -m "feat(dozer-app): Todo 数据层改用 UDS,移除 MARKDOWN tab"
```

---

## Task 7: dozer-app 剩余 CRUD 接线(编辑/排序/计划日期/派发)+ 清理

**Files:**
- Modify: `crates/dozer-app/src/extensions/todo.rs`
- Modify: `crates/dozer-app/src/app.rs`
- Modify: `crates/dozer-app/src/workspace.rs`
- Modify: `crates/dozer-app/src/main.rs`

**Interfaces:**
- Consumes: `Client::{edit_todo_text, reorder_todo, set_todo_plan_date, record_todo_dispatch}`(Task 5)、Task 6 打好的 `todo::update` 异步骨架。
- Produces: `WorkspaceState::task_title_for_session(&self, session_id: &str) -> Option<&str>`——新签名(去掉 `app_meta`/`project_id` 参数),`workspace.rs` 调用点同步更新。

- [ ] **Step 1: `ContentEdit` 回车提交改成异步 `edit_todo_text`**

在 `todo::update` 里,`Message::ContentEdit(action)` 分支(Task 6 Step 5 里临时改成的"只 perform 不提交"版本)重写为:

```rust
        Message::ContentEdit(action) => {
            let is_enter = matches!(
                action,
                iced_widget::text_editor::Action::Edit(iced_widget::text_editor::Edit::Enter)
            );
            if let Some((idx, draft)) = ws_state.editing_content.as_mut() {
                draft.perform(action.clone());
                if is_enter {
                    let idx = *idx;
                    let new_text = draft.text().trim().to_string();
                    let old_item = ws_state.items.get(idx).cloned();
                    ws_state.editing_content = None;
                    if let Some(old_item) = old_item
                        && !new_text.is_empty()
                        && old_item.text != new_text
                    {
                        let id = old_item.id;
                        let client = client.clone();
                        handle.spawn(async move {
                            let res = client
                                .edit_todo_text(id, &new_text)
                                .await
                                .map(|_| ())
                                .map_err(|e| e.to_string());
                            emit(Message::Mutated(res));
                        });
                    }
                }
            }
        }
```

（`action` 在 `is_enter` 判断之后还要 `perform`,所以先 `clone()` 一份给 `perform`,原值仍可用于 `matches!` 判断——上面代码已经这样处理:先算 `is_enter`,`perform` 用 `action.clone()`,原 `action` 不再被用到,写法上没有借用冲突。）

失焦提交路径(`WorkspaceState::commit_content_edit`,被 `App::set_todo_content_focused` 在失焦时调用)现在指向的旧 `commit_content_edit` 自由函数已在 Task 6 删除。重写 `impl WorkspaceState` 里的 `commit_content_edit` 方法——**这个方法现在需要 `client`/`handle`/`emit`,但 `WorkspaceState` 方法拿不到这些**,所以改成返回一个"待提交"的信号,由调用方(`app.rs`)负责真正发起异步调用:

```rust
    /// 失焦退出任务内容编辑态。返回 `Some((id, new_text))` 表示有改动需要
    /// 提交,调用方(`App::set_todo_content_focused`)据此发起
    /// `client.edit_todo_text`;返回 `None` 表示无改动或草稿为空,纯丢弃。
    pub fn commit_content_edit(&mut self) -> Option<(i64, String)> {
        let (idx, draft) = self.editing_content.take()?;
        let new_text = draft.text().trim().to_string();
        if new_text.is_empty() {
            return None;
        }
        let item = self.items.get(idx)?;
        if item.text == new_text {
            return None;
        }
        Some((item.id, new_text))
    }
```

（原来这个方法签名是 `pub fn commit_content_edit(&mut self, project_path: &Path)` 且没有返回值——现在改成上面这样,`project_path` 参数删除。）

- [ ] **Step 2: `app.rs`——`set_todo_content_focused` 适配新的 `commit_content_edit` 返回值**

找到调用 `ws.todo.commit_content_edit()` 的地方(`App` 的方法,大概率叫 `set_todo_content_focused`,约 3225-3234 行——Task 6 Step 7 已经把这一行从 `commit_content_edit(&path)` 改成了无参的 `commit_content_edit()`)。读取现有实现:

Run: `grep -n "fn set_todo_content_focused" -A 20 crates/dozer-app/src/app.rs`

把其中 `ws.todo.commit_content_edit();` 这一行(Task 6 留下的临时版本,丢弃草稿、不关心返回值)改成:

```rust
                if let Some((id, new_text)) = ws.todo.commit_content_edit() {
                    let client = io.client.clone();
                    let handle = io.handle.clone();
                    let proxy = io.proxy.clone();
                    handle.spawn(async move {
                        let res = client
                            .edit_todo_text(id, &new_text)
                            .await
                            .map(|_| ())
                            .map_err(|e| e.to_string());
                        let _ = proxy.send_event(Message::Todo(todo::Message::Mutated(res)));
                    });
                }
```

（需要确认这个方法体内此时能拿到 `io`——如果现有实现是走 `with_focused_project`/`with_project` 闭包就已经有 `io`;如果是别的写法,读一遍实际代码后按同样模式补上 `client`/`handle`/`proxy` 的获取,不要引入新的借用冲突。）

- [ ] **Step 3: `DragEnd` 改成异步 `reorder_todo`**

重写 Task 6 里临时做成 no-op 的 `Message::DragEnd` 分支:

```rust
        Message::DragEnd => {
            let Some(drag) = ws_state.drag.take() else {
                return;
            };
            if drag.source_idx == drag.target_idx {
                return;
            }
            let Some(source_item) = ws_state.items.get(drag.source_idx) else {
                return;
            };
            let id = source_item.id;
            // target_idx == usize::MAX 表示拖到待办块末尾(悬停到已完成
            // 卡片),此时 after_id 取"当前最后一个待办"的 id;否则取
            // target_idx 对应任务的 id(挪到它之后)。
            let after_id = if drag.target_idx == usize::MAX {
                // 先排除被拖动的任务本身再取"待办块末尾"——否则被拖的刚好
                // 就是原本最后一条待办时,会把自己算成自己的 after_id 而
                // 被过滤掉,错误地退化成"挪到最前"。
                ws_state
                    .items
                    .iter()
                    .filter(|it| !it.done && it.id != id)
                    .last()
                    .map(|it| it.id)
            } else {
                ws_state.items.get(drag.target_idx).map(|it| it.id)
            };
            let client = client.clone();
            handle.spawn(async move {
                let res = client
                    .reorder_todo(id, after_id)
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string());
                emit(Message::Mutated(res));
            });
        }
```

- [ ] **Step 4: `CalendarPick` 改成异步 `set_todo_plan_date`**

重写 Task 6 里临时做成 no-op 的 `Message::CalendarPick` 分支:

```rust
        Message::CalendarPick(idx, day) => {
            if let Some(item) = ws_state.items.get(idx) {
                let id = item.id;
                let client = client.clone();
                handle.spawn(async move {
                    let res = client
                        .set_todo_plan_date(id, Some(&day))
                        .await
                        .map(|_| ())
                        .map_err(|e| e.to_string());
                    emit(Message::Mutated(res));
                });
            }
            ws_state.close_calendar_popup();
        }
```

- [ ] **Step 5: `todo_dispatch_to_existing` 改成异步 `record_todo_dispatch`**

重写 `App::todo_dispatch_to_existing`(`app.rs` 约 6031-6047 行):

```rust
    fn todo_dispatch_to_existing(&mut self, idx: usize, session_id: String) {
        let Some(project_id) = self.active_project_id else {
            return;
        };
        let task = self
            .active_workspace()
            .and_then(|ws| ws.todo.items().get(idx))
            .cloned();
        let Some(task) = task else {
            return;
        };
        self.with_focused_project(|ws, io| {
            ws.todo.close_dispatch_popup();
            ws.dispatch_todo_to_existing(io, &session_id, &task.text);
            let id = task.id;
            let client = io.client.clone();
            let handle = io.handle.clone();
            let proxy = io.proxy.clone();
            handle.spawn(async move {
                let res = client
                    .record_todo_dispatch(id, &session_id)
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string());
                let _ = proxy.send_event(Message::Todo(todo::Message::Mutated(res)));
            });
        });
        let _ = project_id; // 保留:确认早退条件不变,project_id 本身不再直接使用
    }
```

（原来的 `self.todo.record_dispatch(project_id, &text, session_id);` 一行已经不需要——`self.todo` 已在 Task 6 删除。`TodoInfo` 需要 `derive(Clone)`,已在 Task 1 定义时加过,确认一下 Task 1 写的 derive 列表里有 `Clone`。）

- [ ] **Step 6: 简化 `task_title_for_session`**

重写 `WorkspaceState::task_title_for_session`(`todo.rs`,原签名 `pub fn task_title_for_session<'a>(&'a self, app_meta: &AppState, project_id: i64, session_id: &str) -> Option<&'a str>`):

```rust
    /// 反查:这个 `session_id` 是不是某条 Todo 任务派发出来的会话,是的话
    /// 返回该任务原文——给 Agent 卡片"当前工作内容"当主选数据源用。
    /// `TodoInfo` 自带 `dispatch_session_id`,不再需要额外的元数据表查询。
    pub fn task_title_for_session<'a>(&'a self, session_id: &str) -> Option<&'a str> {
        self.items
            .iter()
            .find(|item| item.dispatch_session_id.as_deref() == Some(session_id))
            .map(|item| item.text.as_str())
    }
```

- [ ] **Step 7: 更新 `task_title_for_session` 调用点**

`crates/dozer-app/src/workspace.rs` 约 2999-3000 行:

```rust
            ws.todo
                .task_title_for_session(app.todo_meta(), project_id, &tab.info.id)
```

改成:

```rust
            ws.todo.task_title_for_session(&tab.info.id)
```

（外层 `.and_then(|project_id| ...)` 闭包如果只是为了拿 `project_id` 传进去,现在不再需要这个闭包参数了——读一遍该处上下文(`sed -n '2985,3005p' crates/dozer-app/src/workspace.rs`),如果 `project_id` 闭包变量除了这一处没有其它用途,把 `.and_then(|project_id| ...)` 简化成直接调用;如果这个闭包外层还依赖 `ws.project_id()` 返回 `Some` 才执行(即"必须有项目才查"这个条件本身还要保留),改成 `.filter(|_| ws.project_id().is_some())` 或等价写法,保持"没有项目时不查"的原有行为。）

- [ ] **Step 8: 简化 `todo_calendar_overlay`(如果它依赖已删除的 `AppState`)**

Run: `sed -n '2460,2530p' crates/dozer-app/src/extensions/todo.rs`(此时行号可能因 Task 6 的删除而漂移,用 `grep -n "fn todo_calendar_overlay"` 先定位准确行号)读出现有实现。

如果函数体内有形如 `app_meta.meta_for(project_id, todo_line_key(&item.text))...plan_date` 的读取,改成直接读 `ws.todo.items()[idx].plan_date`(`idx` 是当前 `calendar_open` 指向的任务下标,函数入参里应该已经能拿到 `ws`/`ws_state`),并把函数签名里的 `app_meta: &AppState` 参数删除。

同步更新调用点 `app.rs` 约 7405 行:

```rust
            match todo::todo_calendar_overlay(&self.todo, ws, self.window_size) {
```

删除 `&self.todo` 实参(`self.todo` 已经不存在)。

- [ ] **Step 9: 检查其余引用了 `&ws.todo`/`app.todo_meta()` 的视图代码**

Run: `grep -n "todo_meta()\|&self\.todo\b" crates/dozer-app/src/app.rs crates/dozer-app/src/workspace.rs`
Expected: 无匹配(全部已在 Task 6/7 清理完)。若还有残留,按上面同样的思路(直接从 `TodoInfo` 字段读取,不再查 `AppState`)逐个修掉。

- [ ] **Step 10: 重写/新增受影响的单元测试**

`todo.rs` 测试模块里:
- 新增 `task_title_for_session` 的新签名测试(替换 Task 6 Step 9 删除的旧版本):

```rust
    #[test]
    fn task_title_for_session_finds_dispatched_task() {
        let mut ws_state = WorkspaceState::default();
        ws_state.items = vec![
            todo_info(1, "任务A", false),
            todo_info(2, "任务B", false),
        ];
        ws_state.items[1].dispatch_session_id = Some("sess-1".to_string());
        assert_eq!(
            ws_state.task_title_for_session("sess-1"),
            Some("任务B")
        );
    }

    #[test]
    fn task_title_for_session_none_when_no_dispatch_matches() {
        let mut ws_state = WorkspaceState::default();
        ws_state.items = vec![todo_info(1, "任务A", false)];
        assert_eq!(ws_state.task_title_for_session("sess-1"), None);
    }
```

（`todo_info(id, text, done)` 是个小测试辅助,若 Task 6 Step 9 没写过就在这里补一个:`fn todo_info(id: i64, text: &str, done: bool) -> TodoInfo { TodoInfo { id, project_id: 1, text: text.into(), done, rank: 0, created_ms: 0, completed_at_ms: None, plan_date: None, dispatch_session_id: None, dispatch_at_ms: None } }`。）

- 给 `commit_content_edit` 的新签名(Step 1)写测试:

```rust
    #[test]
    fn commit_content_edit_returns_id_and_text_when_changed() {
        let mut ws_state = WorkspaceState::default();
        ws_state.items = vec![todo_info(7, "旧文字", false)];
        ws_state.editing_content = Some((
            0,
            iced_widget::text_editor::Content::with_text("新文字"),
        ));
        let result = ws_state.commit_content_edit();
        assert_eq!(result, Some((7, "新文字".to_string())));
        assert!(ws_state.editing_content.is_none());
    }

    #[test]
    fn commit_content_edit_none_when_unchanged() {
        let mut ws_state = WorkspaceState::default();
        ws_state.items = vec![todo_info(7, "同样的文字", false)];
        ws_state.editing_content = Some((
            0,
            iced_widget::text_editor::Content::with_text("同样的文字"),
        ));
        assert_eq!(ws_state.commit_content_edit(), None);
    }

    #[test]
    fn commit_content_edit_none_when_draft_empty() {
        let mut ws_state = WorkspaceState::default();
        ws_state.items = vec![todo_info(7, "旧文字", false)];
        ws_state.editing_content = Some((0, iced_widget::text_editor::Content::new()));
        assert_eq!(ws_state.commit_content_edit(), None);
    }
```

- [ ] **Step 11: 编译 + 运行测试**

Run: `cargo build -p dozer-app 2>&1 | head -150`
Expected: 无报错(逐一修复直到干净)

Run: `cargo test -p dozer-app`
Expected: 全部 PASS

- [ ] **Step 12: 全 workspace 编译确认**

Run: `cargo build --workspace && cargo clippy -p dozer-app --all-targets`
Expected: 全绿,无 clippy 警告

- [ ] **Step 13: 提交**

```bash
git add crates/dozer-app/src/extensions/todo.rs crates/dozer-app/src/app.rs \
  crates/dozer-app/src/workspace.rs
git commit -m "feat(dozer-app): Todo 编辑/排序/计划日期/派发接入 UDS"
```

---

## Task 8: dozer-mcp 新增 4 个 Todo 工具

**Files:**
- Modify: `crates/dozer-mcp/src/server.rs`
- Create: `crates/dozer-mcp/tests/todo_tools.rs`

**Interfaces:**
- Consumes: `Client::{list_todos, add_todo, toggle_todo, edit_todo_text}`(Task 5)。
- Produces: `DozerMcpServer` 上的 4 个新 `#[tool]` 方法,对外通过 MCP stdio 暴露给外部 CLI agent。

- [ ] **Step 1: 新增 `resolve_project_id` 共用辅助 + 参数结构体**

在 `crates/dozer-mcp/src/server.rs`,`DozerMcpServer` 的私有 `impl` 块(`fetch_context` 所在的那个,非 `#[tool_router]` 块)里新增:

```rust
    /// session_id → project_id 的解析逻辑,`get_preview_context`(`fetch_
    /// context`)已经有一份等价实现;4 个新 Todo 工具复用同一套,抽成
    /// 独立方法避免四处重复。
    async fn resolve_project_id(&self) -> anyhow::Result<i64> {
        let sessions = self
            .client
            .list()
            .await
            .map_err(|e| anyhow::anyhow!("连接 dozerd 失败: {e}"))?;
        let session = sessions
            .iter()
            .find(|s| s.id == self.session_id)
            .ok_or_else(|| anyhow::anyhow!("会话不存在于 dozerd"))?;
        session
            .project_id
            .ok_or_else(|| anyhow::anyhow!("会话尚未归属任何项目"))
    }
```

在文件顶部参数结构体区域(`NoParams`/`SubmitSessionSummaryParams` 附近)新增:

```rust
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AddTodoParams {
    pub text: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ToggleTodoParams {
    pub id: i64,
    pub done: bool,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct EditTodoTextParams {
    pub id: i64,
    pub text: String,
}
```

- [ ] **Step 2: 新增 4 个 `#[tool]` 方法**

在 `#[tool_router(server_handler)] impl DozerMcpServer` 块内,`submit_session_summary` 方法之后追加:

```rust
    #[tool(
        description = "列出当前项目的任务列表(id/文字/是否完成)。想知道当前有哪些待办任务时调用。"
    )]
    pub async fn list_todos(
        &self,
        Parameters(NoParams {}): Parameters<NoParams>,
    ) -> Result<CallToolResult, McpError> {
        let project_id = self
            .resolve_project_id()
            .await
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        let todos = self
            .client
            .list_todos(project_id)
            .await
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        let value = json!(
            todos
                .into_iter()
                .map(|t| json!({ "id": t.id, "text": t.text, "done": t.done }))
                .collect::<Vec<_>>()
        );
        Ok(CallToolResult::structured(json!({ "todos": value })))
    }

    #[tool(description = "新增一条任务,置顶到列表最前。")]
    pub async fn add_todo(
        &self,
        Parameters(AddTodoParams { text }): Parameters<AddTodoParams>,
    ) -> Result<CallToolResult, McpError> {
        let project_id = self
            .resolve_project_id()
            .await
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        let todo = self
            .client
            .add_todo(project_id, &text)
            .await
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        Ok(CallToolResult::structured(
            json!({ "id": todo.id, "text": todo.text, "done": todo.done }),
        ))
    }

    #[tool(description = "勾选或取消勾选一条任务的完成状态。")]
    pub async fn toggle_todo(
        &self,
        Parameters(ToggleTodoParams { id, done }): Parameters<ToggleTodoParams>,
    ) -> Result<CallToolResult, McpError> {
        let todo = self
            .client
            .toggle_todo(id, done)
            .await
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        Ok(CallToolResult::structured(
            json!({ "id": todo.id, "text": todo.text, "done": todo.done }),
        ))
    }

    #[tool(description = "修改一条任务的文字内容。")]
    pub async fn edit_todo_text(
        &self,
        Parameters(EditTodoTextParams { id, text }): Parameters<EditTodoTextParams>,
    ) -> Result<CallToolResult, McpError> {
        let todo = self
            .client
            .edit_todo_text(id, &text)
            .await
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        Ok(CallToolResult::structured(
            json!({ "id": todo.id, "text": todo.text, "done": todo.done }),
        ))
    }
```

- [ ] **Step 3: 编译检查**

Run: `cargo build -p dozer-mcp`
Expected: 编译通过

- [ ] **Step 4: 写集成测试(仿 `get_preview_context.rs` 的 `start_daemon` 模式)**

创建 `crates/dozer-mcp/tests/todo_tools.rs`:

```rust
use dozer_client::Client;
use dozer_mcp::server::{
    AddTodoParams, DozerMcpServer, EditTodoTextParams, NoParams, ToggleTodoParams,
};
use rmcp::handler::server::wrapper::Parameters;
use std::sync::Arc;
use std::time::Duration;

fn temp_sock() -> std::path::PathBuf {
    let id = uuid::Uuid::new_v4();
    std::path::PathBuf::from(format!("/tmp/dz-mcp-todo-{}.sock", &id.to_string()[..8]))
}

struct CleanupGuard(std::path::PathBuf);
impl Drop for CleanupGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

async fn start_daemon() -> (std::path::PathBuf, CleanupGuard) {
    let sock = temp_sock();
    let registry = Arc::new(dozerd::registry::SessionRegistry::new());
    let db = std::path::PathBuf::from(format!("/tmp/dz-mcp-todo-{}.db", uuid::Uuid::new_v4()));
    let store = Arc::new(dozerd::acceptance::AcceptanceStore::open(&db).unwrap());
    let projects = Arc::new(dozerd::projects::ProjectStore::new(&db).unwrap());
    let bookmarks = Arc::new(dozerd::bookmarks::BookmarkStore::new(&db).unwrap());
    let transcripts = Arc::new(dozerd::transcripts::TranscriptStore::open(&db).unwrap());
    let session_summaries =
        Arc::new(dozerd::session_summary::SessionSummaryStore::open(&db).unwrap());
    let backfill_registry = Arc::new(dozerd::session_summary_backfill::BackfillRegistry::new());
    let todos = Arc::new(dozerd::todo::TodoStore::new(&db).unwrap());
    let s = sock.clone();
    tokio::spawn(async move {
        dozerd::server::serve(
            &s,
            registry,
            store,
            projects,
            bookmarks,
            transcripts,
            session_summaries,
            backfill_registry,
            todos,
        )
        .await
    });
    for _ in 0..100 {
        if sock.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let guard = CleanupGuard(sock.clone());
    (sock, guard)
}

async fn session_with_project(sock: &std::path::Path, project_id: i64) -> String {
    let client = Client::new(sock.to_path_buf());
    let session = client
        .create(
            "测试",
            "/bin/sh",
            &["-c".into(), "cat".into()],
            "/tmp",
            80,
            24,
            project_id,
        )
        .await
        .unwrap();
    session.id
}

#[tokio::test]
async fn add_list_toggle_and_edit_todo_via_mcp_tools() {
    let (sock, _guard) = start_daemon().await;
    let session_id = session_with_project(&sock, 1).await;
    let server = DozerMcpServer::new(Client::new(sock), session_id);

    let added = server
        .add_todo(Parameters(AddTodoParams {
            text: "写完 spec".into(),
        }))
        .await
        .unwrap();
    let added_value = added.structured_content.expect("结构化内容");
    assert_eq!(added_value["text"], "写完 spec");
    assert_eq!(added_value["done"], false);
    let id = added_value["id"].as_i64().unwrap();

    let listed = server
        .list_todos(Parameters(NoParams {}))
        .await
        .unwrap();
    let listed_value = listed.structured_content.expect("结构化内容");
    assert_eq!(listed_value["todos"].as_array().unwrap().len(), 1);

    let toggled = server
        .toggle_todo(Parameters(ToggleTodoParams { id, done: true }))
        .await
        .unwrap();
    assert_eq!(
        toggled.structured_content.expect("结构化内容")["done"],
        true
    );

    let edited = server
        .edit_todo_text(Parameters(EditTodoTextParams {
            id,
            text: "改过的文字".into(),
        }))
        .await
        .unwrap();
    assert_eq!(
        edited.structured_content.expect("结构化内容")["text"],
        "改过的文字"
    );
}

#[tokio::test]
async fn list_todos_errors_for_unknown_session() {
    let (sock, _guard) = start_daemon().await;
    let server = DozerMcpServer::new(Client::new(sock), "not-a-real-session".into());
    let err = server
        .list_todos(Parameters(NoParams {}))
        .await
        .unwrap_err();
    assert!(err.message.contains("会话不存在"));
}
```

- [ ] **Step 5: 确认 `server.rs` 的 `#[tool]` 方法/参数结构体对测试文件可见**

`AddTodoParams`/`EditTodoTextParams`/`ToggleTodoParams`/`NoParams` 需要 `pub`(仿 `SubmitSessionSummaryParams` 现有写法已经是 `pub`);`DozerMcpServer` 的 4 个新方法需要 `pub async fn`(`#[tool_router]` 宏展开后已经是 `pub`,与现有 `get_preview_context`/`submit_session_summary` 一致,不需要额外处理)。

- [ ] **Step 6: 运行测试**

Run: `cargo test -p dozer-mcp`
Expected: 全部 PASS(新增 2 个 + 既有 6 个)

- [ ] **Step 7: 提交**

```bash
git add crates/dozer-mcp/src/server.rs crates/dozer-mcp/tests/todo_tools.rs
git commit -m "feat(dozer-mcp): 新增 list_todos/add_todo/toggle_todo/edit_todo_text 工具"
```

---

## Task 9: 全 workspace 验证 + 手工 GUI 验收

**Files:** 无代码改动,纯验证。

- [ ] **Step 1: 全 workspace 构建**

Run: `cargo build --workspace`
Expected: 全绿

- [ ] **Step 2: 全 workspace 测试**

Run: `cargo test --workspace`
Expected: 全部 PASS

- [ ] **Step 3: Clippy**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: 无警告

- [ ] **Step 4: 格式检查**

Run: `cargo fmt -- --check`
Expected: 无需要格式化的文件(若有,运行 `cargo fmt` 后重新 `git add`/`commit` 一次单独的格式化提交)

- [ ] **Step 5: 手工 GUI 验证清单**

Run: `cargo run -p dozer-app`,对照以下清单逐项确认(全部通过才算本次迁移完工):

- [ ] 新增任务:输入文字提交,任务立即出现在列表顶部并有 2 秒选中高亮,重启 `dozerd` 后任务还在。
- [ ] 勾选/取消勾选任务:即时反馈,已完成任务沉到列表底部。
- [ ] 拖拽任务排序:待办块内任意拖动,松手后顺序落盘,重新打开面板顺序保持。
- [ ] 点击任务文字进入行内编辑,改完按回车或点别处,文字更新且不丢改动。
- [ ] 按过滤(全部/待办/进行中/已完成)+关键词搜索,结果符合预期。
- [ ] 点"指派"选一个已有会话,任务状态变成"进行中",且该 agent 卡片"当前工作内容"显示这条任务原文。
- [ ] 点日历图标选计划日期,徽章正确显示且保持。
- [ ] 确认面板顶部**没有** MARKDOWN 标签页,只剩列表视图。
- [ ] 找一个之前用过 Todo 面板、`.dozer/todo.md` 里有历史任务的项目(如果本机没有,先在旧版本 Dozer 或直接手写一份 `.dozer/todo.md` 造一个),用这次改造后的版本打开该项目,确认历史任务(含之前的派发记录/计划日期,如果造的 fixture 里有对应 `todo_meta.json`)被正确导入进列表。
- [ ] 用一个接了 `dozer-mcp` 的外部 CLI agent(如 opencode)在该项目里调用新工具(列出任务/新增任务/勾选/改文字),确认 GUI 侧刷新后能看到变化(轮询间隔 1 秒内)。
- [ ] 确认 `.dozer/todo.md`/`todo_meta.json` 原文件在磁盘上还在、内容未被 Dozer 改动(用 `git status`/`cat` 直接确认这次迁移过程没有意外写回旧文件)。

- [ ] **Step 6: 最终提交(若手工验证发现问题并修复)**

若 Step 5 发现问题,回到对应任务修复、补测试、重新走一遍 Step 1-4,确认全绿后提交修复。若手工验证全部通过且没有代码改动,本任务无需额外提交。
