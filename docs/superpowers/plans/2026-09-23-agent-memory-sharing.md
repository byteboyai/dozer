# Agent 记忆共享模块 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让多个 agent CLI(Claude Code/CodeBuddy/Codex/OpenCode/v8agent)通过 `dozer-mcp` 读写同一份 project 级共享记忆(送达一致、带审计历史),并在项目面板提供一个专属 UI 供人类查看、编辑、新建、删除。

**Architecture:** `dozerd` 新增 `MemoryStore`(SQLite,`memories`+`memory_history` 两表,project_id 作用域)作为唯一权威存储;`dozer-mcp` 新增 `write_memory`/`list_memories`/`get_memory` 三个工具(无 `delete_memory`,删除仅限人工);`dozer-app` 的 `extensions::project` 面板整体替换现有 `discover_memory` 文件链接区,改为列表+详情+历史时间线的专属子视图。

**Tech Stack:** Rust workspace;`rusqlite`(dozerd 存储);`rmcp`(`#[tool_router]`,dozer-mcp 工具);`iced 0.14` + `iced_widget`(dozer-app UI)。

**Spec:** `docs/superpowers/specs/2026-09-23-agent-memory-sharing-design.md`(brainstorming 会话已批准,含两次订正提交 `bfbad02f`/`a4362cb7`)

## Global Constraints

- 作用域仅 project 级(`project_id`),不做跨项目全局记忆——非目标,不要实现。
- `(project_id, title)` 是 upsert 键;`memory_history` 存每次改动**后**的完整快照,不存 diff。
- **没有 `delete_memory` MCP 工具**——agent 只能新建/更新,删除只能人工经 `dozer-app` UI(走 dozerd 协议的 `DeleteMemory` 请求,不经 `dozer-mcp`)。
- 归属信息统一存字符串:agent 写入用 `AgentKind::as_str()`(如 `"claude"`),人工在 UI 里操作用字面量 `"user"`。
- **不做**编辑冲突检测/乐观锁——v1 后写覆盖先写,`memory_history` 已经保住了被覆盖前的内容。
- **不做**语义搜索/`search_memories`——YAGNI,量级小时用不上。
- 旧的 `discover_memory`/`LinksState.memory`/`ProjectToolbarTarget::Memory` 整体下线,**不做数据迁移**;`.dozer/links.json` 里遗留的 `memory` 字段会被 serde 静默忽略(不用写迁移代码)。
- `dozerd::server::serve`/`handle_conn` 的 `Arc<XStore>` 参数已经堆到 12/16 个,接入 `MemoryStore` 前必须先归到一个 `#[derive(Clone)] pub struct Stores`(理由是纯可维护性,不是"同类型编译器发现不了"那条裁决——各 `Arc<XStore>` 本身是不同的具体类型,见 spec 订正)。
- UI 部分只用普通 iced widget(text/button/column/row/text_editor),不涉及 canvas 自绘文本,不触发 `Font::default()`/`Shaping` 那条裁决的风险场景——**不要**引入 `assets::fonts::code_font()`,记忆标题/正文都是普通 UI 文本场景。
- 新代码不引入 `#[allow(clippy::too_many_arguments)]` 来绕开参数结构体裁决。

## Review Focus

- **`write_memory` 传空 `title`**:agent 或人工可能传空字符串——`(project_id, "")` 会占住 unique index 的一个槽位,之后所有空标题写入都会互相覆盖成"同一条"而不是分别失败;需要在 `MemoryStore` 层面明确这属于合法输入(不额外校验,行为可预测),并有测试锁定"空标题也能 upsert、且和非空标题互不冲突"这个边界,不能是未定义行为。
- **同一 `(project_id, title)` 被人工和 agent 几乎同时各写一次**:后写覆盖先写是设计决定,但 `memory_history` 必须两条改动都留痕(不能因为主表只保留最后一次而在历史表也丢一条)。
- **删除一条记忆后再查它的历史**:`memory_history` 不能因为 `memories` 表那一行被物理删除就跟着查不到或外键报错——历史必须在记录死后仍可查。
- **跨 project 的同名 title 互不影响**,且 `get`/`delete` 传入"存在但属于别的 project 的 id"要报"不存在",不能越权读到/删掉别的项目的记忆。
- **`resolve_project_and_agent` 解析失败**(`session_id` 在 `dozerd` 里查不到,比如会话已结束或 `DOZER_SESSION_ID` 传错):三个新 MCP 工具都要走已有的 `McpError::internal_error` 错误路径,不能 panic、不能返回半截结果。

---

### Task 1: `dozerd::memory::MemoryStore`(数据模型 + CRUD + 历史)

**Files:**
- Create: `crates/dozerd/src/memory.rs`
- Modify: `crates/dozerd/src/lib.rs`(加 `pub mod memory;`,紧邻现有 `pub mod todo;` 那行)

**Interfaces:**
- Consumes: `dozer_core::protocol::{MemoryInfo, MemoryDetail, MemoryHistoryEntry}`(Task 2 产出,Task 1 先写 Task 2 需要的字段形状作为本任务内部草稿,Task 2 落地后二者字段必须完全一致——见下方类型定义,两任务共用同一份)。
- Produces:`pub struct MemoryStore`,方法 `new(path: &Path) -> Result<Self>`、`list(&self, project_id: i64) -> Result<Vec<MemoryInfo>>`、`get(&self, project_id: i64, id: i64) -> Result<MemoryDetail>`、`write(&self, project_id: i64, title: &str, kind: &str, description: &str, body: &str, actor: &str) -> Result<MemoryDetail>`、`delete(&self, project_id: i64, id: i64, actor: &str) -> Result<()>`、`history(&self, project_id: i64, memory_id: i64, limit: i64) -> Result<Vec<MemoryHistoryEntry>>`。供 Task 4 使用。

- [ ] **Step 1: 在 `dozer-core` 里先补 `MemoryInfo`/`MemoryDetail`/`MemoryHistoryEntry` 三个类型(Task 2 的一部分提前做,因为 Task 1 编译依赖它们)**

打开 `crates/dozer-core/src/protocol.rs`,在 `TodoInfo` 结构体定义之后(约第 468 行,`CategoryInfo` 之前)插入:

```rust
/// 一条共享记忆(`dozerd` 的 `memories` 表一行)。`(project_id, title)`
/// 唯一,`write_memory` 按这个键做 upsert(2026-09-23 agent 记忆共享设计)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryInfo {
    pub id: i64,
    pub project_id: i64,
    pub title: String,
    /// 自由字符串,不做枚举约束;默认建议集合 `user`/`feedback`/`project`/
    /// `reference`,但其他 agent 塞别的字符串也不报错。
    pub kind: String,
    pub description: String,
    pub updated_ms: u64,
    /// `AgentKind::as_str()` 的结果,或人工操作时的字面量 `"user"`。
    pub updated_by: String,
}

/// `GetMemory`/`WriteMemory` 应答里的完整记录,比 `MemoryInfo` 多正文和
/// 最近历史。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryDetail {
    pub id: i64,
    pub project_id: i64,
    pub title: String,
    pub kind: String,
    pub description: String,
    pub body: String,
    pub created_ms: u64,
    pub created_by: String,
    pub updated_ms: u64,
    pub updated_by: String,
    /// `write`/`delete` 返回的 `MemoryDetail` 里这个字段恒为空(写操作不
    /// 顺带查历史);要看历史需要单独调 `MemoryStore::history`/走
    /// `GetMemory` 请求。
    pub history: Vec<MemoryHistoryEntry>,
}

/// `memory_history` 表一行:某次改动后的完整快照。`change_kind` 是
/// `"created"`/`"updated"`/`"deleted"` 三选一字符串(不用 Rust enum 是因为
/// 这条记录被删除后仍要能反序列化出历史,不希望未来加新 kind 时旧数据的
/// enum 解不出来——字符串更宽容)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryHistoryEntry {
    pub id: i64,
    pub memory_id: i64,
    pub changed_ms: u64,
    pub changed_by: String,
    pub change_kind: String,
    pub title: String,
    pub kind: String,
    pub description: String,
    pub body: String,
}
```

- [ ] **Step 2: Write the failing test(`MemoryStore` 基本 CRUD)**

创建 `crates/dozerd/src/memory.rs`,先写文件头 + 测试模块(实现留空/编译失败是预期的,下一步补实现):

```rust
//! 共享记忆存储:project 级 SQLite,`memories`(现状)+ `memory_history`
//! (审计轨迹)两张表,与 `TodoStore` 共享同一个 `dozer.db`。
//! 详见设计 `docs/superpowers/specs/2026-09-23-agent-memory-sharing-design.md`。

use anyhow::{Context, Result};
use dozer_core::protocol::{MemoryDetail, MemoryHistoryEntry, MemoryInfo};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;
use std::sync::Mutex;

pub struct MemoryStore {
    conn: Mutex<Connection>,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn id_not_found(id: i64) -> anyhow::Error {
    anyhow::anyhow!("记忆不存在: id={id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, MemoryStore) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("m.db");
        let store = MemoryStore::new(&db).unwrap();
        (dir, store)
    }

    #[test]
    fn write_creates_then_updates_same_title() {
        let (_dir, store) = store();
        let created = store
            .write(1, "标题A", "project", "desc", "正文一", "claude")
            .unwrap();
        assert_eq!(created.body, "正文一");
        assert_eq!(created.created_by, "claude");

        let updated = store
            .write(1, "标题A", "project", "desc2", "正文二", "user")
            .unwrap();
        assert_eq!(updated.id, created.id, "同标题应该更新同一条,不新建");
        assert_eq!(updated.body, "正文二");
        assert_eq!(updated.updated_by, "user");

        let listed = store.list(1).unwrap();
        assert_eq!(listed.len(), 1, "upsert 不应产生第二行");
    }

    #[test]
    fn write_records_history_for_create_and_update() {
        let (_dir, store) = store();
        store
            .write(1, "标题A", "project", "d", "v1", "claude")
            .unwrap();
        store
            .write(1, "标题A", "project", "d", "v2", "codebuddy")
            .unwrap();
        let detail = store.get(1, 1).unwrap();
        let history = store.history(1, detail.id, 10).unwrap();
        assert_eq!(history.len(), 2, "两次写入各留一条历史");
        // 倒序:最新的在前。
        assert_eq!(history[0].change_kind, "updated");
        assert_eq!(history[0].body, "v2");
        assert_eq!(history[0].changed_by, "codebuddy");
        assert_eq!(history[1].change_kind, "created");
        assert_eq!(history[1].body, "v1");
    }

    #[test]
    fn empty_title_is_a_valid_independent_key() {
        let (_dir, store) = store();
        let a = store.write(1, "", "project", "", "空标题的内容", "claude").unwrap();
        let b = store.write(1, "标题B", "project", "", "另一条", "claude").unwrap();
        assert_ne!(a.id, b.id);
        assert_eq!(store.list(1).unwrap().len(), 2);
        // 空标题第二次写入应该更新同一条(和非空标题的 upsert 语义一致),
        // 不会因为标题是空串就退化成每次都新建。
        let a2 = store.write(1, "", "project", "", "空标题改了", "claude").unwrap();
        assert_eq!(a2.id, a.id);
        assert_eq!(store.list(1).unwrap().len(), 2, "空标题的 upsert 不应产生第三行");
    }

    #[test]
    fn delete_removes_row_but_history_survives() {
        let (_dir, store) = store();
        let created = store.write(1, "标题A", "project", "d", "v1", "claude").unwrap();
        store.delete(1, created.id, "user").unwrap();

        assert!(store.list(1).unwrap().is_empty());
        assert!(store.get(1, created.id).is_err(), "删除后 get 应该报不存在");

        let history = store.history(1, created.id, 10).unwrap();
        assert_eq!(history.len(), 2, "创建 + 删除各一条历史");
        assert_eq!(history[0].change_kind, "deleted");
        assert_eq!(history[0].changed_by, "user");
        assert_eq!(history[0].body, "v1", "删除快照存的是删除前的最后内容");
    }

    #[test]
    fn projects_are_isolated_for_title_get_and_delete() {
        let (_dir, store) = store();
        let a = store.write(1, "同名", "project", "", "项目1的内容", "claude").unwrap();
        let b = store.write(2, "同名", "project", "", "项目2的内容", "claude").unwrap();
        assert_ne!(a.id, b.id, "不同 project 下同标题不冲突");

        assert!(store.get(2, a.id).is_err(), "不能跨项目读到别的项目的记忆");
        assert!(store.delete(2, a.id, "user").is_err(), "不能跨项目删掉别的项目的记忆");

        // 确认没有被误删。
        assert_eq!(store.get(1, a.id).unwrap().body, "项目1的内容");
    }

    #[test]
    fn get_and_delete_unknown_id_error() {
        let (_dir, store) = store();
        assert!(store.get(1, 999).is_err());
        assert!(store.delete(1, 999, "user").is_err());
    }

    #[test]
    fn list_orders_by_updated_ms_desc() {
        let (_dir, store) = store();
        let a = store.write(1, "先写的", "project", "", "v", "claude").unwrap();
        let b = store.write(1, "后写的", "project", "", "v", "claude").unwrap();
        let listed = store.list(1).unwrap();
        assert_eq!(listed[0].id, b.id, "最近更新的排最前");
        assert_eq!(listed[1].id, a.id);
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p dozerd memory:: 2>&1 | tail -40`
Expected: 编译失败(`MemoryStore::new`/`write`/`list`/`get`/`delete`/`history` 未定义)。

- [ ] **Step 4: 写最小实现让测试通过**

在 `crates/dozerd/src/memory.rs`(测试模块之前)补齐:

```rust
impl MemoryStore {
    pub fn new(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("建库目录")?;
        }
        let conn = Connection::open(path).context("打开 SQLite")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS memories (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                project_id INTEGER NOT NULL,
                title TEXT NOT NULL,
                kind TEXT NOT NULL,
                description TEXT NOT NULL DEFAULT '',
                body TEXT NOT NULL,
                created_ms INTEGER NOT NULL,
                created_by TEXT NOT NULL,
                updated_ms INTEGER NOT NULL,
                updated_by TEXT NOT NULL
             );
             CREATE UNIQUE INDEX IF NOT EXISTS memories_project_title
                ON memories(project_id, title);
             CREATE TABLE IF NOT EXISTS memory_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                memory_id INTEGER NOT NULL,
                project_id INTEGER NOT NULL,
                changed_ms INTEGER NOT NULL,
                changed_by TEXT NOT NULL,
                change_kind TEXT NOT NULL,
                title_snapshot TEXT NOT NULL,
                kind_snapshot TEXT NOT NULL,
                description_snapshot TEXT NOT NULL,
                body_snapshot TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS memory_history_memory
                ON memory_history(memory_id, changed_ms);",
        )
        .context("建表")?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn list(&self, project_id: i64) -> Result<Vec<MemoryInfo>> {
        let conn = self.conn.lock().expect("db lock");
        let mut stmt = conn.prepare(
            "SELECT id, project_id, title, kind, description, updated_ms, updated_by
             FROM memories WHERE project_id = ?1 ORDER BY updated_ms DESC",
        )?;
        let rows = stmt.query_map([project_id], |r| {
            Ok(MemoryInfo {
                id: r.get(0)?,
                project_id: r.get(1)?,
                title: r.get(2)?,
                kind: r.get(3)?,
                description: r.get(4)?,
                updated_ms: r.get::<_, i64>(5)? as u64,
                updated_by: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn get(&self, project_id: i64, id: i64) -> Result<MemoryDetail> {
        let conn = self.conn.lock().expect("db lock");
        let row: Option<(String, String, String, String, i64, String, i64, String)> = conn
            .query_row(
                "SELECT title, kind, description, body, created_ms, created_by, updated_ms, updated_by
                 FROM memories WHERE id = ?1 AND project_id = ?2",
                params![id, project_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?)),
            )
            .optional()?;
        let Some((title, kind, description, body, created_ms, created_by, updated_ms, updated_by)) = row else {
            return Err(id_not_found(id));
        };
        let history = history_query(&conn, project_id, id, 20)?;
        Ok(MemoryDetail {
            id,
            project_id,
            title,
            kind,
            description,
            body,
            created_ms: created_ms as u64,
            created_by,
            updated_ms: updated_ms as u64,
            updated_by,
            history,
        })
    }

    pub fn write(
        &self,
        project_id: i64,
        title: &str,
        kind: &str,
        description: &str,
        body: &str,
        actor: &str,
    ) -> Result<MemoryDetail> {
        let conn = self.conn.lock().expect("db lock");
        let now = now_ms() as i64;
        let existing_id: Option<i64> = conn
            .query_row(
                "SELECT id FROM memories WHERE project_id = ?1 AND title = ?2",
                params![project_id, title],
                |r| r.get(0),
            )
            .optional()?;
        let change_kind = if existing_id.is_some() { "updated" } else { "created" };
        let sql = "INSERT INTO memories
                (project_id, title, kind, description, body, created_ms, created_by, updated_ms, updated_by)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?6, ?7)
             ON CONFLICT(project_id, title) DO UPDATE SET
                kind = excluded.kind,
                description = excluded.description,
                body = excluded.body,
                updated_ms = excluded.updated_ms,
                updated_by = excluded.updated_by
             RETURNING id, created_ms, created_by";
        let (id, created_ms, created_by): (i64, i64, String) = conn.query_row(
            sql,
            params![project_id, title, kind, description, body, now, actor],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        conn.execute(
            "INSERT INTO memory_history
                (memory_id, project_id, changed_ms, changed_by, change_kind,
                 title_snapshot, kind_snapshot, description_snapshot, body_snapshot)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![id, project_id, now, actor, change_kind, title, kind, description, body],
        )?;
        Ok(MemoryDetail {
            id,
            project_id,
            title: title.to_string(),
            kind: kind.to_string(),
            description: description.to_string(),
            body: body.to_string(),
            created_ms: created_ms as u64,
            created_by,
            updated_ms: now as u64,
            updated_by: actor.to_string(),
            history: Vec::new(),
        })
    }

    pub fn delete(&self, project_id: i64, id: i64, actor: &str) -> Result<()> {
        let conn = self.conn.lock().expect("db lock");
        let now = now_ms() as i64;
        let row: Option<(String, String, String, String)> = conn
            .query_row(
                "SELECT title, kind, description, body FROM memories WHERE id = ?1 AND project_id = ?2",
                params![id, project_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        let Some((title, kind, description, body)) = row else {
            return Err(id_not_found(id));
        };
        conn.execute(
            "INSERT INTO memory_history
                (memory_id, project_id, changed_ms, changed_by, change_kind,
                 title_snapshot, kind_snapshot, description_snapshot, body_snapshot)
             VALUES (?1, ?2, ?3, ?4, 'deleted', ?5, ?6, ?7, ?8)",
            params![id, project_id, now, actor, title, kind, description, body],
        )?;
        conn.execute("DELETE FROM memories WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn history(&self, project_id: i64, memory_id: i64, limit: i64) -> Result<Vec<MemoryHistoryEntry>> {
        let conn = self.conn.lock().expect("db lock");
        history_query(&conn, project_id, memory_id, limit)
    }
}

/// `get`(已持锁)和公开的 `history`(自己上锁)共用的查询体,避免
/// `Mutex` 二次加锁死锁。
fn history_query(
    conn: &Connection,
    project_id: i64,
    memory_id: i64,
    limit: i64,
) -> Result<Vec<MemoryHistoryEntry>> {
    let mut stmt = conn.prepare(
        "SELECT id, memory_id, changed_ms, changed_by, change_kind,
                title_snapshot, kind_snapshot, description_snapshot, body_snapshot
         FROM memory_history
         WHERE project_id = ?1 AND memory_id = ?2
         ORDER BY changed_ms DESC LIMIT ?3",
    )?;
    let rows = stmt.query_map(params![project_id, memory_id, limit], |r| {
        Ok(MemoryHistoryEntry {
            id: r.get(0)?,
            memory_id: r.get(1)?,
            changed_ms: r.get::<_, i64>(2)? as u64,
            changed_by: r.get(3)?,
            change_kind: r.get(4)?,
            title: r.get(5)?,
            kind: r.get(6)?,
            description: r.get(7)?,
            body: r.get(8)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}
```

在 `crates/dozerd/src/lib.rs` 里紧邻 `pub mod todo;` 加一行 `pub mod memory;`。

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p dozerd memory:: 2>&1 | tail -40`
Expected: 7 个测试全部 PASS。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-core/src/protocol.rs crates/dozerd/src/memory.rs crates/dozerd/src/lib.rs
git commit -m "feat(dozerd): add MemoryStore for shared agent memory"
```

---

### Task 2: `dozer-core::protocol` 请求/应答 + 序列化回归测试

**Files:**
- Modify: `crates/dozer-core/src/protocol.rs`

**Interfaces:**
- Consumes: `MemoryInfo`/`MemoryDetail`/`MemoryHistoryEntry`(Task 1 已加)。
- Produces:`Request::{WriteMemory, ListMemories, GetMemory, DeleteMemory}`、`Reply::{Memories, MemoryDetail}`,供 Task 4(dozerd 分发)、Task 5(`dozer-client`)、Task 6(`dozer-mcp`)使用。

- [ ] **Step 1: 在 `enum Request` 里加四个新变体**

在 `ProcessTodoNow` 变体之后(紧邻 Todo 请求组,约第 778 行)插入:

```rust
    /// 列出某项目全部共享记忆(不含正文,列表用)。按 `updated_ms` 倒序。
    ListMemories {
        project_id: i64,
    },
    /// 按 `(project_id, title)` upsert:标题已存在则更新,否则新建。
    /// `actor` 是 `AgentKind::as_str()` 或人工操作时的字面量 `"user"`,
    /// 由调用方(`dozer-mcp`/`dozer-app`)决定,不在 `dozerd` 里推导。
    WriteMemory {
        project_id: i64,
        title: String,
        kind: String,
        description: String,
        body: String,
        actor: String,
    },
    /// 查一条记忆的完整正文 + 最近历史。`id` 不属于 `project_id` 时视同
    /// 不存在(项目隔离)。
    GetMemory {
        project_id: i64,
        id: i64,
    },
    /// 人工删除一条记忆(`dozer-mcp` 不暴露对应工具,只有 `dozer-app` UI
    /// 会发这个请求)。删除前的最后状态会被写进一条 `deleted` 历史。
    DeleteMemory {
        project_id: i64,
        id: i64,
        actor: String,
    },
```

- [ ] **Step 2: 在 `enum Reply` 里加两个新变体**

在 `Todo { todo: TodoInfo }` 之后插入:

```rust
    /// `ListMemories` 应答。
    Memories {
        memories: Vec<MemoryInfo>,
    },
    /// `WriteMemory`/`GetMemory` 应答。
    MemoryDetail {
        detail: MemoryDetail,
    },
```

- [ ] **Step 3: Write the failing test(序列化回归)**

紧邻现有 `todo_protocol_types_roundtrip` 测试之后加一个新测试:

```rust
    #[test]
    fn memory_protocol_types_roundtrip() {
        let write_req = Request::WriteMemory {
            project_id: 1,
            title: "标题A".into(),
            kind: "project".into(),
            description: "一句话摘要".into(),
            body: "正文内容".into(),
            actor: "claude".into(),
        };
        let line = encode_line(&write_req);
        assert_eq!(decode_line::<Request>(&line).unwrap(), write_req);

        let get_req = Request::GetMemory {
            project_id: 1,
            id: 5,
        };
        let line = encode_line(&get_req);
        assert_eq!(decode_line::<Request>(&line).unwrap(), get_req);

        let delete_req = Request::DeleteMemory {
            project_id: 1,
            id: 5,
            actor: "user".into(),
        };
        let line = encode_line(&delete_req);
        assert_eq!(decode_line::<Request>(&line).unwrap(), delete_req);

        let info = MemoryInfo {
            id: 1,
            project_id: 1,
            title: "标题A".into(),
            kind: "project".into(),
            description: "desc".into(),
            updated_ms: 1_700_000_000_000,
            updated_by: "claude".into(),
        };
        let list_reply = Reply::Memories {
            memories: vec![info],
        };
        let line = encode_line(&list_reply);
        assert_eq!(decode_line::<Reply>(&line).unwrap(), list_reply);

        let detail = MemoryDetail {
            id: 1,
            project_id: 1,
            title: "标题A".into(),
            kind: "project".into(),
            description: "desc".into(),
            body: "正文".into(),
            created_ms: 1_700_000_000_000,
            created_by: "claude".into(),
            updated_ms: 1_700_000_001_000,
            updated_by: "user".into(),
            history: vec![MemoryHistoryEntry {
                id: 1,
                memory_id: 1,
                changed_ms: 1_700_000_001_000,
                changed_by: "user".into(),
                change_kind: "updated".into(),
                title: "标题A".into(),
                kind: "project".into(),
                description: "desc".into(),
                body: "正文".into(),
            }],
        };
        let detail_reply = Reply::MemoryDetail { detail };
        let line = encode_line(&detail_reply);
        assert_eq!(decode_line::<Reply>(&line).unwrap(), detail_reply);
    }
```

- [ ] **Step 4: Run test to verify it fails, then passes**

Run: `cargo test -p dozer-core memory_protocol_types_roundtrip 2>&1 | tail -30`
先确认编译期就已经因为上面 Step 1/2 加了变体而能通过(这一步主要是防止手滑打错字段名/类型),再确认测试本身 PASS。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-core/src/protocol.rs
git commit -m "feat(protocol): add memory request/reply variants"
```

---

### Task 3: `dozerd::server` 的 `Stores` 结构体重构(机械重构,不改行为)

**Files:**
- Modify: `crates/dozerd/src/server.rs:70-172`(`serve`)、`crates/dozerd/src/server.rs:407-...`(`handle_conn` 签名与函数体开头)
- Modify: `crates/dozerd/src/main.rs`(`serve(...)` 调用点)
- Modify: `crates/dozer-mcp/tests/todo_tools.rs`(`dozerd::server::serve(...)` 调用点)
- Modify: 其余所有调用 `dozerd::server::serve` 的测试文件(见 Step 1 的搜索)

**Interfaces:**
- Produces:`pub struct Stores { pub registry: Arc<SessionRegistry>, pub projects: Arc<ProjectStore>, pub bookmarks: Arc<BookmarkStore>, pub code_health: Arc<CodeHealthStore>, pub transcripts: Arc<TranscriptStore>, pub session_summaries: Arc<SessionSummaryStore>, pub backfill_registry: Arc<BackfillRegistry>, pub todos: Arc<TodoStore>, pub categories: Arc<CategoryStore> }`(`#[derive(Clone)]`),`serve(socket: &Path, ide_lock_dir: PathBuf, stores: Stores, in_flight: InFlight) -> Result<()>`。Task 4 会往这个结构体加 `memories` 字段。

- [ ] **Step 1: 找出所有调用点(先普查,不改代码)**

Run: `grep -rn "server::serve(\|dozerd::server::serve(" crates/ --include="*.rs"`

记录下每一处调用(至少包含 `crates/dozerd/src/main.rs` 和
`crates/dozer-mcp/tests/todo_tools.rs`;如果普查出其他测试文件也调了
`serve`,一并记下,后面 Step 4 要逐个改)。

- [ ] **Step 2: 定义 `Stores` 结构体**

在 `crates/dozerd/src/server.rs` 里 `pub async fn serve(` 定义之前插入:

```rust
/// `serve`/`handle_conn` 共用的存储句柄集合,归到一个具名字段结构体里
/// (而不是继续平铺成十几个位置参数)——纯可维护性考虑:各 `Arc<XStore>`
/// 本身是互不相同的具体类型,位置传错本来就会被 Rust 类型检查拦下,不属于
/// CLAUDE.md"同类型相邻、编译器发现不了"那条裁决针对的情况,这里单纯是
/// 参数表已经太长。全部字段都是 `Arc<..>`,派生 `Clone` 零成本。
#[derive(Clone)]
pub struct Stores {
    pub registry: std::sync::Arc<SessionRegistry>,
    pub projects: std::sync::Arc<crate::projects::ProjectStore>,
    pub bookmarks: std::sync::Arc<crate::bookmarks::BookmarkStore>,
    pub code_health: std::sync::Arc<crate::code_health::CodeHealthStore>,
    pub transcripts: std::sync::Arc<crate::transcripts::TranscriptStore>,
    pub session_summaries: std::sync::Arc<crate::session_summary::SessionSummaryStore>,
    pub backfill_registry: std::sync::Arc<crate::session_summary_backfill::BackfillRegistry>,
    pub todos: std::sync::Arc<crate::todo::TodoStore>,
    pub categories: std::sync::Arc<crate::todo_category::CategoryStore>,
}
```

- [ ] **Step 3: 改 `serve` 签名与内部展开**

把 `pub async fn serve(` 的签名从 12 个位置参数改成:

```rust
pub async fn serve(
    socket: &Path,
    ide_lock_dir: PathBuf,
    stores: Stores,
    in_flight: crate::task_poller::InFlight,
) -> Result<()> {
```

紧接着在函数体最开头解构出原来的局部变量名(**函数体其余代码不用改**,靠解构保持变量名不变):

```rust
    let Stores {
        registry,
        projects,
        bookmarks,
        code_health,
        transcripts,
        session_summaries,
        backfill_registry,
        todos,
        categories,
    } = stores;
```

- [ ] **Step 4: 改 `handle_conn` 签名**

把 `async fn handle_conn(` 的前 13 个 store 类参数(`registry` 到 `categories`)
替换成一个 `stores: Stores`,**保留** `stream`/`preview_contexts`/
`preview_commands`/`ide_bridge`/`in_flight`/`draining`/`shutdown_signal` 这
7 个(它们不是 `Stores` 管的东西:前三个是 `serve` 内部现造的,后三个是运行
时状态,不是存储句柄)。新签名:

```rust
async fn handle_conn(
    stream: UnixStream,
    stores: Stores,
    preview_contexts: Arc<PreviewContextStore>,
    preview_commands: Arc<crate::preview_commands::PreviewCommandBus>,
    ide_bridge: Arc<IdeBridgeRegistry>,
    in_flight: crate::task_poller::InFlight,
    draining: Arc<AtomicBool>,
    shutdown_signal: Arc<Notify>,
) -> Result<()> {
```

在函数体最开头(`loop {` 之前)同样解构:

```rust
    let Stores {
        registry,
        projects,
        bookmarks,
        code_health,
        transcripts,
        session_summaries,
        backfill_registry,
        todos,
        categories,
    } = stores;
```

（函数体里 `match req { ... }` 那一大块引用 `todos`/`categories`/`transcripts`
等局部变量的代码**原样不动**,解构后变量名和原来位置参数名完全一致。）

- [ ] **Step 5: 改 `serve` 内部 `tokio::spawn(handle_conn(...))` 调用点**

把 `serve` 里 accept 循环内那段逐个 `.clone()` 再传位置参数的代码,改成克隆
一次 `Stores`(它整体 `#[derive(Clone)]`,克隆开销就是几个 `Arc::clone`):

```rust
                let stores = stores.clone();
                let preview_contexts = preview_contexts.clone();
                let preview_commands = preview_commands.clone();
                let ide_bridge = ide_bridge.clone();
                let in_flight = in_flight.clone();
                let draining = draining.clone();
                let shutdown_signal = shutdown_signal.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_conn(
                        stream,
                        stores,
                        preview_contexts,
                        preview_commands,
                        ide_bridge,
                        in_flight,
                        draining,
                        shutdown_signal,
                    )
                    .await
                    {
                        tracing::debug!(error = %e, "连接结束");
                    }
                });
```

注意 `serve` 函数体第 Step 3 步解构出的 `registry`/`todos`/`categories` 等
局部变量,在这里要重新组装回一个 `Stores` 实例(放在 accept 循环之前一次性
组装,循环里只 `.clone()` 这一个值):

```rust
    let stores = Stores {
        registry,
        projects,
        bookmarks,
        code_health,
        transcripts,
        session_summaries,
        backfill_registry,
        todos,
        categories,
    };
```

- [ ] **Step 6: 改所有调用点**

`crates/dozerd/src/main.rs` 里原来传 12 个位置参数给 `serve(...)` 的地方,
改成组装一个 `Stores { registry, projects, bookmarks, code_health,
transcripts, session_summaries, backfill_registry, todos, categories }`
再传:`serve(&socket, ide_lock_dir, stores, in_flight).await`。

`crates/dozer-mcp/tests/todo_tools.rs` 的 `start_daemon` 同样改:把原来
散落的 9 个 store 变量收进一个 `Stores { .. }` 字面量,再调
`dozerd::server::serve(&s, ide_lock_dir.path().to_path_buf(), stores,
dozerd::task_poller::new_in_flight())`。

Step 1 普查出的其余调用点(如果有)按同样方式改。

- [ ] **Step 7: Run full existing test suite to verify no behavior change**

Run: `cargo build -p dozerd -p dozer-mcp -p dozer-app 2>&1 | tail -60`
Expected: 编译通过(这一步是纯签名重构,不应该有任何逻辑分支需要改)。

Run: `cargo test -p dozerd -p dozer-mcp 2>&1 | tail -80`
Expected: 重构前跑过的测试全部依然 PASS(数量不变)。

- [ ] **Step 8: Commit**

```bash
git add crates/dozerd/src/server.rs crates/dozerd/src/main.rs crates/dozer-mcp/tests/todo_tools.rs
git commit -m "refactor(dozerd): bundle serve/handle_conn store params into Stores struct"
```

---

### Task 4: 把 `MemoryStore` 接入 `dozerd` 分发

**Files:**
- Modify: `crates/dozerd/src/server.rs`(`Stores` 加字段、`serve`/`handle_conn` 解构补一行、accept 循环克隆补一行、`main.rs` 组装补一行、匹配分支新增四条)
- Modify: `crates/dozerd/src/main.rs`(构造 `MemoryStore` 并塞进 `Stores`)
- Modify: `crates/dozer-mcp/tests/todo_tools.rs`(`start_daemon` 补一行)

**Interfaces:**
- Consumes: Task 1 的 `MemoryStore`,Task 2 的 `Request::{WriteMemory,ListMemories,GetMemory,DeleteMemory}`/`Reply::{Memories,MemoryDetail}`,Task 3 的 `Stores` 结构体。
- Produces:`dozerd` 现在能真正处理这四种请求;供 Task 5(`dozer-client`)对接。

- [ ] **Step 1: `Stores` 加 `memories` 字段,`serve`/`handle_conn` 的解构 + 重新组装各补一行**

`crates/dozerd/src/server.rs`:`Stores` 结构体加
`pub memories: std::sync::Arc<crate::memory::MemoryStore>,`;`serve` 和
`handle_conn` 函数体开头的解构语句、`serve` 里重新组装 `Stores { .. }` 字面量、
accept 循环里没有需要额外改的(`stores.clone()` 已经整体克隆,`memories`
随 `Stores` 一起被克隆,不用单独加一行)。

`crates/dozerd/src/main.rs`:构造 `MemoryStore::new(&db)?`(`db` 复用现有
`TodoStore`/`ProjectStore` 那个同一路径变量),塞进 `Stores { .. }` 字面量。

`crates/dozer-mcp/tests/todo_tools.rs` 的 `start_daemon`:加一行
`let memories = Arc::new(dozerd::memory::MemoryStore::new(&db).unwrap());`,
塞进 `Stores { .. }` 字面量。

- [ ] **Step 2: `handle_conn` 里新增四条匹配分支**

紧邻现有 `Request::GetTodoDetail { id } => { .. }` 分支之后插入(`todos`/
`categories` 等变量已经在作用域里,`memories` 是本步骤新引入的局部变量,
同样来自函数体开头的解构):

```rust
                        Request::ListMemories { project_id } => match memories.list(project_id) {
                            Ok(memories) => Reply::Memories { memories },
                            Err(e) => Reply::Error {
                                message: format!("列共享记忆失败: {e}"),
                            },
                        },
                        Request::WriteMemory {
                            project_id,
                            title,
                            kind,
                            description,
                            body,
                            actor,
                        } => match memories.write(project_id, &title, &kind, &description, &body, &actor) {
                            Ok(detail) => Reply::MemoryDetail { detail },
                            Err(e) => Reply::Error {
                                message: format!("写共享记忆失败: {e}"),
                            },
                        },
                        Request::GetMemory { project_id, id } => match memories.get(project_id, id) {
                            Ok(detail) => Reply::MemoryDetail { detail },
                            Err(e) => Reply::Error {
                                message: format!("查共享记忆失败: {e}"),
                            },
                        },
                        Request::DeleteMemory { project_id, id, actor } => {
                            match memories.delete(project_id, id, &actor) {
                                Ok(()) => Reply::Ok,
                                Err(e) => Reply::Error {
                                    message: format!("删共享记忆失败: {e}"),
                                },
                            }
                        }
```

- [ ] **Step 3: Run tests to verify existing suite still passes and compiles with new arms**

Run: `cargo build -p dozerd 2>&1 | tail -40`
Expected: 编译通过,`match req { .. }` 不再因为漏处理新 `Request` 变体报
"non-exhaustive match"。

Run: `cargo test -p dozerd -p dozer-mcp 2>&1 | tail -60`
Expected: 全部 PASS(还没有新测试专门覆盖这四条分支——集成测试留给
Task 6 通过 `dozer-mcp` 工具间接覆盖 List/Write/Get 三条;`Delete` 分支
留给 Task 9 UI 任务通过 `dozer-client` 的单测直接覆盖,见该任务)。

- [ ] **Step 4: Commit**

```bash
git add crates/dozerd/src/server.rs crates/dozerd/src/main.rs crates/dozer-mcp/tests/todo_tools.rs
git commit -m "feat(dozerd): wire MemoryStore into request dispatch"
```

---

### Task 5: `dozer-client::Client` 封装 + 单测

**Files:**
- Modify: `crates/dozer-client/src/lib.rs`

**Interfaces:**
- Consumes: Task 2 的 `Request`/`Reply` 变体。
- Produces:`Client::list_memories(&self, project_id: i64) -> Result<Vec<MemoryInfo>>`、`Client::write_memory(&self, project_id: i64, title: &str, kind: &str, description: &str, body: &str, actor: &str) -> Result<MemoryDetail>`、`Client::get_memory(&self, project_id: i64, id: i64) -> Result<MemoryDetail>`、`Client::delete_memory(&self, project_id: i64, id: i64, actor: &str) -> Result<()>`。供 Task 6(`dozer-mcp`)、Task 8/9(`dozer-app` UI)使用。

- [ ] **Step 1: 补四个方法(紧邻现有 `edit_todo_text` 之后)**

```rust
    pub async fn list_memories(&self, project_id: i64) -> Result<Vec<dozer_core::protocol::MemoryInfo>> {
        match self.roundtrip(&Request::ListMemories { project_id }).await? {
            Reply::Memories { memories } => Ok(memories),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn write_memory(
        &self,
        project_id: i64,
        title: &str,
        kind: &str,
        description: &str,
        body: &str,
        actor: &str,
    ) -> Result<dozer_core::protocol::MemoryDetail> {
        match self
            .roundtrip(&Request::WriteMemory {
                project_id,
                title: title.into(),
                kind: kind.into(),
                description: description.into(),
                body: body.into(),
                actor: actor.into(),
            })
            .await?
        {
            Reply::MemoryDetail { detail } => Ok(detail),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn get_memory(&self, project_id: i64, id: i64) -> Result<dozer_core::protocol::MemoryDetail> {
        match self.roundtrip(&Request::GetMemory { project_id, id }).await? {
            Reply::MemoryDetail { detail } => Ok(detail),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn delete_memory(&self, project_id: i64, id: i64, actor: &str) -> Result<()> {
        match self
            .roundtrip(&Request::DeleteMemory {
                project_id,
                id,
                actor: actor.into(),
            })
            .await?
        {
            Reply::Ok => Ok(()),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }
```

（`dozer-client` 现有的 `add_todo`/`list_todos` 等方法没有各自的单元测试
——它们的正确性由 `dozer-mcp` 侧的集成测试间接覆盖,`Client` 本身只是薄
封装。本任务的四个方法同理,不另写 `dozer-client` 层面的单测;Task 6/9
的集成测试是实际验证点。）

- [ ] **Step 2: Run build to verify it compiles**

Run: `cargo build -p dozer-client 2>&1 | tail -30`
Expected: 编译通过。

- [ ] **Step 3: Commit**

```bash
git add crates/dozer-client/src/lib.rs
git commit -m "feat(dozer-client): add memory read/write wrapper methods"
```

---

### Task 6: `dozer-mcp` 三个新工具 + 集成测试

**Files:**
- Modify: `crates/dozer-mcp/src/server.rs`
- Create: `crates/dozer-mcp/tests/memory_tools.rs`

**Interfaces:**
- Consumes: Task 5 的 `Client::{list_memories,write_memory,get_memory}`;`resolve_project_id` 改造(见 Step 1)。
- Produces: MCP 工具 `write_memory`/`list_memories`/`get_memory`(agent 可调)。

- [ ] **Step 1: 把 `resolve_project_id` 改成同时带出 `AgentKind`,更新两个既有调用点**

`crates/dozer-mcp/src/server.rs` 里 `resolve_project_id` 现在只返回
`project_id`;`write_memory` 需要额外知道调用方的 `AgentKind` 来填
`actor`,所以把它改名/改造成 `resolve_project_and_agent`,返回
`anyhow::Result<(i64, dozer_core::protocol::AgentKind)>`:

```rust
    /// session_id → (project_id, agent) 的解析逻辑。`agent` 用于
    /// `write_memory` 的 `actor` 归属;`list_todos`/`add_todo` 等既有
    /// 调用方不需要 `agent`,解构时用 `_` 丢弃即可。
    async fn resolve_project_and_agent(&self) -> anyhow::Result<(i64, dozer_core::protocol::AgentKind)> {
        let sessions = self
            .client
            .list()
            .await
            .map_err(|e| anyhow::anyhow!("连接 dozerd 失败: {e}"))?;
        let session = sessions
            .iter()
            .find(|s| s.id == self.session_id)
            .ok_or_else(|| anyhow::anyhow!("会话不存在于 dozerd"))?;
        let project_id = session
            .project_id
            .ok_or_else(|| anyhow::anyhow!("会话尚未归属任何项目"))?;
        Ok((project_id, session.agent))
    }
```

删除原来的 `resolve_project_id` 方法,把它原来的两个调用点
(`list_todos`/`add_todo` 方法体里)改成:

```rust
        let (project_id, _agent) = self
            .resolve_project_and_agent()
            .await
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
```

- [ ] **Step 2: 新增三个工具方法**

紧邻 `edit_todo_text` 之后插入:

```rust
    #[tool(
        description = "记忆优先读写这里,不要用你自己本地的记忆机制。按标题在项目内 upsert:标题已存在就更新,不存在就新建。写入前建议先调 list_memories 看看有没有同名条目可以更新,避免重复记忆。kind 建议用 user/feedback/project/reference 之一,但不强制。"
    )]
    pub async fn write_memory(
        &self,
        Parameters(WriteMemoryParams { title, body, kind, description }): Parameters<WriteMemoryParams>,
    ) -> Result<CallToolResult, McpError> {
        let (project_id, agent) = self
            .resolve_project_and_agent()
            .await
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        let detail = self
            .client
            .write_memory(project_id, &title, &kind, &description, &body, agent.as_str())
            .await
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        Ok(CallToolResult::structured(json!({
            "id": detail.id,
            "title": detail.title,
            "kind": detail.kind,
            "description": detail.description,
            "updated_by": detail.updated_by,
        })))
    }

    #[tool(description = "列出当前项目的共享记忆(标题/分类/摘要/最后更新方,不含正文)。读记忆前先调这个,别猜有没有同名条目。")]
    pub async fn list_memories(
        &self,
        Parameters(NoParams {}): Parameters<NoParams>,
    ) -> Result<CallToolResult, McpError> {
        let (project_id, _agent) = self
            .resolve_project_and_agent()
            .await
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        let memories = self
            .client
            .list_memories(project_id)
            .await
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        let value = json!(
            memories
                .into_iter()
                .map(|m| json!({
                    "id": m.id,
                    "title": m.title,
                    "kind": m.kind,
                    "description": m.description,
                    "updated_by": m.updated_by,
                }))
                .collect::<Vec<_>>()
        );
        Ok(CallToolResult::structured(json!({ "memories": value })))
    }

    #[tool(description = "查一条共享记忆的完整正文。title_or_id 可以传标题(和 list_memories 里看到的一致)或数字 id;传标题时会先内部查一遍 list_memories 做匹配。")]
    pub async fn get_memory(
        &self,
        Parameters(GetMemoryParams { title_or_id }): Parameters<GetMemoryParams>,
    ) -> Result<CallToolResult, McpError> {
        let (project_id, _agent) = self
            .resolve_project_and_agent()
            .await
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        let id = if let Ok(id) = title_or_id.parse::<i64>() {
            id
        } else {
            let memories = self
                .client
                .list_memories(project_id)
                .await
                .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
            memories
                .into_iter()
                .find(|m| m.title == title_or_id)
                .ok_or_else(|| McpError::internal_error(format!("未找到标题为 {title_or_id} 的记忆"), None))?
                .id
        };
        let detail = self
            .client
            .get_memory(project_id, id)
            .await
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        Ok(CallToolResult::structured(json!({
            "id": detail.id,
            "title": detail.title,
            "kind": detail.kind,
            "description": detail.description,
            "body": detail.body,
            "updated_by": detail.updated_by,
            "history": detail.history.iter().map(|h| json!({
                "changed_ms": h.changed_ms,
                "changed_by": h.changed_by,
                "change_kind": h.change_kind,
            })).collect::<Vec<_>>(),
        })))
    }
```

在文件顶部现有 `EditTodoTextParams` 定义之后加三个入参结构体:

```rust
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct WriteMemoryParams {
    pub title: String,
    pub body: String,
    pub kind: String,
    pub description: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GetMemoryParams {
    pub title_or_id: String,
}
```

（`ListMemories` 复用现有 `NoParams`,不用新建。）

- [ ] **Step 3: Write the failing integration test**

创建 `crates/dozer-mcp/tests/memory_tools.rs`(照抄
`crates/dozer-mcp/tests/todo_tools.rs` 的 `start_daemon`/`temp_sock`/
`CleanupGuard` 那套,`Stores` 要包含 Task 4 加的 `memories` 字段):

```rust
use dozer_client::Client;
use dozer_mcp::server::{DozerMcpServer, GetMemoryParams, NoParams, WriteMemoryParams};
use rmcp::handler::server::wrapper::Parameters;
use std::sync::Arc;
use std::time::Duration;

fn temp_sock() -> std::path::PathBuf {
    let id = uuid::Uuid::new_v4();
    std::path::PathBuf::from(format!("/tmp/dz-mcp-mem-{}.sock", &id.to_string()[..8]))
}

struct CleanupGuard(std::path::PathBuf);
impl Drop for CleanupGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

async fn start_daemon() -> (std::path::PathBuf, CleanupGuard) {
    let sock = temp_sock();
    let db = std::path::PathBuf::from(format!("/tmp/dz-mcp-mem-{}.db", uuid::Uuid::new_v4()));
    let stores = dozerd::server::Stores {
        registry: Arc::new(dozerd::registry::SessionRegistry::new()),
        projects: Arc::new(dozerd::projects::ProjectStore::new(&db).unwrap()),
        bookmarks: Arc::new(dozerd::bookmarks::BookmarkStore::new(&db).unwrap()),
        code_health: Arc::new(dozerd::code_health::CodeHealthStore::new(&db).unwrap()),
        transcripts: Arc::new(dozerd::transcripts::TranscriptStore::open(&db).unwrap()),
        session_summaries: Arc::new(dozerd::session_summary::SessionSummaryStore::open(&db).unwrap()),
        backfill_registry: Arc::new(dozerd::session_summary_backfill::BackfillRegistry::new()),
        todos: Arc::new(dozerd::todo::TodoStore::new(&db).unwrap()),
        categories: Arc::new(dozerd::todo_category::CategoryStore::new(&db).unwrap()),
        memories: Arc::new(dozerd::memory::MemoryStore::new(&db).unwrap()),
    };
    let ide_lock_dir = tempfile::tempdir().expect("ide_lock_dir tempdir");
    let s = sock.clone();
    tokio::spawn(async move {
        dozerd::server::serve(
            &s,
            ide_lock_dir.path().to_path_buf(),
            stores,
            dozerd::task_poller::new_in_flight(),
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    (sock.clone(), CleanupGuard(sock))
}

/// 建一个挂到 `project_id` 上的会话,返回 `session_id`——照抄
/// `crates/dozer-mcp/tests/todo_tools.rs::session_with_project` 的既有
/// 写法:`Client::create` 的 `project_id` 参数是裸 `i64`(不是
/// `Option<i64>`),不需要真的先建一个 `ProjectStore` 里的项目记录,
/// `SessionRegistry::create` 不校验这个 id 是否真实存在。
async fn session_with_project(sock: &std::path::Path, project_id: i64) -> String {
    let client = Client::new(sock.to_path_buf());
    let session = client
        .create("mem-test", "/bin/sh", &["-c".into(), "cat".into()], "/tmp", 80, 24, project_id)
        .await
        .expect("建会话");
    session.id
}

#[tokio::test]
async fn write_then_list_then_get_roundtrip() {
    let (sock, _guard) = start_daemon().await;
    let session_id = session_with_project(&sock, 1).await;
    let server = DozerMcpServer::new(Client::new(sock), session_id);

    let write_result = server
        .write_memory(Parameters(WriteMemoryParams {
            title: "用户偏好".into(),
            body: "喜欢简短回复".into(),
            kind: "user".into(),
            description: "沟通风格".into(),
        }))
        .await
        .expect("write_memory 应该成功");
    assert!(write_result.structured_content.is_some());

    let list_result = server
        .list_memories(Parameters(NoParams {}))
        .await
        .expect("list_memories 应该成功");
    let list_json = list_result.structured_content.expect("应有结构化结果");
    assert_eq!(list_json["memories"].as_array().unwrap().len(), 1);
    assert_eq!(list_json["memories"][0]["title"], "用户偏好");

    let get_by_title = server
        .get_memory(Parameters(GetMemoryParams {
            title_or_id: "用户偏好".into(),
        }))
        .await
        .expect("get_memory 按标题应该成功");
    let get_json = get_by_title.structured_content.expect("应有结构化结果");
    assert_eq!(get_json["body"], "喜欢简短回复");
}

#[tokio::test]
async fn write_memory_twice_same_title_updates_not_duplicates() {
    let (sock, _guard) = start_daemon().await;
    let session_id = session_with_project(&sock, 1).await;
    let server = DozerMcpServer::new(Client::new(sock), session_id);

    for body in ["v1", "v2"] {
        server
            .write_memory(Parameters(WriteMemoryParams {
                title: "同名记忆".into(),
                body: body.into(),
                kind: "project".into(),
                description: "".into(),
            }))
            .await
            .expect("write_memory 应该成功");
    }

    let list_result = server
        .list_memories(Parameters(NoParams {}))
        .await
        .expect("list_memories 应该成功");
    let list_json = list_result.structured_content.expect("应有结构化结果");
    assert_eq!(list_json["memories"].as_array().unwrap().len(), 1, "同名应该是更新不是新增");
}

#[tokio::test]
async fn tools_error_when_session_id_unresolvable() {
    // `Client::create` 的 `project_id` 是裸 `i64`,没有"建会话但不挂项目"
    // 这条路径可走(`SessionInfo.project_id: Option<i64>` 里的 `None` 只
    // 对应迁移期孤儿会话,见 `protocol.rs` 该字段文档,不是本 API 能构造
    // 的状态)。改用一个从未创建过的 `session_id`,命中
    // `resolve_project_and_agent` 里 `.find(|s| s.id == self.session_id)`
    // 返回 `None` 的分支,同样能验证"解析失败时三个工具都报错、不 panic"
    // 这条 Review Focus。
    let (sock, _guard) = start_daemon().await;
    let server = DozerMcpServer::new(Client::new(sock), "does-not-exist".into());

    let err = server
        .list_memories(Parameters(NoParams {}))
        .await
        .expect_err("session_id 不存在时 list_memories 应该报错");
    assert!(format!("{err}").contains("不存在于 dozerd"));
}
```

（`session_with_project`/`Client::create` 的签名已对照
`crates/dozer-client/src/lib.rs:64` 和 `crates/dozer-mcp/tests/
todo_tools.rs` 的既有用法核实,直接照抄即可。）

- [ ] **Step 4: Run test to verify it fails, then implement, then verify it passes**

Run: `cargo test -p dozer-mcp --test memory_tools 2>&1 | tail -60`
先确认在 Step 1/2 代码写完前会编译失败(方法/类型不存在),写完后:
Expected: 3 个测试全部 PASS。

Run: `cargo test -p dozer-mcp 2>&1 | tail -60`(含 `todo_tools.rs` 等既有集成测试)
Expected: 全部 PASS,`resolve_project_id` 改名没有破坏既有 Todo 工具行为。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-mcp/src/server.rs crates/dozer-mcp/tests/memory_tools.rs
git commit -m "feat(dozer-mcp): add write_memory/list_memories/get_memory tools"
```

---

### Task 7: 下线旧的 `discover_memory`/"Agent 记忆"链接区

**Files:**
- Modify: `crates/dozer-app/src/extensions/project/links.rs`
- Modify: `crates/dozer-app/src/extensions/project/view.rs`
- Modify: `crates/dozer-app/src/extensions/project/scaffold.rs`
- Modify: `crates/dozer-app/src/extensions/project.rs`
- Modify: `crates/dozer-app/src/app/view.rs`
- Modify: `crates/dozer-app/src/app/update.rs`

**Interfaces:**
- Produces: 干净的基线(旧 Memory 链接功能完全移除,`cargo build`/`cargo clippy` 零残留死代码),供 Task 8 在同一个位置插入新 UI。

- [ ] **Step 1: `links.rs` 删除 Memory 相关代码**

删除:`LinkTarget::Memory` 枚举变体(`LinkTarget` 只剩 `Docs`)、
`MEMORY_FILE_NAMES`/`MEMORY_DIR_NAMES` 常量、`discover_memory`/
`discover_memory_in` 两个函数、`LinksState.memory` 字段、
`LinksState::list`/`list_mut` 里 `LinkTarget::Memory => ..` 的匹配分支
(改成单分支穷尽 `LinkTarget::Docs`)、`load_or_discover`/
`merge_rediscovered` 里所有 `memory: discover_memory(repo)`/对 `state.memory`
的操作。

`load_or_discover` 改成:

```rust
pub fn load_or_discover(repo: &Path) -> LinksState {
    if let Some(state) = load(repo) {
        return state;
    }
    let state = LinksState {
        docs: discover_docs(repo),
        dismissed: Vec::new(),
    };
    let _ = save(repo, &state);
    state
}
```

`merge_rediscovered` 只保留 `discover_docs` 那一段循环,删掉
`discover_memory` 那一段。

`crates/dozer-app/src/extensions/project.rs` 里(第 823 行附近)删掉
`assert!(links::load(repo.path()).unwrap().memory.is_empty());` 这行断言
(字段已经不存在)。

- [ ] **Step 2: `view.rs` 删除 Memory 工具栏按钮与渲染分支**

`ProjectPaneHover` 结构体删掉 `pub memory_add: f32,` 字段;
`ProjectToolbarTarget` 枚举删掉 `Memory` 变体(只剩 `Docs`/`Remote`);
`links_section` 函数体里 `LinkTarget::Memory => "添加记忆",` 那条匹配分支
删除(改成单分支穷尽 `LinkTarget::Docs => "添加文档"`);主 `view` 函数里
第二个 `content.push(links_section(... Memory ...))` 调用块(原来在
"项目文档"那块之后)整段删除——**这一块会在 Task 8 被替换成新的共享记忆
UI 调用**,本任务先删干净,不留占位。

- [ ] **Step 3: `scaffold.rs` 更新步骤描述**

搜索 `scaffold.rs` 里提到"项目文档与 Agent 记忆"的注释/步骤标签,改成
只提"项目文档"(该同步步骤现在只做 `discover_docs`/`merge_rediscovered`
的文档部分)。

- [ ] **Step 4: `app/view.rs`/`app/update.rs` 清理 hover 转发**

`crates/dozer-app/src/app/view.rs:922` 删掉
`memory_add: app.hover_progress(HoverId::ProjectMemoryAdd),` 这一行(连带
`ProjectPaneHover { .. }` 字面量里对应字段)。

`crates/dozer-app/src/app/update.rs:2444` 删掉
`project::ProjectToolbarTarget::Memory => HoverId::ProjectMemoryAdd,`
这一条匹配分支。

搜索 `HoverId::ProjectMemoryAdd` 枚举变体本身的定义处(`app/state.rs` 或
类似文件),一并删除该变体。

- [ ] **Step 5: Run build + clippy to verify no dead code left**

Run: `cargo build -p dozer-app 2>&1 | tail -60`
Expected: 编译通过,无 "unused" / "variant is never constructed" 警告。

Run: `cargo clippy -p dozer-app --all-targets 2>&1 | tail -60`
Expected: 无新增 warning。

Run: `cargo test -p dozer-app project:: 2>&1 | tail -60`
Expected: `extensions::project` 模块下的既有测试(除已删除的那条断言外)
全部 PASS。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/extensions/project.rs crates/dozer-app/src/extensions/project/links.rs crates/dozer-app/src/extensions/project/view.rs crates/dozer-app/src/extensions/project/scaffold.rs crates/dozer-app/src/app/view.rs crates/dozer-app/src/app/update.rs
git commit -m "refactor(project-panel): remove legacy discover_memory link section"
```

---

### Task 8: 项目面板新增"共享记忆"列表 + 新建

**Files:**
- Modify: `crates/dozer-app/src/extensions/project.rs`(`Message` 枚举、`WorkspaceState` 字段)
- Modify: `crates/dozer-app/src/extensions/project/update.rs`
- Modify: `crates/dozer-app/src/extensions/project/view.rs`
- Modify: `crates/dozer-app/src/app/update.rs`(把打开项目时的 `request_memories_refresh` 首次拉取接进项目切换/scaffold 完成流程)

**Interfaces:**
- Consumes: Task 5 的 `Client::list_memories`/`write_memory`。
- Produces: 列表可见、可新建;详情/编辑/删除/历史留给 Task 9。

- [ ] **Step 1: `WorkspaceState` 加字段**

在 `crates/dozer-app/src/extensions/project.rs` 的 `WorkspaceState` 结构体
(第 78-112 行那个)里加:

```rust
    /// 共享记忆列表(不含正文),`RequestMemoriesRefresh` 拉取后写入。
    memories: Vec<dozer_core::protocol::MemoryInfo>,
    /// 新建记忆的表单草稿(`None` = 表单未打开)。
    memory_create_draft: Option<MemoryDraft>,
```

在 `WorkspaceState::new` 里给这两个字段初始化默认值(`Vec::new()`/`None`)
——找到现有 `impl WorkspaceState { pub fn new(..) -> Self { .. } }` 的
字段列表,按现有风格补上这两行。

在 `project.rs` 里(`Message` 枚举附近)加一个小结构体:

```rust
/// 新建共享记忆的表单草稿。四个字段分别对应 `write_memory` 的入参,
/// 用具名结构体而不是四个散落的 `String` 局部变量,避免以后传参时顺序
/// 传错(`title`/`kind`/`description` 三个 `String` 相邻,属于 CLAUDE.md
/// 参数结构体裁决要规避的形状)。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MemoryDraft {
    pub title: String,
    pub kind: String,
    pub description: String,
    pub body: String,
}
```

- [ ] **Step 2: `Message` 枚举加变体**

在 `extensions/project.rs` 的 `Message` 枚举里(`ScaffoldPopupClose` 之后)
加:

```rust
    /// 共享记忆列表拉取结果。
    MemoriesLoaded(Vec<dozer_core::protocol::MemoryInfo>),
    /// 打开"新建记忆"表单。
    MemoryCreateStart,
    /// 新建表单四个字段的输入变化。
    MemoryCreateTitleInput(String),
    MemoryCreateKindInput(String),
    MemoryCreateDescriptionInput(String),
    MemoryCreateBodyInput(String),
    /// 提交新建表单。
    MemoryCreateSubmit,
    /// 取消新建。
    MemoryCreateCancel,
    /// 一次写操作(新建/编辑,Task 9 也会复用这个变体)完成后的结果——
    /// 成功与否都触发一次列表刷新,同 Todo 面板 `Message::Mutated` 的既有
    /// 先例。
    MemoryMutated(Result<(), String>),
```

- [ ] **Step 3: `update.rs` 处理新消息**

在 `crates/dozer-app/src/extensions/project/update.rs` 里加(紧邻
`ScaffoldPopupClose` 分支之后):

```rust
        Message::MemoriesLoaded(memories) => ws_state.memories = memories,
        Message::MemoryCreateStart => {
            ws_state.memory_create_draft = Some(MemoryDraft::default());
        }
        Message::MemoryCreateCancel => {
            ws_state.memory_create_draft = None;
        }
        Message::MemoryCreateTitleInput(v) => {
            if let Some(d) = &mut ws_state.memory_create_draft {
                d.title = v;
            }
        }
        Message::MemoryCreateKindInput(v) => {
            if let Some(d) = &mut ws_state.memory_create_draft {
                d.kind = v;
            }
        }
        Message::MemoryCreateDescriptionInput(v) => {
            if let Some(d) = &mut ws_state.memory_create_draft {
                d.description = v;
            }
        }
        Message::MemoryCreateBodyInput(v) => {
            if let Some(d) = &mut ws_state.memory_create_draft {
                d.body = v;
            }
        }
        Message::MemoryCreateSubmit => {
            let Some(draft) = ws_state.memory_create_draft.take() else {
                return;
            };
            let client = client.clone();
            let project_id_owned = project_id;
            handle.spawn(async move {
                let res = client
                    .write_memory(
                        project_id_owned,
                        &draft.title,
                        &draft.kind,
                        &draft.description,
                        &draft.body,
                        "user",
                    )
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string());
                emit(Message::MemoryMutated(res));
            });
        }
        Message::MemoryMutated(res) => {
            if let Err(e) = res {
                ws_state.error = Some(format!("保存记忆失败: {e}"));
            } else {
                ws_state.error = None;
            }
            let client = client.clone();
            let project_id_owned = project_id;
            let emit = std::sync::Arc::new(emit);
            let emit2 = emit.clone();
            handle.spawn(async move {
                let memories = client.list_memories(project_id_owned).await.unwrap_or_default();
                emit2(Message::MemoriesLoaded(memories));
            });
        }
```

（`emit` 参数本身是 `impl Fn(Message) + Send + 'static`,按值传入
`update`;这里既要在 `MemoryCreateSubmit` 分支里 move 进第一个
`handle.spawn`,又要在 `MemoryMutated` 分支里再 move 进第二个
`handle.spawn`——两个分支各自拿到的是同一次 `update()` 调用传入的 `emit`
所有权,不冲突,`Arc::new(emit)` 只在 `MemoryMutated` 分支内部用于自己那
一次 clone,不用把整个函数签名改成 `Arc<..>`。）

- [ ] **Step 4: `view.rs` 渲染列表 + 新建表单,替换 Task 7 删掉的位置**

在 `crates/dozer-app/src/extensions/project/view.rs` 里(Task 7 删掉
Memory `links_section` 调用的原位置)加一个新函数 + 调用:

```rust
fn memory_section<'a>(
    memories: &'a [dozer_core::protocol::MemoryInfo],
    draft: Option<&'a MemoryDraft>,
    now_ms: u64,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let mut col = column![].spacing(6).width(Length::Fill);
    col = col.push(
        row![
            text("共享记忆")
                .size(byteui::theme::font::body())
                .color(byteui::theme::color::current().body),
            iced_widget::Space::new().width(Length::Fill),
            button(text("+").size(byteui::theme::font::body()))
                .on_press(Message::MemoryCreateStart),
        ]
        .align_y(iced_widget::core::Alignment::Center),
    );

    if let Some(draft) = draft {
        col = col.push(
            column![
                byteui::form::input_text::view("标题", &draft.title, Message::MemoryCreateTitleInput, None),
                byteui::form::input_text::view("分类(user/feedback/project/reference)", &draft.kind, Message::MemoryCreateKindInput, None),
                byteui::form::input_text::view("一句话摘要", &draft.description, Message::MemoryCreateDescriptionInput, None),
                byteui::form::input_text::view("正文", &draft.body, Message::MemoryCreateBodyInput, None),
                row![
                    button(text("保存")).on_press(Message::MemoryCreateSubmit),
                    button(text("取消")).on_press(Message::MemoryCreateCancel),
                ]
                .spacing(8),
            ]
            .spacing(4),
        );
    }

    for m in memories {
        let updated_line = format!(
            "上次由 {} 于 {} 更新",
            m.updated_by,
            crate::workspace::relative_time_text(m.updated_ms, now_ms)
        );
        col = col.push(
            button(
                column![
                    row![
                        text(m.title.clone()).size(byteui::theme::font::body()).color(byteui::theme::color::current().body),
                        text(format!(" [{}]", m.kind)).size(byteui::theme::font::caption()).color(byteui::theme::color::current().dim),
                    ],
                    text(m.description.clone()).size(byteui::theme::font::caption()).color(byteui::theme::color::current().dim),
                    text(updated_line).size(byteui::theme::font::caption_sm()).color(byteui::theme::color::current().dim),
                ]
                .spacing(2),
            )
            .on_press(Message::MemoryDetailOpen(m.id))
            .style(move |_t, _s| iced_widget::button::Style {
                text_color: byteui::theme::color::current().body,
                ..iced_widget::button::Style::default()
            }),
        );
    }

    col.into()
}
```

（`Message::MemoryDetailOpen` 是 Task 9 才会加的变体——本任务先把
`on_press` 写出来,**Task 9 的第一步会补这个变体**,本任务结束时
`cargo build` 应该会因为它不存在而失败;如果你严格按 TDD 一个任务一个
任务做,本任务的 `on_press` 先改成 `Message::Noop` 占位,Task 9 开工第
一步再把它换回 `Message::MemoryDetailOpen(m.id)`——两种做法二选一,选
了哪种在 commit message 里注明,避免下一个任务的执行者误判基线状态。）

在主 `view` 函数里(Task 7 删除旧调用的同一个位置)加(`now_ms` 用
`std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
.map(|d| d.as_millis() as u64).unwrap_or(0)`——如果 `project::view` 所在的
主 `view` 函数已经有一个现成的 `now_ms` 变量在往其他子视图传,直接复用那
一个,不要重复计算):

```rust
    content = content.push(memory_section(&ws_state.memories, ws_state.memory_create_draft.as_ref(), now_ms));
```

- [ ] **Step 5: 打开项目时首次拉取记忆列表**

找到现有项目打开/切换流程里已经会拉取 Todo 列表首屏数据的地方(搜索
`request_todos_refresh` 在 `app/update.rs` 里的调用点作为参照),在同一个
时机加一次:

```rust
    let client_owned = client.clone();
    let emit = /* 复用同一个 emit 闭包,包一层转成 project::Message */;
    handle.spawn(async move {
        let memories = client_owned.list_memories(project_id).await.unwrap_or_default();
        emit(Message::Project(project::Message::MemoriesLoaded(memories)));
    });
```

具体嵌入点和现有 `emit` 闭包的构造方式,以 `request_todos_refresh` 在
`app/update.rs` 里被调用的那个位置为准照抄结构,不要另起一套。

- [ ] **Step 6: Run build to verify it compiles(先用 `Message::Noop` 占位版本)**

Run: `cargo build -p dozer-app 2>&1 | tail -60`
Expected: 编译通过。

- [ ] **Step 7: 人工验收(GUI)**

启动 `cargo run -p dozer-app`,打开一个项目,确认:
- "共享记忆"区域出现在原来"Agent 记忆"链接区的位置,初始为空列表。
- 点"+"出现新建表单,填标题/分类/摘要/正文后点"保存",列表刷新出现新行。
- 用 `dozer-mcp` 的 `write_memory` 工具(可以直接在一个连了 `dozer-mcp`
  的 agent 会话里调用)写一条,确认这个项目面板刷新(或重新打开面板)后
  能看到 agent 写的那条。

- [ ] **Step 8: Commit**

```bash
git add crates/dozer-app/src/extensions/project.rs crates/dozer-app/src/extensions/project/update.rs crates/dozer-app/src/extensions/project/view.rs crates/dozer-app/src/app/update.rs
git commit -m "feat(project-panel): add shared memory list and create form"
```

---

### Task 9: 详情/编辑/删除 + 历史时间线

**Files:**
- Modify: `crates/dozer-app/src/extensions/project.rs`(`Message`/`WorkspaceState`)
- Modify: `crates/dozer-app/src/extensions/project/update.rs`
- Modify: `crates/dozer-app/src/extensions/project/view.rs`

**Interfaces:**
- Consumes: Task 5 的 `Client::{get_memory,write_memory,delete_memory}`。
- Produces: 完整闭环(列表 → 详情 → 编辑保存 / 删除 → 历史时间线)。

- [ ] **Step 1: `WorkspaceState` 加详情/编辑/删除确认状态**

```rust
    /// 当前打开的记忆详情(`None` = 未打开详情面板)。
    memory_detail: Option<dozer_core::protocol::MemoryDetail>,
    /// 详情页的编辑草稿(`None` = 只读展示态,未进入编辑)。
    memory_edit_draft: Option<MemoryDraft>,
    /// 待删除确认的记忆 id(`None` = 未在确认删除)。同 `delete_pending`
    /// 的既有先例(项目删除的二次确认),不用弹独立模态框。
    memory_delete_pending: Option<i64>,
```

`Message` 枚举加:

```rust
    /// 点列表行,打开详情(先设 loading 态,再异步拉正文)。
    MemoryDetailOpen(i64),
    /// 详情拉取完成。
    MemoryDetailLoaded(dozer_core::protocol::MemoryDetail),
    /// 关闭详情面板。
    MemoryDetailClose,
    /// 进入编辑态(用当前详情内容预填草稿)。
    MemoryEditStart,
    MemoryEditTitleInput(String),
    MemoryEditKindInput(String),
    MemoryEditDescriptionInput(String),
    MemoryEditBodyInput(String),
    MemoryEditSubmit,
    MemoryEditCancel,
    /// 点删除按钮,进入二次确认态。
    MemoryDeleteRequest(i64),
    MemoryDeleteCancel,
    MemoryDeleteConfirm,
```

- [ ] **Step 2: `view.rs` 把 `memory_section` 里行按钮的 `on_press` 换回 `MemoryDetailOpen`**

如果 Task 8 用了 `Message::Noop` 占位,这里改回
`Message::MemoryDetailOpen(m.id)`。

- [ ] **Step 3: `update.rs` 处理详情/编辑/删除**

```rust
        Message::MemoryDetailOpen(id) => {
            ws_state.memory_detail = None;
            ws_state.memory_edit_draft = None;
            let client = client.clone();
            let project_id_owned = project_id;
            handle.spawn(async move {
                if let Ok(detail) = client.get_memory(project_id_owned, id).await {
                    emit(Message::MemoryDetailLoaded(detail));
                }
            });
        }
        Message::MemoryDetailLoaded(detail) => {
            ws_state.memory_detail = Some(detail);
        }
        Message::MemoryDetailClose => {
            ws_state.memory_detail = None;
            ws_state.memory_edit_draft = None;
            ws_state.memory_delete_pending = None;
        }
        Message::MemoryEditStart => {
            if let Some(detail) = &ws_state.memory_detail {
                ws_state.memory_edit_draft = Some(MemoryDraft {
                    title: detail.title.clone(),
                    kind: detail.kind.clone(),
                    description: detail.description.clone(),
                    body: detail.body.clone(),
                });
            }
        }
        Message::MemoryEditCancel => {
            ws_state.memory_edit_draft = None;
        }
        Message::MemoryEditTitleInput(v) => {
            if let Some(d) = &mut ws_state.memory_edit_draft {
                d.title = v;
            }
        }
        Message::MemoryEditKindInput(v) => {
            if let Some(d) = &mut ws_state.memory_edit_draft {
                d.kind = v;
            }
        }
        Message::MemoryEditDescriptionInput(v) => {
            if let Some(d) = &mut ws_state.memory_edit_draft {
                d.description = v;
            }
        }
        Message::MemoryEditBodyInput(v) => {
            if let Some(d) = &mut ws_state.memory_edit_draft {
                d.body = v;
            }
        }
        Message::MemoryEditSubmit => {
            let Some(draft) = ws_state.memory_edit_draft.take() else {
                return;
            };
            let client = client.clone();
            let project_id_owned = project_id;
            handle.spawn(async move {
                let res = client
                    .write_memory(project_id_owned, &draft.title, &draft.kind, &draft.description, &draft.body, "user")
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string());
                emit(Message::MemoryMutated(res));
            });
        }
        Message::MemoryDeleteRequest(id) => {
            ws_state.memory_delete_pending = Some(id);
        }
        Message::MemoryDeleteCancel => {
            ws_state.memory_delete_pending = None;
        }
        Message::MemoryDeleteConfirm => {
            let Some(id) = ws_state.memory_delete_pending.take() else {
                return;
            };
            ws_state.memory_detail = None;
            let client = client.clone();
            let project_id_owned = project_id;
            handle.spawn(async move {
                let res = client
                    .delete_memory(project_id_owned, id, "user")
                    .await
                    .map_err(|e| e.to_string());
                emit(Message::MemoryMutated(res));
            });
        }
```

（`Message::MemoryMutated` 已经在 Task 8 实现——删除/编辑成功后都会
触发同一条"刷新列表"逻辑,不用为删除单独写一条。）

- [ ] **Step 4: `view.rs` 渲染详情面板(含历史时间线)**

在 `view.rs` 里加:

```rust
fn memory_detail_view<'a>(
    detail: &'a dozer_core::protocol::MemoryDetail,
    edit_draft: Option<&'a MemoryDraft>,
    delete_pending: bool,
    now_ms: u64,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let mut col = column![].spacing(8).padding(8).width(Length::Fill);

    col = col.push(
        row![
            button(text("← 返回")).on_press(Message::MemoryDetailClose),
            iced_widget::Space::new().width(Length::Fill),
            button(text("编辑")).on_press(Message::MemoryEditStart),
            button(text("删除")).on_press(Message::MemoryDeleteRequest(detail.id)),
        ]
        .spacing(8)
        .align_y(iced_widget::core::Alignment::Center),
    );

    if delete_pending {
        col = col.push(
            row![
                text("确定删除这条记忆?历史记录会保留可查。")
                    .color(byteui::theme::color::current().body),
                button(text("确认删除")).on_press(Message::MemoryDeleteConfirm),
                button(text("取消")).on_press(Message::MemoryDeleteCancel),
            ]
            .spacing(8)
            .align_y(iced_widget::core::Alignment::Center),
        );
    }

    if let Some(draft) = edit_draft {
        col = col.push(
            column![
                byteui::form::input_text::view("标题", &draft.title, Message::MemoryEditTitleInput, None),
                byteui::form::input_text::view("分类", &draft.kind, Message::MemoryEditKindInput, None),
                byteui::form::input_text::view("一句话摘要", &draft.description, Message::MemoryEditDescriptionInput, None),
                byteui::form::input_text::view("正文", &draft.body, Message::MemoryEditBodyInput, None),
                row![
                    button(text("保存")).on_press(Message::MemoryEditSubmit),
                    button(text("取消")).on_press(Message::MemoryEditCancel),
                ]
                .spacing(8),
            ]
            .spacing(4),
        );
    } else {
        col = col.push(text(detail.title.clone()).size(byteui::theme::font::title()));
        col = col.push(text(format!("[{}] {}", detail.kind, detail.description)).color(byteui::theme::color::current().dim));
        col = col.push(text(detail.body.clone()).color(byteui::theme::color::current().body));
    }

    col = col.push(text("历史").size(byteui::theme::font::body()).color(byteui::theme::color::current().body));
    for h in &detail.history {
        let when = crate::workspace::relative_time_text(h.changed_ms, now_ms);
        col = col.push(
            row![
                text(format!("{when} · {} · {}", h.changed_by, h.change_kind))
                    .size(byteui::theme::font::caption())
                    .color(byteui::theme::color::current().dim),
            ],
        );
    }

    col.into()
}
```

把 Task 8 Step 4 写的
`content = content.push(memory_section(&ws_state.memories,
ws_state.memory_create_draft.as_ref(), now_ms));` 这一行改成条件渲染:

```rust
    content = content.push(match &ws_state.memory_detail {
        Some(detail) => memory_detail_view(
            detail,
            ws_state.memory_edit_draft.as_ref(),
            ws_state.memory_delete_pending.is_some(),
            now_ms,
        ),
        None => memory_section(&ws_state.memories, ws_state.memory_create_draft.as_ref(), now_ms),
    });
```

`byteui::theme::font::
title()`/`crate::workspace::relative_time_text` 均已在
`crates/byteui/src/theme/font.rs:68` 和 `crates/dozer-app/src/workspace/
view.rs:1728`(经 `crate::workspace::relative_time_text` 重导出,见
`extensions/conversations.rs:17` 的既有用法)核实存在,直接用,不用再
自行确认。

- [ ] **Step 5: Run build**

Run: `cargo build -p dozer-app 2>&1 | tail -60`
Expected: 编译通过。

Run: `cargo clippy -p dozer-app --all-targets 2>&1 | tail -60`
Expected: 无新增 warning。

- [ ] **Step 6: 人工验收(GUI),对着 Review Focus 逐条过一遍**

- 新建一条记忆,点进详情能看到刚写的内容和一条 `created` 历史。
- 编辑保存后,详情正文更新,历史多一条 `updated`,且旧的那条历史仍在
  (滚动/展开能看到两条)。
- 删除一条记忆,列表消失;（如果 UI 提供了"查看已删除记忆历史"的入口就
  验证,若本次 UI 没做这个入口,至少确认 `dozer-mcp`/`dozer-client` 层面
  `client.get_memory`/history 查询仍能查到——这条对着 Task 1 的单测复核
  即可,不强求 UI 一定要有专门的"已删除记忆"浏览页,spec 未要求)。
- 用两个不同项目各建一条同名记忆,确认互不影响、互不可见。
- 用 agent(调 `write_memory` MCP 工具)写一条,人工在 UI 里编辑保存,
  再让 agent 调 `get_memory` 确认能读到人工改过的最新内容。

- [ ] **Step 7: Commit**

```bash
git add crates/dozer-app/src/extensions/project.rs crates/dozer-app/src/extensions/project/update.rs crates/dozer-app/src/extensions/project/view.rs
git commit -m "feat(project-panel): add memory detail/edit/delete and history timeline"
```
