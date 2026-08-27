# 会话总结生成管线 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 关闭一个 Claude/CodeBuddy/OpenCode/V8agent 的会话 tab 时,在真正 kill 之前用还存活的 PTY 里的 agent 生成一份"标题+摘要"总结并落库(agent 经 `dozer-mcp` 新写工具交回),agent 没在超时窗口内响应时用已摄取的对话数据做启发式兜底,保证每个走过这条路径的 session 都留一行可回看的总结记录。

**Architecture:** 新增 `dozerd` 侧 `session_summaries` sqlite 表 + `dozer-core::protocol` 三对新 Request/Reply + `dozer-client` 三个新方法;`dozer-mcp` 新增第一个写工具并补自动注册;`dozer-app` 把"关闭 tab"对已知四家 agent 的行为从直接 `Kill` 改成先发 `CloseWithSummary`(dozerd 后台注入 prompt→轮询→超时启发式兜底→kill)。

**Tech Stack:** Rust workspace(iced 0.14 / tokio / rusqlite / rmcp),UDS + JSON Lines 协议。

**Spec:** `docs/superpowers/specs/2026-08-27-session-summary-pipeline-design.md`

## Global Constraints

- 在独立分支(如 `feature/session-summary-pipeline`)上开发,不要直接提交到 main;全部 Task 完成、构建/测试/clippy/fmt 全绿后提请审阅,审阅通过再合并。
- 关闭 tab 时 UI 必须立即移除该 tab,不等待任何后台结果(现状行为不变)。
- `dozerd` 生产代码目前没有任何定时器基础设施,本计划只为这一个用例引入轮询,不建通用 scheduler。
- 只对 `AgentKind::Claude`/`Codebuddy`/`Opencode`/`V8agent` 四家 且 `TabBackend::Daemon` 的存活 tab 触发总结流程;其余(纯 shell/`Unknown`/SSH/`Codex`/`Kilo`)维持现状直接 `Kill`,不产出总结记录。
- 总结内容两级:`title`(≤60 字符,超长截断加 `…`)+ `summary`(完整文本,`dozer-mcp` 侧上限 8000 字符截断)。
- 超时 60 秒、轮询间隔 2 秒,写死在调用点常量,但底层函数必须接受这两个值作为参数(供测试注入短间隔,不能让测试真等 60 秒)。
- `RecordSessionSummary` 主键是 `session_id`,`INSERT ... ON CONFLICT DO UPDATE`,后到覆盖先到。
- 每个改动完成后运行:`cargo build`(全 workspace)、对应 crate 的 `cargo test`、`cargo clippy --all-targets -- -D warnings`、`cargo fmt -- --check`。

---

## ⚠️ 修正(2026-08-27,写姊妹项目 B 时发现,Task 1-3 已实现完毕后补的):`session_id` ≠ `conversation_id`

`dozerd::registry` 的 `session_id`(`Session::spawn` 里 `uuid::Uuid::new_v4()` 生成)和
`TranscriptStore` 的 `conversation_id`(取 transcript 文件名的 `file_stem()`,比如 Claude
自己那份 `.jsonl` 的文件名,是 Claude 自己分配的 id)是**两套完全独立生成、互不相关的
UUID**。唯一的桥是 `Session::info().transcript_path`(hook 上报时写入,只在内存
registry 里,不落库)。

**这意味着 Task 8(还没开始实现)里 `finalize_session_summary` 的启发式兜底分支,如果
直接拿 `session_id` 当 `conversation_id` 去查 `transcripts.get_conversation_turns`,
在生产环境里几乎总是查不到数据**(两个 id 根本不相等,只会命中"空结果"分支,一直落到
"(无对话记录)"占位文案)。同理,这也是姊妹项目 B(对话面板导航改版,把
`session_summaries` join 进 `ListConversations` 的结果)的阻塞项——现在
`session_summaries` 表里没有 `conversation_id` 列,没法关联。

Task 1(`SessionSummaryPayload`)和 Task 2(`SessionSummaryStore` 表结构)已经实现完
且不含这个字段/列——**下面是需要在原有实现基础上追加的最小修正,不是推倒重做**:

1. **`crates/dozer-core/src/protocol.rs`**:`SessionSummaryPayload` struct 追加一个
   字段(紧跟 `agent_kind` 之后):
   ```rust
   /// 该会话对应的 transcript conversation_id(取自 `transcript_path` 的
   /// `file_stem()`)。`None` 表示这个会话直到总结产出时都没收到过任何 hook
   /// 事件(纯 shell/agent 没配好 hook),启发式兜底也查不到任何数据。
   pub conversation_id: Option<String>,
   ```
   补一个序列化往返测试(仿 Task 1 已有的 `get_session_summary_reply_roundtrips_with_payload`,
   造一个 `conversation_id: Some("c1".into())` 和一个 `None` 的两个用例)。

2. **`crates/dozerd/src/transcripts/mod.rs`**:把 `ingest_session` 内联的
   `file_path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string()` 这段抽成一个
   `pub` 函数,供 `session_summary.rs`/`server.rs` 复用(避免同一段派生逻辑写两份):
   ```rust
   /// 从 transcript 文件路径派生 `conversation_id`(即文件名去掉扩展名)。
   /// `ingest_session` 与 `dozerd::server` 的 `RecordSessionSummary`/
   /// `CloseWithSummary` 处理器共用同一份派生逻辑。
   pub fn conversation_id_for_path(file_path: &Path) -> Option<String> {
       file_path.file_stem().and_then(|s| s.to_str()).map(String::from)
   }
   ```
   `ingest_session` 内部改用这个函数(`unwrap_or("").to_string()` 换成
   `conversation_id_for_path(file_path).ok_or_else(|| ...)?`,空路径时的报错分支保持原有
   行为不变)。补一个新测试确认非法/无扩展名路径的行为,以及既有 `ingest_session` 相关
   测试全部保持通过(纯重构,不改行为)。

3. **`crates/dozerd/src/session_summary.rs`**:表结构追加 `conversation_id TEXT` 可空列
   (仿 `crates/dozerd/src/transcripts/mod.rs` 里 `conversation_turns` 加 `is_error` 列的
   既有幂等迁移写法——`SELECT 1 FROM pragma_table_info(...)` 判断列是否已存在,不存在才
   `ALTER TABLE ... ADD COLUMN`,因为已经有人跑过 `CREATE TABLE IF NOT EXISTS` 建过旧表结构
   的库需要能平滑升级)。`record`/`get` 两个方法的 SQL 和 Rust 结构体映射都要带上这一列
   (`get` 里 `conversation_id: row.get::<_, Option<String>>(6)?`)。补测试:写入带
   `Some(id)` 的记录读回一致;写入 `None` 的记录读回也是 `None`。

4. **`crates/dozerd/src/server.rs` 的 `RecordSessionSummary` handler(Task 3 已实现,
   需要打补丁)**:构造 `SessionSummaryPayload` 时,`conversation_id` 字段用
   `s.info().transcript_path.as_deref().and_then(|p| crate::transcripts::conversation_id_for_path(Path::new(p)))`
   算出来(`s` 就是 `registry.get(&session_id)` 已经拿到的那个 `Arc<Session>`,不需要
   额外查询)。

5. **Task 8 的 `finalize_session_summary` 落地时,直接按下面的签名写,不要用
   `session_id` 查 `conversation_turns`**:

   ```rust
   async fn finalize_session_summary(
       session_id: String,
       conversation_id: Option<String>,
       registry: Arc<SessionRegistry>,
       session_summaries: Arc<crate::session_summary::SessionSummaryStore>,
       transcripts: Arc<crate::transcripts::TranscriptStore>,
       agent: dozer_core::protocol::AgentKind,
       timeout: std::time::Duration,
       poll_interval: std::time::Duration,
   ) {
       // ... 轮询逻辑不变 ...
       // 超时分支:
       let turns = match &conversation_id {
           Some(cid) => transcripts.get_conversation_turns(cid, -1, u32::MAX).unwrap_or_default(),
           None => Vec::new(),
       };
       let (title, summary) = crate::session_summary::heuristic_from_turns(&turns);
       let payload = dozer_core::protocol::SessionSummaryPayload {
           session_id: session_id.clone(),
           agent_kind: agent,
           conversation_id: conversation_id.clone(),
           title,
           summary,
           status: dozer_core::protocol::SummaryStatus::HeuristicFallback,
           created_ts_ms: /* 同原设计 */,
       };
       // ... 落库 + kill 不变 ...
   }
   ```

   `Request::CloseWithSummary` 处理器里调用它时,在拿到 `s = registry.get(&session_id)`
   之后立刻算好 `conversation_id`(同第 4 点的算法),连同 `agent` 一起传进去,不要在
   `finalize_session_summary` 内部重新查 registry(那时 session 可能已经被后台任务自己
   kill 掉,`registry.get` 依然能查到已死会话——但没必要多这一次查询,拿到 `s` 时早取
   走更简单)。

   **Task 8 原稿里 `finalize_session_summary_falls_back_to_heuristic_on_timeout` 这条
   测试也要跟着改**:调用时传入 `conversation_id: Some(id.clone())`(fixture 里文件名
   本来就是 `{id}.jsonl`,现在通过显式参数传,而不是依赖"session_id 恰好等于
   conversation_id"这个巧合),另补一条 `conversation_id: None` 时兜底直接落"(无对话
   记录)"占位文案、不报错的测试。

**这条修正需要在到达 Task 8 之前落地(Task 4-7 不受影响,可以继续按原计划推进)**,
因为 Task 8 一旦照原稿写完再改,返工成本更高。已经实现的 Task 1-3 只需要打上面 1/3/4
三处小补丁,不需要重写。

---

## Task 1: 协议层新增类型与 Request/Reply(不含 `CloseWithSummary`)

**Files:**
- Modify: `crates/dozer-core/src/protocol.rs`

**Interfaces:**
- Produces:`SummaryStatus`(`AiGenerated`/`HeuristicFallback`)、`SessionSummaryPayload { session_id: String, agent_kind: AgentKind, title: String, summary: String, status: SummaryStatus, created_ts_ms: u64 }`、`Request::RecordSessionSummary { session_id, title, summary }`、`Request::GetSessionSummary { session_id }`、`Reply::SessionSummary { summary: Option<SessionSummaryPayload> }`。后续所有任务都从这里引用这几个类型。

- [ ] **Step 1: 写失败的序列化往返测试**

在 `protocol.rs` 文件末尾已有的 `#[cfg(test)] mod tests` 块里追加(若该模块不存在,在文件末尾新建 `#[cfg(test)] mod tests { use super::*; ... }`,参照文件里已有的 `agent_kind_serializes_snake_case` 一类测试的写法):

```rust
    #[test]
    fn summary_status_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&SummaryStatus::AiGenerated).unwrap(),
            "\"ai_generated\""
        );
        assert_eq!(
            serde_json::to_string(&SummaryStatus::HeuristicFallback).unwrap(),
            "\"heuristic_fallback\""
        );
    }

    #[test]
    fn record_session_summary_request_roundtrips() {
        let req = Request::RecordSessionSummary {
            session_id: "s1".into(),
            title: "改了个函数".into(),
            summary: "用户让改 README,agent 改完了".into(),
        };
        let line = encode_line(&req);
        let back: Request = decode_line(&line).unwrap();
        assert_eq!(req, back);
    }

    #[test]
    fn get_session_summary_reply_roundtrips_with_payload() {
        let reply = Reply::SessionSummary {
            summary: Some(SessionSummaryPayload {
                session_id: "s1".into(),
                agent_kind: AgentKind::Claude,
                title: "t".into(),
                summary: "s".into(),
                status: SummaryStatus::AiGenerated,
                created_ts_ms: 42,
            }),
        };
        let line = encode_line(&reply);
        let back: Reply = decode_line(&line).unwrap();
        assert_eq!(reply, back);
    }

    #[test]
    fn get_session_summary_reply_roundtrips_with_none() {
        let reply = Reply::SessionSummary { summary: None };
        let line = encode_line(&reply);
        let back: Reply = decode_line(&line).unwrap();
        assert_eq!(reply, back);
    }
```

- [ ] **Step 2: 运行测试确认失败(类型不存在,编译错误)**

Run: `cargo test -p dozer-core summary`
Expected: 编译失败,报 `SummaryStatus`/`SessionSummaryPayload`/`RecordSessionSummary`/`GetSessionSummary`/`SessionSummary` 未定义。

- [ ] **Step 3: 加类型与枚举 variant**

在 `AgentKind` 定义(约第 18-29 行)之后、`ConversationSummary` 定义之前插入:

```rust
/// 一次会话总结的产出状态:agent 真的经 dozer-mcp 交回,还是超时后由
/// dozerd 从已摄取对话数据算的启发式兜底(spec 2026-08-27)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SummaryStatus {
    AiGenerated,
    HeuristicFallback,
}

/// 一份持久化的会话总结(`dozerd` 的 `session_summaries` 表一行)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionSummaryPayload {
    pub session_id: String,
    pub agent_kind: AgentKind,
    pub title: String,
    pub summary: String,
    pub status: SummaryStatus,
    pub created_ts_ms: u64,
}
```

在 `pub enum Request` 的 `GetPreviewContext { project_id: i64 }` variant(第 371-374 行)之后、枚举结束的 `}` 之前插入:

```rust
    /// `dozer-mcp` 的写工具提交一份会话总结;`dozerd` 只做"session_id 是否
    /// 存在于 registry"的存在性检查,不做权限校验(与 `Write`/`HookEvent`
    /// 同等信任本机调用方)。主键 `session_id`,重复提交后到覆盖先到。
    RecordSessionSummary {
        session_id: String,
        title: String,
        summary: String,
    },
    /// 查询某会话是否已有总结(`None` 表示尚未生成或本会话不适用)。
    GetSessionSummary {
        session_id: String,
    },
```

在 `pub enum Reply` 的 `PreviewContext { context: Option<PreviewContext> }` variant(第 465-469 行)之后、枚举结束的 `}` 之前插入:

```rust
    /// `GetSessionSummary` 应答。
    SessionSummary {
        summary: Option<SessionSummaryPayload>,
    },
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozer-core summary`
Expected: 全部 PASS(4 个新测试)。

- [ ] **Step 5: 全 crate 测试 + clippy + fmt**

Run: `cargo test -p dozer-core && cargo clippy -p dozer-core --all-targets -- -D warnings && cargo fmt -- --check`
Expected: 全绿(改动只是新增 variant,不影响任何既有 match——`Request`/`Reply` 是 `#[non_exhaustive]` 之外的普通枚举,但目前唯一穷尽 match 它们的地方是 `dozerd::server::handle_conn`,还没加新分支,会在 Task 3 报"未穷尽 match"编译错误,属预期,不在本任务修——本任务只改 `dozer-core`,不牵连 `dozerd`)。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-core/src/protocol.rs
git commit -m "feat(dozer-core): add session summary protocol types"
```

---

## Task 2: `SessionSummaryStore`(dozerd 新表)

**Files:**
- Create: `crates/dozerd/src/session_summary.rs`
- Modify: `crates/dozerd/src/lib.rs`

**Interfaces:**
- Consumes: `dozer_core::protocol::{AgentKind, SessionSummaryPayload, SummaryStatus}`(Task 1 产出)。
- Produces: `pub struct SessionSummaryStore`、`SessionSummaryStore::open(path: &Path) -> Result<Self>`、`SessionSummaryStore::record(&self, payload: &SessionSummaryPayload) -> Result<()>`、`SessionSummaryStore::get(&self, session_id: &str) -> Result<Option<SessionSummaryPayload>>`。Task 3 直接用这三个方法。

- [ ] **Step 1: 写失败的测试**

创建 `crates/dozerd/src/session_summary.rs`:

```rust
//! 会话总结存储:rusqlite 单表,主键 session_id(spec 2026-08-27)。
//! Mutex<Connection>——同 AcceptanceStore,写入频度低,无需连接池。

use anyhow::{Context, Result};
use dozer_core::protocol::{AgentKind, SessionSummaryPayload, SummaryStatus};
use rusqlite::Connection;
use std::path::Path;
use std::sync::Mutex;

fn agent_to_str(a: AgentKind) -> &'static str {
    a.label()
}

fn agent_from_str(s: &str) -> AgentKind {
    match s {
        "claude" => AgentKind::Claude,
        "codebuddy" => AgentKind::Codebuddy,
        "opencode" => AgentKind::Opencode,
        "codex" => AgentKind::Codex,
        "kilo" => AgentKind::Kilo,
        "v8agent" => AgentKind::V8agent,
        _ => AgentKind::Unknown,
    }
}

fn status_to_str(s: SummaryStatus) -> &'static str {
    match s {
        SummaryStatus::AiGenerated => "ai_generated",
        SummaryStatus::HeuristicFallback => "heuristic_fallback",
    }
}

fn status_from_str(s: &str) -> SummaryStatus {
    match s {
        "ai_generated" => SummaryStatus::AiGenerated,
        _ => SummaryStatus::HeuristicFallback,
    }
}

pub struct SessionSummaryStore {
    conn: Mutex<Connection>,
}

impl SessionSummaryStore {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("建库目录")?;
        }
        let conn = Connection::open(path).context("打开 SQLite")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS session_summaries (
                session_id TEXT PRIMARY KEY,
                agent_kind TEXT NOT NULL,
                title TEXT NOT NULL,
                summary TEXT NOT NULL,
                status TEXT NOT NULL,
                created_ts_ms INTEGER NOT NULL
            );",
        )
        .context("建表")?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn record(&self, payload: &SessionSummaryPayload) -> Result<()> {
        self.conn.lock().expect("db lock").execute(
            "INSERT INTO session_summaries
             (session_id, agent_kind, title, summary, status, created_ts_ms)
             VALUES (?1,?2,?3,?4,?5,?6)
             ON CONFLICT(session_id) DO UPDATE SET
                agent_kind = excluded.agent_kind,
                title = excluded.title,
                summary = excluded.summary,
                status = excluded.status,
                created_ts_ms = excluded.created_ts_ms",
            rusqlite::params![
                payload.session_id,
                agent_to_str(payload.agent_kind),
                payload.title,
                payload.summary,
                status_to_str(payload.status),
                payload.created_ts_ms,
            ],
        )?;
        Ok(())
    }

    pub fn get(&self, session_id: &str) -> Result<Option<SessionSummaryPayload>> {
        let conn = self.conn.lock().expect("db lock");
        let mut stmt = conn.prepare(
            "SELECT session_id, agent_kind, title, summary, status, created_ts_ms
             FROM session_summaries WHERE session_id = ?1",
        )?;
        let mut rows = stmt.query(rusqlite::params![session_id])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        let agent_kind: String = row.get(1)?;
        let status: String = row.get(4)?;
        let created_ts_ms: i64 = row.get(5)?;
        Ok(Some(SessionSummaryPayload {
            session_id: row.get(0)?,
            agent_kind: agent_from_str(&agent_kind),
            title: row.get(2)?,
            summary: row.get(3)?,
            status: status_from_str(&status),
            created_ts_ms: created_ts_ms as u64,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(session_id: &str) -> SessionSummaryPayload {
        SessionSummaryPayload {
            session_id: session_id.into(),
            agent_kind: AgentKind::Claude,
            title: "改了个函数".into(),
            summary: "详细过程".into(),
            status: SummaryStatus::AiGenerated,
            created_ts_ms: 1_700_000_000_000,
        }
    }

    #[test]
    fn open_record_get_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionSummaryStore::open(&dir.path().join("t.db")).unwrap();
        assert!(store.get("s1").unwrap().is_none());
        store.record(&payload("s1")).unwrap();
        let got = store.get("s1").unwrap().unwrap();
        assert_eq!(got, payload("s1"));
    }

    #[test]
    fn record_twice_overwrites_not_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionSummaryStore::open(&dir.path().join("t.db")).unwrap();
        store.record(&payload("s1")).unwrap();
        let mut second = payload("s1");
        second.title = "第二次总结".into();
        second.status = SummaryStatus::HeuristicFallback;
        store.record(&second).unwrap();
        let got = store.get("s1").unwrap().unwrap();
        assert_eq!(got.title, "第二次总结");
        assert_eq!(got.status, SummaryStatus::HeuristicFallback);
    }

    #[test]
    fn persists_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        {
            let store = SessionSummaryStore::open(&path).unwrap();
            store.record(&payload("s1")).unwrap();
        }
        let store = SessionSummaryStore::open(&path).unwrap();
        assert!(store.get("s1").unwrap().is_some());
    }
}
```

- [ ] **Step 2: 注册模块并运行测试确认能跑**

在 `crates/dozerd/src/lib.rs` 按字母序插入(在 `pub mod session;` 之前):

```rust
pub mod session_summary;
```

Run: `cargo test -p dozerd session_summary`
Expected: 3 个测试全 PASS(这一步是"实现完直接测",因为整份实现是照抄 `acceptance.rs` 的成熟模式,不必先写空壳看失败——`acceptance.rs` 本身没有走"先失败"这一步的历史先例,此文件同理)。

- [ ] **Step 3: clippy + fmt**

Run: `cargo clippy -p dozerd --all-targets -- -D warnings && cargo fmt -- --check`
Expected: 全绿。

- [ ] **Step 4: Commit**

```bash
git add crates/dozerd/src/session_summary.rs crates/dozerd/src/lib.rs
git commit -m "feat(dozerd): add SessionSummaryStore"
```

---

## Task 3: 把 `SessionSummaryStore` 接进 `dozerd::server`/`main`,实现 Record/Get 两个 handler

**Files:**
- Modify: `crates/dozerd/src/main.rs`
- Modify: `crates/dozerd/src/server.rs`
- Modify: `crates/dozerd/tests/session_survival.rs`
- Modify: `crates/dozerd/tests/hook_events.rs`
- Modify: `crates/dozer-mcp/tests/get_preview_context.rs`
- Modify: `crates/dozer-client/tests/against_real_daemon.rs`

**Interfaces:**
- Consumes: `SessionSummaryStore::open/record/get`(Task 2),`Request::RecordSessionSummary`/`GetSessionSummary`/`Reply::SessionSummary`(Task 1)。
- Produces: `server::serve`/`server::handle_conn` 新签名(末尾追加 `session_summaries: Arc<crate::session_summary::SessionSummaryStore>` 参数)。**这是一个破坏性签名变更,所有调用点必须在本任务内一次性改完**,后续任务(4/6/8)会在这些文件里继续追加新测试,但不会再改这个签名。

- [ ] **Step 1: 改 `server.rs` 签名 + 两个 handler**

`crates/dozerd/src/server.rs` 第 14-21 行的 `pub async fn serve` 签名追加参数:

```rust
pub async fn serve(
    socket: &Path,
    registry: Arc<SessionRegistry>,
    store: Arc<crate::acceptance::AcceptanceStore>,
    projects: Arc<crate::projects::ProjectStore>,
    bookmarks: Arc<crate::bookmarks::BookmarkStore>,
    transcripts: Arc<crate::transcripts::TranscriptStore>,
    session_summaries: Arc<crate::session_summary::SessionSummaryStore>,
) -> Result<()> {
```

循环内 `let transcripts = transcripts.clone();` 之后追加:

```rust
        let session_summaries = session_summaries.clone();
```

`tokio::spawn` 里的 `handle_conn(...)` 调用追加参数:

```rust
            if let Err(e) = handle_conn(
                stream,
                registry,
                store,
                projects,
                bookmarks,
                preview_contexts,
                transcripts,
                session_summaries,
            )
```

`async fn handle_conn` 签名同样追加参数(紧跟 `transcripts: Arc<crate::transcripts::TranscriptStore>,` 之后):

```rust
    session_summaries: Arc<crate::session_summary::SessionSummaryStore>,
```

在 `Request::GetUsageSummary { .. } => { ... }` 分支(第 355-360 行)之后、`match` 结束的 `}` 之前追加两个新分支:

```rust
                        Request::RecordSessionSummary { session_id, title, summary } => {
                            match registry.get(&session_id) {
                                None => Reply::Error { message: format!("会话不存在: {session_id}") },
                                Some(s) => {
                                    let payload = dozer_core::protocol::SessionSummaryPayload {
                                        session_id: session_id.clone(),
                                        agent_kind: s.info().agent,
                                        title,
                                        summary,
                                        status: dozer_core::protocol::SummaryStatus::AiGenerated,
                                        created_ts_ms: std::time::SystemTime::now()
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .map(|d| d.as_millis() as u64)
                                            .unwrap_or(0),
                                    };
                                    match session_summaries.record(&payload) {
                                        Ok(()) => Reply::Ok,
                                        Err(e) => Reply::Error {
                                            message: format!("会话总结落库失败: {e}"),
                                        },
                                    }
                                }
                            }
                        }
                        Request::GetSessionSummary { session_id } => {
                            match session_summaries.get(&session_id) {
                                Ok(summary) => Reply::SessionSummary { summary },
                                Err(e) => Reply::Error {
                                    message: format!("查询会话总结失败: {e}"),
                                },
                            }
                        }
```

- [ ] **Step 2: 改 `main.rs` 打开新库并传参**

`crates/dozerd/src/main.rs` 在 `let transcripts = ...` 块(第 47-49 行)之后追加:

```rust
    let session_summaries = Arc::new(dozerd::session_summary::SessionSummaryStore::open(
        &dozer_core::paths::state_dir().join("dozer.db"),
    )?);
```

`let serve = dozerd::server::serve(...)`(第 55 行)追加参数:

```rust
    let serve = dozerd::server::serve(
        &socket, registry, store, projects, bookmarks, transcripts, session_summaries,
    );
```

- [ ] **Step 3: 更新所有其余调用点(逐一改,不能漏)**

先枚举确认范围:

Run: `grep -rn "server::serve(" --include="*.rs" .`
Expected: 命中 `crates/dozerd/src/server.rs`(定义处,已在 Step 1 改)、`crates/dozerd/src/main.rs`(已在 Step 2 改)、`crates/dozerd/tests/session_survival.rs`(7 处调用)、`crates/dozerd/tests/hook_events.rs`(3 处调用)、`crates/dozer-mcp/tests/get_preview_context.rs`(1 处)、`crates/dozer-client/tests/against_real_daemon.rs`(1 处)。

对 `crates/dozerd/tests/session_survival.rs`:在文件末尾(紧跟 `test_transcripts` 函数,约第 588-591 行)追加一个新 helper:

```rust
/// 每次调用建独立临时库的会话总结存储(测试用；serve 需要)。
fn test_session_summaries() -> std::sync::Arc<dozerd::session_summary::SessionSummaryStore> {
    let db = std::env::temp_dir().join(format!("dozerd-test-{}.db", uuid::Uuid::new_v4()));
    std::sync::Arc::new(dozerd::session_summary::SessionSummaryStore::open(&db).unwrap())
}
```

然后对文件里全部 7 处 `dozerd::server::serve(&sock, registry, ...)` 调用(第 61/191/231/307/412/504 行附近,每处结尾都是 `test_transcripts(),` 后紧跟 `)`),在 `test_transcripts(),` 之后追加一行 `test_session_summaries(),`。用这条命令批量确认改完(每次改完手动跑一次,不要用 sed 盲改——不同调用点前面的参数列表格式略有差异,逐处手动加更保险):

Run(改完后): `grep -c "test_session_summaries()" crates/dozerd/tests/session_survival.rs`
Expected: `7`(与 `serve(` 调用次数一致)。

对 `crates/dozerd/tests/hook_events.rs`:同样在文件末尾 `test_transcripts` 函数(约第 219-222 行)之后追加同一个 `test_session_summaries()` helper(定义与上面完全一致),然后对文件里全部 3 处 `serve(` 调用,在 `transcripts.clone(),`/`transcripts` 参数之后追加 `test_session_summaries(),`/`session_summaries`(注意:该文件里第 34 行那处调用是展开写的多行参数列表,第 177/243 行是复用一个 `serve_fut` 闭包变量——具体改法照抄 Step 1 里 `server.rs` 定义处新增参数的位置,即紧跟 `transcripts` 之后)。

Run(改完后): `grep -c "session_summaries" crates/dozerd/tests/hook_events.rs`
Expected: `4`(1 处 helper 定义 + 3 处调用点引用)。

对 `crates/dozer-mcp/tests/get_preview_context.rs`:第 27 行 `let transcripts = ...` 之后追加:

```rust
    let session_summaries = Arc::new(dozerd::session_summary::SessionSummaryStore::open(&db).unwrap());
```

第 30 行 `dozerd::server::serve(&s, registry, store, projects, bookmarks, transcripts).await` 改成:

```rust
        dozerd::server::serve(&s, registry, store, projects, bookmarks, transcripts, session_summaries).await
```

对 `crates/dozer-client/tests/against_real_daemon.rs`:第 22 行 `let transcripts = ...` 之后追加:

```rust
    let session_summaries = Arc::new(dozerd::session_summary::SessionSummaryStore::open(&db).unwrap());
```

第 24 行 `dozerd::server::serve(&s, r, store, projects, bookmarks, transcripts).await` 改成:

```rust
        dozerd::server::serve(&s, r, store, projects, bookmarks, transcripts, session_summaries).await
```

- [ ] **Step 4: 编译确认没有漏改的调用点**

Run: `cargo build --workspace --tests`
Expected: 编译成功。若报某处 `serve()` 参数数量不对,回到 Step 3 补上遗漏的那一处。

- [ ] **Step 5: 写一个 Record/Get 往返的集成测试确认新 handler 真的接线正确**

在 `crates/dozerd/tests/hook_events.rs` 末尾追加:

```rust
#[tokio::test]
async fn record_and_get_session_summary_roundtrip() {
    let sock = std::env::temp_dir().join(format!("dozerd-summary-{}.sock", uuid::Uuid::new_v4()));
    let registry = Arc::new(SessionRegistry::new());
    let store = test_store();
    let projects = test_projects();
    let bookmarks = test_bookmarks();
    let transcripts = test_transcripts();
    let session_summaries = test_session_summaries();
    tokio::spawn({
        let sock = sock.clone();
        let registry = registry.clone();
        async move {
            dozerd::server::serve(
                &sock, registry, store, projects, bookmarks, transcripts, session_summaries,
            )
            .await
        }
    });
    for _ in 0..100 {
        if sock.exists() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    let s = registry
        .create(dozerd::session::SessionSpec {
            name: "t".into(),
            command: "/bin/sh".into(),
            args: vec!["-c".into(), "sleep 5".into()],
            cwd: std::env::temp_dir().to_string_lossy().into_owned(),
            cols: 80,
            rows: 24,
            project_id: 1,
        })
        .unwrap();
    let id = s.id().to_string();

    let mut c = dozer_client_test_helper::connect(&sock).await;
    c.send(&dozer_core::protocol::Request::GetSessionSummary { session_id: id.clone() })
        .await;
    assert_eq!(
        c.recv().await,
        dozer_core::protocol::Reply::SessionSummary { summary: None }
    );

    c.send(&dozer_core::protocol::Request::RecordSessionSummary {
        session_id: id.clone(),
        title: "标题".into(),
        summary: "摘要".into(),
    })
    .await;
    assert_eq!(c.recv().await, dozer_core::protocol::Reply::Ok);

    c.send(&dozer_core::protocol::Request::GetSessionSummary { session_id: id.clone() }).await;
    match c.recv().await {
        dozer_core::protocol::Reply::SessionSummary { summary: Some(p) } => {
            assert_eq!(p.title, "标题");
            assert_eq!(p.summary, "摘要");
            assert_eq!(p.status, dozer_core::protocol::SummaryStatus::AiGenerated);
        }
        other => panic!("意外应答: {other:?}"),
    }
    let _ = s.kill();
}
```

`hook_events.rs` 文件顶部目前没有一个可复用的裸 UDS `Client` 收发 helper(那是 `session_survival.rs` 独有的 `struct Client`)——检查 `hook_events.rs` 现有测试是怎么发请求收应答的(读文件前 30 行),照抄同一种写法替换掉上面示例里的 `dozer_client_test_helper::connect`(那只是示意,不是真实可编译的路径,必须替换成该文件实际已有的收发方式,比如直接用 `dozer_client::Client::new(sock)` 若该文件已引入这个 crate,或复用 `session_survival.rs` 里那个 `struct Client` 的写法在本文件顶部补一份同款 struct)。

- [ ] **Step 6: 运行测试确认通过**

Run: `cargo test -p dozerd --test hook_events record_and_get_session_summary_roundtrip`
Expected: PASS。

- [ ] **Step 7: 全量回归 + clippy + fmt**

Run: `cargo build --workspace && cargo test -p dozerd -p dozer-mcp -p dozer-client && cargo clippy --all-targets -- -D warnings && cargo fmt -- --check`
Expected: 全绿(尤其确认 Task 1 遗留的"未穷尽 match"编译错误在这一步已经消失)。

- [ ] **Step 8: Commit**

```bash
git add crates/dozerd/src/main.rs crates/dozerd/src/server.rs \
  crates/dozerd/tests/session_survival.rs crates/dozerd/tests/hook_events.rs \
  crates/dozer-mcp/tests/get_preview_context.rs crates/dozer-client/tests/against_real_daemon.rs
git commit -m "feat(dozerd): wire SessionSummaryStore into server, add Record/Get handlers"
```

---

## Task 4: `dozer-client` 的 `record_session_summary`/`get_session_summary` 方法

**Files:**
- Modify: `crates/dozer-client/src/lib.rs`
- Modify: `crates/dozer-client/tests/against_real_daemon.rs`

**Interfaces:**
- Consumes: `Request::RecordSessionSummary`/`GetSessionSummary`/`Reply::SessionSummary`(Task 1)。
- Produces: `Client::record_session_summary(&self, id: &str, title: &str, summary: &str) -> Result<()>`、`Client::get_session_summary(&self, id: &str) -> Result<Option<SessionSummaryPayload>>`。Task 9(dozer-app)不直接用这两个(用的是 Task 8 的 `close_with_summary`),但对话面板改版那份独立 spec 会用 `get_session_summary`。

- [ ] **Step 1: 写失败的集成测试**

在 `crates/dozer-client/tests/against_real_daemon.rs` 末尾追加(需要先看一眼文件里已有测试怎么起 daemon/建 session,照抄同款 `start_daemon`/`client.create` 写法):

```rust
#[tokio::test]
async fn record_and_get_session_summary_via_client() {
    let (sock, _guard) = start_daemon().await;
    let client = Client::new(sock);
    let session = client
        .create("测试", "/bin/sh", &["-c".into(), "sleep 5".into()], "/tmp", 80, 24, 1)
        .await
        .unwrap();

    assert_eq!(client.get_session_summary(&session.id).await.unwrap(), None);

    client
        .record_session_summary(&session.id, "标题", "摘要内容")
        .await
        .unwrap();

    let got = client.get_session_summary(&session.id).await.unwrap().unwrap();
    assert_eq!(got.title, "标题");
    assert_eq!(got.summary, "摘要内容");
    assert_eq!(got.status, dozer_core::protocol::SummaryStatus::AiGenerated);

    client.kill(&session.id).await.unwrap();
}
```

(若该文件里 `start_daemon()` 这个 helper 名字不存在或叫别的名字,以文件实际内容为准替换调用,不要臆造。)

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p dozer-client record_and_get_session_summary_via_client`
Expected: 编译失败,`Client` 没有 `record_session_summary`/`get_session_summary` 方法。

- [ ] **Step 3: 实现两个方法**

在 `crates/dozer-client/src/lib.rs` 的 `get_preview_context` 方法(第 367-375 行)之后追加:

```rust
    pub async fn record_session_summary(&self, id: &str, title: &str, summary: &str) -> Result<()> {
        match self
            .roundtrip(&Request::RecordSessionSummary {
                session_id: id.into(),
                title: title.into(),
                summary: summary.into(),
            })
            .await?
        {
            Reply::Ok => Ok(()),
            other => bail!("意外应答: {other:?}"),
        }
    }

    pub async fn get_session_summary(&self, id: &str) -> Result<Option<SessionSummaryPayload>> {
        match self
            .roundtrip(&Request::GetSessionSummary { session_id: id.into() })
            .await?
        {
            Reply::SessionSummary { summary } => Ok(summary),
            other => bail!("意外应答: {other:?}"),
        }
    }
```

文件顶部 `use dozer_core::protocol::{...}` 导入列表(第 4-8 行)按字母序加入 `SessionSummaryPayload`。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozer-client record_and_get_session_summary_via_client`
Expected: PASS。

- [ ] **Step 5: 全量 + clippy + fmt**

Run: `cargo test -p dozer-client && cargo clippy -p dozer-client --all-targets -- -D warnings && cargo fmt -- --check`
Expected: 全绿。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-client/src/lib.rs crates/dozer-client/tests/against_real_daemon.rs
git commit -m "feat(dozer-client): add record_session_summary/get_session_summary"
```

---

## Task 5: 启发式兜底算法(纯函数)

**Files:**
- Modify: `crates/dozerd/src/session_summary.rs`

**Interfaces:**
- Consumes: `dozer_core::protocol::TurnRecord`(已存在)。
- Produces: `pub fn heuristic_from_turns(turns: &[TurnRecord]) -> (String, String)`。Task 8 的 `finalize_session_summary` 直接调用。

- [ ] **Step 1: 写失败的测试**

在 `crates/dozerd/src/session_summary.rs` 的 `#[cfg(test)] mod tests` 块内追加:

```rust
    fn turn(role: &str, content: &str) -> dozer_core::protocol::TurnRecord {
        dozer_core::protocol::TurnRecord {
            role: role.into(),
            content: content.into(),
            ..Default::default()
        }
    }

    #[test]
    fn heuristic_uses_first_human_turn_as_title() {
        let turns = vec![
            turn("human", "帮我改一下 README"),
            turn("ai", "好的,我来改"),
            turn("human", "再加一段安装说明"),
        ];
        let (title, summary) = heuristic_from_turns(&turns);
        assert_eq!(title, "帮我改一下 README");
        assert_eq!(summary, "帮我改一下 README\n再加一段安装说明");
    }

    #[test]
    fn heuristic_ignores_non_human_turns() {
        let turns = vec![turn("ai", "纯 AI 输出"), turn("tool", "工具结果")];
        let (title, summary) = heuristic_from_turns(&turns);
        assert_eq!(title, "(无对话记录)");
        assert_eq!(summary, "(无对话记录)");
    }

    #[test]
    fn heuristic_truncates_long_title_at_60_chars() {
        let long = "a".repeat(100);
        let turns = vec![turn("human", &long)];
        let (title, _) = heuristic_from_turns(&turns);
        assert_eq!(title.chars().count(), 61); // 60 + "…"
        assert!(title.ends_with('…'));
    }

    #[test]
    fn heuristic_empty_turns_returns_placeholder() {
        let (title, summary) = heuristic_from_turns(&[]);
        assert_eq!(title, "(无对话记录)");
        assert_eq!(summary, "(无对话记录)");
    }
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p dozerd heuristic`
Expected: 编译失败,`heuristic_from_turns` 未定义。

- [ ] **Step 3: 实现**

在 `crates/dozerd/src/session_summary.rs` 顶部 `use` 之后、`pub struct SessionSummaryStore` 之前追加:

```rust
const HEURISTIC_PLACEHOLDER: &str = "(无对话记录)";
const HEURISTIC_TITLE_MAX_CHARS: usize = 60;

/// 没等到 agent 交回总结时的兜底算法:用已摄取的对话数据(仅取人类回合,
/// 与 `truncate_activity` 的 60 字符惯例保持一致)拼一份"没有语义压缩"的
/// 降级总结。空结果返回固定占位文案而不是空字符串,保证消费方不用处理
/// "有的 session 干脆没有总结"这种特例(spec 2026-08-27 D5)。
pub fn heuristic_from_turns(turns: &[dozer_core::protocol::TurnRecord]) -> (String, String) {
    let human: Vec<&str> = turns
        .iter()
        .filter(|t| t.role == "human")
        .map(|t| t.content.as_str())
        .collect();
    if human.is_empty() {
        return (HEURISTIC_PLACEHOLDER.into(), HEURISTIC_PLACEHOLDER.into());
    }
    (truncate_chars(human[0], HEURISTIC_TITLE_MAX_CHARS), human.join("\n"))
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let head: String = s.chars().take(max).collect();
        format!("{head}…")
    }
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozerd heuristic`
Expected: 4 个测试全 PASS。

- [ ] **Step 5: clippy + fmt**

Run: `cargo clippy -p dozerd --all-targets -- -D warnings && cargo fmt -- --check`
Expected: 全绿。

- [ ] **Step 6: Commit**

```bash
git add crates/dozerd/src/session_summary.rs
git commit -m "feat(dozerd): add heuristic fallback summary algorithm"
```

---

## Task 6: `dozer-mcp` 新写工具 `submit_session_summary`

**Files:**
- Modify: `crates/dozer-mcp/src/server.rs`
- Create: `crates/dozer-mcp/tests/submit_session_summary.rs`

**Interfaces:**
- Consumes: `Client::record_session_summary`(Task 4)。
- Produces: `DozerMcpServer::submit_session_summary` 工具方法(agent 侧调用名 `submit_session_summary`,入参 `title: String, summary: String`)。

- [ ] **Step 1: 写失败的测试**

创建 `crates/dozer-mcp/tests/submit_session_summary.rs`(`start_daemon`/`temp_sock`/`CleanupGuard` 三个 helper 照抄 `tests/get_preview_context.rs` 里的写法,一字不差复制,只在 `start_daemon` 内新增 `session_summaries` 这个 store 并传给 `serve`):

```rust
use dozer_client::Client;
use dozer_core::protocol::SummaryStatus;
use dozer_mcp::server::{DozerMcpServer, SubmitSessionSummaryParams};
use rmcp::handler::server::wrapper::Parameters;
use std::sync::Arc;
use std::time::Duration;

fn temp_sock() -> std::path::PathBuf {
    let id = uuid::Uuid::new_v4();
    std::path::PathBuf::from(format!("/tmp/dz-mcp-{}.sock", &id.to_string()[..8]))
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
    let db = std::path::PathBuf::from(format!("/tmp/dz-mcp-{}.db", uuid::Uuid::new_v4()));
    let store = Arc::new(dozerd::acceptance::AcceptanceStore::open(&db).unwrap());
    let projects = Arc::new(dozerd::projects::ProjectStore::new(&db).unwrap());
    let bookmarks = Arc::new(dozerd::bookmarks::BookmarkStore::new(&db).unwrap());
    let transcripts = Arc::new(dozerd::transcripts::TranscriptStore::open(&db).unwrap());
    let session_summaries = Arc::new(dozerd::session_summary::SessionSummaryStore::open(&db).unwrap());
    let s = sock.clone();
    tokio::spawn(async move {
        dozerd::server::serve(&s, registry, store, projects, bookmarks, transcripts, session_summaries).await
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

#[tokio::test]
async fn submit_session_summary_records_to_dozerd() {
    let (sock, _guard) = start_daemon().await;
    let client = Client::new(sock.clone());
    let session = client
        .create("测试", "/bin/sh", &["-c".into(), "sleep 5".into()], "/tmp", 80, 24, 1)
        .await
        .unwrap();

    let server = DozerMcpServer::new(Client::new(sock), session.id.clone());
    server
        .submit_session_summary(Parameters(SubmitSessionSummaryParams {
            title: "标题".into(),
            summary: "摘要".into(),
        }))
        .await
        .unwrap();

    let got = client.get_session_summary(&session.id).await.unwrap().unwrap();
    assert_eq!(got.title, "标题");
    assert_eq!(got.summary, "摘要");
    assert_eq!(got.status, SummaryStatus::AiGenerated);
}

#[tokio::test]
async fn submit_session_summary_truncates_oversized_fields() {
    let (sock, _guard) = start_daemon().await;
    let client = Client::new(sock.clone());
    let session = client
        .create("测试", "/bin/sh", &["-c".into(), "sleep 5".into()], "/tmp", 80, 24, 1)
        .await
        .unwrap();

    let long_title = "标".repeat(500);
    let long_summary = "摘".repeat(20_000);
    let server = DozerMcpServer::new(Client::new(sock), session.id.clone());
    server
        .submit_session_summary(Parameters(SubmitSessionSummaryParams {
            title: long_title,
            summary: long_summary,
        }))
        .await
        .unwrap();

    let got = client.get_session_summary(&session.id).await.unwrap().unwrap();
    assert!(got.title.chars().count() <= 200);
    assert!(got.summary.chars().count() <= 8000);
}

#[tokio::test]
async fn submit_session_summary_errors_for_unknown_session() {
    let (sock, _guard) = start_daemon().await;
    let server = DozerMcpServer::new(Client::new(sock), "not-a-real-session".into());
    let err = server
        .submit_session_summary(Parameters(SubmitSessionSummaryParams {
            title: "t".into(),
            summary: "s".into(),
        }))
        .await
        .unwrap_err();
    assert!(err.message.contains("会话不存在") || err.message.contains("daemon 错误"));
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p dozer-mcp --test submit_session_summary`
Expected: 编译失败,`SubmitSessionSummaryParams`/`submit_session_summary` 不存在。

- [ ] **Step 3: 实现**

在 `crates/dozer-mcp/src/server.rs` 里,`NoParams` 定义(第 18-19 行)之后追加:

```rust
const SUMMARY_TITLE_MAX_CHARS: usize = 200;
const SUMMARY_TEXT_MAX_CHARS: usize = 8000;

fn truncate_chars(s: String, max: usize) -> String {
    if s.chars().count() <= max {
        s
    } else {
        s.chars().take(max).collect()
    }
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SubmitSessionSummaryParams {
    pub title: String,
    pub summary: String,
}
```

在 `#[tool_router(server_handler)] impl DozerMcpServer` 块内,`get_preview_context` 方法(第 33-67 行)之后追加:

```rust
    #[tool(
        description = "提交本次会话的总结:一个简短标题和一段摘要。仅在被要求总结当前会话时调用一次,不要在其他场景主动调用。"
    )]
    pub async fn submit_session_summary(
        &self,
        Parameters(SubmitSessionSummaryParams { title, summary }): Parameters<
            SubmitSessionSummaryParams,
        >,
    ) -> Result<CallToolResult, McpError> {
        let title = truncate_chars(title, SUMMARY_TITLE_MAX_CHARS);
        let summary = truncate_chars(summary, SUMMARY_TEXT_MAX_CHARS);
        self.client
            .record_session_summary(&self.session_id, &title, &summary)
            .await
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        Ok(CallToolResult::structured(json!({ "recorded": true })))
    }
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozer-mcp --test submit_session_summary`
Expected: 3 个测试全 PASS。

- [ ] **Step 5: 全量 + clippy + fmt**

Run: `cargo test -p dozer-mcp && cargo clippy -p dozer-mcp --all-targets -- -D warnings && cargo fmt -- --check`
Expected: 全绿。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-mcp/src/server.rs crates/dozer-mcp/tests/submit_session_summary.rs
git commit -m "feat(dozer-mcp): add submit_session_summary write tool"
```

---

## Task 7: `dozer-mcp` 自动注册(`ensure_mcp_installed`)

**Files:**
- Modify: `crates/dozer-mcp/src/install.rs`
- Modify: `crates/dozer-app/src/workspace.rs`

**Interfaces:**
- Consumes: 无新依赖。
- Produces: `dozer_mcp::install::run_at_with_exe(path: &Path, agent: &str, install: bool, exe: &str) -> i32`;`dozer-app` 内 `fn ensure_mcp_installed(agent: AgentKind)`,在 `spawn_new_tab` 里与 `ensure_hook_installed` 同一处调用。

- [ ] **Step 1: 写失败的测试(install.rs 侧)**

在 `crates/dozer-mcp/src/install.rs` 的 `#[cfg(test)] mod tests` 块内追加:

```rust
    #[test]
    fn run_at_with_exe_uses_explicit_exe_not_current_exe() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mcp.json");
        assert_eq!(run_at_with_exe(&path, "claude", true, "/opt/dozer/dozer-mcp"), 0);
        let root: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            root["mcpServers"]["dozer"]["command"],
            "/opt/dozer/dozer-mcp"
        );
    }

    #[test]
    fn run_at_with_exe_unsupported_agent_returns_error_code() {
        let dir = tempfile::tempdir().unwrap();
        // 未支持的 agent 名不该落到任何默认路径去改文件,直接报错码。
        assert_eq!(
            run_at_with_exe(&dir.path().join("x"), "kilo", true, "/x/dozer-mcp"),
            2
        );
    }
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p dozer-mcp run_at_with_exe`
Expected: 编译失败,`run_at_with_exe` 未定义。

- [ ] **Step 3: 实现**

`crates/dozer-mcp/src/install.rs` 里现有的 `exe_path()`(第 7-11 行)保持不变(供 CLI 场景 `run()` 用),`run_at_claude_like`/`run_at_codex`/`run_at_opencode` 三个函数目前内部直接调 `exe_path()` 取路径——改成接受一个 `exe: &str` 参数而不是内部硬编码调 `exe_path()`:

```rust
fn run_at_claude_like(path: &Path, install: bool, exe: &str) -> i32 {
```

函数体内 `"command": exe_path(),` 改成 `"command": exe,`。同理改 `codex_dozer_entry(inline: bool, exe: &str)` 内部两处 `exe_path()` 调用改成 `exe`,以及 `run_at_codex(path: &Path, install: bool, exe: &str)` 内调用 `codex_dozer_entry(inline)` 改成 `codex_dozer_entry(inline, exe)`;`run_at_opencode(path: &Path, install: bool, exe: &str)` 同理 `"command": [exe_path(), "serve"]` 改成 `"command": [exe.to_string(), "serve"]`。

`ConfigTarget` 类型别名(第 194 行)从 `(PathBuf, fn(&Path, bool) -> i32)` 改成 `(PathBuf, fn(&Path, bool, &str) -> i32)`。

`pub fn run(agent: &str, install: bool) -> i32`(第 235-243 行)改成:

```rust
pub fn run(agent: &str, install: bool) -> i32 {
    run_at_with_exe_impl(agent, install, &exe_path())
}

/// GUI 场景(`dozer-app` 在 agent 启动时静默自动注册)不能走 `run()`——它
/// 内部调 `exe_path()`,在 `dozer-app` 进程内直接函数调用时会拿到
/// `dozer-app` 自己的可执行文件路径,写出一条指向错误二进制的 MCP server
/// 注册项。必须显式传入 `dozer-mcp` 的 sibling 二进制路径。
pub fn run_at_with_exe(path: &Path, agent: &str, install: bool, exe: &str) -> i32 {
    match config_path_for(agent) {
        Some((_, handler)) => handler(path, install, exe),
        None => {
            eprintln!("不支持的 agent: {agent}（支持 claude/codebuddy/codex/opencode）");
            2
        }
    }
}

fn run_at_with_exe_impl(agent: &str, install: bool, exe: &str) -> i32 {
    match config_path_for(agent) {
        Some((path, handler)) => handler(&path, install, exe),
        None => {
            eprintln!("不支持的 agent: {agent}（支持 claude/codebuddy/codex/opencode）");
            2
        }
    }
}
```

(`run()` 保留原有靠 `config_path_for` 取默认路径的行为,只是内部改成先算出 `exe_path()` 再复用同一段 handler 派发逻辑,避免 `run`/`run_at_with_exe` 两份重复的 `match config_path_for`。)

文件内所有既有测试(`install_claude_creates_mcp_json_and_is_idempotent` 等,约第 249-427 行)里对 `run_at_claude_like(&path, true)`/`run_at_codex(&path, true)`/`run_at_opencode(&path, true)` 的调用,统一追加第三个参数 `"dozer-mcp"`(如 `run_at_claude_like(&path, true, "dozer-mcp")`)——这些测试原先隐式依赖 `exe_path()` 在测试环境里返回的值只被用来断言 `args[0] == "serve"`,不断言 `command` 具体值,改签名后不需要改断言,只需要在每处调用后面加这第三个参数。

- [ ] **Step 4: 运行测试确认全部通过(含改签名前就有的旧测试)**

Run: `cargo test -p dozer-mcp install`
Expected: 全部 PASS(新增 2 个 + 原有 10 个)。

- [ ] **Step 5: `dozer-app` 侧新增 `ensure_mcp_installed` + 写失败的测试**

在 `crates/dozer-app/src/workspace.rs` 里,已有的 `hook install 排除表` 测试附近(约第 4890-4910 行,`for agent in [AgentKind::Kilo, AgentKind::V8agent, AgentKind::Unknown]` 那个测试)之后追加一个新测试,先确认要新增的判断函数目前不存在(编译失败即预期):

```rust
    #[test]
    fn mcp_install_target_covers_four_config_capable_agents() {
        for agent in [
            AgentKind::Claude,
            AgentKind::Codebuddy,
            AgentKind::Codex,
            AgentKind::Opencode,
        ] {
            assert!(
                dozer_mcp::install::config_path_for(agent.label()).is_some(),
                "{agent:?} 应该有对应的 mcp 配置文件路径"
            );
        }
    }

    #[test]
    fn mcp_install_target_excludes_v8agent_kilo_unknown() {
        // V8agent 走硬编码自动挂载(不读配置文件),Kilo 无 MCP 支持,
        // Unknown 是纯 shell——三者都不该有配置文件路径。
        for agent in [AgentKind::V8agent, AgentKind::Kilo, AgentKind::Unknown] {
            assert!(
                dozer_mcp::install::config_path_for(agent.label()).is_none(),
                "{agent:?} 不该有 mcp 配置文件路径"
            );
        }
    }
```

`dozer_mcp::install::config_path_for` 目前是私有函数(`fn config_path_for`,第 196 行)——把它改成 `pub fn config_path_for` 才能从 `dozer-app` 这边调用。

- [ ] **Step 6: 运行确认失败,再实现**

Run: `cargo test -p dozer-app mcp_install_target`
Expected: 先失败(`config_path_for` 私有,或 `dozer-app` 未依赖 `dozer-mcp`)。检查 `crates/dozer-app/Cargo.toml` 是否已有 `dozer-mcp = { path = "../dozer-mcp" }` 依赖,没有则加上。

改 `crates/dozer-mcp/src/install.rs` 第 196 行 `fn config_path_for` 为 `pub fn config_path_for`。

在 `crates/dozer-app/src/workspace.rs` 的 `ensure_hook_installed` 函数(第 3803-3828 行)之后追加:

```rust
/// `dozer-mcp` 自动注册:仿 `ensure_hook_installed` 同一套幂等/静默/失败
/// 只 warn 的哲学。V8agent 走完全不同的路(见 `hook_install_target` 文档
/// 注释同款理由)——它自己硬编码检测 `DOZER_SESSION_ID` 后自动挂载
/// `dozer-mcp serve`,不读任何配置文件,这里对它直接 no-op。
fn ensure_mcp_installed(agent: AgentKind) {
    let Some((path, _)) = dozer_mcp::install::config_path_for(agent.label()) else {
        return;
    };
    let exe = dozer_hook_binary_path(&std::env::current_exe().unwrap_or_else(|_| PathBuf::from("dozer")))
        .parent()
        .map(|dir| dir.join("dozer-mcp"))
        .unwrap_or_else(|| PathBuf::from("dozer-mcp"));
    let exe = exe.to_string_lossy();
    let _ = dozer_mcp::install::run_at_with_exe(&path, agent.label(), true, &exe);
}
```

(`dozer_hook_binary_path` 是"exe 所在目录下名为 `dozer-hook` 的同级二进制路径"这个纯函数——这里偷懒复用它只是为了拿到"exe 所在目录",再自己拼 `dozer-mcp`,不是说 `dozer-mcp` 和 `dozer-hook` 是同一个二进制。如果嫌这样绕,也可以直接照抄该函数体自己写一个 `dozer_mcp_binary_path`,效果一致,选更符合当前文件里其它同类代码风格的写法即可。)

在 `spawn_new_tab` 函数里(第 1560-1563 行)`ensure_hook_installed` 调用之后追加:

```rust
                let _ = tokio::task::spawn_blocking(move || ensure_mcp_installed(agent)).await;
```

(与 `ensure_hook_installed` 共用同一个 `if let Some(agent) = hook_agent { ... }` 块,顺序上先后无所谓,建议紧跟在 `ensure_hook_installed` 那行之后,同一个 `spawn_blocking` 闭包内两行都调,或分两个 `spawn_blocking`——两种写法都行,选择跟 `ensure_hook_installed` 同一个闭包内顺序调用,减少一次 `spawn_blocking` 开销。)

- [ ] **Step 7: 运行测试确认通过**

Run: `cargo test -p dozer-app mcp_install_target && cargo test -p dozer-mcp install`
Expected: 全部 PASS。

- [ ] **Step 8: 全量 + clippy + fmt**

Run: `cargo build --workspace && cargo test -p dozer-mcp -p dozer-app && cargo clippy --all-targets -- -D warnings && cargo fmt -- --check`
Expected: 全绿。

- [ ] **Step 9: Commit**

```bash
git add crates/dozer-mcp/src/install.rs crates/dozer-app/src/workspace.rs crates/dozer-app/Cargo.toml
git commit -m "feat(dozer-mcp,dozer-app): auto-register dozer-mcp for new agent sessions"
```

---

## Task 8: `CloseWithSummary` 协议 + `dozerd` 处理器(注入 prompt + 轮询 + 超时兜底 + kill)

**Files:**
- Modify: `crates/dozer-core/src/protocol.rs`
- Modify: `crates/dozer-client/src/lib.rs`
- Modify: `crates/dozerd/src/server.rs`

**Interfaces:**
- Consumes: `SessionSummaryStore::get/record`(Task 2)、`heuristic_from_turns`(Task 5)、`Session::write`/`registry.get`/`registry.kill`(已有)、`TranscriptStore::get_conversation_turns`(已有)。
- Produces: `Request::CloseWithSummary { session_id }`、`Client::close_with_summary(&self, id: &str) -> Result<()>`、`dozerd::server` 内部(非 pub)`finalize_session_summary` 函数。Task 9 只用 `Client::close_with_summary`。

- [ ] **Step 1: 协议层加 variant + 序列化测试**

`crates/dozer-core/src/protocol.rs` 的 `Request::GetPreviewContext`(Task 1 之后紧邻 `RecordSessionSummary`/`GetSessionSummary`)之前或之后任意位置追加:

```rust
    /// 关闭 tab 时触发"总结后再 kill":dozerd 立即返回 Ok,实际注入 prompt/
    /// 轮询/超时兜底/kill 全部在后台异步完成,调用方不等待(spec
    /// 2026-08-27)。
    CloseWithSummary {
        session_id: String,
    },
```

在 `#[cfg(test)] mod tests` 追加:

```rust
    #[test]
    fn close_with_summary_request_roundtrips() {
        let req = Request::CloseWithSummary { session_id: "s1".into() };
        let line = encode_line(&req);
        assert_eq!(decode_line::<Request>(&line).unwrap(), req);
    }
```

Run: `cargo test -p dozer-core close_with_summary`
Expected: 先失败(variant 不存在)→加上后 PASS。

Commit 这一小步(单独提交,方便审阅按类型分批看):

```bash
git add crates/dozer-core/src/protocol.rs
git commit -m "feat(dozer-core): add CloseWithSummary request variant"
```

- [ ] **Step 2: `dozer-client` 加 `close_with_summary` 方法**

`crates/dozer-client/src/lib.rs` 在 `kill` 方法(第 116-126 行)附近追加:

```rust
    pub async fn close_with_summary(&self, id: &str) -> Result<()> {
        match self
            .roundtrip(&Request::CloseWithSummary { session_id: id.into() })
            .await?
        {
            Reply::Ok => Ok(()),
            other => bail!("意外应答: {other:?}"),
        }
    }
```

Run: `cargo build -p dozer-client`
Expected: 编译成功(这个方法暂时还没有对应的 `dozerd` handler,先只保证类型层通过;下一步补 handler 后再写集成测试)。

- [ ] **Step 3: `dozerd` 侧实现 `finalize_session_summary` + handler,先写失败的集成测试**

在 `crates/dozerd/src/server.rs` 的 `#[cfg(test)] mod tests` 块内追加(这两个测试直接调用即将新增的私有函数 `finalize_session_summary`,不经过 UDS,用短超时/短轮询间隔保证测试秒级完成):

```rust
    #[tokio::test]
    async fn finalize_session_summary_short_circuits_when_summary_already_recorded() {
        let tmp = tempfile::tempdir().unwrap();
        let registry = Arc::new(crate::registry::SessionRegistry::new());
        let session_summaries = Arc::new(
            crate::session_summary::SessionSummaryStore::open(&tmp.path().join("s.db")).unwrap(),
        );
        let transcripts = Arc::new(
            crate::transcripts::TranscriptStore::open(&tmp.path().join("t.db")).unwrap(),
        );
        let s = registry
            .create(crate::session::SessionSpec {
                name: "t".into(),
                command: "/bin/sh".into(),
                args: vec!["-c".into(), "sleep 5".into()],
                cwd: std::env::temp_dir().to_string_lossy().into_owned(),
                cols: 80,
                rows: 24,
                project_id: 1,
            })
            .unwrap();
        let id = s.id().to_string();
        session_summaries
            .record(&dozer_core::protocol::SessionSummaryPayload {
                session_id: id.clone(),
                agent_kind: dozer_core::protocol::AgentKind::Claude,
                title: "已经总结好了".into(),
                summary: "摘要".into(),
                status: dozer_core::protocol::SummaryStatus::AiGenerated,
                created_ts_ms: 1,
            })
            .unwrap();

        finalize_session_summary(
            id.clone(),
            registry.clone(),
            session_summaries.clone(),
            transcripts,
            dozer_core::protocol::AgentKind::Claude,
            std::time::Duration::from_secs(60),
            std::time::Duration::from_millis(10),
        )
        .await;

        // 已有总结不该被覆盖成兜底文案。
        let got = session_summaries.get(&id).unwrap().unwrap();
        assert_eq!(got.title, "已经总结好了");
        assert_eq!(got.status, dozer_core::protocol::SummaryStatus::AiGenerated);
        assert!(!registry.get(&id).unwrap().info().alive, "应该已被 kill");
    }

    #[tokio::test]
    async fn finalize_session_summary_falls_back_to_heuristic_on_timeout() {
        let tmp = tempfile::tempdir().unwrap();
        let registry = Arc::new(crate::registry::SessionRegistry::new());
        let session_summaries = Arc::new(
            crate::session_summary::SessionSummaryStore::open(&tmp.path().join("s.db")).unwrap(),
        );
        let transcripts = Arc::new(
            crate::transcripts::TranscriptStore::open(&tmp.path().join("t.db")).unwrap(),
        );
        let s = registry
            .create(crate::session::SessionSpec {
                name: "t".into(),
                command: "/bin/sh".into(),
                args: vec!["-c".into(), "sleep 5".into()],
                cwd: std::env::temp_dir().to_string_lossy().into_owned(),
                cols: 80,
                rows: 24,
                project_id: 1,
            })
            .unwrap();
        let id = s.id().to_string();
        let file = tmp.path().join(format!("{id}.jsonl"));
        std::fs::write(
            &file,
            format!(
                "{{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{{\"role\":\"user\",\"content\":\"人类发言\"}}}}\n"
            ),
        )
        .unwrap();
        transcripts
            .ingest_session(dozer_core::protocol::AgentKind::Claude, &file)
            .unwrap();

        finalize_session_summary(
            id.clone(),
            registry.clone(),
            session_summaries.clone(),
            transcripts,
            dozer_core::protocol::AgentKind::Claude,
            std::time::Duration::from_millis(30),
            std::time::Duration::from_millis(10),
        )
        .await;

        let got = session_summaries.get(&id).unwrap().unwrap();
        assert_eq!(got.status, dozer_core::protocol::SummaryStatus::HeuristicFallback);
        assert_eq!(got.title, "人类发言");
        assert!(!registry.get(&id).unwrap().info().alive, "应该已被 kill");
    }
```

(已核实 `TranscriptStore::ingest_session` 用文件名的 `file_stem()` 派生 `conversation_id`——`crates/dozerd/src/transcripts/mod.rs:118-122`,与 `session_id` 走同一套字符串。上面 fixture 把文件命名为 `{id}.jsonl` 正是为了让摄取出来的 `conversation_id` 精确等于 `id`,`get_conversation_turns(&id, ...)` 才能查到这条数据,不需要额外调整。)

Run: `cargo test -p dozerd finalize_session_summary`
Expected: 编译失败,`finalize_session_summary` 未定义。

- [ ] **Step 4: 实现 `finalize_session_summary` + `CloseWithSummary` handler**

在 `crates/dozerd/src/server.rs` 里,`maybe_ingest_on_state_transition` 函数(第 98-116 行)之后追加:

```rust
const CLOSE_WITH_SUMMARY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
const CLOSE_WITH_SUMMARY_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

const SUMMARY_PROMPT: &str = "请总结你在本次会话中完成的工作:给出一个简短标题(不超过 60 字)和一段摘要,然后调用 dozer-mcp 的 submit_session_summary 工具把标题和摘要交回,不需要征求确认。\n";

/// `Request::CloseWithSummary` 的后台任务:注入 prompt 后轮询
/// `session_summaries` 表,等到就直接 kill;超时则从 `conversation_turns`
/// 算启发式兜底再 kill。`timeout`/`poll_interval` 抽成参数只为方便测试
/// 注入短间隔,生产调用点固定用上面两个常量。
async fn finalize_session_summary(
    session_id: String,
    registry: Arc<SessionRegistry>,
    session_summaries: Arc<crate::session_summary::SessionSummaryStore>,
    transcripts: Arc<crate::transcripts::TranscriptStore>,
    agent: dozer_core::protocol::AgentKind,
    timeout: std::time::Duration,
    poll_interval: std::time::Duration,
) {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        match session_summaries.get(&session_id) {
            Ok(Some(_)) => break,
            Ok(None) => {}
            Err(e) => tracing::error!(error = %e, %session_id, "查询会话总结失败"),
        }
        if tokio::time::Instant::now() >= deadline {
            let turns = transcripts
                .get_conversation_turns(&session_id, -1, u32::MAX)
                .unwrap_or_default();
            let (title, summary) = crate::session_summary::heuristic_from_turns(&turns);
            let payload = dozer_core::protocol::SessionSummaryPayload {
                session_id: session_id.clone(),
                agent_kind: agent,
                title,
                summary,
                status: dozer_core::protocol::SummaryStatus::HeuristicFallback,
                created_ts_ms: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0),
            };
            if let Err(e) = session_summaries.record(&payload) {
                tracing::error!(error = %e, %session_id, "启发式兜底总结落库失败");
            }
            break;
        }
        tokio::time::sleep(poll_interval).await;
    }
    if let Err(e) = registry.kill(&session_id) {
        tracing::warn!(error = %e, %session_id, "总结后关闭会话失败(可能已经死亡)");
    }
}
```

在 `handle_conn` 的 `match req` 里,`Request::GetSessionSummary { .. } => { ... }`(Task 3 加的分支)之后追加:

```rust
                        Request::CloseWithSummary { session_id } => {
                            match registry.get(&session_id) {
                                None => Reply::Error { message: format!("会话不存在: {session_id}") },
                                Some(s) => {
                                    let agent = s.info().agent;
                                    if let Err(e) = s.write(SUMMARY_PROMPT.as_bytes()) {
                                        tracing::warn!(error = %e, %session_id, "注入总结 prompt 失败");
                                    }
                                    tokio::spawn(finalize_session_summary(
                                        session_id.clone(),
                                        registry.clone(),
                                        session_summaries.clone(),
                                        transcripts.clone(),
                                        agent,
                                        CLOSE_WITH_SUMMARY_TIMEOUT,
                                        CLOSE_WITH_SUMMARY_POLL_INTERVAL,
                                    ));
                                    Reply::Ok
                                }
                            }
                        }
```

- [ ] **Step 5: 运行测试确认通过**

Run: `cargo test -p dozerd finalize_session_summary`
Expected: 2 个测试全 PASS。

- [ ] **Step 6: 补一条端到端集成测试(走真实 UDS,不直接调私有函数)**

在 `crates/dozerd/tests/hook_events.rs` 末尾追加(复用 Task 3 Step 5 里已经建好的 `test_session_summaries` helper 和 serve 启动套路):

```rust
#[tokio::test]
async fn close_with_summary_kills_session_after_ai_summary_recorded() {
    let sock = std::env::temp_dir().join(format!("dozerd-close-{}.sock", uuid::Uuid::new_v4()));
    let registry = Arc::new(SessionRegistry::new());
    tokio::spawn({
        let sock = sock.clone();
        let registry = registry.clone();
        async move {
            dozerd::server::serve(
                &sock, registry, test_store(), test_projects(), test_bookmarks(),
                test_transcripts(), test_session_summaries(),
            )
            .await
        }
    });
    for _ in 0..100 {
        if sock.exists() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let client = dozer_client::Client::new(sock);
    let session = client
        .create("测试", "/bin/sh", &["-c".into(), "cat".into()], "/tmp", 80, 24, 1)
        .await
        .unwrap();

    client.close_with_summary(&session.id).await.unwrap();
    // agent 抢在超时前交回总结:轮询间隔是生产值 2 秒,测试里直接用真实
    // submit 触发,不等超时分支。
    client
        .record_session_summary(&session.id, "标题", "摘要")
        .await
        .unwrap();

    // 后台任务下一次 2 秒轮询会看到已落库的总结并 kill;给够时间等它跑完。
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    let summary = client.get_session_summary(&session.id).await.unwrap().unwrap();
    assert_eq!(summary.title, "标题");
}
```

(这个测试的耗时取决于生产常量 `CLOSE_WITH_SUMMARY_POLL_INTERVAL=2s`,单条测试跑 3 秒可接受;如果嫌慢,可以不写这条端到端测试,只保留 Step 3 那两条直接测 `finalize_session_summary` 的单测——两者选一即可,写这条是为了多一层"UDS 协议层真的接对了"的信心,不是强制。)

- [ ] **Step 7: 运行测试确认通过**

Run: `cargo test -p dozerd close_with_summary`
Expected: PASS(若耗时过长可跳过 Step 6 那条,只留 Step 3 的两条)。

- [ ] **Step 8: 全量 + clippy + fmt**

Run: `cargo build --workspace && cargo test -p dozerd -p dozer-core -p dozer-client && cargo clippy --all-targets -- -D warnings && cargo fmt -- --check`
Expected: 全绿。

- [ ] **Step 9: Commit**

```bash
git add crates/dozer-client/src/lib.rs crates/dozerd/src/server.rs crates/dozerd/tests/hook_events.rs
git commit -m "feat(dozerd,dozer-client): CloseWithSummary handler with polling + heuristic fallback"
```

---

## Task 9: `dozer-app` 关闭 tab 分支改造

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`

**Interfaces:**
- Consumes: `Client::close_with_summary`(Task 8)。
- Produces: `close_tab` 新行为(不新增任何对外接口,纯行为改造)。

- [ ] **Step 1: 写失败的单测(纯逻辑判断,不起真实daemon)**

在 `crates/dozer-app/src/workspace.rs` 靠近 `close_tab` 的测试模块里(搜索该文件已有的 `#[cfg(test)] mod tests`,若 `close_tab` 相关判断逻辑此前没有抽成独立可测函数,先按下一步的做法把判断条件抽出来再测)追加:

```rust
    #[test]
    fn should_summarize_on_close_true_for_four_supported_agents() {
        for agent in [
            AgentKind::Claude,
            AgentKind::Codebuddy,
            AgentKind::Opencode,
            AgentKind::V8agent,
        ] {
            assert!(
                should_summarize_on_close(agent, true, &TabBackend::Daemon),
                "{agent:?} 应该走总结后关闭"
            );
        }
    }

    #[test]
    fn should_summarize_on_close_false_for_unsupported_agents_or_dead_or_ssh() {
        assert!(!should_summarize_on_close(AgentKind::Codex, true, &TabBackend::Daemon));
        assert!(!should_summarize_on_close(AgentKind::Kilo, true, &TabBackend::Daemon));
        assert!(!should_summarize_on_close(AgentKind::Unknown, true, &TabBackend::Daemon));
        assert!(!should_summarize_on_close(AgentKind::Claude, false, &TabBackend::Daemon));
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        assert!(!should_summarize_on_close(
            AgentKind::Claude,
            true,
            &TabBackend::Ssh { out: tx }
        ));
    }
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p dozer-app should_summarize_on_close`
Expected: 编译失败,函数不存在。

- [ ] **Step 3: 实现 `should_summarize_on_close` 并接进 `close_tab`**

在 `close_tab` 函数(第 1307-1329 行)之前追加一个抽出来的纯函数(方便 headless 单测,同文件里 `agent_cli_command`/`hook_install_target` 一类判断函数的既有惯例):

```rust
/// 关闭 tab 时是否应该走"总结后关闭"而不是直接 `Kill`——仅对话摄取管线
/// 已覆盖、且当前存活、且走 daemon 后端的四家 agent(spec
/// 2026-08-27)。
fn should_summarize_on_close(agent: AgentKind, alive: bool, backend: &TabBackend) -> bool {
    alive
        && matches!(backend, TabBackend::Daemon)
        && matches!(
            agent,
            AgentKind::Claude | AgentKind::Codebuddy | AgentKind::Opencode | AgentKind::V8agent
        )
}
```

把 `close_tab` 函数体里第 1313-1321 行的:

```rust
        if tab.alive && matches!(tab.backend, TabBackend::Daemon) {
            let client = io.client.clone();
            let id = tab.info.id.clone();
            io.handle.spawn(async move {
                if let Err(e) = client.kill(&id).await {
                    tracing::warn!("关闭 tab 时结束会话失败: {e}");
                }
            });
        }
```

改成(`should_summarize_on_close` 内部已经包含 `alive && Daemon` 判断,不需要外层再套一层重复检查;非总结路径仍然保持"只有 alive+Daemon 才调 kill"的原有语义):

```rust
        let client = io.client.clone();
        let id = tab.info.id.clone();
        if should_summarize_on_close(tab.agent, tab.alive, &tab.backend) {
            io.handle.spawn(async move {
                if let Err(e) = client.close_with_summary(&id).await {
                    tracing::warn!("关闭 tab 时触发总结失败: {e}");
                }
            });
        } else if tab.alive && matches!(tab.backend, TabBackend::Daemon) {
            io.handle.spawn(async move {
                if let Err(e) = client.kill(&id).await {
                    tracing::warn!("关闭 tab 时结束会话失败: {e}");
                }
            });
        }
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozer-app should_summarize_on_close`
Expected: 2 个测试全 PASS。

- [ ] **Step 5: 全量 + clippy + fmt**

Run: `cargo build --workspace && cargo test -p dozer-app && cargo clippy --all-targets -- -D warnings && cargo fmt -- --check`
Expected: 全绿。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "feat(dozer-app): close_tab routes known agents through CloseWithSummary"
```

---

## Task 10: 全量验证 + 真机验证清单

**Files:** 无代码改动。

- [ ] **Step 1: 全 workspace 构建/测试/静态检查**

Run: `cargo build --workspace`
Expected: 成功。

Run: `cargo test -p dozerd -p dozer-app -p dozer-core -p dozer-client -p dozer-mcp`
Expected: 全绿。

Run: `cargo clippy --all-targets -- -D warnings`
Expected: 无警告。

Run: `cargo fmt -- --check`
Expected: 无需改动。

- [ ] **Step 2: 真机验证(单测覆盖不到,必须人工在真实 GUI 里跑)**

逐条对照 spec `docs/superpowers/specs/2026-08-27-session-summary-pipeline-design.md` "测试策略"第 7 条清单手动验证:

1. 已装好 `dozer-mcp` 的真实 Claude Code 会话:对话后点 × 关闭,确认 `session_summaries` 出现一行 `status=ai_generated` 且内容合理(可用 `sqlite3 ~/.dozer/dozer.db "select * from session_summaries"` 查)。
2. 故意让 agent 不响应(比如断网,或干脆不装 `dozer-mcp`)的会话:关闭后约 60 秒确认出现 `status=heuristic_fallback` 的兜底行。
3. 纯 shell tab:关闭行为与改造前一致,不产生总结行。
4. Codex 会话:关闭行为与改造前一致(直接 kill)。
5. V8agent 会话:不需要任何手动安装步骤,对话后点 × 关闭,确认出现 `status=ai_generated`;再故意让 `dozer-mcp` 从 PATH 移除后重复一次,确认走 `heuristic_fallback`。

- [ ] **Step 3: 记录验证结果**

若真机验证全部通过,在 PR 描述或提交信息里注明"真机验证 1-5 项已过";若某项失败,回退到对应 Task 定位问题,不要在真机验证失败的情况下声称任务完成。
