# 浏览器收藏夹 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 给浏览器域(`Workspace::browser`)加一个全局 + 本项目两级收藏夹:dozerd SQLite 持久化,地址栏星标增删,tab 栏下拉面板浏览/打开/删除。

**Architecture:** 新表 `bookmarks` 挂在 dozerd 现有的 `dozer.db`(与 `projects`/`acceptance` 同库,`project_id` 外键复用 `projects.id`);协议新增 `AddBookmark`/`RemoveBookmark`/`ListBookmarks` 三个 Request + `Reply::Bookmarks`;`dozer-app` 侧新增一个纯逻辑模块 `bookmarks.rs`(状态判定 + 乐观本地增删,不碰 iced/网络)和 `Workspace` 上的少量新字段,UI 走既有 `browser_pane` 渲染函数的追加。

**Tech Stack:** Rust workspace;`rusqlite`(bundled sqlite3);`iced` 0.14(`iced_widget`);Unix Domain Socket 行协议(`serde_json` 逐行编解码,已有基础设施,不新增依赖)。

## Global Constraints

- boy CLI 已废弃,本计划不涉及 `crates/legacy-boy`。
- GUI 只用 iced 0.14 生态,新增 UI 严格复用现有 `button`/`container`/`row`/`column` 宏与 `theme.rs` 精选色值,不引入新组件库。
- 主题色:GOLD `#F2D94E`(甲方动作专属,星标"已收藏"态用)、CREAM `#FFE5B4`(正文文字)、DIM(次要/未激活)、CARD/BORDER(卡片背景/描边)——均已在 `theme.rs` 定义,直接引用,不新增色值。
- 不引入新的 crate 依赖(`rusqlite`/`tempfile` 已是 `dozerd` 现有依赖)。
- 所有新增/修改的公开行为都要有对应单测;真实 iced 渲染效果留给 `cargo run -p dozer-app` 人工验收,不做自动化 GUI 测试(与 Todo 面板等既有惯例一致)。

---

### Task 1: 协议层新增收藏夹类型与消息

**Files:**
- Modify: `crates/dozer-core/src/protocol.rs`

**Interfaces:**
- Produces:
  - `pub enum BookmarkScope { Global, Project }`(`Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize`,`#[serde(rename_all = "snake_case")]`)
  - `pub struct BookmarkInfo { pub id: i64, pub scope: BookmarkScope, pub project_id: Option<i64>, pub url: String, pub title: String, pub created_ms: u64 }`(`Debug, Clone, PartialEq, Serialize, Deserialize`)
  - `Request::AddBookmark { scope: BookmarkScope, project_id: Option<i64>, url: String, title: String }`
  - `Request::RemoveBookmark { id: i64 }`
  - `Request::ListBookmarks { project_id: Option<i64> }`
  - `Reply::Bookmarks { bookmarks: Vec<BookmarkInfo> }`

- [x] **Step 1: 写失败的序列化往返测试**

在 `crates/dozer-core/src/protocol.rs` 的 `mod tests` 里追加(紧跟 `acceptance_count_request_roundtrips` 之后即可):

```rust
    #[test]
    fn bookmark_scope_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&BookmarkScope::Global).unwrap(),
            "\"global\""
        );
        assert_eq!(
            serde_json::to_string(&BookmarkScope::Project).unwrap(),
            "\"project\""
        );
    }

    #[test]
    fn bookmark_messages_roundtrip() {
        let req = Request::AddBookmark {
            scope: BookmarkScope::Project,
            project_id: Some(7),
            url: "https://example.com".into(),
            title: "example".into(),
        };
        assert_eq!(decode_line::<Request>(&encode_line(&req)).unwrap(), req);

        let req = Request::RemoveBookmark { id: 3 };
        assert_eq!(decode_line::<Request>(&encode_line(&req)).unwrap(), req);

        let req = Request::ListBookmarks {
            project_id: Some(7),
        };
        assert_eq!(decode_line::<Request>(&encode_line(&req)).unwrap(), req);

        let req = Request::ListBookmarks { project_id: None };
        assert_eq!(decode_line::<Request>(&encode_line(&req)).unwrap(), req);

        let reply = Reply::Bookmarks {
            bookmarks: vec![BookmarkInfo {
                id: 1,
                scope: BookmarkScope::Global,
                project_id: None,
                url: "https://example.com".into(),
                title: "example".into(),
                created_ms: 5,
            }],
        };
        let line = encode_line(&reply);
        assert!(line.contains(r#""type":"bookmarks""#));
        assert_eq!(decode_line::<Reply>(line.trim()).unwrap(), reply);
    }
```

- [x] **Step 2: 运行测试确认失败(类型不存在)**

Run: `cargo test -p dozer-core bookmark`
Expected: 编译失败,报 `BookmarkScope`/`BookmarkInfo`/`Request::AddBookmark` 等未定义。

- [x] **Step 3: 添加类型与枚举变体**

在 `ProjectInfo` 定义之后插入:

```rust
/// 收藏夹范围:全局(跨项目共享)或挂靠某个项目(`project_id` 必填)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BookmarkScope {
    Global,
    Project,
}

/// 一条收藏记录。`project_id`:`scope=Global` 时恒为 `None`,
/// `scope=Project` 时是该项目在 `projects` 表里的 `id`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BookmarkInfo {
    pub id: i64,
    pub scope: BookmarkScope,
    pub project_id: Option<i64>,
    pub url: String,
    pub title: String,
    pub created_ms: u64,
}
```

在 `Request` 枚举的 `GetAcceptanceCount { repo: String },` 之后追加:

```rust
    /// 加入收藏(幂等:同 scope+project_id+url 已存在则 no-op)。
    AddBookmark {
        scope: BookmarkScope,
        project_id: Option<i64>,
        url: String,
        title: String,
    },
    /// 移除收藏(按记录 id;不存在则 no-op)。
    RemoveBookmark {
        id: i64,
    },
    /// 列出"全局 + 指定项目"的收藏合集;`project_id: None` 时只返回全局。
    ListBookmarks {
        project_id: Option<i64>,
    },
```

在 `Reply` 枚举的 `AcceptanceCount { count: u64 },` 之后追加:

```rust
    /// 收藏夹列表。
    Bookmarks {
        bookmarks: Vec<BookmarkInfo>,
    },
```

- [x] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozer-core bookmark`
Expected: PASS(`bookmark_scope_serializes_snake_case`、`bookmark_messages_roundtrip` 两条)

- [x] **Step 5: 跑一遍全量 protocol 测试,确认没有破坏既有序列化**

Run: `cargo test -p dozer-core`
Expected: 全绿

- [x] **Step 6: Commit**

```bash
git add crates/dozer-core/src/protocol.rs
git commit -m "feat(protocol): add bookmark types and Add/Remove/List messages"
```

---

### Task 2: dozerd 收藏夹 SQLite 存储

**Files:**
- Create: `crates/dozerd/src/bookmarks.rs`
- Modify: `crates/dozerd/src/lib.rs`(加 `pub mod bookmarks;`)

**Interfaces:**
- Consumes: `dozer_core::protocol::{BookmarkInfo, BookmarkScope}`(Task 1)
- Produces:
  - `pub struct BookmarkStore`
  - `impl BookmarkStore { pub fn new(path: &Path) -> Result<Self>; pub fn add(&self, scope: BookmarkScope, project_id: Option<i64>, url: &str, title: &str) -> Result<BookmarkInfo>; pub fn remove(&self, id: i64) -> Result<()>; pub fn list(&self, project_id: Option<i64>) -> Result<Vec<BookmarkInfo>>; }`

- [x] **Step 1: 写失败的存储测试**

创建 `crates/dozerd/src/bookmarks.rs`,先写测试模块(此时 `BookmarkStore` 还不存在,编译会失败):

```rust
//! 收藏夹存储:rusqlite,与 `ProjectStore`/`AcceptanceStore` 共享同一个
//! `dozer.db`(见 `main.rs` 里三者各自 `new`/`open` 时传入同一路径)。
//! `bookmarks` 表用两条局部唯一索引分别去重全局/项目收藏——SQLite 的
//! 表级 `UNIQUE` 把多个 NULL 视为互不相同,`project_id` 为 NULL 时不能靠
//! 它给全局收藏去重,必须用 `WHERE scope = 'global'` 的局部索引。

use anyhow::{Context, Result};
use dozer_core::protocol::{BookmarkInfo, BookmarkScope};
use rusqlite::Connection;
use std::path::Path;
use std::sync::Mutex;

pub struct BookmarkStore {
    conn: Mutex<Connection>,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn scope_str(scope: BookmarkScope) -> &'static str {
    match scope {
        BookmarkScope::Global => "global",
        BookmarkScope::Project => "project",
    }
}

fn scope_from_str(s: &str) -> BookmarkScope {
    match s {
        "project" => BookmarkScope::Project,
        _ => BookmarkScope::Global,
    }
}

fn row_to_bookmark(row: &rusqlite::Row) -> rusqlite::Result<BookmarkInfo> {
    let scope: String = row.get(1)?;
    Ok(BookmarkInfo {
        id: row.get(0)?,
        scope: scope_from_str(&scope),
        project_id: row.get(2)?,
        url: row.get(3)?,
        title: row.get(4)?,
        created_ms: row.get::<_, i64>(5)? as u64,
    })
}

impl BookmarkStore {
    pub fn new(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("建库目录")?;
        }
        let conn = Connection::open(path).context("打开 SQLite")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS bookmarks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                scope TEXT NOT NULL CHECK(scope IN ('global', 'project')),
                project_id INTEGER,
                url TEXT NOT NULL,
                title TEXT NOT NULL,
                created_ms INTEGER NOT NULL
             );
             CREATE UNIQUE INDEX IF NOT EXISTS ux_bookmarks_global
                ON bookmarks(url) WHERE scope = 'global';
             CREATE UNIQUE INDEX IF NOT EXISTS ux_bookmarks_project
                ON bookmarks(project_id, url) WHERE scope = 'project';",
        )
        .context("建表")?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// 幂等添加:`INSERT OR IGNORE` 命中已存在行时不报错、不改标题,
    /// 用 `scope+project_id+url`(`IS` 做 NULL 安全比较)查回那条已有记录。
    pub fn add(
        &self,
        scope: BookmarkScope,
        project_id: Option<i64>,
        url: &str,
        title: &str,
    ) -> Result<BookmarkInfo> {
        let conn = self.conn.lock().expect("db lock");
        let ts = now_ms() as i64;
        conn.execute(
            "INSERT OR IGNORE INTO bookmarks (scope, project_id, url, title, created_ms)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![scope_str(scope), project_id, url, title, ts],
        )?;
        conn.query_row(
            "SELECT id, scope, project_id, url, title, created_ms FROM bookmarks
             WHERE scope = ?1 AND project_id IS ?2 AND url = ?3",
            rusqlite::params![scope_str(scope), project_id, url],
            row_to_bookmark,
        )
        .map_err(Into::into)
    }

    pub fn remove(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock().expect("db lock");
        conn.execute("DELETE FROM bookmarks WHERE id = ?1", [id])?;
        Ok(())
    }

    /// 全局收藏 ∪(`project_id` 给定时)该项目的收藏,按加入时间升序。
    /// `project_id = None` 时 `project_id IS ?1` 恒假,天然只剩全局收藏,
    /// 不需要额外分支。
    pub fn list(&self, project_id: Option<i64>) -> Result<Vec<BookmarkInfo>> {
        let conn = self.conn.lock().expect("db lock");
        let mut stmt = conn.prepare(
            "SELECT id, scope, project_id, url, title, created_ms FROM bookmarks
             WHERE scope = 'global' OR (scope = 'project' AND project_id IS ?1)
             ORDER BY created_ms ASC",
        )?;
        let rows = stmt.query_map([project_id], row_to_bookmark)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, BookmarkStore) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.db");
        let store = BookmarkStore::new(&db).unwrap();
        (dir, store)
    }

    #[test]
    fn add_list_and_remove() {
        let (_dir, store) = store();
        let b = store
            .add(BookmarkScope::Global, None, "https://a.com", "A")
            .unwrap();
        assert_eq!(b.url, "https://a.com");
        assert_eq!(b.scope, BookmarkScope::Global);
        assert_eq!(b.project_id, None);

        let listed = store.list(None).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, b.id);

        store.remove(b.id).unwrap();
        assert!(store.list(None).unwrap().is_empty());
    }

    #[test]
    fn project_scope_isolated_per_project() {
        let (_dir, store) = store();
        store
            .add(BookmarkScope::Project, Some(1), "https://a.com", "A")
            .unwrap();
        store
            .add(BookmarkScope::Project, Some(2), "https://b.com", "B")
            .unwrap();
        store
            .add(BookmarkScope::Global, None, "https://g.com", "G")
            .unwrap();

        let for_1 = store.list(Some(1)).unwrap();
        assert_eq!(for_1.len(), 2, "全局 + 项目1自己的");
        assert!(for_1.iter().any(|b| b.url == "https://a.com"));
        assert!(for_1.iter().any(|b| b.url == "https://g.com"));
        assert!(
            !for_1.iter().any(|b| b.url == "https://b.com"),
            "不该看到项目2的收藏"
        );

        let none = store.list(None).unwrap();
        assert_eq!(none.len(), 1, "未开项目只看到全局");
        assert_eq!(none[0].url, "https://g.com");
    }

    #[test]
    fn add_is_idempotent_and_does_not_update_title() {
        let (_dir, store) = store();
        let first = store
            .add(BookmarkScope::Global, None, "https://a.com", "A")
            .unwrap();
        let second = store
            .add(BookmarkScope::Global, None, "https://a.com", "改了标题也不生效")
            .unwrap();
        assert_eq!(first.id, second.id);
        assert_eq!(second.title, "A");
        assert_eq!(store.list(None).unwrap().len(), 1);
    }

    #[test]
    fn same_url_can_exist_in_both_global_and_project_scope() {
        let (_dir, store) = store();
        store
            .add(BookmarkScope::Global, None, "https://a.com", "A")
            .unwrap();
        store
            .add(BookmarkScope::Project, Some(1), "https://a.com", "A")
            .unwrap();
        assert_eq!(store.list(Some(1)).unwrap().len(), 2, "全局和项目各一条,互不去重");
    }

    #[test]
    fn remove_unknown_id_is_noop() {
        let (_dir, store) = store();
        store.remove(9999).unwrap();
    }

    #[test]
    fn list_orders_by_created_ms_ascending() {
        let (_dir, store) = store();
        store
            .add(BookmarkScope::Global, None, "https://first.com", "First")
            .unwrap();
        store
            .add(BookmarkScope::Global, None, "https://second.com", "Second")
            .unwrap();
        let listed = store.list(None).unwrap();
        assert_eq!(listed[0].url, "https://first.com");
        assert_eq!(listed[1].url, "https://second.com");
    }
}
```

- [x] **Step 2: 声明模块**

在 `crates/dozerd/src/lib.rs` 加一行(按字母序插到 `acceptance` 之后):

```rust
pub mod acceptance;
pub mod bookmarks;
pub mod projects;
```

- [x] **Step 3: 运行测试确认通过**

Run: `cargo test -p dozerd bookmarks::`
Expected: 6 个测试全部 PASS。

- [x] **Step 4: Commit**

```bash
git add crates/dozerd/src/bookmarks.rs crates/dozerd/src/lib.rs
git commit -m "feat(dozerd): add BookmarkStore backed by dozer.db"
```

---

### Task 3: dozerd server/main 接线

**Files:**
- Modify: `crates/dozerd/src/server.rs`
- Modify: `crates/dozerd/src/main.rs`

**Interfaces:**
- Consumes: `crate::bookmarks::BookmarkStore`(Task 2)、`Request::{AddBookmark,RemoveBookmark,ListBookmarks}`/`Reply::Bookmarks`(Task 1)
- Produces: `serve(socket, registry, store, projects, bookmarks)` 新签名(供 Task 4/集成测试之外无消费方,`main.rs` 是唯一调用点)

- [x] **Step 1: 改 `serve`/`handle_conn` 签名,加一个参数**

`crates/dozerd/src/server.rs`:

```rust
pub async fn serve(
    socket: &Path,
    registry: Arc<SessionRegistry>,
    store: Arc<crate::acceptance::AcceptanceStore>,
    projects: Arc<crate::projects::ProjectStore>,
    bookmarks: Arc<crate::bookmarks::BookmarkStore>,
) -> Result<()> {
```

对应循环体里 clone 三兄弟那一段,追加 `bookmarks`:

```rust
        let registry = registry.clone();
        let store = store.clone();
        let projects = projects.clone();
        let bookmarks = bookmarks.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_conn(stream, registry, store, projects, bookmarks).await {
```

`handle_conn` 签名同步加参数:

```rust
async fn handle_conn(
    stream: UnixStream,
    registry: Arc<SessionRegistry>,
    store: Arc<crate::acceptance::AcceptanceStore>,
    projects: Arc<crate::projects::ProjectStore>,
    bookmarks: Arc<crate::bookmarks::BookmarkStore>,
) -> Result<()> {
```

- [x] **Step 2: 三个新 Request 分支**

紧跟 `Request::GetAcceptanceCount { repo } => ...,` 之后(`match req` 的最后一支之前)加:

```rust
                        Request::AddBookmark {
                            scope,
                            project_id,
                            url,
                            title,
                        } => match bookmarks.add(scope, project_id, &url, &title) {
                            Ok(_) => Reply::Ok,
                            Err(e) => Reply::Error {
                                message: format!("加入收藏失败: {e}"),
                            },
                        },
                        Request::RemoveBookmark { id } => match bookmarks.remove(id) {
                            Ok(()) => Reply::Ok,
                            Err(e) => Reply::Error {
                                message: format!("移除收藏失败: {e}"),
                            },
                        },
                        Request::ListBookmarks { project_id } => {
                            match bookmarks.list(project_id) {
                                Ok(bookmarks) => Reply::Bookmarks { bookmarks },
                                Err(e) => Reply::Error {
                                    message: format!("列收藏失败: {e}"),
                                },
                            }
                        }
```

- [x] **Step 3: `main.rs` 构造 `BookmarkStore` 并传入 `serve`**

```rust
    let projects = Arc::new(dozerd::projects::ProjectStore::new(
        &dozer_core::paths::state_dir().join("dozer.db"),
    )?);
    let store = Arc::new(dozerd::acceptance::AcceptanceStore::open(
        &dozer_core::paths::state_dir().join("dozer.db"),
    )?);
    let bookmarks = Arc::new(dozerd::bookmarks::BookmarkStore::new(
        &dozer_core::paths::state_dir().join("dozer.db"),
    )?);
    let serve = dozerd::server::serve(&socket, registry, store, projects, bookmarks);
```

- [x] **Step 4: 编译 + 跑既有测试确认没弄坏别的**

Run: `cargo build -p dozerd && cargo test -p dozerd`
Expected: 编译通过,`server.rs` 里 `agent_state_mapping_matches_spec_d6` 等既有测试照常 PASS。

- [x] **Step 5: Commit**

```bash
git add crates/dozerd/src/server.rs crates/dozerd/src/main.rs
git commit -m "feat(dozerd): wire AddBookmark/RemoveBookmark/ListBookmarks into server"
```

---

### Task 4: dozer-client Client 方法

**Files:**
- Modify: `crates/dozer-client/src/lib.rs`

**Interfaces:**
- Consumes: `Request::{AddBookmark,RemoveBookmark,ListBookmarks}`/`Reply::{Ok,Bookmarks,Error}`(Task 1)
- Produces:
  - `Client::add_bookmark(&self, scope: BookmarkScope, project_id: Option<i64>, url: &str, title: &str) -> Result<()>`
  - `Client::remove_bookmark(&self, id: i64) -> Result<()>`
  - `Client::list_bookmarks(&self, project_id: Option<i64>) -> Result<Vec<BookmarkInfo>>`

- [x] **Step 1: 扩 import**

```rust
use dozer_core::protocol::{
    AgentKind, AgentState, BookmarkInfo, BookmarkScope, ProjectInfo, Reply, Request, SessionInfo,
    decode_line, encode_line,
};
```

- [x] **Step 2: 三个方法(紧跟 `acceptance_count` 之后)**

```rust
    pub async fn add_bookmark(
        &self,
        scope: BookmarkScope,
        project_id: Option<i64>,
        url: &str,
        title: &str,
    ) -> Result<()> {
        match self
            .roundtrip(&Request::AddBookmark {
                scope,
                project_id,
                url: url.into(),
                title: title.into(),
            })
            .await?
        {
            Reply::Ok => Ok(()),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn remove_bookmark(&self, id: i64) -> Result<()> {
        match self.roundtrip(&Request::RemoveBookmark { id }).await? {
            Reply::Ok => Ok(()),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn list_bookmarks(&self, project_id: Option<i64>) -> Result<Vec<BookmarkInfo>> {
        match self
            .roundtrip(&Request::ListBookmarks { project_id })
            .await?
        {
            Reply::Bookmarks { bookmarks } => Ok(bookmarks),
            other => bail!("意外应答: {other:?}"),
        }
    }
```

- [x] **Step 3: 编译确认**

Run: `cargo build -p dozer-client`
Expected: 编译通过(`dozer-client` 本身没有单测,靠 `dozer-core` 的协议测试兜底,同现有约定)。

- [x] **Step 4: Commit**

```bash
git add crates/dozer-client/src/lib.rs
git commit -m "feat(dozer-client): add bookmark RPC methods"
```

---

### Task 5: dozer-app 收藏夹纯逻辑模块

**Files:**
- Create: `crates/dozer-app/src/bookmarks.rs`
- Modify: `crates/dozer-app/src/main.rs`(加 `mod bookmarks;`)

**Interfaces:**
- Consumes: `dozer_core::protocol::{BookmarkInfo, BookmarkScope}`
- Produces:
  - `pub const OPTIMISTIC_BOOKMARK_ID: i64`
  - `pub struct BookmarkStatus { pub global: Option<i64>, pub project: Option<i64> }` + `impl BookmarkStatus { pub fn is_bookmarked(&self) -> bool }`
  - `pub fn bookmark_status(bookmarks: &[BookmarkInfo], url: &str, project_id: Option<i64>) -> BookmarkStatus`
  - `pub fn optimistic_add(bookmarks: &mut Vec<BookmarkInfo>, scope: BookmarkScope, project_id: Option<i64>, url: &str, title: &str, created_ms: u64)`
  - `pub fn optimistic_remove(bookmarks: &mut Vec<BookmarkInfo>, id: i64)`

- [x] **Step 1: 写失败的单测(先写整份文件,含测试)**

创建 `crates/dozer-app/src/bookmarks.rs`:

```rust
//! 浏览器收藏夹:纯逻辑辅助,不碰 iced/网络。
//!
//! `Workspace::bookmarks` 是"全局 + 当前项目"合集的本地缓存,由
//! `ListBookmarks` 落地时整份替换(见 `workspace.rs`
//! `spawn_bookmarks_refresh`)。这里只提供两类纯函数:
//! - `bookmark_status`:给定当前 URL,判定它在全局/本项目两边各自是否
//!   已收藏(星标图标颜色、小菜单"加入"/"移出"文案据此渲染)。
//! - `optimistic_add`/`optimistic_remove`:用户点击的当帧本地立即改
//!   `Workspace::bookmarks`,不等 dozerd 往返——星标/面板立即反馈,真实
//!   持久化结果由随后一次 `ListBookmarks` 全量刷新纠正(不做显式回滚,
//!   见设计文档"数据流与状态机"一节)。

use dozer_core::protocol::{BookmarkInfo, BookmarkScope};

/// 乐观本地插入的占位 id:落库前不知道真实自增 id,只在"点击→下一次
/// 全量刷新落地"这一帧内部当哨兵用,不参与任何持久化或跨帧比较。
pub const OPTIMISTIC_BOOKMARK_ID: i64 = -1;

/// 当前 URL 在全局/本项目两边各自的收藏状态(`Some(id)` = 已收藏,
/// `id` 是"移出收藏"要传的记录 id)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BookmarkStatus {
    pub global: Option<i64>,
    pub project: Option<i64>,
}

impl BookmarkStatus {
    pub fn is_bookmarked(&self) -> bool {
        self.global.is_some() || self.project.is_some()
    }
}

pub fn bookmark_status(
    bookmarks: &[BookmarkInfo],
    url: &str,
    project_id: Option<i64>,
) -> BookmarkStatus {
    let global = bookmarks
        .iter()
        .find(|b| b.scope == BookmarkScope::Global && b.url == url)
        .map(|b| b.id);
    let project = project_id.and_then(|pid| {
        bookmarks
            .iter()
            .find(|b| b.scope == BookmarkScope::Project && b.project_id == Some(pid) && b.url == url)
            .map(|b| b.id)
    });
    BookmarkStatus { global, project }
}

/// 已存在(同 scope+project_id+url)则 no-op,幂等,与 dozerd 侧
/// `INSERT OR IGNORE` 语义一致。
pub fn optimistic_add(
    bookmarks: &mut Vec<BookmarkInfo>,
    scope: BookmarkScope,
    project_id: Option<i64>,
    url: &str,
    title: &str,
    created_ms: u64,
) {
    let exists = bookmarks
        .iter()
        .any(|b| b.scope == scope && b.project_id == project_id && b.url == url);
    if exists {
        return;
    }
    bookmarks.push(BookmarkInfo {
        id: OPTIMISTIC_BOOKMARK_ID,
        scope,
        project_id,
        url: url.to_string(),
        title: title.to_string(),
        created_ms,
    });
}

/// 按 id 过滤;未知 id 是 no-op,同 dozerd 侧 `remove` 语义。
pub fn optimistic_remove(bookmarks: &mut Vec<BookmarkInfo>, id: i64) {
    bookmarks.retain(|b| b.id != id);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bm(id: i64, scope: BookmarkScope, project_id: Option<i64>, url: &str) -> BookmarkInfo {
        BookmarkInfo {
            id,
            scope,
            project_id,
            url: url.into(),
            title: url.into(),
            created_ms: 0,
        }
    }

    #[test]
    fn bookmark_status_detects_global_and_project_independently() {
        let list = vec![
            bm(1, BookmarkScope::Global, None, "https://a.com"),
            bm(2, BookmarkScope::Project, Some(7), "https://a.com"),
        ];
        let status = bookmark_status(&list, "https://a.com", Some(7));
        assert_eq!(status.global, Some(1));
        assert_eq!(status.project, Some(2));
        assert!(status.is_bookmarked());
    }

    #[test]
    fn bookmark_status_project_none_when_no_project_open() {
        let list = vec![bm(1, BookmarkScope::Global, None, "https://a.com")];
        let status = bookmark_status(&list, "https://a.com", None);
        assert_eq!(status.global, Some(1));
        assert_eq!(status.project, None);
    }

    #[test]
    fn bookmark_status_project_scoped_to_current_project_only() {
        let list = vec![bm(1, BookmarkScope::Project, Some(7), "https://a.com")];
        let status = bookmark_status(&list, "https://a.com", Some(8));
        assert_eq!(status.project, None, "不该看到别的项目的收藏");
    }

    #[test]
    fn is_bookmarked_false_when_neither_side_has_it() {
        let list: Vec<BookmarkInfo> = vec![];
        assert!(!bookmark_status(&list, "https://a.com", Some(1)).is_bookmarked());
    }

    #[test]
    fn optimistic_add_appends_new_entry() {
        let mut list = vec![];
        optimistic_add(&mut list, BookmarkScope::Global, None, "https://a.com", "A", 100);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, OPTIMISTIC_BOOKMARK_ID);
        assert_eq!(list[0].title, "A");
    }

    #[test]
    fn optimistic_add_is_idempotent_for_same_scope_and_url() {
        let mut list = vec![];
        optimistic_add(&mut list, BookmarkScope::Global, None, "https://a.com", "A", 100);
        optimistic_add(&mut list, BookmarkScope::Global, None, "https://a.com", "改名", 200);
        assert_eq!(list.len(), 1, "已存在则不重复插入");
        assert_eq!(list[0].title, "A");
    }

    #[test]
    fn optimistic_add_allows_same_url_in_different_scope() {
        let mut list = vec![];
        optimistic_add(&mut list, BookmarkScope::Global, None, "https://a.com", "A", 100);
        optimistic_add(&mut list, BookmarkScope::Project, Some(1), "https://a.com", "A", 100);
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn optimistic_remove_filters_by_id() {
        let mut list = vec![
            bm(1, BookmarkScope::Global, None, "https://a.com"),
            bm(2, BookmarkScope::Global, None, "https://b.com"),
        ];
        optimistic_remove(&mut list, 1);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, 2);
    }

    #[test]
    fn optimistic_remove_unknown_id_is_noop() {
        let mut list = vec![bm(1, BookmarkScope::Global, None, "https://a.com")];
        optimistic_remove(&mut list, 999);
        assert_eq!(list.len(), 1);
    }
}
```

- [x] **Step 2: 声明模块**

`crates/dozer-app/src/main.rs`,按字母序插到 `assets;` 之后:

```rust
mod assets;
mod bookmarks;
mod chrome_style;
```

- [x] **Step 3: 运行测试确认通过**

Run: `cargo test -p dozer-app bookmarks::`
Expected: 8 个测试全部 PASS。

- [x] **Step 4: Commit**

```bash
git add crates/dozer-app/src/bookmarks.rs crates/dozer-app/src/main.rs
git commit -m "feat(dozer-app): add pure bookmark status/optimistic-update helpers"
```

---

### Task 6: `Workspace` 状态 + `Message` + 异步刷新接线

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`

**Interfaces:**
- Consumes: `crate::bookmarks::{bookmark_status, optimistic_add, optimistic_remove}`(Task 5)、`Client::{add_bookmark,remove_bookmark,list_bookmarks}`(Task 4)、`dozer_core::protocol::{BookmarkInfo, BookmarkScope}`(Task 1)
- Produces(供 Task 7 的 UI 渲染代码使用):
  - `Workspace.bookmarks: Vec<BookmarkInfo>`(private 字段,`workspace.rs` 内其余函数可直接访问)
  - `Workspace.browser_bookmarks_open: bool`
  - `Workspace.browser_star_menu_open: bool`
  - `Message::BrowserStarClick`
  - `Message::BrowserBookmarkAdd(BookmarkScope)`
  - `Message::BrowserBookmarkRemove(i64)`
  - `Message::BrowserBookmarksToggle`

- [x] **Step 1: import 新协议类型**

`crates/dozer-app/src/workspace.rs` 顶部:

```rust
use dozer_core::protocol::{
    AgentKind, AgentState, BookmarkInfo, BookmarkScope, ProjectInfo, SessionInfo,
};
```

- [x] **Step 2: `Workspace` 结构体新增三个字段**

紧跟 `browser_error: Option<String>,` 之后(约第 1478 行):

```rust
    browser_error: Option<String>,
    /// 收藏夹本地缓存(全局 + 当前项目合集),`Message::BrowserBookmarksLoaded`
    /// 落地时整份替换;加入/移出走乐观本地更新,见 `crate::bookmarks`。
    bookmarks: Vec<BookmarkInfo>,
    /// 收藏夹下拉面板(tab 栏"收藏夹"图标按钮)开合。
    browser_bookmarks_open: bool,
    /// 地址栏星标"加入/移出收藏"小菜单开合。
    browser_star_menu_open: bool,
```

- [x] **Step 3: `empty_for_project_placeholder()` 补三个字段初始值**

在该函数里紧跟 `browser_tab_first: 0,` 之后:

```rust
            browser_tab_first: 0,
            bookmarks: Vec::new(),
            browser_bookmarks_open: false,
            browser_star_menu_open: false,
```

- [x] **Step 4: 编译确认字段接线正确(此时新 Message 变体还没加,先只验证结构体)**

Run: `cargo build -p dozer-app 2>&1 | head -50`
Expected: 大概率还有别的报错(Message 变体/字段用途都还没接),预期能看到的只是"未使用字段"类 warning,不应有关于 `bookmarks`/`browser_bookmarks_open`/`browser_star_menu_open` 字段缺失初始化的 error。若报"missing field",说明还有别的构造点用了旧式全字段字面量,回去补上(全仓库搜 `Workspace {` 逐个确认)。

- [x] **Step 5: `Message` 枚举加五个新变体**

紧跟 `BrowserAddrEvent(AddrEvent),` 之后(约第 1092 行):

```rust
    /// 浏览器:地址栏编辑事件。
    BrowserAddrEvent(AddrEvent),
    /// 浏览器:点击地址栏星标,开合"加入/移出收藏"小菜单。
    BrowserStarClick,
    /// 浏览器:星标菜单里点"加入全局/本项目收藏"。
    BrowserBookmarkAdd(BookmarkScope),
    /// 浏览器:星标菜单/收藏面板里点"移出收藏"(按 dozerd 记录 id)。
    BrowserBookmarkRemove(i64),
    /// 浏览器:tab 栏"收藏夹"图标按钮,开合下拉面板。
    BrowserBookmarksToggle,
```

再找到 `AcceptanceCountLoaded(ProjectId, Option<u64>),` 那一行(约 1150 行),之后追加:

```rust
    /// 项目:当前项目验收次数刷新结果(项目卡"N 次验收"副行用)。
    AcceptanceCountLoaded(ProjectId, Option<u64>),
    /// 浏览器:收藏夹"全局+当前项目"合集刷新结果(项目打开/切换,或一次
    /// 增删收藏之后的重新拉取)。
    BrowserBookmarksLoaded(ProjectId, Vec<BookmarkInfo>),
    /// 浏览器:一次 `AddBookmark`/`RemoveBookmark` 往返完成——无论成功
    /// 失败都触发一次 `BrowserBookmarksLoaded` 式的全量刷新去纠正本地
    /// 乐观更新;失败时额外把错误文案落进 `browser_error`。
    BrowserBookmarksMutated(ProjectId, Result<(), String>),
```

- [x] **Step 6: `spawn_bookmarks_refresh` 方法**

在 `impl Workspace` 块里,紧跟 `spawn_acceptance_count_refresh` 方法之后(约第 2166 行):

```rust
    /// 异步拉取"全局 + 当前项目"收藏夹合集 → `BrowserBookmarksLoaded`。
    fn spawn_bookmarks_refresh(&self, io: &ShellIo) {
        let Some(p) = &self.project else { return };
        let project_id = p.id;
        let client = io.client.clone();
        let proxy = io.proxy.clone();
        io.handle.spawn(async move {
            let bookmarks = client
                .list_bookmarks(Some(project_id))
                .await
                .unwrap_or_default();
            let _ = proxy.send_event(Message::BrowserBookmarksLoaded(project_id, bookmarks));
        });
    }
```

- [x] **Step 7: 两处刷新时机接线**

`from_restore`(约第 1771 行),紧跟 `ws.spawn_acceptance_count_refresh(io);` 之后:

```rust
        ws.spawn_acceptance_count_refresh(io);
        ws.spawn_bookmarks_refresh(io);
```

`adopt_project`(约第 2286 行),同样紧跟 `self.spawn_acceptance_count_refresh(io);` 之后:

```rust
        self.spawn_acceptance_count_refresh(io);
        self.spawn_bookmarks_refresh(io);
```

- [x] **Step 8: `update()` 里新增 6 个消息分支**

紧跟现有 `Message::BrowserAddrEvent(ev) => { ... }` 分支之后(约第 4332 行,`BrowserOpenUrl`/`BrowserSelectTab` 等其余 Browser* 分支同一片区域):

```rust
            Message::BrowserStarClick => {
                self.with_focused_project(|ws, _io| {
                    ws.browser_error = None;
                    ws.browser_star_menu_open = !ws.browser_star_menu_open;
                });
            }
            Message::BrowserBookmarksToggle => {
                self.with_focused_project(|ws, _io| {
                    ws.browser_bookmarks_open = !ws.browser_bookmarks_open;
                });
            }
            Message::BrowserBookmarkAdd(scope) => {
                self.with_focused_project(|ws, io| {
                    ws.browser_star_menu_open = false;
                    let Some(project_id) = ws.project.as_ref().map(|p| p.id) else {
                        return;
                    };
                    let Some(tab) = ws.browser.tabs().get(ws.browser.active_idx()) else {
                        return;
                    };
                    let TabKind::Web { url } = tab.kind.clone() else {
                        return;
                    };
                    let title = tab.title.clone();
                    let target_project_id = match scope {
                        BookmarkScope::Global => None,
                        BookmarkScope::Project => Some(project_id),
                    };
                    let created_ms = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis() as u64)
                        .unwrap_or(0);
                    bookmarks::optimistic_add(
                        &mut ws.bookmarks,
                        scope,
                        target_project_id,
                        &url,
                        &title,
                        created_ms,
                    );
                    let client = io.client.clone();
                    let proxy = io.proxy.clone();
                    io.handle.spawn(async move {
                        let res = client
                            .add_bookmark(scope, target_project_id, &url, &title)
                            .await
                            .map_err(|e| e.to_string());
                        let _ = proxy.send_event(Message::BrowserBookmarksMutated(project_id, res));
                    });
                });
            }
            Message::BrowserBookmarkRemove(id) => {
                self.with_focused_project(|ws, io| {
                    ws.browser_star_menu_open = false;
                    let Some(project_id) = ws.project.as_ref().map(|p| p.id) else {
                        return;
                    };
                    bookmarks::optimistic_remove(&mut ws.bookmarks, id);
                    let client = io.client.clone();
                    let proxy = io.proxy.clone();
                    io.handle.spawn(async move {
                        let res = client.remove_bookmark(id).await.map_err(|e| e.to_string());
                        let _ = proxy.send_event(Message::BrowserBookmarksMutated(project_id, res));
                    });
                });
            }
            Message::BrowserBookmarksLoaded(project_id, bookmarks) => {
                self.with_project(project_id, move |ws, _io| {
                    ws.bookmarks = bookmarks;
                });
            }
            Message::BrowserBookmarksMutated(project_id, res) => {
                self.with_project(project_id, move |ws, io| {
                    if let Err(msg) = res {
                        ws.browser_error = Some(msg);
                    }
                    ws.spawn_bookmarks_refresh(io);
                });
            }
```

同时在文件顶部加 `use crate::bookmarks;`(紧跟其余 `use crate::` 系列 import,如 `use crate::preview_state;` 之后)。

- [x] **Step 9: 编译 + 跑 dozer-app 全量测试**

Run: `cargo build -p dozer-app && cargo test -p dozer-app`
Expected: 编译通过;既有测试(包括 Task 5 的 8 个 `bookmarks::` 测试)全绿。此时 UI 还没接星标/面板按钮,`browser_bookmarks_open`/`browser_star_menu_open`/新 Message 变体会有"从未被构造"之外的 dead-code 提示是正常的,留给 Task 7 消化;若报真正的类型错误(签名对不上),对照本步骤代码块逐字核对。

- [x] **Step 10: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "feat(dozer-app): wire bookmark state, messages, and async refresh into Workspace"
```

---

### Task 7: 图标资源 + 地址栏星标/收藏面板 UI

**Files:**
- Create: `crates/dozer-app/assets/icons/star.svg`
- Create: `crates/dozer-app/assets/icons/bookmark.svg`
- Modify: `crates/dozer-app/src/icons.rs`
- Modify: `crates/dozer-app/src/workspace.rs`(`browser_pane` 及其辅助渲染函数)

**Interfaces:**
- Consumes: `Workspace.bookmarks`/`browser_bookmarks_open`/`browser_star_menu_open`(Task 6)、`bookmarks::bookmark_status`(Task 5)
- Produces: 无(叶子任务,UI 渲染)

- [x] **Step 1: 新增两个 Lucide SVG 资源(MIT/ISC,同现有 `assets/icons/LICENSE` 覆盖范围)**

`crates/dozer-app/assets/icons/star.svg`:

```xml
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
  <path d="M11.525 2.295a.53.53 0 0 1 .95 0l2.31 4.679a2.123 2.123 0 0 0 1.595 1.16l5.166.756a.53.53 0 0 1 .294.904l-3.736 3.638a2.123 2.123 0 0 0-.611 1.878l.882 5.14a.53.53 0 0 1-.771.56l-4.618-2.428a2.122 2.122 0 0 0-1.973 0L6.396 21.01a.53.53 0 0 1-.77-.56l.881-5.139a2.122 2.122 0 0 0-.611-1.879L2.16 9.795a.53.53 0 0 1 .294-.906l5.165-.755a2.122 2.122 0 0 0 1.597-1.16z" />
</svg>
```

`crates/dozer-app/assets/icons/bookmark.svg`:

```xml
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
  <path d="M17 3a2 2 0 0 1 2 2v15a1 1 0 0 1-1.496.868l-4.512-2.578a2 2 0 0 0-1.984 0l-4.512 2.578A1 1 0 0 1 5 20V5a2 2 0 0 1 2-2z" />
</svg>
```

- [x] **Step 2: `icons.rs` 加两个 `IconKind` 变体**

`IconKind` 枚举里紧跟 `ListChecks,` 之后:

```rust
    ListChecks,
    /// 浏览器地址栏"加入/移出收藏"星标(Lucide star)。已收藏态靠调用方
    /// 传 GOLD 而非切换到另一份实心图标——`icons::view` 只管描边色,单一
    /// 资源足够表达"已收藏/未收藏"两态(YAGNI,不新增 filled 变体)。
    Star,
    /// tab 栏"收藏夹"下拉面板触发图标(Lucide bookmark)。
    Bookmark,
```

`bytes()` 方法里紧跟 `IconKind::ListChecks => include_bytes!("../assets/icons/list-checks.svg"),` 之后:

```rust
            IconKind::ListChecks => include_bytes!("../assets/icons/list-checks.svg"),
            IconKind::Star => include_bytes!("../assets/icons/star.svg"),
            IconKind::Bookmark => include_bytes!("../assets/icons/bookmark.svg"),
```

- [x] **Step 3: 编译确认图标资源接线正确**

Run: `cargo build -p dozer-app`
Expected: 编译通过(两个新 `include_bytes!` 路径存在即可)。

- [x] **Step 4: `browser_pane` 里加星标/收藏夹两个图标按钮 + 三个新渲染函数**

在 `crates/dozer-app/src/workspace.rs` 里,`browser_pane` 函数内找到这一段(约第 8218-8240 行):

```rust
    let editing = ws.browser.addr_editing();
    let addr_text = if editing {
        format!("{}▏", ws.browser.addr_buffer())
    } else {
        "输入网址".to_string()
    };
    let addr = button(lh(text(addr_text)
        .size(workspace_font::body())
        .color(if editing { theme::CREAM } else { theme::DIM })))
    .on_press(Message::BrowserAddrClick)
    .width(Length::Fill)
    .style(move |_t, _s| button::Style {
        background: Some(theme::TERM_BG.into()),
        text_color: theme::CREAM,
        border: Border {
            color: if editing { theme::GOLD } else { theme::BORDER },
            width: 1.0,
            radius: 2.0.into(),
        },
        ..button::Style::default()
    });

    let mut content = column![tab_bar, tab_divider(), addr].spacing(region.gap);
```

替换成:

```rust
    let editing = ws.browser.addr_editing();
    let addr_text = if editing {
        format!("{}▏", ws.browser.addr_buffer())
    } else {
        "输入网址".to_string()
    };
    let addr = button(lh(text(addr_text)
        .size(workspace_font::body())
        .color(if editing { theme::CREAM } else { theme::DIM })))
    .on_press(Message::BrowserAddrClick)
    .width(Length::Fill)
    .style(move |_t, _s| button::Style {
        background: Some(theme::TERM_BG.into()),
        text_color: theme::CREAM,
        border: Border {
            color: if editing { theme::GOLD } else { theme::BORDER },
            width: 1.0,
            radius: 2.0.into(),
        },
        ..button::Style::default()
    });

    let addr_row = row![addr, browser_star_button(ws), browser_bookmarks_toggle_button()]
        .spacing(4)
        .align_y(iced_widget::core::Alignment::Center);

    let mut content = column![tab_bar, tab_divider(), addr_row].spacing(region.gap);

    if ws.browser_star_menu_open {
        content = content.push(browser_star_menu_popup(ws));
    }
    if ws.browser_bookmarks_open {
        content = content.push(browser_bookmarks_panel(ws));
    }
```

紧接着,在 `browser_pane` 函数（闭合 `}` 之后)追加以下辅助渲染函数与 `current_browser_url` 工具函数(放在 `browser_pane` 和下一个函数 `terminal_pane` 之间):

```rust
/// 当前浏览器激活 tab 若是网页,取其 URL;文件/验收 tab 返回 `None`
/// (星标按钮据此判定是否可点、菜单据此判定收藏状态)。
fn current_browser_url(ws: &Workspace) -> Option<String> {
    match ws.browser.tabs().get(ws.browser.active_idx()).map(|t| &t.kind) {
        Some(TabKind::Web { url }) => Some(url.clone()),
        _ => None,
    }
}

/// 地址栏星标:当前 URL 在全局/本项目任一边已收藏则 GOLD 实心,否则
/// DIM;非网页 tab(文件/验收)禁用。
fn browser_star_button(ws: &Workspace) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let url = current_browser_url(ws);
    let starred = url
        .as_ref()
        .map(|u| {
            bookmarks::bookmark_status(&ws.bookmarks, u, ws.project.as_ref().map(|p| p.id))
                .is_bookmarked()
        })
        .unwrap_or(false);
    let color = if starred { theme::GOLD } else { theme::DIM };
    let mut btn = button(icons::view(icons::IconKind::Star, crate::icon_size::row(), color))
        .width(Length::Fixed(crate::workspace_geometry::tab_button_size()))
        .height(Length::Fixed(crate::workspace_geometry::tab_button_size()))
        .padding(0)
        .style(move |_t, _s| button::Style {
            background: None,
            text_color: color,
            ..button::Style::default()
        });
    if url.is_some() {
        btn = btn.on_press(Message::BrowserStarClick);
    }
    btn.into()
}

/// tab 栏"收藏夹"下拉面板触发按钮,颜色恒定(不像星标那样带收藏状态)。
fn browser_bookmarks_toggle_button<'a>() -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    button(icons::view(
        icons::IconKind::Bookmark,
        crate::icon_size::row(),
        theme::DIM,
    ))
    .on_press(Message::BrowserBookmarksToggle)
    .width(Length::Fixed(crate::workspace_geometry::tab_button_size()))
    .height(Length::Fixed(crate::workspace_geometry::tab_button_size()))
    .padding(0)
    .style(|_t, _s| button::Style {
        background: None,
        text_color: theme::DIM,
        ..button::Style::default()
    })
    .into()
}

fn bookmark_menu_row(label: String, msg: Message) -> Element<'static, Message, iced_widget::Theme, iced_widget::Renderer> {
    button(lh(text(label)
        .size(workspace_font::body())
        .color(theme::CREAM)))
    .on_press(msg)
    .width(Length::Fill)
    .padding([6, 12])
    .style(|_t: &iced_widget::Theme, _s| button::Style {
        background: None,
        text_color: theme::CREAM,
        ..button::Style::default()
    })
    .into()
}

/// 星标小菜单:未收藏显示"加入…",已收藏显示"移出…"(打勾态)。当前
/// tab 非网页时(`current_browser_url` 返回 `None`)不该能弹出这个菜单
/// (`browser_star_button` 已经不给非网页 tab 挂 `on_press`),这里仍防御
/// 性处理为空内容,不 panic。
fn browser_star_menu_popup(ws: &Workspace) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let Some(url) = current_browser_url(ws) else {
        return column![].into();
    };
    let project_id = ws.project.as_ref().map(|p| p.id);
    let status = bookmarks::bookmark_status(&ws.bookmarks, &url, project_id);

    let mut col = column![match status.global {
        Some(id) => bookmark_menu_row("移出全局收藏".to_string(), Message::BrowserBookmarkRemove(id)),
        None => bookmark_menu_row(
            "加入全局收藏".to_string(),
            Message::BrowserBookmarkAdd(BookmarkScope::Global)
        ),
    }]
    .spacing(2);

    if project_id.is_some() {
        col = col.push(match status.project {
            Some(id) => bookmark_menu_row("移出本项目收藏".to_string(), Message::BrowserBookmarkRemove(id)),
            None => bookmark_menu_row(
                "加入本项目收藏".to_string(),
                Message::BrowserBookmarkAdd(BookmarkScope::Project),
            ),
        });
    }

    container(col)
        .padding(6)
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(theme::CARD.into()),
            border: Border {
                color: theme::BORDER,
                width: 1.0,
                radius: 6.0.into(),
            },
            ..container::Style::default()
        })
        .into()
}

/// 一组收藏条目:标题(点击新开 tab)+ `×` 删除按钮,风格照抄 tab 关闭
/// 按钮。
fn bookmark_group<'a>(
    title: &'static str,
    items: &[&'a BookmarkInfo],
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    let mut col =
        column![lh(text(title).size(workspace_font::subtitle()).color(theme::DIM))].spacing(2);
    for b in items {
        let open = button(lh(text(b.title.clone())
            .size(workspace_font::body())
            .color(theme::CREAM)))
        .on_press(Message::BrowserOpenUrl(b.url.clone()))
        .width(Length::Fill)
        .style(|_t: &iced_widget::Theme, _s| button::Style {
            background: None,
            text_color: theme::CREAM,
            ..button::Style::default()
        });
        let remove = button(lh(text("×").size(workspace_font::body()).color(theme::DIM)))
            .on_press(Message::BrowserBookmarkRemove(b.id))
            .style(|_t: &iced_widget::Theme, _s| button::Style {
                background: None,
                text_color: theme::DIM,
                ..button::Style::default()
            });
        col = col.push(
            row![open, remove]
                .spacing(4)
                .align_y(iced_widget::core::Alignment::Center),
        );
    }
    col.into()
}

/// 收藏夹下拉面板:分"全局收藏"/"本项目收藏"两组,都为空时显示占位文案。
fn browser_bookmarks_panel(ws: &Workspace) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let project_id = ws.project.as_ref().map(|p| p.id);
    let global: Vec<&BookmarkInfo> = ws
        .bookmarks
        .iter()
        .filter(|b| b.scope == BookmarkScope::Global)
        .collect();
    let project: Vec<&BookmarkInfo> = ws
        .bookmarks
        .iter()
        .filter(|b| b.scope == BookmarkScope::Project && b.project_id == project_id)
        .collect();

    let both_empty = global.is_empty() && project.is_empty();
    let mut col = column![].spacing(6);
    col = col.push(bookmark_group("全局收藏", &global));
    if project_id.is_some() {
        col = col.push(bookmark_group("本项目收藏", &project));
    }
    if both_empty {
        col = col.push(lh(text("暂无收藏")
            .size(workspace_font::subtitle())
            .color(theme::DIM)));
    }

    container(col)
        .padding(6)
        .width(Length::Fill)
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(theme::CARD.into()),
            border: Border {
                color: theme::BORDER,
                width: 1.0,
                radius: 6.0.into(),
            },
            ..container::Style::default()
        })
        .into()
}
```

- [x] **Step 5: 编译,逐条修正类型报错**

Run: `cargo build -p dozer-app 2>&1 | head -100`
Expected: 大概率有若干 borrow/类型细节需要按报错信息微调(例如 `Vec<&BookmarkInfo>` 的生命周期、`match` 表达式当 `column!` 宏参数时的类型推断);逐条修正直至 `cargo build -p dozer-app` 干净通过。**不要**为了让它编译过而删掉设计要求的行为(比如"本项目未打开时不渲染该组"),只调整语法层面的写法。

- [x] **Step 6: `cargo test`/`clippy`/`fmt` 全绿**

Run: `cargo test -p dozer-app && cargo clippy -p dozer-app --all-targets && cargo fmt --check`
Expected: 全部通过;`fmt --check` 若报差异,跑 `cargo fmt` 后重新 `git diff` 确认改动仍符合本计划描述的逻辑,再继续。

- [x] **Step 7: Commit**

```bash
git add crates/dozer-app/assets/icons/star.svg crates/dozer-app/assets/icons/bookmark.svg \
        crates/dozer-app/src/icons.rs crates/dozer-app/src/workspace.rs
git commit -m "feat(dozer-app): render bookmark star button, add/remove menu, and list panel"
```

---

### Task 8: 全量校验与人工验收

**Files:** 无新增/修改(纯校验任务)

- [x] **Step 1: 全 workspace 构建**

Run: `cargo build`
Expected: 全部 crate 编译通过。

- [x] **Step 2: 全 workspace 测试**

Run: `cargo test`
Expected: 全绿,重点关注 `dozer-core`(Task 1)、`dozerd`(Task 2/3)、`dozer-app`(Task 5/6)新增的测试都在列表里跑到了。

- [x] **Step 3: clippy + fmt**

Run: `cargo clippy --all-targets && cargo fmt --check`
Expected: 无警告、无格式差异。

- [ ] **Step 4: 人工验收(`cargo run -p dozer-app`)**

启动 GUI,打开任意一个项目,点左图标栏"地球"进入浏览器,在地址栏输入一个网址打开:
1. 点星标 → 弹出菜单选"加入全局收藏" → 星标变 GOLD 实心。
2. 点 tab 栏"收藏夹"图标 → 面板展开,"全局收藏"组里能看到刚加的这条,标题与 tab 标题一致。
3. 关掉这个 tab,再点面板里的这条 → 应新开 tab 打开同一网址。
4. 点星标 → 菜单文案应显示"移出全局收藏"(带打勾态);点"加入本项目收藏" → 面板"本项目收藏"组同步出现这条。
5. 切到另一个项目页签,打开浏览器 → 面板"本项目收藏"组应为空(或只有该项目自己的),但"全局收藏"组应看到第 1 步加的那条(全局共享)。
6. 面板/菜单里点 `×`/"移出" → 条目消失,星标回到 DIM 空心。
7. 重启 `dozerd`(或直接重启整个 `cargo run -p dozer-app`)→ 之前加的收藏应该还在(验证真的落了库,不是纯内存)。

若上述任一步与预期不符,回到对应 Task 排查(状态判定在 Task 5/6,持久化在 Task 2/3,渲染在 Task 7)。

- [x] **Step 5: 最终确认没有遗留未提交的改动**

Run: `git status`
Expected: 干净(所有改动都已在前面各 Task 的 Step 里提交)。
