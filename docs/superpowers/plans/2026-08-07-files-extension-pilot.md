# Files(项目文件树)面板扩展化 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `workspace.rs` 里的 Files 面板(`project_pane` 项目信息卡 + 文件树 + 右键菜单/删除确认浮层,约 20 个 `ProjectTreeXxx`/`ProjectGitRefreshed`/`AcceptanceCountLoaded`/`RightClickAt` 消息)拆成自洽模块 `extensions::files`(自己的 `Message`/`WorkspaceState`/`AppState`/`update`/`view`),`workspace.rs` 内核只留包装转发——阶段 1 扩展化重构的第四个试点。

**Architecture:** 新文件 `crates/dozer-app/src/extensions/files.rs`:`WorkspaceState`(挂 `Workspace`,对应现有 11 个字段)、`AppState`(挂 `App`,对应现有 2 个字段:`context_menu`/`last_right_click`)、`Message`(21 个变体,含一个设计文档遗漏、代码审查时补上的 `OpenFile`——文件行点击要跨过
`files::Message` 边界打开预览,处理方式同 `CopyPath`,见 Task 1 Step 1 `OpenFile` 变体的
文档注释)、`update`(处理 `CopyPath`/`OpenFile` 之外的全部消息)、`spawn_git_refresh`(内核直调的自由函数,替代 `Workspace::spawn_project_git_refresh`)、`view`/`context_menu_popup`/`delete_confirm_popup`(三个独立导出的渲染函数——后两个是 `App::view()` 顶层互斥浮层判断链的成员,不在 `project_pane` 的 `Element` 树里)。`TreeEditMode`/`TreeEdit`/`ContextMenu` 三个现有类型随字段一起搬入。`FileTree`(已在 `project.rs`)不搬,`files::WorkspaceState` 直接持有一份。

**Tech Stack:** Rust workspace;iced 0.14;`tokio::runtime::Handle` + `emit: impl Fn(Message) + Send + 'static` 回调风格(同 Git Log/浏览器两个试点),不用 `iced::Task`/`Command`。

## Global Constraints

- 纯重构,不改变任何用户可见行为——文件树展开/收起记忆、右键菜单选项、删除走系统废纸篓(`trash::delete`)、粘贴冲突处理(已存在同名项报错)、重命名/新建的行内编辑体验、`worktrees` 速览条只在 `LeftView::GitLog` 显示,一律原样保留。
- 不建 `Extension` trait/注册表,不拆独立 crate,不引入 `iced::Task`/`Command` 风格异步。
- `ProjectFsChanged` 保留在顶层 `Message`,不包进 `files::Message`——它同时触发 Files 的 git 刷新和 Git Log 的快照重建,是双派发入口(设计文档"关键语义确认")。
- `ProjectTreeCopyPath` 迁移后仍由内核在到达 `files::update` 之前拦截(真正的系统剪贴板写入需要 `main.rs` 的 `Clipboard` 句柄),`files::update` 收到它必须 `unreachable!`。
- `GitRefreshed`/`PasteDone`/`OpDone`/`AcceptanceCountLoaded` 四个异步结果消息必须自带 `ProjectId`,按 `with_project(project_id, ..)` 路由,不能走 `with_focused_project`。
- 每个任务结束都要 `cargo build -p dozer-app && cargo test -p dozer-app && cargo clippy -p dozer-app --all-targets && cargo fmt` 干净通过。
- 设计文档:`docs/superpowers/specs/2026-08-07-files-extension-pilot-design.md`(有疑问以它为准)。

---

### Task 1: `extensions::files` 类型骨架——`TreeEdit`/`ContextMenu`/`WorkspaceState`/`AppState`/`Message`

**Files:**
- Create: `crates/dozer-app/src/extensions/files.rs`
- Modify: `crates/dozer-app/src/extensions.rs`(加 `pub mod files;`)

**Interfaces:**
- Produces:`pub(super) enum TreeEditMode`、`pub(super) struct TreeEdit`、
  `pub(super) struct ContextMenu`、`pub struct WorkspaceState`(含 `Default` 实现)、
  `pub struct AppState`(含 `Default` 实现)、`pub enum Message`。本任务结束时这些类型
  还没有任何方法/`update`/`view`,只是定义齐全、能编译。

- [ ] **Step 1: 创建文件,写类型定义**

创建 `crates/dozer-app/src/extensions/files.rs`:

```rust
//! Files(项目文件树)面板:项目信息卡 + git 分支/脏标/worktree 速览数据 +
//! 文件树 + 右键菜单/删除确认浮层。阶段 1 扩展化重构第四个试点,设计见
//! `docs/superpowers/specs/2026-08-07-files-extension-pilot-design.md`。
use crate::delivery::FileGitStatus;
use crate::project::{FileTree, PathKind, WorktreeInfo};
use crate::workspace::AddrEvent;
use std::collections::HashMap;
use std::path::PathBuf;

/// 项目树行内编辑的模式:新建文件/新建文件夹/重命名(携带原路径)。
/// 现有 `workspace.rs::TreeEditMode` 的搬家版本,定义不变。
#[derive(Debug, Clone, PartialEq)]
pub(super) enum TreeEditMode {
    NewFile,
    NewFolder,
    Rename(PathBuf),
}

/// 项目树行内编辑态:新建/重命名共用。现有 `workspace.rs::TreeEdit` 的搬家
/// 版本,定义不变。
#[derive(Debug, Clone, PartialEq)]
pub(super) struct TreeEdit {
    parent_dir: PathBuf,
    mode: TreeEditMode,
    buffer: String,
}

/// 项目树右键菜单当前打开状态:定位坐标 + 目标(路径/是否目录)。现有
/// `workspace.rs::ContextMenu` 的搬家版本,定义不变。
#[derive(Debug, Clone, PartialEq)]
pub(super) struct ContextMenu {
    x: f32,
    y: f32,
    target: PathBuf,
    is_dir: bool,
}

/// 挂在每个 Workspace 上的 Files 面板状态(项目信息卡数据 + 文件树 + 树操作
/// 弹层)。对应现有 `Workspace` 上 11 个字段。
#[derive(Default)]
pub struct WorkspaceState {
    file_tree: Option<FileTree>,
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

/// 挂在 App 上的右键菜单浮层状态(屏幕空间单例,不随项目切换各自保留)。
/// 对应现有 `App` 上 `context_menu`/`last_right_click` 两个字段。
#[derive(Default)]
pub struct AppState {
    context_menu: Option<ContextMenu>,
    last_right_click: (f32, f32),
}

/// 对应现在顶层 `Message` 里的 20 个 `ProjectTreeXxx`/`ProjectGitRefreshed`/
/// `AcceptanceCountLoaded`/`RightClickAt` 变体,去前缀原样搬来,外加一个
/// `OpenFile`——它不是哪个 `ProjectTreeXxx` 去前缀来的,是设计文档遗漏、
/// 代码审查时才发现的缺口:原 `project_pane` 对非目录行发的是跨域消息
/// `Message::PreviewOpenPath`,不在 `ProjectTreeXxx` 家族里,`files::Message`
/// 需要自己的变体才能表达"点了一个文件"。
#[derive(Debug, Clone)]
pub enum Message {
    Toggle(PathBuf),
    GitRefreshed(
        i64,
        Option<String>,
        bool,
        HashMap<PathBuf, FileGitStatus>,
        Vec<WorktreeInfo>,
    ),
    AcceptanceCountLoaded(i64, Option<u64>),
    RightClickAt { x: f32, y: f32 },
    ContextMenuOpen { path: PathBuf, is_dir: bool },
    ContextMenuClose,
    /// 单击文件行(非目录):在内核里打开预览。设计文档遗漏了这一条——
    /// 原 `project_pane` 对非目录行的 `on_press` 发的是跨域消息
    /// `Message::PreviewOpenPath(row.path.clone())`(预览域,核心,不属于
    /// Files),`files::Message` 里没有能表达"打开预览"的变体就没法照抄。
    /// 处理方式同 `CopyPath`:内核拦截,不进 `update`,收到时转发成
    /// `self.update(Message::PreviewOpenPath(path))`(Task 4 Step 4 补一条
    /// 拦截分支,写在 `CopyPath` 拦截分支旁边)。
    OpenFile(PathBuf),
    /// 内核拦截,不进 `update`——真正的系统剪贴板写入需要 `main.rs` 的
    /// `Clipboard` 句柄,`update()` 拿不到(见设计文档"关键语义确认")。
    CopyPath(PathBuf, PathKind),
    RevealInFinder(PathBuf),
    Copy(PathBuf, bool),
    Paste(PathBuf),
    PasteDone(i64, Result<PathBuf, String>),
    DeleteRequest(PathBuf, bool),
    DeleteConfirm,
    DeleteCancel,
    OpDone {
        project_id: i64,
        parent: Result<PathBuf, String>,
        expand: bool,
    },
    NewFile(PathBuf),
    NewFolder(PathBuf),
    RenameStart(PathBuf),
    ReloadFromDisk,
    EditEvent(AddrEvent),
}
```

（`FileGitStatus`/`WorktreeInfo` 目前在 `delivery.rs`/`project.rs` 定义为 `pub`,直接
`use` 即可,不需要改可见性。`AddrEvent` 已经是 `pub` 且被浏览器/Todo 两个试点共用,同样
直接引用 `crate::workspace::AddrEvent`。）

- [ ] **Step 2: 声明模块**

`crates/dozer-app/src/extensions.rs`,按字母序插入:

```rust
pub mod browser;
pub mod files;
pub mod git_log;
pub mod todo;
```

- [ ] **Step 3: 编译确认**

```bash
cargo build -p dozer-app
```

Expected: 编译通过(`Message`/`WorkspaceState`/`AppState` 目前未被任何地方引用,允许
`dead_code` 警告,不允许报错)。

- [ ] **Step 4: Commit**

```bash
git add crates/dozer-app/src/extensions/files.rs crates/dozer-app/src/extensions.rs
git commit -m "feat(dozer-app): add extensions::files type skeleton"
```

---

### Task 2: `WorkspaceState`/`AppState` 方法 + `Message` + `update` + `spawn_git_refresh`

**Files:**
- Modify: `crates/dozer-app/src/extensions/files.rs`

**Interfaces:**
- Consumes:Task 1 的 `TreeEditMode`/`TreeEdit`/`ContextMenu`/`WorkspaceState`/`AppState`/
  `Message`。
- Produces:`WorkspaceState::new(file_tree: FileTree) -> Self`、
  `WorkspaceState::reset_for_project(&mut self, file_tree: FileTree)`、
  `WorkspaceState::worktrees(&self) -> &[WorktreeInfo]`、
  `WorkspaceState::tree_delete_confirm_is_some(&self) -> bool`、
  `AppState::context_menu_is_some(&self) -> bool`、
  `pub fn update(ws_state: &mut WorkspaceState, app_state: &mut AppState, msg: Message, project_id: i64, handle: &tokio::runtime::Handle, emit: impl Fn(Message) + Send + 'static)`、
  `pub fn spawn_git_refresh(project_id: i64, repo_path: PathBuf, handle: &tokio::runtime::Handle, emit: impl Fn(Message) + Send + 'static)`。

- [ ] **Step 1: `WorkspaceState` 构造/重置/访问器方法**

在 `impl WorkspaceState` 里加(紧跟 `#[derive(Default)] pub struct WorkspaceState {...}`
之后新开一个 `impl` 块):

```rust
impl WorkspaceState {
    /// 打开一个新项目时构造(现有 `Workspace::from_restore` 里
    /// `file_tree: Some(FileTree::new(..))` 那一步的搬家版本)。
    pub fn new(file_tree: FileTree) -> Self {
        Self {
            file_tree: Some(file_tree),
            ..Self::default()
        }
    }

    /// 复用中的 `Workspace` 认领另一个项目时重置(现有
    /// `Workspace::adopt_project` 里对应 5 行赋值的搬家版本,行为原样保留——
    /// 包括现状本来就没重置 `worktrees`/`tree_clipboard`/`tree_error`/
    /// `tree_delete_confirm`/`tree_edit` 这一点,纯迁移不新增行为)。
    pub fn reset_for_project(&mut self, file_tree: FileTree) {
        self.file_tree = Some(file_tree);
        self.tree_selected = None;
        self.branch = None;
        self.dirty = false;
        self.git_statuses = HashMap::new();
        self.project_acceptance_count = None;
    }

    /// 供内核 `worktree_strip`(Git Log 视图外层装饰,不属于
    /// `extensions::git_log`)读取——`worktrees` 数据来自这次 git 刷新,但
    /// 消费方是 Git Log 视图,见设计文档"关键语义确认"。
    pub fn worktrees(&self) -> &[WorktreeInfo] {
        &self.worktrees
    }

    /// 供内核判断"删除确认浮层该不该显示"(`App::view()` 顶层互斥浮层
    /// 判断链用,见设计文档 §5)。
    pub fn tree_delete_confirm_is_some(&self) -> bool {
        self.tree_delete_confirm.is_some()
    }

    /// "新建文件"/"新建文件夹"的公共起点(现有 `Workspace::start_tree_new`
    /// 的搬家版本,逻辑不变)。
    fn start_tree_new(&mut self, parent: PathBuf, mode: TreeEditMode) {
        self.tree_error = None;
        if let Some(tree) = &mut self.file_tree {
            tree.ensure_expanded(&parent);
        }
        self.tree_edit = Some(TreeEdit {
            parent_dir: parent,
            mode,
            buffer: String::new(),
        });
    }

    /// 行内编辑框回车提交(现有 `Workspace::submit_tree_edit` 的搬家版本:
    /// `io: &ShellIo` 换成 `handle`/`emit`,`self.project_id()` 换成显式传入
    /// 的 `project_id`,其余逻辑原样照抄,含三种模式的重名校验/路径分量
    /// 校验)。
    fn submit_tree_edit(
        &mut self,
        project_id: i64,
        handle: &tokio::runtime::Handle,
        emit: impl Fn(Message) + Send + 'static,
    ) {
        let Some(edit) = self.tree_edit.take() else {
            return;
        };
        self.tree_error = None;
        let name = edit.buffer.trim();
        if name.is_empty() {
            return;
        }
        if !crate::project::is_single_path_component(name) {
            self.tree_error = Some("名字不能包含路径分隔符".to_string());
            self.tree_edit = Some(TreeEdit {
                parent_dir: edit.parent_dir,
                mode: edit.mode,
                buffer: name.to_string(),
            });
            return;
        }
        let new_path = edit.parent_dir.join(name);
        match edit.mode {
            TreeEditMode::Rename(old_path) => {
                if new_path == old_path {
                    return;
                }
                if new_path.exists() {
                    self.tree_error = Some(format!("{} 已存在同名项", new_path.display()));
                    self.tree_edit = Some(TreeEdit {
                        parent_dir: edit.parent_dir,
                        mode: TreeEditMode::Rename(old_path),
                        buffer: name.to_string(),
                    });
                    return;
                }
                let parent = edit.parent_dir.clone();
                handle.spawn(async move {
                    let result = tokio::task::spawn_blocking(move || {
                        std::fs::rename(&old_path, &new_path).map_err(|e| e.to_string())
                    })
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()));
                    let outcome = result.map(|()| parent);
                    emit(Message::OpDone {
                        project_id,
                        parent: outcome,
                        expand: false,
                    });
                });
            }
            TreeEditMode::NewFile => {
                if new_path.exists() {
                    self.tree_error = Some(format!("{} 已存在同名项", new_path.display()));
                    self.tree_edit = Some(TreeEdit {
                        parent_dir: edit.parent_dir,
                        mode: TreeEditMode::NewFile,
                        buffer: name.to_string(),
                    });
                    return;
                }
                let parent = edit.parent_dir.clone();
                handle.spawn(async move {
                    let result = tokio::task::spawn_blocking(move || {
                        std::fs::File::create(&new_path)
                            .map(|_| ())
                            .map_err(|e| e.to_string())
                    })
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()));
                    let outcome = result.map(|()| parent);
                    emit(Message::OpDone {
                        project_id,
                        parent: outcome,
                        expand: true,
                    });
                });
            }
            TreeEditMode::NewFolder => {
                if new_path.exists() {
                    self.tree_error = Some(format!("{} 已存在同名项", new_path.display()));
                    self.tree_edit = Some(TreeEdit {
                        parent_dir: edit.parent_dir,
                        mode: TreeEditMode::NewFolder,
                        buffer: name.to_string(),
                    });
                    return;
                }
                let parent = edit.parent_dir.clone();
                handle.spawn(async move {
                    let result = tokio::task::spawn_blocking(move || {
                        std::fs::create_dir(&new_path).map_err(|e| e.to_string())
                    })
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()));
                    let outcome = result.map(|()| parent);
                    emit(Message::OpDone {
                        project_id,
                        parent: outcome,
                        expand: true,
                    });
                });
            }
        }
    }
}
```

- [ ] **Step 2: `AppState` 访问器方法**

```rust
impl AppState {
    /// 供内核判断"右键菜单该不该显示"(`App::view()` 顶层互斥浮层判断链
    /// 用)。
    pub fn context_menu_is_some(&self) -> bool {
        self.context_menu.is_some()
    }
}
```

- [ ] **Step 3: `update`**

紧跟在类型定义之后加:

```rust
/// 处理 `CopyPath` 之外的全部消息。内核在到达这里之前已经拦截了
/// `CopyPath`(真正的剪贴板写入需要 `main.rs` 的 `Clipboard` 句柄),它传
/// 进来会 `unreachable!`(同 Todo 试点 `DispatchToExisting`/`DispatchNew`
/// 的处理方式)。`project_id` 由内核路由时已经解析出正确值传入。
pub fn update(
    ws_state: &mut WorkspaceState,
    app_state: &mut AppState,
    msg: Message,
    project_id: i64,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    match msg {
        Message::Toggle(dir) => {
            ws_state.tree_selected = Some(dir.clone());
            if let Some(tree) = &mut ws_state.file_tree {
                tree.toggle(&dir);
            }
        }
        Message::GitRefreshed(_, branch, dirty, statuses, worktrees) => {
            ws_state.branch = branch;
            ws_state.dirty = dirty;
            ws_state.git_statuses = statuses;
            ws_state.worktrees = worktrees;
        }
        Message::AcceptanceCountLoaded(_, n) => {
            ws_state.project_acceptance_count = n;
        }
        Message::RightClickAt { x, y } => {
            app_state.last_right_click = (x, y);
        }
        Message::ContextMenuOpen { path, is_dir } => {
            let (x, y) = app_state.last_right_click;
            ws_state.tree_selected = Some(path.clone());
            app_state.context_menu = Some(ContextMenu {
                x,
                y,
                target: path,
                is_dir,
            });
        }
        Message::ContextMenuClose => {
            app_state.context_menu = None;
        }
        Message::RevealInFinder(path) => {
            app_state.context_menu = None;
            let _ = std::process::Command::new("open").arg("-R").arg(&path).spawn();
        }
        Message::Copy(path, is_dir) => {
            app_state.context_menu = None;
            ws_state.tree_clipboard = Some((path, is_dir));
        }
        Message::Paste(target_dir) => {
            app_state.context_menu = None;
            ws_state.tree_error = None;
            let Some((source, source_is_dir)) = ws_state.tree_clipboard.clone() else {
                return;
            };
            handle.spawn(async move {
                let result = tokio::task::spawn_blocking(move || {
                    crate::project::paste_item(&source, source_is_dir, &target_dir)
                })
                .await
                .unwrap_or_else(|e| Err(e.to_string()));
                emit(Message::PasteDone(project_id, result));
            });
        }
        Message::PasteDone(_, result) => match result {
            Ok(new_path) => {
                if let (Some(tree), Some(parent)) = (&mut ws_state.file_tree, new_path.parent()) {
                    tree.refresh(parent);
                }
            }
            Err(e) => ws_state.tree_error = Some(e),
        },
        Message::DeleteRequest(path, is_dir) => {
            app_state.context_menu = None;
            ws_state.tree_delete_confirm = Some((path, is_dir));
        }
        Message::DeleteCancel => {
            ws_state.tree_delete_confirm = None;
        }
        Message::DeleteConfirm => {
            let Some((path, _)) = ws_state.tree_delete_confirm.take() else {
                return;
            };
            ws_state.tree_error = None;
            handle.spawn(async move {
                let parent = path.parent().map(|p| p.to_path_buf());
                let result = tokio::task::spawn_blocking(move || {
                    trash::delete(&path).map_err(|e| e.to_string())
                })
                .await
                .unwrap_or_else(|e| Err(e.to_string()));
                let outcome = match (result, parent) {
                    (Ok(()), Some(p)) => Ok(p),
                    (Ok(()), None) => Err("删除的是项目根,无父目录可刷新".to_string()),
                    (Err(e), _) => Err(e),
                };
                emit(Message::OpDone {
                    project_id,
                    parent: outcome,
                    expand: false,
                });
            });
        }
        Message::OpDone { parent, expand, .. } => match parent {
            Ok(parent) => {
                ws_state.tree_error = None;
                if let Some(tree) = &mut ws_state.file_tree {
                    tree.refresh(&parent);
                    if expand {
                        tree.ensure_expanded(&parent);
                    }
                }
            }
            Err(e) => ws_state.tree_error = Some(e),
        },
        Message::NewFile(parent) => {
            app_state.context_menu = None;
            ws_state.start_tree_new(parent, TreeEditMode::NewFile);
        }
        Message::NewFolder(parent) => {
            app_state.context_menu = None;
            ws_state.start_tree_new(parent, TreeEditMode::NewFolder);
        }
        Message::ReloadFromDisk => {
            app_state.context_menu = None;
            ws_state.tree_error = None;
            if let Some(tree) = &mut ws_state.file_tree {
                tree.reload_from_disk();
            }
        }
        Message::RenameStart(path) => {
            app_state.context_menu = None;
            ws_state.tree_error = None;
            let Some(parent) = path.parent().map(|p| p.to_path_buf()) else {
                return;
            };
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            ws_state.tree_edit = Some(TreeEdit {
                parent_dir: parent,
                mode: TreeEditMode::Rename(path),
                buffer: name,
            });
        }
        Message::EditEvent(ev) => {
            let Some(edit) = &mut ws_state.tree_edit else {
                return;
            };
            match ev {
                AddrEvent::Text(s) => edit.buffer.push_str(&s),
                AddrEvent::Backspace => {
                    edit.buffer.pop();
                }
                AddrEvent::Cancel => ws_state.tree_edit = None,
                AddrEvent::Submit => ws_state.submit_tree_edit(project_id, handle, emit),
            }
        }
        Message::CopyPath(..) => {
            unreachable!("由内核拦截处理,见 files::Message::CopyPath 文档")
        }
        Message::OpenFile(..) => {
            unreachable!("由内核拦截处理,见 files::Message::OpenFile 文档")
        }
    }
}

/// 内核在项目打开(`from_restore`/`adopt_project`)/`ProjectFsChanged`/手动
/// 刷新等 4 个现有调用点直接调用,异步跑 4 个 git 查询,完成后经 `emit` 送回
/// `GitRefreshed`。现有 `Workspace::spawn_project_git_refresh` 的搬家版本,
/// 逻辑不变。
pub fn spawn_git_refresh(
    project_id: i64,
    repo_path: PathBuf,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    handle.spawn(async move {
        let (b, d, s, w) = tokio::task::spawn_blocking(move || {
            (
                crate::delivery::branch(&repo_path),
                crate::delivery::is_dirty(&repo_path),
                crate::delivery::file_statuses(&repo_path),
                crate::delivery::worktrees(&repo_path),
            )
        })
        .await
        .unwrap_or((None, false, HashMap::new(), Vec::new()));
        emit(Message::GitRefreshed(project_id, b, d, s, w));
    });
}
```

- [ ] **Step 4: 编译确认**

```bash
cargo build -p dozer-app
```

Expected: 编译通过。`update`/`spawn_git_refresh`/`WorkspaceState::new` 等目前仍未被内核
调用,`dead_code` 警告可接受,不允许报错。`trash`/`project`/`delivery` 三个 crate 内
模块已经是 `workspace.rs` 的既有依赖,`Cargo.toml` 不需要改。

- [ ] **Step 5: 新增单测**

在 `files.rs` 文件末尾加 `#[cfg(test)] mod tests`:

测试用 `#[tokio::test]` + `tokio::runtime::Handle::current()`(同 Git Log/浏览器两个
试点已经建立的写法,不用 `Handle::try_current()` 兜底手动建 runtime),`handle.spawn`
真正发起的异步任务(粘贴/删除/重命名/新建的实际文件系统操作)不等待其完成——同浏览器
试点 `update_bookmark_add_global_optimistically_inserts` 的既有测试哲学:只断言
`update()` 内的同步状态转换(乐观更新/校验/提前返回),异步任务的收尾效果通过直接构造
对应的"结果消息"(`PasteDone`/`OpDone`)单独测试,不依赖真正等到 spawn 出的任务跑完。

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn ws_with_tree(root: PathBuf) -> WorkspaceState {
        WorkspaceState::new(FileTree::new(root))
    }

    #[tokio::test]
    async fn toggle_sets_selected_and_toggles_tree() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        let mut app_state = AppState::default();
        let handle = tokio::runtime::Handle::current();
        update(
            &mut ws_state,
            &mut app_state,
            Message::Toggle(sub.clone()),
            1,
            &handle,
            |_| {},
        );
        assert_eq!(ws_state.tree_selected, Some(sub));
    }

    #[tokio::test]
    async fn context_menu_open_uses_last_right_click_and_sets_selected() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("a.txt");
        std::fs::write(&target, "x").unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        let mut app_state = AppState::default();
        let handle = tokio::runtime::Handle::current();
        update(
            &mut ws_state,
            &mut app_state,
            Message::RightClickAt { x: 10.0, y: 20.0 },
            1,
            &handle,
            |_| {},
        );
        update(
            &mut ws_state,
            &mut app_state,
            Message::ContextMenuOpen {
                path: target.clone(),
                is_dir: false,
            },
            1,
            &handle,
            |_| {},
        );
        assert!(app_state.context_menu_is_some());
        assert_eq!(ws_state.tree_selected, Some(target));
    }

    #[tokio::test]
    async fn context_menu_close_clears_menu() {
        let mut ws_state = ws_with_tree(std::env::temp_dir());
        let mut app_state = AppState::default();
        app_state.context_menu = Some(ContextMenu {
            x: 0.0,
            y: 0.0,
            target: PathBuf::from("/x"),
            is_dir: false,
        });
        let handle = tokio::runtime::Handle::current();
        update(
            &mut ws_state,
            &mut app_state,
            Message::ContextMenuClose,
            1,
            &handle,
            |_| {},
        );
        assert!(!app_state.context_menu_is_some());
    }

    #[tokio::test]
    async fn copy_sets_clipboard() {
        let dir = tempfile::tempdir().unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        let mut app_state = AppState::default();
        let handle = tokio::runtime::Handle::current();
        update(
            &mut ws_state,
            &mut app_state,
            Message::Copy(dir.path().join("a.txt"), false),
            1,
            &handle,
            |_| {},
        );
        assert_eq!(
            ws_state.tree_clipboard,
            Some((dir.path().join("a.txt"), false))
        );
    }

    #[tokio::test]
    async fn paste_done_err_sets_tree_error() {
        let dir = tempfile::tempdir().unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        let mut app_state = AppState::default();
        let handle = tokio::runtime::Handle::current();
        update(
            &mut ws_state,
            &mut app_state,
            Message::PasteDone(1, Err("boom".to_string())),
            1,
            &handle,
            |_| {},
        );
        assert_eq!(ws_state.tree_error.as_deref(), Some("boom"));
    }

    #[tokio::test]
    async fn paste_done_ok_clears_error_and_refreshes_parent() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        ws_state.tree_error = Some("stale".to_string());
        let new_file = sub.join("new.txt");
        std::fs::write(&new_file, "x").unwrap();
        let mut app_state = AppState::default();
        let handle = tokio::runtime::Handle::current();
        update(
            &mut ws_state,
            &mut app_state,
            Message::PasteDone(1, Ok(new_file)),
            1,
            &handle,
            |_| {},
        );
        assert!(ws_state.tree_error.is_none());
    }

    #[tokio::test]
    async fn delete_request_then_cancel_clears_confirm() {
        let mut ws_state = ws_with_tree(std::env::temp_dir());
        let mut app_state = AppState::default();
        let handle = tokio::runtime::Handle::current();
        update(
            &mut ws_state,
            &mut app_state,
            Message::DeleteRequest(PathBuf::from("/x"), false),
            1,
            &handle,
            |_| {},
        );
        assert!(ws_state.tree_delete_confirm_is_some());
        update(
            &mut ws_state,
            &mut app_state,
            Message::DeleteCancel,
            1,
            &handle,
            |_| {},
        );
        assert!(!ws_state.tree_delete_confirm_is_some());
    }

    #[tokio::test]
    async fn op_done_err_sets_tree_error_ok_refreshes() {
        let dir = tempfile::tempdir().unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        let mut app_state = AppState::default();
        let handle = tokio::runtime::Handle::current();
        update(
            &mut ws_state,
            &mut app_state,
            Message::OpDone {
                project_id: 1,
                parent: Err("nope".to_string()),
                expand: false,
            },
            1,
            &handle,
            |_| {},
        );
        assert_eq!(ws_state.tree_error.as_deref(), Some("nope"));
        update(
            &mut ws_state,
            &mut app_state,
            Message::OpDone {
                project_id: 1,
                parent: Ok(dir.path().to_path_buf()),
                expand: false,
            },
            1,
            &handle,
            |_| {},
        );
        assert!(ws_state.tree_error.is_none());
    }

    #[tokio::test]
    async fn new_file_starts_tree_edit_and_closes_menu() {
        let dir = tempfile::tempdir().unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        let mut app_state = AppState::default();
        app_state.context_menu = Some(ContextMenu {
            x: 0.0,
            y: 0.0,
            target: dir.path().to_path_buf(),
            is_dir: true,
        });
        let handle = tokio::runtime::Handle::current();
        update(
            &mut ws_state,
            &mut app_state,
            Message::NewFile(dir.path().to_path_buf()),
            1,
            &handle,
            |_| {},
        );
        assert!(!app_state.context_menu_is_some());
        assert!(ws_state.tree_edit.is_some());
    }

    #[tokio::test]
    async fn rename_start_prefills_buffer_with_current_name() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("old.txt");
        std::fs::write(&target, "x").unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        let mut app_state = AppState::default();
        let handle = tokio::runtime::Handle::current();
        update(
            &mut ws_state,
            &mut app_state,
            Message::RenameStart(target.clone()),
            1,
            &handle,
            |_| {},
        );
        let edit = ws_state.tree_edit.as_ref().unwrap();
        assert_eq!(edit.buffer, "old.txt");
        assert!(matches!(edit.mode, TreeEditMode::Rename(ref p) if *p == target));
    }

    #[tokio::test]
    async fn edit_event_text_and_backspace_mutate_buffer() {
        let dir = tempfile::tempdir().unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        let mut app_state = AppState::default();
        ws_state.tree_edit = Some(TreeEdit {
            parent_dir: dir.path().to_path_buf(),
            mode: TreeEditMode::NewFile,
            buffer: String::new(),
        });
        let handle = tokio::runtime::Handle::current();
        update(
            &mut ws_state,
            &mut app_state,
            Message::EditEvent(AddrEvent::Text("ab".to_string())),
            1,
            &handle,
            |_| {},
        );
        assert_eq!(ws_state.tree_edit.as_ref().unwrap().buffer, "ab");
        update(
            &mut ws_state,
            &mut app_state,
            Message::EditEvent(AddrEvent::Backspace),
            1,
            &handle,
            |_| {},
        );
        assert_eq!(ws_state.tree_edit.as_ref().unwrap().buffer, "a");
        update(
            &mut ws_state,
            &mut app_state,
            Message::EditEvent(AddrEvent::Cancel),
            1,
            &handle,
            |_| {},
        );
        assert!(ws_state.tree_edit.is_none());
    }

    #[tokio::test]
    async fn submit_edit_empty_name_silently_cancels_without_clearing_edit() {
        let dir = tempfile::tempdir().unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        let mut app_state = AppState::default();
        ws_state.tree_edit = Some(TreeEdit {
            parent_dir: dir.path().to_path_buf(),
            mode: TreeEditMode::NewFile,
            buffer: "   ".to_string(),
        });
        let handle = tokio::runtime::Handle::current();
        update(
            &mut ws_state,
            &mut app_state,
            Message::EditEvent(AddrEvent::Submit),
            1,
            &handle,
            |_| panic!("空名字不该发起任何异步操作"),
        );
        // `submit_tree_edit` 对空名字提前 `return`,不取走 `tree_edit`——
        // 现有行为原样保留,行内编辑框仍显示,用户可以继续输入。
        assert!(ws_state.tree_edit.is_some());
    }

    #[tokio::test]
    async fn submit_edit_rejects_path_separator_in_name() {
        let dir = tempfile::tempdir().unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        let mut app_state = AppState::default();
        ws_state.tree_edit = Some(TreeEdit {
            parent_dir: dir.path().to_path_buf(),
            mode: TreeEditMode::NewFile,
            buffer: "a/b".to_string(),
        });
        let handle = tokio::runtime::Handle::current();
        update(
            &mut ws_state,
            &mut app_state,
            Message::EditEvent(AddrEvent::Submit),
            1,
            &handle,
            |_| panic!("名字非法时不该发起任何异步操作"),
        );
        assert_eq!(
            ws_state.tree_error.as_deref(),
            Some("名字不能包含路径分隔符")
        );
        assert!(ws_state.tree_edit.is_some(), "编辑框保留,让用户改名重试");
    }

    #[tokio::test]
    async fn submit_edit_rejects_existing_same_name() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("dup.txt"), "x").unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        let mut app_state = AppState::default();
        ws_state.tree_edit = Some(TreeEdit {
            parent_dir: dir.path().to_path_buf(),
            mode: TreeEditMode::NewFile,
            buffer: "dup.txt".to_string(),
        });
        let handle = tokio::runtime::Handle::current();
        update(
            &mut ws_state,
            &mut app_state,
            Message::EditEvent(AddrEvent::Submit),
            1,
            &handle,
            |_| panic!("已存在同名项时不该发起任何异步操作"),
        );
        assert!(ws_state.tree_error.as_deref().unwrap().contains("已存在同名项"));
    }

    #[tokio::test]
    async fn git_refreshed_updates_four_fields() {
        let mut ws_state = ws_with_tree(std::env::temp_dir());
        let mut app_state = AppState::default();
        let mut statuses = HashMap::new();
        statuses.insert(
            PathBuf::from("/x"),
            crate::delivery::FileGitStatus {
                kind: crate::delivery::ChangeKind::Modified,
                staged: false,
                unstaged: true,
            },
        );
        let handle = tokio::runtime::Handle::current();
        update(
            &mut ws_state,
            &mut app_state,
            Message::GitRefreshed(1, Some("main".to_string()), true, statuses, Vec::new()),
            1,
            &handle,
            |_| {},
        );
        assert_eq!(ws_state.branch.as_deref(), Some("main"));
        assert!(ws_state.dirty);
        assert_eq!(ws_state.worktrees().len(), 0);
    }

    #[tokio::test]
    #[should_panic(expected = "由内核拦截处理")]
    async fn copy_path_reaching_update_panics() {
        let mut ws_state = ws_with_tree(std::env::temp_dir());
        let mut app_state = AppState::default();
        let handle = tokio::runtime::Handle::current();
        update(
            &mut ws_state,
            &mut app_state,
            Message::CopyPath(PathBuf::from("/x"), PathKind::Absolute),
            1,
            &handle,
            |_| {},
        );
    }
}
```

`tempfile` 已经是 `crates/dozer-app/Cargo.toml` 的既有依赖(`tempfile = "3"`),不需要
额外添加。`FileGitStatus { kind: ChangeKind, staged: bool, unstaged: bool }` /
`ChangeKind { New, Modified, Deleted }` 已确认为 `crates/dozer-app/src/delivery.rs`
现有定义,上面测试代码里的字段名/变体名直接可编译。

- [ ] **Step 6: 跑测试**

```bash
cargo test -p dozer-app files::
```

Expected: 全部新增测试 PASS。

- [ ] **Step 7: Commit**

```bash
git add crates/dozer-app/src/extensions/files.rs crates/dozer-app/Cargo.toml
git commit -m "feat(dozer-app): add files WorkspaceState/AppState/Message/update"
```

---

### Task 3: `extensions::files::view`/`context_menu_popup`/`delete_confirm_popup`

**Files:**
- Modify: `crates/dozer-app/src/extensions/files.rs`

**Interfaces:**
- Consumes:Task 1/2 的 `WorkspaceState`/`AppState`/`Message`/`TreeEdit`/`TreeEditMode`/
  `ContextMenu`。
- Produces:`pub fn view<'a>(ws_state: &'a WorkspaceState, project: Option<&'a
  crate::project::ProjectInfo>, daemon_ok: bool, width: Length, outer: Border) ->
  Element<'a, Message, ..>`、`pub fn context_menu_popup<'a>(app_state: &'a AppState,
  ws_state: &'a WorkspaceState) -> Element<'a, Message, ..>`、`pub fn
  delete_confirm_popup(ws_state: &WorkspaceState) -> Element<'_, Message, ..>`。

- [ ] **Step 1: 文件顶部补齐 `view` 需要的 import**

在 `files.rs` 顶部 `use` 块追加(与现有 `workspace.rs` 里 `project_pane` 用到的这批
完全对应):

```rust
use crate::{delivery, icons, theme};
use iced_widget::core::{Border, Element, Length};
use iced_widget::{button, column, container, row, scrollable, text, Scrollable};
```

（`iced_widget::core::alignment`/`Padding`/`Alignment` 等更细的类型按实际编译报错逐条
补,这里只列最容易漏、非补不可的几个。）

- [ ] **Step 2: 私有辅助 —— `tree_row_font_size`/`project_branch_label`/`env_status_text`/`status_bar_container` 的可见性处理**

这四个函数现在定义在 `workspace.rs` 里且是私有 `fn`(`env_status_text` 在
`terminal_pane` 也用到,其余三个目前只有 Files 用)。改
`crates/dozer-app/src/workspace.rs` 里这四个函数的签名前缀为 `pub(crate) fn`(不搬
文件——`tree_row_font_size`/`project_branch_label`/`status_bar_container` 是纯排版
辅助,和 `theme::geometry`/`theme::font` 里其它一堆同类辅助函数放在一起更合适,搬进
`files.rs` 反而制造一个新的"两处都有一份"的维护面)。`files.rs` 里用
`crate::workspace::{tree_row_font_size, project_branch_label, env_status_text,
status_bar_container}` 引用。

运行确认还没改坏:

```bash
cargo build -p dozer-app
```

Expected: 编译通过(只是可见性放宽,函数体不变)。

- [ ] **Step 3: `view` 主体(整体照搬现有 `project_pane`,签名与类型改成本模块的)**

把 `workspace.rs` 里 `project_pane`"已打开项目"那一半(现 6922-6934 行的函数开头
+ 6934-7121 行 `match &ws.project { Some(p) => { .. } ..}` 里 `Some(p)` 分支的
全部内容 + 7146-7178 行的 `body`/`container` 收尾,**不含** `None` 分支,`None`
分支处理方式见下方)整段复制进 `files.rs`,函数签名改成:

```rust
pub fn view<'a>(
    ws_state: &'a WorkspaceState,
    project: Option<&'a crate::project::ProjectInfo>,
    daemon_ok: bool,
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
```

函数体内做以下逐处替换:
- `match &ws.project { Some(p) => { .. } None => { .. } }` 改成
  `let Some(p) = project else { return column![].into(); };` 起手(`None` 分支的
  内容不带进来,原因见下方),后面 `Some(p)` 分支缩进的内容整体降一级缩进接上。
- `ws.branch`/`ws.dirty`/`ws.git_statuses`/`ws.tree_error`/`ws.file_tree`/
  `ws.tree_edit`/`ws.project_acceptance_count` 全部改成 `ws_state.` 加同名字段。
- `Message::ProjectTreeToggle(row.path.clone())` 改成
  `Message::Toggle(row.path.clone())`(目录行的点击消息)。
- `Message::PreviewOpenPath(row.path.clone())` 改成
  `Message::OpenFile(row.path.clone())`(非目录行的点击消息——原代码按
  `row.is_dir` 在这两条消息间二选一构造 `msg`,这是设计文档遗漏、代码审查时
  才补上的一条,`OpenFile` 定义见 Task 1 Step 1)。
- 函数结尾 `container(column![body, project_status_bar(app, ws, outer)])` 改成
  `container(column![body, project_status_bar(daemon_ok, ws_state, outer)])`
  (对应 Step 4 里 `project_status_bar` 的新签名)。
- 函数返回类型/尾部 `.into()` 保持不变,只是外层 `Element` 的 `Message` 泛型参数
  自动跟随文件顶部新 `use` 的本模块 `Message`。

**`None` 分支的特殊处理**:现有 `project_pane` 在 `ws.project` 为 `None` 时渲染
"最近项目"列表,每个按钮 `on_press(Message::ProjectSelect(p.id))`——`ProjectSelect`
是顶层项目切换消息,不属于 Files 域。`files::view` 返回类型是
`Element<'a, Message, ..>`(`files::Message`),没法直接塞一个顶层 `Message` 的
`on_press`。处理方式:`recent_projects` 列表和"未打开项目"整个 `None` 分支不属于
Files 面板的职责(它是"还没有项目可看"时的兜底 UI,本质上是项目切换器,不是文件树)——
**这一段留在内核**,`files::view` 只负责"已打开项目"的渲染;`project: Option<&ProjectInfo>`
为 `None` 时 `files::view` 直接返回一个空 `column![].into()`,内核在 `LeftView::Files`
分支里判断 `ws.project.is_none()` 时改渲染现有的"最近项目"逻辑(这段小逻辑挪进
`workspace.rs` 一个新的私有函数,比如 `fn no_project_placeholder(ws: &Workspace) ->
Element<Message>`,内容就是现有 `project_pane` `None` 分支那几行,原样保留)。这是
写计划时发现的一个 Task 2 设计遗漏的补丁:设计文档没有单独提这一点,因为
`project: Option<&ProjectInfo>` 这个参数形状已经暗示了"没有项目时 files 模块给不出
有意义的树",这里补上具体处理方式,不算违背设计文档的既定架构。

- [ ] **Step 4: `project_status_bar` 私有辅助迁入(签名改吃 `daemon_ok: bool`)**

把 `workspace.rs` 里 `project_status_bar`(现 7181-7226 行左右,渲染
"●环境状态 · 文件·git{分支}·组件" 底栏)整段复制进 `files.rs`,签名从
`fn project_status_bar<'a>(app: &'a App, ws: &'a Workspace, outer: Border)` 改成:

```rust
fn project_status_bar<'a>(
    daemon_ok: bool,
    ws_state: &'a WorkspaceState,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    // 原函数体里 `env_status_text(app.daemon_error.is_none())` 改成
    // `crate::workspace::env_status_text(daemon_ok)`;
    // `ws.branch`/`ws.dirty` 改成 `ws_state.branch`/`ws_state.dirty`。
}
```

- [ ] **Step 5: `tree_edit_row` 私有辅助迁入**

把 `workspace.rs` 里 `tree_edit_row`(现 7615-7638 行)整段原样复制进 `files.rs`,
只改返回类型的 `Message` 泛型参数指向本模块的 `Message`(函数体不引用任何
`Message` 变体,原样照抄即可编译)。

- [ ] **Step 6: `context_menu_popup` + `menu_item` 迁入**

把 `workspace.rs` 里 `menu_item`(现 7584-7611 行)与 `context_menu_popup`(现
7644-7755 行)整段复制进 `files.rs`,做以下替换:

```rust
pub fn context_menu_popup<'a>(
    app_state: &'a AppState,
    ws_state: &'a WorkspaceState,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    let Some(menu) = &app_state.context_menu else {
        return column![].into();
    };
    // ...原函数体,做以下替换:
    // - `ws.tree_clipboard` → `ws_state.tree_clipboard`
    // - `Message::ProjectTreeNewFile(..)` → `Message::NewFile(..)`
    // - `Message::ProjectTreeNewFolder(..)` → `Message::NewFolder(..)`
    // - `Message::ProjectTreeCopy(..)` → `Message::Copy(..)`
    // - `Message::ProjectTreePaste(..)` → `Message::Paste(..)`
    // - `Message::ProjectTreeDeleteRequest(..)` → `Message::DeleteRequest(..)`
    // - `Message::ProjectTreeRenameStart(..)` → `Message::RenameStart(..)`
    // - `Message::ProjectTreeCopyPath(..)` → `Message::CopyPath(..)`
    // - `Message::ProjectTreeRevealInFinder(..)` → `Message::RevealInFinder(..)`
    // - `Message::ProjectTreeReloadFromDisk` → `Message::ReloadFromDisk`
    // - `project::PathKind::Absolute/Relative` → `crate::project::PathKind::Absolute/Relative`
    //   (已经是完整路径,原样保留)
}
```

`menu_item` 函数体不引用具体 `Message` 变体(签名里的 `msg: Message` 参数类型
自动跟随本模块 `Message`),原样照抄即可编译。

- [ ] **Step 7: `delete_confirm_popup` 迁入**

把 `workspace.rs` 里 `delete_confirm_popup`(现 7758-7830 行左右)整段复制进
`files.rs`,签名从 `fn delete_confirm_popup(ws: &Workspace)` 改成
`pub fn delete_confirm_popup(ws_state: &WorkspaceState)`,函数体 `ws.tree_delete_confirm`
改成 `ws_state.tree_delete_confirm`,两处 `Message::ProjectTreeDeleteCancel`/
`Message::ProjectTreeDeleteConfirm` 改成 `Message::DeleteCancel`/
`Message::DeleteConfirm`。

- [ ] **Step 8: 编译,逐条修正**

```bash
cargo build -p dozer-app
```

逐条修正报错(多半是漏改的 `Message::ProjectTreeXxx` 前缀、`ws.` 该改
`ws_state.` 的字段访问、缺的 `use`)。`workspace.rs` 里原 `project_pane`/
`project_status_bar`/`tree_edit_row`/`menu_item`/`context_menu_popup`/
`delete_confirm_popup` 六个函数此时**还没删除**(留到 Task 4 内核接线时一并删除,
避免 `dead_code` 编译错误——如果现在删,`workspace.rs` 里 `LeftView::Files` 分支等
调用点会立刻报错未定义,不如等 Task 4 一次性替换调用点+删除旧定义)。

- [ ] **Step 9: `cargo clippy`/`fmt`**

```bash
cargo clippy -p dozer-app --all-targets
cargo fmt
```

- [ ] **Step 10: Commit**

```bash
git add crates/dozer-app/src/extensions/files.rs crates/dozer-app/src/workspace.rs
git commit -m "feat(dozer-app): add files::view/context_menu_popup/delete_confirm_popup"
```

---

### Task 4: 内核接线

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`
- Modify: `crates/dozer-app/src/main.rs`

**Interfaces:**
- Consumes:Task 1-3 的 `files::WorkspaceState`/`files::AppState`/`files::Message`/
  `files::update`/`files::spawn_git_refresh`/`files::view`/`files::context_menu_popup`/
  `files::delete_confirm_popup`。

- [ ] **Step 1: `Workspace` 结构体字段合并**

`crates/dozer-app/src/workspace.rs` 里 `pub struct Workspace { .. }`(现约 1397-1479
行),删除 11 个字段:`file_tree`/`branch`/`dirty`/`git_statuses`/`worktrees`/
`project_acceptance_count`/`tree_selected`/`tree_clipboard`/`tree_error`/
`tree_delete_confirm`/`tree_edit`,加:

```rust
/// Files 面板 per-project 状态——见 `extensions::files::WorkspaceState`。
files: files::WorkspaceState,
```

`Workspace::new()`(现约 1735-1770 行)对应 11 行初始化删除,加一行
`files: files::WorkspaceState::default(),`。

`Workspace::from_restore()`(现约 1658-1667 行)里:

```rust
let file_tree = Some(FileTree::new(PathBuf::from(&project.path)));
```

改成:

```rust
let files = files::WorkspaceState::new(FileTree::new(PathBuf::from(&project.path)));
```

`Self { .. }` 构造块里 `file_tree,` 一行改成 `files,`。

`Workspace::adopt_project()`(现约 2105-2132 行)里这 5 行:

```rust
self.file_tree = Some(FileTree::new(PathBuf::from(&project.path)));
self.tree_selected = None;
self.branch = None;
self.dirty = false;
self.git_statuses = HashMap::new();
```

改成一行:

```rust
self.files.reset_for_project(FileTree::new(PathBuf::from(&project.path)));
```

（`self.project_acceptance_count = None;` 这一行原本在 5 行之后单独出现,一并纳入
`reset_for_project` 内部,原地删除。）

- [ ] **Step 2: `App` 结构体字段合并**

`pub struct App { .. }` 里删除 `context_menu`/`last_right_click` 两个字段,加:

```rust
/// Files 面板右键菜单浮层状态——见 `extensions::files::AppState`。
files: files::AppState,
```

`App` 的构造处(现约 2787-2788 行 `context_menu: None, last_right_click: (0.0,
0.0),`)改成 `files: files::AppState::default(),`。

- [ ] **Step 3: 顶层 `Message` 枚举**

删除 20 个变体:`ProjectTreeToggle`/`ProjectGitRefreshed`/`AcceptanceCountLoaded`/
`RightClickAt`/`ProjectTreeContextMenu`/`ProjectTreeContextMenuClose`/
`ProjectTreeCopyPath`/`ProjectTreeRevealInFinder`/`ProjectTreeCopy`/
`ProjectTreePaste`/`ProjectTreePasteDone`/`ProjectTreeDeleteRequest`/
`ProjectTreeDeleteConfirm`/`ProjectTreeDeleteCancel`/`ProjectTreeOpDone`/
`ProjectTreeNewFile`/`ProjectTreeNewFolder`/`ProjectTreeRenameStart`/
`ProjectTreeReloadFromDisk`/`ProjectTreeEditEvent`,加:

```rust
/// Files 面板的全部消息,内核只转发不解读——见 `extensions::files::Message`。
Files(files::Message),
```

`ProjectFsChanged(ProjectId, git_watch::Relevance)` 保留不动。

- [ ] **Step 4: `update()` 里 Files 相关分支**

删除原 `Message::ProjectTreeToggle`/`ProjectGitRefreshed`/`ProjectTreeContextMenu`/
`ProjectTreeContextMenuClose`/`ProjectTreeCopyPath`/`ProjectTreeRevealInFinder`/
`ProjectTreeCopy`/`ProjectTreePaste`/`ProjectTreePasteDone`/
`ProjectTreeDeleteRequest`/`ProjectTreeDeleteCancel`/`ProjectTreeDeleteConfirm`/
`ProjectTreeOpDone`/`ProjectTreeNewFile`/`ProjectTreeNewFolder`/
`ProjectTreeReloadFromDisk`/`ProjectTreeRenameStart`/`ProjectTreeEditEvent`/
`AcceptanceCountLoaded`/`RightClickAt` 这 20 个分支(散落在现约 4240-4610 行区间),
加 4 支(比设计文档多一支 `OpenFile`——见 Task 1 Step 1 关于这条消息的说明,是
设计遗漏、代码审查时才补上的):

```rust
Message::Files(files::Message::CopyPath(path, kind)) => {
    let _ = (path, kind); // main.rs 拦截处理写剪贴板,这里维持现状空分支
}
Message::Files(files::Message::OpenFile(path)) => {
    // 单击文件行打开预览——`files` 模块不认识预览域,这条消息由内核拦截
    // 转发成核心的 `PreviewOpenPath`(同 `ProjectTreeCopyPath` 现状,不能
    // 落进下面的兜底分支,否则会命中 `files::update` 里的 `unreachable!`)。
    self.update(Message::PreviewOpenPath(path));
}
Message::Files(
    msg @ (files::Message::GitRefreshed(project_id, ..)
    | files::Message::PasteDone(project_id, ..)
    | files::Message::OpDone { project_id, .. }
    | files::Message::AcceptanceCountLoaded(project_id, ..)),
) => {
    let handle = self.handle.clone();
    let proxy = self.proxy.clone();
    let emit = move |m| {
        let _ = proxy.send_event(Message::Files(m));
    };
    let app_files = &mut self.files;
    let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
        return;
    };
    files::update(&mut ws.files, app_files, msg, project_id, &handle, emit);
}
Message::Files(msg) => {
    let Some(project_id) = self.active_project_id else {
        return;
    };
    let handle = self.handle.clone();
    let proxy = self.proxy.clone();
    let emit = move |m| {
        let _ = proxy.send_event(Message::Files(m));
    };
    let app_files = &mut self.files;
    let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
        return;
    };
    files::update(&mut ws.files, app_files, msg, project_id, &handle, emit);
}
```

`AcceptanceCountLoaded` 现有触发点(`spawn_acceptance_count_refresh`,构造
`Message::AcceptanceCountLoaded(project_id, n)` 的地方)改成构造
`Message::Files(files::Message::AcceptanceCountLoaded(project_id, n))`。

- [ ] **Step 5: 4 处 `spawn_project_git_refresh` 调用点改用新入口**

`workspace.rs` 里删除 `Workspace::spawn_project_git_refresh` 方法(现 2056-2076
行),4 个调用点(`from_restore`/`adopt_project`/一处手动刷新/`ProjectFsChanged`
分支)统一改成:

```rust
files::spawn_git_refresh(project_id, repo_path.clone(), &io.handle, {
    let proxy = io.proxy.clone();
    move |m| {
        let _ = proxy.send_event(Message::Files(m));
    }
});
```

（`project_id`/`repo_path` 从各调用点现有的 `self.project_id()`/`self.project`
取,变量名按各处上下文实际命名调整,逻辑等价于原方法体内联到调用处。）

- [ ] **Step 6: `ProjectFsChanged` 分支改用新访问器**

现有分支里 `ws.spawn_project_git_refresh(io)` 一行按 Step 5 的新写法替换,其余
Git Log 快照重建的条件判断逻辑不动。

- [ ] **Step 7: `worktree_strip` 调用点**

现约 6668 行 `Some(worktree_strip(&ws.worktrees))` 改成
`Some(worktree_strip(ws.files.worktrees()))`。

- [ ] **Step 8: `App::view()` 的 `LeftView::Files` 分支 + 顶层浮层判断链**

`LeftView::Files` 分支(现约 6601-6627 行)里 `project_pane(app, ws, ..)` 调用改成:

```rust
LeftView::Files => {
    let (list_portion, content_portion) = split_portions(app.shell_layout.files_split);
    row![
        if ws.project.is_some() {
            files::view(
                &ws.files,
                ws.project.as_ref(),
                app.daemon_error.is_none(),
                Length::FillPortion(list_portion),
                zone_pane_border(zone, lc),
            )
            .map(Message::Files)
        } else {
            no_project_placeholder(app, ws, Length::FillPortion(list_portion))
        },
        divider_bar(
            Divider::LeftPairSplit,
            theme::region::project_pane().background.unwrap_or(theme::color::BG),
            theme::region::preview_pane().background.unwrap_or(theme::color::BG),
        ),
        preview_pane(ws, Length::FillPortion(content_portion), zone_pane_border(zone, rc)),
    ]
    .width(Length::Fill)
    .into()
}
```

新增私有函数 `no_project_placeholder`(把原 `project_pane` `None` 分支——"未打开项目"
文案 + `ws.recent_projects` 列表按钮,`on_press(Message::ProjectSelect(p.id))`——
原样搬进 `workspace.rs`,签名 `fn no_project_placeholder<'a>(app: &'a App, ws: &'a
Workspace, width: Length) -> Element<'a, Message, ..>`,渲染上仍需要外层的
`header`/`body`/`project_status_bar` 包装结构,照抄原 `project_pane` 对应部分,
`project_status_bar` 调用改成 `files::project_status_bar` 不可行(它是私有
`fn`)——**改为**:`no_project_placeholder` 只渲染"未打开项目"提示 + 最近项目列表,
不含底部状态条(现有 UI 在没有项目时本来就没有 git 分支/文件可显示的状态条内容,
底部状态条原样跳过,不是新的行为改变——写这一步时对照现有渲染截图/`cargo run`
确认这一点站得住,如果发现底栏其实也要出现,把 `project_status_bar` 的可见性也在
Task 3 Step 4 一并改成 `pub(crate)` 供这里调用)。

顶层互斥浮层判断链(现约 4595-4629 行)两处调用/条件替换:

```rust
} else if ws.files.tree_delete_confirm_is_some() {
    let dismiss = MouseArea::new(container(column![]).width(Length::Fill).height(Length::Fill))
        .on_press(Message::Files(files::Message::DeleteCancel));
    stack![base, dismiss, files::delete_confirm_popup(&ws.files).map(Message::Files)]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
} else if self.files.context_menu_is_some() {
    let dismiss = MouseArea::new(container(column![]).width(Length::Fill).height(Length::Fill))
        .on_press(Message::Files(files::Message::ContextMenuClose));
    stack![base, dismiss, files::context_menu_popup(&self.files, &ws.files).map(Message::Files)]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}
```

- [ ] **Step 9: 右键坐标捕获与右键菜单打开的触发点**

`main.rs` 里原先发 `Message::RightClickAt { x, y }` 的地方(现约 492 行)改发
`Message::Files(files::Message::RightClickAt { x, y })`。`workspace.rs` 里原
`ProjectTreeContextMenu { path, is_dir }` 的构造点(项目树行的右键手势,现约
7098 行 `Message::ProjectTreeContextMenu { path: row.path.clone(), is_dir:
row.is_dir }` 一带,已经在 Task 3 迁移 `view` 时随 `project_pane` 一起搬进
`files.rs`,构造类型自动是 `files::Message::ContextMenuOpen`)不需要单独处理,
Step 3(Task 3)里已经在"替换规则"清单中覆盖(`ProjectTreeContextMenu` →
`ContextMenuOpen`,若 Task 3 Step 3 的替换清单里漏列这一条,在本步骤补上)。

- [ ] **Step 10: 删除旧函数定义**

删除 `workspace.rs` 里原 `project_pane`/`project_status_bar`/`tree_edit_row`/
`menu_item`/`context_menu_popup`/`delete_confirm_popup`/
`Workspace::start_tree_new`/`Workspace::submit_tree_edit` 八个函数/方法的旧定义
(Task 3 已经把它们的内容复制进 `files.rs`,这里删掉 `workspace.rs` 里的原件,
避免死代码/重复定义)。`TreeEditMode`/`TreeEdit`/`ContextMenu` 三个类型定义(现
416-441 行)一并删除(Task 1 已经在 `files.rs` 里重新定义)。

- [ ] **Step 11: `main.rs` 里 `ProjectTreeCopyPath` 改路径**

现约 869 行:

```rust
Message::ProjectTreeCopyPath(path, kind) => {
    let root = app.active_project_path().unwrap_or_else(|| path.clone());
    let s = crate::project::path_string(kind, &path, &root);
    clipboard.write(iced_winit::core::clipboard::Kind::Standard, s);
    app.update(Message::ProjectTreeContextMenuClose);
    window.request_redraw();
}
```

改成:

```rust
Message::Files(files::Message::CopyPath(path, kind)) => {
    let root = app.active_project_path().unwrap_or_else(|| path.clone());
    let s = crate::project::path_string(kind, &path, &root);
    clipboard.write(iced_winit::core::clipboard::Kind::Standard, s);
    app.update(Message::Files(files::Message::ContextMenuClose));
    window.request_redraw();
}
```

- [ ] **Step 12: 编译,逐条修正**

```bash
cargo build -p dozer-app
```

Expected: 先会有一长串报错(引用了已删除的 `ws.file_tree`/`ws.branch`/…/旧
`Message::ProjectTreeXxx` 变体的其它地方,比如某处状态栏/顶栏也读了
`ws.dirty`/`ws.branch` 判断脏点颜色)。逐条改成 `ws.files.xxx()`(缺访问器就在
`files.rs` 补一个只读 getter,遵循 Task 2 已经建立的"字段私有、按需开访问器"
风格)或 `Message::Files(files::Message::Xxx)`。

- [ ] **Step 13: 全量测试 + clippy + fmt**

```bash
cargo build -p dozer-app
cargo test -p dozer-app
cargo clippy -p dozer-app --all-targets
cargo fmt
```

Expected: 全绿。

- [ ] **Step 14: Commit**

```bash
git add crates/dozer-app/src/workspace.rs crates/dozer-app/src/main.rs
git commit -m "refactor(dozer-app): route Files messages through extensions::files"
```

---

### Task 5: 全量校验与人工验收

**Files:** 无代码改动。

- [ ] **Step 1: 全 workspace 构建 + 测试 + clippy + fmt**

```bash
cargo build
cargo test -p dozer-app
cargo clippy --all-targets
cargo fmt --check
```

Expected: 全绿。

- [ ] **Step 2: 人工验收(`cargo run -p dozer-app`)**

对照 Files 面板现有行为逐项走一遍,确认拆分没有改变任何可见行为:
- 展开/收起目录,切换到别的 `LeftView` 再切回来,展开态保持记忆。
- 单击一个文件行(非目录),预览面板正确打开该文件(验证 `OpenFile` 的内核拦截
  确实生效,没有落进 `files::update` 触发 `unreachable!` panic)。
- 右键文件/文件夹,菜单选项齐全(文件:复制/删除/重命名/复制绝对路径/复制相对
  路径/在 Finder 中打开/从磁盘重新加载;文件夹额外有新建文件/新建文件夹/粘贴)。
- 复制一个文件,粘贴到另一个目录,新文件出现在目标目录且父目录自动展开。
- 删除一个文件,确认框正确显示文件名,确认后文件从树中消失,可在系统废纸篓/
  Finder 找回。
- 新建文件/新建文件夹,行内编辑框正确出现在目标目录下,回车提交后新项可见。
- 重命名一个文件,行内编辑框预填原名,改名后树反映新名字,取消(Esc)不改名。
- 项目信息卡:项目名/git 分支图标+名字/脏标颜色(有未提交改动时变金)/验收次数
  副行(有验收记录时才显示)。
- 底部状态条:daemon 环境点、"文件·git {分支}·组件" 文案正确。
- git 状态装饰:新增/修改/删除文件在树里有对应颜色点。
- 切到 `LeftView::GitLog`,worktree 速览条仍正常显示在提交图上方。
- 切换项目页签(多项目并行),Files 面板状态(文件树展开态/git 状态)各自独立,
  不串项目。
- 未打开任何项目时,左侧显示"未打开项目"提示 + 最近项目列表,点击能打开对应项目。

- [ ] **Step 3: 确认没有遗留未提交的改动**

```bash
git status
```

Expected: 干净,无未跟踪/未暂存改动(`.dozer/` 目录等项目运行时产物除外)。
