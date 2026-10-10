# Conversations 面板迁移到服务 trait Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 会话列表面板(`extensions/conversations.rs`)的 1 个服务调用改为经 `dozer-services` 的 `ConversationService`;刷新失败不再把已显示的会话列表清空,并按 Todo/Project 的口径分档提示。

**Architecture:** `dozer-services` 新增 `ConversationService`(1 个方法 + 返回值别名 `ConversationRows`)与 `Services.conversation`;`Client` 实现它(复用 `into_service_error`);`Message::SessionsRefreshed` 的结果类型换成 `Result<Vec<SessionRow>, ServiceError>`(不新增变体、不新增 host 臂);面板 `update` 的失败分支:首次失败置空列表、已有列表保留,`Unavailable` 静默,其余弹带 key 的 Error Toast(同文本 60 秒内一次)。

**Tech Stack:** Rust 2024 workspace、tokio、iced 0.14(只涉及面板 update/spawn 层)、Python 门禁脚本。

**Spec:** `docs/superpowers/specs/2026-10-11-conversation-services-design.md`(先例:`2026-10-10-todo-services-design.md`、`2026-10-10-project-services-design.md`、`2026-10-11-groupchat-services-design.md`)

## Global Constraints

- `dozer-services` 只依赖 `dozer-model`;不加 `dozer-core`/`dozer-client`/`tokio` 到 `[dependencies]`。
- 线上协议零变化:`Request`/`Reply` 不动,`crates/dozer-core/tests/model_golden.rs` 不改且通过。
- 面板日志来源名必须是 `conversations`(已声明的 `LOG`);不新增裸 `tracing::`/`eprintln!`(`scripts/check-log-scope.sh`)。
- 瞬时失败走 Toast(`outbox`);`R-HOST-PANEL-ARMS` 不上升(不新增 `app/update.rs` 的面板专属臂)。
- 不迁 host 侧的会话打开/翻页/总结任务/homespace/hook 调用;**唯一例外**见 Task 4 的 Ruling(`conversation_summary_generate` 里构造 `SessionsRefreshed` 的那一处调用必须随消息类型一起改)。
- 在独立 worktree 实施;不提交用户 WIP(`Info.plist`、版本号相关改动);`Cargo.lock` 提交前剔除 bytegit/byteui 的 `source =` 行(若 `git status` 显示 `Cargo.lock` 只有这类噪声,直接 `git checkout Cargo.lock`)。
- main 上已知问题不修不算回归:`files::tests::delete_confirm_spec_reflects_pending_target` 失败、`git_log::tests::build_*` 间歇失败、门禁 `R-HOST-EXECUTORS 0 -> 1`。
- 提交信息末尾加 `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`。

## Review Focus

- **已有会话列表时刷新失败不得清空**(此前 `Err` 一律置空);**首次加载失败仍得到 `Some(空)`**,不得停在 `None`(否则界面一直"加载中")→ Task 4 测试。
- `Unavailable` 不弹 Toast;`Rejected`/`Protocol` 恰好一条带 key `conversations-load`,60 秒内同文本不重复、换文本再弹 → Task 4 测试。
- `spawn_refresh` 把 `cwd`、`agent: None`、`limit: 500`、`offset: 0` 原样传给服务 → Task 4 测试。
- `conversation_summary_generate`(host,约 `app/update.rs` 6245 行)里的"总结已生成，但刷新会话失败：{e}"文本在换成 `ServiceError` 后仍逐字相同(`Display` = message)→ Task 4 手动核对 + 现有测试。
- `impl ConversationService for Client` 必须调 inherent 方法而不是递归调 trait 方法;元组向量要原样带回 → Task 2 适配测试。

---

### Task 1: `ConversationService` trait(`dozer-services`)

**Files:**
- Modify: `crates/dozer-services/src/lib.rs`

**Interfaces:**
- Consumes: `dozer_model::{AgentKind, ConversationSummary, SessionSummaryPayload}`。
- Produces:
  - `pub type ConversationRows = Vec<(ConversationSummary, Option<SessionSummaryPayload>)>;`
  - `pub trait ConversationService: Send + Sync { fn list_conversations_with_summaries(&self, cwd: String, agent: Option<AgentKind>, limit: u32, offset: u32) -> BoxFuture<'_, Result<ConversationRows, ServiceError>>; }`

- [ ] **Step 0: 记录基线(只测量,写进 ledger;同步执行,跑完再进 Step 1,避免并发编译污染)**

```bash
W=$(ls -d .superpowers/sdd/*/ | head -1)
grep -c 'dozer_client\|io\.client()' crates/dozer-app/src/extensions/conversations.rs     # 期望 2
cargo test --workspace --no-fail-fast > ${W}baseline-tests.txt 2>&1; grep -E '^test result' ${W}baseline-tests.txt | awk '{s+=$4;f+=$6} END{print "passed",s,"failed",f}'; grep -E '^test .* FAILED' ${W}baseline-tests.txt
cargo clippy --all-targets 2>&1 | grep -c '^warning'
python3 scripts/audit/check_panel_boundary.py; echo exit=$?
```
Expected: 失败仅已知的;门禁仅 `R-HOST-EXECUTORS 0 -> 1`;把数字写进 ledger。

- [ ] **Step 1: 写失败测试**

在 `crates/dozer-services/src/lib.rs` 现有 `#[cfg(test)] mod tests` 末尾追加(`unit()` 等辅助已存在;`AgentKind` 已 import):

```rust
    // ---- ConversationService 契约 ----

    struct MemConversation(Result<ConversationRows, ServiceError>);

    impl ConversationService for MemConversation {
        fn list_conversations_with_summaries(
            &self,
            _cwd: String,
            _agent: Option<AgentKind>,
            _limit: u32,
            _offset: u32,
        ) -> BoxFuture<'_, Result<ConversationRows, ServiceError>> {
            let r = self.0.clone();
            Box::pin(async move { r })
        }
    }

    /// `handle.spawn` 要 `'static + Send`:持有 `Arc<dyn ConversationService>` 的 async 块必须满足。
    #[test]
    fn dyn_conversation_future_runs_inside_tokio_spawn() {
        let svc: Arc<dyn ConversationService> = Arc::new(MemConversation(Ok(vec![])));
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let out = rt.block_on(async move {
            tokio::spawn(async move {
                svc.list_conversations_with_summaries("/p".into(), None, 500, 0).await
            })
            .await
            .unwrap()
        });
        assert_eq!(out, Ok(vec![]));
    }

    #[test]
    fn conversation_read_failure_is_distinguishable_from_an_empty_list() {
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let empty = MemConversation(Ok(vec![]));
        let down = MemConversation(Err(ServiceError::new(ServiceErrorKind::Unavailable, "x")));
        assert!(rt.block_on(empty.list_conversations_with_summaries("/p".into(), None, 1, 0)).is_ok());
        assert_eq!(
            rt.block_on(down.list_conversations_with_summaries("/p".into(), None, 1, 0))
                .unwrap_err()
                .kind,
            ServiceErrorKind::Unavailable
        );
    }
```
Run: `cargo test -p dozer-services 2>&1 | tail -8` → Expected: 编译失败 `cannot find trait ConversationService`(RED)。

- [ ] **Step 2: 实现**

顶部 `use dozer_model::{...}` 合并追加 `ConversationSummary`(已在)、`SessionSummaryPayload`(若未在则加)。在 `GroupChatService` 之后、`Services` 之前加:

```rust
/// `list_conversations_with_summaries` 的结果:每行一个会话及其可选的总结。
pub type ConversationRows = Vec<(ConversationSummary, Option<SessionSummaryPayload>)>;

/// 会话列表面板的读取。参数按值传。目前只有这一个方法;host 侧的会话相关调用随各自迁移再加入。
pub trait ConversationService: Send + Sync {
    fn list_conversations_with_summaries(
        &self,
        cwd: String,
        agent: Option<AgentKind>,
        limit: u32,
        offset: u32,
    ) -> BoxFuture<'_, Result<ConversationRows, ServiceError>>;
}
```
**不要**在本任务给 `Services` 加字段(Task 3 做,否则 `dozer-app` 编译失败)。

Run: `cargo test -p dozer-services 2>&1 | grep -E 'test result|error'` → Expected: 全绿(原 10 个 + 新 2 个 = 12 passed)。

- [ ] **Step 3: 依赖验收与提交**

```bash
cargo tree -p dozer-services --edges normal | grep -vE 'dozer-model|serde|proc-macro2|quote|syn|unicode-ident|dozer-services' | head   # 期望无输出
cargo fmt -p dozer-services && cargo clippy -p dozer-services --all-targets 2>&1 | grep -c '^warning'   # 期望 0
git add crates/dozer-services && git commit -m "feat(services): Conversation S1 ConversationService trait

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 2: `Client` 实现 `ConversationService`

**Files:**
- Modify: `crates/dozer-client/src/lib.rs`(impl + 测试模块 `conversation_service_tests`)

**Interfaces:**
- Consumes: Task 1;`into_service_error`、`BoxFuture`、`ServiceError`;`todo_service_tests::serve_once`(已 `pub(super)`)。
- Produces: `impl ConversationService for Client`(同名 inherent 方法不改)。

- [ ] **Step 1: 写失败测试**

在文件末尾追加:

```rust
#[cfg(test)]
mod conversation_service_tests {
    use super::todo_service_tests::serve_once;
    use super::*;
    use dozer_services::{ConversationService, ServiceErrorKind};

    #[tokio::test]
    async fn rows_come_back_as_ok() {
        let sock = serve_once("conv-ok", Some(Reply::ConversationsWithSummaries { rows: vec![] }));
        let got = ConversationService::list_conversations_with_summaries(
            &Client::new(sock.clone()), "/p".into(), None, 500, 0,
        )
        .await;
        assert_eq!(got, Ok(vec![]));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn hang_up_is_unavailable_with_the_legacy_text() {
        let sock = serve_once("conv-hangup", None);
        let e = ConversationService::list_conversations_with_summaries(
            &Client::new(sock.clone()), "/p".into(), None, 500, 0,
        )
        .await
        .unwrap_err();
        assert_eq!((e.kind, e.message.as_str()), (ServiceErrorKind::Unavailable, "daemon 断开"));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn error_reply_is_rejected_with_the_legacy_prefix() {
        let sock = serve_once("conv-reject", Some(Reply::Error { message: "boom".into() }));
        let e = ConversationService::list_conversations_with_summaries(
            &Client::new(sock.clone()), "/p".into(), None, 500, 0,
        )
        .await
        .unwrap_err();
        assert_eq!((e.kind, e.message.as_str()), (ServiceErrorKind::Rejected, "daemon 错误: boom"));
        let _ = std::fs::remove_file(&sock);
    }
}
```
Run: `cargo test -p dozer-client conversation_service_tests 2>&1 | grep -E '^error' | head -3` → Expected: 编译失败(`Client: ConversationService` 未实现)(RED)。

- [ ] **Step 2: 实现**

顶部 `use dozer_services::{...}` 追加 `ConversationRows, ConversationService`。在 `impl GroupChatService for Client` 之后追加(用 `Client::method(self, ..)` 路径调 inherent 方法):

```rust
impl ConversationService for Client {
    fn list_conversations_with_summaries(
        &self,
        cwd: String,
        agent: Option<dozer_core::protocol::AgentKind>,
        limit: u32,
        offset: u32,
    ) -> BoxFuture<'_, Result<ConversationRows, ServiceError>> {
        Box::pin(async move {
            Client::list_conversations_with_summaries(self, &cwd, agent, limit, offset)
                .await
                .map_err(into_service_error)
        })
    }
}
```
Run: `cargo test -p dozer-client 2>&1 | grep -E 'test result|FAILED|^error'` → Expected: 原有测试 + 3 个新测试全绿。

- [ ] **Step 3: 提交**

```bash
cargo fmt -p dozer-client
cargo clippy -p dozer-client --all-targets 2>&1 | grep -c '^warning'   # 期望 0
git checkout Cargo.lock 2>/dev/null
git add crates/dozer-client && git status --short
git commit -m "feat(client): Conversation S2 Client 实现 ConversationService

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 3: `Services.conversation` + 默认适配 + `FakeConversation`

**Files:**
- Modify: `crates/dozer-services/src/lib.rs`(`Services` 加字段;测试里的构造)
- Modify: `crates/dozer-app/src/panel_host.rs`(`services_for`)
- Modify: `crates/dozer-app/src/service_fakes.rs`(新增 `FakeConversation` 与 `conversation_row`;`base_services()` 补字段)

**Interfaces:**
- Consumes: Task 1/2。
- Produces:
  - `Services { usage, todo, project, memory, group_chat, conversation: Arc<dyn ConversationService> }`;`services_for(&Client)` 也适配 `conversation`。
  - `service_fakes::conversation_row(id: &str, title: &str) -> (ConversationSummary, Option<SessionSummaryPayload>)`:`ConversationSummary { conversation_id: id, agent: AgentKind::Claude, file_path: format!("/t/{id}.jsonl"), title, first_ts: 0, last_ts: 0, turn_count: 1 }`,总结为 `None`。
  - `FakeConversation`(`Clone`,克隆共享状态,`Default` = 成功且空):`.with_rows(ConversationRows)`、`FakeConversation::failing(kind, message)`、`calls() -> Vec<String>`(格式 `"list_conversations_with_summaries(/p,None,500,0)"`,`agent` 用 `{:?}`)、`into_services()`(`Services { conversation: Arc::new(self), ..base_services() }`)。

- [ ] **Step 1: 写失败测试**

(a) `panel_host.rs` 测试模块(`default_services_route_group_chat_…` 旁)追加:
```rust
    #[test]
    fn default_services_route_conversation_through_the_client_adapter() {
        let rt = test_runtime();
        let (io, _rx) = recording_io(&rt);
        // recording_io 的 Client 指向不存在的 socket → Unavailable
        let e = rt
            .block_on(io.services().conversation.list_conversations_with_summaries("/p".into(), None, 500, 0))
            .unwrap_err();
        assert_eq!(e.kind, dozer_services::ServiceErrorKind::Unavailable);
    }
```
(b) `dozer-services` 的 `services_is_cloneable_and_shares_the_service` 同时构造 `conversation: Arc::new(MemConversation(Ok(vec![])))` 并追加 `Arc::ptr_eq(&s.conversation, &t.conversation)`。

Run: `cargo test -p dozer-services 2>&1 | grep -E '^error' | head -3` → Expected: 编译失败(`Services` 没有字段 `conversation`)(RED)。

- [ ] **Step 2: 实现**

1. `Services` 加 `pub conversation: Arc<dyn ConversationService>,`;`services_for` 加 `conversation: Arc::new(client.clone()),`。
2. `service_fakes.rs`:实现 `FakeConversation` 与 `conversation_row`,**模式同已有的 `FakeGroupChat`**(`Arc<Mutex<State>>`;`rows: Result<ConversationRows, ServiceError>`;每次调用 `calls.push(..)`)。`base_services()` 补 `conversation: Arc::new(FakeConversation::default())`。
3. 新增的构造函数在 Task 4 之前未被使用时,加 `#[allow(dead_code)]`(Task 4 去掉)。

Run: `cargo test -p dozer-services 2>&1 | grep -E 'test result|^error'`;`cargo test -p dozer-app default_services_route_conversation 2>&1 | grep -E 'test result|^error'` → Expected: 通过。

- [ ] **Step 3: 全 workspace 编译与提交**

```bash
cargo build --workspace 2>&1 | grep -E '^error'          # 期望无输出
cargo test -p dozer-app -- panel_host usage_probe 2>&1 | grep -E 'test result|FAILED'
cargo clippy --all-targets 2>&1 | grep -c '^warning'      # 期望 ≤ 基线
cargo fmt -p dozer-app -p dozer-services
git checkout Cargo.lock 2>/dev/null
git add crates/dozer-services crates/dozer-app/src && git status --short
git commit -m "feat(host): Conversation S3 Services.conversation + FakeConversation

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 4: 面板与 host 切换,刷新失败分档

> 说明(Ruling,计划阶段):`SessionsRefreshed` 的结果类型一变,`app/update.rs` 里**另一处**构造它的地方(`conversation_summary_generate`,约 6245 行,总结任务完成后刷新列表)也必须同时改,否则编译失败。这一处属 host 侧总结任务,规格 §1 列为"不迁",但它**只**需要把 `list_conversations_with_summaries` 这一次调用换成服务调用(该函数其余的 `submit_summary_job`/`get_summary_job` 等仍用 `Client`)。代价:host 多一处最小改动;若漏改,编译器会报错而不是静默出错。

**Files:**
- Modify: `crates/dozer-app/src/extensions/conversations.rs`(消息类型、`update` 失败分支、`spawn_refresh`、`on_activate`、`WorkspaceState` 的 `outbox`/`last_load_failure`/`take_outbox`、冷却常量、测试)
- Modify: `crates/dozer-app/src/workspace/state.rs`(`spawn_conversations_refresh`)
- Modify: `crates/dozer-app/src/app/update.rs`(`conversation_summary_generate` 里的那一次调用;`drain_outboxes` 加一行)
- Modify: `crates/dozer-app/src/service_fakes.rs`(去掉临时 `#[allow(dead_code)]`)
- Modify: `crates/dozer-app/src/usage_probe/host.rs`(行号引用若漂移,只改行号)

**Interfaces:**
- Consumes: Task 3 的 `Services`、`services_for`、`FakeConversation`、`conversation_row`;`ServiceError`/`ServiceErrorKind`。
- Produces:
  - `Message::SessionsRefreshed(ProjectId, Result<Vec<SessionRow>, dozer_services::ServiceError>)`
  - `conversations::spawn_refresh(project_id: i64, cwd: PathBuf, service: Arc<dyn ConversationService>, handle: &Handle, emit: impl Fn(Message) + Send + 'static)`
  - `WorkspaceState::take_outbox(&mut self) -> Vec<crate::extensions::toast::Pending>`

- [ ] **Step 1: 写失败测试**

在 `extensions/conversations.rs` 的 `mod tests` 追加(`runtime`/`TIMEOUT`/`offline_ctx` 已被 H6 测试 import;另加 import:`use crate::service_fakes::{FakeConversation, conversation_row}; use dozer_services::{ServiceError, ServiceErrorKind};`)。同时把已有测试 `sessions_refreshed_err_lands_as_empty_vec_not_none` 里的 `Err("boom".into())` 改为 `Err(ServiceError::new(ServiceErrorKind::Protocol, "boom"))`:

```rust
    fn session_row(id: &str) -> SessionRow {
        let (c, s) = conversation_row(id, "标题");
        SessionRow::from_row(&c, s.as_ref())
    }

    #[test]
    fn spawn_refresh_maps_rows_and_passes_the_arguments_through() {
        let rt = runtime();
        let fake = FakeConversation::default().with_rows(vec![conversation_row("c1", "标题")]);
        let (tx, rx) = std::sync::mpsc::channel();
        let tx = std::sync::Mutex::new(tx);
        spawn_refresh(
            7,
            PathBuf::from("/tmp/p"),
            fake.clone().into_services().conversation,
            rt.handle(),
            move |m| { let _ = tx.lock().unwrap().send(m); },
        );
        match rx.recv_timeout(TIMEOUT).unwrap() {
            Message::SessionsRefreshed(7, Ok(rows)) => {
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0].conversation_id, "c1");
                assert_eq!(rows[0].display_title, "标题");
            }
            other => panic!("unexpected: {other:?}"),
        }
        assert_eq!(
            fake.calls(),
            vec!["list_conversations_with_summaries(/tmp/p,None,500,0)".to_string()]
        );
    }

    #[test]
    fn a_failing_service_emits_the_error_unchanged() {
        let rt = runtime();
        let fake = FakeConversation::failing(ServiceErrorKind::Rejected, "daemon 错误: boom");
        let (tx, rx) = std::sync::mpsc::channel();
        let tx = std::sync::Mutex::new(tx);
        spawn_refresh(7, PathBuf::from("/tmp/p"), fake.into_services().conversation, rt.handle(),
            move |m| { let _ = tx.lock().unwrap().send(m); });
        match rx.recv_timeout(TIMEOUT).unwrap() {
            Message::SessionsRefreshed(7, Err(e)) => {
                assert_eq!((e.kind, e.message.as_str()), (ServiceErrorKind::Rejected, "daemon 错误: boom"));
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn first_load_failure_shows_an_empty_list_not_a_stuck_loading_state() {
        let mut ws = WorkspaceState::default();
        update(&mut ws, Message::SessionsRefreshed(1, Err(ServiceError::new(ServiceErrorKind::Unavailable, "daemon 断开"))));
        assert_eq!(ws.sessions(), Some([].as_slice()));
        assert!(ws.take_outbox().is_empty(), "Unavailable 由顶栏徽标表达,不弹 Toast");
    }

    #[test]
    fn a_failed_refresh_keeps_the_sessions_already_shown() {
        let mut ws = WorkspaceState::default();
        update(&mut ws, Message::SessionsRefreshed(1, Ok(vec![session_row("c1")])));
        update(&mut ws, Message::SessionsRefreshed(1, Err(ServiceError::new(ServiceErrorKind::Unavailable, "daemon 断开"))));
        assert_eq!(ws.sessions().map(|s| s.len()), Some(1), "刷新失败不得抹掉已显示的会话");
    }

    #[test]
    fn repeated_rejected_failures_toast_once_per_text_with_a_key() {
        let mut ws = WorkspaceState::default();
        update(&mut ws, Message::SessionsRefreshed(1, Ok(vec![session_row("c1")])));
        for _ in 0..3 {
            update(&mut ws, Message::SessionsRefreshed(1, Err(ServiceError::new(ServiceErrorKind::Rejected, "daemon 错误: boom"))));
        }
        let toasts = ws.take_outbox();
        assert_eq!(toasts.len(), 1, "相同文本 60 秒内只提示一次");
        assert_eq!(toasts[0].level, crate::extensions::toast::Level::Error);
        assert!(toasts[0].text.contains("daemon 错误: boom"));
        assert_eq!(toasts[0].key.as_deref(), Some("conversations-load"));
        assert_eq!(ws.sessions().map(|s| s.len()), Some(1));
        update(&mut ws, Message::SessionsRefreshed(1, Err(ServiceError::new(ServiceErrorKind::Protocol, "另一种错误"))));
        assert_eq!(ws.take_outbox().len(), 1, "换了文本要再提示");
    }
```
Run: `cargo test -p dozer-app extensions::conversations 2>&1 | grep -E '^error' | head -6` → Expected: 编译失败(`take_outbox` 不存在、`SessionsRefreshed` 类型不符、`spawn_refresh` 参数不符)(RED)。

- [ ] **Step 2: 面板侧实现**

`extensions/conversations.rs`:
1. 顶部 `use dozer_services::{ConversationService, ServiceError, ServiceErrorKind}; use std::sync::Arc;`(`Mutex` 等已 import 的复用)。
2. `Message::SessionsRefreshed(ProjectId, Result<Vec<SessionRow>, ServiceError>)`(同步改其文档注释)。
3. `WorkspaceState`(`#[derive(Default)]` 仍成立)加字段:
```rust
    /// 待发提示;`App::drain_outboxes` 排空成 Toast。
    outbox: crate::extensions::toast::Outbox,
    /// 上一次已提示的刷新失败(文本 + 时刻),用于冷却去重。
    last_load_failure: Option<(String, std::time::Instant)>,
```
并加 `pub fn take_outbox(&mut self) -> Vec<crate::extensions::toast::Pending> { self.outbox.take() }`;模块级常量 `pub(crate) const LOAD_FAILURE_TOAST_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(60);`。
4. `update` 的 `SessionsRefreshed` 分支改为:
```rust
        Message::SessionsRefreshed(_, result) => match result {
            Ok(rows) => ws_state.sessions = Some(rows),
            Err(e) => {
                // 首次加载失败:置空列表(不停在 None,界面不会一直"加载中");已有列表则保留。
                if ws_state.sessions.is_none() {
                    ws_state.sessions = Some(Vec::new());
                }
                if e.kind == ServiceErrorKind::Unavailable {
                    dozer_core::log_debug!(LOG, "对话会话列表查询失败(dozerd 不可用): {}", e.message);
                } else {
                    let now = std::time::Instant::now();
                    let repeat = matches!(
                        &ws_state.last_load_failure,
                        Some((last, at)) if *last == e.message
                            && now.duration_since(*at) < LOAD_FAILURE_TOAST_COOLDOWN
                    );
                    if !repeat {
                        ws_state.outbox.push_keyed(
                            LOG,
                            crate::extensions::toast::Level::Error,
                            format!("会话列表加载失败: {}", e.message),
                            "conversations-load",
                        );
                        ws_state.last_load_failure = Some((e.message, now));
                    }
                }
            }
        },
```
5. `spawn_refresh`:
```rust
pub fn spawn_refresh(
    project_id: i64,
    cwd: PathBuf,
    service: Arc<dyn ConversationService>,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    handle.spawn(async move {
        let result = service
            .list_conversations_with_summaries(cwd.to_string_lossy().into_owned(), None, 500, 0)
            .await
            .map(|rows| {
                rows.iter()
                    .map(|(c, s)| SessionRow::from_row(c, s.as_ref()))
                    .collect()
            });
        emit(Message::SessionsRefreshed(project_id, result));
    });
}
```
`on_activate`:`ctx.io.client()` → `ctx.io.services().conversation.clone()`。

- [ ] **Step 3: host 侧实现**

- `workspace/state.rs`(约 865 行):`conversations::spawn_refresh(project_id, cwd, &io.client, &io.handle, emit);` → `conversations::spawn_refresh(project_id, cwd, crate::panel_host::services_for(&io.client).conversation, &io.handle, emit);`。
- `app/update.rs` `conversation_summary_generate`(约 6245 行):在 `let client = self.client.clone();` 之后加 `let conversation = crate::panel_host::services_for(&client).conversation;`,并把
```rust
            let result = client
                .list_conversations_with_summaries(&cwd, None, 500, 0)
                .await
                .map(|rows| { … })
                .map_err(|e| e.to_string());
```
改为
```rust
            let result: Result<Vec<dozer_model::SessionRow>, dozer_services::ServiceError> = conversation
                .list_conversations_with_summaries(cwd.clone(), None, 500, 0)
                .await
                .map(|rows| {
                    rows.iter()
                        .map(|(c, s)| dozer_model::SessionRow::from_row(c, s.as_ref()))
                        .collect()
                });
```
其后的 `completion = result.as_ref().map(|_| ()).map_err(|e| format!("总结已生成，但刷新会话失败：{e}"))` 文本不变(`ServiceError` 的 `Display` = message,与原 `String` 相同)。
- `app/update.rs` `drain_outboxes`(约 214–221 行):在 `pending.extend(ws.project_panel.take_outbox());` 之后加 `pending.extend(ws.conversations.take_outbox());`。
- host 对 `SessionsRefreshed` 的专属臂(约 1514 行)只透传 `result`,类型随之改变,**不改逻辑**;编译器若报错再就地调整类型标注。
- 去掉 `service_fakes.rs` 里 Task 3 临时加的 `#[allow(dead_code)]`。

- [ ] **Step 4: 运行面板测试**

```bash
cargo test -p dozer-app extensions::conversations 2>&1 | grep -E 'test result|FAILED|^error'
```
Expected: 全绿(含 5 个新测试;`on_activate_refreshes_the_session_list_of_that_project` 仍通过,因默认服务束由 dead-socket `Client` 适配,得到 `SessionsRefreshed(7, Err(_))`)。

- [ ] **Step 5: 探针守卫、全量、门禁、提交**

```bash
cargo test -p dozer-app usage_probe::host 2>&1 | grep -E 'test result|FAILED|实际是'   # drain_outboxes 多一行,host_site 行号会整体 +1:只改 host_site() 与注释里的行号,不改守卫逻辑
W=$(ls -d .superpowers/sdd/*/ | head -1)
cargo test --workspace --no-fail-fast > ${W}t4.txt 2>&1; grep -E '^test result' ${W}t4.txt | awk '{s+=$4;f+=$6} END{print s,f}'; grep -E '^test .* FAILED' ${W}t4.txt
cargo clippy --all-targets 2>&1 | grep -c '^warning'
python3 scripts/audit/check_panel_boundary.py        # 只许剩 R-HOST-EXECUTORS 0 -> 1;R-HOST-PANEL-ARMS 不得上升
bash scripts/check-log-scope.sh | tail -1
cargo fmt -p dozer-app && cargo fmt --check && echo fmt-ok
```
Expected: 失败仅已知的;clippy 警告数 ≤ 基线;门禁无新增回退;log scope ok。
```bash
git checkout Cargo.lock 2>/dev/null; git add crates/dozer-app/src && git status --short
git commit -m "feat(conversations): Conversation S4 面板改用 ConversationService,刷新失败保留列表并分档提示

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 5: `R-CLIENT` 基线下降、验收与文档回填

**Files:**
- Modify: `scripts/audit/panel-boundary.baseline.json`(仅 `extensions/conversations.rs` 的 `R-CLIENT` 条目)
- Modify: `docs/dozer-v2/bytehost-H0/08-client-use-inventory.md`、`CLAUDE.md`

**Interfaces:**
- Produces: `extensions/conversations.rs` 不再出现在 `R-CLIENT` 基线里。

- [ ] **Step 1: 确认门禁现状并下调基线**

```bash
grep -n 'dozer_client\|io\.client()' crates/dozer-app/src/extensions/conversations.rs | head -3   # 期望无输出
python3 scripts/audit/check_panel_boundary.py   # 此时 conversations.rs 的 R-CLIENT 实测应为 0(低于基线),门禁只显示既有 R-HOST-EXECUTORS
python3 - <<'EOF'
import json
p = "scripts/audit/panel-boundary.baseline.json"
base = json.load(open(p, encoding="utf-8"))
rel = "extensions/conversations.rs"
if rel in base:
    base[rel].pop("R-CLIENT", None)
    if not base[rel]:
        del base[rel]
json.dump(base, open(p, "w", encoding="utf-8"), ensure_ascii=False, indent=1, sort_keys=True)
open(p, "a").write("\n")
EOF
git diff --stat scripts/audit/panel-boundary.baseline.json
python3 scripts/audit/check_panel_boundary.py; echo exit=$?
python3 scripts/audit/test_check_panel_boundary.py 2>&1 | tail -1
```
Expected: 基线 diff 只含 `extensions/conversations.rs` 条目的删除;门禁仍只报 `R-HOST-EXECUTORS 0 -> 1`。

- [ ] **Step 2: 验收清单(逐条跑、读输出)**

```bash
cargo tree -p dozer-services --edges normal                    # 1: 仅 dozer-model + serde 系
python3 scripts/audit/unit_edges.py ext:conversations          # 2: 无 dozer_client
cargo test -p dozer-app extensions::conversations 2>&1 | grep -E 'test result|FAILED'   # 3
cargo test -p dozer-core --test model_golden 2>&1 | grep 'test result'                  # 5
W=$(ls -d .superpowers/sdd/*/ | head -1)
cargo test --workspace --no-fail-fast > ${W}final.txt 2>&1; grep -E '^test result' ${W}final.txt | awk '{s+=$4;f+=$6} END{print s,f}'; grep -E '^test .* FAILED' ${W}final.txt   # 4
cargo clippy --all-targets 2>&1 | grep -c '^warning'; cargo fmt --check && echo fmt-ok
bash scripts/check-log-scope.sh && echo log-scope-ok
git status --short                                             # 无 Info.plist、无无关 Cargo 改动
```
任一不满足即不收尾,回到对应任务修。

- [ ] **Step 3: 文档回填并提交**

- `docs/dozer-v2/bytehost-H0/08-client-use-inventory.md` 末尾追加一节"Conversations 迁移(2026-10-11)":`extensions/conversations.rs` 经 `ConversationService`(`dozer-services`)取数;`R-CLIENT` 基线中 `conversations.rs` 已清零;刷新失败改为分档:已有列表保留(此前会清空)、首次失败仍显示空列表、`Unavailable` 静默、其余弹 `conversations-load` Toast(同文本 60 秒内一次);`conversation_summary_generate` 里的列表刷新调用随消息类型一起改用服务。**登记后续项**:`SessionsRefreshed` 没有代数保护;读失败冷却逻辑在 Todo/Project/Conversations 各有一份,可抽到 `toast::Outbox`;host 侧其余会话调用(打开、翻页、总结任务、homespace、hook)仍直接用 `Client`;剩余直接用 `Client` 的面板:`codehealth`、`agent_context`、`browser`、`edit_history`、`settings`、`project_create`。
- `CLAUDE.md` Workspace 布局表 `dozer-services` 行里"Usage、Todo、Project、GroupChat 已迁,其余面板待迁"改为"Usage、Todo、Project、GroupChat、Conversations 已迁,其余面板待迁"。
```bash
git add scripts/audit docs CLAUDE.md
git commit -m "chore(audit): Conversation S5 R-CLIENT 基线下调 + 文档回填

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

之后按 executing-plans 的 Final Review(整分支 fresh reviewer)与 finishing-a-development-branch;**合并进 main 属于需先询问用户的外部副作用**,合并前核对 main 上用户 WIP(`Cargo.lock`、`crates/dozer-app/Cargo.toml`、`Info.plist` 等)与本分支的重叠,用 `git stash` → 合并 → `git stash pop`。
