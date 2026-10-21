# S6c-2 Agent 面板卡片、选择器与视图实施报告

日期：2026-10-21。

计划：[2026-10-18-agent-s6c2-views.md](../plans/2026-10-18-agent-s6c2-views.md)。
前置：`feat/panel-agent-1` 顶端 `dc513cbd`（S6c-1 已通过审核）。

## 结论

分支 `feat/panel-agent-2`（worktree `../dozer-panel-agent-2`）**从 `dc513cbd` 切出**，未合并 `main`、未打 tag、未推送。卡片数据、transcript 尾部、样式助手、选择器/确认框内容、会话列表与终端条视图已搬进 `dozer-panel-agent`；Dozer 改为薄调用。Part B（Digger 升级钉版）按计划**未执行**。

提交：

```
60e6390b feat(agent): 视图、卡片、选择器与终端条搬进 dozer-panel-agent
5b001578 feat(agent): 搬卡片数据/transcript 尾部/样式/选择器与确认框到 dozer-panel-agent
ed64bbe5 refactor(agent): 抽出可测的卡片数据(card_model/work_line),行为不变
```

## 基线核对

- `cargo test --workspace --no-fail-fast`：**3408 通过 / 0 失败 / 5 ignored**。
- `dozer-app` 1261 / 2 ignored；`dozer-panel-term` 78 / 1 ignored；`dozer-panel-agent` 33；`dozer-client` 56。
- `cargo clippy --workspace --all-targets`：退出码 0，**45 条警告**（开工时实测）。
- `cargo build --workspace`、`check-i18n.sh`、`check_panel_boundary.py` 均通过。

## 搬走的函数与测试

- `transcript_tail`：`latest_model_mode_and_activity` + `extract_line_activity` / `join_text_blocks` / `truncate_activity`；12 条 `latest_model_and_mode_*` / `activity_*` 测试随代码搬走，另加 2 条敌意输入/字符截断测试。
- `card`：`CardCtx` / `CardModel` / `card_model` / `card_title` / `work_line` / `refresh_plan` / `apply_refresh` / `workspace_git_info` / `spawn_refresh`；Dozer 侧旧名以 `pub(crate) use … as …` 保持测试不动（`agent_card_refresh_plan_*`、`apply_agent_card_refresh_*`、`workspace_git_info_*` 仍通过）。新增 `card_model` 表驱动测试（Step 2 单独提交）。
- `style`：`agent_state_label` / `dot_color` / `format_model_label` / `agent_dot_color` / `agent_icon` / `group_tabs_by_agent`；Dozer 测试改为从库导入。
- `picker`：`PickerEntry` / `picker_entries` / `close_confirm_spec`；Dozer 的 `agent_picker_spec` 改为从 `picker_entries()` 转换，确认框改取 `close_confirm_spec`。
- 视图：新增 `display::PanelDisplay`（及其 `AgentHover` / `TabArgs` / `OverflowEntry` / `TerminalSlots`）与 `view::list::agent_list`、`view::terminal::{terminal_pane, overflow_popup}`。Dozer 删除 `agent_card` / `agent_picker_toggle_button` / `tab_bar` / `tab_item` / `active_tab_view`，`agent_list_pane` / `terminal_pane` / `term_tab_overflow_popup` 改为构造 `DozerDisplay` 转调库。
- 卡片刷新从 S6c-1 的 `AgentHostEvent::StateChanged` 收回库内：`update` 的 `AgentState` 分支直接 `card::spawn_refresh`，新增 `Message::CardRefreshed`；Dozer 删除 `Message::AgentCardRefreshed` 与 `spawn_agent_card_refresh`。

## 测试与门禁（均实跑）

| 命令 | 结果 |
| --- | --- |
| `cargo build --workspace` | 退出码 0 |
| `cargo test --workspace --no-fail-fast` | 退出码 0；**3426 通过 / 0 失败 / 5 ignored**（+18） |
| `dozer-app` / `dozer-panel-agent` | 1251 / 2 ignored；61 |
| `cargo clippy --workspace --all-targets` | 退出码 0；**41 条警告**（基线 45，少 4） |
| `cargo fmt --check` | 退出码 1；仅 `main` 既有的 9 个文件（`app/view.rs`、`extensions/files/*`、`dozer-panel-files/*`），本期文件干净 |
| `bash scripts/check-i18n.sh` | 退出码 0；`unresolved=0`，16 个 owner 目录 |
| `scripts/audit/check_panel_boundary.py` | 通过（213 refs in 5 files） |
| `cargo tree -p dozer-panel-agent` | 无 `dozer-app/client/core`、`dozerd`、`wry`、`winit`、`portable-pty`、`russh` |

### 干净快照（`git archive HEAD`）

- `cargo metadata --locked --offline`：退出码 0。
- `cargo test --workspace --no-fail-fast`（在 `/private/tmp/s6c2-clean` 下跑，避免 `/tmp` 符号链接影响预览资源测试）：**3423 通过 / 3 失败 / 5 ignored**；3 条失败全是 `git_log` dogfood 测试（archive 无 `.git`），属环境原因，3423+3 = 3426 与 worktree 一致。
- `python3 scripts/i18n/check_migration.py`：退出码 0。
- `python3 scripts/audit/check_panel_boundary.py`：通过。
- 两份 S6c-1 黄金测试未改动：`session_stream_golden.txt` 自 `e92d5fb2` 后无提交；协议编码黄金仍是 `dozer-client` 内的写死字符串，两者均通过。

## D8 / D9 落地

- **D8**：`AgentCapabilities` 保持两项；未新增 `review_entry` / `send_to_agent`。终端底部「发送到 Agent」上下文条与展开面板通过 `TerminalSlots { floating, footer }` 注入（Dozer 填，Digger 不填即不显示）。
- **D9**：页签部件未迁入 byteui；由 `PanelDisplay` 的 `panel_tab` / `overflow_button` / `overflow_menu` / `drag_surface` 四件注入，Dozer 转调现有 `chrome::tab_widget` 与 `terminal::tab_drag_surface`，零视觉改动。

## Step 3b（S6c-1 遗留）落地

- **(a) 两套实现收口**：`AgentStateChanged` 统一经 `Message::Agent` → 库 `update`（含卡片刷新）；顶层 `Message::SelectTab` / `SelectTabNoDrag` / `CloseTab` / `AgentPickerToggle` / `TermTabOverflowToggle` 及其处理器已删除；视图只发库 `Message`。`TermScroll` / `TermSelStart` / `TermSelUpdate` / `TermOutput` / `SessionExited` 仍保留在 Dozer：前三者是对 `AgentPanelState.tabs` 的直接模型操作（SSH 面板共用同一 `SessionTab` 类型但不在库状态里），后两者同时服务 SSH 与启动恢复的事件泵。SSH 相关与 `on_tab_attached` 保持原样。
- **(b)** `UpdateCtx.locale` 已删除，`DozerDisplay` 也不再写死 `"en"`。
- **(c)** 两份黄金测试与夹具字节未变（见上）。

## 与计划的偏差

1. `PanelDisplay` 的生命周期写成 `trait PanelDisplay<'a, M>`（方法返回 `Element<'a, M>`），而不是 `PanelDisplay<M>`；否则视图返回值会被宿主适配器的局部借用绑死，无法从 `agent_list_pane` 返回。
2. `TaskTitleSource` 未加 `Send + Sync`：`Workspace`（UI 单线程）无法满足，且它只在视图里用；加 `Send+Sync` 会迫使产品复制一份 Todo 状态。
3. 终端 canvas 的事件映射经 `PanelDisplay::term_event_fn() -> fn(TermEvent) -> M`（无捕获函数指针）；`dozer-panel-term::view::view` 的 `map` 由 `fn` 放宽为 `impl Fn`（仍兼容函数指针，S6b 行为不变）。
4. 选择器 `Git Shell` 图标沿用旧的 `GitBranch`（`OS Shell` 为 `SquareTerminal`），与旧菜单逐项一致。
5. 库自带 `locales/`（owner `dozer.agent`）并用**库内独立 resolver** 解析，不往 `dozer-app` 的全局 resolver 注册同名 owner（避免与 agent_context 的资源互相覆盖）；四个错误 key 仍是 `LocalizedText::keyed` 的英文默认值，用户可见文本不变。

## 目测验收清单（写进报告，不阻塞）

按用户 2026-10-20 决定，整条 Dozer 侧链路完成后一次性验收。请与改造前并排对照：

空列表文案；新建各类 Agent（＋按钮悬停变金、macOS 原生选择器菜单内容与顺序）；卡片三行（标题含模型与模式、工作内容与 `@分支`、状态点与文字）、选中高亮、悬停；多种 Agent 分组及组标题计数；`cd` 出项目根后卡片分支随之变化、`cd` 回来恢复；页签条（状态点颜色、关闭×、溢出下拉的箭头与定位、窄窗口下翻页窗口化、拖拽换位）；非零退出码提示；关运行中会话的确认框文案；「发送到 Agent」条与展开面板仍在终端底部；窗口缩放；中英文切换后各处文案。

信心来自 `cargo test --workspace --no-fail-fast` 的 3426 通过、0 失败，以及两份 S6c-1 黄金测试不变。
