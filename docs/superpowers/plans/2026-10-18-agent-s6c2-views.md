# S6c-2 Agent 面板卡片、选择器与面板外壳视图 `dozer-panel-agent`(下) Implementation Plan

> **迁移说明(2026-10-20):** 本文原在 Digger 仓库,现移入 Dozer 仓库;文内 `docs/superpowers/...` 路径均相对 **Dozer 仓库根**。写作「Digger(仓库 `../digger`)」的部分,实施仍在 Digger 仓库的 worktree 里进行,但文档不在那里。Digger 自有文档(`docs/adr/*`、`2026-10-14-window-shell-design.md`、`docs/superpowers/notes/`)仍在 Digger 仓库。最新要求与决定见 `docs/superpowers/specs/2026-10-18-agent-panel-s6-design.md` §0。


> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 Agent 面板的**视图与卡片数据**搬进 `dozer-panel-agent`:会话列表(按 Agent 分组的卡片)、卡片刷新(transcript 尾部解析 + 工作区 git 覆盖)、Agent 选择器的菜单内容、关页签确认框内容、终端条(页签条 + 溢出菜单 + 终端网格 + 空态)。Dozer 改为薄调用并清理旧代码,**用户可见行为不变**;**本期不合并 `main`、不切 tag、不推送**(总设计 §0 第 9 条);Part B(Digger 升级钉版)依赖 tag,本期**不执行**(Digger 此期也不接入面板,接入是 S6d)。

**Architecture:** 视图经 `PanelDisplay<M>`(产品展示适配,形状同 `dozer-panel-usage`)注入产品专属部分:面板头、折叠按钮、页签部件(`panel_tab`、溢出按钮与菜单、拖拽感应层)、区域度量、悬停进度、终端字体与焦点。库自有 `AgentHover` 槽位枚举(同 todo 的 `HoverSlot`),产品把它映射到自己的 `HoverId`。卡片的纯数据先抽成 `CardModel`(可测),再由视图渲染。卡片刷新由库在处理 `AgentState` 时自己触发(S6c-1 里这一项走 `AgentHostEvent`,本期收回库内)。终端条里 Dozer 专属的「发送到 Agent」上下文条经两个插槽(`footer`、`floating`)由产品塞入。

**Tech Stack:** Rust、iced 0.14、byteui(`interaction::{icons, cards, tab_strip}`、`feedback::{status, dialog}`)、`bytegit`、`dozer-panel-term`(`view::view`、`TermStyle`)、`dozer-i18n`、`serde_json`。

**Spec:** `docs/superpowers/specs/2026-10-18-agent-panel-s6c-design.md`(§6.2 视图、§6.3、§6.4、§6.6 S6c-2、§7)。前置计划:`2026-10-18-agent-s6c1-session-state.md`(**已完成并通过审核,顶端 `dc513cbd`,分支 `feat/panel-agent-1`**:`AgentPanelState`、`SessionTab`、`Message`、`adapter.rs` 都由它定义;审核记录 `docs/superpowers/reviews/2026-10-21-s6c1-session-state-review.md`,其观察 1、2 是本期必须收口的遗留,见 Task 3 Step 3b)。

## Global Constraints

- **依赖闭包:** `cargo tree -p dozer-panel-agent` 不得出现 `dozer-app`、`dozer-client`、`dozer-core`、`dozerd`、`wry`、`winit`、`portable-pty`、`russh`。新增允许:`bytegit`、`serde_json`。
- **Dozer 用户可见行为不变**;被搬走的测试随代码走,**不删、不改期望值**;抽库前后 Rust 测试总数守恒(加新增)。
- **卡片文案逐字不变:** 标题 `"{title} ({model}, {mode})"`(只有一项则只带那一项,都无则不带括号);工作行 `"{work}  @{branch}"`(`work` 缺省则只有 `@{branch}`;脏标后缀 `(Uncommitted)`;无分支为 `—`);状态文案四种;截断 60 字符加 `…`。
- **选择器内容不变:** `AgentKind::known()` 派生、按展示名不分大小写排序,分隔线后固定 `Git Shell`、`OS Shell`。
- **i18n 文案逐字不变:** `agent.no_conversations`、`agent.new_session`、`agent.close_tab_title`、`agent.close_tab_description`、`agent.state_{running,awaiting_input,turn_ended,idle}`、`term.no_sessions` 原文搬进 owner `dozer.agent`(按 `grep` 实测);确认框的「取消」「关闭」两个通用键在库内各存一份同文(`close_confirm.cancel` / `close_confirm.confirm`),不让库依赖 `common.*`。
- 只用 git tag 依赖;Dozer 内部 `path = "../.."` 除外。**本期不打 tag、不推送、不合并到 `main`**;这些只在用户一次性人工验收整条 Dozer 侧链路之后、逐项向用户确认再做。
- 主工作区别人未提交的改动不动;实现在独立 worktree `../dozer-panel-agent-2`、分支 `feat/panel-agent-2`,**从 `feat/panel-agent-1` 顶端切出**(叠分支,不是 `main`):先在主仓库确认 `git rev-parse --short feat/panel-agent-1` 为 `dc513cbd`(不是就停下来问),再 `git worktree add ../dozer-panel-agent-2 -b feat/panel-agent-2 feat/panel-agent-1`。命令与路径都带完整 worktree 前缀。
- 不引入新的日志依赖。
- **不搬的:** `agent_context`(「发送到 Agent」上下文条与展开面板)、审阅(`review*`)、对话面板、SSH 面板视图与连接、`TermTarget` / 键盘焦点判定(`keyboard_term_target`)、IME 预编辑状态、原生菜单与确认框的宿主窗口(`platform::confirm_overlay`)、`chrome::tab_widget`(SSH、预览、浏览器还在用)。
- 与设计的细化(我的决定):
  - **D8** 设计 §6.3 的四个能力开关里,`review_entry` 与 `send_to_agent` **不需要**:终端「审阅」按钮已在 P1j 移除(`terminal.rs` 注释),没有对应视图元素;「发送到 Agent」条是终端条里的一个插槽,Digger 不填即不显示。`AgentCapabilities` 保持 S6c-1 的两项。
  - **D9(需要你定,见交付说明)** 设计 D4 要求页签条用 byteui 现成组件;实测 byteui 只有 `tabs::tab_core` 与 `tab_strip::tab_window`,**没有** `panel_tab`、溢出按钮与溢出菜单(它们在 `dozer-app/src/chrome/tab_widget.rs`,并依赖 `controlled_tooltip`、`top_bar_font`、`chrome::menu::shell`)。本计划**不把它们搬进库**:经 `PanelDisplay` 的四个页签方法注入,Dozer 转调现有 `tab_widget`(零视觉风险);Digger 的实现留给 S6d。长期做法是把页签部件整体迁入 byteui(同时服务 S5b-2 P4 的预览页签),那是单独的跨仓库工作,不在本计划。

- **叠分支的基线数字(`dc513cbd`,实测):** worktree 里(含 `.git`)`cargo test --workspace --no-fail-fast` = 3408 通过 / 0 失败 / 5 ignored;`dozer-app` 1261 / 2 ignored;`dozer-panel-term` 78 / 1 ignored;`dozer-panel-agent` 33;`dozer-client` 56;`dozer-services` 24;`check_migration.py` 退出码 0。干净快照(无 `.git`)里另有 3 条 `git_log` dogfood 测试因缺 git 历史失败,属环境原因。clippy 警告数开工时自己记录(Task 1 Step 1),完工不得多于基线。
- **实现者须知(前两期审核里真踩过的坑):** ① `.gitignore` 的 `examples/` 会吞文件,新增的示例/夹具提交后用 `git status --ignored` 核对;② 改依赖必须同时提交 `Cargo.lock`,并在干净快照里验证 `cargo metadata --locked --offline`;③ 完工前把 `git archive HEAD` 解到临时目录,在那里重跑 `cargo test --workspace --no-fail-fast`(预期只有上述 3 条 git_log 环境失败),不要只在自己的 worktree 里跑;④ 绝不覆盖式重新生成 `scripts/i18n/inventory.json`(`check_migration.py` 不读它);⑤ `cargo fmt --check` 与 `scripts/check-log-scope.sh` 在 `main` 上本来就失败,只要求自己改的文件干净(用 `rustfmt --check` 与基线逐文件比 diff 数);⑥ 报告里每条"通过"必须是刚实跑的,附命令与退出码,审核人会逐条重跑;发现与计划不符、基线对不上时先停下来问。
- **验收推迟:** 原计划里"请用户目测验收,未验收不继续"改为"把清单写进报告,不阻塞";信心来自黄金测试与测试数守恒。

## Review Focus

1. **卡片文案组合:** 模型/模式 {有,仅模型,仅模式,无} × 工作内容 {任务标题,transcript 最后一句,无} × 分支 {有,脏,无,工作区覆盖};含中文、emoji、无空格长串、多行文本只取首行、恰好 60 与 61 字符的边界(按字符数不是字节数)。→ Task 1。
2. **transcript 尾部异常:** 文件不存在、空文件、读取非 UTF-8 失败、整行坏 JSON、CRLF、BOM、巨大单行、只有 AI 回复没有人类发言;失败时**保留旧值**(卡片不从有值闪回无值)。→ Task 1。
3. **刷新竞态:** 页签已关闭后到达的 `CardRefreshed` 被忽略;`workspace` 的 `None` 无条件清除覆盖(含从偏离项目根回到根);不存在的页签触发 `AgentState` 时不起刷新任务。→ Task 1。
4. **退化界面状态:** 0 个页签、`active` 越界(关页签后一帧)、`term_tab_first` 越界、溢出锚点存在但页签为空、标题为空串、标题极长;构造视图不 panic,空态文案正确。→ Task 2。
5. **选择器与确认框:** 新增一个 `AgentKind` 自动出现且只出现一次、排序稳定;`Unknown` 不出现;确认框的目标会话已不存在时标题回落「agent」,不 panic。→ Task 1。

---

## Part A — Dozer(仓库 `../dozer`,worktree `../dozer-panel-agent-2`,分支 `feat/panel-agent-2`)

### Task 1: 纯逻辑搬迁:卡片数据、transcript 尾部、样式助手、选择器与确认框内容

**Files:**
- Create:`crates/dozer-panel-agent/src/{transcript_tail,card,style,picker}.rs`、`crates/dozer-panel-agent/locales/{en,zh-CN}.json`(补键)
- Modify:`crates/dozer-panel-agent/{Cargo.toml,src/lib.rs,src/update.rs}`(`AgentState` 分支内触发刷新、新增 `Message::CardRefreshed`)、`crates/dozer-app/src/transcript.rs`(`latest_model_mode_and_activity` 及其私有助手、12 条对应测试搬走,`pub use` 转调)、`crates/dozer-app/src/workspace/{state.rs,hook.rs,view.rs,tests.rs}`(旧函数改为转调或删除)、`crates/dozer-app/src/app/update.rs`(`agent_state_changed` 去掉卡片刷新调用;`AgentCardRefreshed` 分支删除)
- Test:库内各模块;`crates/dozer-app/src/workspace/tests.rs`(随代码搬走的测试)

**Interfaces:**
- Produces:
  - `transcript_tail::latest_model_mode_and_activity(jsonl: &str) -> (Option<String>, Option<String>, Option<String>)`(原样搬,含 `extract_line_activity`、`join_text_blocks`、`truncate_activity`)。
  - `card::{CardCtx<'a> { pub titles: &'a dyn TaskTitleSource, pub project_branch: Option<&'a str>, pub project_dirty: bool }, CardModel { pub title: String, pub work_line: String, pub status: String, pub dot: Color, pub active: bool }, card_model(tab: &SessionTab, active: bool, ctx: &CardCtx<'_>, locale: &str) -> CardModel, work_line(work: Option<&str>, branch: Option<&str>, dirty: bool) -> String, refresh_plan(agent: AgentKind, cwd: &Path, project_root: Option<&Path>) -> (bool, bool, bool), apply_refresh(tabs: &mut [SessionTab], tab_id: usize, llm_model: Option<String>, mode: Option<String>, activity: Option<String>, workspace: Option<WorkspaceGitInfo>), workspace_git_info(cwd: &Path) -> Option<WorkspaceGitInfo>, spawn_refresh(handle: &tokio::runtime::Handle, emit: impl Fn(Message) + Send + 'static, tab_id: usize, agent: AgentKind, transcript_path: Option<String>, cwd: PathBuf, project_root: Option<PathBuf>)}`。`refresh_plan` / `apply_refresh` / `workspace_git_info` 的行为与原 `agent_card_refresh_plan` / `apply_agent_card_refresh` / `workspace_git_info` **逐字一致**。
  - `adapter::TaskTitleSource: Send + Sync { fn task_title_for_session(&self, session_id: &str) -> Option<String>; }`。
  - `update::Message::CardRefreshed { tab_id: usize, llm_model: Option<String>, mode: Option<String>, activity: Option<String>, workspace: Option<WorkspaceGitInfo> }`。
  - `style::{dot_color(state: AgentState, alive: bool) -> Color, agent_state_label(state: AgentState, locale: &str) -> String, format_model_label(raw: &str) -> String, agent_dot_color(agent: AgentKind) -> Color, agent_icon(agent: AgentKind) -> IconKind, group_tabs_by_agent(tabs: &[SessionTab]) -> Vec<(AgentKind, Vec<usize>)>}`(原样搬自 `workspace/hook.rs`、`workspace/view.rs`;`dozer-app` 里原路径改 `pub(crate) use`,用量面板适配与 `conversations` 的调用点不动)。
  - `picker::{PickerEntry { Launch { launch: PickerLaunch, label: String, icon: IconKind, icon_color: Color }, Separator }, picker_entries() -> Vec<PickerEntry>, close_confirm_spec<M: Clone>(state: &AgentPanelState, locale: &str, wrap: impl Fn(Message) -> M) -> Option<ConfirmDialog<M>>}`。
- Consumes:S6c-1 的 `AgentPanelState`、`SessionTab`、`Message`、`UpdateCtx`、`launch::PickerLaunch`、`tabs::tab_title`。

- [ ] **Step 1: 记录基线。** worktree 里 `cargo test --workspace 2>&1 | grep "^test result"`;再记要搬走的测试清单与数量:`transcript.rs` 里 `latest_model_and_mode_*` 与 `activity_*`(12 条)、`workspace/tests.rs` 里 `agent_card_refresh_plan_*`、`apply_agent_card_refresh_*`(3 条)、`dot_color_states`、`agent_state_label_covers_all`、`format_model_label_*`(2 条)、`group_tabs_by_agent_*`(4 条)、`qodercn_is_wired_*`、`agent_picker_*`(3 条)、`agent_dot_color_*`、`agent_icon_*`、`agent_picker_items_*`(2 条)。以 `grep` 实测为准,清单写进提交说明。
- [ ] **Step 2: 在现状上先抽出可测的卡片数据(单独一个提交,不搬位置)。** 在 `workspace/view.rs` 内把 `agent_card` 的文案组装抽成 `card_model` / `work_line`,`agent_card` 与 `work_content_and_workspace_row` 改为用它们。写表驱动测试 `card_model_matches_the_legacy_text_for_every_combination`:覆盖 Review Focus 1 的全部组合,期望字符串**按现状代码手推并写死**,抽取前后(提交前后各跑一次)都必须通过。
- [ ] **Step 3: 写失败测试**(库里新增):
  - `latest_model_mode_and_activity_survives_hostile_input`(CRLF、BOM 开头、整行坏 JSON、10 MB 单行、只有 AI 回复、空串:不 panic,只有 AI 回复时 `activity == None`);
  - `truncate_activity_counts_chars_not_bytes`(60 个汉字不加 `…`,61 个加 `…`,多行只取首行,全空白行得空串);
  - `refresh_keeps_previous_values_when_the_transcript_is_unreadable`(`apply_refresh` 对 `None` 的 model/mode/activity 保留旧值;`workspace` 的 `None` 清除覆盖);
  - `refresh_for_a_closed_tab_is_ignored`;
  - `agent_state_message_requests_a_card_refresh_for_an_existing_tab_only`(对存在的 `tab_id` 恰好触发一次 `spawn_refresh`,不管状态是否 `TurnEnded`;不存在的 `tab_id` 不触发;用 `emit` 收集到的 `CardRefreshed` 断言,`cwd` 在项目根内且 agent 无可提取内容时同步收到全 `None` 的一条);
  - `picker_entries_list_every_known_agent_once_sorted_then_a_separator_then_two_shells`(`Unknown` 不出现;标签不分大小写升序);
  - `close_confirm_falls_back_to_a_generic_label_when_the_session_is_gone`、`close_confirm_is_none_without_a_pending_close`。
- [ ] **Step 4: 运行确认失败**,然后搬代码与测试。`spawn_refresh` 是原 `Workspace::spawn_agent_card_refresh` 的库版:`io.proxy.send_event` → `emit`,`io.handle.spawn` → `handle.spawn`,`project_id` 参数去掉(库状态本就按项目)。`update` 的 `AgentState` 分支里,在写完 `tab.agent_state` 等字段后调用它(`project_root` 取 `ctx.project.path`)。同一提交里删掉 S6c-1 在 `AgentHostEvent::StateChanged` 处理里触发卡片刷新的那一步(其余联动照旧)。
- [ ] **Step 5: 验证。** `cargo test -p dozer-panel-agent -p dozer-app` 总数 = 基线 + 新增;`bash scripts/check-i18n.sh`、`python3 scripts/audit/check_panel_boundary.py` 通过;`cargo tree -p dozer-panel-agent | grep -E "dozer-(app|client|core)|dozerd|wry|winit|portable|russh"` 无输出。
- [ ] **Step 6: 提交**(说明里列出搬走的函数与测试清单)。

### Task 2: 展示适配与视图

**Files:**
- Create:`crates/dozer-panel-agent/src/display.rs`、`src/view/{mod,list,card,terminal,tabs}.rs`
- Modify:`crates/dozer-panel-agent/src/{lib.rs,update.rs}`(`Message::Hover(AgentHover, bool)`)
- Test:`crates/dozer-panel-agent/src/view/*.rs`

**Interfaces:**
- Produces:
  - `display::AgentHover { PickerToggle, TabItem(usize), TabClose(usize), TabOverflow, OverflowRow(usize) }`(`Copy + Eq + Hash`)。
  - `display::PaneStyle { pub gap: f32, pub padding: Padding, pub background: Option<Color> }`;`display::AgentPane { List, Terminal }`。
  - `display::TabArgs<M> { pub title: String, pub active: bool, pub dot: Color, pub item: AgentHover, pub close: AgentHover, pub on_select: M, pub on_close: M }`、`display::OverflowEntry { pub index: usize, pub title: String, pub active: bool, pub dot: Color, pub row: AgentHover }`。
  - `display::PanelDisplay<M>`(产品注入):`wrap(&self, Message) -> M`;`locale(&self) -> &str`;`hover_progress(&self, AgentHover) -> f32`;`hover_tooltip_ready(&self, AgentHover) -> bool`;`pane_style(&self, AgentPane) -> PaneStyle`;`panel_head(&self, icon: IconKind, title: &str, actions: Option<Element<M>>) -> Element<M>`;`list_collapse_button(&self, tooltip_collapse: &str, tooltip_expand: &str) -> Element<M>`;`tab_divider(&self) -> Element<M>`;页签四件:`panel_tab(&self, TabArgs<M>) -> Element<M>`、`overflow_button(&self, tab_count: usize, on_press: M) -> Option<Element<M>>`、`overflow_menu(&self, entries: Vec<OverflowEntry>, anchor: (f32, f32), on_select: fn(usize) -> Message, on_close: fn(usize) -> Message, on_dismiss: Message) -> Element<M>`(产品内部包成 `M`)、`drag_surface(&self, content: Element<M>, index: usize, on_move: M) -> Element<M>`;终端三件:`term_style(&self) -> &TermStyle`、`terminal_focused(&self) -> bool`、`ime_preedit(&self) -> Option<&str>`、`term_event(&self, TermEvent) -> M`;`tab_bar_avail_px(&self) -> f32`。
  - `view::{list::agent_list<'a, M>(state: &'a AgentPanelState, host: &'a impl PanelDisplay<M>, cards: &CardCtx<'a>, width: Length, outer: Border) -> Element<'a, M>, terminal::terminal_pane<'a, M>(state, host, slots: TerminalSlots<'a, M>, width: Length, outer: Border) -> Element<'a, M>, terminal::overflow_popup<'a, M>(state, host) -> Option<Element<'a, M>>}`;`TerminalSlots<'a, M> { pub floating: Option<Element<'a, M>>, pub footer: Option<Element<'a, M>> }`。
- Consumes:Task 1 的 `card_model`、`style::*`、`picker::*`;S6b 的 `dozer_panel_term::view::view` 与 `TermStyle`;S6c-1 的 `AgentPanelState`(`tabs`、`active`、`term_tab_first`、`term_tab_overflow_anchor`)与 `tabs::tab_title` / `tab_display_width`。

- [ ] **Step 1: 写失败测试**(每条独立,用一个测试用 `FakeDisplay` 实现 `PanelDisplay`):
  - `agent_list_with_no_tabs_shows_the_empty_text_and_with_tabs_one_group_header_per_non_empty_kind`(用 `group_tabs_by_agent` 的结果核对分组数;分组标题文案 `"{label}（{n}）"` 全角括号);
  - `card_press_selects_the_tab_without_arming_a_drag`(卡片按下产生的是 `Message::SelectTabNoDrag(idx)`,不是 `SelectTab`——2026-08-17 修过的真实 bug,在视图层固定下来);
  - `tab_bar_hides_the_overflow_button_for_zero_tabs_and_shows_it_for_one`;
  - `tab_window_uses_the_hosts_available_width_and_term_tab_first`(窄宽度下可见页签是 `tab_window` 给出的区间);
  - `degenerate_states_build_without_panicking`(0 页签、`active` 越界、`term_tab_first` 越界、溢出锚点有但页签空、空标题、10 000 字符标题:构造 `agent_list` / `terminal_pane` / `overflow_popup` 不 panic,`overflow_popup` 在页签空时为 `None`);
  - `nonzero_last_exit_is_shown_for_the_active_tab_only`(`last_exit == Some(0)` 与 `None` 不显示,非零显示 `exit {code}`);
  - `terminal_pane_without_tabs_shows_the_no_sessions_text`;
  - `slots_are_placed_floating_over_the_grid_and_footer_below_it`(用带标记的假元素,断言两个插槽都进入视图树且 `floating` 不改变网格的占位)。
- [ ] **Step 2: 运行确认失败。**
- [ ] **Step 3: 实现。** 视图逻辑从 `workspace/view.rs`(`agent_list_pane`、`agent_card`、`agent_picker_toggle_button`)与 `term/terminal.rs`(`terminal_pane`、`tab_bar`、`term_tab_overflow_popup`、`tab_item`、`active_tab_view`)原样搬,只做耦合替换:`app.hover_progress(HoverId::…)` → `host.hover_progress(AgentHover::…)`;`tab_widget::*` → 对应的四个页签方法;`theme::region::*()` → `host.pane_style(..)`;`home_panel_head_with_actions` → `host.panel_head`;`app.list_collapse_button(..)` → `host.list_collapse_button`;`Message::*` → 库 `Message` 经 `host.wrap`;`term_view::view` → `dozer_panel_term::view::view(.., host.term_style(), |t, e| host.term_event(e))`;`ws.todo.task_title_for_session` / `ws.project_panel.branch()` → `CardCtx`。度量、间距、颜色、控件顺序逐字保持。
- [ ] **Step 4: 验证。** `cargo test -p dozer-panel-agent`;依赖闭包同 Task 1。
- [ ] **Step 5: 提交。**

### Task 3: Dozer 接线与清理

**Files:**
- Modify:`crates/dozer-app/src/host_adapters/agent_panel.rs`(`PanelDisplay` 与 `TaskTitleSource` 的 Dozer 实现)、`crates/dozer-app/src/workspace/view.rs`(删除已搬函数,只留 `review_*`、`preview_*` 等)、`crates/dozer-app/src/term/terminal.rs`(留 `TermTarget`、`keyboard_term_target`、一个调用库的薄 `terminal_pane`)、`crates/dozer-app/src/app/view.rs`(约 25、243、269、899 行的调用点)、`crates/dozer-app/src/app/update.rs`(`AgentPickerToggle` 的选择器内容改取 `picker_entries()`)、`crates/dozer-app/src/platform/confirm_overlay.rs`(约 193 行改取 `close_confirm_spec`)、`crates/dozer-app/src/app/message.rs`
- Test:`crates/dozer-app/src/workspace/tests.rs`、`crates/dozer-app/src/host_adapters/agent_panel.rs`

**Interfaces:**
- Consumes:Task 1、2 的全部产出。
- Produces:`HoverId` 映射函数 `fn hover_id(slot: AgentHover) -> HoverId`(`PickerToggle` → `HoverId::named(PanelKind::Agent, "picker_toggle")`,`TabItem(i)` → `HoverId::TermTabItem(i)`,`TabClose(i)` → `HoverId::TermTabClose(i)`,`TabOverflow` → `HoverId::TermTabOverflow`,`OverflowRow(i)` → `HoverId::TabOverflowRow(i)`),以及把库的 `Message::Hover(slot, h)` 映回 `Message::Hover(hover_id(slot), h)`。`PickerEntry` → Dozer 的 `MenuSpec`(原生菜单与 iced 兜底两条路共用同一份转换)。

- [ ] **Step 1: 写失败测试:** `hover_slots_map_one_to_one_to_the_legacy_hover_ids`(五种槽位 × 若干下标,互不碰撞、与旧 `HoverId` 逐项相等);`picker_menu_spec_matches_the_legacy_eleven_rows`(沿用 `agent_picker_items_has_eleven_rows_matching_old_picker` 与 `…separator_splits…` 两条的断言,改成走新转换后仍通过);`tab_drag_armed_reflects_the_terminal_group_only`(`dragging_group(TabGroup::Terminal)` 才为真)。
- [ ] **Step 2: 实现适配。** `PanelDisplay` 的页签四件转调现有 `chrome::tab_widget::{panel_tab, tab_overflow_button, tab_overflow_menu}` 与 `terminal::tab_drag_surface`;`pane_style` 读 `theme::region::{agent_list_pane, terminal_pane}`;`term_event` 复用 S6b 的 `map`(带 `TermTarget::Shared`);`terminal_focused` 与 `ime_preedit` 沿用 `keyboard_term_target(..) == TermTarget::Shared` 的原判断。`TerminalSlots` 里 `floating` 与 `footer` 填 `agent_context::expanded_panel` 与 `agent_context::strip`(包含原先 `STRIP_HEIGHT + region.gap` 的底部内边距计算),Digger 不填。
- [ ] **Step 3: 替换调用点并删除旧代码。** `grep -rn "fn agent_list_pane\|fn agent_card\|fn agent_picker_spec\|fn agent_picker_popup\|fn agent_close_confirm_spec\|fn tab_item\|fn tab_bar\|fn active_tab_view\|fn term_tab_overflow_popup\|fn spawn_agent_card_refresh\|fn apply_agent_card_refresh" crates` 只命中 `dozer-panel-agent` 与薄转调处。`Message::AgentCardRefreshed` 删除。
- [ ] **Step 3b: 收口 S6c-1 遗留(审核观察 1、2,必做)。** (a) S6c-1 报告偏差 3:库 `update` 与 Dozer 侧仍并存两套实现(`app/update.rs` 的 `term_output`、`SessionExited`、`SelectTab` / `CloseTab` / `TermScroll` 等 UI 发起的变体仍走 Dozer 自己的处理)。把这些变体改为经 `Message::Agent` 转库 `update`,删除 Dozer 侧重复处理函数;`ssh_tabs` 相关与 `on_tab_attached` 的 SSH/恢复装配保持原样(SSH 不属 `AgentPanelState`)。每删一个处理函数,确认它原有的测试已在库里有等价覆盖或随之搬走,**测试名不减**。(b) `UpdateCtx.locale` 无人读取、Dozer 侧写死 `"en"`:删除该字段(文案由 `LocalizedText` 在 Toast 层解析);若实现中发现确有读取需求,先停下报告。(c) 两份 S6c-1 黄金测试(协议编码、会话字节流)与夹具 `session_stream_golden.txt` 字节不得改动,收口前后都要过。
- [ ] **Step 4: 验证。** `cargo test --workspace` 总数 = 基线 + 全部新增;两份 S6c-1 黄金文件字节未变;`cargo clippy --workspace --all-targets`、`cargo fmt --check`、`bash scripts/check-i18n.sh`、`python3 scripts/audit/check_panel_boundary.py`。
- [ ] **Step 5: 提交**(列出被替换/删除的函数清单)。

### Task 4: 门禁、干净快照复验与报告(不合并、不打 tag)

- [ ] **Step 1: 全量。** `cargo build --workspace && cargo test --workspace --no-fail-fast && cargo clippy --workspace --all-targets`;测试总数 = 基线 3408 + 新增,失败 0;clippy 警告数不多于 Task 1 Step 1 记录的基线;`cargo fmt --check` 只许出现 `main` 上既有的文件;`cargo tree -p dozer-panel-agent` 满足 Global Constraints。
- [ ] **Step 2: 目测验收清单写进报告,不阻塞**(整条链路完成后用户一次性验收,与改造前并排对照):空列表文案;新建各类 Agent(＋按钮悬停变金、macOS 原生选择器菜单内容与顺序);卡片三行(标题含模型与模式、工作内容与 `@分支`、状态点与文字)、选中高亮、悬停;多种 Agent 分组及组标题计数;`cd` 出项目根后卡片分支随之变化、`cd` 回来恢复;页签条(状态点颜色、关闭×、溢出下拉的箭头与定位、窄窗口下翻页窗口化、拖拽换位);非零退出码提示;关运行中会话的确认框文案;「发送到 Agent」条与展开面板仍在终端底部;窗口缩放;中英文切换后各处文案。
- [ ] **Step 3: 干净快照复验。** `git archive HEAD` 解到临时目录:`cargo metadata --locked --offline`、`cargo test --workspace --no-fail-fast`(预期仅 3 条 git_log 环境失败)、`python3 scripts/i18n/check_migration.py`、`python3 scripts/audit/check_panel_boundary.py`。
- [ ] **Step 4: 报告与提交。** 报告写到 `docs/superpowers/reviews/2026-10-<日期>-s6c2-views-report.md`,写明:基线核对、每个任务的实测命令与退出码、搬走的函数与测试清单、测试总数、D8、D9 的落地情况、Step 3b 三项的落地、Step 2 清单。提交后**停在 `feat/panel-agent-2`**:不合并 `main`、不打 tag、不推送。

## Part B — Digger(**本期不执行**:依赖 `libs-*` tag,tag 要等整条链路验收后才切;留到 tag 之后另行安排)

### Task 5: 同日升级 Digger 的 dozer libs 钉版

**Files:**
- Modify:`Cargo.toml`(各 `dozer-*` 库的 `tag`)、`Cargo.lock`

- [ ] **Step 1: 改钉版。** 把 Digger 里所有 dozer libs 依赖的 `tag` 改为新 tag(与同日约定一致,全部一起改)。本期 Digger 不依赖 `dozer-panel-agent`,所以只会引入既有库的版本变化。
- [ ] **Step 2: 验证。** `cargo update --workspace`(只刷新钉版相关项)后跑 Cargo.lock 重复检查(同一 crate 不得出现两个版本)与 `scripts/check-shared-deps.sh`;`cargo build --workspace && cargo test --workspace && cargo clippy --workspace --all-targets && cargo fmt --check`。
- [ ] **Step 3: 提交** `chore(deps): align digger with dozer libs <tag>`。

---

## Self-Review

- **Spec 覆盖:** §6.2 视图(列表、卡片、选择器、终端条)→ Task 2;选择器内容与确认框内容 → Task 1;§6.3 `TaskTitleSource`、`PanelDisplay` → Task 1、2,能力开关两项由 S6c-1 提供、另两项按 D8 改为插槽;§6.4 transcript 尾部解析 → Task 1;§7 的测试守恒、依赖闭包、用户目测验收 → Task 4;§6.6 的 tag → Task 4 Step 4;同日升级 → Task 5。设计 D4(用 byteui 页签组件)无法按原样执行,见 D9。
- **类型一致:** `AgentHover`、`PanelDisplay`、`TabArgs`、`OverflowEntry`、`TerminalSlots`、`CardCtx`、`CardModel` 在 Task 1、2 一次定义,Task 3 引用同名;`Message::CardRefreshed` 与 `Message::Hover` 在 Task 1、2 加入 S6c-1 的 `Message`;`TaskTitleSource` 放在 S6c-1 的 `adapter.rs`。
- **风险点:** ① 视图里 `Message` 变体名以 S6c-1 实际落地为准(`SelectTabNoDrag`、`TabDragMove`、`OverflowToggle` 等),实现者对照 S6c-1 的合并结果;② `overflow_menu` 的回调用 `fn(usize) -> Message`,若产品需要捕获环境,先停下报告,不要改成带分配的装箱方案而不说明;③ `PanelDisplay` 的方法数偏多(页签四件、终端四件),是 D9 的直接代价,B 方案(页签部件迁入 byteui)落地后会缩小;④ 卡片刷新从 `AgentHostEvent` 收回库内,是对 S6c-1 的一处改动,Task 1 Step 4 在同一提交里改,以免两处重复触发;⑤ `common.cancel` / `common.close` 在库内各存一份同文,以后若通用键统一再合并;⑥ tag 与推送是对外动作;⑦ 展示适配里 `floating` 插槽的定位由产品负责,Digger 不填就没有该层。
