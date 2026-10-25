# 交接任务 8:S3b-2 Digger MCP 待办工具(S6e-3b)

> 给开发 agent 的任务说明,写于 2026-10-25。S3b-1(待办面板)已合并到 Digger `main`(`092c4f8`)。本任务只在 Digger 仓库里把 `digger-mcp` 的待办工具打开并补测试。**范围是计划的 Task 1–2 与 Task 3 的 Step 1–2(写清单、跑门禁),然后停下。** 发布(Task 3 Step 4)与文档状态行(Step 5)都不在本任务内。

**目标:** Digger 里的 agent 通过 MCP 能读写当前项目的待办(`list_todos` / `add_todo` / `toggle_todo` / `edit_todo_text`),改动在待办面板里一个轮询周期(1 秒)内可见。

**先读(只读):**
1. 计划(照这份做,每个任务先写失败测试):`/Users/chrischiang/Projects/CoralProjects/byteboy/dozer/docs/superpowers/plans/2026-10-25-digger-todo-s3b2-mcp-and-release.md`。Global Constraints 与 Review Focus 是硬要求。
2. 设计稿:同目录上级 `specs/2026-10-25-digger-todo-panel-s3b-design.md`(§7 Q5)、`specs/2026-10-18-agent-mcp-s6e-design.md` §2 与 §6.3。
3. `crates/digger-mcp/src/main.rs`、`tests/{tool_set,memory_roundtrip}.rs`:`McpFeatures` 现在 `todos: false`;`memory_roundtrip.rs` 里有起隔离 `diggerd` 并直接调工具函数(`DozerMcpServer::with_options` + `Parameters(..)`)的现成做法。
4. 工具实现(只读,在 Dozer 仓库):`crates/dozer-mcp/src/{server.rs,features.rs}`。`tool_names()` 的分组顺序是 memory → summary → todos → preview → file_edit,测试里的期望顺序以它为准。
5. S3b-1 的 `crates/desktop/src/adapters/todo.rs`(`TodoPanel` 的 `set_project` / `activate` / `poll` / `update` / `items`)和它的集成测试 `crates/desktop/tests/todo_panel_daemon.rs`:Task 2 照它的写法。

## 基线

- Digger 仓库 `/Users/chrischiang/Projects/CoralProjects/byteboy/digger`,基线 `main` 的 `092c4f8`(开工先 `git rev-parse main` 核对;若之后又有提交,先看 `git log`)。
- 新建 worktree `../digger-mcp-todos`,分支 `feat/mcp-todos`,从 `main` 切出。
- **不要动主工作区。** 它有用户未提交的改动:13 个 S6 文档的删除、`.qoder/`。**不碰 `feat/preview-pairing` / `digger-p4`。**
- 依赖已钉 `libs-2026-10-24e`(含 `dozer-mcp`)。**不改 `dozer-mcp`、不改 `dozerd`、不改 Dozer。** 若发现必须改,停下报告,不要绕。
- 测试基线:S3b-1 合并后干净快照 `cargo test --workspace` 是 622 个通过、0 失败(S3b-1 报告数;**开工先自己在干净快照里重新跑一遍记数**)。已知偶发:`apps::tests::closing_during_startup_waits_and_releases_the_root` 在机器同时重编译时会失败,单独重跑稳定通过;遇到就单独重跑量化,**不要靠重跑到绿**。

## 与计划不同的地方(2026-10-25 核实后订正,计划里已同步改)

- **空白任务文字现状不被拒绝。** `dozer-mcp::add_todo` 直接转给 `Client::add_todo`,`dozerd` 的 `TodoStore::add` 也不校验,Dozer 里同样如此。所以 Task 1 的测试是 `blank_todo_text_is_accepted_today_like_dozer`(**钉住现状**:`add_todo("")` 成功),不是"被拒绝"。**不要**为此改 `dozer-mcp` / `dozerd`;在报告和验收清单里把它列为已知行为,是否收紧由用户另议。
- 其余测试照计划:`add_then_list_then_toggle_then_edit`(最后用独立 `Client` 读回,证明落进 `digger.db`)、`a_session_outside_any_registered_project_gets_a_readable_error`(会话未归属项目时返回人能读懂的错误,不 panic,不写到别的项目;现状错误文本是"会话尚未归属任何项目"一类)、`todo_ids_are_not_scoped_to_the_session_project`(钉住现状:项目 A 的会话能改项目 B 的任务)。
- `tool_set.rs` 把 `todo_and_summary_tools_stay_off` 改成 `only_the_summary_tool_stays_off`;`submit_session_summary` 仍然关(`summary: false`)。

## 容易踩的坑

- **Digger 会话里同时配了 `dozer` 与 `digger` 两条 MCP**(S6e 设计 §2):`dozer-mcp serve` 缺 `DOZER_SESSION_ID` 会报错退出,agent 的 MCP 状态里 `dozer` 可能一直显示失败。这是已知并存现象,与待办工具无关;写进验收清单,别当成你的 bug 去修。
- **`memory_roundtrip.rs` 里复制了一份 `McpFeatures`**,与入口要保持一致(改动小的方式二选一:同步改,或抽成共享辅助)。漏改会让一个测试仍然按旧开关跑。
- **已经开着的 Agent 会话看不到新工具**(agent 只在会话启动时读一次工具清单):验收清单要写明升级后要**重开**会话。
- **Task 2 依赖 S3b-1 的轮询**:测试里用 `TodoPanel::poll(now)` 加上回流消息喂 `update`,不要在测试里 sleep 真等一秒。若测试失败,说明 S3b-1 的轮询有缺口——**停下报告**,不要在本任务里绕开。
- 测试的 `diggerd` 一律独立 `DIGGER_DATA_DIR` + 临时 `HOME`;`CARGO_TARGET_DIR` 不要与别的工作区共享。

## 约束

- 只改 `digger-mcp` 入口的特性开关与测试、新增 `desktop` 的一个集成测试和验收清单。库不改、Dozer 不改、`dozerd` 不改。
- **不碰真实数据,不碰用户常驻的 `diggerd`,不碰沙箱 `/private/tmp/digger-sandbox`,不碰 `/Applications` 里正在运行的应用。**
- `Cargo.lock` 里本地 `[patch]` 造成的改动不提交;commit 前看 staged 全貌(`git diff --cached --stat`);提交信息末尾带 `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`。
- **不合并 `main`、不打 tag、不推送、不发布、不安装、不改版本号。**
- **不在本任务内:** 发布 Digger 新版本(计划 Task 3 Step 4,逐项由用户确认)、各设计稿状态行的更新(Step 5,等真机验收后)、`submit_session_summary`、收紧 `toggle_todo` / `edit_todo_text` 的项目范围。

## 交付

- Task 1、Task 2 各自一个提交;Task 3 Step 1 的验收清单 `docs/superpowers/reviews/2026-10-25-todo-s3b2-acceptance.md`(计划里的 8 条,外加"空白任务文字可被新增"这条已知行为)一个提交。
- 完工前 `git archive HEAD` 解到独立临时目录(**目录名不带 "digger"**),在其自己的 `target` 下跑:
  - `cargo fmt --check`
  - `cargo clippy --workspace --all-targets -- -D warnings`(只允许 `block v0.1.6` 那条未来兼容性提示)
  - `bash scripts/check-shared-deps.sh && bash scripts/check-host-deps.sh && bash scripts/check-host-deps.sh --self-test && bash scripts/check-shell-regions.sh`
  - `cargo test --workspace --no-fail-fast`
  - `cargo metadata --locked --offline`
  - `bash scripts/release.sh --dry-run`(只跑门禁,不改文件不打包)
- 报告写到 `docs/superpowers/reviews/2026-10-<日期>-todo-s3b2-report.md`:测试总数(基线 + 新增)及核对算式、每个 Review Focus 对应的测试名、与计划不一致的地方及原因、每条"通过"附命令与退出码、已知行为清单(空白任务、跨项目 id、`dozer` MCP 并存失败、旧会话看不到新工具)。
- 停在 `feat/mcp-todos`。真机清单由用户在**新装的版本**里执行,所以发布排在用户确认之后,不属于你。
