# CodeHealth 面板迁移到服务 trait Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 代码健康度面板(`extensions/codehealth/aggregate.rs`)的 3 个服务调用改为经 `dozer-services` 的 `CodeHealthService`;缓存读取失败不再把已显示的报告换成"尚未扫描"的空面板,并按 Todo/Project 的口径分档提示。

**Architecture:** `dozer-services` 新增 `CodeHealthService`(3 个方法)与 `Services.codehealth`;`Client` 实现它(复用 `into_service_error`);`spawn_load_cached`/`spawn_scan` 改吃 `Arc<dyn CodeHealthService>`;新增 `Message::LoadFailed(project_id, kind, message, Box<PanelState>)`(携带照旧算出的 git 数据,同 Usage 先例),`update` 对它:首次失败采用携带的 panel、已有报告则保留,`Unavailable` 静默,其余弹带 key 的 Error Toast(同文本 60 秒内一次);`list` 失败只写 warn 日志,对比降级为无基线。

**Tech Stack:** Rust 2024 workspace、tokio、iced 0.14(只涉及面板 update/spawn 层)、Python 门禁脚本。

**Spec:** `docs/superpowers/specs/2026-10-11-codehealth-services-design.md`(先例:`2026-10-09-dozer-services-x0-design.md`、`2026-10-10-todo-services-design.md`、`2026-10-11-conversation-services-design.md`)

## Global Constraints

- `dozer-services` 只依赖 `dozer-model`;不加 `dozer-core`/`dozer-client`/`tokio` 到 `[dependencies]`。
- 线上协议零变化:`Request`/`Reply` 不动,`crates/dozer-core/tests/model_golden.rs` 不改且通过。
- **面板日志来源名是 `code_health`**(`PanelKind::CodeHealth` 对应的面板名,`app/state.rs` 的测试会扫源码强制;规格里写的 `codehealth` 是笔误,以此为准);新增的 `LOG` 声明放在 `extensions/codehealth/mod.rs`(`dozer_core::scope!(pub(crate) LOG, panel, "code_health");`),不新增裸 `tracing::`/`eprintln!`(`scripts/check-log-scope.sh`)。
- 保存失败的内联提示文本逐字不变:`"未保存，无法用于下次比较：{e}"`(`e.message` 等于原 `anyhow` 文本);`scan_error`/`save_error` 等持久状态字段与其展示不动。
- 瞬时失败走 Toast(`outbox`);`R-HOST-PANEL-ARMS` 不上升(`LoadFailed` 并入已有 `Loaded` 的专属臂,不新增臂)。
- 不改本地扫描/差异/热点/Git 采集逻辑;不迁其它面板。
- 在独立 worktree 实施;不提交用户 WIP(`Info.plist`、版本号相关改动);`Cargo.lock` 提交前剔除 bytegit/byteui 的 `source =` 行(若 `git status` 显示 `Cargo.lock` 只有这类噪声,直接 `git checkout Cargo.lock`)。
- main 上已知问题不修不算回归:`files::tests::delete_confirm_spec_reflects_pending_target` 失败、`git_log::tests::build_*` 间歇失败、门禁 `R-HOST-EXECUTORS 0 -> 1`。
- 提交信息末尾加 `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`。

## Review Focus

- **已有报告时 `get` 失败不得覆盖现有面板**(此前 `Loaded` 无条件整体替换);**从未加载过时仍采用携带的 panel**(与现状一致,git 数据照常出现)→ Task 4 测试。
- `Unavailable` 不弹 Toast;`Rejected`/`Protocol` 恰好一条 key 为 `codehealth-load`,60 秒内同文本不重复、换文本再弹 → Task 4 测试。
- `list` 失败(缓存读取与扫描各一处)**不阻断**:缓存读取仍得到 `Loaded`,扫描仍得到 `Scanned(Ok(..))`;写 warn 日志而不是无声吞掉 → Task 4 测试。
- `save` 失败时 `panel.save_error` 文本逐字为 `"未保存，无法用于下次比较：{服务文本}"` → Task 4 测试。
- `impl CodeHealthService for Client` 必须调 inherent 方法而不是递归调 trait 方法;`Option` 返回原样带回 → Task 2 适配测试。

---

### Task 1: `CodeHealthService` trait(`dozer-services`)

**Files:**
- Modify: `crates/dozer-services/src/lib.rs`

**Interfaces:**
- Consumes: `dozer_model::CodeHealthReportInfo`(已在 `dozer-model`)。
- Produces: `pub trait CodeHealthService: Send + Sync`,返回 `BoxFuture<'_, Result<_, ServiceError>>`,参数按值传:
  - `get_code_health_report(project_id: i64) -> Option<CodeHealthReportInfo>`
  - `list_code_health_reports(project_id: i64, limit: u32) -> Vec<CodeHealthReportInfo>`
  - `save_code_health_report(project_id: i64, info: CodeHealthReportInfo) -> ()`

- [ ] **Step 0: 记录基线(只测量,写进 ledger;同步执行,跑完再进 Step 1,避免并发编译污染)**

```bash
W=$(ls -d .superpowers/sdd/*/ | head -1)
grep -c 'dozer_client\|io\.client()' crates/dozer-app/src/extensions/codehealth/aggregate.rs     # 期望 3
cargo test --workspace --no-fail-fast > ${W}baseline-tests.txt 2>&1; grep -E '^test result' ${W}baseline-tests.txt | awk '{s+=$4;f+=$6} END{print "passed",s,"failed",f}'; grep -E '^test .* FAILED' ${W}baseline-tests.txt
cargo clippy --all-targets 2>&1 | grep -c '^warning'
python3 scripts/audit/check_panel_boundary.py; echo exit=$?
```
Expected: 失败仅已知的;门禁仅 `R-HOST-EXECUTORS 0 -> 1`;把数字写进 ledger。

- [ ] **Step 1: 写失败测试**

在 `crates/dozer-services/src/lib.rs` 现有 `#[cfg(test)] mod tests` 末尾追加(`unit()` 等辅助已存在):

```rust
    // ---- CodeHealthService 契约 ----

    use dozer_model::CodeHealthReportInfo;

    struct MemCodeHealth(Result<Option<CodeHealthReportInfo>, ServiceError>);

    impl CodeHealthService for MemCodeHealth {
        fn get_code_health_report(
            &self,
            _p: i64,
        ) -> BoxFuture<'_, Result<Option<CodeHealthReportInfo>, ServiceError>> {
            let r = self.0.clone();
            Box::pin(async move { r })
        }
        fn list_code_health_reports(
            &self,
            _p: i64,
            _l: u32,
        ) -> BoxFuture<'_, Result<Vec<CodeHealthReportInfo>, ServiceError>> {
            Box::pin(async { Ok(vec![]) })
        }
        fn save_code_health_report(
            &self,
            _p: i64,
            _i: CodeHealthReportInfo,
        ) -> BoxFuture<'_, Result<(), ServiceError>> {
            unit()
        }
    }

    /// `handle.spawn` 要 `'static + Send`:持有 `Arc<dyn CodeHealthService>` 的 async 块必须满足。
    #[test]
    fn dyn_code_health_futures_run_inside_tokio_spawn() {
        let svc: Arc<dyn CodeHealthService> = Arc::new(MemCodeHealth(Ok(None)));
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let (got, list) = rt.block_on(async move {
            tokio::spawn(async move {
                let got = svc.get_code_health_report(7).await;
                let list = svc.list_code_health_reports(7, 2).await;
                (got, list)
            })
            .await
            .unwrap()
        });
        assert_eq!(got, Ok(None));
        assert_eq!(list, Ok(vec![]));
    }

    #[test]
    fn code_health_read_failure_is_distinguishable_from_no_report() {
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let none = MemCodeHealth(Ok(None));
        let down = MemCodeHealth(Err(ServiceError::new(ServiceErrorKind::Unavailable, "x")));
        assert_eq!(rt.block_on(none.get_code_health_report(1)), Ok(None));
        assert_eq!(
            rt.block_on(down.get_code_health_report(1)).unwrap_err().kind,
            ServiceErrorKind::Unavailable
        );
    }
```
(若 `CodeHealthReportInfo` 已在测试模块顶部 import 过,去掉重复的 `use`。)

Run: `cargo test -p dozer-services 2>&1 | tail -8` → Expected: 编译失败 `cannot find trait CodeHealthService`(RED)。

- [ ] **Step 2: 实现**

顶部 `use dozer_model::{...}` 合并追加 `CodeHealthReportInfo`。在 `ConversationService` 之后、`Services` 之前加:

```rust
/// 代码健康度面板的落盘读写。参数按值传。
pub trait CodeHealthService: Send + Sync {
    fn get_code_health_report(
        &self,
        project_id: i64,
    ) -> BoxFuture<'_, Result<Option<CodeHealthReportInfo>, ServiceError>>;
    fn list_code_health_reports(
        &self,
        project_id: i64,
        limit: u32,
    ) -> BoxFuture<'_, Result<Vec<CodeHealthReportInfo>, ServiceError>>;
    fn save_code_health_report(
        &self,
        project_id: i64,
        info: CodeHealthReportInfo,
    ) -> BoxFuture<'_, Result<(), ServiceError>>;
}
```
**不要**在本任务给 `Services` 加字段(Task 3 做,否则 `dozer-app` 编译失败)。

Run: `cargo test -p dozer-services 2>&1 | grep -E 'test result|error'` → Expected: 全绿(原 12 个 + 新 2 个 = 14 passed)。

- [ ] **Step 3: 依赖验收与提交**

```bash
cargo tree -p dozer-services --edges normal | grep -vE 'dozer-model|serde|proc-macro2|quote|syn|unicode-ident|dozer-services' | head   # 期望无输出
cargo fmt -p dozer-services && cargo clippy -p dozer-services --all-targets 2>&1 | grep -c '^warning'   # 期望 0
git add crates/dozer-services && git commit -m "feat(services): CodeHealth S1 CodeHealthService trait

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 2: `Client` 实现 `CodeHealthService`

**Files:**
- Modify: `crates/dozer-client/src/lib.rs`(impl + 测试模块 `code_health_service_tests`)

**Interfaces:**
- Consumes: Task 1;`into_service_error`、`BoxFuture`、`ServiceError`;`todo_service_tests::serve_once`(已 `pub(super)`)。
- Produces: `impl CodeHealthService for Client`(同名 inherent 方法不改)。

- [ ] **Step 1: 写失败测试**

先读 `Client::get_code_health_report` / `list_code_health_reports` / `save_code_health_report` 的 `match`(`dozer-client/src/lib.rs` 约 409–470 行)确认应答变体:`get` → `Reply::CodeHealthReport { report }`,`list` → `Reply::CodeHealthReports { reports }`,`save` → `Reply::Ok`。在文件末尾追加:

```rust
#[cfg(test)]
mod code_health_service_tests {
    use super::todo_service_tests::serve_once;
    use super::*;
    use dozer_services::{CodeHealthService, ServiceErrorKind};

    #[tokio::test]
    async fn get_returns_the_option_unchanged() {
        let sock = serve_once("ch-get", Some(Reply::CodeHealthReport { report: None }));
        let got = CodeHealthService::get_code_health_report(&Client::new(sock.clone()), 7).await;
        assert_eq!(got, Ok(None));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn list_comes_back_as_ok() {
        let sock = serve_once("ch-list", Some(Reply::CodeHealthReports { reports: vec![] }));
        let got = CodeHealthService::list_code_health_reports(&Client::new(sock.clone()), 7, 2).await;
        assert_eq!(got, Ok(vec![]));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn hang_up_is_unavailable_with_the_legacy_text() {
        let sock = serve_once("ch-hangup", None);
        let e = CodeHealthService::get_code_health_report(&Client::new(sock.clone()), 7)
            .await
            .unwrap_err();
        assert_eq!((e.kind, e.message.as_str()), (ServiceErrorKind::Unavailable, "daemon 断开"));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn error_reply_is_rejected_with_the_legacy_prefix() {
        let sock = serve_once("ch-reject", Some(Reply::Error { message: "boom".into() }));
        let e = CodeHealthService::list_code_health_reports(&Client::new(sock.clone()), 7, 2)
            .await
            .unwrap_err();
        assert_eq!((e.kind, e.message.as_str()), (ServiceErrorKind::Rejected, "daemon 错误: boom"));
        let _ = std::fs::remove_file(&sock);
    }
}
```
(`save` 的 `()` 返回通过 Task 4 的扫描测试经假实现覆盖;Client 层不再单测它——`save_code_health_report` 内部构造请求体较重,其 `Reply::Ok` 分支与 `delete_category` 等同形。)

Run: `cargo test -p dozer-client code_health_service_tests 2>&1 | grep -E '^error' | head -3` → Expected: 编译失败(`Client: CodeHealthService` 未实现)(RED)。

- [ ] **Step 2: 实现**

顶部 `use dozer_services::{...}` 追加 `CodeHealthService`。在 `impl ConversationService for Client` 之后追加(用 `Client::method(self, ..)` 路径调 inherent 方法):

```rust
impl CodeHealthService for Client {
    fn get_code_health_report(
        &self,
        project_id: i64,
    ) -> BoxFuture<'_, Result<Option<dozer_core::protocol::CodeHealthReportInfo>, ServiceError>> {
        Box::pin(async move {
            Client::get_code_health_report(self, project_id)
                .await
                .map_err(into_service_error)
        })
    }
    fn list_code_health_reports(
        &self,
        project_id: i64,
        limit: u32,
    ) -> BoxFuture<'_, Result<Vec<dozer_core::protocol::CodeHealthReportInfo>, ServiceError>> {
        Box::pin(async move {
            Client::list_code_health_reports(self, project_id, limit)
                .await
                .map_err(into_service_error)
        })
    }
    fn save_code_health_report(
        &self,
        project_id: i64,
        info: dozer_core::protocol::CodeHealthReportInfo,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            Client::save_code_health_report(self, project_id, &info)
                .await
                .map_err(into_service_error)
        })
    }
}
```
类型路径以文件里已有 import 为准(若已 `use dozer_core::protocol::CodeHealthReportInfo` 则去前缀)。

Run: `cargo test -p dozer-client 2>&1 | grep -E 'test result|FAILED|^error'` → Expected: 原有测试 + 4 个新测试全绿。

- [ ] **Step 3: 提交**

```bash
cargo fmt -p dozer-client
cargo clippy -p dozer-client --all-targets 2>&1 | grep -c '^warning'   # 期望 0
git checkout Cargo.lock 2>/dev/null
git add crates/dozer-client && git status --short
git commit -m "feat(client): CodeHealth S2 Client 实现 CodeHealthService

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 3: `Services.codehealth` + 默认适配 + `FakeCodeHealth`

**Files:**
- Modify: `crates/dozer-services/src/lib.rs`(`Services` 加字段;测试里的构造)
- Modify: `crates/dozer-app/src/panel_host.rs`(`services_for`)
- Modify: `crates/dozer-app/src/service_fakes.rs`(新增 `FakeCodeHealth`;`base_services()` 补字段)

**Interfaces:**
- Consumes: Task 1/2。
- Produces:
  - `Services { usage, todo, project, memory, group_chat, conversation, codehealth: Arc<dyn CodeHealthService> }`;`services_for(&Client)` 也适配 `codehealth`。
  - `FakeCodeHealth`(`Clone`,克隆共享状态,`Default` = 全部成功:get → `Ok(None)`、list → `Ok(vec![])`、save → `Ok(())`),链式设置:`.with_latest(CodeHealthReportInfo)`(get 返回 `Some`)、`.with_get_error(kind, message)`、`.with_list_error(kind, message)`、`.with_save_error(kind, message)`;`calls() -> Vec<String>`(格式 `"get_code_health_report(7)"`、`"list_code_health_reports(7,2)"`、`"save_code_health_report(7)"`);`into_services()`(`Services { codehealth: Arc::new(self), ..base_services() }`)。

- [ ] **Step 1: 写失败测试**

(a) `panel_host.rs` 测试模块(`default_services_route_conversation_…` 旁)追加:
```rust
    #[test]
    fn default_services_route_codehealth_through_the_client_adapter() {
        let rt = test_runtime();
        let (io, _rx) = recording_io(&rt);
        // recording_io 的 Client 指向不存在的 socket → Unavailable
        let e = rt
            .block_on(io.services().codehealth.get_code_health_report(1))
            .unwrap_err();
        assert_eq!(e.kind, dozer_services::ServiceErrorKind::Unavailable);
    }
```
(b) `dozer-services` 的 `services_is_cloneable_and_shares_the_service` 同时构造 `codehealth: Arc::new(MemCodeHealth(Ok(None)))` 并追加 `Arc::ptr_eq(&s.codehealth, &t.codehealth)`。

Run: `cargo test -p dozer-services 2>&1 | grep -E '^error' | head -3` → Expected: 编译失败(`Services` 没有字段 `codehealth`)(RED)。

- [ ] **Step 2: 实现**

1. `Services` 加 `pub codehealth: Arc<dyn CodeHealthService>,`;`services_for` 加 `codehealth: Arc::new(client.clone()),`。
2. `service_fakes.rs`:实现 `FakeCodeHealth`,**模式同已有的 `FakeConversation`/`FakeProject`**(`Arc<Mutex<State>>`,`State { latest: Option<CodeHealthReportInfo>, get_error/list_error/save_error: Option<ServiceError>, calls: Vec<String> }`;`get` 在 `get_error` 为 `Some` 时返回 `Err`,否则 `Ok(latest.clone())`;`list`/`save` 同理)。`base_services()` 补 `codehealth: Arc::new(FakeCodeHealth::default())`。
3. 新增的构造函数在 Task 4 之前未被使用时,加 `#[allow(dead_code)]`(Task 4 去掉)。

Run: `cargo test -p dozer-services 2>&1 | grep -E 'test result|^error'`;`cargo test -p dozer-app default_services_route_codehealth 2>&1 | grep -E 'test result|^error'` → Expected: 通过。

- [ ] **Step 3: 全 workspace 编译与提交**

```bash
cargo build --workspace 2>&1 | grep -E '^error'          # 期望无输出
cargo test -p dozer-app -- panel_host usage_probe 2>&1 | grep -E 'test result|FAILED'
cargo clippy --all-targets 2>&1 | grep -c '^warning'      # 期望 ≤ 基线
cargo fmt -p dozer-app -p dozer-services
git checkout Cargo.lock 2>/dev/null
git add crates/dozer-services crates/dozer-app/src && git status --short
git commit -m "feat(host): CodeHealth S3 Services.codehealth + FakeCodeHealth

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 4: 面板与 host 切换,缓存读取失败分档

> 说明(Ruling,计划阶段):`on_activate` 与 `spawn_load_cached`、`spawn_scan` 的签名一变,面板与 host 的调用点必须在同一个提交里改;`LoadFailed` 是新消息,其 host 路由并入已有 `Loaded` 的专属臂。另:现有测试 `on_activate_only_loads_the_cached_report_and_never_starts_a_scan` 用指向不存在 socket 的 `Client` 并断言 `Loaded(7, _)`——迁移后同样的环境会得到 `LoadFailed`,所以该测试改为注入 `FakeCodeHealth` 的成功场景(新增的失败测试覆盖 `LoadFailed`)。

**Files:**
- Modify: `crates/dozer-app/src/extensions/codehealth/mod.rs`(`LOG` 声明、`Message::LoadFailed`、`WorkspaceState` 的 `outbox`/`last_load_failure`/`take_outbox`、冷却常量、`update` 分支、测试;把测试模块里的 `sample_report` 改为 `pub(crate)`)
- Modify: `crates/dozer-app/src/extensions/codehealth/aggregate.rs`(`spawn_load_cached`、`spawn_scan`、`on_activate`、新增测试模块)
- Modify: `crates/dozer-app/src/workspace/state.rs`(`spawn_codehealth_scan`)
- Modify: `crates/dozer-app/src/app/update.rs`(`Loaded` 臂并入 `LoadFailed`;`drain_outboxes` 加一行)
- Modify: `crates/dozer-app/src/service_fakes.rs`(去掉临时 `#[allow(dead_code)]`)
- Modify: `crates/dozer-app/src/usage_probe/host.rs`(行号引用若漂移,只改行号)

**Interfaces:**
- Consumes: Task 3 的 `Services`、`services_for`、`FakeCodeHealth`;`ServiceError`/`ServiceErrorKind`。
- Produces:
  - `Message::LoadFailed(i64, dozer_services::ServiceErrorKind, String, Box<PanelState>)`
  - `spawn_load_cached(project_id: i64, project_path: PathBuf, service: Arc<dyn CodeHealthService>, handle: &Handle, emit: impl Fn(Message) + Send + 'static)`;`spawn_scan` 同形
  - `WorkspaceState::take_outbox(&mut self) -> Vec<crate::extensions::toast::Pending>`

- [ ] **Step 1: 写失败测试**

(a) `mod.rs` 的 `mod tests`:把 `fn sample_report()` 改为 `pub(crate) fn sample_report()`;追加 import `use dozer_services::ServiceErrorKind;`,并追加 `update` 层测试:

```rust
    fn failed(kind: ServiceErrorKind, text: &str, marker: u64) -> Message {
        Message::LoadFailed(
            7,
            kind,
            text.into(),
            Box::new(PanelState {
                scanned_at_ms: Some(marker),
                ..PanelState::default()
            }),
        )
    }

    #[test]
    fn first_load_failure_adopts_the_carried_panel() {
        let mut ws = WorkspaceState::default();
        update(&mut ws, failed(ServiceErrorKind::Unavailable, "daemon 断开", 1));
        assert_eq!(ws.panel.scanned_at_ms, Some(1), "从未加载过:采用携带的 panel(含 git 数据),同现状");
        assert!(ws.take_outbox().is_empty(), "Unavailable 由顶栏徽标表达,不弹 Toast");
    }

    #[test]
    fn a_load_failure_keeps_the_report_already_shown() {
        let mut ws = WorkspaceState::default();
        ws.panel = PanelState {
            report: Some(sample_report()),
            scanned_at_ms: Some(9),
            ..PanelState::default()
        };
        update(&mut ws, failed(ServiceErrorKind::Unavailable, "daemon 断开", 1));
        assert!(ws.report().is_some(), "读失败不得把已显示的报告换成空面板");
        assert_eq!(ws.panel.scanned_at_ms, Some(9));
    }

    #[test]
    fn repeated_rejected_load_failures_toast_once_per_text_with_a_key() {
        let mut ws = WorkspaceState::default();
        for _ in 0..3 {
            update(&mut ws, failed(ServiceErrorKind::Rejected, "daemon 错误: boom", 1));
        }
        let toasts = ws.take_outbox();
        assert_eq!(toasts.len(), 1, "相同文本 60 秒内只提示一次");
        assert_eq!(toasts[0].level, crate::extensions::toast::Level::Error);
        assert!(toasts[0].text.contains("daemon 错误: boom"));
        assert_eq!(toasts[0].key.as_deref(), Some("codehealth-load"));
        update(&mut ws, failed(ServiceErrorKind::Protocol, "另一种错误", 1));
        assert_eq!(ws.take_outbox().len(), 1, "换了文本要再提示");
    }
```
并把现有测试 `on_activate_only_loads_the_cached_report_and_never_starts_a_scan` 在 `offline_ctx` 之后加一行 `let ctx = crate::panel_host::ActivationCtx { io: ctx.io.with_services(crate::service_fakes::FakeCodeHealth::default().into_services()), ..ctx };`(成功场景);再加一个失败场景测试:
```rust
    #[test]
    fn on_activate_reports_a_failed_cache_read_as_load_failed() {
        let rt = runtime();
        // 默认服务束指向不存在的 socket → get 失败 → LoadFailed(7, Unavailable, ..)
        let (ctx, rx) = offline_ctx::<Message>(&rt, 7, Some(std::env::temp_dir()));
        on_activate(&ctx);
        assert!(matches!(
            rx.recv_timeout(TIMEOUT).unwrap(),
            Message::LoadFailed(7, ServiceErrorKind::Unavailable, _, _)
        ));
    }
```
(b) `aggregate.rs` 末尾新增 `#[cfg(test)] mod tests`(`sample_report` 经 `super::tests::sample_report` 访问;`runtime`/`TIMEOUT` 来自 `crate::panel_host::testing`):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::panel_host::testing::{TIMEOUT, runtime};
    use crate::service_fakes::FakeCodeHealth;
    use dozer_services::{ServiceError, ServiceErrorKind};
    use std::sync::{Mutex, mpsc};

    fn collect() -> (impl Fn(Message) + Send + 'static, mpsc::Receiver<Message>) {
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        (move |m| { let _ = tx.lock().unwrap().send(m); }, rx)
    }

    #[test]
    fn a_cached_report_loads_into_the_panel() {
        let rt = runtime();
        let dir = tempfile::tempdir().unwrap();
        let info = to_info(&crate::extensions::codehealth::tests::sample_report());
        let fake = FakeCodeHealth::default().with_latest(info);
        let (emit, rx) = collect();
        spawn_load_cached(7, dir.path().to_path_buf(), fake.clone().into_services().codehealth, rt.handle(), emit);
        match rx.recv_timeout(TIMEOUT).unwrap() {
            Message::Loaded(7, panel) => assert!(panel.report.is_some()),
            other => panic!("unexpected: {other:?}"),
        }
        assert_eq!(
            fake.calls(),
            vec!["get_code_health_report(7)".to_string(), "list_code_health_reports(7,2)".to_string()]
        );
    }

    #[test]
    fn a_failed_cache_read_emits_load_failed_with_a_fallback_panel() {
        let rt = runtime();
        let dir = tempfile::tempdir().unwrap();
        let fake = FakeCodeHealth::default().with_get_error(ServiceErrorKind::Rejected, "daemon 错误: boom");
        let (emit, rx) = collect();
        spawn_load_cached(7, dir.path().to_path_buf(), fake.into_services().codehealth, rt.handle(), emit);
        match rx.recv_timeout(TIMEOUT).unwrap() {
            Message::LoadFailed(7, kind, text, panel) => {
                assert_eq!((kind, text.as_str()), (ServiceErrorKind::Rejected, "daemon 错误: boom"));
                assert!(panel.report.is_none());
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn a_failed_history_read_still_loads_the_report() {
        let rt = runtime();
        let dir = tempfile::tempdir().unwrap();
        let info = to_info(&crate::extensions::codehealth::tests::sample_report());
        let fake = FakeCodeHealth::default()
            .with_latest(info)
            .with_list_error(ServiceErrorKind::Unavailable, "daemon 断开");
        let (emit, rx) = collect();
        spawn_load_cached(7, dir.path().to_path_buf(), fake.into_services().codehealth, rt.handle(), emit);
        match rx.recv_timeout(TIMEOUT).unwrap() {
            Message::Loaded(7, panel) => {
                assert!(panel.report.is_some());
                assert!(panel.previous_report.is_none(), "历史读失败:对比降级为无基线");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    /// 一个能被扫描的最小项目目录。
    fn tiny_project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").unwrap();
        dir
    }

    #[test]
    fn a_failed_save_text_is_prefix_plus_the_service_message_unchanged() {
        let rt = runtime();
        let dir = tiny_project();
        let fake = FakeCodeHealth::default().with_save_error(ServiceErrorKind::Rejected, "daemon 错误: boom");
        let (emit, rx) = collect();
        spawn_scan(7, dir.path().to_path_buf(), fake.into_services().codehealth, rt.handle(), emit);
        match rx.recv_timeout(TIMEOUT * 6).unwrap() {
            Message::Scanned(7, Ok(panel)) => assert_eq!(
                panel.save_error.as_deref(),
                Some("未保存，无法用于下次比较：daemon 错误: boom")
            ),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn a_failed_history_read_does_not_block_the_scan() {
        let rt = runtime();
        let dir = tiny_project();
        let fake = FakeCodeHealth::default().with_list_error(ServiceErrorKind::Unavailable, "daemon 断开");
        let (emit, rx) = collect();
        spawn_scan(7, dir.path().to_path_buf(), fake.clone().into_services().codehealth, rt.handle(), emit);
        match rx.recv_timeout(TIMEOUT * 6).unwrap() {
            Message::Scanned(7, Ok(panel)) => {
                assert!(panel.report.is_some());
                assert!(panel.save_error.is_none());
            }
            other => panic!("unexpected: {other:?}"),
        }
        assert!(fake.calls().iter().any(|c| c == "save_code_health_report(7)"), "{:?}", fake.calls());
    }
}
```
(`ServiceError` 若未被用到,从 import 里去掉,以免 unused 警告。)

Run: `cargo test -p dozer-app extensions::codehealth 2>&1 | grep -E '^error' | head -6` → Expected: 编译失败(`LoadFailed` 不存在、`take_outbox` 不存在、`spawn_*` 参数类型不符)(RED)。

- [ ] **Step 2: 面板侧实现**

`extensions/codehealth/mod.rs`:
1. 加 `dozer_core::scope!(pub(crate) LOG, panel, "code_health");`(放在其它 `use`/`mod` 之后);`use dozer_services::ServiceErrorKind;`。
2. `Message` 加(带文档注释,放在 `Loaded` 之后):
```rust
    /// 缓存读取失败(`get` 失败):携带照旧算出的"无报告"面板(含 git 数据)。从未成功加载过则采用它;
    /// 已有报告则**保持现有面板**。`Unavailable` 静默(顶栏徽标已表达"dozerd 不可用");其余弹带 key 的
    /// Error Toast,相同文本 60 秒内只提示一次。
    LoadFailed(i64, ServiceErrorKind, String, Box<PanelState>),
```
3. `WorkspaceState` 加字段(`Default` 仍成立):`outbox: crate::extensions::toast::Outbox`、`last_load_failure: Option<(String, std::time::Instant)>`;加 `pub fn take_outbox(&mut self) -> Vec<crate::extensions::toast::Pending> { self.outbox.take() }` 与常量 `pub(crate) const LOAD_FAILURE_TOAST_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(60);`。
4. `update` 加分支:
```rust
        Message::LoadFailed(_project_id, kind, message, fallback) => {
            if ws_state.panel.report.is_none() {
                ws_state.panel = *fallback;
            }
            if kind == ServiceErrorKind::Unavailable {
                dozer_core::log_debug!(LOG, "代码健康度缓存读取失败(dozerd 不可用): {message}");
            } else {
                let now = std::time::Instant::now();
                let repeat = matches!(
                    &ws_state.last_load_failure,
                    Some((last, at)) if *last == message
                        && now.duration_since(*at) < LOAD_FAILURE_TOAST_COOLDOWN
                );
                if !repeat {
                    ws_state.outbox.push_keyed(
                        LOG,
                        crate::extensions::toast::Level::Error,
                        format!("代码健康度加载失败: {message}"),
                        "codehealth-load",
                    );
                    ws_state.last_load_failure = Some((message, now));
                }
            }
        }
```

`extensions/codehealth/aggregate.rs`:
1. 顶部 `use dozer_services::CodeHealthService; use std::sync::Arc; use super::LOG;`。
2. `spawn_load_cached`:
```rust
pub fn spawn_load_cached(
    project_id: i64,
    project_path: PathBuf,
    service: Arc<dyn CodeHealthService>,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    handle.spawn(async move {
        let latest = service.get_code_health_report(project_id).await;
        let history = match service.list_code_health_reports(project_id, 2).await {
            Ok(h) => h,
            Err(e) => {
                dozer_core::log_warn!(LOG, error = %e, "读取历史健康度报告失败,对比降级为无基线");
                Vec::new()
            }
        };
        let (load_error, latest) = match latest {
            Ok(l) => (None, l),
            Err(e) => (Some(e), None),
        };

        let current = latest.and_then(|info| {
            let ts = info.scanned_at_ms;
            parse_report(&info.report_json).map(|r| (r, Some(ts)))
        });
        let previous = history
            .get(1)
            .and_then(|info| parse_report(&info.report_json));

        let panel = tokio::task::spawn_blocking(move || {
            finish_panel(&project_path, current, previous, None)
        })
        .await
        .unwrap_or_default();

        match load_error {
            None => emit(Message::Loaded(project_id, Box::new(panel))),
            Some(e) => emit(Message::LoadFailed(project_id, e.kind, e.message, Box::new(panel))),
        }
    });
}
```
`on_activate`:`ctx.io.client()` → `ctx.io.services().codehealth.clone()`。
3. `spawn_scan`:参数 `client: &dozer_client::Client` → `service: Arc<dyn CodeHealthService>`,去掉 `let client = client.clone();`;
   - 读历史:`let history = match service.list_code_health_reports(project_id, 2).await { Ok(h) => h, Err(e) => { dozer_core::log_warn!(LOG, error = %e, "读取上一份健康度报告失败,本次扫描不含对比"); Vec::new() } };`
   - 保存:`service.save_code_health_report(project_id, info).await.err().map(|e| format!("未保存，无法用于下次比较：{e}"))`(`info` 按值;`e` 的 `Display` 即 message,文本不变)。

- [ ] **Step 3: host 侧实现**

- `workspace/state.rs`(约 886 行):`codehealth::spawn_scan(project_id, project_path, &io.client, &io.handle, emit);` → `codehealth::spawn_scan(project_id, project_path, crate::panel_host::services_for(&io.client).codehealth, &io.handle, emit);`。
- `app/update.rs` 约 1653 行,把
  `Message::CodeHealth(msg @ codehealth::Message::Loaded(project_id, ..)) => {` 改为
  `Message::CodeHealth(msg @ (codehealth::Message::Loaded(project_id, ..) | codehealth::Message::LoadFailed(project_id, ..))) => {`(rustfmt 会折行,以格式化结果为准);**不新增臂**。
- `drain_outboxes`(约 214–222 行):在 `pending.extend(ws.conversations.take_outbox());` 之后加 `pending.extend(ws.codehealth.take_outbox());`。
- 去掉 `service_fakes.rs` 里 Task 3 临时加的 `#[allow(dead_code)]`。

- [ ] **Step 4: 运行面板测试**

```bash
cargo test -p dozer-app extensions::codehealth 2>&1 | grep -E 'test result|FAILED|^error'
```
Expected: 全绿(含 3 个 `update` 层测试、1 个失败 `on_activate` 测试、5 个 `aggregate` 测试;既有测试保持通过)。若 `tiny_project` 的扫描在测试里过慢或返回 `Failed`,把项目目录里加几个文件或延长超时,**不要**改扫描逻辑。

- [ ] **Step 5: 探针守卫、全量、门禁、提交**

```bash
cargo test -p dozer-app usage_probe::host 2>&1 | grep -E 'test result|FAILED|实际是'   # drain_outboxes 多一行(且 Loaded 臂可能折行),host_site 行号会整体漂移:用 grep -n 重新定位后只改 host_site() 与注释里的行号,不改守卫逻辑
W=$(ls -d .superpowers/sdd/*/ | head -1)
cargo test --workspace --no-fail-fast > ${W}t4.txt 2>&1; grep -E '^test result' ${W}t4.txt | awk '{s+=$4;f+=$6} END{print s,f}'; grep -E '^test .* FAILED' ${W}t4.txt
cargo clippy --all-targets 2>&1 | grep -c '^warning'
python3 scripts/audit/check_panel_boundary.py        # 只许剩 R-HOST-EXECUTORS 0 -> 1;R-HOST-PANEL-ARMS 不得上升
bash scripts/check-log-scope.sh | tail -1
cargo fmt -p dozer-app && cargo fmt --check && echo fmt-ok
```
Expected: 失败仅已知的;clippy 警告数 ≤ 基线;门禁无新增回退;log scope ok;`app/state.rs` 的"面板日志来源名必须是 PanelKind 面板名"测试通过。
```bash
git checkout Cargo.lock 2>/dev/null; git add crates/dozer-app/src && git status --short
git commit -m "feat(codehealth): CodeHealth S4 面板改用 CodeHealthService,缓存读取失败保留报告并分档提示

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 5: `R-CLIENT` 基线下降、验收与文档回填

**Files:**
- Modify: `scripts/audit/panel-boundary.baseline.json`(仅 `extensions/codehealth/*` 的 `R-CLIENT` 条目)
- Modify: `docs/dozer-v2/bytehost-H0/08-client-use-inventory.md`、`CLAUDE.md`

**Interfaces:**
- Produces: `extensions/codehealth/**` 不再出现在 `R-CLIENT` 基线里。

- [ ] **Step 1: 确认门禁现状并下调基线**

```bash
grep -rn 'dozer_client\|io\.client()' crates/dozer-app/src/extensions/codehealth/ | head -3   # 期望无输出
python3 scripts/audit/check_panel_boundary.py   # 此时 codehealth 的 R-CLIENT 实测应为 0(低于基线),门禁只显示既有 R-HOST-EXECUTORS
python3 - <<'EOF'
import json
p = "scripts/audit/panel-boundary.baseline.json"
base = json.load(open(p, encoding="utf-8"))
for rel in [r for r in list(base) if r.startswith("extensions/codehealth/")]:
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
Expected: 基线 diff 只含 `extensions/codehealth/*` 条目的删除;门禁仍只报 `R-HOST-EXECUTORS 0 -> 1`。

- [ ] **Step 2: 验收清单(逐条跑、读输出)**

```bash
cargo tree -p dozer-services --edges normal                    # 1: 仅 dozer-model + serde 系
python3 scripts/audit/unit_edges.py ext:codehealth             # 2: 无 dozer_client
cargo test -p dozer-app extensions::codehealth 2>&1 | grep -E 'test result|FAILED'   # 3
cargo test -p dozer-core --test model_golden 2>&1 | grep 'test result'               # 5
W=$(ls -d .superpowers/sdd/*/ | head -1)
cargo test --workspace --no-fail-fast > ${W}final.txt 2>&1; grep -E '^test result' ${W}final.txt | awk '{s+=$4;f+=$6} END{print s,f}'; grep -E '^test .* FAILED' ${W}final.txt   # 4
cargo clippy --all-targets 2>&1 | grep -c '^warning'; cargo fmt --check && echo fmt-ok
bash scripts/check-log-scope.sh && echo log-scope-ok
git status --short                                             # 无 Info.plist、无无关 Cargo 改动
```
任一不满足即不收尾,回到对应任务修。

- [ ] **Step 3: 文档回填并提交**

- `docs/dozer-v2/bytehost-H0/08-client-use-inventory.md` 末尾追加一节"CodeHealth 迁移(2026-10-11)":`extensions/codehealth/**` 经 `CodeHealthService`(`dozer-services`)取数与保存;`R-CLIENT` 基线中 `codehealth/aggregate.rs` 已清零;缓存读取失败改为 `LoadFailed`(已有报告保留、首次失败采用携带的无报告面板、`Unavailable` 静默、其余弹 `codehealth-load` Toast,同文本 60 秒内一次);历史读取失败改为写 warn 日志(对比降级为无基线);保存失败的内联提示文本不变。**登记后续项**:`Loaded`/`Scanned` 没有代数保护;读失败冷却逻辑在 Todo/Project/Conversations/CodeHealth 各有一份,可抽到 `toast::Outbox`;剩余直接用 `Client` 的面板:`agent_context`、`browser`、`edit_history`、`settings`、`project_create`。
- `CLAUDE.md` Workspace 布局表 `dozer-services` 行里"Usage、Todo、Project、GroupChat、Conversations 已迁,其余面板待迁"改为"Usage、Todo、Project、GroupChat、Conversations、CodeHealth 已迁,其余面板待迁"。
```bash
git add scripts/audit docs CLAUDE.md
git commit -m "chore(audit): CodeHealth S5 R-CLIENT 基线下调 + 文档回填

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

之后按 executing-plans 的 Final Review(整分支 fresh reviewer)与 finishing-a-development-branch;**合并进 main 属于需先询问用户的外部副作用**,合并前核对 main 上用户 WIP(`Cargo.lock`、`crates/dozer-app/Cargo.toml`、`Info.plist` 等)与本分支的重叠,用 `git stash` → 合并 → `git stash pop`。
