# Dozer P1g 最小可用项目层 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 引入项目对象（打开文件夹/git 仓库、持久化、当前/最近项目）+ 左一项目卡与懒加载文件树 + 点文件进预览 + 新终端 tab 与验收闭环重锚到当前项目。

**Architecture:** 项目存 dozerd SQLite（`projects` + `meta` 两表，当前项目=`meta['active_project_id']`），经既有 UDS 协议新增项目消息；文件树是 dozer-app 侧纯状态机（懒加载单目录 read_dir，同步读——单目录快，spawn_blocking 留后续）；git 分支/脏走 git CLI（扩展 delivery.rs）；重锚做最小改动（当前项目优先、无则回落现状）。

**Tech Stack:** Rust workspace、rusqlite(bundled)、git CLI、tokio、iced 0.14、rfd（文件夹选择）、tempfile（测试）。

**Spec:** `docs/superpowers/specs/2026-07-19-dozer-p1g-project-layer-design.md`

## Global Constraints

- dozerd 是项目状态权威（存 SQLite）；dozer-app GUI 侧读文件系统建树（dozerd 不碰文件树，延续哑管道）。
- 当前项目重锚做最小改动：当前项目路径优先，无当前项目则回落现状（终端 `$HOME`、验收 `tab.effective_cwd()`）。
- 文件树固定隐藏名单：`.git`、`target`、`node_modules`、`.DS_Store`（.gitignore 精确过滤留后续）。
- 主题：项目名 `theme::CREAM`、git 分支 `theme::BODY`/脏标记 `*` 用 `theme::GOLD`、路径/次要 `theme::DIM`、错误 `theme::RED`。
- 协议向后兼容：新消息为新增变体，不动既有。
- 测试全 headless；GUI 行为归末任务人工验收。
- commit 中文、结尾 `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`；每 Task 收尾 `cargo clippy --all-targets && cargo fmt` 零警告。

---

### Task 1: dozer-core 协议——ProjectInfo + 项目消息

**Files:**
- Modify: `crates/dozer-core/src/protocol.rs`

**Interfaces:**
- Produces:
  - `ProjectInfo { id: i64, path: String, name: String, last_active_ms: u64 }`（Debug/Clone/PartialEq/serde）
  - `Request::{ListProjects, OpenProject{path:String}, SetActiveProject{id:i64}, GetActiveProject}`
  - `Reply::{Projects{projects:Vec<ProjectInfo>}, Project{project:Option<ProjectInfo>}}`

- [ ] **Step 1: 写失败测试**（`protocol.rs` 的 `mod tests` 追加）

```rust
    #[test]
    fn project_messages_roundtrip() {
        let req = Request::OpenProject { path: "/repo/x".into() };
        assert_eq!(decode_line::<Request>(encode_line(&req).trim()).unwrap(), req);
        let req = Request::SetActiveProject { id: 7 };
        assert_eq!(decode_line::<Request>(encode_line(&req).trim()).unwrap(), req);

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
        assert_eq!(decode_line::<Reply>(encode_line(&reply).trim()).unwrap(), reply);
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-core project_messages`
Expected: 编译错误（`ProjectInfo`/变体未定义）。

- [ ] **Step 3: 最小实现**

`protocol.rs`：`AgentState` 附近加类型：

```rust
/// 项目（甲方资产域的根；P1g）。id 为 dozerd SQLite 主键。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectInfo {
    pub id: i64,
    pub path: String,
    pub name: String,
    pub last_active_ms: u64,
}
```

`Request` 追加变体：

```rust
    /// 打开一个目录为项目（已存在则更新活跃时间），并置为当前项目。
    OpenProject { path: String },
    /// 列出所有项目（按活跃时间倒序）。
    ListProjects,
    /// 置当前项目。
    SetActiveProject { id: i64 },
    /// 取当前项目（无则 None）。
    GetActiveProject,
```

`Reply` 追加变体：

```rust
    /// 项目列表。
    Projects { projects: Vec<ProjectInfo> },
    /// 单个/当前项目（无则 None）。
    Project { project: Option<ProjectInfo> },
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-core`
Expected: 全绿（含新测）。dozerd 若因 `Request` match non-exhaustive 报错，在 `server.rs` 的 `Request` match 里临时加 4 个占位分支 `Request::ListProjects | Request::OpenProject{..} | Request::SetActiveProject{..} | Request::GetActiveProject => Reply::Error{message:"P1g 未实现".into()},`（Task 3 替换）。先只保证 dozer-core 绿。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-core
git commit -m "feat(协议): ProjectInfo + 项目消息(Open/List/SetActive/GetActive)——P1g 项目层契约

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 2: dozerd projects.rs——SQLite 项目存储

**Files:**
- Create: `crates/dozerd/src/projects.rs`
- Modify: `crates/dozerd/src/lib.rs`（加 `pub mod projects;`）

**Interfaces:**
- Consumes: Task 1 `ProjectInfo`。
- Produces:
  - `ProjectStore::open(path: &Path) -> anyhow::Result<ProjectStore>`
  - `ProjectStore::open_and_activate(&self, path: &str) -> anyhow::Result<ProjectInfo>`（upsert by path + 置当前）
  - `ProjectStore::list(&self) -> anyhow::Result<Vec<ProjectInfo>>`（last_active_ms 倒序）
  - `ProjectStore::set_active(&self, id: i64) -> anyhow::Result<()>`
  - `ProjectStore::active(&self) -> anyhow::Result<Option<ProjectInfo>>`

- [ ] **Step 1: 写失败测试**（`projects.rs` 尾部）

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_activate_list_and_persist() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.db");
        let store = ProjectStore::open(&db).unwrap();
        assert!(store.active().unwrap().is_none());
        assert!(store.list().unwrap().is_empty());

        let a = store.open_and_activate("/repo/a").unwrap();
        assert_eq!(a.name, "a");
        assert_eq!(store.active().unwrap().unwrap().id, a.id);

        // 同 path 再开:不新增,复用同 id,活跃时间刷新
        let a2 = store.open_and_activate("/repo/a").unwrap();
        assert_eq!(a2.id, a.id);
        assert_eq!(store.list().unwrap().len(), 1);

        // 开第二个 → 成为当前;list 倒序 b 在前
        let b = store.open_and_activate("/repo/b").unwrap();
        assert_eq!(store.active().unwrap().unwrap().id, b.id);
        let list = store.list().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, b.id, "最近活跃在前");

        // 切回 a
        store.set_active(a.id).unwrap();
        assert_eq!(store.active().unwrap().unwrap().id, a.id);

        // 重开库:当前项目与列表持久化
        drop(store);
        let store = ProjectStore::open(&db).unwrap();
        assert_eq!(store.active().unwrap().unwrap().id, a.id);
        assert_eq!(store.list().unwrap().len(), 2);
    }

    #[test]
    fn name_is_basename() {
        let dir = tempfile::tempdir().unwrap();
        let store = ProjectStore::open(&dir.path().join("t.db")).unwrap();
        assert_eq!(store.open_and_activate("/a/b/proj").unwrap().name, "proj");
        assert_eq!(store.open_and_activate("/").unwrap().name, "/");
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozerd projects`
Expected: 编译错误（模块不存在）。

- [ ] **Step 3: 最小实现**

`crates/dozerd/src/projects.rs`：

```rust
//! 项目存储：rusqlite（spec P1g D1）。projects 表 + meta 表（当前项目指针）。
//! 与 AcceptanceStore 各持一个到 dozer.db 的连接；项目/验收写频度极低，
//! 多连接足够（无需连接池）。

use anyhow::{Context, Result};
use dozer_core::protocol::ProjectInfo;
use rusqlite::{Connection, OptionalExtension};
use std::path::Path;
use std::sync::Mutex;

pub struct ProjectStore {
    conn: Mutex<Connection>,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn basename(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

impl ProjectStore {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("建库目录")?;
        }
        let conn = Connection::open(path).context("打开 SQLite")?;
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
        Ok(Self { conn: Mutex::new(conn) })
    }

    /// upsert（按 path）+ 刷新活跃时间 + 置为当前项目，返回该项目。
    pub fn open_and_activate(&self, path: &str) -> Result<ProjectInfo> {
        let conn = self.conn.lock().expect("db lock");
        let ts = now_ms();
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
        conn.execute(
            "UPDATE projects SET last_active_ms = ?1 WHERE id = ?2",
            rusqlite::params![now_ms(), id],
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
}

fn row_to_project(row: &rusqlite::Row) -> rusqlite::Result<ProjectInfo> {
    Ok(ProjectInfo {
        id: row.get(0)?,
        path: row.get(1)?,
        name: row.get(2)?,
        last_active_ms: row.get::<_, i64>(3)? as u64,
    })
}
```

`lib.rs` 加 `pub mod projects;`（在 `pub mod acceptance;` 后）。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozerd projects`
Expected: 2 测试通过。

- [ ] **Step 5: Commit**

```bash
git add crates/dozerd
git commit -m "feat(dozerd): projects.rs SQLite 项目存储——upsert/list/当前项目(meta 指针)持久化

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 3: dozerd serve 接线项目消息

**Files:**
- Modify: `crates/dozerd/src/server.rs`（serve/handle_conn 增 `projects` 参数 + 4 分支）
- Modify: `crates/dozerd/src/main.rs`（构造 ProjectStore）
- Modify: `crates/dozerd/tests/session_survival.rs`、`crates/dozerd/tests/hook_events.rs`（serve 调用点补参数 + test_projects 助手）
- Modify: `crates/dozer-client/tests/against_real_daemon.rs`（serve 调用点补参数）

**Interfaces:**
- Consumes: Task 2 `ProjectStore`。
- Produces: `serve(socket, registry, store, projects: Arc<ProjectStore>)`；4 个项目请求 → 对应 Reply。

- [ ] **Step 1: 写失败测试**（`hook_events.rs` 追加端到端）

```rust
#[tokio::test]
async fn project_open_list_active_roundtrip() {
    let sock = std::env::temp_dir().join(format!("dozerd-proj-{}.sock", uuid::Uuid::new_v4()));
    let db = std::env::temp_dir().join(format!("dozerd-proj-{}.db", uuid::Uuid::new_v4()));
    let registry = Arc::new(SessionRegistry::new());
    let store = Arc::new(dozerd::acceptance::AcceptanceStore::open(&db).unwrap());
    let projects = Arc::new(dozerd::projects::ProjectStore::open(&db).unwrap());
    tokio::spawn({
        let (sock, registry, store, projects) =
            (sock.clone(), registry.clone(), store.clone(), projects.clone());
        async move { dozerd::server::serve(&sock, registry, store, projects).await }
    });
    for _ in 0..100 {
        if sock.exists() { break; }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let opened = match send_req(&sock, &Request::OpenProject { path: "/repo/z".into() }).await {
        Reply::Project { project: Some(p) } => p,
        other => panic!("{other:?}"),
    };
    assert_eq!(opened.name, "z");
    match send_req(&sock, &Request::GetActiveProject).await {
        Reply::Project { project: Some(p) } => assert_eq!(p.id, opened.id),
        other => panic!("{other:?}"),
    }
    match send_req(&sock, &Request::ListProjects).await {
        Reply::Projects { projects } => assert_eq!(projects.len(), 1),
        other => panic!("{other:?}"),
    }
}
```

（`hook_events.rs` / `session_survival.rs` 的 `test_store()` 旁加 `test_projects()`；同库文件即可。）

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozerd`
Expected: 编译错误（serve 签名 4 参、项目分支未实现）。

- [ ] **Step 3: 最小实现**

`server.rs`：`serve` 与 `handle_conn` 各加 `projects: Arc<crate::projects::ProjectStore>` 参数，accept 循环 clone 传入。请求分支（`RecordAcceptance` 之后）：

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

（删除 Task 1 Step 4 的占位分支。）

`main.rs`：AcceptanceStore 构造后加：

```rust
    let projects = Arc::new(dozerd::projects::ProjectStore::open(
        &dozer_core::paths::state_dir().join("dozer.db"),
    )?);
```

`serve(&socket, registry, store)` 改 `serve(&socket, registry, store, projects)`。

测试文件：`session_survival.rs`/`hook_events.rs` 的 `serve(&sock, registry, test_store())` 改为 `serve(&sock, registry, test_store(), test_projects())`；加助手：

```rust
fn test_projects() -> std::sync::Arc<dozerd::projects::ProjectStore> {
    let db = std::env::temp_dir().join(format!("dozerd-test-{}.db", uuid::Uuid::new_v4()));
    std::sync::Arc::new(dozerd::projects::ProjectStore::open(&db).unwrap())
}
```

`against_real_daemon.rs`：`serve(&s, r, store)` 改 `serve(&s, r, store, projects)`，其中 `let projects = Arc::new(dozerd::projects::ProjectStore::open(&db).unwrap());`（复用同一 `db` 临时路径）。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozerd && cargo test -p dozer-client`
Expected: 全绿（含新端到端测试）。

- [ ] **Step 5: Commit**

```bash
git add crates/dozerd crates/dozer-client/tests
git commit -m "feat(dozerd): serve 接线项目消息(Open/List/SetActive/GetActive) + main 构造 ProjectStore

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 4: dozer-client 项目方法 + delivery::branch

**Files:**
- Modify: `crates/dozer-client/src/lib.rs`
- Modify: `crates/dozer-app/src/delivery.rs`

**Interfaces:**
- Consumes: Task 1 项目消息。
- Produces:
  - `Client::open_project(&self, path: &str) -> Result<Option<ProjectInfo>>`
  - `Client::list_projects(&self) -> Result<Vec<ProjectInfo>>`
  - `Client::set_active_project(&self, id: i64) -> Result<Option<ProjectInfo>>`
  - `Client::active_project(&self) -> Result<Option<ProjectInfo>>`
  - `delivery::branch(repo: &Path) -> Option<String>`

- [ ] **Step 1: 写失败测试**（`delivery.rs` tests 追加）

```rust
    #[test]
    fn branch_of_repo() {
        let (_d, repo) = mkrepo();
        let b = branch(&repo).expect("有分支");
        assert!(b == "main" || b == "master", "分支名: {b}");
        assert!(branch(std::path::Path::new("/")).is_none(), "非 git 无分支");
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app branch_of_repo`
Expected: 编译错误（`branch` 未定义）。

- [ ] **Step 3: 实现**

`delivery.rs` 加：

```rust
/// 当前分支名（`git rev-parse --abbrev-ref HEAD`）；非 git / 无提交返回 None。
pub fn branch(repo: &Path) -> Option<String> {
    let out = git(repo, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    let line = out.lines().next()?.trim();
    (!line.is_empty() && line != "HEAD").then(|| line.to_string())
}
```

`dozer-client/src/lib.rs`：头部 `use` 增 `ProjectInfo`：

```rust
use dozer_core::protocol::{
    AgentState, ProjectInfo, Reply, Request, SessionInfo, decode_line, encode_line,
};
```

`impl Client` 追加：

```rust
    pub async fn open_project(&self, path: &str) -> Result<Option<ProjectInfo>> {
        match self.roundtrip(&Request::OpenProject { path: path.into() }).await? {
            Reply::Project { project } => Ok(project),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn list_projects(&self) -> Result<Vec<ProjectInfo>> {
        match self.roundtrip(&Request::ListProjects).await? {
            Reply::Projects { projects } => Ok(projects),
            other => bail!("意外应答: {other:?}"),
        }
    }

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

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app branch_of_repo && cargo build -p dozer-client`
Expected: 全绿。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-client crates/dozer-app/src/delivery.rs
git commit -m "feat(client): 项目方法(open/list/set_active/active) + delivery::branch 当前分支

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 5: project.rs——文件树状态机

**Files:**
- Create: `crates/dozer-app/src/project.rs`
- Modify: `crates/dozer-app/src/main.rs`（`mod delivery;` 后加 `mod project;`）

**Interfaces:**
- Produces:
  - `FileTree::new(root: PathBuf) -> FileTree`
  - `FileTree::root(&self) -> &Path`
  - `FileTree::toggle(&mut self, dir: &Path)`（展开则同步 read_dir 缓存子项；收起则去标记）
  - `FileTree::visible_rows(&self) -> Vec<TreeRow>`
  - `TreeRow { pub path: PathBuf, pub name: String, pub depth: usize, pub is_dir: bool, pub expanded: bool }`
  - `const HIDDEN: [&str; 4]`

**设计注记**：`toggle` 展开时同步 `read_dir`（单目录快）。spec D2 提的 spawn_blocking 对超大目录更稳，本切片先同步，留作后续；行为等价、可测。

- [ ] **Step 1: 写失败测试**（`project.rs` 尾部）

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn mktree() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        std::fs::create_dir(r.join("src")).unwrap();
        std::fs::write(r.join("src/main.rs"), "").unwrap();
        std::fs::write(r.join("README.md"), "").unwrap();
        std::fs::create_dir(r.join(".git")).unwrap(); // 隐藏
        std::fs::create_dir(r.join("target")).unwrap(); // 隐藏
        d
    }

    #[test]
    fn root_rows_hide_and_sort() {
        let d = mktree();
        let t = FileTree::new(d.path().to_path_buf());
        let rows = t.visible_rows();
        let names: Vec<_> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["src", "README.md"], "目录在前、隐藏名单剔除");
        assert!(rows[0].is_dir && !rows[0].expanded);
        assert_eq!(rows[0].depth, 0);
    }

    #[test]
    fn expand_shows_children_with_depth() {
        let d = mktree();
        let mut t = FileTree::new(d.path().to_path_buf());
        t.toggle(&d.path().join("src"));
        let rows = t.visible_rows();
        let names: Vec<_> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["src", "main.rs", "README.md"]);
        let child = rows.iter().find(|r| r.name == "main.rs").unwrap();
        assert_eq!(child.depth, 1);
        // 再 toggle 收起
        t.toggle(&d.path().join("src"));
        assert_eq!(t.visible_rows().len(), 2);
    }

    #[test]
    fn unreadable_dir_does_not_panic() {
        let d = tempfile::tempdir().unwrap();
        let mut t = FileTree::new(d.path().to_path_buf());
        // toggle 一个不存在的子目录:不 panic,不产生子行
        t.toggle(&d.path().join("nope"));
        assert!(t.visible_rows().is_empty());
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app project`
Expected: 编译错误（模块不存在）。

- [ ] **Step 3: 实现**

`crates/dozer-app/src/project.rs`：

```rust
//! 文件树状态机（P1g）：懒加载单目录、展开集、可见行摊平。纯数据，不碰
//! iced；展开时同步 read_dir（单目录快）。固定隐藏名单过滤。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// 文件树里恒不显示的目录/文件名。
pub const HIDDEN: [&str; 4] = [".git", "target", "node_modules", ".DS_Store"];

#[derive(Debug, Clone, PartialEq)]
struct Entry {
    path: PathBuf,
    name: String,
    is_dir: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TreeRow {
    pub path: PathBuf,
    pub name: String,
    pub depth: usize,
    pub is_dir: bool,
    pub expanded: bool,
}

pub struct FileTree {
    root: PathBuf,
    expanded: HashSet<PathBuf>,
    children: HashMap<PathBuf, Vec<Entry>>,
}

/// 读一个目录:剔隐藏名单,目录在前、各自按名排序。read_dir 失败返回空。
fn read_children(dir: &Path) -> Vec<Entry> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut entries: Vec<Entry> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if HIDDEN.contains(&name.as_str()) {
                return None;
            }
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            Some(Entry { path: e.path(), name, is_dir })
        })
        .collect();
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));
    entries
}

impl FileTree {
    pub fn new(root: PathBuf) -> Self {
        let mut children = HashMap::new();
        children.insert(root.clone(), read_children(&root));
        Self {
            root,
            expanded: HashSet::new(),
            children,
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 展开/收起一个目录。展开时若未缓存则同步读一次。
    pub fn toggle(&mut self, dir: &Path) {
        if self.expanded.remove(dir) {
            return; // 已展开 → 收起
        }
        self.children
            .entry(dir.to_path_buf())
            .or_insert_with(|| read_children(dir));
        // 空目录/不可读:缓存为空 vec,不标 expanded(无可展开内容)
        if self.children.get(dir).map(|c| !c.is_empty()).unwrap_or(false) {
            self.expanded.insert(dir.to_path_buf());
        }
    }

    /// 从 root 的子项起 DFS 摊平成可见行（展开的目录才递归其子项）。
    pub fn visible_rows(&self) -> Vec<TreeRow> {
        let mut out = Vec::new();
        self.push_rows(&self.root, 0, &mut out);
        out
    }

    fn push_rows(&self, dir: &Path, depth: usize, out: &mut Vec<TreeRow>) {
        let Some(entries) = self.children.get(dir) else {
            return;
        };
        for e in entries {
            let expanded = self.expanded.contains(&e.path);
            out.push(TreeRow {
                path: e.path.clone(),
                name: e.name.clone(),
                depth,
                is_dir: e.is_dir,
                expanded,
            });
            if e.is_dir && expanded {
                self.push_rows(&e.path, depth + 1, out);
            }
        }
    }
}
```

`main.rs` 的 `mod delivery;` 后加 `mod project;`。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app project`
Expected: 3 测试通过。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app/src/project.rs crates/dozer-app/src/main.rs
git commit -m "feat(项目): 文件树状态机——懒加载/展开集/可见行摊平/隐藏名单

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 6: workspace 接入项目——项目卡 + 文件树 UI + 重锚 + 打开/最近 + 启动恢复

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`
- Modify: `crates/dozer-app/src/main.rs`（rfd 文件夹选择，dispatch 拦截 `ProjectPickFolder`）

**Interfaces:**
- Consumes: Task 4 client 项目方法 + `delivery::branch`；Task 5 `FileTree`/`TreeRow`；`ProjectInfo`。
- Produces:
  - `Workspace` 字段 `project: Option<ProjectInfo>`、`file_tree: Option<FileTree>`、`branch: Option<String>`、`dirty: bool`、`recent_projects: Vec<ProjectInfo>`
  - `Message::{ProjectPickFolder, ProjectOpen(PathBuf), ProjectOpened(Option<ProjectInfo>, Vec<ProjectInfo>), ProjectSelect(i64), ProjectTreeToggle(PathBuf), ProjectGitRefreshed(Option<String>, bool)}`
  - `effective_project_repo(active: Option<&Path>, session_cwd: &Path) -> PathBuf`（纯函数）

- [ ] **Step 1: 写失败测试**（`workspace.rs` tests 追加）

```rust
    #[test]
    fn effective_project_repo_prefers_active() {
        use std::path::Path;
        assert_eq!(
            effective_project_repo(Some(Path::new("/proj")), Path::new("/home/me")),
            PathBuf::from("/proj")
        );
        assert_eq!(
            effective_project_repo(None, Path::new("/home/me")),
            PathBuf::from("/home/me")
        );
    }

    #[test]
    fn project_card_branch_label() {
        assert_eq!(project_branch_label(Some("main"), false), "main");
        assert_eq!(project_branch_label(Some("main"), true), "main*");
        assert_eq!(project_branch_label(None, false), "—");
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app effective_project_repo project_card`
Expected: 编译错误（函数未定义）。

- [ ] **Step 3: 实现**

`workspace.rs`：

1. `use crate::project::FileTree;`、`use dozer_core::protocol::{AgentState, ProjectInfo, SessionInfo};`（补 `ProjectInfo`）。

2. `Message` 追加：

```rust
    /// 项目:点"打开项目…"→ rfd 文件夹选择(main.rs 执行)。
    ProjectPickFolder,
    /// 项目:打开某路径为项目(rfd 选中/最近点击回送)。
    ProjectOpen(PathBuf),
    /// 项目:打开完成(当前项目 + 最近列表)。
    ProjectOpened(Option<ProjectInfo>, Vec<ProjectInfo>),
    /// 项目:切换到最近项目。
    ProjectSelect(i64),
    /// 项目:文件树展开/收起某目录。
    ProjectTreeToggle(PathBuf),
    /// 项目:git 分支/脏刷新结果。
    ProjectGitRefreshed(Option<String>, bool),
```

3. `Workspace` 加字段（两处构造 `bootstrap`/`with_daemon_error` 初始化 `project: None, file_tree: None, branch: None, dirty: false, recent_projects: Vec::new(),`）：

```rust
    /// 当前项目（None=未打开）。
    project: Option<ProjectInfo>,
    /// 当前项目的文件树（随 project 建立）。
    file_tree: Option<FileTree>,
    /// 当前项目 git 分支（非 git 为 None）。
    branch: Option<String>,
    /// 当前项目工作树是否脏。
    dirty: bool,
    /// 最近项目（切换用）。
    recent_projects: Vec<ProjectInfo>,
```

4. `bootstrap` 结尾（`Self { ... }` 之前）拉当前项目与列表，装进初值——把上面的初始化改为实拉：在 `bootstrap` 内 `client.list()` 之后加：

```rust
        let active = client.active_project().await.ok().flatten();
        let recent = client.list_projects().await.unwrap_or_default();
        let file_tree = active
            .as_ref()
            .map(|p| FileTree::new(PathBuf::from(&p.path)));
```

并把 `Self{...}` 里对应字段设为 `project: active, file_tree, branch: None, dirty: false, recent_projects: recent,`（git 分支/脏在窗口起来后由 `ProjectGitRefreshed` 异步补；启动时先 None/false）。`with_daemon_error` 仍全 None/空。

5. `update` 追加分支：

```rust
            Message::ProjectPickFolder => {} // 副作用在 main.rs（rfd）
            Message::ProjectOpen(path) => {
                let client = self.client.clone();
                let proxy = self.proxy.clone();
                let path_s = path.to_string_lossy().into_owned();
                self.handle.spawn(async move {
                    let opened = client.open_project(&path_s).await.ok().flatten();
                    let recent = client.list_projects().await.unwrap_or_default();
                    let _ = proxy.send_event(Message::ProjectOpened(opened, recent));
                });
            }
            Message::ProjectSelect(id) => {
                let client = self.client.clone();
                let proxy = self.proxy.clone();
                self.handle.spawn(async move {
                    let opened = client.set_active_project(id).await.ok().flatten();
                    let recent = client.list_projects().await.unwrap_or_default();
                    let _ = proxy.send_event(Message::ProjectOpened(opened, recent));
                });
            }
            Message::ProjectOpened(project, recent) => {
                self.recent_projects = recent;
                self.file_tree = project
                    .as_ref()
                    .map(|p| FileTree::new(PathBuf::from(&p.path)));
                self.branch = None;
                self.dirty = false;
                // 起个 git 刷新
                if let Some(p) = &project {
                    let repo = PathBuf::from(&p.path);
                    let proxy = self.proxy.clone();
                    self.handle.spawn(async move {
                        let (b, d) = tokio::task::spawn_blocking(move || {
                            (delivery::branch(&repo), delivery::is_dirty(&repo))
                        })
                        .await
                        .unwrap_or((None, false));
                        let _ = proxy.send_event(Message::ProjectGitRefreshed(b, d));
                    });
                }
                self.project = project;
            }
            Message::ProjectTreeToggle(dir) => {
                if let Some(t) = &mut self.file_tree {
                    t.toggle(&dir);
                }
            }
            Message::ProjectGitRefreshed(branch, dirty) => {
                self.branch = branch;
                self.dirty = dirty;
            }
```

6. **重锚**——纯函数 + 三处接线：

```rust
/// 交付/验收使用的仓库：当前项目优先，无则回落会话 cwd（P1f 现状）。
fn effective_project_repo(active: Option<&Path>, session_cwd: &Path) -> PathBuf {
    active.map(|p| p.to_path_buf()).unwrap_or_else(|| session_cwd.to_path_buf())
}

/// 项目卡分支标签：`分支` / `分支*`（脏）/ `—`（非 git）。
fn project_branch_label(branch: Option<&str>, dirty: bool) -> String {
    match branch {
        Some(b) if dirty => format!("{b}*"),
        Some(b) => b.to_string(),
        None => "—".to_string(),
    }
}
```

`AgentStateChanged(TurnEnded)`、`DeliveryChecked`、`AcceptanceOpen` 三处的 `let cwd = tab.effective_cwd();` 改为：

```rust
                        let cwd = effective_project_repo(
                            self.project.as_ref().map(|p| Path::new(&p.path)),
                            &tab.effective_cwd(),
                        );
```

（注意借用：`self.project` 与 `tab`（`self.tab_by_id_mut` 借的 `&mut self`）冲突时，先把 `self.project` 的路径克隆出来再取 tab。实施以借用检查通过为准，行为不变——当前项目路径优先。）

`spawn_new_tab` 的 `home` 改为项目根优先：

```rust
        let cwd = self
            .project
            .as_ref()
            .map(|p| p.path.clone())
            .unwrap_or_else(|| std::env::var("HOME").unwrap_or_else(|_| "/".into()));
```

并把 `client.create("shell", &shell, &[], &home, ...)` 的 `&home` 改为 `&cwd`。

7. **左一 UI**：`view()` 的 `col1` 改为 `project_pane(self)`；新增：

```rust
/// 左一项目栏：项目卡（名称 + git 分支/脏 + 路径）+ 文件树；无项目时"打开项目…" + 最近。
fn project_pane(ws: &Workspace) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let mut content = column![text("项目").size(13).color(theme::CREAM)].spacing(4);

    let open_btn = button(text("打开项目…").size(12).color(theme::CREAM))
        .on_press(Message::ProjectPickFolder)
        .style(|_t, _s| button::Style {
            background: Some(theme::CARD.into()),
            text_color: theme::CREAM,
            border: Border { color: theme::BORDER, width: 1.0, radius: 2.0.into() },
            ..button::Style::default()
        });

    match &ws.project {
        Some(p) => {
            content = content.push(text(p.name.clone()).size(14).color(theme::CREAM));
            let label = project_branch_label(ws.branch.as_deref(), ws.dirty);
            let bcolor = if ws.dirty { theme::GOLD } else { theme::BODY };
            content = content.push(text(label).size(11).color(bcolor));
            content = content.push(text(p.path.clone()).size(10).color(theme::DIM));
            content = content.push(open_btn);
            // 文件树
            if let Some(tree) = &ws.file_tree {
                for row in tree.visible_rows() {
                    let indent = "  ".repeat(row.depth);
                    let glyph = if row.is_dir {
                        if row.expanded { "▾ " } else { "▸ " }
                    } else {
                        "  "
                    };
                    let label = format!("{indent}{glyph}{}", row.name);
                    let msg = if row.is_dir {
                        Message::ProjectTreeToggle(row.path.clone())
                    } else {
                        Message::PreviewOpenPath(row.path.clone())
                    };
                    let color = if row.is_dir { theme::BODY } else { theme::CYAN };
                    content = content.push(
                        button(text(label).size(12).color(color))
                            .on_press(msg)
                            .width(Length::Fill)
                            .style(|_t, _s| button::Style {
                                background: None,
                                text_color: theme::BODY,
                                ..button::Style::default()
                            }),
                    );
                }
            }
        }
        None => {
            content = content.push(text("未打开项目").size(12).color(theme::DIM));
            content = content.push(open_btn);
            for p in &ws.recent_projects {
                content = content.push(
                    button(text(p.name.clone()).size(12).color(theme::CREAM))
                        .on_press(Message::ProjectSelect(p.id))
                        .style(|_t, _s| button::Style {
                            background: None,
                            text_color: theme::CREAM,
                            ..button::Style::default()
                        }),
                );
            }
        }
    }

    container(content.padding(8))
        .width(Length::Fixed(PROJECT_COL_WIDTH))
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(theme::PANEL.into()),
            border: Border { color: theme::BORDER, width: 1.0, radius: 0.0.into() },
            ..container::Style::default()
        })
        .into()
}
```

`main.rs` `dispatch`：`Message::ProjectPickFolder` 拦截（同 `PreviewPickFile` 模式）：

```rust
                Message::ProjectPickFolder => {
                    if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                        workspace.update(Message::ProjectOpen(dir));
                    }
                }
```

（`dispatch` 的 `pending_focus` 段：`ProjectOpen`/`ProjectSelect` 不改焦点意图，保持终端焦点，无需加。）

- [ ] **Step 4: 跑测试确认通过 + 冒烟**

```bash
cargo test -p dozer-app && cargo clippy -p dozer-app --all-targets && cargo fmt
cargo build -p dozer-app && ./target/aarch64-apple-darwin/debug/dozer & sleep 8 && kill %1
```

Expected: 测试全绿、零警告；app 存活 8 秒无 panic。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-app
git commit -m "feat(项目): 左一项目栏——项目卡 + 文件树 + 点文件进预览 + 重锚终端/验收到项目 + 打开/最近/启动恢复

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 7: 全量回归 + 端到端人工验收 + 落档

**Files:**
- Create: `docs/superpowers/specs/2026-07-19-p1g-acceptance.md`
- Modify: `docs/superpowers/specs/2026-07-14-dozer-phase1-design.md`（验收通过后 §7 项目栏相关或 §3 回填 P1g 达成）
- Modify: 本计划文件（勾选 checkbox）

**Interfaces:**
- Consumes: 全部前序任务。
- Produces: 用户签字的验收记录；下一阶段起点状态。

- [ ] **Step 1: 全量回归 + 冒烟**

```bash
cargo test && cargo clippy --all-targets && cargo fmt --check
cargo build -p dozer-app && ./target/aarch64-apple-darwin/debug/dozer & sleep 8 && kill %1
```

Expected: 全绿零警告；app 存活 8 秒。**验收前 `pkill dozerd` 重启新 daemon**（版本偏斜教训）。

- [ ] **Step 2: 用户人工验收（逐项 ✓/✗，验收权在用户）**

```markdown
# P1g 人工验收清单（用户实机执行）
1. 首启无项目 → 左一"打开项目…" + 最近列表（若有）
2. 打开本仓为项目 → 左一见项目名 + `main*`（脏时金*）+ 路径 + 文件树（.git/target 不显示，目录在前）
3. 点目录 ▸ 展开/收起；点文件 → 左二 Flyfish 预览该文件
4. 新终端 tab（＋）→ 落在项目根（`pwd` 确认），非 $HOME
5. `.dozer/goal.md` 定标 → claude 改文件 → 回合毕横幅（验收挂到项目仓库，即使 tab 曾 cd 别处）
6. 打开第二个 git 仓库为项目 → 项目名/分支/文件树全跟随；最近列表可切回
7. 关 app 重开 → 当前项目自动恢复（左一直接是它）
8. 打开一个非 git 目录 → 文件树可用，分支区显示 `—`，不崩
```

- [ ] **Step 3: 结果落档 + Commit**

验收记录写入 `docs/superpowers/specs/2026-07-19-p1g-acceptance.md`（沿用格式：逐项结果表 + 反馈修复流水）；全部通过后规格 §7 项目栏段落追加"P1g 达成（<日期>，最小可用项目层：项目对象/文件树/git 状态/点文件进预览/重锚终端与验收到项目；H0 独立页/组件视图/doctor 随后续），验收记录见 specs/2026-07-19-p1g-acceptance.md"。

```bash
git add docs
git commit -m "验收：P1g 最小可用项目层人工验收记录 + 规格回填

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

## Self-Review（已执行）

1. **Spec 覆盖**：目标 1（项目对象/持久化/当前/最近）=T1/T2/T3/T4 client；目标 2（项目卡 + 文件树）=T5 状态机 + T6 UI；目标 3（重锚终端/验收）=T6 `effective_project_repo` + spawn_new_tab；目标 4（打开/最近切换）=T6 + main rfd。裁决 D1（SQLite 两表 meta 指针）=T2；D2（GUI 侧懒加载 + 隐藏名单）=T5（同步 read_dir，注记偏差）；D3（git CLI branch/dirty）=T4 branch + T6 刷新；D4（重锚最小改动）=T6；D5（单当前项目）=T2 meta 单指针；D6（启动恢复）=T6 bootstrap 拉 active。错误处理五条：非 git（T6 branch=None→`—`）、目录不可读（T5 read_children 返空不崩）、路径失效（T5 空树 + T6 打开失败提示）、落库失败（dozerd Reply::Error → client bail → 但 open 走 spawn，失败则 project=None）、非 git 仍可作项目（T6 None 分支）。
2. **占位符扫描**：无 TBD；T6 借用调整（self.project vs tab）给了明确方向（克隆项目路径先于取 tab，借用检查为准）。
3. **类型一致性**：`ProjectInfo{id:i64,path,name,last_active_ms}`、`serve(socket,registry,store,projects)`、`open_and_activate(&str)->ProjectInfo`、`active()->Option<ProjectInfo>`、`Client::{open_project,list_projects,set_active_project,active_project}`、`delivery::branch(&Path)->Option<String>`、`FileTree::{new,toggle,visible_rows,root}`+`TreeRow{path,name,depth,is_dir,expanded}`、`effective_project_repo(Option<&Path>,&Path)->PathBuf`、`project_branch_label(Option<&str>,bool)->String`、`Message::Project*` 系列在 T1-T6 交叉一致。
