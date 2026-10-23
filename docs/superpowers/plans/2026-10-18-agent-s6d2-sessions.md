# S6d-2 会话与终端 Implementation Plan

> **迁移说明(2026-10-20):** 本文原在 Digger 仓库,现移入 Dozer 仓库;文内 `docs/superpowers/...` 路径均相对 **Dozer 仓库根**。写作「Digger(仓库 `../digger`)」的部分,实施仍在 Digger 仓库的 worktree 里进行,但文档不在那里。Digger 自有文档(`docs/adr/*`、`2026-10-14-window-shell-design.md`、`docs/superpowers/notes/`)仍在 Digger 仓库。最新要求与决定见 `docs/superpowers/specs/2026-10-18-agent-panel-s6-design.md` §0。


> **2026-10-21 修订(本轮范围与叠分支要求,以此为准):**
> - **本轮只做 Part A(Dozer 库补丁,即总设计 §0 第 9 条链上的「S6d-2 Part A」)。Part B(Task 2–7,Digger 一侧)本期不执行**:它依赖 `libs-*` tag,而 tag 要等整条 Dozer 侧链路经用户一次性人工验收后才切;Part B 留到那之后另行安排。
> - Part A 从 **`feat/panel-agent-2` 顶端 `8cd9bdc9`** 切出(叠分支,不是 `main`):先在主仓库确认 `git rev-parse --short feat/panel-agent-2` 为 `8cd9bdc9`(不是就停下来问),再 `git worktree add ../dozer-restore-patch -b feat/panel-agent-restore feat/panel-agent-2`。命令与路径带完整 worktree 前缀。
> - **不合并 `main`、不打 tag、不推送**;Task 1 Step 5 的「请用户目测」改为把清单写进报告、不阻塞(总设计 §0 第 9 条)。
> - **新增库补丁项 ⑨(来自 S6c-2 审核观察 1):** 库里没有终端滚动与选区消息,滚动/选区(`TermScroll` / `TermSelStart` / `TermSelUpdate`)目前只在 Dozer 的 `app/update.rs` 里对活动页签直接做 `model.scroll_display / selection_start / selection_update`。Digger 的 `term_event` 要把 `TermEvent` 映射成库 `Message`,没有这些变体就无法工作,所以 Task 1 一并补上(见 Task 1)。除 ⑤⑥⑨ 外有缺口仍然停下报告。
> - **基线(`8cd9bdc9`,实测):** worktree 内(含 `.git`)`cargo test --workspace --no-fail-fast` = 3426 通过 / 0 失败 / 5 ignored;`dozer-app` 1251 / 2 ignored;`dozer-panel-agent` 61;`dozer-panel-term` 78 / 1 ignored;`check_migration.py` 退出码 0。干净快照(无 `.git`)里另有 3 条 `git_log` dogfood 测试因缺 git 历史失败,属环境原因。clippy 警告数开工时自己记录,完工不得更多(`dozer-app` bin 当前 19 条,库与新文件零警告)。
> - **实现者须知(前几期审核里真踩过的坑):** ① `.gitignore` 的 `examples/` 会吞文件,新增示例/夹具提交后用 `git status --ignored` 核对;② 改依赖必须同时提交 `Cargo.lock`,并在干净快照里验证 `cargo metadata --locked --offline`;③ 完工前把 `git archive HEAD` 解到**独立临时目录且用该目录自己的 `target`**(不要和别处共用 `CARGO_TARGET_DIR`,`dozerd` 的 `product_isolation` 按 `<repo>/target/` 找示例),重跑 `cargo test --workspace --no-fail-fast`,预期只有那 3 条 git_log 环境失败;④ 绝不覆盖式重新生成 `scripts/i18n/inventory.json`;⑤ `cargo fmt --check` 与 `scripts/check-log-scope.sh` 在 `main` 上本来就失败,只要求自己改的文件干净(用 `rustfmt --check --edition 2024` 对改动文件);⑥ 报告里每条「通过」必须是刚实跑的,附命令与退出码,审核人会逐条重跑;基线对不上或计划与代码不符时先停下来问;⑦ S6c-1、S6c-2 的黄金测试与夹具(`session_stream_golden.txt`、`session_protocol_encoding_golden`、S6b 的终端黄金)字节不得改动,补丁前后都要过。

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Digger 的 Agent 面板真正可用:每个项目一份 Agent 状态,面板首次可见时恢复(没有则新开一个 shell)该项目在 `diggerd` 里的存活会话,新建会话(选择器)、终端渲染与键盘/粘贴/复制/拖入/输入法、关页签确认、窗口缩放时网格尺寸同步、关项目页签杀会话、关窗口只 detach 且等待在途的 kill。页签外观先用**朴素版**(选中 + 关闭),Dozer 同款外观、溢出菜单、拖拽换位留给 S6d-3。

**Architecture:** Dozer 一侧先补一个小库补丁(会话恢复的共享路径 `restore_sessions` / `adopt_restored`,Dozer 的 `from_restore` 同步改用它,避免两条恢复路径),切新 tag。Digger 一侧在 `adapters/agent.rs` 把 S6d-1 的 `AgentShell` 扩成 `AgentPanel`:`HashMap<i64, AgentPanelState>`、`Arc<dyn SessionService>`(来自 `daemon::Daemon` 的 `Client`)、`HookInstaller`(调同目录的 `digger-hook install`)、空 `TaskTitleSource`、全关的 `AgentCapabilities`;消息 `Msg::Agent(i64, dozer_panel_agent::Message)` 带项目 id;库的 `emit` 写入通道,在 `boot` 接成 iced 流。输入经 S6d-1 的 `agent_keys` / `ImeSink`,尺寸由 `adapters/agent_layout.rs` 算网格并去抖发送。

**Tech Stack:** Rust、iced 0.14、`dozer-panel-agent` / `dozer-panel-term` / `dozer-services` / `dozer-client`(Dozer `libs-*` tag)、tokio、byteui(`context_menu`、`feedback::dialog`)。

**Spec:** `docs/superpowers/specs/2026-10-18-agent-panel-s6d-design.md`(§6.2、§6.3、§6.4、§6.5、§6.6 选择器与确认框、§6.7、§7 S6d-2、§8)。前置计划:`…-agent-s6c1-session-state.md`、`…-agent-s6c2-views.md`(Part A 的基线,均已通过审核、叠在 `feat/panel-agent-2` 上,**未合并**);`…-agent-s6d1-probe-and-shell.md` 是 Digger 侧,Part B 才用。

## Global Constraints

- **不得在 Digger 里复制库的逻辑。** 库缺什么,只能走 Part A 的 Dozer 补丁;补丁范围超出 Task 1 列出的两项就停下报告。
- 依赖方向与 S6d-1 相同:`dozer-panel-agent`、`dozer-panel-term`、`dozer-client` 只允许 `digger-desktop` 依赖;`check-host-deps.sh`、`check-shared-deps.sh`、Cargo.lock 重复检查必须通过。
- **会话语义与 Dozer 一致:** 关项目页签 = abort 转发并 `kill` 该项目全部会话;关窗口只 detach、会话随 `diggerd` 存活;只有显式关页签才 `kill`;应用退出前必须等在途的 kill 跑完(有上限,见 Task 3)。
- **UI 线程不等守护进程:** 列会话、attach、写入、resize 全在后台任务里,结果经 `emit` 回投。
- **懒启动(设计 D1):** 某项目的 Agent 面板第一次可见时才恢复/新建;从未打开过的项目不起任何进程;同一项目只会启动一次(见 Review Focus 2)。
- 键位规则沿用 S6d-1:⌘ 组合键不进 PTY,⌘C(有选区)复制、⌘V 粘贴;字符键用 `modified_key`(带 shift 的结果),具名键用 `key`。
- 失败不静默:守护进程调用失败的通知显示在 Agent 面板顶部一行(红字,可点关闭),同时 `eprintln!`(沿用用量面板的做法)。
- **发布要求(项目硬性规定):每个面板迁移完成后,Dozer 与 Digger 各重新发布一个版本。** Agent 面板的迁移在 S6d-3(页签外观、溢出、拖拽、真机回归)才算完成,所以发布步骤放在 S6d-3 的收尾;本期的朴素页签是临时件,**本期不单独发布**。发布(升版本、打包、安装)是对外动作,先向用户确认。
- 主工作区别人未提交的改动不动;Dozer 在 worktree `../dozer-restore-patch`(分支 `feat/panel-agent-restore`,从 `feat/panel-agent-2` 切出)、Digger 在 `../digger-s6d2`(分支 `feat/agent-sessions`,Part B 才建)。**本期不打 tag、不推送、不合并**;这些只在用户一次性人工验收之后、逐项向用户确认再做。

## Review Focus

1. **跨项目串线与迟到消息:** 项目 A 的 `Output` / `AgentState` / `Attached` 不改项目 B 的状态;项目页签已关闭后到达的消息被忽略;后台项目收到消息不抢前台焦点。→ Task 3。
2. **重复启动与半途切换:** 面板连续两次变为可见、恢复进行中切走又切回、同一项目两个异步恢复结果先后到达,都只得到一份会话集合,不会多开 shell;恢复结果到达时该项目页签已关闭则丢弃并 detach(不留孤儿转发任务)。→ Task 3。
3. **字节流连续:** 恢复时快照 + 后续事件拼接处不重不漏;`diggerd` 中途断开(`Disconnected`)时页签标记退出、不崩、不反复重连刷屏。→ Task 3。
4. **输入送错处:** Agent 面板不可见、焦点在别的输入框、窗口正在关闭、没有任何会话时按键/粘贴/拖入被丢弃且不 panic;输入法预编辑在面板被切走或焦点丢失时清除。→ Task 5。
5. **退化尺寸与缩放风暴:** 终端块算出 0 或负数、拖分隔线连续上百次缩放:网格至少 1×1、同一网格不重复发 `resize`、不发 0。→ Task 6。
6. **退出时在途任务:** 刚关页签(kill 在途)立刻关窗口:kill 在上限内完成,超时则放弃并仍然退出,不卡死窗口。→ Task 3。

---

## Part A — Dozer(仓库 `../dozer`,worktree `../dozer-restore-patch`,分支 `feat/panel-agent-restore`,**从 `feat/panel-agent-2` 顶端切出**;**本轮执行**)

### Task 1: 库补丁:会话恢复的共享路径,以及终端滚动/选区消息

**Files:**
- Create/Modify:`crates/dozer-panel-agent/src/{restore.rs,state.rs,lib.rs}`、`crates/dozer-app/src/workspace/state.rs`(`fetch_project_restore`、`from_restore` 改用库)、`crates/dozer-panel-agent/src/update.rs`(`Message::Resize`,若 Step 1 发现缺失)
- Test:`crates/dozer-panel-agent/src/restore.rs`、`crates/dozer-app/src/workspace/tests.rs`

**Interfaces:**
- Produces:
  - `restore::RestoredSession { pub info: SessionInfo, pub snapshot: Vec<u8>, pub events: tokio::sync::mpsc::UnboundedReceiver<SessionEvent> }`。
  - `restore::fetch_restored(svc: &dyn SessionService, project_id: i64) -> BoxFuture<'_, Vec<RestoredSession>>`:`list_sessions` 过滤出 `project_id` 且存活的会话,逐个 `attach(id, 0)`;单个 `attach` 失败只跳过该会话,不影响其余(与 `fetch_project_restore` 现有行为一致)。
  - `AgentPanelState::adopt_restored(&mut self, sessions: Vec<RestoredSession>, handle: &tokio::runtime::Handle, emit: impl Fn(Message) + Clone + Send + 'static)`:原 `from_restore` 里建页签的循环——每个会话一个 `SessionTab`(`TerminalModel::new(DEFAULT_COLS, DEFAULT_ROWS)`、`set_answer_dynamic_color(info.agent == Opencode)`、喂快照并丢弃应答、`ingest_osc(&snapshot)`、起转发任务、`backend: Daemon`),`tab_id` 从 `next_tab_id` 起递增。
  - **⑨ 终端滚动与选区(补丁项,Step 1 已确认缺失):** 库 `Message` 新增 `Scroll(i32)`、`SelStart { col: usize, row: usize, right: bool }`、`SelUpdate { col: usize, row: usize, right: bool }`,三者都只作用于**活动页签**(`tabs.get_mut(active)`),分别调 `model.scroll_display(delta)` / `selection_start(col,row,right)` / `selection_update(col,row,right)`;活动页签不存在时静默忽略。再加 `pub fn term_message(event: dozer_panel_term::view::TermEvent) -> Message`(`Input(b)`→`Message::Input(b)`,`Scroll(n)`→`Scroll(n)`,`SelStart{col,row,right_half}`→`SelStart{col,row,right:right_half}`,`SelUpdate` 同理),Digger 的 `PanelDisplay::term_event` 直接用它。**Dozer 侧:`TermTarget::Shared` 的 `TermScroll` / `TermSelStart` / `TermSelUpdate` 改为转库 `update`(经 `Message::Agent`),`TermTarget::SshPanel` 保持原样(SSH 页签不在库状态里)**;删掉被取代的 `Shared` 分支,行为不变。
  - 若库 `Message` 缺 `Resize`:`Message::Resize { cols: u16, rows: u16 }`,对全部存活页签调用 `tabs::resize_tab`(行为同原 `resize_all`:已退出的不发,`Custom` 走 `TabOut::Resize`)。
- Consumes:S6c-1 的 `SessionService`、`SessionTab`、`AgentPanelState`、`tabs::resize_tab`。

- [ ] **Step 1: 接缝核对。** 对照已合并的 `dozer-panel-agent` 逐项确认 Digger 要用的公开面:① `AgentPanelState::new`;② `update::update` / `spawn_new_tab` / `UpdateCtx`;③ `take_outbox` / `take_requests` / `take_host_events`;④ 页签选择/关闭/确认消息;⑤ **会话恢复入口**(预期缺失,本任务补);⑥ **一次性调整全部页签尺寸的消息**(缺失则补);⑦ `active_app_cursor_mode` / `active_selection_text`;⑧ `PanelDisplay` 与 `view::{agent_list, terminal_pane}`;⑨ **终端滚动/选区消息与 `TermEvent` → `Message` 映射**(缺失,本任务补)。把结论写成清单放进提交说明;除⑤⑥⑨外有缺口就停下报告。
- [ ] **Step 2: 写失败测试:** `fetch_restored_returns_only_live_sessions_of_the_project`(假服务含三项目的会话)、`a_failing_attach_skips_that_session_only`、`adopt_restored_builds_one_tab_per_session_with_distinct_ids_and_the_snapshot_fed`(可见内容与快照一致,`next_tab_id` 递增)、`adopt_restored_forwards_later_events_to_the_right_tab`、`adopt_restored_with_no_sessions_leaves_the_state_empty`(调用方据此新开 shell,库不自作主张);`resize_message_resizes_every_live_tab_and_skips_exited_ones`(若补了 `Resize`);滚动与选区:`scroll_moves_the_display_of_the_active_tab_only`、`selection_start_and_update_apply_to_the_active_tab_and_the_selected_text_follows`、`scroll_and_selection_without_an_active_tab_are_ignored`、`term_message_maps_every_term_event_variant`(`Input`/`Scroll`/`SelStart`/`SelUpdate` 四种,`right_half` → `right`);Dozer 侧:`Shared` 的滚动/选区转库后与旧行为一致(沿用旧测试断言,改走新路径后仍通过),SSH 面板的三条路径不受影响;原 `from_restore` 相关测试**原样通过**,会话字节流黄金文件字节未变。
- [ ] **Step 3: 运行确认失败,实现,并让 `from_restore` / `fetch_project_restore` 改用库。** 恢复后"没有会话则新开 shell"的决定仍在调用方(Dozer 的 `ensure_project_terminal`)。
- [ ] **Step 4: 验证。** `cargo test --workspace`(总数 = 基线 + 新增)、clippy、fmt、i18n 与边界脚本;`cargo tree -p dozer-panel-agent` 满足既有依赖闭包约束。
- [ ] **Step 5: 干净快照复验与报告(不合并、不打 tag、不推送)。** `git archive HEAD` 解到独立临时目录(用它自己的 `target`):`cargo metadata --locked --offline`、`cargo test --workspace --no-fail-fast`(预期仅 3 条 git_log 环境失败)、`python3 scripts/i18n/check_migration.py`、`python3 scripts/audit/check_panel_boundary.py`。报告写到 `docs/superpowers/reviews/2026-10-<日期>-s6d2a-restore-patch-report.md`,写明:Step 1 的接缝清单、基线核对、每个步骤的实测命令与退出码、测试总数、黄金测试不变的证据,以及**目测清单**(整条 Dozer 链路完成后用户一次性验收):重启后各项目会话仍在且内容连续、输入正常、终端滚轮滚动与鼠标选区/⌘C 复制、SSH 面板终端滚动与选区不受影响。提交后**停在 `feat/panel-agent-restore`**。

## Part B — Digger(仓库 `../digger`,worktree `../digger-s6d2`,分支 `feat/agent-sessions`)——**本期不执行**,依赖 `libs-*` tag,待整条 Dozer 链路验收并切 tag 后另行安排

### Task 2: 服务与 hook 接线

**Files:**
- Create:`crates/desktop/src/adapters/agent_hooks.rs`
- Modify:workspace `Cargo.toml` 与 `Cargo.lock`(升到 Part A 的新 tag)、`crates/desktop/src/daemon.rs`(`Daemon::sessions()`)、`crates/desktop/src/adapters/mod.rs`
- Test:`agent_hooks.rs`、`daemon.rs`

**Interfaces:**
- Produces:
  - `Daemon::sessions(&self) -> Arc<dyn SessionService>`(包装同一个 `dozer_client::Client`,连 `diggerd` 的 socket)。
  - `agent_hooks::DiggerHooks { hook_exe: PathBuf }`,实现 `HookInstaller`:`hook_exe()` 返回同目录 `digger-hook` 的路径字符串(找不到退回名字本身);`ensure_hook(agent)`:Claude / Codebuddy / Codex / Qodercn / Opencode / Goose 运行 `<hook_exe> install <agent.label()>`(**不继承 stdin、丢弃输出、有 10 秒上限**),`Unknown` / `Aider` / `V8agent` 什么都不做(与 Dozer `hook_install_target` 的 `None` 名单一致);`ensure_mcp(agent)`:空操作(能力关闭,设计 D5)。
- Consumes:S6a 的 `digger-hook install` 子命令与 `Daemon`;Part A 的 tag。

- [ ] **Step 1: 升依赖。** 所有 dozer libs 的 `tag` 改为 Part A 的新 tag,加 `dozer-panel-agent`(若 S6d-1 未加),跑 Cargo.lock 重复检查与 `check-shared-deps.sh`。
- [ ] **Step 2: 写失败测试:** `hook_exe_is_the_sibling_digger_hook`、`hooks_run_install_for_known_agents_and_skip_aider_unknown_v8agent`(用一个记录参数的假脚本充当 `digger-hook`,断言收到的参数 `install claude` 等,三类不调用)、`a_missing_hook_binary_does_not_panic_or_block`、`a_hanging_hook_is_abandoned_after_the_timeout`(假脚本 `sleep 60`,在上限内返回)、`ensure_mcp_is_a_noop`、`sessions_service_talks_to_the_daemon_socket`(沿用 `daemon.rs` 的假服务端夹具,`list_sessions` 往返)。
- [ ] **Step 3: 运行确认失败,实现,验证,提交。**

### Task 3: `AgentPanel`:每项目状态、路由、懒启动、关闭与退出

**Files:**
- Modify:`crates/desktop/src/adapters/agent.rs`(`AgentShell` → `AgentPanel`)、`crates/desktop/src/product.rs`(`Msg::Agent`、`Msg::AgentRestored`、`boot` 接流、`on_visible`、`Msg::Projects` 后同步、`CloseRequested` / `ShutdownFinished` 链)
- Test:`agent.rs`、`product.rs`

**Interfaces:**
- Produces:
  - `AgentPanel`:字段 `states: HashMap<i64, AgentPanelState>`、`starting: HashSet<i64>`(恢复在途)、`svc`、`hooks`、`caps`、`handle`、`tx/rx`(`UnboundedSender<(i64, Message)>`)、`notice: Option<String>`、`critical: Arc<Mutex<Vec<JoinHandle<()>>>>`、`focus: bool`、`terminal_rect: Option<Rectangle>`、`last_grid: HashMap<i64, (u16, u16)>`。
  - `ensure_started(&mut self, project: &ProjectInfo, grid: (u16, u16)) -> Task<Msg>`:若 `states` 已有或 `starting` 含该项目 → 什么都不做;否则 `starting.insert`,后台 `restore::fetch_restored`,完成后回投 `Msg::AgentRestored(pid, Arc<Mutex<Option<Vec<RestoredSession>>>>)`(接收端不可 `Clone`,用 `Arc<Mutex<Option<_>>>` 包一层,处理时 `take`)。
  - `on_restored(&mut self, pid: i64, project: Option<&ProjectInfo>, sessions: ..., grid) -> Task<Msg>`:`starting.remove`;项目已不在已开页签(`project == None`)→ 丢弃并 abort 各 `events` 转发(drop 接收端);否则建 `AgentPanelState`,`adopt_restored`;没有会话则 `spawn_new_tab(PickerLaunch::Agent(None))`(根在项目路径)。
  - `update(&mut self, pid: i64, msg: Message, project: Option<&ProjectInfo>, grid) -> Task<Msg>`:没有 `states[pid]` 的消息忽略;调库 `update`(`UpdateCtx` 取 `project`、`svc`、`handle`、`caps`、`hooks`、`locale = "zh-CN"`?——取 Digger 现有的语言设置,没有则 `"en"`,`track_exit_critical` 指向 `self.critical`);随后排空 `take_outbox()`(写 `notice` 与 `eprintln!`)、`take_requests()`(`ShowPicker` 交给 Task 4)、`take_host_events()`(Digger 一期忽略:没有会话列表/git/磁盘占用联动)。
  - `close_project(&mut self, pid: i64) -> Task<Msg>`:abort 转发、对该项目全部会话 `kill`(任务计入 `critical`)、移除状态、`starting.remove`、`last_grid.remove`。
  - `prepare_shutdown(&mut self)`:只 abort 转发,不 kill。`drain_critical(&self) -> impl Future<Output = ()>`:并行等 `critical` 里的任务,**整体上限 3 秒**,超时放弃。
- Consumes:Part A 的 `restore::{fetch_restored, RestoredSession}`、`adopt_restored`;Task 2 的服务与 hook;S6d-1 的 `AgentShell` 状态机(`Unavailable` 时不启动任何会话)。

- [ ] **Step 1: 写失败测试**(`FakeSession` 由库的 `service_fakes` 思路在 Digger 内自备一个同形替身;每条独立):
  - `messages_for_one_project_never_touch_another`(两项目各一页签,给 A 发 `Output`/`AgentState`/`Exited`,B 的终端内容与状态不变);
  - `a_message_for_a_project_without_state_is_ignored`、`late_messages_after_close_project_are_ignored`;
  - `first_visibility_restores_live_sessions_and_does_not_open_a_shell`(假服务有该项目的两个存活会话 → 两页签,无新建);
  - `first_visibility_without_sessions_opens_one_shell_in_the_project_directory`(`create` 的 `cwd == project.path`、`project_id` 正确);
  - `ensure_started_twice_starts_once`(连续调用两次,`list_sessions` 只被调一次)、`switching_away_and_back_during_restore_does_not_double_start`;
  - `restore_result_for_a_closed_project_is_discarded_and_detached`(项目页签已关,`states` 不出现该项目,转发被 drop);
  - `restore_snapshot_and_following_events_are_contiguous`(假服务快照 + 立刻到达的事件,终端最终内容与逐字期望一致);
  - `disconnected_event_marks_the_tab_exited_and_does_not_reconnect`;
  - `close_project_kills_every_session_of_that_project_only`(另一项目的会话无 `kill`);
  - `prepare_shutdown_detaches_without_killing`;
  - `drain_critical_waits_for_inflight_kills_but_gives_up_after_the_limit`(一个 100ms 的 kill 被等到;一个永不结束的任务让整体在上限内返回,上限用可注入的 `Duration` 以便测试 50ms);
  - `unavailable_daemon_starts_nothing_and_keeps_the_panel_usable_after_retry`;
  - `service_failures_surface_as_a_notice_and_can_be_dismissed`(`create` 失败 → `notice` 有文本,`DismissNotice` 清除)。
- [ ] **Step 2: 运行确认失败,实现。** `product.rs`:`Msg::Projects` 处理前后比较已开项目 id 集合,消失的 id 调 `close_project`;`on_visible(agent)` 与"项目切换后 Agent 面板仍可见"都调 `ensure_started`;`CloseRequested` 里调 `prepare_shutdown`,`ShutdownFinished` 链在 `apps.shutdown()` 之外并列 `drain_critical`。
- [ ] **Step 3: 验证。** `cargo test -p digger-desktop`;沙箱里起 Digger 开一个项目,面板可见后 `diggerd` 里出现该项目的一个会话,关项目页签后会话消失,关窗口后会话仍在。
- [ ] **Step 4: 提交。**

### Task 4: 展示适配、配对视图、选择器与关闭确认

**Files:**
- Modify:`crates/desktop/src/adapters/agent.rs`、`crates/desktop/src/product.rs`(`agent_view`、覆盖层)
- Create:`crates/desktop/src/adapters/agent_display.rs`(`PanelDisplay` 实现与朴素页签)、`crates/desktop/src/adapters/menu_overlay.rs`(菜单弹层:外框 + 菜单行 + 分隔线 + 点空白/Esc 关闭 + 钳制在窗口内;若 S5b-1 已先建则复用并只补缺的部分;测试:`menu_is_clamped_inside_the_window`、`empty_item_list_opens_nothing`、`disabled_entries_are_not_clickable`、`separator_is_not_clickable`)
- Test:`agent_display.rs`、`product.rs`

**Interfaces:**
- Produces:
  - `AgentDisplay<'a> { panel: &'a AgentPanel, pid: i64, term_style: &'a TermStyle, locale: &'a str }` 实现 `PanelDisplay<Msg>`:`wrap` → `Msg::Agent(pid, m)`;`panel_head` / `list_collapse_button`(按下发 `Msg::Shell(ShellMsg::TogglePane(agent_id))`)/ `tab_divider` / `pane_style` 仿 `ProjectPanel` 与 Dozer 区域度量;`hover_progress` 恒 0、`hover_tooltip_ready` 恒假(设计 D3);`term_style` / `terminal_focused`(= `panel.focus`)/ `ime_preedit` / `term_event`(用库的 `term_message(event)` 得到库 `Message`,包成 `Msg::Agent(pid, ..)`,同时置 `focus = true`);`tab_bar_avail_px`;**朴素页签四件**:`panel_tab` = 状态点 + 标题的按钮 + 一个 × 按钮(`tab_core` 即可);`overflow_button` → `None`;`overflow_menu` → 空元素;`drag_surface` → 原样返回内容。文件头注明「S6d-3 替换为 Dozer 同款页签」。
  - `ProductShell::agent_view(&self, area: &BodyArea) -> Element<Msg>`:有项目且已启动 → `pair_view(area, 列表, 终端, Msg::Shell(DividerPress(Pair(side))), bg, panel_bg)`;列表块收起时只画终端块;顶部 `notice` 行;`Unavailable`/`Checking`/无项目沿用 S6d-1 的壳;终端块外层包 `ImeSink`。
  - 选择器:`AgentRequest::ShowPicker { at }` → `AgentPanel.picker = Some(at)`,`stack!` 叠一层 `adapters/menu_overlay.rs` 的菜单(byteui 的 `interaction::context_menu` 只是「右键触发」封装(`MouseArea::on_right_press`),**没有**菜单外框与菜单项渲染——那部分(`shell`、`item`、分隔线、悬停样式)在 Dozer 的 `chrome/menu.rs`。Digger 因此自备一个菜单弹层模块 `crates/desktop/src/adapters/menu_overlay.rs`(外框与行样式对照 Dozer `chrome/menu.rs` 与 `workspace.json` 的 `context_menu` 区域;谁先做谁建,另一方复用),长期应迁入 byteui;条目取 `picker::picker_entries()`,图标与颜色取条目自带),点选发 `Message::PickerSelect(launch)`,点空白或 Esc 发 `PickerClose`。
  - 关闭确认:`close_confirm_spec(state, locale, wrap)` 返回的 `ConfirmDialog` 用 byteui 的对话框元素叠在面板上(同 `project_delete_confirm` 的覆盖方式)。
- Consumes:库的 `view::{agent_list, terminal_pane}`、`picker::*`、`PanelDisplay`;S6d-1 的 `ImeSink`、`terminal_style()`。

- [ ] **Step 1: 写失败测试:** `hover_is_static_and_tooltips_never_show`;`term_event_maps_to_the_agent_message_of_this_project_and_takes_focus`;`list_collapse_button_toggles_the_pane_of_the_agent_panel`;`plain_tab_close_sends_close_for_that_tab`;`picker_overlay_lists_every_known_agent_then_the_two_shells_and_selecting_creates_a_session_in_the_project_directory`(经假服务断言 `create` 的 `command` 与 `cwd`);`escape_and_outside_click_close_the_picker_without_creating`;`close_confirm_appears_only_for_running_or_awaiting_input_tabs_and_confirming_kills_that_session`;`collapsed_list_shows_only_the_terminal_block`;`agent_view_with_no_project_or_unavailable_daemon_never_builds_the_pair`。
- [ ] **Step 2: 运行确认失败,实现。**
- [ ] **Step 3: 验证。** `cargo test -p digger-desktop`;沙箱真机起一次,截图:空态、一个 shell 会话、选择器、确认框。
- [ ] **Step 4: 提交。**

### Task 5: 键盘、指针焦点、粘贴复制、拖入与输入法

**Files:**
- Modify:`crates/desktop/src/product.rs`(订阅与更新分支)、`crates/desktop/src/adapters/agent.rs`、`crates/desktop/src/adapters/agent_keys.rs`(新增 `event_bytes`)
- Test:`agent_keys.rs`、`agent.rs`、`product.rs`

**Interfaces:**
- Produces:
  - `agent_keys::event_bytes(ev: &iced::keyboard::Event, app_cursor: bool) -> Option<Vec<u8>>`:只处理 `KeyPressed`;`Key::Character` 用 `modified_key`,具名键用 `key`;其余同 Task 1 的 `key_bytes`(⌘ 恒 `None`)。
  - `Msg::AgentKey(iced::keyboard::Event)`、`Msg::AgentPointer(Point)`(任意鼠标按下)、`Msg::AgentDrop(PathBuf)`、`Msg::AgentPaste(i64, String)`(剪贴板读取结果);`subscription` 的 `listen_with` 在 `status == Ignored` 时产出前两种(及窗口 `FileDropped` → `AgentDrop`),`status == Captured` 的按键不产出。
  - `AgentPanel::on_key(&mut self, ev, visible: bool, project: Option<&ProjectInfo>) -> Task<Msg>`:不可见 / 无焦点 / 无活动会话 / 窗口关闭中 → 丢弃;⌘C 且有选区 → `iced::clipboard::write`;⌘V → `iced::clipboard::read()` → `Msg::AgentPaste`;其余 → `event_bytes` → `Message::Input(bytes)`。
  - `AgentPanel::on_pointer(&mut self, p: Point)`:`focus = terminal_rect.is_some_and(|r| r.contains(p))`。`AgentDrop` 在面板可见时把 `dropped_path_bytes(path)` 写入活动会话。
  - 输入法:`ImeSink` 的 `enabled = panel.focus && 可见`;`on_commit(text)` → `Message::Input(ime_commit(text))`;`on_preedit(p)` 存入 `panel.preedit`(供 `PanelDisplay::ime_preedit`);面板被切走或 `focus` 变假时清除 `preedit`。光标矩形 = 终端块原点 + (内边距, 页签条高度) + 光标格 × 单元格尺寸。
- Consumes:S6d-1 的 `agent_keys`、`ImeSink`/`ImeState`;库的 `Message::{Input, Paste}`、`active_app_cursor_mode`、`active_selection_text`。

- [ ] **Step 1: 写失败测试:** `shifted_characters_use_the_modified_key`(`Shift+a` 得 `A`、`Shift+1` 得 `!`);`keys_are_dropped_when_the_panel_is_hidden_unfocused_or_closing_or_has_no_session`;`a_pointer_press_inside_the_terminal_focuses_it_and_outside_unfocuses`;`captured_key_events_never_reach_the_terminal`(订阅映射函数在 `Captured` 时返回 `None`);`command_c_copies_only_with_a_selection_and_otherwise_does_nothing`;`command_v_reads_the_clipboard_and_pastes_with_bracketed_paste_when_the_app_enabled_it`;`dropped_file_writes_its_quoted_path`;`ime_commit_writes_the_text_and_clears_the_preedit`;`preedit_is_cleared_when_focus_is_lost_or_the_panel_is_hidden`;`input_goes_to_the_active_session_of_the_focused_project_only`。
- [ ] **Step 2: 运行确认失败,实现。**
- [ ] **Step 3: 验证。** `cargo test -p digger-desktop`;沙箱真机:输入 `echo hi`、方向键历史、Ctrl+C 打断 `sleep 100`、`vim` 进出、粘贴多行、⌘C 复制选区、拖入文件路径;在 Project 面板输入框里打字时终端不收到按键。
- [ ] **Step 4: 提交。**

### Task 6: 网格尺寸同步

**Files:**
- Create:`crates/desktop/src/adapters/agent_layout.rs`
- Modify:`crates/desktop/src/product.rs`(`update_host` 末尾调用)、`crates/desktop/src/adapters/agent.rs`
- Test:`agent_layout.rs`

**Interfaces:**
- Produces:`agent_layout::terminal_block(pane: Rect, list_collapsed: bool, pair: Option<PairRects>) -> Rect`(配对且列表展开取 `pair.second`,否则取整个面板内容矩形);`agent_layout::grid(block: Rect, pane: &PaneStyle, tab_bar_h: f32, style: &TermStyle) -> (u16, u16)`(块宽高减内边距与页签条高度后经 `TermStyle::grid_size`,**至少 `(1, 1)`**,不超过 `u16::MAX`);`AgentPanel::sync_grid(&mut self, pid, grid) -> Task<Msg>`(与 `last_grid[pid]` 相同则不发;不同则发 `Message::Resize { cols, rows }` 并记录;`(0, _)`/`(_, 0)` 不发)。同时更新 `terminal_rect`(Task 5 的焦点判断用)。
- [ ] **Step 1: 写失败测试:** `grid_matches_the_style_formula_for_a_known_block`(用 S6d-1 Task 3 里与 Dozer 同输入同输出的那组数值);`degenerate_blocks_give_at_least_one_by_one`(宽高为 0、负、小于内边距)、`huge_blocks_clamp_to_u16`、`collapsed_list_uses_the_whole_panel_and_expanded_uses_the_second_block`、`repeated_sync_with_the_same_grid_sends_nothing`、`a_hundred_resizes_to_the_same_grid_send_once`、`a_new_project_starts_with_the_current_grid_so_its_first_session_is_created_at_the_right_size`。
- [ ] **Step 2: 运行确认失败,实现。** 在 `update_host` 的 `sync_native` 之后调用(每条消息一次廉价比较)。`ensure_started` 的 `grid` 参数取自同一计算,避免新会话先以 80×24 创建再缩。
- [ ] **Step 3: 验证。** `cargo test -p digger-desktop`;沙箱真机:`stty size` 与窗口一致,拖分隔线、收起列表、缩放窗口后再看 `stty size` 同步,`vim` 内重绘正确。
- [ ] **Step 4: 提交。**

### Task 7: 门禁、真机验证与交付

- [ ] **Step 1: 全量。** `cargo build --workspace && cargo test --workspace && cargo clippy --workspace --all-targets && cargo fmt --check`;`check-host-deps.sh`、`check-shared-deps.sh`、Cargo.lock 重复检查。
- [ ] **Step 2: 真机回归**(沙箱 `DIGGER_DATA_DIR` 与临时 `HOME`;点击和输入法由用户做,我启动并截图):① 开项目 A,打开 Agent 面板,出现一个 shell;输入命令、方向键、粘贴、复制、Ctrl+C;② 选择器新建 Claude / OS Shell / Git Shell,卡片出现、状态点随 hook 更新;③ 开项目 B,会话互不可见;切回 A 内容完整;④ 退出 Digger 再启动,A、B 的会话在打开面板时恢复且内容连续;⑤ 关项目页签 → `diggerd` 里该项目会话消失;⑥ 刚关页签立刻关窗口,不卡死;⑦ `diggerd` 改名后启动:面板显示原因,改回后「重试」恢复;⑧ 中文输入法输入一句(依 S6d-1 探针结论);⑨ Dozer 与 Digger 同机并存各开 Claude 会话,状态点互不串。逐项记录,任一项不符先停下。
- [ ] **Step 3: 报告** 写明:Part A 补丁与新 tag、各项真机结果与截图路径、哪些依赖人工、朴素页签与 Dozer 的外观差异清单(交给 S6d-3)、遇到的与计划不符之处。合并前向用户确认。

---

## Self-Review

- **Spec 覆盖:** 设计 §6.2 适配与每项目状态、消息带项目 id → Task 3、4;§6.3 懒启动/恢复/关项目页签/不可用态 → Task 3(+Part A);§6.4 键盘/焦点/复制粘贴/拖入/输入法 → Task 5;§6.5 网格尺寸 → Task 6;§6.6 选择器与确认框 → Task 4(溢出菜单留 S6d-3);§6.7 退出语义 → Task 3;G5 重启恢复 → Part A + Task 3 + Task 7;G6 → Task 3、7。**不在本期:** Dozer 同款页签外观、溢出下拉、拖拽换位、悬停动画(S6d-3)。
- **类型一致:** `RestoredSession` / `fetch_restored` / `adopt_restored` 在 Task 1 定义,Task 3 引用;`AgentPanel` 字段在 Task 3 一次定义,Task 4–6 增方法不增字段(`picker`、`preedit` 为 Task 4、5 追加的字段,在各自任务的 Interfaces 里说明);`event_bytes` 在 Task 5 定义,基于 S6d-1 的 `key_bytes`;`terminal_block` / `grid` 在 Task 6 定义,Task 3 的 `ensure_started(grid)` 参数用它。
- **风险点:** ① Part A 之外若 `dozer-panel-agent` 还有别的缺口(Step 1 清单⑤⑥⑨以外),必须停下报告,不在 Digger 里绕;② `Message` 变体名以 S6c-1 / S6c-2 合并结果为准(`Input`、`Paste`、`PickerSelect`、`PickerClose`、`DismissNotice`、`Resize`);③ macOS 输入法期间 iced 是否仍会发 `KeyPressed`(造成拼音字母同时进终端)只能真机验证,若出现双重输入,在 `on_key` 里以 `ImeState.active` 抑制并写进报告;④ `drain_critical` 的 3 秒上限是我的取舍,过短会丢 kill、过长拖慢退出;⑤ 恢复结果用 `Arc<Mutex<Option<_>>>` 过 iced 的 `Clone` 约束,处理分支只能 `take` 一次;⑥ 朴素页签是临时件,S6d-3 前不对外发布;⑦ Digger 一期没有 Toast,通知条是新增的最小 UI;⑧ 打 tag、推送、合并均为对外动作。
