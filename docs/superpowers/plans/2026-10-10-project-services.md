# Project 面板迁移到服务 trait Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Project 面板(`extensions/project.rs`、`extensions/project/**`)的 10 个服务调用改为经 `dozer-services` 的 `MemoryService`/`ProjectService` 取数与写入;`list_memories` 读失败不再被吞成空列表。

**Architecture:** `dozer-services` 新增两个 trait 与 `Services.memory`/`Services.project`;`Client` 实现它们(复用 `into_service_error`);面板的 `update` 改吃 `&Services`,各 `spawn_*` 改吃 `Arc<dyn ProjectService>`/`Arc<dyn MemoryService>`;读失败走新的 `Message::MemoriesLoadFailed(ServiceErrorKind, String)`:保留旧列表,`Unavailable` 静默,其余弹带 key 的 Error Toast(同文本 60 秒内只弹一次)。

**Tech Stack:** Rust 2024 workspace、tokio、iced 0.14(只涉及面板 update 层)、Python 门禁脚本。

**Spec:** `docs/superpowers/specs/2026-10-10-project-services-design.md`(先例:`2026-10-10-todo-services-design.md`、`2026-10-09-dozer-services-x0-design.md`)

## Global Constraints

- `dozer-services` 只依赖 `dozer-model`;不加 `dozer-core`/`dozer-client`/`tokio` 到 `[dependencies]`。
- 线上协议零变化:`Request`/`Reply` 不动,`crates/dozer-core/tests/model_golden.rs` 不改且通过。
- 写/维护操作失败文本逐字不变(原 `e.to_string()` 等于 `ServiceError.message`;`roundtrip` 已把 `"daemon 断开"`、`"daemon 错误: …"` 装进 `ServiceError`)。
- 面板日志来源名必须是 `project`(`project/update.rs` 已声明 `LOG`);不新增裸 `tracing::`/`eprintln!`(`scripts/check-log-scope.sh`)。
- 瞬时失败走 Toast(`outbox`);`ws_state.error`(记忆写失败的面板内字段)与 `MemoryDetailLoadFailed` 保持不变。
- `R-HOST-PANEL-ARMS` 不上升:`MemoriesLoadFailed` 不新增 `app/update.rs` 的面板专属臂(走已有 `Message::Project(msg)` 通用路径)。
- 不动 `extensions/project_create.rs`(`open_project`/`list_projects`)与其它 host 侧调用。
- 在独立 worktree 实施;不提交用户 WIP(`Info.plist`、版本号相关改动);`Cargo.lock` 提交前剔除 bytegit/byteui 的 `source =` 行:`perl -ni -e 'print unless /^source = "git\+https:\/\/github.com\/byteboyai\/(bytegit|byteui)\?/' Cargo.lock`(若 `git status` 显示 `Cargo.lock` 只有这类噪声,直接 `git checkout Cargo.lock`)。
- main 上已知问题不修不算回归:`files::tests::delete_confirm_spec_reflects_pending_target` 失败、`git_log::tests::build_*` 间歇失败、门禁 `R-HOST-EXECUTORS 0 -> 1`。
- 提交信息末尾加 `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`。

## Review Focus

- `MemoriesLoadFailed` **不得清空**已有 `memories`(此前 `MemoriesLoaded(vec![])` 会整体替换);`Unavailable` 不弹 Toast,`Rejected`/`Protocol` 60 秒内同文本只弹一次 → Task 4 测试。
- `spawn_repair_run` 切换后行为逐条等价:补录条数文案(`Ok(0)`→AlreadyOk、`Ok(n)`→`导入 n 个历史文件`)、批次成功/批次不存在(`总结批次不存在，请重试`)/查询失败三条路径,以及轮询终止条件 → Task 4 测试。
- `spawn_scaffold_run` 补录失败只写 warn 日志、仍然 emit `ScaffoldDone` → Task 4 测试。
- `rename_project` 返回 `None` 时仍报"项目不存在";`spawn_delete_project` 在 `remove_project` 失败时不得继续调用 `delete_project_transcripts` 或动文件系统 → Task 4 测试。
- `impl MemoryService/ProjectService for Client` 内部必须调 inherent 方法而不是递归调 trait 方法;返回非 `()` 的值(如补录条数)要原样带回 → Task 2 适配测试。

---

### Task 1: `MemoryService` + `ProjectService`(`dozer-services`)

**Files:**
- Modify: `crates/dozer-services/src/lib.rs`

**Interfaces:**
- Consumes: `dozer_model::{MemoryInfo, MemoryDetail, ProjectInfo, SummaryBatchInfo, AgentKind}`(均已在 `dozer-model`)。
- Produces(全部返回 `BoxFuture<'_, Result<_, ServiceError>>`,参数按值传):
  - `pub trait MemoryService: Send + Sync`:
    - `list_memories(project_id: i64) -> Vec<MemoryInfo>`
    - `get_memory(project_id: i64, id: i64) -> MemoryDetail`
    - `write_memory(project_id: i64, title: String, kind: String, description: String, body: String, actor: String) -> MemoryDetail`
    - `delete_memory(project_id: i64, id: i64, actor: String) -> ()`
  - `pub trait ProjectService: Send + Sync`:
    - `rename_project(id: i64, name: String) -> Option<ProjectInfo>`
    - `remove_project(id: i64) -> ()`
    - `delete_project_transcripts(cwd: String) -> u32`
    - `backfill_project_transcripts(cwd: String) -> u32`
    - `submit_summary_batch(cwd: String, provider: Option<AgentKind>, model: Option<String>) -> (i64, u32)`
    - `get_summary_batch(batch_id: i64) -> Option<SummaryBatchInfo>`

- [ ] **Step 0: 记录基线(只测量,写进 ledger;先等基线跑完再改代码,避免并发编译污染)**

```bash
W=$(ls -d .superpowers/sdd/*/ | head -1)
grep -rc 'dozer_client\|io\.client()' crates/dozer-app/src/extensions/project.rs crates/dozer-app/src/extensions/project | grep -v ':0'
cargo test --workspace --no-fail-fast > ${W}baseline-tests.txt 2>&1; grep -E '^test result' ${W}baseline-tests.txt | awk '{s+=$4;f+=$6} END{print "passed",s,"failed",f}'; grep -E '^test .* FAILED' ${W}baseline-tests.txt
cargo clippy --all-targets 2>&1 | grep -c '^warning'
python3 scripts/audit/check_panel_boundary.py; echo exit=$?
```
Expected: 失败仅已知的;门禁仅 `R-HOST-EXECUTORS 0 -> 1`;把四个数字写进 ledger。**同步执行,跑完再进 Step 1。**

- [ ] **Step 1: 写失败测试**

在 `crates/dozer-services/src/lib.rs` 现有 `#[cfg(test)] mod tests` 末尾(最后一个测试之后、模块结尾 `}` 之前)追加。`MemMemory`/`MemProject` 实现全部方法,trait 不存在时编译失败(RED):

```rust
    // ---- MemoryService / ProjectService 契约 ----

    use dozer_model::{AgentKind, MemoryDetail, MemoryInfo, ProjectInfo, SummaryBatchInfo};

    struct MemMemory(Result<Vec<MemoryInfo>, ServiceError>);

    impl MemoryService for MemMemory {
        fn list_memories(&self, _p: i64) -> BoxFuture<'_, Result<Vec<MemoryInfo>, ServiceError>> {
            let r = self.0.clone();
            Box::pin(async move { r })
        }
        fn get_memory(&self, _p: i64, _i: i64) -> BoxFuture<'_, Result<MemoryDetail, ServiceError>> {
            Box::pin(async { Err(ServiceError::new(ServiceErrorKind::Protocol, "mem")) })
        }
        fn write_memory(
            &self, _p: i64, _t: String, _k: String, _d: String, _b: String, _a: String,
        ) -> BoxFuture<'_, Result<MemoryDetail, ServiceError>> {
            Box::pin(async { Err(ServiceError::new(ServiceErrorKind::Protocol, "mem")) })
        }
        fn delete_memory(&self, _p: i64, _i: i64, _a: String) -> BoxFuture<'_, Result<(), ServiceError>> {
            unit()
        }
    }

    struct MemProject;

    impl ProjectService for MemProject {
        fn rename_project(&self, _i: i64, _n: String) -> BoxFuture<'_, Result<Option<ProjectInfo>, ServiceError>> {
            Box::pin(async { Ok(None) })
        }
        fn remove_project(&self, _i: i64) -> BoxFuture<'_, Result<(), ServiceError>> { unit() }
        fn delete_project_transcripts(&self, _c: String) -> BoxFuture<'_, Result<u32, ServiceError>> {
            Box::pin(async { Ok(3) })
        }
        fn backfill_project_transcripts(&self, _c: String) -> BoxFuture<'_, Result<u32, ServiceError>> {
            Box::pin(async { Ok(5) })
        }
        fn submit_summary_batch(
            &self, _c: String, _p: Option<AgentKind>, _m: Option<String>,
        ) -> BoxFuture<'_, Result<(i64, u32), ServiceError>> {
            Box::pin(async { Ok((7, 2)) })
        }
        fn get_summary_batch(&self, _b: i64) -> BoxFuture<'_, Result<Option<SummaryBatchInfo>, ServiceError>> {
            Box::pin(async { Ok(None) })
        }
    }

    /// `handle.spawn` 要 `'static + Send`:持有 `Arc<dyn …>` 的 async 块必须满足。
    #[test]
    fn dyn_project_and_memory_futures_run_inside_tokio_spawn() {
        let memory: Arc<dyn MemoryService> = Arc::new(MemMemory(Ok(vec![])));
        let project: Arc<dyn ProjectService> = Arc::new(MemProject);
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let (list, backfilled, batch) = rt.block_on(async move {
            tokio::spawn(async move {
                let list = memory.list_memories(7).await;
                let backfilled = project.backfill_project_transcripts("/p".into()).await;
                let batch = project.submit_summary_batch("/p".into(), None, None).await;
                (list, backfilled, batch)
            })
            .await
            .unwrap()
        });
        assert_eq!(list, Ok(vec![]));
        assert_eq!(backfilled, Ok(5));
        assert_eq!(batch, Ok((7, 2)));
    }

    #[test]
    fn memory_read_failure_is_distinguishable_from_an_empty_list() {
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let empty = MemMemory(Ok(vec![]));
        let down = MemMemory(Err(ServiceError::new(ServiceErrorKind::Unavailable, "x")));
        assert!(rt.block_on(empty.list_memories(1)).is_ok());
        assert_eq!(
            rt.block_on(down.list_memories(1)).unwrap_err().kind,
            ServiceErrorKind::Unavailable
        );
    }
```
(`unit()` 是 Todo 契约测试里已有的辅助函数,同模块可直接用。)

Run: `cargo test -p dozer-services 2>&1 | tail -8` → Expected: 编译失败 `cannot find trait MemoryService`(RED)。

- [ ] **Step 2: 实现**

顶部 `use dozer_model::{...}` 追加 `AgentKind, MemoryDetail, MemoryInfo, ProjectInfo, SummaryBatchInfo`(保持字母序合并进现有列表)。在 `TodoService` 之后、`Services` 之前加:

```rust
/// 共享记忆 CRUD(Project 面板)。参数按值传。
pub trait MemoryService: Send + Sync {
    fn list_memories(&self, project_id: i64) -> BoxFuture<'_, Result<Vec<MemoryInfo>, ServiceError>>;
    fn get_memory(&self, project_id: i64, id: i64) -> BoxFuture<'_, Result<MemoryDetail, ServiceError>>;
    fn write_memory(
        &self,
        project_id: i64,
        title: String,
        kind: String,
        description: String,
        body: String,
        actor: String,
    ) -> BoxFuture<'_, Result<MemoryDetail, ServiceError>>;
    fn delete_memory(
        &self,
        project_id: i64,
        id: i64,
        actor: String,
    ) -> BoxFuture<'_, Result<(), ServiceError>>;
}

/// 项目维护(Project 面板):改名、取消登记、转录补录/删除、总结批次。
/// 返回值保持 `Client` 原样——面板用到补录条数、批次 id/进度、改名结果。
pub trait ProjectService: Send + Sync {
    fn rename_project(
        &self,
        id: i64,
        name: String,
    ) -> BoxFuture<'_, Result<Option<ProjectInfo>, ServiceError>>;
    fn remove_project(&self, id: i64) -> BoxFuture<'_, Result<(), ServiceError>>;
    fn delete_project_transcripts(&self, cwd: String) -> BoxFuture<'_, Result<u32, ServiceError>>;
    fn backfill_project_transcripts(&self, cwd: String) -> BoxFuture<'_, Result<u32, ServiceError>>;
    fn submit_summary_batch(
        &self,
        cwd: String,
        provider: Option<AgentKind>,
        model: Option<String>,
    ) -> BoxFuture<'_, Result<(i64, u32), ServiceError>>;
    fn get_summary_batch(
        &self,
        batch_id: i64,
    ) -> BoxFuture<'_, Result<Option<SummaryBatchInfo>, ServiceError>>;
}
```
**不要**在本任务给 `Services` 加字段(Task 3 做,否则 `dozer-app` 编译失败)。

Run: `cargo test -p dozer-services 2>&1 | grep -E 'test result|error'` → Expected: 全绿(原 6 个 + 新 2 个 = 8 passed)。

- [ ] **Step 3: 依赖验收与提交**

```bash
cargo tree -p dozer-services --edges normal | grep -vE 'dozer-model|serde|proc-macro2|quote|syn|unicode-ident|dozer-services' | head   # 期望无输出
cargo fmt -p dozer-services && cargo clippy -p dozer-services --all-targets 2>&1 | grep -c '^warning'   # 期望 0
git add crates/dozer-services && git commit -m "feat(services): Project S1 MemoryService + ProjectService trait

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 2: `Client` 实现两个 trait

**Files:**
- Modify: `crates/dozer-client/src/lib.rs`(两个 impl + 测试模块 `project_service_tests`;把 Todo 的 `todo_service_tests` 里的 `temp_sock`/`serve_once` 改为 `pub(super)` 以便复用)

**Interfaces:**
- Consumes: Task 1;`into_service_error`、`BoxFuture`、`ServiceError`。
- Produces: `impl MemoryService for Client`、`impl ProjectService for Client`(同名 inherent 方法不改)。

- [ ] **Step 1: 写失败测试**

先把 `mod todo_service_tests` 里的 `fn temp_sock` 与 `fn serve_once` 改为 `pub(super) fn`。然后在文件末尾追加:

```rust
#[cfg(test)]
mod project_service_tests {
    use super::todo_service_tests::serve_once;
    use super::*;
    use dozer_services::{MemoryService, ProjectService, ServiceErrorKind};

    #[tokio::test]
    async fn list_memories_comes_back_as_ok() {
        let sock = serve_once("mem-list", Some(Reply::Memories { memories: vec![] }));
        let got = MemoryService::list_memories(&Client::new(sock.clone()), 7).await;
        assert_eq!(got, Ok(vec![]));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn a_unit_write_returns_unit_on_success() {
        let sock = serve_once("proj-remove", Some(Reply::Ok));
        let got = ProjectService::remove_project(&Client::new(sock.clone()), 3).await;
        assert_eq!(got, Ok(()));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn a_count_is_carried_back_unchanged() {
        let sock = serve_once("proj-backfill", Some(Reply::BackfillDone { imported_files: 3 }));
        let got = ProjectService::backfill_project_transcripts(&Client::new(sock.clone()), "/p".into()).await;
        assert_eq!(got, Ok(3));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn hang_up_is_unavailable_with_the_legacy_text() {
        let sock = serve_once("proj-hangup", None);
        let e = MemoryService::list_memories(&Client::new(sock.clone()), 7).await.unwrap_err();
        assert_eq!((e.kind, e.message.as_str()), (ServiceErrorKind::Unavailable, "daemon 断开"));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn error_reply_is_rejected_with_the_legacy_prefix() {
        let sock = serve_once("proj-reject", Some(Reply::Error { message: "boom".into() }));
        let e = ProjectService::remove_project(&Client::new(sock.clone()), 3).await.unwrap_err();
        assert_eq!((e.kind, e.message.as_str()), (ServiceErrorKind::Rejected, "daemon 错误: boom"));
        let _ = std::fs::remove_file(&sock);
    }
}
```
Run: `cargo test -p dozer-client project_service_tests 2>&1 | grep -E '^error' | head -3` → Expected: 编译失败(`Client: MemoryService` 未实现)(RED)。

- [ ] **Step 2: 实现**

顶部 `use dozer_services::{...}` 加 `MemoryService, ProjectService`。在 `impl TodoService for Client` 之后追加。**每个方法都用 `Client::method(self, ..)` 路径调 inherent 方法**:

```rust
impl MemoryService for Client {
    fn list_memories(&self, project_id: i64) -> BoxFuture<'_, Result<Vec<MemoryInfo>, ServiceError>> {
        Box::pin(async move {
            Client::list_memories(self, project_id).await.map_err(into_service_error)
        })
    }
    fn get_memory(&self, project_id: i64, id: i64) -> BoxFuture<'_, Result<MemoryDetail, ServiceError>> {
        Box::pin(async move {
            Client::get_memory(self, project_id, id).await.map_err(into_service_error)
        })
    }
    fn write_memory(
        &self,
        project_id: i64,
        title: String,
        kind: String,
        description: String,
        body: String,
        actor: String,
    ) -> BoxFuture<'_, Result<MemoryDetail, ServiceError>> {
        Box::pin(async move {
            Client::write_memory(self, project_id, &title, &kind, &description, &body, &actor)
                .await
                .map_err(into_service_error)
        })
    }
    fn delete_memory(&self, project_id: i64, id: i64, actor: String) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move {
            Client::delete_memory(self, project_id, id, &actor).await.map_err(into_service_error)
        })
    }
}

impl ProjectService for Client {
    fn rename_project(&self, id: i64, name: String) -> BoxFuture<'_, Result<Option<ProjectInfo>, ServiceError>> {
        Box::pin(async move {
            Client::rename_project(self, id, &name).await.map_err(into_service_error)
        })
    }
    fn remove_project(&self, id: i64) -> BoxFuture<'_, Result<(), ServiceError>> {
        Box::pin(async move { Client::remove_project(self, id).await.map_err(into_service_error) })
    }
    fn delete_project_transcripts(&self, cwd: String) -> BoxFuture<'_, Result<u32, ServiceError>> {
        Box::pin(async move {
            Client::delete_project_transcripts(self, &cwd).await.map_err(into_service_error)
        })
    }
    fn backfill_project_transcripts(&self, cwd: String) -> BoxFuture<'_, Result<u32, ServiceError>> {
        Box::pin(async move {
            Client::backfill_project_transcripts(self, &cwd).await.map_err(into_service_error)
        })
    }
    fn submit_summary_batch(
        &self,
        cwd: String,
        provider: Option<AgentKind>,
        model: Option<String>,
    ) -> BoxFuture<'_, Result<(i64, u32), ServiceError>> {
        Box::pin(async move {
            Client::submit_summary_batch(self, &cwd, provider, model.as_deref())
                .await
                .map_err(into_service_error)
        })
    }
    fn get_summary_batch(
        &self,
        batch_id: i64,
    ) -> BoxFuture<'_, Result<Option<SummaryBatchInfo>, ServiceError>> {
        Box::pin(async move {
            Client::get_summary_batch(self, batch_id).await.map_err(into_service_error)
        })
    }
}
```
类型名(`MemoryInfo`、`MemoryDetail`、`ProjectInfo`、`SummaryBatchInfo`)在 `dozer-client` 里目前是 `dozer_core::protocol::…` 完整路径;impl 里统一用 `dozer_core::protocol::` 前缀写(它们是 `dozer_model` 同一类型的再导出),或在文件顶部 import 里补齐——以编译器为准,保持与现有 `Client` 方法签名的写法一致即可。

Run: `cargo test -p dozer-client 2>&1 | grep -E 'test result|FAILED|^error'` → Expected: 原有测试 + 5 个新测试全绿。

- [ ] **Step 3: 提交**

```bash
cargo fmt -p dozer-client
perl -ni -e 'print unless /^source = "git\+https:\/\/github.com\/byteboyai\/(bytegit|byteui)\?/' Cargo.lock
git add crates/dozer-client Cargo.lock && git status --short
git commit -m "feat(client): Project S2 Client 实现 MemoryService + ProjectService

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 3: `Services.project`/`memory` + 默认适配 + 假实现

**Files:**
- Modify: `crates/dozer-services/src/lib.rs`(`Services` 加两个字段;测试里的构造)
- Modify: `crates/dozer-app/src/panel_host.rs`(`services_for`)
- Modify: `crates/dozer-app/src/service_fakes.rs`(新增 `FakeMemory`、`FakeProject`、`base_services()`、`memory_detail()`;重构 `FakeTodo::into_services`)
- Modify: `crates/dozer-app/src/usage_probe/fake_usage.rs`(`into_services` 改用 `..base_services()`)

**Interfaces:**
- Consumes: Task 1/2。
- Produces:
  - `Services { usage, todo, project: Arc<dyn ProjectService>, memory: Arc<dyn MemoryService> }`
  - `services_for(&Client)` 的四个字段都由 `client.clone()` 适配。
  - `crate::service_fakes::base_services() -> Services`:四个字段都是默认成功的假实现(`FakeUsage::ok(vec![])`、`FakeTodo::default()`、`FakeProject::default()`、`FakeMemory::default()`)。各假实现的 `into_services(self)` 为 `Services { <字段>: Arc::new(self), ..base_services() }`。
  - `service_fakes::memory_detail(id: i64, title: &str) -> MemoryDetail`:字段全填的固定值(`project_id: 1`、`kind: "project"`、`history: vec![]` 等)。
  - `FakeMemory`(`Clone`,克隆共享状态,`Default` = 读写都成功、空列表):`FakeMemory::ok(list: Vec<MemoryInfo>)`、`failing_list(kind, message)`、`failing_writes(kind, message)`、`calls() -> Vec<String>`、`into_services()`。`get_memory(p,i)` 成功时返回 `memory_detail(i, "t")`;`write_memory` 成功时返回 `memory_detail(900, &title)`。调用记录形如 `"list_memories(7)"`、`"write_memory(7,标题)"`、`"delete_memory(7,3,user)"`。
  - `FakeProject`(`Clone`,共享状态,`Default` = 全部成功:rename → `Ok(None)`、remove → `Ok(())`、两个 transcripts → `Ok(0)`、submit → `Ok((1, 1))`、`get_summary_batch` → `Ok(None)`)。链式设置:`.with_rename(Result<Option<ProjectInfo>, ServiceError>)`、`.with_remove(Result<(), ServiceError>)`、`.with_delete_transcripts(Result<u32, ServiceError>)`、`.with_backfill(Result<u32, ServiceError>)`、`.with_submit(Result<(i64, u32), ServiceError>)`、`.with_batches(Vec<Result<Option<SummaryBatchInfo>, ServiceError>>)`(按调用顺序依次返回,用完后重复最后一项)。`calls() -> Vec<String>`:`"rename_project(1,新名)"`、`"remove_project(1)"`、`"delete_project_transcripts(/p)"`、`"backfill(/p)"`、`"submit_summary_batch(/p)"`、`"get_summary_batch(7)"`。
  - `service_fakes::batch(total: u32, queued: u32, running: u32, succeeded: u32, failed: u32, skipped: u32) -> SummaryBatchInfo`:其余字段取固定值(`batch_id: 7`、`cwd: "/p"`、`strategy: "missing_or_stale"`、`cancelled: 0`、时间戳 0)。

- [ ] **Step 1: 写失败测试**

(a) `panel_host.rs` 测试模块(`default_services_route_todo_through_the_client_adapter` 旁)追加:
```rust
    #[test]
    fn default_services_route_project_and_memory_through_the_client_adapter() {
        let rt = test_runtime();
        let (io, _rx) = recording_io(&rt);
        // recording_io 的 Client 指向不存在的 socket → Unavailable
        let e = rt.block_on(io.services().memory.list_memories(1)).unwrap_err();
        assert_eq!(e.kind, dozer_services::ServiceErrorKind::Unavailable);
        let e = rt.block_on(io.services().project.remove_project(1)).unwrap_err();
        assert_eq!(e.kind, dozer_services::ServiceErrorKind::Unavailable);
    }
```
(b) `dozer-services` 里 `services_is_cloneable_and_shares_the_service` 同时构造两个新字段(用 Task 1 的 `MemMemory(Ok(vec![]))`、`MemProject`),并追加 `Arc::ptr_eq` 断言。

Run: `cargo test -p dozer-services 2>&1 | grep -E '^error' | head -3`、`cargo test -p dozer-app default_services_route_project 2>&1 | grep -E '^error' | head -3` → Expected: 编译失败(`Services` 没有字段 `project`)(RED)。

- [ ] **Step 2: 实现**

1. `Services` 加 `pub project: Arc<dyn ProjectService>,` 与 `pub memory: Arc<dyn MemoryService>,`。
2. `services_for` 加 `project: Arc::new(client.clone()), memory: Arc::new(client.clone()),`。
3. 在 `service_fakes.rs` 实现 `FakeMemory`、`FakeProject`、`base_services()`、`memory_detail()`、`batch()`,**模式同已有的 `FakeTodo`**(`Arc<Mutex<State>>`,`write`-类方法记录调用并按预设返回)。`FakeProject` 的 `get_summary_batch` 用 `State.batches: Vec<_>` 加游标:第 n 次调用返回 `batches[min(n, len-1)]`,`batches` 为空时返回 `Ok(None)`。`MemoryInfo`/`ProjectInfo` 字段以 `crates/dozer-model/src/project.rs` 为准(本计划已列出 `MemoryInfo`/`MemoryDetail` 的字段名;`ProjectInfo` 只在 `with_rename` 的测试里构造,先读该文件)。
4. `FakeTodo::into_services` 与 `FakeUsage::into_services` 改为 `Services { todo: Arc::new(self), ..base_services() }` / `Services { usage: Arc::new(self), ..base_services() }`。
5. 假实现的构造函数在 Task 4 前未被使用时,给 `impl` 加 `#[allow(dead_code)]`(Task 4 去掉)。

Run: `cargo test -p dozer-services 2>&1 | grep -E 'test result|^error'`;`cargo test -p dozer-app default_services_route_project 2>&1 | grep -E 'test result|^error'` → Expected: 通过。

- [ ] **Step 3: 全 workspace 编译与提交**

```bash
cargo build --workspace 2>&1 | grep -E '^error'          # 期望无输出
cargo test -p dozer-app -- panel_host usage_probe 2>&1 | grep -E 'test result|FAILED'
cargo clippy --all-targets 2>&1 | grep -c '^warning'      # 期望 ≤ 基线
cargo fmt -p dozer-app -p dozer-services
git checkout Cargo.lock 2>/dev/null
git add crates/dozer-services crates/dozer-app/src && git status --short
git commit -m "feat(host): Project S3 Services.project/memory + FakeMemory/FakeProject

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 4: Project 面板与 host 切换,读失败可见

> 说明(Ruling,计划阶段,沿用 Todo):`update` 的签名是一处原子改动,读/写/维护无法拆开,因此本任务一次完成切换。单提交较大,靠 Step 1 的失败测试与 Step 4 的全量测试兜底。另:规格 §3.2 把"`&Services` 还是分别传两个 `Arc`"留给计划——选 `&Services`(少一个参数,`update` 已是 8 参数并带 `#[allow(clippy::too_many_arguments)]`,不再加重)。`SUMMARY_BACKFILL_POLL_INTERVAL` 在测试里缩为 5ms(`cfg!(test)`),否则每条轮询路径测试要多等 2 秒。

**Files:**
- Modify: `crates/dozer-app/src/extensions/project.rs`(`Message::MemoriesLoadFailed`、`WorkspaceState` 的 `outbox`/`last_load_failure`/`take_outbox`、冷却常量、测试里的 `test_client()`/`noop_client`)
- Modify: `crates/dozer-app/src/extensions/project/update.rs`(签名、写操作、`MemoriesLoadFailed` 臂、`spawn_*`、`submit_name_edit`、`request_memories_refresh`、`on_activate`、轮询间隔常量)
- Modify: `crates/dozer-app/src/extensions/project/delete.rs`(`spawn_delete_project`)
- Modify: `crates/dozer-app/src/app/update.rs`(`project::update` 3 处、`spawn_scaffold_run`、`spawn_delete_project`、`drain_outboxes`)与 `crates/dozer-app/src/app/app.rs`(`set_project_name_focused`)
- Modify: `crates/dozer-app/src/usage_probe/host.rs`(行号引用若漂移,只改行号)

**Interfaces:**
- Consumes: Task 3 的 `Services`、`services_for`、`FakeMemory`、`FakeProject`、`batch()`;`ServiceError`/`ServiceErrorKind`。
- Produces:
  - `project::Message::MemoriesLoadFailed(dozer_services::ServiceErrorKind, String)`
  - `project::update(ws_state, msg, project_id, current_name, repo_path, services: &Services, handle, emit)`
  - `project::request_memories_refresh(project_id: i64, memory: &Arc<dyn MemoryService>, handle: &Handle, emit: impl Fn(Message) + Send + 'static)`
  - `project::spawn_scaffold_run(repo_path: PathBuf, project: Arc<dyn ProjectService>, handle: &Handle, emit: impl Fn(Message) + Send + 'static, skip_git_init: bool)`
  - `project::spawn_repair_run(repo_path: PathBuf, project_id: i64, project: Arc<dyn ProjectService>, handle: &Handle, emit)`
  - `project::submit_name_edit(ws_state, project_id, current_name, project: Arc<dyn ProjectService>, handle, emit)`
  - `project::delete::spawn_delete_project(project_id, repo_path, scope, project: Arc<dyn ProjectService>, handle, on_done)`
  - `WorkspaceState::take_outbox(&mut self) -> Vec<crate::extensions::toast::Pending>`

- [ ] **Step 1: 写失败测试**

在 `extensions/project.rs` 的 `mod tests`(紧挨现有测试之后)追加。先在该模块加 import:`use crate::service_fakes::{FakeMemory, FakeProject, batch}; use dozer_services::{ServiceError, ServiceErrorKind}; use std::sync::{Mutex, mpsc}; use crate::panel_host::testing::{TIMEOUT, runtime};`(与已有 import 重复的省略)。并把现有的 `fn test_client()` 改为(这一步先**保留旧名与旧类型**不动——旧测试在 Step 2 一起改;下面的新测试只用新辅助):

```rust
    fn collect() -> (impl Fn(Message) + Send + Sync + 'static, mpsc::Receiver<Message>) {
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        (move |m| { let _ = tx.lock().unwrap().send(m); }, rx)
    }

    fn memory_info(id: i64, title: &str) -> dozer_model::MemoryInfo {
        dozer_model::MemoryInfo {
            id,
            project_id: 1,
            title: title.into(),
            kind: "project".into(),
            description: String::new(),
            updated_ms: 0,
            updated_by: "user".into(),
        }
    }

    #[test]
    fn unavailable_memory_read_failure_keeps_the_list_and_stays_silent() {
        let mut ws = WorkspaceState::new(None, links::LinksState::default());
        let rt = runtime();
        let services = FakeMemory::default().into_services();
        let (e1, _r1) = collect();
        update(&mut ws, Message::MemoriesLoaded(vec![memory_info(1, "a")]), 1, "demo",
            std::path::Path::new("/tmp/demo"), &services, rt.handle(), e1);
        let (e2, _r2) = collect();
        update(&mut ws, Message::MemoriesLoadFailed(ServiceErrorKind::Unavailable, "daemon 断开".into()),
            1, "demo", std::path::Path::new("/tmp/demo"), &services, rt.handle(), e2);
        assert_eq!(ws.memories().len(), 1, "读失败不得清空已有记忆");
        assert!(ws.take_outbox().is_empty(), "Unavailable 由顶栏徽标表达,不弹 Toast");
    }

    #[test]
    fn repeated_rejected_memory_read_failures_toast_once_per_text() {
        let mut ws = WorkspaceState::new(None, links::LinksState::default());
        let rt = runtime();
        let services = FakeMemory::default().into_services();
        for _ in 0..3 {
            let (e, _r) = collect();
            update(&mut ws, Message::MemoriesLoadFailed(ServiceErrorKind::Rejected, "daemon 错误: boom".into()),
                1, "demo", std::path::Path::new("/tmp/demo"), &services, rt.handle(), e);
        }
        let toasts = ws.take_outbox();
        assert_eq!(toasts.len(), 1, "相同文本 60 秒内只提示一次");
        assert_eq!(toasts[0].level, crate::extensions::toast::Level::Error);
        assert!(toasts[0].text.contains("daemon 错误: boom"));
        assert_eq!(toasts[0].key.as_deref(), Some("project-memories-load"));
        let (e, _r) = collect();
        update(&mut ws, Message::MemoriesLoadFailed(ServiceErrorKind::Protocol, "另一种错误".into()),
            1, "demo", std::path::Path::new("/tmp/demo"), &services, rt.handle(), e);
        assert_eq!(ws.take_outbox().len(), 1, "换了文本要再提示");
    }

    #[test]
    fn failing_memory_list_emits_load_failed_instead_of_an_empty_loaded() {
        let rt = runtime();
        let memory = FakeMemory::failing_list(ServiceErrorKind::Unavailable, "x").into_services().memory;
        let (emit, rx) = collect();
        request_memories_refresh(1, &memory, rt.handle(), emit);
        assert!(
            matches!(rx.recv_timeout(TIMEOUT).unwrap(), Message::MemoriesLoadFailed(ServiceErrorKind::Unavailable, _)),
            "读失败必须发 MemoriesLoadFailed,不得发 MemoriesLoaded(vec![])"
        );
    }

    #[test]
    fn memory_write_failure_text_is_the_service_message_unchanged() {
        let rt = runtime();
        let fake = FakeMemory::failing_writes(ServiceErrorKind::Rejected, "daemon 错误: boom");
        let services = fake.clone().into_services();
        let mut ws = WorkspaceState::new(None, links::LinksState::default());
        let repo = std::path::Path::new("/tmp/demo");
        let (e, _r) = collect();
        update(&mut ws, Message::MemoryCreateStart, 1, "demo", repo, &services, rt.handle(), e);
        let (e, _r) = collect();
        update(&mut ws, Message::MemoryCreateTitleInput("标题".into()), 1, "demo", repo, &services, rt.handle(), e);
        let (e, rx) = collect();
        update(&mut ws, Message::MemoryCreateSubmit, 1, "demo", repo, &services, rt.handle(), e);
        match rx.recv_timeout(TIMEOUT).unwrap() {
            Message::MemoryMutated(Err(text)) => assert_eq!(text, "daemon 错误: boom"),
            other => panic!("意外消息: {other:?}"),
        }
        assert_eq!(fake.calls(), vec!["write_memory(1,标题)".to_string()]);
    }

    #[test]
    fn renaming_to_a_missing_project_reports_it_does_not_exist() {
        let rt = runtime();
        let project = FakeProject::default().into_services().project; // rename → Ok(None)
        let mut ws = WorkspaceState::new(None, links::LinksState::default());
        ws.name_editing = Some("新名".into());
        let (emit, rx) = collect();
        submit_name_edit(&mut ws, 1, "旧名", project, rt.handle(), emit);
        match rx.recv_timeout(TIMEOUT).unwrap() {
            Message::NameRenamed(1, Err(text)) => assert_eq!(text, "项目不存在"),
            other => panic!("意外消息: {other:?}"),
        }
    }

    /// 收集 `spawn_repair_run` 的全部消息,直到出现 `SummaryBackfillFailed` 或批次全部完成的 `SummaryBackfillProgress`。
    fn run_repair(project: FakeProject) -> Vec<Message> {
        let rt = runtime();
        let dir = tempfile::tempdir().unwrap();
        let (emit, rx) = collect();
        spawn_repair_run(dir.path().to_path_buf(), 1, project.into_services().project, rt.handle(), emit);
        let mut out = Vec::new();
        loop {
            let m = rx.recv_timeout(TIMEOUT).expect("修复流程应在超时内结束");
            let done = matches!(&m, Message::SummaryBackfillFailed(..))
                || matches!(&m, Message::SummaryBackfillProgress(_, completed, total, ..) if completed >= total);
            out.push(m);
            if done {
                return out;
            }
        }
    }

    #[test]
    fn repair_run_success_reports_the_import_count_and_final_progress() {
        let project = FakeProject::default()
            .with_backfill(Ok(2))
            .with_submit(Ok((7, 3)))
            .with_batches(vec![Ok(Some(batch(3, 0, 0, 3, 0, 0)))]);
        let msgs = run_repair(project);
        assert!(msgs.iter().any(|m| matches!(
            m,
            Message::TranscriptBackfillFinished(1, scaffold::ScaffoldStepResult::Created(t)) if t == "导入 2 个历史文件"
        )), "{msgs:?}");
        assert!(matches!(msgs.last().unwrap(), Message::SummaryBackfillProgress(1, 3, 3, 3, 0, 0)), "{msgs:?}");
    }

    #[test]
    fn repair_run_reports_a_missing_batch() {
        let project = FakeProject::default().with_batches(vec![Ok(None)]);
        let msgs = run_repair(project);
        assert!(matches!(msgs.last().unwrap(),
            Message::SummaryBackfillFailed(1, t) if t == "总结批次不存在，请重试"), "{msgs:?}");
    }

    #[test]
    fn repair_run_reports_a_failed_batch_query_with_the_service_text() {
        let project = FakeProject::default().with_batches(vec![Err(ServiceError::new(
            ServiceErrorKind::Rejected, "daemon 错误: nope"))]);
        let msgs = run_repair(project);
        assert!(matches!(msgs.last().unwrap(),
            Message::SummaryBackfillFailed(1, t) if t == "daemon 错误: nope"), "{msgs:?}");
    }

    #[test]
    fn scaffold_run_still_finishes_when_the_backfill_fails() {
        let rt = runtime();
        let dir = tempfile::tempdir().unwrap();
        let fake = FakeProject::default().with_backfill(Err(ServiceError::new(
            ServiceErrorKind::Unavailable, "daemon 断开")));
        let (emit, rx) = collect();
        spawn_scaffold_run(dir.path().to_path_buf(), fake.clone().into_services().project, rt.handle(), emit, true);
        assert!(matches!(rx.recv_timeout(TIMEOUT).unwrap(), Message::ScaffoldDone));
        assert_eq!(fake.calls().len(), 1, "补录失败不阻止流程结束,也不重试");
    }

    #[test]
    fn delete_stops_after_a_failed_deregistration() {
        let rt = runtime();
        let dir = tempfile::tempdir().unwrap();
        let fake = FakeProject::default().with_remove(Err(ServiceError::new(
            ServiceErrorKind::Rejected, "boom")));
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        delete::spawn_delete_project(
            1, dir.path().to_path_buf(), delete::DeleteScope::WithAgentCache,
            fake.clone().into_services().project, rt.handle(),
            move |id, dereg, errors| { let _ = tx.lock().unwrap().send((id, dereg, errors)); },
        );
        let (id, dereg, errors) = rx.recv_timeout(TIMEOUT).unwrap();
        assert_eq!((id, dereg), (1, false));
        assert_eq!(errors, vec!["取消项目登记失败: boom".to_string()]);
        assert_eq!(fake.calls(), vec!["remove_project(1)".to_string()], "不得继续删转录");
    }
```
同时把 `WorkspaceState` 的 `memories` 取值方法名以实际为准(若不存在 `memories()` accessor,用字段名或补一个只读 accessor,和 Todo 的 `items()` 同款)。

Run: `cargo test -p dozer-app extensions::project 2>&1 | grep -E '^error' | head -6` → Expected: 编译失败(`MemoriesLoadFailed` 不存在、`update`/`spawn_*` 参数类型不符、`take_outbox` 不存在)(RED)。

- [ ] **Step 2: 实现(面板侧)**

`extensions/project.rs`:
1. `Message` 在 `MemoriesLoaded(..)` 之后加(带文档注释):
```rust
    /// 记忆列表读取失败:**不改任何列表**。`Unavailable` 静默(顶栏徽标已表达"dozerd 不可用");
    /// 其余弹带 key 的 Error Toast,相同文本 60 秒内只提示一次(见 `last_load_failure`)。
    MemoriesLoadFailed(dozer_services::ServiceErrorKind, String),
```
2. `WorkspaceState` 加字段(`#[derive(Default)]` 仍成立):
```rust
    /// 待发提示;`App::drain_outboxes` 排空成 Toast。
    pub(crate) outbox: crate::extensions::toast::Outbox,
    /// 上一次已提示的读失败(文本 + 时刻),用于冷却去重。
    pub(crate) last_load_failure: Option<(String, std::time::Instant)>,
```
并加 `pub fn take_outbox(&mut self) -> Vec<crate::extensions::toast::Pending> { self.outbox.take() }`,常量 `pub(crate) const LOAD_FAILURE_TOAST_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(60);`。
3. 测试里的 `fn test_client()` 改为 `fn test_services() -> dozer_services::Services { crate::service_fakes::base_services() }`;全文件 `&test_client()` → `&test_services()`(共 33 处,可用 `sed -i '' 's/&test_client()/\&test_services()/g'`);`delete_project_request_then_scope_select_then_cancel` 里 `let noop_client = dozer_client::Client::new(..)` 改为 `let noop_services = crate::service_fakes::base_services();` 并把 `&noop_client` 改为 `&noop_services`(该文件内其它 `noop_client` 同改)。

`extensions/project/update.rs`:
1. 顶部 `use dozer_services::{MemoryService, ProjectService, ServiceErrorKind, Services}; use std::sync::Arc;`。
2. `update` 的 `client: &dozer_client::Client` → `services: &Services`。
3. 逐处改写(`e.to_string()` → `e.message`,`.map(|_| ())` 去掉的地方仅限返回 `()` 的方法;`write_memory` 返回 `MemoryDetail`,保留 `.map(|_| ())`):
   - `NameEditSubmit` → `submit_name_edit(ws_state, project_id, current_name, services.project.clone(), handle, emit)`;`RepairProject` → `spawn_repair_run(repo_path.to_path_buf(), project_id, services.project.clone(), handle, emit)`。
   - `MemoryCreateSubmit`:`let memory = services.memory.clone();` 后 `memory.write_memory(project_id, draft.title, draft.kind, draft.description, draft.body, "user".to_string()).await.map(|_| ()).map_err(|e| e.message)`。
   - `MemoryMutated`:把内联的 `list_memories(..).unwrap_or_default()` 任务替换为 `request_memories_refresh(project_id, &services.memory, handle, emit);`。
   - `MemoryDetailOpen`:`let memory = services.memory.clone();` 后 `let res = memory.get_memory(project_id, id).await; emit(memory_detail_message(res));`。
   - `MemoryEditSubmit`:同 `write_memory(..., "user".to_string())`;成功分支 `memory.get_memory(project_id, detail.id)`;失败分支 `Err(e) => emit(Message::MemoryMutated(Err(e.message)))`。
   - `MemoryDeleteConfirm`:`memory.delete_memory(project_id, id, "user".to_string()).await.map_err(|e| e.message)`。
   - 新增臂(放在 `Message::MemoryMutated` 之前或之后均可):
```rust
        Message::MemoriesLoadFailed(kind, message) => {
            if kind == ServiceErrorKind::Unavailable {
                dozer_core::log_debug!(LOG, "记忆列表读取失败(dozerd 不可用): {message}");
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
                        format!("记忆加载失败: {message}"),
                        "project-memories-load",
                    );
                    ws_state.last_load_failure = Some((message, now));
                }
            }
        }
```
4. `request_memories_refresh`:
```rust
pub fn request_memories_refresh(
    project_id: i64,
    memory: &Arc<dyn MemoryService>,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    let memory = memory.clone();
    handle.spawn(async move {
        match memory.list_memories(project_id).await {
            Ok(memories) => emit(Message::MemoriesLoaded(memories)),
            Err(e) => emit(Message::MemoriesLoadFailed(e.kind, e.message)),
        }
    });
}
```
`on_activate` 里 `ctx.io.client()` → `&ctx.io.services().memory`。
5. `spawn_scaffold_run(repo_path, project: Arc<dyn ProjectService>, handle, emit, skip_git_init)`:`let _ = client.backfill_project_transcripts(&cwd).await;` →
```rust
        if let Err(e) = project.backfill_project_transcripts(cwd.clone()).await {
            dozer_core::log_warn!(LOG, error = %e, "补录转录失败");
        }
```
6. `spawn_repair_run(repo_path, project_id, project: Arc<dyn ProjectService>, handle, emit)`:`client.backfill_project_transcripts(&cwd)` → `project.backfill_project_transcripts(cwd.clone())`,`Failed(e.to_string())` → `Failed(e.message)`;`client.submit_summary_batch(&cwd, None, None)` → `project.submit_summary_batch(cwd.clone(), None, None)`,`e.to_string()` → `e.message`;`client.get_summary_batch(batch_id)` → `project.get_summary_batch(batch_id)`,两处 `e.to_string()` → `e.message`。
7. `const SUMMARY_BACKFILL_POLL_INTERVAL: std::time::Duration = if cfg!(test) { std::time::Duration::from_millis(5) } else { std::time::Duration::from_secs(2) };`
8. `submit_name_edit(ws_state, project_id, current_name, project: Arc<dyn ProjectService>, handle, emit)`:`project.rename_project(project_id, name).await.map_err(|e| e.message).and_then(|opt| opt.ok_or_else(|| "项目不存在".to_string()))`。

`extensions/project/delete.rs`:`client: dozer_client::Client` → `project: std::sync::Arc<dyn dozer_services::ProjectService>`;`client.remove_project(project_id)` → `project.remove_project(project_id)`;`client.delete_project_transcripts(&cwd)` → `project.delete_project_transcripts(cwd.clone())`(两处错误文案里的 `{e}` 照旧,`ServiceError` 的 Display 即原文本)。

- [ ] **Step 3: 实现(host 侧)**

`app/update.rs`:
- 三处 `project::update(.., &client, ..)`(约 3400、3456、3508 行):把各自上方的 `let client = self.client.clone();` 改为 `let services = crate::panel_host::services_for(&self.client);`,调用处 `&client` → `&services`。
- `project::spawn_scaffold_run(repo_path, client, ..)`(约 4133 行):`let client = self.client.clone();` → `let project = crate::panel_host::services_for(&self.client).project;`,传 `project`。
- `project_delete_confirm`(约 4250 行):同理 `.project`,传给 `spawn_delete_project`。
- `drain_outboxes`(约 219–221 行):在 `pending.extend(ws.todo.take_outbox());` 那一排加 `pending.extend(ws.project_panel.take_outbox());`。

`app/app.rs` `set_project_name_focused`(约 2425 行):`let client = self.client.clone();` → `let project = crate::panel_host::services_for(&self.client).project;`,`submit_name_edit(.., client, ..)` → `project`。

去掉 Task 3 里临时加的 `#[allow(dead_code)]`(`service_fakes.rs`)。

- [ ] **Step 4: 运行面板测试**

```bash
cargo test -p dozer-app extensions::project 2>&1 | grep -E 'test result|FAILED|^error'
```
Expected: 全绿(含 11 个新测试,既有项目面板测试因 `test_services()` 替换继续通过)。若 `MemoriesLoadFailed` 在 `app/update.rs` 里被某个专属臂拦截,按提示处理;**不得**新增面板专属臂。

- [ ] **Step 5: 探针守卫、全量、门禁、提交**

```bash
cargo test -p dozer-app usage_probe::host 2>&1 | grep -E 'test result|FAILED|实际是'   # 行号漂移只改 host_site() 与注释里的行号,不改守卫逻辑
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
git commit -m "feat(project): Project S4 面板改用 ProjectService/MemoryService,记忆读失败保留列表并分档提示

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 5: `R-CLIENT` 基线下降、验收与文档回填

**Files:**
- Modify: `scripts/audit/panel-boundary.baseline.json`(仅 `extensions/project*` 的 `R-CLIENT` 条目)
- Modify: `docs/dozer-v2/bytehost-H0/08-client-use-inventory.md`、`CLAUDE.md`

**Interfaces:**
- Produces: `extensions/project.rs`、`extensions/project/update.rs`、`extensions/project/delete.rs` 不再出现在 `R-CLIENT` 基线里(`project_create.rs`、`project/…` 以外的文件不变)。

- [ ] **Step 1: 确认门禁现状并下调基线**

```bash
grep -rn 'dozer_client\|io\.client()' crates/dozer-app/src/extensions/project.rs crates/dozer-app/src/extensions/project/ | head -5   # 期望无输出
python3 scripts/audit/check_panel_boundary.py   # 此时 project* 的 R-CLIENT 实测应为 0(低于基线),门禁只显示既有 R-HOST-EXECUTORS
python3 - <<'EOF'
import json
p = "scripts/audit/panel-boundary.baseline.json"
base = json.load(open(p, encoding="utf-8"))
for rel in [r for r in list(base) if r == "extensions/project.rs" or r.startswith("extensions/project/")]:
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
Expected: 基线 diff 只含 `extensions/project*` 条目的删除(`extensions/project_create.rs` 不在其中,它不属于 `project/` 前缀也不等于 `project.rs`);门禁仍只报 `R-HOST-EXECUTORS 0 -> 1`。

- [ ] **Step 2: 验收清单(逐条跑、读输出)**

```bash
cargo tree -p dozer-services --edges normal                    # 1: 仅 dozer-model + serde 系
python3 scripts/audit/unit_edges.py ext:project                # 2: 无 dozer_client
cargo test -p dozer-app extensions::project 2>&1 | grep -E 'test result|FAILED'   # 3/4
cargo test -p dozer-core --test model_golden 2>&1 | grep 'test result'            # 5
W=$(ls -d .superpowers/sdd/*/ | head -1)
cargo test --workspace --no-fail-fast > ${W}final.txt 2>&1; grep -E '^test result' ${W}final.txt | awk '{s+=$4;f+=$6} END{print s,f}'; grep -E '^test .* FAILED' ${W}final.txt   # 6
cargo clippy --all-targets 2>&1 | grep -c '^warning'; cargo fmt --check && echo fmt-ok
bash scripts/check-log-scope.sh && echo log-scope-ok
git status --short                                             # 无 Info.plist、无无关 Cargo 改动
```
任一不满足即不收尾,回到对应任务修。

- [ ] **Step 3: 文档回填并提交**

- `docs/dozer-v2/bytehost-H0/08-client-use-inventory.md` 末尾追加一节"Project 迁移(2026-10-10)":`extensions/project.rs` 与 `extensions/project/**` 经 `MemoryService`/`ProjectService` 取数,`R-CLIENT` 基线中 `project*` 已清零;记忆读失败改为 `MemoriesLoadFailed`(保留旧列表,`Unavailable` 静默,其余弹 `project-memories-load` Toast,同文本 60 秒内一次);`spawn_scaffold_run` 补录失败改写 warn 日志。**登记后续项**:`MemoriesLoaded`/`MemoryDetailLoaded` 没有项目/代数保护;读失败冷却逻辑在 Todo 与 Project 各有一份(`LOAD_FAILURE_TOAST_COOLDOWN`),可抽到 `toast::Outbox`;`project_create.rs` 仍直接用 `Client`(`open_project`/`list_projects`)。
- `CLAUDE.md` Workspace 布局表 `dozer-services` 行里"Usage、Todo 已迁,其余面板待迁"改为"Usage、Todo、Project 已迁,其余面板待迁"。
```bash
git add scripts/audit docs CLAUDE.md
git commit -m "chore(audit): Project S5 R-CLIENT 基线下调 + 文档回填

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

之后按 executing-plans 的 Final Review(整分支 fresh reviewer)与 finishing-a-development-branch;**合并进 main 属于需先询问用户的外部副作用**,合并前核对 main 上用户 WIP(`Cargo.lock`、`crates/dozer-app/Cargo.toml`、`Info.plist` 及 `extensions/files/*` 等)与本分支的重叠,用 `git stash` → 合并 → `git stash pop`。
