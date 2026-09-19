# dozerd 优雅停止 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让顶栏 Settings 弹窗新增一个"停止 dozerd"/"重新启动 dozerd"按钮,点击后 dozerd 对所有存活 agent 会话跑一遍现有的"总结后关闭"流程,再优雅退出进程;不改动 macOS 原生 Quit Dozer 菜单。

**Architecture:** 新增 UDS 协议请求 `Request::Shutdown`(`dozer-core`)。`dozerd` 收到后原子置位一个 `draining` 标记(挡住新 `CreateSession`),把当前存活会话逐个丢进已有的 `finalize_session_summary`(与 `Request::CloseWithSummary` 复用同一函数/同一组 60s 超时+2s 轮询常量)并发跑完,再写回 `Reply::Ok`、用 `tokio::sync::Notify` 通知 `serve()` 的 accept 循环退出,进程随 `main()` 返回自然终止。`dozer-client` 新增 `shutdown_daemon()`,外层包 90s 通信层超时。`dozer-app` 的 `extensions::settings` 模块新增"高级"区块状态机(默认/确认中/停止中/已停止/重启中,失败态内嵌在默认态与已停止态里),复用已有的 `App.daemon_error: Option<String>` 字段做全局提示,不新建 UI 组件;重新启动直接复用现有 `runtime::ensure_daemon()`(它本身就会在连不上时自动 `spawn_dozerd()` 并重试)。

**Tech Stack:** Rust workspace(`dozer-core`/`dozerd`/`dozer-client`/`dozer-app`)、tokio(`AtomicBool` + `Notify` 做 accept 循环的优雅退出信号)、iced 0.14(Settings 弹窗状态机)。

**Spec:** `docs/superpowers/specs/2026-09-19-dozerd-graceful-shutdown-design.md`(写 plan 过程中发现两处需要修订 spec 的技术细节,已经在写 plan 前同步改过 spec 并提交:①"总结后关闭"实际是往 PTY 注入 prompt 等 agent 自己总结、单会话最长 60s,不是最初设想的轻量 10s 操作;②全局提示复用现有 `App.daemon_error` 字段,不新建 footbar 段落,也不做主动心跳检测意外断连)。

## Global Constraints

- **在独立分支上开发,不要直接提交到 main。** 建分支 `feature/dozerd-graceful-shutdown`,从执行任务时刻的 main 最新提交切出(main 上有并发的自动化开发提交,开工前先 `git fetch`/确认本地 main 是最新的,不要假设某个固定 commit——`git log --oneline -1 main` 现场核实)。全部任务完成后提请代码审阅,审阅通过后再合并回 main,不在过程中合并中间状态。
- **不改动 macOS 原生 "Quit Dozer" 菜单**——它依然只退出 GUI 进程,不触碰 dozerd(spec「非目标」)。
- **不引入 `dozer-app` 持有 `dozerd` 进程 PID/`Child` 句柄的机制**——固定走 UDS 协议往返(spec「非目标」)。
- 停止时对所有存活会话的收尾复用现成的 `finalize_session_summary`(`crates/dozerd/src/server.rs`),沿用生产环境固定的 `CLOSE_WITH_SUMMARY_TIMEOUT`(60s)/`CLOSE_WITH_SUMMARY_POLL_INTERVAL`(2s)两个常量,不新增专属超时值(spec「dozerd 处理逻辑」)。
- `dozer-client::shutdown_daemon()` 外层通信超时固定 90s(spec「dozer-client」)。
- 不做"停止中途取消"——用户确认后此操作不可撤回(spec「非目标」)。
- 全局状态提示复用 `App.daemon_error: Option<String>`(`crates/dozer-app/src/app/app.rs:266`),不新建 UI 组件,不做主动心跳检测意外断连(spec「复用 App.daemon_error」)。

---

### Task 1: 修复已损坏的 `dozer-client` 集成测试基线

**背景(为什么先做这个):** `crates/dozer-client/tests/against_real_daemon.rs` 目前**编译不过**——它的 `start_daemon()` 调用 `dozerd::server::serve(...)` 时漏传了 `ide_lock_dir: PathBuf` 这个参数(这个参数是后来某次改动加到 `serve()` 签名里的,这份测试文件没有跟着更新)。这是一个跟本次需求无关的既有缺陷,但 Task 5 需要在这个文件里新增 `shutdown_daemon()` 的测试,必须先让它能编译。

**Files:**
- Modify: `crates/dozer-client/tests/against_real_daemon.rs:13-49`(`start_daemon()` 函数)

**Interfaces:**
- Consumes: `dozerd::server::serve` 现有签名(`socket: &Path, ide_lock_dir: PathBuf, registry, projects, bookmarks, transcripts, session_summaries, backfill_registry, todos, categories, in_flight`)。
- Produces: `start_daemon() -> (PathBuf, Arc<SessionRegistry>, CleanupGuard)`,签名不变,后续所有已有测试原样可用。

- [ ] **Step 1: 确认当前确实编译失败**

Run: `cargo check -p dozer-client --tests 2>&1 | tail -20`
Expected: `error[E0061]: this function takes 11 arguments but 10 arguments were supplied`,指向 `against_real_daemon.rs:28`。

- [ ] **Step 2: 补上缺失的 `ide_lock_dir` 参数**

在 `start_daemon()` 内部,`serve()` 调用之前构造一个临时目录并传入,写法与 `crates/dozerd/tests/session_survival.rs:60-77` 完全一致:

```rust
async fn start_daemon() -> (std::path::PathBuf, Arc<SessionRegistry>, CleanupGuard) {
    let (sock, id) = temp_sock();
    let registry = Arc::new(SessionRegistry::new());
    let s = sock.clone();
    let r = registry.clone();
    let db = std::path::PathBuf::from(format!("/tmp/dz-{}.db", uuid::Uuid::new_v4()));
    let projects = Arc::new(dozerd::projects::ProjectStore::new(&db).unwrap());
    let bookmarks = Arc::new(dozerd::bookmarks::BookmarkStore::new(&db).unwrap());
    let transcripts = Arc::new(dozerd::transcripts::TranscriptStore::open(&db).unwrap());
    let session_summaries =
        Arc::new(dozerd::session_summary::SessionSummaryStore::open(&db).unwrap());
    let backfill_registry = Arc::new(dozerd::session_summary_backfill::BackfillRegistry::new());
    let todos = Arc::new(dozerd::todo::TodoStore::new(&db).unwrap());
    let categories = Arc::new(dozerd::todo_category::CategoryStore::new(&db).unwrap());
    let ide_lock_dir = tempfile::tempdir().expect("ide_lock_dir tempdir");
    tokio::spawn(async move {
        dozerd::server::serve(
            &s,
            ide_lock_dir.path().to_path_buf(),
            r,
            projects,
            bookmarks,
            transcripts,
            session_summaries,
            backfill_registry,
            todos,
            categories,
            dozerd::task_poller::new_in_flight(),
        )
        .await
    });
    for _ in 0..100 {
        if sock.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    (sock, registry, CleanupGuard(id))
}
```

`crates/dozer-client/Cargo.toml` 目前的 `[dev-dependencies]` 只有 `dozerd`/`uuid`,没有 `tempfile`(已核实),需要加一行。找到:

```toml
[dev-dependencies]
dozerd = { path = "../dozerd" }
uuid.workspace = true
```

改成:

```toml
[dev-dependencies]
dozerd = { path = "../dozerd" }
uuid.workspace = true
tempfile = "3"
```

(与 `crates/dozerd/Cargo.toml` 里 `tempfile = "3"` 的写法保持一致,不用 `.workspace = true`。)

- [ ] **Step 3: 确认整份文件恢复编译且既有测试全部通过**

Run: `cargo test -p dozer-client --test against_real_daemon 2>&1 | tail -40`
Expected: 全部既有测试(`full_client_lifecycle`、`reader_task_exits_when_receiver_dropped` 等)PASS,无编译错误。

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-client/tests/against_real_daemon.rs crates/dozer-client/Cargo.toml
git commit -m "fix(dozer-client): 补上 against_real_daemon 测试里漏传的 ide_lock_dir 参数"
```

---

### Task 2: 协议层新增 `Request::Shutdown`

**Files:**
- Modify: `crates/dozer-core/src/protocol.rs:332-334`(`Request` 枚举,`Kill` 变体之后)
- Test: `crates/dozer-core/src/protocol.rs`(同文件内 `#[cfg(test)] mod tests`,与 `close_with_summary_request_roundtrips` 相邻)

**Interfaces:**
- Produces: `dozer_core::protocol::Request::Shutdown`(无字段的枚举变体),供 Task 3(dozerd 处理)与 Task 5(dozer-client)使用。

- [ ] **Step 1: 写失败的序列化往返测试**

在 `crates/dozer-core/src/protocol.rs` 的 `#[cfg(test)] mod tests` 里,紧跟 `close_with_summary_request_roundtrips` 之后加:

```rust
#[test]
fn shutdown_request_roundtrips() {
    let req = Request::Shutdown;
    let line = encode_line(&req);
    assert_eq!(decode_line::<Request>(&line).unwrap(), req);
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozer-core shutdown_request_roundtrips 2>&1 | tail -20`
Expected: 编译失败,`no variant named 'Shutdown' found for enum 'Request'`。

- [ ] **Step 3: 加上枚举变体**

在 `crates/dozer-core/src/protocol.rs` 里找到:

```rust
    Kill {
        session_id: String,
    },
```

改成:

```rust
    Kill {
        session_id: String,
    },
    /// 请求 daemon 对所有存活会话完成"总结后关闭"收尾,再退出进程自身。
    /// 与 `Kill`(杀单个会话)、`CloseWithSummary`(单会话总结后关闭)不同,
    /// 这是唯一一个"daemon 进程整体退出"的请求。
    Shutdown,
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozer-core shutdown_request_roundtrips 2>&1 | tail -20`
Expected: PASS。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-core/src/protocol.rs
git commit -m "feat(protocol): 新增 Request::Shutdown"
```

---

### Task 3: dozerd 处理 `Request::Shutdown`——draining 状态位 + 并发收尾 + 优雅退出

**Files:**
- Modify: `crates/dozerd/src/server.rs`(顶部 `use` 块、`handle_conn` 签名与匹配分支、`serve()` 主循环)
- Test: `crates/dozerd/src/server.rs`(同文件 `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: Task 2 的 `Request::Shutdown`;已有的 `finalize_session_summary`(`server.rs:225-290`)、`CLOSE_WITH_SUMMARY_TIMEOUT`/`CLOSE_WITH_SUMMARY_POLL_INTERVAL`(`server.rs:208-209`)、`conversation_id_for_session`(`server.rs:201-206`)、`SUMMARY_PROMPT`(`server.rs:211`)。
- Produces: `async fn drain_all_sessions(registry: Arc<SessionRegistry>, session_summaries: Arc<crate::session_summary::SessionSummaryStore>, transcripts: Arc<crate::transcripts::TranscriptStore>, timeout: std::time::Duration, poll_interval: std::time::Duration)`——`pub(crate)` 可见性即可(只在本文件内测试直接调用,不跨 crate)。`serve()` 的公开签名(11 个参数)保持不变,`draining`/`shutdown_signal` 两个新状态在函数内部构造,不对外暴露。

- [ ] **Step 1: 写 `drain_all_sessions` 的失败测试(0 会话 + 超时兜底两种)**

在 `crates/dozerd/src/server.rs` 现有的 `#[cfg(test)] mod tests` 块末尾(`use super::*;` 之后任意位置,建议紧跟文件末尾已有测试之后)加:

```rust
#[tokio::test]
async fn drain_all_sessions_with_no_sessions_returns_immediately() {
    let registry = Arc::new(crate::registry::SessionRegistry::new());
    let tmp = tempfile::tempdir().unwrap();
    let db = tmp.path().join("t.db");
    let transcripts = Arc::new(crate::transcripts::TranscriptStore::open(&db).unwrap());
    let session_summaries =
        Arc::new(crate::session_summary::SessionSummaryStore::open(&db).unwrap());

    let started = std::time::Instant::now();
    drain_all_sessions(
        registry,
        session_summaries,
        transcripts,
        std::time::Duration::from_secs(60),
        std::time::Duration::from_secs(2),
    )
    .await;
    assert!(
        started.elapsed() < std::time::Duration::from_millis(200),
        "0 会话不应该等待任何轮询间隔"
    );
}

#[tokio::test]
async fn drain_all_sessions_falls_back_to_heuristic_on_timeout_and_kills_session() {
    let registry = Arc::new(crate::registry::SessionRegistry::new());
    let session = registry
        .create(crate::session::SessionSpec {
            name: "test".into(),
            command: "/bin/sh".into(),
            args: vec!["-c".into(), "sleep 5".into()],
            cwd: std::env::temp_dir().to_string_lossy().into_owned(),
            cols: 80,
            rows: 24,
            project_id: 1,
        })
        .unwrap();
    let sid = session.id().to_string();
    let tmp = tempfile::tempdir().unwrap();
    let db = tmp.path().join("t.db");
    let transcripts = Arc::new(crate::transcripts::TranscriptStore::open(&db).unwrap());
    let session_summaries =
        Arc::new(crate::session_summary::SessionSummaryStore::open(&db).unwrap());

    // 短超时/短轮询只为测试注入,不代表生产行为——生产调用点固定用
    // `CLOSE_WITH_SUMMARY_TIMEOUT`/`CLOSE_WITH_SUMMARY_POLL_INTERVAL`。
    drain_all_sessions(
        registry.clone(),
        session_summaries.clone(),
        transcripts,
        std::time::Duration::from_millis(80),
        std::time::Duration::from_millis(10),
    )
    .await;

    let summary = session_summaries
        .get(&sid)
        .unwrap()
        .expect("超时后应该落一份启发式兜底总结");
    assert_eq!(
        summary.status,
        dozer_core::protocol::SummaryStatus::HeuristicFallback
    );
    assert!(
        !registry.get(&sid).unwrap().info().alive,
        "超时兜底后应该 kill 掉这个会话"
    );
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozerd drain_all_sessions 2>&1 | tail -20`
Expected: 编译失败,`cannot find function 'drain_all_sessions' in this scope`。

- [ ] **Step 3: 加上 `use` 与 `drain_all_sessions` 函数**

`crates/dozerd/src/server.rs` 顶部的 `use` 块:

```rust
use crate::ide_bridge::IdeBridgeRegistry;
use crate::preview_context::PreviewContextStore;
use crate::registry::SessionRegistry;
use crate::session::{SessionEvent, SessionSpec};
use anyhow::Result;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use dozer_core::protocol::{Reply, Request, decode_line, encode_line};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::broadcast;
```

改成(新增 `AtomicBool`/`Ordering`,`broadcast` 那行并入 `Notify`):

```rust
use crate::ide_bridge::IdeBridgeRegistry;
use crate::preview_context::PreviewContextStore;
use crate::registry::SessionRegistry;
use crate::session::{SessionEvent, SessionSpec};
use anyhow::Result;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use dozer_core::protocol::{Reply, Request, decode_line, encode_line};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Notify, broadcast};
```

紧跟在 `finalize_session_summary` 函数(以 `if let Err(e) = registry.kill(&session_id) { ... }` 结尾,`server.rs` 约290行)之后加:

```rust
/// `Request::Shutdown` 收尾:对 `registry` 里当前存活的每个会话复用
/// `finalize_session_summary`(与 `Request::CloseWithSummary` 同一函数),
/// 全部完成后返回。`timeout`/`poll_interval` 抽成参数只为方便测试注入
/// 短间隔,生产调用点(`Request::Shutdown` 分支)固定用
/// `CLOSE_WITH_SUMMARY_TIMEOUT`/`CLOSE_WITH_SUMMARY_POLL_INTERVAL`——同
/// `finalize_session_summary` 自身的既有约定。并发 spawn 每个会话各自的
/// 收尾任务,再逐个 await:每个任务自带截止时间且互不依赖,总耗时约等于
/// 其中最慢的一个,不需要再套一层外部超时。
async fn drain_all_sessions(
    registry: Arc<SessionRegistry>,
    session_summaries: Arc<crate::session_summary::SessionSummaryStore>,
    transcripts: Arc<crate::transcripts::TranscriptStore>,
    timeout: std::time::Duration,
    poll_interval: std::time::Duration,
) {
    let ids: Vec<String> = registry.list().into_iter().map(|info| info.id).collect();
    let mut handles = Vec::with_capacity(ids.len());
    for id in ids {
        let Some(s) = registry.get(&id) else { continue };
        let agent = s.info().agent;
        let conversation_id = conversation_id_for_session(&s);
        if let Err(e) = s.write(SUMMARY_PROMPT.as_bytes()) {
            tracing::warn!(error = %e, session_id = %id, "停止 dozerd 前注入总结 prompt 失败");
        }
        handles.push(tokio::spawn(finalize_session_summary(
            id,
            conversation_id,
            registry.clone(),
            session_summaries.clone(),
            transcripts.clone(),
            agent,
            timeout,
            poll_interval,
        )));
    }
    for h in handles {
        let _ = h.await;
    }
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozerd drain_all_sessions 2>&1 | tail -30`
Expected: 两个测试 PASS。

- [ ] **Step 5: `handle_conn` 签名加两个新参数**

找到(`server.rs:305-318`):

```rust
async fn handle_conn(
    stream: UnixStream,
    registry: Arc<SessionRegistry>,
    projects: Arc<crate::projects::ProjectStore>,
    bookmarks: Arc<crate::bookmarks::BookmarkStore>,
    preview_contexts: Arc<PreviewContextStore>,
    ide_bridge: Arc<IdeBridgeRegistry>,
    transcripts: Arc<crate::transcripts::TranscriptStore>,
    session_summaries: Arc<crate::session_summary::SessionSummaryStore>,
    backfill_registry: Arc<crate::session_summary_backfill::BackfillRegistry>,
    todos: Arc<crate::todo::TodoStore>,
    categories: Arc<crate::todo_category::CategoryStore>,
    in_flight: crate::task_poller::InFlight,
) -> Result<()> {
```

改成(末尾追加两个参数,不改动已有参数顺序):

```rust
async fn handle_conn(
    stream: UnixStream,
    registry: Arc<SessionRegistry>,
    projects: Arc<crate::projects::ProjectStore>,
    bookmarks: Arc<crate::bookmarks::BookmarkStore>,
    preview_contexts: Arc<PreviewContextStore>,
    ide_bridge: Arc<IdeBridgeRegistry>,
    transcripts: Arc<crate::transcripts::TranscriptStore>,
    session_summaries: Arc<crate::session_summary::SessionSummaryStore>,
    backfill_registry: Arc<crate::session_summary_backfill::BackfillRegistry>,
    todos: Arc<crate::todo::TodoStore>,
    categories: Arc<crate::todo_category::CategoryStore>,
    in_flight: crate::task_poller::InFlight,
    draining: Arc<AtomicBool>,
    shutdown_signal: Arc<Notify>,
) -> Result<()> {
```

- [ ] **Step 6: 在 `line = lines.next_line() => { ... }` 分支顶部加一个退出标记**

找到(`server.rs:326-330`):

```rust
    loop {
        tokio::select! {
            line = lines.next_line() => {
                let Some(line) = line? else { break }; // 客户端断连：直接退出，不动会话
                let reply = match decode_line::<Request>(&line) {
```

改成:

```rust
    loop {
        tokio::select! {
            line = lines.next_line() => {
                let Some(line) = line? else { break }; // 客户端断连：直接退出，不动会话
                let mut should_exit_after_reply = false;
                let reply = match decode_line::<Request>(&line) {
```

- [ ] **Step 7: `CreateSession` 分支加 draining 检查**

找到(`server.rs:334-363`,函数体在 `match req` 里)完整的这一段:

```rust
                        Request::CreateSession { name, command, args, cwd, cols, rows, project_id } => {
                            match registry.create(SessionSpec { name, command, args, cwd: cwd.clone(), cols, rows, project_id }) {
                                Ok(s) => {
                                    // `session_started` 失败(绑端口/写锁文件出错)时不会在
                                    // registry 里留下这次调用对应的记录——这种情况下绝不能
                                    // spawn 退出监听器,否则这个会话将来退出时会去 `session_ended`
                                    // 一个它从未真正占过的项目名额,把同项目下另一个真正活着的
                                    // 会话的 bridge 提前拆掉(复现过的 bug,见代码审查记录)。
                                    if ide_bridge.session_started(project_id, &cwd).await {
                                        let ide_bridge_watch = ide_bridge.clone();
                                        let mut exit_rx = s.subscribe();
                                        tokio::spawn(async move {
                                            loop {
                                                match exit_rx.recv().await {
                                                    Ok(SessionEvent::Exited { .. }) => {
                                                        ide_bridge_watch.session_ended(project_id).await;
                                                        break;
                                                    }
                                                    Ok(_) => continue,
                                                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                                                    Err(broadcast::error::RecvError::Closed) => break,
                                                }
                                            }
                                        });
                                    }
                                    Reply::Created { session: s.info() }
                                }
                                Err(e) => Reply::Error { message: e.to_string() },
                            }
                        }
```

整段替换成(只在最外层套一个 `if draining.load(...) { ... } else { ... }`,`else` 分支里是原封不动的原逻辑,新增一层缩进):

```rust
                        Request::CreateSession { name, command, args, cwd, cols, rows, project_id } => {
                            if draining.load(Ordering::SeqCst) {
                                Reply::Error { message: "dozerd 正在停止,无法创建新会话".into() }
                            } else {
                                match registry.create(SessionSpec { name, command, args, cwd: cwd.clone(), cols, rows, project_id }) {
                                    Ok(s) => {
                                        // `session_started` 失败(绑端口/写锁文件出错)时不会在
                                        // registry 里留下这次调用对应的记录——这种情况下绝不能
                                        // spawn 退出监听器,否则这个会话将来退出时会去 `session_ended`
                                        // 一个它从未真正占过的项目名额,把同项目下另一个真正活着的
                                        // 会话的 bridge 提前拆掉(复现过的 bug,见代码审查记录)。
                                        if ide_bridge.session_started(project_id, &cwd).await {
                                            let ide_bridge_watch = ide_bridge.clone();
                                            let mut exit_rx = s.subscribe();
                                            tokio::spawn(async move {
                                                loop {
                                                    match exit_rx.recv().await {
                                                        Ok(SessionEvent::Exited { .. }) => {
                                                            ide_bridge_watch.session_ended(project_id).await;
                                                            break;
                                                        }
                                                        Ok(_) => continue,
                                                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                                                        Err(broadcast::error::RecvError::Closed) => break,
                                                    }
                                                }
                                            });
                                        }
                                        Reply::Created { session: s.info() }
                                    }
                                    Err(e) => Reply::Error { message: e.to_string() },
                                }
                            }
                        }
```

(空格缩进按 `cargo fmt` 跑一遍会自动整理,不用手工对齐——Step 11 会跑。)

- [ ] **Step 8: 在 `Kill` 之后加 `Shutdown` 分支**

找到(`server.rs:404-407`):

```rust
                        Request::Kill { session_id } => match registry.kill(&session_id) {
                            Ok(()) => Reply::Ok,
                            Err(e) => Reply::Error { message: e.to_string() },
                        },
```

改成(在其后追加新分支):

```rust
                        Request::Kill { session_id } => match registry.kill(&session_id) {
                            Ok(()) => Reply::Ok,
                            Err(e) => Reply::Error { message: e.to_string() },
                        },
                        Request::Shutdown => {
                            if draining.compare_exchange(
                                false, true, Ordering::SeqCst, Ordering::SeqCst,
                            ).is_err() {
                                Reply::Error { message: "dozerd 正在停止中".into() }
                            } else {
                                drain_all_sessions(
                                    registry.clone(),
                                    session_summaries.clone(),
                                    transcripts.clone(),
                                    CLOSE_WITH_SUMMARY_TIMEOUT,
                                    CLOSE_WITH_SUMMARY_POLL_INTERVAL,
                                )
                                .await;
                                should_exit_after_reply = true;
                                Reply::Ok
                            }
                        }
```

- [ ] **Step 9: 回复写完之后检查退出标记**

找到(`server.rs:804-806`):

```rust
                    },
                };
                w.write_all(encode_line(&reply).as_bytes()).await?;
            }
```

改成:

```rust
                    },
                };
                w.write_all(encode_line(&reply).as_bytes()).await?;
                if should_exit_after_reply {
                    shutdown_signal.notify_one();
                    return Ok(());
                }
            }
```

- [ ] **Step 10: `serve()` 构造 draining/shutdown_signal,accept 循环改成 `select!`**

找到(`server.rs` 约93-136 行,从 `let listener = UnixListener::bind(socket)?;` 到 `serve()` 结尾):

```rust
    let listener = UnixListener::bind(socket)?;
    tracing::info!(socket = %socket.display(), "dozerd 监听中");
    loop {
        let (stream, _) = match listener.accept().await {
            Ok(pair) => pair,
            Err(e) => {
                tracing::warn!(error = %e, "accept 失败，跳过本次连接");
                continue;
            }
        };
        let registry = registry.clone();
        let projects = projects.clone();
        let bookmarks = bookmarks.clone();
        let preview_contexts = preview_contexts.clone();
        let ide_bridge = ide_bridge.clone();
        let transcripts = transcripts.clone();
        let session_summaries = session_summaries.clone();
        let backfill_registry = backfill_registry.clone();
        let todos = todos.clone();
        let categories = categories.clone();
        let in_flight = in_flight.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_conn(
                stream,
                registry,
                projects,
                bookmarks,
                preview_contexts,
                ide_bridge,
                transcripts,
                session_summaries,
                backfill_registry,
                todos.clone(),
                categories.clone(),
                in_flight,
            )
            .await
            {
                tracing::debug!(error = %e, "连接结束");
            }
        });
    }
}
```

改成:

```rust
    let listener = UnixListener::bind(socket)?;
    tracing::info!(socket = %socket.display(), "dozerd 监听中");
    let draining = Arc::new(AtomicBool::new(false));
    let shutdown_signal = Arc::new(Notify::new());
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _) = match accepted {
                    Ok(pair) => pair,
                    Err(e) => {
                        tracing::warn!(error = %e, "accept 失败，跳过本次连接");
                        continue;
                    }
                };
                let registry = registry.clone();
                let projects = projects.clone();
                let bookmarks = bookmarks.clone();
                let preview_contexts = preview_contexts.clone();
                let ide_bridge = ide_bridge.clone();
                let transcripts = transcripts.clone();
                let session_summaries = session_summaries.clone();
                let backfill_registry = backfill_registry.clone();
                let todos = todos.clone();
                let categories = categories.clone();
                let in_flight = in_flight.clone();
                let draining = draining.clone();
                let shutdown_signal = shutdown_signal.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_conn(
                        stream,
                        registry,
                        projects,
                        bookmarks,
                        preview_contexts,
                        ide_bridge,
                        transcripts,
                        session_summaries,
                        backfill_registry,
                        todos.clone(),
                        categories.clone(),
                        in_flight,
                        draining,
                        shutdown_signal,
                    )
                    .await
                    {
                        tracing::debug!(error = %e, "连接结束");
                    }
                });
            }
            _ = shutdown_signal.notified() => {
                tracing::info!("收到 Shutdown 请求收尾完成，dozerd 退出");
                let _ = std::fs::remove_file(socket);
                return Ok(());
            }
        }
    }
}
```

- [ ] **Step 11: `cargo fmt` + 编译整个 workspace**

Run: `cargo fmt -p dozerd && cargo build -p dozerd 2>&1 | tail -40`
Expected: 编译通过,无警告。

- [ ] **Step 12: 跑 dozerd 全部既有单测,确认没有回归**

Run: `cargo test -p dozerd --lib 2>&1 | tail -50`
Expected: 全部 PASS(包括 Step 1/4 新增的两个 `drain_all_sessions` 测试)。

- [ ] **Step 13: Commit**

```bash
git add crates/dozerd/src/server.rs
git commit -m "feat(dozerd): 处理 Request::Shutdown——draining 状态位 + 并发收尾 + 优雅退出"
```

---

### Task 4: dozerd 端到端集成测试(真实 UDS)

**Files:**
- Create: `crates/dozerd/tests/shutdown.rs`

**Interfaces:**
- Consumes: Task 3 落地的 `Request::Shutdown` 完整行为、`dozerd::server::serve` 现有签名(不变)。
- Produces: 无新公开接口,纯测试文件。

- [ ] **Step 1: 写测试文件**

```rust
use dozer_core::protocol::{Reply, Request, decode_line, encode_line};
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

struct Client {
    lines: tokio::io::Lines<BufReader<tokio::net::unix::OwnedReadHalf>>,
    w: tokio::net::unix::OwnedWriteHalf,
}

impl Client {
    async fn connect(sock: &PathBuf) -> Client {
        let stream = UnixStream::connect(sock).await.expect("connect dozerd");
        let (r, w) = stream.into_split();
        Client {
            lines: BufReader::new(r).lines(),
            w,
        }
    }

    async fn send(&mut self, req: &Request) {
        self.w
            .write_all(encode_line(req).as_bytes())
            .await
            .expect("send");
    }

    async fn recv(&mut self) -> Reply {
        let line = tokio::time::timeout(Duration::from_secs(5), self.lines.next_line())
            .await
            .expect("reply within 5s")
            .expect("io ok")
            .expect("stream open");
        decode_line(&line).expect("valid reply")
    }
}

async fn start_test_daemon() -> PathBuf {
    let sock = std::env::temp_dir().join(format!("dozerd-shutdown-test-{}.sock", uuid::Uuid::new_v4()));
    let ide_lock_dir = tempfile::tempdir().expect("ide_lock_dir tempdir");
    let s = sock.clone();
    tokio::spawn(async move {
        dozerd::server::serve(
            &s,
            ide_lock_dir.path().to_path_buf(),
            test_registry(),
            test_projects(),
            test_bookmarks(),
            test_transcripts(),
            test_session_summaries(),
            test_backfill_registry(),
            test_todos(),
            test_categories(),
            dozerd::task_poller::new_in_flight(),
        )
        .await
    });
    for _ in 0..100 {
        if sock.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    sock
}

fn create_long_running_session_req() -> Request {
    Request::CreateSession {
        name: "跑着的会话".into(),
        command: "/bin/sh".into(),
        args: vec!["-c".into(), "sleep 30".into()],
        cwd: std::env::temp_dir().to_string_lossy().into_owned(),
        cols: 80,
        rows: 24,
        project_id: 1,
    }
}

#[tokio::test]
async fn shutdown_with_no_sessions_exits_daemon_promptly() {
    let sock = start_test_daemon().await;
    let mut c = Client::connect(&sock).await;

    let started = std::time::Instant::now();
    c.send(&Request::Shutdown).await;
    assert_eq!(c.recv().await, Reply::Ok);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "0 会话时应该几乎立刻收尾完成，不应该跑满 60s 常量"
    );

    let mut removed = false;
    for _ in 0..100 {
        if !sock.exists() {
            removed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(removed, "daemon 应在 Shutdown 收尾后删除 socket 文件并退出");
}

#[tokio::test]
async fn create_session_rejected_while_draining() {
    let sock = start_test_daemon().await;

    let mut c1 = Client::connect(&sock).await;
    c1.send(&create_long_running_session_req()).await;
    let Reply::Created { session } = c1.recv().await else {
        panic!("expect Created")
    };
    let sid = session.id;

    let mut c_shutdown = Client::connect(&sock).await;
    c_shutdown.send(&Request::Shutdown).await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut c_probe = Client::connect(&sock).await;
    c_probe.send(&create_long_running_session_req()).await;
    let Reply::Error { message } = c_probe.recv().await else {
        panic!("draining 期间 CreateSession 应该被拒绝")
    };
    assert!(message.contains("停止"), "错误文案应说明正在停止: {message}");

    let mut c_finish = Client::connect(&sock).await;
    c_finish
        .send(&Request::RecordSessionSummary {
            session_id: sid,
            title: "标题".into(),
            summary: "摘要".into(),
        })
        .await;
    assert_eq!(c_finish.recv().await, Reply::Ok);

    assert_eq!(c_shutdown.recv().await, Reply::Ok);
}

#[tokio::test]
async fn duplicate_shutdown_rejected_while_draining() {
    let sock = start_test_daemon().await;

    let mut c1 = Client::connect(&sock).await;
    c1.send(&create_long_running_session_req()).await;
    let Reply::Created { session } = c1.recv().await else {
        panic!("expect Created")
    };
    let sid = session.id;

    let mut c_shutdown = Client::connect(&sock).await;
    c_shutdown.send(&Request::Shutdown).await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut c_dup = Client::connect(&sock).await;
    c_dup.send(&Request::Shutdown).await;
    let Reply::Error { message } = c_dup.recv().await else {
        panic!("draining 期间重复 Shutdown 应该被拒绝")
    };
    assert!(message.contains("停止"), "错误文案应说明正在停止: {message}");

    let mut c_finish = Client::connect(&sock).await;
    c_finish
        .send(&Request::RecordSessionSummary {
            session_id: sid,
            title: "标题".into(),
            summary: "摘要".into(),
        })
        .await;
    assert_eq!(c_finish.recv().await, Reply::Ok);

    assert_eq!(c_shutdown.recv().await, Reply::Ok);
}

#[tokio::test]
async fn recorded_summary_is_not_overwritten_by_heuristic_before_shutdown_completes() {
    let sock = start_test_daemon().await;

    let mut c1 = Client::connect(&sock).await;
    c1.send(&create_long_running_session_req()).await;
    let Reply::Created { session } = c1.recv().await else {
        panic!("expect Created")
    };
    let sid = session.id;

    let mut c_shutdown = Client::connect(&sock).await;
    c_shutdown.send(&Request::Shutdown).await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut c_finish = Client::connect(&sock).await;
    c_finish
        .send(&Request::RecordSessionSummary {
            session_id: sid.clone(),
            title: "真实标题".into(),
            summary: "真实摘要".into(),
        })
        .await;
    assert_eq!(c_finish.recv().await, Reply::Ok);

    // 在 daemon 真正退出、socket 消失之前查一次——此时 drain 正在等
    // 2s 轮询间隔醒来，socket 还活着。
    let mut c_check = Client::connect(&sock).await;
    c_check
        .send(&Request::GetSessionSummary {
            session_id: sid.clone(),
        })
        .await;
    let Reply::SessionSummary { summary } = c_check.recv().await else {
        panic!("expect SessionSummary reply")
    };
    let summary = summary.expect("summary should exist");
    assert_eq!(
        summary.status,
        dozer_core::protocol::SummaryStatus::AiGenerated,
        "已经真实提交的总结不应该被超时兜底覆盖"
    );
    assert_eq!(summary.title, "真实标题");

    assert_eq!(c_shutdown.recv().await, Reply::Ok);
}

fn test_registry() -> std::sync::Arc<dozerd::registry::SessionRegistry> {
    std::sync::Arc::new(dozerd::registry::SessionRegistry::new())
}

fn test_projects() -> std::sync::Arc<dozerd::projects::ProjectStore> {
    let db = std::env::temp_dir().join(format!("dozerd-test-{}.db", uuid::Uuid::new_v4()));
    std::sync::Arc::new(dozerd::projects::ProjectStore::new(&db).unwrap())
}

fn test_bookmarks() -> std::sync::Arc<dozerd::bookmarks::BookmarkStore> {
    let db = std::env::temp_dir().join(format!("dozerd-test-{}.db", uuid::Uuid::new_v4()));
    std::sync::Arc::new(dozerd::bookmarks::BookmarkStore::new(&db).unwrap())
}

fn test_transcripts() -> std::sync::Arc<dozerd::transcripts::TranscriptStore> {
    let db = std::env::temp_dir().join(format!("dozerd-test-{}.db", uuid::Uuid::new_v4()));
    std::sync::Arc::new(dozerd::transcripts::TranscriptStore::open(&db).unwrap())
}

fn test_session_summaries() -> std::sync::Arc<dozerd::session_summary::SessionSummaryStore> {
    let db = std::env::temp_dir().join(format!("dozerd-test-{}.db", uuid::Uuid::new_v4()));
    std::sync::Arc::new(dozerd::session_summary::SessionSummaryStore::open(&db).unwrap())
}

fn test_backfill_registry() -> std::sync::Arc<dozerd::session_summary_backfill::BackfillRegistry> {
    std::sync::Arc::new(dozerd::session_summary_backfill::BackfillRegistry::new())
}

fn test_todos() -> std::sync::Arc<dozerd::todo::TodoStore> {
    let db = std::env::temp_dir().join(format!("dozerd-test-{}.db", uuid::Uuid::new_v4()));
    std::sync::Arc::new(dozerd::todo::TodoStore::new(&db).unwrap())
}

fn test_categories() -> std::sync::Arc<dozerd::todo_category::CategoryStore> {
    let db = std::env::temp_dir().join(format!("dozerd-cat-{}.db", uuid::Uuid::new_v4()));
    std::sync::Arc::new(dozerd::todo_category::CategoryStore::new(&db).unwrap())
}
```

- [ ] **Step 2: 运行测试确认通过**

Run: `cargo test -p dozerd --test shutdown 2>&1 | tail -60`
Expected: 4 个测试全部 PASS(后三个各自需要真实等到 2s 轮询间隔醒来,单个测试耗时 2-4s 属预期,不是卡住)。

- [ ] **Step 3: Commit**

```bash
git add crates/dozerd/tests/shutdown.rs
git commit -m "test(dozerd): Request::Shutdown 端到端集成测试"
```

---

### Task 5: `dozer-client::shutdown_daemon()`

**Files:**
- Modify: `crates/dozer-client/src/lib.rs`(顶部 `use`、`Client` impl 块、新增内部单测模块)
- Modify: `crates/dozer-client/tests/against_real_daemon.rs`(新增集成测试)

**Interfaces:**
- Consumes: Task 2 的 `Request::Shutdown`;既有的 `roundtrip`(`lib.rs:38-52`,已经在收到 `Reply::Error` 时把它转成 `Err`)。
- Produces: `pub async fn shutdown_daemon(&self) -> Result<()>`——Task 6(Settings GUI)调用。

- [ ] **Step 1: 写失败的单测(超时/成功/协议错误三种分支的纯函数映射)**

在 `crates/dozer-client/src/lib.rs` 文件末尾新增:

```rust
#[cfg(test)]
mod shutdown_tests {
    use super::*;

    #[test]
    fn interpret_shutdown_reply_ok_maps_to_success() {
        let outcome: std::result::Result<Result<Reply>, tokio::time::error::Elapsed> =
            Ok(Ok(Reply::Ok));
        assert!(interpret_shutdown_reply(outcome).is_ok());
    }

    #[test]
    fn interpret_shutdown_reply_unexpected_reply_is_error() {
        let outcome: std::result::Result<Result<Reply>, tokio::time::error::Elapsed> =
            Ok(Ok(Reply::Sessions { sessions: vec![] }));
        assert!(interpret_shutdown_reply(outcome).is_err());
    }

    #[test]
    fn interpret_shutdown_reply_inner_error_propagates() {
        let outcome: std::result::Result<Result<Reply>, tokio::time::error::Elapsed> =
            Ok(Err(anyhow!("daemon 错误: dozerd 正在停止中")));
        let err = interpret_shutdown_reply(outcome).unwrap_err();
        assert!(err.to_string().contains("正在停止中"));
    }

    #[tokio::test]
    async fn interpret_shutdown_reply_timeout_is_error() {
        let elapsed = tokio::time::timeout(
            std::time::Duration::from_millis(0),
            std::future::pending::<Result<Reply>>(),
        )
        .await
        .unwrap_err();
        let err = interpret_shutdown_reply(Err(elapsed)).unwrap_err();
        assert!(err.to_string().contains("超时"));
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozer-client shutdown_tests 2>&1 | tail -20`
Expected: 编译失败,`cannot find function 'interpret_shutdown_reply' in this scope`。

- [ ] **Step 3: 加 `Duration` 导入 + `interpret_shutdown_reply` + `shutdown_daemon`**

`crates/dozer-client/src/lib.rs` 顶部 `use` 块加一行(找到 `use std::path::PathBuf;`,改成):

```rust
use std::path::PathBuf;
use std::time::Duration;
```

在 `Client` 的 `impl` 块内、`kill`/`close_with_summary` 附近(比如紧跟 `close_with_summary` 之后)新增方法:

```rust
    /// 请求 dozerd 对所有存活会话收尾后退出。内部等待时长与 daemon 侧
    /// 收尾耗时挂钩(最长约 60s),外层包一个 90s 超时兜底纯通信层面的
    /// 异常(进程卡死、socket 异常等),超时视为失败,不代表 daemon 一定
    /// 没停。
    pub async fn shutdown_daemon(&self) -> Result<()> {
        let outcome =
            tokio::time::timeout(Duration::from_secs(90), self.roundtrip(&Request::Shutdown))
                .await;
        interpret_shutdown_reply(outcome)
    }
```

在 `impl Client` 块外(文件顶层任意位置,建议紧跟 `impl Client` 块之后)新增纯函数:

```rust
/// `shutdown_daemon()` 的超时/协议错误/成功三分支映射,拆成纯函数是为了
/// 不用真的等 90s 或起一个假 UDS server 就能测到每条分支(`Client`::
/// `shutdown_daemon` 本身只做一次 `tokio::time::timeout` 包裹,逻辑全在
/// 这里)。
fn interpret_shutdown_reply(
    outcome: std::result::Result<Result<Reply>, tokio::time::error::Elapsed>,
) -> Result<()> {
    match outcome {
        Err(_) => Err(anyhow!("等待 dozerd 停止超时")),
        Ok(Err(e)) => Err(e),
        Ok(Ok(Reply::Ok)) => Ok(()),
        Ok(Ok(other)) => Err(anyhow!("意外应答: {other:?}")),
    }
}
```

- [ ] **Step 4: 运行单测确认通过**

Run: `cargo test -p dozer-client shutdown_tests 2>&1 | tail -30`
Expected: 4 个测试 PASS。

- [ ] **Step 5: 加真实daemon 的集成测试(成功路径)**

在 `crates/dozer-client/tests/against_real_daemon.rs` 末尾新增:

```rust
#[tokio::test]
async fn shutdown_daemon_with_no_sessions_succeeds() {
    let (sock, _registry, _guard) = start_daemon().await;
    let client = Client::new(sock.clone());
    assert!(client.list().await.unwrap().is_empty());

    client.shutdown_daemon().await.unwrap();

    let mut removed = false;
    for _ in 0..100 {
        if !sock.exists() {
            removed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(removed, "daemon 应在 shutdown_daemon() 成功后退出并清理 socket");
}
```

- [ ] **Step 6: 运行测试确认通过**

Run: `cargo test -p dozer-client --test against_real_daemon shutdown_daemon 2>&1 | tail -30`
Expected: PASS。

- [ ] **Step 7: 跑 `dozer-client` 全部测试确认无回归**

Run: `cargo test -p dozer-client 2>&1 | tail -60`
Expected: 全部 PASS。

- [ ] **Step 8: Commit**

```bash
git add crates/dozer-client/src/lib.rs crates/dozer-client/tests/against_real_daemon.rs
git commit -m "feat(dozer-client): 新增 shutdown_daemon()"
```

---

### Task 6: Settings 弹窗"高级"区块——停止/重新启动 dozerd 状态机

**Files:**
- Modify: `crates/dozer-app/src/extensions/settings.rs`

**Interfaces:**
- Consumes: Task 5 的 `client.shutdown_daemon()`;既有的 `crate::runtime::ensure_daemon(&dozer_client::Client) -> Result<(), String>`(`runtime.rs:34-54`,连不上时自动 `spawn_dozerd()` 并重试 3 次);既有的 `dozer_client::Client::list() -> Result<Vec<SessionInfo>>`。
- Produces: `settings::Message` 新增 7 个变体(见下),`settings::State` 新增 `advanced: AdvancedState` 字段,`settings::update` 签名新增 `client: &dozer_client::Client` 参数(供 Task 7 的 `app/update.rs` 调用点更新)。

- [ ] **Step 1: 写状态转换的失败测试**

在 `crates/dozer-app/src/extensions/settings.rs` 的 `#[cfg(test)] mod tests` 块内,更新 `test_state` 辅助函数并新增测试。找到:

```rust
    fn test_state(github: ConnectState, gitlab: ConnectState, gitee: ConnectState) -> State {
        State {
            github,
            gitlab,
            gitee,
            suppress_next_blur: false,
            connect_tasks: HashMap::new(),
        }
    }
```

改成:

```rust
    fn test_state(github: ConnectState, gitlab: ConnectState, gitee: ConnectState) -> State {
        State {
            github,
            gitlab,
            gitee,
            suppress_next_blur: false,
            connect_tasks: HashMap::new(),
            advanced: AdvancedState::Idle { error: None },
        }
    }
```

在文件末尾(`close_is_not_a_sync_message` 测试之后)新增:

```rust
    #[test]
    fn advanced_stop_cancel_returns_to_idle() {
        let mut state = test_state(
            ConnectState::NotConnected,
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        state.advanced = AdvancedState::ConfirmingStop { session_count: 2 };
        apply_sync_message(&mut state, &Message::AdvancedStopCancel);
        assert_eq!(state.advanced, AdvancedState::Idle { error: None });
    }

    #[test]
    fn advanced_session_count_ready_ok_opens_confirm() {
        let mut state = test_state(
            ConnectState::NotConnected,
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        state.advanced = AdvancedState::FetchingCount;
        apply_sync_message(&mut state, &Message::AdvancedSessionCountReady(Ok(3)));
        assert_eq!(
            state.advanced,
            AdvancedState::ConfirmingStop { session_count: 3 }
        );
    }

    #[test]
    fn advanced_session_count_ready_err_returns_to_idle_with_error() {
        let mut state = test_state(
            ConnectState::NotConnected,
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        state.advanced = AdvancedState::FetchingCount;
        apply_sync_message(
            &mut state,
            &Message::AdvancedSessionCountReady(Err("daemon 断开".into())),
        );
        assert_eq!(
            state.advanced,
            AdvancedState::Idle {
                error: Some("daemon 断开".into())
            }
        );
    }

    #[test]
    fn advanced_stop_result_ok_marks_stopped() {
        let mut state = test_state(
            ConnectState::NotConnected,
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        state.advanced = AdvancedState::Stopping;
        apply_sync_message(&mut state, &Message::AdvancedStopResult(Ok(())));
        assert_eq!(state.advanced, AdvancedState::Stopped { error: None });
    }

    #[test]
    fn advanced_stop_result_err_returns_to_idle_with_error() {
        let mut state = test_state(
            ConnectState::NotConnected,
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        state.advanced = AdvancedState::Stopping;
        apply_sync_message(
            &mut state,
            &Message::AdvancedStopResult(Err("等待 dozerd 停止超时".into())),
        );
        assert_eq!(
            state.advanced,
            AdvancedState::Idle {
                error: Some("等待 dozerd 停止超时".into())
            }
        );
    }

    #[test]
    fn advanced_restart_result_ok_returns_to_idle() {
        let mut state = test_state(
            ConnectState::NotConnected,
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        state.advanced = AdvancedState::RestartingDozerd;
        apply_sync_message(&mut state, &Message::AdvancedRestartResult(Ok(())));
        assert_eq!(state.advanced, AdvancedState::Idle { error: None });
    }

    #[test]
    fn advanced_restart_result_err_stays_stopped_with_error() {
        let mut state = test_state(
            ConnectState::NotConnected,
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        state.advanced = AdvancedState::RestartingDozerd;
        apply_sync_message(
            &mut state,
            &Message::AdvancedRestartResult(Err("无法连接 dozerd".into())),
        );
        assert_eq!(
            state.advanced,
            AdvancedState::Stopped {
                error: Some("无法连接 dozerd".into())
            }
        );
    }

    #[test]
    fn advanced_async_messages_are_not_sync_messages() {
        let mut state = test_state(
            ConnectState::NotConnected,
            ConnectState::NotConnected,
            ConnectState::NotConnected,
        );
        assert!(!apply_sync_message(&mut state, &Message::AdvancedStopClicked));
        assert!(!apply_sync_message(&mut state, &Message::AdvancedStopConfirm));
        assert!(!apply_sync_message(
            &mut state,
            &Message::AdvancedRestartClicked
        ));
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozer-app --lib settings:: 2>&1 | tail -30`
Expected: 编译失败,`AdvancedState`/新 `Message` 变体不存在。

- [ ] **Step 3: 新增 `AdvancedState` 类型 + `State` 字段**

找到:

```rust
pub struct State {
    pub github: ConnectState,
    pub gitlab: ConnectState,
    pub gitee: ConnectState,
    /// `OpenTokenPage` 拉起系统浏览器时置位——那会让本窗口收到一次真实
    /// `Focused(false)`,若照常触发失焦即关闭会把正在填的 PAT 表单整个
    /// 关掉(代码评审 finding:PAT-link click can auto-close Settings)。
    /// `SettingsOverlay::handle_focus` 读到就消费掉、吞掉这一次失焦。
    pub(crate) suppress_next_blur: bool,
    /// 正在进行的 `ConnectSubmit` 异步任务句柄,按 provider 存一份。
    /// `ConnectCancel` 用它真正中止任务,防止取消后 token 仍被异步写进
    /// Keychain/本地文件(代码评审 finding:Cancel doesn't stop in-flight
    /// connect task)。
    connect_tasks: HashMap<GitProvider, tokio::task::AbortHandle>,
}
```

改成(新增字段 + 上方新增 `AdvancedState` 类型定义):

```rust
/// "高级"区块(停止/重新启动 dozerd)的状态机。`Idle`/`Stopped` 各自内嵌
/// `error: Option<String>`——失败态就是"回到默认态/已停止态,附一行错误
/// 文案",不单独建一个 `Failed` 变体(spec「UI 设计」:停止失败按钮恢复
/// 默认态、重启失败保留"重新启动"按钮,两者语义上就是各自基础态的一个
/// 变体,不是第三种独立状态)。
#[derive(Debug, Clone, PartialEq)]
pub enum AdvancedState {
    Idle { error: Option<String> },
    FetchingCount,
    ConfirmingStop { session_count: u32 },
    Stopping,
    Stopped { error: Option<String> },
    RestartingDozerd,
}

pub struct State {
    pub github: ConnectState,
    pub gitlab: ConnectState,
    pub gitee: ConnectState,
    /// `OpenTokenPage` 拉起系统浏览器时置位——那会让本窗口收到一次真实
    /// `Focused(false)`,若照常触发失焦即关闭会把正在填的 PAT 表单整个
    /// 关掉(代码评审 finding:PAT-link click can auto-close Settings)。
    /// `SettingsOverlay::handle_focus` 读到就消费掉、吞掉这一次失焦。
    pub(crate) suppress_next_blur: bool,
    /// 正在进行的 `ConnectSubmit` 异步任务句柄,按 provider 存一份。
    /// `ConnectCancel` 用它真正中止任务,防止取消后 token 仍被异步写进
    /// Keychain/本地文件(代码评审 finding:Cancel doesn't stop in-flight
    /// connect task)。
    connect_tasks: HashMap<GitProvider, tokio::task::AbortHandle>,
    pub advanced: AdvancedState,
}
```

`State::load()` 里(找到 `connect_tasks: HashMap::new(),` 那一行,在 `State::load()` 函数体内)加一行:

```rust
            connect_tasks: HashMap::new(),
            advanced: AdvancedState::Idle { error: None },
        }
    }
```

- [ ] **Step 4: `Message` 枚举新增 7 个变体**

找到:

```rust
#[derive(Debug, Clone)]
pub enum Message {
    Close,
    ThemeSelected(ColorScheme),
    ConnectClicked(GitProvider),
    TokenChanged(GitProvider, String),
    ConnectCancel(GitProvider),
    OpenTokenPage(GitProvider),
    ConnectSubmit(GitProvider),
    ConnectResult(GitProvider, Result<String, String>),
    Disconnect(GitProvider),
}
```

改成:

```rust
#[derive(Debug, Clone)]
pub enum Message {
    Close,
    ThemeSelected(ColorScheme),
    ConnectClicked(GitProvider),
    TokenChanged(GitProvider, String),
    ConnectCancel(GitProvider),
    OpenTokenPage(GitProvider),
    ConnectSubmit(GitProvider),
    ConnectResult(GitProvider, Result<String, String>),
    Disconnect(GitProvider),
    AdvancedStopClicked,
    AdvancedSessionCountReady(Result<u32, String>),
    AdvancedStopCancel,
    AdvancedStopConfirm,
    AdvancedStopResult(Result<(), String>),
    AdvancedRestartClicked,
    AdvancedRestartResult(Result<(), String>),
}
```

- [ ] **Step 5: `apply_sync_message` 加纯状态转换分支**

找到 `apply_sync_message` 函数末尾:

```rust
        Message::Close | Message::ConnectSubmit(_) => false,
    }
}
```

改成(在这之前插入新分支,`Close`/`ConnectSubmit` 那行加上三个新的异步消息一起归为 `false`):

```rust
        Message::AdvancedStopCancel => {
            state.advanced = AdvancedState::Idle { error: None };
            true
        }
        Message::AdvancedSessionCountReady(result) => {
            state.advanced = match result {
                Ok(n) => AdvancedState::ConfirmingStop { session_count: *n },
                Err(e) => AdvancedState::Idle {
                    error: Some(e.clone()),
                },
            };
            true
        }
        Message::AdvancedStopResult(result) => {
            state.advanced = match result {
                Ok(()) => AdvancedState::Stopped { error: None },
                Err(e) => AdvancedState::Idle {
                    error: Some(e.clone()),
                },
            };
            true
        }
        Message::AdvancedRestartResult(result) => {
            state.advanced = match result {
                Ok(()) => AdvancedState::Idle { error: None },
                Err(e) => AdvancedState::Stopped {
                    error: Some(e.clone()),
                },
            };
            true
        }
        Message::Close
        | Message::ConnectSubmit(_)
        | Message::AdvancedStopClicked
        | Message::AdvancedStopConfirm
        | Message::AdvancedRestartClicked => false,
    }
}
```

- [ ] **Step 6: 运行测试确认通过**

Run: `cargo test -p dozer-app --lib settings:: 2>&1 | tail -40`
Expected: 新增的 8 个测试全部 PASS(既有 Git 账户测试不受影响)。

- [ ] **Step 7: `update()` 签名加 `client` 参数 + 三个异步分支**

找到:

```rust
pub fn update(
    state: &mut Option<State>,
    msg: Message,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    if let Message::Close = msg {
        *state = None;
        return;
    }
    let Some(s) = state else { return };
    if apply_sync_message(s, &msg) {
        return;
    }
    let Message::ConnectSubmit(provider) = msg else {
        unreachable!("已在 apply_sync_message 或顶部处理");
    };
    let token = match s.slot_mut(provider) {
```

改成:

```rust
pub fn update(
    state: &mut Option<State>,
    msg: Message,
    client: &dozer_client::Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    if let Message::Close = msg {
        *state = None;
        return;
    }
    let Some(s) = state else { return };
    if apply_sync_message(s, &msg) {
        return;
    }
    let provider = match msg {
        Message::ConnectSubmit(provider) => provider,
        Message::AdvancedStopClicked => {
            if !matches!(s.advanced, AdvancedState::Idle { .. }) {
                return;
            }
            s.advanced = AdvancedState::FetchingCount;
            let client = client.clone();
            handle.spawn(async move {
                let result = client
                    .list()
                    .await
                    .map(|sessions| sessions.len() as u32)
                    .map_err(|e| e.to_string());
                emit(Message::AdvancedSessionCountReady(result));
            });
            return;
        }
        Message::AdvancedStopConfirm => {
            if !matches!(s.advanced, AdvancedState::ConfirmingStop { .. }) {
                return;
            }
            s.advanced = AdvancedState::Stopping;
            let client = client.clone();
            handle.spawn(async move {
                let result = client.shutdown_daemon().await.map_err(|e| e.to_string());
                emit(Message::AdvancedStopResult(result));
            });
            return;
        }
        Message::AdvancedRestartClicked => {
            if !matches!(s.advanced, AdvancedState::Stopped { .. }) {
                return;
            }
            s.advanced = AdvancedState::RestartingDozerd;
            let client = client.clone();
            handle.spawn(async move {
                let result = crate::runtime::ensure_daemon(&client).await;
                emit(Message::AdvancedRestartResult(result));
            });
            return;
        }
        _ => unreachable!("已在 apply_sync_message 或上面处理"),
    };
    let token = match s.slot_mut(provider) {
```

- [ ] **Step 8: 新增 `advanced_row` 视图函数**

在 `provider_row` 函数之后、`settings_card` 函数之前新增:

```rust
fn advanced_row(state: &AdvancedState) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let colors = byteui::theme::color::current();
    let hint = |s: String| {
        text(s)
            .size(byteui::theme::font::label())
            .color(colors.dim)
    };
    let error_line = |e: &Option<String>| -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
        match e {
            Some(msg) => text(msg.clone())
                .size(byteui::theme::font::label())
                .color(colors.red)
                .into(),
            None => Space::new().into(),
        }
    };
    match state {
        AdvancedState::Idle { error } => {
            let btn = button(text("停止 dozerd").size(byteui::theme::font::body()))
                .on_press(Message::AdvancedStopClicked)
                .padding([6, 14]);
            column![
                hint("停止后所有正在运行的 agent 会话会结束并生成总结,可随时重新启动。".into()),
                row![Space::new().width(Length::Fill), btn],
                error_line(error),
            ]
            .spacing(6)
            .into()
        }
        AdvancedState::FetchingCount => {
            let btn = button(text("检查中…").size(byteui::theme::font::body())).padding([6, 14]);
            row![
                hint("停止后所有正在运行的 agent 会话会结束并生成总结,可随时重新启动。".into()),
                Space::new().width(Length::Fill),
                btn
            ]
            .spacing(6)
            .align_y(Alignment::Center)
            .into()
        }
        AdvancedState::ConfirmingStop { session_count } => {
            let body = if *session_count == 0 {
                "当前没有正在运行的会话,dozerd 会直接停止。".to_string()
            } else {
                format!(
                    "这会结束当前 {session_count} 个正在运行的 agent 会话并生成总结(可能需要约 1 分钟)。"
                )
            };
            let cancel = button(text("取消").size(byteui::theme::font::body()))
                .on_press(Message::AdvancedStopCancel)
                .padding([6, 14]);
            let confirm = button(
                text("确认停止")
                    .size(byteui::theme::font::body())
                    .color(colors.red),
            )
            .on_press(Message::AdvancedStopConfirm)
            .padding([6, 14]);
            column![
                hint(body),
                row![Space::new().width(Length::Fill), cancel, confirm].spacing(8),
            ]
            .spacing(6)
            .into()
        }
        AdvancedState::Stopping => {
            let btn = button(text("停止中…(等待会话总结,最长约 1 分钟)").size(byteui::theme::font::body()))
                .padding([6, 14]);
            row![Space::new().width(Length::Fill), btn].into()
        }
        AdvancedState::Stopped { error } => {
            let btn = button(text("重新启动 dozerd").size(byteui::theme::font::body()))
                .on_press(Message::AdvancedRestartClicked)
                .padding([6, 14]);
            column![
                hint("dozerd 已停止,部分功能不可用。".into()),
                row![Space::new().width(Length::Fill), btn],
                error_line(error),
            ]
            .spacing(6)
            .into()
        }
        AdvancedState::RestartingDozerd => {
            let btn = button(text("启动中…").size(byteui::theme::font::body())).padding([6, 14]);
            row![Space::new().width(Length::Fill), btn].into()
        }
    }
}
```

- [ ] **Step 9: `settings_card` 接入"高级"区块**

找到:

```rust
    let content = column![
        theme_title,
        scheme_row("深色 · ByteBoy2077", ColorScheme::Dark, current),
        scheme_row("浅色 · ByteBoy2077-Light", ColorScheme::Light, current),
        git_title,
        provider_row(GitProvider::GitHub, &state.github),
        provider_row(GitProvider::GitLab, &state.gitlab),
        provider_row(GitProvider::Gitee, &state.gitee),
        crate::dialog::actions(row![close]),
    ]
    .spacing(14);
```

改成(新增标题 + `advanced_row`):

```rust
    let advanced_title = text("高级")
        .size(byteui::theme::font::subtitle())
        .color(colors.cream);
    let content = column![
        theme_title,
        scheme_row("深色 · ByteBoy2077", ColorScheme::Dark, current),
        scheme_row("浅色 · ByteBoy2077-Light", ColorScheme::Light, current),
        git_title,
        provider_row(GitProvider::GitHub, &state.github),
        provider_row(GitProvider::GitLab, &state.gitlab),
        provider_row(GitProvider::Gitee, &state.gitee),
        advanced_title,
        advanced_row(&state.advanced),
        crate::dialog::actions(row![close]),
    ]
    .spacing(14);
```

- [ ] **Step 10: 编译 + 跑 `dozer-app` 全部测试**

Run: `cargo build -p dozer-app 2>&1 | tail -60`
Expected: 编译失败(因为 `app/update.rs` 的调用点还没更新签名),先看清报错在 Task 7 修。

Run: `cargo test -p dozer-app --lib settings:: 2>&1 | tail -60`
Expected: 这一步应该已经能跑(测试不经过 `update()`,只用 `apply_sync_message`),全部 PASS。

- [ ] **Step 11: Commit**

```bash
git add crates/dozer-app/src/extensions/settings.rs
git commit -m "feat(app): Settings 弹窗新增高级区块——停止/重新启动 dozerd 状态机"
```

(此时 `cargo build -p dozer-app` 仍会因 Task 7 未完成而失败,属预期,Task 7 会修复。)

---

### Task 7: App 级接线——`client` 参数穿透 + `daemon_error` 提示

**Files:**
- Modify: `crates/dozer-app/src/app/update.rs:1819-1835`

**Interfaces:**
- Consumes: Task 6 的 `settings::update(state, msg, client, handle, emit)` 新签名;既有的 `App.client: dozer_client::Client`(`Clone`)、`App.daemon_error: Option<String>`(`app/app.rs:266`)。
- Produces: 无新公开接口,收尾 App 与 Settings 模块的接线。

- [ ] **Step 1: 更新调用点**

找到:

```rust
            Message::Settings(msg) => {
                // 主题切换要在设置 update(它会 set_scheme 改全局配色)之后,把
                // 所有已开预览 webview 按新主题重载——flyfish 的 theme 是 URL
                // 参数(见 preview::flyfish_url),不重载不会跟着变。两个预览
                // 面板(Files/Project)各推进各自 wry tab 的 reload_nonce。
                let theme_changed = matches!(msg, settings::Message::ThemeSelected(_));
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let emit = move |m| {
                    let _ = proxy.send_event(Message::Settings(m));
                };
                settings::update(&mut self.settings, msg, &handle, emit);
                if theme_changed && let Some(ws) = self.active_workspace_mut() {
                    ws.preview.reload_all_webviews_for_theme();
                    ws.project_preview.reload_all_webviews_for_theme();
                }
            }
```

改成:

```rust
            Message::Settings(msg) => {
                // 主题切换要在设置 update(它会 set_scheme 改全局配色)之后,把
                // 所有已开预览 webview 按新主题重载——flyfish 的 theme 是 URL
                // 参数(见 preview::flyfish_url),不重载不会跟着变。两个预览
                // 面板(Files/Project)各推进各自 wry tab 的 reload_nonce。
                let theme_changed = matches!(msg, settings::Message::ThemeSelected(_));
                // 停止/重新启动 dozerd 的结果要顺带更新 `daemon_error`——
                // 这是"daemon 连不上"的整程序共享状态(`app/view.rs`/
                // `term/terminal.rs` 已经在读),不新建 UI 组件(spec「复用
                // App.daemon_error」)。跟 `theme_changed` 一样,要在
                // `msg` 被 move 进 `settings::update` 之前取值。
                let stop_succeeded = matches!(msg, settings::Message::AdvancedStopResult(Ok(())));
                let restart_succeeded =
                    matches!(msg, settings::Message::AdvancedRestartResult(Ok(())));
                let client = self.client.clone();
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let emit = move |m| {
                    let _ = proxy.send_event(Message::Settings(m));
                };
                settings::update(&mut self.settings, msg, &client, &handle, emit);
                if stop_succeeded {
                    self.daemon_error = Some("dozerd 已停止,部分功能不可用".to_string());
                }
                if restart_succeeded {
                    self.daemon_error = None;
                }
                if theme_changed && let Some(ws) = self.active_workspace_mut() {
                    ws.preview.reload_all_webviews_for_theme();
                    ws.project_preview.reload_all_webviews_for_theme();
                }
            }
```

- [ ] **Step 2: 编译整个 workspace**

Run: `cargo build 2>&1 | tail -60`
Expected: 编译通过,无错误无警告。

- [ ] **Step 3: 跑 `dozer-app` 全部单测**

Run: `cargo test -p dozer-app --lib 2>&1 | tail -80`
Expected: 全部 PASS,无回归。

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/src/app/update.rs
git commit -m "feat(app): Settings 停止/重启 dozerd 接线 daemon_error 提示"
```

---

### Task 8: 全工作区验证 + 人工验收

**Files:** 无代码改动,纯验证步骤。

- [ ] **Step 1: 全量 build/test/clippy/fmt**

```bash
cargo build 2>&1 | tail -60
cargo test --workspace 2>&1 | tail -150
cargo clippy --all-targets 2>&1 | tail -80
cargo fmt --check 2>&1 | tail -40
```

Expected: 全部通过;`cargo fmt --check` 若有差异,跑 `cargo fmt` 后重新 `git add`/`git commit`(单独一个 `chore: cargo fmt` commit)。

- [ ] **Step 2: 跑一遍真实 GUI 人工验收清单**

按 spec `docs/superpowers/specs/2026-09-19-dozerd-graceful-shutdown-design.md`「测试策略」一节的人工验收清单逐条过:

```bash
cargo run -p dozer-app
```

1. 开 2-3 个不同 agent 的会话,进 Settings 弹窗「高级」区块点"停止 dozerd",确认二次确认文案里的会话数正确,确认后等待期间其它面板仍可正常浏览(只是新建会话被挡),完成后检查每个会话确实生成了总结记录(Todo/会话面板可查)。
2. 停止完成后确认 `daemon_error` 提示出现(终端面板顶部警示条,或没有打开项目时的空态提示),依赖 dozerd 的面板显示预期的"连接失败"状态。
3. 点"重新启动 dozerd",确认真正拉起且提示消失。
4. Quit Dozer 菜单退出 GUI 前后,用 `ps aux | grep dozerd` 确认 `dozerd` 进程原样存活,未被本次改动意外影响。

- [ ] **Step 3: 提请代码审阅**

按 Global Constraints 里的分支要求,推送 `feature/dozerd-graceful-shutdown` 分支,提请审阅;审阅通过后再合并回 main,不在过程中合并中间状态。
