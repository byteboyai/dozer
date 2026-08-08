# 验收(Acceptance)面板扩展化设计

**状态:已批准(brainstorming 会话,2026-08-08)**

## 背景

阶段 1 扩展化重构此前已完工五个试点(git-log/浏览器/Todo/Files/Usage),都是"内部代码
组织重构,不改变任何用户可见行为"的性质——面板本来就是 rail 图标可选视图,只是把状态/
消息/渲染从 `workspace.rs` 里搬进 `extensions::` 各自的模块。

这次不一样:验收(甲方验收/`AcceptanceView`)现在**不是**一个 rail 图标可选视图,而是
`PreviewPane` 里的一个特殊 tab(`TabKind::Acceptance`,全局至多一个),只能通过终端面板
里"有新交付,进入验收"的金色横幅点开——横幅只看**当前激活 tab** 的 `delivery_pending`,
不激活就看不到入口。这次是一次真正的产品级 UX 改动:把验收提升成右侧 rail 图标可选的
独立面板(`RightView::Acceptance`,跟 Agent/Conversations/Usage 平级),横幅整体去掉,
改用图标徽标提示"有待验收交付"。

brainstorming 过程中还确认了一条更根本的架构原则(用户明确提出):**每个 extension 必须
保持独立,彼此只能通过消息通信,不能互相耦合**。这直接否决了"验收面板点变更文件跳转到
左侧 `PreviewPane`"这条路(会在 `extensions::acceptance` 和左侧文件预览之间建立隐性
依赖),改为验收面板自带一个精简版预览——展示变更文件的 unified diff(红/绿行),完全
自包含,不产生任何跨面板消息。这个决定顺带解除了验收对 `PreviewPane` 的最后一点依赖,
让 `preview.rs` 也能借这次机会回归"纯文件/网页 tab"的单一职责。

## 目标 / 非目标

**目标**:
1. 新增 `RightView::Acceptance`(第 4 个右侧视图)、`RailButton::RightAcceptance`、新
   图标 `IconKind::BadgeCheck`(`assets/icons/badge-check.svg`,Lucide `badge-check`——
   `ListChecks` 已被 Todo 占用)。
2. 新建 `extensions::acceptance`,拥有自己的 `Message`/`WorkspaceState`/`update`/`view`。
   现有 `AcceptanceView` 的 9 个字段(`repo`/`source_tab_id`/`goal`/`changes`/`checked`/
   `comment`/`comment_editing`/`error`/`accepted_version`)搬进 `AcceptanceSession`,
   新增 `expanded: HashSet<usize>`/`diffs: HashMap<usize, Result<String, String>>` 两个
   字段支撑"点变更文件展开 diff"这个新交互。`WorkspaceState` 持有
   `Option<AcceptanceSession>`(至多一个进行中验收会话,原样保留现状语义)。
3. 现有 8 条 `Acceptance*` 消息(`AcceptanceOpen`/`Loaded`/`Toggle`/`CommentClick`/
   `CommentEvent`/`Accept`/`Reject`/`Done`)去前缀搬进 `extensions::acceptance::Message`,
   新增 `ToggleDiff(usize)`/`DiffLoaded(usize, Result<String, String>)` 两条支撑 diff
   展开/收起。
4. 变更文件列表改成手风琴交互(复用本仓 Review 面板"展开/收起 AI 回合过程区"的既有模式):
   点文件名展开该文件相对验收基线的 unified diff(+行绿/-行红/上下文暗色,等宽字体),
   再点收起。彻底不再需要 `Message::PreviewOpenPath`——验收面板不产生任何跨面板消息,
   完全自包含。
5. 新增内核级公用函数 `delivery::file_diff(repo: &Path, path: &str) -> Result<String,
   String>`(单文件相对验收基线的 unified diff 原始文本,过长截断),`extensions::
   acceptance` 调用,不属于任何 extension——同 `extensions::git_log`/
   `extensions::acceptance` 都会独立调用 `delivery.rs` 但互不知道对方存在的既有模式。
6. 图标徽标:内核渲染 `right_icon_rail` 时读 `ws.tabs.get(ws.active).map(|t|
   t.delivery_pending).unwrap_or(false)`,为真在图标角上画一个金点——这条判断条件跟
   现有横幅触发条件完全一致(只看当前激活 tab,不扫其余后台 tab),只是从"横幅"变成
   "图标角标"。
7. 点击 `RightIconSelect(RightView::Acceptance)`:内核用当前激活 tab 的 id 发起等价于
   现有 `AcceptanceOpen(tab_id)` 的异步加载——不需要用户先看到横幅,随时点随时能看。
8. `preview.rs` 删除 `TabKind::Acceptance`/`open_acceptance`/`acceptance_active`/
   `desired_webviews` 里的 `overlay_active` 分支,`PreviewPane` 回归纯文件/网页 tab
   单一职责。
9. 终端面板里"有新交付,进入验收"的金色横幅(含 `banner_text` 函数、`Message::
   AcceptanceOpen(tab.tab_id)` 的按钮)整体删除。

**非目标**:
- 不改变验收的核心业务逻辑:`delivery::accept`/打回注回来源会话 PTY(文案
  `"[Dozer 验收打回] {comment}\n"`)/`client.record_acceptance` 落库、"全局至多一个
  验收会话,后开的覆盖先开的"、通过后不自动清空(停留显示"已沉淀 v<n>"直到用户主动
  离开/开始下一个)——这些原样保留,brainstorming 已确认。
- 不做"多个 tab 同时有待验收交付"的列表/切换,继续维持现状的单会话语义
  (brainstorming 已确认)。
- diff 只展示 unified diff 文本(等宽字体 + 逐行按 `+`/`-`/` ` 前缀染色),不做语法
  高亮、不做并排(side-by-side)视图、不支持在 diff 里做任何编辑操作。
- 不建 `Extension` trait/注册表,不拆独立 crate。

## 关键语义确认(brainstorming 会话定案)

- **这是产品级 UX 改动,不是纯代码重构**:跟前五个试点最大的不同——用户可见行为确实
  变了(横幅消失、多了一个 rail 图标、点变更文件不再跳左侧预览而是原地展开 diff)。
  设计过程因此比其余试点多了三轮来回:先确认"pane"具体指"rail 图标可选独立视图"而非
  其它形态;再确认 RightView(不是 LeftView)、去掉横幅(不是保留跳转);最后确认
  "自带精简 preview"(不是复用左侧 `PreviewPane`)。
- **extension 独立性是用户明确提出的架构硬性原则**:"每个 extension 应该保持独立,
  它们之间应该通过消息的方式通信,但是不能耦合在一起"——这条否决了"点变更文件跳转到
  `PreviewPane`"的初稿设计(那会让 `extensions::acceptance` 依赖 `crate::preview` 的
  `open_path`/`Message::PreviewOpenPath`)。改成验收模块自带 diff 渲染后,`extensions::
  acceptance` 除了内核拦截的 `Reject`(必须碰终端会话 PTY,见下)之外,不产生任何跨
  模块消息或函数调用依赖——`delivery.rs` 是内核公用模块,不算"耦合到另一个
  extension"。这条原则应作为以后所有新 extension 设计的默认起点,不只是这次的特例。
- **`file_diff` 与 `git_log::commit_detail` 独立实现,不共用代码**:两者都是"git2 算
  diff、按行染色"的同类需求,但一个是"提交 vs 父提交"(`diff_tree_to_tree`),一个是
  "验收基线 vs 工作区"(`diff_tree_to_workdir_with_index`)——API 调用不同,且共用
  会在两个 extension 之间(经由 `git_log`)建立不必要的耦合路径。各自在 `delivery.rs`/
  `git_log.rs` 独立实现,允许代码形状相似,不追求 DRY。
- **`Open`/`Reject` 两条是内核拦截点,不是"写计划时再定"的待定项**:brainstorming 首稿
  曾把 `Open` 要不要拦截写成待定("按实际借用情况定"),自查时回头核对现有
  `Message::AcceptanceOpen` 处理器发现这其实已经由代码决定了——`Open` 内部会
  `ws.tab_by_id_mut(tab_id)` 清空 `tab.delivery_pending`、取 `tab.effective_cwd()`,
  这两步都是终端会话域的操作,`extensions::acceptance` 不该认识 `SessionTab`,必须
  内核拦截(同 `Reject`)。`Accept` 不用拦截——发起 `delivery::accept`/`client.
  record_acceptance`(网络调用)只需要 `client: &Client`,`update` 接收这个参数即可
  处理(同浏览器试点 `browser::update` 接收 `client` 的方式)。`Reject` 需要往
  **来源终端会话**(`source_tab_id` 对应的 `SessionTab`)写入文本,内核拦截、直调
  `client.write(session_id, ..)`,再把"清空验收会话状态"这步转发给模块——同 Todo
  试点 `DispatchToExisting` 的处理方式,是这一原则的第五次应用;`Open` 是第六次
  应用,取完 `cwd`/清完 `delivery_pending` 后调用 `acceptance::spawn_open`(自由
  函数,做真正的异步扫描)。
- **三个异步结果消息都要按 `project_id` 路由**:`Loaded`/`DiffLoaded`/`Done` 都是
  异步任务的结果,自查时核对了一遍——`DiffLoaded` 首稿漏带了 `project_id`(只有
  `usize`/`Result`),已经补上,三个变体统一在内核走 `with_project(project_id, ..)`
  而非 `with_focused_project`(同前四个试点已验证的不变式)。`Toggle`/`ToggleDiff`/
  `CommentClick`/`CommentEvent`/`Accept` 这些用户交互消息走 `with_focused_project`。
- **图标徽标的判断条件由内核算,不下放给模块**:`ws.tabs.get(ws.active).map(|t|
  t.delivery_pending)` 是终端会话域的数据,`extensions::acceptance` 不该认识
  `SessionTab`——内核直接读、直接画在图标上,不经过 `acceptance::Message`/
  `WorkspaceState`。这跟 Todo 试点"内核摘要传给扩展"的方向类似,但这次连摘要都不用
  传:摘要只是画在图标上的一个 bool,不是模块渲染内容的一部分。

## 架构与数据流

### 1. 状态类型

```rust
/// 挂在每个 Workspace 上的验收面板状态。
#[derive(Default)]
pub struct WorkspaceState {
    session: Option<AcceptanceSession>,
}

/// 一次进行中的验收(现有 `AcceptanceView` 的搬家版本,新增
/// `expanded`/`diffs` 两个字段支撑 diff 手风琴)。
pub struct AcceptanceSession {
    repo: PathBuf,
    source_tab_id: usize,
    goal: Option<Goal>,
    changes: Vec<FileChange>,
    checked: Vec<bool>,
    /// 当前展开了 diff 的文件下标(对应 `changes` 的下标)。
    expanded: HashSet<usize>,
    /// diff 懒加载缓存:未展开过或仍在加载中的文件不在这个 map 里。
    diffs: HashMap<usize, Result<String, String>>,
    comment: String,
    comment_editing: bool,
    error: Option<String>,
    accepted_version: Option<u32>,
}
```

`WorkspaceState` 对外暴露只读访问器(`session()`)和内核需要的判断方法,具体清单写
计划时按 `view`/内核接线实际需要的调用点定(参考前几个试点"字段私有、按需开访问器"
的节奏)。

### 2. `Message`

```rust
pub enum Message {
    /// 内核拦截,不进 `update`——要清空 `SessionTab.delivery_pending`、取
    /// `effective_cwd()`,这两步都是终端会话域操作,见"关键语义确认"。
    Open(usize),
    Loaded(i64, PathBuf, usize, Option<Goal>, Vec<FileChange>),
    Toggle(usize),
    ToggleDiff(usize),
    /// 异步结果,带 `project_id`——同 `Loaded`/`Done`,必须按它走
    /// `with_project`,不能走 `with_focused_project`(见"关键语义确认"
    /// 里的路由不变式)。
    DiffLoaded(i64, usize, Result<String, String>),
    CommentClick,
    CommentEvent(AddrEvent),
    Accept,
    /// 内核拦截,不进 `update`——打回意见要写回来源终端会话的 PTY,
    /// `update` 拿不到 `Client`/session 这些终端域能力(见"关键语义
    /// 确认")。
    Reject,
    Done(i64, Result<u32, String>),
}
```

对应现有 8 个 `Acceptance*` 变体去前缀搬来,新增 `ToggleDiff`/`DiffLoaded` 两条。
`Open`/`Reject` 传进 `update` 会 `unreachable!`(同 Files 试点 `CopyPath`)。

### 3. `update`

```rust
pub fn update(
    ws_state: &mut WorkspaceState,
    msg: Message,
    project_id: i64,
    client: &Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
)
```

- `Open`:`unreachable!`——内核拦截处理,见下节"内核直调/拦截的处理"。
- `Loaded(..)`:落地 `AcceptanceSession`(`checked` 按 `goal.criteria.len()` 初始化全
  false,`expanded`/`diffs` 初始为空)。
- `Toggle(i)`:切换 `checked[i]`。
- `ToggleDiff(i)`:若 `i` 在 `expanded` 里则移除(收起);否则加入 `expanded`,若
  `diffs` 里还没有这个下标的缓存,发起异步 `delivery::file_diff(repo, &changes[i]
  .path)`,完成后 `emit(DiffLoaded(project_id, i, result))`。
- `DiffLoaded(_, i, result)`:写入 `diffs[i] = result`(内核已经按消息自带的
  `project_id` 路由到正确的 `ws_state`,函数体内不需要再读一次)。
- `CommentClick`/`CommentEvent`:同现有逻辑,原样搬。
- `Accept`:异步 `delivery::accept(repo)` + 成功后 `client.record_acceptance(..)`,
  完成后 `emit(Done(project_id, result))`——现有 `acceptance_accept` 方法体原样照抄,
  `client`/`handle` 从参数拿而不是 `io: &ShellIo`。
- `Done(_, result)`:`Ok(n) => accepted_version = Some(n)`,`Err(e) => error =
  Some(e)`,成功时内核额外触发 `spawn_acceptance_count_refresh`(这一步仍在内核侧,
  `update` 不负责,同现有 `Message::AcceptanceDone` 处理器"落地状态"和"触发计数刷新"
  两件事分层的方式)。
- `Reject`:`unreachable!("由内核拦截处理,见 Message::Reject 文档")`。

### 4. 内核直调/拦截的处理

**`Open(tab_id)`**:内核在转发进 `acceptance::update` 之前拦截。`self
.with_focused_project(|ws, io| { .. })` 内部:`ws.tab_by_id_mut(tab_id)` 清空
`tab.delivery_pending = false`、取 `tab.effective_cwd()` 算出 `cwd`(逻辑同现有
`effective_project_repo(active_repo.as_deref(), &tab.effective_cwd())`),然后调用:

```rust
pub fn spawn_open(
    project_id: i64,
    tab_id: usize,
    cwd: PathBuf,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
)
```

——自由函数,内部异步跑 `delivery::repo_root`/`goal::parse_goal`/`delivery::changes`
(现有 `Message::AcceptanceOpen` 处理器 `io.handle.spawn` 那部分逻辑原样照抄),完成后
`emit(Message::Loaded(project_id, repo, tab_id, goal, changes))`。

**`Reject`**:内核在 `Message::Acceptance(acceptance::Message::Reject)` 到达
`acceptance::update` 之前拦截:取 `session.source_tab_id`/`comment`,查
`ws.tabs`(终端域)里对应的活会话,写入 `"[Dozer 验收打回] {comment}\n"`
(现有 `acceptance_reject` 方法的核心逻辑),然后调用 `extensions::acceptance` 暴露的
一个普通函数(比如 `clear_session(&mut WorkspaceState)`)清空验收状态——不经过
`Message`/`update`,同 Todo 试点"内核完成终端相关部分后直调扩展普通函数"的处理方式。

`delivery::file_diff` 签名:

```rust
/// 单个文件相对验收基线(`delivery::changes` 用的同一套 base 解析:
/// 有上次沉淀取那个 ref,没有则 HEAD)的 unified diff 原始文本。用
/// `git2::Repository::diff_tree_to_workdir_with_index`(基线 tree vs
/// 当前工作区,含已 stage 的改动)。过长截断,截断阈值/提示文案参考
/// `git_log::commit_detail` 的 `MAX_PATCH_CHARS` 处理方式(独立实现,
/// 不共用常量)。
pub fn file_diff(repo: &Path, path: &str) -> Result<String, String>
```

### 5. `view`

```rust
pub fn view<'a>(
    ws_state: &'a WorkspaceState,
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer>
```

`session` 为 `None` 时渲染空态("没有待验收的交付",风格同 Usage 面板空态)。有
`session` 时结构基本照抄现有 `acceptance_content`:目标标题+标准复选框列表(点击发
`Toggle`)、变更文件列表(点击发 `ToggleDiff`,展开时在该行下方插入一段等宽文本渲染
`diffs.get(i)` 的内容——`Ok(patch)` 按行首字符 `+`/`-`/` ` 分三色渲染,`Err(e)` 显示
红字错误,还没加载完显示"加载中…")、验收意见输入框(点击发 `CommentClick`,键盘事件
发 `CommentEvent`)、通过/打回两个按钮、错误文案。`accepted_version.is_some()` 时只
显示"✓ 已沉淀 v<n>"(现有 `acceptance_content` 提前 return 那部分逻辑不变)。

### 6. 内核侧(`workspace.rs`/`preview.rs`)改动

- `RightView` 加 `Acceptance` 变体,`RailButton` 加 `RightAcceptance`。
- `right_icon_rail` 新增第 4 个按钮,读 `ws.tabs.get(ws.active).map(|t|
  t.delivery_pending).unwrap_or(false)` 决定要不要画金点徽标。
- `Message::RightIconSelect(RightView::Acceptance)` 分支(切到该视图时,若之前不在
  该视图上):用当前激活 tab 的 id 发起等价于 `Open(tab_id)` 的调用。
- 顶层 `Message` 删除 8 个 `Acceptance*` 变体,加
  `Acceptance(acceptance::Message)`。`AcceptanceView`/`acceptance_accept`/
  `acceptance_reject`/`acceptance_content`/`banner_text`/`criteria_line`/
  `file_change_line` 等现有类型/方法/函数搬进 `extensions::acceptance` 或删除
  (`banner_text` 随横幅一起删)。
- `terminal_pane` 删除"交付横幅"那一段(`banner_text`/`Message::AcceptanceOpen` 按钮)。
- `preview.rs`:删除 `TabKind::Acceptance`、`open_acceptance`、`acceptance_active`,
  `desired_webviews` 里 `overlay_active` 相关逻辑一并删除(`visible: idx ==
  self.active` 不再需要 `&& !overlay_active` 那部分)。
- `right_panel_area` 的 `RightView::Acceptance` 分支调用 `acceptance::view(&ws
  .acceptance, ..).map(Message::Acceptance)`。
- `update()` 对 `Message::Acceptance(..)` 分四支(汇总第 3/4 节已经分别讲过的路由
  规则):`Open(tab_id)` 全部拦截(见"内核直调/拦截的处理");`Reject` 全部拦截(同上);
  `Loaded`/`DiffLoaded`/`Done` 三个按自带的 `project_id` 走
  `with_project(project_id, ..)` 转发进 `acceptance::update`;其余(`Toggle`/
  `ToggleDiff`/`CommentClick`/`CommentEvent`/`Accept`)走 `with_focused_project`
  转发。跟 Files 试点的四支结构同源,这次因为没有 `AppState` 不需要
  `loaded_workspace_mut` 借用分离技巧。

## 错误处理

不新增错误处理路径,原样保留:
- 目标未定标(`.dozer/goal.md` 不存在/解析失败)→ 显示"未定标——先在仓库写
  .dozer/goal.md…"提示,不阻止验收流程(可以直接看变更文件/写意见/通过或打回)。
- `delivery::accept` 失败 → 错误文案显示在面板里,不清空验收状态,允许重试。
- 落库(`record_acceptance`)失败但 git ref 已写成功 → 沉淀本身算成功(git ref 是真相
  源),只提示"已沉淀 v<n>,但记录落库失败: {e}"(现有 `acceptance_accept` 行为)。
- 打回时来源会话已结束(非活)→ 提示"会话已结束,意见无处可注"(现有行为)。
- `file_diff` 失败(仓库读取错误/文件已被后续改动删除等)→ 该文件展开区显示错误文案,
  不影响其它文件/不影响整个面板。

## 测试策略

- `delivery::file_diff` 新增单测:新增文件(全 `+` 行)、修改文件(`+`/`-`/` ` 混合)、
  删除文件、过长截断。
- `extensions::acceptance::update` 新增单测:`Toggle` 切换、`ToggleDiff` 展开/收起 +
  未缓存时触发加载、`DiffLoaded` 写入缓存、`Accept`/`Done` 状态落地、`Open`/`Reject`
  传入时各自 `unreachable!`。
- 现有纯函数测试(`criteria_check_line_renders_gold_check` 等,函数名随搬迁调整)
  原样保留。
- `preview.rs` 里 `TabKind::Acceptance`/`acceptance_active` 相关的现有测试
  (`open_acceptance` 那两个)随删除一并移除。
- 人工验收:右侧新图标(有/无待验收徽标两态)、点图标随时进入验收(不需要先看到
  横幅)、展开/收起变更文件 diff(红绿行正确)、勾选标准、写验收意见、通过流程(沉淀
  版本号+计数刷新)、打回流程(意见注回来源会话 PTY,可在终端里看到)、`preview.rs`
  精简后普通文件预览不受影响。

## 依赖变更

无新增依赖(`git2` 已经是既有依赖,`diff_tree_to_workdir_with_index` 是其现有 API)。
