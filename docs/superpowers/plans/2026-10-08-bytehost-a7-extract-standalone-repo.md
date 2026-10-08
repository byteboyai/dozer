# bytehost A7:拆成独立仓库,并让第二个宿主(Digger)能嵌入 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 bytehost 应用宿主从 dozer 仓库里拆成独立仓库 `byteboyai/bytehost`(带 git 历史、发 tag),让 Dozer 只靠 tag 依赖它,同时让没有 dozerd、技术栈是 Tauri 的 Digger 也能在自己进程里嵌入宿主、复用同一套状态机与 webview 安全策略。**纯重构:行为、wire 协议、落盘格式一字不变。**

**Architecture:** 先在 dozer 工作区里**就地**做四次抽取(每次都全绿,边界在原地验证),最后一步才做机械的"搬家":
(1) `AppService` 从 `dozerd` 搬进 `bytehost-apps`(`server` feature),日志改走 `tracing`;
(2) 新 crate `bytehost-client`:与传输无关的 `AppHostApi` trait(只要实现 `request`+`subscribe` 两个方法,其余类型化方法是默认实现),附进程内实现 `InProcess` 与一套"两种实现都必须通过"的契约测试;
(3) 新 crate `bytehost-webview`:应用 webview 的**纯**安全策略(导航判定、IPC nonce、数据存储标识、清除队列),wry 与 Tauri 共用;
(4) 新 crate `bytehost-panel`:与 UI 框架无关的状态机(应用面板、日志查看器、设置页的安装/审批流程),以 `AppKey` trait 摆脱 dozer 的 `AppSlot`、以自己的 `NoticeLevel` 摆脱 Toast;iced 视图留在 dozer。
最后 `git filter-repo` 保历史拆仓,dozer 改成 git tag 依赖并删掉重复。

**Tech Stack:** Rust 1.98 / edition 2024、tokio、tracing、serde、`url`、`uuid`、`git-filter-repo`(已装)。

**Spec:** `docs/superpowers/specs/2026-10-04-bytehost-app-host-design.md`(§287 Digger 是第二个消费者;§301 A8 待办"Digger 的 supervisor 怎么实现"——本计划只提供**进程内嵌入**这一支,不替 Digger 裁决守护进程方案)。治理规则:`docs/architecture/byteboy-repositories.md`(平台仓不得依赖产品;正式依赖固定 tag;跨仓交付顺序)。前序:A6f 验收报告。

## Global Constraints

- **行为保持**:本切片不得改变任何用户可见行为或线上字节。`AppRequest`/`AppReply`/`AppEvent` 的 JSON 形状、`RECORD_FORMAT_VERSION = 1`、`apps/<id>/package/<version>`/`data/`/`logs/` 目录布局、受管运行时目录、端口持久化文件,全部不动。
- **`data_store_identifier(app_id)` 算法与测试里钉死的向量一字不改**(改了所有应用的本地数据会"消失")。
- **应用 webview 的导航策略只有一份真相**(`bytehost-webview::AppOrigin::allows_navigation`);dozer 里不得留下第二份拷贝;`build_app_webview_pins_the_restrictive_settings` 继续存在并通过;应用适配宿主严格 CSP、不放宽。
- 新仓库内**任何 crate 不得依赖 `dozer*`、`iced*`、`wry`、`tauri*`**;`bytehost-apps` 默认 feature 依赖只能是 serde 家族(沿用原门禁并扩到新 crate);平台库不泄漏可替换后端的类型。
- 日志:`bytehost-apps` 的 `server` feature 用 `tracing`(target 固定为 `bytehost::service` 等,**不带 dozer 字样**);dozerd/dozer-app 里仍禁止裸 `tracing::*!`/`eprintln!`(`scripts/check-log-scope.sh`)。**日志里仍不得出现应用日志内容、安装输出、带令牌的 URL。**
- 持久状态不进 Toast;Toast/UI 映射只在 dozer 侧(`bytehost-panel` 只产出 `Effect::Notice{level,..}`)。
- 字体、byteui 组件、`CLAUDE.md` 其余裁决不变。
- 正式依赖一律 git tag,**禁止**把相对路径写进正式 `Cargo.toml`(本地联调用不提交的 `.cargo/config.toml` `[patch]`)。
- 新增函数参数 ≥7 个且有相邻同类型参数时用具名字段结构体。
- 提交前 `git diff --cached --stat` 看全貌,只 `git add` 指定路径;不动工作区里已有的 `Cargo.lock` 改动(`[patch]` 造成,非内容)。
- 变异验证:每个**新**测试(契约测试、`AppKey` 适配、`Notice` 映射、依赖闸门)都要临时破坏实现确认会失败再恢复;纯搬家的测试用"搬前搬后用例数一致 + 全绿"证明。
- **对外发布步骤(建远端仓库、推送、打 tag)由用户亲自确认后执行**;计划里标 🔒 的步骤执行者必须停下来问,不得自行 `git push`。

## Review Focus

1. **数据存储标识被改动**:任何对 FNV 常量/前缀的改动都会让用户的应用本地数据丢失(Task 3 钉向量,搬家后原样运行)。
2. **导航策略出现第二份拷贝**:搬走后 dozer 里若仍保留旧 `AppOrigin`,两边会渐渐不一致,应用就能导航到别的 origin(Task 3,以 `grep` 门禁钉住)。
3. **`AppService` 搬家改变退出语义**:dozerd 退出必须仍是"撤下站点、保留 `desired`、杀掉进程组",下次启动自动恢复;端口持久化、首次占用冲突报错不能变(Task 1,live 测试 + 现有集成测试逐字通过)。
4. **日志 target 变化导致 `RUST_LOG` 过滤失效/泄露内容**:事件日志只写 app id 与事件名,不写令牌(Task 1)。
5. **错误类别丢失**:`AppApiError::Host(AppFailure)` 与 `Transport` 的区分是 GUI "dozerd 不可用是持久状态而非 Toast"的依据,UDS 实现与进程内实现必须给出同样的类别(Task 2 契约测试)。
6. **仓库拆出去后单独构建不了**:新仓库必须在**没有 dozer 在旁边**的干净检出里 `fmt`/`clippy -D warnings`/`test`/门禁全绿(Task 6);dozer 必须只靠 tag 构建(Task 7)。
7. **订阅与重订阅语义在泛型化后变形**:`State<K>` 的推送合并(`dirty`)、退避、`Subscribed` 后补拉行为必须与 A6f 逐条一致(Task 4,原 35 个 `app_host` 用例零改动语义地迁移)。

## File Structure

新仓库 `byteboyai/bytehost`(Task 6 之后的最终形状;Task 1–5 是在 dozer 工作区里用 `crates/` 同名目录预演):

| 路径 | 来源 | 职责 |
|---|---|---|
| `crates/bytehost-apps/` | 原 `crates/bytehost-apps`(+ `src/service.rs`) | 应用模型/计划/注册表/生命周期/gateway/运行时;`server` feature 含 `AppService` |
| `crates/bytehost-client/` | **新** | `AppHostApi` trait、`AppApiError`、`AppChange`、`InProcess`、契约测试 |
| `crates/bytehost-webview/` | **新**(自 `dozer-app/src/app_webview.rs` 的纯部分) | 导航判定、IPC nonce 与注入脚本、数据存储标识、清除队列 |
| `crates/bytehost-panel/` | **新**(自 `app_host.rs`/`app_logs.rs`/`settings_apps.rs` 的非视图部分) | `AppKey`、`NoticeLevel`、`LogsState`、`PanelState<K>`、`InstallFlowState` |
| `scripts/` | 原 `scripts/bytehost/*`、`check-bytehost-apps-deps.sh` | 运行时 pin、Excalidraw 配方、样例应用、依赖闸门 |
| `docs/` | 复制 `specs/2026-10-04-bytehost-app-host-design.md` 与各验收报告 | 设计与历史 |
| `.github/workflows/ci.yml` | 新(仿 byteui) | fmt/clippy/test/门禁 |

dozer 侧改动:

| 文件 | 变更 |
|---|---|
| `crates/dozerd/src/app_service.rs` | **删除**(搬进 `bytehost-apps::service`);`main.rs`/`server.rs`/`lib.rs`/各集成测试改 import |
| `crates/dozer-client/src/lib.rs` | `impl AppHostApi for Client`;删除 `app_*` 固有方法与 `AppChange`(改 re-export) |
| `crates/dozer-app/src/app_webview.rs` | 只留与 `AppSlot`/`WebviewSpec`/objc2 绑定的部分,纯策略改调 `bytehost-webview` |
| `crates/dozer-app/src/extensions/app_host.rs`、`app_logs.rs`、`settings_apps.rs` | 状态机搬走;留下薄适配(`impl AppKey for AppSlot`、`Notice→Toast`、iced 视图) |
| `Cargo.toml`(各 crate) | `path` 依赖 → git tag 依赖(Task 7) |
| `CLAUDE.md`、`docs/architecture/byteboy-repositories.md` | 更新仓库边界与门禁说明 |

---

### Task 1: `AppService` 从 dozerd 搬进 `bytehost-apps`

**Files:**
- Create: `crates/bytehost-apps/src/service.rs`(整体搬自 `crates/dozerd/src/app_service.rs`,含 `change_reply` 与全部单测)
- Modify: `crates/bytehost-apps/src/lib.rs`(`#[cfg(feature = "server")] pub mod service;`)、`crates/bytehost-apps/Cargo.toml`(`server` feature 加 `dep:tracing`)
- Modify: `crates/dozerd/src/lib.rs`(删 `pub mod app_service`)、`main.rs:100`、`server.rs`(`apps` 字段类型与 `change_reply` 路径)、`crates/dozerd/tests/*.rs`(import)
- Move: `crates/dozerd/tests/process_apps_live.rs` → `crates/bytehost-apps/tests/process_apps_live.rs`(样例路径随之调整;仍 `#[ignore]`)
- Delete: `crates/dozerd/src/app_service.rs`

**Interfaces:**
- Produces:`bytehost_apps::service::{AppService, change_reply}`,签名与原来一致:
  `AppService::{unavailable(reason)->Arc<Self>, start(root:&Path)->Arc<Self>, start_with(root, GatewayConfig), start_with_monitor_for_test(root, GatewayConfig, MonitorConfig), handle(&self, AppRequest)->Result<AppReply,AppFailure>, subscribe(&self)->Option<broadcast::Receiver<AppEvent>>, gateway_stopped(&self)->bool, shutdown(&self)}`。

- [x] **Step 1: 记录基线用例数**
  Run: `cargo test -p dozerd --lib app_service:: 2>&1 | grep "test result"` 与 `cargo test -p dozerd --test app_requests 2>&1 | grep "test result"`;把 passed 数记进提交信息(搬后必须一致)。

- [x] **Step 2: 搬文件并替换日志宏**
  `git mv crates/dozerd/src/app_service.rs crates/bytehost-apps/src/service.rs`。把文件顶部 `dozer_core::scope!(LOG, module, "apps");` 删掉,所有 `dozer_core::log_error!(LOG, ...)` / `log_warn!` / `log_info!` 改成 `tracing::error!(target: "bytehost::service", ...)` 等(字段写法不变:`error = %e`、`port = p`、`app = %app`)。`use` 里与 `bytehost_apps::` 自引用的前缀改成 `crate::`。
  只在 `service.rs` 内换宏,**不改任何逻辑**。

- [x] **Step 3: 接线**
  `Cargo.toml`:`server = [..., "dep:tracing"]`,`tracing = { workspace = true, optional = true }`。`lib.rs` 加 `#[cfg(feature = "server")] pub mod service;`。dozerd 里所有 `crate::app_service::` / `dozerd::app_service::` 改成 `bytehost_apps::service::`;`dozerd/src/lib.rs` 去掉模块声明。

- [x] **Step 4: 搬 live 测试**
  `git mv crates/dozerd/tests/process_apps_live.rs crates/bytehost-apps/tests/process_apps_live.rs`;测试里的 `dozerd::app_service::AppService` 改 `bytehost_apps::service::AppService`,样例路径(`scripts/bytehost/samples/...`)改成相对 `CARGO_MANIFEST_DIR` 的 `../../scripts/bytehost/samples/...`。`dev-dependencies` 补测试需要的 crate(以编译报错为准,只加已在 workspace 里的)。

- [x] **Step 5: 验证**
  Run: `cargo test -p bytehost-apps --all-features`(用例数 = 原 bytehost-apps 数 + 原 `app_service::` 数)、`cargo test -p dozerd`(集成测试逐字通过,`app_requests` 仍 5 个)、`cargo test -p bytehost-apps --test process_apps_live -- --ignored`(3 个通过)、`scripts/check-log-scope.sh`、`scripts/check-bytehost-apps-deps.sh`(默认 feature 依赖不变,`tracing` 只在 `server` 下)。
  Expected: 全绿;用例总数不少于基线。

- [x] **Step 6: 变异验证(退出语义)**
  临时把 `shutdown` 里"保留 `desired`"的那一步改成清掉 `desired`,确认 `a_python_app_runs_through_the_wire_and_dies_with_dozerd`(或等价的重启恢复用例)失败;恢复。

- [x] **Step 7: Commit**
  ```bash
  git add crates/bytehost-apps crates/dozerd
  git commit -m "refactor(bytehost-apps,dozerd): move AppService into bytehost-apps so a host without dozerd can embed it (A7 task 1)"
  ```

---

### Task 2: `bytehost-client`:与传输无关的 `AppHostApi` + 进程内实现 + 契约测试

**Files:**
- Create: `crates/bytehost-client/{Cargo.toml, src/lib.rs, src/in_process.rs, src/conformance.rs}`
- Modify: `crates/dozer-client/src/lib.rs`、`crates/dozer-client/Cargo.toml`、`crates/dozer-app/src/app/app.rs`、`crates/dozer-app/src/extensions/settings.rs`、`crates/dozer-app/src/extensions/app_host.rs`(`Failure`)、`crates/dozerd/tests/app_requests.rs`
- Test: `crates/bytehost-client/tests/in_process.rs`、`crates/dozerd/tests/app_requests.rs`

**Interfaces:**
- Produces(`bytehost-client`;依赖 `bytehost-apps`(默认 feature)、`tokio`(`sync`、`rt`)、`thiserror` 不要——手写 `Display`):
  ```rust
  /// 订阅里的一条事件;都是失效信号,收到后重拉 `app_list()`。
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub enum AppChange { Changed(AppId), Resync, Disconnected }

  /// 一次调用的失败:宿主带类别的失败,或传输/协议层的(连不上、协议不符)。
  #[derive(Debug, Clone, PartialEq)]
  pub enum AppApiError { Host(AppFailure), Transport(String) }
  impl AppApiError { pub fn text(&self) -> &str; pub fn kind(&self) -> Option<AppErrorKind> }
  impl std::fmt::Display for AppApiError { .. }  impl std::error::Error for AppApiError {}

  pub trait AppHostApi: Send + Sync {
      fn request(&self, request: AppRequest)
          -> impl Future<Output = Result<AppReply, AppApiError>> + Send;
      fn subscribe(&self)
          -> impl Future<Output = Result<mpsc::UnboundedReceiver<AppChange>, AppApiError>> + Send;
      // 以下全是默认实现(基于 `request`),签名与原 `dozer_client::Client::app_*` 一一对应,
      // 只是错误类型从 anyhow 换成 AppApiError:
      fn app_list(&self) -> impl Future<Output = Result<Vec<AppSummary>, AppApiError>> + Send { .. }
      fn app_plan(&self, source: AppSource, provenance: Provenance, trust: TrustLevel) -> .. InstallPlan
      fn app_install(&self, approved: ApprovedInstallPlan, source: AppSource) -> .. ()
      fn app_start(&self, id: AppId) -> .. String
      fn app_stop(&self, id: AppId) -> .. ()
      fn app_uninstall(&self, id: AppId, mode: UninstallMode) -> .. ()
      fn app_launch_url(&self, id: AppId) -> .. String
      fn app_probe_runtimes(&self) -> .. Vec<RuntimeProbe>
      fn app_logs(&self, id: AppId, max_lines: u32) -> .. (String, bool)
      fn app_runtime_plan(&self, runtime: ManagedRuntime) -> .. RuntimeInstallPlan
      fn app_install_runtime(&self, plan: RuntimeInstallPlan) -> .. ()
      fn app_uninstall_runtime(&self, runtime: ManagedRuntime, version: &str) -> .. ()
  }
  ```
  默认实现里"回了不该回的变体"一律 `AppApiError::Transport("意外应答: {other:?}".into())`(与现 `bail!("意外应答…")` 同义)。
  `in_process`(feature `in-process`,隐含 `bytehost-apps/server`):`pub struct InProcess(Arc<AppService>); impl InProcess { pub fn new(svc: Arc<AppService>) -> Self }`;`request` → `svc.handle(..)`,`Err(AppFailure)` → `AppApiError::Host`;`subscribe` → `svc.subscribe()`,`None` → `Host(Unavailable)`,否则起一个任务把 broadcast 经 `change_reply` 转成 `AppChange`(`Closed`→`Disconnected` 后结束;接收端被丢弃即退出)。
  `conformance`(feature `conformance`):`pub async fn run<A: AppHostApi>(api: &A, source: AppSource, app_id: AppId)`,`source` 是一个**静态应用目录**(调用方负责造)。

- [x] **Step 1: 写契约测试(先红)**
  `conformance::run` 依次断言:① `app_list()` 初始为空;② `app_plan(source, Local, Trusted)` 得到的计划 `id == app_id`;③ `app_install(approve(plan), source.clone())` 成功后 `app_list()` 含它且 `observed == Stopped`;④ `app_start(id)` 返回 `http://<id>.localhost:` 开头的启动地址(静态应用),随后 `app_list()` 里 `Running`;⑤ 订阅 **先于** `app_stop` 建立,`app_stop` 后 5s 内收到 `Changed(app_id)`;⑥ `app_start(未知 id)` 的错误 `kind() == Some(NotFound)`,**不是** `Transport`;⑦ `app_uninstall(id, ProgramAndData)` 后列表为空;⑧ 一个订阅接收端被 drop 后,服务端不残留(`InProcess`:任务退出)。
  `tests/in_process.rs`:造临时根目录 + `AppService::start_with(root, GatewayConfig{port:0})`,跑 `conformance::run(&InProcess::new(svc), ..)`;结束 `svc.shutdown()`。
  此时 `bytehost-client` 尚未实现 → 编译失败(预期红)。

- [x] **Step 2: 实现 trait、错误类型、`InProcess`,跑红→绿**
  Run: `cargo test -p bytehost-client --features in-process,conformance`
  Expected: PASS。

- [x] **Step 3: dozer-client 实现同一 trait**
  `impl AppHostApi for Client`:`request` = 现 `app_request` 的逻辑(`Reply::App{Failed}` → `Host`,其余协议问题 → `Transport`);`subscribe` = 现 `app_subscribe` 的主体(`Subscribed` 握手失败 → `Host`/`Transport`),`AppChange` 改为 re-export `bytehost_client::AppChange`。**删除**固有的 `app_*` 方法与重复的 `AppChange` 定义;`dozer-client` 的 `app_subscribe_tests` 保留并改调 trait 方法。

- [x] **Step 4: dozerd 集成测试跑同一份契约**
  `crates/dozerd/tests/app_requests.rs` 新增 `uds_client_passes_the_same_conformance_suite`:起测试 daemon,`conformance::run(&Client::new(sock), ..)`。(`dozerd` dev-dependency 加 `bytehost-client` 的 `conformance` feature。)

- [x] **Step 5: 调用点迁移**
  `dozer-app` 里 16 处 `client.app_*(..)`(`app/app.rs` 6 处、`extensions/settings.rs` 10 处)加 `use bytehost_client::AppHostApi;`,错误处理从 `Failure::from_client_error(&e)` 改为 `Failure::from(e)`:`app_host::Failure` 改成 `pub use bytehost_client::AppApiError as Failure;` 的薄包装不可行(`from_client_error` 被多处引用)→ 做法:保留 `Failure` 枚举,新增 `impl From<AppApiError> for Failure`,`from_client_error` 删除,编译器会指出全部调用点。
  `dozerd/tests/app_requests.rs` 的 18 处同样迁移。

- [x] **Step 6: 变异验证**
  ① 把 `InProcess::subscribe` 的 `None` 分支改成返回空流 → 契约 ⑤/Unavailable 用例失败;② 把默认实现里 `NotFound` 映射成 `Transport` → 契约 ⑥ 失败;③ 让 UDS `request` 吞掉 `Failed` → 契约 ⑥ 失败。逐个确认后恢复。

- [x] **Step 7: 验证并提交**
  Run: `cargo test -p bytehost-client --all-features`、`cargo test -p dozer-client`、`cargo test -p dozerd`、`cargo test -p dozer-app -- app_host settings_apps`(`Failure` 的 `text()`/类别判断用例不变)、`cargo clippy --all-targets`。
  ```bash
  git add crates/bytehost-client crates/dozer-client crates/dozer-app crates/dozerd Cargo.toml
  git commit -m "refactor(bytehost-client,dozer-client): transport-agnostic AppHostApi with in-process and UDS implementations sharing one conformance suite (A7 task 2)"
  ```

---

### Task 3: `bytehost-webview`:应用 webview 的纯安全策略

**Files:**
- Create: `crates/bytehost-webview/{Cargo.toml, src/lib.rs}`(依赖 `url`、`uuid`(v4);**无** wry/iced/objc2)
- Modify: `crates/dozer-app/src/app_webview.rs`、`crates/dozer-app/src/runtime.rs`(`build_app_webview` 的 import)、`crates/dozer-app/Cargo.toml`

**Interfaces:**
- Produces(`bytehost_webview`,全部 `pub`,自 `app_webview.rs` 逐字搬出,仅把 `AppSlot` 换成 `&str`):
  ```rust
  pub struct AppOrigin { .. }
  impl AppOrigin { pub fn from_url(url: &str) -> Option<Self>; pub fn app_id(&self) -> &str; pub fn allows_navigation(&self, url: &str) -> bool }
  pub fn data_store_identifier(app_id: &str) -> [u8; 16];
  pub enum AppIpc { Focus, MouseUp, ZoomIn, ZoomOut, ZoomReset }
  impl AppIpc { pub fn parse(body: &str, nonce: &str) -> Option<Self> }
  pub fn new_ipc_nonce() -> String;
  pub fn app_init_script(nonce: &str) -> String;
  pub fn supports_store_removal(os_major: isize) -> bool;
  pub enum StoreRemovalOutcome { Done, InUse, Unsupported, Failed(String) }
  pub struct StoreRemovals { .. }   // request / take_ready(now, webview_in_pool: impl Fn(&str)->bool) / finish / retry 同原签名
  ```
  `AppOrigin::from_url` 里原来调用 `crate::app::valid_app_id(id)` → 改为 `bytehost_apps::id::AppId::new(id).is_ok()`(两者同口径:1–63 个 `[a-z0-9-]`、首尾非 `-`;**先写一个等价性测试**遍历边界样例确认二者一致,再替换)。因此 `bytehost-webview` 依赖 `bytehost-apps`(默认 feature)。
- Stays in dozer:`is_app_webview_id`、`webview_id`、`slot_for_webview_id`、`AppViews`、`app_webview_spec`、`host_os_major`(objc2)、`take_ready` 的适配(`AppSlot::intern(id).is_some_and(..)` 包成闭包传入)。

- [x] **Step 1: 先写等价性测试与"向量钉死"测试**
  在 `bytehost-webview` 写 `valid_app_id_matches_appid_new`(边界:空串、63/64 字符、首尾 `-`、大写、下划线、`a--b`)与 `data_store_identifier_vector_is_pinned`(**直接复制原测试里的向量**)。

- [x] **Step 2: 搬代码与全部原单测**
  把 `AppOrigin`、`data_store_identifier`、`AppIpc`、`new_ipc_nonce`、`app_init_script`、`supports_store_removal`、`StoreRemovalOutcome`、`StoreRemovals` 及其 `#[cfg(test)]` 用例原样搬到新 crate;`dozer-app` 的 `app_webview.rs` 改为 `use bytehost_webview::*` 并保留绑定部分。搬后用例数 = 搬前纯策略用例数(搬前先 `grep -c "#\[test\]"` 记数)。

- [x] **Step 3: 钉"只有一份"的门禁测试**
  `crates/dozer-app` 里加测试 `no_second_copy_of_the_navigation_policy`:读取 `src/` 下所有 `.rs`,断言字符串 `fn allows_navigation` 只在 `bytehost-webview` 里出现(测试读文件路径可相对 `CARGO_MANIFEST_DIR`;在 Task 7 拆仓后改为断言 dozer-app 源码里**没有** `fn allows_navigation`)。

- [x] **Step 4: 验证**
  Run: `cargo test -p bytehost-webview`、`cargo test -p dozer-app -- app_webview build_app_webview`(含 `build_app_webview_pins_the_restrictive_settings`)、`cargo clippy --all-targets`。

- [x] **Step 5: 变异验证**
  ① 把 `allows_navigation` 里主机比较改成不区分端口 → origin 用例失败;② 把 FNV `PRIME` 末位改 1 → 向量测试失败;③ 让 `AppIpc::parse` 忽略 nonce → nonce 用例失败。恢复。

- [x] **Step 6: Commit**
  ```bash
  git add crates/bytehost-webview crates/dozer-app Cargo.toml
  git commit -m "refactor(bytehost-webview,dozer-app): app webview security policy as a pure crate shared by wry and Tauri hosts (A7 task 3)"
  ```

---

### Task 4: `bytehost-panel`(上):`AppKey`、日志状态机、应用面板状态机

**Files:**
- Create: `crates/bytehost-panel/{Cargo.toml, src/lib.rs, src/key.rs, src/notice.rs, src/logs.rs, src/host.rs}`(依赖 `bytehost-apps`(默认)、`bytehost-client`(仅类型))
- Modify: `crates/dozer-app/src/extensions/{app_host.rs, app_logs.rs}`、`crates/dozer-app/src/app/app.rs`、`app/view.rs`、`app_slots.rs`

**Interfaces:**
- Produces:
  ```rust
  /// 面板状态机用来标识"哪个应用"的键。dozer 用进程内槽号(`AppSlot`),Digger 可直接用 `AppId`。
  pub trait AppKey: Copy + Eq + std::hash::Hash + std::fmt::Debug + Send + 'static {
      fn from_app_id(id: &AppId) -> Option<Self>;   // 过滤掉无法映射的 id
      fn app_id(&self) -> &str;
  }
  pub enum NoticeLevel { Info, Success, Warning, Error }
  // logs.rs:LogsState / LogsView / LogsEffect / REFRESH_INTERVAL / FETCH_LINES 原样(去掉 iced 的 scroll_id)
  // host.rs:
  pub struct PanelState<K: AppKey> { .. }            // 原 app_host::State
  pub enum PanelMessage<K> { .. }                    // 原 app_host::Message(AppSlot→K)
  pub enum PanelEffect<K> { .., Notice { level: NoticeLevel, text: String, key: String } }  // 原 Effect::Toast 改名
  pub use 原样: Failure(= bytehost_client::AppApiError)、Act、PanelView、LogsView(app_host 版)、issue_texts、POLL_INTERVAL、SAFETY_POLL_INTERVAL、RESUBSCRIBE_BACKOFF
  ```
  方法签名与原 `State` 完全一致(`poll_wanted`、`poll_interval`、`poll_if_due`、`subscribe_if_due`、`update`、`tick_logs(now, visible)`、`any_logs_open(visible)`、`take_log_scroll`、`view_model`、`logs_view`、`display_name`……),仅 `AppSlot`→`K`。
- Stays in dozer:`impl AppKey for AppSlot`(`from_app_id` = `AppSlot::intern(id.as_str())`,`app_id` = `self.id()`);`NoticeLevel→toast::Level` 的映射;`scroll_id()`(iced 的 `widget::Id`);全部 iced 视图。

- [x] **Step 1: 测试替身键与"键适配"测试(先红)**
  在 `bytehost-panel` 写测试用 `#[derive(Clone,Copy,PartialEq,Eq,Hash,Debug)] struct TestKey(u8)` 的 `AppKey` 实现(用一张 up-to-256 的常量表映射 id)。在 `dozer-app` 写 `app_slot_implements_appkey`:`AppSlot::intern("a")` 往返、`AppKey::app_id` 往返。(`AppSlot::intern` 与 `bytehost_apps::AppId::new` 同口径,非法形状两边都拒,无法构造"`AppId` 认得但 `AppSlot` 认不得"的 id,故非法 id → `None` 的分支由 `TestKey` 侧覆盖。)

- [x] **Step 2: 搬 `app_logs.rs` 的状态机**
  `LogsState` 等整体搬入 `logs.rs`;`scroll_id` 留在 dozer 的 `app_logs.rs`(薄文件:`pub fn scroll_id`)。原 9 个 `app_logs` 用例搬走并通过。

- [x] **Step 3: 搬 `app_host.rs` 的状态机并泛型化**
  `State`→`PanelState<K>`,`Message`→`PanelMessage<K>`,`Effect`→`PanelEffect<K>`,`Effect::Toast`→`PanelEffect::Notice { level: NoticeLevel, text, key }`(字段名 `level`/`text`/`key` 保持);内部所有 `AppSlot` 换 `K`,`list_loaded` 里 `AppSlot::intern(..)` 换 `K::from_app_id(..)`。38 个用例用 `TestKey` 迁移,断言内容**逐字不变**(只改类型名)。因 `AppSlot` 无 `Default`,手写 `impl<K: AppKey> Default for PanelState<K>`;`id()` 返回 `String`(原 `&str`,避免借用局部);`Failure` 直接 `pub use bytehost_client::AppApiError as Failure`。
  dozer 的 `app_host.rs` 剩余:`impl AppKey for AppSlot`、`pub type State/Message/Effect` 别名、`notice_level(NoticeLevel) -> toast::Level`、`Effect::Notice` → `push_toast_keyed`。
  为过 Task 6 门禁 `grep -rn "AppSlot\|toast" crates/bytehost-panel/src`(小写 `toast` 也匹配,`Toast` 不匹配):`bytehost-panel` 里的注释、测试名、字段名一律不用 `AppSlot`/`toast` 字面量,改用 `键`/`Notice`/`notice`。
  **顺带**:`Failure` 变成 `AppApiError` 别名后,`settings.rs`/`app.rs` 里原先有意义的 `.map_err(Failure::from)`(旧 `Failure` 是独立类型)变成同类型转换,clippy `useless_conversion` 报警,已删除这些调用点。

- [x] **Step 4: 验证**
  Run: `cargo test -p bytehost-panel`(48 = 38 host + 9 logs + 1 新增)、`cargo test -p dozer-app -- app_host app_logs settings_apps`(48 通过)、`cargo test -p dozer-app`(1925 通过,仅既有 flaky `files::delete_confirm_spec_reflects_pending_target` 失败,与 A7 无关)、`cargo clippy --all-targets`(仅既有 warning)、`cargo fmt`;`grep -rn "AppSlot\|toast" crates/bytehost-panel/src` 无结果(exit 1);`bash scripts/check-log-scope.sh`、`bash scripts/check-bytehost-apps-deps.sh` 均 ok。

- [x] **Step 5: 变异验证**
  复用 A6f 的四处,全部"破坏即失败、恢复即通过":(1)`dirty` 补拉改 `if false` → 风暴用例 `a_change_triggers_exactly_one_refetch_even_in_a_storm` 失败;(2)`tick_logs` 忽略 `visible` 改 `if false` → `a_log_viewer_whose_panel_is_not_visible_is_not_refreshed` 失败;(3)`SubscriptionLost` 退避不递增(去掉 `(attempt+1).min(..)`)→ `losing_the_subscription_goes_back_to_fast_polling_and_retries_with_backoff` 失败;(4)新增 `an_app_id_the_key_cannot_map_is_skipped_from_the_order` 覆盖 `from_app_id` 返回 `None` 的 id 不进 `order`/rail。四处均已恢复。

- [ ] **Step 6: Commit**
  ```bash
  git add crates/bytehost-panel crates/dozer-app Cargo.toml
  git commit -m "refactor(bytehost-panel,dozer-app): UI-framework-agnostic app panel and log viewer state machines keyed by AppKey (A7 task 4)"
  # 实际提交:先 `git add crates/bytehost-panel crates/dozer-app`,再
  # `git restore --staged crates/dozer-app/packaging/macos/Info.plist`(该文件有与 A7 无关的既有改动,不得入库)。
  ```

---

### Task 5: `bytehost-panel`(下):安装/审批/运行时流程状态机

**Files:**
- Create: `crates/bytehost-panel/src/install.rs`
- Modify: `crates/dozer-app/src/extensions/settings_apps.rs`(拆成"状态+展示函数"与"iced 视图"两个文件:新增 `settings_apps_view.rs`,从原文件第 859 行起的视图部分搬过去)、`extensions/settings.rs`、`extensions.rs`

**Interfaces:**
- Produces(`bytehost_panel::install`,自 `settings_apps.rs` 第 1–858 行搬出):`State`(→`InstallFlowState`)、`Message`、`Effect`(`Toast`→`Notice`)、`Load`、`Flow`、`ActKind`,以及展示函数 `plan_view`、`runtime_line`、`observed_label`、`managed_runtime_label`、`runtime_rows`、`plan_lines`、`provenance_label`、`trust_label`、`enforcement_label`、`probe_is_installing`、`orphan_result_effects` 原签名不变;日志查看器部分已在 Task 4 用 `LogsState`,这里复用。
- Stays in dozer:全部 `iced_widget` 视图(`view`、`app_row`、`log_viewer`、`flow_view`、`review_view`…)与 `at_bottom`;`settings.rs::run_apps_message` 的副作用执行(`Effect`→`Client` 调用,改用 `AppHostApi`);`tick_armed`/`arm_tick` 仍是状态机的一部分(随状态机搬走,测试 `only_one_tick_is_ever_armed_until_it_fires` 随行)。
- 注意:安装来源/信任等级今天固定为本机目录(`Local`/`Trusted`)——本任务**不改**这一点(A6h 才扩展),只是把"点了安装→出计划→审批→安装"的状态机搬走,让 Digger 的前端可以驱动同一流程。

- [ ] **Step 1: 记录基线**
  `cargo test -p dozer-app -- settings_apps 2>&1 | grep "test result"`(35 个),搬后总数必须不少于它。

- [ ] **Step 2: 拆文件并搬状态机**
  `git mv` 不适用(拆分),手工:新建 `install.rs` 放原 1–858 行与 `#[cfg(test)]` 全部非视图用例;`settings_apps_view.rs` 放视图;`settings_apps.rs` 变成 `pub use bytehost_panel::install::*;` + `pub use super::settings_apps_view::view;`。`Effect::Toast` 的使用点(`settings.rs`)改为 `Effect::Notice` 并映射 `NoticeLevel`。

- [ ] **Step 3: 验证**
  Run: `cargo test -p bytehost-panel`、`cargo test -p dozer-app -- settings_apps app_host`、`cargo clippy --all-targets`。`grep -rn "iced" crates/bytehost-panel` 无结果。

- [ ] **Step 4: 变异验证**
  ① 让 `update(InstallClicked)` 跳过审批直接装 → 对应用例失败(审批卡展示的就是被批准的那份计划);② 让 `ProbesLoaded` 不再报告完成的任务 → 失败。恢复。

- [ ] **Step 5: Commit**
  ```bash
  git add crates/bytehost-panel crates/dozer-app
  git commit -m "refactor(bytehost-panel,dozer-app): install/approval/runtime flow state machine without iced (A7 task 5)"
  ```

---

### Task 6: 拆仓:`git filter-repo` 保历史,新仓库独立构建与 CI

**Files:**(都在**新仓库**里;在 dozer 仓库只提交 Task 6 的验收记录)
- Create: 新仓库 `README.md`、`.github/workflows/ci.yml`、`scripts/check-deps.sh`(原 `check-bytehost-apps-deps.sh` 扩展)、`docs/`(复制设计规格与 A0–A6 验收报告)、`examples/embed.rs`
- Test: `crates/bytehost-apps/tests/embedded_host.rs`

**Interfaces:**
- 新仓库顶层 `Cargo.toml`:`[workspace] members = ["crates/*"]`,`[workspace.package] edition = "2024"`,`[workspace.dependencies]` 抄 dozer 的 `serde/serde_json/tokio/tracing/uuid` 版本;`rust-toolchain` 不固定(CI 用 stable)。
- `scripts/check-deps.sh` 门禁(全部对 `cargo tree -e normal,build --target all --all-features` 执行):① 任一 crate 依赖树不得出现 `dozer*`、`iced*`、`wry`、`tauri*`、`objc2*`(`bytehost-webview` 也不得有 `objc2`);② `bytehost-apps` 默认 feature 依赖闭包只能是 serde 家族;③ `bytehost-panel` 的依赖只能是 `bytehost-apps`、`bytehost-client` 及其传递依赖;④ `grep -rn "AppSlot\|toast" crates/bytehost-panel/src` 必须无结果。

- [ ] **Step 1: 在 dozer 里确认 Task 1–5 全绿后,做一个克隆来拆**
  ```bash
  git clone --no-local . /tmp/claude-501/bytehost-extract && cd /tmp/claude-501/bytehost-extract
  git filter-repo \
    --path crates/bytehost-apps --path crates/bytehost-client \
    --path crates/bytehost-webview --path crates/bytehost-panel \
    --path scripts/bytehost --path scripts/check-bytehost-apps-deps.sh \
    --path docs/superpowers/specs/2026-10-04-bytehost-app-host-design.md \
    --path-glob 'docs/superpowers/specs/*bytehost-a*-acceptance-report.md' \
    --path-rename scripts/check-bytehost-apps-deps.sh:scripts/check-deps.sh \
    --path-rename scripts/bytehost/:scripts/
  ```
  Expected:`git log --oneline | wc -l` ≥ 41(原 bytehost-apps 历史保留);`ls` 只剩上述路径。**这一步在一次性克隆里做,不碰 dozer 仓库。**

- [ ] **Step 2: 补顶层文件**
  写顶层 `Cargo.toml`、`README.md`(公开 API 列表:四个 crate 各一句话 + "谁是消费者:Dozer(已接入)、Digger(嵌入待做)" + 兼容性承诺:wire 只追加、落盘格式版本、tag 即发布)、`.github/workflows/ci.yml`(仿 `byteui`:`fmt --check`、`clippy --all-targets --all-features -- -D warnings`、`test --all-features`、`scripts/check-deps.sh`;runner `macos-latest`)。

- [ ] **Step 3: 独立构建(无 dozer 在旁)**
  在**一个全新目录**(例如 `/tmp/claude-501/bytehost-clean`,`git clone` 刚才的结果)里依次运行:`cargo fmt --check`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo test --all-features`、`bash scripts/check-deps.sh`、`cargo test -p bytehost-apps --all-features --test process_apps_live -- --ignored`(需要 python3/node)。
  Expected:全绿。**若 clippy 在 dozer 里是 warning 但这里 `-D warnings` 失败,在新仓库修掉(只改告警,不改逻辑)。**

- [ ] **Step 4: 嵌入验证(Digger 的用法,没有 dozerd)**
  `tests/embedded_host.rs`:不依赖任何 dozer 代码,用 `AppService::start_with(tmp, GatewayConfig{port:0})` + `InProcess`,装并启动 `scripts/samples/py-notes`,经 gateway 带 Cookie 请求拿到 200 与 `runtime:`,断言不带 Cookie 为 403,`shutdown()` 后端口关闭。`examples/embed.rs` 是同样流程的 30 行演示(作为文档)。

- [ ] **Step 5: 变异验证(门禁本身)**
  临时给 `bytehost-panel` 的 `Cargo.toml` 加一个 `iced_core` 依赖 → `check-deps.sh` 必须失败;在 `bytehost-panel/src` 里写一个 `AppSlot` 字样 → 必须失败。恢复。

- [ ] **Step 6: 🔒 发布(停下来问用户)**
  需要用户确认并执行(或明确授权后由执行者执行):
  1. 在 GitHub 建 `byteboyai/bytehost`(空仓库);
  2. `git remote add origin … && git push origin main`;
  3. `git tag v0.1.0 && git push origin v0.1.0`。
  执行者在此**停下**,把 `git log --oneline | head`、`git tag`、验收输出贴给用户,等回复。

- [ ] **Step 7: Commit(在 dozer 仓库)**
  在 `docs/superpowers/specs/` 新建 `2026-10-08-bytehost-a7-acceptance-report.md`,记录 Step 3–5 的真实输出与新仓库首个提交 id/tag,然后:
  ```bash
  git add docs/superpowers/specs/2026-10-08-bytehost-a7-acceptance-report.md
  git commit -m "docs(bytehost): A7 extraction acceptance (standalone repo builds and passes without dozer)"
  ```

---

### Task 7: dozer 改依赖 tag,删掉重复,更新文档与门禁

**前置:** Task 6 Step 6 已由用户完成(远端仓库与 `v0.1.0` tag 存在)。

**Files:**
- Modify: `Cargo.toml`(workspace `exclude`/`members`)、`crates/{dozer-app,dozer-client,dozer-core,dozerd}/Cargo.toml`
- Delete: `crates/bytehost-apps`、`crates/bytehost-client`、`crates/bytehost-webview`、`crates/bytehost-panel`、`scripts/bytehost/`、`scripts/check-bytehost-apps-deps.sh`
- Modify: `CLAUDE.md`(bytehost-apps 行重写)、`docs/architecture/byteboy-repositories.md`(`bytehost` 行改为"应用宿主平台库,已建立 v0.1.0";原"Host SDK/PTY"那条职责单独标为待定,不再叫 `bytehost`)、`scripts/check-log-scope.sh` 若引用旧路径

**Interfaces:**
- 依赖写法(与 byteui 一致):
  ```toml
  bytehost-apps   = { git = "https://github.com/byteboyai/bytehost", tag = "v0.1.0" }
  bytehost-apps   = { git = "https://github.com/byteboyai/bytehost", tag = "v0.1.0", features = ["server"] }  # 仅 dozerd
  bytehost-client = { git = "…", tag = "v0.1.0" }
  bytehost-webview = { git = "…", tag = "v0.1.0" }
  bytehost-panel  = { git = "…", tag = "v0.1.0" }
  ```
  同一个仓库里的多个 crate 用同一个 tag,cargo 会解析成同一份检出。

- [ ] **Step 1: 先本地联调(不提交)**
  在 `.cargo/config.toml` 加 `[patch."https://github.com/byteboyai/bytehost"]` 把四个 crate 指到 `../bytehost/crates/*`,确认 dozer 在 patch 下全绿——这一步证明拆分本身没破坏任何东西。

- [ ] **Step 2: 切到 tag**
  改各 `Cargo.toml`,**去掉 patch**,`cargo update -p bytehost-apps` 生成 `Cargo.lock`(注意:本机 `Cargo.lock` 里被 `[patch]` 抹掉的 `source` 行由这一步恢复,这是预期的内容变化,与此前的"本机 patch 噪声"不同,要一并提交)。删除已搬走的 crate 目录与脚本。

- [ ] **Step 3: 收紧"只有一份"门禁**
  把 Task 3 的 `no_second_copy_of_the_navigation_policy` 改为:断言 `dozer-app/src` 下**没有** `fn allows_navigation` 与 `fn data_store_identifier`。

- [ ] **Step 4: 验证(对照 A6f 基线)**
  Run: `cargo build`、`cargo clippy --all-targets`(无 error)、`cargo test -p dozerd`、`cargo test -p dozer-client`、`cargo test -p dozer-core`、`cargo test -p dozer-app`(只允许既有的 `delete_confirm_spec_reflects_pending_target` 失败)、`scripts/check-log-scope.sh`、`cargo fmt --check`。再在**全新克隆**里 `cargo build -p dozerd`(无 `.cargo/config.toml` patch)确认只靠 tag 能构建。
  把 `dozer-app` 通过数与 A6f 末次(1969)对比:应 ≥ 1969 − (已随状态机搬走的用例数) + 0;搬走的用例数在新仓库同名通过,总和不得减少。

- [ ] **Step 5: 更新文档**
  `CLAUDE.md` 的 `bytehost-apps` 一行改为指向新仓库并保留所有"关键裁决"(严格 CSP、数据存储归属、`Advisory`、监管线程持锁规则…——这些规则现在属于新仓库的 README/设计文档,dozer 的 CLAUDE.md 只留"消费方必须遵守的"部分与跳转);`byteboy-repositories.md` 按上面更新。

- [ ] **Step 6: Commit**
  ```bash
  git add Cargo.toml Cargo.lock crates CLAUDE.md docs/architecture/byteboy-repositories.md scripts
  git commit -m "refactor: consume bytehost from its own repository at v0.1.0; drop the in-tree copies (A7 task 7)"
  ```

---

## 已知局限与不做的事

- **不替 Digger 做 A8 裁决。** 本计划只证明"进程内嵌入"可行(Task 6 Step 4);Digger 若要自带守护进程,仍是它自己的切片。
- **Tauri 侧的 webview 创建代码不在本切片。** `bytehost-webview` 只给纯策略;怎么在 Tauri 里建子 webview、拦截导航,属于 Digger 的工作(必须调用 `AppOrigin::allows_navigation` 与 `AppIpc::parse`)。
- 展示文案(中文标签、审批卡措辞)仍在 `bytehost-panel` 里;Digger 若要换语言/措辞,下一步再抽成可注入的表。
- 容器运行时、升级回滚(A6g)、压缩包/URL 来源(A6h)不在本切片。**A6g/A6h 的计划写在 dozer 仓库里,执行时要改在新仓库(`bytehost`)里做**——A7 完成后需要把这两份计划里的路径(`crates/bytehost-apps/...` 基本不变)和"提交到哪个仓库"更新一遍。
- 新仓库的发布节奏/changelog 约定(SemVer、wire 只追加)只在 README 里声明,尚无自动化校验。

## 自检记录

- **范围对照用户选择:** 独立仓库(Task 6/7)、`AppService` 搬迁(Task 1)、传输无关客户端 trait(Task 2)、UI 无关状态机(Task 4/5)、webview 安全策略纯逻辑(Task 3)——五项均有任务。
- **类型一致:** `AppHostApi`/`AppApiError`/`AppChange` 在 Task 2 定义,Task 4 的 `Failure` 复用 `AppApiError`;`AppKey` 在 Task 4 定义,Task 5 复用 `LogsState`;`StoreRemovals::take_ready` 的闭包由 `&str` 版(Task 3)在 dozer 侧适配 `AppSlot`。
- **顺序依赖:** Task 2 依赖 Task 1(`InProcess` 要 `AppService`);Task 4 依赖 Task 2(`Failure`);Task 5 依赖 Task 4;Task 6 依赖全部;Task 7 依赖 Task 6 的发布。Task 1/2/3 之间除上述外相互独立,可并行评审但**不要并行改同一文件**(`app.rs`、`settings.rs` 会被 Task 2/4/5 反复触碰,按序执行)。
