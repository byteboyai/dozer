# S6e-3a 预览桥接(共享库抽取 + Digger 接入预览/定位/编辑工具) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 Digger 里的 Agent 能用 `get_preview_context`、`preview_navigate`、`locate_in_file`、`apply_precise_edit` 四个工具:Digger 的预览面板向 `diggerd` 推送预览上下文,并执行 Agent 发来的预览命令。

**Architecture:** 守护进程侧协议与 `digger-mcp` 工具实现都已存在(与 `dozerd`/`dozer-mcp` 同一份代码),缺的只是 GUI 侧。Dozer 把"上下文构造、模式 token、命令目标解析"写在 `dozer-app` 里(`pub(crate)`),Digger 用不了。**先把这三块纯函数抽进 `dozer-panel-preview`**(Dozer 改引用、测试守恒、切新 `libs-*` tag),Digger 再接入:推送与轮询两个循环依赖各自产品的 `dozer_client::Client`,不进共享库(`R-CLIENT` 门禁:面板库不得依赖 `dozer_client`)。

**Tech Stack:** Rust、iced 0.14、`dozer-panel-preview`、`dozer-model`(`PreviewContext`/`PreviewCommand`)、`dozer_client::Client`、tokio。

**Spec:** `docs/superpowers/specs/2026-10-18-agent-mcp-s6e-design.md` §6.3(S6e-3:预览/定位/编辑随预览引擎接入 Digger、且 Digger 的 GUI 开始向 `diggerd` 推送预览上下文并执行 `PreviewCommand` 之后打开 `preview` / `file_edit`;届时只改 `digger-mcp` 入口的特性开关与 GUI 侧,不再动 `dozer-mcp`)。**待办工具(`todos`)不在本计划**:Digger 没有待办面板。

## Global Constraints

- **Dozer 行为字节不变。** Task 1–2 是纯搬运:被搬的函数体不改;测试名守恒(不删、不改名),workspace 测试总数 = 基线 + 本计划新增。
- **库边界:** `dozer-panel-preview` 只依赖 `dozer-model`/`dozer-services`,**不得**引入 `dozer_client`、`dozer_core`、tokio 任务循环。
- **库里新增函数都是纯函数**,不读时钟(`now_ms` 由调用方传入)、不碰 IO。
- **Digger 一次只持有当前项目的预览页签**(`PreviewHub` 切项目即丢旧页签);非当前项目的命令一律回 `NotFound`,不挂起、不 panic。
- **核心原则:** Dozer/Digger 方便用户预览 AI 的工作结果,AI 来改产物。本计划打开的 `file_edit` 是"AI 编辑",不新增用户直接编辑入口。
- **日志:** Dozer 侧一律 `dozer_core::log_*!`;Digger 侧沿用现有 `eprintln! // cli-output` 做法或其现有日志设施,**不写终端输入字节**。
- **依赖版本:** Digger 钉的 `bytehost`/`byteui`/`bytegit` 与新 `libs-*` tag 内钉的版本逐个核对一致(现为 `v0.4.11` / `v0.4.5` / `v0.8.1`)。
- **对外动作先问用户:** 切 `libs-*` tag、推送、合并 `main`、发布。

## Review Focus

1. **命令的目标项目不是 Digger 当前前台项目:** 期望回 `PreviewCommandOutcome::NotFound`(带原因),MCP 调用方立刻得到结果而不是等到超时。→ Task 6。
2. **目标页签是 `Suspended` 壳(只恢复了页签、未加载):** 期望先物化再执行,不回"找不到"。→ Task 6。
3. **`diggerd` 不可用或连接中断时轮询与推送:** 期望不 panic、不忙等(失败后退避),恢复后自动继续;推送失败只记日志。→ Task 5、6。
4. **活动页签不是文本/表格(图片、webview 渲染页)或页签被关光:** 期望推 `None`,且在关闭/切换的当下立即推(不等防抖),Agent 不会读到上一个文件的旧上下文。→ Task 1、5。
5. **同一路径同时在 Files 与 Project 两个面板打开:** 期望按路径定位时优先 Files(与 Dozer 一致),`Tab` 目标按显式面板名走。→ Task 1、6。

---

## 分支与环境

- **Part A(Dozer):** 分支 `feat/preview-bridge-lib`,从 Dozer `main` 当前顶端切出,独立 worktree `../dozer-preview-bridge`。**不合并 `main`、不打 tag、不推送**,验收后再由用户决定。
- **Part B(Digger):** 依赖新 tag,**Task 4 起要等 Part A 的 tag 存在**。Part B 之前可以先在 Digger 用不提交的本地 `[patch]`(照 `scripts/dev-patch.example.toml`)联调,但**不得提交 patch 与指向它的 `Cargo.lock`**。分支 `feat/preview-bridge`,worktree `../digger-preview-bridge`,从 Digger `main` 切出。
- 不碰任一仓库主工作区里别人未提交的改动。回归一律用隔离的数据目录与临时 `HOME`。
- 完工前 `git archive HEAD` 解到**独立临时目录(路径不含 "digger")**,在其自己的 `target` 下重跑全量,跑完删掉该目录(磁盘紧张)。

---

## Part A — Dozer:抽进共享库

### Task 1: `dozer-panel-preview` 新增 `agent_bridge` 模块

**Files:**
- Create: `crates/dozer-panel-preview/src/agent_bridge.rs`
- Modify: `crates/dozer-panel-preview/src/lib.rs`(`pub mod agent_bridge;`,并 `pub use agent_bridge::*;`)
- Test: `crates/dozer-panel-preview/src/agent_bridge.rs` 内 `#[cfg(test)] mod tests`

**Interfaces:**
- Produces(全部 `pub`,纯函数):
  - `preview_context_from_web_state(path: &str, selection: Option<TextRange>, selected_text: Option<String>, viewport: Option<(u32, u32)>, revision: u64, now_ms: u64) -> dozer_model::PreviewContext`:**原样搬自** `crates/dozer-app/src/workspace/hook.rs` 的同名函数。
  - `mode_token(mode: PreviewMode) -> &'static str`:原样搬自同文件。
  - `build_preview_context(pane: &PreviewPane, now_ms: u64) -> Option<dozer_model::PreviewContext>`:取 `pane.tabs()[pane.active_idx()]`;`TabKind::File(path)` 才有上下文;`tab.uses_editor_host()` 走 `preview_context_from_web_state`(用 `tab.web_selection`/`web_selected_text`/`web_viewport`/`web_revision`,`read_only = tab.backend_read_only()`);否则若 `tab.tabular_view()` 有值则构造 `PreviewTabularContext`(`sheet`/`scroll_row`/`scroll_col`/`selected_cell`/`selected_range`,逻辑与 `crates/dozer-app/src/workspace/state.rs` `enqueue_preview_context_push` 里的块一致);其余返回 `None`;最后写 `ctx.mode = tab.current_mode().map(mode_token)`、`ctx.read_only`。
  - `resolve_command_target(files: &PreviewPane, project: &PreviewPane, cmd: &dozer_model::PreviewCommand) -> Option<(ViewerPanel, usize)>`:搬自 `Workspace::resolve_preview_command_target`。`PreviewCommandTarget::Tab { panel, tab_id }` → `panel == "project"` 取 `ViewerPanel::Project`,否则 `ViewerPanel::Files`;`Path { path }` → 先在 `files` 里找 `TabKind::File(p) == path` 的页签,找不到再找 `project`;都没有 `None`。
- Consumes: `PreviewPane::{tabs, active_idx}`(已有)、`PreviewTab::{uses_editor_host, tabular_view, backend_read_only, current_mode, web_*}`(已有)、`ViewerPanel`(`dozer_preview_core`,已 `pub use`)。

- [ ] **Step 1: 先把测试搬过来(此时编译失败)。** 把 `crates/dozer-app/src/workspace/tests.rs` 里的 `preview_context_from_web_state_maps_selection_and_viewport`、`preview_context_from_web_state_cursor_only_and_missing_viewport`、`mode_token_covers_all_modes` 三条**原样复制**进 `agent_bridge.rs` 的测试模块(路径引用改成 `crate::`)。再新增:
  - `build_preview_context_is_none_for_blank_and_for_non_text_tabs`:只有 Blank 页签的 pane → `None`;
  - `build_preview_context_for_a_code_tab_carries_selection_mode_and_read_only`:用现有测试夹具造一个 `uses_editor_host` 页签,设 `web_selection`/`web_selected_text`/`web_revision`,断言 `ctx.path`、选区 1-based、`ctx.mode == Some("code")`、`ctx.read_only` 等于 `backend_read_only()`;
  - `build_preview_context_for_a_tabular_tab_fills_the_tabular_block`:断言 `ctx.tabular` 有值且选中格/范围映射正确(单格 → `selected_cell`,多格 → `selected_range`);
  - `resolve_target_prefers_files_over_project_for_the_same_path`(**Review Focus 5**);
  - `resolve_target_for_an_explicit_tab_uses_the_named_panel`(`"project"` → Project,其它 → Files);
  - `resolve_target_returns_none_when_the_path_is_open_nowhere`。
- [ ] **Step 2: 运行确认失败。** `cargo test -p dozer-panel-preview agent_bridge` → 编译失败(函数未定义)。
- [ ] **Step 3: 实现上面四个函数。** 前两个直接搬代码;后两个按 Interfaces 的描述写,复用 Dozer 现有逻辑,**不改行为**。
- [ ] **Step 4: 运行确认通过。** `cargo test -p dozer-panel-preview` 全绿;`cargo clippy -p dozer-panel-preview --all-targets` 无新增警告。
- [ ] **Step 5: 提交** `feat(preview): 抽出 agent_bridge(预览上下文、模式 token、命令目标解析)`。

### Task 2: Dozer 改引用共享库并删掉原实现

**Files:**
- Modify: `crates/dozer-app/src/workspace/hook.rs`(删 `preview_context_from_web_state`、`mode_token`)、`crates/dozer-app/src/workspace/state.rs`(`enqueue_preview_context_push` 里的构造块改调 `dozer_panel_preview::build_preview_context`;`resolve_preview_command_target` 改为调 `resolve_command_target` 再把 `ViewerPanel` 映射成 `PanelKind`)、`crates/dozer-app/src/workspace/tests.rs`(删已搬走的 3 条测试)
- Test: `crates/dozer-app/src/workspace/tests.rs` 里 `resolve_preview_command_target` 相关现有测试**保持不动并必须仍然通过**

**Interfaces:**
- Consumes: Task 1 的四个函数。
- Produces: `Workspace::resolve_preview_command_target(&self, cmd: &PreviewCommand) -> Option<(PanelKind, usize)>` 签名不变。

- [ ] **Step 1: 先跑基线并记下数字。** `cargo test --workspace --no-fail-fast`,记下总数 N(此步在改动前做)。
- [ ] **Step 2: 改引用并删原实现。** `enqueue_preview_context_push` 只保留"项目为空返回 / 算 `now_ms` / nonce 防抖 / 异步推送"这几段,把中间"从活动页签算 `PreviewContext`"换成 `dozer_panel_preview::build_preview_context(&self.preview, now_ms)`。`resolve_preview_command_target` 调 `resolve_command_target(&self.preview, &self.project_preview, cmd)` 后把 `ViewerPanel::Files/Project` 映射成 `PanelKind::Files/Project`。
- [ ] **Step 3: 验证行为不变。** `cargo test --workspace --no-fail-fast`:总数 = N + 6(`dozer-app` 里少 3 条、`dozer-panel-preview` 里同名多 3 条,二者抵消;再加 Task 1 新增的 6 条),`--list` 对账:被搬的 3 条名字在新位置存在、旧位置不存在;`cargo clippy --workspace --all-targets` 与基线相同;`cargo fmt --check`;`scripts/check-log-scope.sh`(若存在)通过。
- [ ] **Step 4: 提交** `refactor(app): 预览上下文与命令目标解析改用 dozer-panel-preview::agent_bridge`。

### Task 3: 门禁与新 tag(**对外动作,先问用户**)

- [ ] **Step 1: 门禁。** 干净快照里 `cargo metadata --locked`、`cargo test --workspace`、`cargo clippy --workspace --all-targets`;`R-CLIENT`/`R-PROTOCOL` 等现有门禁脚本全过。
- [ ] **Step 2: 报告。** 写 `docs/superpowers/reviews/2026-10-<日期>-s6e3a-parta-report.md`:基线 N、新增数、被搬测试的新旧位置、每条"通过"附命令与退出码。
- [ ] **Step 3: 停下等用户确认**,再合并 `main` 并切 `libs-<日期><字母>` tag(按 `libs-*` 现有命名;推送同样先问)。tag 里 `bytehost`/`byteui`/`bytegit` 版本记入报告,供 Task 4 核对。

---

## Part B — Digger:接入

### Task 4: 依赖钉到新 tag

**Files:** Modify `Cargo.toml`(所有 `dozer-*` 的 `tag`,一起改)、`Cargo.lock`

- [ ] **Step 1: 改钉版并更新锁。** `cargo update --workspace`,然后 `cargo metadata --locked`、`cargo tree -d`、`scripts/check-shared-deps.sh`、`scripts/check-host-deps.sh` 全过;核对 `bytehost`/`byteui`/`bytegit` 版本与新 tag 内钉的逐个一致。
- [ ] **Step 2: 全量。** `cargo test --workspace --no-fail-fast` 总数与基线一致(基线以开工时 `main` 实测为准)。
- [ ] **Step 3: 提交** `chore(deps): align digger with dozer libs <tag>`。

### Task 5: 预览上下文推送

**Files:**
- Create: `crates/desktop/src/adapters/preview_bridge.rs`
- Modify: `crates/desktop/src/adapters/mod.rs`、`crates/desktop/src/adapters/preview.rs`(暴露只读取活动面板 `PreviewPane` 的入口)、`crates/desktop/src/product.rs`(预览相关消息处理后触发推送)
- Test: `preview_bridge.rs` 内测试

**Interfaces:**
- Produces:
  - `PreviewBridge { client: dozer_client::Client, handle: tokio::runtime::Handle, nonce: Arc<AtomicU64> }`,`PreviewBridge::new(client, handle) -> Self`。
  - `PreviewBridge::push(&self, project_id: i64, context: Option<PreviewContext>, delay_ms: u64)`:`nonce` 自增后 spawn 任务,`sleep(delay_ms)` 后若 `nonce` 已被更晚的调用取代则放弃,否则 `client.update_preview_context(project_id, context)`;失败只记日志(**Review Focus 3**)。与 Dozer `enqueue_preview_context_push` 同一防抖语义:普通变化 250 ms,"切页签 / 失焦 / 关闭 / 切项目"立即(0 ms)。
  - `product.rs`:`fn sync_preview_context(&mut self, immediate: bool)`——取当前项目与其 Files 面板的 `PreviewPane`,调 `dozer_panel_preview::build_preview_context(pane, now_ms)`,再 `bridge.push`。**只推当前项目的 Files 面板**(与 Dozer 一致)。
- Consumes: Task 1 的 `build_preview_context`。

- [ ] **Step 1: 写失败测试:**
  - `push_debounces_so_only_the_last_context_is_sent`(假 `Client` 服务端夹具沿用 `daemon.rs` 的做法;连发 5 次 `push(.., 50ms)`,断言服务端只收到最后一次);
  - `an_immediate_push_supersedes_a_queued_debounced_one`;
  - `closing_the_last_text_tab_pushes_none_immediately`(**Review Focus 4**);
  - `a_failing_daemon_does_not_panic_and_a_later_push_still_works`(**Review Focus 3**:先让服务端拒绝再恢复)。
- [ ] **Step 2: 确认失败,实现,确认通过。**
- [ ] **Step 3: 接线。** 在 `product.rs` 中凡导致"活动页签 / 选区 / 视口 / 模式 / 项目"变化的预览消息处理之后调用 `sync_preview_context`:普通变化 `immediate = false`;`Msg::` 里的切页签、关页签、项目切换、预览面板失焦走 `immediate = true`。**写前先 grep `product.rs` 里 `Msg::Preview*` 的全部分支,逐个确认是否需要触发,把清单写进报告。**
- [ ] **Step 4: 验证。** `cargo test -p digger-desktop`;隔离环境起 Digger 开一个文本文件、选一段文字,用 `digger-mcp` 的 `get_preview_context`(或直接 UDS 请求 `GetPreviewContext`)读回,选区一致。
- [ ] **Step 5: 提交** `feat(preview): 向 diggerd 推送预览上下文`。

### Task 6: 预览命令轮询与执行

**Files:**
- Modify: `crates/desktop/src/adapters/preview.rs`(新增 `PreviewHub::apply_preview_command`)、`crates/desktop/src/adapters/preview_bridge.rs`(轮询器)、`crates/desktop/src/product.rs`(`Msg::PreviewCommandsFetched`)
- Test: `preview.rs`、`preview_bridge.rs`

**Interfaces:**
- Produces:
  - `PreviewHub::apply_preview_command(&mut self, project_id: i64, cmd: &PreviewCommand) -> PreviewCommandOutcome`:
    1. `project_id` 不是当前项目 → `PreviewCommandOutcome::NotFound { request_id, detail: "项目不在前台" }`(**Review Focus 1**);
    2. 用 `resolve_command_target(files_pane, project_pane, cmd)` 定位,`None` → `NotFound { detail: "找不到该路径对应的预览 tab" }`;
    3. 目标页签 `is_pending_load(tab_id)` → 先调库的 `dozer_panel_preview::load_preview_tab(&mut pane, ctx, tab_id, &mut fx)` 并执行产生的 effects(沿用 `PreviewHub::run` 的做法)(**Review Focus 2**);
    4. `pane.apply_preview_command(tab_id, cmd)` 返回终态。
  - `PreviewBridge::spawn_poller(&self, project: Arc<AtomicI64>, started: Arc<AtomicBool>, proxy: impl Fn(i64, Vec<PreviewCommand>) + Send + 'static)`:幂等(`started.swap`);每 400 ms 取 `project`(≤0 跳过)→ `client.take_pending_preview_commands(pid)`;失败时**退避**(连续失败翻倍,上限 5 s,成功即复位)(**Review Focus 3**);非空才回调。
  - `Msg::PreviewCommandsFetched(i64, Vec<PreviewCommand>)`:处理时逐条 `apply_preview_command`,结果用 `client.report_preview_command_outcome(outcome)` 在后台回报。
- Consumes: Task 1 的 `resolve_command_target`;库的 `load_preview_tab`、`PreviewPane::{is_pending_load, apply_preview_command}`。

- [ ] **Step 1: 写失败测试:**
  - `a_command_for_a_non_current_project_is_not_found`;
  - `a_command_for_an_unknown_path_is_not_found`;
  - `a_command_for_a_suspended_tab_materializes_it_before_executing`(用库的恢复壳夹具造一个 `Suspended` 页签,断言执行后不是 `NotFound` 且页签已加载);
  - `a_path_open_in_both_panels_is_executed_in_files`(**Review Focus 5**);
  - `a_stale_revision_edit_is_rejected_with_stale_revision`(沿用库里 `StaleRevision` 语义;经 Digger 入口再确认一次);
  - `the_poller_backs_off_while_the_daemon_is_unreachable_and_recovers`;
  - `the_poller_is_idempotent`(`started` 为真时再调不再起第二个循环)。
- [ ] **Step 2: 确认失败,实现,确认通过。**
- [ ] **Step 3: 启动接线。** `boot` 里在 `Daemon::ensure` 成功之后 `spawn_poller`;当前项目 id 在项目切换时写入那个 `AtomicI64`。
- [ ] **Step 4: 验证。** `cargo test -p digger-desktop`;隔离环境真机:用 `digger-mcp` 的 `preview_navigate` / `locate_in_file` 发命令,Digger 预览面板跳转并高亮。
- [ ] **Step 5: 提交** `feat(preview): 轮询并执行 Agent 发来的预览命令`。

### Task 7: 打开 MCP 特性开关与文档

**Files:**
- Modify: `crates/digger-mcp/src/main.rs`(`McpFeatures { memory: true, preview: true, file_edit: true, 其余 false }`)、`crates/digger-mcp/tests/tool_set.rs`、`docs/adr/ADR-015-product-owned-daemon.md`(MCP 一节更新)
- Test: `crates/digger-mcp/tests/tool_set.rs`

**Interfaces:** Consumes `dozer_mcp::McpFeatures` 的 `preview`、`file_edit`。

- [ ] **Step 1: 改测试(先失败):** 把 `no_todo_preview_edit_or_summary_tool_is_listed` 拆成:
  - `memory_preview_and_edit_tools_are_listed`:`tool_names()` 含 `write_memory`、`list_memories`、`get_memory`、`get_preview_context`、`preview_navigate`、`locate_in_file`、`apply_precise_edit`;
  - `no_todo_or_summary_tool_is_listed`:仍断言 `list_todos`、`add_todo`、`toggle_todo`、`edit_todo_text`、`submit_session_summary` 不出现。
- [ ] **Step 2: 运行确认失败。** `cargo test -p digger-mcp tool_set`。
- [ ] **Step 3: 改 `main.rs` 的开关,确认通过。**
- [ ] **Step 4: 同机并存回归。** `quiet_mode`、`install_entry` 现有测试仍过;确认 Dozer 会话里的 `digger-mcp` 与 Digger 会话里的 `dozer-mcp` 仍是零工具安静模式。
- [ ] **Step 5: 文档。** ADR-015 的 MCP 一节改为"记忆 + 预览/定位/编辑",注明待办工具仍待待办面板;设计稿 §6.3 的状态由审核方更新。
- [ ] **Step 6: 提交** `feat(mcp): digger-mcp 打开 preview 与 file_edit 工具`。

### Task 8: 全量门禁、真机与报告

- [ ] **Step 1: 全量。** 干净快照(路径不含 "digger")里 `cargo metadata --locked --offline`、`cargo test --workspace --no-fail-fast`、`cargo clippy --workspace --all-targets`、`check-host-deps.sh`、`check-shared-deps.sh`;用完删临时目录。
- [ ] **Step 2: 真机(隔离 `DIGGER_DATA_DIR` 与临时 `HOME`,命令行部分你做,点击项写"待用户"):** ① 打开文本文件选一段 → Claude 会话里调 `get_preview_context` 返回同一选区;② `preview_navigate` 打开另一个文件并跳到某行;③ `locate_in_file` 定位文本;④ `apply_precise_edit` 改一处 → 页签内容更新、文件落盘、`revision` 递增;⑤ 带过期 `expected_revision` 的编辑被拒;⑥ 关光所有文本页签后 `get_preview_context` 返回无活动预览;⑦ 切到另一个项目后对旧项目发命令得到 `NotFound`;⑧ 杀掉 `diggerd` 再恢复,轮询自动恢复;⑨ 与 Dozer 同机并存,两边上下文互不串。
- [ ] **Step 3: 报告** `docs/superpowers/reviews/2026-10-<日期>-s6e3a-report.md`:基线、测试总数与名字守恒、每条"通过"附命令与退出码、Task 5 Step 3 的分支清单、待用户项。停在 `feat/preview-bridge`,合并与发布先问用户。

---

## Self-Review

- **Spec 覆盖:** §6.3 的两个前提——"Digger 的 GUI 向 `diggerd` 推送预览上下文"→ Task 5;"执行 `PreviewCommand`"→ Task 6;"只改 `digger-mcp` 入口的特性开关与 GUI 侧"→ Task 7;共享库先行(用户 2026-10-24 选择 B)→ Task 1–3。待办工具明确排除。
- **类型一致:** `build_preview_context`、`resolve_command_target`、`PreviewBridge::{push, spawn_poller}`、`PreviewHub::apply_preview_command`、`Msg::PreviewCommandsFetched` 各在一处定义,后续任务按同名引用。
- **风险点:** ① Task 5 Step 3 的"哪些预览消息要触发推送"没有穷举,是实现者必须自己 grep 出来的;漏了会表现为 Agent 读到旧选区。② Digger 一次只持当前项目的页签,多项目并行时后台项目的命令只能 `NotFound`,与 Dozer 的"每项目一份 Workspace"不同,是已知差异,报告里写明。③ Task 3 的 tag 与合并是对外动作,Part B 在它之前只能用不提交的本地 `[patch]`。④ 新工具暴露了"AI 改文件"能力,真机项 ④⑤ 必须验证过期版本被拒,避免静默覆盖。⑤ 本计划不改 `dozer-mcp`;若发现工具描述里写死了 Dozer 专属说法,只报告不改。
