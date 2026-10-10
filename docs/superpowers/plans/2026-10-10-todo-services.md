# Todo 面板迁移到服务 trait Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Todo 面板(`extensions/todo/**`)的 2 个读与 12 个写改为经 `dozer-services` 的 `TodoService` 取数,读失败不再被吞成空列表。

**Architecture:** `dozer-services` 新增 `TodoService`(14 个方法,参数按值传,`BoxFuture` 返回)与 `Services.todo`;`dozer-client::Client` 实现它(复用 X0 的 `into_service_error`);面板的 `update`/`request_*_refresh`/`on_activate` 改吃 `Arc<dyn TodoService>`;读失败走新的 `Message::LoadFailed(ServiceErrorKind, String)`:保留旧列表,`Unavailable` 静默,其余弹带 key 的 Error Toast。

**Tech Stack:** Rust 2024 workspace、tokio、iced 0.14(只涉及面板 update 层)、Python 门禁脚本。

**Spec:** `docs/superpowers/specs/2026-10-10-todo-services-design.md`(X0 背景:`docs/superpowers/specs/2026-10-09-dozer-services-x0-design.md`)

## Global Constraints

- `dozer-services` 只依赖 `dozer-model`;不加 `dozer-core`/`dozer-client`/`tokio` 到 `[dependencies]`。
- 线上协议零变化:`Request`/`Reply` 不动,`crates/dozer-core/tests/model_golden.rs` 不改且通过。
- 写操作失败文本逐字不变(`Mutated`/`CategoryMutated` 里的 `Err(String)` 等于原 `e.to_string()`,即 `ServiceError.message`)。
- 面板日志来源名必须是 `todo`(已声明的 `LOG`);不新增裸 `tracing::`/`eprintln!`(`scripts/check-log-scope.sh`)。
- 瞬时失败走 Toast(`outbox`),不新增面板内 `Option<String>` 错误字段。
- `R-HOST-PANEL-ARMS` 不上升:`LoadFailed` 不新增 `app/update.rs` 的面板专属臂(落在已有 `Message::Todo(msg)` 通用路径)。
- 不动 host 侧 Todo 调用(`get_todo_detail`、`assign_todo_agent`、`process_todo_now`、`todo_detail_process`)与终端相关代码。
- 在独立 worktree 实施;不提交用户 WIP(`Info.plist`、版本号相关改动);`Cargo.lock` 提交前剔除 `source = "git+https://github.com/byteboyai/(bytegit|byteui)?` 行:`perl -ni -e 'print unless /^source = "git\+https:\/\/github.com\/byteboyai\/(bytegit|byteui)\?/' Cargo.lock`。
- main 上已知问题不修不算回归:`files::tests::delete_confirm_spec_reflects_pending_target` 失败、`git_log::tests::build_*` 间歇失败、门禁 `R-HOST-EXECUTORS 0 -> 1`。
- 提交信息末尾加 `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`。

## Review Focus

- 读失败(`LoadFailed`)**不得清空**已有 `items`/`categories`;此前 `Loaded(vec![])` 会整体替换 → Task 4 测试钉住。
- `Unavailable` 不弹 Toast,`Rejected`/`Protocol` 恰好一条且带 key 去重;轮询(`poll_todo_if_visible`)每次失败不能刷屏 → Task 4 测试。
- `CategoryNewChild`/`CategoryNewSibling` 要用新建分类的 `id` 去发 `CategoryRenameStart`:所以 `add_category` 返回 `CategoryInfo` 而不是 `()`(规格写的是全部 `()`,此处有意偏离,见 Task 1 的 Ruling 注记)→ Task 4 测试钉住该流程。
- `CategoryMutated` 的双刷新(分类 + 任务)中任一读失败也只发 `LoadFailed`,不得清空另一份 → Task 4。
- `Client` 与同名 inherent 方法共存:`TodoService for Client` 内部必须调 inherent 方法而不是递归调用 trait 方法 → Task 2 的适配测试会因无限递归/栈溢出或错误结果而暴露。

---

### Task 1: `TodoService` trait(`dozer-services`)

**Files:**
- Modify: `crates/dozer-services/src/lib.rs`

**Interfaces:**
- Consumes: `dozer_model::{TodoInfo, CategoryInfo, CategoryMoveDirection, TodoStoredStatus}`(均已在 `dozer-model`)。
- Produces: `pub trait TodoService: Send + Sync`,14 个方法(全部返回 `BoxFuture<'_, Result<_, ServiceError>>`,参数按值传):
  - `list_todos(project_id: i64) -> Vec<TodoInfo>`、`list_categories(project_id: i64) -> Vec<CategoryInfo>`
  - `add_todo(project_id: i64, text: String)`、`toggle_todo(id: i64, done: bool)`、`edit_todo_text(id: i64, text: String)`、`reorder_todo(id: i64, after_id: Option<i64>)`、`set_todo_plan_date(id: i64, plan_date: Option<String>)`、`set_todo_status(id: i64, status: TodoStoredStatus)`、`set_todo_category(id: i64, category_id: Option<i64>)` —— 都返回 `()`
  - `add_category(project_id: i64, parent_id: Option<i64>, name: String) -> CategoryInfo`(**不是 `()`**,面板要用新分类 id)
  - `rename_category(id: i64, name: String)`、`delete_category(id: i64)`、`reparent_category(id: i64, new_parent_id: Option<i64>)`、`move_category_sibling(id: i64, direction: CategoryMoveDirection)` —— 都返回 `()`

> Ruling(计划阶段):规格 §3.1 把写操作都写成 `()`,但 `todo/update.rs` 的 `CategoryNewChild`/`CategoryNewSibling` 需要 `add_category` 返回的 `CategoryInfo.id`(紧接着发 `CategoryRenameStart(id)`)。因此仅 `add_category` 返回 `CategoryInfo`。代价:若误改成 `()` 新建分类后不再自动进入重命名。

- [ ] **Step 0: 记录基线(只测量,写进 ledger)**

```bash
W=$(ls -d .superpowers/sdd/*/ | head -1)
grep -rc 'dozer_client\|io\.client()' crates/dozer-app/src/extensions/todo | grep -v ':0'     # todo/state.rs、todo/update.rs 的引用数(应各为个位数)
cargo test --workspace --no-fail-fast > ${W}baseline-tests.txt 2>&1; grep -E '^test result' ${W}baseline-tests.txt | awk '{s+=$4;f+=$6} END{print "passed",s,"failed",f}'; grep -E '^test .* FAILED' ${W}baseline-tests.txt
cargo clippy --all-targets 2>&1 | grep -c '^warning'
python3 scripts/audit/check_panel_boundary.py; echo exit=$?
```
Expected: 失败仅已知的(`files::…delete_confirm…`,可能含间歇的 `git_log::build_*`);门禁仅 `R-HOST-EXECUTORS 0 -> 1`;把四个数字写进 ledger。

- [ ] **Step 1: 写失败测试**

在 `crates/dozer-services/src/lib.rs` 现有 `#[cfg(test)] mod tests` 的末尾(最后一个测试之后、模块结尾 `}` 之前)追加。`MemTodo` 实现全部 14 个方法,让测试在 trait 不存在时编译失败(RED):

```rust
    // ---- TodoService 契约 ----

    use dozer_model::{CategoryInfo, CategoryMoveDirection, TodoInfo, TodoStoredStatus};

    fn unit() -> BoxFuture<'static, Result<(), ServiceError>> {
        Box::pin(async { Ok(()) })
    }

    /// 内存实现:读返回预设结果,写一律成功。
    struct MemTodo(Result<Vec<TodoInfo>, ServiceError>);

    impl TodoService for MemTodo {
        fn list_todos(&self, _p: i64) -> BoxFuture<'_, Result<Vec<TodoInfo>, ServiceError>> {
            let r = self.0.clone();
            Box::pin(async move { r })
        }
        fn list_categories(&self, _p: i64) -> BoxFuture<'_, Result<Vec<CategoryInfo>, ServiceError>> {
            Box::pin(async { Ok(vec![]) })
        }
        fn add_todo(&self, _p: i64, _t: String) -> BoxFuture<'_, Result<(), ServiceError>> { unit() }
        fn toggle_todo(&self, _i: i64, _d: bool) -> BoxFuture<'_, Result<(), ServiceError>> { unit() }
        fn edit_todo_text(&self, _i: i64, _t: String) -> BoxFuture<'_, Result<(), ServiceError>> { unit() }
        fn reorder_todo(&self, _i: i64, _a: Option<i64>) -> BoxFuture<'_, Result<(), ServiceError>> { unit() }
        fn set_todo_plan_date(&self, _i: i64, _d: Option<String>) -> BoxFuture<'_, Result<(), ServiceError>> { unit() }
        fn set_todo_status(&self, _i: i64, _s: TodoStoredStatus) -> BoxFuture<'_, Result<(), ServiceError>> { unit() }
        fn set_todo_category(&self, _i: i64, _c: Option<i64>) -> BoxFuture<'_, Result<(), ServiceError>> { unit() }
        fn add_category(&self, _p: i64, _par: Option<i64>, _n: String) -> BoxFuture<'_, Result<CategoryInfo, ServiceError>> {
            Box::pin(async { Err(ServiceError::new(ServiceErrorKind::Protocol, "mem")) })
        }
        fn rename_category(&self, _i: i64, _n: String) -> BoxFuture<'_, Result<(), ServiceError>> { unit() }
        fn delete_category(&self, _i: i64) -> BoxFuture<'_, Result<(), ServiceError>> { unit() }
        fn reparent_category(&self, _i: i64, _p: Option<i64>) -> BoxFuture<'_, Result<(), ServiceError>> { unit() }
        fn move_category_sibling(&self, _i: i64, _d: CategoryMoveDirection) -> BoxFuture<'_, Result<(), ServiceError>> { unit() }
    }

    /// `handle.spawn` 要 `'static + Send`:持有 `Arc<dyn TodoService>` 的 async 块必须满足。
    #[test]
    fn dyn_todo_service_futures_run_inside_tokio_spawn() {
        let svc: Arc<dyn TodoService> = Arc::new(MemTodo(Ok(vec![])));
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let (list, write) = rt.block_on(async move {
            tokio::spawn(async move {
                let list = svc.list_todos(7).await;
                let write = svc.add_todo(7, "x".into()).await;
                (list, write)
            })
            .await
            .unwrap()
        });
        assert_eq!(list, Ok(vec![]));
        assert_eq!(write, Ok(()));
    }

    #[test]
    fn todo_read_failure_is_distinguishable_from_an_empty_list() {
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let empty = MemTodo(Ok(vec![]));
        let down = MemTodo(Err(ServiceError::new(ServiceErrorKind::Unavailable, "x")));
        assert!(rt.block_on(empty.list_todos(1)).is_ok());
        assert_eq!(
            rt.block_on(down.list_todos(1)).unwrap_err().kind,
            ServiceErrorKind::Unavailable
        );
    }
```

Run: `cargo test -p dozer-services 2>&1 | tail -8` → Expected: 编译失败 `cannot find trait TodoService`(RED)。

- [ ] **Step 2: 实现 trait**

在 `lib.rs` 里 `UsageService` 定义之后、`Services` 定义之前加(顶部 `use dozer_model::{ConversationSummary, UsagePayload};` 改为同时引入下面的类型):

```rust
use dozer_model::{
    CategoryInfo, CategoryMoveDirection, ConversationSummary, TodoInfo, TodoStoredStatus,
    UsagePayload,
};
```

```rust
/// Todo 面板的读写。参数按值传(future 不借用调用方局部变量,能进 `handle.spawn`)。
/// 写操作成功只返回 `()`——面板靠随后重新拉列表拿权威状态;唯一例外 `add_category`,
/// 面板要用新分类的 id 立刻进入重命名。
pub trait TodoService: Send + Sync {
    fn list_todos(&self, project_id: i64) -> BoxFuture<'_, Result<Vec<TodoInfo>, ServiceError>>;
    fn list_categories(
        &self,
        project_id: i64,
    ) -> BoxFuture<'_, Result<Vec<CategoryInfo>, ServiceError>>;
    fn add_todo(&self, project_id: i64, text: String) -> BoxFuture<'_, Result<(), ServiceError>>;
    fn toggle_todo(&self, id: i64, done: bool) -> BoxFuture<'_, Result<(), ServiceError>>;
    fn edit_todo_text(&self, id: i64, text: String) -> BoxFuture<'_, Result<(), ServiceError>>;
    fn reorder_todo(
        &self,
        id: i64,
        after_id: Option<i64>,
    ) -> BoxFuture<'_, Result<(), ServiceError>>;
    fn set_todo_plan_date(
        &self,
        id: i64,
        plan_date: Option<String>,
    ) -> BoxFuture<'_, Result<(), ServiceError>>;
    fn set_todo_status(
        &self,
        id: i64,
        status: TodoStoredStatus,
    ) -> BoxFuture<'_, Result<(), ServiceError>>;
    fn set_todo_category(
        &self,
        id: i64,
        category_id: Option<i64>,
    ) -> BoxFuture<'_, Result<(), ServiceError>>;
    fn add_category(
        &self,
        project_id: i64,
        parent_id: Option<i64>,
        name: String,
    ) -> BoxFuture<'_, Result<CategoryInfo, ServiceError>>;
    fn rename_category(&self, id: i64, name: String) -> BoxFuture<'_, Result<(), ServiceError>>;
    fn delete_category(&self, id: i64) -> BoxFuture<'_, Result<(), ServiceError>>;
    fn reparent_category(
        &self,
        id: i64,
        new_parent_id: Option<i64>,
    ) -> BoxFuture<'_, Result<(), ServiceError>>;
    fn move_category_sibling(
        &self,
        id: i64,
        direction: CategoryMoveDirection,
    ) -> BoxFuture<'_, Result<(), ServiceError>>;
}
```

**不要**在本任务给 `Services` 加字段(Task 3 做,否则 `dozer-app` 会编译失败)。

Run: `cargo test -p dozer-services 2>&1 | grep -E 'test result|error'` → Expected: 全部通过(原 4 个 + 新 2 个 = 6 passed)。

- [ ] **Step 3: 依赖验收与提交**

```bash
cargo tree -p dozer-services --edges normal | grep -vE 'dozer-model|serde|proc-macro2|quote|syn|unicode-ident|dozer-services' | head   # 期望无输出
cargo fmt -p dozer-services && cargo clippy -p dozer-services --all-targets 2>&1 | grep -c '^warning'   # 期望 0
git add crates/dozer-services && git commit -m "feat(services): Todo S1 TodoService trait(14 方法)

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 2: `Client` 实现 `TodoService`

**Files:**
- Modify: `crates/dozer-client/src/lib.rs`(新 impl + 测试模块 `todo_service_tests`)

**Interfaces:**
- Consumes: Task 1 的 `TodoService`;X0 的 `into_service_error`、`BoxFuture`、`ServiceError`。
- Produces: `impl dozer_services::TodoService for dozer_client::Client`。同名 inherent 方法(`Client::list_todos` 等)不改。

- [ ] **Step 1: 写失败测试**

`crates/dozer-client/src/lib.rs` 末尾追加。先读 `app_subscribe_tests` 的 `temp_sock`/`fake_server` 写法(约 1500–1560 行)保持风格;本模块自带一个"回一次应答/或直接挂断"的小服务端:

```rust
#[cfg(test)]
mod todo_service_tests {
    use super::*;
    use dozer_services::{ServiceErrorKind, TodoService};

    fn temp_sock(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "dz-todo-svc-{tag}-{}.sock",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    /// 读一行请求后回 `reply`;`None` = 不应答直接挂断。
    fn serve_once(tag: &str, reply: Option<Reply>) -> PathBuf {
        let sock = temp_sock(tag);
        let listener = tokio::net::UnixListener::bind(&sock).unwrap();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (r, mut w) = stream.into_split();
            let mut lines = BufReader::new(r).lines();
            let _ = lines.next_line().await;
            if let Some(reply) = reply {
                let _ = w.write_all(encode_line(&reply).as_bytes()).await;
            }
        });
        sock
    }

    #[tokio::test]
    async fn list_todos_comes_back_as_ok() {
        let sock = serve_once("list", Some(Reply::Todos { todos: vec![] }));
        let got = TodoService::list_todos(&Client::new(sock.clone()), 7).await;
        assert_eq!(got, Ok(vec![]));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn a_write_returns_unit_on_success() {
        let sock = serve_once("write", Some(Reply::Ok));
        let got = TodoService::delete_category(&Client::new(sock.clone()), 3).await;
        assert_eq!(got, Ok(()));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn hang_up_is_unavailable_with_the_legacy_text() {
        let sock = serve_once("hangup", None);
        let e = TodoService::list_todos(&Client::new(sock.clone()), 7)
            .await
            .unwrap_err();
        assert_eq!((e.kind, e.message.as_str()), (ServiceErrorKind::Unavailable, "daemon 断开"));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn error_reply_is_rejected_with_the_legacy_prefix() {
        let sock = serve_once("reject", Some(Reply::Error { message: "boom".into() }));
        let e = TodoService::delete_category(&Client::new(sock.clone()), 3)
            .await
            .unwrap_err();
        assert_eq!((e.kind, e.message.as_str()), (ServiceErrorKind::Rejected, "daemon 错误: boom"));
        let _ = std::fs::remove_file(&sock);
    }
}
```

Run: `cargo test -p dozer-client todo_service_tests 2>&1 | grep -E '^error' | head -3` → Expected: 编译失败(`the trait bound Client: TodoService is not satisfied`)(RED)。

- [ ] **Step 2: 实现**

在 `impl UsageService for Client` 之后追加。顶部 `use dozer_services::{...}` 加上 `TodoService`;`dozer_core::protocol` 的 import 里若已有 `CategoryInfo, CategoryMoveDirection, TodoInfo` 则复用,`TodoStoredStatus` 用 `dozer_core::protocol::TodoStoredStatus`(与 `set_todo_status` 现签名一致)。**每个方法都调同名 inherent 方法**(用 `Client::method(self, ..)` 路径写法,inherent 优先于 trait,避免递归):

```rust
impl TodoService for Client {
    fn list_todos(&self, project_id: i64) -> BoxFuture<'_, Result<Vec<TodoInfo>, ServiceError>> {
        Box::pin(async move {
            Client::list_todos(self, project_id)
                .await
                .map_err(into_service_error)
        })
    }
    fn list_categories(
        &self,
        project_id: i64,
    ) -> BoxFuture<'_, Result<Vec<CategoryInfo>, ServiceError>> {
        Box::pin(async move {
            Client::list_categories(self, project_id)
                .await
                .map_err(into_service_error)
        })
    }
    fn add_todo(&self, project_id: i64, text: String) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            Client::add_todo(self, project_id, &text)
                .await
                .map(|_| ())
                .map_err(into_service_error)
        })
    }
    fn toggle_todo(&self, id: i64, done: bool) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            Client::toggle_todo(self, id, done)
                .await
                .map(|_| ())
                .map_err(into_service_error)
        })
    }
    fn edit_todo_text(&self, id: i64, text: String) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            Client::edit_todo_text(self, id, &text)
                .await
                .map(|_| ())
                .map_err(into_service_error)
        })
    }
    fn reorder_todo(
        &self,
        id: i64,
        after_id: Option<i64>,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            Client::reorder_todo(self, id, after_id)
                .await
                .map(|_| ())
                .map_err(into_service_error)
        })
    }
    fn set_todo_plan_date(
        &self,
        id: i64,
        plan_date: Option<String>,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            Client::set_todo_plan_date(self, id, plan_date.as_deref())
                .await
                .map(|_| ())
                .map_err(into_service_error)
        })
    }
    fn set_todo_status(
        &self,
        id: i64,
        status: dozer_core::protocol::TodoStoredStatus,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            Client::set_todo_status(self, id, status)
                .await
                .map(|_| ())
                .map_err(into_service_error)
        })
    }
    fn set_todo_category(
        &self,
        id: i64,
        category_id: Option<i64>,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            Client::set_todo_category(self, id, category_id)
                .await
                .map(|_| ())
                .map_err(into_service_error)
        })
    }
    fn add_category(
        &self,
        project_id: i64,
        parent_id: Option<i64>,
        name: String,
    ) -> BoxFuture<'_, Result<CategoryInfo, ServiceError>> {
        Box::pin(async move {
            Client::add_category(self, project_id, parent_id, &name)
                .await
                .map_err(into_service_error)
        })
    }
    fn rename_category(&self, id: i64, name: String) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            Client::rename_category(self, id, &name)
                .await
                .map(|_| ())
                .map_err(into_service_error)
        })
    }
    fn delete_category(&self, id: i64) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            Client::delete_category(self, id)
                .await
                .map_err(into_service_error)
        })
    }
    fn reparent_category(
        &self,
        id: i64,
        new_parent_id: Option<i64>,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            Client::reparent_category(self, id, new_parent_id)
                .await
                .map(|_| ())
                .map_err(into_service_error)
        })
    }
    fn move_category_sibling(
        &self,
        id: i64,
        direction: CategoryMoveDirection,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            Client::move_category_sibling(self, id, direction)
                .await
                .map(|_| ())
                .map_err(into_service_error)
        })
    }
}
```

若 `dozer_services::TodoService` 的 `set_todo_status` 参数类型 `dozer_model::TodoStoredStatus` 与 `dozer_core::protocol::TodoStoredStatus` 不是同一类型(O7 后 `protocol` 是 `pub use` 再导出,应当是同一类型),编译器会报;按报错把 impl 里的路径改成 `dozer_model::TodoStoredStatus`。

Run: `cargo test -p dozer-client 2>&1 | grep -E 'test result|FAILED|^error'` → Expected: 原有测试 + 4 个新测试全绿。

- [ ] **Step 3: 提交**

```bash
cargo fmt -p dozer-client
perl -ni -e 'print unless /^source = "git\+https:\/\/github.com\/byteboyai\/(bytegit|byteui)\?/' Cargo.lock
git add crates/dozer-client Cargo.lock && git status --short
git commit -m "feat(client): Todo S2 Client 实现 TodoService

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 3: `Services.todo` + 默认 Client 适配 + `FakeTodo`

**Files:**
- Modify: `crates/dozer-services/src/lib.rs`(`Services` 加字段,测试里的构造)
- Modify: `crates/dozer-app/src/panel_host.rs`(`services_for`、`PanelIo::new` 默认、测试)
- Modify: `crates/dozer-app/src/usage_probe/fake_usage.rs`(`into_services` 补 `todo`)
- Create: `crates/dozer-app/src/service_fakes.rs`(`#[cfg(test)]` 的 `FakeTodo`);Modify: `crates/dozer-app/src/main.rs`(或 crate 根声明处)加 `#[cfg(test)] mod service_fakes;`

**Interfaces:**
- Consumes: Task 1/2。
- Produces:
  - `dozer_services::Services { pub usage: Arc<dyn UsageService>, pub todo: Arc<dyn TodoService> }`
  - `crate::panel_host::services_for(client: &Client) -> Services`(`pub(crate)`):两个字段都由 `client.clone()` 适配。
  - `crate::service_fakes::FakeTodo`(`Clone`,克隆共享同一份状态):`FakeTodo::ok(todos: Vec<TodoInfo>, categories: Vec<CategoryInfo>)`、`FakeTodo::failing_reads(kind: ServiceErrorKind, message: &str)`、`FakeTodo::failing_writes(kind: ServiceErrorKind, message: &str)`、`FakeTodo::calls(&self) -> Vec<String>`、`FakeTodo::into_services(self) -> Services`(usage 用 `FakeUsage::ok(vec![])`);实现 `TodoService`;每次调用记录一条形如 `"reorder_todo(1,None)"` 的字符串(见下文代码)。`FakeTodo::default()` = 读写都成功、空列表。

- [ ] **Step 1: 写失败测试**

(a) `panel_host.rs` 测试模块(`with_services_replaces_the_default_client_adapter` 附近)追加:
```rust
    #[test]
    fn default_services_route_todo_through_the_client_adapter() {
        let rt = test_runtime();
        let (io, _rx) = recording_io(&rt);
        // recording_io 的 Client 指向不存在的 socket → Unavailable
        let e = rt
            .block_on(io.services().todo.list_todos(1))
            .unwrap_err();
        assert_eq!(e.kind, dozer_services::ServiceErrorKind::Unavailable);
    }
```
(b) `dozer-services` 里 `services_is_cloneable_and_shares_the_service` 测试改为同时构造 `todo` 字段(用 Task 1 的 `MemTodo`):`Services { usage: Arc::new(Mem(Ok(vec![]))), todo: Arc::new(MemTodo(Ok(vec![]))) }`,并追加断言 `Arc::ptr_eq(&s.todo, &t.todo)`。

Run: `cargo test -p dozer-services 2>&1 | grep -E '^error' | head -3` 与 `cargo test -p dozer-app default_services_route_todo 2>&1 | grep -E '^error' | head -3` → Expected: 编译失败(`Services` 没有字段 `todo`)(RED)。

- [ ] **Step 2: 实现**

1. `dozer-services/src/lib.rs`:`Services` 加 `pub todo: Arc<dyn TodoService>,`。
2. `panel_host.rs`:
```rust
/// 默认的服务束:各能力都由 `Client` 适配。host 的其它入口(`App` 的消息处理/轮询)也用它,
/// 保证"面板拿到的服务"只有一处构造。
pub(crate) fn services_for(client: &Client) -> Services {
    Services {
        usage: Arc::new(client.clone()),
        todo: Arc::new(client.clone()),
    }
}
```
并把 `PanelIo::new` 里的 `services: Services { usage: Arc::new(client.clone()) }` 改为 `services: services_for(&client)`。
3. `usage_probe/fake_usage.rs` 的 `into_services`:
```rust
    pub(crate) fn into_services(self) -> Services {
        Services {
            usage: Arc::new(self),
            todo: Arc::new(crate::service_fakes::FakeTodo::default()),
        }
    }
```
4. 新建 `crates/dozer-app/src/service_fakes.rs`:
```rust
//! 进程内服务假实现(只在测试里):不起 UDS,返回固定结果并记录调用。
use dozer_model::{CategoryInfo, CategoryMoveDirection, TodoInfo, TodoStoredStatus};
use dozer_services::{BoxFuture, ServiceError, ServiceErrorKind, Services, TodoService};
use std::sync::{Arc, Mutex};

struct State {
    todos: Result<Vec<TodoInfo>, ServiceError>,
    categories: Result<Vec<CategoryInfo>, ServiceError>,
    write_error: Option<ServiceError>,
    calls: Vec<String>,
}

/// `Clone` 的克隆共享同一份状态(测试里一份给面板、一份用来看调用记录)。
#[derive(Clone)]
pub(crate) struct FakeTodo {
    state: Arc<Mutex<State>>,
}

impl Default for FakeTodo {
    fn default() -> Self {
        Self::ok(vec![], vec![])
    }
}

impl FakeTodo {
    pub(crate) fn ok(todos: Vec<TodoInfo>, categories: Vec<CategoryInfo>) -> Self {
        Self::with(Ok(todos), Ok(categories), None)
    }
    pub(crate) fn failing_reads(kind: ServiceErrorKind, message: &str) -> Self {
        let e = ServiceError::new(kind, message);
        Self::with(Err(e.clone()), Err(e), None)
    }
    pub(crate) fn failing_writes(kind: ServiceErrorKind, message: &str) -> Self {
        Self::with(Ok(vec![]), Ok(vec![]), Some(ServiceError::new(kind, message)))
    }
    fn with(
        todos: Result<Vec<TodoInfo>, ServiceError>,
        categories: Result<Vec<CategoryInfo>, ServiceError>,
        write_error: Option<ServiceError>,
    ) -> Self {
        Self {
            state: Arc::new(Mutex::new(State { todos, categories, write_error, calls: vec![] })),
        }
    }
    pub(crate) fn calls(&self) -> Vec<String> {
        self.state.lock().unwrap().calls.clone()
    }
    pub(crate) fn into_services(self) -> Services {
        Services {
            usage: Arc::new(crate::usage_probe::fake_usage::FakeUsage::ok(vec![])),
            todo: Arc::new(self),
        }
    }
    fn write(&self, call: String) -> BoxFuture<'_, Result<(), ServiceError>> {
        let mut s = self.state.lock().unwrap();
        s.calls.push(call);
        let r = match &s.write_error {
            Some(e) => Err(e.clone()),
            None => Ok(()),
        };
        Box::pin(async move { r })
    }
}

impl TodoService for FakeTodo {
    fn list_todos(&self, project_id: i64) -> BoxFuture<'_, Result<Vec<TodoInfo>, ServiceError>> {
        let mut s = self.state.lock().unwrap();
        s.calls.push(format!("list_todos({project_id})"));
        let r = s.todos.clone();
        Box::pin(async move { r })
    }
    fn list_categories(&self, project_id: i64) -> BoxFuture<'_, Result<Vec<CategoryInfo>, ServiceError>> {
        let mut s = self.state.lock().unwrap();
        s.calls.push(format!("list_categories({project_id})"));
        let r = s.categories.clone();
        Box::pin(async move { r })
    }
    fn add_todo(&self, project_id: i64, text: String) -> BoxFuture<'_, Result<(), ServiceError>> {
        self.write(format!("add_todo({project_id},{text})"))
    }
    fn toggle_todo(&self, id: i64, done: bool) -> BoxFuture<'_, Result<(), ServiceError>> {
        self.write(format!("toggle_todo({id},{done})"))
    }
    fn edit_todo_text(&self, id: i64, text: String) -> BoxFuture<'_, Result<(), ServiceError>> {
        self.write(format!("edit_todo_text({id},{text})"))
    }
    fn reorder_todo(&self, id: i64, after_id: Option<i64>) -> BoxFuture<'_, Result<(), ServiceError>> {
        self.write(format!("reorder_todo({id},{after_id:?})"))
    }
    fn set_todo_plan_date(&self, id: i64, plan_date: Option<String>) -> BoxFuture<'_, Result<(), ServiceError>> {
        self.write(format!("set_todo_plan_date({id},{plan_date:?})"))
    }
    fn set_todo_status(&self, id: i64, status: TodoStoredStatus) -> BoxFuture<'_, Result<(), ServiceError>> {
        self.write(format!("set_todo_status({id},{status:?})"))
    }
    fn set_todo_category(&self, id: i64, category_id: Option<i64>) -> BoxFuture<'_, Result<(), ServiceError>> {
        self.write(format!("set_todo_category({id},{category_id:?})"))
    }
    /// 写成功时返回一个固定 id=900 的分类,供"新建后进入重命名"的测试使用;失败时返回预设错误。
    fn add_category(
        &self,
        project_id: i64,
        parent_id: Option<i64>,
        name: String,
    ) -> BoxFuture<'_, Result<CategoryInfo, ServiceError>> {
        let mut s = self.state.lock().unwrap();
        s.calls.push(format!("add_category({project_id},{parent_id:?},{name})"));
        let r = match &s.write_error {
            Some(e) => Err(e.clone()),
            None => Ok(CategoryInfo {
                id: 900,
                project_id,
                parent_id,
                name,
                rank: 0,
                created_ms: 0,
                auto_poll_enabled: false,
            }),
        };
        Box::pin(async move { r })
    }
    fn rename_category(&self, id: i64, name: String) -> BoxFuture<'_, Result<(), ServiceError>> {
        self.write(format!("rename_category({id},{name})"))
    }
    fn delete_category(&self, id: i64) -> BoxFuture<'_, Result<(), ServiceError>> {
        self.write(format!("delete_category({id})"))
    }
    fn reparent_category(&self, id: i64, new_parent_id: Option<i64>) -> BoxFuture<'_, Result<(), ServiceError>> {
        self.write(format!("reparent_category({id},{new_parent_id:?})"))
    }
    fn move_category_sibling(&self, id: i64, direction: CategoryMoveDirection) -> BoxFuture<'_, Result<(), ServiceError>> {
        self.write(format!("move_category_sibling({id},{direction:?})"))
    }
}
```
`dozer-app` crate 根 `crates/dozer-app/src/main.rs` 第 32 行附近已有
```rust
#[cfg(test)]
mod usage_probe;
```
同位置按同样写法加 `#[cfg(test)] mod service_fakes;`。

Run: `cargo test -p dozer-services 2>&1 | grep -E 'test result|^error'`;`cargo test -p dozer-app default_services_route_todo 2>&1 | grep -E 'test result|^error'` → Expected: 通过。

- [ ] **Step 3: 全 workspace 编译与提交**

```bash
cargo build --workspace 2>&1 | grep -E '^error|FakeTodo|never used'   # 期望无 error;若 FakeTodo 的构造函数出现 dead_code,Task 4 才会用到——加 #[allow(dead_code)] 并在 Task 4 去掉
cargo test -p dozer-app -- panel_host usage_probe 2>&1 | grep -E 'test result|FAILED'
cargo fmt -p dozer-app -p dozer-services
perl -ni -e 'print unless /^source = "git\+https:\/\/github.com\/byteboyai\/(bytegit|byteui)\?/' Cargo.lock
git add crates/dozer-services crates/dozer-app/src Cargo.lock && git status --short
git commit -m "feat(host): Todo S3 Services.todo + services_for + FakeTodo

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 4: Todo 面板切换到 `TodoService`,读失败可见

> 说明(Ruling,计划阶段):规格 §4 把读/写拆成两个提交,但 `update` 的签名是一处原子改动——`Mutated` 分支同时调读刷新与写,无法让读用 `Services`、写继续用 `Client` 而不引入过渡参数。因此本任务一次完成读与写的切换,以测试分组体现"先读后写",代价:单提交较大,靠 Step 1 的失败测试与 Step 4 的全量测试兜底。

**Files:**
- Modify: `crates/dozer-app/src/extensions/todo/state.rs`(`Message::LoadFailed`、`request_*_refresh`、`on_activate`、去掉 `use dozer_client::Client`)
- Modify: `crates/dozer-app/src/extensions/todo/update.rs`(签名、写操作、`LoadFailed` 臂、去掉 `use dozer_client::Client`)
- Modify: `crates/dozer-app/src/extensions/todo/mod.rs`(测试)
- Modify: `crates/dozer-app/src/app/update.rs`(`todo::update(..)` 调用点,约 4812 行)与 `crates/dozer-app/src/app/app.rs`(`poll_todo_if_visible`,约 1664–1672 行)
- Modify: `crates/dozer-app/src/usage_probe/host.rs`(行号引用若漂移,只改行号)

**Interfaces:**
- Consumes: Task 3 的 `Services`、`services_for`、`FakeTodo`;`TodoService`;`ServiceError`/`ServiceErrorKind`。
- Produces:
  - `todo::Message::LoadFailed(dozer_services::ServiceErrorKind, String)`
  - `todo::request_todos_refresh(project_id: i64, service: &Arc<dyn TodoService>, handle: &Handle, emit: impl Fn(Message) + Send + 'static)`;`request_categories_refresh` 同形
  - `todo::update(ws_state, msg, project_id: i64, service: &Arc<dyn TodoService>, handle: &Handle, emit: impl Fn(Message) + Send + Sync + 'static)`
  - `on_activate` 用 `ctx.io.services().todo`

- [ ] **Step 1: 写失败测试(面板 update 层 + 读刷新 + 写文本)**

在 `extensions/todo/mod.rs` 的 `mod tests` 末尾追加(`todo_info(id,text,done)` 助手已在该模块;`runtime`/`TIMEOUT` 已被 H6 测试 import)。先加 import:`use crate::service_fakes::FakeTodo; use dozer_services::ServiceErrorKind; use std::sync::{Mutex, mpsc};`

```rust
    fn svc(fake: &FakeTodo) -> std::sync::Arc<dyn dozer_services::TodoService> {
        fake.clone().into_services().todo
    }

    fn collect() -> (impl Fn(Message) + Send + Sync + 'static, mpsc::Receiver<Message>) {
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        (move |m| { let _ = tx.lock().unwrap().send(m); }, rx)
    }

    #[test]
    fn unavailable_read_failure_keeps_both_lists_and_stays_silent() {
        let rt = runtime();
        let service = svc(&FakeTodo::default());
        let mut ws = WorkspaceState::default();
        let (e1, _r1) = collect();
        update(&mut ws, Message::Loaded(vec![todo_info(1, "a", false)]), 7, &service, rt.handle(), e1);
        let (e2, _r2) = collect();
        update(&mut ws, Message::LoadFailed(ServiceErrorKind::Unavailable, "daemon 断开".into()), 7, &service, rt.handle(), e2);
        assert_eq!(ws.items().len(), 1, "读失败不得清空已有任务");
        assert!(ws.take_outbox().is_empty(), "Unavailable 由顶栏徽标表达,不弹 Toast");
    }

    #[test]
    fn rejected_read_failure_keeps_lists_and_toasts_once_with_a_key() {
        let rt = runtime();
        let service = svc(&FakeTodo::default());
        let mut ws = WorkspaceState::default();
        let (e1, _r1) = collect();
        update(&mut ws, Message::Loaded(vec![todo_info(1, "a", false)]), 7, &service, rt.handle(), e1);
        let (e2, _r2) = collect();
        update(&mut ws, Message::LoadFailed(ServiceErrorKind::Rejected, "daemon 错误: boom".into()), 7, &service, rt.handle(), e2);
        assert_eq!(ws.items().len(), 1);
        let toasts = ws.take_outbox();
        assert_eq!(toasts.len(), 1);
        assert_eq!(toasts[0].level, crate::extensions::toast::Level::Error);
        assert!(toasts[0].text.contains("daemon 错误: boom"));
        assert_eq!(toasts[0].key.as_deref(), Some("todo-load"));
    }

    #[test]
    fn failing_reads_emit_load_failed_instead_of_an_empty_loaded() {
        let rt = runtime();
        let service = svc(&FakeTodo::failing_reads(ServiceErrorKind::Unavailable, "x"));
        let (emit, rx) = collect();
        let emit = std::sync::Arc::new(emit);
        let e1 = emit.clone();
        request_todos_refresh(7, &service, rt.handle(), move |m| e1(m));
        request_categories_refresh(7, &service, rt.handle(), move |m| emit(m));
        for _ in 0..2 {
            assert!(
                matches!(rx.recv_timeout(TIMEOUT).unwrap(), Message::LoadFailed(ServiceErrorKind::Unavailable, _)),
                "读失败必须发 LoadFailed,不得发 Loaded(vec![])"
            );
        }
    }

    #[test]
    fn write_failure_text_is_the_service_message_unchanged() {
        let rt = runtime();
        let fake = FakeTodo::failing_writes(ServiceErrorKind::Rejected, "daemon 错误: boom");
        let mut ws = WorkspaceState::default();
        let (emit, rx) = collect();
        update(&mut ws, Message::ReorderTo { id: 1, after_id: None }, 7, &svc(&fake), rt.handle(), emit);
        match rx.recv_timeout(TIMEOUT).unwrap() {
            Message::Mutated(Err(text)) => assert_eq!(text, "daemon 错误: boom"),
            other => panic!("意外消息: {other:?}"),
        }
        assert_eq!(fake.calls(), vec!["reorder_todo(1,None)".to_string()]);
    }

    #[test]
    fn new_category_enters_rename_with_the_id_the_service_returned() {
        let rt = runtime();
        let fake = FakeTodo::default();
        let mut ws = WorkspaceState::default();
        let (emit, rx) = collect();
        update(&mut ws, Message::CategoryNewSibling(None), 7, &svc(&fake), rt.handle(), emit);
        let first = rx.recv_timeout(TIMEOUT).unwrap();
        let second = rx.recv_timeout(TIMEOUT).unwrap();
        assert!(matches!(first, Message::CategoryRenameStart(900)), "{first:?}");
        assert!(matches!(second, Message::CategoryMutated(Ok(()))), "{second:?}");
        assert_eq!(fake.calls(), vec!["add_category(7,None,新分类)".to_string()]);
    }

    #[test]
    fn a_failed_category_mutation_refreshes_both_lists_from_the_service() {
        let rt = runtime();
        let fake = FakeTodo::failing_writes(ServiceErrorKind::Rejected, "daemon 错误: nope");
        let mut ws = WorkspaceState::default();
        let (emit, rx) = collect();
        update(&mut ws, Message::CategoryMutated(Err("daemon 错误: nope".into())), 7, &svc(&fake), rt.handle(), emit);
        let a = rx.recv_timeout(TIMEOUT).unwrap();
        let b = rx.recv_timeout(TIMEOUT).unwrap();
        let kinds = |m: &Message| matches!(m, Message::Loaded(_) | Message::CategoriesLoaded(_));
        assert!(kinds(&a) && kinds(&b), "{a:?} {b:?}");
        assert_eq!(ws.take_outbox().len(), 1, "分类操作失败的 Toast 保持不变");
    }
```
并把现有 `on_activate_requests_both_the_todo_list_and_the_category_tree` 里的 `offline_ctx` 之后加一行 `let ctx = crate::panel_host::ActivationCtx { io: ctx.io.with_services(FakeTodo::default().into_services()), ..ctx };`(否则指向不存在的 socket 会得到 `LoadFailed` 而不是 `Loaded`)。

Run: `cargo test -p dozer-app extensions::todo 2>&1 | grep -E '^error' | head -5` → Expected: 编译失败(`LoadFailed` 不存在、`update` 参数类型不符)(RED)。

- [ ] **Step 2: 面板侧实现**

`todo/state.rs`:
1. 删除 `use dozer_client::Client;`;加 `use dozer_services::{ServiceErrorKind, TodoService}; use std::sync::Arc;`(`Arc` 若已导入则复用)。
2. `Message` 在 `CategoriesLoaded(Vec<CategoryInfo>),` 之后加(带文档注释):
```rust
    /// 读失败(任务列表或分类树):**不改任何列表**。`Unavailable` 静默(顶栏徽标已表达
    /// "dozerd 不可用");其余弹带 key 的 Error Toast。轮询时反复失败靠 key 去重。
    LoadFailed(ServiceErrorKind, String),
```
3. 两个刷新函数改为:
```rust
pub fn request_todos_refresh(
    project_id: i64,
    service: &Arc<dyn TodoService>,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    let service = service.clone();
    handle.spawn(async move {
        match service.list_todos(project_id).await {
            Ok(todos) => emit(Message::Loaded(todos)),
            Err(e) => emit(Message::LoadFailed(e.kind, e.message)),
        }
    });
}

pub fn request_categories_refresh(
    project_id: i64,
    service: &Arc<dyn TodoService>,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    let service = service.clone();
    handle.spawn(async move {
        match service.list_categories(project_id).await {
            Ok(categories) => emit(Message::CategoriesLoaded(categories)),
            Err(e) => emit(Message::LoadFailed(e.kind, e.message)),
        }
    });
}
```
4. `on_activate`:`ctx.io.client()` 两处改为 `&ctx.io.services().todo`。

`todo/update.rs`:
1. 删除 `use dozer_client::Client;`;加 `use dozer_services::{ServiceErrorKind, TodoService}; use std::sync::Arc;`。
2. `submit_new_todo` 与 `update` 的参数 `client: &Client` 改为 `service: &Arc<dyn TodoService>`;每处 `let client = client.clone();` → `let service = service.clone();`。
3. 写操作的统一替换规则(逐处改,共 12 个服务方法的调用点):
   - `client.add_todo(project_id, &text).await.map(|_| ()).map_err(|e| e.to_string())` → `service.add_todo(project_id, text).await.map_err(|e| e.message)`(`text` 已是 `String`,移入 future;`submit_new_todo` 里 `text.clone()` 已给乐观项用,不用再 clone)
   - `toggle_todo(id, target)`、`reorder_todo(id, after_id)`、`set_todo_category(id, category_id)`、`delete_category(id)`、`reparent_category(id, new_parent)`、`move_category_sibling(id, direction)`、`set_todo_status(id, stored_target)`:去掉 `.map(|_| ())`(已返回 `()`),`map_err(|e| e.to_string())` → `map_err(|e| e.message)`。
   - `edit_todo_text(id, &text)` → `edit_todo_text(id, text)`;`rename_category(id, &new_name)` → `rename_category(id, new_name)`;`set_todo_plan_date(id, Some(&day))` → `set_todo_plan_date(id, Some(day))`。
   - `add_category(project_id_owned, Some(parent_id), "新分类")` → `service.add_category(project_id_owned, Some(parent_id), "新分类".to_string())`;`Err(e) => emit(Message::CategoryMutated(Err(e.to_string())))` → `Err(e) => emit(Message::CategoryMutated(Err(e.message)))`(`CategoryNewSibling` 同理)。
4. `Message::Mutated` 与 `CategoryMutated` 分支里的刷新调用:`request_todos_refresh(project_id, client, handle, emit)` → `(project_id, service, handle, emit)`;`CategoryMutated` 分支里 `client1/client2` 改为 `service1/service2`,第二个任务改为:
```rust
            let service2 = service.clone();
            ...
            request_categories_refresh(project_id, &service1, &handle1, move |m| emit1(m));
            handle2.spawn(async move {
                match service2.list_todos(project_id).await {
                    Ok(todos) => emit2(Message::Loaded(todos)),
                    Err(e) => emit2(Message::LoadFailed(e.kind, e.message)),
                }
            });
```
5. `update` 的 match 里加(放在 `Message::CategoriesLoaded` 之后):
```rust
        Message::LoadFailed(kind, message) => {
            if kind == ServiceErrorKind::Unavailable {
                dozer_core::log_debug!(LOG, "任务/分类读取失败(dozerd 不可用): {message}");
            } else {
                ws_state.outbox.push_keyed(
                    LOG,
                    crate::extensions::toast::Level::Error,
                    format!("任务加载失败: {message}"),
                    "todo-load",
                );
            }
        }
```
(`LOG` 已在 `todo/update.rs` 第 8 行声明为来源 `todo`;`dozer_core::log_debug!` 的形参形式以 `crates/dozer-core/src/log.rs:283` 的宏定义为准。)

`app/update.rs`(约 4802–4813):
```rust
        let services = crate::panel_host::services_for(&self.client);
        let service = services.todo;
        ...
        self.with_focused_project(move |ws, _io| {
            todo::update(&mut ws.todo, msg, project_id, &service, &handle, emit);
        });
```
(原来的 `let client = self.client.clone();` 删除,若该变量在后面没有其它用途;`handle`/`proxy` 保持。)

`app/app.rs`(约 1664–1672):`let client = self.client.clone();` → `let service = crate::panel_host::services_for(&self.client).todo;`,两个 `todo::request_*_refresh(project_id, &client, ..)` 改为 `&service`。

- [ ] **Step 3: 运行面板测试**

```bash
cargo test -p dozer-app extensions::todo 2>&1 | grep -E 'test result|FAILED|^error'
```
Expected: 全绿(含 6 个新测试与改过的 H6 测试)。若 `LoadFailed` 在 `app/update.rs` 的 `Message::Todo(msg) => match msg {…}` 里被某个 `_ =>` 之外的臂错误拦截,按编译器/测试提示处理;**不得**新增面板专属臂。

- [ ] **Step 4: 探针守卫、全量、门禁、提交**

```bash
cargo test -p dozer-app usage_probe::host 2>&1 | grep -E 'test result|FAILED|实际是'   # 若 host_site 行号因 update.rs 行数变化漂移,只改 host_site() 与相关注释里的行号,不改守卫逻辑
W=$(ls -d .superpowers/sdd/*/ | head -1)
cargo test --workspace --no-fail-fast > ${W}t4.txt 2>&1; grep -E '^test result' ${W}t4.txt | awk '{s+=$4;f+=$6} END{print s,f}'; grep FAILED ${W}t4.txt | grep '^test '
cargo clippy --all-targets 2>&1 | grep -c '^warning'
python3 scripts/audit/check_panel_boundary.py        # 只许剩 R-HOST-EXECUTORS 0 -> 1;R-HOST-PANEL-ARMS 不得上升
bash scripts/check-log-scope.sh | tail -1
cargo fmt -p dozer-app
```
Expected: 失败仅两个已知(`files::…delete_confirm…`、间歇的 `git_log::build_*`);clippy 警告数 ≤ 基线;门禁无新增回退;log scope ok。
```bash
git checkout Cargo.lock 2>/dev/null; git add crates/dozer-app/src && git status --short
git commit -m "feat(todo): Todo S4 面板改用 TodoService,读失败保留列表并分档提示

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 5: `R-CLIENT` 基线下降、验收与文档回填

**Files:**
- Modify: `scripts/audit/panel-boundary.baseline.json`(仅 `todo/*` 的 `R-CLIENT` 条目)
- Modify: `docs/dozer-v2/bytehost-H0/08-client-use-inventory.md`、`CLAUDE.md`

**Interfaces:**
- Produces: `extensions/todo/state.rs`、`extensions/todo/update.rs` 不再出现在 `R-CLIENT` 基线里。

- [ ] **Step 1: 确认门禁现状并下调基线**

```bash
python3 scripts/audit/check_panel_boundary.py   # 此时 todo/* 的 R-CLIENT 实测应为 0(低于基线),门禁只显示既有 R-HOST-EXECUTORS
grep -rn 'dozer_client\|io\.client()' crates/dozer-app/src/extensions/todo | grep -v '^.*://' | head   # 期望无输出(注释里的提及可忽略,但代码里必须为 0)
python3 - <<'EOF'
import json
p = "scripts/audit/panel-boundary.baseline.json"
base = json.load(open(p, encoding="utf-8"))
for rel in ("extensions/todo/state.rs", "extensions/todo/update.rs"):
    if rel in base:
        base[rel].pop("R-CLIENT", None)
        if not base[rel]:
            del base[rel]
json.dump(base, open(p, "w", encoding="utf-8"), ensure_ascii=False, indent=1, sort_keys=True)
open(p, "a").write("\n")
EOF
git diff --stat scripts/audit/panel-boundary.baseline.json
python3 scripts/audit/check_panel_boundary.py; echo exit=$?
```
Expected: 基线 diff 只含 `extensions/todo/*` 条目的删除;门禁仍只报 `R-HOST-EXECUTORS 0 -> 1`。

- [ ] **Step 2: 验收清单(逐条跑、读输出)**

```bash
cargo tree -p dozer-services --edges normal                    # 1: 仅 dozer-model + serde 系
python3 scripts/audit/unit_edges.py ext:todo                   # 2: 无 dozer_client;S 类不含 Client
cargo test -p dozer-app extensions::todo 2>&1 | grep -E 'test result|FAILED'   # 3/4: 读失败与写文本测试通过
cargo test -p dozer-core --test model_golden 2>&1 | grep 'test result'          # 5
cargo test --workspace --no-fail-fast > ${W}final.txt 2>&1; grep -E '^test result' ${W}final.txt | awk '{s+=$4;f+=$6} END{print s,f}'; grep FAILED ${W}final.txt | grep '^test '   # 6
cargo clippy --all-targets 2>&1 | grep -c '^warning'; cargo fmt --check && echo fmt-ok
bash scripts/check-log-scope.sh && echo log-scope-ok
git status --short                                             # 无 Info.plist、无无关 Cargo 改动
```
任一不满足即不收尾,回到对应任务修。

- [ ] **Step 3: 文档回填并提交**

- `docs/dozer-v2/bytehost-H0/08-client-use-inventory.md` 末尾追加一节"Todo 迁移(2026-10-10)":`extensions/todo/**` 经 `TodoService` 取数;`R-CLIENT` 基线中 `todo/*` 已清零;**登记后续项**:`todo::Message::Loaded`/`CategoriesLoaded` 没有 project_id 与代数保护(同 Usage 修过的 G3b),慢的旧结果可能覆盖新结果,本次未修。
- `CLAUDE.md` Workspace 布局表 `dozer-services` 行里"Usage 已迁,其余面板待迁"改为"Usage、Todo 已迁,其余面板待迁"。
```bash
git add scripts/audit docs CLAUDE.md
git commit -m "chore(audit): Todo S5 R-CLIENT 基线下调 + 文档回填

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

之后按 executing-plans 的 Final Review(整分支 fresh reviewer)与 finishing-a-development-branch;**合并进 main 属于需先询问用户的外部副作用**,合并前核对 main 上用户 WIP(`Cargo.lock`、`crates/dozer-app/Cargo.toml`、`Info.plist` 及 `extensions/files/*` 等)与本分支的重叠,用 `git stash` → 合并 → `git stash pop`。
