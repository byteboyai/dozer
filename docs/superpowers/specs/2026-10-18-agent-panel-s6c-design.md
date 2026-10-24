# S6c Agent 面板库 `dozer-panel-agent`

> **迁移说明(2026-10-20):** 本文原在 Digger 仓库,现移入 Dozer 仓库;文内 `docs/superpowers/...` 路径均相对 **Dozer 仓库根**。写作「Digger(仓库 `../digger`)」的部分,实施仍在 Digger 仓库的 worktree 里进行,但文档不在那里。Digger 自有文档(`docs/adr/*`、`2026-10-14-window-shell-design.md`、`docs/superpowers/notes/`)仍在 Digger 仓库。最新要求与决定见 `docs/superpowers/specs/2026-10-18-agent-panel-s6-design.md` §0。


- 日期:2026-10-18
- 状态:**已实现**(S6c-1、S6c-2 均在 Dozer 完成,库为 `dozer-panel-agent`,tag `libs-2026-10-23b`);Digger 已按该 tag 接入并合入本地 `main`(2026-10-24)
- 实施状态(2026-10-24):拖拽换位入口按审核调整为 `update::reorder_term_tab(state, from, to)`,库不再提供"消息一到就换位"的入口,拖拽手势由产品的状态机负责;`PanelDisplay::drag_surface` 只有 `(content, index)` 两个参数。
- 路径:架构级,**分两期交付,每期单独写实施计划**(S6c-1 会话服务与会话/页签状态、S6c-2 列表卡片、选择器与面板外壳)
- 相关:`2026-10-18-agent-panel-s6-design.md`(S6 总设计,§6.6)、`2026-10-18-agent-s6a-daemon-identity.md`(`diggerd`、`ProductId`、hook 并排共存)、`2026-10-18-agent-s6b-term-lib.md`(`dozer-panel-term`)、Dozer `2026-10-10-project-services-design.md`(面板服务化的模板)、Dozer `2026-08-16-agent-card-workspace-design.md`

## 1. 用户意图

Digger 要有与 Dozer 一模一样的 Agent 面板。S6 总设计已定:Digger 自带独立的 `diggerd`(方案 A)、产品各自的 `digger.db`、hook 并排共存;S6a 做守护进程与产品身份,S6b 做终端控件库。本文是 S6c:把 **Agent 面板本身**抽成 Dozer 里的共享库,Dozer 与 Digger 共用同一份实现。Dozer 里改了 Agent 面板,Digger 要同步。

## 2. 现状(读代码得到)

- **面板不是独立单元。** Agent 的状态散在 3388 行的 `Workspace`(`workspace/state.rs`)里:`tabs: Vec<SessionTab>`、`active`、`next_tab_id`、`pending`、`term_tab_first`、`term_tab_overflow_anchor`、`agent_picker_open`、`pending_close_tab`、`picked_agent`,以及终端尺寸快照;视图在 `workspace/view.rs`(`agent_list_pane`、卡片行)与 `term/terminal.rs`(终端条);消息混在 `App` 顶层 `Message` 里约 20 个变体(`TermInput`、`TermOutput`、`SessionExited`、`AgentStateChanged`、`AgentCardRefreshed`、`SelectTab`、`CloseTab`、`TermTabCloseConfirm/Cancel`、`AgentPickerToggle/Close/Select`、`TabAttached`、`TabDrag*`、`TermScroll`、`TermTabOverflow*`、`TermSelStart/Update`、`TermPaste` 等)。
- **没有服务化。** 面板经 `ShellIo.client: dozer_client::Client` 直接调守护进程:`create_for_agent`、`attach`(返回快照、偏移、`TermEvent` 接收端)、`write`、`resize`、`kill`、`close_with_summary`、`list`。`Services.daemon` 只有 `list_sessions` / `shutdown_daemon` / `ensure_running` 三个方法。`TermEvent` 定义在 `dozer-client` 里,库不能依赖它;`SessionInfo`、`AgentState`、`AgentKind` 已在 `dozer-model`(可共享)。
- **新建会话要先装 hook 与 MCP**:选 Agent 后在 `spawn_blocking` 里调 `ensure_hook_installed(agent)` 与 `ensure_mcp_installed(agent)`,再 `create_for_agent(...)`。
- **键盘与输入法不走 iced 控件**:`platform/window_events.rs` 在 winit 的 `WindowEvent` 里调 `keymap::key_to_bytes(&winit::Key, &ModifiersState, app_cursor)`(293 行,11 个测试,纯函数)与 `ime_commit` 等,把结果作为 `Message::TermInput` 投回。Digger 的外壳是 iced,事件类型是 iced 的 `keyboard::Key` / `Modifiers`,不是 winit 的。
- **会话页签被 SSH 面板共用**:`SessionTab`(含 `TerminalModel`、OSC 扫描、cwd、退出码、`llm_model`、`permission_mode`、`last_activity`、工作区分支覆盖)与 `TabBackend::{Daemon, Ssh}`;SSH 面板有自己的 `ssh_tabs`。
- **卡片的数据来源**:`transcript.rs`(827 行)里 `latest_model_mode_and_activity` 从 transcript 尾部取模型、`permissionMode`、最后一句话;"当前工作内容"优先取待办面板派发记录的任务标题(`todo::WorkspaceState::task_title_for_session`);分支与脏标来自 `bytegit`。
- **Agent 面板还牵着别的功能**:终端页签的"审阅"入口(会话审阅,属于会话面板)、关闭时的 `close_with_summary`、"发送选中内容到 Agent"(`extensions/agent_context.rs`,736 行)。

## 3. 目标

- G1 在 Dozer 里新建共享库 `dozer-panel-agent`,承载 Agent 面板的状态、消息、更新与视图;Dozer 改为依赖它,**用户可见行为不变**。
- G2 面板对守护进程的全部访问经一个新的 `SessionService` trait(放进 `dozer-services`),不再碰 `dozer_client::Client`。
- G3 `dozer-panel-term` 补上输入翻译(`key_to_bytes`、IME、拖入路径),用产品中立的键位类型,Dozer 与 Digger 各自把自己的事件转成它。
- G4 Dozer 里依赖 Agent 面板的其它功能(审阅入口、关闭时总结、发送到 Agent、MCP 安装、待办任务标题)经**能力开关与注入接缝**保持工作,Digger 里对应项不显示。
- G5 SSH 面板继续在 Dozer 里工作,与 Agent 面板共用同一个会话页签类型。

## 4. 非目标

- 不做 Digger 一侧的接入(S6d)。
- 不做会话(审阅)面板、群聊面板、`dozer-mcp` 的 Digger 版。
- 不改变 Dozer 的任何用户可见行为、守护进程协议、数据位置。
- 不改 SSH 面板的功能;不把 SSH 抽进库。

## 5. 反目标

- 库里出现 `dozer-app`、`dozer-client`、`dozer-core`、`dozerd` 的依赖,或 Digger 里复制一份 Agent 面板实现。
- Digger 里出现"点了没反应"的入口:不支持的功能要么不显示,要么给出明确提示。
- 会话字节流丢失或乱序:`attach` 的快照与后续事件拼接处不重不漏(与改造前一致)。
- 关闭 Digger 窗口时杀掉仍在运行的会话(会话要随守护进程存活)。

## 6. 设计

### 6.1 `SessionService`(S6c-1)

放进 `dozer-services`,只依赖 `dozer-model`。方法按面板现有调用补全,返回 `BoxFuture<Result<_, ServiceError>>`:

| 方法 | 返回 | 对应现有调用 |
|---|---|---|
| `list_sessions()` | `Vec<SessionInfo>` | `client.list()` |
| `create_for_agent(spec: SessionSpec)` | `SessionInfo` | `create_for_agent(...)`(`SessionSpec` 含 shell、参数、cwd、cols、rows、project_id、`AgentKind`) |
| `attach(id: String, from_offset: u64)` | `(Vec<u8>, u64, UnboundedReceiver<SessionEvent>)` | `attach` |
| `write(id, data)` / `resize(id, cols, rows)` / `kill(id)` | `()` | 同名 |
| `close_with_summary(id)` | `()` | 同名(仅在 `caps.summary_on_close` 时调用) |

`SessionEvent`(原 `dozer_client::TermEvent`)搬进 `dozer-model`,`dozer-client` 改为重新导出,协议编码不变。`Client` 实现该 trait(`map_err(into_service_error)`,同项目面板的做法);`Services` 增加 `session: Arc<dyn SessionService>`。

### 6.2 库的边界

`dozer-panel-agent` 依赖:`dozer-model`、`dozer-services`、`dozer-i18n`、`dozer-panel-host`、`dozer-panel-term`、byteui、`bytehost-core`、`bytehost-desktop`、`bytegit`、iced、`serde*`、`tokio`;**不含** `dozer-app` / `dozer-client` / `dozer-core` / `dozerd` / `wry`。

库拥有:`AgentPanelState`(从 `Workspace` 里搬出上面列的那组字段)、`Message`(面板自有,约 20 个变体,产品用 `wrap` 包进自己的顶层消息,同 todo 面板)、`update`、`view`(终端条:页签条 + `dozer_panel_term::view`;列表:按 `AgentKind` 分组的卡片;Agent 选择器)、`SessionTab` 与 `TabBackend`。

`TabBackend` 为 `Daemon` 或 `Custom(mpsc::UnboundedSender<Vec<u8>>)`(字节外送通道;SSH 用后者,调整大小的指令由 SSH 一侧自己的通道带,库不认识 SSH)。SSH 面板在 Dozer 里继续使用库的 `SessionTab`,不再拥有自己的副本。

### 6.3 注入接缝与能力开关

- `AgentCapabilities { review_entry: bool, summary_on_close: bool, mcp_install: bool, send_to_agent: bool }`,产品注入;Dozer 全开,Digger 全关。
- `HookInstaller`(trait):`ensure_hook(agent: AgentKind)`、`ensure_mcp(agent: AgentKind)`(后者仅在 `caps.mcp_install` 时被调)。Dozer 用 `dozer_hook`;Digger 用 `digger-hook` 的安装逻辑(S6a 已做并排共存)。
- `TaskTitleSource`(trait):`task_title_for_session(session_id) -> Option<String>`;Dozer 转给待办面板,Digger 先给空实现,卡片回落到 `last_activity`。
- `PanelDisplay`:同其它面板库的形状,只含 Agent 视图用到的方法(悬停进度、面板头、分割线、`locale`)。
- 分支与脏标走 `bytegit`(库直接依赖)。

### 6.4 transcript 尾部解析

`latest_model_mode_and_activity` 及其依赖的最小解析搬进库(模型 id 美化、`permissionMode`、最后一句话截断到 60 字符)。`review_entries_from_turns` 等审阅相关解析**留在 `dozer-app`**。

### 6.5 输入翻译(补充 S6b,在 S6c-1 里做)

`dozer-panel-term` 新增 `input` 模块:中立类型 `TermKey`(字符 / 具名键)、`TermMods`(ctrl、alt、shift、super);`key_to_bytes(key: &TermKey, mods: TermMods, app_cursor: bool) -> Option<Vec<u8>>`(搬自 `keymap.rs`,11 个测试随走)、`ime_commit(text) -> Vec<u8>`、`dropped_path_bytes(path) -> Vec<u8>`。Dozer 的 `window_events.rs` 把 winit 值转成中立类型;Digger 把 iced 的 `keyboard::Key` / `Modifiers` 转成中立类型(S6d)。

### 6.6 分期

- **S6c-1**:`SessionEvent` 搬进 `dozer-model`;`SessionService` 与 `Client` 实现;`dozer-panel-term::input`;库的状态、消息、更新(会话生命周期、页签、输出摄取、OSC、尺寸同步、选择器、关闭确认)与 `SessionTab`;Dozer 改引用;SSH 改用库的 `SessionTab`。
- **S6c-2**:列表视图、卡片、transcript 尾部解析、`PanelDisplay`、终端条与页签条视图、能力开关与注入接缝;Dozer 改引用并清理;目测验收;发 tag。

每期都是"Dozer 抽库 → 目测验收 → 发 tag(S6c-2)→ Digger 接入(S6d)"的节奏,Dozer 每步编译且测试全绿、测试总数守恒。

## 7. 验证

- Dozer:前后测试总数守恒;`workspace/tests.rs` 里与 Agent 相关的测试随代码搬走、不删不改期望值;依赖闭包里没有被禁的 crate;**会话字节流黄金测试**(用一个假的 `SessionService` 喂入"快照 + 乱序分块事件 + 退出",最终 `TerminalModel` 的可见内容与改造前逐字一致);`Client` 的协议编码序列化黄金测试(`SessionEvent` 搬家后编码不变)。
- 用户在 Dozer 上目测验收:新建各类 Agent 会话、终端输入输出、页签增删与溢出、拖拽排序、会话列表卡片(状态点、标题、模型、模式、分支与脏标)、关闭确认、重启后会话恢复、SSH 面板终端、发送选中内容到 Agent、审阅入口,与改造前一致。
- Digger 真机验证放到 S6d。

## 8. 我的决定与未解问题

我的决定(可以推翻):

- D1 `SessionService` 放进 `dozer-services`,`SessionEvent` 搬进 `dozer-model`;不为 Agent 另起一个服务 crate。
- D2 SSH 与 Agent 共用库里的 `SessionTab`,`TabBackend` 用"字节外送通道"抽象,不把 SSH 概念带进库。
- D3 输入翻译放进 `dozer-panel-term` 并用中立键位类型,而不是让库依赖 winit。
- D4 终端条(页签条 + 终端控件)放进 `dozer-panel-agent`,页签条改用 byteui 已有的 `interaction::tab_strip` / `tabs`,不再依赖 `dozer-app` 的 `chrome::tab_widget`;若 byteui 没有等价组件,先停下报告。
- D5 审阅入口、关闭时总结、发送到 Agent、MCP 安装由能力开关关闭,Digger 里不显示。
- D6 分两期(S6c-1、S6c-2)。

未解:

- Q1 Digger 里 Agent 面板放哪一侧?Dozer 的默认侧我没有核实。我倾向放右栏最前:`[agent, apps, usage]`(左栏已有 project、files、markdown、info);这是产品布局决定,请你定。
- Q2 Digger 重启后是否像 Dozer 一样自动恢复上次打开的会话页签(Dozer 在 `App::bootstrap` 里做 GUI 级恢复)?我倾向保持一致,恢复按项目各自的页签集合。
- Q3 `TaskTitleSource` 在 Digger 里先给空实现,卡片"当前工作内容"回落到 transcript 的最后一句话——待办面板接入 Digger 之后再接上任务标题,是否接受?

## 9. 留给实现者决定

`AgentPanelState` 里每个字段的最终归属、`Message` 变体的精确清单与载荷、`SessionSpec` 的字段名、`TermKey` / `TermMods` 的具体形状、`Custom` 后端通道的消息类型、OSC 扫描器是否一并搬进 `dozer-panel-term`(倾向搬)、卡片视图里各区域的度量取值来源。
