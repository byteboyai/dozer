# ProjectCreate 对话框迁移到服务 trait(第十个面板)

> 日期:2026-10-11。状态:待用户审阅。依据:`2026-10-10-project-services-design.md`(`ProjectService` 的来历与口径)、`2026-10-11-groupchat-services-design.md`/`2026-10-11-context-services-design.md`(失败本来就可见的纯接缝迁移先例)、`docs/dozer-v2/bytehost-H0/08-client-use-inventory.md`。

## 1. 已对齐的理解

**目标(沿用前几个面板):** 创建/克隆项目对话框(`extensions/project_create.rs`)可进程内测试/复用——登记项目只依赖小 trait,测试注入内存实现。
**范围:** 只迁该文件里的 2 个服务调用:`open_project`、`list_projects`(都在 `spawn_create_local`/`spawn_clone` 里),以及 `update` 往下传的 `&Client`,和 host 的 1 个调用点(`app/update.rs` 里 `Message::ProjectCreate(msg)` 的转发)。
**明确不在范围:** `settings`(下一轮单独做);host 的 `project_tab_opened` 语义;`bytegit::clone`、`std::fs`、`project_meta::write_description` 等本地副作用(不是 dozerd 服务)。
**不变量:** 线上协议零变化;**用户可见行为零变化**——`open_project` 失败已可见,文案 `"注册项目失败: {e}"` 逐字不变;`Done` 的结果形状 `(Option<ProjectInfo>, Vec<ProjectInfo>, bool)` 不变。

**用户已确认的口径:**
- `open_project`/`list_projects` **并入已有的 `ProjectService`**(它们本来就是项目登记表的操作,与 `rename_project`/`remove_project` 同属一个域),不新增 trait。
- `list_projects` 失败时保持现行为(最近项目当空列表),但**不再无声吞掉**,改写一条 warn 日志;这不是用户可见的行为变化。

## 2. 现状(读代码得到)

- `update(state: &mut Option<State>, msg, client: &dozer_client::Client, handle, emit)`:`SubmitLocal`/`SubmitClone` 先做表单校验(失败写 `state.error` 后 return,不 spawn),通过后 `client.clone()` 并 `handle.spawn` 调 `spawn_create_local(target, description, create_git, &client)` / `spawn_clone(url, target, description, &client)`,结果 `Message::Done(SubmitResult)`。
- `spawn_create_local`:`create_dir_all` + `write_description`(都在 `spawn_blocking`)→ `client.open_project(&path)`(失败 `"注册项目失败: {e}"`)→ `client.list_projects().await.unwrap_or_default()` → `Ok((opened, recent, !create_git))`。
- `spawn_clone`:`bytegit::clone`(`spawn_blocking`)→ `write_description` → 同样的 `open_project`/`list_projects` → `Ok((opened, recent, false))`(克隆下来的仓库天然带 `.git`,不需要 `skip_git_init`)。
- `Done` 的处理:成功分支 host 拦截(`app/update.rs`:关对话框并 `project_tab_opened(project, recent, skip_git_init)`,里面会把工作区的 `recent_projects` 设为 `recent`);失败分支落进 `project_create::update`,填 `State::error` 并保持对话框打开。
- `Client::open_project(&str) -> Result<Option<ProjectInfo>>`、`Client::list_projects() -> Result<Vec<ProjectInfo>>`(inherent)。
- 日志:本文件未声明 `LOG`(`PanelKind` 无 `ProjectCreate`,它是弹窗而非面板;日志来源名在计划阶段按 `app/state.rs` 的来源名测试确认用 `module` 来源 `project_create`)。
- `R-CLIENT` 基线:`extensions/project_create.rs` 为 1(`update` 的 `&dozer_client::Client` 参数类型;另有两个 `spawn_*` 的同名参数类型也算同一个文件内的引用,以脚本实测为准)。

## 3. 设计

### 3.1 `ProjectService` 增加两个方法(`dozer-services`,仍只依赖 `dozer-model`)

| 方法 | 返回 |
|---|---|
| `open_project(path: String)` | `Option<ProjectInfo>` |
| `list_projects()` | `Vec<ProjectInfo>` |

参数按值传,返回 `BoxFuture<'_, Result<_, ServiceError>>`;`ProjectInfo` 已在 `dozer-model`。`Client` 实现这两个方法(调用同名 inherent 方法,`map_err(into_service_error)`);`FakeProject` 补两个方法(预设返回值、可让其失败、记录调用)。**`Services` 的字段不变**(`project` 已存在)。

### 3.2 对话框与 host

- `spawn_create_local`/`spawn_clone`/`update` 的 `client: &dozer_client::Client` 改为 `project: Arc<dyn ProjectService>`(`update` 里 `project.clone()` 给 spawn 的闭包;与 `submit_name_edit` 的先例同形);
- `open_project`:`project.open_project(path_s).await.map_err(|e| format!("注册项目失败: {e}"))?`(`ServiceError` 的 `Display` 即 message,文案不变);
- `list_projects`:`match project.list_projects().await { Ok(v) => v, Err(e) => { log_warn!(…, error = %e, "创建项目后读取最近项目失败,最近项目当空"); Vec::new() } }`;
- host:`Message::ProjectCreate(msg)` 的转发把 `let client = self.client.clone();` 换成 `let project = crate::panel_host::services_for(&self.client).project;`。

## 4. 步骤(每步先写失败测试;每步一个提交)

- **P0 基线:** 记录测试数、clippy 警告数、`R-CLIENT` 中 `project_create.rs`(1)、门禁输出。
- **P1 `ProjectService` 加两个方法:** `dozer-services` 契约测试(扩展现有的 `MemProject`:`open_project`、`list_projects` 的内存实现;`tokio::spawn` Send 检查覆盖新方法;读失败可与空列表区分)。
- **P2 `Client` 实现:** 回放服务端适配测试(`open_project` 成功且 `Option` 原样带回(`Some`/`None` 各一例)、`list_projects` 成功、`HangUp → Unavailable "daemon 断开"`、`ErrorReply → Rejected "daemon 错误: boom"`)。
- **P3 假实现:** `FakeProject` 补 `open_project`/`list_projects`(`.with_open(Result<Option<ProjectInfo>, ServiceError>)`、`.with_list_projects(Result<Vec<ProjectInfo>, ServiceError>)`;调用记录 `"open_project(<path>)"`、`"list_projects()"`);`Services` 字段不变。
- **P4 对话框与 host 切换:** 对话框层测试(用 `FakeProject`,临时目录当目标路径):
  - `spawn_create_local`:成功得到 `Ok((opened, recent, !create_git))` 且 `open_project` 收到目标路径;`open_project` 失败得到 `Err("注册项目失败: {服务文本}")`;`list_projects` 失败仍成功且 `recent` 为空;目录与描述文件确实创建。
  - `spawn_clone`:用本地裸仓库当远程(`bytegit::clone` 对本地路径可用,计划阶段先验证 `git_available()`,不可用则跳过该测试),成功得到 `Ok((opened, recent, false))`;`open_project` 失败同上。
  - `update`:`SubmitLocal` 校验失败不 spawn(服务无调用);校验通过后 `Done` 到达。
  - 现有 `apply_field_message` 的纯状态测试不动;测试注释里"不需要构造 `dozer_client::Client`"的说法按新接口更新。
  - 探针 `host_site` 行号如因替换漂移,只改行号。
- **P5 门禁与验收:** `R-CLIENT` 基线中 `project_create.rs` 清零;回填 `08-client-use-inventory.md`、`CLAUDE.md` 的 `dozer-services` 行(…、Browser、ProjectCreate 已迁,其余面板待迁——只剩 `settings`)。

## 5. 验收标准(机器可验证)

1. `cargo tree -p dozer-services --edges normal` 仍只有 `dozer-model` 与 serde 系。
2. `extensions/project_create.rs` 不再含 `dozer_client` / `io.client()`;`R-CLIENT` 在该文件为 0,基线只降。
3. 创建/克隆成功路径、`open_project` 失败文案、`list_projects` 失败仍成功且 `recent` 为空,均有测试(P4)。
4. 全量测试数 ≥ 基线 + 新增;已知失败(`files::delete_confirm_spec_reflects_pending_target`、并行负载下的间歇失败、`R-HOST-EXECUTORS 0->1`)不算回归也不顺手修。
5. `R-HOST-PANEL-ARMS` 不上升;clippy 警告数不高于基线;`cargo fmt --check`、`scripts/check-log-scope.sh` 通过;`model_golden` 仍通过。

## 6. 明确不做

- 不改 `project_tab_opened` 的语义(例如 `list_projects` 失败时保留已有最近项目);不改 `Done` 的结果形状;不改表单校验。
- 不迁 `settings`;不改线上协议。
- 不把 `bytegit::clone`/文件系统副作用抽成服务(它们是本地副作用,不是 dozerd 服务)。

## 7. 风险与已知问题

- **`ProjectService` 是已经被 Project 面板使用的 trait:** 新增两个方法要求所有实现者(`Client`、`FakeProject`)同步补上;编译器会逐个指出,Project 面板的测试不应受影响。
- **克隆测试依赖系统 `git`:** `bytegit::clone` 内部调用 `git` 可执行文件(CLAUDE.md 里的有意选择);测试里用 `bytegit::git_available()` 守卫,不可用时跳过,避免在无 git 的环境里变红。
- **日志来源名:** 计划阶段按 `app/state.rs` 的来源名测试确认 `project_create` 作为 `module` 来源可用(该测试对面板名有强制,对非面板代码允许 `module` 来源)。
- **`Cargo.lock` 噪声:** main 上已提交的锁文件缺 bytegit/byteui 的 `source` 行,干净 worktree 里跑 cargo 会补上;提交前 `git checkout Cargo.lock`(同前几次)。
- **WIP 文件:** 用户 WIP(`Cargo.lock`、`crates/dozer-app/Cargo.toml`、`Info.plist`)在 main 工作区未提交;在独立 worktree 实施,不提交 `Info.plist` 与版本号改动。

## 8. 分支与执行

规格分支 `spec/project-create-services`(worktree `../dozer-pcreate-spec`);实施另开分支。执行计划由 writing-plans 产出,待本规格审阅通过后进行;合并进 main 前需询问用户。
