# Browser 面板(收藏夹)迁移到服务 trait Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 浏览器面板的收藏夹(`extensions/browser.rs`)的 3 个服务调用改为经 `dozer-services` 的 `BookmarkService`;收藏夹读取(刷新)失败不再把本地缓存清空,并按"`Unavailable` 静默、其余写内联 `state.error`"提示。

**Architecture:** `dozer-services` 新增 `BookmarkService`(3 个方法)与 `Services.bookmark`;`Client` 实现它(复用 `into_service_error`);`Message::BookmarksLoaded` 的载荷改为 `Result<Vec<BookmarkInfo>, ServiceError>`(不新增变体、不新增 host 臂);`update`/`request_bookmarks_refresh` 改吃 `&Arc<dyn BookmarkService>`;host 14 处调用点用 `&services_for(<client>).bookmark` 逐处机械替换。写操作失败文本与乐观更新逻辑不变。

**Tech Stack:** Rust 2024 workspace、tokio、iced 0.14(只涉及面板 update/spawn 层)、Python 门禁脚本。

**Spec:** `docs/superpowers/specs/2026-10-11-bookmark-services-design.md`(先例:`2026-10-11-conversation-services-design.md`、`2026-10-10-todo-services-design.md`)

## Global Constraints

- `dozer-services` 只依赖 `dozer-model`;不加 `dozer-core`/`dozer-client`/`tokio` 到 `[dependencies]`。
- 线上协议零变化:`Request`/`Reply` 不动,`crates/dozer-core/tests/model_golden.rs` 不改且通过。
- 面板日志来源名是 **`web`**(`PanelKind::Web` 的面板名,`app/state.rs` 的测试会扫源码强制);在 `browser.rs` 顶部声明 `dozer_core::scope!(LOG, panel, "web");`(第一次写日志才声明,本次要写 debug 日志);不新增裸 `tracing::`/`eprintln!`(`scripts/check-log-scope.sh`)。
- 写失败文本逐字不变(`e.message` 等于原 `e.to_string()`),仍写进 `state.error`;乐观更新(`optimistic_add`/`optimistic_remove`)与 `Project` scope 无 `project_id` 时不发 RPC 的既有逻辑不变。
- 读失败:保留本地缓存;`Unavailable` 只写 debug 日志;其余 `state.error = Some(format!("收藏夹加载失败: {message}"))`。**不给 `State` 加 outbox、不改 `drain_outboxes`**。
- `R-HOST-PANEL-ARMS` 不上升:不新增 `app/update.rs` 的面板专属臂。
- 在独立 worktree 实施;不提交用户 WIP(`Info.plist`、版本号相关改动);`Cargo.lock` 提交前 `git checkout Cargo.lock`(main 上已提交的锁文件缺 bytegit/byteui 的 `source` 行,干净 worktree 里跑 cargo 会补上,属噪声)。
- main 上已知问题不修不算回归:`files::tests::delete_confirm_spec_reflects_pending_target` 失败、并行负载下的间歇失败(`git_log::build_*`、`assets::tests::serves_vendored_asset_with_mime`、dozerd 的 `memory`/`headless_agent` 测试)、门禁 `R-HOST-EXECUTORS 0 -> 1`。
- 提交信息末尾加 `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`。

## Review Focus

- **读失败不得清空本地缓存**(此前 `unwrap_or_default()` + 整体替换);`Unavailable` 不写 `state.error`;`Rejected`/`Protocol` 写 `"收藏夹加载失败: {message}"`;缓存保持 → Task 4 测试。
- `BookmarkAdd` 把 `scope`、`target_project_id`(`Global` → `None`,`Project` → 传入的 `project_id`)、URL、标题原样传给服务;`Project` scope 且无 `project_id` 时**不调服务** → Task 4 测试。
- 写失败得到 `BookmarksMutated(_, Err(服务消息))`,文本逐字不变;`BookmarksMutated` 之后仍触发一次 `list` → Task 4 测试。
- host 14 处调用点(`app/update.rs` 12 处 `browser::update`、`workspace/state.rs` 2 处 `request_bookmarks_refresh`)全部切换,`BookmarksLoaded` 载荷类型变化在 `browser_bookmarks_loaded` 与两处构造点一致 → Task 4 编译期 + 全量测试。
- `impl BookmarkService for Client` 必须调 inherent 方法而不是递归调 trait 方法;`add`/`remove` 返回 `()` → Task 2 适配测试。

---

### Task 1: `BookmarkService` trait(`dozer-services`)

**Files:**
- Modify: `crates/dozer-services/src/lib.rs`

**Interfaces:**
- Consumes: `dozer_model::{BookmarkInfo, BookmarkScope}`(已在 `dozer-model`)。
- Produces: `pub trait BookmarkService: Send + Sync`,返回 `BoxFuture<'_, Result<_, ServiceError>>`,参数按值传:
  - `list_bookmarks(project_id: Option<i64>) -> Vec<BookmarkInfo>`
  - `add_bookmark(scope: BookmarkScope, project_id: Option<i64>, url: String, title: String) -> ()`
  - `remove_bookmark(id: i64) -> ()`

- [ ] **Step 0: 记录基线(只测量,写进 ledger;同步执行,跑完再进 Step 1,避免并发编译污染)**

```bash
W=$(ls -d .superpowers/sdd/*/ | head -1)
grep -c 'dozer_client\|io\.client()' crates/dozer-app/src/extensions/browser.rs     # 期望 1
cargo test --workspace --no-fail-fast > ${W}baseline-tests.txt 2>&1; grep -E '^test result' ${W}baseline-tests.txt | awk '{s+=$4;f+=$6} END{print "passed",s,"failed",f}'; grep -E '^test .* FAILED' ${W}baseline-tests.txt
cargo clippy --all-targets 2>&1 | grep -c '^warning'
python3 scripts/audit/check_panel_boundary.py; echo exit=$?
```
Expected: 失败仅已知的;门禁仅 `R-HOST-EXECUTORS 0 -> 1`;把数字写进 ledger。

- [ ] **Step 1: 写失败测试**

在 `crates/dozer-services/src/lib.rs` 现有 `#[cfg(test)] mod tests` 末尾追加(`unit()` 等辅助已存在):

```rust
    // ---- BookmarkService 契约 ----

    use dozer_model::{BookmarkInfo, BookmarkScope};

    struct MemBookmark(Result<Vec<BookmarkInfo>, ServiceError>);

    impl BookmarkService for MemBookmark {
        fn list_bookmarks(
            &self,
            _p: Option<i64>,
        ) -> BoxFuture<'_, Result<Vec<BookmarkInfo>, ServiceError>> {
            let r = self.0.clone();
            Box::pin(async move { r })
        }
        fn add_bookmark(
            &self,
            _s: BookmarkScope,
            _p: Option<i64>,
            _u: String,
            _t: String,
        ) -> BoxFuture<'_, Result<(), ServiceError>> {
            unit()
        }
        fn remove_bookmark(&self, _i: i64) -> BoxFuture<'_, Result<(), ServiceError>> {
            unit()
        }
    }

    /// `handle.spawn` 要 `'static + Send`:持有 `Arc<dyn BookmarkService>` 的 async 块必须满足。
    #[test]
    fn dyn_bookmark_futures_run_inside_tokio_spawn() {
        let svc: Arc<dyn BookmarkService> = Arc::new(MemBookmark(Ok(vec![])));
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let (list, added) = rt.block_on(async move {
            tokio::spawn(async move {
                let list = svc.list_bookmarks(Some(7)).await;
                let added = svc
                    .add_bookmark(BookmarkScope::Global, None, "http://a.com".into(), "A".into())
                    .await;
                (list, added)
            })
            .await
            .unwrap()
        });
        assert_eq!(list, Ok(vec![]));
        assert_eq!(added, Ok(()));
    }

    #[test]
    fn bookmark_read_failure_is_distinguishable_from_an_empty_list() {
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let empty = MemBookmark(Ok(vec![]));
        let down = MemBookmark(Err(ServiceError::new(ServiceErrorKind::Unavailable, "x")));
        assert!(rt.block_on(empty.list_bookmarks(None)).is_ok());
        assert_eq!(
            rt.block_on(down.list_bookmarks(None)).unwrap_err().kind,
            ServiceErrorKind::Unavailable
        );
    }
```
(`BookmarkInfo` 需要 `PartialEq` 才能 `assert_eq!(list, Ok(vec![]))`——`vec![]` 比较的是空 Vec,只要 `BookmarkInfo: PartialEq` 即可;它在 `dozer-model` 里已派生。)

Run: `cargo test -p dozer-services 2>&1 | tail -8` → Expected: 编译失败 `cannot find trait BookmarkService`(RED)。

- [ ] **Step 2: 实现**

顶部 `use dozer_model::{...}` 合并追加 `BookmarkInfo, BookmarkScope`。在 `EditHistoryService` 之后、`Services` 之前加:

```rust
/// 浏览器收藏夹的读写。参数按值传。
pub trait BookmarkService: Send + Sync {
    fn list_bookmarks(
        &self,
        project_id: Option<i64>,
    ) -> BoxFuture<'_, Result<Vec<BookmarkInfo>, ServiceError>>;
    fn add_bookmark(
        &self,
        scope: BookmarkScope,
        project_id: Option<i64>,
        url: String,
        title: String,
    ) -> BoxFuture<'_, Result<(), ServiceError>>;
    fn remove_bookmark(&self, id: i64) -> BoxFuture<'_, Result<(), ServiceError>>;
}
```
**不要**在本任务给 `Services` 加字段(Task 3 做,否则 `dozer-app` 编译失败)。

Run: `cargo test -p dozer-services 2>&1 | grep -E 'test result|error'` → Expected: 全绿(原 19 个 + 新 2 个 = 21 passed)。

- [ ] **Step 3: 依赖验收与提交**

```bash
cargo tree -p dozer-services --edges normal | grep -vE 'dozer-model|serde|proc-macro2|quote|syn|unicode-ident|dozer-services' | head   # 期望无输出
cargo fmt -p dozer-services && cargo clippy -p dozer-services --all-targets 2>&1 | grep -c '^warning'   # 期望 0
git add crates/dozer-services && git commit -m "feat(services): Bookmark S1 BookmarkService trait

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 2: `Client` 实现 `BookmarkService`

**Files:**
- Modify: `crates/dozer-client/src/lib.rs`(impl + 测试模块 `bookmark_service_tests`)

**Interfaces:**
- Consumes: Task 1;`into_service_error`、`BoxFuture`、`ServiceError`;`todo_service_tests::serve_once`(已 `pub(super)`)。
- Produces: `impl BookmarkService for Client`(同名 inherent 方法不改)。

- [ ] **Step 1: 写失败测试**

应答变体已核对(`dozer-client/src/lib.rs` 约 372–408 行):`list` → `Reply::Bookmarks { bookmarks }`,`add`/`remove` → `Reply::Ok`。在文件末尾追加:

```rust
#[cfg(test)]
mod bookmark_service_tests {
    use super::todo_service_tests::serve_once;
    use super::*;
    use dozer_services::{BookmarkService, ServiceErrorKind};

    #[tokio::test]
    async fn list_comes_back_as_ok() {
        let sock = serve_once("bm-list", Some(Reply::Bookmarks { bookmarks: vec![] }));
        let got = BookmarkService::list_bookmarks(&Client::new(sock.clone()), Some(7)).await;
        assert_eq!(got, Ok(vec![]));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn add_and_remove_return_unit_on_success() {
        let sock = serve_once("bm-add", Some(Reply::Ok));
        let got = BookmarkService::add_bookmark(
            &Client::new(sock.clone()),
            dozer_core::protocol::BookmarkScope::Global,
            None,
            "http://a.com".into(),
            "A".into(),
        )
        .await;
        assert_eq!(got, Ok(()));
        let _ = std::fs::remove_file(&sock);

        let sock = serve_once("bm-remove", Some(Reply::Ok));
        let got = BookmarkService::remove_bookmark(&Client::new(sock.clone()), 3).await;
        assert_eq!(got, Ok(()));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn hang_up_is_unavailable_with_the_legacy_text() {
        let sock = serve_once("bm-hangup", None);
        let e = BookmarkService::list_bookmarks(&Client::new(sock.clone()), None)
            .await
            .unwrap_err();
        assert_eq!((e.kind, e.message.as_str()), (ServiceErrorKind::Unavailable, "daemon 断开"));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn error_reply_is_rejected_with_the_legacy_prefix() {
        let sock = serve_once("bm-reject", Some(Reply::Error { message: "boom".into() }));
        let e = BookmarkService::remove_bookmark(&Client::new(sock.clone()), 3)
            .await
            .unwrap_err();
        assert_eq!((e.kind, e.message.as_str()), (ServiceErrorKind::Rejected, "daemon 错误: boom"));
        let _ = std::fs::remove_file(&sock);
    }
}
```
Run: `cargo test -p dozer-client bookmark_service_tests 2>&1 | grep -E '^error' | head -3` → Expected: 编译失败(`Client: BookmarkService` 未实现)(RED)。

- [ ] **Step 2: 实现**

顶部 `use dozer_services::{...}` 追加 `BookmarkService`。在 `impl EditHistoryService for Client` 之后追加(用 `Client::method(self, ..)` 路径调 inherent 方法):

```rust
impl BookmarkService for Client {
    fn list_bookmarks(
        &self,
        project_id: Option<i64>,
    ) -> BoxFuture<'_, Result<Vec<dozer_core::protocol::BookmarkInfo>, ServiceError>> {
        Box::pin(async move {
            Client::list_bookmarks(self, project_id)
                .await
                .map_err(into_service_error)
        })
    }
    fn add_bookmark(
        &self,
        scope: dozer_core::protocol::BookmarkScope,
        project_id: Option<i64>,
        url: String,
        title: String,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            Client::add_bookmark(self, scope, project_id, &url, &title)
                .await
                .map_err(into_service_error)
        })
    }
    fn remove_bookmark(&self, id: i64) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            Client::remove_bookmark(self, id)
                .await
                .map_err(into_service_error)
        })
    }
}
```
类型路径以文件里已有 import 为准(`BookmarkInfo`/`BookmarkScope` 在该文件顶部已经 `use dozer_core::protocol::{…}`,则去掉前缀)。

Run: `cargo test -p dozer-client 2>&1 | grep -E 'test result|FAILED|^error'` → Expected: 原有测试 + 4 个新测试全绿。

- [ ] **Step 3: 提交**

```bash
cargo fmt -p dozer-client
cargo clippy -p dozer-client --all-targets 2>&1 | grep -c '^warning'   # 期望 0
git checkout Cargo.lock 2>/dev/null
git add crates/dozer-client && git status --short
git commit -m "feat(client): Bookmark S2 Client 实现 BookmarkService

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 3: `Services.bookmark` + 默认适配 + `FakeBookmark`

**Files:**
- Modify: `crates/dozer-services/src/lib.rs`(`Services` 加字段;测试里的构造)
- Modify: `crates/dozer-app/src/panel_host.rs`(`services_for`)
- Modify: `crates/dozer-app/src/service_fakes.rs`(新增 `FakeBookmark`;`base_services()` 补字段)

**Interfaces:**
- Consumes: Task 1/2。
- Produces:
  - `Services { usage, todo, project, memory, group_chat, conversation, codehealth, context, edit_history, bookmark: Arc<dyn BookmarkService> }`;`services_for(&Client)` 也适配 `bookmark`。
  - `FakeBookmark`(`Clone`,克隆共享状态,`Default` = 全部成功、列表为空),链式设置:`.with_items(Vec<BookmarkInfo>)`、`.with_list_error(kind, message)`、`.with_write_error(kind, message)`(add 与 remove 共用);`calls() -> Vec<String>`(格式 `"list_bookmarks(Some(1))"` / `"list_bookmarks(None)"`、`"add_bookmark(Project,Some(7),http://a.com,标题)"`、`"remove_bookmark(3)"`,`scope` 与 `project_id` 用 `{:?}`);`into_services()`(`Services { bookmark: Arc::new(self), ..base_services() }`)。

- [ ] **Step 1: 写失败测试**

(a) `panel_host.rs` 测试模块(`default_services_route_edit_history_…` 旁)追加:
```rust
    #[test]
    fn default_services_route_bookmark_through_the_client_adapter() {
        let rt = test_runtime();
        let (io, _rx) = recording_io(&rt);
        // recording_io 的 Client 指向不存在的 socket → Unavailable
        let e = rt
            .block_on(io.services().bookmark.list_bookmarks(None))
            .unwrap_err();
        assert_eq!(e.kind, dozer_services::ServiceErrorKind::Unavailable);
    }
```
(b) `dozer-services` 的 `services_is_cloneable_and_shares_the_service` 同时构造 `bookmark: Arc::new(MemBookmark(Ok(vec![])))` 并追加 `Arc::ptr_eq(&s.bookmark, &t.bookmark)`。

Run: `cargo test -p dozer-services 2>&1 | grep -E '^error' | head -3` → Expected: 编译失败(`Services` 没有字段 `bookmark`)(RED)。

- [ ] **Step 2: 实现**

1. `Services` 加 `pub bookmark: Arc<dyn BookmarkService>,`;`services_for` 加 `bookmark: Arc::new(client.clone()),`。
2. `service_fakes.rs`:实现 `FakeBookmark`,**模式同已有的 `FakeContext`**(`Arc<Mutex<State>>`,`State { items: Vec<BookmarkInfo>, list_error/write_error: Option<ServiceError>, calls: Vec<String> }`)。`base_services()` 补 `bookmark: Arc::new(FakeBookmark::default())`。
3. 新增的构造函数在 Task 4 之前未被使用时,加 `#[allow(dead_code)]`(Task 4 去掉)。

Run: `cargo test -p dozer-services 2>&1 | grep -E 'test result|^error'`;`cargo test -p dozer-app default_services_route_bookmark 2>&1 | grep -E 'test result|^error'` → Expected: 通过。

- [ ] **Step 3: 全 workspace 编译与提交**

```bash
cargo build --workspace 2>&1 | grep -E '^error'          # 期望无输出
cargo test -p dozer-app -- panel_host usage_probe 2>&1 | grep -E 'test result|FAILED'
cargo clippy --all-targets 2>&1 | grep -c '^warning'      # 期望 ≤ 基线
cargo fmt -p dozer-app -p dozer-services
git checkout Cargo.lock 2>/dev/null
git add crates/dozer-services crates/dozer-app/src && git status --short
git commit -m "feat(host): Bookmark S3 Services.bookmark + FakeBookmark

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 4: 面板与 host 切换,收藏夹读失败保留缓存

> 说明(Ruling,计划阶段):`update`/`request_bookmarks_refresh` 的签名、`BookmarksLoaded` 的载荷类型与 host 的 14 处调用点必须在同一个提交里改,否则编译不过。`update` 的新参数选 `service: &Arc<dyn BookmarkService>`(与 Todo 同形),这样 host 各处可直接写 `&crate::panel_host::services_for(<client>).bookmark`,不需要新增 `let` 绑定。

**Files:**
- Modify: `crates/dozer-app/src/extensions/browser.rs`(`LOG` 声明、`BookmarksLoaded` 载荷、`update`、`request_bookmarks_refresh`、去掉 `use dozer_client::Client`、测试)
- Modify: `crates/dozer-app/src/app/update.rs`(12 处 `browser::update`;`browser_bookmarks_loaded` 的参数类型)
- Modify: `crates/dozer-app/src/workspace/state.rs`(2 处 `request_bookmarks_refresh`)
- Modify: `crates/dozer-app/src/service_fakes.rs`(去掉临时 `#[allow(dead_code)]`)
- Modify: `crates/dozer-app/src/usage_probe/host.rs`(行号引用若漂移,只改行号)

**Interfaces:**
- Consumes: Task 3 的 `Services`、`services_for`、`FakeBookmark`;`ServiceError`/`ServiceErrorKind`。
- Produces:
  - `Message::BookmarksLoaded(Option<i64>, Result<Vec<BookmarkInfo>, dozer_services::ServiceError>)`
  - `update(state: &mut State, msg: Message, project_id: Option<i64>, service: &Arc<dyn BookmarkService>, handle: &Handle, emit: impl Fn(Message) + Send + 'static)`
  - `request_bookmarks_refresh(project_id: Option<i64>, service: &Arc<dyn BookmarkService>, handle: &Handle, emit: impl Fn(Message) + Send + 'static)`

- [ ] **Step 1: 写失败测试**

在 `browser.rs` 的 `mod tests` 追加(该模块已有 `bm(id, scope, project_id, url)` 辅助;另加 `use crate::panel_host::testing::{TIMEOUT, runtime}; use crate::service_fakes::FakeBookmark; use dozer_services::ServiceErrorKind; use std::sync::mpsc;`。这些新测试用**普通 `#[test]` + 多线程 `runtime()`**(现有 `#[tokio::test]` 是单线程,在里面阻塞等通道会死锁)):

```rust
    fn collect() -> (impl Fn(Message) + Send + 'static, mpsc::Receiver<Message>) {
        let (tx, rx) = mpsc::channel();
        (move |m| { let _ = tx.send(m); }, rx)
    }

    fn fake_service(fake: &FakeBookmark) -> std::sync::Arc<dyn dozer_services::BookmarkService> {
        fake.clone().into_services().bookmark
    }

    #[test]
    fn refresh_emits_the_list_or_the_service_error() {
        let rt = runtime();
        let fake = FakeBookmark::default()
            .with_items(vec![bm(1, BookmarkScope::Global, None, "https://a.com")]);
        let (emit, rx) = collect();
        request_bookmarks_refresh(Some(1), &fake_service(&fake), rt.handle(), emit);
        match rx.recv_timeout(TIMEOUT).unwrap() {
            Message::BookmarksLoaded(Some(1), Ok(v)) => assert_eq!(v.len(), 1),
            other => panic!("unexpected: {other:?}"),
        }
        assert_eq!(fake.calls(), vec!["list_bookmarks(Some(1))".to_string()]);

        let fake = FakeBookmark::default().with_list_error(ServiceErrorKind::Rejected, "daemon 错误: boom");
        let (emit, rx) = collect();
        request_bookmarks_refresh(None, &fake_service(&fake), rt.handle(), emit);
        match rx.recv_timeout(TIMEOUT).unwrap() {
            Message::BookmarksLoaded(None, Err(e)) => {
                assert_eq!((e.kind, e.message.as_str()), (ServiceErrorKind::Rejected, "daemon 错误: boom"));
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn a_failed_load_keeps_the_cache_and_unavailable_stays_silent() {
        let rt = runtime();
        let cached = vec![bm(1, BookmarkScope::Global, None, "https://a.com")];
        let mut state = State { bookmarks: cached.clone(), ..State::default() };
        let (emit, _rx) = collect();
        update(
            &mut state,
            Message::BookmarksLoaded(Some(1), Err(dozer_services::ServiceError::new(ServiceErrorKind::Unavailable, "daemon 断开"))),
            Some(1),
            &fake_service(&FakeBookmark::default()),
            rt.handle(),
            emit,
        );
        assert_eq!(state.bookmarks, cached, "读失败不得清空缓存");
        assert!(state.error.is_none(), "Unavailable 由顶栏徽标表达,不写内联错误");
    }

    #[test]
    fn a_rejected_load_keeps_the_cache_and_sets_the_inline_error() {
        let rt = runtime();
        let cached = vec![bm(1, BookmarkScope::Global, None, "https://a.com")];
        let mut state = State { bookmarks: cached.clone(), ..State::default() };
        let (emit, _rx) = collect();
        update(
            &mut state,
            Message::BookmarksLoaded(Some(1), Err(dozer_services::ServiceError::new(ServiceErrorKind::Rejected, "daemon 错误: boom"))),
            Some(1),
            &fake_service(&FakeBookmark::default()),
            rt.handle(),
            emit,
        );
        assert_eq!(state.bookmarks, cached);
        assert_eq!(state.error.as_deref(), Some("收藏夹加载失败: daemon 错误: boom"));
    }

    #[test]
    fn add_passes_scope_target_project_url_and_title_to_the_service() {
        let rt = runtime();
        let fake = FakeBookmark::default();
        let mut state = State::default();
        let id = state.tabs.open_url("http://a.com".into());
        let _ = id;
        let (url, title) = {
            let t = &state.tabs.tabs()[state.tabs.active_idx()];
            (t.url.clone(), t.title.clone())
        };
        let (emit, rx) = collect();
        update(&mut state, Message::BookmarkAdd(BookmarkScope::Project), Some(7), &fake_service(&fake), rt.handle(), emit);
        assert!(matches!(rx.recv_timeout(TIMEOUT).unwrap(), Message::BookmarksMutated(Some(7), Ok(()))));
        assert_eq!(fake.calls(), vec![format!("add_bookmark(Project,Some(7),{url},{title})")]);

        let fake = FakeBookmark::default();
        let (emit, rx) = collect();
        update(&mut state, Message::BookmarkAdd(BookmarkScope::Global), Some(7), &fake_service(&fake), rt.handle(), emit);
        assert!(matches!(rx.recv_timeout(TIMEOUT).unwrap(), Message::BookmarksMutated(Some(7), Ok(()))));
        assert_eq!(fake.calls(), vec![format!("add_bookmark(Global,None,{url},{title})")], "Global 收藏的目标项目恒为 None");
    }

    #[test]
    fn add_failure_text_is_the_service_message_unchanged() {
        let rt = runtime();
        let fake = FakeBookmark::default().with_write_error(ServiceErrorKind::Rejected, "daemon 错误: boom");
        let mut state = State::default();
        state.tabs.open_url("http://a.com".into());
        let (emit, rx) = collect();
        update(&mut state, Message::BookmarkAdd(BookmarkScope::Global), Some(1), &fake_service(&fake), rt.handle(), emit);
        match rx.recv_timeout(TIMEOUT).unwrap() {
            Message::BookmarksMutated(Some(1), Err(text)) => assert_eq!(text, "daemon 错误: boom"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn remove_calls_the_service_and_mutated_triggers_one_list() {
        let rt = runtime();
        let fake = FakeBookmark::default();
        let mut state = State {
            bookmarks: vec![bm(3, BookmarkScope::Global, None, "https://a.com")],
            ..State::default()
        };
        let (emit, rx) = collect();
        update(&mut state, Message::BookmarkRemove(3), Some(1), &fake_service(&fake), rt.handle(), emit);
        assert!(matches!(rx.recv_timeout(TIMEOUT).unwrap(), Message::BookmarksMutated(Some(1), Ok(()))));
        assert_eq!(fake.calls(), vec!["remove_bookmark(3)".to_string()]);

        let (emit, rx) = collect();
        update(&mut state, Message::BookmarksMutated(Some(1), Ok(())), Some(1), &fake_service(&fake), rt.handle(), emit);
        assert!(matches!(rx.recv_timeout(TIMEOUT).unwrap(), Message::BookmarksLoaded(Some(1), Ok(_))));
        assert_eq!(fake.calls().last().map(String::as_str), Some("list_bookmarks(Some(1))"));
    }
```
另外**先**把已有测试里的 `BookmarksLoaded(Some(1), fresh.clone())` 改为 `BookmarksLoaded(Some(1), Ok(fresh.clone()))`(`update_bookmarks_loaded_replaces_local_cache`)。

Run: `cargo test -p dozer-app extensions::browser 2>&1 | grep -E '^error' | head -6` → Expected: 编译失败(`update`/`request_bookmarks_refresh` 参数类型不符、`BookmarksLoaded` 载荷不符)(RED)。

- [ ] **Step 2: 面板侧实现**

`browser.rs`:
1. 删除 `use dozer_client::Client;`;加 `use dozer_services::{BookmarkService, ServiceErrorKind}; use std::sync::Arc;`;在 import 之后加 `dozer_core::scope!(LOG, panel, "web");`。
2. `Message::BookmarksLoaded(Option<i64>, Vec<BookmarkInfo>)` → `BookmarksLoaded(Option<i64>, Result<Vec<BookmarkInfo>, dozer_services::ServiceError>)`(同步更新其文档注释)。
3. `update` 的参数 `client: &Client` → `service: &Arc<dyn BookmarkService>`。`BookmarkAdd`:
```rust
            let service = service.clone();
            handle.spawn(async move {
                let res = service
                    .add_bookmark(scope, target_project_id, url, title)
                    .await
                    .map_err(|e| e.message);
                emit(Message::BookmarksMutated(project_id, res));
            });
```
(`url`/`title` 在乐观插入时用的是 `&url`/`&title`,此处按值移入;乐观插入之后它们不再被使用。)`BookmarkRemove`:`let service = service.clone();` 后 `service.remove_bookmark(id).await.map_err(|e| e.message)`。`BookmarksMutated` 之后:`request_bookmarks_refresh(project_id, service, handle, emit);`。
4. `Message::BookmarksLoaded` 分支:
```rust
        Message::BookmarksLoaded(_, Ok(bookmarks)) => state.bookmarks = bookmarks,
        Message::BookmarksLoaded(_, Err(e)) => {
            // 读失败:缓存保持不动(此前 `unwrap_or_default()` 会把失败变成空列表整体替换)。
            if e.kind == ServiceErrorKind::Unavailable {
                dozer_core::log_debug!(LOG, "收藏夹读取失败(dozerd 不可用): {}", e.message);
            } else {
                state.error = Some(format!("收藏夹加载失败: {}", e.message));
            }
        }
```
5. `request_bookmarks_refresh`:
```rust
pub fn request_bookmarks_refresh(
    project_id: Option<i64>,
    service: &Arc<dyn BookmarkService>,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    let service = service.clone();
    handle.spawn(async move {
        let result = service.list_bookmarks(project_id).await;
        emit(Message::BookmarksLoaded(project_id, result));
    });
}
```
6. 测试辅助:`client_for_test()` 整个函数删除;新增
```rust
    fn service_for_test() -> std::sync::Arc<dyn dozer_services::BookmarkService> {
        crate::service_fakes::FakeBookmark::default().into_services().bookmark
    }
```
全模块里 `let client = client_for_test();` → `let service = service_for_test();`,以及各处 `&client,`/`&client)` 参数 → `&service,`/`&service)`(可用 `sed -i '' -e 's/let client = client_for_test();/let service = service_for_test();/' -e 's/&client,/\&service,/g'` 并检查 `update(&mut state, m, Some(1), &client, &handle, |_| {})` 这类单行形式一并替换;替换后 `grep -n 'client' extensions/browser.rs` 在测试区应无残留)。

- [ ] **Step 3: host 侧实现**

- `app/update.rs` 的 12 处 `browser::update(…)`(用 `grep -n 'browser::update(' crates/dozer-app/src/app/update.rs` 定位):把实参里的 client 换成 `&crate::panel_host::services_for(<原 client 表达式去掉 &>).bookmark`:`&client` → `&crate::panel_host::services_for(&client).bookmark`;`&self.client` → `&crate::panel_host::services_for(&self.client).bookmark`;`&io.client` → `&crate::panel_host::services_for(&io.client).bookmark`。(借用的临时值活到该语句结束,够用。)原来仅为传给 `browser::update` 而存在的 `let client = self.client.clone();` 若变成未使用,删掉。
- `browser_bookmarks_loaded(&mut self, pid: Option<i64>, bookmarks: Result<Vec<BookmarkInfo>, dozer_services::ServiceError>)`,函数体里 `browser::Message::BookmarksLoaded(None, bookmarks)` / `(Some(pid), bookmarks)` 保持;`Message::Browser(BookmarksLoaded(pid, bookmarks)) => self.browser_bookmarks_loaded(pid, bookmarks)` 的臂不变(类型随载荷流转)。
- `workspace/state.rs` 约 650、1106 行的 2 处 `browser::request_bookmarks_refresh(…, &io.client, …)` → `&crate::panel_host::services_for(&io.client).bookmark`。
- 去掉 `service_fakes.rs` 里 Task 3 临时加的 `#[allow(dead_code)]`。

- [ ] **Step 4: 运行面板测试**

```bash
cargo test -p dozer-app extensions::browser 2>&1 | grep -E 'test result|FAILED|^error' -A6 | head -20
```
Expected: 全绿(含 6 个新测试;约 13 个既有 `update_*` 测试因换成成功的假实现继续通过,断言不变)。若某个既有测试依赖"指向不存在 socket 的失败"这一副作用而变红,改为在该测试里显式使用失败的假实现,不改断言语义。

- [ ] **Step 5: 探针守卫、全量、门禁、提交**

```bash
cargo test -p dozer-app usage_probe::host 2>&1 | grep -E 'test result|FAILED|实际是'   # 预计不漂移;若漂移(替换折行)只改 host_site() 与注释里的行号,不改守卫逻辑
W=$(ls -d .superpowers/sdd/*/ | head -1)
cargo test --workspace --no-fail-fast > ${W}t4.txt 2>&1; grep -E '^test result' ${W}t4.txt | awk '{s+=$4;f+=$6} END{print s,f}'; grep -E '^test .* FAILED' ${W}t4.txt
cargo clippy --all-targets 2>&1 | grep -c '^warning'
python3 scripts/audit/check_panel_boundary.py        # 只许剩 R-HOST-EXECUTORS 0 -> 1;R-HOST-PANEL-ARMS 不得上升
bash scripts/check-log-scope.sh | tail -1
cargo fmt -p dozer-app && cargo fmt --check && echo fmt-ok
```
Expected: 失败仅已知的(并行负载下的间歇失败——若出现,单独重跑该测试三次确认);clippy 警告数 ≤ 基线;门禁无新增回退;log scope ok;`app/state.rs` 的"面板日志来源名必须是 PanelKind 面板名"测试通过(`web`)。
```bash
git checkout Cargo.lock 2>/dev/null; git add crates/dozer-app/src && git status --short
git commit -m "feat(browser): Bookmark S4 浏览器收藏夹改用 BookmarkService,读失败保留缓存

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 5: `R-CLIENT` 基线下降、验收与文档回填

**Files:**
- Modify: `scripts/audit/panel-boundary.baseline.json`(仅 `extensions/browser.rs` 的 `R-CLIENT` 条目)
- Modify: `docs/dozer-v2/bytehost-H0/08-client-use-inventory.md`、`CLAUDE.md`

**Interfaces:**
- Produces: `extensions/browser.rs` 不再出现在 `R-CLIENT` 基线里。

- [ ] **Step 1: 确认门禁现状并下调基线**

```bash
grep -n 'dozer_client\|io\.client()' crates/dozer-app/src/extensions/browser.rs | head -3   # 期望无输出
python3 scripts/audit/check_panel_boundary.py   # 此时 browser.rs 的 R-CLIENT 实测应为 0(低于基线),门禁只显示既有 R-HOST-EXECUTORS
python3 - <<'EOF'
import json
p = "scripts/audit/panel-boundary.baseline.json"
base = json.load(open(p, encoding="utf-8"))
rel = "extensions/browser.rs"
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
Expected: 基线 diff 只含 `extensions/browser.rs` 条目的删除;门禁仍只报 `R-HOST-EXECUTORS 0 -> 1`。

- [ ] **Step 2: 验收清单(逐条跑、读输出)**

```bash
cargo tree -p dozer-services --edges normal                    # 1: 仅 dozer-model + serde 系
python3 scripts/audit/unit_edges.py ext:browser                # 2: 无 dozer_client
cargo test -p dozer-app extensions::browser 2>&1 | grep -E 'test result|FAILED'   # 3
cargo test -p dozer-core --test model_golden 2>&1 | grep 'test result'            # 5
W=$(ls -d .superpowers/sdd/*/ | head -1)
cargo test --workspace --no-fail-fast > ${W}final.txt 2>&1; grep -E '^test result' ${W}final.txt | awk '{s+=$4;f+=$6} END{print s,f}'; grep -E '^test .* FAILED' ${W}final.txt   # 4
cargo clippy --all-targets 2>&1 | grep -c '^warning'; cargo fmt --check && echo fmt-ok
bash scripts/check-log-scope.sh && echo log-scope-ok
git status --short                                             # 无 Info.plist、无无关 Cargo 改动
```
任一不满足即不收尾,回到对应任务修。

- [ ] **Step 3: 文档回填并提交**

- `docs/dozer-v2/bytehost-H0/08-client-use-inventory.md` 末尾追加一节"Browser(收藏夹)迁移(2026-10-11)":`extensions/browser.rs` 经 `BookmarkService`(`dozer-services`)取数与写入;`R-CLIENT` 基线中 `browser.rs` 已清零;`BookmarksLoaded` 载荷改为 `Result`,读失败不再清空缓存(`Unavailable` 静默,其余写内联 `state.error`:`收藏夹加载失败: …`);写失败文本与乐观更新不变。面板日志来源名为 `web`。**登记后续项**:`BookmarksLoaded`/`BookmarksMutated` 没有代数保护(慢的旧结果可能覆盖新结果);剩余直接用 `Client` 的面板:`settings`、`project_create`。
- `CLAUDE.md` Workspace 布局表 `dozer-services` 行里"…EditHistory 已迁,其余面板待迁"改为"…EditHistory、Browser 已迁,其余面板待迁"。
```bash
git add scripts/audit docs CLAUDE.md
git commit -m "chore(audit): Bookmark S5 R-CLIENT 基线下调 + 文档回填

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

之后按 executing-plans 的 Final Review(整分支 fresh reviewer)与 finishing-a-development-branch;**合并进 main 属于需先询问用户的外部副作用**,合并前核对 main 上用户 WIP(`Cargo.lock`、`crates/dozer-app/Cargo.toml`、`Info.plist` 等)与本分支的重叠,用 `git stash` → 合并 → `git stash pop`。
