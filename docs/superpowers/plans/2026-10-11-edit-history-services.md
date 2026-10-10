# EditHistory 面板迁移到服务 trait Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Agent 修改历史弹窗(`extensions/edit_history.rs`)的 2 个服务调用改为经 `dozer-services` 的 `EditHistoryService`;用户可见行为零变化。

**Architecture:** `dozer-services` 新增 `EditHistoryService`(2 个方法)、参数结构体 `PreciseEdit`(代替 11 个位置参数)与 `Services.edit_history`;`Client` 实现它(解构 `PreciseEdit` 调原 11 参数 inherent 方法,复用 `into_service_error`);`request_load`/`update` 改吃 `Arc<dyn EditHistoryService>`;host 两处调用点改用 `services_for(&self.client).edit_history`。`apply`、`Message`、`Followup`、`RevertStatus` 与所有失败文本不变。

**Tech Stack:** Rust 2024 workspace、tokio、iced 0.14(只涉及弹窗 spawn 层)、Python 门禁脚本。

**Spec:** `docs/superpowers/specs/2026-10-11-edit-history-services-design.md`(先例:`2026-10-11-context-services-design.md`、`2026-10-11-groupchat-services-design.md`)

## Global Constraints

- `dozer-services` 只依赖 `dozer-model`;不加 `dozer-core`/`dozer-client`/`tokio` 到 `[dependencies]`。
- 线上协议零变化:`Request`/`Reply` 不动,`crates/dozer-core/tests/model_golden.rs` 不改且通过;`Client::apply_precise_edit` 的 inherent 签名不改(`dozer-mcp` 等仍用它)。
- 撤销时传给服务的内容逐字不变:`expected_text = 该条 new_text`、`new_text = 该条 old_text`、`start_line/start_col` 取该条起点、`end_line/end_col` 取该条 `new_end_line/new_end_col`、`summary = "撤销:{summary}"`、`actor = "dozer"`、`session_id = "dozer-gui"`。
- 失败文本逐字不变(`e.message` 等于原 `e.to_string()`);不新增 `LoadFailed`、outbox、冷却;`revert_failure`/`RevertStatus` 不动。
- 本文件不声明 `LOG`,迁移不新增日志;`R-HOST-PANEL-ARMS` 不上升(不新增 `app/update.rs` 的面板专属臂)。
- 在独立 worktree 实施;不提交用户 WIP(`Info.plist`、版本号相关改动);`Cargo.lock` 提交前 `git checkout Cargo.lock`(main 上已提交的锁文件缺 bytegit/byteui 的 `source` 行,干净 worktree 里跑 cargo 会补上,属噪声)。
- main 上已知问题不修不算回归:`files::tests::delete_confirm_spec_reflects_pending_target` 失败、`git_log::tests::build_*`/`assets::tests::serves_vendored_asset_with_mime`/dozerd 在并行负载下的间歇失败、门禁 `R-HOST-EXECUTORS 0 -> 1`。
- 提交信息末尾加 `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`。

## Review Focus

- **撤销的 `PreciseEdit` 与现在的 11 个位置参数一一对应、逐字不变**(尤其:`end_line/end_col` 取 `new_end_*`,`expected_text`/`new_text` 对调,`summary` 的 `"撤销:"` 前缀,`actor`/`session_id` 常量)→ Task 4 面板测试 + Task 2 的"捕获请求行"测试(防 `PreciseEdit` 解构成位置参数时错位)。
- `request_load` 把 `filter` 与 `DEFAULT_LIMIT`(200)原样传给服务;失败文本逐字不变 → Task 4 测试。
- `RevertDone(Ok(Applied))` 之后 `Followup::Reload` 仍触发一次 `list` → Task 4 测试。
- `impl EditHistoryService for Client` 必须调 inherent 方法而不是递归调 trait 方法;`MutationOutcome` 原样带回 → Task 2 适配测试。
- host 两处调用点(`open_edit_history`、`edit_history_message` 的 `other` 分支)都已切换,`Message::EditHistory(m)` 信封与路由不变 → Task 4 编译期 + 全量测试。

---

### Task 1: `EditHistoryService` + `PreciseEdit`(`dozer-services`)

**Files:**
- Modify: `crates/dozer-services/src/lib.rs`

**Interfaces:**
- Consumes: `dozer_model::{FileEditHistoryInfo, MutationOutcome}`(均已在 `dozer-model`)。
- Produces:
  - `#[derive(Debug, Clone, PartialEq)] pub struct PreciseEdit { pub project_id: i64, pub path: String, pub start_line: u32, pub start_col: u32, pub end_line: u32, pub end_col: u32, pub expected_text: String, pub new_text: String, pub summary: String, pub actor: String, pub session_id: String }`
  - `pub trait EditHistoryService: Send + Sync`,返回 `BoxFuture<'_, Result<_, ServiceError>>`:
    - `list_file_edit_history(project_id: i64, path_filter: Option<String>, limit: u32) -> Vec<FileEditHistoryInfo>`
    - `apply_precise_edit(edit: PreciseEdit) -> MutationOutcome`

- [ ] **Step 0: 记录基线(只测量,写进 ledger;同步执行,跑完再进 Step 1,避免并发编译污染)**

```bash
W=$(ls -d .superpowers/sdd/*/ | head -1)
grep -c 'dozer_client\|io\.client()' crates/dozer-app/src/extensions/edit_history.rs     # 期望 2
cargo test --workspace --no-fail-fast > ${W}baseline-tests.txt 2>&1; grep -E '^test result' ${W}baseline-tests.txt | awk '{s+=$4;f+=$6} END{print "passed",s,"failed",f}'; grep -E '^test .* FAILED' ${W}baseline-tests.txt
cargo clippy --all-targets 2>&1 | grep -c '^warning'
python3 scripts/audit/check_panel_boundary.py; echo exit=$?
```
Expected: 失败仅已知的;门禁仅 `R-HOST-EXECUTORS 0 -> 1`;把数字写进 ledger。

- [ ] **Step 1: 写失败测试**

在 `crates/dozer-services/src/lib.rs` 现有 `#[cfg(test)] mod tests` 末尾追加(`unit()` 等辅助已存在):

```rust
    // ---- EditHistoryService 契约 ----

    use dozer_model::{FileEditHistoryInfo, MutationOutcome};

    fn sample_edit() -> PreciseEdit {
        PreciseEdit {
            project_id: 1,
            path: "src/a.rs".into(),
            start_line: 2,
            start_col: 1,
            end_line: 2,
            end_col: 9,
            expected_text: "new".into(),
            new_text: "old".into(),
            summary: "撤销:改".into(),
            actor: "dozer".into(),
            session_id: "dozer-gui".into(),
        }
    }

    struct MemEditHistory(Result<Vec<FileEditHistoryInfo>, ServiceError>);

    impl EditHistoryService for MemEditHistory {
        fn list_file_edit_history(
            &self,
            _p: i64,
            _f: Option<String>,
            _l: u32,
        ) -> BoxFuture<'_, Result<Vec<FileEditHistoryInfo>, ServiceError>> {
            let r = self.0.clone();
            Box::pin(async move { r })
        }
        fn apply_precise_edit(&self, _e: PreciseEdit) -> BoxFuture<'_, Result<MutationOutcome, ServiceError>> {
            Box::pin(async { Ok(MutationOutcome::NotFound) })
        }
    }

    #[test]
    fn precise_edit_is_cloneable_and_comparable() {
        let a = sample_edit();
        let b = a.clone();
        assert_eq!(a, b);
        assert_ne!(a, PreciseEdit { end_col: 10, ..b });
    }

    /// `handle.spawn` 要 `'static + Send`:持有 `Arc<dyn EditHistoryService>` 的 async 块必须满足。
    #[test]
    fn dyn_edit_history_futures_run_inside_tokio_spawn() {
        let svc: Arc<dyn EditHistoryService> = Arc::new(MemEditHistory(Ok(vec![])));
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let (list, outcome) = rt.block_on(async move {
            tokio::spawn(async move {
                let list = svc.list_file_edit_history(1, None, 200).await;
                let outcome = svc.apply_precise_edit(sample_edit()).await;
                (list, outcome)
            })
            .await
            .unwrap()
        });
        assert_eq!(list, Ok(vec![]));
        assert_eq!(outcome, Ok(MutationOutcome::NotFound));
    }

    #[test]
    fn edit_history_read_failure_is_distinguishable_from_an_empty_list() {
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let empty = MemEditHistory(Ok(vec![]));
        let down = MemEditHistory(Err(ServiceError::new(ServiceErrorKind::Unavailable, "x")));
        assert!(rt.block_on(empty.list_file_edit_history(1, None, 1)).is_ok());
        assert_eq!(
            rt.block_on(down.list_file_edit_history(1, None, 1)).unwrap_err().kind,
            ServiceErrorKind::Unavailable
        );
    }
```
(`MutationOutcome` 需要 `PartialEq` 才能 `assert_eq!`;若 `dozer-model` 里它没有 `PartialEq`,改用 `matches!(outcome, Ok(MutationOutcome::NotFound))`。)

Run: `cargo test -p dozer-services 2>&1 | tail -8` → Expected: 编译失败 `cannot find trait EditHistoryService` / `PreciseEdit`(RED)。

- [ ] **Step 2: 实现**

顶部 `use dozer_model::{...}` 合并追加 `FileEditHistoryInfo, MutationOutcome`。在 `ContextService` 之后、`Services` 之前加:

```rust
/// `apply_precise_edit` 的调用参数。原 inherent 方法有 11 个位置参数(连续 4 个 `u32` 坐标、多个 `String`),
/// 顺序传错编译器发现不了,所以在接缝上用具名字段结构体(仓库约定:参数 ≥7 且同类型相邻)。
/// 这是调用参数,不是线上形状,所以不放 `dozer-model`。
#[derive(Debug, Clone, PartialEq)]
pub struct PreciseEdit {
    pub project_id: i64,
    pub path: String,
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
    pub expected_text: String,
    pub new_text: String,
    pub summary: String,
    pub actor: String,
    pub session_id: String,
}

/// 修改历史弹窗的读写。参数按值传。
pub trait EditHistoryService: Send + Sync {
    fn list_file_edit_history(
        &self,
        project_id: i64,
        path_filter: Option<String>,
        limit: u32,
    ) -> BoxFuture<'_, Result<Vec<FileEditHistoryInfo>, ServiceError>>;
    fn apply_precise_edit(
        &self,
        edit: PreciseEdit,
    ) -> BoxFuture<'_, Result<MutationOutcome, ServiceError>>;
}
```
**不要**在本任务给 `Services` 加字段(Task 3 做,否则 `dozer-app` 编译失败)。

Run: `cargo test -p dozer-services 2>&1 | grep -E 'test result|error'` → Expected: 全绿(原 16 个 + 新 3 个 = 19 passed)。

- [ ] **Step 3: 依赖验收与提交**

```bash
cargo tree -p dozer-services --edges normal | grep -vE 'dozer-model|serde|proc-macro2|quote|syn|unicode-ident|dozer-services' | head   # 期望无输出
cargo fmt -p dozer-services && cargo clippy -p dozer-services --all-targets 2>&1 | grep -c '^warning'   # 期望 0
git add crates/dozer-services && git commit -m "feat(services): EditHistory S1 EditHistoryService + PreciseEdit

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 2: `Client` 实现 `EditHistoryService`

**Files:**
- Modify: `crates/dozer-client/src/lib.rs`(impl + 测试模块 `edit_history_service_tests`)

**Interfaces:**
- Consumes: Task 1;`into_service_error`、`BoxFuture`、`ServiceError`;`todo_service_tests::serve_once`(已 `pub(super)`)。
- Produces: `impl EditHistoryService for Client`(`Client::list_file_edit_history`/`Client::apply_precise_edit` 的 inherent 签名不改)。

- [ ] **Step 1: 写失败测试**

应答变体已核对:`list` → `Reply::FileEditHistory { entries }`,`apply` → `Reply::MutationResult { outcome }`。请求变体:`Request::ApplyPreciseEdit { project_id, path, start_line, start_col, end_line, end_col, expected_text, new_text, summary, actor, session_id }`、`Request::ListFileEditHistory { project_id, path_filter, limit }`。在文件末尾追加(新增一个**捕获请求行**的回放服务端 `serve_capture`,不改现有的 `serve_once`):

```rust
#[cfg(test)]
mod edit_history_service_tests {
    use super::todo_service_tests::{serve_once, temp_sock};
    use super::*;
    use dozer_services::{EditHistoryService, PreciseEdit, ServiceErrorKind};
    use std::sync::{Arc, Mutex};

    /// 读一行请求,记下它,再回 `reply`。返回 (socket 路径, 记录请求行的共享槽)。
    fn serve_capture(tag: &str, reply: Reply) -> (PathBuf, Arc<Mutex<Option<String>>>) {
        let sock = temp_sock(tag);
        let listener = tokio::net::UnixListener::bind(&sock).unwrap();
        let seen = Arc::new(Mutex::new(None));
        let slot = Arc::clone(&seen);
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (r, mut w) = stream.into_split();
            let mut lines = BufReader::new(r).lines();
            if let Ok(Some(line)) = lines.next_line().await {
                *slot.lock().unwrap() = Some(line);
            }
            let _ = w.write_all(encode_line(&reply).as_bytes()).await;
        });
        (sock, seen)
    }

    fn edit() -> PreciseEdit {
        PreciseEdit {
            project_id: 7,
            path: "src/a.rs".into(),
            start_line: 1,
            start_col: 2,
            end_line: 3,
            end_col: 4,
            expected_text: "NEW".into(),
            new_text: "OLD".into(),
            summary: "撤销:改".into(),
            actor: "dozer".into(),
            session_id: "dozer-gui".into(),
        }
    }

    #[tokio::test]
    async fn list_comes_back_as_ok() {
        let sock = serve_once("eh-list", Some(Reply::FileEditHistory { entries: vec![] }));
        let got = EditHistoryService::list_file_edit_history(&Client::new(sock.clone()), 7, None, 200).await;
        assert_eq!(got, Ok(vec![]));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn list_sends_the_filter_and_limit_it_was_given() {
        let (sock, seen) = serve_capture("eh-list-req", Reply::FileEditHistory { entries: vec![] });
        let _ = EditHistoryService::list_file_edit_history(
            &Client::new(sock.clone()), 7, Some("src/a.rs".into()), 200,
        )
        .await;
        let line = seen.lock().unwrap().clone().expect("服务端应收到请求");
        match decode_line::<Request>(&line).unwrap() {
            Request::ListFileEditHistory { project_id, path_filter, limit } => {
                assert_eq!((project_id, path_filter.as_deref(), limit), (7, Some("src/a.rs"), 200));
            }
            other => panic!("unexpected: {other:?}"),
        }
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn apply_sends_every_field_in_the_right_place() {
        let (sock, seen) = serve_capture(
            "eh-apply-req",
            Reply::MutationResult { outcome: MutationOutcome::NotFound },
        );
        let got = EditHistoryService::apply_precise_edit(&Client::new(sock.clone()), edit()).await;
        assert_eq!(got, Ok(MutationOutcome::NotFound), "outcome 原样带回");
        let line = seen.lock().unwrap().clone().expect("服务端应收到请求");
        match decode_line::<Request>(&line).unwrap() {
            Request::ApplyPreciseEdit {
                project_id, path, start_line, start_col, end_line, end_col,
                expected_text, new_text, summary, actor, session_id,
            } => {
                assert_eq!(
                    (project_id, path.as_str(), start_line, start_col, end_line, end_col),
                    (7, "src/a.rs", 1, 2, 3, 4)
                );
                assert_eq!((expected_text.as_str(), new_text.as_str()), ("NEW", "OLD"));
                assert_eq!((summary.as_str(), actor.as_str(), session_id.as_str()), ("撤销:改", "dozer", "dozer-gui"));
            }
            other => panic!("unexpected: {other:?}"),
        }
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn hang_up_is_unavailable_with_the_legacy_text() {
        let sock = serve_once("eh-hangup", None);
        let e = EditHistoryService::list_file_edit_history(&Client::new(sock.clone()), 7, None, 200)
            .await
            .unwrap_err();
        assert_eq!((e.kind, e.message.as_str()), (ServiceErrorKind::Unavailable, "daemon 断开"));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn error_reply_is_rejected_with_the_legacy_prefix() {
        let sock = serve_once("eh-reject", Some(Reply::Error { message: "boom".into() }));
        let e = EditHistoryService::apply_precise_edit(&Client::new(sock.clone()), edit())
            .await
            .unwrap_err();
        assert_eq!((e.kind, e.message.as_str()), (ServiceErrorKind::Rejected, "daemon 错误: boom"));
        let _ = std::fs::remove_file(&sock);
    }
}
```
(`MutationOutcome` 与 `FileEditHistoryInfo` 若在该文件里尚未 import,按编译器提示补 `dozer_core::protocol::MutationOutcome`;`Request`/`decode_line` 在 `dozer_core::protocol` 里,文件顶部已 import。)

Run: `cargo test -p dozer-client edit_history_service_tests 2>&1 | grep -E '^error' | head -3` → Expected: 编译失败(`Client: EditHistoryService` 未实现)(RED)。

- [ ] **Step 2: 实现**

顶部 `use dozer_services::{...}` 追加 `EditHistoryService, PreciseEdit`。在 `impl ContextService for Client` 之后追加(用 `Client::method(self, ..)` 路径调 inherent 方法):

```rust
impl EditHistoryService for Client {
    fn list_file_edit_history(
        &self,
        project_id: i64,
        path_filter: Option<String>,
        limit: u32,
    ) -> BoxFuture<'_, Result<Vec<dozer_core::protocol::FileEditHistoryInfo>, ServiceError>> {
        Box::pin(async move {
            Client::list_file_edit_history(self, project_id, path_filter.as_deref(), limit)
                .await
                .map_err(into_service_error)
        })
    }
    fn apply_precise_edit(
        &self,
        edit: PreciseEdit,
    ) -> BoxFuture<'_, Result<dozer_core::protocol::MutationOutcome, ServiceError>> {
        Box::pin(async move {
            let PreciseEdit {
                project_id,
                path,
                start_line,
                start_col,
                end_line,
                end_col,
                expected_text,
                new_text,
                summary,
                actor,
                session_id,
            } = edit;
            Client::apply_precise_edit(
                self,
                project_id,
                &path,
                start_line,
                start_col,
                end_line,
                end_col,
                &expected_text,
                &new_text,
                &summary,
                &actor,
                &session_id,
            )
            .await
            .map_err(into_service_error)
        })
    }
}
```
(解构用完整字段列表而不是 `..`,这样 `PreciseEdit` 将来加字段时编译器会报错。`apply_precise_edit` 的 inherent 方法有 `#[allow(clippy::too_many_arguments)]` 吗?本任务不动它;clippy 若对 impl 里的调用没有警告则无需处理。)

Run: `cargo test -p dozer-client 2>&1 | grep -E 'test result|FAILED|^error'` → Expected: 原有测试 + 5 个新测试全绿。

- [ ] **Step 3: 提交**

```bash
cargo fmt -p dozer-client
cargo clippy -p dozer-client --all-targets 2>&1 | grep -c '^warning'   # 期望 0
git checkout Cargo.lock 2>/dev/null
git add crates/dozer-client && git status --short
git commit -m "feat(client): EditHistory S2 Client 实现 EditHistoryService

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 3: `Services.edit_history` + 默认适配 + `FakeEditHistory`

**Files:**
- Modify: `crates/dozer-services/src/lib.rs`(`Services` 加字段;测试里的构造)
- Modify: `crates/dozer-app/src/panel_host.rs`(`services_for`)
- Modify: `crates/dozer-app/src/service_fakes.rs`(新增 `FakeEditHistory`;`base_services()` 补字段)

**Interfaces:**
- Consumes: Task 1/2。
- Produces:
  - `Services { usage, todo, project, memory, group_chat, conversation, codehealth, context, edit_history: Arc<dyn EditHistoryService> }`;`services_for(&Client)` 也适配 `edit_history`。
  - `FakeEditHistory`(`Clone`,克隆共享状态,`Default` = 全部成功:list → `Ok(vec![])`、apply → `Ok(MutationOutcome::Applied { new_start_line: 0, new_start_col: 0, new_end_line: 0, new_end_col: 0, history_id: 0 })`),链式设置:`.with_entries(Vec<FileEditHistoryInfo>)`、`.with_list_error(kind, message)`、`.with_outcome(MutationOutcome)`、`.with_edit_error(kind, message)`;`list_calls() -> Vec<(i64, Option<String>, u32)>`;`edits() -> Vec<PreciseEdit>`;`into_services()`(`Services { edit_history: Arc::new(self), ..base_services() }`)。

- [ ] **Step 1: 写失败测试**

(a) `panel_host.rs` 测试模块(`default_services_route_context_…` 旁)追加:
```rust
    #[test]
    fn default_services_route_edit_history_through_the_client_adapter() {
        let rt = test_runtime();
        let (io, _rx) = recording_io(&rt);
        // recording_io 的 Client 指向不存在的 socket → Unavailable
        let e = rt
            .block_on(io.services().edit_history.list_file_edit_history(1, None, 200))
            .unwrap_err();
        assert_eq!(e.kind, dozer_services::ServiceErrorKind::Unavailable);
    }
```
(b) `dozer-services` 的 `services_is_cloneable_and_shares_the_service` 同时构造 `edit_history: Arc::new(MemEditHistory(Ok(vec![])))` 并追加 `Arc::ptr_eq(&s.edit_history, &t.edit_history)`。

Run: `cargo test -p dozer-services 2>&1 | grep -E '^error' | head -3` → Expected: 编译失败(`Services` 没有字段 `edit_history`)(RED)。

- [ ] **Step 2: 实现**

1. `Services` 加 `pub edit_history: Arc<dyn EditHistoryService>,`;`services_for` 加 `edit_history: Arc::new(client.clone()),`。
2. `service_fakes.rs`:实现 `FakeEditHistory`,**模式同已有的 `FakeContext`**(`Arc<Mutex<State>>`,`State { entries: Vec<FileEditHistoryInfo>, list_error/edit_error: Option<ServiceError>, outcome: MutationOutcome, list_calls: Vec<(i64, Option<String>, u32)>, edits: Vec<PreciseEdit> }`;`list` 记录 `(project_id, path_filter, limit)` 后按 `list_error` 返回 `Err` 或 `Ok(entries.clone())`;`apply` 记录 `edit` 后按 `edit_error` 返回 `Err` 或 `Ok(outcome.clone())`)。`base_services()` 补 `edit_history: Arc::new(FakeEditHistory::default())`。
3. 新增的构造函数在 Task 4 之前未被使用时,加 `#[allow(dead_code)]`(Task 4 去掉)。

Run: `cargo test -p dozer-services 2>&1 | grep -E 'test result|^error'`;`cargo test -p dozer-app default_services_route_edit_history 2>&1 | grep -E 'test result|^error'` → Expected: 通过。

- [ ] **Step 3: 全 workspace 编译与提交**

```bash
cargo build --workspace 2>&1 | grep -E '^error'          # 期望无输出
cargo test -p dozer-app -- panel_host usage_probe 2>&1 | grep -E 'test result|FAILED'
cargo clippy --all-targets 2>&1 | grep -c '^warning'      # 期望 ≤ 基线
cargo fmt -p dozer-app -p dozer-services
git checkout Cargo.lock 2>/dev/null
git add crates/dozer-services crates/dozer-app/src && git status --short
git commit -m "feat(host): EditHistory S3 Services.edit_history + FakeEditHistory

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 4: 弹窗与 host 切换

**Files:**
- Modify: `crates/dozer-app/src/extensions/edit_history.rs`(`request_load`、`update`;新增测试)
- Modify: `crates/dozer-app/src/app/update.rs`(两个调用点)
- Modify: `crates/dozer-app/src/service_fakes.rs`(去掉临时 `#[allow(dead_code)]`)
- Modify: `crates/dozer-app/src/usage_probe/host.rs`(行号引用若漂移,只改行号)

**Interfaces:**
- Consumes: Task 3 的 `Services`、`services_for`、`FakeEditHistory`;`PreciseEdit`;`ServiceError`/`ServiceErrorKind`。
- Produces:
  - `request_load(project_id: i64, filter: Option<String>, service: Arc<dyn EditHistoryService>, handle: &Handle, emit: impl Fn(Message) + Send + 'static)`
  - `update(state: &mut Option<State>, msg: Message, service: Arc<dyn EditHistoryService>, handle: &Handle, emit: impl Fn(Message) + Send + 'static)`

- [ ] **Step 1: 写失败测试**

在 `edit_history.rs` 的 `mod tests` 追加(该模块已有 `info(id, path)` 和 `open(filter)` 辅助;另加 `use crate::panel_host::testing::{TIMEOUT, runtime}; use crate::service_fakes::FakeEditHistory; use dozer_services::{PreciseEdit, ServiceErrorKind}; use std::sync::mpsc;`):

```rust
    fn collect() -> (impl Fn(Message) + Send + 'static, mpsc::Receiver<Message>) {
        let (tx, rx) = mpsc::channel();
        (move |m| { let _ = tx.send(m); }, rx)
    }

    /// 已加载出一条记录(坐标故意互不相同,用来核对撤销的字段映射)并选中它的弹窗状态。
    fn loaded_state() -> Option<State> {
        let mut e = info(5, "src/a.rs");
        e.start_line = 10;
        e.start_col = 11;
        e.end_line = 12;
        e.end_col = 13;
        e.new_end_line = 20;
        e.new_end_col = 21;
        e.old_text = "OLD".into();
        e.new_text = "NEW".into();
        e.summary = "改了".into();
        let mut s = open(None);
        apply(&mut s, Message::Loaded { project_id: 1, filter: None, result: Ok(vec![e]) });
        s
    }

    #[test]
    fn request_load_passes_the_filter_and_default_limit_to_the_service() {
        let rt = runtime();
        let fake = FakeEditHistory::default().with_entries(vec![info(1, "src/a.rs")]);
        let (emit, rx) = collect();
        request_load(1, Some("src/a.rs".into()), fake.clone().into_services().edit_history, rt.handle(), emit);
        match rx.recv_timeout(TIMEOUT).unwrap() {
            Message::Loaded { project_id: 1, filter, result: Ok(v) } => {
                assert_eq!(filter.as_deref(), Some("src/a.rs"));
                assert_eq!(v.len(), 1);
            }
            other => panic!("unexpected: {other:?}"),
        }
        assert_eq!(fake.list_calls(), vec![(1, Some("src/a.rs".to_string()), DEFAULT_LIMIT)]);
        assert_eq!(DEFAULT_LIMIT, 200);
    }

    #[test]
    fn a_failed_load_carries_the_service_text_unchanged() {
        let rt = runtime();
        let fake = FakeEditHistory::default().with_list_error(ServiceErrorKind::Rejected, "daemon 错误: boom");
        let (emit, rx) = collect();
        request_load(1, None, fake.into_services().edit_history, rt.handle(), emit);
        match rx.recv_timeout(TIMEOUT).unwrap() {
            Message::Loaded { result: Err(text), .. } => assert_eq!(text, "daemon 错误: boom"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn revert_sends_the_inverse_edit_with_the_exact_field_mapping() {
        let rt = runtime();
        let fake = FakeEditHistory::default();
        let mut state = loaded_state();
        let (emit, rx) = collect();
        update(&mut state, Message::Revert(5), fake.clone().into_services().edit_history, rt.handle(), emit);
        assert!(matches!(
            rx.recv_timeout(TIMEOUT).unwrap(),
            Message::RevertDone { id: 5, result: Ok(_) }
        ));
        assert_eq!(
            fake.edits(),
            vec![PreciseEdit {
                project_id: 1,
                path: "src/a.rs".into(),
                start_line: 10,
                start_col: 11,
                end_line: 20,   // 结束坐标取修改后的 new_end_*
                end_col: 21,
                expected_text: "NEW".into(), // 对调:expected = 该条 new_text
                new_text: "OLD".into(),      // new = 该条 old_text
                summary: "撤销:改了".into(),
                actor: "dozer".into(),
                session_id: "dozer-gui".into(),
            }]
        );
    }

    #[test]
    fn a_failed_revert_carries_the_service_text_unchanged() {
        let rt = runtime();
        let fake = FakeEditHistory::default().with_edit_error(ServiceErrorKind::Unavailable, "daemon 断开");
        let mut state = loaded_state();
        let (emit, rx) = collect();
        update(&mut state, Message::Revert(5), fake.into_services().edit_history, rt.handle(), emit);
        match rx.recv_timeout(TIMEOUT).unwrap() {
            Message::RevertDone { id: 5, result: Err(text) } => assert_eq!(text, "daemon 断开"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn a_successful_revert_reloads_the_list() {
        let rt = runtime();
        let fake = FakeEditHistory::default();
        let mut state = loaded_state();
        let (emit, rx) = collect();
        let applied = MutationOutcome::Applied {
            new_start_line: 1, new_start_col: 1, new_end_line: 1, new_end_col: 1, history_id: 9,
        };
        update(
            &mut state,
            Message::RevertDone { id: 5, result: Ok(applied) },
            fake.clone().into_services().edit_history,
            rt.handle(),
            emit,
        );
        assert!(matches!(rx.recv_timeout(TIMEOUT).unwrap(), Message::Loaded { .. }));
        assert_eq!(fake.list_calls().len(), 1, "撤销成功后应重新加载一次");
    }
```
Run: `cargo test -p dozer-app extensions::edit_history 2>&1 | grep -E '^error' | head -5` → Expected: 编译失败(`request_load`/`update` 参数类型不符)(RED)。

- [ ] **Step 2: 弹窗侧实现**

`edit_history.rs`:
1. 顶部 `use dozer_services::{EditHistoryService, PreciseEdit}; use std::sync::Arc;`。
2. `request_load`:
```rust
pub fn request_load(
    project_id: i64,
    filter: Option<String>,
    service: Arc<dyn EditHistoryService>,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    handle.spawn(async move {
        let result = service
            .list_file_edit_history(project_id, filter.clone(), DEFAULT_LIMIT)
            .await
            .map_err(|e| e.message);
        emit(Message::Loaded {
            project_id,
            filter,
            result,
        });
    });
}
```
3. `update`:参数 `client: &dozer_client::Client` → `service: Arc<dyn EditHistoryService>`;`Followup::Reload` 里 `request_load(s.project_id, s.filter.clone(), service, handle, emit)`;`Followup::Revert(e)` 分支去掉 `let client = client.clone();`,任务体:
```rust
            handle.spawn(async move {
                let result = service
                    .apply_precise_edit(PreciseEdit {
                        project_id: e.project_id,
                        path: e.target_path.clone(),
                        start_line: e.start_line,
                        start_col: e.start_col,
                        end_line: e.new_end_line,
                        end_col: e.new_end_col,
                        expected_text: e.new_text.clone(),
                        new_text: e.old_text.clone(),
                        summary: format!("撤销:{}", e.summary),
                        actor: "dozer".to_string(),
                        session_id: "dozer-gui".to_string(),
                    })
                    .await
                    .map_err(|err| err.message);
                emit(Message::RevertDone { id: e.id, result });
            });
```
并保留原来紧挨着的文档注释(说明 Revert 为何反向调用)。

- [ ] **Step 3: host 侧实现**

`app/update.rs` 两处(用 `grep -n 'edit_history::request_load\|eh::update(' crates/dozer-app/src/app/update.rs` 定位):
- `open_edit_history`(约 5650 行):`let client = self.client.clone();` → `let edit_history = crate::panel_host::services_for(&self.client).edit_history;`,`request_load(project_id, filter, &client, …)` → 传 `edit_history`。
- `edit_history_message` 的 `other` 分支(约 5710 行):同上,`eh::update(&mut self.edit_history, other, &client, &handle, …)` → 传 `edit_history`。

去掉 `service_fakes.rs` 里 Task 3 临时加的 `#[allow(dead_code)]`。

- [ ] **Step 4: 运行弹窗测试**

```bash
cargo test -p dozer-app extensions::edit_history 2>&1 | grep -E 'test result|FAILED|^error'
```
Expected: 全绿(含 5 个新测试;原有 `apply`/`ask_agent_text` 等测试保持通过)。

- [ ] **Step 5: 探针守卫、全量、门禁、提交**

```bash
cargo test -p dozer-app usage_probe::host 2>&1 | grep -E 'test result|FAILED|实际是'   # 预计不漂移;若漂移只改 host_site() 与注释里的行号,不改守卫逻辑
W=$(ls -d .superpowers/sdd/*/ | head -1)
cargo test --workspace --no-fail-fast > ${W}t4.txt 2>&1; grep -E '^test result' ${W}t4.txt | awk '{s+=$4;f+=$6} END{print s,f}'; grep -E '^test .* FAILED' ${W}t4.txt
cargo clippy --all-targets 2>&1 | grep -c '^warning'
python3 scripts/audit/check_panel_boundary.py        # 只许剩 R-HOST-EXECUTORS 0 -> 1;R-HOST-PANEL-ARMS 不得上升
bash scripts/check-log-scope.sh | tail -1
cargo fmt -p dozer-app && cargo fmt --check && echo fmt-ok
```
Expected: 失败仅已知的(含并行负载下的间歇失败——若出现,单独重跑该测试三次确认);clippy 警告数 ≤ 基线;门禁无新增回退;log scope ok。
```bash
git checkout Cargo.lock 2>/dev/null; git add crates/dozer-app/src && git status --short
git commit -m "feat(edit_history): EditHistory S4 修改历史弹窗改用 EditHistoryService

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 5: `R-CLIENT` 基线下降、验收与文档回填

**Files:**
- Modify: `scripts/audit/panel-boundary.baseline.json`(仅 `extensions/edit_history.rs` 的 `R-CLIENT` 条目)
- Modify: `docs/dozer-v2/bytehost-H0/08-client-use-inventory.md`、`CLAUDE.md`

**Interfaces:**
- Produces: `extensions/edit_history.rs` 不再出现在 `R-CLIENT` 基线里。

- [ ] **Step 1: 确认门禁现状并下调基线**

```bash
grep -n 'dozer_client\|io\.client()' crates/dozer-app/src/extensions/edit_history.rs | head -3   # 期望无输出
python3 scripts/audit/check_panel_boundary.py   # 此时 edit_history.rs 的 R-CLIENT 实测应为 0(低于基线),门禁只显示既有 R-HOST-EXECUTORS
python3 - <<'EOF'
import json
p = "scripts/audit/panel-boundary.baseline.json"
base = json.load(open(p, encoding="utf-8"))
rel = "extensions/edit_history.rs"
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
Expected: 基线 diff 只含 `extensions/edit_history.rs` 条目的删除;门禁仍只报 `R-HOST-EXECUTORS 0 -> 1`。

- [ ] **Step 2: 验收清单(逐条跑、读输出)**

```bash
cargo tree -p dozer-services --edges normal                    # 1: 仅 dozer-model + serde 系
python3 scripts/audit/unit_edges.py ext:edit_history           # 2: 无 dozer_client
cargo test -p dozer-app extensions::edit_history 2>&1 | grep -E 'test result|FAILED'   # 3
cargo test -p dozer-core --test model_golden 2>&1 | grep 'test result'                 # 5
W=$(ls -d .superpowers/sdd/*/ | head -1)
cargo test --workspace --no-fail-fast > ${W}final.txt 2>&1; grep -E '^test result' ${W}final.txt | awk '{s+=$4;f+=$6} END{print s,f}'; grep -E '^test .* FAILED' ${W}final.txt   # 4
cargo clippy --all-targets 2>&1 | grep -c '^warning'; cargo fmt --check && echo fmt-ok
bash scripts/check-log-scope.sh && echo log-scope-ok
git status --short                                             # 无 Info.plist、无无关 Cargo 改动
```
任一不满足即不收尾,回到对应任务修。

- [ ] **Step 3: 文档回填并提交**

- `docs/dozer-v2/bytehost-H0/08-client-use-inventory.md` 末尾追加一节"EditHistory 迁移(2026-10-11)":`extensions/edit_history.rs` 经 `EditHistoryService`(`dozer-services`)取数与撤销;`apply_precise_edit` 的 11 个位置参数在接缝上收成 `PreciseEdit` 结构体(inherent 方法不变,`dozer-mcp` 不受影响);`R-CLIENT` 基线中 `edit_history.rs` 已清零;用户可见行为零变化(失败本来就可见)。**登记后续项**:`Loaded` 靠 `project_id`/`filter` 比对丢弃过期结果(没有代数,但已有等价保护);剩余直接用 `Client` 的面板:`browser`、`settings`、`project_create`。
- `CLAUDE.md` Workspace 布局表 `dozer-services` 行里"Usage、Todo、Project、GroupChat、Conversations、CodeHealth、AgentContext 已迁,其余面板待迁"改为"Usage、Todo、Project、GroupChat、Conversations、CodeHealth、AgentContext、EditHistory 已迁,其余面板待迁"。
```bash
git add scripts/audit docs CLAUDE.md
git commit -m "chore(audit): EditHistory S5 R-CLIENT 基线下调 + 文档回填

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

之后按 executing-plans 的 Final Review(整分支 fresh reviewer)与 finishing-a-development-branch;**合并进 main 属于需先询问用户的外部副作用**,合并前核对 main 上用户 WIP(`Cargo.lock`、`crates/dozer-app/Cargo.toml`、`Info.plist` 等)与本分支的重叠,用 `git stash` → 合并 → `git stash pop`。
