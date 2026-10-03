# H0-06:未决项证据(O1、O2、O3、O6)

> 基线提交:`ca83119`(bytehost-h0 分支,代码与 `main` 的 `4845fb85` 一致)。
> 本文只摆事实与倾向,**不下裁决**——规格 §8 要求这些保持未决。每节三段:事实、切口分析、倾向与理由(明确标"倾向,非裁决")。
> 命令均可在仓库根复现。

---

## O1 / Q15:Agent 的面板主体与会话运行服务怎么切

### 事实

含 agent 相关符号(`AgentKind|agent_launch|launch_agent|spawn_agent|headless_agent|agent_context|hook`)的文件数,按 crate:

| crate | 文件数 |
|---|---|
| `dozerd` | 25 |
| `dozer-app` | 29 |
| `dozer-hook` | 11 |
| `dozer-core` | 3 |
| `dozer-mcp` | 2 |
| `dozer-client` | 1 |

复现:`for d in crates/dozerd/src crates/dozer-app/src …; do grep -rlE "…" $d | wc -l; done`。

**dozerd 侧(机制):** `session.rs`(552 行,`portable_pty` 的 PTY 会话)、`registry.rs`(108 行,会话表)、`headless_agent.rs`(1033 行,无头一次性 agent 调用,群聊用)、`agent_context.rs`(364 行,`agent_context_items` 表的数据权威)、`default_agent_config.rs`、`session_summary*.rs`。`AgentKind`/`AgentState`/`SessionInfo` 定义在 `dozer-core::protocol`(`protocol.rs:7,20`)。

**dozer-app 侧(展示与编排):**
- `workspace/hook.rs`(484 行):hook/MCP 安装(`ensure_hook_installed`、`ensure_mcp_installed`)、`agent_icon`/`agent_dot_color` 等纯函数(`Task 4` 里被 3 个面板引用)、会话事件转发桥(`forward_events`)。
- `workspace/state.rs`:`spawn_new_tab`(`:1670`,用 `AgentKind` 起终端 tab)、`spawn_agent_card_refresh`(`:1003`)。
- `workspace/view.rs`:`agent_list_pane`(`:94`)、`agent_picker_items`(`:281`)。
- `extensions/agent_context.rs`(432 行):context 条的展示状态,"数据权威在 dozerd"(文件头)。
- `term/`(2053 行:`term_model.rs`、`term_view.rs`、`terminal.rs`):终端网格与 tab 栏。
- `app/update.rs`:`AgentPickerToggle/Select`(`:1609-1629`)、`agent_state_changed`(`:5924`)、`conversation_session_open`(`:6010`)、`agent_context_message/refresh`(`:5475,5535`)。

**`PanelKind::Agent` 究竟是什么:** 它是右栏的"**终端区 + agent 卡片列表**":`term/terminal.rs` 的 `terminal_pane` 渲染共享终端条,表头上的折叠按钮用的就是 `PanelKind::Agent`(`:188-193`);`app/layout.rs:1146-1198`、`app/app.rs:3248-3375` 对"Agent 未镜像"时的渲染顺序与尺寸做特判(`01-panelkind.md` S6)。也就是说**"Agent 面板"在代码里和"终端"是同一个面板**,分不开。

### 切口分析

天然切口已经存在于进程边界上:**dozerd 持有运行机制(PTY、会话、hook 事件、上下文存储)**,GUI 进程只通过 `dozer-client`(UDS)取数据。GUI 内的 Agent 面板主体 = 终端网格 + 卡片列表,依赖 `dozer-core::protocol` 里的 Agent 类型。切口问题不在"面板 vs 服务"(服务 = dozerd,已经是),而在:
1. **`workspace/hook.rs` 在 GUI 进程里装 hook/MCP**——这是"Agent 运行环境配置",按机制归属应属于 dozerd(要求文档 Q15 的疑问:Agent 是否与 Git 对称);它同时塞着 `agent_icon` 等 UI 纯函数(Task 4 要搬出)。
2. **Agent 面板与 Terminal 不可分**(O6):任何"共享 Agent 面板"的决定都同时是"共享 Terminal"的决定。

与 Git 的对称性:bytegit 之前,git 调用散落在 7 处;Agent 的"运行机制"已经集中在 dozerd(25 个文件),**分布比 Git 当年集中得多**,不需要同等规模的"抽底层"工程。散落的主要是 GUI 侧的 hook 安装与展示纯函数。

### 倾向与理由(倾向,非裁决)

不把 Agent 做成与 bytegit 对称的新库;先把 `workspace/hook.rs` 里的"hook/MCP 安装"划到 dozerd 侧、把 UI 纯函数拆到 Agent 面板侧。**理由:** 机制已集中在 dozerd,独立库的收益小于成本。**前提:** O6(Terminal 是否共享)必须同时定,因为 Agent 面板主体 ≈ 终端区。

---

## O2:Project context 进入 host 的范围

### 事实

- 权威数据在 dozerd:`dozer-core::protocol::ProjectInfo`(`protocol.rs:570`)。
- app 侧:`Workspace.project`(来自 `ProjectInfo`)、`open_projects.rs`(72 行;本地持久化"开了哪些项目、顺序、哪个在前台",文件头注明"项目自己的路径/名字元数据不重复存,权威数据在 dozerd")、`project_meta.rs`(73 行;读写 `.dozer/description.md`)。
- `crate::project` **不是** project context,而是 `FileTree`(见 `04-shared-modules.md` §1)。
- 引用统计(`python3 scripts/audit/edges.py | awk '$3=="open_projects"||$3=="project_meta"'`):`open_projects` 只被 `app` 引用 2 次;`project_meta` 被 `project`、`project_create`(面板)与 `app`、`workspace` 引用。
- 面板里的"当前项目"依赖:每个项目一份面板状态存在 `Workspace`(14 个面板状态字段,`03-host-to-panel.md` §3);面板自己并不直接读 `ProjectInfo`,而是由 host 把 `ws.project.path`、`project_id` 当参数传下去(例:`spawn_project_git_refresh(project_id, repo_path, io)`、`fire_panel_switch_in` 里的 `ws.project.as_ref()`)。

### 切口分析

面板实际需要的"当前项目"信息:**`project_id`(i64)+ 项目路径**,以及"哪个项目是当前前台"。`ProjectInfo` 的其余字段(分支、磁盘占用、git 状态)是 Project 面板自己展示的内容。

### 倾向与理由(倾向,非裁决)

host 只提供:**打开的项目集合 + 当前项目 id + 路径**(即 `open_projects.rs` 已经是的形状),并把"每个项目一份的面板状态"作为按 `(project_id, panel_id)` 索引的容器。`project_meta` 随 Project 面板。**理由:** 现有面板对项目的依赖本来就只有 id 和路径;把 `ProjectInfo` 整体暴露会把 Dozer 的项目元数据带进 host。**这是最小形态是否够用的判断**:够用,依据是上面"面板由 host 传 id+路径"的事实。

---

## O3:Files Tree 的"打开目标"协议

### 事实

- Files 面板对 `preview` 的直接引用只有 `files/view.rs` 里的 **`preview::TextPosition`(6 次)与 `preview::TextRange`(4 次)**两个值类型(`python3 scripts/audit/edges.py | awk '$2=="ext:files" && $3=="preview"'`)。**Files 面板本身不渲染预览**(`files/state.rs:405-407` 的文档注释:"文件那支要跨到 `Message::PreviewOpenPath`,该面板本身不渲染预览;这条整体跨内核边界")。
- **打开动作已经走内核分派:** `files::Message::TreeRowDoubleClick { path, is_dir }` 由 `App::update` 按 `is_dir` 分派——目录转发 `Message::Toggle`,文件转 `Message::PreviewOpenPath(path)`;`files::update()` 收到它会 `unreachable!`(`files/state.rs:405-413`)。
- 打开入口有三个 host 函数:`preview_open_path(path)`(`app/update.rs:5469`)→ `preview_open_path_at(path, target_line: Option<usize>)`(`:5629`);Project 面板有自己的 `project_preview_open_path`(`:5824`)和 `ProjectPreviewOpenPath` 消息(`:2702`)。
- 其他调用者:搜索命中(`:5598`、`:5726`)、agent 编辑定位(`:5500`)等,都调同一个 `preview_open_path*`。

### 切口分析

"通用打开命令"在代码里**已经有雏形**:`PreviewOpenPath(PathBuf)` + 可选行号。现状的问题只有两个:(1)消息名里写着 `Preview`,产品层 Preview 与通用"打开目标"没分开;(2)Project 面板有第二套(`ProjectPreviewOpenPath`),窗格由 `PanelKind::Project` 选(`01-panelkind.md` S4)。最小需要携带:**路径、(可选)行号、(可选)来源面板(决定落进哪个预览窗格)**。

三种协议形态:

| 形态 | 做法 | 利 | 弊 |
|---|---|---|---|
| A. 单一消息 + 产品注册处理者 | `OpenTarget { path, line, origin }`,产品在 composition root 注册处理者(Dozer→Preview,Digger→Writing) | 最贴近现状;Files 只产出一种消息 | 处理者选择逻辑(按扩展名?按来源?)要定义 |
| B. 面板返回 `Effect::Open(...)` | Files 的 `update` 返回 Effect,host 派发给处理者 | 与 git_log 迁移(E3-012)用同一套 Effect 机制,统一 | 需要先有 Effect 机制(规格 O9/要求文档 Q9 相关) |
| C. 事件总线 `FileOpenRequested` | 发布事件,处理者订阅 | 解耦最彻底 | 总线未设计(要求文档 Q9),一对一的"打开"用广播过重 |

### 倾向与理由(倾向,非裁决)

倾向 **B**(与其他 Effect 化的越界点同机制),在 Effect 机制确定前**以 A 作为过渡**——因为 A 与现状最接近,可以先做,且不阻塞。**不选定。**

---

## O6:Terminal 是否纳入共享范围

### 事实

- `term/` 2053 行(`term_model.rs`、`term_view.rs`、`terminal.rs`、`mod.rs`);对其他模块的引用:`app` 8、`workspace` 5、`extensions` 3、`theme` 2、`assets` 2、`chrome` 1(`python3 scripts/audit/edges.py | awk '$2=="layer:term"'`)。
- `term/terminal.rs` 与 `PanelKind` 的关系:`keyboard_term_target`(`:36-50`)特判 `PanelKind::Ssh`(左侧 SSH 面板有自己的内嵌终端,`TermTarget::SshPanel`);表头折叠按钮用 `PanelKind::Agent`(`:188-193`)。
- 终端的状态与消息:`TermTarget::{Shared, SshPanel}`——两个宿主共用同一套终端模型。
- 相关根模块 `keymap.rs`(294 行,键盘/IME → 终端字节)、`osc.rs`(170 行,OSC 7/133 扫描)都是终端专属纯函数。

### 切口分析

如果共享,切口是:**终端模型(`term_model`,含 alacritty 网格)+ 视图 + `keymap`/`osc`** 一起作为"终端能力",由 host 提供给 Agent 面板与 SSH 面板;`PanelKind::Ssh`/`Agent` 的特判(E3-007)要变成"终端宿主声明自己的 `TermTarget`"。**这与 O1 是同一刀**:Agent 面板主体 ≈ 终端区。

**H0 不判断 Digger 是否需要终端**(规格 O6:需用户确认产品需求)。

### 倾向与理由(倾向,非裁决)

在 Digger 需求确认前,**不把终端放进 host 共享清单**,但 H1 以后凡是改 Agent 面板的切片都要把 `term/` 当成同一个边界对待。**理由:** 终端是否共享决定 Agent 面板怎么切,两件事不能分开裁决。

---

## 仍需用户产品确认的问题

1. **O6:** Digger 是否需要终端(及 SSH 内嵌终端)?——决定 `term/` 是否进共享范围,并连带 O1。
2. **O11:** Digger 的 Todo 语义(选题 vs 验收项)与对非编码 agent 的摄取支持。——决定 Todo 面板能否原样共享。
3. **新增(H0 发现):`dozer-core::protocol` 的归属。** 它(3085 行)含全部 Dozer 领域类型,Digger 复用面板就要依赖它;"沿用同一份 protocol / 按领域拆分"需要产品层决定,不属于 host 边界范围,但会限制 host 之外的所有面板复用(`04-shared-modules.md` §4)。
