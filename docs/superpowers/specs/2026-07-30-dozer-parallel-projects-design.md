# Dozer 多项目并行设计——顶栏项目页签 + dozerd 协议改造

> 状态：设计中，待用户终审。
> 需求来源：`docs/superpowers/specs/2026-07-29-dozer-shell-icon-rail-design.md` §3.1 已把这个功能标记为
> "超出外壳重排范畴、牵动 dozerd 协议层的独立后续设计任务"，并留下四个待解决问题。
> 2026-07-30 用户通过 `design/缺陷/img.png` 反馈顶栏项目页签缺失，讨论后确认要现在推进，本文档是
> 那一轮 brainstorming 的产出。
> 上游文档：
> - `docs/superpowers/specs/2026-07-29-dozer-shell-icon-rail-design.md` §3（原始动机与已决议的方案B）
> - `docs/superpowers/specs/2026-07-19-dozer-p1g-project-layer-design.md` D5（此前明确"单当前项目，多项目并列留后续"的决策，本文档正是那个"后续"）

## 1. 现状与动机

Dozer 今天只能有一个"当前项目"。顶栏没有项目页签，只有一个全局搜索框。切换项目走
`Message::ProjectSelect`/`ProjectOpen` 流程，而这个流程是**破坏性**的：`Workspace::update` 里
`Message::ProjectOpened` 的处理会调用 `close_all_tabs_for_switch()`（`workspace.rs:1856-1867`），
它对每个终端 tab 调 `close_tab(0)`，而 `close_tab`（`workspace.rs:1999-2013`）只要该 tab 的会话还
活着就会调 `client.kill(&id)`——也就是说，**切换项目会真的杀掉上一个项目里所有终端会话**，包括
里面正在运行的 agent 进程。

这与用户的核心诉求直接冲突：查看项目 B 的同时，项目 A 里的 agent 应该能在后台继续跑；切回项目
A 时，那些会话应该还活着，不是需要重新起。

本设计要解决的正是这个问题：**顶栏改成可并行打开多个项目的页签**，每个页签背后是完全独立、同时
存活的一套状态（各自的文件树/终端会话/预览/对话），切换页签只是切换"当前渲染哪一套"，不结束
任何会话；只有用户显式关闭一个页签，才结束该项目下的会话。

这已经不是"外壳重排"的延伸——`Session`/`SessionInfo`（dozerd 内部与协议两处）今天完全没有项目
归属字段，`SessionRegistry` 是一个不分项目的扁平 `HashMap`，`dozerd` 的 `ProjectStore`
（`crates/dozerd/src/projects.rs`）只维护一个全局单数的"活跃项目"指针（SQLite `meta` 表的
`active_project_id`）。要做到"项目 A 的会话独立于项目 B 存活"，daemon 侧就必须先知道"哪个会话
属于哪个项目"——这是本设计的协议层改动部分。

## 2. 范围裁剪（已与用户核对）

- **真正并行**，不是"最近项目快捷入口"的轻量方案——每个页签都是完整独立、同时存活的状态，不是
  点一下才临时切换的单例。这是与用户核对过的、比"UI 上像多项目、底下仍是单个活跃项目"更贵但
  更符合诉求的路径。
- **并行项目数不设硬上限**——v1 不加任何限制代码，YAGNI；等真的出现"打开太多项目导致卡顿/资源
  问题"的实际情况再回头处理。
- **重启后的恢复策略是"懒加载"**：页签集合（有哪些项目、什么顺序）整体恢复，但只有重启前正在
  聚焦的那一个立即拉取完整状态，其余页签停在"存根"态，点开时才真正加载——这是为了让"不设硬
  上限"和"重启不卡顿"两个决定不冲突（否则开着 20 个页签重启，20 份文件树/git 状态/对话列表/
  终端重挂同时发起，启动会很慢）。**这不影响会话本身的存活**：`dozerd` 是独立于 GUI 进程存在的
  常驻 daemon，会话的生死只取决于 daemon 有没有被关，与 GUI 侧"这个项目的状态有没有被加载到
  内存"完全无关——懒加载只是延后 GUI 自己那份 UI 状态（文件树/git 状态/对话列表）的重建时机。
- **关闭页签是破坏性操作**：用户主动点 × 关闭一个项目页签（不是切换到别的页签），等同于"我不要
  这个项目了"的明确信号，daemon 侧会结束该项目名下所有终端会话——这与今天"关一个终端 tab 就杀
  一个会话"是同一种语义，只是这次是整个项目一起关。项目本身还在"最近项目"列表里，想用随时可以
  重新打开（会重新起新会话，不是恢复旧会话）。

## 3. 架构总览

### 3.1 类型改造

`Workspace`（现有类型，保留名字和字段——今天已经承担"当前项目的活的状态"这个含义，本次只是把
不该属于它的外壳字段搬走）：

**保留在 `Workspace` 里的字段**（今天已有，本次不改）：`tabs`/`active`/`next_tab_id`/`pending`/
`preview`/`preview_error`/`allowed_files`/`acceptance`/`review`/`conversations`/`project`/
`file_tree`/`branch`/`dirty`/`git_statuses`/`project_goal`/`project_acceptance_count`/
`term_tab_first`/`preview_tab_first`/`tree_selected`/`tree_clipboard`/`tree_error`/
`tree_delete_confirm`/`tree_edit`。

**搬出 `Workspace`、并入新 `App` 类型的字段**（今天在 `Workspace` 上，本质是"整个程序只有一份"
的外壳/窗口状态，不该因为项目数量变多而重复）：`client`/`handle`/`proxy`/`cols`/`rows`/
`term_focused`/`daemon_error`/`blink_on`/`shell_layout`/`left_view`/`right_view`/`left_collapsed`/
`right_collapsed`/`maximized`/`window_size`/`dragging`/`context_menu`/`last_right_click`。

新增类型：

```rust
/// 单个项目页签的加载状态：懒加载用。`Stub` 只有列表/页签渲染需要的最小信息
/// （来自 dozerd 的 `ProjectInfo`），`Loaded` 是完整的、今天这个 `Workspace`
/// 结构本身。
enum WorkspaceSlot {
    Stub(ProjectInfo),
    Loaded(Workspace),
}

/// 顶层容器，main.rs 持有的就是这个（取代今天直接持有单个 `Workspace`）。
struct App {
    // 从 Workspace 搬来的外壳字段，原样保留，字段名/类型不变。
    client: Client,
    handle: tokio::runtime::Handle,
    proxy: EventLoopProxy<Message>,
    cols: u16,
    rows: u16,
    term_focused: bool,
    daemon_error: Option<String>,
    blink_on: bool,
    shell_layout: ShellLayout,
    left_view: LeftView,
    right_view: RightView,
    left_collapsed: bool,
    right_collapsed: bool,
    maximized: Option<MaximizedPane>,
    window_size: (f32, f32),
    dragging: Option<Divider>,
    context_menu: Option<ContextMenuState>,
    last_right_click: Option<(f32, f32)>,

    // 新增：多项目并行的核心状态。
    projects: HashMap<i64, WorkspaceSlot>,
    project_order: Vec<i64>,
    active_project_id: Option<i64>,
}
```

`App` 上提供 `active_workspace(&self) -> Option<&Workspace>` / `active_workspace_mut(&mut self)
-> Option<&mut Workspace>`——后者如果对应槽位是 `Stub`，就地促成 `Loaded`（见 §4）再返回引用。
`update()`/`view()` 里所有原来直接读 `self.xxx`（xxx 是搬进 `Workspace` 的字段）的地方，改成先
`self.active_workspace_mut()` 拿到当前项目的 `Workspace`，再 `.xxx`。

**这是一次体量大但性质机械的改写**——`update()`/`view()` 两个函数今天体量都很大，几乎每个消息
分支和每个渲染函数都会摸到至少一个"搬家"字段。实现计划阶段这部分会被拆成一个不能再细分的大
任务（性质与图标栏重排那次的 Task 3 相同：编译器不允许"只改一半"的中间态），并沿用那次总结出的
经验——如果编译报错数量远超预期，先停下来检查是不是漏了某个访问点，而不是逐条硬修。

### 3.2 全局不变量

- 外壳字段（图标栏选中哪个视图、放大态、窗口尺寸等）**不随项目切换重置**——这是这套架构自然带
  出来的行为：你在右栏看"对话"，切到另一个项目右栏还是"对话"，只是内容换了。不需要额外代码
  维护这条不变量，只需要不要把这些字段错放回 `Workspace` 里。
- `App.projects` 里任意时刻最多有一个 `active_project_id` 指向的槽位，且它必须是 `Loaded`
  （懒加载促成是"访问即促成"，不存在"active 但还是 Stub"的状态）。

## 4. dozerd 协议改造

`crates/dozer-core/src/protocol.rs`：

- **删除** `Request::SetActiveProject { id }`、`Request::GetActiveProject`，以及它们专用的回包
  语义——daemon 不再维护"当前活跃项目"这个概念，这部分状态完全下放给 GUI 侧 `App` 自己持有
  并本地持久化（见 §5）。
- `SessionInfo` 新增 `project_id: i64` 字段——本次协议改动的核心，会话第一次有了项目归属。
  dozerd 内部的 `Session`/`SessionSpec`（`crates/dozerd/src/session.rs`）同步加这个字段。
- `Request::CreateSession` 新增必填参数 `project_id: i64`——GUI 起一个新终端 tab 时，把"这个
  tab 属于哪个项目"显式传给 daemon，而不是像今天这样只传一个不关联任何项目的 `cwd` 字符串。
- `Request::ListSessions`/`Reply::Sessions` **请求形状不变**（不加过滤参数）——daemon 依然一次性
  把所有会话吐出来，每条 `SessionInfo` 自带 `project_id`；GUI 本地按 `project_id` 过滤出"这个
  项目的会话"。现实中会话总数是几个到几十个的量级，没有必要为过滤这点数据专门给协议加参数、
  多背一个协议版本兼容面。
- `Request::OpenProject { path }`/`Request::ListProjects` 保留、行为基本不变（`OpenProject`
  依然是"按路径 upsert 一条 `ProjectInfo`，返回它"），只是不再有"顺带 set active"这个副作用——
  这个副作用本来就是"活跃项目"概念的一部分，现在整个概念被拿掉了。
- dozerd 侧 `ProjectStore`（`crates/dozerd/src/projects.rs`）的 SQLite `meta` 表（原来存单行
  `active_project_id`）连带删除，属于清理性质的改动。

**关闭页签杀会话不新增协议方法**：GUI 已经知道"当前项目 `Loaded` 的 `Workspace.tabs` 里有哪些
会话 id"，复用已有的单会话 `kill(id)` 循环调用即可（今天 `close_all_tabs_for_switch` 已经是这
个模式，只是触发时机从"切换"改成"关闭页签"）。边界情况：如果关闭一个还没被点开过的 `Stub`
页签，GUI 内存里没有它的会话列表——关闭前先按 `project_id` 过滤一次 `ListSessions` 拿到要杀的
会话 id，不需要真的把整个 `Workspace` 促成 `Loaded` 再关。

## 5. GUI 侧数据流

新增本地持久化文件 `open_projects.json`（复用 `layout.rs`/`preview_state.rs` 已有的"本地 JSON +
`Workspace`/`App` 变更时后台异步写"模式，不进 daemon 的 SQLite）：

```rust
struct OpenProjectsState {
    project_ids: Vec<i64>,       // 对应 App.project_order，重启恢复顺序用
    active_project_id: Option<i64>,
}
```

只记"当前开了哪些项目页签、什么顺序、哪个在前台"——项目的路径/名字这些元数据不重复存，需要时
向 daemon 的 `ListProjects` 要权威数据。

**打开一个新项目页签**（点顶栏 `+`）：`client.open_project(path)` upsert 拿到 `ProjectInfo` →
若这个 `project_id` 已经在 `App.projects` 里（同一个项目被再次打开），直接把 `active_project_id`
切过去，不新建页签；否则新建一个 `Loaded` 态条目（用户主动打开的，直接热加载，不走 `Stub`）、
追加进 `project_order`、切焦点过去 → 写 `open_projects.json`。

**切换页签**（点已存在的页签）：`active_project_id` 改成目标 id；若该槽位是 `Stub`，就地促成
`Loaded`——按 `project_id` 过滤 `ListSessions` 重新挂回存活会话、没有存活会话就
`ensure_project_terminal()`、拉文件树/git 状态/对话列表（这部分逻辑今天已经在
`Message::ProjectOpened` 里写过一遍，改造成"促成单个 `Workspace`"的独立函数，供这里和启动恢复
共用）。**不杀任何会话，不动其它项目的状态**。切换完成后要触发一次终端网格重算
（`sync_terminal_grid`）——这是图标栏重排那轮最终审查已经确立的模式（放大/收起/拖拽结束/图标
切换都会触发），"切换到的新项目其终端 pane 尺寸可能与切走前不同"是同一类需要重算的场景，直接
把"切换页签"加进那个已有的触发点列表，不是新概念。

**关闭页签**（点 ×）：按 §4 的边界情况处理杀完该项目所有会话后，从 `App.projects`/
`project_order` 移除；如果关的正是当前 active 的页签，焦点挪到它左边那个（没有就随便挑一个剩下
的，同浏览器关标签页的常见做法）→ 写 `open_projects.json`。

**启动恢复**：读 `open_projects.json` → 按记录顺序为每个 `project_id` 插入一个 `Stub`（若该 id
在 daemon 的 `ListProjects` 里已经不存在——比如项目被删过——直接跳过，不显示这个页签）→ 之前
记录的 `active_project_id` 那一个立即促成 `Loaded`（复用切换页签的促成逻辑），其余保持 `Stub`。

## 6. 顶栏页签 UI

沿用 `docs/superpowers/specs/2026-07-29-dozer-shell-icon-rail-design.md` §3 已经画过的顶栏形状：
`[红黄绿] Dozer  [项目1(active)] [项目2] [+] …… ⚙`。

**页签溢出**：不设硬上限意味着页签数量可能超过顶栏宽度。直接复用仓库里已经在用的模式——
`preview_pane`/终端 `tab_bar` 现在都是"索引窗口化 + clip + 左右箭头"，不是横向 scrollable。
项目页签用同一套交互语言，不单独发明新的溢出处理方式。

**每个页签显示"后台是否有代理在工作"**：这是这个功能存在的核心理由之一——切走了还想知道另一个
项目有没有在动。复用仓库里已有的 `dot_color(state: AgentState, alive: bool)`（`workspace.rs:4272`）
颜色编码，不新造一套颜色语义。取该项目所有存活会话里"最值得关注"的那个状态，按这个仓库自己的
颜色语言已经定好的优先级（`dot_color` 的注释原话："金是甲方动作专属色：该出手了"——`GOLD` 是
整个主题里唯一保留给"轮到用户动作"的颜色，语义上最高）：

1. 任意会话 `TurnEnded`（金）——最高优先级，代表"这个项目有一轮已经跑完在等你看"。
2. 否则任意会话 `AwaitingInput`（紫）——代表"这个项目卡在等你输入"。
3. 否则任意会话 `Running`（绿，跟随 `tab_item` 的闪烁逻辑一起闪）——代表"还在跑，无需动作"。
4. 否则任意会话 `Idle`（绿，不闪）。
5. 该项目没有存活会话——不显示指示器（同 `dot_color` 的 `!alive → DIM` 语义，但顶栏页签场景下
   "没有存活会话"和"有一个死会话"是同一件事，直接不画点，不画一个 `DIM` 点占位）。

## 7. 测试与实现方式

延续本仓库既有的测试哲学（布局/纯函数走单元测试，视图整体效果走"编译通过 + 真机目测"）：

- 纯逻辑单元测试：`open_projects.json` 的 load/save 往返（照抄 `layout.rs`/`preview_state.rs`
  已有的测试模式）、`Stub → Loaded` 促成逻辑、页签指示器的颜色选取逻辑（给定一组会话状态 → 取
  哪个颜色，纯函数）、关闭页签时"要杀哪些会话 id"的计算。
- `dozerd` 侧：`Session`/`SessionSpec` 新增 `project_id` 字段、`CreateSession` 要求必填该
  参数——照 `dozerd` 已有的测试惯例（`projects.rs`/`session.rs` 已有单元测试）补等价覆盖。
- 视图整体效果（顶栏页签渲染、溢出滚动、指示器颜色实际显示）走真机目测，不引入新的测试哲学。

## 8. 非目标 / 遗留悬空项

- **页签拖拽重排**——`project_order` 目前只由"打开顺序"和"关闭时移除"决定，不支持用户手动拖拽
  调整顺序。与图标栏重排那次"拖拽重排图标留后续"是同一类型的裁剪，不在本轮范围。
- **并行项目数软上限**——本文档 §2 已确认 v1 不做；如果后续发现资源问题（大量 PTY 进程/大量
  git 状态轮询/大量对话列表刷新同时跑），需要单独一轮讨论加软上限机制，不在本设计范围内预先
  设计。
- **`Session`/`SessionInfo` 的 `project_id` 迁移**——今天已经存活的会话（如果实现这个功能时
  daemon 还在跑、有旧会话在线）不会自动带上 `project_id`。这类"迁移期孤儿会话"怎么处理（是
  daemon 重启后清空，还是允许一批 `project_id` 缺失的历史会话继续存在）留给实现计划阶段决定，
  不是架构层面的设计问题。
- **多客户端场景**——如果未来 daemon 需要同时服务 GUI 之外的其它客户端（比如一个假设的 CLI），
  "哪些项目当前算并行打开"这个概念要不要在 daemon 侧也有一份权威状态，本文档不涉及；当前只有
  GUI 一个客户端，daemon 变成纯会话仓库是当下最简单、足够的选择。
