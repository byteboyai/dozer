# Settings 对话框迁移到服务 trait Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 设置弹窗(`extensions/settings.rs`)不再直接持有 `dozer_client::Client`:Advanced 页经新的 `DaemonService`,「应用」页对 `bytehost_client::AppHostApi` 泛型;用户可见行为零变化。完成后 `extensions/**` 的 `R-CLIENT` 整体归零。

**Architecture:** `dozer-services` 新增 `DaemonService`(`list_sessions`/`shutdown_daemon`/`ensure_running`)与 `Services.daemon`;`dozer-app` 新增适配器 `ClientDaemon(Client)`(`ensure_running` 调 `runtime::ensure_daemon`,因它要拉起进程,不能放 `dozer-client`);`settings::update`/`run_apps_message` 改吃 `Arc<dyn DaemonService>` 与泛型 `A: AppHostApi + Clone + Send + Sync + 'static`;host 唯一调用点传 `services_for(&client).daemon` 与 `client.clone()`。

**Tech Stack:** Rust 2024 workspace、tokio、iced 0.14(只涉及 spawn 层)、bytehost-client(`AppHostApi`,RPITIT)、Python 门禁脚本。

**Spec:** `docs/superpowers/specs/2026-10-11-settings-services-design.md`(先例:`2026-10-11-project-create-services-design.md`)

## Global Constraints

- `dozer-services` 只依赖 `dozer-model`;不加 `dozer-core`/`dozer-client`/`tokio`/`bytehost-*` 到 `[dependencies]`。
- 线上协议零变化:`Request`/`Reply` 不动,`crates/dozer-core/tests/model_golden.rs` 不改且通过;`Client::list`/`shutdown_daemon` 的 inherent 签名不改;`bytehost-*` 的 wire 形状不动。
- 错误文本逐字不变:`AdvancedSessionCountReady`/`AdvancedStopResult` 的 `Err` 文本仍等于原 `e.to_string()`(现为 `ServiceError.message`);`AdvancedRestartResult` 的 `Err` 文本仍等于 `ensure_daemon` 返回的原 `String`;「应用」页失败类别(`Failure::Host`/`Transport`)与文本不变,`AppId::new` 失败仍走 `Failure::Transport(format!("{e}"))` 且**不发请求**。
- `ensure_daemon` 留在 `runtime.rs`,不搬进 `dozer-client`;`ClientDaemon::ensure_running` 在测试里**不调用**(会真拉起 dozerd 进程)。
- `R-HOST-PANEL-ARMS` 不上升:不新增 `app/update.rs` 的面板专属臂。
- 在独立 worktree 实施;不提交用户 WIP(`Info.plist`、版本号相关改动);`Cargo.lock` 提交前 `git checkout Cargo.lock`(main 上已提交的锁文件缺 bytegit/byteui 的 `source` 行,干净 worktree 里跑 cargo 会补上,属噪声)。
- main 上已知问题不修不算回归:`files::tests::delete_confirm_spec_reflects_pending_target` 失败、并行负载下的间歇失败(`git_log::build_*`、`assets::tests::serves_vendored_asset_with_mime`、dozerd 的 `memory`/`headless_agent` 测试)、门禁 `R-HOST-EXECUTORS 0 -> 1`。
- 提交信息末尾加 `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`。

## Review Focus

- `AdvancedStopClicked` 统计的是 `alive` 会话数(死会话不计),且仅当 `advanced` 为 `Idle` 时才调服务;`AdvancedStopConfirm` 仅 `ConfirmingStop` 时调 `shutdown_daemon`;`AdvancedRestartClicked` 仅 `Stopped` 时调 `ensure_running` → Task 3 测试。
- 三条 Advanced 路径的失败文本逐字等于服务消息(不加前缀、不改写)→ Task 3 测试。
- 「应用」页 `Stop`/`Rollback`/`Uninstall`/`FetchLogs` 遇到非法 `AppId` 时得 `Failure::Transport` 且 `FakeApps` 收不到任何请求 → Task 3 测试。
- `Install` 把批准的计划与来源**原样**送进 `AppRequest::Install`(`approved` 与 `source` 不被改写)→ Task 3 测试。
- `ClientDaemon::list_sessions`/`shutdown_daemon` 在 socket 不存在时得 `Unavailable`(而不是 panic 或挂起);`ensure_running` 的 `String` 错误文本被原样保留到 `ServiceError.message` → Task 2 测试(后者用纯函数 `ensure_error_to_service` 测,不真拉起进程)。

---

### Task 1: `DaemonService` + `Services.daemon`(`dozer-services`)

**Files:**
- Modify: `crates/dozer-services/src/lib.rs`

**Interfaces:**
- Consumes: `dozer_model::SessionInfo`;`BoxFuture`、`ServiceError`。
- Produces:
  - `pub trait DaemonService: Send + Sync { fn list_sessions(&self) -> BoxFuture<'_, Result<Vec<SessionInfo>, ServiceError>>; fn shutdown_daemon(&self) -> BoxFuture<'_, Result<(), ServiceError>>; fn ensure_running(&self) -> BoxFuture<'_, Result<(), ServiceError>>; }`
  - `Services { …, pub daemon: Arc<dyn DaemonService> }`

- [ ] **Step 0: 记录基线(只测量,写进 ledger;同步执行,跑完再进 Step 1)**

```bash
W=$(ls -d .superpowers/sdd/*/ | head -1)
grep -c 'dozer_client' crates/dozer-app/src/extensions/settings.rs     # 记下数字(R-CLIENT 基线为 2)
cargo test --workspace --no-fail-fast > ${W}baseline-tests.txt 2>&1; grep -E '^test result' ${W}baseline-tests.txt | awk '{s+=$4;f+=$6} END{print "passed",s,"failed",f}'; grep -E '^test .* FAILED' ${W}baseline-tests.txt
cargo clippy --all-targets 2>&1 | grep -c '^warning'
python3 scripts/audit/check_panel_boundary.py; echo exit=$?
```
Expected: 失败仅已知的;门禁仅 `R-HOST-EXECUTORS 0 -> 1`;数字写进 ledger。

- [ ] **Step 1: 写失败测试**

在 `crates/dozer-services/src/lib.rs` 的 `#[cfg(test)] mod tests` 里追加(`SessionInfo` 的构造参考 `dozer-app` 里 `count_live_sessions_ignores_dead_sessions_in_registry_snapshot` 的字段全集,先 `grep -n "SessionInfo {" -A14 crates/dozer-app/src/extensions/settings.rs` 复制;这里用 `Vec::new()` 避开构造):

```rust
    struct MemDaemon {
        list: Result<Vec<SessionInfo>, ServiceError>,
        stop: Result<(), ServiceError>,
        ensure: Result<(), ServiceError>,
    }
    impl DaemonService for MemDaemon {
        fn list_sessions(&self) -> BoxFuture<'_, Result<Vec<SessionInfo>, ServiceError>> {
            let r = self.list.clone();
            Box::pin(async move { r })
        }
        fn shutdown_daemon(&self) -> BoxFuture<'_, Result<(), ServiceError>> {
            let r = self.stop.clone();
            Box::pin(async move { r })
        }
        fn ensure_running(&self) -> BoxFuture<'_, Result<(), ServiceError>> {
            let r = self.ensure.clone();
            Box::pin(async move { r })
        }
    }

    /// `handle.spawn` 要 `'static + Send`:持有 `Arc<dyn DaemonService>` 的 async 块必须满足。
    #[test]
    fn dyn_daemon_service_futures_run_inside_tokio_spawn() {
        let svc: Arc<dyn DaemonService> = Arc::new(MemDaemon {
            list: Ok(Vec::new()),
            stop: Ok(()),
            ensure: Ok(()),
        });
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let (l, s, e) = rt.block_on(async move {
            tokio::spawn(async move {
                (svc.list_sessions().await, svc.shutdown_daemon().await, svc.ensure_running().await)
            })
            .await
            .unwrap()
        });
        assert_eq!((l, s, e), (Ok(Vec::new()), Ok(()), Ok(())));
    }

    #[test]
    fn daemon_list_failure_is_distinguishable_from_no_sessions() {
        let ok = MemDaemon { list: Ok(Vec::new()), stop: Ok(()), ensure: Ok(()) };
        let down = MemDaemon {
            list: Err(ServiceError::new(ServiceErrorKind::Unavailable, "daemon 断开")),
            stop: Ok(()),
            ensure: Ok(()),
        };
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        assert_eq!(rt.block_on(ok.list_sessions()), Ok(Vec::new()));
        assert_eq!(rt.block_on(down.list_sessions()).unwrap_err().kind, ServiceErrorKind::Unavailable);
    }
```
Run: `cargo test -p dozer-services 2>&1 | grep -E '^error' | head -3` → Expected: 编译失败(`DaemonService` 未定义)(RED)。

- [ ] **Step 2: 实现**

1. `use dozer_model::{…}` 加入 `SessionInfo`。
2. 在 `BookmarkService` 之后追加:
```rust
/// dozerd 自身的生命周期(设置 → 高级)。参数无,future 不借用调用方局部变量。
/// `ensure_running`:daemon 已响应则直接成功,否则拉起并重试;拉起进程属宿主职责,
/// 因此生产实现在 `dozer-app`,不在 `dozer-client`。
pub trait DaemonService: Send + Sync {
    fn list_sessions(&self) -> BoxFuture<'_, Result<Vec<SessionInfo>, ServiceError>>;
    fn shutdown_daemon(&self) -> BoxFuture<'_, Result<(), ServiceError>>;
    fn ensure_running(&self) -> BoxFuture<'_, Result<(), ServiceError>>;
}
```
3. `Services` 增加 `pub daemon: Arc<dyn DaemonService>,`。

Run: `cargo test -p dozer-services 2>&1 | grep -E 'test result|^error'` → Expected: `dozer-services` 全绿(原有 + 新 2 个)。此时 `cargo build --workspace` 会因 `Services` 构造点(`services_for`、`base_services`)缺 `daemon` 字段而失败,属预期,Task 2 补上。

- [ ] **Step 3: 依赖验收与提交**

```bash
cargo tree -p dozer-services --edges normal | grep -vE 'dozer-model|serde|proc-macro2|quote|syn|unicode-ident|dozer-services' | head   # 期望无输出
cargo fmt -p dozer-services && cargo clippy -p dozer-services --all-targets 2>&1 | grep -c '^warning'   # 期望 0
git checkout Cargo.lock 2>/dev/null
git add crates/dozer-services && git commit -m "feat(services): Settings S1 DaemonService 与 Services.daemon

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```
(本提交之后到 Task 2 完成之前 workspace 不能编译;Task 1–2 连续执行。)

---

### Task 2: `ClientDaemon` 适配器、`services_for`、`FakeDaemon`

**Files:**
- Modify: `crates/dozer-app/src/panel_host.rs`(`ClientDaemon` 与 `services_for`)
- Modify: `crates/dozer-app/src/service_fakes.rs`(`FakeDaemon`、`base_services`)

**Interfaces:**
- Consumes: Task 1;`dozer_client::{Client, into_service_error}`;`crate::runtime::ensure_daemon(&Client) -> Result<(), String>`。
- Produces:
  - `pub(crate) struct ClientDaemon(pub(crate) Client)`,`impl DaemonService for ClientDaemon`。
  - `fn ensure_error_to_service(msg: String) -> ServiceError`(`Unavailable`,文本原样),模块私有、测试可见。
  - `service_fakes::FakeDaemon`:`Default`(`list: Ok(vec![])`、`stop: Ok(())`、`ensure: Ok(())`)、`Clone`;链式 `.with_sessions(Result<Vec<SessionInfo>, ServiceError>)`、`.with_stop(Result<(), ServiceError>)`、`.with_ensure(Result<(), ServiceError>)`;`.calls() -> Vec<String>`,记录 `"list_sessions()"`、`"shutdown_daemon()"`、`"ensure_running()"`;`.into_services() -> Services`(其余字段取 `base_services()`)。
  - `service_fakes::session(id: &str, alive: bool) -> SessionInfo`(字段取 `count_live_sessions` 测试里那份全集)。

- [ ] **Step 1: 写失败测试**

RED 一:`cargo build -p dozer-app 2>&1 | grep -E '^error' | head -3` → Expected: 编译失败(`Services` 缺 `daemon` 字段,两个构造点)。

RED 二:在 `panel_host.rs` 的测试模块(没有就在文件末尾新建 `#[cfg(test)] mod daemon_tests`)追加:

```rust
    use dozer_services::{DaemonService, ServiceErrorKind};

    fn offline() -> ClientDaemon {
        ClientDaemon(Client::new(std::path::PathBuf::from(
            "/tmp/dozer-settings-daemon-nonexistent.sock",
        )))
    }

    #[test]
    fn client_daemon_reports_unavailable_when_the_socket_is_missing() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let d = offline();
        let e = rt.block_on(d.list_sessions()).unwrap_err();
        assert_eq!(e.kind, ServiceErrorKind::Unavailable);
        let e = rt.block_on(d.shutdown_daemon()).unwrap_err();
        assert_eq!(e.kind, ServiceErrorKind::Unavailable);
    }

    #[test]
    fn ensure_error_text_is_preserved_verbatim() {
        let e = ensure_error_to_service("无法连接 dozerd（已重试 3 次）：x".into());
        assert_eq!(e.kind, ServiceErrorKind::Unavailable);
        assert_eq!(e.message, "无法连接 dozerd（已重试 3 次）：x");
        assert_eq!(e.to_string(), "无法连接 dozerd（已重试 3 次）：x");
    }
```
(`client_daemon…` 测试**只**调 `list_sessions`/`shutdown_daemon`,**绝不**调 `ensure_running`。)

- [ ] **Step 2: 实现**

1. `panel_host.rs`:
```rust
/// 把 `Client` 适配成 `DaemonService`。`ensure_running` 要拉起 dozerd 进程(`runtime::ensure_daemon`),
/// 那段在 dozer-app,`dozer-client` 够不着,所以适配器放在这里。
pub(crate) struct ClientDaemon(pub(crate) Client);

fn ensure_error_to_service(msg: String) -> dozer_services::ServiceError {
    dozer_services::ServiceError::new(dozer_services::ServiceErrorKind::Unavailable, msg)
}

impl dozer_services::DaemonService for ClientDaemon {
    fn list_sessions(&self) -> dozer_services::BoxFuture<'_, Result<Vec<dozer_model::SessionInfo>, dozer_services::ServiceError>> {
        Box::pin(async move { self.0.list().await.map_err(dozer_client::into_service_error) })
    }
    fn shutdown_daemon(&self) -> dozer_services::BoxFuture<'_, Result<(), dozer_services::ServiceError>> {
        Box::pin(async move { self.0.shutdown_daemon().await.map_err(dozer_client::into_service_error) })
    }
    fn ensure_running(&self) -> dozer_services::BoxFuture<'_, Result<(), dozer_services::ServiceError>> {
        Box::pin(async move { crate::runtime::ensure_daemon(&self.0).await.map_err(ensure_error_to_service) })
    }
}
```
`services_for` 加 `daemon: Arc::new(ClientDaemon(client.clone())),`。导入以文件里已有的 `use` 为准(若已 `use dozer_services::…` 就去前缀)。

2. `service_fakes.rs`:`FakeDaemon`(`Arc<Mutex<DaemonState>>`,同 `FakeProject` 的写法)与 `session()`;`base_services()` 补 `daemon: Arc::new(FakeDaemon::default()),`。新增但 Task 3 前未用的构造/辅助加 `#[allow(dead_code)]`(Task 3 去掉)。

Run: `cargo build --workspace 2>&1 | grep -E '^error'` → Expected: 无输出。

- [ ] **Step 3: 验证与提交**

```bash
cargo test -p dozer-app panel_host 2>&1 | grep -E 'test result|FAILED'
cargo clippy --all-targets 2>&1 | grep -c '^warning'      # 期望 ≤ 基线
cargo fmt -p dozer-app -p dozer-services
git checkout Cargo.lock 2>/dev/null
git add crates/dozer-app/src && git status --short
git commit -m "feat(host): Settings S2 ClientDaemon 适配器 + FakeDaemon + services_for.daemon

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 3: `FakeApps`、`settings` 切换与 host 调用点

**Files:**
- Modify: `crates/dozer-app/src/service_fakes.rs`(`FakeApps`)
- Modify: `crates/dozer-app/src/extensions/settings.rs`(`update`、`run_apps_message`、测试)
- Modify: `crates/dozer-app/src/app/update.rs`(`settings::update` 调用点,约 3966–3972 行)
- Modify: `crates/dozer-app/src/usage_probe/host.rs`(行号引用若漂移,只改行号)

**Interfaces:**
- Consumes: Task 2 的 `FakeDaemon`、`session`;`bytehost_client::{AppHostApi, AppApiError}`、`bytehost_apps::proto::{AppRequest, AppReply}`。
- Produces:
  - `service_fakes::FakeApps`:`Clone`;`FakeApps::new(handler: impl Fn(&AppRequest) -> Result<AppReply, AppApiError> + Send + Sync + 'static)`;`FakeApps::unscripted()`(任何请求返回 `AppApiError::Transport("未预设")`);`.requests() -> Vec<String>`(每个请求的 `Debug` 文本,按到达顺序);`impl AppHostApi for FakeApps`(只实现 `request`/`subscribe`,`subscribe` 返回 `Err(Transport("未预设"))`)。
  - `pub fn update<A>(state: &mut Option<State>, msg: Message, daemon: Arc<dyn DaemonService>, apps: A, handle: &tokio::runtime::Handle, emit: impl Fn(Message) + Clone + Send + 'static) where A: AppHostApi + Clone + Send + Sync + 'static`
  - `fn run_apps_message<A>(s: &mut State, msg: settings_apps::Message, apps: A, handle: &Handle, emit: impl Fn(Message) + Send + 'static) where A: AppHostApi + Clone + Send + Sync + 'static`

- [ ] **Step 1: 写失败测试**

先读 `settings_apps::Message` / `Effect` 的变体名(`grep -n "pub enum Message" -A40 crates/dozer-app/src/extensions/settings_apps.rs`),下面测试里的 `settings_apps::Message::…` 按实际变体名调整;并确认 `settings::Message` 是否 `derive(Debug)`(下面 panic 用到 `{other:?}`)。

在 `settings.rs` 的 `mod tests` 追加(`use crate::panel_host::testing::{TIMEOUT, runtime}; use crate::service_fakes::{FakeApps, FakeDaemon, session}; use dozer_services::{ServiceError, ServiceErrorKind}; use std::sync::{Arc, mpsc};`):

```rust
    fn daemon(fake: &FakeDaemon) -> Arc<dyn dozer_services::DaemonService> {
        fake.clone().into_services().daemon
    }

    fn open_state(advanced: AdvancedState) -> Option<State> {
        let mut s = test_state(ConnectState::default(), ConnectState::default(), ConnectState::default());
        s.advanced = advanced;
        Some(s)
    }

    fn run(state: &mut Option<State>, msg: Message, fake: &FakeDaemon, apps: &FakeApps, rt: &tokio::runtime::Runtime) -> mpsc::Receiver<Message> {
        let (tx, rx) = mpsc::channel();
        let tx = std::sync::Mutex::new(tx);
        update(state, msg, daemon(fake), apps.clone(), rt.handle(), move |m| {
            let _ = tx.lock().unwrap().send(m);
        });
        rx
    }

    #[test]
    fn stop_clicked_counts_only_alive_sessions() {
        let rt = runtime();
        let fake = FakeDaemon::default().with_sessions(Ok(vec![session("a", true), session("b", false), session("c", true)]));
        let mut st = open_state(AdvancedState::Idle { error: None });
        let rx = run(&mut st, Message::AdvancedStopClicked, &fake, &FakeApps::unscripted(), &rt);
        assert!(matches!(st.as_ref().unwrap().advanced, AdvancedState::FetchingCount));
        match rx.recv_timeout(TIMEOUT).unwrap() {
            Message::AdvancedSessionCountReady(r) => assert_eq!(r, Ok(2)),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn stop_clicked_failure_text_is_the_service_message_verbatim() {
        let rt = runtime();
        let fake = FakeDaemon::default().with_sessions(Err(ServiceError::new(ServiceErrorKind::Unavailable, "daemon 断开")));
        let mut st = open_state(AdvancedState::Idle { error: None });
        let rx = run(&mut st, Message::AdvancedStopClicked, &fake, &FakeApps::unscripted(), &rt);
        match rx.recv_timeout(TIMEOUT).unwrap() {
            Message::AdvancedSessionCountReady(r) => assert_eq!(r, Err("daemon 断开".to_string())),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn stop_confirm_calls_shutdown_only_from_confirming_state() {
        let rt = runtime();
        let fake = FakeDaemon::default();
        // 不在 ConfirmingStop:不调服务、状态不变。
        let mut st = open_state(AdvancedState::Idle { error: None });
        let rx = run(&mut st, Message::AdvancedStopConfirm, &fake, &FakeApps::unscripted(), &rt);
        assert!(rx.recv_timeout(std::time::Duration::from_millis(300)).is_err());
        assert!(fake.calls().is_empty());
        // 在 ConfirmingStop:进入 Stopping 并调 shutdown_daemon。
        let mut st = open_state(AdvancedState::ConfirmingStop { live_sessions: 2 });
        let rx = run(&mut st, Message::AdvancedStopConfirm, &fake, &FakeApps::unscripted(), &rt);
        assert!(matches!(st.as_ref().unwrap().advanced, AdvancedState::Stopping));
        assert!(matches!(rx.recv_timeout(TIMEOUT).unwrap(), Message::AdvancedStopResult(Ok(()))));
        assert_eq!(fake.calls(), vec!["shutdown_daemon()".to_string()]);
    }

    #[test]
    fn stop_confirm_failure_text_is_verbatim() {
        let rt = runtime();
        let fake = FakeDaemon::default().with_stop(Err(ServiceError::new(ServiceErrorKind::Protocol, "等待 dozerd 停止超时")));
        let mut st = open_state(AdvancedState::ConfirmingStop { live_sessions: 0 });
        let rx = run(&mut st, Message::AdvancedStopConfirm, &fake, &FakeApps::unscripted(), &rt);
        match rx.recv_timeout(TIMEOUT).unwrap() {
            Message::AdvancedStopResult(r) => assert_eq!(r, Err("等待 dozerd 停止超时".to_string())),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn restart_calls_ensure_running_only_from_stopped_and_keeps_error_text() {
        let rt = runtime();
        let fake = FakeDaemon::default().with_ensure(Err(ServiceError::new(ServiceErrorKind::Unavailable, "无法连接 dozerd（已重试 3 次）：x")));
        let mut st = open_state(AdvancedState::Idle { error: None });
        let rx = run(&mut st, Message::AdvancedRestartClicked, &fake, &FakeApps::unscripted(), &rt);
        assert!(rx.recv_timeout(std::time::Duration::from_millis(300)).is_err());
        assert!(fake.calls().is_empty());
        let mut st = open_state(AdvancedState::Stopped { error: None });
        let rx = run(&mut st, Message::AdvancedRestartClicked, &fake, &FakeApps::unscripted(), &rt);
        assert!(matches!(st.as_ref().unwrap().advanced, AdvancedState::RestartingDozerd));
        match rx.recv_timeout(TIMEOUT).unwrap() {
            Message::AdvancedRestartResult(r) => assert_eq!(r, Err("无法连接 dozerd（已重试 3 次）：x".to_string())),
            other => panic!("unexpected: {other:?}"),
        }
        assert_eq!(fake.calls(), vec!["ensure_running()".to_string()]);
    }
```
(`AdvancedState` 各变体的真实字段名以 `settings.rs` 为准——`grep -n "enum AdvancedState" -A16`;`ConnectState::default()` 若不存在,照 `test_state` 现有调用处用的构造。)

「应用」页(`FakeApps` 先按 Interfaces 实现再写这些测试;`AppSummary`/`RuntimeProbe` 用空 `Vec` 避免构造):

```rust
    fn apps_ok() -> FakeApps {
        FakeApps::new(|req| match req {
            AppRequest::List => Ok(AppReply::Apps { apps: vec![] }),
            AppRequest::ProbeRuntimes => Ok(AppReply::Runtimes { runtimes: vec![] }),
            _ => Err(AppApiError::Transport("未预设".into())),
        })
    }

    #[test]
    fn opening_the_apps_tab_probes_runtimes_and_lists_apps() {
        let rt = runtime();
        let apps = apps_ok();
        let mut st = open_state(AdvancedState::Idle { error: None });
        let rx = run(&mut st, Message::TabSelected(SettingsTab::Apps), &FakeDaemon::default(), &apps, &rt);
        let mut got = 0;
        while got < 2 {
            match rx.recv_timeout(TIMEOUT).unwrap() {
                Message::Apps(_) => got += 1,
                other => panic!("unexpected: {other:?}"),
            }
        }
        let reqs = apps.requests();
        assert!(reqs.iter().any(|r| r.contains("ProbeRuntimes")), "{reqs:?}");
        assert!(reqs.iter().any(|r| r.starts_with("List")), "{reqs:?}");
    }

    #[test]
    fn stop_with_an_invalid_id_fails_as_transport_and_sends_no_request() {
        let rt = runtime();
        let apps = FakeApps::unscripted();
        let mut st = open_state(AdvancedState::Idle { error: None });
        // 触发 Effect::Stop(id) 的消息名按 settings_apps::Message 实际变体填写;id 取 AppId::new 一定拒绝的值(如 "Bad Id!")。
        let rx = run(&mut st, Message::Apps(<触发 Stop("Bad Id!") 的 settings_apps::Message>), &FakeDaemon::default(), &apps, &rt);
        match rx.recv_timeout(TIMEOUT).unwrap() {
            Message::Apps(settings_apps::Message::ActionDone(id, _, Err(crate::extensions::app_host::Failure::Transport(_)))) => assert_eq!(id, "Bad Id!"),
            other => panic!("unexpected: {other:?}"),
        }
        assert!(apps.requests().is_empty(), "非法 id 不发请求");
    }

    #[test]
    fn install_forwards_the_approved_plan_and_source_unchanged() {
        // 触发 Effect::Install { approved, source } 的消息按实际变体填写(需要一个已批准的 InstallPlan,
        // 构造方式参考 settings_apps.rs 现有测试里的 plan 夹具);断言 apps.requests() 里恰有一条
        // 以 "Install" 开头,且其 Debug 文本同时包含 plan 的 id 与 source 字符串。
    }
```
Run: `cargo test -p dozer-app extensions::settings 2>&1 | grep -E '^error' | head -5` → Expected: 编译失败(`update`/`FakeApps` 签名不符)(RED)。

- [ ] **Step 2: 实现**

1. `service_fakes.rs` 加 `FakeApps`:
```rust
#[derive(Clone)]
pub(crate) struct FakeApps {
    handler: Arc<dyn Fn(&AppRequest) -> Result<AppReply, AppApiError> + Send + Sync>,
    seen: Arc<Mutex<Vec<String>>>,
}
impl FakeApps {
    pub(crate) fn new(h: impl Fn(&AppRequest) -> Result<AppReply, AppApiError> + Send + Sync + 'static) -> Self {
        Self { handler: Arc::new(h), seen: Arc::default() }
    }
    pub(crate) fn unscripted() -> Self {
        Self::new(|_| Err(AppApiError::Transport("未预设".into())))
    }
    pub(crate) fn requests(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }
}
impl bytehost_client::AppHostApi for FakeApps {
    fn request(&self, req: AppRequest) -> impl std::future::Future<Output = Result<AppReply, AppApiError>> + Send {
        self.seen.lock().unwrap().push(format!("{req:?}"));
        let r = (self.handler)(&req);
        async move { r }
    }
    fn subscribe(&self) -> impl std::future::Future<Output = Result<tokio::sync::mpsc::UnboundedReceiver<AppChange>, AppApiError>> + Send {
        async { Err(AppApiError::Transport("未预设".into())) }
    }
}
```
`request`/`subscribe` 的精确签名与 `AppChange`/`mpsc` 的路径以 `bytehost-client` 的 trait 定义为准(`~/.cargo/git/checkouts/bytehost-*/*/crates/bytehost-client/src/lib.rs` 约 86–100 行);`AppRequest` 若不是 `Debug` 则改用变体名手写 `kind()` 记录。

2. `settings.rs`:
   - `use dozer_services::DaemonService; use std::sync::Arc;`
   - `update` 签名改为 Interfaces 里的泛型形式;`run_apps_message(s, apps_msg, apps.clone(), …)` 两处调用;
   - `AdvancedStopClicked`:`let daemon = daemon.clone();` spawn 内 `daemon.list_sessions().await.map(|s| count_live_sessions(&s)).map_err(|e| e.message)`;
   - `AdvancedStopConfirm`:`daemon.shutdown_daemon().await.map_err(|e| e.message)`;
   - `AdvancedRestartClicked`:`daemon.ensure_running().await.map_err(|e| e.message)`;
   - `run_apps_message<A>`:`client: &dozer_client::Client` → `apps: A`,循环里 `let client = apps.clone();` 改名 `let apps = apps.clone();` 并把所有 `client.app_*` 换成 `apps.app_*`(`AppHostApi` 的 `use` 已在函数内)。
   - 文件内不再出现 `dozer_client`。
3. `app/update.rs`(约 3966–3972):
```rust
                let services = crate::panel_host::services_for(&self.client);
                let apps = self.client.clone();
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let emit = move |m| {
                    let _ = proxy.send_event(Message::Settings(m));
                };
                settings::update(&mut self.settings, msg, services.daemon, apps, &handle, emit);
```
4. 去掉 Task 2 临时的 `#[allow(dead_code)]`。

Run: `cargo build --workspace 2>&1 | grep -E '^error' -A8 | head -30` → Expected: 无输出。若编译器报"future is not `Send`"(`A` 的默认方法 future),记 Ruling 并按报错补约束(例如在 `where` 里加 `for<'a> …`),不改 `AppHostApi`。

- [ ] **Step 3: 运行 settings 测试**

```bash
cargo test -p dozer-app extensions::settings 2>&1 | grep -E 'test result|FAILED|^error' -A6 | head -20
```
Expected: 全绿(新增 Advanced 5 个 + 「应用」页 3 个;既有 `apply_sync_message`、`count_live_sessions…` 等纯状态测试保持通过)。

- [ ] **Step 4: 探针守卫、全量、门禁、提交**

```bash
cargo test -p dozer-app usage_probe::host 2>&1 | grep -E 'test result|FAILED|实际是'   # 若 host_site 行号漂移,只改行号
W=$(ls -d .superpowers/sdd/*/ | head -1)
cargo test --workspace --no-fail-fast > ${W}t3.txt 2>&1; grep -E '^test result' ${W}t3.txt | awk '{s+=$4;f+=$6} END{print s,f}'; grep -E '^test .* FAILED' ${W}t3.txt
cargo clippy --all-targets 2>&1 | grep -c '^warning'
python3 scripts/audit/check_panel_boundary.py        # 只许剩 R-HOST-EXECUTORS 0 -> 1;R-HOST-PANEL-ARMS 不得上升
bash scripts/check-log-scope.sh | tail -1
cargo fmt -p dozer-app && cargo fmt --check && echo fmt-ok
```
Expected: 失败仅已知的(间歇失败单独重跑三次确认);clippy ≤ 基线;门禁无新增回退;`app/state.rs` 来源名测试通过。
```bash
git checkout Cargo.lock 2>/dev/null; git add crates/dozer-app/src && git status --short
git commit -m "feat(settings): Settings S3 设置弹窗改用 DaemonService 与泛型 AppHostApi

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 4: `R-CLIENT` 清零、验收与文档回填

**Files:**
- Modify: `scripts/audit/panel-boundary.baseline.json`(仅 `extensions/settings.rs` 的 `R-CLIENT` 条目)
- Modify: `docs/dozer-v2/bytehost-H0/08-client-use-inventory.md`、`CLAUDE.md`

**Interfaces:**
- Produces: `extensions/**` 不再有任何 `R-CLIENT` 基线条目。

- [ ] **Step 1: 确认并下调基线**

```bash
grep -n 'dozer_client\|io\.client()' crates/dozer-app/src/extensions/settings.rs | head -3   # 期望无输出
python3 scripts/audit/check_panel_boundary.py   # settings.rs 的 R-CLIENT 实测应为 0(低于基线),仍只报既有 R-HOST-EXECUTORS
python3 - <<'PY'
import json
p = "scripts/audit/panel-boundary.baseline.json"
base = json.load(open(p, encoding="utf-8"))
rel = "extensions/settings.rs"
if rel in base:
    base[rel].pop("R-CLIENT", None)
    if not base[rel]:
        del base[rel]
json.dump(base, open(p, "w", encoding="utf-8"), ensure_ascii=False, indent=1, sort_keys=True)
open(p, "a").write("\n")
PY
git diff --stat scripts/audit/panel-boundary.baseline.json
python3 scripts/audit/check_panel_boundary.py; echo exit=$?
python3 scripts/audit/test_check_panel_boundary.py 2>&1 | tail -1
grep -c '"R-CLIENT"' scripts/audit/panel-boundary.baseline.json   # 期望 0:extensions/** 的 R-CLIENT 全部清零
```
Expected: 基线 diff 只含 `extensions/settings.rs` 条目的删除;门禁仍只报 `R-HOST-EXECUTORS 0 -> 1`。(基线 JSON 的实际结构以文件为准——先 `head` 看一眼再改脚本。)

- [ ] **Step 2: 验收清单(逐条跑、读输出)**

```bash
cargo tree -p dozer-services --edges normal                    # 1: 仅 dozer-model + serde 系
grep -rn 'dozer_client' crates/dozer-app/src/extensions | grep -v '^\S*:[0-9]*:\s*//' | head   # 2: 无输出
cargo test -p dozer-app extensions::settings 2>&1 | grep -E 'test result|FAILED'   # 3
cargo test -p dozer-core --test model_golden 2>&1 | grep 'test result'             # 5
W=$(ls -d .superpowers/sdd/*/ | head -1)
cargo test --workspace --no-fail-fast > ${W}final.txt 2>&1; grep -E '^test result' ${W}final.txt | awk '{s+=$4;f+=$6} END{print s,f}'; grep -E '^test .* FAILED' ${W}final.txt   # 4
cargo clippy --all-targets 2>&1 | grep -c '^warning'; cargo fmt --check && echo fmt-ok
bash scripts/check-log-scope.sh && echo log-scope-ok
git status --short                                             # 无 Info.plist、无无关 Cargo 改动
```
任一不满足即不收尾,回到对应任务修。

- [ ] **Step 3: 文档回填并提交**

- `docs/dozer-v2/bytehost-H0/08-client-use-inventory.md` 末尾追加一节"Settings 迁移(2026-10-11)":Advanced 页经新的 `DaemonService`(`list_sessions`/`shutdown_daemon`/`ensure_running`),生产实现 `ClientDaemon` 在 `dozer-app`(`ensure_running` 要拉起 dozerd 进程,`dozer-client` 够不着);「应用」页 `run_apps_message` 对 `AppHostApi` 泛型(该 trait 是 RPITIT,非 dyn 兼容,不进 `Services`),host 传 `Client` 副本;`R-CLIENT` 中 `settings.rs` 清零,**至此 `extensions/**` 的 `R-CLIENT` 全部清零**;用户可见行为零变化。**登记后续项**:`window_events`/`run_app_host_effects` 对应用宿主的直接调用不在 `extensions/`,仍直连 `Client`(门禁不计);`ClientDaemon::ensure_running` 未被自动化测试覆盖(会真拉起进程)。
- `CLAUDE.md` Workspace 布局表 `dozer-services` 行里"…Browser、ProjectCreate 已迁,其余面板待迁"改为"…Browser、ProjectCreate、Settings 已迁,全部面板已迁"。
```bash
git checkout Cargo.lock 2>/dev/null
git add scripts/audit docs CLAUDE.md && git status --short
git commit -m "chore(audit): Settings S4 R-CLIENT 基线清零 + 文档回填

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```
