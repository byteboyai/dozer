# Files(项目文件树)面板扩展化设计

**状态:已批准(brainstorming 会话,2026-08-07)**

## 背景

Git Log(App 级共享状态)、浏览器(per-project 状态 + 收藏夹整体划归)、Todo(状态两块 +
派发逻辑跨终端/会话领域)三个阶段 1 试点均已完工并接入内核,只剩各自的人工 GUI 验收未走。
原始候选清单(git-log/浏览器/Todo)到此已全部落地,这次是重新评估后新定的**第四个**候选:
Files 面板(左侧栏项目信息卡 + git 分支/脏标 + 文件树)。

评估时点检了 `workspace.rs`(现 10223 行)里还没套上阶段 1 模式的几块:Conversations(仅
2 个消息,体量太小,价值有限)、Usage(已有独立 `usage.rs` 且几乎全是纯函数,只差正式套壳)、
Files(`project_pane` 一个函数 + 约 20 个 `ProjectTreeXxx` 消息,是三者里最大的一块)。选
Files 是因为它对瘦身 `workspace.rs` 的贡献最直接,且引入了前三个试点都没覆盖过的新形状:
**多个带副作用的异步文件系统操作(复制/粘贴/删除/重命名)+ 确认/取消弹层状态**,能验证"内核
拦截碰系统资源的消息"这套打法在文件系统场景下是否依然成立。

沿用同一套"自己的 `Message`/`State`/`update`/`view`,内核包装转发"的阶段 1 模式。

## 目标 / 非目标

**目标**:
1. 新建 `extensions::files`,拥有自己的 `Message`/`update`/`view`,内核只留一个包装变体
   `Message::Files(files::Message)` 做转发。
2. `project_pane`(项目信息卡:项目名/git 分支图标+名字/脏标颜色/验收次数 + 文件树可滚动
   列表)整体搬入,渲染逻辑原样保留。
3. `FileTree`(已在 `project.rs`,不用重写,直接被 `files::WorkspaceState` 持有)、
   `TreeEdit`/`TreeEditMode`(项目树行内编辑,新建/重命名共用)、`ContextMenu`(右键菜单
   浮层状态)三个现有类型随所属字段一起搬进 `extensions::files`。
4. **状态拆成两块**(跟 Todo 试点一样是两块,但理由不同——见下节"关键语义确认"):
   - `files::WorkspaceState`——挂在每个 `Workspace` 上,对应现有 11 个字段
     (`file_tree`/`branch`/`dirty`/`git_statuses`/`worktrees`/`project_acceptance_count`/
     `tree_selected`/`tree_clipboard`/`tree_error`/`tree_delete_confirm`/`tree_edit`)。
   - `files::AppState`——挂在 `App` 上,对应现有 2 个字段(`context_menu`/
     `last_right_click`)。
5. `spawn_project_git_refresh`(现 `Workspace` 方法,异步跑 `delivery::branch`/
   `is_dirty`/`file_statuses`/`worktrees` 四个 git 查询,产出 `Message::ProjectGitRefreshed`)
   原样变成 `extensions::files` 的自由函数,内核在全部 4 个现有调用点直接调用,不经过
   `Message`/`update` 分发。
6. `ProjectTreeCopyPath`(内核拦截,真正的系统剪贴板写入需要 `main.rs` 的 `Clipboard` 句柄)
   由内核在到达 `files::update` 之前拦截,不下放——跟 Todo 试点
   `DispatchToExisting`/`DispatchNew` 同一处理方式的第四次应用。
7. 内核为 `worktrees` 字段开一个只读访问器(如 `files::WorkspaceState::worktrees(&self)
   -> &[WorktreeInfo]`),供内核里的 `worktree_strip`(只在 `LeftView::GitLog` 时渲染,
   跟 Git Log 试点是两回事,是独立的自由函数)继续读取——见下节说明为什么这个字段挂
   Files 却被 Git Log 视图消费。

**非目标**:
- 不碰 `preview_pane`(核心,布局上跟 `project_pane` 拼在同一个 `row!` 里,内部逻辑完全
  独立,不动)。
- 不碰 `extensions::git_log`——`ProjectFsChanged` 里触发 Git Log 快照重建的那段分支逻辑
  留在内核不动(这条消息本来就同时喂给 Files 和 Git Log 两个独立扩展)。
- 不改任何用户可见行为:文件树展开/收起记忆、右键菜单选项、删除走系统废纸篓
  (`trash::delete`)、粘贴冲突处理、重命名/新建的行内编辑体验,一律原样保留。
- 不建 `Extension` trait/注册表,不拆独立 crate。

## 关键语义确认(brainstorming 会话定案)

- **状态两分的理由跟 Todo 不同**:Todo 的两块状态是因为派发逻辑本身有跨项目的业务耦合;
  Files 这里 11 个字段几乎全是纯 per-project 数据,唯一的例外是 `context_menu`/
  `last_right_click`——这两个现在就是 App 级(不是这次要改的),原因是右键坐标在"知道点的
  是哪个项目"之前就已经产生,且菜单本身是屏幕空间的单例浮层,不随项目切换各自保留状态
  (切项目时应该直接关掉菜单,这是现状,原样保留)。`update` 不需要像 Todo 那样为业务原因
  反复横跳两块状态,只有 `ContextMenuOpen`(见下)这一个消息同时碰两块。
- **`worktrees` 字段归属 vs 消费方不一致**:它跟 `branch`/`dirty`/`git_statuses` 是同一次
  `spawn_project_git_refresh` 刷出来的,理应跟着一起进 `files::WorkspaceState`;但渲染上
  它只在 `left_view == LeftView::GitLog` 时被内核里的 `worktree_strip` 自由函数拿去画在
  git log 图上方(不属于 `extensions::git_log` 自己的 `view`,是内核在 git log 视图外面
  包了一层)。处理方式:字段留在 `files::WorkspaceState`(跟着数据来源走),内核访问改走
  一个只读访问器,不算跨模块违规——这是继"内核摘要传入避免模块认识核心领域类型"
  (Todo 试点)之后,同一类"数据归属与消费方不必是同一模块"问题的另一种解法:上次是内核
  摘要传给扩展,这次是扩展开访问器给内核读。
- **`ProjectFsChanged` 保持内核级双派发,不下放**:这条消息是文件系统监听的总入口,一到
  同时触发两件事——调 `files::spawn_git_refresh`(刷 Files 的四个字段)、以及条件触发
  Git Log 快照重建(`git_watch::Relevance::GitRefs` 且是当前聚焦项目时)。内核继续拦截、
  分别转发给两个扩展各自的入口,两个扩展互不知道对方存在。这条消息本身**不**包进
  `files::Message`。
- **`ProjectTreeCopyPath` 内核拦截,`ProjectTreeRevealInFinder` 不用**:两条都是"点右键
  菜单触发 OS 级副作用"的消息,但拦截与否不同——`CopyPath` 真正的剪贴板写入需要
  `iced_winit::Clipboard` 句柄,只有 `main.rs` 事件循环里才有,`update()` 拿不到,所以
  消息类型属于 `files::Message`,但内核 `update()` 匹配到它时不转发进 `files::update`
  (维持现状"空分支,`main.rs` 里另外处理"),完事后 `main.rs` 自己调
  `app.update(Message::Files(files::Message::ContextMenuClose))` 关菜单。`RevealInFinder`
  虽然也是 OS 副作用(`open -R` 子进程),但不需要窗口句柄之类"`update()` 拿不到的资源",
  整条逻辑原样进 `files::update`,不用内核特案。
- **异步操作按 `project_id` 路由,不是新设计**:`ProjectTreePaste`/`ProjectTreeDeleteConfirm`
  发起的 `tokio::spawn` 结果(`PasteDone`/`OpDone`)现在就带 `project_id`,落地时用
  `self.with_project(project_id, ...)` 而不是"当前聚焦项目"——这是保护并行多项目场景不
  发生结果错投的既有不变式,迁移时原样保留。
- **`FileTree`/`TreeEdit`/`ContextMenu` 三个类型直接搬迁,不用改设计**:`FileTree` 已经是
  `project.rs` 里独立的纯数据类型(懒加载展开、可见行摊平,不碰 iced),本来就没有跟
  `workspace.rs` 耦合,`files::WorkspaceState` 直接持有一份即可。`TreeEdit`/
  `TreeEditMode`/`ContextMenu` 目前定义在 `workspace.rs` 里,原样剪切进
  `extensions::files`。

## 架构与数据流

### 1. 状态类型

```rust
/// 挂在每个 Workspace 上的 Files 面板状态(项目信息卡 + 文件树 + 树操作弹层)。
pub struct WorkspaceState {
    file_tree: Option<crate::project::FileTree>,
    branch: Option<String>,
    dirty: bool,
    git_statuses: HashMap<PathBuf, FileGitStatus>,
    worktrees: Vec<WorktreeInfo>,
    project_acceptance_count: Option<u64>,
    tree_selected: Option<PathBuf>,
    tree_clipboard: Option<(PathBuf, bool)>,
    tree_error: Option<String>,
    tree_delete_confirm: Option<(PathBuf, bool)>,
    tree_edit: Option<TreeEdit>,
}

impl WorkspaceState {
    /// 供内核 `worktree_strip`(Git Log 视图外层装饰)读取,详见"关键语义确认"。
    pub fn worktrees(&self) -> &[WorktreeInfo] {
        &self.worktrees
    }
}

/// 挂在 App 上的右键菜单浮层状态(屏幕空间单例,不随项目切换各自保留)。
pub struct AppState {
    context_menu: Option<ContextMenu>,
    last_right_click: (f32, f32),
}
```

`TreeEditMode`/`TreeEdit`/`ContextMenu` 原样剪切进 `extensions::files`(定义不变)。

### 2. `Message`

```rust
pub enum Message {
    Toggle(PathBuf),
    GitRefreshed(i64, Option<String>, bool, HashMap<PathBuf, FileGitStatus>, Vec<WorktreeInfo>),
    AcceptanceCountLoaded(i64, Option<u64>),
    RightClickAt { x: f32, y: f32 },
    ContextMenuOpen { path: PathBuf, is_dir: bool },
    ContextMenuClose,
    CopyPath(PathBuf, crate::project::PathKind), // 内核拦截,不进 update
    RevealInFinder(PathBuf),
    Copy(PathBuf, bool),
    Paste(PathBuf),
    PasteDone(i64, Result<PathBuf, String>),
    DeleteRequest(PathBuf, bool),
    DeleteConfirm,
    DeleteCancel,
    OpDone { project_id: i64, parent: Result<PathBuf, String>, expand: bool },
    NewFile(PathBuf),
    NewFolder(PathBuf),
    RenameStart(PathBuf),
    ReloadFromDisk,
    EditEvent(AddrEvent),
}
```

对应现在顶层 `Message` 里的 20 个 `ProjectTreeXxx`/`ProjectGitRefreshed`/
`AcceptanceCountLoaded`/`RightClickAt` 变体,去前缀原样搬来。**`GitRefreshed`/
`PasteDone`/`OpDone`/`AcceptanceCountLoaded` 这四个是异步结果消息,`project_id` 必须留在
变体内部,不是待定项**:现有代码里它们用 `self.with_project(project_id, ..)` 路由(哪怕
用户已经切到别的项目,结果也要投回发起它的项目,防止"结果错投"——这是并行多项目场景的
既有不变式),不能跟其余消息一样走 `with_focused_project`;内核 `update()` 的顶层 `match`
必须先看到 `project_id` 才能决定走哪条路由,所以不能包在外层、也不能省略,原样保留现有
签名形状。

### 3. `update`

```rust
/// 处理 `CopyPath` 之外的全部消息。内核在到达这里之前已经拦截了 `CopyPath`
/// (见"关键语义确认"),它传进来会 `unreachable!`(同 Todo 试点
/// `DispatchToExisting`/`DispatchNew` 的处理方式)。
pub fn update(
    ws_state: &mut WorkspaceState,
    app_state: &mut AppState,
    msg: Message,
    project_id: i64,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + Sync + 'static,
)
```

`project_id` 参数是内核路由时已经解析出的值(见下节"内核侧改动"),对 `GitRefreshed`/
`PasteDone`/`OpDone`/`AcceptanceCountLoaded` 这四个变体而言与消息自带的 `project_id`
字段恒相等——内核路由阶段已经用它选中了正确的 `ws_state`,函数体内不需要再读一次消息里
的那份。

- `Toggle(dir)`:`ws_state.tree_selected = Some(dir.clone())` + `file_tree.toggle(&dir)`
  (照搬现有 `ProjectTreeToggle`)。
- `GitRefreshed(..)`:直接赋值 `branch`/`dirty`/`git_statuses`/`worktrees` 四个字段。
- `AcceptanceCountLoaded(n)`:`ws_state.project_acceptance_count = n`。
- `RightClickAt { x, y }`:`app_state.last_right_click = (x, y)`。
- `ContextMenuOpen { path, is_dir }`:`ws_state.tree_selected = Some(path.clone())` +
  `app_state.context_menu = Some(ContextMenu { x: app_state.last_right_click.0, y: ..1,
  target: path, is_dir })`(照搬现有分两步:先记 `last_right_click`,菜单打开时读出来)。
- `ContextMenuClose`:`app_state.context_menu = None`。
- `RevealInFinder(path)`:`app_state.context_menu = None` + `Command::new("open").arg("-R")…
  .spawn()`(不需要内核特案,见"关键语义确认")。
- `Copy(path, is_dir)`:`app_state.context_menu = None` + `ws_state.tree_clipboard =
  Some((path, is_dir))`。
- `Paste(target_dir)`:`app_state.context_menu = None`,`ws_state.tree_error = None`,取
  `tree_clipboard`,`handle.spawn(...)` 跑 `project::paste_item`,结果经 `emit` 送回
  `Message::Files(files::Message::PasteDone(project_id, result))`。
- `PasteDone(_, result)`:成功则 `file_tree.refresh(parent)`,失败则 `tree_error = Some(e)`。
- `DeleteRequest(path, is_dir)`:`app_state.context_menu = None` + `ws_state
  .tree_delete_confirm = Some((path, is_dir))`。
- `DeleteCancel`:`ws_state.tree_delete_confirm = None`。
- `DeleteConfirm`:取出 `tree_delete_confirm`,`tree_error = None`,`handle.spawn(...)` 跑
  `trash::delete`,结果经 `emit` 送回 `OpDone { project_id, parent, expand: false }`。
- `OpDone { parent, expand, .. }`:成功则 `tree_error = None` + `file_tree.refresh(parent)`
  (`expand` 为真则 `ensure_expanded`),失败则 `tree_error = Some(e)`。
- `NewFile(parent)`/`NewFolder(parent)`:`app_state.context_menu = None` +
  `ws_state.start_tree_new(parent, TreeEditMode::NewFile/NewFolder)`(现有 `Workspace`
  方法 `start_tree_new` 搬成 `WorkspaceState` 方法)。
- `ReloadFromDisk`:`app_state.context_menu = None`,`tree_error = None`,
  `file_tree.reload_from_disk()`。
- `RenameStart(path)`:`app_state.context_menu = None`,`tree_error = None`,构造
  `tree_edit = Some(TreeEdit { .. TreeEditMode::Rename(path) .. })`。
- `EditEvent(ev)`:`Text`/`Backspace` 改 `tree_edit.buffer`,`Cancel` 清空 `tree_edit`,
  `Submit` 调 `ws_state.submit_tree_edit(..)`(现有 `Workspace` 方法搬成 `WorkspaceState`
  方法;原逻辑里 `submit_tree_edit` 需要 `io: &ShellIo` 发起重命名的异步落盘+结果消息,
  搬迁后改吃 `handle`/`emit`,同 `Paste`/`DeleteConfirm`)。
- `CopyPath`:`unreachable!("由内核拦截处理")`。

### 4. 内核直调的普通函数(不经过 `update`)

```rust
/// 内核在项目打开/`ProjectFsChanged`/手动刷新等 4 个现有调用点直接调用,
/// 异步跑 4 个 git 查询,完成后经 emit 送回 `GitRefreshed`。现有
/// `Workspace::spawn_project_git_refresh` 的搬家版本,逻辑不变。
pub fn spawn_git_refresh(
    project_id: i64,
    repo_path: PathBuf,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + Sync + 'static,
);
```

### 5. `view`

```rust
pub fn view<'a>(
    ws_state: &'a WorkspaceState,
    app_state: &'a AppState,
    project: Option<&'a ProjectInfo>,
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer>
```

现有 `project_pane`(项目信息卡 + 文件树可滚动列表)、`tree_edit_row`、
`context_menu_popup` 整体搬过来,签名从吃 `app: &App, ws: &Workspace` 改吃上面这组参数,
`Message` 类型从顶层换成本模块的。`project` 用 `Option<&ProjectInfo>` 而不是让
`WorkspaceState` 自己持有(项目身份是内核概念,`files` 模块只认"文件树数据",不重复
`Workspace.project` 这份状态——同 Todo 试点"内核摘要传入,模块不认识核心领域类型"
的原则)。

### 6. 内核侧(`workspace.rs`)改动

`App` 新增字段 `files: files::AppState`(取代 `context_menu`/`last_right_click`)。

`Workspace` 上 11 个字段合并成 `files: files::WorkspaceState`。

顶层 `Message` 删除 20 个 `ProjectTreeXxx`/`ProjectGitRefreshed`/`AcceptanceCountLoaded`/
`RightClickAt` 变体,加 `Message::Files(files::Message)`。`ProjectFsChanged` 保留在顶层
不变(双派发入口,见"关键语义确认")。

`update()` 分三支,对应三种不同的路由需要:
- `Message::Files(files::Message::CopyPath(path, kind))`:维持现状空分支(`main.rs`
  单独处理写剪贴板 + 事后 `app.update(Message::Files(files::Message::ContextMenuClose))`)。
- `Message::Files(msg @ (files::Message::GitRefreshed(project_id, ..)
  | files::Message::PasteDone(project_id, ..)
  | files::Message::OpDone { project_id, .. }
  | files::Message::AcceptanceCountLoaded(project_id, ..)))`:异步结果,必须按消息自带的
  `project_id` 路由(哪怕已经切走了别的项目),用
  `loaded_workspace_mut(&mut self.projects, project_id)` 直接借 `&mut Workspace` 调用
  `files::update(&mut ws.files, &mut self.files, msg, project_id, &self.handle, emit)`。
- `Message::Files(msg)` 兜底(其余全部用户交互消息):`with_focused_project` 内取
  `project_id`(`ws.project_id()`),同样调用 `files::update(..)`。

两支都要处理`self.files`(App 级)与目标 `Workspace` 同时可变借用——同 Todo 试点:
`with_focused_project`/`with_project` 拿不到 `self.files`,改用
`loaded_workspace_mut(&mut self.projects, project_id)` 直接从 `self.projects` 借
`&mut Workspace`,跟 `&mut self.files` 是两个不同字段,Rust 允许分别借用。

`ProjectFsChanged` 分支里 `ws.spawn_project_git_refresh(io)` 改成调用
`files::spawn_git_refresh(project_id, repo_path, &self.handle, emit)`(另外 3 个现有调用
点——项目打开/`adopt_project`/手动刷新——同样改调这个新入口)。

内核 `worktree_strip(&ws.worktrees)` 改成 `worktree_strip(ws.files.worktrees())`。

`App::view()` 的 `LeftView::Files` 分支:`files::view(&ws.files, &app.files,
ws.project.as_ref(), ..).map(Message::Files)`,紧邻的 `preview_pane(..)` 调用不变。

`main.rs` 里原 `Message::ProjectTreeCopyPath(path, kind) => { .. }` 改匹配
`Message::Files(files::Message::CopyPath(path, kind))`。

## 错误处理

不新增错误处理路径,原样保留:
- 文件树读取失败(`read_dir` 出错)→ 缓存为空 vec,不 panic(`FileTree` 现有行为)。
- 粘贴/删除失败 → `tree_error` 显示红字,下次操作发起时清空;不影响文件树其余部分。
- 删除的是项目根(无父目录可刷新)→ 现有 `Err("删除的是项目根,无父目录可刷新".to_string())`
  路径保留。
- `RevealInFinder` 在非 macOS 或拉起失败 → 静默忽略,不是值得打断用户的错误(现有行为)。

## 测试策略

- `FileTree`(`project.rs`)现有单测不受影响,不搬动。
- 新增 `files::update`/`WorkspaceState`/`AppState` 的单测,镜像前三个试点的写法:
  tree toggle、右键菜单开关(含 `tree_selected` 联动)、复制/粘贴成功与失败路由、删除
  确认/取消、重命名提交/取消、新建文件/文件夹后 `tree_edit` 状态、`GitRefreshed` 落地后
  四个字段更新、`worktrees()` 访问器。
- 人工验收:展开/收起目录记忆、右键菜单全部选项(新建文件/文件夹/复制/粘贴/删除/重命名/
  复制路径/在 Finder 中显示)、删除走系统废纸篓可在 Finder 里找回、粘贴到不同目录、
  git 状态装饰(新增/修改/删除文件的颜色点)、`LeftView::GitLog` 时 worktree 速览条仍正常
  显示——对照 Files 面板现有验收清单走一遍,确认拆分没有改变任何可见行为。
- 编译期防回归:`cargo build -p dozer-app && cargo test -p dozer-app && cargo clippy -p
  dozer-app --all-targets && cargo fmt --check` 全绿。

## 依赖变更

无新增依赖。
