# S6c-1 会话服务与会话/页签状态 `dozer-panel-agent`(上) Implementation Plan

> **迁移说明(2026-10-20):** 本文原在 Digger 仓库,现移入 Dozer 仓库;文内 `docs/superpowers/...` 路径均相对 **Dozer 仓库根**。写作「Digger(仓库 `../digger`)」的部分,实施仍在 Digger 仓库的 worktree 里进行,但文档不在那里。Digger 自有文档(`docs/adr/*`、`2026-10-14-window-shell-design.md`、`docs/superpowers/notes/`)仍在 Digger 仓库。最新要求与决定见 `docs/superpowers/specs/2026-10-18-agent-panel-s6-design.md` §0。


> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 Dozer 里新增 `SessionService`(守护进程会话的唯一访问接缝),给 `dozer-panel-term` 补输入翻译与 OSC 扫描,新建共享库 `dozer-panel-agent` 并把**会话与页签的状态、消息、更新**(新建、attach、输出摄取、状态变更、选择器、关闭与确认、尺寸同步、输入写入)从 `Workspace` / `App` 里搬进去;Dozer 与 SSH 面板改为使用库里的 `SessionTab`。Dozer **用户可见行为不变**。本期**只动 Dozer**;卡片列表、选择器菜单与面板外壳视图是 S6c-2。

**Architecture:** `dozer-model` 新增 `SessionEvent`(原 `dozer_client::TermEvent`,`dozer-client` 重新导出);`dozer-services` 新增 `SessionService` / `SessionSpec`,`dozer-client::Client` 实现它。`dozer-panel-term` 新增 `input`(中立键位类型 + `key_to_bytes` / `ime_commit` / `dropped_path_bytes`)与 `osc`(`OscScanner`)。`dozer-panel-agent` 拥有 `AgentPanelState`(从 `Workspace` 搬出 `tabs`、`active`、`next_tab_id`、`pending`、`term_tab_first`、`term_tab_overflow_anchor`、`agent_picker_open`、`pending_close_tab`、`picked_agent`)、面板自有 `Message`、`update`、`SessionTab` 与 `TabBackend::{Daemon, Custom}`。所有后台任务经 `emit: impl Fn(Message)` 回灌,不再用 `winit::EventLoopProxy`;错误经 `NoticeOutbox`;跨面板的后续动作(会话列表刷新、git/磁盘占用刷新、卡片刷新)经 `AgentHostEvent` 发件箱交给产品。

**Tech Stack:** Rust、`dozer-model` / `dozer-services` / `dozer-client`、`dozer-panel-host`(`NoticeOutbox` / `HostOutbox`)、`dozer-panel-term`、`dozer-i18n`、tokio、iced(仅 `Message` 类型与定时器,不含视图)。

**Spec:** `docs/superpowers/specs/2026-10-18-agent-panel-s6c-design.md`(§6.1、§6.2、§6.5、§6.6 S6c-1)。模板:S5a / S6b 计划的 Dozer 段(worktree、黄金测试、门禁做法)。

## Global Constraints

- **依赖闭包:** `cargo tree -p dozer-panel-agent` 里不得出现 `dozer-app`、`dozer-client`、`dozer-core`、`dozerd`、`wry`、`winit`、`portable-pty`、`russh`(SSH 客户端)。
- **Dozer 用户可见行为不变**;被搬走的测试随代码一起走,**不删、不改期望值**;抽库前后 Rust 测试总数守恒。
- **协议编码不变:** `TermEvent` 搬家后在线上与内存里的序列化、`Request` / `Reply` 的 JSON 与改造前逐字一致(序列化黄金测试)。
- **字节流不重不漏:** attach 返回的快照与后续事件的拼接处与改造前一致(会话字节流黄金测试)。
- **关闭窗口不杀会话:** 只有显式关页签才 `kill` / `close_with_summary`;应用退出只 detach(现有行为,保持)。
- 只用 git tag 依赖;Dozer 仓库内部 `path = "../.."` 依赖除外。**本期不打 tag、不推送、不合并到 `main`**(叠分支规则见 S6 总设计 §0 第 9 条):Digger 不消费,tag 留到整条 Dozer 侧链路验收之后一并发。
- **前置:S6b 已完成并通过审核**(`dozer-panel-term` 已存在,在分支 `feat/panel-term`,顶端 `726f86c7`)。Task 1 不依赖它,可先做,但整份计划都在 S6b 的分支之上做。
- **主工作区别人未提交的改动不动**:Dozer 主工作区有未提交的 `Cargo.lock`、`dozer-app/Cargo.toml`、`Info.plist` 改动。实现在独立 worktree `../dozer-panel-agent`、分支 `feat/panel-agent-1` 里做;**分支从 `feat/panel-term` 的当前顶端切出,不从 `main` 切**:`git worktree add ../dozer-panel-agent -b feat/panel-agent-1 feat/panel-term`(开工前先 `git rev-parse --short feat/panel-term` 确认是 `726f86c7`,若不是,说明有人又动过那条分支,先停下问)。
- **叠分支的基线数字(S6b 顶端 `726f86c7` 实测,开工时在自己的 worktree 里复测并与之核对,不一致先停下查):** `cargo test --workspace --no-fail-fast` = 3379 通过 / 0 失败 / 5 ignored;`cargo test -p dozer-app` = 1286 通过 / 2 ignored;`cargo test -p dozer-panel-term` = 60 通过 / 1 ignored;`cargo clippy --workspace --all-targets` 退出码 0、39 条警告;`python3 scripts/i18n/check_migration.py` 退出码 0 且 `unresolved=0`(`total=2677`);`python3 scripts/audit/check_panel_boundary.py` 通过(213 refs in 5 files)。本计划各处写的"基线"都以这组数字为准,不是 `main` 的数。
- **实现者须知(来自 S6a、S6b 的审核,都是真踩过的坑):**
  1. **`.gitignore` 里的 `examples/` 会吞掉任何 `examples/` 目录下的新文件。** 本计划若新增示例或测试辅助二进制放在 `examples/` 下,提交前用 `git status --ignored --short | grep '^!!'` 确认没有被忽略的源文件(排除 `target`、`node_modules`、`__pycache__`);必要时 `git add -f` 并加 `.gitignore` 例外。
  2. **改了任何 `Cargo.toml` 的依赖,必须同时提交 `Cargo.lock`。** 每个提交后,在干净快照(`git archive HEAD | tar -x -C <临时目录>`)里跑 `cargo metadata --locked --offline` 应退出码 0。
  3. **验收前的测试要在干净快照里再跑一遍**,不能只在自己的 worktree 里跑:被忽略的文件在你的磁盘上存在,会让测试在本地假绿。用 `cargo test -p <crate> --no-fail-fast`(全部目标,会构建 examples),不要只用 `--test <名>`。`cargo test --workspace` 不加 `--no-fail-fast` 时遇到失败的测试二进制就停,后面的二进制不跑,测试总数会偏低,报告里的数字要用 `--no-fail-fast`。
  4. **i18n 清单脚本别覆盖式重新生成 `docs/dozer-v2/i18n/inventory.json`**(`build_inventory.py` 会产生约 2.3 万行无关 diff 并清零 `migrated`)。`check_migration.py` 是在内存里重跑 `build_inventory.build()` 判断的,不读那个文件;新建 `dozer-panel-agent` 后若 `check_migration.py` 报"未分类 UI 候选",是文件归属或排除表(`FILE_TASK`、`EXCLUDED_FILES`、`NON_UI_STRINGS`)没跟上,改脚本里的表,**不要**重新生成清单。
  5. **`cargo fmt --check` 与 `scripts/check-log-scope.sh` 在 `main` 上本来就失败**(9 个既有未格式化的文件;`workspace/preview_panel.rs:233` 一处裸 `tracing::warn!`)。不要为了让它们变绿去改这些既有文件;只要求你**新增/修改的文件**通过 rustfmt、不新增日志门禁命中。
  6. **报告里写的每条"通过""仍为 0""已验证"都要是你刚实跑得到的**,附命令与退出码;审核人会逐条重跑,S6b 的报告就有一条没有重跑就写了"通过",被审核推翻。
- 不引入新的日志依赖。
- **不搬的:** 卡片列表与卡片刷新(`spawn_agent_card_refresh` / `apply_agent_card_refresh` / `agent_card_refresh_plan`)、选择器菜单内容(`agent_picker_items`)、终端条与页签条视图、`terminal.rs`、`TermTarget`、transcript 解析、审阅(`review*`)、`agent_context`、SSH 的连接/泵(`spawn_ssh_tab` 等)——都留到 S6c-2 或留在 `dozer-app`。
- 与设计的一处细化(我的决定,标 D7):设计 §6.2 写 `Custom(mpsc::UnboundedSender<Vec<u8>>)`,实际 SSH 的外送通道同时承载数据与尺寸(`SshOut::{Data, Resize}`),所以库里定义 `TabOut { Data(Vec<u8>), Resize { cols: u16, rows: u16 } }`,`TabBackend::Custom(mpsc::UnboundedSender<TabOut>)`;Dozer 的 `SshOut` 改为 `pub type SshOut = TabOut`。

## Review Focus

1. **会话字节流:** attach 快照 + 乱序到达的输出分块 + 退出,最终 `TerminalModel` 可见内容、`alive`、退出标记行与改造前逐字一致;`Lagged` / `Disconnected` 事件不丢字节也不崩。→ Task 1、4。
2. **关页签的下标与确认:** 关当前页签、关当前之前/之后的页签、关最后一个(之后自动新建)、确认框弹出期间页签列表变化(按 `session id` 而不是下标定位,查不到就什么都不做)。→ Task 4。
3. **迟到的消息:** 页签已关闭后到达的 `TermOutput`、`AgentStateChanged`、`SessionExited`、`TabAttached`(`pending` 已被移除)被忽略,不 panic;**记录现状**:`TabAttached` 迟到时守护进程侧会话不会被 kill(可能泄漏会话),按现状保持并上报,不在本期修。→ Task 4。
4. **创建/attach/初始写失败:** 创建失败 → 带 key 的错误通知、不出页签;attach 失败 → 通知、不出页签(`pending` 清理);启动命令写失败 → 通知但页签保留;跟随文本(待办派发)写失败 → 通知。→ Task 4。
5. **尺寸同步:** 网格缩到 0 或 1、已退出(`alive == false`)的页签不调用 `resize`、`Custom` 后端收到 `TabOut::Resize`、`Daemon` 后端调用 `SessionService::resize`;新页签随 `term_tab_bar_avail_px` 卷入可见窗口。→ Task 4。

---

## Part A — Dozer(仓库 `../dozer`,worktree `../dozer-panel-agent`,分支 `feat/panel-agent-1`)

### Task 1: 基线、黄金测试、`SessionEvent` 与 `SessionService`

**Files:**
- Modify:`crates/dozer-model/src/agent.rs`(新增 `SessionEvent`)、`crates/dozer-model/src/lib.rs`、`crates/dozer-services/src/lib.rs`(新增 `SessionSpec`、`SessionService`、`Services.session`)、`crates/dozer-client/src/lib.rs`(`TermEvent` 改为重新导出、`impl SessionService for Client`)、`crates/dozer-app/src/service_fakes.rs`、`crates/dozer-app/src/host_adapters/services.rs`(`Services` 构造处补 `session`)
- Create:`crates/dozer-app/tests/fixtures/session_stream_golden.txt`
- Test:`crates/dozer-client/src/lib.rs`、`crates/dozer-app/src/workspace/tests.rs`

**Interfaces:**
- Produces:
  - `dozer_model::SessionEvent { Output(Vec<u8>), Exited(Option<i32>), Lagged, Disconnected, Agent { agent: AgentKind, state: AgentState, transcript_path: Option<String> } }`(`Debug`;变体与载荷与原 `TermEvent` 完全一致);`dozer_client::TermEvent` 保留为 `pub use dozer_model::SessionEvent as TermEvent;`。
  - `dozer_services::SessionSpec { pub name: String, pub command: String, pub args: Vec<String>, pub cwd: String, pub cols: u16, pub rows: u16, pub project_id: i64, pub agent: AgentKind }`。
  - `dozer_services::SessionService: Send + Sync`:`list_sessions(&self) -> BoxFuture<'_, Result<Vec<SessionInfo>, ServiceError>>`、`create(&self, spec: SessionSpec) -> BoxFuture<'_, Result<SessionInfo, ServiceError>>`、`attach(&self, id: String, from_offset: u64) -> BoxFuture<'_, Result<(Vec<u8>, u64, tokio::sync::mpsc::UnboundedReceiver<SessionEvent>), ServiceError>>`、`write(&self, id: String, data: Vec<u8>) -> BoxFuture<'_, Result<(), ServiceError>>`、`resize(&self, id: String, cols: u16, rows: u16) -> BoxFuture<'_, Result<(), ServiceError>>`、`kill(&self, id: String) -> BoxFuture<'_, Result<(), ServiceError>>`、`close_with_summary(&self, id: String) -> BoxFuture<'_, Result<(), ServiceError>>`。`Services` 增加 `pub session: Arc<dyn SessionService>`。
  - `dozer_app` 测试替身:`FakeSession`(记录调用、可预置 `attach` 的快照与事件序列、可注入各方法的失败)放进 `service_fakes.rs`,后面各任务复用。

- [ ] **Step 1: 记录基线。** worktree 里 `cargo test --workspace --no-fail-fast > <文件> 2>&1`(输出与退出码分别落盘),按 crate 汇总通过/失败/ignored 数,**并与上面"叠分支的基线数字"逐项核对**(3379 / 0 / 5 等);再单独记 `cargo test -p dozer-app workspace::` 的数量。核对不一致先停下查原因,不要带着不明差异开工。
- [ ] **Step 2: 写两份黄金测试(在改任何代码之前,在 Dozer 现有代码上)。**
  - `crates/dozer-client/src/lib.rs`:`session_protocol_encoding_golden`——对 `Request::CreateSession`、`Request::Attach`、`Request::Write`、`Request::Resize`、`Request::Kill`、`Reply::Created`、`Reply::Attached` 各取一个样例,把 `encode_line` 的输出与一份写死的字符串逐字比较(字符串先在现状上跑出再写死)。
  - `crates/dozer-app/src/workspace/tests.rs`:`session_stream_golden`——用现有的 `Workspace::on_tab_attached(TabAttachedArgs {..})` 装配一个页签(快照取一段含颜色、光标移动、OSC 7 / OSC 133 序列的字节),再对 `tab.ingest_osc` + `tab.model.feed` 依次送入 20 个乱序大小的分块,最后送退出(`alive = false` + `exited_marker()`),把 `visible_lines()`、`cursor()`、`cwd`、`last_exit`、`alive` 序列化,与写进 `crates/dozer-app/tests/fixtures/session_stream_golden.txt` 的内容比对(`UPDATE_GOLDEN=1` 只在这一步生成一次并提交)。**黄金文件在搬迁过程中不得重新生成。**
- [ ] **Step 3: 写失败测试**(`dozer-client`):`session_event_is_the_same_type_as_term_event`(编译期断言 `TypeId` 相等)、`client_implements_session_service_end_to_end`(起一个假的 Unix socket 服务端,依次 `create` / `attach` / `write` / `resize` / `kill`,断言收到的 `Request` 与回复映射;错误经 `into_service_error` 变成 `ServiceError`)。
- [ ] **Step 4: 运行确认失败**,然后实现;`SessionService for Client` 里每个方法只调同名 inherent 方法并 `map_err(into_service_error)`。
- [ ] **Step 5: 验证。** `cargo test --workspace` 总数 = 基线 + 新增;两份黄金测试通过。
- [ ] **Step 6: 提交** `feat(services): SessionService 与 SessionEvent;加会话协议与字节流黄金测试`。

### Task 2: 终端输入翻译与 OSC 扫描搬进 `dozer-panel-term`

**Files:**
- Create:`crates/dozer-panel-term/src/input.rs`(搬自 `crates/dozer-app/src/keymap.rs`)、`src/osc.rs`(搬自 `crates/dozer-app/src/osc.rs`)
- Modify:`crates/dozer-panel-term/src/lib.rs`、`crates/dozer-app/src/keymap.rs`(转调 + winit 适配)、`crates/dozer-app/src/osc.rs`(`pub use`)、`crates/dozer-app/src/platform/window_events.rs`(约 1029–1037 行的调用点)
- Test:随代码搬(`keymap` 11 条、`osc` 的测试)

**Interfaces:**
- Produces:
  - `input::TermKey { Char(String), Named(TermNamedKey) }`,`TermNamedKey` 覆盖 `keymap.rs` 现有分支用到的全部具名键(`Space`、`Enter`、`Backspace`、`Tab`、`Escape`、四个方向键、`Home`、`End`、`PageUp`、`PageDown`、`Delete`、`Insert`、`F1`–`F12` 等,以现有 `match` 的分支为准,**不增不减**);`input::TermMods { pub ctrl: bool, pub alt: bool, pub shift: bool, pub super_key: bool }`(`Copy + Default`)。
  - `input::key_to_bytes(key: &TermKey, mods: TermMods, app_cursor: bool) -> Option<Vec<u8>>`、`input::ime_commit(text: &str) -> Vec<u8>`、`input::dropped_path_bytes(path: &Path) -> Vec<u8>`(三者的输出与原 `keymap::key_to_bytes` / `ime_commit` / `dropped_path_bytes` 完全一致)。
  - `osc::{OscScanner, OscEvent}`(原样搬,`OscEvent::{Cwd(PathBuf), CmdStart, CmdExit(i32)}`)。
  - Dozer 一侧 `keymap.rs`:保留原三个函数的**原签名**(吃 `winit::keyboard::{Key, ModifiersState}`),内部把 winit 值转成中立类型再调库,使 `window_events.rs` 的调用点**不用改**。

- [ ] **Step 1: 写失败测试**(库里新增,其余随代码搬):`every_winit_named_key_handled_by_the_legacy_table_has_a_term_named_key`(在 `dozer-app` 里,遍历旧表里出现过的 winit `NamedKey`,断言转换函数都能映射到 `TermNamedKey`,不会落到"静默丢弃")、`winit_adapter_matches_the_legacy_function_for_a_key_matrix`(对 `{字符键, 空格, 回车, 退格, Tab, Esc, 方向键 × app_cursor 两种, Ctrl+A..Z, Alt+字符, 功能键}` 的矩阵,**旧实现**与**新实现经适配**的输出逐项相等——旧实现的期望值先在现状上跑出再写死)、`osc_scanner_handles_sequences_split_across_chunks`(把一条 OSC 7 在每个字节边界各切一次,事件序列都相同)、`osc_scanner_ignores_malformed_and_oversized_sequences`(无结束符的 `ESC ] 7;`、超长 payload、嵌套 `ESC`,不 panic、不无限缓冲)。
- [ ] **Step 2: 运行确认失败。**
- [ ] **Step 3: 搬代码与测试;`dozer-app` 里留适配。** 注意 `dozer-panel-term` 的依赖闭包**不得**因此引入 `winit`:中立类型就是为了避免它。
- [ ] **Step 4: 验证。** `cargo test -p dozer-panel-term -p dozer-app` 总数守恒(加新增);`cargo tree -p dozer-panel-term | grep -E "winit|dozer-(app|client|core)"` 无输出;会话字节流黄金测试仍通过。
- [ ] **Step 5: 提交。**

### Task 3: 建 `dozer-panel-agent`,搬纯逻辑与测试

**Files:**
- Create:`crates/dozer-panel-agent/Cargo.toml`、`src/lib.rs`、`src/launch.rs`、`src/tabs.rs`、`src/i18n.rs`、`locales/{en,zh-CN}.json`
- Modify:workspace `Cargo.toml`、`crates/dozer-app/Cargo.toml`、`crates/dozer-app/src/workspace/hook.rs`(纯函数改为转调)、`crates/dozer-app/src/workspace/tests.rs`(相关测试随代码搬)、`crates/dozer-app/src/i18n.rs`(owner `dozer.agent` 注册)、`scripts/i18n/{check_migration,build_inventory,scan_sources}.py`、`scripts/audit/check_panel_boundary.py`
- Test:`crates/dozer-panel-agent/src/{launch,tabs}.rs`

**Interfaces:**
- Produces(原样搬自 `workspace/hook.rs` 与 `workspace/state.rs`,只改 `use`):`launch::{PickerLaunch { Agent(Option<AgentKind>), Git }, picker_launch_command(launch: PickerLaunch, hook_exe: &str) -> Option<String>, should_answer_dynamic_color(picked: Option<AgentKind>, info_agent: AgentKind) -> bool, attached_agent(picked: Option<AgentKind>, info_agent: AgentKind) -> AgentKind}`;`tabs::{tab_title(agent: AgentKind, cwd: Option<&Path>, fallback: &str) -> String, tab_display_width(title: &str) -> f32, should_summarize_on_close(agent: AgentKind, alive: bool, backend: &TabBackend) -> bool, exited_marker() -> Vec<u8>}`;`PROJECT_OWNER` 同款的 `AGENT_OWNER: &str = "dozer.agent"`。
- **hook 路径与安装留在 Dozer:** `dozer_hook_binary_path`、`ensure_hook_installed`、`ensure_mcp_installed` 不搬;库只通过 `HookInstaller`(Task 4)调用。

- [ ] **Step 1: 记录** 这批纯函数在 `workspace/tests.rs` 里对应的测试(`tab_title_*`、`attached_agent_*`、`tab_display_width_*`、`should_summarize_on_close_*`、`should_answer_dynamic_color_*`、`picker_launch_command_*`,约 12 条),搬走后总数必须不变。
- [ ] **Step 2: 写失败测试**(库里新增):`should_summarize_on_close_is_false_for_custom_backends_and_dead_tabs_for_every_agent_kind`(对 `AgentKind` 的**每个**变体 × `alive` × `{Daemon, Custom}` 的全组合,断言 `Custom` 与 `!alive` 恒为 `false`;`Daemon && alive` 的结果与现状一致,期望值先在现状上抓出再写死)、`tab_title_is_never_empty`(`fallback` 为空串、`cwd` 为根目录 `/`、`cwd` 含非 UTF-8 的情形)、`picker_launch_command_covers_every_agent_kind_without_panicking`。
- [ ] **Step 3: 建 crate**(依赖闭包见 Global Constraints),搬代码与测试,`workspace/hook.rs` 里的函数改为 `pub(crate) use dozer_panel_agent::...`,调用点不动。搬 `i18n` 与 `locales/`:把 `dozer.host` 下**只被被搬走的代码使用**的键(`error.new_session_failed`、`error.attach_session_failed`、`error.term_initial_write_failed`、`error.term_dispatch_write_failed`,以 `grep` 实测为准)原文搬进 `dozer.agent`,`dozer-app` 里原键若已无调用者则删除;**用户看到的中英文文案逐字不变**。
- [ ] **Step 4: 验证。** `bash scripts/check-i18n.sh` 通过;`cargo test -p dozer-panel-agent -p dozer-app` 总数守恒(加新增);`python3 scripts/audit/check_panel_boundary.py` 通过。
- [ ] **Step 5: 提交。**

### Task 4: 状态、消息、更新

**Files:**
- Create:`crates/dozer-panel-agent/src/{state,update,adapter}.rs`
- Modify:`crates/dozer-app/src/workspace/state.rs`(`Workspace` 里那组字段换成 `agent: AgentPanelState`;`spawn_new_tab` / `on_tab_attached` / `close_tab` / `close_all_tabs_for_switch` / `reorder_term_tab` / `ensure_project_terminal` / `send_input` / `resize_all` / `resize_one` / `tab_by_id_mut` 缩成转调)、`crates/dozer-app/src/workspace/hook.rs`(`forward_events` 搬走)、`crates/dozer-app/src/app/message.rs`(约 20 个变体改为 `Message::Agent(ProjectId, agent::Message)` 一条,其余保留)、`crates/dozer-app/src/app/update.rs`(约 1500–1530、1742–1830、2041–2060 行的分支与 `term_output` / `agent_state_changed` 缩成转调 + 排空发件箱)、`crates/dozer-app/src/host_adapters/agent_panel.rs`(新建:`HookInstaller` 的 Dozer 实现与 `AgentHostEvent` 的处理)、`crates/dozer-app/src/ssh.rs`(`SshOut` 改为 `TabOut` 的别名,`TabBackend::Ssh` 改为 `Custom`)
- Test:`crates/dozer-panel-agent/src/update.rs`

**Interfaces:**
- Produces:
  - `tabs::{SessionTab, TabBackend { Daemon, Custom(mpsc::UnboundedSender<TabOut>) }, TabOut { Data(Vec<u8>), Resize { cols: u16, rows: u16 } }}`。`SessionTab` 字段与原一致(含 `info`、`model: TerminalModel`、`alive`、`agent_state`、`agent`、`transcript_path`、`osc: OscScanner`、`cwd`、`last_exit`、`llm_model`、`permission_mode`、`last_activity`、`workspace_override`、`tab_id`、`forwarder: JoinHandle<()>`、`backend`),`WorkspaceGitInfo` 一并搬入;方法 `effective_cwd(&self) -> PathBuf`、`ingest_osc(&mut self, bytes: &[u8])` 原样。另提供自由函数 `tabs::resize_tab(tab: &mut SessionTab, svc: &Arc<dyn SessionService>, handle: &tokio::runtime::Handle, cols: u16, rows: u16)` 与 `tabs::send_to_tab(tab: &SessionTab, svc: &Arc<dyn SessionService>, handle: &tokio::runtime::Handle, bytes: Vec<u8>)`(**SSH 面板在 Dozer 里直接用它们**,不再有自己的 `resize_one` / `ssh_send_input` 副本)。
  - `state::AgentPanelState`:`new() -> Self`;字段(`pub(crate)` 或访问器,实现者定):`tabs: Vec<SessionTab>`、`active: usize`、`next_tab_id: usize`、`pending: HashMap<usize, JoinHandle<()>>`、`term_tab_first: usize`、`term_tab_overflow_anchor: Option<(f32, f32)>`、`agent_picker_open: bool`、`pending_close_tab: Option<String>`、`picked_agent: Option<AgentKind>`、`outbox: NoticeOutbox`、`requests: Vec<AgentRequest>`、`host_events: Vec<AgentHostEvent>`;方法 `take_outbox() -> Vec<Notice>`、`take_requests() -> Vec<AgentRequest>`、`take_host_events() -> Vec<AgentHostEvent>`、`tab_by_id_mut(&mut self, id: usize) -> Option<&mut SessionTab>`、`active_app_cursor_mode(&self) -> bool`、`active_selection_text(&self) -> Option<String>`。
  - `adapter::{AgentRequest { ShowPicker { at: Option<(f32, f32)> } }, AgentHostEvent { StateChanged { tab_id: usize, agent: AgentKind, state: AgentState, transcript_path: Option<String> }, TurnEnded { tab_id: usize }, SessionExited { tab_id: usize } }, AgentCapabilities { pub summary_on_close: bool, pub mcp_install: bool }(Default 全 false;`AgentCapabilities::all()` 全 true), trait HookInstaller: Send + Sync { fn ensure_hook(&self, agent: AgentKind); fn ensure_mcp(&self, agent: AgentKind); fn hook_exe(&self) -> String; }}`。
  - `update::Message`:覆盖现有约 20 个变体的面板自有部分(`Input(Vec<u8>)`、`Paste(String)`、`Output { tab_id, bytes }`、`Exited { tab_id }`、`AgentState { tab_id, agent, state, transcript_path }`、`SelectTab(usize)`、`SelectTabNoDrag(usize)`、`CloseTab(usize)`、`CloseConfirm`、`CloseCancel`、`PickerToggle`、`PickerClose`、`PickerSelect(PickerLaunch)`、`Attached { tab_id, info: SessionInfo, snapshot: Vec<u8>, picked_agent: Option<AgentKind> }`、`TabDragMove` / `TabDragEnd`、`OverflowToggle` / `OverflowDismiss`、`Failed { key: &'static str, error: String }` 等;**变体名与载荷以搬代码时逐个对应现有 `Message::Term*` / `Tab*` / `AgentPicker*` 为准,不增不减**)。
  - `update::update(state: &mut AgentPanelState, msg: Message, ctx: &UpdateCtx, emit: impl Fn(Message) + Clone + Send + 'static)`,其中 `UpdateCtx<'a> { pub project: &'a ProjectInfo, pub svc: &'a Arc<dyn SessionService>, pub handle: &'a tokio::runtime::Handle, pub cols: u16, pub rows: u16, pub term_tab_bar_avail_px: f32, pub caps: AgentCapabilities, pub hooks: &'a Arc<dyn HookInstaller>, pub locale: &'a str }`。新建会话(原 `spawn_new_tab`)改为 `update::spawn_new_tab(state, ctx, emit, launch: PickerLaunch, follow_up: Option<String>) -> Option<usize>`。
- Consumes:Task 1 的 `SessionService` / `SessionSpec` / `SessionEvent`,Task 2 的 `osc::OscScanner`、`input`,Task 3 的 `launch`、`tabs` 纯函数。

- [ ] **Step 1: 写失败测试**(用 Task 1 的 `FakeSession`,每条独立):
  - `session_stream_through_the_service_matches_the_golden`(经 `FakeSession` 预置"快照 + 20 个分块 + 退出",对 `update` 依次送 `Attached` / `Output` / `Exited`,最终与 `session_stream_golden.txt` 比对——这是本期最重要的守恒依据);
  - `attach_events_are_forwarded_in_order_and_lagged_or_disconnected_do_not_panic`;
  - `closing_a_tab_before_the_active_one_keeps_the_same_tab_active`、`closing_the_active_last_tab_selects_the_previous_one`、`closing_resets_term_tab_first`;
  - `close_with_summary_is_used_only_for_supported_live_daemon_tabs_when_the_capability_is_on_else_kill`(对 `{caps.summary_on_close 真/假} × {alive 真/假} × {Daemon/Custom}`,断言 `FakeSession` 收到的调用是 `close_with_summary` / `kill` / 无);
  - `close_confirm_resolves_by_session_id_not_by_index`(弹确认后用别的操作改变页签顺序,再 `CloseConfirm`,被关的仍是原会话;会话已不存在则什么都不做);
  - `running_or_awaiting_input_tab_asks_for_confirmation_others_close_directly`;
  - `late_messages_for_a_closed_tab_are_ignored`(`Output` / `AgentState` / `Exited` / `Attached` 对不存在的 `tab_id`);
  - `create_failure_pushes_a_keyed_notice_and_adds_no_tab`、`attach_failure_pushes_a_keyed_notice_and_clears_pending`、`initial_launch_write_failure_keeps_the_tab_and_notifies`、`follow_up_write_failure_notifies`(通知的 key 与原 Toast 的 key 一致:`new-session-failed`、`attach-session-failed`、`term-initial-write`、`term-dispatch-write`);
  - `hooks_are_installed_before_the_session_is_created_only_for_a_known_agent`(`HookInstaller` 的调用顺序先于 `create`;`PickerLaunch::Agent(None)` 与 `Git` 不调用;`caps.mcp_install == false` 时不调 `ensure_mcp`);
  - `resize_skips_dead_tabs_and_routes_by_backend`(`Daemon` → `FakeSession::resize`;`Custom` → 通道里收到 `TabOut::Resize`;`alive == false` → 都不发;`(0,0)` 与 `(1,1)` 不 panic);
  - `new_tab_is_scrolled_into_view_when_the_tab_bar_overflows`(沿用原 `tab_window_reveal` 的数值断言);
  - `picker_toggle_pushes_a_show_picker_request_instead_of_opening_a_menu`(`AgentRequest::ShowPicker`,库里不再调原生菜单);
  - `state_change_and_turn_end_are_reported_to_the_host_not_handled_here`(`AgentStateChanged` 后 `take_host_events()` 含 `StateChanged`,回合结束含 `TurnEnded`)。
- [ ] **Step 2: 运行确认失败。** `cargo test -p dozer-panel-agent`。
- [ ] **Step 3: 搬代码,逐处替换耦合。** 清单:`io.client.*` → `ctx.svc.*`(`create_for_agent` 的 8 个形参装进 `SessionSpec`);`proxy.send_event(Message::Toast(..))` → `emit(Message::Failed { key, error })`,`update` 里据此 `outbox.push_keyed(...)`;`proxy.send_event(Message::TabAttached(..))` → `emit(Message::Attached { .. })`;`forward_events(project_id, tab_id, rx, proxy)` → 库内 `forward_events(tab_id, rx, emit)`;`ensure_hook_installed` / `ensure_mcp_installed` / `dozer_hook_binary_path` → `ctx.hooks.*`;`crate::chrome::native_menu::show(agent_picker_items(), cursor)` → `AgentRequest::ShowPicker`;`io.track_exit_critical(task)`(退出前必须等跑完的 kill / 总结任务)→ `ctx` 里加一个 `track_exit_critical: &dyn Fn(JoinHandle<()>)`(由 Dozer 传 `ShellIo::track_exit_critical`),**行为保持:关页签发出的 kill / 总结任务在应用退出前被等待**。
- [ ] **Step 4: Dozer 侧接线。** `host_adapters/agent_panel.rs` 实现 `HookInstaller`(转调 `ensure_hook_installed` / `ensure_mcp_installed` / `dozer_hook_binary_path`,`caps` 传 `AgentCapabilities::all()`);`App` 每次处理 `Message::Agent` 后排空 `take_outbox()`(接 `NoticeOutbox` → 现有 Toast)、`take_requests()`(`ShowPicker` → 现有 `native_menu::show`,选中后回送 `Message::Agent(.., PickerSelect(..))`)、`take_host_events()`(`StateChanged` / `TurnEnded` / `SessionExited` → 原 `agent_state_changed` 里的其余动作:卡片刷新、会话列表刷新、git 与磁盘占用刷新、审阅刷新);`SshOut` → `TabOut`,SSH 的 `spawn_ssh_tab` 等**只改外送通道的类型与 `resize_one` / `ssh_send_input` 改调库函数**,连接与泵逻辑不动。
- [ ] **Step 5: 验证。** `cargo test -p dozer-panel-agent -p dozer-app` 总数守恒(两库之和 = 基线 + 全部新增);会话字节流黄金测试与协议编码黄金测试通过且黄金文件字节未变;`cargo tree -p dozer-panel-agent | grep -E "dozer-(app|client|core)|dozerd|wry|winit|portable|russh"` 无输出。
- [ ] **Step 6: 提交**(提交说明里列出 `Workspace` / `App` 里被替换的字段与函数清单)。

### Task 5: 清理、门禁与验收

- [ ] **Step 1: 清理。** `grep -rn "fn spawn_new_tab\|fn on_tab_attached\|fn close_tab\|async fn forward_events" crates` 只命中 `dozer-panel-agent` 与转调处;`workspace/state.rs` 里不再有被搬走的字段;SSH 面板里没有 `resize_one` / `ssh_send_input` 的重复实现。
- [ ] **Step 2: 依赖闭包与登记。** `cargo tree -p dozer-panel-agent` 满足 Global Constraints;`scripts/audit/check_panel_boundary.py`、`scripts/i18n/*.py` 登记完成。
- [ ] **Step 3: 全量。** `cargo build --workspace && cargo test --workspace && cargo clippy --workspace --all-targets && cargo fmt --check`;`bash scripts/check-i18n.sh`;总数与基线一致(加新增);两份黄金文件字节未变。
- [ ] **Step 4: 目测验收清单(推迟,用户 2026-10-20 决定:整条 Dozer 侧链路 S6a–S6c、S6d-2 Part A、S6e-1 全部完成后一并验收,本期不阻塞)。** 把下面这份清单原样写进报告,供最后验收使用:新建各类 Agent 会话(含选择器)、终端输入输出、页签增删与溢出、拖拽排序、运行中会话的关闭确认、会话退出后的标记行、窗口缩放时终端尺寸随之同步、重启后会话仍在、SSH 面板终端的输入输出与缩放、待办派发文本写入会话、关闭窗口再启动会话仍存活,与改造前一致。**因为不做人工验收,本期的信心完全来自两份黄金测试与测试总数守恒,这些必须真跑通,不能略。**
- [ ] **Step 4b: 干净快照复验。** 在 `git archive HEAD` 解出的临时目录里重跑:`cargo test --workspace --no-fail-fast`(退出码 0,总数与 Step 3 一致)、`cargo metadata --locked --offline`(退出码 0)、`python3 scripts/i18n/check_migration.py`(退出码 0)、`python3 scripts/audit/check_panel_boundary.py`。四项结果写进报告,标明"干净快照"。
- [ ] **Step 5: 提交并留在 `feat/panel-agent-1` 分支上,不合并到 `main`,不打 tag、不推送。** 报告写明:分支基于哪个提交(应为 `726f86c7`)、搬走的字段与函数清单、`TabAttached` 迟到不 kill 会话的现状(若属实)、测试总数、两份黄金测试结论、D7 的落地情况。

---

## Self-Review

- **Spec 覆盖:** 设计 §6.1 `SessionService` / `SessionEvent` → Task 1;§6.5 输入翻译(含决定把 OSC 一并搬进终端库)→ Task 2;§6.2 库边界、`SessionTab` / `TabBackend`、状态/消息/更新 → Task 3、4;§6.3 的 `AgentCapabilities`(本期只需要 `summary_on_close`、`mcp_install` 两项,其余两项 `review_entry`、`send_to_agent` 在 S6c-2 随视图加入)、`HookInstaller` → Task 4;G2(经 `SessionService` 访问)、G5(SSH 共用 `SessionTab`)→ Task 4;§7 的会话字节流与协议编码黄金测试 → Task 1;`TaskTitleSource`、卡片、transcript 尾部解析、`PanelDisplay` → S6c-2。
- **类型一致:** `SessionEvent` / `SessionSpec` / `SessionService` 在 Task 1 定义,Task 4 引用同名;`TermKey` / `TermMods` / `OscScanner` 在 Task 2 定义,Task 3、4 引用;`TabOut` 与 `TabBackend::Custom` 在 Task 4 定义,Dozer 的 SSH 侧经别名使用;`AgentPanelState` / `UpdateCtx` / `Message` 在 Task 4 一次定义;`FakeSession` 在 Task 1 定义,Task 4 的测试复用。
- **风险点:** ① `Message` 变体清单我只核对了 `app/message.rs` 里的名字与 `update.rs` 里的几个分支,载荷需实现者逐个对应,Task 4 Interfaces 已注明"不增不减";② `agent_state_changed` 还牵着会话列表、git、磁盘占用、审阅、卡片刷新五件事,计划用 `AgentHostEvent` 保持它们仍由 Dozer 触发,若发现还有我没列出的联动,先停下报告;③ SSH 的 `ssh_tabs` 与 `ssh_active` 语义和 Agent 页签不同,本期只共用 `SessionTab` 与两个自由函数,不强行合并成一个容器;④ `TabAttached` 迟到不 kill 会话的现状是我从代码推断的,未跑过,Task 4 的迟到消息测试会把它固定下来;⑤ `track_exit_critical` 的语义(退出前等待 kill / 总结任务)是易被丢的细节,Task 4 Step 3 已单列;⑥ 本期不打 tag,S6c-2 一并发。
