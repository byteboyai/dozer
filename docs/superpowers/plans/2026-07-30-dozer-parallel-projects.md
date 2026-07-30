# Dozer 多项目并行实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 顶栏能并行打开多个项目页签，每个页签背后是完全独立、同时存活的一套状态（文件树/终端会话/预览/对话），切换页签不结束任何会话，只有显式关闭页签才结束该项目下的会话。

**Architecture:** dozerd 侧去掉全局单数"活跃项目"概念，会话获得 `project_id` 归属，daemon 变成纯会话仓库；GUI 侧 `Workspace`（现有类型）瘦身为"单个项目的活状态"，外壳字段搬进新 `App` 容器，`App.projects: HashMap<i64, WorkspaceSlot>` 承载并行页签，懒加载（`Stub`/`Loaded`）解决启动恢复不卡顿。

**Tech Stack:** Rust workspace（`dozer-core`/`dozerd`/`dozer-client`/`dozer-app`），iced 0.14，rusqlite，tokio。

## Global Constraints

- 设计依据：`docs/superpowers/specs/2026-07-30-dozer-parallel-projects-design.md`（下称"设计文档"）。
- 真正并行，不是快捷切换——每个页签独立、同时存活，不结束会话（设计文档 §2）。
- 并行项目数不设硬上限（设计文档 §2）。
- 关闭页签是破坏性操作，结束该项目下所有会话；切换页签不是（设计文档 §2、§4）。
- 启动恢复用懒加载：页签集合整体恢复，只有此前聚焦的那个立即拉取完整状态（设计文档 §2、§5）。
- daemon 变成纯会话仓库，不再维护"活跃项目"概念（设计文档 §4）。
- 颜色只取自 `crates/dozer-app/src/theme.rs` 现有常量，禁止新增硬编码色值。
- `dozer-app` 是纯 `[[bin]]` crate（无 `[lib]` target），`pub` 不豁免 `dead_code` lint——本计划新模块/新类型在未被消费前需要 `#[allow(dead_code)]`。
- **`main` 分支的 `crates/dozer-app/src/{main.rs,workspace.rs}` 当前有另一路并行在做的"浏览器 tab"功能改动尚未提交**（`browser`/`browser_error`/`browser_tab_first` 字段、`Message::Browser*` 消息族、`browser_webviews` 池）。本计划按这些改动已经存在的前提编写；执行前先确认 worktree fork 出来的实际内容，若与本计划描述有出入（字段增减、行号漂移），以实际代码为准，按"意图对齐、不逐字硬套"的原则应用改动——这是本计划全程反复强调的原则，不是走过场。执行阶段走隔离 worktree，避免和这路并行工作互相踩踏。

---

### Task 1: dozer-core 协议改造——SessionInfo 加 project_id，删除活跃项目协议消息

**Files:**
- Modify: `crates/dozer-core/src/protocol.rs`

**Interfaces:**
- Produces：`SessionInfo.project_id: Option<i64>`（新字段，`#[serde(default)]`）；`Request::CreateSession` 新增必填字段 `project_id: i64`；`Request::SetActiveProject`/`Request::GetActiveProject` 删除。

- [ ] **Step 1: 写失败测试**

在 `crates/dozer-core/src/protocol.rs` 的 `mod tests` 里追加：

```rust
    #[test]
    fn session_info_carries_project_id() {
        let info = SessionInfo {
            id: "a".into(),
            name: "n".into(),
            command: "/bin/sh".into(),
            cwd: "/tmp".into(),
            alive: true,
            created_ms: 1,
            agent_state: AgentState::Idle,
            transcript_path: None,
            project_id: Some(7),
        };
        let line = encode_line(&info);
        let back: SessionInfo = decode_line(line.trim()).unwrap();
        assert_eq!(back.project_id, Some(7));
    }

    #[test]
    fn old_session_info_without_project_id_decodes_none() {
        // 迁移期：daemon 重启前已存活的会话首次读出时没有 project_id 字段。
        let old =
            r#"{"id":"a","name":"n","command":"/bin/sh","cwd":"/tmp","alive":true,"created_ms":1}"#;
        let info: SessionInfo = decode_line(old).unwrap();
        assert_eq!(info.project_id, None);
    }

    #[test]
    fn create_session_request_carries_project_id() {
        let req = Request::CreateSession {
            name: "主线".into(),
            command: "/bin/sh".into(),
            args: vec!["-c".into(), "cat".into()],
            cwd: "/tmp".into(),
            cols: 80,
            rows: 24,
            project_id: 3,
        };
        let line = encode_line(&req);
        let back: Request = decode_line(line.trim()).unwrap();
        assert_eq!(back, req);
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-core session_info_carries_project_id old_session_info_without_project_id_decodes_none create_session_request_carries_project_id`
Expected: FAIL（`SessionInfo`/`Request::CreateSession` 还没有 `project_id` 字段，编译错误）。

- [ ] **Step 3: `SessionInfo` 加字段**

把：

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub name: String,
    pub command: String,
    pub cwd: String,
    pub alive: bool,
    pub created_ms: u64,
    /// 会话内 agent 的最新状态；旧协议帧无此字段时回落 Idle。
    #[serde(default)]
    pub agent_state: AgentState,
    /// 当前会话 agent 的 transcript 文件路径（Claude Code JSONL；hook 携带）。
    #[serde(default)]
    pub transcript_path: Option<String>,
}
```

改为：

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub name: String,
    pub command: String,
    pub cwd: String,
    pub alive: bool,
    pub created_ms: u64,
    /// 会话内 agent 的最新状态；旧协议帧无此字段时回落 Idle。
    #[serde(default)]
    pub agent_state: AgentState,
    /// 当前会话 agent 的 transcript 文件路径（Claude Code JSONL；hook 携带）。
    #[serde(default)]
    pub transcript_path: Option<String>,
    /// 会话归属的项目 id（多项目并行；P2a）。`None` 表示迁移期孤儿会话——
    /// daemon 重启前已存活、早于本字段引入时创建的会话，首次读出时没有
    /// 归属信息，不强行捏造一个。
    #[serde(default)]
    pub project_id: Option<i64>,
}
```

- [ ] **Step 4: `Request::CreateSession` 加字段**

把：

```rust
    CreateSession {
        name: String,
        command: String,
        args: Vec<String>,
        cwd: String,
        cols: u16,
        rows: u16,
    },
```

改为：

```rust
    CreateSession {
        name: String,
        command: String,
        args: Vec<String>,
        cwd: String,
        cols: u16,
        rows: u16,
        /// 这个会话属于哪个项目——GUI 侧发起 CreateSession 时必须显式指定，
        /// 不再像单项目时代那样只靠一个隐式全局"当前项目"（P2a）。
        project_id: i64,
    },
```

- [ ] **Step 5: 删除活跃项目相关的 Request/Reply 变体**

把 `Request` 枚举里的：

```rust
    /// 打开一个目录为项目（已存在则更新活跃时间），并置为当前项目。
    OpenProject {
        path: String,
    },
    /// 列出所有项目（按活跃时间倒序）。
    ListProjects,
    /// 置当前项目。
    SetActiveProject {
        id: i64,
    },
    /// 取当前项目（无则 None）。
    GetActiveProject,
```

改为：

```rust
    /// 打开一个目录为项目（已存在则更新活跃时间，返回该项目信息）。
    /// P2a 起不再有"顺带置为当前项目"的副作用——daemon 不维护活跃项目
    /// 概念，"当前显示哪个项目"完全是 GUI 侧的本地状态。
    OpenProject {
        path: String,
    },
    /// 列出所有项目（按活跃时间倒序）。
    ListProjects,
```

`Reply::Project { project: Option<ProjectInfo> }` **保留不动**——`OpenProject` 依然用它做回包，只是不再有别的请求变体消费它。

- [ ] **Step 6: 更新受影响的既有测试**

`project_messages_roundtrip` 测试里引用了 `Request::SetActiveProject`，把这部分删掉，只保留 `OpenProject`/`Reply::Projects`/`Reply::Project` 的往返验证：

```rust
    #[test]
    fn project_messages_roundtrip() {
        let req = Request::OpenProject {
            path: "/repo/x".into(),
        };
        assert_eq!(
            decode_line::<Request>(encode_line(&req).trim()).unwrap(),
            req
        );

        let reply = Reply::Projects {
            projects: vec![ProjectInfo {
                id: 1,
                path: "/repo/x".into(),
                name: "x".into(),
                last_active_ms: 5,
            }],
        };
        let line = encode_line(&reply);
        assert!(line.contains(r#""type":"projects""#));
        assert_eq!(decode_line::<Reply>(line.trim()).unwrap(), reply);

        let reply = Reply::Project { project: None };
        assert_eq!(
            decode_line::<Reply>(encode_line(&reply).trim()).unwrap(),
            reply
        );
    }
```

`request_roundtrips_as_single_json_line` 测试里构造 `Request::CreateSession` 时缺字段会编译不过，补上 `project_id: 1`：

```rust
    #[test]
    fn request_roundtrips_as_single_json_line() {
        let req = Request::CreateSession {
            name: "主线".into(),
            command: "/bin/sh".into(),
            args: vec!["-c".into(), "cat".into()],
            cwd: "/tmp".into(),
            cols: 80,
            rows: 24,
            project_id: 1,
        };
        let line = encode_line(&req);
        assert!(line.ends_with('\n'));
        assert_eq!(line.matches('\n').count(), 1);
        let back: Request = decode_line(line.trim()).unwrap();
        assert_eq!(back, req);
    }
```

- [ ] **Step 7: 编译 + 测试**

Run: `cargo build -p dozer-core && cargo test -p dozer-core`
Expected: 编译通过（`dozer-core` 自身），本 crate 测试全绿。**`dozerd`/`dozer-client`/`dozer-app` 此时会编译失败**（还在用旧的 `CreateSession`/`SetActiveProject`/`GetActiveProject`）——这是预期的，Task 2/3 会修，本 task 不必等它们绿。

- [ ] **Step 8: Commit**

```bash
git add crates/dozer-core/src/protocol.rs
git commit -m "feat(protocol): SessionInfo 加 project_id 归属 + 删除活跃项目协议消息"
```

---

### Task 2: dozerd 内部改造——Session 加 project_id，ProjectStore 去掉活跃项目

**Files:**
- Modify: `crates/dozerd/src/session.rs`
- Modify: `crates/dozerd/src/projects.rs`
- Modify: `crates/dozerd/src/server.rs`

**Interfaces:**
- Consumes: Task 1 的 `SessionInfo.project_id: Option<i64>`、`Request::CreateSession.project_id: i64`。
- Produces：`SessionSpec.project_id: i64`；`ProjectStore::open(path) -> Result<ProjectInfo>`（替代 `open_and_activate`，`set_active`/`active` 方法删除）。

- [ ] **Step 1: `SessionSpec` 加字段，`Session::info()` 带出 project_id**

`crates/dozerd/src/session.rs` 里把：

```rust
#[derive(Debug, Clone)]
pub struct SessionSpec {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub cols: u16,
    pub rows: u16,
}
```

改为：

```rust
#[derive(Debug, Clone)]
pub struct SessionSpec {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub cols: u16,
    pub rows: u16,
    pub project_id: i64,
}
```

`Session::info()` 里把：

```rust
    pub fn info(&self) -> SessionInfo {
        SessionInfo {
            id: self.id.clone(),
            name: self.spec.name.clone(),
            command: self.spec.command.clone(),
            cwd: self.spec.cwd.clone(),
            alive: self.alive.load(Ordering::SeqCst),
            created_ms: self.created_ms,
            agent_state: *self.agent_state.lock().expect("agent_state lock"),
            transcript_path: self.transcript_path.lock().expect("tp lock").clone(),
        }
    }
```

改为：

```rust
    pub fn info(&self) -> SessionInfo {
        SessionInfo {
            id: self.id.clone(),
            name: self.spec.name.clone(),
            command: self.spec.command.clone(),
            cwd: self.spec.cwd.clone(),
            alive: self.alive.load(Ordering::SeqCst),
            created_ms: self.created_ms,
            agent_state: *self.agent_state.lock().expect("agent_state lock"),
            transcript_path: self.transcript_path.lock().expect("tp lock").clone(),
            project_id: Some(self.spec.project_id),
        }
    }
```

- [ ] **Step 2: 修 `session.rs` 测试里的 `spec()` 辅助函数**

`crates/dozerd/src/session.rs` 测试模块里的 `spec(cmd: &str) -> SessionSpec` 辅助函数（多个测试用它构造 `SessionSpec`）现在缺 `project_id` 字段编译不过，补上：

```rust
    fn spec(cmd: &str) -> SessionSpec {
        SessionSpec {
            name: "t".into(),
            command: "/bin/sh".into(),
            args: vec!["-c".into(), cmd.into()],
            cwd: std::env::temp_dir().to_string_lossy().into_owned(),
            cols: 80,
            rows: 24,
            project_id: 1,
        }
    }
```

`zsh_session_gets_zdotdir_injected` 测试里手写的另一处 `SessionSpec` 字面量同样要加 `project_id: 1`。

新增一个测试验证 `info()` 带出 project_id：

```rust
    #[tokio::test]
    async fn info_carries_project_id_from_spec() {
        let s = Session::spawn(SessionSpec {
            project_id: 42,
            ..spec("sleep 5")
        })
        .unwrap();
        assert_eq!(s.info().project_id, Some(42));
        let _ = s.kill();
    }
```

- [ ] **Step 3: 跑测试确认新测试通过、既有测试仍绿**

Run: `cargo test -p dozerd --lib session::`
Expected: 全绿，含新增的 `info_carries_project_id_from_spec`。

- [ ] **Step 4: `ProjectStore` 去掉活跃项目**

`crates/dozerd/src/projects.rs` 顶部模块注释把 "`projects` 表 + `meta` 表（当前项目指针）" 改成 "`projects` 表（不再有活跃项目指针，P2a 起 daemon 变成纯会话仓库）"。

`open()` 建表：把：

```rust
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS projects (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                path TEXT NOT NULL UNIQUE,
                name TEXT NOT NULL,
                last_active_ms INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
             );",
        )
        .context("建表")?;
```

改为（去掉 `meta` 建表）：

```rust
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS projects (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                path TEXT NOT NULL UNIQUE,
                name TEXT NOT NULL,
                last_active_ms INTEGER NOT NULL
             );",
        )
        .context("建表")?;
```

把：

```rust
    /// upsert（按 path）+ 刷新活跃时间 + 置为当前项目，返回该项目。
    pub fn open_and_activate(&self, path: &str) -> Result<ProjectInfo> {
        let conn = self.conn.lock().expect("db lock");
        let ts = next_active_stamp(&conn);
        conn.execute(
            "INSERT INTO projects (path, name, last_active_ms) VALUES (?1, ?2, ?3)
             ON CONFLICT(path) DO UPDATE SET last_active_ms = ?3",
            rusqlite::params![path, basename(path), ts],
        )?;
        let info = conn.query_row(
            "SELECT id, path, name, last_active_ms FROM projects WHERE path = ?1",
            [path],
            row_to_project,
        )?;
        conn.execute(
            "INSERT INTO meta (key, value) VALUES ('active_project_id', ?1)
             ON CONFLICT(key) DO UPDATE SET value = ?1",
            [info.id.to_string()],
        )?;
        Ok(info)
    }

    pub fn list(&self) -> Result<Vec<ProjectInfo>> {
        let conn = self.conn.lock().expect("db lock");
        let mut stmt = conn.prepare(
            "SELECT id, path, name, last_active_ms FROM projects ORDER BY last_active_ms DESC",
        )?;
        let rows = stmt.query_map([], row_to_project)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn set_active(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock().expect("db lock");
        conn.execute(
            "INSERT INTO meta (key, value) VALUES ('active_project_id', ?1)
             ON CONFLICT(key) DO UPDATE SET value = ?1",
            [id.to_string()],
        )?;
        let ts = next_active_stamp(&conn);
        conn.execute(
            "UPDATE projects SET last_active_ms = ?1 WHERE id = ?2",
            rusqlite::params![ts, id],
        )?;
        Ok(())
    }

    pub fn active(&self) -> Result<Option<ProjectInfo>> {
        let conn = self.conn.lock().expect("db lock");
        let id: Option<i64> = conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'active_project_id'",
                [],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .and_then(|s| s.parse().ok());
        let Some(id) = id else { return Ok(None) };
        conn.query_row(
            "SELECT id, path, name, last_active_ms FROM projects WHERE id = ?1",
            [id],
            row_to_project,
        )
        .optional()
        .map_err(Into::into)
    }
```

改为（`open_and_activate` 更名 `open`，去掉写 `meta` 那一段；`set_active`/`active` 整个删除）：

```rust
    /// upsert（按 path）+ 刷新活跃时间，返回该项目。P2a 起不再"置为当前
    /// 项目"——daemon 没有这个概念了，"当前显示哪个"是 GUI 侧本地状态。
    pub fn open(&self, path: &str) -> Result<ProjectInfo> {
        let conn = self.conn.lock().expect("db lock");
        let ts = next_active_stamp(&conn);
        conn.execute(
            "INSERT INTO projects (path, name, last_active_ms) VALUES (?1, ?2, ?3)
             ON CONFLICT(path) DO UPDATE SET last_active_ms = ?3",
            rusqlite::params![path, basename(path), ts],
        )?;
        conn.query_row(
            "SELECT id, path, name, last_active_ms FROM projects WHERE path = ?1",
            [path],
            row_to_project,
        )
        .map_err(Into::into)
    }

    pub fn list(&self) -> Result<Vec<ProjectInfo>> {
        let conn = self.conn.lock().expect("db lock");
        let mut stmt = conn.prepare(
            "SELECT id, path, name, last_active_ms FROM projects ORDER BY last_active_ms DESC",
        )?;
        let rows = stmt.query_map([], row_to_project)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
```

`use rusqlite::{Connection, OptionalExtension};` 这行的 `OptionalExtension` 导入现在没人用了（只有 `active()` 用过 `.optional()`），删掉，改成 `use rusqlite::Connection;`。

- [ ] **Step 5: 更新 `projects.rs` 里的测试**

`open_activate_list_and_persist` 测试整个改写（原来测的是"开二打开活跃指针会跟着变、可以 set_active 切回去"，现在这些语义不存在了，改成测"upsert 幂等 + list 排序 + 持久化"）：

```rust
    #[test]
    fn open_list_and_persist() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.db");
        let store = ProjectStore::open(&db).unwrap();
        assert!(store.list().unwrap().is_empty());

        let a = store.open("/repo/a").unwrap();
        assert_eq!(a.name, "a");

        // 同 path 再开:不新增,复用同 id,活跃时间刷新
        let a2 = store.open("/repo/a").unwrap();
        assert_eq!(a2.id, a.id);
        assert_eq!(store.list().unwrap().len(), 1);

        // 开第二个:两条都在,最近活跃的在前
        let b = store.open("/repo/b").unwrap();
        let list = store.list().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, b.id, "最近活跃在前");

        // 重开库:列表持久化
        drop(store);
        let store = ProjectStore::open(&db).unwrap();
        assert_eq!(store.list().unwrap().len(), 2);
    }
```

`name_is_basename` 测试把 `open_and_activate` 换成 `open`：

```rust
    #[test]
    fn name_is_basename() {
        let dir = tempfile::tempdir().unwrap();
        let store = ProjectStore::open(&dir.path().join("t.db")).unwrap();
        assert_eq!(store.open("/a/b/proj").unwrap().name, "proj");
        assert_eq!(store.open("/").unwrap().name, "/");
    }
```

- [ ] **Step 6: 跑测试**

Run: `cargo test -p dozerd --lib projects::`
Expected: 全绿。

- [ ] **Step 7: `server.rs` 接线**

`crates/dozerd/src/server.rs` 的 `handle_conn` 里，把：

```rust
                        Request::CreateSession { name, command, args, cwd, cols, rows } => {
                            match registry.create(SessionSpec { name, command, args, cwd, cols, rows }) {
                                Ok(s) => Reply::Created { session: s.info() },
                                Err(e) => Reply::Error { message: e.to_string() },
                            }
                        }
```

改为：

```rust
                        Request::CreateSession { name, command, args, cwd, cols, rows, project_id } => {
                            match registry.create(SessionSpec { name, command, args, cwd, cols, rows, project_id }) {
                                Ok(s) => Reply::Created { session: s.info() },
                                Err(e) => Reply::Error { message: e.to_string() },
                            }
                        }
```

把：

```rust
                        Request::OpenProject { path } => match projects.open_and_activate(&path) {
                            Ok(p) => Reply::Project { project: Some(p) },
                            Err(e) => Reply::Error { message: format!("打开项目失败: {e}") },
                        },
                        Request::ListProjects => match projects.list() {
                            Ok(projects) => Reply::Projects { projects },
                            Err(e) => Reply::Error { message: format!("列项目失败: {e}") },
                        },
                        Request::SetActiveProject { id } => match projects.set_active(id) {
                            Ok(()) => match projects.active() {
                                Ok(p) => Reply::Project { project: p },
                                Err(e) => Reply::Error { message: format!("取当前项目失败: {e}") },
                            },
                            Err(e) => Reply::Error { message: format!("置当前项目失败: {e}") },
                        },
                        Request::GetActiveProject => match projects.active() {
                            Ok(p) => Reply::Project { project: p },
                            Err(e) => Reply::Error { message: format!("取当前项目失败: {e}") },
                        },
```

改为：

```rust
                        Request::OpenProject { path } => match projects.open(&path) {
                            Ok(p) => Reply::Project { project: Some(p) },
                            Err(e) => Reply::Error { message: format!("打开项目失败: {e}") },
                        },
                        Request::ListProjects => match projects.list() {
                            Ok(projects) => Reply::Projects { projects },
                            Err(e) => Reply::Error { message: format!("列项目失败: {e}") },
                        },
```

- [ ] **Step 8: 编译 + 全量测试**

Run: `cargo build -p dozerd && cargo test -p dozerd`
Expected: 编译通过，全绿。

- [ ] **Step 9: Commit**

```bash
git add crates/dozerd/src/session.rs crates/dozerd/src/projects.rs crates/dozerd/src/server.rs
git commit -m "feat(dozerd): Session 加 project_id 归属，ProjectStore 去掉活跃项目概念"
```

---

### Task 3: dozer-client 改造

**Files:**
- Modify: `crates/dozer-client/src/lib.rs`

**Interfaces:**
- Consumes: Task 1 的 `Request::CreateSession.project_id`。
- Produces：`Client::create(..., project_id: i64)`（签名变化）；`Client::set_active_project`/`Client::active_project` 删除。

- [ ] **Step 1: `create()` 加参数**

把：

```rust
    pub async fn create(
        &self,
        name: &str,
        command: &str,
        args: &[String],
        cwd: &str,
        cols: u16,
        rows: u16,
    ) -> Result<SessionInfo> {
        match self
            .roundtrip(&Request::CreateSession {
                name: name.into(),
                command: command.into(),
                args: args.to_vec(),
                cwd: cwd.into(),
                cols,
                rows,
            })
            .await?
        {
            Reply::Created { session } => Ok(session),
            other => bail!("意外应答: {other:?}"),
        }
    }
```

改为：

```rust
    #[allow(clippy::too_many_arguments)]
    pub async fn create(
        &self,
        name: &str,
        command: &str,
        args: &[String],
        cwd: &str,
        cols: u16,
        rows: u16,
        project_id: i64,
    ) -> Result<SessionInfo> {
        match self
            .roundtrip(&Request::CreateSession {
                name: name.into(),
                command: command.into(),
                args: args.to_vec(),
                cwd: cwd.into(),
                cols,
                rows,
                project_id,
            })
            .await?
        {
            Reply::Created { session } => Ok(session),
            other => bail!("意外应答: {other:?}"),
        }
    }
```

- [ ] **Step 2: 删除 `set_active_project`/`active_project`**

把这两个方法整个删掉：

```rust
    pub async fn set_active_project(&self, id: i64) -> Result<Option<ProjectInfo>> {
        match self.roundtrip(&Request::SetActiveProject { id }).await? {
            Reply::Project { project } => Ok(project),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn active_project(&self) -> Result<Option<ProjectInfo>> {
        match self.roundtrip(&Request::GetActiveProject).await? {
            Reply::Project { project } => Ok(project),
            other => bail!("意外应答: {other:?}"),
        }
    }
```

- [ ] **Step 3: 编译**

Run: `cargo build -p dozer-client`
Expected: 编译通过。**`dozer-app` 此时仍然编译失败**（还在调旧签名的 `create()`/已删除的 `set_active_project`/`active_project`）——预期中，Task 4-7 处理。

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-client/src/lib.rs
git commit -m "feat(client): create() 加 project_id 参数，删除 set_active_project/active_project"
```

---

### Task 4: dozer-app 新增基础类型——WorkspaceSlot + open_projects.json 持久化（暂不接线）

**Files:**
- Create: `crates/dozer-app/src/open_projects.rs`
- Modify: `crates/dozer-app/src/workspace.rs`（只加 `WorkspaceSlot` 类型定义，不改 `Workspace`/`main.rs` 任何既有逻辑）
- Modify: `crates/dozer-app/src/main.rs`（加 `mod open_projects;`）

**Interfaces:**
- Consumes: 无（纯新增）。
- Produces：`open_projects::OpenProjectsState { project_ids: Vec<i64>, active_project_id: Option<i64> }` + `load()`/`save()`/`load_from()`/`save_to()`；`workspace::WorkspaceSlot { Stub(ProjectInfo), Loaded(Workspace) }`。Task 5 消费这两个类型。

这个 task 的目的是先把新类型和它们自己的持久化逻辑独立写好、测好，不动任何现有代码路径——照抄 `layout.rs`/`preview_state.rs` 已经验证过的"本地 JSON + load/save 往返测试"模式。

- [ ] **Step 1: 读一遍 `layout.rs` 作参照**

`crates/dozer-app/src/layout.rs` 是最贴近的既有模式（本地 JSON、`load`/`save`/`load_from`/`save_to` 四函数、`config_dir()` 定位文件）。新文件按同样结构写，不要另起一套约定。

- [ ] **Step 2: 写 `open_projects.rs` 的失败测试**

```rust
//! 并行项目页签集合的本地持久化——记"当前开了哪些项目、什么顺序、哪个
//! 在前台"，项目自己的路径/名字元数据不重复存，权威数据在 dozerd。

use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct OpenProjectsState {
    pub project_ids: Vec<i64>,
    pub active_project_id: Option<i64>,
}

fn file_path() -> PathBuf {
    dozer_core::paths::config_dir().join("open_projects.json")
}

pub fn load() -> OpenProjectsState {
    load_from(&file_path())
}

pub fn save(state: &OpenProjectsState) -> io::Result<()> {
    save_to(&file_path(), state)
}

fn load_from(path: &Path) -> OpenProjectsState {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_to(path: &Path, state: &OpenProjectsState) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(state).expect("OpenProjectsState 总能序列化");
    std::fs::write(path, json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_from_missing_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nope.json");
        assert_eq!(load_from(&path), OpenProjectsState::default());
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("open_projects.json");
        let state = OpenProjectsState {
            project_ids: vec![3, 1, 2],
            active_project_id: Some(1),
        };
        save_to(&path, &state).unwrap();
        assert_eq!(load_from(&path), state);
    }

    #[test]
    fn load_from_corrupt_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.json");
        std::fs::write(&path, "not json").unwrap();
        assert_eq!(load_from(&path), OpenProjectsState::default());
    }
}
```

（`load_from`/`save_to` 已经在上面写出完整实现，不是先写测试再补实现——这个模块结构简单、直接照抄 `layout.rs` 的验证过的形状，TDD 的"先写失败测试"对这种纯抄写没有额外价值；写完整个文件后直接跑测试验证。）

- [ ] **Step 3: `main.rs` 加模块声明**

在 `crates/dozer-app/src/main.rs` 顶部一串 `mod` 声明里加一行 `mod open_projects;`（按现有字母序插入位置——`mod layout;` 之后、`mod osc;` 之前）。

- [ ] **Step 4: 跑测试**

Run: `cargo test -p dozer-app open_projects::`
Expected: 3 个测试全绿。

- [ ] **Step 5: `workspace.rs` 加 `WorkspaceSlot` 类型**

在 `workspace.rs` 里 `pub struct Workspace {` 定义之前找个合适位置（比如挨着 `MaximizedPane`/`ShellLayout` 这些类型定义的区域）新增：

```rust
/// 单个项目页签的加载状态：懒加载用。`Stub` 只有页签渲染需要的最小信息
/// （启动恢复时,还没被聚焦过的页签停在这一态）,`Loaded` 是完整的
/// `Workspace`（P2a 多项目并行）。
#[allow(dead_code)] // Task 5 接入 App 后消费
pub enum WorkspaceSlot {
    Stub(dozer_core::protocol::ProjectInfo),
    Loaded(Box<Workspace>),
}
```

（`Loaded` 用 `Box<Workspace>` 不是裸 `Workspace`——`Workspace` 本身体量大，`WorkspaceSlot` 会活在 `HashMap` 里，`Box` 避免每次枚举匹配都搬一份大结构体；这不是过早优化，是这个类型的合理默认写法。）

- [ ] **Step 6: 编译**

Run: `cargo build -p dozer-app 2>&1 | grep -c "^error"`
Expected: 错误数量与 Task 3 结束时相同（`WorkspaceSlot` 是纯新增、未被引用的类型，不会让原有编译错误变多或变少；本 task 不修任何既有编译错误，那是 Task 5 的范围）。

- [ ] **Step 7: Commit**

```bash
git add crates/dozer-app/src/open_projects.rs crates/dozer-app/src/main.rs crates/dozer-app/src/workspace.rs
git commit -m "feat(shell): 新增 open_projects.json 持久化 + WorkspaceSlot 懒加载类型(暂未接线)"
```

---

### Task 5: dozer-app 大切换——Workspace 瘦身 + App 接管 main.rs（不可再拆的大任务）

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`（外壳字段搬出 `Workspace`，新增 `App` 类型；`create_session`/`ensure_project_terminal` 等调用点补 `project_id` 参数）
- Modify: `crates/dozer-app/src/main.rs`（`Runner`/`build_workspace` 等持有 `App` 而非 `Workspace`）

**Interfaces:**
- Consumes: Task 3 的 `Client::create(..., project_id: i64)`；Task 4 的 `WorkspaceSlot`。
- Produces：`App` 类型（持有外壳字段 + `projects: HashMap<i64, WorkspaceSlot>` + `project_order: Vec<i64>` + `active_project_id: Option<i64>`）；`App::active_workspace(&self) -> Option<&Workspace>` / `App::active_workspace_mut(&mut self) -> Option<&mut Workspace>`。Task 6/7 消费这些。

**这是本计划里工程量最大、不能再拆的一个任务**——原因和图标栏重排那次的 Task 3 一样：Rust 编译器不允许"只改一半"的中间态。把 `Workspace` 的外壳字段搬进新的 `App` 类型，意味着 `update()`/`view()` 里每一处引用这些字段的地方都要同时改成经 `App` 间接访问，中间任何一刻都是不能编译的状态，没法拆成多个"各自能独立编译"的子任务。

#### 5.1 现状（写计划时读到的实际内容，执行前务必重新核实）

`Workspace` 结构体定义在 `workspace.rs`（写计划时在 842 行附近，实际行号因为并行的浏览器 tab 改动会有漂移）。**外壳字段**（本 task 要搬出去的）：`client`、`handle`、`proxy`、`cols`、`rows`、`term_focused`、`daemon_error`、`blink_on`、`shell_layout`、`left_view`、`right_view`、`left_collapsed`、`right_collapsed`、`maximized`、`window_size`、`dragging`、`context_menu`、`last_right_click`。**项目态字段**（留在 `Workspace` 里，不动）：`tabs`、`active`、`next_tab_id`、`pending`、`preview`、`preview_error`、`browser`、`browser_error`（浏览器 tab 改动加的，同样是项目态，留下）、`allowed_files`、`acceptance`、`review`、`conversations`、`project`、`file_tree`、`branch`、`dirty`、`recent_projects`、`git_statuses`、`project_goal`、`project_acceptance_count`、`term_tab_first`、`preview_tab_first`、`browser_tab_first`（浏览器 tab 改动加的）、`tree_selected`、`tree_clipboard`、`tree_error`、`tree_delete_confirm`、`tree_edit`。

**执行前第一步永远是重新 `grep -n "pub struct Workspace" workspace.rs` 读一遍实际字段列表**，按"是不是这个程序只有一份的窗口/外壳状态"这条标准去分类，不要死记上面这份写计划时的快照——如果并行的浏览器 tab 工作又加了新字段，按同样标准判断它是外壳态还是项目态（`browser`/`browser_error`/`browser_tab_first` 目前判断是项目态，因为每个项目应该有自己独立的浏览器 tab 集合，这点如果和实际代码语义对不上，以代码实际语义为准，不要因为这份计划这么写就不假思索照做）。

#### 5.2 `App` 类型定义

```rust
/// 顶层容器：main.rs 持有的就是这个（取代此前直接持有单个 `Workspace`）。
/// 外壳字段是整个程序只有一份的窗口态，`projects` 承载并行打开的项目
/// 页签——每个 `Workspace` 是完全独立、同时存活的一套项目态（P2a）。
pub struct App {
    client: Client,
    handle: Handle,
    proxy: EventLoopProxy<Message>,
    cols: u16,
    rows: u16,
    term_focused: bool,
    daemon_error: Option<String>,
    blink_on: bool,
    shell_layout: ShellLayout,
    left_view: LeftView,
    right_view: RightView,
    left_collapsed: bool,
    right_collapsed: bool,
    maximized: Option<MaximizedPane>,
    window_size: (f32, f32),
    dragging: Option<Divider>,
    context_menu: Option<ContextMenuState>,
    last_right_click: (f32, f32),

    projects: HashMap<i64, WorkspaceSlot>,
    project_order: Vec<i64>,
    active_project_id: Option<i64>,
}
```

（`ContextMenuState`/`Divider`/`MaximizedPane`/`LeftView`/`RightView`/`ShellLayout` 这些类型名沿用 `workspace.rs` 里已有的实际类型名——写计划时读到的名字，执行时按实际代码核实，类型名如有出入以实际为准。）

`App` 上的核心方法：

```rust
impl App {
    /// 按 `active_project_id` 取当前项目的 `Workspace` 只读引用；`Stub`
    /// 态和"没有任何页签"都返回 `None`（只读场景不促成加载）。
    pub fn active_workspace(&self) -> Option<&Workspace> {
        let id = self.active_project_id?;
        match self.projects.get(&id)? {
            WorkspaceSlot::Loaded(ws) => Some(ws),
            WorkspaceSlot::Stub(_) => None,
        }
    }

    /// 同上，可变引用版本；如果对应槽位是 `Stub`，就地促成 `Loaded`
    /// （拉取该项目的完整状态）后再返回引用。促成逻辑见 Task 6/7。
    pub fn active_workspace_mut(&mut self) -> Option<&mut Workspace> {
        let id = self.active_project_id?;
        self.ensure_loaded(id);
        match self.projects.get_mut(&id)? {
            WorkspaceSlot::Loaded(ws) => Some(ws),
            WorkspaceSlot::Stub(_) => None,
        }
    }
}
```

`ensure_loaded(&mut self, id: i64)` 这个方法本 task **只写签名和一个占位实现**（`Stub → Loaded` 的真正促成逻辑——拉文件树/会话列表/对话——属于 Task 7 的范围，此时项目还没有"打开/切换页签"的消息流，无从测试起）。

这里需要一个"占位用的空 `Workspace`"构造函数。**注意不要用旧的 `Workspace::with_daemon_error`**——那个构造函数原本是"整个单项目时代的 app 连不上 daemon"这个概念的载体，而 `daemon_error` 字段已经按 5.1 的分类搬到 `App` 上了（daemon 连不连得上是整个程序共享的状态，不是某个项目自己的状态），`Workspace::with_daemon_error` 这个概念本身不该在 `Workspace` 瘦身后继续存在——它会在 Task 7 变成 `App::with_daemon_error`（`App` 级别，daemon 连不上时不加载任何项目）。`ensure_loaded` 需要的"占位"是一个完全不同的概念（"这个项目的真实状态还没加载完"，不是"daemon 坏了"），需要一个新的、专门的空构造函数：

```rust
    /// `Stub` 促成 `Loaded` 之前的占位内容:一个"什么都没有"的空
    /// `Workspace`（`project: None`，无 tab）。`App::view()` 按
    /// `ws.project.is_none()` 识别"这是占位,不是真的空项目"，画一条
    /// "加载中…"文案——正常促成过的 `Workspace` 恒有 `project: Some(_)`,
    /// 不会和这个占位混淆。
    fn placeholder_loading() -> Workspace {
        Workspace::empty_for_project_placeholder()
    }
```

（`Workspace::empty_for_project_placeholder()` 是一个更底层的辅助——构造一个字段全部取合理空值的 `Workspace`：`tabs: Vec::new()`、`project: None`、其余项目态字段各自的"空"值。这个函数与 §7.1 的 `Workspace::empty_for_project(...)`是两个不同的东西：后者接收一堆已经算好的真实数据、用于"真正加载完成"的场景；这里的 `empty_for_project_placeholder` 不接收任何参数、纯粹是"占位"，二者不要混用。执行时把这个纯空构造函数写在 `Workspace` 的 `impl` 块里，字段列表对照 Task 5 实际迁移后的 `Workspace` 定义。）

```rust
    fn ensure_loaded(&mut self, id: i64) {
        if let Some(WorkspaceSlot::Stub(_)) = self.projects.get(&id) {
            self.projects.insert(
                id,
                WorkspaceSlot::Loaded(Box::new(Workspace::placeholder_loading())),
            );
        }
    }
```

#### 5.3 迁移 `update()`/`view()`

**机械规则**：`update()`/`view()`（以及 `workspace.rs` 里所有以 `&Workspace`/`&mut Workspace` 为接收者、内部访问了外壳字段的函数）里，原来直接 `self.xxx`（xxx 是 5.1 列出的外壳字段）的地方，要看这段代码此刻的"接收者"是谁：

- 如果这段代码本来就在 `App` 的方法里（比如 `main.rs` 里 `Runner::Ready` 分支直接摸外壳字段的地方），迁移后这些字段就在 `self` 上，**不用改**，因为它们现在就长在 `App` 上。
- 如果这段代码在 `Workspace` 自己的方法里（比如 `Workspace::update`/`Workspace::view` 内部读 `self.shell_layout`），这些访问点全部编译失败——因为字段已经不在 `Workspace` 上了。**这是问题所在，也是本 task 的核心工作**：`Workspace::update`/`Workspace::view` 这两个巨大的函数本身要整体搬到 `App` 上变成 `App::update`/`App::view`，内部对项目态字段的访问（`self.tabs`、`self.preview` 等）不变，对外壳态字段的访问也不变（因为搬过去之后它们本来就在同一个 `self` 上）——**真正需要"经过一层间接"的，是那些 `Workspace` 方法内部既读外壳字段又读项目态字段、但函数本身不方便整体搬走的场景**（如果有），这类场景改成 `App::update`/`App::view` 里先 `let Some(ws) = self.active_workspace_mut() else { return };` 拿到当前项目的 `Workspace`，外壳字段继续 `self.xxx`，项目态字段改成 `ws.xxx`。

具体推荐的迁移顺序（每步之后都编译一次，看报错数量是否符合预期）：

1. 把 `struct Workspace` 里 5.1 列出的外壳字段整体删掉，加进新的 `App` 结构体（5.2 已给出完整定义）。这一步之后编译报错会激增——`Workspace::bootstrap`/`Workspace::with_daemon_error` 两个构造函数字面量里缺字段，`Workspace::update`/`Workspace::view` 内部大量 `self.xxx` 找不到字段。**这是预期的，继续往下走，不要现在就去逐条修。**
2. 把 `impl Workspace { pub fn update(...) }` 和 `impl Workspace { pub fn view(...) }` 这两个方法**整体剪切**，粘到一个新的 `impl App { pub fn update(...) }`/`impl App { pub fn view(...) }` 里。函数体先不改内容，只是换了个"挂在谁身上"。
3. 现在编译报错会集中在这两个搬家后的函数体内部：所有原来隐式引用 `self`（现在是 `App`）里项目态字段的地方（比如 `self.tabs.push(...)`），因为 `tabs` 已经不在 `App` 上、还在 `Workspace` 上。这些地方按前面说的规则改成先 `let Some(ws) = self.active_workspace_mut() else { return };`（`update` 里，消息处理需要项目态时）或 `let Some(ws) = self.active_workspace() else { return placeholder_element(); };`（`view` 里，具体的"没有任何项目打开时画什么"由 Task 6 定，本 task 先返回一个"未打开项目"占位 `Element`，用现有的 `text("未打开任何项目")` 之类最简单的东西即可，不必是最终视觉），再把 `self.tabs` 改成 `ws.tabs`（对整个函数体做一次「区分这行访问的是外壳字段还是项目态字段」的通读，外壳字段留 `self.`，项目态字段改 `ws.`）。
4. `Workspace::bootstrap` 构造函数：外壳字段那部分（`client`/`handle`/`proxy`/`cols`/`rows` 等）从 `Self { ... }` 字面量里删掉（不再是 `Workspace` 的字段）——但函数参数列表**保留** `client: Client, handle: Handle, proxy: EventLoopProxy<Message>`，因为函数体内部仍然要用它们做真正的异步恢复工作（`client.list()`/`client.attach()`/`handle.spawn(forward_events(...))`），只是不再把它们**存成 `Workspace` 自己的字段**，用完即弃。`Workspace::with_daemon_error` 这个构造函数**整个概念不再需要**——它原本代表的"daemon 连不上"是 `daemon_error` 字段（已搬到 `App`）驱动的整个程序级状态，不是某个项目自己的状态；这一步直接删掉这个函数，如果编译器提示还有地方在调用它，那些调用点本身也需要按"这其实是 App 级别的场景"去改（Task 7 会补 `App::with_daemon_error`，本 task 暂时可以用一个最简单的 `Workspace::placeholder_loading()`——本节稍后 5.2 已经给出定义——作为过渡期占位，让编译先通过）。
5. `main.rs` 的 `Runner::Loading(Option<Workspace>)`/`Ready { ..., workspace: Workspace, ... }` 改成持有 `App` 而不是 `Workspace`；`build_workspace(...)` 函数改造成构造 `App`。**本 task 只需要让它编译通过，不需要语义完全正确**——`App::bootstrap`/`App::with_daemon_error` 这两个真正的 `App` 级别构造函数是 Task 7 的产出（Task 7 §7.5 会整个替换掉 `build_workspace` 函数体）；本 task 过渡期可以先写一个最简单的、能编译的版本，比如 `App` 的所有外壳字段填合理默认值、`projects`/`project_order` 留空、`active_project_id: None`，不接真正的 daemon 探测/会话恢复逻辑——那些逻辑本来就属于 Task 7。
6. `workspace.update(...)`/`workspace.view()` 这些调用点（`main.rs` 里散落多处）全部改成 `app.update(...)`/`app.view()`。

#### 5.4 `create_session`/`ensure_project_terminal` 补 `project_id`

`workspace.rs` 里任何调用 `client.create(...)` 的地方（`ensure_project_terminal`、`spawn_new_tab` 等），现在 `Client::create` 多了一个必填的 `project_id: i64` 参数（Task 3 的产出）。这些调用点原来在 `impl Workspace` 内部，迁移后如果 `Workspace` 本身不知道自己是"哪个项目"——**这是一个真实的缺口**：`Workspace.project: Option<ProjectInfo>` 这个字段本来就留在 `Workspace` 上（5.1 里项目态字段列表），`project.as_ref().map(|p| p.id)` 就是这个 `Workspace` 对应的 project_id，调用 `client.create(...)` 时传这个值即可（`.expect("Workspace 存在即已知归属项目")`——如果 `Workspace` 存在于 `App.projects` 这个 map 里，它必然有归属项目，这个 `expect` 不该失败，失败说明前面某处逻辑错了，值得让它 panic 而不是静默吞掉）。

#### 5.5 编译预期与自检

Run: `cargo build -p dozer-app 2>&1 | grep -c "^error"`

**如果这一步的报错数量远超预期（比如几十上百条），先停下来检查 5.3 的 Step 1-6 是不是有遗漏**，而不是逐条硬修到编译通过为止——大量报错通常意味着某个前置 Step 漏做了（比如某处还在直接 `self.field` 而没有改成 `ws.field`），不是"这个任务本来就该改这么多地方"。这条经验是图标栏重排那次 Task 3 总结出来的，同样适用于这里。

- [ ] **Step 1-6**：按 5.3 的迁移顺序逐步执行，每步后 `cargo build -p dozer-app` 一次，确认报错数量符合预期（递减，不是递增或原地不动）。
- [ ] **Step 7**: 全部编译通过后，跑 `cargo test -p dozer-app`。**预期大量测试失败或编译不过**——`workspace.rs` 里现有测试大量直接构造 `Workspace`/调用 `Workspace::update`（比如很多几何测试），这些测试本身也要跟着搬家逻辑调整（能直接测 `Workspace` 部分的留在 `Workspace` 测试里；测到外壳字段/`App` 级别行为的挪到新的 `App` 测试或本 task 先注释掉标记 `// TODO(Task 6/7)`——不，计划不允许 TODO 占位，所以：**凡是测试因为字段搬家编译不过，本 task 负责让它们重新编译通过**（改成从正确的类型上取字段），凡是测试的语义因为"现在有多个 Workspace 并行"而不再成立（比如断言"切换项目会关闭所有终端"这类和新设计直接矛盾的测试），本 task 负责删除并在 commit message 里说明删了什么、为什么。
- [ ] **Step 8**: `cargo clippy -p dozer-app --all-targets` 和 `cargo fmt -p dozer-app -- --check` 跑绿。

- [ ] **Step 9: Commit**

```bash
git add crates/dozer-app/src/workspace.rs crates/dozer-app/src/main.rs
git commit -m "feat(shell): Workspace 瘦身为单项目态,新增 App 容器接管外壳字段(P2a 大切换)"
```

---

### Task 6: 打开/切换/关闭项目页签 + 顶栏 UI（含代理状态指示器）

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`（`App::update` 新增页签相关 `Message` 分支；`App::view` 顶栏加页签渲染）

**Interfaces:**
- Consumes: Task 5 的 `App`/`WorkspaceSlot`/`active_workspace_mut`。
- Produces：`Message::ProjectTabOpen(PathBuf)`、`Message::ProjectTabOpened(ProjectInfo)`、`Message::ProjectTabSwitch(i64)`、`Message::ProjectTabClose(i64)`。

- [ ] **Step 1: `Message` 新增变体**

在 `Message` 枚举里紧邻既有 `Project*` 变体之后新增：

```rust
    /// 顶栏"+"或"最近项目"点开一个项目为新页签(main.rs rfd 选中后回送,
    /// 或从最近项目列表点击)。
    ProjectTabOpen(PathBuf),
    /// `ProjectTabOpen` 异步完成:daemon upsert 返回的项目信息。
    ProjectTabOpened(dozer_core::protocol::ProjectInfo),
    /// 点已存在的项目页签:切到该项目,不结束任何会话。
    ProjectTabSwitch(i64),
    /// 点项目页签的 ×:关闭该页签,结束该项目下所有会话。
    ProjectTabClose(i64),
```

- [ ] **Step 2: `App::update` 新增分支**

```rust
            Message::ProjectTabOpen(path) => {
                let client = self.client.clone();
                let proxy = self.proxy.clone();
                let path_s = path.to_string_lossy().into_owned();
                self.handle.spawn(async move {
                    if let Ok(Some(info)) = client.open_project(&path_s).await {
                        let _ = proxy.send_event(Message::ProjectTabOpened(info));
                    }
                });
            }
            Message::ProjectTabOpened(info) => {
                if self.projects.contains_key(&info.id) {
                    // 同一个项目已经开着页签了:只是切过去,不新建。
                    self.active_project_id = Some(info.id);
                } else {
                    let ws = Workspace::bootstrap_for_project(
                        self.client.clone(),
                        self.handle.clone(),
                        self.proxy.clone(),
                        info.clone(),
                    );
                    self.projects
                        .insert(info.id, WorkspaceSlot::Loaded(Box::new(ws)));
                    self.project_order.push(info.id);
                    self.active_project_id = Some(info.id);
                }
                self.spawn_open_projects_save();
            }
            Message::ProjectTabSwitch(id) => {
                if self.projects.contains_key(&id) {
                    self.active_project_id = Some(id);
                    self.ensure_loaded(id);
                    self.sync_terminal_grid();
                }
                self.spawn_open_projects_save();
            }
            Message::ProjectTabClose(id) => {
                if let Some(slot) = self.projects.remove(&id) {
                    let session_ids: Vec<String> = match &slot {
                        WorkspaceSlot::Loaded(ws) => {
                            ws.tabs.iter().map(|t| t.info.id.clone()).collect()
                        }
                        WorkspaceSlot::Stub(_) => Vec::new(),
                    };
                    let client = self.client.clone();
                    self.handle.spawn(async move {
                        for sid in session_ids {
                            if let Err(e) = client.kill(&sid).await {
                                tracing::warn!(session = %sid, "关闭项目页签时结束会话失败: {e}");
                            }
                        }
                    });
                    self.project_order.retain(|pid| *pid != id);
                    if self.active_project_id == Some(id) {
                        self.active_project_id = self.project_order.last().copied();
                    }
                }
                self.spawn_open_projects_save();
            }
```

`Workspace::bootstrap_for_project(client, handle, proxy, info) -> Workspace` 是 Task 5 里 `Workspace::bootstrap` 收缩后的兄弟构造函数——`bootstrap`（无参数版）用于"启动时恢复上次聚焦的那个项目"，`bootstrap_for_project`（多一个 `info: ProjectInfo` 参数）用于"用户主动打开一个新项目页签"，两者内部共享的"按 project_id 过滤会话、拉文件树/git/对话"逻辑应该是同一个私有辅助函数，签名类似：

```rust
    /// 供 `bootstrap`/`bootstrap_for_project`/`ensure_loaded` 共用:按给定
    /// 项目信息构造一个完整加载的 `Workspace`(拉会话/文件树/git/对话)。
    async fn load_for_project(
        client: Client,
        handle: Handle,
        proxy: EventLoopProxy<Message>,
        project: dozer_core::protocol::ProjectInfo,
    ) -> Workspace {
        // 按 project.id 过滤 client.list() 拿到的会话、重挂存活会话、
        // 没有存活会话则新建一个终端 tab、拉文件树/git 状态/对话列表——
        // 具体实现复用 Task 5 迁移后 Workspace::bootstrap 里已经写好的
        // 那部分逻辑(过滤条件从"全部"改成"project_id == project.id")。
        todo!("Task 7 补完整实现")
    }
```

（`bootstrap_for_project` 本 task 先给出函数签名和调用点，内部实现指向 `load_for_project`——**完整的"促成"逻辑属于 Task 7**，因为需要 §7 的懒加载/启动恢复流程一起设计测试用例，本 task 交付的是页签开关/切换的消息流通不通，不是加载语义的正确性。这不是"计划里留 TODO"——`todo!()` 宏本身会让编译通过但运行时 panic，这里明确写出"这是本 task 交付边界，下一 task 补"，且不影响本 task 自己的测试全部聚焦在"消息分支路由对不对"上，不依赖 `load_for_project` 的真正行为。）

- [ ] **Step 3: 顶栏页签渲染**

`App::view` 里原来渲染顶栏搜索框的地方（`top_bar` 函数，Task 5 迁移后应该已经是 `App` 的方法或者独立函数、接收 `&App`），改成渲染项目页签：

```rust
/// 顶栏项目页签行:每个页签是项目名 + 代理状态指示点 + 关闭按钮,末尾一个
/// "+"打开新项目。复用 `preview_pane`/终端 `tab_bar` 已经在用的"索引窗口化
/// + clip + 左右箭头"溢出模式(设计文档 §6),不新发明滚动交互。
fn project_tabs_row(app: &App) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let items: Vec<Element<'_, Message, iced_widget::Theme, iced_widget::Renderer>> = app
        .project_order
        .iter()
        .filter_map(|id| {
            let slot = app.projects.get(id)?;
            let (name, dot) = match slot {
                WorkspaceSlot::Stub(info) => (info.name.clone(), None),
                WorkspaceSlot::Loaded(ws) => (
                    ws.project
                        .as_ref()
                        .map(|p| p.name.clone())
                        .unwrap_or_default(),
                    project_tab_dot_color(ws),
                ),
            };
            let active = app.active_project_id == Some(*id);
            let mut row_children: Vec<Element<'_, Message, iced_widget::Theme, iced_widget::Renderer>> =
                Vec::new();
            if let Some(color) = dot {
                row_children.push(
                    container(text("●").size(10).color(color))
                        .into(),
                );
            }
            row_children.push(text(name).size(13).color(theme::CREAM).into());
            row_children.push(
                button(text("×").size(13).color(theme::DIM))
                    .on_press(Message::ProjectTabClose(*id))
                    .style(|_t, _s| button::Style {
                        background: None,
                        text_color: theme::DIM,
                        ..button::Style::default()
                    })
                    .into(),
            );
            let content = row(row_children).spacing(4).align_y(iced_widget::core::Alignment::Center);
            Some(
                button(content)
                    .on_press(Message::ProjectTabSwitch(*id))
                    .style(move |_t, _s| button::Style {
                        background: Some(if active { theme::CARD.into() } else { theme::PANEL.into() }),
                        border: Border {
                            color: if active { theme::GOLD } else { theme::BORDER },
                            width: 1.0,
                            radius: 6.0.into(),
                        },
                        ..button::Style::default()
                    })
                    .padding([4, 8])
                    .into(),
            )
        })
        .collect();
    let add_btn = button(text("+").size(14).color(theme::CREAM))
        .on_press(Message::ProjectPickFolder)
        .style(|_t, _s| button::Style {
            background: Some(theme::CARD.into()),
            text_color: theme::CREAM,
            border: Border {
                color: theme::BORDER,
                width: 1.0,
                radius: 4.0.into(),
            },
            ..button::Style::default()
        });
    row(items)
        .push(add_btn)
        .spacing(4)
        .align_y(iced_widget::core::Alignment::Center)
        .into()
}

/// 页签指示点颜色:取该项目所有存活会话里最值得关注的状态(设计文档 §6
/// 定的优先级——TurnEnded(金) > AwaitingInput(紫) > Running(绿,闪) >
/// Idle(绿) > 无存活会话(不画点))。复用既有 `dot_color` 的颜色语义,不新
/// 造一套。
fn project_tab_dot_color(ws: &Workspace) -> Option<Color> {
    let alive_states: Vec<AgentState> = ws
        .tabs
        .iter()
        .filter(|t| t.alive)
        .map(|t| t.agent_state)
        .collect();
    if alive_states.iter().any(|s| *s == AgentState::TurnEnded) {
        return Some(theme::GOLD);
    }
    if alive_states.iter().any(|s| *s == AgentState::AwaitingInput) {
        return Some(theme::PURPLE);
    }
    if alive_states.iter().any(|s| *s == AgentState::Running) {
        return Some(theme::GREEN);
    }
    if !alive_states.is_empty() {
        return Some(theme::GREEN);
    }
    None
}
```

（顶栏的"页签溢出走索引窗口化"——本 task 先用一个不做窗口化的简单 `row`（现实中项目页签数量比文件/终端 tab 少得多，先用最简单的实现让功能可用；如果要做完整的窗口化溢出，是一个后续可以单独补的小任务，不阻塞本 task 交付核心功能，设计文档 §6 的"复用既有模式"这条留在 Recommendations/后续里，不在本 task 强制实现——**如果你觉得这条裁剪不对，先跟人类伙伴确认，不要擅自决定要不要做**。）

- [ ] **Step 4: 写 `project_tab_dot_color` 的单元测试**

```rust
    #[test]
    fn project_tab_dot_color_priority() {
        // 构造带不同 agent_state 组合的 tabs,验证优先级。
        // (需要一个能快速构造 SessionTab 的测试辅助——如果 workspace.rs
        // 测试模块已有类似辅助函数,复用它;没有就在这里新增一个最小的。)
        let mut ws = test_workspace_with_no_tabs(); // 复用/新增测试辅助
        assert_eq!(project_tab_dot_color(&ws), None, "无存活会话不画点");

        push_test_tab(&mut ws, AgentState::Idle, true);
        assert_eq!(project_tab_dot_color(&ws), Some(theme::GREEN));

        push_test_tab(&mut ws, AgentState::AwaitingInput, true);
        assert_eq!(
            project_tab_dot_color(&ws),
            Some(theme::PURPLE),
            "AwaitingInput 优先级高于 Idle"
        );

        push_test_tab(&mut ws, AgentState::TurnEnded, true);
        assert_eq!(
            project_tab_dot_color(&ws),
            Some(theme::GOLD),
            "TurnEnded 优先级最高"
        );
    }
```

（`test_workspace_with_no_tabs`/`push_test_tab` 两个测试辅助函数——执行时先 grep `workspace.rs` 测试模块看有没有已经存在的、语义相近的辅助函数可以直接复用或者小改；如果没有，按最小化原则新增，只提供这个测试需要的字段，不要为了"以防万一"添加用不到的参数。）

- [ ] **Step 5: 编译 + 测试**

Run: `cargo build -p dozer-app && cargo test -p dozer-app && cargo clippy -p dozer-app --all-targets 2>&1 | grep -E "warning|error" || echo clean && cargo fmt -p dozer-app -- --check`
Expected: 全绿/clean/无输出。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "feat(shell): 顶栏项目页签(打开/切换/关闭) + 代理状态指示器"
```

---

### Task 7: 懒加载促成逻辑 + 启动恢复 + 整体回归

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`（补完 `App::ensure_loaded`/`Workspace::load_for_project`/`Workspace::bootstrap`/`Workspace::bootstrap_for_project` 的真正实现）

**Interfaces:**
- Consumes: Task 4 的 `open_projects::{load, save}`；Task 6 的 `Workspace::bootstrap_for_project`/`load_for_project` 签名。
- Produces：`App::bootstrap(client, handle, proxy) -> App`（取代 Task 5 临时版本，main.rs 改调这个）。

- [ ] **Step 1: `Workspace::load_for_project` 补真正实现**

把 Task 6 留的 `todo!()` 换成真正逻辑——按 `project_id` 过滤 `client.list()`、重挂存活会话、没有存活会话就新建一个、拉文件树/git/对话/验收计数。这部分逻辑今天已经在 Task 5 迁移后的 `Workspace::bootstrap`（单项目版）里写过一遍（原 `bootstrap` 函数体的"重挂会话 + 拉各项状态"那部分），把它抽成这个共享函数，`bootstrap`/`bootstrap_for_project`/`ensure_loaded` 都调它，不要写三份重复逻辑：

```rust
    async fn load_for_project(
        client: Client,
        handle: Handle,
        proxy: EventLoopProxy<Message>,
        project: dozer_core::protocol::ProjectInfo,
    ) -> Workspace {
        let mut tabs = Vec::new();
        let mut next_tab_id = 0usize;
        if let Ok(sessions) = client.list().await {
            for info in sessions
                .into_iter()
                .filter(|s| s.alive && s.project_id == Some(project.id))
            {
                let tab_id = next_tab_id;
                next_tab_id += 1;
                if let Ok((snapshot, _next_offset, rx)) = client.attach(&info.id, 0).await {
                    let mut model = TerminalModel::new(DEFAULT_COLS, DEFAULT_ROWS);
                    let _ = model.feed(&snapshot);
                    let forwarder = handle.spawn(forward_events(tab_id, rx, proxy.clone()));
                    tabs.push(SessionTab {
                        agent_state: info.agent_state,
                        transcript_path: info.transcript_path.clone(),
                        info,
                        model,
                        alive: true,
                        tab_id,
                        forwarder,
                        osc: OscScanner::new(),
                        cwd: None,
                        last_exit: None,
                        delivery_pending: false,
                        last_turn_head: None,
                    });
                    if let Some(t) = tabs.last_mut() {
                        t.ingest_osc(&snapshot);
                    }
                }
            }
        }
        let file_tree = FileTree::new(PathBuf::from(&project.path));
        let project_goal = load_project_goal(&project.path);
        let mut ws = Workspace::empty_for_project(
            client, handle, proxy, project, tabs, next_tab_id, file_tree, project_goal,
        );
        if ws.tabs.is_empty() {
            ws.ensure_project_terminal();
        }
        ws.restore_preview_state();
        ws.spawn_project_git_refresh();
        ws.spawn_conversations_refresh();
        ws.spawn_acceptance_count_refresh();
        ws
    }
```

（`Workspace::empty_for_project(...)` 是一个纯构造函数——把 Task 5 迁移后 `Workspace::bootstrap`/`with_daemon_error` 里那个巨大的 `Self { ... }` 字面量抽出来，接收上面这些已经算好的值，返回一个字段齐全的 `Workspace`。这一步是把"哪些字段该填什么初始值"这个逻辑去重，签名和字段列表按 Task 5 实际迁移后的 `Workspace` 结构体来定，不在这里重复整个字段列表——执行时直接对照 Task 5 交付的 `Workspace` 定义写。）

- [ ] **Step 2: `App::ensure_loaded` 换成真正促成**

把 Task 5 里那个用 `Workspace::placeholder_loading()` 直接同步促成的占位实现：

```rust
    fn ensure_loaded(&mut self, id: i64) {
        if let Some(WorkspaceSlot::Stub(_)) = self.projects.get(&id) {
            self.projects.insert(
                id,
                WorkspaceSlot::Loaded(Box::new(Workspace::placeholder_loading())),
            );
        }
    }
```

改为真正调用 `load_for_project`——但 `ensure_loaded` 是同步方法（`&mut self`，不是 `async`），而 `load_for_project` 是异步的，二者矛盾。**处理方式**：`ensure_loaded` 改成两步——先立刻把槽位换成 `Workspace::placeholder_loading()` 占位（避免同一帧内 `active_workspace_mut()` 拿到 `None`；这个占位构造函数 Task 5 §5.2 已经定义好，本 task 直接复用，不用改），同时 `self.handle.spawn` 一个异步任务跑真正的 `load_for_project`，完成后通过 `proxy.send_event` 送回一个新消息 `Message::ProjectTabLoaded(id, Workspace)` 把占位换成真正加载好的内容：

```rust
    fn ensure_loaded(&mut self, id: i64) {
        let Some(WorkspaceSlot::Stub(info)) = self.projects.get(&id) else {
            return; // 已经是 Loaded,或者 id 根本不在,都不用做什么
        };
        let info = info.clone();
        let client = self.client.clone();
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        // 立刻换成"加载中"占位,避免本帧内 active_workspace_mut() 拿到 None
        // ——用户点开一个 Stub 页签的那一刻,画面应该马上有反应(哪怕只是
        // "加载中"),而不是要等异步任务跑完才有任何视觉变化。
        self.projects.insert(
            id,
            WorkspaceSlot::Loaded(Box::new(Workspace::placeholder_loading())),
        );
        self.handle.spawn(async move {
            let ws = Workspace::load_for_project(client, handle, proxy.clone(), info).await;
            let _ = proxy.send_event(Message::ProjectTabLoaded(id, Box::new(ws)));
        });
    }
```

在 `Message` 枚举里新增：

```rust
    /// `ensure_loaded` 异步促成完成:把占位换成真正加载好的 `Workspace`。
    ProjectTabLoaded(i64, Box<Workspace>),
```

在 `App::update` 里新增分支：

```rust
            Message::ProjectTabLoaded(id, ws) => {
                if self.projects.contains_key(&id) {
                    self.projects.insert(id, WorkspaceSlot::Loaded(ws));
                    if self.active_project_id == Some(id) {
                        self.sync_terminal_grid();
                    }
                }
            }
```

（`id` 不在 `self.projects` 里的情况——用户点开一个页签又在异步加载完成前把它关掉了，这时 `ProjectTabLoaded` 到达时目标 id 已经不存在，直接丢弃这次结果，不报错，符合"用户已经不关心这个项目了"的直觉。）

- [ ] **Step 3: `Workspace::bootstrap`/`bootstrap_for_project` 改调共享函数**

`bootstrap_for_project`（供 `Message::ProjectTabOpened` 用）：

```rust
    pub async fn bootstrap_for_project(
        client: Client,
        handle: Handle,
        proxy: EventLoopProxy<Message>,
        project: dozer_core::protocol::ProjectInfo,
    ) -> Workspace {
        Self::load_for_project(client, handle, proxy, project).await
    }
```

`App::bootstrap`（取代 Task 5 临时版本，`main.rs` 的 `build_workspace` 改调这个）：

```rust
    pub async fn bootstrap(
        client: Client,
        handle: Handle,
        proxy: EventLoopProxy<Message>,
    ) -> App {
        let shell_layout = layout::load();
        let open_state = open_projects::load();
        let known_projects = client.list_projects().await.unwrap_or_default();
        let known_ids: std::collections::HashSet<i64> =
            known_projects.iter().map(|p| p.id).collect();

        let mut projects = HashMap::new();
        let mut project_order = Vec::new();
        for id in &open_state.project_ids {
            // daemon 已经不认识的项目(比如被删过)直接跳过,不显示这个页签
            // (设计文档 §5)。
            let Some(info) = known_projects.iter().find(|p| p.id == *id) else {
                continue;
            };
            projects.insert(*id, WorkspaceSlot::Stub(info.clone()));
            project_order.push(*id);
        }

        let active_project_id = open_state
            .active_project_id
            .filter(|id| known_ids.contains(id))
            .or_else(|| project_order.first().copied());

        let mut app = App {
            client: client.clone(),
            handle: handle.clone(),
            proxy: proxy.clone(),
            cols: DEFAULT_COLS,
            rows: DEFAULT_ROWS,
            term_focused: true,
            daemon_error: None,
            blink_on: true,
            left_view: shell_layout.left_view,
            right_view: shell_layout.right_view,
            left_collapsed: shell_layout.left_collapsed,
            right_collapsed: shell_layout.right_collapsed,
            shell_layout,
            maximized: None,
            window_size: INITIAL_WINDOW_SIZE,
            dragging: None,
            context_menu: None,
            last_right_click: (0.0, 0.0),
            projects,
            project_order,
            active_project_id,
        };

        // 只有此前聚焦的那一个立即促成 Loaded,其余保持 Stub,点开时才加载
        // (设计文档 §2 的懒加载策略——不设硬上限 + 重启不卡顿的组合诉求)。
        if let Some(id) = app.active_project_id {
            app.ensure_loaded(id);
        }
        app
    }
```

- [ ] **Step 4: 关闭页签/切换页签写盘**

`App::spawn_open_projects_save`（Task 6 已经在 `ProjectTabOpened`/`ProjectTabSwitch`/`ProjectTabClose` 里调用了这个方法名，本 task 补实现）：

```rust
    fn spawn_open_projects_save(&self) {
        let state = open_projects::OpenProjectsState {
            project_ids: self.project_order.clone(),
            active_project_id: self.active_project_id,
        };
        self.handle.spawn(async move {
            if let Err(e) = open_projects::save(&state) {
                tracing::warn!("并行项目页签集合写盘失败: {e}");
            }
        });
    }
```

- [ ] **Step 5: `App::with_daemon_error` + `main.rs` 改调 `App::bootstrap`**

`Workspace::with_daemon_error` 在 Task 5 已经整个删除（那个概念现在是 `App` 级别的，`daemon_error` 字段本来就搬到了 `App` 上）。`main.rs` 的降级路径需要的 `App::with_daemon_error`——daemon 连不上时，不应该尝试加载任何项目，`App` 应该是空的 `projects`/`project_order`/`active_project_id: None`，只留 `daemon_error` 有值给 `view()` 画错误文案——这个函数此前没有定义过，本 task 补上：

```rust
    pub fn with_daemon_error(
        client: Client,
        handle: Handle,
        proxy: EventLoopProxy<Message>,
        message: String,
    ) -> App {
        App {
            client,
            handle,
            proxy,
            cols: DEFAULT_COLS,
            rows: DEFAULT_ROWS,
            term_focused: true,
            daemon_error: Some(message),
            blink_on: true,
            left_view: LeftView::Files,
            right_view: RightView::Agent,
            left_collapsed: false,
            right_collapsed: false,
            shell_layout: ShellLayout::default(),
            maximized: None,
            window_size: INITIAL_WINDOW_SIZE,
            dragging: None,
            context_menu: None,
            last_right_click: (0.0, 0.0),
            projects: HashMap::new(),
            project_order: Vec::new(),
            active_project_id: None,
        }
    }
```

（字段列表和默认值对照 Task 5 交付的 `App` 结构体定义——如果 Task 5 实际迁移后 `App` 的外壳字段集合与本计划 5.2 描述的有出入，这里的字面量按实际字段调整，缺一个补一个，不要因为这份计划写了这些就假设字段列表不会变。）

`main.rs` 里 `build_workspace` 函数（Task 5 已经改造成构造 `App`）现在改调 `App::bootstrap`/`App::with_daemon_error`：

```rust
async fn build_workspace(
    client: dozer_client::Client,
    handle: tokio::runtime::Handle,
    proxy: winit::event_loop::EventLoopProxy<Message>,
) -> App {
    match ensure_daemon(&client).await {
        Ok(()) => App::bootstrap(client, handle, proxy).await,
        Err(message) => App::with_daemon_error(client, handle, proxy, message),
    }
}
```

- [ ] **Step 6: 全量回归**

Run: `cargo build --all-targets && cargo test --workspace && cargo clippy --all-targets && cargo fmt --all -- --check`
Expected: 整个 workspace（不只是 `dozer-app`）编译/测试/lint 全绿。

- [ ] **Step 7: 真机验收清单（留用户，逐条过一遍本计划的核心场景）**

- [ ] 顶栏能打开多个项目页签，各自独立显示文件树/终端/预览/对话。
- [ ] 查看项目 B 时，项目 A 的终端里 agent 仍在跑（不因为切走而结束）；切回 A，会话还在。
- [ ] 关闭一个项目页签，该项目下所有终端会话结束（daemon 侧确认进程真的退出）。
- [ ] 页签上能看到代理状态指示点，颜色随该项目里的会话状态变化。
- [ ] 重启 app：之前打开的页签集合恢复；只有重启前聚焦的那个立即可用，其余页签点开时能正常加载（不报错、不空白）。
- [ ] 打开一个已经开着的项目（同路径）：切到已有页签，不产生重复页签。
- [ ] 既有单项目流程（项目树、预览、终端、验收、⌘K 一类）未受影响（抽样检查，不用逐项过之前所有计划的验收单）。

- [ ] **Step 8: Commit**

```bash
git add crates/dozer-app/src/workspace.rs crates/dozer-app/src/main.rs
git commit -m "feat(shell): 懒加载促成逻辑 + 启动恢复(P2a 多项目并行收尾)"
```

---

## 自检记录（写计划时）

- **Spec 覆盖**：设计文档 §2(范围裁剪)→ Task 6/7 的实现边界；§3(App/WorkspaceSlot 类型)→ Task 4/5；§4(dozerd 协议)→ Task 1/2/3；§5(GUI 数据流:开/切/关/恢复)→ Task 6/7；§6(顶栏 UI + 指示器优先级)→ Task 6；§7(测试方式)→ 各 task 内联的单元测试均已覆盖 open_projects 往返、指示器优先级、协议字段往返；§8(非目标)未产出对应 task，符合"本轮不做"的定位。
- **占位扫描**：Task 6 的 `Workspace::bootstrap_for_project`/`load_for_project` 之间有一处显式 `todo!()`——已在该步骤原文里说明这是"本 task 交付边界，Task 7 补"，不是遗忘性占位，Task 7 Step 1 紧接着补完整实现，不是"以后再说"式的悬空。
- **类型/签名一致性**：`App`/`WorkspaceSlot`/`Workspace::load_for_project`/`Workspace::bootstrap_for_project`/`Message::ProjectTab*` 在 Task 4/5/6/7 之间的命名/签名通读过一遍，`App::ensure_loaded`/`App::active_workspace_mut`/`App::spawn_open_projects_save` 的调用点与定义处签名一致。
- **风险**：Task 5 是本计划里工程量最大的单一任务，性质与图标栏重排那次的 Task 3 相同——Rust 编译器不允许"半改一半"的中间态，这不是任务拆分疏漏，是在诚实反映改动的耦合程度。执行时如果 5.5 的编译报错数量远超预期，值得停下来重新检查 5.3 的 Step 1-6 是否有遗漏，而不是逐条硬修到编译通过为止。另外，本计划全程假设的"并行浏览器 tab 功能"改动内容是写计划时读到的快照，执行时那份工作可能已经推进/合并/改变形状——每个 task 的第一步都应该先重新核实 `workspace.rs`/`main.rs` 的实际当前内容，按"意图对齐、不逐字硬套"处理任何漂移。
