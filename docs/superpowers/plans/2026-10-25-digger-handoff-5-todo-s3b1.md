# 交接任务 5:S3b-1 Digger 待办面板接入

> 给开发 agent 的任务说明,写于 2026-10-25。库(`dozer-panel-todo`)和服务端(`diggerd` 的待办存储)都已就绪,本任务只做 Digger 桌面侧的接线。MCP 待办工具(S3b-2)与发布不在本任务内。

**目标:** Digger 出现与 Dozer 一致的待办面板(分类树 | 任务列表 webview + 新增输入),数据来自 `diggerd`,增删改查、排序、计划日期、状态、分类、指派 agent 可用,面板可见时每秒轮询刷新。

**设计与计划(只读,都在 Dozer 仓库 `/Users/chrischiang/Projects/CoralProjects/byteboy/dozer`):**
- `docs/superpowers/specs/2026-10-25-digger-todo-panel-s3b-design.md`:先读 §2 现状、§5.3 决定 D1–D6、§7 默认值。
- `docs/superpowers/plans/2026-10-25-digger-todo-s3b1-panel.md`:**照这份做**,6 个任务,每个任务先写失败测试。顶部的 Global Constraints 与 Review Focus 是硬要求。

## 基线

- Digger 仓库 `/Users/chrischiang/Projects/CoralProjects/byteboy/digger`,基线 `main` 的 `a9ff9ef`(开工先 `git rev-parse main` 核对)。
- 新建 worktree `../digger-todo-s3b1`,分支 `feat/todo-panel`,从 `a9ff9ef` 切出。
- **不要动主工作区。** 它有别人未提交的改动(13 个 S6 文档的删除、`.qoder/`)。
- **不碰 `feat/preview-pairing` / `digger-p4`。**
- 依赖已钉 `libs-2026-10-24e`,`Cargo.lock` 已提交。新增的 `dozer-panel-todo` 必须钉**同一个** tag;`bash scripts/check-shared-deps.sh` 是硬门禁。
- 测试基线:干净快照里 `cargo test --workspace` 588 个通过,0 失败;`cargo clippy --workspace --all-targets -- -D warnings` 只允许 `block v0.1.6` 那 1 条未来兼容性提示。
- 已知偶发:`apps::tests::closing_during_startup_waits_and_releases_the_root` 在机器同时跑别的重编译时失败过一次,单独重跑稳定通过。遇到就单独重跑量化(次数、失败数写进报告),**不要靠重跑到绿**,也不要去改它。

## 先读哪里

1. `crates/desktop/src/adapters/usage_native.rs`(266 行):你的 `todo_native.rs` 要照它的结构做(epoch、`thread_local VIEW`、`window::run`、自定义协议、`build.rs` 嵌入资源)。
2. `crates/desktop/src/adapters/usage.rs`:`activate` / `take_notices` / `list_view` / `content_view` 的写法。
3. `crates/desktop/src/adapters/project.rs`:`ProjectPanel::new` 里 `Services` 的构造(只有一项真实、其余 `UnavailableService`),以及"按 project id 存状态"的做法。
4. `crates/desktop/src/shell.rs` 的 `rules()` / `meta()` / `defaults()`,与 `product.rs` 里 `usage_*` 的全部接线点(`grep -n usage product.rs`):待办的接线逐点对应。
5. Dozer 的对照实现(只读):`dozer/crates/dozer-app/src/app/update.rs` 的 `todo_content_event`(约 3237)、`todo_assign_agent`(约 3096);`app/app.rs` 的 `poll_todo_if_visible`(约 1703)与 `todo_webview` 的推送(约 1310–1350)。
6. 库:`dozer/crates/dozer-panel-todo/src/{adapter,state,update,view,protocol}.rs`。**库不改。**

## 已裁决与默认值(设计稿 §7)

用户尚未逐条裁决,**按默认值做**;计划里这几处已经按默认写好,不要自行扩大范围:

1. 面板默认在**左区**,在 Project / Files 之后,配对规则同 Project / Files。
2. **不做详情弹窗。** 点卡片「打开详情」暂时无反应,只写一行 stderr。
3. **不开 `task_poller` / `summaries`。** 指派 agent 只写库里的归属字段,不触发执行。
4. **写失败没有 Toast**:落 stderr,列表刷新回原样,与 Project 面板一致。不要为此造提示机制。
5. MCP 待办工具不在本任务内。

## 容易踩的坑(都在计划的 Review Focus 里有测试,这里再强调)

- **资源重名**:用量与待办的入口页都叫 `host.html`。`build.rs` 拷到 `OUT_DIR` 时待办一律加 `todo-` 前缀,否则后拷的覆盖先拷的,其中一个面板白屏。
- **`thread_local` 陷阱**:`view()` 在主线程,订阅回调在 tokio 工作线程。订阅回调要读的状态一律用进程级原子;`thread_local` 只许放 wry 视图句柄,且只在 `window::run` 闭包里访问。(S6d 里键盘输入就因此整个失效过。)
- **webview 恒在 iced 之上**:内容区下方有 iced 的新增输入框、左栏有清空栏。webview 矩形必须用库导出的 `top_row_inner_h_px()` 与 `WorkspaceState::add_input_height()` 算,**不要手写像素常数**,并加几何测试。
- **`assign_todo_agent` 不在 `TodoService` trait 里**:它是 `Client` 的固有方法,`TodoPanel` 要同时持有 `Arc<dyn TodoService>` 和一个 `Client`。
- **库的 `Message` 变体要逐个有去向**:用穷尽 `match`(不写 `_`)让库将来新增变体时编译失败。
- **没有轮询就看不到 agent 改的任务**:`TodoPanel::poll` 与只在面板可见时订阅的定时器是 S3b-2 的前提。

## 约束

- 库不改、Dozer 不改、`diggerd` 不改。若发现必须改,**停下来报告**,不要在 Digger 里绕开,也不要自己去改 Dozer。
- 字体:UI 文本一律 `Font::default()`;只有代码编辑器和终端用 JetBrains Mono。
- 所有测试用隔离的 `DIGGER_DATA_DIR` 和临时 `HOME`;**不碰真实数据,不碰用户常驻的 `diggerd`**(用户正在用 `/Applications/Digger.app`,进程是别人的)。
- 沙箱 Digger 在 `/private/tmp/digger-sandbox`,是用户的验收环境,**不要启动、停止或改动它**;你自己的验证另建隔离目录。
- `Cargo.lock` 里本地 `[patch]` 造成的改动不提交;commit 前看 staged 全貌(`git diff --cached --stat`)。
- 提交信息末尾带 `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`。
- **不合并 `main`、不打 tag、不推送、不发布、不安装。**
- **不在本任务内:** MCP 待办工具(S3b-2)、详情弹窗、文本框右键菜单、语言切换、Dozer 与 Digger 的发布、真机清单的执行(那由用户做)。

## 交付

- 计划 6 个任务全部完成,每个任务各自一个提交。
- 完工前 `git archive HEAD` 解到独立临时目录(**目录名不带 "digger"**),在其自己的 `target` 下跑:
  - `cargo fmt --check`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `bash scripts/check-shared-deps.sh && bash scripts/check-host-deps.sh && bash scripts/check-host-deps.sh --self-test && bash scripts/check-shell-regions.sh`
  - `cargo test --workspace --no-fail-fast`
  - `cargo metadata --locked --offline`
- 报告写到 `docs/superpowers/reviews/2026-10-<日期>-todo-s3b1-report.md`:测试总数(基线 588 + 新增)、每个 Review Focus 对应的测试名、与计划不一致的地方及原因、每条"通过"附命令与退出码。
- 真机清单 `docs/superpowers/reviews/2026-10-25-todo-s3b1-acceptance.md` 按计划 Task 6 写好,留给用户在沙箱里勾。
- 停在 `feat/todo-panel`。
