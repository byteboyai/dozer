# Todo 任务派发 agent + 会话关联 + 详情弹窗双向对话 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 Todo"派发"从"往活着的终端 PTY 写字节"改造成纯粹的 agent 指派记录,新增分类维度的自动轮询开关驱动 dozerd 后台无人值守 headless 处理任务,新增任务详情弹窗展示关联会话回合 + 支持人类回复触发继续处理,会话面板展示"关联任务"标签。

**Architecture:** `dozer-core::protocol` 新增/替换任务派发相关 `Request`/`Reply`;`dozerd` 新增 `assigned_agent`/`auto_poll_enabled`/`task_id` 三列(`todos`/`todo_categories`/`session_summaries`),新增 `process_task_headless`(headless one-shot 变体,允许真正调用工具)、`task_processor`(单任务处理编排)、`task_poller`(周期扫描,dozerd 第一个后台 scheduler)三个模块;`dozer-client` 加对应方法;`dozer-app` 的 `extensions/todo.rs` 把"派发到活着的 tab"UI 换成"选 agent 类型",新增详情弹窗(原生 iced 回合列表 + 回复框),`app.rs` 接入路由/焦点闸门/会话面板关联标签。

**Tech Stack:** Rust workspace;`rusqlite`;`tokio`(新增周期性 `tokio::time::interval`);`iced 0.14`。

**Spec:** `docs/superpowers/specs/2026-09-02-todo-agent-dispatch-session-design.md`

## Global Constraints

- **在独立分支上开发,不要直接提交到 `main`**(如 `feature/todo-agent-dispatch-session`)。全部 14 个 Task 完成、`cargo build/test/clippy/fmt` 全绿、Task 14 的手工 GUI 验收清单走完后,提请审阅;审阅通过后再合并回 `main`。这条独立于每个 Task 内部的逐步 commit——分支内可以正常按 Task 提交,只是不要把这些 commit 直接落在 `main` 上。
- 不做派发历史:`dispatch_session_id` 保持"只留最新一次"的覆盖语义。
- 不做"人类回复中止正在跑的 headless 进程"——无人值守执行的风险边界已在 brainstorming 阶段确认接受。
- 不新增"标记任务完成"的专用协议——复用 agent 已有的 `mcp__dozer__toggle_todo`。
- 不做会话面板"点击关联任务标签跳转回 Todo 面板"的深链——只展示文本标签。
- 完全替换旧的"派发到现有活着的 tab"UI/协议(`RecordTodoDispatch`/`todo_dispatch_to_existing`/`dispatch_todo_to_existing`),不保留双轨。
- Opencode/V8agent 在 headless 一次性调用里真正执行工具调用时是否需要额外的跳过权限参数,现有代码从未验证过(现有 `summarize_headless` 只用它们输出文字,不触发工具调用)——Task 6 里必须先手工实测,不能凭空假设参数名。
- 每个任务完成后运行 `cargo build`(涉及的 crate)+ 对应 `cargo test` + 提交一次 commit。全部任务完成后跑一次全 workspace 的 `cargo build && cargo clippy --all-targets && cargo fmt`。

---

## Task 1: 协议层——新增字段 + 替换/新增 Request/Reply

**Files:**
- Modify: `crates/dozer-core/src/protocol.rs`

**Interfaces:**
- Produces: `TodoInfo.assigned_agent: Option<AgentKind>`;`CategoryInfo.auto_poll_enabled: bool`;`SessionSummaryPayload.task_id: Option<i64>`;`Request::{AssignTodoAgent, SetCategoryAutoPoll, ProcessTodoNow, GetTodoDetail}`;`Reply::TodoDetail { info: TodoInfo, turns: Vec<TurnRecord> }`。这些字段名/类型是后续全部任务(dozerd 各 store、client、GUI)必须原样匹配的契约。

- [ ] **Step 1: `TodoInfo`/`CategoryInfo`/`SessionSummaryPayload` 加新字段**

`TodoInfo`(`protocol.rs:219-242`)在 `category_id` 字段后加:

```rust
    #[serde(default)]
    pub category_id: Option<i64>,
    /// 指派给哪个 agent 种类(纯记录,不触发执行;`None` = 未指派)。只有
    /// `AgentKind::label()` 有 headless 适配器的四种(Claude/Codebuddy/
    /// Opencode/V8agent)会出现在这里,选择器不出现其余三种。
    #[serde(default)]
    pub assigned_agent: Option<AgentKind>,
```

`CategoryInfo`(`protocol.rs:247-256`)在 `created_ms` 字段后加:

```rust
    pub created_ms: u64,
    /// 该分类下的任务是否开启自动轮询处理(dozerd 后台定时扫描触发
    /// headless 处理),默认关闭。
    #[serde(default)]
    pub auto_poll_enabled: bool,
```

`SessionSummaryPayload`(`protocol.rs:59-72`)在 `created_ts_ms` 字段后加:

```rust
    pub created_ts_ms: u64,
    /// 该会话是否由 Todo 任务处理产生,是则记该任务 `TodoInfo.id`。会话
    /// 面板据此展示"关联任务"标签。与交互式 PTY 会话无关时为 `None`。
    #[serde(default)]
    pub task_id: Option<i64>,
```

- [ ] **Step 2: 替换 `RecordTodoDispatch`,新增三个 `Request` 变体**

在 `Request` 枚举(`protocol.rs:293` 起)里把:

```rust
    /// 记录一次派发(指派到已有会话)。
    RecordTodoDispatch {
        id: i64,
        session_id: String,
    },
```

替换成:

```rust
    /// 指派任务给某个 agent 种类。纯记录,不触发任何执行、不写 PTY——
    /// dozerd 只更新 `assigned_agent`/`dispatch_at_ms`。
    AssignTodoAgent {
        id: i64,
        agent: AgentKind,
    },
    /// 打开/关闭某分类下任务的自动轮询处理。
    SetCategoryAutoPoll {
        id: i64,
        enabled: bool,
    },
    /// 立即触发一次该任务的 headless 处理,不等轮询。`human_reply` 为
    /// `None` 表示这次触发没有新增人类文本(指派后从未回复过就先点了
    /// "处理")。命中"该任务正在处理中"时应答 `Reply::Error`。
    ProcessTodoNow {
        id: i64,
        human_reply: Option<String>,
    },
    /// 详情弹窗打开时一次性拿任务信息 + 关联会话的完整回合列表。
    /// `dispatch_session_id` 为 `None`(从未处理过)时 `turns` 返回空数组。
    GetTodoDetail {
        id: i64,
    },
```

- [ ] **Step 3: 新增 `Reply::TodoDetail`**

在 `Reply` 枚举(`protocol.rs:646-651` 附近,`Todo { todo }` 之后)加:

```rust
    /// `GetTodoDetail` 应答。
    TodoDetail {
        info: TodoInfo,
        turns: Vec<TurnRecord>,
    },
```

- [ ] **Step 4: 修补因新增必填字段而编译不过的既有字面量**

`TodoInfo` 字面量(`protocol.rs` 测试区,原 `RecordTodoDispatch`/`Todo`/`Todos` 往返测试里,约 1400 行)加 `assigned_agent: None,`:

```rust
        let todo = TodoInfo {
            id: 1,
            project_id: 1,
            text: "写完 spec".into(),
            done: false,
            paused: false,
            rank: 0,
            created_ms: 1_700_000_000_000,
            completed_at_ms: None,
            plan_date: Some("08-10".into()),
            dispatch_session_id: None,
            dispatch_at_ms: None,
            category_id: None,
            assigned_agent: None,
        };
```

`CategoryInfo` 字面量(`category_protocol_types_roundtrip` 测试里)加 `auto_poll_enabled: false,`:

```rust
        let category = CategoryInfo {
            id: 10,
            project_id: 1,
            parent_id: Some(2),
            name: "前端".into(),
            rank: 0,
            created_ms: 1_700_000_000_000,
            auto_poll_enabled: false,
        };
```

`SessionSummaryPayload` 字面量(三处:`protocol.rs` 约 799 行 `conversations_with_summaries_reply_roundtrips`、约 1192 行 `get_session_summary_reply_roundtrips_with_payload`、约 1210 行 `get_session_summary_reply_roundtrips_with_no_conversation_id`)各加 `task_id: None,`。

- [ ] **Step 5: 新增/补充测试**

在 `#[cfg(test)] mod tests` 里追加:

```rust
    #[test]
    fn assign_todo_agent_and_todo_detail_roundtrip() {
        let req = Request::AssignTodoAgent {
            id: 1,
            agent: AgentKind::Claude,
        };
        let line = encode_line(&req);
        assert_eq!(decode_line::<Request>(&line).unwrap(), req);

        let req = Request::SetCategoryAutoPoll { id: 10, enabled: true };
        let line = encode_line(&req);
        assert_eq!(decode_line::<Request>(&line).unwrap(), req);

        let req = Request::ProcessTodoNow {
            id: 1,
            human_reply: Some("继续吧".into()),
        };
        let line = encode_line(&req);
        assert_eq!(decode_line::<Request>(&line).unwrap(), req);

        let req = Request::GetTodoDetail { id: 1 };
        let line = encode_line(&req);
        assert_eq!(decode_line::<Request>(&line).unwrap(), req);

        let turn = TurnRecord {
            turn_index: 0,
            role: "human".into(),
            content: "先看看这个 bug".into(),
            tool_calls: vec![],
            thinking: false,
            thinking_text: None,
            ts: Some(1),
            is_error: false,
            tokens_in: 0,
            tokens_out: 0,
            tokens_cache_read: 0,
            tokens_cache_write: 0,
        };
        let todo = TodoInfo {
            id: 1,
            project_id: 1,
            text: "修个 bug".into(),
            done: false,
            paused: false,
            rank: 0,
            created_ms: 1,
            completed_at_ms: None,
            plan_date: None,
            dispatch_session_id: Some("mint-1".into()),
            dispatch_at_ms: Some(1),
            category_id: None,
            assigned_agent: Some(AgentKind::Claude),
        };
        let reply = Reply::TodoDetail { info: todo, turns: vec![turn] };
        let line = encode_line(&reply);
        assert_eq!(decode_line::<Reply>(&line).unwrap(), reply);
    }

    #[test]
    fn todo_info_assigned_agent_defaults_to_none_when_absent_from_json() {
        // 老协议帧没有 assigned_agent 字段,新增字段要能优雅缺省,不报错。
        let json = r#"{"id":1,"project_id":1,"text":"任务","done":false,"paused":false,
            "rank":0,"created_ms":0,"completed_at_ms":null,"plan_date":null,
            "dispatch_session_id":null,"dispatch_at_ms":null}"#;
        let todo: TodoInfo = serde_json::from_str(json).unwrap();
        assert_eq!(todo.assigned_agent, None);
    }
```

`TurnRecord` 具体字段以 `protocol.rs:101-127` 现有定义为准,如果字段集合已经变化,按实际字段写(不要凭空加字段)。

- [ ] **Step 6: 编译 + 测试**

Run: `cargo test -p dozer-core`
Expected: 全部 PASS(含新增的 2 个测试)

- [ ] **Step 7: 提交**

```bash
git add crates/dozer-core/src/protocol.rs
git commit -m "feat(protocol): Todo 指派 agent + 分类轮询开关 + 任务详情协议"
```

---

## Task 2: dozerd TodoStore——assigned_agent 列 + 指派/铸造会话方法

**Files:**
- Modify: `crates/dozerd/src/todo.rs`

**Interfaces:**
- Consumes: `TodoInfo`(Task 1 新增 `assigned_agent` 字段)。
- Produces: `TodoStore::assign_agent(id: i64, agent: AgentKind) -> Result<TodoInfo>`;`TodoStore::set_dispatch_session(id: i64, session_id: &str) -> Result<TodoInfo>`(替换原 `record_dispatch`,Task 7 的 `task_processor` 用它铸造/复用 `dispatch_session_id`)。

- [ ] **Step 1: 迁移新增 `assigned_agent` 列**

在 `TodoStore::new`(`todo.rs:52` 起)里,`has_paused` 迁移块之后加(照抄 `has_category_id` 的既有写法):

```rust
        let has_assigned_agent: bool = conn
            .prepare("SELECT 1 FROM pragma_table_info('todos') WHERE name = 'assigned_agent'")?
            .exists([])?;
        if !has_assigned_agent {
            conn.execute("ALTER TABLE todos ADD COLUMN assigned_agent TEXT", [])
                .context("迁移 assigned_agent 列")?;
        }
```

同时在 `CREATE TABLE IF NOT EXISTS todos (...)` 的列定义里(给全新建库场景)加 `assigned_agent TEXT,`。

- [ ] **Step 2: `TODO_COLUMNS`/`row_to_todo` 加列 + 加 agent 字符串转换辅助函数**

```rust
const TODO_COLUMNS: &str = "id, project_id, text, done, paused, rank, created_ms, \
    completed_at_ms, plan_date, dispatch_session_id, dispatch_at_ms, category_id, \
    assigned_agent";

/// 只覆盖有 headless 适配器的四种(与 `headless_agent::bare_program_name`
/// 覆盖面一致)——`assigned_agent` 列不会存其余三种。未识别字符串(理论上
/// 不会出现,防御性)落回 `None`,不是恐慌。
fn agent_from_label(s: &str) -> Option<dozer_core::protocol::AgentKind> {
    use dozer_core::protocol::AgentKind;
    match s {
        "claude" => Some(AgentKind::Claude),
        "codebuddy" => Some(AgentKind::Codebuddy),
        "opencode" => Some(AgentKind::Opencode),
        "v8agent" => Some(AgentKind::V8agent),
        _ => None,
    }
}
```

`row_to_todo` 加一行(`category_id: row.get(11)?,` 之后):

```rust
        category_id: row.get(11)?,
        assigned_agent: row
            .get::<_, Option<String>>(12)?
            .and_then(|s| agent_from_label(&s)),
```

- [ ] **Step 3: 把 `record_dispatch` 换成 `assign_agent` + 新增 `set_dispatch_session`**

删除原 `record_dispatch` 方法(`todo.rs:224-234`),替换成:

```rust
    /// 指派任务给某个 agent 种类,纯记录,不碰 `dispatch_session_id`
    /// (那是"真正跑过一次处理"才会有的值,见 `set_dispatch_session`)。
    pub fn assign_agent(
        &self,
        id: i64,
        agent: dozer_core::protocol::AgentKind,
    ) -> Result<TodoInfo> {
        let conn = self.conn.lock().expect("db lock");
        let now = now_ms() as i64;
        let sql = format!(
            "UPDATE todos SET assigned_agent = ?1, dispatch_at_ms = ?2
             WHERE id = ?3 RETURNING {TODO_COLUMNS}"
        );
        conn.query_row(&sql, params![agent.label(), now, id], row_to_todo)
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => id_not_found(id),
                e => e.into(),
            })
    }

    /// 首次真正触发 headless 处理时,由 `task_processor` 铸造好
    /// `session_id` 后写回;之后每次处理复用同一个,不再变化(除非任务
    /// 被重新指派给不同 agent,由调用方决定要不要铸造新的)。
    pub fn set_dispatch_session(&self, id: i64, session_id: &str) -> Result<TodoInfo> {
        let conn = self.conn.lock().expect("db lock");
        let sql = format!(
            "UPDATE todos SET dispatch_session_id = ?1 WHERE id = ?2 RETURNING {TODO_COLUMNS}"
        );
        conn.query_row(&sql, params![session_id, id], row_to_todo)
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => id_not_found(id),
                e => e.into(),
            })
    }
```

- [ ] **Step 4: 写单测**

在 `todo.rs` 底部 `#[cfg(test)] mod tests` 里追加(仿现有 `record_dispatch`/`edit_text` 类测试的既有写法——先建一个临时 `TodoStore`、`add` 一条任务、再操作、断言返回值字段):

```rust
    #[test]
    fn assign_agent_sets_agent_and_dispatch_at_but_not_session() {
        let tmp = tempfile::tempdir().unwrap();
        let store = TodoStore::new(&tmp.path().join("t.db")).unwrap();
        let todo = store.add(1, "任务").unwrap();
        let updated = store
            .assign_agent(todo.id, dozer_core::protocol::AgentKind::Claude)
            .unwrap();
        assert_eq!(updated.assigned_agent, Some(dozer_core::protocol::AgentKind::Claude));
        assert!(updated.dispatch_at_ms.is_some());
        assert_eq!(updated.dispatch_session_id, None);
    }

    #[test]
    fn set_dispatch_session_writes_session_id_without_touching_agent() {
        let tmp = tempfile::tempdir().unwrap();
        let store = TodoStore::new(&tmp.path().join("t.db")).unwrap();
        let todo = store.add(1, "任务").unwrap();
        store
            .assign_agent(todo.id, dozer_core::protocol::AgentKind::Codebuddy)
            .unwrap();
        let updated = store.set_dispatch_session(todo.id, "mint-abc").unwrap();
        assert_eq!(updated.dispatch_session_id, Some("mint-abc".into()));
        assert_eq!(updated.assigned_agent, Some(dozer_core::protocol::AgentKind::Codebuddy));
    }

    #[test]
    fn assign_agent_unknown_id_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let store = TodoStore::new(&tmp.path().join("t.db")).unwrap();
        assert!(
            store
                .assign_agent(999, dozer_core::protocol::AgentKind::Claude)
                .is_err()
        );
    }
```

`store.add(1, "任务")` 的确切签名以 `todo.rs` 现有 `add` 方法为准(核实参数顺序/返回类型,不要凭空假设)。

- [ ] **Step 5: 编译 + 测试**

Run: `cargo test -p dozerd todo::`
Expected: 全部 PASS

- [ ] **Step 6: 提交**

```bash
git add crates/dozerd/src/todo.rs
git commit -m "feat(dozerd): TodoStore 新增 assigned_agent 列 + assign_agent/set_dispatch_session"
```

---

## Task 3: dozerd CategoryStore——auto_poll_enabled 列 + 开关方法

**Files:**
- Modify: `crates/dozerd/src/todo_category.rs`

**Interfaces:**
- Produces: `CategoryStore::set_auto_poll(id: i64, enabled: bool) -> Result<CategoryInfo>`;`CategoryStore::list_auto_poll_enabled_all() -> Result<Vec<CategoryInfo>>`(跨全部项目扫描,Task 8 的轮询器用——dozerd 是单进程服务多个项目,轮询不按当前打开哪个项目限定)。

- [ ] **Step 1: 迁移新增 `auto_poll_enabled` 列**

在 `CategoryStore::new`(`todo_category.rs:41` 起)的 `CREATE TABLE IF NOT EXISTS` 之后加(仿 `todo.rs` 的 `has_category_id` 写法,`todo_category.rs` 目前没有任何迁移块,这是第一个):

```rust
        let has_auto_poll_enabled: bool = conn
            .prepare(
                "SELECT 1 FROM pragma_table_info('todo_categories') WHERE name = 'auto_poll_enabled'",
            )?
            .exists([])?;
        if !has_auto_poll_enabled {
            conn.execute(
                "ALTER TABLE todo_categories ADD COLUMN auto_poll_enabled INTEGER NOT NULL DEFAULT 0",
                [],
            )
            .context("迁移 auto_poll_enabled 列")?;
        }
```

`CREATE TABLE IF NOT EXISTS todo_categories (...)` 的列定义(给全新建库场景)加 `auto_poll_enabled INTEGER NOT NULL DEFAULT 0,`。

- [ ] **Step 2: `CATEGORY_COLUMNS`/`row_to_category` 加列**

```rust
const CATEGORY_COLUMNS: &str = "id, project_id, parent_id, name, rank, created_ms, auto_poll_enabled";
```

`row_to_category` 加一行:

```rust
        created_ms: row.get::<_, i64>(5)? as u64,
        auto_poll_enabled: row.get(6)?,
```

- [ ] **Step 3: 新增 `set_auto_poll` + `list_auto_poll_enabled_all`**

```rust
    /// 打开/关闭某分类下任务的自动轮询处理。
    pub fn set_auto_poll(&self, id: i64, enabled: bool) -> Result<CategoryInfo> {
        let conn = self.conn.lock().expect("db lock");
        let sql = format!(
            "UPDATE todo_categories SET auto_poll_enabled = ?1
             WHERE id = ?2 RETURNING {CATEGORY_COLUMNS}"
        );
        conn.query_row(&sql, params![enabled, id], row_to_category)
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => id_not_found(id),
                e => e.into(),
            })
    }

    /// 跨全部项目扫描开启了自动轮询的分类,供 `task_poller` 用——dozerd
    /// 单进程服务多个项目,轮询不按"当前打开哪个项目"限定范围。
    pub fn list_auto_poll_enabled_all(&self) -> Result<Vec<CategoryInfo>> {
        let conn = self.conn.lock().expect("db lock");
        let sql = format!(
            "SELECT {CATEGORY_COLUMNS} FROM todo_categories WHERE auto_poll_enabled = 1"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], row_to_category)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
```

- [ ] **Step 4: 写单测**

```rust
    #[test]
    fn set_auto_poll_toggles_and_list_all_reflects_it() {
        let tmp = tempfile::tempdir().unwrap();
        let store = CategoryStore::new(&tmp.path().join("t.db")).unwrap();
        let cat = store.add(1, None, "系统bug").unwrap();
        assert!(!cat.auto_poll_enabled);
        assert!(store.list_auto_poll_enabled_all().unwrap().is_empty());

        let updated = store.set_auto_poll(cat.id, true).unwrap();
        assert!(updated.auto_poll_enabled);
        let all = store.list_auto_poll_enabled_all().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, cat.id);

        store.set_auto_poll(cat.id, false).unwrap();
        assert!(store.list_auto_poll_enabled_all().unwrap().is_empty());
    }
```

`store.add(1, None, "系统bug")` 的确切签名以 `todo_category.rs` 现有 `add` 方法为准(核实参数顺序,不要凭空假设)。

- [ ] **Step 5: 编译 + 测试**

Run: `cargo test -p dozerd todo_category::`
Expected: 全部 PASS

- [ ] **Step 6: 提交**

```bash
git add crates/dozerd/src/todo_category.rs
git commit -m "feat(dozerd): CategoryStore 新增 auto_poll_enabled 列 + 开关/全局扫描方法"
```

---

## Task 4: dozerd SessionSummaryStore——task_id 列 + 全站字面量补齐

**Files:**
- Modify: `crates/dozerd/src/session_summary.rs`
- Modify: `crates/dozerd/src/session_summary_backfill.rs`
- Modify: `crates/dozerd/src/server.rs`

**Interfaces:**
- Consumes: `SessionSummaryPayload.task_id`(Task 1)。
- Produces: `SessionSummaryStore::record`/`get`/`get_many` 全部读写 `task_id`;`SessionSummaryPayload` 全部构造点补 `task_id` 字段,workspace 恢复可编译。

- [ ] **Step 1: 迁移新增 `task_id` 列**

在 `SessionSummaryStore::open`(`session_summary.rs:76` 起)的 `has_conversation_id` 迁移块之后加(同款 `pragma_table_info` 探测写法):

```rust
        let has_task_id: bool = conn
            .prepare("SELECT 1 FROM pragma_table_info('session_summaries') WHERE name = 'task_id'")?
            .exists([])?;
        if !has_task_id {
            conn.execute("ALTER TABLE session_summaries ADD COLUMN task_id INTEGER", [])
                .context("迁移 task_id 列")?;
        }
```

- [ ] **Step 2: `record`/`get`/`get_many` 读写 `task_id`**

`record`(`session_summary.rs:125-145`)的 INSERT 语句加 `task_id` 列:

```rust
    pub fn record(&self, payload: &SessionSummaryPayload) -> Result<()> {
        self.conn.lock().expect("db lock").execute(
            "INSERT INTO session_summaries
             (session_id, agent_kind, conversation_id, title, summary, status, created_ts_ms, task_id)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)
             ON CONFLICT(session_id) DO UPDATE SET
                agent_kind = excluded.agent_kind,
                conversation_id = excluded.conversation_id,
                title = excluded.title,
                summary = excluded.summary,
                status = excluded.status,
                created_ts_ms = excluded.created_ts_ms,
                task_id = excluded.task_id",
            rusqlite::params![
                payload.session_id,
                agent_to_str(payload.agent_kind),
                payload.conversation_id,
                payload.title,
                payload.summary,
                status_to_str(payload.status),
                payload.created_ts_ms,
                payload.task_id,
            ],
        )?;
        Ok(())
    }
```

`get`(约 `session_summary.rs:147-166`)的 SELECT 加列、构造加字段:

```rust
    pub fn get(&self, session_id: &str) -> Result<Option<SessionSummaryPayload>> {
        let conn = self.conn.lock().expect("db lock");
        let mut stmt = conn.prepare(
            "SELECT session_id, agent_kind, conversation_id, title, summary, status, created_ts_ms, task_id
             FROM session_summaries WHERE session_id = ?1",
        )?;
        let mut rows = stmt.query(rusqlite::params![session_id])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        let agent_kind: String = row.get(1)?;
        let status: String = row.get(5)?;
        let created_ts_ms: i64 = row.get(6)?;
        Ok(Some(SessionSummaryPayload {
            session_id: row.get(0)?,
            agent_kind: agent_from_str(&agent_kind),
            conversation_id: row.get(2)?,
            title: row.get(3)?,
            summary: row.get(4)?,
            status: status_from_str(&status),
            created_ts_ms: created_ts_ms as u64,
            task_id: row.get(7)?,
        }))
    }
```

`get_many`(约 `session_summary.rs:169-200`)同样在 SELECT 列表末尾加 `, task_id`,读行的地方加 `task_id: row.get(7)?,`——按 `get` 同款改法,`get_many` 内部循环体的具体变量名以现有代码为准。

- [ ] **Step 3: 补齐全站 `SessionSummaryPayload` 字面量的 `task_id` 字段**

以下每处都加 `task_id: None,`(这次改动都不产生新的任务关联,只是让编译过):

- `crates/dozer-core/src/protocol.rs` 约 799 行(`conversations_with_summaries_reply_roundtrips` 测试)
- `crates/dozer-core/src/protocol.rs` 约 1192 行(`get_session_summary_reply_roundtrips_with_payload` 测试)
- `crates/dozer-core/src/protocol.rs` 约 1210 行(`get_session_summary_reply_roundtrips_with_no_conversation_id` 测试)——这三处已经在 Task 1 Step 4 里补过,这里核实一遍即可,不用重复改。
- `crates/dozerd/src/session_summary.rs` 约 218 行(测试辅助函数 `payload(session_id)`)
- `crates/dozerd/src/session_summary_backfill.rs` 约 121 行(补总结落库)
- `crates/dozerd/src/server.rs` 约 193 行(超时降级的启发式总结)
- `crates/dozerd/src/server.rs` 约 625 行(agent 经 `dozer-mcp` 交回的总结)
- `crates/dozerd/src/server.rs` 约 932 行(测试 fixture)

- [ ] **Step 4: 新增 task_id 往返测试**

在 `session_summary.rs` 测试区追加:

```rust
    #[test]
    fn record_and_get_roundtrips_task_id() {
        let tmp = tempfile::tempdir().unwrap();
        let store = SessionSummaryStore::open(&tmp.path().join("t.db")).unwrap();
        let mut p = payload("s1");
        p.task_id = Some(42);
        store.record(&p).unwrap();
        let got = store.get("s1").unwrap().unwrap();
        assert_eq!(got.task_id, Some(42));
    }
```

- [ ] **Step 5: 编译 + 测试**

Run: `cargo build -p dozerd -p dozer-core && cargo test -p dozerd session_summary:: && cargo test -p dozer-core`
Expected: 全绿(这一步会暴露 Step 3 有没有漏改的字面量——编译器报错就是漏改的位置)

- [ ] **Step 6: 提交**

```bash
git add crates/dozerd/src/session_summary.rs crates/dozerd/src/session_summary_backfill.rs crates/dozerd/src/server.rs crates/dozer-core/src/protocol.rs
git commit -m "feat(dozerd): session_summaries 新增 task_id 列,读写+全站字面量补齐"
```

---

## Task 5: dozerd TranscriptStore——headless 任务处理专用写入方法

**Files:**
- Modify: `crates/dozerd/src/transcripts/mod.rs`

**Interfaces:**
- Consumes: 无新依赖,复用现有 `conn: Mutex<Connection>`。
- Produces: `TranscriptStore::record_task_turns(conversation_id: &str, agent: AgentKind, project_dir: &str, task_title: &str, human_content: &str, ai_content: &str) -> Result<()>`——Task 7 的 `task_processor` 用它落一轮问答。**必须**同时写 `conversations` 表(占位行),否则 `get_conversation_turns` 的内连接会对这个 `conversation_id` 永远返回空(见 spec"背景"一节记录的坑,本任务附带回归测试)。

- [ ] **Step 1: 写失败的回归测试(证明现在的坑真实存在)**

在 `transcripts/mod.rs` 测试区追加(先证明"只插 `conversation_turns` 不插 `conversations`,读接口拿不到数据"这个坑,再在 Step 2 修):

```rust
    #[test]
    fn get_conversation_turns_returns_empty_without_matching_conversations_row() {
        let tmp = tempfile::tempdir().unwrap();
        let store = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
        // 直接绕过 `ingest_session`,只手动插 conversation_turns,不插 conversations——
        // 复现"headless 任务处理如果忘记同时插 conversations 占位行"这个坑。
        store
            .conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO conversation_turns
                 (conversation_id, turn_index, message_key, role, content, tools_summary,
                  thinking, ts, tool_calls, mutating_tool_calls, files_touched,
                  tokens_in, tokens_out, tokens_cache_read, tokens_cache_write, raw_json, is_error)
                 VALUES ('orphan',0,'orphan:0','human','hi','[]',0,1,0,0,'[]',0,0,0,0,'{}',0)",
                [],
            )
            .unwrap();
        let turns = store.get_conversation_turns("orphan", -1, 10).unwrap();
        assert!(turns.is_empty(), "没有 conversations 行时,JOIN 应该拿不到任何数据");
    }
```

- [ ] **Step 2: 运行测试确认复现**

Run: `cargo test -p dozerd get_conversation_turns_returns_empty_without_matching_conversations_row -- --exact`
Expected: PASS(这条测试本身就是在断言"坑存在",不是失败态——它证明了 Step 3 新方法为什么必须同时插两张表)

- [ ] **Step 3: 新增 `record_task_turns`**

在 `TranscriptStore` impl 块里、`get_conversation_turns` 附近加(`conversations`/`conversation_turns` 的 INSERT 语句照抄 `ingest_session` 里已有的两段,只是这次是"确保占位行存在"而不是"从文件增量摄取"):

```rust
    /// headless 任务处理专用:确保 `conversation_id` 对应的 `conversations`
    /// 占位行存在(`file_path` 用空字符串——这个会话没有真实 transcript
    /// 文件,和"扫描磁盘文件"的摄取路径完全独立),再插入一条人类回合 +
    /// 一条 AI 回合。`turn_index` 从该 `conversation_id` 现有最大值 + 1 起
    /// 连续分配,`message_key` 用 `"{conversation_id}:{turn_index}"` 保证
    /// 主键不冲突。
    pub fn record_task_turns(
        &self,
        conversation_id: &str,
        agent: AgentKind,
        project_dir: &str,
        task_title: &str,
        human_content: &str,
        ai_content: &str,
    ) -> Result<()> {
        let mut conn = self.conn.lock().expect("db lock");
        let tx = conn.transaction()?;
        let now = now_ms();
        tx.execute(
            "INSERT INTO conversations
             (conversation_id, agent_kind, dir, file_path, title, first_ts, last_ts,
              turn_count, parsed_offset, file_size_at_parse)
             VALUES (?1,?2,?3,'',?4,?5,?5,0,0,0)
             ON CONFLICT(conversation_id) DO NOTHING",
            params![
                conversation_id,
                agent_to_str(agent),
                project_dir,
                task_title,
                now,
            ],
        )?;
        let starting_turn_index: i64 = tx.query_row(
            "SELECT COALESCE(MAX(turn_index), -1) + 1 FROM conversation_turns
             WHERE conversation_id = ?1",
            [conversation_id],
            |row| row.get(0),
        )?;
        for (offset, (role, content)) in
            [("human", human_content), ("ai", ai_content)].into_iter().enumerate()
        {
            let turn_index = starting_turn_index + offset as i64;
            let message_key = format!("{conversation_id}:{turn_index}");
            tx.execute(
                "INSERT INTO conversation_turns
                 (conversation_id, turn_index, message_key, role, content, tools_summary,
                  thinking, ts, tool_calls, mutating_tool_calls, files_touched,
                  tokens_in, tokens_out, tokens_cache_read, tokens_cache_write, raw_json,
                  is_error)
                 VALUES (?1,?2,?3,?4,?5,'[]',0,?6,0,0,'[]',0,0,0,0,'{}',0)",
                params![conversation_id, turn_index, message_key, role, content, now],
            )?;
        }
        let turn_count: u32 = tx.query_row(
            "SELECT COUNT(*) FROM conversation_turns WHERE conversation_id = ?1",
            [conversation_id],
            |row| row.get(0),
        )?;
        tx.execute(
            "UPDATE conversations SET last_ts = ?1, turn_count = ?2 WHERE conversation_id = ?3",
            params![now, turn_count, conversation_id],
        )?;
        tx.commit()?;
        Ok(())
    }
```

`now_ms()`/`agent_to_str()` 若 `transcripts/mod.rs` 里已有同名私有函数则直接复用,不要重复定义(核实一下现有文件顶部的辅助函数列表)。

- [ ] **Step 4: 写正向单测**

```rust
    #[test]
    fn record_task_turns_creates_conversations_row_and_two_turns() {
        let tmp = tempfile::tempdir().unwrap();
        let store = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
        store
            .record_task_turns(
                "task-session-1",
                AgentKind::Claude,
                "/tmp/proj",
                "修个 bug",
                "先看看这个 bug",
                "已经修好了,提交在 abc123",
            )
            .unwrap();
        let turns = store.get_conversation_turns("task-session-1", -1, 10).unwrap();
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].role, "human");
        assert_eq!(turns[1].role, "ai");
        assert_eq!(turns[1].content, "已经修好了,提交在 abc123");
    }

    #[test]
    fn record_task_turns_appends_on_second_call_without_duplicating_conversations_row() {
        let tmp = tempfile::tempdir().unwrap();
        let store = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
        store
            .record_task_turns("task-session-1", AgentKind::Claude, "/tmp/proj", "修个 bug", "第一句", "第一次回复")
            .unwrap();
        store
            .record_task_turns("task-session-1", AgentKind::Claude, "/tmp/proj", "修个 bug", "第二句", "第二次回复")
            .unwrap();
        let turns = store.get_conversation_turns("task-session-1", -1, 10).unwrap();
        assert_eq!(turns.len(), 4);
        assert_eq!(turns[2].turn_index, 2);
        assert_eq!(turns[3].content, "第二次回复");
    }
```

- [ ] **Step 5: 编译 + 测试**

Run: `cargo test -p dozerd transcripts::`
Expected: 全部 PASS

- [ ] **Step 6: 提交**

```bash
git add crates/dozerd/src/transcripts/mod.rs
git commit -m "feat(dozerd): TranscriptStore 新增 record_task_turns,含 conversations 占位行回归测试"
```

---

## Task 6: dozerd headless_agent.rs——process_task_headless(真正执行工具调用的变体)

**Files:**
- Modify: `crates/dozerd/src/headless_agent.rs`

**Interfaces:**
- Produces: `pub async fn process_task_headless(agent: AgentKind, project_dir: &Path, session_id: &str, task_text: &str, prior_turns_text: &str, human_instruction: &str) -> Result<String, HeadlessError>`——`Ok(String)` 是捕获的完整 stdout,不做结构化提取。Task 7 的 `task_processor` 是唯一调用方。

- [ ] **Step 1: 手工核实 Opencode/V8agent 在一次性模式下真正调工具会不会卡住**

这一步是真实的探索性验证,不是可以跳过的形式:

```bash
# Opencode:让它在一次性模式下真的建一个文件,看会不会卡在授权确认上
cd /tmp && mkdir -p oc-probe && cd oc-probe
opencode run "在当前目录创建一个内容为 hello 的 probe.txt 文件"
ls probe.txt && cat probe.txt   # 有输出且内容对 → 不需要额外参数
```

```bash
# V8agent:同样探测(如果本机没装 v8agent-cli,跳过这一条,在 Task 6 的
# 实现里对 V8agent 分支加一行注释"未验证,行为参照 Opencode 的探测结论"
# 并在 PR 描述里标注这一条需要用户在合本机装了 v8agent-cli 的机器上补验)
cd /tmp && mkdir -p v8-probe && cd v8-probe
DOZER_SESSION_ID=probe-session V8AGENT_ONESHOT=1 v8agent run "在当前目录创建一个内容为 hello 的 probe.txt 文件" 2>&1 | head -50
ls probe.txt 2>&1
```

- 如果两条探测都在合理时间内(几十秒)完成且文件被正确创建 → 说明这两家
  一次性模式下调工具不需要额外的跳过权限参数,`build_task_command`(Step 3)
  两个分支不加任何权限相关 flag。
- 如果某一家卡住不退出(等到 `HEADLESS_TIMEOUT` 量级仍无响应)→ 说明需要
  参数,查该 CLI 的 `--help` 找跳过确认的选项(常见命名模式:`--yes`/
  `-y`/`--no-confirm`/`--auto-approve`),把探测到的真实参数写进 Step 3
  对应分支,并在这一步的探测记录里写清楚探测方法和最终采用的参数。

- [ ] **Step 2: 新增超时常量 + 拼 prompt 的辅助函数**

在 `HEADLESS_TIMEOUT` 常量附近加:

```rust
/// 任务处理可能要跑真正的编辑/构建,不是"读一遍对话写两句话",给更长的
/// 超时窗口。后续按实测调整,不是精确校准过的值。
const TASK_PROCESS_TIMEOUT: Duration = Duration::from_secs(600);
/// 喂进 prompt 的历史往来记录预算(字符数),超预算从最早的回合开始丢弃,
/// 保留离"现在要处理的指示"最近的尾部——同 `MAX_TRANSCRIPT_CHARS` 的
/// 既有口径,避免历史很长的任务把 prompt 撑爆或撞 CLI 参数长度上限。
const MAX_PRIOR_TURNS_CHARS_FOR_TASK: usize = 12_000;

fn task_instruction_text(task_text: &str, prior_turns_text: &str, human_instruction: &str) -> String {
    let history_block = if prior_turns_text.is_empty() {
        "(这是第一次处理,还没有任何往来记录)".to_string()
    } else {
        let truncated = truncate_chars(prior_turns_text, MAX_PRIOR_TURNS_CHARS_FOR_TASK);
        format!("到目前为止的往来记录:\n{truncated}")
    };
    format!(
        "你正在处理 Dozer 里的一个任务,可以真正读写这个项目目录下的文件、\
         执行命令来完成它。\n任务描述:{task_text}\n{history_block}\n\
         我现在的指示:{human_instruction}\n\
         请直接开始处理,完成后用简短的文字说明你做了什么、结果如何。"
    )
}
```

- [ ] **Step 3: 新增 `build_task_command`(四家 CLI 各自的命令构造,注入 cwd + DOZER_SESSION_ID)**

```rust
/// 与 `build_command` 并列,不复用其分隔符协议。四家分支都要
/// `current_dir(project_dir)`(`build_command` 完全没设置这个,总结不需要
/// 碰项目文件;这次必须要,否则 agent 编辑的是 dozerd 进程自己的 cwd)
/// 和 `DOZER_SESSION_ID` 环境变量(让 agent 侧 `dozer-mcp`/v8agent-cli 的
/// MCP 挂载识别到正确的 session,agent 才能调 `toggle_todo` 之类工具)。
/// V8agent 分支**不**像 `build_command` 那样 `env_remove("DOZER_SESSION_ID")`
/// ——总结场景故意不让 v8agent-cli 挂 MCP,这次场景反过来需要它挂上。
fn build_task_command(
    agent: AgentKind,
    program: &str,
    project_dir: &Path,
    session_id: &str,
    prompt: &str,
) -> Option<(tokio::process::Command, Option<Vec<u8>>)> {
    match agent {
        AgentKind::Claude => {
            let mut cmd = tokio::process::Command::new(program);
            cmd.current_dir(project_dir)
                .env("DOZER_SESSION_ID", session_id)
                .arg("-p")
                .arg(prompt)
                .arg("--dangerously-skip-permissions");
            Some((cmd, None))
        }
        AgentKind::Codebuddy => {
            let mut cmd = tokio::process::Command::new(program);
            cmd.current_dir(project_dir)
                .env("DOZER_SESSION_ID", session_id)
                .arg("-p")
                .arg(prompt)
                .arg("-y");
            Some((cmd, None))
        }
        AgentKind::Opencode => {
            let mut cmd = tokio::process::Command::new(program);
            cmd.current_dir(project_dir)
                .env("DOZER_SESSION_ID", session_id)
                .arg("run")
                .arg(prompt);
            Some((cmd, None))
        }
        AgentKind::V8agent => {
            let mut cmd = tokio::process::Command::new(program);
            cmd.current_dir(project_dir)
                .env("DOZER_SESSION_ID", session_id)
                .env("V8AGENT_ONESHOT", "1");
            Some((cmd, Some(prompt.as_bytes().to_vec())))
        }
        AgentKind::Unknown | AgentKind::Codex | AgentKind::Kilo => None,
    }
}
```

如果 Step 1 探测发现 Opencode/V8agent 需要额外的跳过确认参数,在对应分支的 `.arg(...)` 链上补上探测到的真实参数,并在函数注释里记一句"已实测确认需要"。

- [ ] **Step 4: 新增 `process_task_headless`(对外入口)**

```rust
/// 对外唯一入口:喂入任务文本 + 已有往来记录 + 本次人类指示,让 `agent`
/// 在 `project_dir` 下跑一次真正的处理(允许调用工具、不要求结构化输出)。
/// `Ok(String)` 是捕获的完整 stdout。`session_id` 只用于注入
/// `DOZER_SESSION_ID`,不影响本函数自身的返回值。
pub async fn process_task_headless(
    agent: AgentKind,
    project_dir: &Path,
    session_id: &str,
    task_text: &str,
    prior_turns_text: &str,
    human_instruction: &str,
) -> Result<String, HeadlessError> {
    let Some(bare) = bare_program_name(agent) else {
        return Err(HeadlessError::Unsupported);
    };
    let program = resolve_binary_path(bare).await.unwrap_or_else(|| bare.to_string());
    let prompt = task_instruction_text(task_text, prior_turns_text, human_instruction);
    let Some((cmd, stdin_bytes)) =
        build_task_command(agent, &program, project_dir, session_id, &prompt)
    else {
        return Err(HeadlessError::Unsupported);
    };
    run_task_and_capture(cmd, stdin_bytes, TASK_PROCESS_TIMEOUT).await
}

/// 与 `run_and_extract` 并列,但不做分隔符提取——原样返回完整 stdout(容
/// 忍非 UTF-8 字节用 `from_utf8_lossy`,这次输出是任意自由文本,不是受控
/// 的 JSON 载荷)。
async fn run_task_and_capture(
    mut cmd: tokio::process::Command,
    stdin_bytes: Option<Vec<u8>>,
    timeout: Duration,
) -> Result<String, HeadlessError> {
    use std::process::Stdio;
    cmd.stdin(if stdin_bytes.is_some() { Stdio::piped() } else { Stdio::null() });
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::null());
    let mut child = cmd.spawn().map_err(|e| HeadlessError::Spawn(e.to_string()))?;
    if let Some(bytes) = stdin_bytes {
        use tokio::io::AsyncWriteExt;
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(&bytes).await;
        }
    }
    let output = tokio::time::timeout(timeout, child.wait_with_output())
        .await
        .map_err(|_| HeadlessError::Timeout)?
        .map_err(|e| HeadlessError::Spawn(e.to_string()))?;
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}
```

- [ ] **Step 5: 写单测(复用现有"真实存在的 `sh`/不存在的二进制名"确定性手法)**

```rust
    #[tokio::test]
    async fn process_task_headless_returns_unsupported_for_kind_without_adapter() {
        let err = process_task_headless(
            AgentKind::Codex,
            Path::new("/tmp"),
            "s1",
            "task",
            "",
            "go",
        )
        .await
        .unwrap_err();
        assert_eq!(err, HeadlessError::Unsupported);
    }

    #[test]
    fn build_task_command_sets_current_dir_and_session_env() {
        let (cmd, _) = build_task_command(
            AgentKind::Claude,
            "sh",
            Path::new("/tmp/probe-dir"),
            "sess-1",
            "prompt text",
        )
        .unwrap();
        let std_cmd = cmd.as_std();
        assert_eq!(std_cmd.get_current_dir(), Some(Path::new("/tmp/probe-dir")));
        let envs: std::collections::HashMap<_, _> = std_cmd
            .get_envs()
            .filter_map(|(k, v)| Some((k.to_str()?.to_string(), v?.to_str()?.to_string())))
            .collect();
        assert_eq!(envs.get("DOZER_SESSION_ID"), Some(&"sess-1".to_string()));
    }

    #[tokio::test]
    async fn run_task_and_capture_returns_stdout_verbatim() {
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c").arg("echo hello-task");
        let out = run_task_and_capture(cmd, None, Duration::from_secs(5)).await.unwrap();
        assert!(out.contains("hello-task"));
    }

    #[tokio::test]
    async fn run_task_and_capture_times_out() {
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c").arg("sleep 5");
        let err = run_task_and_capture(cmd, None, Duration::from_millis(50))
            .await
            .unwrap_err();
        assert_eq!(err, HeadlessError::Timeout);
    }
```

`Command::as_std()`/`get_current_dir()`/`get_envs()` 是 `tokio::process::Command`/`std::process::Command` 现有 API,不需要额外依赖。

- [ ] **Step 6: 编译 + 测试**

Run: `cargo test -p dozerd headless_agent::`
Expected: 全部 PASS

- [ ] **Step 7: 提交**

```bash
git add crates/dozerd/src/headless_agent.rs
git commit -m "feat(dozerd): 新增 process_task_headless——允许真正执行工具调用的 headless 变体"
```

---

## Task 7: dozerd task_processor.rs——单任务处理编排(新文件)

**Files:**
- Create: `crates/dozerd/src/task_processor.rs`
- Modify: `crates/dozerd/src/lib.rs`(注册新模块)

**Interfaces:**
- Consumes: `TodoStore::set_dispatch_session`(Task 2)、`SessionSummaryStore::record`(Task 4)、`TranscriptStore::{get_conversation_turns, record_task_turns}`(Task 5)、`headless_agent::process_task_headless`(Task 6)、`ProjectStore::list`(现有)。
- Produces: `pub fn needs_processing(turns: &[TurnRecord]) -> bool`(纯函数,判定逻辑独立可测);`pub async fn process_task(todos: &TodoStore, categories:  &CategoryStore, session_summaries: &SessionSummaryStore, transcripts: &TranscriptStore, projects: &ProjectStore, todo: &TodoInfo, human_reply: Option<&str>) -> Result<(), String>`——Task 8 的轮询器和 Task 9 的 `ProcessTodoNow` handler 共用这一个函数,避免两条触发路径各写一份逻辑。

- [ ] **Step 1: 新建文件,写 `needs_processing` 纯函数 + 单测**

```rust
//! 单个任务的 headless 处理编排:铸造/复用会话、拼历史、调
//! `headless_agent::process_task_headless`、落回合。轮询器
//! (`task_poller.rs`)和"处理"按钮的 `ProcessTodoNow` handler 共用这个
//! 模块,避免两条触发路径各写一份逻辑(spec 2026-09-02)。

use crate::todo::TodoStore;
use crate::todo_category::CategoryStore;
use crate::transcripts::TranscriptStore;
use crate::session_summary::SessionSummaryStore;
use crate::projects::ProjectStore;
use dozer_core::protocol::{SessionSummaryPayload, SummaryStatus, TodoInfo, TurnRecord};

/// 待处理判定:不新增标记字段,直接看该任务关联会话最新一条回合的
/// `role`。没有任何回合(刚指派/从未处理过)、或最新一条是 `"human"`
/// (人类刚回复,或上一次处理失败没能写入 ai 回合)→ 待处理;最新一条是
/// `"ai"` → 不需要处理。
pub fn needs_processing(turns: &[TurnRecord]) -> bool {
    match turns.last() {
        None => true,
        Some(t) => t.role != "ai",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(role: &str) -> TurnRecord {
        TurnRecord {
            turn_index: 0,
            role: role.into(),
            content: "x".into(),
            tool_calls: vec![],
            thinking: false,
            thinking_text: None,
            ts: Some(1),
            is_error: false,
            tokens_in: 0,
            tokens_out: 0,
            tokens_cache_read: 0,
            tokens_cache_write: 0,
        }
    }

    #[test]
    fn empty_turns_need_processing() {
        assert!(needs_processing(&[]));
    }

    #[test]
    fn latest_human_turn_needs_processing() {
        assert!(needs_processing(&[turn("ai"), turn("human")]));
    }

    #[test]
    fn latest_ai_turn_does_not_need_processing() {
        assert!(!needs_processing(&[turn("human"), turn("ai")]));
    }
}
```

`TurnRecord` 字段集合以 Task 1 里核实过的 `protocol.rs:101-127` 现有定义为准。

- [ ] **Step 2: 运行测试确认 `needs_processing` 通过**

Run: `cargo test -p dozerd task_processor::`
Expected: 全部 PASS(此时 `process_task` 还没写,只测纯函数)

- [ ] **Step 3: 写 `process_task`**

```rust
/// 处理一个任务:解析项目目录 → 铸造/复用会话 id → 拼历史 → 调
/// `process_task_headless` → 落回合。成功/失败(spawn 失败/超时/不支持的
/// agent 种类)都会写一条 `ai` 回合(失败时内容是可读错误描述),不静默
/// 丢弃、不在本函数内重试——重试节奏由调用方(轮询器的下一轮 tick,或
/// 人类再次点"处理")决定。
pub async fn process_task(
    todos: &TodoStore,
    session_summaries: &SessionSummaryStore,
    transcripts: &TranscriptStore,
    projects: &ProjectStore,
    todo: &TodoInfo,
    human_reply: Option<&str>,
) -> Result<(), String> {
    let Some(agent) = todo.assigned_agent else {
        return Err("任务未指派 agent".into());
    };
    let project = projects
        .list()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|p| p.id == todo.project_id)
        .ok_or_else(|| "找不到该任务所属的项目".to_string())?;

    let session_id = match &todo.dispatch_session_id {
        Some(id) => id.clone(),
        None => {
            let minted = format!("task-{}-{}", todo.id, now_ms());
            todos
                .set_dispatch_session(todo.id, &minted)
                .map_err(|e| e.to_string())?;
            session_summaries
                .record(&SessionSummaryPayload {
                    session_id: minted.clone(),
                    agent_kind: agent,
                    conversation_id: Some(minted.clone()),
                    title: truncate_title(&todo.text),
                    summary: String::new(),
                    status: SummaryStatus::AiGenerated,
                    created_ts_ms: now_ms(),
                    task_id: Some(todo.id),
                })
                .map_err(|e| e.to_string())?;
            minted
        }
    };

    let prior_turns = transcripts
        .get_conversation_turns(&session_id, -1, u32::MAX)
        .map_err(|e| e.to_string())?;
    let prior_turns_text = prior_turns
        .iter()
        .map(|t| {
            let label = if t.role == "human" { "人类" } else { "AI" };
            format!("{label}: {}", t.content)
        })
        .collect::<Vec<_>>()
        .join("\n");
    let human_instruction = human_reply.unwrap_or(&todo.text);

    let result = crate::headless_agent::process_task_headless(
        agent,
        std::path::Path::new(&project.path),
        &session_id,
        &todo.text,
        &prior_turns_text,
        human_instruction,
    )
    .await;

    let ai_content = match result {
        Ok(stdout) => stdout,
        Err(e) => format!("(headless 处理失败: {e:?})"),
    };
    transcripts
        .record_task_turns(
            &session_id,
            agent,
            &project.path,
            &todo.text,
            human_instruction,
            &ai_content,
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn truncate_title(s: &str) -> String {
    if s.chars().count() <= 80 {
        s.to_string()
    } else {
        let head: String = s.chars().take(80).collect();
        format!("{head}…")
    }
}
```

`ProjectStore::list()` 的确切签名/返回类型以 `crates/dozerd/src/projects.rs:151` 现有 `list` 方法为准(核实一下是否是 `Result<Vec<ProjectInfo>>`,不要凭空假设)。

- [ ] **Step 4: 写集成测试(用真实存在的 `sh` 假装 agent CLI,不需要装 claude/codebuddy)**

这个测试没法直接用 `process_task`(它内部按 `AgentKind` 找 `bare_program_name` 固定到 `claude`/`codebuddy`/`opencode`/`v8agent` 四个裸命令名,不接受注入假程序名),所以这一步只测试到"没有 `assigned_agent` 时提前返回错误"和"目标项目不存在时提前返回错误"这两个不需要真的起子进程的分支,真正端到端跑通 `process_task_headless` 的验证放在 Task 14 的手工验收里(需要本机真的装了至少一家 CLI):

```rust
    #[tokio::test]
    async fn process_task_errors_without_assigned_agent() {
        let tmp = tempfile::tempdir().unwrap();
        let todos = TodoStore::new(&tmp.path().join("t.db")).unwrap();
        let session_summaries = SessionSummaryStore::open(&tmp.path().join("t.db")).unwrap();
        let transcripts = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
        let projects = ProjectStore::new(&tmp.path().join("t.db")).unwrap();
        let todo = todos.add(1, "任务").unwrap();
        let err = process_task(&todos, &session_summaries, &transcripts, &projects, &todo, None)
            .await
            .unwrap_err();
        assert!(err.contains("未指派"));
    }

    #[tokio::test]
    async fn process_task_errors_when_project_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let todos = TodoStore::new(&tmp.path().join("t.db")).unwrap();
        let session_summaries = SessionSummaryStore::open(&tmp.path().join("t.db")).unwrap();
        let transcripts = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
        let projects = ProjectStore::new(&tmp.path().join("t.db")).unwrap();
        let todo = todos.add(999, "任务").unwrap();
        let assigned = todos
            .assign_agent(todo.id, dozer_core::protocol::AgentKind::Claude)
            .unwrap();
        let err = process_task(&todos, &session_summaries, &transcripts, &projects, &assigned, None)
            .await
            .unwrap_err();
        assert!(err.contains("项目"));
    }
```

- [ ] **Step 5: 在 `lib.rs` 注册模块**

`crates/dozerd/src/lib.rs` 加一行(仿其它模块的既有声明方式,如 `pub mod todo;`):

```rust
pub mod task_processor;
```

- [ ] **Step 6: 编译 + 测试**

Run: `cargo test -p dozerd task_processor::`
Expected: 全部 PASS

- [ ] **Step 7: 提交**

```bash
git add crates/dozerd/src/task_processor.rs crates/dozerd/src/lib.rs
git commit -m "feat(dozerd): 新增 task_processor——单任务 headless 处理编排,轮询器与手动触发共用"
```

---

## Task 8: dozerd task_poller.rs——后台周期轮询(新文件,dozerd 第一个 scheduler)

**Files:**
- Create: `crates/dozerd/src/task_poller.rs`
- Modify: `crates/dozerd/src/lib.rs`(注册新模块)

**Interfaces:**
- Consumes: `CategoryStore::list_auto_poll_enabled_all`(Task 3)、`TodoStore`(Task 2,需要"列出某分类下未完成任务"的能力——如果现有 `TodoStore` 没有按 `category_id` 过滤的列表方法,本任务里新增一个)、`task_processor::{needs_processing, process_task}`(Task 7)。
- Produces: `pub struct TaskPoller`(持有 `Arc<Mutex<HashSet<i64>>>` 正在处理中的任务 id 集合);`pub fn spawn(...)`——main.rs 启动时调用一次,常驻扫描。`in_flight` 集合通过 `Arc` 共享给 Task 9 的 `ProcessTodoNow` handler,两条触发路径共用同一份去重状态。

- [ ] **Step 1: 核实/补齐 `TodoStore` 按分类列未完成任务的方法**

先检查 `crates/dozerd/src/todo.rs` 现有的 `list`/`list_by_category` 之类方法是否已经支持"某个 `category_id` 下未完成(`done = 0`)且已指派(`assigned_agent IS NOT NULL`)的任务"这个查询形状。如果没有,加一个:

```rust
    /// 某分类下已指派但未完成的任务(供 `task_poller` 扫描用)。
    pub fn list_assigned_incomplete_in_category(&self, category_id: i64) -> Result<Vec<TodoInfo>> {
        let conn = self.conn.lock().expect("db lock");
        let sql = format!(
            "SELECT {TODO_COLUMNS} FROM todos
             WHERE category_id = ?1 AND done = 0 AND assigned_agent IS NOT NULL
             ORDER BY rank ASC"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([category_id], row_to_todo)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
```

配套单测:

```rust
    #[test]
    fn list_assigned_incomplete_in_category_filters_correctly() {
        let tmp = tempfile::tempdir().unwrap();
        let store = TodoStore::new(&tmp.path().join("t.db")).unwrap();
        let t1 = store.add(1, "已指派未完成").unwrap();
        store.assign_agent(t1.id, dozer_core::protocol::AgentKind::Claude).unwrap();
        store.set_category(t1.id, Some(10)).unwrap();
        let t2 = store.add(1, "未指派").unwrap();
        store.set_category(t2.id, Some(10)).unwrap();
        let t3 = store.add(1, "已指派已完成").unwrap();
        store.assign_agent(t3.id, dozer_core::protocol::AgentKind::Claude).unwrap();
        store.set_category(t3.id, Some(10)).unwrap();
        store.toggle(t3.id, true).unwrap();

        let result = store.list_assigned_incomplete_in_category(10).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, t1.id);
    }
```

`store.toggle(id, done)` 的确切方法名/签名以 `todo.rs` 现有实现为准(核实,不要凭空假设方法名——如果现有方法叫别的名字,用实际名字)。

- [ ] **Step 2: 编译 + 测试这一步的新方法**

Run: `cargo test -p dozerd list_assigned_incomplete_in_category`
Expected: PASS

- [ ] **Step 3: 新建 `task_poller.rs`**

```rust
//! dozerd 第一个周期性后台任务(spec 2026-09-02):按固定间隔扫描开启了
//! `auto_poll_enabled` 的分类,把待处理的任务逐个交给
//! `task_processor::process_task`。全仓此前没有任何 `tokio::time::interval`
//! 用例(搜索确认过,唯一的周期性需求),范围限定在这一个用例,不做成
//! 通用 scheduler。

use crate::projects::ProjectStore;
use crate::session_summary::SessionSummaryStore;
use crate::todo::TodoStore;
use crate::todo_category::CategoryStore;
use crate::transcripts::TranscriptStore;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_secs(30);

/// 正在处理中的任务 id 集合,`task_poller` 和 `ProcessTodoNow` handler
/// (Task 9)共用同一份,防止同一任务被两条触发路径并发跑两次。
pub type InFlight = Arc<Mutex<HashSet<i64>>>;

pub fn new_in_flight() -> InFlight {
    Arc::new(Mutex::new(HashSet::new()))
}

/// 启动常驻轮询任务。返回的 `JoinHandle` 调用方通常不需要 `.await`
/// (dozerd 进程存活期间一直跑,随进程退出而结束),但保留返回值供测试
/// 场景需要时可以 `.abort()`。
pub fn spawn(
    todos: Arc<TodoStore>,
    categories: Arc<CategoryStore>,
    session_summaries: Arc<SessionSummaryStore>,
    transcripts: Arc<TranscriptStore>,
    projects: Arc<ProjectStore>,
    in_flight: InFlight,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(POLL_INTERVAL);
        loop {
            ticker.tick().await;
            tick_once(&todos, &categories, &session_summaries, &transcripts, &projects, &in_flight)
                .await;
        }
    })
}

/// 单次扫描,拆成独立函数供测试直接调用(不需要真的等 30 秒)。
pub async fn tick_once(
    todos: &TodoStore,
    categories: &CategoryStore,
    session_summaries: &SessionSummaryStore,
    transcripts: &TranscriptStore,
    projects: &ProjectStore,
    in_flight: &InFlight,
) {
    let Ok(enabled_categories) = categories.list_auto_poll_enabled_all() else {
        return;
    };
    for cat in enabled_categories {
        let Ok(candidates) = todos.list_assigned_incomplete_in_category(cat.id) else {
            continue;
        };
        for todo in candidates {
            let turns = transcripts
                .get_conversation_turns(
                    todo.dispatch_session_id.as_deref().unwrap_or(""),
                    -1,
                    u32::MAX,
                )
                .unwrap_or_default();
            if !crate::task_processor::needs_processing(&turns) {
                continue;
            }
            {
                let mut guard = in_flight.lock().expect("in_flight lock");
                if !guard.insert(todo.id) {
                    continue; // 已经在处理中,跳过这次
                }
            }
            let _ = crate::task_processor::process_task(
                todos,
                session_summaries,
                transcripts,
                projects,
                &todo,
                None,
            )
            .await;
            in_flight.lock().expect("in_flight lock").remove(&todo.id);
        }
    }
}
```

- [ ] **Step 4: 写单测(用可注入的假 in_flight 状态验证去重,不真的起子进程)**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn tick_once_skips_task_already_in_flight() {
        let tmp = tempfile::tempdir().unwrap();
        let todos = TodoStore::new(&tmp.path().join("t.db")).unwrap();
        let categories = CategoryStore::new(&tmp.path().join("t.db")).unwrap();
        let session_summaries = SessionSummaryStore::open(&tmp.path().join("t.db")).unwrap();
        let transcripts = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
        let projects = ProjectStore::new(&tmp.path().join("t.db")).unwrap();

        let cat = categories.add(1, None, "自动分类").unwrap();
        categories.set_auto_poll(cat.id, true).unwrap();
        let todo = todos.add(1, "任务").unwrap();
        todos.assign_agent(todo.id, dozer_core::protocol::AgentKind::Claude).unwrap();
        todos.set_category(todo.id, Some(cat.id)).unwrap();

        let in_flight = new_in_flight();
        in_flight.lock().unwrap().insert(todo.id); // 模拟"已经在处理中"

        // process_task 会因为项目 id=1 不存在而报错退出,但这条测试要验证
        // 的是"被 in_flight 挡住,压根没进到 process_task 那一步"——
        // 用 in_flight 集合在 tick_once 结束后仍然只有这一个 id、且没有
        // 新写入任何 conversation_turns 来间接验证(没有 project 时
        // process_task 会直接返回 Err,不会走到写回合那一步,所以两种
        // 情况在"有没有新回合"这个观测点上其实等价——改用更直接的观测:
        // 断言 in_flight 集合在 tick_once 之后没有被 remove 掉这个 id,
        // 说明 tick_once 一开始就 continue 了,没有进入处理分支)。
        tick_once(&todos, &categories, &session_summaries, &transcripts, &projects, &in_flight)
            .await;
        assert!(in_flight.lock().unwrap().contains(&todo.id));
    }

    #[tokio::test]
    async fn tick_once_ignores_categories_without_auto_poll() {
        let tmp = tempfile::tempdir().unwrap();
        let todos = TodoStore::new(&tmp.path().join("t.db")).unwrap();
        let categories = CategoryStore::new(&tmp.path().join("t.db")).unwrap();
        let session_summaries = SessionSummaryStore::open(&tmp.path().join("t.db")).unwrap();
        let transcripts = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
        let projects = ProjectStore::new(&tmp.path().join("t.db")).unwrap();

        let cat = categories.add(1, None, "未开启轮询").unwrap();
        let todo = todos.add(1, "任务").unwrap();
        todos.assign_agent(todo.id, dozer_core::protocol::AgentKind::Claude).unwrap();
        todos.set_category(todo.id, Some(cat.id)).unwrap();

        let in_flight = new_in_flight();
        tick_once(&todos, &categories, &session_summaries, &transcripts, &projects, &in_flight)
            .await;
        assert!(in_flight.lock().unwrap().is_empty(), "未开启轮询的分类不该被扫到");
    }
}
```

- [ ] **Step 5: 在 `lib.rs` 注册模块**

```rust
pub mod task_poller;
```

- [ ] **Step 6: 编译 + 测试**

Run: `cargo test -p dozerd task_poller::`
Expected: 全部 PASS

- [ ] **Step 7: 提交**

```bash
git add crates/dozerd/src/task_poller.rs crates/dozerd/src/todo.rs crates/dozerd/src/lib.rs
git commit -m "feat(dozerd): 新增 task_poller——分类维度自动轮询,in_flight 去重与手动触发共用"
```

---

## Task 9: dozerd server.rs + main.rs——接线新 Request、移除旧派发、启动轮询器

**Files:**
- Modify: `crates/dozerd/src/server.rs`
- Modify: `crates/dozerd/src/main.rs`

**Interfaces:**
- Consumes: Task 1-8 全部新增类型/方法。
- Produces: `serve()` 新增 `in_flight: task_poller::InFlight` 参数(用 `#[allow(clippy::too_many_arguments)]` 已经在这个函数上,继续沿用现有豁免,不新增违反 CLAUDE.md 参数结构体规则的场景——这个函数早就超过 7 参且全部是同类型 `Arc<Store>`,新增一个 `Arc<Mutex<...>>` 不改变"参数天然同质"的既有豁免依据)。

- [ ] **Step 1: `server.rs` 替换 `RecordTodoDispatch` handler,新增三个 handler**

把(`server.rs:463-469`):

```rust
                        Request::RecordTodoDispatch { id, session_id } => {
                            match todos.record_dispatch(id, &session_id) {
                                Ok(todo) => Reply::Todo { todo },
                                Err(e) => Reply::Error {
                                    message: format!("记录派发失败: {e}"),
                                },
                            }
                        }
```

替换成:

```rust
                        Request::AssignTodoAgent { id, agent } => {
                            match todos.assign_agent(id, agent) {
                                Ok(todo) => Reply::Todo { todo },
                                Err(e) => Reply::Error {
                                    message: format!("指派任务失败: {e}"),
                                },
                            }
                        }
                        Request::SetCategoryAutoPoll { id, enabled } => {
                            match categories.set_auto_poll(id, enabled) {
                                Ok(category) => Reply::Category { category },
                                Err(e) => Reply::Error {
                                    message: format!("设置自动轮询失败: {e}"),
                                },
                            }
                        }
                        Request::ProcessTodoNow { id, human_reply } => {
                            let Ok(todo) = todos.get(id) else {
                                break Reply::Error { message: format!("任务不存在: id={id}") };
                            };
                            {
                                let mut guard = in_flight.lock().expect("in_flight lock");
                                if !guard.insert(id) {
                                    break Reply::Error { message: "该任务正在处理中".into() };
                                }
                            }
                            let result = crate::task_processor::process_task(
                                &todos,
                                &session_summaries,
                                &transcripts,
                                &projects,
                                &todo,
                                human_reply.as_deref(),
                            )
                            .await;
                            in_flight.lock().expect("in_flight lock").remove(&id);
                            match result {
                                Ok(()) => match todos.get(id) {
                                    Ok(todo) => Reply::Todo { todo },
                                    Err(e) => Reply::Error { message: e.to_string() },
                                },
                                Err(e) => Reply::Error { message: format!("处理任务失败: {e}") },
                            }
                        }
                        Request::GetTodoDetail { id } => {
                            match todos.get(id) {
                                Ok(info) => {
                                    let turns = info
                                        .dispatch_session_id
                                        .as_deref()
                                        .and_then(|sid| {
                                            transcripts.get_conversation_turns(sid, -1, u32::MAX).ok()
                                        })
                                        .unwrap_or_default();
                                    Reply::TodoDetail { info, turns }
                                }
                                Err(e) => Reply::Error {
                                    message: format!("获取任务详情失败: {e}"),
                                },
                            }
                        }
```

`todos.get(id)` 需要 `TodoStore` 有一个按 id 查单条的方法——核实 `crates/dozerd/src/todo.rs` 现有是否已有 `pub fn get(&self, id: i64) -> Result<TodoInfo>`(大概率已经有,`edit_text`/`set_plan_date` 这类方法都靠 `RETURNING` 拿最新值,但"纯查询不改数据"的 `get` 可能还没有;如果没有,在 Task 2 里应该已经顺手补的话这里就不用再加——如果 Task 2 没加,现在补一个,用 `SELECT {TODO_COLUMNS} FROM todos WHERE id = ?1` 走 `query_row`,不带 `UPDATE`)。这里的 `match ... { break Reply::Error ... }` 写法要匹配 `handle_conn` 函数里 `match req { ... }` 外层实际的控制流结构(是 `match` 表达式整体求值成 `Reply` 还是 `loop { break }`,核实 `server.rs` 现有其它分支的写法,不要凭空引入新的控制流模式——多数分支都是 `match req { Request::X { .. } => { match store.op() { Ok(v) => Reply::Y{...}, Err(e) => Reply::Error{...} } } }` 这种直接表达式求值,没有 `break`,照抄这个模式,不要用 `break`)。

- [ ] **Step 2: `serve()` 签名加 `in_flight` 参数,透传给 `handle_conn`**

`serve()`(`server.rs:15-26`)加一个参数:

```rust
#[allow(clippy::too_many_arguments)]
pub async fn serve(
    socket: &Path,
    registry: Arc<SessionRegistry>,
    store: Arc<crate::acceptance::AcceptanceStore>,
    projects: Arc<crate::projects::ProjectStore>,
    bookmarks: Arc<crate::bookmarks::BookmarkStore>,
    transcripts: Arc<crate::transcripts::TranscriptStore>,
    session_summaries: Arc<crate::session_summary::SessionSummaryStore>,
    backfill_registry: Arc<crate::session_summary_backfill::BackfillRegistry>,
    todos: Arc<crate::todo::TodoStore>,
    categories: Arc<crate::todo_category::CategoryStore>,
    in_flight: crate::task_poller::InFlight,
) -> Result<()> {
```

在 `loop { ... }` 里 accept 连接后新增的 `.clone()` 段落(仿 `todos`/`categories` 现有的 `.clone()` 写法)加 `let in_flight = in_flight.clone();`,并在调用 `handle_conn(...)` 的参数列表里把它一并传进去(`handle_conn` 函数签名同步加这个参数,函数体内 `Request::ProcessTodoNow` 分支用它)。

- [ ] **Step 3: `main.rs` 构造 `in_flight`,启动轮询器,更新 `serve()` 调用**

在 `main.rs` 构造完 `todos`/`categories` 之后(`main.rs:98-100` 附近)加:

```rust
    let in_flight = dozerd::task_poller::new_in_flight();
    dozerd::task_poller::spawn(
        todos.clone(),
        categories.clone(),
        session_summaries.clone(),
        transcripts.clone(),
        projects.clone(),
        in_flight.clone(),
    );
```

`serve(...)` 调用(`main.rs:106-117`)的参数列表末尾加 `in_flight,`。

- [ ] **Step 4: 编译 + 现有测试全绿(这一步是纯接线,不新增测试,靠既有 `server.rs`/`main.rs` 相关测试和全 crate 编译验证没有破坏现有分支)**

Run: `cargo build -p dozerd && cargo test -p dozerd`
Expected: 全绿。如果 `server.rs` 里有测试直接构造 `serve(...)` 调用(核实一下,有的话补上新参数),编译器会精确报出哪些调用点需要补参数。

- [ ] **Step 5: 提交**

```bash
git add crates/dozerd/src/server.rs crates/dozerd/src/main.rs
git commit -m "feat(dozerd): 接线 AssignTodoAgent/SetCategoryAutoPoll/ProcessTodoNow/GetTodoDetail,启动轮询器"
```

---

## Task 10: dozer-client——新方法替换 record_todo_dispatch

**Files:**
- Modify: `crates/dozer-client/src/lib.rs`

**Interfaces:**
- Produces: `Client::{assign_todo_agent, set_category_auto_poll, process_todo_now, get_todo_detail}`,移除 `Client::record_todo_dispatch`。

- [ ] **Step 1: 移除 `record_todo_dispatch`,新增四个方法**

删除 `record_todo_dispatch`(`dozer-client/src/lib.rs:345-357`),替换成:

```rust
    pub async fn assign_todo_agent(&self, id: i64, agent: AgentKind) -> Result<TodoInfo> {
        match self
            .roundtrip(&Request::AssignTodoAgent { id, agent })
            .await?
        {
            Reply::Todo { todo } => Ok(todo),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn set_category_auto_poll(&self, id: i64, enabled: bool) -> Result<CategoryInfo> {
        match self
            .roundtrip(&Request::SetCategoryAutoPoll { id, enabled })
            .await?
        {
            Reply::Category { category } => Ok(category),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    /// 立即触发一次任务处理(不等轮询)。可能耗时数分钟(headless 处理
    /// 上限 10 分钟),调用方需要走异步 `handle.spawn`,不能阻塞 UI 线程。
    pub async fn process_todo_now(
        &self,
        id: i64,
        human_reply: Option<&str>,
    ) -> Result<TodoInfo> {
        match self
            .roundtrip(&Request::ProcessTodoNow {
                id,
                human_reply: human_reply.map(|s| s.to_string()),
            })
            .await?
        {
            Reply::Todo { todo } => Ok(todo),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn get_todo_detail(&self, id: i64) -> Result<(TodoInfo, Vec<TurnRecord>)> {
        match self.roundtrip(&Request::GetTodoDetail { id }).await? {
            Reply::TodoDetail { info, turns } => Ok((info, turns)),
            Reply::Error { message } => Err(anyhow::anyhow!(message)),
            other => bail!("意外应答: {other:?}"),
        }
    }
```

- [ ] **Step 2: 编译(核实 dozer-client 里没有别的地方还在用 `record_todo_dispatch`)**

Run: `cargo build -p dozer-client`
Expected: 编译通过。如果报错说明有遗漏的调用点,按报错位置补齐(这一步不该有,因为改动只在这一个文件内)。

- [ ] **Step 3: 提交**

```bash
git add crates/dozer-client/src/lib.rs
git commit -m "feat(dozer-client): 新增 assign_todo_agent/set_category_auto_poll/process_todo_now/get_todo_detail,移除 record_todo_dispatch"
```

---

## Task 11: dozer-app extensions/todo.rs——消息/状态/agent 选择器覆盖旧派发层/详情弹窗状态骨架

**Files:**
- Modify: `crates/dozer-app/src/extensions/todo.rs`

**Interfaces:**
- Consumes: `Client::{assign_todo_agent, process_todo_now, get_todo_detail}`(Task 10)。
- Produces: `Message::{AssignAgent, DetailOpen, DetailClose, DetailReplyInput, DetailReplySubmit}`(替换 `DispatchToExisting`);`WorkspaceState` 新增详情弹窗字段;`detail_reply_field_id()`/`CaptureDetailReplyFocus`/`take_detail_reply_focused()`(镜像 `CaptureCategoryRenameFocus` 三件套)。这一层只管本地状态和向上转发,真正的 RPC 调用/焦点闸门在 Task 12 的 `app.rs`。

- [ ] **Step 1: `Message` 枚举替换 `DispatchToExisting`,新增详情弹窗四个消息**

把(`todo.rs:764-766`):

```rust
    DispatchOpen(usize),
    DispatchClose,
    DispatchToExisting(usize, String),
```

改成:

```rust
    DispatchOpen(usize),
    DispatchClose,
    /// 选定 agent 类型完成指派(纯记录,不触发执行)。内核拦截,转发到
    /// `App::todo_assign_agent`——真正的 RPC 调用在 `app.rs`,`todo::update`
    /// 只负责关掉选择层。
    AssignAgent(usize, dozer_core::protocol::AgentKind),
    /// 点卡片"详情"按钮,打开任务详情弹窗。内核拦截,转发到
    /// `App::todo_detail_open`(发 `GetTodoDetail` RPC 拉取回合列表)。
    DetailOpen(usize),
    /// 关闭详情弹窗(Esc / 点外部 / 点关闭按钮)。
    DetailClose,
    /// 详情弹窗回复框草稿变化(`text_input::on_input`,给全量当前字符串)。
    DetailReplyInput(String),
    /// 点"处理"按钮:内核拦截,转发到 `App::todo_detail_process`(乐观插入
    /// 一条本地回合 + 发 `ProcessTodoNow` RPC)。`todo::update` 只清空
    /// 草稿、置处理中标记。
    DetailReplySubmit,
```

- [ ] **Step 2: `WorkspaceState` 加详情弹窗字段**

在 `category_rename_focus_pending: bool,` 字段(`todo.rs` 结构体末尾附近)之后加:

```rust
    /// 详情弹窗展开态(卡片下标),`None` = 未展开。跟 `dispatch_open`/
    /// `calendar_open` 同一种"同时只能有一个"模型。
    detail_open: Option<usize>,
    /// 详情弹窗拉到的回合列表(`GetTodoDetail` 应答),弹窗关闭时清空。
    detail_turns: Vec<dozer_core::protocol::TurnRecord>,
    /// 回复框草稿(`text_input` 的 value)。
    detail_reply_draft: String,
    /// 回复框是否持有 iced 内部真实焦点,每帧由 `CaptureDetailReplyFocus`
    /// 写入,镜像 `category_rename_focused`。
    detail_reply_focused: bool,
    /// "处理"按钮是否正在等待 `ProcessTodoNow` RPC 返回——耗时可能到 10
    /// 分钟,期间按钮显示 loading 态、禁用重复提交。
    detail_processing: bool,
```

- [ ] **Step 3: 加对应的 getter/setter + `Operation`/focus 三件套(镜像 `category_rename_field_id`/`CaptureCategoryRenameFocus`/`take_category_rename_focused`)**

紧跟 `WorkspaceState` 的 `impl` 块里(`dispatch_popup_open`/`status_popup_open` 附近)加:

```rust
    /// 详情弹窗是否打开(内核键盘 Esc 关闭用,同 `dispatch_popup_open`)。
    pub fn detail_popup_open(&self) -> bool {
        self.detail_open.is_some()
    }

    pub fn detail_turns(&self) -> &[dozer_core::protocol::TurnRecord] {
        &self.detail_turns
    }

    pub fn detail_reply_draft(&self) -> &str {
        &self.detail_reply_draft
    }

    pub fn detail_processing(&self) -> bool {
        self.detail_processing
    }

    pub fn detail_reply_focused(&self) -> bool {
        self.detail_reply_focused
    }

    pub fn set_detail_reply_focused_flag(&mut self, focused: bool) {
        self.detail_reply_focused = focused;
    }

    /// `App::todo_detail_open` 拉到 `GetTodoDetail` 应答后写回本地状态。
    pub fn open_detail(&mut self, idx: usize, turns: Vec<dozer_core::protocol::TurnRecord>) {
        self.detail_open = Some(idx);
        self.detail_turns = turns;
        self.detail_reply_draft.clear();
    }

    pub fn close_detail(&mut self) {
        self.detail_open = None;
        self.detail_turns.clear();
        self.detail_reply_draft.clear();
        self.detail_processing = false;
    }

    /// 乐观本地插入一条人类回合(提交回复时,不等 RPC 回来就先看到)。
    pub fn push_optimistic_human_turn(&mut self, content: String) {
        let next_index = self.detail_turns.last().map(|t| t.turn_index + 1).unwrap_or(0);
        self.detail_turns.push(dozer_core::protocol::TurnRecord {
            turn_index: next_index,
            role: "human".into(),
            content,
            tool_calls: vec![],
            thinking: false,
            thinking_text: None,
            ts: None,
            is_error: false,
            tokens_in: 0,
            tokens_out: 0,
            tokens_cache_read: 0,
            tokens_cache_write: 0,
        });
        self.detail_processing = true;
    }

    /// `ProcessTodoNow` RPC 权威结果回来后,用服务端最新回合列表整体替换
    /// (替换掉乐观插入的那条,避免和服务端最终写入的 `turn_index`/
    /// `message_key` 不一致)。
    pub fn replace_detail_turns(&mut self, turns: Vec<dozer_core::protocol::TurnRecord>) {
        self.detail_turns = turns;
        self.detail_processing = false;
    }
```

在 `add_field_id`/`content_field_id`/`category_rename_field_id` 三个函数(`todo.rs:868-878` 附近)之后加第四个:

```rust
pub fn detail_reply_field_id() -> Id {
    Id::new("todo-detail-reply-field")
}

static DETAIL_REPLY_FOCUSED: std::sync::LazyLock<std::sync::Mutex<bool>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(false));

pub fn take_detail_reply_focused() -> bool {
    std::mem::replace(&mut *DETAIL_REPLY_FOCUSED.lock().unwrap(), false)
}

pub struct CaptureDetailReplyFocus;
impl Operation<()> for CaptureDetailReplyFocus {
    fn focusable(&mut self, id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Focusable) {
        if id == Some(&detail_reply_field_id()) {
            *DETAIL_REPLY_FOCUSED.lock().unwrap() = state.is_focused();
        }
    }

    fn traverse(&mut self, operate: &mut dyn for<'a> FnMut(&'a mut (dyn Operation<()> + 'a))) {
        operate(self);
    }
}
```

- [ ] **Step 4: `update()` 里加对应处理分支**

在 `Message::DispatchClose => { ... }`(`todo.rs:1335-1336`)之后加:

```rust
        Message::AssignAgent(_, _) => {
            // 真正的 RPC 调用在 `App::todo_assign_agent`(app.rs),这里
            // 只负责关掉选择层——与 `DispatchClose` 同款收尾。
            ws_state.dispatch_open = None;
            ws_state.dispatch_anchor = None;
        }
        Message::DetailClose => ws_state.close_detail(),
        Message::DetailReplyInput(text) => ws_state.detail_reply_draft = text,
        Message::DetailReplySubmit => {
            // 乐观插入 + 置处理中标记在这里做(纯本地状态);真正发
            // `ProcessTodoNow` RPC 在 `App::todo_detail_process`(app.rs),
            // 那边会读 `detail_reply_draft` 拿文本、读 `items()[idx].id`
            // 拿任务 id。
            let draft = std::mem::take(&mut ws_state.detail_reply_draft);
            if !draft.trim().is_empty() {
                ws_state.push_optimistic_human_turn(draft);
            }
        }
        Message::DetailOpen(_) => {
            // 真正拉 `GetTodoDetail` 在 `App::todo_detail_open`(app.rs),
            // `todo::update` 不处理这条(no-op arm 保持 match 穷尽,同
            // `AddResizeStart` 的既有模式)。
        }
```

`ws_state.dispatch_open`/`dispatch_anchor` 字段是私有的(`todo.rs` 结构体定义没标 `pub`),`update` 函数本身在同一个模块内,直接字段访问没问题——核实一下 `update` 函数当前的可见性范围/是否是同一 `impl` 或裸函数(`todo.rs` 现有 `Message::DispatchClose` 分支已经是这么写的,照抄旁边就行)。

- [ ] **Step 5: 把 `todo_dispatch_overlay` 里"列出活着的 tab"换成"选 agent 类型"**

把整个函数体(`todo.rs:2578-2646`)的 `tabs`/`items` 构造部分,从"扫 `ws.tabs`/`ws.ssh_tabs`"换成固定四选项:

```rust
/// Todo 指派选择层(窗口级 overlay 版):选一个 agent 种类完成指派,不再
/// 要求"存在活着的 tab"(2026-09-02 起,指派与执行解耦——指派只是记录,
/// 真正执行靠分类轮询开关或详情弹窗手动"处理")。样式沿用
/// `crate::menu::item_row_fill` + `menu::shell`,定位靠 `dispatch_anchor`。
pub fn todo_dispatch_overlay<'a>(
    ws: &Workspace,
    window_size: (f32, f32),
) -> Option<Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>> {
    let ws_state = &ws.todo;
    let idx = ws_state.dispatch_open?;
    let anchor = ws_state.dispatch_anchor?;

    // 只列出有 headless 适配器的四种(与 `AgentKind::label()` 的四个可指派
    // 值一致,`dozerd::headless_agent::bare_program_name` 同一份覆盖面)。
    let candidates = [
        AgentKind::Claude,
        AgentKind::Codebuddy,
        AgentKind::Opencode,
        AgentKind::V8agent,
    ];
    let items: Vec<Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>> = candidates
        .into_iter()
        .map(|agent| {
            let icon = icons::view(
                agent_icon(agent),
                byteui::theme::icon_size::row(),
                byteui::theme::color::current().cream,
            );
            crate::menu::item_row(
                Some(icon),
                agent.label().to_string(),
                byteui::theme::color::current().cream,
                Some(Message::AssignAgent(idx, agent)),
            )
        })
        .collect();
    let popup = crate::menu::shell(items, Length::Shrink);

    let (ax, ay) = anchor;
    let window_w = window_size.0;
    let window_h = window_size.1;
    let pop_w = 224.0_f32;
    let pop_h = 160.0_f32;
    let x = ax.min((window_w - pop_w).max(0.0));
    let y = ay.min((window_h - pop_h).max(0.0));
    Some(
        container(popup)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(Padding {
                top: y,
                left: x,
                right: 0.0,
                bottom: 0.0,
            })
            .into(),
    )
}
```

`agent_icon(agent)` 是现有辅助函数(原代码里已经在用,`todo.rs` 里 `let icon = icons::view(agent_icon(*agent), ...)`),沿用即可。

- [ ] **Step 6: `task_title_for_session` 更新注释(逻辑不变,语义说明要跟上)**

`todo.rs:447-455` 的文档注释更新(逻辑代码本身不用改,`dispatch_session_id` 字段名没变):

```rust
    /// 反查:这个 `session_id` 是不是某条 Todo 任务铸造出来的会话,是的话
    /// 返回该任务原文——给 Agent 卡片"当前工作内容"当主选数据源用。注意
    /// `dispatch_session_id` 现在是"首次真正处理时铸造"的值(2026-09-02
    /// 起,指派与执行解耦),任务被指派但还没真正处理过时该字段是 `None`,
    /// 这个反查天然不会命中——不代表这个反查逻辑本身需要改。
    pub fn task_title_for_session<'a>(&'a self, session_id: &str) -> Option<&'a str> {
```

- [ ] **Step 7: 编译(先不追求全绿——`app.rs`/`main.rs` 的对应改动在 Task 12/13,这一步先确认 `extensions/todo.rs` 自身没有语法错误)**

Run: `cargo build -p dozer-app 2>&1 | grep -A3 "extensions/todo.rs"`
Expected: 报错应该都集中在 `app.rs` 那边引用了旧的 `Message::DispatchToExisting`/`ws.todo.dispatch_open` 这类符号(Task 12 修),`extensions/todo.rs` 自身不应该有报错。

- [ ] **Step 8: 提交**

```bash
git add crates/dozer-app/src/extensions/todo.rs
git commit -m "feat(dozer-app): Todo 派发层改成选 agent 类型,详情弹窗本地状态骨架"
```

---

## Task 12: dozer-app app.rs——路由/焦点闸门 getter-setter/详情弹窗渲染/会话面板关联标签

**Files:**
- Modify: `crates/dozer-app/src/app.rs`
- Modify: `crates/dozer-app/src/workspace.rs`(移除 `dispatch_todo_to_existing`;`conversation_list_pane` 加关联任务文案)
- Modify: `crates/dozer-app/src/conversation.rs`(`SessionRow` 加 `task_id`)

**Interfaces:**
- Consumes: Task 11 新消息、Task 10 新 client 方法。
- Produces: `App::{detail_reply_focused, set_detail_reply_focused, todo_assign_agent, todo_detail_open, todo_detail_process}`;详情弹窗渲染函数;`SessionRow.task_id`。

- [ ] **Step 1: 移除 `workspace.rs::dispatch_todo_to_existing`**

删除整个方法(`workspace.rs:970-998`)。这个方法删除后,`app.rs::todo_dispatch_to_existing`(下一步一并删除)不再引用它,不会有悬空调用。

- [ ] **Step 2: `app.rs` 替换路由 + 删除 `todo_dispatch_to_existing`,新增三个方法**

把(`app.rs:4474-4476`):

```rust
            Message::Todo(todo::Message::DispatchToExisting(idx, session_id)) => {
                self.todo_dispatch_to_existing(idx, session_id)
            }
```

替换成:

```rust
            Message::Todo(todo::Message::AssignAgent(idx, agent)) => {
                self.todo_assign_agent(idx, agent)
            }
            Message::Todo(todo::Message::DetailOpen(idx)) => self.todo_detail_open(idx),
            Message::Todo(todo::Message::DetailReplySubmit) => self.todo_detail_process(),
```

删除 `todo_dispatch_to_existing` 方法整体(`app.rs:6218-6250`),替换成三个新方法(放在同一位置):

```rust
    /// 指派任务给某个 agent 种类,纯记录,不触发任何执行。异步确认经
    /// `Mutated` 刷新列表——与其它写操作同一条乐观更新链路。
    fn todo_assign_agent(&mut self, idx: usize, agent: dozer_core::protocol::AgentKind) {
        let Some(id) = self
            .active_workspace()
            .and_then(|ws| ws.todo.items().get(idx))
            .map(|item| item.id)
        else {
            return;
        };
        self.with_focused_project(|ws, _io| {
            ws.todo.close_dispatch_popup();
        });
        let handle = self.handle.clone();
        let client = self.client.clone();
        let proxy = self.proxy.clone();
        handle.spawn(async move {
            let res = client
                .assign_todo_agent(id, agent)
                .await
                .map(|_| ())
                .map_err(|e| e.to_string());
            let _ = proxy.send_event(Message::Todo(todo::Message::Mutated(res)));
        });
    }

    /// 打开任务详情弹窗:先本地记下 `idx`(弹窗定位/后续"处理"要用),再
    /// 异步拉 `GetTodoDetail`。RPC 结果经专门的 `Message::TodoDetailLoaded`
    /// 落地——`todo::Message::Mutated` 那条通用刷新链路只刷 `items`/
    /// `categories`,不携带回合数据,不能复用。
    fn todo_detail_open(&mut self, idx: usize) {
        let Some(id) = self
            .active_workspace()
            .and_then(|ws| ws.todo.items().get(idx))
            .map(|item| item.id)
        else {
            return;
        };
        self.with_focused_project(|ws, _io| {
            ws.todo.open_detail(idx, Vec::new());
        });
        let handle = self.handle.clone();
        let client = self.client.clone();
        let proxy = self.proxy.clone();
        handle.spawn(async move {
            if let Ok((_, turns)) = client.get_todo_detail(id).await {
                let _ = proxy.send_event(Message::TodoDetailLoaded(idx, turns));
            }
        });
    }

    /// 详情弹窗"处理"按钮:乐观插入已经在 `todo::update`(`DetailReplySubmit`
    /// 分支)做过,这里只管发 `ProcessTodoNow` RPC 并在结果回来后用服务端
    /// 权威回合列表刷新。耗时可能到 10 分钟,走 `handle.spawn` 不阻塞 UI。
    fn todo_detail_process(&mut self) {
        let Some((idx, id, reply_text)) = self.active_workspace().and_then(|ws| {
            let idx = ws.todo.detail_open_idx()?;
            let id = ws.todo.items().get(idx)?.id;
            Some((idx, id, ws.todo.last_reply_text()))
        }) else {
            return;
        };
        let handle = self.handle.clone();
        let client = self.client.clone();
        let proxy = self.proxy.clone();
        handle.spawn(async move {
            let _ = client.process_todo_now(id, reply_text.as_deref()).await;
            if let Ok((_, turns)) = client.get_todo_detail(id).await {
                let _ = proxy.send_event(Message::TodoDetailLoaded(idx, turns));
            }
        });
    }
```

`todo_detail_process` 需要 `ws.todo` 暴露 `detail_open_idx() -> Option<usize>` 和 `last_reply_text() -> Option<String>` 两个小 getter——回到 Task 11 的 `WorkspaceState` impl 块里补上:

```rust
    pub fn detail_open_idx(&self) -> Option<usize> {
        self.detail_open
    }

    /// 详情弹窗最后一次乐观插入的人类回合内容(`push_optimistic_human_turn`
    /// 刚插入的那条),供 `App::todo_detail_process` 转发进 `ProcessTodoNow`
    /// RPC 的 `human_reply` 参数。`detail_turns` 里最后一条一定是刚插入的
    /// 人类回合(`DetailReplySubmit` 处理顺序:先插入本地乐观回合,`app.rs`
    /// 再读这个值发 RPC)。
    pub fn last_reply_text(&self) -> Option<String> {
        self.detail_turns
            .last()
            .filter(|t| t.role == "human")
            .map(|t| t.content.clone())
    }
```

（这个补充步骤实际应该并入 Task 11 Step 3,如果 Task 11 已经完成合并,就在 Task 12 这里直接补上这两个方法,不用回退重开 Task 11 的 commit。）

- [ ] **Step 3: 新增 `Message::TodoDetailLoaded`,`app.rs` 顶层 `Message` 枚举 + 路由**

在 `Message::ConversationSessionsRefreshed(ProjectId, Result<Vec<SessionRow>, String>),`(`app.rs:1544`)附近加:

```rust
    /// `GetTodoDetail` 异步结果:`usize` 是打开弹窗时记录的卡片下标(用来
    /// 校验弹窗还开着同一个任务,不是用 id 找——`items()` 下标和渲染时
    /// 用的下标必须一致,同 `todo::Message` 全线用下标寻址任务的既有约定)。
    TodoDetailLoaded(usize, Vec<TurnRecord>),
```

在 `update()` 的顶层 `match msg { ... }` 里(挨着 `Message::Todo(...)` 分支群)加:

```rust
            Message::TodoDetailLoaded(idx, turns) => {
                self.with_focused_project(move |ws, _io| {
                    if ws.todo.detail_open_idx() == Some(idx) {
                        ws.todo.replace_detail_turns(turns);
                    }
                });
            }
```

- [ ] **Step 4: 焦点 getter/setter(镜像本会话早些时候修的 `category_rename_focused` 模式)**

在 `category_rename_focused`/`set_category_rename_focused`(`app.rs:3299` 附近,本次会话稍早已经补过 getter 的那处)之后加:

```rust
    /// 详情弹窗回复框是否持有 iced 真实焦点(main.rs 键盘路由用)。
    pub fn detail_reply_focused(&self) -> bool {
        self.active_workspace()
            .is_some_and(|ws| ws.todo.detail_reply_focused())
    }

    /// 每帧渲染循环调用:把 `CaptureDetailReplyFocus` 问到的真实焦点态
    /// 写进当前工作区的 Todo(`main.rs` 键盘路由随后读 `detail_reply_focused`
    /// 消费)。这个输入没有"失焦提交"的语义(提交靠点"处理"按钮,不是
    /// 失焦/回车),所以不需要 `category_rename_focused`/`todo_content_focused`
    /// 那种"失焦边缘触发落盘"的逻辑,直接写回标记位即可。
    pub fn set_detail_reply_focused(&mut self, focused: bool) {
        if let Some(ws) = self.active_workspace_mut() {
            ws.todo.set_detail_reply_focused_flag(focused);
        }
    }
```

- [ ] **Step 5: 每帧渲染循环里接入 `CaptureDetailReplyFocus`(仿 `CaptureCategoryRenameFocus` 的接入点,`main.rs` 里)**

这一步实际改的是 `main.rs`,但和 Task 13 的键盘闸门改动分开提交(这一步是"焦点捕获接入渲染循环",Task 13 是"键盘路由读取该焦点态")——为了让 Task 12 的改动能独立编译测试,把这一步挪到 Task 13 一起做(Task 12 到这里先不改 `main.rs`,Task 13 Step 1 会同时处理焦点捕获接入和键盘闸门两件事)。

- [ ] **Step 6: 新增详情弹窗原生渲染函数**

在 `category_picker_popup`(`app.rs:7369-7445`,本次会话早些时候修过定位 bug 的那个函数)附近加一个新函数(整体走同一套"外层 `Length::Fill` + `padding` 定位、内层定宽卡片"手法,内层卡片换成"任务信息 + 回合列表 + 回复框"):

```rust
    /// 任务详情弹窗:原生 iced 渲染(不复用会话面板的 webview trace——
    /// 那套渲染实际内容在 `dozer://review-trace/host.html` 里,任务详情
    /// 只需要看人类/agent 往来文本,不需要工具调用折叠/trace 可视化,
    /// 塞进一个跟随光标定位、随时开合的原生弹窗里没有必要也不合适)。
    fn todo_detail_popup<'a>(&self) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
        let Some(ws) = self.active_workspace() else {
            return column![].into();
        };
        let Some(idx) = ws.todo.detail_open_idx() else {
            return column![].into();
        };
        let Some(item) = ws.todo.items().get(idx) else {
            return column![].into();
        };

        let header = column![
            text(item.text.clone()).size(byteui::theme::font::subtitle()),
            text(
                item.assigned_agent
                    .map(|a| format!("指派给:{}", a.label()))
                    .unwrap_or_else(|| "未指派".to_string())
            )
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().dim),
        ]
        .spacing(4);

        let mut turns_col = column![].spacing(8);
        for turn in ws.todo.detail_turns() {
            let label = if turn.role == "human" {
                "你".to_string()
            } else {
                item.assigned_agent
                    .map(|a| a.label().to_string())
                    .unwrap_or_else(|| "AI".to_string())
            };
            turns_col = turns_col.push(
                column![
                    text(label)
                        .size(byteui::theme::font::caption())
                        .color(byteui::theme::color::current().gold),
                    text(turn.content.clone())
                        .size(byteui::theme::font::body())
                        .width(Length::Fill),
                ]
                .spacing(2),
            );
        }
        let turns_scroll = Scrollable::new(turns_col)
            .width(Length::Fill)
            .height(Length::Fixed(320.0))
            .direction(scrollable::Direction::Vertical(
                byteui::interaction::scrollbar::scrollbar(),
            ))
            .style(|_t, _s| byteui::interaction::scrollbar::scrollbar_style());

        let reply_box = container(byteui::form::input_text::view(
            "回复...",
            ws.todo.detail_reply_draft(),
            false,
            Some(todo::detail_reply_field_id()),
            false,
            None,
            false,
            |s| Message::Todo(todo::Message::DetailReplyInput(s)),
        ))
        .width(Length::Fill);
        let submit_label = if ws.todo.detail_processing() { "处理中…" } else { "处理" };
        let submit = button(text(submit_label))
            .on_press_maybe(
                (!ws.todo.detail_processing()).then_some(Message::Todo(todo::Message::DetailReplySubmit)),
            )
            .padding([6, 12]);

        let card = column![header, turns_scroll, row![reply_box, submit].spacing(8)]
            .spacing(12)
            .padding(16)
            .width(Length::Fixed(480.0));
        let card = container(card).style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(byteui::theme::color::current().card.into()),
            border: Border {
                color: byteui::theme::color::current().border,
                width: 1.0,
                radius: 8.0.into(),
            },
            ..container::Style::default()
        });

        container(card)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(iced_widget::core::alignment::Horizontal::Center)
            .align_y(iced_widget::core::alignment::Vertical::Center)
            .into()
    }
```

`byteui::form::input_text::view` 的真实签名(`crates/byteui/src/form/input_text.rs:8-17`)是 `view(placeholder: &str, value: &str, secure: bool, id: Option<widget::Id>, highlight: bool, on_submit: Option<Message>, bare: bool, on_input: impl Fn(String) -> Message)`——上面已经按这个顺序写,`on_submit` 传 `None`(回复不认回车提交,只走"处理"按钮,避免耗时长达 10 分钟的 RPC 被误触发)。`todo::detail_reply_field_id()` 是 Task 11 Step 3 新增的公开函数,`extensions::todo` 模块已经在 `app.rs` 里以 `todo::` 别名引入(照抄 `todo::Message::DetailReplyInput`/`todo::category_rename_field_id()` 现有的引用方式)。这个弹窗没有居中定位到光标锚点的需求(不像 `category_picker_popup` 那样要跟着点击位置走)——弹窗内容居中于窗口即可,`align_x`/`align_y` 已经处理。

- [ ] **Step 7: 把新弹窗接入 Todo 面板的 overlay 栈**

`app.rs:7846-7864` 是一条 `if ws.todo.calendar_popup_open() { ... } else if ws.todo.dispatch_popup_open() { ... } else if ws.todo.status_filter_popup_open() { ... } else { stack![base].into() }` 的互斥分支链(每种浮层各自一个 `else if`,`dismiss` 各自对应自己的 Close 消息),不是一个单独的多层 `stack![...]`。在 `} else if ws.todo.dispatch_popup_open() { ... }` 分支(`app.rs:7846-7864`)和 `} else if ws.todo.status_filter_popup_open() {`(`app.rs:7865`)之间插入一个新分支:

```rust
        } else if ws.todo.detail_popup_open() {
            // 任务详情弹窗:窗口级 overlay,原生渲染(不走 wry webview)。
            // 点弹层外任意处经 dismiss 收起,与其它 Todo 浮层同款约定。
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::Todo(todo::Message::DetailClose));
            stack![base, dismiss, self.todo_detail_popup()]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else if ws.todo.status_filter_popup_open() {
```

`todo_detail_popup()` 内部已经处理了"没有打开的任务"这种空态(返回空 `column![]`),不需要像 `todo_dispatch_overlay`/`todo_calendar_overlay` 那样返回 `Option` 再 `match`——这条分支的判断条件 `ws.todo.detail_popup_open()` 本身已经保证走到这里时一定有 `detail_open` 值,直接调用即可。

- [ ] **Step 8: `SessionRow` 加 `task_id`,渲染处加"关联任务"标签**

`conversation.rs::SessionRow`(`conversation.rs:31-38`)加字段:

```rust
pub struct SessionRow {
    pub conversation_id: String,
    pub agent: AgentKind,
    pub last_ts: u64,
    pub display_title: String,
    pub summary: Option<String>,
    pub summary_status: Option<SummaryStatus>,
    /// 该会话关联的 Todo 任务 id(有则来自 `SessionSummaryPayload.task_id`),
    /// 会话面板据此展示"关联任务"标签。`None` = 与任务无关的普通交互式
    /// 会话。
    pub task_id: Option<i64>,
}
```

`SessionRow::from_row`(`conversation.rs:40-49`)加映射:

```rust
    pub fn from_row(c: &ConversationSummary, s: Option<&SessionSummaryPayload>) -> Self {
        Self {
            conversation_id: c.conversation_id.clone(),
            agent: c.agent,
            last_ts: c.last_ts,
            display_title: s
                .map(|s| s.title.clone())
                .unwrap_or_else(|| c.title.clone()),
            summary: s.map(|s| s.summary.clone()),
            summary_status: s.map(|s| s.status),
            task_id: s.and_then(|s| s.task_id),
        }
    }
```

`conversation.rs` 自身的 `from_summary_maps_fields` 测试(约第 65 行起)如果直接构造了 `SessionRow` 字面量,补 `task_id: None,`(核实测试里是不是走 `from_row` 而不是裸字面量——如果是走 `from_row` 则不用改,`from_row` 本身已经在 Step 8 改过)。

真正渲染会话列表每一行的是 `crates/dozer-app/src/workspace.rs::conversation_list_pane`(`workspace.rs:2677` 起,不是 `app.rs`)。副行文案 `sub`(`workspace.rs:2755-2765`,组装成 `"agent · 相对时间"` 或 `"● 当前 · agent · 相对时间"`)已经是"标题下面第二行状态文案"的既有位置,关联任务信息追加进这一行,不新开一行、不引入新的视觉语言。把:

```rust
        let sub = if current {
            format!(
                "● 当前 · {agent_label} · {}",
                relative_time_text(g.last_ts, now_ms)
            )
        } else {
            format!("{agent_label} · {}", relative_time_text(g.last_ts, now_ms))
        };
```

改成:

```rust
        let task_suffix = g
            .task_id
            .and_then(|task_id| {
                ws.todo
                    .items()
                    .iter()
                    .find(|t| t.id == task_id)
                    .map(|t| t.text.as_str())
            })
            .map(|text| {
                let truncated: String = text.chars().take(30).collect();
                if text.chars().count() > 30 {
                    format!(" · 关联任务:{truncated}…")
                } else {
                    format!(" · 关联任务:{truncated}")
                }
            })
            .unwrap_or_default();
        let sub = if current {
            format!(
                "● 当前 · {agent_label} · {}{task_suffix}",
                relative_time_text(g.last_ts, now_ms)
            )
        } else {
            format!("{agent_label} · {}{task_suffix}", relative_time_text(g.last_ts, now_ms))
        };
```

`ws` 在 `conversation_list_pane` 函数体内已经是可用的 `&Workspace` 参数(`ws.conversation_sessions`/`ws.open_transcript_paths()` 等既有用法都在同一个函数里),`ws.todo.items()` 直接可调,不需要额外传参。这里用"标题下面第二行文案追加一段"而不是加一整个新徽章元素,是为了不引入新的视觉语言、不打乱现有 `row![icon, column![title, sub]]` 布局结构。

- [ ] **Step 9: 编译**

Run: `cargo build -p dozer-app 2>&1 | tail -60`
Expected: 应该只剩 Task 13(main.rs 键盘闸门)相关的少量遗留(如果 `detail_reply_focused`/`set_detail_reply_focused` 还没被 `main.rs` 调用,不会报错,只是"未使用"警告——不影响这一步的编译通过判定)。

- [ ] **Step 10: 提交**

```bash
git add crates/dozer-app/src/app.rs crates/dozer-app/src/workspace.rs crates/dozer-app/src/conversation.rs
git commit -m "feat(dozer-app): Todo 指派/详情弹窗路由与原生渲染,会话面板展示关联任务标签"
```

---

## Task 13: dozer-app main.rs——详情回复框焦点接入键盘路由闸门

**Files:**
- Modify: `crates/dozer-app/src/main.rs`

**Interfaces:**
- Consumes: `todo::CaptureDetailReplyFocus`/`todo::take_detail_reply_focused`(Task 11);`App::set_detail_reply_focused`(Task 12)。

- [ ] **Step 1: 每帧渲染循环接入 `CaptureDetailReplyFocus`**

找到 `main.rs` 里 `CaptureCategoryRenameFocus`/`take_category_rename_focused` 被每帧调用、结果写进 `set_category_rename_focused` 的那几行(`main.rs:2452-2462`,本次会话早些时候读过),照抄同款接入详情回复框:

```rust
    if /* Todo 面板可见的既有判定条件,同 CaptureCategoryRenameFocus 那段 */ {
        interface.operate(&mut todo::CaptureDetailReplyFocus);
        let detail_reply_focused = todo::take_detail_reply_focused();
        app.set_detail_reply_focused(detail_reply_focused);
    }
```

具体的"Todo 面板可见"判定条件、`interface.operate` 的确切调用形式,原样照抄 `CaptureCategoryRenameFocus`(`main.rs:2452-2462`)那几行的写法,只是把 `CategoryRenameFocus` 换成 `DetailReplyFocus`、`set_category_rename_focused` 换成 `set_detail_reply_focused`——不要重新设计接入方式。

- [ ] **Step 2: 键盘路由闸门加条件**

在(本次会话早些时候已经改过一次的)`main.rs:1187-1204` 闸门列表里,`|| app.category_rename_focused()`(本次会话稍早刚加的那一行)之后加:

```rust
                || app.category_rename_focused()
                || app.detail_reply_focused()
```

- [ ] **Step 3: 编译**

Run: `cargo build -p dozer-app`
Expected: 通过。这一步之后 Task 12 Step 9 提到的"未使用"警告应该消失。

- [ ] **Step 4: 提交**

```bash
git add crates/dozer-app/src/main.rs
git commit -m "fix(dozer-app): 详情弹窗回复框焦点接入键盘路由闸门,避免按键泄漏进终端"
```

---

## Task 14: 全 workspace 验证 + 手工 GUI 验收

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
Expected: 无需要格式化的文件(若有,运行 `cargo fmt` 后单独提交一次格式化 commit)

- [ ] **Step 5: 手工 GUI 验证清单**

Run: `cargo run -p dozer-app`,本机至少装好一家有 headless 适配器的 CLI(Claude/CodeBuddy/Opencode/V8agent 任一)并能在终端里正常跑通,对照以下清单逐项确认:

- [ ] Todo 卡片点"派发"(或改名后的对应按钮),弹出的是"选 agent 类型"四选一列表,不再是"选活着的 tab"。
- [ ] 选中某个 agent 类型后,任务状态/UI 上有"已指派给 X"的展示,**没有**任何文字被写进任何终端会话。
- [ ] 分类维度找到自动轮询开关入口,打开后:对该分类下已指派的任务,等待一个轮询周期(约 30 秒)内,任务的关联会话出现了 agent 的处理结果(可以在详情弹窗或会话面板里看到)。
- [ ] 点任务卡片"详情"按钮,弹窗展示任务文本、指派的 agent、回合列表(如果还没处理过,回合列表为空,这是正确行为)。
- [ ] 在详情弹窗回复框输入文字,输入内容**没有**泄漏进任何终端会话(重点验收项,对应 Task 13——用本次会话早些时候修过的同类 bug 的验收方法:输入包含空格和中文的文字,确认终端侧毫无反应)。
- [ ] 点"处理"按钮,按钮进入"处理中"态,一段时间后(可能到 10 分钟,建议先用一个能快速完成的简单任务测试)回合列表刷新出 agent 的回复,项目目录下能看到 agent 真的做了写文件/跑命令这类实际操作(如果任务要求了这类操作)。
- [ ] 会话面板的会话列表里,能找到这次任务处理产生的那一行,标题/内容与详情弹窗一致,且展示了"关联任务"标签。
- [ ] 关闭详情弹窗再重新打开同一个任务,回合列表能正确保留(不是每次重开都清空)。
- [ ] 确认 Todo 面板搜索框内嵌的分类筛选下拉(本次会话早些时候修过定位 bug 的那个)在这轮改动后位置依然正常,没有被详情弹窗的改动带歪。

- [ ] **Step 6: 最终提交(若手工验证发现问题并修复)**

若 Step 5 发现问题,回到对应任务修复、补测试、重新走一遍 Step 1-4,确认全绿后提交修复。若手工验证全部通过且没有代码改动,本任务无需额外提交。
