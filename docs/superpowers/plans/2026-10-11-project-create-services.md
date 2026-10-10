# ProjectCreate 对话框迁移到服务 trait Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 创建/克隆项目对话框(`extensions/project_create.rs`)的 2 个服务调用(`open_project`、`list_projects`)改为经 `dozer-services` 已有的 `ProjectService`;用户可见行为零变化。

**Architecture:** 给 `ProjectService` 增加 `open_project`、`list_projects` 两个方法(`Services` 字段不变);`Client` 实现它们(复用 `into_service_error`);`update`/`spawn_create_local`/`spawn_clone` 改吃 `Arc<dyn ProjectService>`;host 的 1 处转发改用 `services_for(&client).project`。`list_projects` 失败保持"最近项目当空列表",但改写 warn 日志而不是无声吞掉。

**Tech Stack:** Rust 2024 workspace、tokio、iced 0.14(只涉及对话框 spawn 层)、Python 门禁脚本。

**Spec:** `docs/superpowers/specs/2026-10-11-project-create-services-design.md`(先例:`2026-10-10-project-services-design.md`、`2026-10-11-context-services-design.md`)

## Global Constraints

- `dozer-services` 只依赖 `dozer-model`;不加 `dozer-core`/`dozer-client`/`tokio` 到 `[dependencies]`。
- 线上协议零变化:`Request`/`Reply` 不动,`crates/dozer-core/tests/model_golden.rs` 不改且通过;`Client::open_project`/`list_projects` 的 inherent 签名不改。
- 失败文案逐字不变:`"注册项目失败: {e}"`(`e` 的 `Display` 即服务消息);`Done` 的结果形状 `(Option<ProjectInfo>, Vec<ProjectInfo>, bool)` 不变;表单校验、`bytegit::clone`、`std::fs`、`project_meta::write_description` 等本地副作用不动。
- `list_projects` 失败:`recent` 仍为空列表且整体仍成功,写一条 warn 日志(来源 `module` `project_create`:`dozer_core::scope!(LOG, module, "project_create");`——它是弹窗而非 `PanelKind` 面板,按 CLAUDE.md 用 `module` 来源);不改 host 的 `project_tab_opened` 语义。
- `R-HOST-PANEL-ARMS` 不上升:不新增 `app/update.rs` 的面板专属臂。
- 在独立 worktree 实施;不提交用户 WIP(`Info.plist`、版本号相关改动);`Cargo.lock` 提交前 `git checkout Cargo.lock`(main 上已提交的锁文件缺 bytegit/byteui 的 `source` 行,干净 worktree 里跑 cargo 会补上,属噪声)。
- main 上已知问题不修不算回归:`files::tests::delete_confirm_spec_reflects_pending_target` 失败、并行负载下的间歇失败(`git_log::build_*`、`assets::tests::serves_vendored_asset_with_mime`、dozerd 的 `memory`/`headless_agent` 测试)、门禁 `R-HOST-EXECUTORS 0 -> 1`。
- 提交信息末尾加 `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`。

## Review Focus

- `open_project` 失败得到 `Err("注册项目失败: {服务文本}")`,**且之后不再调用 `list_projects`**;成功时 `Ok((opened, recent, skip))` 里 `skip` 对本地创建是 `!create_git`、对克隆恒为 `false` → Task 4 测试。
- `list_projects` 失败仍整体成功、`recent` 为空,并写 warn 日志(不是无声吞掉)→ Task 4 测试。
- `open_project` 收到的是目标路径的字符串形式(`target.to_string_lossy()`),在目录与描述文件创建之后才调用 → Task 4 测试。
- `SubmitLocal` 表单校验失败时**不 spawn、服务无调用**(既有短路顺序)→ Task 4 测试。
- `impl ProjectService for Client` 新增的两个方法必须调 inherent 方法而不是递归调 trait 方法;`Option<ProjectInfo>` 的 `Some`/`None` 都原样带回 → Task 2 适配测试。

---

### Task 1: `ProjectService` 增加 `open_project`、`list_projects`(`dozer-services`)

**Files:**
- Modify: `crates/dozer-services/src/lib.rs`

**Interfaces:**
- Consumes: `dozer_model::ProjectInfo`(已在 `dozer-model`)。
- Produces(追加到已有的 `pub trait ProjectService`):
  - `open_project(path: String) -> BoxFuture<'_, Result<Option<ProjectInfo>, ServiceError>>`
  - `list_projects() -> BoxFuture<'_, Result<Vec<ProjectInfo>, ServiceError>>`

- [ ] **Step 0: 记录基线(只测量,写进 ledger;同步执行,跑完再进 Step 1,避免并发编译污染)**

```bash
W=$(ls -d .superpowers/sdd/*/ | head -1)
grep -c 'dozer_client\|io\.client()' crates/dozer-app/src/extensions/project_create.rs     # 记下数字(基线 R-CLIENT 为 1)
cargo test --workspace --no-fail-fast > ${W}baseline-tests.txt 2>&1; grep -E '^test result' ${W}baseline-tests.txt | awk '{s+=$4;f+=$6} END{print "passed",s,"failed",f}'; grep -E '^test .* FAILED' ${W}baseline-tests.txt
cargo clippy --all-targets 2>&1 | grep -c '^warning'
python3 scripts/audit/check_panel_boundary.py; echo exit=$?
```
Expected: 失败仅已知的;门禁仅 `R-HOST-EXECUTORS 0 -> 1`;把数字写进 ledger。

- [ ] **Step 1: 写失败测试**

在 `crates/dozer-services/src/lib.rs` 现有 `#[cfg(test)] mod tests` 里,先在已有的 `impl ProjectService for MemProject { … }` 里**追加**两个方法(此时 trait 里还没有它们,编译会失败,即 RED):

```rust
        fn open_project(&self, _p: String) -> BoxFuture<'_, Result<Option<ProjectInfo>, ServiceError>> {
            Box::pin(async { Ok(None) })
        }
        fn list_projects(&self) -> BoxFuture<'_, Result<Vec<ProjectInfo>, ServiceError>> {
            Box::pin(async { Ok(vec![]) })
        }
```
并在模块末尾追加:

```rust
    /// 新增的两个方法同样能在 `tokio::spawn` 里使用(`Arc<dyn ProjectService>` 的 future 要 `Send + 'static`)。
    #[test]
    fn dyn_project_open_and_list_run_inside_tokio_spawn() {
        let project: Arc<dyn ProjectService> = Arc::new(MemProject);
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let (opened, listed) = rt.block_on(async move {
            tokio::spawn(async move {
                let opened = project.open_project("/p".into()).await;
                let listed = project.list_projects().await;
                (opened, listed)
            })
            .await
            .unwrap()
        });
        assert_eq!(opened, Ok(None));
        assert_eq!(listed, Ok(vec![]));
    }
```
Run: `cargo test -p dozer-services 2>&1 | tail -8` → Expected: 编译失败 `method open_project is not a member of trait ProjectService`(RED)。

- [ ] **Step 2: 实现**

在 `pub trait ProjectService` 的末尾(`get_summary_batch` 之后)追加:

```rust
    /// 登记(或取回)一个项目;路径不存在等情形服务端返回 `Ok(None)` 或 `Err`,面板按原样展示。
    fn open_project(&self, path: String) -> BoxFuture<'_, Result<Option<ProjectInfo>, ServiceError>>;
    /// 项目列表(最近优先),供创建后刷新最近项目。
    fn list_projects(&self) -> BoxFuture<'_, Result<Vec<ProjectInfo>, ServiceError>>;
```
Run: `cargo test -p dozer-services 2>&1 | grep -E 'test result|error'` → Expected: `dozer-services` 全绿(原 21 个 + 新 1 个 = 22 passed);此时 `cargo build --workspace` 会因 `Client`/`FakeProject` 尚未实现新方法而失败,属预期,Task 2/3 补上。

- [ ] **Step 3: 依赖验收与提交**

```bash
cargo tree -p dozer-services --edges normal | grep -vE 'dozer-model|serde|proc-macro2|quote|syn|unicode-ident|dozer-services' | head   # 期望无输出
cargo fmt -p dozer-services && cargo clippy -p dozer-services --all-targets 2>&1 | grep -c '^warning'   # 期望 0
git add crates/dozer-services && git commit -m "feat(services): ProjectCreate S1 ProjectService 增加 open_project/list_projects

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```
(注意:本提交之后到 Task 2/3 完成之前,整个 workspace 不能编译;这是有意的,Task 1–3 作为一个连续的编译修复链执行,不在中间停留。)

---

### Task 2: `Client` 实现新增的两个方法

**Files:**
- Modify: `crates/dozer-client/src/lib.rs`(`impl ProjectService for Client` 加两个方法 + 测试)

**Interfaces:**
- Consumes: Task 1;`into_service_error`、`BoxFuture`、`ServiceError`;`todo_service_tests::serve_once`(已 `pub(super)`)。
- Produces: `impl ProjectService for Client` 的 `open_project`、`list_projects`(inherent 方法不改)。

- [ ] **Step 1: 写失败测试**

应答变体已核对(`dozer-client/src/lib.rs` 约 341–370 行):`open_project` → `Reply::Project { project }`(`project: Option<ProjectInfo>`),`list_projects` → `Reply::Projects { projects }`。在 `project_service_tests` 模块里追加(`ProjectInfo` 用 `dozer_core::protocol::ProjectInfo`,字段 `id, path, name, last_active_ms, created_ms, updated_ms`):

```rust
    fn project_info(id: i64, path: &str) -> dozer_core::protocol::ProjectInfo {
        dozer_core::protocol::ProjectInfo {
            id,
            path: path.into(),
            name: "demo".into(),
            last_active_ms: 1,
            created_ms: 2,
            updated_ms: 3,
        }
    }

    #[tokio::test]
    async fn open_project_carries_some_and_none_back_unchanged() {
        let info = project_info(5, "/p/demo");
        let sock = serve_once("pc-open-some", Some(Reply::Project { project: Some(info.clone()) }));
        let got = ProjectService::open_project(&Client::new(sock.clone()), "/p/demo".into()).await;
        assert_eq!(got, Ok(Some(info)));
        let _ = std::fs::remove_file(&sock);

        let sock = serve_once("pc-open-none", Some(Reply::Project { project: None }));
        let got = ProjectService::open_project(&Client::new(sock.clone()), "/p/none".into()).await;
        assert_eq!(got, Ok(None));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn list_projects_comes_back_as_ok() {
        let info = project_info(5, "/p/demo");
        let sock = serve_once("pc-list", Some(Reply::Projects { projects: vec![info.clone()] }));
        let got = ProjectService::list_projects(&Client::new(sock.clone())).await;
        assert_eq!(got, Ok(vec![info]));
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn new_methods_map_hang_up_and_error_reply_like_the_others() {
        let sock = serve_once("pc-hangup", None);
        let e = ProjectService::list_projects(&Client::new(sock.clone())).await.unwrap_err();
        assert_eq!((e.kind, e.message.as_str()), (ServiceErrorKind::Unavailable, "daemon 断开"));
        let _ = std::fs::remove_file(&sock);

        let sock = serve_once("pc-reject", Some(Reply::Error { message: "boom".into() }));
        let e = ProjectService::open_project(&Client::new(sock.clone()), "/p".into()).await.unwrap_err();
        assert_eq!((e.kind, e.message.as_str()), (ServiceErrorKind::Rejected, "daemon 错误: boom"));
        let _ = std::fs::remove_file(&sock);
    }
```
Run: `cargo test -p dozer-client project_service_tests 2>&1 | grep -E '^error' | head -3` → Expected: 编译失败(`Client` 的 `ProjectService` 实现缺少 `open_project`/`list_projects`)(RED)。

- [ ] **Step 2: 实现**

在 `impl ProjectService for Client { … }` 里追加(用 `Client::method(self, ..)` 路径调 inherent 方法):

```rust
    fn open_project(
        &self,
        path: String,
    ) -> BoxFuture<'_, Result<Option<dozer_core::protocol::ProjectInfo>, ServiceError>> {
        Box::pin(async move {
            Client::open_project(self, &path)
                .await
                .map_err(into_service_error)
        })
    }
    fn list_projects(
        &self,
    ) -> BoxFuture<'_, Result<Vec<dozer_core::protocol::ProjectInfo>, ServiceError>> {
        Box::pin(async move { Client::list_projects(self).await.map_err(into_service_error) })
    }
```
类型路径以文件里已有 import 为准(若已 `use dozer_core::protocol::ProjectInfo` 则去前缀)。

Run: `cargo test -p dozer-client 2>&1 | grep -E 'test result|FAILED|^error'` → Expected: 原有测试 + 3 个新测试全绿。

- [ ] **Step 3: 提交**

```bash
cargo fmt -p dozer-client
cargo clippy -p dozer-client --all-targets 2>&1 | grep -c '^warning'   # 期望 0
git checkout Cargo.lock 2>/dev/null
git add crates/dozer-client && git status --short
git commit -m "feat(client): ProjectCreate S2 Client 实现 open_project/list_projects

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 3: `FakeProject` 补两个方法

**Files:**
- Modify: `crates/dozer-app/src/service_fakes.rs`

**Interfaces:**
- Consumes: Task 1/2。
- Produces:
  - `FakeProject` 新增链式设置:`.with_open(Result<Option<ProjectInfo>, ServiceError>)`(默认 `Ok(None)`)、`.with_list_projects(Result<Vec<ProjectInfo>, ServiceError>)`(默认 `Ok(vec![])`);调用记录 `"open_project(<path>)"`、`"list_projects()"`(与已有的 `calls()` 同一份)。
  - `service_fakes::project_info(id: i64, path: &str) -> ProjectInfo`:`name: "demo"`、`last_active_ms: 1`、`created_ms: 2`、`updated_ms: 3`。

- [ ] **Step 1: 写失败测试**

`dozer-app` 里此刻因 `FakeProject` 缺方法而无法编译(Task 1 的连锁结果)。**这就是 RED**:

Run: `cargo build -p dozer-app 2>&1 | grep -E '^error' | head -3` → Expected: 编译失败(`FakeProject` 的 `ProjectService` 实现缺少 `open_project`/`list_projects`)。

- [ ] **Step 2: 实现**

1. `ProjectState` 增加字段 `open: Result<Option<ProjectInfo>, ServiceError>` 与 `projects: Result<Vec<ProjectInfo>, ServiceError>`;`Default` 里 `open: Ok(None)`、`projects: Ok(vec![])`。
2. `impl FakeProject` 追加:
```rust
    pub(crate) fn with_open(self, r: Result<Option<ProjectInfo>, ServiceError>) -> Self {
        self.state.lock().unwrap().open = r;
        self
    }
    pub(crate) fn with_list_projects(self, r: Result<Vec<ProjectInfo>, ServiceError>) -> Self {
        self.state.lock().unwrap().projects = r;
        self
    }
```
3. `impl ProjectService for FakeProject` 追加:
```rust
    fn open_project(&self, path: String) -> BoxFuture<'_, Result<Option<ProjectInfo>, ServiceError>> {
        let mut s = self.state.lock().unwrap();
        s.calls.push(format!("open_project({path})"));
        let r = s.open.clone();
        Box::pin(async move { r })
    }
    fn list_projects(&self) -> BoxFuture<'_, Result<Vec<ProjectInfo>, ServiceError>> {
        let mut s = self.state.lock().unwrap();
        s.calls.push("list_projects()".to_string());
        let r = s.projects.clone();
        Box::pin(async move { r })
    }
```
4. 模块级辅助:
```rust
/// 固定 `ProjectInfo`。
pub(crate) fn project_info(id: i64, path: &str) -> ProjectInfo {
    ProjectInfo {
        id,
        path: path.into(),
        name: "demo".into(),
        last_active_ms: 1,
        created_ms: 2,
        updated_ms: 3,
    }
}
```
5. 新增的构造函数/辅助在 Task 4 之前未被使用时,加 `#[allow(dead_code)]`(Task 4 去掉)。

Run: `cargo build --workspace 2>&1 | grep -E '^error'` → Expected: 无输出。

- [ ] **Step 3: 全 workspace 验证与提交**

```bash
cargo test -p dozer-services -p dozer-client 2>&1 | grep -E 'test result' | head -3
cargo test -p dozer-app -- project panel_host usage_probe 2>&1 | grep -E 'test result|FAILED'
cargo clippy --all-targets 2>&1 | grep -c '^warning'      # 期望 ≤ 基线
cargo fmt -p dozer-app -p dozer-services
git checkout Cargo.lock 2>/dev/null
git add crates/dozer-app/src && git status --short
git commit -m "feat(host): ProjectCreate S3 FakeProject 补 open_project/list_projects

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 4: 对话框与 host 切换

**Files:**
- Modify: `crates/dozer-app/src/extensions/project_create.rs`(`update`、`spawn_create_local`、`spawn_clone`;`LOG` 声明;测试)
- Modify: `crates/dozer-app/src/app/update.rs`(`Message::ProjectCreate(msg)` 的转发)
- Modify: `crates/dozer-app/src/service_fakes.rs`(去掉临时 `#[allow(dead_code)]`)
- Modify: `crates/dozer-app/src/usage_probe/host.rs`(行号引用若漂移,只改行号)

**Interfaces:**
- Consumes: Task 3 的 `FakeProject::{with_open, with_list_projects, calls}`、`project_info`、`ProjectService`。
- Produces:
  - `update(state: &mut Option<State>, msg: Message, project: Arc<dyn ProjectService>, handle: &Handle, emit: impl Fn(Message) + Send + 'static)`
  - `async fn spawn_create_local(target: PathBuf, description: String, create_git: bool, project: &Arc<dyn ProjectService>) -> SubmitResult`;`async fn spawn_clone(url: String, target: PathBuf, description: String, project: &Arc<dyn ProjectService>) -> SubmitResult`

- [ ] **Step 1: 写失败测试**

在 `project_create.rs` 的 `mod tests` 追加(另加 `use crate::panel_host::testing::{TIMEOUT, runtime}; use crate::service_fakes::{FakeProject, project_info}; use dozer_services::{ServiceError, ServiceErrorKind}; use std::sync::{Arc, mpsc};`;测试用普通 `#[test]` + 多线程 `runtime()`,`rt.block_on(...)` 驱动 async 函数):

```rust
    fn project(fake: &FakeProject) -> Arc<dyn dozer_services::ProjectService> {
        fake.clone().into_services().project
    }

    #[test]
    fn create_local_registers_the_target_and_returns_recent() {
        let rt = runtime();
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("demo");
        let info = project_info(5, &target.to_string_lossy());
        let fake = FakeProject::default()
            .with_open(Ok(Some(info.clone())))
            .with_list_projects(Ok(vec![info.clone()]));
        let got = rt.block_on(spawn_create_local(target.clone(), "说明".into(), true, &project(&fake)));
        assert_eq!(got, Ok((Some(info.clone()), vec![info], false)), "create_git=true → skip_git_init=false");
        assert!(target.is_dir(), "先建目录再登记");
        assert_eq!(
            fake.calls(),
            vec![format!("open_project({})", target.to_string_lossy()), "list_projects()".to_string()]
        );

        let target2 = root.path().join("demo2");
        let fake = FakeProject::default();
        let got = rt.block_on(spawn_create_local(target2, String::new(), false, &project(&fake)));
        assert_eq!(got, Ok((None, vec![], true)), "create_git=false → skip_git_init=true");
    }

    #[test]
    fn create_local_open_failure_has_the_legacy_text_and_skips_the_list() {
        let rt = runtime();
        let root = tempfile::tempdir().unwrap();
        let fake = FakeProject::default()
            .with_open(Err(ServiceError::new(ServiceErrorKind::Rejected, "daemon 错误: boom")));
        let got = rt.block_on(spawn_create_local(root.path().join("demo"), String::new(), true, &project(&fake)));
        assert_eq!(got, Err("注册项目失败: daemon 错误: boom".to_string()));
        assert_eq!(fake.calls().len(), 1, "open 失败后不再读最近项目");
    }

    #[test]
    fn create_local_still_succeeds_with_empty_recent_when_the_list_fails() {
        let rt = runtime();
        let root = tempfile::tempdir().unwrap();
        let info = project_info(5, "/p/demo");
        let fake = FakeProject::default()
            .with_open(Ok(Some(info.clone())))
            .with_list_projects(Err(ServiceError::new(ServiceErrorKind::Unavailable, "daemon 断开")));
        let got = rt.block_on(spawn_create_local(root.path().join("demo"), String::new(), true, &project(&fake)));
        assert_eq!(got, Ok((Some(info), vec![], false)), "最近项目读取失败:当空,整体仍成功");
    }

    #[test]
    fn clone_registers_the_cloned_directory() {
        if !bytegit::git_available() {
            return; // 无系统 git(bytegit::clone 内部调用 git 可执行文件),跳过
        }
        let rt = runtime();
        let remote = bytegit::testutil::TempRepo::new();
        remote.commit_file("a.txt", "x", "c1");
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("cloned");
        let info = project_info(6, &target.to_string_lossy());
        let fake = FakeProject::default()
            .with_open(Ok(Some(info.clone())))
            .with_list_projects(Ok(vec![info.clone()]));
        let got = rt.block_on(spawn_clone(
            remote.path().to_string_lossy().into_owned(),
            target.clone(),
            "说明".into(),
            &project(&fake),
        ));
        assert_eq!(got, Ok((Some(info.clone()), vec![info], false)), "克隆自带 .git,skip_git_init 恒 false");
        assert!(target.join(".git").exists());
        assert_eq!(fake.calls()[0], format!("open_project({})", target.to_string_lossy()));
    }

    #[test]
    fn submit_local_validation_failure_makes_no_service_call() {
        let rt = runtime();
        let fake = FakeProject::default();
        let mut state = Some(State {
            local: LocalForm { root_dir: "/tmp".into(), name: "bad/name".into(), ..Default::default() },
            ..Default::default()
        });
        let (tx, rx) = mpsc::channel();
        update(&mut state, Message::SubmitLocal, project(&fake), rt.handle(), move |m| { let _ = tx.send(m); });
        assert!(state.as_ref().unwrap().error.is_some());
        assert!(!state.as_ref().unwrap().busy);
        assert!(rx.recv_timeout(std::time::Duration::from_millis(300)).is_err(), "校验失败不 spawn");
        assert!(fake.calls().is_empty());
    }

    #[test]
    fn submit_local_end_to_end_delivers_done() {
        let rt = runtime();
        let root = tempfile::tempdir().unwrap();
        let info = project_info(5, "/p/demo");
        let fake = FakeProject::default().with_open(Ok(Some(info.clone())));
        let mut state = Some(State {
            local: LocalForm {
                root_dir: root.path().to_string_lossy().into_owned(),
                name: "demo".into(),
                ..Default::default()
            },
            ..Default::default()
        });
        let (tx, rx) = mpsc::channel();
        update(&mut state, Message::SubmitLocal, project(&fake), rt.handle(), move |m| { let _ = tx.send(m); });
        assert!(state.as_ref().unwrap().busy);
        match rx.recv_timeout(TIMEOUT).unwrap() {
            Message::Done(Ok((opened, _, skip))) => assert_eq!((opened, skip), (Some(info), false)),
            other => panic!("unexpected: {other:?}"),
        }
    }
```
(远程仓库的建法与 `usage_probe/scenarios.rs` 的 `repo_with_one_commit()` 一致:`TempRepo::new()` 加 `commit_file("a.txt", "x", "c1")`。`submit_local_end_to_end_delivers_done` 里 `skip` 取 `!create_git`;执行时先读 `LocalForm::default()` 的 `create_git` 默认值,若为 `true` 则断言 `skip == false`,若为 `false` 则 `skip == true`,按实际默认值写死,不要两个都接受。)

Run: `cargo test -p dozer-app extensions::project_create 2>&1 | grep -E '^error' | head -5` → Expected: 编译失败(`spawn_create_local`/`spawn_clone`/`update` 的参数类型不符)(RED)。

- [ ] **Step 2: 对话框侧实现**

`project_create.rs`:
1. 顶部加 `use dozer_services::ProjectService; use std::sync::Arc;` 与 `dozer_core::scope!(LOG, module, "project_create");`。
2. `update` 的 `client: &dozer_client::Client` → `project: Arc<dyn ProjectService>`;`SubmitLocal`/`SubmitClone` 里去掉 `let client = client.clone();`,spawn 体改为:
```rust
            handle.spawn(async move {
                let result = spawn_create_local(target, description, create_git, &project).await;
                emit(Message::Done(result));
            });
```
(`SubmitClone` 同理调用 `spawn_clone(url, target, description, &project)`。两个 arm 各自只移动一次 `project`。)
3. `spawn_create_local`/`spawn_clone` 的 `client: &dozer_client::Client` → `project: &Arc<dyn ProjectService>`;两处登记段改为:
```rust
    let path_s = target.to_string_lossy().into_owned();
    let opened = project
        .open_project(path_s)
        .await
        .map_err(|e| format!("注册项目失败: {e}"))?;
    let recent = match project.list_projects().await {
        Ok(v) => v,
        Err(e) => {
            dozer_core::log_warn!(LOG, error = %e, "创建项目后读取最近项目失败,最近项目当空");
            Vec::new()
        }
    };
```
4. 测试注释里"不需要真的构造 `dozer_client::Client`"的说法改为"不需要真的跑 `project`/`handle`"(两处:约 1334 行与 1421 行)。

- [ ] **Step 3: host 侧实现**

`app/update.rs` 的 `Message::ProjectCreate(msg)` 转发(约 3118–3130 行):
```rust
            Message::ProjectCreate(msg) => {
                let project = crate::panel_host::services_for(&self.client).project;
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let emit = move |m| {
                    let _ = proxy.send_event(Message::ProjectCreate(m));
                };
                project_create::update(&mut self.project_create, msg, project, &handle, emit);
            }
```
去掉 `service_fakes.rs` 里 Task 3 临时加的 `#[allow(dead_code)]`。

- [ ] **Step 4: 运行对话框测试**

```bash
cargo test -p dozer-app extensions::project_create 2>&1 | grep -E 'test result|FAILED|^error' -A6 | head -20
```
Expected: 全绿(含 6 个新测试;克隆测试在无系统 git 时直接返回);既有纯状态测试保持通过。

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
Expected: 失败仅已知的(并行负载下的间歇失败——若出现,单独重跑该测试三次确认);clippy 警告数 ≤ 基线;门禁无新增回退;log scope ok;`app/state.rs` 的来源名测试通过。
```bash
git checkout Cargo.lock 2>/dev/null; git add crates/dozer-app/src && git status --short
git commit -m "feat(project_create): ProjectCreate S4 创建/克隆对话框改用 ProjectService

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 5: `R-CLIENT` 基线下降、验收与文档回填

**Files:**
- Modify: `scripts/audit/panel-boundary.baseline.json`(仅 `extensions/project_create.rs` 的 `R-CLIENT` 条目)
- Modify: `docs/dozer-v2/bytehost-H0/08-client-use-inventory.md`、`CLAUDE.md`

**Interfaces:**
- Produces: `extensions/project_create.rs` 不再出现在 `R-CLIENT` 基线里。

- [ ] **Step 1: 确认门禁现状并下调基线**

```bash
grep -n 'dozer_client\|io\.client()' crates/dozer-app/src/extensions/project_create.rs | head -3   # 期望无输出(注释里的提及也要清掉,门禁排除注释,但保持干净)
python3 scripts/audit/check_panel_boundary.py   # 此时 project_create.rs 的 R-CLIENT 实测应为 0(低于基线),门禁只显示既有 R-HOST-EXECUTORS
python3 - <<'EOF'
import json
p = "scripts/audit/panel-boundary.baseline.json"
base = json.load(open(p, encoding="utf-8"))
rel = "extensions/project_create.rs"
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
Expected: 基线 diff 只含 `extensions/project_create.rs` 条目的删除;门禁仍只报 `R-HOST-EXECUTORS 0 -> 1`。

- [ ] **Step 2: 验收清单(逐条跑、读输出)**

```bash
cargo tree -p dozer-services --edges normal                    # 1: 仅 dozer-model + serde 系
python3 scripts/audit/unit_edges.py ext:project_create         # 2: 无 dozer_client
cargo test -p dozer-app extensions::project_create 2>&1 | grep -E 'test result|FAILED'   # 3
cargo test -p dozer-core --test model_golden 2>&1 | grep 'test result'                    # 5
W=$(ls -d .superpowers/sdd/*/ | head -1)
cargo test --workspace --no-fail-fast > ${W}final.txt 2>&1; grep -E '^test result' ${W}final.txt | awk '{s+=$4;f+=$6} END{print s,f}'; grep -E '^test .* FAILED' ${W}final.txt   # 4
cargo clippy --all-targets 2>&1 | grep -c '^warning'; cargo fmt --check && echo fmt-ok
bash scripts/check-log-scope.sh && echo log-scope-ok
git status --short                                             # 无 Info.plist、无无关 Cargo 改动
```
任一不满足即不收尾,回到对应任务修。

- [ ] **Step 3: 文档回填并提交**

- `docs/dozer-v2/bytehost-H0/08-client-use-inventory.md` 末尾追加一节"ProjectCreate 迁移(2026-10-11)":`extensions/project_create.rs`(创建/克隆项目对话框)经 `ProjectService`(新增 `open_project`/`list_projects` 两个方法)登记项目;`R-CLIENT` 基线中 `project_create.rs` 已清零;用户可见行为零变化(`注册项目失败: …` 文案不变);`list_projects` 失败仍当空列表但改写 warn 日志(来源 `project_create`)。**登记后续项**:`list_projects` 失败时 host 的 `project_tab_opened` 会把工作区最近项目设为空(可考虑失败时保留旧列表);只剩 `settings` 一个面板还直接用 `Client`(Advanced 页的 `list`/`shutdown_daemon`/`ensure_daemon`,以及走 bytehost 非 dyn 兼容的 `AppHostApi` 的「应用」页)。
- `CLAUDE.md` Workspace 布局表 `dozer-services` 行里"…EditHistory、Browser 已迁,其余面板待迁"改为"…EditHistory、Browser、ProjectCreate 已迁,其余面板待迁"。
```bash
git add scripts/audit docs CLAUDE.md
git commit -m "chore(audit): ProjectCreate S5 R-CLIENT 基线下调 + 文档回填

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

之后按 executing-plans 的 Final Review(整分支 fresh reviewer)与 finishing-a-development-branch;**合并进 main 属于需先询问用户的外部副作用**,合并前核对 main 上用户 WIP(`Cargo.lock`、`crates/dozer-app/Cargo.toml`、`Info.plist` 等)与本分支的重叠,用 `git stash` → 合并 → `git stash pop`。
