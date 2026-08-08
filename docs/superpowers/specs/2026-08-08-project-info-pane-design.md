# 项目信息(Project)面板设计

**状态:已批准(brainstorming 会话,2026-08-08)**

## 背景

用户提出新增一个"项目"面板,展示项目基本信息,并把现在顶栏(`top_bar`)的目标胶囊
(`goal_capsule_text`)移动进去。挖掘现状后发现这不是一次单纯的"新增面板"——它牵出
三处既有内容的归属问题:

1. `extensions::files` 的项目信息卡(项目名/git 分支+脏标/验收次数,画在文件树上方)
   跟"项目基本信息"直接重叠。
2. Files 面板底部状态条(`project_status_bar`,显示"文件·git {分支}·组件")也依赖
   同一份 branch/dirty 数据。
3. `branch`/`dirty`/`git_statuses`/`worktrees` 四个字段现在是**一次组合 git 刷新**
   (`files::spawn_git_refresh`)产出、全部塞在 `extensions::files::WorkspaceState`
   里的——其中 `git_statuses`(文件行彩色点)真正只有 Files 用,`branch`/`dirty`/
   `worktrees` 展示需求全部转移到新面板和 Git Log(worktree 速览条)。

brainstorming 过程中额外确认了两条会影响后续工作但这次不做的事:分支切换
(checkout)功能——现状完全没有,值得单独一轮设计,这次不做,留给用户以后升级
Files 文件树面板时处理;目标(goal)获得一个简单的应用内编辑入口(现状纯只读,
只能手改 `.dozer/goal.md`)。

## 目标 / 非目标

**目标**:
1. 新增 `LeftView::Project`(第 5 个左侧视图)+ `RailButton::LeftProject` + 新图标,
   替代现在 Files 面板头部的项目信息卡。
2. 新建 `extensions::project`,拥有自己的 `Message`/`WorkspaceState`/`update`/
   `view`。`WorkspaceState` 接管:`branch`/`dirty`/`worktrees`(现在挂在
   `files::WorkspaceState`)、`project_acceptance_count`(现在是
   `files::WorkspaceState` 字段,通过 `files::Message::AcceptanceCountLoaded`
   刷新)、`goal: Option<Goal>`(现在是 `Workspace.project_goal`,内核字段,
   `load_project_goal` 同步读取)。
3. **组合 git 刷新保留在内核,但分发成两条独立消息**:现有
   `Workspace::spawn_project_git_refresh`(一次 `spawn_blocking` 里调
   `delivery::branch`/`is_dirty`/`file_statuses`/`worktrees` 四个纯函数,4 个既有
   调用点:项目打开/`adopt_project`/回合结束/`ProjectFsChanged`)改成:拿到结果后
   分发 `Message::Files(files::Message::StatusesRefreshed(statuses))`(给 Files)
   和 `Message::Project(project::Message::GitRefreshed(project_id, branch, dirty,
   worktrees))`(给 Project)。两个 extension 互不知道对方存在,内核是唯一知道
   "这两份数据同源"的地方——`delivery.rs` 本身已经是公共的纯函数服务层,这次改动
   只是把"编排一次刷新、分发给多个消费者"这件事也交给内核,不下放给任何 extension。
4. `extensions::files::WorkspaceState` 删除 `branch`/`dirty`/`worktrees`/
   `project_acceptance_count`,只留 `file_tree`/`git_statuses`(文件行彩色点用)/
   `tree_*` 编辑态。Files 面板删除头部项目信息卡、删除底部状态条("文件·git
   {分支}·组件"那条),`view` 只剩文件树本体。
5. Git Log 面板读 worktree 速览条的访问器从 `files::WorkspaceState::worktrees()`
   挪到 `project::WorkspaceState::worktrees()`,渲染逻辑(`worktree_strip`,只在
   `LeftView::GitLog` 时显示)不变——不重复在 Project 面板里再画一份 worktree
   列表,同一份数据只在一处展示。
6. 顶栏目标胶囊(`goal_capsule_text` 调用点、`top_bar` 里那一段)整体删除,不留
   残余入口——跟 Usage/Todo/Acceptance 一致,rail 图标本身就是唯一入口。
7. 新面板展示:项目名、git 分支+脏标(信息卡原有展示逻辑原样保留)、验收次数、
   目标(标题+标准列表)。
8. 目标获得简单的应用内编辑入口:点标题进入编辑(复用本仓一贯的 `AddrEvent` 自绘
   输入风格);标准列表支持增删(复用 Todo 面板"加一条/删一条"的既有交互形状);
   每次修改立即写回 `.dozer/goal.md`(不做"暂存改动,手动保存",同 Todo 面板"改了
   就存盘"的既有习惯)。没有 `.dozer/goal.md` 时显示"未定标"+"设置目标"入口,
   输入标题即创建文件。新增 `goal::write_goal(repo: &Path, goal: &Goal) ->
   std::io::Result<()>`,按固定格式(`# {title}\n\n每行一条 - [ ] {criterion}`)
   整份重新生成文件。

**非目标**:
- 不做分支切换(checkout)——已确认留到以后单独一轮 brainstorming,这次只把
  `branch`/`dirty` 挪个地方展示,不新增任何"切换到另一个分支"的交互。
- 保存目标时整份重新生成 `.dozer/goal.md`,不做增量式保留其它手写内容的 patch
  ——`goal.rs` 文档本来就只定义"首个非空行=标题、`- [ ]`/`- [x]` 列表项=标准"这
  两种行的语义,不支持的自由格式文本本来就不在 schema 内,重新生成不算破坏数据。
- 不在 Project 面板里重复展示 worktree 列表(已在 Git Log 上方展示)。
- 不改变验收/Todo/Files 三个面板已有的业务逻辑,只搬状态归属和展示位置。
- 不建 `Extension` trait/注册表,不拆独立 crate。

## 关键语义确认(brainstorming 会话定案)

- **状态归属的判断标准是"谁展示就归谁",不是"谁先算出来的"**:`branch`/`dirty`/
  `worktrees`/`git_statuses` 原本一次 git 刷新产出、混在一起挂在 Files 上,是因为
  Files 试点当初唯一的消费方就是 Files 自己。这次因为多了 Project 面板这个新消费方,
  必须重新按"谁在展示"切分归属,不能延续"历史上是谁算出来的就归谁"——这是这次设计
  最核心的一条判断依据,以后再出现类似"一次计算、多处消费"的情况,应该用同一个
  标准处理。
- **多消费方的组合数据,编排权收归内核,不是拆成多次独立计算,也不是塞进某一个
  extension 代管**:brainstorming 一开始考虑过"两次独立 git 刷新"(方案 A)——
  简单但要多算一次;也考虑过"数据留在 Files,内核读访问器传给 Project"(方案
  B)——省事但语义别扭,Files 内部还留着自己不用的数据。最终定案:内核保留**一次**
  组合计算,算完后内核自己分发成多条独立消息喂给各个 extension——这是"git 已经是
  公共服务(`delivery.rs`),内核只是编排/分发点"这个原则的具体应用,以后任何"一份
  数据多个 extension 都要"的场景都应该套用这个模式,而不是重新在两个方案之间纠结。
- **这条原则的更大意义**:这次改动不只是让 Project/Files 两个 extension 各自更
  独立,也让"内核"这个概念本身更内聚——git 刷新的编排逻辑从"挂在 Workspace 上、
  被动等某个 extension 调用"变成"内核主动拥有、主动分发给多个 extension",内核
  从"转发者"变成真正的"编排者"。阶段 1 扩展化重构走到第 7 个试点,这条是目前
  为止最重要的一条架构结论。
- **排期依赖**:这个面板要改的 `Message` 枚举/`Workspace` 字段/`ProjectFsChanged`
  处理器,跟验收面板([[dozer-files-extension-pilot]] 系列的第六个试点,计划见
  `docs/superpowers/plans/2026-08-08-acceptance-pane.md`,implementation 由用户
  安排另一个 agent 执行中)改的是同一个文件(`workspace.rs`)。两者动的字段/消息
  不重叠(验收面板管 `Message::Acceptance`,这个管 `ProjectFsChanged` 的分发逻辑 +
  `Message::Project`),但同时改同一个大文件、都涉及删旧字段/加新分支,合并冲突
  风险不低——跟 Todo 试点等 Browser 试点 Task 5/6 落地是同一类排期问题。**这个面板
  的 implementation 应该等验收面板先合并完再开始**,设计/写计划阶段不受此排期
  限制。

## 架构与数据流

### 1. 状态类型

```rust
/// 挂在每个 Workspace 上的项目信息面板状态。
pub struct WorkspaceState {
    branch: Option<String>,
    dirty: bool,
    worktrees: Vec<WorktreeInfo>,
    project_acceptance_count: Option<u64>,
    goal: Option<Goal>,
    /// 目标标题的行内编辑态(None=未在编辑;创建新目标时也复用这个字段,
    /// `goal` 为 `None` 且 `title_editing` 有值就是"正在设置首个目标")。
    title_editing: Option<String>,
    /// 新增标准的输入草稿。
    add_criterion_draft: String,
    error: Option<String>,
}
```

`WorkspaceState` 对外暴露只读访问器(`branch()`/`dirty()`/`worktrees()`/
`goal()`/……),`worktrees()` 是供内核 `worktree_strip` 读取的那个(职责从
`files::WorkspaceState::worktrees()` 原样搬过来)。

### 2. `Message`

```rust
pub enum Message {
    /// 组合 git 刷新结果里跟 Project 有关的那部分,由内核分发。
    GitRefreshed(i64, Option<String>, bool, Vec<WorktreeInfo>),
    /// 从 `files::Message::AcceptanceCountLoaded` 搬来。
    AcceptanceCountLoaded(i64, Option<u64>),
    TitleEditStart,
    TitleEditEvent(AddrEvent),
    CriterionAddInputChanged(String),
    CriterionAddSubmit,
    CriterionRemove(usize),
}
```

`extensions::files::Message` 删除 `AcceptanceCountLoaded`/`GitRefreshed` 两个
变体,新增 `StatusesRefreshed(HashMap<PathBuf, FileGitStatus>)`(只剩文件级状态
这一份)。

### 3. `update` 与目标读写

```rust
pub fn update(
    ws_state: &mut WorkspaceState,
    msg: Message,
    project_id: i64,
    repo_path: &Path,
)
```

- `GitRefreshed(_, branch, dirty, worktrees)`:直接赋值三个字段。
- `AcceptanceCountLoaded(_, n)`:赋值 `project_acceptance_count`。
- `TitleEditStart`:`title_editing = Some(goal.as_ref().map(|g|
  g.title.clone()).unwrap_or_default())`(有目标预填当前标题,没有目标则空,
  两种场景共用同一个编辑态)。
- `TitleEditEvent(AddrEvent::Submit)`:取 `title_editing` 的内容,若目标已存在
  只改标题(标准列表不变),若不存在则新建 `Goal { title, criteria: vec![] }`;
  调用 `goal::write_goal(repo_path, &goal)`,成功则 `ws_state.goal =
  Some(goal)`、清空 `title_editing`,失败则写 `error`、保留编辑态不清空(允许
  重试)。
- `TitleEditEvent(AddrEvent::Text/Backspace/Cancel)`:同现有 `AddrEvent` 处理
  惯例(Files/Todo/Acceptance 都是这个模式)。
- `CriterionAddSubmit`:在 `goal.criteria` 末尾追加 `add_criterion_draft`
  (trim 后非空才追加),写盘,清空草稿。没有 `goal`(理论上不会发生,标准列表
  UI 只在有目标时渲染)时忽略。
- `CriterionRemove(i)`:从 `goal.criteria` 移除下标 `i`,写盘。

`goal::write_goal`:

```rust
/// 按固定格式整份重写 `.dozer/goal.md`:首行 `# {title}`,空行,然后每条
/// 标准各占一行 `- [ ] {criterion}`。不保留/不尝试合并文件里其它手写内容
/// (见设计文档"非目标")。若 `.dozer` 目录不存在则先创建。
pub fn write_goal(repo: &Path, goal: &Goal) -> std::io::Result<()>
```

### 4. 内核编排:组合 git 刷新分发

现有 `Workspace::spawn_project_git_refresh`(4 个调用点:`from_restore`/
`adopt_project`/回合结束/`ProjectFsChanged`)改成分发两条消息而非一条:

```rust
fn spawn_project_git_refresh(project_id: i64, repo_path: PathBuf, io: &ShellIo) {
    let proxy = io.proxy.clone();
    io.handle.spawn(async move {
        let (b, d, s, w) = tokio::task::spawn_blocking({
            let repo_path = repo_path.clone();
            move || {
                (
                    delivery::branch(&repo_path),
                    delivery::is_dirty(&repo_path),
                    delivery::file_statuses(&repo_path),
                    delivery::worktrees(&repo_path),
                )
            }
        })
        .await
        .unwrap_or((None, false, HashMap::new(), Vec::new()));
        let _ = proxy.send_event(Message::Files(files::Message::StatusesRefreshed(s)));
        let _ = proxy.send_event(Message::Project(project::Message::GitRefreshed(
            project_id, b, d, w,
        )));
    });
}
```

这个函数因为要同时认识 `files::Message`/`project::Message` 两个类型,不适合放进
任何一个 extension,是纯粹的内核私有自由函数(不再是 `Workspace` 的方法——原来是
方法是因为通过 `&self.project` 取 `project_id`/`repo_path`,现在改成参数显式传入,
调用点从"`ws.spawn_project_git_refresh(io)`"变成"`spawn_project_git_refresh
(project_id, repo_path, io)`",4 个既有调用点相应调整取参数的方式)。

`Message::Project(msg)` 的路由方式同前几个试点:`GitRefreshed`/
`AcceptanceCountLoaded` 是异步结果,带 `project_id`,走 `with_project`;
`TitleEditStart`/`TitleEditEvent`/`CriterionAdd*`/`CriterionRemove` 是用户
交互消息,走 `with_focused_project`。

### 5. `view` 与 rail 接线

`extensions::project::view` 结构:项目名 + 分支图标/标签(脏标变金,搬自现有
信息卡)+ "N 次验收"副行 + 目标区块(标题可点击进入编辑;有标准则逐条列出、每条
带删除按钮;底部"＋"输入框提交新标准;没有目标时显示"未定标"文案 + 可点击的
"设置目标"占位,点击进入同一个标题编辑态)。

`LeftView` 加 `Project`,`RailButton` 加 `LeftProject`,`left_icon_rail` 加第
5 个按钮(新图标——写计划时从 Lucide 里选一个贴切的,比如 `info` 或
`layout-panel-top`,不影响架构决策)。

Files 面板(`extensions::files::view`)删除项目信息卡与底部状态条部分,只剩文件树
本体渲染。顶栏 `top_bar` 删除目标胶囊那一段。

## 错误处理

- 写 `.dozer/goal.md` 失败(权限等)→ 面板内显示错误文案,不清空当前编辑内容,
  允许重试(同 Todo/Files 现有"写盘失败保留输入"的处理口径)。
- 非 git 项目 → `branch`/`worktrees` 是 `None`/空,面板照常显示"—"(现有
  `project_branch_label` 逻辑不变,不算错误状态)。
- `.dozer/goal.md` 解析失败(格式不对到连标题都提取不出来)→ 现状 `parse_goal`
  返回 `None`,面板按"未定标"处理,不报错(原样保留现有容错行为)。

## 测试策略

- `goal::write_goal` 新增单测:创建新文件(`.dozer` 目录不存在时自动建)、覆盖
  已有文件、标准列表清空后只剩标题一行。
- `extensions::project::update` 新增单测:`TitleEditStart` 预填当前标题、提交
  后创建/更新目标并写盘、`CriterionAddSubmit`/`CriterionRemove` 增删后写盘、
  `GitRefreshed`/`AcceptanceCountLoaded` 落地赋值。
- 现有 `goal_capsule_text`/`project_branch_label` 等纯函数测试原样搬(断言不变,
  只搬文件位置)。
- `extensions::files` 相关测试:`StatusesRefreshed` 替代原 `GitRefreshed` 的
  测试断言,删除信息卡/状态条渲染相关的既有测试(若有专门针对这两部分的测试)。
- 人工验收:新面板信息展示齐全;点标题编辑、增删标准,立即生效且重启 app 后还在;
  没有目标时的创建流程;Files 面板确认不再显示项目卡/底部状态条;顶栏确认不再有
  目标胶囊;Git Log 的 worktree 速览条不受影响,切换项目页签各自独立。

## 依赖变更

无新增依赖。
