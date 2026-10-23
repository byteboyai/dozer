# S6d-3 页签外观、溢出菜单、拖拽换位、对照验收与发布 Implementation Plan

> **迁移说明(2026-10-20):** 本文原在 Digger 仓库,现移入 Dozer 仓库;文内 `docs/superpowers/...` 路径均相对 **Dozer 仓库根**。写作「Digger(仓库 `../digger`)」的部分,实施仍在 Digger 仓库的 worktree 里进行,但文档不在那里。Digger 自有文档(`docs/adr/*`、`2026-10-14-window-shell-design.md`、`docs/superpowers/notes/`)仍在 Digger 仓库。最新要求与决定见 `docs/superpowers/specs/2026-10-18-agent-panel-s6-design.md` §0。


> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 S6d-2 的朴素页签换成 Dozer 同款外观,补齐溢出菜单、拖拽换位、悬停高亮与延迟提示,与 Dozer 逐状态对照验收,做压力与同机并存回归,**完成 Agent 面板迁移并发布 Dozer 与 Digger 各一个新版本**(项目硬性要求)。

**Architecture:** 全部在 `digger-desktop`。`adapters/agent_tabs.rs` 实现 `PanelDisplay` 的页签四件(`panel_tab`、`overflow_button`、`overflow_menu`、`drag_surface`),外观对照 Dozer `chrome/tab_widget.rs`,用 byteui 的 `tabs::tab_core` 与 `tab_strip::{tab_window, tab_drag_past_threshold}` 搭;菜单外框复用 S6d-2 的 `adapters/menu_overlay.rs`(byteui 没有菜单渲染)。悬停用「阶跃」代替动画(进入即满、离开即空),延迟提示借用现有的 1 秒 `Tick`。拖拽状态机放在 `AgentPanel`(Dozer 里它在 `App`,是跨页签组共享的)。新增 `examples/agent_gallery.rs`:不连守护进程,用预置的假会话把面板摆到固定状态,供与 Dozer 对照截图。

**Tech Stack:** Rust、iced 0.14、byteui、`dozer-panel-agent`(`PanelDisplay`、`AgentHover`、`TabArgs`、`OverflowEntry`)、tokio。

**Spec:** `docs/superpowers/specs/2026-10-18-agent-panel-s6d-design.md`(§6.2 页签四件、§6.6 溢出菜单、D3、D4、§7 S6d-3、§8)。前置:S6d-1、S6d-2 已合并。

## Global Constraints

- **库不改。** 缺口(尤其是拖拽状态归属,见 Task 3 Step 1)只能报告,不在 Digger 里绕开库的逻辑。
- **外观以 Dozer 为准:** 度量取自 Dozer 现状,逐项抄值(写进测试):页签内边距 左 10 / 右 4 / 上下 0(`PANEL_TAB_PAD_LEFT/X/Y`);选中态 `card` 底 + 1px `border` + 圆角 6;悬停态 `tab_hover` 底 + 圆角 6;标题 `font::body()`、`wrapping None`、`clip`;关闭按钮边长 `geometry::tab_button_size()`,颜色 `dim`→`gold` 随悬停;溢出按钮图标 `ChevronsDown`、命中区 `tab_arrow_button_size()`、页签条里排在最前;溢出菜单宽度 = `clamp(窗口宽 − 锚点 x − 24, 220, 560)`、最大高度 320、行内边距 4、行间距 2、选中行前有固定宽度的 `>`(`ChevronRight`,`cream`)列、行关闭按钮常显;页签条可用宽度 = 终端块宽 − 72(两颗按钮 32×2 + 间距 4×2)。
- **已知且接受的差异(设计 D3):** 悬停是阶跃不是渐变;延迟提示精度是 1 秒(悬停满 2 秒后的下一次 `Tick` 才出现)。任何别的差异都要记进对照清单,不得悄悄存在。
- 溢出菜单、选择器、确认框同一时刻只允许一个;Esc 关最上层;确认框打开时终端不收键盘输入。
- **发布要求(项目硬性规定):** Agent 面板迁移完成后,Dozer 与 Digger 各重新发布一个版本(Dozer:`scripts/build-macos-app.sh release`;Digger:`scripts/release.sh`),装到本机启动确认。升版本、提交版本号、安装都是对外动作,**先向用户确认再执行**。
- 主工作区别人未提交的改动不动;worktree `../digger-s6d3`、分支 `feat/agent-chrome`(从已含 S6d-2 的 `main` 切出)。

## Review Focus

1. **页签条的极端内容:** 1 个页签、50 个页签、空标题、超长标题、CJK 与 emoji 标题;溢出菜单打开时关闭其中一个页签(列表缩短、锚点不变)、关到只剩 0 个时菜单自动关闭;窗口缩窄到页签条可用宽度为 0。→ Task 2。
2. **拖拽的边缘:** 拖过阈值才换位;松手点在窗口外;拖动中被拖的会话退出/被关;拖动中切项目或切走面板;输出持续涌入时页签下标变化;单个页签时拖拽无效。→ Task 3。
3. **悬停残留:** 被悬停的页签关闭或换位后悬停记录失效;鼠标离开窗口不留高亮;提示在悬停对象消失后不残留。→ Task 1。
4. **覆盖层互斥与层级:** 选择器开着点溢出按钮、溢出菜单开着点「＋」、确认框开着按 Esc;覆盖层靠近窗口右/下边缘;右栏切换面板时覆盖层随之关闭。→ Task 2。
5. **主题与缩放:** 切换明暗主题(若 Digger 支持)与 `icon_size::scale()` 非 1 时,页签、菜单度量随之缩放且不重叠。→ Task 2、4。

---

### Task 1: 悬停、光标与延迟提示

**Files:**
- Modify:`crates/desktop/src/adapters/agent.rs`、`crates/desktop/src/adapters/agent_display.rs`、`crates/desktop/src/product.rs`(`Msg::AgentCursor(Point)`、订阅里的 `CursorMoved` / `CursorLeft`、`Tick` 分支)
- Test:`agent_display.rs`、`agent.rs`

**Interfaces:**
- Produces:`AgentPanel` 新增 `hover: Option<(AgentHover, Instant)>`、`cursor: Point`;`AgentPanel::on_hover(&mut self, slot: AgentHover, entered: bool, now: Instant)`(进入记录、离开清除,仅当离开的正是当前记录的槽位时才清);`AgentPanel::clear_hover()`;`PanelDisplay::hover_progress(slot)` = 当前悬停槽位等于 `slot` 时 `1.0` 否则 `0.0`;`hover_tooltip_ready(slot)` = 同槽位且 `now − 进入时刻 ≥ 2s`;`AgentPanel::tooltip_deadline(&self) -> Option<Instant>`(供 `Tick` 判断是否需要重绘,没有悬停时为 `None`,避免无谓重绘)。库的 `Message::Hover(slot, h)` 在 `update` 里转 `on_hover`。
- [ ] **Step 1: 写失败测试:** `entering_a_slot_makes_its_progress_one_and_others_zero`、`leaving_a_slot_that_is_not_the_current_one_changes_nothing`(防止「新进入先于旧离开」的事件顺序清掉新悬停)、`tooltip_is_ready_only_after_two_seconds_on_the_same_slot`(注入时钟)、`removing_or_reordering_tabs_clears_a_hover_whose_index_no_longer_exists`、`cursor_leaving_the_window_clears_hover`、`no_hover_means_no_tooltip_deadline`。
- [ ] **Step 2: 运行确认失败,实现,验证(`cargo test -p digger-desktop agent_`),提交。** 现有 1 秒 `Tick` 在 `tooltip_deadline` 为 `Some` 时才触发重绘,其余情况不变。

### Task 2: 页签四件、溢出菜单与覆盖层互斥

**Files:**
- Modify:`crates/desktop/src/adapters/agent_tabs.rs`(从 S6d-2 的朴素版换成 Dozer 同款;若 S6d-2 把朴素版放在 `agent_display.rs` 里,则新建本文件并删掉朴素版)、`crates/desktop/src/adapters/menu_overlay.rs`(补溢出菜单需要的 `shell(rows, width)` 外框函数)、`crates/desktop/src/adapters/agent.rs`(覆盖层状态)、`crates/desktop/src/product.rs`(覆盖层叠放、Esc)
- Test:`agent_tabs.rs`、`agent.rs`、`product.rs`

**Interfaces:**
- Produces:
  - `agent_tabs::{panel_tab, overflow_button, overflow_menu, drag_surface}` 实现 S6c-2 的 `PanelDisplay` 签名;纯函数 `agent_tabs::overflow_geometry(anchor: (f32, f32), window: (f32, f32), scale: f32) -> OverflowGeometry { x: f32, y: f32, width: f32, max_height: f32 }`(按 Global Constraints 的公式:宽度夹在 220–560、`x` 钳在 `[0, window.w − width]`、`y` 钳在 `[0, window.h − max_height]`,常量乘 `scale`)与 `agent_tabs::tab_bar_avail_px(block_width: f32, scale: f32) -> f32`(`block_width − 72·scale`,不小于 0)。
  - `agent_tabs::controlled_tooltip<'a, M>(content, label: String, show: bool) -> Element`:`show` 为假返回裸内容,为真包进 `Tooltip`(位置 `Top`、间距 4、气泡 `size 12` `cream` 字、内边距 `[5, 9]`、`icons::tooltip_bubble_style()`)。
  - `AgentPanel` 新增 `overlay: Option<Overlay>`,`Overlay { Picker { at }, Overflow { anchor }, }`(确认框由库的 `pending_close_tab` 决定,不放在这里);`open_overlay(o)` 先关掉已有的再开,保证互斥;`close_overlay()`;面板不可见或切项目时调用 `close_overlay()`。
- [ ] **Step 1: 写失败测试:** `overflow_geometry_clamps_width_between_220_and_560`(锚点靠右、靠左各一)、`overflow_geometry_keeps_the_menu_inside_the_window_at_every_corner`、`overflow_geometry_scales_with_icon_size_scale`、`tab_bar_avail_is_block_width_minus_72_and_never_negative`;`tab_title_colors_follow_active_and_hover`(选中恒 `cream`;未选中按 `hover_progress` 在 `dim`→`gold` 间取值,0 与 1 两端)、`close_button_is_invisible_at_rest_and_gold_on_hover`;`overflow_menu_has_one_row_per_tab_with_the_chevron_only_on_the_active_row`(所有行文字起点相同)、`closing_a_row_keeps_the_menu_open_with_one_fewer_row_and_closes_it_at_zero`、`dismiss_layer_press_closes_the_menu`;`overlays_are_mutually_exclusive`(依次开 Picker、Overflow,只剩后者)、`escape_closes_the_topmost_overlay_then_nothing_else`、`confirm_dialog_blocks_terminal_keys`、`switching_panels_or_projects_closes_overlays`;`panel_tab_with_empty_or_huge_titles_builds_without_panicking`(0 字、10 000 字、CJK、emoji)。
- [ ] **Step 2: 运行确认失败,实现。** 度量常量集中放在 `agent_tabs.rs` 顶部并注明来源(Dozer `tab_widget.rs` 的对应常量);溢出菜单的行用 `tabs::tab_core`,行背景用与横向页签同一份容器样式函数(不再给选中行画常驻方框)。
- [ ] **Step 3: 验证与提交。** `cargo test -p digger-desktop`;沙箱真机截图:5 个页签、窄窗口下的翻页窗口化、溢出下拉、悬停胶囊、关闭按钮显形、提示气泡。

### Task 3: 拖拽换位

**Files:**
- Modify:`crates/desktop/src/adapters/agent.rs`、`crates/desktop/src/adapters/agent_tabs.rs`(`drag_surface`)、`crates/desktop/src/product.rs`(订阅里的左键松开、窗口失焦)
- Test:`agent.rs`

**Interfaces:**
- Produces:`AgentPanel` 新增 `drag: Option<TabDrag { source: usize, press_pos: Point }>`;`on_select_tab(pid, idx)`(库的 `SelectTab` 到达时记录 `drag = Some(TabDrag { source: idx, press_pos: cursor })`;`SelectTabNoDrag` 不记);`on_drag_move(pid, to)`:无 `drag`、未越过 `tab_drag_past_threshold(press_pos, cursor)`、`to == source`、`to` 越界 → 什么都不做;否则调库的页签换位(`reorder_term_tab(source, to)`),并把 `drag.source = to`(续拖以新位置为准);`on_pointer_released()` 与 `clear_drag()`(失焦、切项目、切走面板、被拖的会话被关时调用)。
- [ ] **Step 1: 接缝核对。** 确认拖拽状态由谁持有:若库的 `Message::TabDragMove` / `TabDragEnd` 已自带状态机,则本任务只转发消息并删掉上面的 `drag` 字段(报告写明);若库只暴露 `reorder_term_tab`,按 Interfaces 在 Digger 持有状态。**两者都不满足(没有换位入口)就停下报告。**
- [ ] **Step 2: 写失败测试:** `select_tab_arms_the_drag_but_select_without_drag_does_not`(卡片点选不会因光标划过页签条而换位——Dozer 2026-08-17 修过的真实 bug)、`no_reorder_before_the_movement_threshold`、`reorder_moves_the_tab_and_keeps_the_dragged_tab_under_the_cursor`(拖 0→2 再 2→1,顺序与激活页签符合库的语义)、`out_of_range_targets_are_ignored`、`release_clears_the_drag_even_when_released_outside_the_window`、`closing_the_dragged_tab_or_switching_project_clears_the_drag`、`a_single_tab_can_not_be_dragged`、`hover_is_cleared_after_a_reorder`。
- [ ] **Step 3: 运行确认失败,实现,验证,提交。** 沙箱真机:拖拽换位流畅、松手后顺序保持、拖拽时终端仍在收输出。

### Task 4: 画廊与 Dozer 逐状态对照

**Files:**
- Create:`crates/desktop/examples/agent_gallery.rs`、`docs/superpowers/notes/2026-10-18-agent-parity.md`
- Test:无(人工对照为主);画廊里的假会话数据放在 `agent_gallery.rs`

**Interfaces:**
- Produces:画廊接受 `--state <name>`:`empty`、`one-shell`、`grouped`(Claude ×2、Codebuddy ×1、OS Shell ×1,状态点覆盖 Running / AwaitingInput / TurnEnded / Idle / 已退出)、`active-card`、`many-tabs`(12 个页签)、`narrow`(窄窗口)、`overflow-open`、`picker-open`、`confirm-open`、`notice`、`daemon-unavailable`。用 `AgentPanel` + 假 `SessionService`(预置快照与事件)构造,不连 `diggerd`。
- [ ] **Step 1: 写画廊。** 每个状态一个函数,构造 `AgentPanelState` 后经 `adopt_restored` 注入假会话;卡片文字(模型、模式、工作内容、分支)通过假 transcript 文件与 `CardCtx` 取得。
- [ ] **Step 2: 逐状态截图对照。** 对每个状态,我在沙箱里启动画廊并截图;用户在 Dozer 里摆出同一状态(Dozer 一侧由用户操作,我只给出每个状态的摆法清单)并截图。把两边并排比较,差异逐条记进 `agent-parity.md`:`状态 | 差异 | 原因 | 处理(修 / 接受并写理由)`。已接受的只能是 Global Constraints 里列出的两条;其余必须修。
- [ ] **Step 3: 修差异,重复 Step 2 直至清单里没有「待修」。** 度量调整只改 Digger 一侧的 `pane_style` 与 `agent_tabs.rs` 常量,不改库。
- [ ] **Step 4: 提交(含对照文档与关键截图路径)。**

### Task 5: 压力、容错与同机并存回归

- [ ] **Step 1: 压力**(沙箱真机,观察 UI 是否卡顿、CPU 占用):`yes | head -200000`;`cat` 一个约 50 MB 的文本;输出持续涌入时拖分隔线与缩放窗口;同时开 20 个页签;5 个项目各 3 个会话来回切换;`vim` 全屏进出;窗口最小化再恢复。出现卡顿先判断是库(Dozer 一侧同样会卡)还是 Digger 适配,**库的问题报告给用户,不在 Digger 里加限流绕过**。
- [ ] **Step 2: 守护进程中途死亡。** 会话在跑时 `kill diggerd`:页签标记已退出、面板顶部出现通知、不崩、不反复重连;重新拉起 `diggerd` 后点「重试」恢复可用;旧页签保持已退出可关闭。
- [ ] **Step 3: 同机并存**(设计 §8):Dozer 与 Digger 同时运行,各开一个 Claude 会话,两边状态点各自随 hook 更新、互不影响;Dozer 会话里触发 `digger-hook` 无效果、反之亦然;两个 `~/.claude/settings.json` 条目并存且重复启动不增加。
- [ ] **Step 4: 输入法与中文。** 依 S6d-1 探针结论,真机输入一整句中文、含候选框位置;终端里显示中文与 emoji 行不消失(字体剔除生效)。
- [ ] **Step 5: 记录** 每项结果、截图路径、发现的问题与处置,写进 `docs/superpowers/notes/2026-10-18-agent-regression.md`。任一项不符先停下,不要为了「过」而改验证。

### Task 6: 文档

- [ ] **Step 1:** `docs/superpowers/specs/2026-10-18-agent-panel-s6-design.md`、`…-s6c-design.md`、`…-s6d-design.md` 的状态改为「已实现」并补「实现结果」(含 Part A 补丁、IME 探针结论、已接受的差异清单);`2026-10-14-window-shell-design.md` 的 S6 行同步;`docs/adr/ADR-015-product-owned-daemon.md` 补一节「Agent 面板」;在 ADR-014 的 Consequences 里加一句 Agent 面板共享库。
- [ ] **Step 2:** 在笔记里写明「页签部件与菜单弹层仍是 Digger 里临时复制的一份(`agent_tabs.rs`、`menu_overlay.rs`),应迁入 byteui」,作为后续跨仓库任务的起点。

### Task 7: 门禁、发布与合并

- [ ] **Step 1: 全量。** `cargo build --workspace && cargo test --workspace && cargo clippy --workspace --all-targets && cargo fmt --check`;`check-host-deps.sh`(含 `--self-test`)、`check-shared-deps.sh`、`check-shell-regions.sh`、Cargo.lock 重复检查。
- [ ] **Step 2: 发布 Dozer 与 Digger 各一个新版本。** 先向用户确认,再执行:① Dozer:走它自己的发布流程(`scripts/build-macos-app.sh release`:升版本号 → 门禁 → 打包),装到本机启动,确认版本号与 Agent 面板可用;② Digger:`bash scripts/release.sh`(门禁 → 升版本 → 打包 → 签名校验 → 压缩),装到本机启动,确认版本号、`Digger.app/Contents/MacOS` 含 `digger`、`diggerd`、`digger-hook` 三个二进制、Agent 面板可用且旧布局不丢。记录两个新版本号与产物路径。
- [ ] **Step 3: 合并到 `main`(快进,向用户确认后),清理 worktree 与分支,** 不碰主工作区未提交的改动。
- [ ] **Step 4: 汇报:** 各仓库测试通过数、对照清单结论(已修 / 已接受)、压力与容错结论、两个新版本号、哪些没验、遗留的跨仓库任务(页签与菜单迁入 byteui;`dozer-mcp` 的 Digger 版 S6e)。

---

## Self-Review

- **Spec 覆盖:** §6.2 页签四件 → Task 2、3;§6.6 溢出菜单与覆盖层 → Task 2;D3 悬停与提示(阶跃)→ Task 1;G4 外观与行为一致 → Task 4;§8 真机与同机并存 → Task 5;§7 S6d-3 的发布与项目硬性要求 → Task 7;文档状态 → Task 6。**不在本期:** `dozer-mcp` 的 Digger 版(S6e)、会话面板与群聊面板、页签部件迁入 byteui。
- **类型一致:** `AgentHover`、`TabArgs`、`OverflowEntry`、`PanelDisplay` 的四个页签方法来自 S6c-2;`AgentPanel` 在 S6d-2 定义,本期增字段 `hover`、`cursor`、`overlay`、`drag`(各在所属任务的 Interfaces 里说明);`menu_overlay` 来自 S6d-2 Task 4。
- **风险点:** ① 拖拽状态归属取决于 S6c-1 的实际落地,Task 3 Step 1 先核对;② 悬停阶跃与 Dozer 的渐变观感有差,已列为接受的差异,若用户不接受,要么给 Digger 加动画表(牵动 `Tick` 频率),要么推迟;③ 对照验收需要用户在 Dozer 里摆出同一状态,画廊只覆盖 Digger 一侧;④ 压力问题多半出在库(Dozer 同样受影响),报告而不绕过;⑤ 发布(升版本、打包、安装)是对外动作,先确认;⑥ 页签与菜单的重复件是技术债,Task 6 记账。
