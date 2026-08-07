//! Files(项目文件树)面板:项目信息卡 + git 分支/脏标/worktree 速览数据 +
//! 文件树 + 右键菜单/删除确认浮层。阶段 1 扩展化重构第四个试点,设计见
//! `docs/superpowers/specs/2026-08-07-files-extension-pilot-design.md`。
use crate::delivery::{FileGitStatus, WorktreeInfo};
use crate::project::{FileTree, PathKind};
use crate::theme::terminal_font;
use crate::workspace::AddrEvent;
use crate::{delivery, icons, theme};
use dozer_core::protocol::ProjectInfo;
use iced_widget::core::text::LineHeight;
use iced_widget::core::{Border, Element, Length, Padding};
use iced_widget::{MouseArea, Scrollable, button, column, container, row, scrollable, text};
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
/// `AcceptanceCountLoaded`/`RightClickAt` 变体,去前缀原样搬来。
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
    RightClickAt {
        x: f32,
        y: f32,
    },
    ContextMenuOpen {
        path: PathBuf,
        is_dir: bool,
    },
    ContextMenuClose,
    /// 单击文件行:在内核里打开预览。该面板本身不渲染预览(预览是被窗格),
    /// 故文件行点击要跨过 `files::Message` 边界、由内核拦截映射到
    /// `Message::PreviewOpenPath`——本模块不渲染预览,`update()` 不处理它。
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

impl AppState {
    /// 供内核判断"右键菜单该不该显示"(`App::view()` 顶层互斥浮层判断链
    /// 用)。
    pub fn context_menu_is_some(&self) -> bool {
        self.context_menu.is_some()
    }
}

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
            let _ = std::process::Command::new("open")
                .arg("-R")
                .arg(&path)
                .spawn();
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
        Message::OpenFile(_) => {
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

/// 项目信息卡 + 文件树可滚动列表(现有 `workspace.rs::project_pane` 的搬家
/// 版本,签名改吃本模块状态)。`project` 用 `Option<&ProjectInfo>`——项目身份
/// 是内核概念,本模块只认"文件树数据"。
pub fn view<'a>(
    ws_state: &'a WorkspaceState,
    project: Option<&'a ProjectInfo>,
    daemon_ok: bool,
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    let region = theme::region::project_pane();
    // `project` 为 `None` 时本模块给不出有意义的文件树——"未打开项目"的兜底
    // UI(最近项目列表)是项目切换器,属内核职责,由内核在 `LeftView::Files`
    // 分支自行渲染,这里返回空列。
    let Some(p) = project else {
        return column![].into();
    };
    // 头部:项目信息卡,固定在文件树上方,不随滚动条滚走(需求 1)。
    let mut header = column![].spacing(region.gap).width(Length::Fill);
    // 文件树行:唯一进入 scrollable 的内容。
    let mut tree_col = column![].spacing(region.gap);

    let label = crate::workspace::project_branch_label(ws_state.branch.as_deref(), ws_state.dirty);
    let bcolor = if ws_state.dirty {
        theme::color::GOLD
    } else {
        theme::color::BODY
    };
    // 需求 3:git 分支名前加 git-branch icon;需求 2:去掉完整文件路径。
    let mut card_col = column![
        text(p.name.clone())
            .size(theme::font::title())
            .color(theme::color::CREAM),
        row![
            icons::view(
                icons::IconKind::GitBranch,
                crate::theme::icon_size::row(),
                bcolor
            ),
            text(label).size(theme::font::label()).color(bcolor),
        ]
        .spacing(6)
        .align_y(iced_widget::core::Alignment::Center),
    ]
    .spacing(2);
    if let Some(n) = ws_state.project_acceptance_count.filter(|n| *n > 0) {
        card_col = card_col.push(
            text(format!("{n} 次验收"))
                .size(theme::font::caption())
                .color(theme::color::GOLD),
        );
    }
    let card =
        container(card_col)
            .width(Length::Fill)
            .padding(10)
            .style(|_t: &iced_widget::Theme| container::Style {
                background: Some(theme::color::CARD.into()),
                border: Border {
                    color: theme::color::BORDER,
                    width: 1.0,
                    radius: 8.0.into(),
                },
                ..container::Style::default()
            });
    header = header.push(card);
    if let Some(err) = &ws_state.tree_error {
        header = header.push(
            text(format!("⚠ {err}"))
                .size(theme::font::label())
                .color(theme::color::RED),
        );
    }
    if let Some(tree) = &ws_state.file_tree {
        for row in tree.visible_rows() {
            let is_renaming = matches!(
                &ws_state.tree_edit,
                Some(TreeEdit { mode: TreeEditMode::Rename(p), .. }) if *p == row.path
            );
            if is_renaming {
                let buffer = ws_state
                    .tree_edit
                    .as_ref()
                    .map(|e| e.buffer.as_str())
                    .unwrap_or("");
                tree_col = tree_col.push(tree_edit_row(row.depth, buffer));
                continue;
            }
            let indent = "  ".repeat(row.depth);
            let status: Option<(delivery::ChangeKind, bool)> = if row.is_dir {
                delivery::dir_status(&row.path, &ws_state.git_statuses)
                    .map(|d| (d.kind, d.unstaged))
            } else {
                ws_state
                    .git_statuses
                    .get(&row.path)
                    .map(|s| (s.kind, s.unstaged))
            };
            let name_color = theme::color::BODY;
            let row_icon: Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> =
                if row.is_dir {
                    let chevron = if row.expanded {
                        icons::IconKind::ChevronDown
                    } else {
                        icons::IconKind::ChevronRight
                    };
                    let folder = if row.expanded {
                        icons::IconKind::FolderOpen
                    } else {
                        icons::IconKind::Folder
                    };
                    row![
                        icons::view(
                            chevron,
                            crate::theme::icon_size::chevron(),
                            theme::color::DIM
                        ),
                        icons::view(folder, crate::theme::icon_size::row(), theme::color::DIM),
                    ]
                    .spacing(crate::theme::icon_size::tree_row_gap())
                    .align_y(iced_widget::core::Alignment::Center)
                    .into()
                } else {
                    row![
                        iced_widget::space::Space::new()
                            .width(Length::Fixed(
                                crate::theme::icon_size::chevron()
                                    + crate::theme::icon_size::tree_row_gap(),
                            ))
                            .height(Length::Shrink),
                        icons::view(
                            icons::icon_for_file(&row.name),
                            crate::theme::icon_size::row(),
                            theme::color::DIM
                        ),
                    ]
                    .spacing(0)
                    .align_y(iced_widget::core::Alignment::Center)
                    .into()
                };
            let mut line = row![
                text(indent)
                    .size(crate::workspace::tree_row_font_size())
                    .line_height(LineHeight::Relative(terminal_font::line_height_factor()))
                    .color(name_color),
                row_icon,
                text(row.name.clone())
                    .size(crate::workspace::tree_row_font_size())
                    .line_height(LineHeight::Relative(terminal_font::line_height_factor()))
                    .color(name_color),
            ]
            .spacing(6)
            .align_y(iced_widget::core::Alignment::Center);
            if let Some((kind, unstaged)) = status {
                line = line.push(iced_widget::space::horizontal());
                line = line.push(
                    text(tree_row_dot_glyph(unstaged))
                        .size(theme::font::dot_xs())
                        .color(tree_row_dot_color(kind)),
                );
            }
            let msg = if row.is_dir {
                Message::Toggle(row.path.clone())
            } else {
                Message::OpenFile(row.path.clone())
            };
            let is_selected = ws_state.tree_selected.as_deref() == Some(row.path.as_path());
            let row_btn: iced_widget::Button<
                '_,
                Message,
                iced_widget::Theme,
                iced_widget::Renderer,
            > = button(line)
                .on_press(msg)
                .width(Length::Fill)
                .style(move |_t, _s| button::Style {
                    background: if is_selected {
                        Some(theme::color::CARD.into())
                    } else {
                        None
                    },
                    text_color: theme::color::BODY,
                    ..button::Style::default()
                });
            tree_col = tree_col.push(MouseArea::new(row_btn).on_right_press(
                Message::ContextMenuOpen {
                    path: row.path.clone(),
                    is_dir: row.is_dir,
                },
            ));
            let is_new_target = matches!(
                &ws_state.tree_edit,
                Some(TreeEdit {
                    mode: TreeEditMode::NewFile | TreeEditMode::NewFolder,
                    parent_dir,
                    ..
                }) if *parent_dir == row.path
            );
            if is_new_target && row.expanded {
                let buffer = ws_state
                    .tree_edit
                    .as_ref()
                    .map(|e| e.buffer.as_str())
                    .unwrap_or("");
                tree_col = tree_col.push(tree_edit_row(row.depth + 1, buffer));
            }
        }
    }

    // 头部(项目信息卡)固定在文件树上方、不进 scrollable,所以即使文件树
    // 出现滚动条,项目信息也始终可见;scrollable 只承载文件树行。
    let body = container(
        column![
            header,
            Scrollable::new(tree_col)
                .width(Length::Fill)
                .height(Length::Fill)
                .direction(scrollable::Direction::Vertical(
                    crate::scrollbar::scrollbar()
                ))
                .style(|_t, _s| crate::scrollbar::scrollbar_style()),
        ]
        .spacing(region.gap),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .padding(region.padding)
    .style(move |_t: &iced_widget::Theme| container::Style {
        background: region.background.map(Into::into),
        border: outer,
        ..container::Style::default()
    });

    // 底栏(`project_status_bar`)是贴在 `body` 下方的独立元素,若它自己的
    // 底角不收圆,方角会戳出 `body` 已收圆的左下角,在 zone 圆角 CARD 背景上
    // 顶出一个小尖角——所以把 `outer` 的圆角半径透给底栏,只收底角,保留它
    // 自己那条 1px 上边分隔线。
    container(column![
        body,
        project_status_bar(daemon_ok, ws_state, outer)
    ])
    .width(width)
    .height(Length::Fill)
    .into()
}

/// 项目栏底状态条：左 环境/dozerd 点，右 [文件|git {分支}|组件]（文件高亮,组件占位）。
fn project_status_bar<'a>(
    daemon_ok: bool,
    ws_state: &'a WorkspaceState,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    let (env, dot) = crate::workspace::env_status_text(daemon_ok);
    let left = row![
        text("●").size(theme::font::dot_sm()).color(dot),
        text(env)
            .size(theme::font::caption())
            .color(theme::color::BODY)
    ]
    .spacing(6);
    let git = format!(
        "git {}",
        crate::workspace::project_branch_label(ws_state.branch.as_deref(), ws_state.dirty)
    );
    let tabs = row![
        text("文件")
            .size(theme::font::caption())
            .color(theme::color::CREAM),
        text("·")
            .size(theme::font::caption())
            .color(theme::color::DIM),
        text(git)
            .size(theme::font::caption())
            .color(theme::color::BODY),
        text("·")
            .size(theme::font::caption())
            .color(theme::color::DIM),
        text("组件")
            .size(theme::font::caption())
            .color(theme::color::DIM),
    ]
    .spacing(6);
    crate::workspace::status_bar_container(
        row![left, iced_widget::space::horizontal(), tabs]
            .align_y(iced_widget::core::Alignment::Center),
        outer,
    )
}

/// 行内编辑框(新建/重命名共用):自绘输入,尾缀 "▏" 模拟光标,与地址栏/
/// 验收意见框同款风格(键盘走 main.rs 拦截层,不用 iced 原生 text_input)。
fn tree_edit_row(
    depth: usize,
    buffer: &str,
) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let indent = "  ".repeat(depth);
    container(
        text(format!("{indent}{buffer}▏"))
            .size(crate::workspace::tree_row_font_size())
            .line_height(LineHeight::Relative(terminal_font::line_height_factor()))
            .color(theme::color::CREAM),
    )
    .width(Length::Fill)
    .padding([2, 4])
    .style(|_t: &iced_widget::Theme| container::Style {
        background: Some(theme::color::CARD.into()),
        border: Border {
            color: theme::color::CREAM,
            width: 1.0,
            radius: 2.0.into(),
        },
        ..container::Style::default()
    })
    .into()
}

/// 右键菜单一项:图标+文字按钮,CARD 底+BORDER 描边悬停态由 iced 默认
/// button 交互色处理(本仓其余按钮同款,不额外定制)。
fn menu_item<'a>(
    icon: icons::IconKind,
    label: &'static str,
    msg: Message,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    button(
        row![
            icons::view(icon, crate::theme::icon_size::row(), theme::color::CREAM),
            text(label)
                .size(theme::font::body())
                .color(theme::color::CREAM),
        ]
        .spacing(crate::theme::geometry::menu_gap())
        .align_y(iced_widget::core::Alignment::Center),
    )
    .on_press(msg)
    .width(Length::Fixed(crate::theme::geometry::menu_item_width()))
    .padding([
        crate::theme::geometry::menu_pad_v(),
        crate::theme::geometry::menu_pad_h(),
    ])
    .style(|_t, _s| button::Style {
        background: Some(theme::color::CARD.into()),
        text_color: theme::color::CREAM,
        ..button::Style::default()
    })
    .into()
}

/// 右键菜单浮层本体:纵向按钮列表,`container` 用 `Padding{top,left,..}`
/// 手算定位到点击坐标——`Stack` 各层共享同一份 bounds,不像原生系统菜单
/// 那样自带绝对定位,这是本仓一贯的手算像素定位风格(`ime_cursor_area`/
/// `preview_content_bounds` 同款)。
pub fn context_menu_popup<'a>(
    app_state: &'a AppState,
    ws_state: &'a WorkspaceState,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    let Some(menu) = &app_state.context_menu else {
        return column![].into();
    };
    let mut items: Vec<Element<'_, Message, iced_widget::Theme, iced_widget::Renderer>> =
        Vec::new();
    if menu.is_dir {
        items.push(menu_item(
            icons::IconKind::FilePlus,
            "新建文件",
            Message::NewFile(menu.target.clone()),
        ));
        items.push(menu_item(
            icons::IconKind::FolderPlus,
            "新建文件夹",
            Message::NewFolder(menu.target.clone()),
        ));
    }
    items.push(menu_item(
        icons::IconKind::Copy,
        "复制",
        Message::Copy(menu.target.clone(), menu.is_dir),
    ));
    if menu.is_dir {
        let has_clipboard = ws_state.tree_clipboard.is_some();
        let paste_msg = Message::Paste(menu.target.clone());
        items.push(if has_clipboard {
            menu_item(icons::IconKind::ClipboardPaste, "粘贴", paste_msg)
        } else {
            // 剪贴槽为空:置灰且不挂 on_press,真正不可点(同 P1L tab 箭头
            // "到头变灰"的既有处理口径,不是视觉变灰但仍能点)。
            button(
                row![
                    icons::view(
                        icons::IconKind::ClipboardPaste,
                        crate::theme::icon_size::row(),
                        theme::color::DIM
                    ),
                    text("粘贴")
                        .size(theme::font::body())
                        .color(theme::color::DIM),
                ]
                .spacing(crate::theme::geometry::menu_gap())
                .align_y(iced_widget::core::Alignment::Center),
            )
            .width(Length::Fixed(crate::theme::geometry::menu_item_width()))
            .padding([
                crate::theme::geometry::menu_pad_v(),
                crate::theme::geometry::menu_pad_h(),
            ])
            .style(|_t, _s| button::Style {
                background: Some(theme::color::CARD.into()),
                text_color: theme::color::DIM,
                ..button::Style::default()
            })
            .into()
        });
    }
    items.push(menu_item(
        icons::IconKind::Trash,
        "删除",
        Message::DeleteRequest(menu.target.clone(), menu.is_dir),
    ));
    items.push(menu_item(
        icons::IconKind::Rename,
        "重命名",
        Message::RenameStart(menu.target.clone()),
    ));
    items.push(menu_item(
        icons::IconKind::Copy,
        "复制绝对路径",
        Message::CopyPath(menu.target.clone(), crate::project::PathKind::Absolute),
    ));
    items.push(menu_item(
        icons::IconKind::Copy,
        "复制相对路径",
        Message::CopyPath(menu.target.clone(), crate::project::PathKind::Relative),
    ));
    items.push(menu_item(
        icons::IconKind::FolderOpen,
        "在 Finder 中打开",
        Message::RevealInFinder(menu.target.clone()),
    ));
    items.push(menu_item(
        icons::IconKind::RefreshCw,
        "从磁盘重新加载",
        Message::ReloadFromDisk,
    ));

    let region = theme::region::context_menu();
    let list = container(column(items).spacing(region.gap))
        .padding(region.padding)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: region.border.unwrap_or_default(),
            ..container::Style::default()
        });

    container(list)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(Padding {
            top: menu.y,
            left: menu.x,
            right: 0.0,
            bottom: 0.0,
        })
        .into()
}

/// 删除确认框:居中浮层,显示目标文件名 + 确认/取消两个按钮。
pub fn delete_confirm_popup(
    ws_state: &WorkspaceState,
) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let Some((path, is_dir)) = &ws_state.tree_delete_confirm else {
        return column![].into();
    };
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    let kind = if *is_dir { "文件夹" } else { "文件" };
    let dialog = container(
        column![
            text(format!("删除{kind} \"{name}\"?"))
                .size(theme::font::subtitle())
                .color(theme::color::CREAM),
            text("会移入系统回收站,可从回收站找回。")
                .size(theme::font::label())
                .color(theme::color::DIM),
            row![
                button(
                    text("取消")
                        .size(theme::font::body())
                        .color(theme::color::CREAM)
                )
                .on_press(Message::DeleteCancel)
                .padding([6, 12])
                .style(|_t, _s| button::Style {
                    background: Some(theme::color::CARD.into()),
                    text_color: theme::color::CREAM,
                    border: Border {
                        color: theme::color::BORDER,
                        width: 1.0,
                        radius: 4.0.into()
                    },
                    ..button::Style::default()
                }),
                button(
                    text("删除")
                        .size(theme::font::body())
                        .color(theme::color::RED)
                )
                .on_press(Message::DeleteConfirm)
                .padding([6, 12])
                .style(|_t, _s| button::Style {
                    background: Some(theme::color::CARD.into()),
                    text_color: theme::color::RED,
                    border: Border {
                        color: theme::color::RED,
                        width: 1.0,
                        radius: 4.0.into()
                    },
                    ..button::Style::default()
                }),
            ]
            .spacing(8),
        ]
        .spacing(8),
    )
    .padding(16)
    .style(|_t: &iced_widget::Theme| container::Style {
        background: Some(theme::color::CARD.into()),
        border: Border {
            color: theme::color::BORDER,
            width: 1.0,
            radius: 6.0.into(),
        },
        ..container::Style::default()
    });

    container(dialog)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(iced_widget::core::alignment::Horizontal::Center)
        .align_y(iced_widget::core::alignment::Vertical::Center)
        .into()
}

/// 文件树色点颜色编码 git 状态(D2):修改=金,新增=绿,删除=红。
fn tree_row_dot_color(kind: delivery::ChangeKind) -> iced_widget::core::Color {
    match kind {
        delivery::ChangeKind::Modified => theme::color::GOLD,
        delivery::ChangeKind::New => theme::color::GREEN,
        delivery::ChangeKind::Deleted => theme::color::RED,
    }
}

/// 色点字形编码暂存态(D2):全部暂存(无未暂存改动)→ 实心 `●`;有任何未
/// 暂存改动(不论是否同时有暂存部分)→ 空心 `○`。尾缀字符不重复编码 kind
/// (颜色已经够用),避免过度设计。
fn tree_row_dot_glyph(unstaged: bool) -> &'static str {
    if unstaged { "○" } else { "●" }
}

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
        let mut app_state = AppState {
            context_menu: Some(ContextMenu {
                x: 0.0,
                y: 0.0,
                target: PathBuf::from("/x"),
                is_dir: false,
            }),
            ..AppState::default()
        };
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
    async fn paste_done_ok_refreshes_parent_without_touching_stale_error() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        // 现有 `ProjectTreePasteDone` 的 `Ok` 分支不清 `tree_error`(只有
        // `ProjectTreeOpDone` 的 `Ok` 分支才清)——纯迁移原样保留这个不对称,
        // 不是这次重构该修的行为。
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
        assert_eq!(ws_state.tree_error.as_deref(), Some("stale"));
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
        let mut app_state = AppState {
            context_menu: Some(ContextMenu {
                x: 0.0,
                y: 0.0,
                target: dir.path().to_path_buf(),
                is_dir: true,
            }),
            ..AppState::default()
        };
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
    async fn submit_edit_empty_name_cancels_and_closes_edit_box() {
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
        // 现有 `submit_tree_edit` 先 `take()` 再判断空名字,空名字直接 `return`
        // 时 `tree_edit` 已经被取走——提交空名字会关闭行内编辑框(等价于取消),
        // 不是保留编辑框让用户继续输入。
        assert!(ws_state.tree_edit.is_none());
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
        assert!(
            ws_state
                .tree_error
                .as_deref()
                .unwrap()
                .contains("已存在同名项")
        );
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
