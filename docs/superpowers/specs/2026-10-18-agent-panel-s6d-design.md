# S6d Digger 接入 Agent 面板

> **迁移说明(2026-10-20):** 本文原在 Digger 仓库,现移入 Dozer 仓库;文内 `docs/superpowers/...` 路径均相对 **Dozer 仓库根**。写作「Digger(仓库 `../digger`)」的部分,实施仍在 Digger 仓库的 worktree 里进行,但文档不在那里。Digger 自有文档(`docs/adr/*`、`2026-10-14-window-shell-design.md`、`docs/superpowers/notes/`)仍在 Digger 仓库。最新要求与决定见 `docs/superpowers/specs/2026-10-18-agent-panel-s6-design.md` §0。


- 日期:2026-10-18
- 状态:**代码已实现并合入 Digger 本地 `main`(`52df4b9`,2026-10-24,未推送);真机回归、Dozer 对照与发布未完成**
- 实施状态(2026-10-24):
  - S6d-1(探针与骨架)已完成;输入法探针五项由用户真机验证通过(2026-10-23)。
  - S6d-2 Part B(会话、恢复、选择器、网格同步、键盘/指针/粘贴/拖入/输入法)已完成并经审核,含一次补测试与"关项目时回收在途 create 的孤儿会话"的修复。
  - S6d-3(页签外观、悬停阶跃、溢出菜单、拖拽换位、`agent_gallery` 画廊)已完成并经审核;为让画廊复用产品适配层,`crates/desktop` 拆出了 `lib.rs`(计划外的结构变动)。
  - 未完成:S6d-2 Task 7 真机回归;S6d-3 的 Dozer 逐状态截图对照、压力与同机并存回归、发布(Dozer 与 Digger 各一个版本)。
- 路径:架构级,**分三期交付,每期单独写实施计划**(S6d-1 探针与骨架、S6d-2 会话与终端、S6d-3 页签条与菜单、真机回归)
- 相关:`2026-10-18-agent-panel-s6-design.md`(S6 总设计)、`2026-10-18-agent-panel-s6c-design.md`、`plans/2026-10-18-agent-s6a-daemon-identity.md`(`diggerd` 已在跑)、`plans/2026-10-18-agent-s6b-term-lib.md`、`plans/2026-10-18-agent-s6c1-session-state.md`、`plans/2026-10-18-agent-s6c2-views.md`、`2026-10-18-project-panel-s5a-design.md`(Project 面板的接入方式是本文的模板)、`2026-10-14-window-shell-design.md`

## 1. 用户意图

Digger 要有与 Dozer 一模一样的 Agent 面板:在当前项目目录里新建各种 Agent 会话,终端输入输出,会话列表卡片,页签增删与溢出,关闭确认,重启后会话还在。库(`dozer-panel-agent`、`dozer-panel-term`)和守护进程(`diggerd`)都已由 S6a–S6c 备好,本文只讲 Digger 一侧怎么接。

## 2. 现状(读代码得到)

- **Digger 外壳不是 Dozer 的 `App`。** `crates/desktop/src/product.rs`(1472 行)是 `ProductShell`,跑在 `iced::application` 上(Dozer 是自己的 winit 事件循环)。面板在 `shell.rs` 登记 id、图标、配对规则;`ShellHost::body` 返回各面板的 `El`;键盘与鼠标只有一条 `iced::event::listen_with` 订阅(只转给 `bytehost_chrome::map_event`)。
- **面板接入模板:** `adapters/project.rs` 的 `ProjectPanel` 持有库的状态,实现库的 `PanelDisplay<Msg>`;库消息包成 `Msg::Project(..)`,**不带项目 id**,由 `resolve_project` 路由到聚焦项目;后台结果经 `receiver_stream` 接成 iced 流。当前项目来自 `projects.current_project()`;项目页签行在 `projects/`。
- **配对面板:** 用量用 `PaneRules` + `pair_view`(列表是第一块、可拖窄收起)。Agent 在 Dozer 里同样是"列表 + 终端"配对。
- **没有的东西:** 没有悬停动画体系(用量适配的折叠悬停是空操作)、没有 tooltip 延迟、没有页签部件(`panel_tab`、溢出菜单)、没有已注册的等宽字体、没有键盘到终端的路径、没有输入法(IME)路径。Dozer 的输入走 winit 事件(`platform/window_events.rs`),Digger 拿不到 winit 事件,只有 iced 的 `Event`。
- **守护进程(S6a 已做):** `daemon::Daemon::ensure()` 拉起 `diggerd`,`digger-hook` 并排安装;`SessionService` 由 `dozer_client::Client` 实现(S6c-1)。关闭 Digger 窗口不终止 `diggerd`。
- **Dozer 的会话语义(要照做):** 关项目页签 = abort 转发任务并 `kill` 该项目全部会话(`project_tab_close` → `close_all_tabs_for_switch`);启动时 attach 守护进程里该项目已存活的会话,没有时默认新开一个根在项目目录的终端;⌘ 组合键是应用快捷键,一律不进 PTY(⌘C 复制、⌘V 粘贴);启动 Agent 是往 shell 里输入命令行(所以 CLI 没装时,错误是 shell 自己在终端里打印的 `command not found`)。

## 3. 目标

- G1 Digger 右栏最前出现 Agent 面板,图标 `Brain`,配对:会话列表 | 终端。
- G2 每个项目一份 Agent 状态;会话的工作目录是该项目路径;切项目页签看到各自的会话。
- G3 终端可用:渲染、键盘输入(含方向键、控制键、功能键)、粘贴与复制、滚动、选区、拖入文件路径、窗口缩放时网格尺寸同步、**中文输入法**。
- G4 页签条、溢出菜单、拖拽换位、新建选择器、关闭确认的外观与行为与 Dozer 一致。
- G5 重启 Digger 后各项目的存活会话自动 attach 回来,字节流不丢不重。
- G6 `diggerd` 不可用时面板给出可读提示与重试,主窗口其余功能不受影响。

## 4. 非目标

- 不做文件树、会话(转录)面板、群聊面板、待办任务标题、「发送到 Agent」、关闭时总结、MCP 安装、SSH 面板(能力开关全关,插槽不填)。
- 不改库、不改 Dozer;库里缺什么先停下报告,不在 Digger 里复制一份。
- 不做 byteui 页签部件迁移(用户已定走注入,见 S6c-2 的 D9)。

## 5. 反目标

- 按键丢失、重复,或进了别的控件(项目面板的输入框、编辑器 webview)。
- 项目 A 的会话输出出现在项目 B 的终端里,或后台项目的消息改了前台项目的状态。
- 关闭 Digger 窗口时杀掉会话;或关项目页签时**没**杀会话(与 Dozer 相反)。
- 在 UI 线程上等守护进程(列会话、attach、写入都不得阻塞渲染)。
- 已有用户的布局被重置;Agent 面板挤掉 Project 在左栏第一位的位置(左栏不动)。
- Digger 里复制 Dozer 的终端或卡片实现。

## 6. 设计

### 6.1 登记与布局(`shell.rs`、`product.rs`)

- 新面板 id `digger.agent`,标题「Agent」,图标 `IconKind::Brain`(与 Dozer 图标栏一致)。
- `rules`:配对面板,`default_side: Right`,第一块是列表、第二块是终端,`min_first_px` 取卡片行不被挤爆的宽度(沿用 `list_min_width` 的写法,数值由实现者按卡片实测)。
- 默认布局:右栏 `[agent, apps, usage]`;**已有用户的旧布局文件里没有 `digger.agent`** → 按 S1 规则回到默认栏,再把它放在右栏最前(同 S5a D5 对 Project 的处理),**不改变该用户右栏当前选中的面板**。左栏不动。新用户的默认选中:见 Q1。

### 6.2 面板适配 `adapters/agent.rs`

- `AgentPanel` 持有 `HashMap<i64, AgentPanelState>`(**每项目一份**),`Arc<dyn SessionService>`、`Arc<dyn HookInstaller>`(`digger-hook`)、`AgentCapabilities::default()`(全关)、空的 `TaskTitleSource`、tokio `Handle`。
- 消息 `Msg::Agent(i64, dozer_panel_agent::Message)`:**带项目 id**(与 Project 不同)。理由:会话的后台结果(输出、状态变更、attach 完成)在项目不聚焦时照样到达,不能按"当前项目"路由。
- 实现 `PanelDisplay<Msg>`:`wrap` 绑定项目 id;`pane_style` 取与 Dozer 区域度量等价的数值;`panel_head`、`list_collapse_button`、`tab_divider` 仿 `ProjectPanel`/`UsagePanel` 的写法;`term_style` 构造 `TermStyle`(见 6.5 字体);`terminal_focused`/`ime_preedit` 读 6.4 的焦点与预编辑状态;`hover_progress` 恒 0、`hover_tooltip_ready` 恒假(Digger 没有悬停动画体系,见 D3)。
- 页签四件(`panel_tab`、`overflow_button`、`overflow_menu`、`drag_surface`)在 `adapters/agent_tabs.rs` 里用 byteui 的 `tabs::tab_core`、`tab_strip::tab_window`、`context_menu` 实现,外观对照 Dozer 的 `chrome/tab_widget.rs`。**这是 S6c-2 D9 的直接代价**:约 300 行与 Dozer 重复的页签渲染,在 byteui 迁移前存在;文件头注明来源与迁移意图。
- 后台结果:`AgentPanel` 的 `emit` 写入 `UnboundedSender`,接收端在 `boot` 里接成 iced 流(`receiver_stream` 同款),回投 `Msg::Agent`。

### 6.3 项目生命周期

- **懒启动(D1):** 某项目的 Agent 面板**第一次变为可见**时(`on_visible` 与项目页签切换后仍可见时),才为它建 `AgentPanelState`:`list_sessions` 过滤该项目的存活会话 → 逐个 `attach` 恢复;一个都没有就新开一个根在项目目录的 shell(与 Dozer 的"默认一个终端"一致)。从未打开过 Agent 面板的项目不会启动任何进程。
- 关项目页签:abort 转发任务并 `kill` 该项目全部会话(**与 Dozer 一致**);并清理 `AgentPanelState`。
- 无已开项目:面板显示空态文案(与 Project 面板一致),不建会话。
- `diggerd` 不可用:面板显示 `DaemonError` 的可读文案与「重试」按钮(重试走 `Daemon::ensure`);不影响其它面板。

### 6.4 键盘、鼠标与输入法

- **键盘:** 在现有 `listen_with` 订阅里增加分支:`Event::Keyboard` 且 `status == Ignored`(被 `text_input` 等控件捕获的不会到这里)且 `terminal_focused` 时,把 iced 的 `keyboard::Key` + `Modifiers` 转成 `dozer_panel_term::input::{TermKey, TermMods}`,调 `key_to_bytes`,经 `Msg::Agent(pid, Message::Input(bytes))` 写入当前项目的当前会话。转换函数放 `adapters/agent_keys.rs`,有矩阵测试:与 Dozer 的 winit 版在同一组按键上产出**相同字节**(期望值取自 Dozer `keymap.rs` 的测试)。
- **焦点:** `terminal_focused` = Agent 面板可见 && 最近一次指针按下发生在终端区域内 && 其后没有别处的按下。⌘ 组合键不进 PTY:⌘C(有选区时)复制、⌘V 粘贴(走 iced 剪贴板任务,按 `bracketed_paste` 包裹),其余 ⌘ 组合键放行给应用。规则逐条对照 Dozer `window_events.rs` 约 929–1010 行,实现者列出对照表。
- **鼠标:** 终端控件的滚轮、选区事件经库的 `TermEvent` → `Message` 映射;拖入文件(`window::Event::FileDropped`)转成 `dropped_path_bytes` 写入。
- **输入法(D6,最大风险):** iced 的 `Event::InputMethod`(预编辑/提交)只有在控件调用 `shell.request_input_method` 时才送达,canvas 控件做不到。方案:在 Digger 里写一个薄包装控件 `ImeSink`(`adapters/agent_ime.rs`),包住 S6b 的终端 `Element`,在 `Widget::update` 里处理 `InputMethod` 事件并请求输入法(候选框位置取终端光标格的矩形),提交文本经 `ime_commit` 写入会话,预编辑文本放进 `ime_preedit`。**S6d-1 先做探针**验证这条路在 iced 0.14 + macOS 上可行;不可行时退路见 Q2。

### 6.5 字体与网格尺寸

- 字体:把 Dozer 内嵌的 `JetBrainsMono[wght].ttf`(SIL OFL)拷进 Digger 资源(`include_bytes!`),启动时与 Dozer 同样注册进 fontdb;`TermStyle` 的字号与行高系数取与 Dozer `assets/theme/terminal.json` 相同的数值。同时做 Dozer 的 `sanitize_font_db`(剔除会毒化 CJK 回退的位图字体),否则中文行可能整行消失。
- 网格尺寸:每次窗口缩放、分隔线拖动、配对分割线拖动、面板切入后,由 `ShellModel` 的矩形(配对时取 `pair_rects.second`)减去页签条与内边距得到像素,经 `TermStyle::grid_size` 得 `(cols, rows)`,发 `Message` 让库 `resize_all`。公式的取整与 Dozer 一致,**不得**在 `view` 里算(避免每帧重算与发送)。

### 6.6 选择器、确认框与溢出菜单

- 选择器:库发 `AgentRequest::ShowPicker { at }`,Digger 用自备的 `adapters/menu_overlay.rs`(byteui 的 `context_menu` 只是右键触发封装,没有菜单渲染)在 `at` 处弹层(条目取 `picker_entries()`),点选后回送 `PickerSelect`,点空白或 Esc 关闭。
- 关闭确认:库的 `close_confirm_spec` 返回 `ConfirmDialog`,Digger 用 `stack!` 覆盖卡片(同 `project_delete_confirm` 的做法),不开独立窗口。
- 溢出菜单:`overflow_menu` 同样是 `stack!` 覆盖层,锚点用最近的光标位置。
- 这些覆盖层要盖住右栏的 webview 面板吗?Agent 面板自身没有 webview,但右栏切换时 apps/usage 的原生 webview 要在 Agent 面板可见时隐藏——沿用现有 `sync_native` 的可见性判断,不新增逻辑。

### 6.7 退出语义

- 关闭窗口只 detach:abort 转发任务、保留会话。关页签发出的 `kill` 任务必须在应用退出前跑完:并入现有 `ShutdownFinished` 链(与 `apps.shutdown()` 并列),库通过 `track_exit_critical` 注入。

## 7. 分期

- **S6d-1 探针与骨架:** 依赖升级(含 `dozer-panel-agent`、`dozer-panel-term`);字体注册;**IME 探针**与**键位转换矩阵测试**;面板登记、布局迁移、空面板、`diggerd` 不可用态。结束时 Digger 右栏有 Agent 面板的壳,探针结论已定。
- **S6d-2 会话与终端:** `AgentPanel` 适配、每项目状态、懒启动与恢复、键盘/粘贴/复制/鼠标/拖入、网格尺寸同步、退出语义。结束时能在 Digger 里开 shell、跑命令、切项目、重启恢复。
- **S6d-3 页签条与菜单、回归:** 页签四件、选择器、确认框、溢出菜单、拖拽换位、卡片;Dozer 与 Digger 同机并存的真机回归。

## 8. 验证

- 单元:`FakeSession` 驱动的路由测试(项目 A 的 `Output` 不改项目 B 的终端;关项目页签 kill 的恰是该项目会话;`diggerd` 不可用态);键位矩阵与 Dozer 逐字节一致;布局迁移(旧布局无 `digger.agent` → 右栏最前、选中面板不变、左栏不变);网格尺寸公式与 Dozer 同输入同输出;`ImeSink` 的事件处理(预编辑、提交、取消)用合成事件测。
- 真机(只能截图,点击要人来做;沙箱数据根与临时 `HOME`,不碰真实数据):开项目 → 开 Agent 面板 → 默认 shell 出现;输入 `ls`、方向键、Ctrl+C、`vim` 进出;粘贴多行(bracketed paste);缩放窗口后 `stty size` 与窗口一致;开两个项目页签各一个会话,互不串;退出再启动,会话恢复且内容连续;关项目页签后 `diggerd` 里该项目会话消失;Dozer 与 Digger 同时跑,各开 Claude 会话,状态点各自更新;中文输入法输入与候选框位置(人来验)。
- 门禁:全量测试、clippy、fmt、`check-shared-deps.sh`、`check-host-deps.sh`、Cargo.lock 重复检查。

## 9. 我的决定与未解问题

我的决定(可以推翻):

- D1 懒启动:Agent 面板对某项目第一次可见时才恢复/新建会话,而不是项目一打开就启动(Dozer 是后者)。避免用户没用 Agent 的项目也拉起 shell 进程;代价是后台项目的状态点在首次打开前不更新。
- D2 Agent 消息带项目 id,与 Project 面板的"聚焦路由"不同。
- D3 Digger 暂无悬停动画与延迟 tooltip:悬停进度恒 0、tooltip 不显示,按钮靠控件自身的 hover 样式。外观与 Dozer 有细微差别,属已知差异。
- D4 页签渲染在 Digger 里临时复制一份(S6c-2 D9 的后果),byteui 迁移后删除。
- D5 键盘经订阅 + `status == Ignored` + 自管焦点标志送达,不引入新的全局焦点系统。
- D6 IME 走 `ImeSink` 包装控件,先探针后定。
- D7 关项目页签 kill 会话(与 Dozer 一致),不是 detach。
- D8 §2 里「CLI 没装时的可读错误」:启动 Agent 是往 shell 里输入命令,错误由 shell 在终端里打印,不另做提示(S6 总设计 §6.7 的相应一句作废)。

未解:

- Q1 新用户的默认选中:右栏默认选中 Agent 还是仍是用量?我倾向 Agent(它是主工作面板);这会改变首次打开 Digger 看到的画面。已有用户不受影响。
- Q2 若 S6d-1 的 IME 探针失败(iced 0.14 在 canvas/自定义控件上拿不到 macOS 输入法事件):接受「首版不支持中文输入法、拷贝粘贴可输入中文」并记为已知缺口,还是为此升级 iced / 改用 winit 级钩子?后者牵动 bytehost,代价大。你是中文输入用户,这条请你定。
- Q3 卡片「当前工作内容」在 Digger 里回落到 transcript 最后一句话(S6 已定);但 Digger 的 `diggerd` 关闭了 transcript 摄取,卡片读 transcript 文件本身(`spawn_refresh` 读本地文件)不受影响,是否接受?

## 10. 留给实现者决定

`min_first_px` 数值、`pane_style` 各度量、页签四件的像素细节、`ImeSink` 的具体控件结构、焦点标志的存放位置、⌘ 组合键与 Dozer 的逐条对照表、拖拽换位的状态机放哪、后台项目的状态点是否在懒启动前显示占位。
