# bytehost A4a:应用宿主的类别化失败 + 首次端口落盘修正 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在做 GUI 之前,先把 A4 依赖的两处后端前置项落实:(1) 失败带稳定的类别(`AppErrorKind`),GUI 不靠解析中文文案分流;(2) 首次运行的 gateway 端口"绑定成功之后才落盘",选中的端口被占时自动换一个重试。

**Architecture:** `bytehost-apps::proto` 新增 `AppErrorKind`/`AppFailure` 与 `AppReply::Failed`;`ManagerError::kind()` 做映射;`AppService::handle` 的错误类型由 `String` 改为 `AppFailure`,`server.rs` 把它装进 `Reply::App{ reply: Failed }`(不再走 `Reply::Error`);`dozer-client::app_request` 把 `Failed` 还原成可 `downcast_ref::<AppFailure>()` 的 `anyhow::Error`。端口:`port::load_or_choose_port` 拆成只读的 `load_port` 与原子的 `persist_port`;`AppService::first_run` 负责选-绑-存与重试。

**Tech Stack:** Rust、serde、tokio、anyhow;不新增依赖。

**Spec:** `docs/superpowers/specs/2026-10-04-bytehost-app-host-design.md`(A4 行与 §6.3、§6.4)。

## A4 的切分与本次裁决(规格 A4 一行太大,拆三片)

| 片 | 内容 | 状态 |
|---|---|---|
| **A4a(本计划)** | 类别化失败;首次端口落盘修正与重试 | 可执行 |
| **A4b** | GUI:应用列表轮询 → `App::sync_installed_apps`;`PanelKind::App` 面板里的 wry 视图(`http://<id>.localhost:端口/`,每应用 `data_store_identifier`)与**导航策略**(禁止离开本 origin 的顶层导航/`window.open`——这是静态应用出站网络从 `Advisory` 升到 `Enforced` 的前提);`AppFailure::Unavailable` 时的 host 提示页 | 需先探针:`runtime.rs` 的 `WebviewSpec` 池怎么容纳一种新 kind |
| **A4c** | 安装计划/审批的最小界面(走独立原生窗口机制)、Settings 里的运行时探测展示 | 依赖 A4b |

本计划里做的裁决(都可在评审时推翻,代价写在后面):
- **推送事件不做,GUI 轮询 `List`**(A4b 里 2 秒一次,仅当存在应用面板或设置页打开时)。代价:状态变化最多晚 2 秒可见;换来协议少一个订阅通道。
- **失败类别只有 6 个**(`unavailable`/`rejected`/`not_found`/`conflict`/`unsupported`/`internal`),线上协议只增不改。`Verify`/`Manifest` 的细分(例如"摘要不符"与"清单不合法")现在都是 `rejected`,细分留给 A4c 的审批界面需要时再加。
- **首次端口落盘写失败 = 服务不可用**,而不是"继续跑、下次换端口"——换端口会让所有应用的本地存储丢失(origin 含端口)。

## Global Constraints

- `bytehost-apps` 默认 feature 的依赖仍只有 serde 家族;不得依赖任何 `dozer*` crate(`scripts/check-bytehost-apps-deps.sh` 必须仍然通过)。
- 线上协议向后兼容:`AppReply` 只新增变体;`AppErrorKind` 只往后加。
- `gateway.json` 的格式不变(`{"port": N}`);坏文件仍是错误且原样保留,**绝不**覆盖。
- 日志:`dozerd` 内一律 `dozer_core::log_*!(LOG, ...)`,不写令牌。

## Review Focus

- 首次运行选中的端口被占:换端口重试,**只记住真正绑上的那个**;连续 5 次都被占 → 服务不可用且**磁盘上什么都没写**。
- 端口写不进磁盘:服务不可用,gateway 端口被释放(不是留着一个下次会变的端口继续跑)。
- 已有 `gateway.json` 的老用户:行为与 A2 完全一致(端口被占仍是不可用,不换端口)。
- 服务不可用时每个请求都回 `Unavailable` 类别(不再是裸字符串),GUI 能据此出提示页。
- `Reply::Error` 不再被应用请求使用:旧 GUI 对未知 `AppReply::Failed` 的反应(A4b 之前没有 GUI 调用者,不涉及)。

---

### Task 1: 类别化失败(proto → manager → service → server → client)

**Files:**
- Modify: `crates/bytehost-apps/src/proto.rs`、`crates/bytehost-apps/src/manager.rs`、`crates/dozerd/src/app_service.rs`、`crates/dozerd/src/server.rs`、`crates/dozer-client/src/lib.rs`、`crates/dozerd/tests/app_requests.rs`

**Interfaces:**
- Produces: `AppErrorKind`、`AppFailure{kind,message}`(`Display`+`Error`)、`AppReply::Failed{failure}`(flatten:`{"reply":"failed","kind":..,"message":..}`)、`ManagerError::kind()`、`AppService::handle(..) -> Result<AppReply, AppFailure>`;客户端:`err.downcast_ref::<AppFailure>()`。

- [ ] **Step 1: 写失败测试。** 下面 diff 里的:`proto.rs::failures_carry_a_stable_kind_tag_next_to_the_message`、`manager.rs::manager_errors_map_to_stable_wire_kinds`、`app_service.rs` 里 `an_unavailable_service_answers_every_request_with_its_reason`(改为断言 `kind`)与 `failures_are_classified_not_just_described`、`tests/app_requests.rs` 里两处 `downcast_ref::<AppFailure>()`。
- [ ] **Step 2: RED。** `cargo test -p bytehost-apps --all-features -- failures_carry manager_errors_map` → 编译失败(类型不存在)。
- [ ] **Step 3: 实现。**

```diff
diff --git a/crates/bytehost-apps/src/manager.rs b/crates/bytehost-apps/src/manager.rs
index 456befc4..aaab444d 100644
--- a/crates/bytehost-apps/src/manager.rs
+++ b/crates/bytehost-apps/src/manager.rs
@@ -79,4 +79,21 @@ impl std::fmt::Display for ManagerError {
 }
 
+impl ManagerError {
+    /// 线上失败类别(GUI 据此呈现,见 [`AppErrorKind`])。
+    pub fn kind(&self) -> crate::proto::AppErrorKind {
+        use crate::proto::AppErrorKind as K;
+        match self {
+            Self::Io(_) => K::Internal,
+            Self::Manifest(_) | Self::Verify(_) | Self::BadSource(_) | Self::MissingSource(_) => {
+                K::Rejected
+            }
+            Self::UnsupportedRuntime(_) => K::Unsupported,
+            Self::AlreadyInstalled(_) | Self::Busy(_) | Self::BadState { .. } => K::Conflict,
+            Self::NotInstalled(_) => K::NotFound,
+            Self::ShuttingDown => K::Unavailable,
+        }
+    }
+}
+
 impl std::error::Error for ManagerError {}
 
@@ -630,4 +647,36 @@ impl ReconcileReport {
 #[cfg(test)]
 mod tests {
+    #[test]
+    fn manager_errors_map_to_stable_wire_kinds() {
+        use crate::proto::AppErrorKind as K;
+        let id = || AppId::new("a").unwrap();
+        let cases = [
+            (ManagerError::BadSource("x".into()), K::Rejected),
+            (ManagerError::MissingSource("/x".into()), K::Rejected),
+            (
+                ManagerError::UnsupportedRuntime("python".into()),
+                K::Unsupported,
+            ),
+            (
+                ManagerError::AlreadyInstalled(Version::new(1, 0, 0)),
+                K::Conflict,
+            ),
+            (ManagerError::Busy(id()), K::Conflict),
+            (
+                ManagerError::BadState {
+                    app: id(),
+                    state: ObservedState::Stopped,
+                },
+                K::Conflict,
+            ),
+            (ManagerError::NotInstalled(id()), K::NotFound),
+            (ManagerError::ShuttingDown, K::Unavailable),
+            (ManagerError::Io(io::Error::other("x")), K::Internal),
+        ];
+        for (error, kind) in cases {
+            assert_eq!(error.kind(), kind, "{error}");
+        }
+    }
+
     use super::*;
     use crate::gateway::GatewayConfig;
diff --git a/crates/bytehost-apps/src/proto.rs b/crates/bytehost-apps/src/proto.rs
index 3ffecf44..37f16970 100644
--- a/crates/bytehost-apps/src/proto.rs
+++ b/crates/bytehost-apps/src/proto.rs
@@ -54,4 +54,47 @@ pub struct RuntimeProbe {
 }
 
+/// 失败的类别——GUI 据此决定怎么呈现(不可用走提示页,被拒绝走审批界面,冲突/不存在走 Toast……),
+/// 不靠解析人类可读的 `message`。**新增类别只能往后加**(线上协议)。
+#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
+#[serde(rename_all = "snake_case")]
+pub enum AppErrorKind {
+    /// 应用宿主本身不可用(gateway 端口被占、应用目录打不开、dozerd 正在停止)。
+    Unavailable,
+    /// 请求内容被拒绝:清单/摘要/批准不符、来源不合法、包里缺东西。
+    Rejected,
+    /// 应用没有安装。
+    NotFound,
+    /// 与当前状态冲突:版本已装、应用在运行、状态不允许该操作。
+    Conflict,
+    /// 一期不支持的应用类型。
+    Unsupported,
+    /// 其他(I/O、后台任务崩溃)。
+    Internal,
+}
+
+/// 一次失败:类别 + 给人看的原因。
+#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
+pub struct AppFailure {
+    pub kind: AppErrorKind,
+    pub message: String,
+}
+
+impl AppFailure {
+    pub fn new(kind: AppErrorKind, message: impl Into<String>) -> Self {
+        Self {
+            kind,
+            message: message.into(),
+        }
+    }
+}
+
+impl std::fmt::Display for AppFailure {
+    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
+        f.write_str(&self.message)
+    }
+}
+
+impl std::error::Error for AppFailure {}
+
 #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
 #[serde(tag = "op", rename_all = "snake_case")]
@@ -109,4 +152,9 @@ pub enum AppReply {
         runtimes: Vec<RuntimeProbe>,
     },
+    /// 请求失败(带类别)。
+    Failed {
+        #[serde(flatten)]
+        failure: AppFailure,
+    },
 }
 
@@ -171,4 +219,31 @@ mod tests {
     }
 
+    #[test]
+    fn failures_carry_a_stable_kind_tag_next_to_the_message() {
+        let reply = AppReply::Failed {
+            failure: AppFailure::new(AppErrorKind::Unavailable, "端口被占用"),
+        };
+        assert_eq!(
+            round_trip(&reply),
+            json!({"reply": "failed", "kind": "unavailable", "message": "端口被占用"})
+        );
+        for (kind, tag) in [
+            (AppErrorKind::Rejected, "rejected"),
+            (AppErrorKind::NotFound, "not_found"),
+            (AppErrorKind::Conflict, "conflict"),
+            (AppErrorKind::Unsupported, "unsupported"),
+            (AppErrorKind::Internal, "internal"),
+        ] {
+            assert_eq!(serde_json::to_value(kind).unwrap(), json!(tag));
+        }
+        assert!(
+            serde_json::from_value::<AppReply>(
+                json!({"reply": "failed", "kind": "exploded", "message": "x"})
+            )
+            .is_err(),
+            "未知类别被拒绝"
+        );
+    }
+
     #[test]
     fn unknown_ops_and_unknown_sources_are_rejected_not_ignored() {
diff --git a/crates/dozer-client/src/lib.rs b/crates/dozer-client/src/lib.rs
index 2362559e..bb4b6f51 100644
--- a/crates/dozer-client/src/lib.rs
+++ b/crates/dozer-client/src/lib.rs
@@ -183,4 +183,8 @@ impl Client {
     pub async fn app_request(&self, request: AppRequest) -> Result<AppReply> {
         match self.roundtrip(&Request::App { request }).await? {
+            // 失败带类别:用 `err.downcast_ref::<AppFailure>()` 取回(其余错误是传输/协议问题)。
+            Reply::App {
+                reply: AppReply::Failed { failure },
+            } => Err(anyhow::Error::new(failure)),
             Reply::App { reply } => Ok(reply),
             other => bail!("意外应答: {other:?}"),
diff --git a/crates/dozerd/src/server.rs b/crates/dozerd/src/server.rs
index 87e29a65..5d743319 100644
--- a/crates/dozerd/src/server.rs
+++ b/crates/dozerd/src/server.rs
@@ -460,5 +460,5 @@ async fn handle_conn(
                         Request::App { request } => match apps.handle(request).await {
                             Ok(reply) => Reply::App { reply },
-                            Err(message) => Reply::Error { message },
+                            Err(failure) => Reply::App { reply: bytehost_apps::proto::AppReply::Failed { failure } },
                         },
                         Request::ListSessions => Reply::Sessions { sessions: registry.list() },
diff --git a/crates/dozerd/tests/app_requests.rs b/crates/dozerd/tests/app_requests.rs
index cbf01a50..5e513d33 100644
--- a/crates/dozerd/tests/app_requests.rs
+++ b/crates/dozerd/tests/app_requests.rs
@@ -4,5 +4,5 @@ use bytehost_apps::gateway::GatewayConfig;
 use bytehost_apps::id::AppId;
 use bytehost_apps::plan::{Approval, Provenance, TrustLevel};
-use bytehost_apps::proto::AppSource;
+use bytehost_apps::proto::{AppErrorKind, AppFailure, AppSource};
 use bytehost_apps::registry::UninstallMode;
 use dozer_client::Client;
@@ -166,6 +166,8 @@ async fn the_whole_install_run_stop_uninstall_cycle_works_over_the_socket() {
     assert!(c.app_list().await.unwrap().is_empty());
 
-    let err = c.app_start(id("excalidraw")).await.unwrap_err().to_string();
-    assert!(err.contains("没有安装"), "{err}");
+    let err = c.app_start(id("excalidraw")).await.unwrap_err();
+    let failure = err.downcast_ref::<AppFailure>().expect("带类别的失败");
+    assert_eq!(failure.kind, AppErrorKind::NotFound);
+    assert!(failure.message.contains("没有安装"), "{failure}");
     let runtimes = c.app_probe_runtimes().await.unwrap();
     assert_eq!(runtimes.len(), 3);
@@ -177,6 +179,11 @@ async fn an_unavailable_app_host_does_not_break_the_rest_of_the_daemon() {
     let d = start_daemon(AppService::unavailable("gateway 端口 12345 已被占用")).await;
     assert!(d.client.list().await.unwrap().is_empty(), "会话列表照常");
-    let err = d.client.app_list().await.unwrap_err().to_string();
-    assert!(err.contains("12345") && err.contains("占用"), "{err}");
+    let err = d.client.app_list().await.unwrap_err();
+    let failure = err.downcast_ref::<AppFailure>().expect("带类别的失败");
+    assert_eq!(failure.kind, AppErrorKind::Unavailable);
+    assert!(
+        failure.message.contains("12345") && failure.message.contains("占用"),
+        "{failure}"
+    );
 }
```

`app_service.rs` 里本任务相关的部分:`handle` 的返回类型、`Unavailable` 分支、`blocking`、探测任务的错误(完整 diff 见 Task 2 的统一 diff,按本任务只取这几处,其余属于 Task 2)。

- [ ] **Step 4: GREEN。** `cargo fmt -p bytehost-apps -p dozerd -p dozer-client && cargo test -p bytehost-apps --all-features && cargo test -p dozerd --test app_requests && cargo test -p dozer-client` → 全过(预期 154 / 3 / 1+1)。
- [ ] **Step 5: 变异检查。** `ManagerError::NotInstalled` 改映射到 `Internal` → `manager_errors_map_to_stable_wire_kinds` 必须 FAILED;还原。
- [ ] **Step 6: Commit。** `git commit -am "feat(bytehost-apps, dozerd): categorised app failures on the wire (A4a task 1)"`

### Task 2: 首次端口"绑定后才落盘" + 冲突重试

**Files:** Modify `crates/bytehost-apps/src/port.rs`、`crates/bytehost-apps/src/gateway/mod.rs`(仅文档注释)、`crates/dozerd/src/app_service.rs`

**Interfaces:**
- Consumes: Task 1 的 `AppFailure`/`AppErrorKind`。
- Produces: `port::load_port(&Path) -> io::Result<Option<u16>>`、`port::persist_port(&Path, u16) -> io::Result<()>`(`load_or_choose_port` 删除,仓内唯一调用方是 `AppService::start`);`AppService::first_run(root, pick)`(私有,测试可注入 `pick`)、`FIRST_RUN_ATTEMPTS = 5`。

- [ ] **Step 1: 写失败测试。** `port.rs`:`nothing_is_stored_until_persist_and_then_the_same_port_comes_back`、`a_missing_root_directory_is_created_by_persist`(并把老的坏文件/显式端口测试改调 `load_port`);`app_service.rs`:`the_first_run_retries_a_taken_port_and_persists_only_the_one_it_bound`、`the_first_run_gives_up_after_repeated_collisions_without_persisting_anything`、`the_first_run_is_unavailable_when_the_port_cannot_be_saved`。
- [ ] **Step 2: RED。** `cargo test -p dozerd --lib app_service::tests::the_first_run` → 编译失败(`first_run`/`load_port` 不存在)。
- [ ] **Step 3: 实现。**

```diff
diff --git a/crates/bytehost-apps/src/gateway/mod.rs b/crates/bytehost-apps/src/gateway/mod.rs
index 6bdf7cd4..c76fadce 100644
--- a/crates/bytehost-apps/src/gateway/mod.rs
+++ b/crates/bytehost-apps/src/gateway/mod.rs
@@ -79,5 +79,5 @@ impl Default for Limits {
 #[derive(Debug, Clone, Copy)]
 pub struct GatewayConfig {
-    /// 监听端口(127.0.0.1)。`0` 让系统分配——只给测试用;生产用 `port::load_or_choose_port` 的值。
+    /// 监听端口(127.0.0.1)。`0` 让系统分配——只给测试用;生产用 `port::load_port`/`pick_port` 的值。
     pub port: u16,
 }
diff --git a/crates/bytehost-apps/src/port.rs b/crates/bytehost-apps/src/port.rs
index 0398daeb..624a6569 100644
--- a/crates/bytehost-apps/src/port.rs
+++ b/crates/bytehost-apps/src/port.rs
@@ -27,6 +27,7 @@ pub fn pick_port() -> u16 {
 }
 
-/// 读 `<root>/gateway.json` 里持久化的端口;文件不存在就选一个、原子写入、返回。
-pub fn load_or_choose_port(root: &Path) -> io::Result<u16> {
+/// 读 `<root>/gateway.json` 里持久化的端口;文件不存在返回 `None`(首次运行,由调用方选端口、**绑定成功后**
+/// 再 [`persist_port`])。坏文件、越界端口一律是 `InvalidData` 错误,绝不"顺手重新选一个"。
+pub fn load_port(root: &Path) -> io::Result<Option<u16>> {
     let path = root.join("gateway.json");
     match fs::read_to_string(&path) {
@@ -48,24 +49,26 @@ pub fn load_or_choose_port(root: &Path) -> io::Result<u16> {
                 ));
             }
-            Ok(settings.port)
-        }
-        Err(e) if e.kind() == io::ErrorKind::NotFound => {
-            let port = pick_port();
-            fs::create_dir_all(root)?;
-            let json = serde_json::to_string_pretty(&GatewaySettings { port })
-                .map_err(io::Error::other)?;
-            let tmp = path.with_extension("tmp");
-            {
-                let mut file = fs::File::create(&tmp)?;
-                io::Write::write_all(&mut file, json.as_bytes())?;
-                file.sync_all()?; // 先落盘再改名,避免断电后留下空文件(空文件会变成永久的 InvalidData)
-            }
-            fs::rename(&tmp, &path)?;
-            Ok(port)
+            Ok(Some(settings.port))
         }
+        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
         Err(e) => Err(e),
     }
 }
 
+/// 原子写入 `<root>/gateway.json`(先落盘再改名,避免断电后留下空文件——空文件会变成永久的 `InvalidData`)。
+/// 只在端口**真的绑定成功之后**调用:选了但没绑上的端口不该被记住。
+pub fn persist_port(root: &Path, port: u16) -> io::Result<()> {
+    let path = root.join("gateway.json");
+    fs::create_dir_all(root)?;
+    let json = serde_json::to_string_pretty(&GatewaySettings { port }).map_err(io::Error::other)?;
+    let tmp = path.with_extension("tmp");
+    {
+        let mut file = fs::File::create(&tmp)?;
+        io::Write::write_all(&mut file, json.as_bytes())?;
+        file.sync_all()?;
+    }
+    fs::rename(&tmp, &path)
+}
+
 #[cfg(test)]
 mod tests {
@@ -81,11 +84,11 @@ mod tests {
 
     #[test]
-    fn the_first_call_chooses_and_persists_and_later_calls_return_the_same_port() {
+    fn nothing_is_stored_until_persist_and_then_the_same_port_comes_back() {
         let tmp = tempfile::tempdir().unwrap();
-        let first = load_or_choose_port(tmp.path()).unwrap();
-        assert!((PORT_MIN..=PORT_MAX).contains(&first));
-        assert!(tmp.path().join("gateway.json").is_file());
-        for _ in 0..5 {
-            assert_eq!(load_or_choose_port(tmp.path()).unwrap(), first);
+        assert_eq!(load_port(tmp.path()).unwrap(), None, "只读不写");
+        assert!(!tmp.path().join("gateway.json").exists());
+        persist_port(tmp.path(), 23456).unwrap();
+        for _ in 0..3 {
+            assert_eq!(load_port(tmp.path()).unwrap(), Some(23456));
         }
         assert!(
@@ -96,9 +99,9 @@ mod tests {
 
     #[test]
-    fn a_missing_root_directory_is_created() {
+    fn a_missing_root_directory_is_created_by_persist() {
         let tmp = tempfile::tempdir().unwrap();
         let root = tmp.path().join("a/b");
-        let port = load_or_choose_port(&root).unwrap();
-        assert_eq!(load_or_choose_port(&root).unwrap(), port);
+        persist_port(&root, 24000).unwrap();
+        assert_eq!(load_port(&root).unwrap(), Some(24000));
     }
 
@@ -115,5 +118,5 @@ mod tests {
         ] {
             fs::write(&path, bad).unwrap();
-            let err = load_or_choose_port(tmp.path()).unwrap_err();
+            let err = load_port(tmp.path()).unwrap_err();
             assert_eq!(err.kind(), io::ErrorKind::InvalidData, "{bad}");
             assert_eq!(
@@ -129,5 +132,5 @@ mod tests {
         let tmp = tempfile::tempdir().unwrap();
         fs::write(tmp.path().join("gateway.json"), r#"{"port": 51234}"#).unwrap();
-        assert_eq!(load_or_choose_port(tmp.path()).unwrap(), 51234);
+        assert_eq!(load_port(tmp.path()).unwrap(), Some(51234));
     }
 
diff --git a/crates/dozerd/src/app_service.rs b/crates/dozerd/src/app_service.rs
index 5a74cb70..375fbf7b 100644
--- a/crates/dozerd/src/app_service.rs
+++ b/crates/dozerd/src/app_service.rs
@@ -13,8 +13,8 @@ use std::time::{SystemTime, UNIX_EPOCH};
 
 use bytehost_apps::HOST_VERSION;
-use bytehost_apps::gateway::{Gateway, GatewayConfig};
+use bytehost_apps::gateway::{Gateway, GatewayConfig, GatewayError};
 use bytehost_apps::manager::{AppManager, ManagerError};
-use bytehost_apps::port::load_or_choose_port;
-use bytehost_apps::proto::{AppReply, AppRequest, RuntimeProbe};
+use bytehost_apps::port::{load_port, persist_port, pick_port};
+use bytehost_apps::proto::{AppErrorKind, AppFailure, AppReply, AppRequest, RuntimeProbe};
 use bytehost_apps::runtime::{SystemRunner, probe_all};
 
@@ -41,8 +41,10 @@ impl AppService {
     }
 
-    /// 用**持久化的端口**启动(首次启动随机选一个并写进 `<root>/gateway.json`)。永不失败。
+    /// 用**持久化的端口**启动。首次运行(没有 `<root>/gateway.json`)随机选端口、**绑定成功之后**才写盘;
+    /// 选中的端口恰好被占就换一个再试(此时还没有任何应用数据绑定在旧端口上)。永不失败。
     pub async fn start(root: &Path) -> Arc<Self> {
-        match load_or_choose_port(root) {
-            Ok(port) => Self::start_with(root, GatewayConfig { port }).await,
+        match load_port(root) {
+            Ok(Some(port)) => Self::start_with(root, GatewayConfig { port }).await,
+            Ok(None) => Self::first_run(root, pick_port).await,
             Err(e) => {
                 dozer_core::log_error!(LOG, error = %e, "读取 gateway 端口失败,应用宿主不可用");
@@ -52,4 +54,35 @@ impl AppService {
     }
 
+    /// 首次运行选端口的重试次数(范围 12768 个端口,连续占用 5 个几乎不可能,真发生就说明环境有问题)。
+    const FIRST_RUN_ATTEMPTS: usize = 5;
+
+    async fn first_run(root: &Path, mut pick: impl FnMut() -> u16) -> Arc<Self> {
+        for _ in 0..Self::FIRST_RUN_ATTEMPTS {
+            let port = pick();
+            match Gateway::start(GatewayConfig { port }).await {
+                Ok(gateway) => {
+                    // 绑定成功才记住;写不进去就不能继续——否则下次启动换端口,所有应用的本地存储都会丢。
+                    if let Err(e) = persist_port(root, port) {
+                        gateway.stop().await;
+                        dozer_core::log_error!(LOG, error = %e, "gateway 端口写盘失败,应用宿主不可用");
+                        return Self::unavailable(format!("应用宿主不可用:端口无法保存:{e}"));
+                    }
+                    return Self::finish_start(root, Arc::new(gateway)).await;
+                }
+                Err(GatewayError::PortInUse(p)) => {
+                    dozer_core::log_warn!(LOG, port = p, "首次选的端口被占用,换一个重试");
+                }
+                Err(e) => {
+                    dozer_core::log_error!(LOG, error = %e, "gateway 启动失败,应用宿主不可用");
+                    return Self::unavailable(format!("应用宿主不可用:{e}"));
+                }
+            }
+        }
+        Self::unavailable(format!(
+            "应用宿主不可用:连续 {} 次选到的端口都被占用",
+            Self::FIRST_RUN_ATTEMPTS
+        ))
+    }
+
     /// 用给定的 gateway 配置启动(测试用 `port: 0`)。永不失败。
     pub async fn start_with(root: &Path, config: GatewayConfig) -> Arc<Self> {
@@ -61,4 +94,8 @@ impl AppService {
             }
         };
+        Self::finish_start(root, gateway).await
+    }
+
+    async fn finish_start(root: &Path, gateway: Arc<Gateway>) -> Arc<Self> {
         let manager = match AppManager::new(root, HOST_VERSION, gateway.clone()) {
             Ok(m) => Arc::new(m),
@@ -81,8 +118,10 @@ impl AppService {
     }
 
-    pub async fn handle(&self, request: AppRequest) -> Result<AppReply, String> {
+    pub async fn handle(&self, request: AppRequest) -> Result<AppReply, AppFailure> {
         let manager = match &self.state {
             State::Ready { manager, .. } => manager,
-            State::Unavailable(reason) => return Err(reason.clone()),
+            State::Unavailable(reason) => {
+                return Err(AppFailure::new(AppErrorKind::Unavailable, reason.clone()));
+            }
         };
         match request {
@@ -124,5 +163,7 @@ impl AppService {
                 let probes = tokio::task::spawn_blocking(|| probe_all(&SystemRunner::default()))
                     .await
-                    .map_err(|e| format!("探测任务失败: {e}"))?;
+                    .map_err(|e| {
+                        AppFailure::new(AppErrorKind::Internal, format!("探测任务失败: {e}"))
+                    })?;
                 Ok(AppReply::Runtimes {
                     runtimes: probes
@@ -171,5 +212,5 @@ fn log_report(report: &bytehost_apps::manager::ReconcileReport, what: &str) {
 }
 
-async fn blocking<T, F>(manager: &Arc<AppManager>, f: F) -> Result<T, String>
+async fn blocking<T, F>(manager: &Arc<AppManager>, f: F) -> Result<T, AppFailure>
 where
     T: Send + 'static,
@@ -179,6 +220,6 @@ where
     tokio::task::spawn_blocking(move || f(&manager))
         .await
-        .map_err(|e| format!("应用宿主任务失败: {e}"))?
-        .map_err(|e| e.to_string())
+        .map_err(|e| AppFailure::new(AppErrorKind::Internal, format!("应用宿主任务失败: {e}")))?
+        .map_err(|e| AppFailure::new(e.kind(), e.to_string()))
 }
 
@@ -310,5 +351,7 @@ source = "web/"
             AppRequest::Stop { id: id("a") },
         ] {
-            assert_eq!(svc.handle(req).await.unwrap_err(), "原因 X");
+            let failure = svc.handle(req).await.unwrap_err();
+            assert_eq!(failure.message, "原因 X");
+            assert_eq!(failure.kind, AppErrorKind::Unavailable);
         }
         svc.shutdown().await; // 不可用的服务关停也是空操作
@@ -322,5 +365,5 @@ source = "web/"
         let taken = squatter.port();
         let svc = AppService::start_with(tmp.path(), GatewayConfig { port: taken }).await;
-        let err = svc.handle(AppRequest::List).await.unwrap_err();
+        let err = svc.handle(AppRequest::List).await.unwrap_err().message;
         assert!(
             err.contains(&taken.to_string()) && err.contains("占用"),
@@ -337,5 +380,5 @@ source = "web/"
         let svc =
             AppService::start_with(&blocker.join("bytehost"), GatewayConfig { port: 0 }).await;
-        let err = svc.handle(AppRequest::List).await.unwrap_err();
+        let err = svc.handle(AppRequest::List).await.unwrap_err().message;
         assert!(err.contains("应用目录"), "{err}");
     }
@@ -468,5 +511,5 @@ source = "web/"
         std::fs::write(root.join("gateway.json"), "{not json").unwrap();
         let svc = AppService::start(&root).await;
-        let err = svc.handle(AppRequest::List).await.unwrap_err();
+        let err = svc.handle(AppRequest::List).await.unwrap_err().message;
         assert!(
             err.contains("应用宿主不可用") && err.contains("gateway.json"),
@@ -539,5 +582,8 @@ source = "web/"
             .await
             .unwrap_err();
-        assert!(err.contains("停止"), "{err}");
+        assert!(
+            err.message.contains("停止") && err.kind == AppErrorKind::Unavailable,
+            "{err}"
+        );
         let b = write_app(&tmp.path().join("src/b"), "beta", "B");
         let plan = svc
@@ -563,5 +609,8 @@ source = "web/"
             .await
             .unwrap_err();
-        assert!(err.contains("停止"), "{err}");
+        assert!(
+            err.message.contains("停止") && err.kind == AppErrorKind::Unavailable,
+            "{err}"
+        );
         let AppReply::Apps { apps } = svc.handle(AppRequest::List).await.unwrap() else {
             panic!()
@@ -595,3 +644,89 @@ source = "web/"
         svc.shutdown().await;
     }
+    /// 一个已被占住的端口(持有 listener 直到测试结束)。
+    fn squat() -> (std::net::TcpListener, u16) {
+        let l = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
+        let p = l.local_addr().unwrap().port();
+        (l, p)
+    }
+
+    fn free_port() -> u16 {
+        squat().1
+    }
+
+    fn persisted_port(root: &Path) -> Option<u16> {
+        bytehost_apps::port::load_port(root).unwrap()
+    }
+
+    /// 首次运行选中的端口恰好被占:换一个再试,**只记住真正绑上的那个**。
+    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
+    async fn the_first_run_retries_a_taken_port_and_persists_only_the_one_it_bound() {
+        let tmp = tempfile::tempdir().unwrap();
+        let root = tmp.path().join("bytehost");
+        let (_held, taken) = squat();
+        let good = free_port();
+        let mut picks = vec![taken, good].into_iter();
+        let svc = AppService::first_run(&root, move || picks.next().unwrap()).await;
+        assert!(svc.handle(AppRequest::List).await.is_ok(), "服务可用");
+        assert_eq!(persisted_port(&root), Some(good));
+        svc.shutdown().await;
+    }
+
+    /// 连续都被占:服务不可用,并且**什么都没写盘**(下次启动还能重新选)。
+    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
+    async fn the_first_run_gives_up_after_repeated_collisions_without_persisting_anything() {
+        let tmp = tempfile::tempdir().unwrap();
+        let root = tmp.path().join("bytehost");
+        let (_held, taken) = squat();
+        let svc = AppService::first_run(&root, move || taken).await;
+        let failure = svc.handle(AppRequest::List).await.unwrap_err();
+        assert_eq!(failure.kind, AppErrorKind::Unavailable);
+        assert!(failure.message.contains("占用"), "{failure}");
+        assert_eq!(persisted_port(&root), None, "没绑上的端口不被记住");
+    }
+
+    /// 端口写不进磁盘:不能带着一个下次会变的端口继续跑——服务不可用,gateway 被撤下。
+    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
+    async fn the_first_run_is_unavailable_when_the_port_cannot_be_saved() {
+        let tmp = tempfile::tempdir().unwrap();
+        let blocker = tmp.path().join("not-a-dir");
+        std::fs::write(&blocker, "x").unwrap();
+        let port = free_port();
+        let svc = AppService::first_run(&blocker.join("bytehost"), move || port).await;
+        let failure = svc.handle(AppRequest::List).await.unwrap_err();
+        assert_eq!(failure.kind, AppErrorKind::Unavailable);
+        assert!(failure.message.contains("无法保存"), "{failure}");
+        assert!(svc.gateway_stopped());
+        assert!(
+            std::net::TcpListener::bind(("127.0.0.1", port)).is_ok(),
+            "gateway 已释放端口"
+        );
+    }
+
+    /// 失败带类别:相对路径的来源是 `Rejected`,没装的应用是 `NotFound`。
+    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
+    async fn failures_are_classified_not_just_described() {
+        let tmp = tempfile::tempdir().unwrap();
+        let svc =
+            AppService::start_with(&tmp.path().join("bytehost"), GatewayConfig { port: 0 }).await;
+        let plan = AppRequest::Plan {
+            source: AppSource::LocalDir {
+                path: "relative/dir".into(),
+            },
+            provenance: Provenance::Local,
+            trust: TrustLevel::Trusted,
+        };
+        assert_eq!(
+            svc.handle(plan).await.unwrap_err().kind,
+            AppErrorKind::Rejected
+        );
+        assert_eq!(
+            svc.handle(AppRequest::Stop { id: id("ghost") })
+                .await
+                .unwrap_err()
+                .kind,
+            AppErrorKind::NotFound
+        );
+        svc.shutdown().await;
+    }
 }
```

- [ ] **Step 4: GREEN。** `cargo fmt -p dozerd -p bytehost-apps && cargo test -p bytehost-apps --all-features && cargo test -p dozerd --lib app_service && bash scripts/check-bytehost-apps-deps.sh` → 全过(`app_service` 14 个)。
- [ ] **Step 5: 变异检查。** 把 `for _ in 0..Self::FIRST_RUN_ATTEMPTS` 改成 `for _ in 0..1` → `the_first_run_retries_a_taken_port_and_persists_only_the_one_it_bound` 必须 FAILED;还原。
- [ ] **Step 6: 全量。** `cargo clippy -p bytehost-apps -p dozerd -p dozer-client --all-targets` 无新警告;`cargo test -p dozerd` 全过。
- [ ] **Step 7: Commit + 文档。** `git commit -am "feat(bytehost-apps, dozerd): persist the gateway port only after it bound; retry on first-run collisions (A4a task 2)"`;并在规格 A4 行后追加"A4 拆为 A4a/A4b/A4c,见 `plans/2026-10-05-bytehost-a4a-failure-kinds-and-port.md`",同一个提交里把 `CLAUDE.md` 的 bytehost-apps 行补一句"失败带类别(`AppErrorKind`)"。

## 已知局限

- `AppReply::Failed` 之外,传输层/协议层错误(连不上 dozerd、协议版本不符)仍是普通 `anyhow` 错误,`downcast_ref::<AppFailure>()` 取不到——GUI 要把"取不到"当成"dozerd 不可用"处理(A4b)。
- 首次运行的重试只处理 `PortInUse`;其他绑定错误(权限、地址不可用)直接判不可用,不重试。
