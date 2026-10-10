# AgentContext 面板迁移到服务 trait Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 终端下方的 Agent 上下文条(`extensions/agent_context.rs`)的 3 个服务调用改为经 `dozer-services` 的 `ContextService`;用户可见行为零变化。

**Architecture:** `dozer-services` 新增 `ContextService`(3 个方法)与 `Services.context`;`Client` 实现它(复用 `into_service_error`);`request_refresh`/`request_add`/`update` 改吃 `Arc<dyn ContextService>`;host 三处调用点改用 `services_for(&self.client).context`。`apply`、`Message`、`Followup` 与所有失败文本不变。

**Tech Stack:** Rust 2024 workspace、tokio、iced 0.14(只涉及面板 spawn 层)、Python 门禁脚本。

**Spec:** `docs/superpowers/specs/2026-10-11-context-services-design.md`(先例:`2026-10-11-groupchat-services-design.md`、`2026-10-11-conversation-services-design.md`)

## Global Constraints

- `dozer-services` 只依赖 `dozer-model`;不加 `dozer-core`/`dozer-client`/`tokio` 到 `[dependencies]`。
- 线上协议零变化:`Request`/`Reply` 不动,`crates/dozer-core/tests/model_golden.rs` 不改且通过。
- 面板日志来源名保持 `agent`(现有 `LOG`,是 `PanelKind` 面板名);不新增裸 `tracing::`/`eprintln!`(`scripts/check-log-scope.sh`)。
- 失败文本逐字不变:`"加载上下文列表失败: {e}"`、`"已发送到终端,但未能记录到上下文列表: {e}"`、`"移除失败: {e}"`(`e.message` 等于原 `e.to_string()`);不新增 `LoadFailed`、不改 Toast 口径、不加冷却。
- 不迁 `extensions/edit_history.rs`;`R-HOST-PANEL-ARMS` 不上升(不新增 `app/update.rs` 的面板专属臂)。
- 在独立 worktree 实施;不提交用户 WIP(`Info.plist`、版本号相关改动);`Cargo.lock` 提交前 `git checkout Cargo.lock`(main 上已提交的锁文件缺 bytegit/byteui 的 `source` 行,干净 worktree 里跑 cargo 会补上,属噪声)。
- main 上已知问题不修不算回归:`files::tests::delete_confirm_spec_reflects_pending_target` 失败、`git_log::tests::build_*` 与 dozerd 在负载下的间歇失败、门禁 `R-HOST-EXECUTORS 0 -> 1`。
- 提交信息末尾加 `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`。

## Review Focus

- `request_refresh` 的 `missing` 判定(`root.join(entity_ref).exists()` 取反)切换后不变;服务失败得到 `Loaded(Err(服务消息))`,**不替换**已有列表的既有行为(`apply` 里)保持 → Task 4 测试。
- `request_add` 的 `kind`:`is_dir=true` → `"dir"`,否则 `"file"`;`relative` 原样 → Task 4 测试。
- `Followup::RemoveItem(id)` 调 `remove_context_item(project_id, id)`;失败得到 `Removed(Err(文本))`;`Followup::Refresh` 触发 `list` → Task 4 测试。
- `impl ContextService for Client` 必须调 inherent 方法而不是递归调 trait 方法;`add` 的 `ContextItemInfo` 原样带回 → Task 2 适配测试。
- host 三处调用点(`agent_context_message`、`agent_context_refresh`、文件树右键添加)都已切换,且 `Message::AgentContext(project_id, m)` 信封与路由不变 → Task 4 编译期 + 全量测试。

---

### Task 1: `ContextService` trait(`dozer-services`)

**Files:**
- Modify: `crates/dozer-services/src/lib.rs`

**Interfaces:**
- Consumes: `dozer_model::ContextItemInfo`(已在 `dozer-model`)。
- Produces: `pub trait ContextService: Send + Sync`,返回 `BoxFuture<'_, Result<_, ServiceError>>`,参数按值传:
  - `list_context_items(project_id: i64) -> Vec<ContextItemInfo>`
  - `add_context_item(project_id: i64, entity_kind: String, entity_ref: String) -> ContextItemInfo`
  - `remove_context_item(project_id: i64, id: i64) -> ()`

- [ ] **Step 0: 记录基线(只测量,写进 ledger;同步执行,跑完再进 Step 1,避免并发编译污染)**

```bash
W=$(ls -d .superpowers/sdd/*/ | head -1)
grep -c 'dozer_client\|io\.client()' crates/dozer-app/src/extensions/agent_context.rs     # 期望 3
cargo test --workspace --no-fail-fast > ${W}baseline-tests.txt 2>&1; grep -E '^test result' ${W}baseline-tests.txt | awk '{s+=$4;f+=$6} END{print "passed",s,"failed",f}'; grep -E '^test .* FAILED' ${W}baseline-tests.txt
cargo clippy --all-targets 2>&1 | grep -c '^warning'
python3 scripts/audit/check_panel_boundary.py; echo exit=$?
```
Expected: 失败仅已知的;门禁仅 `R-HOST-EXECUTORS 0 -> 1`;把数字写进 ledger。

- [ ] **Step 1: 写失败测试**

在 `crates/dozer-services/src/lib.rs` 现有 `#[cfg(test)] mod tests` 末尾追加(`unit()` 等辅助已存在):

```rust
    // ---- ContextService 契约 ----

    use dozer_model::ContextItemInfo;

    struct MemContext(Result<Vec<ContextItemInfo>, ServiceError>);

    impl ContextService for MemContext {
        fn list_context_items(
            &self,
            _p: i64,
        ) -> BoxFuture<'_, Result<Vec<ContextItemInfo>, ServiceError>> {
            let r = self.0.clone();
            Box::pin(async move { r })
        }
        fn add_context_item(
            &self,
            _p: i64,
            _k: String,
            _r: String,
        ) -> BoxFuture<'_, Result<ContextItemInfo, ServiceError>> {
            Box::pin(async { Err(ServiceError::new(ServiceErrorKind::Protocol, "mem")) })
        }
        fn remove_context_item(&self, _p: i64, _i: i64) -> BoxFuture<'_, Result<(), ServiceError>> {
            unit()
        }
    }

    /// `handle.spawn` 要 `'static + Send`:持有 `Arc<dyn ContextService>` 的 async 块必须满足。
    #[test]
    fn dyn_context_futures_run_inside_tokio_spawn() {
        let svc: Arc<dyn ContextService> = Arc::new(MemContext(Ok(vec![])));
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let (list, removed) = rt.block_on(async move {
            tokio::spawn(async move {
                let list = svc.list_context_items(7).await;
                let removed = svc.remove_context_item(7, 3).await;
                (list, removed)
            })
            .await
            .unwrap()
        });
        assert_eq!(list, Ok(vec![]));
        assert_eq!(removed, Ok(()));
    }

    #[test]
    fn context_read_failure_is_distinguishable_from_an_empty_list() {
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let empty = MemContext(Ok(vec![]));
        let down = MemContext(Err(ServiceError::new(ServiceErrorKind::Unavailable, "x")));
        assert!(rt.block_on(empty.list_context_items(1)).is_ok());
        assert_eq!(
            rt.block_on(down.list_context_items(1)).unwrap_err().kind,
            ServiceErrorKind::Unavailable
        );
    }
```
(若 `ContextItemInfo` 已在测试模块顶部 import 过,去掉重复的 `use`。)

Run: `cargo test -p dozer-services 2>&1 | tail -8` → Expected: 编译失败 `cannot find trait ContextService`(RED)。

- [ ] **Step 2: 实现**

顶部 `use dozer_model::{...}` 合并追加 `ContextItemInfo`。在 `CodeHealthService` 之后、`Services` 之前加:

```rust
/// Agent 上下文条的读写。参数按值传。
pub trait ContextService: Send + Sync {
    fn list_context_items(
        &self,
        project_id: i64,
    ) -> BoxFuture<'_, Result<Vec<ContextItemInfo>, ServiceError>>;
    fn add_context_item(
        &self,
        project_id: i64,
        entity_kind: String,
        entity_ref: String,
    ) -> BoxFuture<'_, Result<ContextItemInfo, ServiceError>>;
    fn remove_context_item(&self, project_id: i64, id: i64) -> BoxFuture<'_, Result<(), ServiceError>>;
}
```
**不要**在本任务给 `Services` 加字段(Task 3 做,否则 `dozer-app` 编译失败)。

Run: `cargo test -p dozer-services 2>&1 | grep -E 'test result|error'` → Expected: 全绿(原 14 个 + 新 2 个 = 16 passed)。

- [ ] **Step 3: 依赖验收与提交**

```bash
cargo tree -p dozer-services --edges normal | grep -vE 'dozer-model|serde|proc-macro2|quote|syn|unicode-ident|dozer-services' | head   # 期望无输出
cargo fmt -p dozer-services && cargo clippy -p dozer-services --all-targets 2>&1 | grep -c '^warning'   # 期望 0
git add crates/dozer-services && git commit -m "feat(services): Context S1 ContextService trait

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 2: `Client` 实现 `ContextService`

**Files:**
- Modify: `crates/dozer-client/src/lib.rs`(impl + 测试模块 `context_service_tests`)

**Interfaces:**
- Consumes: Task 1;`into_service_error`、`BoxFuture`、`ServiceError`;`todo_service_tests::serve_once`(已 `pub(super)`)。
- Produces: `impl ContextService for Client`(同名 inherent 方法不改)。

- [ ] **Step 1: 写失败测试**

先读 `Client::list_context_items` / `add_context_item` / `remove_context_item` 的 `match`(`dozer-client/src/lib.rs` 约 819–862 行)确认应答变体:`list` → `Reply::ContextItems { items }`(以代码为准),`add` → `Reply::ContextItem { item }`(以代码为准),`remove` → `Reply::Ok`。在文件末尾追加(变体名按读到的写):

```rust
#[cfg(test)]
mod context_service_tests {
    use super::todo_service_tests::serve_once;
    use super::*;
    use dozer_services::{ContextService, ServiceErrorKind};

    #[tokio::test]
    async fn list_comes_back_as_ok() {
        let sock = serve_once("ctx-list", Some(Reply::ContextItems { items: vec![] }));
        let got = ContextService::list_context_items(&Client::new(sock.clone()), 7).await;
        assert_eq!(got, Ok(vec![]));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn remove_returns_unit_on_success() {
        let sock = serve_once("ctx-remove", Some(Reply::Ok));
        let got = ContextService::remove_context_item(&Client::new(sock.clone()), 7, 3).await;
        assert_eq!(got, Ok(()));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn hang_up_is_unavailable_with_the_legacy_text() {
        let sock = serve_once("ctx-hangup", None);
        let e = ContextService::list_context_items(&Client::new(sock.clone()), 7)
            .await
            .unwrap_err();
        assert_eq!((e.kind, e.message.as_str()), (ServiceErrorKind::Unavailable, "daemon 断开"));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn error_reply_is_rejected_with_the_legacy_prefix() {
        let sock = serve_once("ctx-reject", Some(Reply::Error { message: "boom".into() }));
        let e = ContextService::remove_context_item(&Client::new(sock.clone()), 7, 3)
            .await
            .unwrap_err();
        assert_eq!((e.kind, e.message.as_str()), (ServiceErrorKind::Rejected, "daemon 错误: boom"));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn add_carries_the_item_back_unchanged() {
        let item = dozer_core::protocol::ContextItemInfo {
            id: 5,
            project_id: 7,
            entity_kind: "file".into(),
            entity_ref: "src/a.rs".into(),
            created_ms: 9,
        };
        let sock = serve_once("ctx-add", Some(Reply::ContextItem { item: item.clone() }));
        let got = ContextService::add_context_item(
            &Client::new(sock.clone()),
            7,
            "file".into(),
            "src/a.rs".into(),
        )
        .await;
        assert_eq!(got, Ok(item));
        let _ = std::fs::remove_file(&sock);
    }
}
```
Run: `cargo test -p dozer-client context_service_tests 2>&1 | grep -E '^error' | head -3` → Expected: 编译失败(`Client: ContextService` 未实现)(RED)。若 `Reply` 变体名与上面不同,先按 `match` 里读到的修正测试,再看 RED。

- [ ] **Step 2: 实现**

顶部 `use dozer_services::{...}` 追加 `ContextService`。在 `impl CodeHealthService for Client` 之后追加(用 `Client::method(self, ..)` 路径调 inherent 方法):

```rust
impl ContextService for Client {
    fn list_context_items(
        &self,
        project_id: i64,
    ) -> BoxFuture<'_, Result<Vec<dozer_core::protocol::ContextItemInfo>, ServiceError>> {
        Box::pin(async move {
            Client::list_context_items(self, project_id)
                .await
                .map_err(into_service_error)
        })
    }
    fn add_context_item(
        &self,
        project_id: i64,
        entity_kind: String,
        entity_ref: String,
    ) -> BoxFuture<'_, Result<dozer_core::protocol::ContextItemInfo, ServiceError>> {
        Box::pin(async move {
            Client::add_context_item(self, project_id, &entity_kind, &entity_ref)
                .await
                .map_err(into_service_error)
        })
    }
    fn remove_context_item(&self, project_id: i64, id: i64) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            Client::remove_context_item(self, project_id, id)
                .await
                .map_err(into_service_error)
        })
    }
}
```
类型路径以文件里已有 import 为准。

Run: `cargo test -p dozer-client 2>&1 | grep -E 'test result|FAILED|^error'` → Expected: 原有测试 + 5 个新测试全绿。

- [ ] **Step 3: 提交**

```bash
cargo fmt -p dozer-client
cargo clippy -p dozer-client --all-targets 2>&1 | grep -c '^warning'   # 期望 0
git checkout Cargo.lock 2>/dev/null
git add crates/dozer-client && git status --short
git commit -m "feat(client): Context S2 Client 实现 ContextService

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 3: `Services.context` + 默认适配 + `FakeContext`

**Files:**
- Modify: `crates/dozer-services/src/lib.rs`(`Services` 加字段;测试里的构造)
- Modify: `crates/dozer-app/src/panel_host.rs`(`services_for`)
- Modify: `crates/dozer-app/src/service_fakes.rs`(新增 `FakeContext`;`base_services()` 补字段)

**Interfaces:**
- Consumes: Task 1/2。
- Produces:
  - `Services { usage, todo, project, memory, group_chat, conversation, codehealth, context: Arc<dyn ContextService> }`;`services_for(&Client)` 也适配 `context`。
  - `FakeContext`(`Clone`,克隆共享状态,`Default` = 全部成功、列表为空),链式设置:`.with_items(Vec<ContextItemInfo>)`、`.with_list_error(kind, message)`、`.with_write_error(kind, message)`(add 与 remove 共用);`calls() -> Vec<String>`(格式 `"list_context_items(7)"`、`"add_context_item(7,file,src/a.rs)"`、`"remove_context_item(7,3)"`);`into_services()`(`Services { context: Arc::new(self), ..base_services() }`)。`add_context_item` 成功时返回 `ContextItemInfo { id: 77, project_id, entity_kind, entity_ref, created_ms: 0 }`。

- [ ] **Step 1: 写失败测试**

(a) `panel_host.rs` 测试模块(`default_services_route_codehealth_…` 旁)追加:
```rust
    #[test]
    fn default_services_route_context_through_the_client_adapter() {
        let rt = test_runtime();
        let (io, _rx) = recording_io(&rt);
        // recording_io 的 Client 指向不存在的 socket → Unavailable
        let e = rt
            .block_on(io.services().context.list_context_items(1))
            .unwrap_err();
        assert_eq!(e.kind, dozer_services::ServiceErrorKind::Unavailable);
    }
```
(b) `dozer-services` 的 `services_is_cloneable_and_shares_the_service` 同时构造 `context: Arc::new(MemContext(Ok(vec![])))` 并追加 `Arc::ptr_eq(&s.context, &t.context)`。

Run: `cargo test -p dozer-services 2>&1 | grep -E '^error' | head -3` → Expected: 编译失败(`Services` 没有字段 `context`)(RED)。

- [ ] **Step 2: 实现**

1. `Services` 加 `pub context: Arc<dyn ContextService>,`;`services_for` 加 `context: Arc::new(client.clone()),`。
2. `service_fakes.rs`:实现 `FakeContext`,**模式同已有的 `FakeCodeHealth`**(`Arc<Mutex<State>>`,`State { items: Vec<ContextItemInfo>, list_error/write_error: Option<ServiceError>, calls: Vec<String> }`)。`base_services()` 补 `context: Arc::new(FakeContext::default())`。
3. 新增的构造函数在 Task 4 之前未被使用时,加 `#[allow(dead_code)]`(Task 4 去掉)。

Run: `cargo test -p dozer-services 2>&1 | grep -E 'test result|^error'`;`cargo test -p dozer-app default_services_route_context 2>&1 | grep -E 'test result|^error'` → Expected: 通过。

- [ ] **Step 3: 全 workspace 编译与提交**

```bash
cargo build --workspace 2>&1 | grep -E '^error'          # 期望无输出
cargo test -p dozer-app -- panel_host usage_probe 2>&1 | grep -E 'test result|FAILED'
cargo clippy --all-targets 2>&1 | grep -c '^warning'      # 期望 ≤ 基线
cargo fmt -p dozer-app -p dozer-services
git checkout Cargo.lock 2>/dev/null
git add crates/dozer-services crates/dozer-app/src && git status --short
git commit -m "feat(host): Context S3 Services.context + FakeContext

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 4: 面板与 host 切换

**Files:**
- Modify: `crates/dozer-app/src/extensions/agent_context.rs`(`request_refresh`、`request_add`、`update`;新增测试)
- Modify: `crates/dozer-app/src/app/update.rs`(三个调用点)
- Modify: `crates/dozer-app/src/service_fakes.rs`(去掉临时 `#[allow(dead_code)]`)
- Modify: `crates/dozer-app/src/usage_probe/host.rs`(行号引用若漂移,只改行号)

**Interfaces:**
- Consumes: Task 3 的 `Services`、`services_for`、`FakeContext`;`ServiceError`/`ServiceErrorKind`。
- Produces:
  - `request_refresh(project_id: i64, root: PathBuf, service: Arc<dyn ContextService>, handle: &Handle, emit: impl Fn(Message) + Send + 'static)`
  - `request_add(project_id: i64, is_dir: bool, relative: String, service: Arc<dyn ContextService>, handle: &Handle, emit: impl Fn(Message) + Send + 'static)`
  - `update(state: &mut State, project_id: i64, root: PathBuf, msg: Message, service: Arc<dyn ContextService>, handle: &Handle, emit: impl Fn(Message) + Clone + Send + 'static)`

- [ ] **Step 1: 写失败测试**

在 `agent_context.rs` 的 `mod tests` 追加(`use crate::panel_host::testing::{TIMEOUT, runtime}; use crate::service_fakes::FakeContext; use dozer_services::ServiceErrorKind; use std::sync::mpsc;`;注意 `std::sync::mpsc::Sender` 是 `Clone`,所以 `emit` 满足 `update` 要求的 `Clone`):

```rust
    fn collect() -> (impl Fn(Message) + Clone + Send + 'static, mpsc::Receiver<Message>) {
        let (tx, rx) = mpsc::channel();
        (move |m| { let _ = tx.send(m); }, rx)
    }

    fn info(id: i64, entity_ref: &str) -> ContextItemInfo {
        ContextItemInfo {
            id,
            project_id: 7,
            entity_kind: "file".into(),
            entity_ref: entity_ref.into(),
            created_ms: 0,
        }
    }

    #[test]
    fn refresh_marks_entries_missing_when_the_path_is_gone() {
        let rt = runtime();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("here.txt"), "x").unwrap();
        let fake = FakeContext::default().with_items(vec![info(1, "here.txt"), info(2, "gone.txt")]);
        let (emit, rx) = collect();
        request_refresh(7, dir.path().to_path_buf(), fake.clone().into_services().context, rt.handle(), emit);
        match rx.recv_timeout(TIMEOUT).unwrap() {
            Message::Loaded(Ok(entries)) => {
                assert_eq!(entries.len(), 2);
                assert!(!entries[0].missing, "存在的文件不算缺失");
                assert!(entries[1].missing, "不存在的路径算缺失");
            }
            other => panic!("unexpected: {other:?}"),
        }
        assert_eq!(fake.calls(), vec!["list_context_items(7)".to_string()]);
    }

    #[test]
    fn a_failed_refresh_carries_the_service_text_unchanged() {
        let rt = runtime();
        let fake = FakeContext::default().with_list_error(ServiceErrorKind::Rejected, "daemon 错误: boom");
        let (emit, rx) = collect();
        request_refresh(7, std::env::temp_dir(), fake.into_services().context, rt.handle(), emit);
        match rx.recv_timeout(TIMEOUT).unwrap() {
            Message::Loaded(Err(text)) => assert_eq!(text, "daemon 错误: boom"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn add_passes_the_kind_and_relative_path_through() {
        let rt = runtime();
        for (is_dir, kind) in [(false, "file"), (true, "dir")] {
            let fake = FakeContext::default();
            let (emit, rx) = collect();
            request_add(7, is_dir, "src/a".into(), fake.clone().into_services().context, rt.handle(), emit);
            match rx.recv_timeout(TIMEOUT).unwrap() {
                Message::Added(Ok(item)) => assert_eq!(item.id, 77),
                other => panic!("unexpected: {other:?}"),
            }
            assert_eq!(fake.calls(), vec![format!("add_context_item(7,{kind},src/a)")]);
        }
    }

    #[test]
    fn a_failed_add_carries_the_service_text_unchanged() {
        let rt = runtime();
        let fake = FakeContext::default().with_write_error(ServiceErrorKind::Unavailable, "daemon 断开");
        let (emit, rx) = collect();
        request_add(7, false, "a".into(), fake.into_services().context, rt.handle(), emit);
        match rx.recv_timeout(TIMEOUT).unwrap() {
            Message::Added(Err(text)) => assert_eq!(text, "daemon 断开"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn update_remove_calls_the_service_and_reports_removed() {
        let rt = runtime();
        let fake = FakeContext::default();
        let mut state = State::default();
        let (emit, rx) = collect();
        update(&mut state, 7, std::env::temp_dir(), Message::Remove(3), fake.clone().into_services().context, rt.handle(), emit);
        assert!(matches!(rx.recv_timeout(TIMEOUT).unwrap(), Message::Removed(Ok(()))));
        assert_eq!(fake.calls(), vec!["remove_context_item(7,3)".to_string()]);
    }

    #[test]
    fn update_failed_remove_carries_the_service_text_unchanged() {
        let rt = runtime();
        let fake = FakeContext::default().with_write_error(ServiceErrorKind::Rejected, "daemon 错误: nope");
        let mut state = State::default();
        let (emit, rx) = collect();
        update(&mut state, 7, std::env::temp_dir(), Message::Remove(3), fake.into_services().context, rt.handle(), emit);
        match rx.recv_timeout(TIMEOUT).unwrap() {
            Message::Removed(Err(text)) => assert_eq!(text, "daemon 错误: nope"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn update_refresh_followup_lists_from_the_service() {
        let rt = runtime();
        let fake = FakeContext::default();
        let mut state = State::default();
        let (emit, rx) = collect();
        update(&mut state, 7, std::env::temp_dir(), Message::Removed(Ok(())), fake.clone().into_services().context, rt.handle(), emit);
        assert!(matches!(rx.recv_timeout(TIMEOUT).unwrap(), Message::Loaded(Ok(_))));
        assert_eq!(fake.calls(), vec!["list_context_items(7)".to_string()]);
    }
```
Run: `cargo test -p dozer-app extensions::agent_context 2>&1 | grep -E '^error' | head -6` → Expected: 编译失败(`request_refresh`/`request_add`/`update` 的参数类型不符)(RED)。

- [ ] **Step 2: 面板侧实现**

`agent_context.rs`:
1. 顶部 `use dozer_services::ContextService; use std::sync::Arc;`。
2. `request_refresh`:
```rust
pub fn request_refresh(
    project_id: i64,
    root: PathBuf,
    service: Arc<dyn ContextService>,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    handle.spawn(async move {
        let res = match service.list_context_items(project_id).await {
            Err(e) => Err(e.message),
            Ok(items) => tokio::task::spawn_blocking(move || {
                items
                    .into_iter()
                    .map(|info| {
                        let missing = !root.join(&info.entity_ref).exists();
                        ContextEntry { info, missing }
                    })
                    .collect::<Vec<_>>()
            })
            .await
            .map_err(|e| e.to_string()),
        };
        emit(Message::Loaded(res));
    });
}
```
3. `request_add`:参数 `client: &dozer_client::Client` → `service: Arc<dyn ContextService>`,去掉 `let client = client.clone();`;任务体 `service.add_context_item(project_id, kind.to_string(), relative).await.map_err(|e| e.message)`(`kind` 是 `&'static str`,转 `String`)。
4. `update`:参数同上换成 `service: Arc<dyn ContextService>`;`Followup::Refresh => request_refresh(project_id, root, service, handle, emit)`;`Followup::RemoveItem(id)` 分支去掉 `let client = client.clone();`,任务体 `service.remove_context_item(project_id, id).await.map_err(|e| e.message)`。

- [ ] **Step 3: host 侧实现**

`app/update.rs` 三处(用 `grep -n 'agent_context::request_add\|fn agent_context_refresh\|ctx::update(' crates/dozer-app/src/app/update.rs` 定位):
- 文件树右键添加(约 3320 行):`let client = self.client.clone();` → `let context = crate::panel_host::services_for(&self.client).context;`,`request_add(project_id, is_dir, relative, &client, &handle, …)` → 传 `context`。
- `agent_context_refresh`(约 5630 行):同上,`request_refresh(project_id, root, &client, &handle, …)` → 传 `context`。
- `agent_context_message` 的 `other` 分支(约 5601 行):`let client = self.client.clone();` → `let context = crate::panel_host::services_for(&self.client).context;`,`ctx::update(&mut ws.agent_context, project_id, root, other, &client, &handle, emit)` → 传 `context`。

去掉 `service_fakes.rs` 里 Task 3 临时加的 `#[allow(dead_code)]`。

- [ ] **Step 4: 运行面板测试**

```bash
cargo test -p dozer-app extensions::agent_context 2>&1 | grep -E 'test result|FAILED|^error'
```
Expected: 全绿(含 7 个新测试;原有 `apply` 纯状态测试保持通过)。

- [ ] **Step 5: 探针守卫、全量、门禁、提交**

```bash
cargo test -p dozer-app usage_probe::host 2>&1 | grep -E 'test result|FAILED|实际是'   # 预计不漂移;若漂移(rustfmt 折行)只改 host_site() 与注释里的行号,不改守卫逻辑
W=$(ls -d .superpowers/sdd/*/ | head -1)
cargo test --workspace --no-fail-fast > ${W}t4.txt 2>&1; grep -E '^test result' ${W}t4.txt | awk '{s+=$4;f+=$6} END{print s,f}'; grep -E '^test .* FAILED' ${W}t4.txt
cargo clippy --all-targets 2>&1 | grep -c '^warning'
python3 scripts/audit/check_panel_boundary.py        # 只许剩 R-HOST-EXECUTORS 0 -> 1;R-HOST-PANEL-ARMS 不得上升
bash scripts/check-log-scope.sh | tail -1
cargo fmt -p dozer-app && cargo fmt --check && echo fmt-ok
```
Expected: 失败仅已知的(含 dozerd 负载下的间歇失败——若出现,单独重跑 `cargo test -p dozerd` 三次确认);clippy 警告数 ≤ 基线;门禁无新增回退;log scope ok。
```bash
git checkout Cargo.lock 2>/dev/null; git add crates/dozer-app/src && git status --short
git commit -m "feat(agent_context): Context S4 上下文条改用 ContextService

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 5: `R-CLIENT` 基线下降、验收与文档回填

**Files:**
- Modify: `scripts/audit/panel-boundary.baseline.json`(仅 `extensions/agent_context.rs` 的 `R-CLIENT` 条目)
- Modify: `docs/dozer-v2/bytehost-H0/08-client-use-inventory.md`、`CLAUDE.md`

**Interfaces:**
- Produces: `extensions/agent_context.rs` 不再出现在 `R-CLIENT` 基线里。

- [ ] **Step 1: 确认门禁现状并下调基线**

```bash
grep -n 'dozer_client\|io\.client()' crates/dozer-app/src/extensions/agent_context.rs | head -3   # 期望无输出
python3 scripts/audit/check_panel_boundary.py   # 此时 agent_context.rs 的 R-CLIENT 实测应为 0(低于基线),门禁只显示既有 R-HOST-EXECUTORS
python3 - <<'EOF'
import json
p = "scripts/audit/panel-boundary.baseline.json"
base = json.load(open(p, encoding="utf-8"))
rel = "extensions/agent_context.rs"
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
Expected: 基线 diff 只含 `extensions/agent_context.rs` 条目的删除;门禁仍只报 `R-HOST-EXECUTORS 0 -> 1`。

- [ ] **Step 2: 验收清单(逐条跑、读输出)**

```bash
cargo tree -p dozer-services --edges normal                    # 1: 仅 dozer-model + serde 系
python3 scripts/audit/unit_edges.py ext:agent_context          # 2: 无 dozer_client
cargo test -p dozer-app extensions::agent_context 2>&1 | grep -E 'test result|FAILED'   # 3
cargo test -p dozer-core --test model_golden 2>&1 | grep 'test result'                  # 5
W=$(ls -d .superpowers/sdd/*/ | head -1)
cargo test --workspace --no-fail-fast > ${W}final.txt 2>&1; grep -E '^test result' ${W}final.txt | awk '{s+=$4;f+=$6} END{print s,f}'; grep -E '^test .* FAILED' ${W}final.txt   # 4
cargo clippy --all-targets 2>&1 | grep -c '^warning'; cargo fmt --check && echo fmt-ok
bash scripts/check-log-scope.sh && echo log-scope-ok
git status --short                                             # 无 Info.plist、无无关 Cargo 改动
```
任一不满足即不收尾,回到对应任务修。

- [ ] **Step 3: 文档回填并提交**

- `docs/dozer-v2/bytehost-H0/08-client-use-inventory.md` 末尾追加一节"AgentContext 迁移(2026-10-11)":`extensions/agent_context.rs` 经 `ContextService`(`dozer-services`)取数与写入;`R-CLIENT` 基线中 `agent_context.rs` 已清零;用户可见行为零变化(失败本来就可见)。**登记后续项**:`Loaded` 没有代数保护;剩余直接用 `Client` 的面板:`browser`、`edit_history`、`settings`、`project_create`。
- `CLAUDE.md` Workspace 布局表 `dozer-services` 行里"Usage、Todo、Project、GroupChat、Conversations、CodeHealth 已迁,其余面板待迁"改为"Usage、Todo、Project、GroupChat、Conversations、CodeHealth、AgentContext 已迁,其余面板待迁"。
```bash
git add scripts/audit docs CLAUDE.md
git commit -m "chore(audit): Context S5 R-CLIENT 基线下调 + 文档回填

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

之后按 executing-plans 的 Final Review(整分支 fresh reviewer)与 finishing-a-development-branch;**合并进 main 属于需先询问用户的外部副作用**,合并前核对 main 上用户 WIP(`Cargo.lock`、`crates/dozer-app/Cargo.toml`、`Info.plist` 等)与本分支的重叠,用 `git stash` → 合并 → `git stash pop`。
