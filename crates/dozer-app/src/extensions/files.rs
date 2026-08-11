//! Files(项目文件树)面板:文件树 + 右键菜单/删除确认浮层。阶段 1 扩展化
//! 重构第四个试点,设计见
//! `docs/superpowers/specs/2026-08-07-files-extension-pilot-design.md`。
use crate::delivery::FileGitStatus;
use crate::project::{FileTree, PathKind};
use crate::theme::terminal_font;
use crate::workspace::AddrEvent;
use crate::{delivery, icons, theme};
use iced_widget::core::text::LineHeight;
use iced_widget::core::{Border, Color, Element, Length, Padding};
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
    git_statuses: HashMap<PathBuf, FileGitStatus>,
    tree_selected: Option<PathBuf>,
    tree_clipboard: Option<(PathBuf, bool)>,
    tree_error: Option<String>,
    tree_delete_confirm: Option<(PathBuf, bool)>,
    tree_edit: Option<TreeEdit>,
    /// 搜索框里正在键入的草稿文本(尚未提交时不影响树)。
    tree_search: String,
    /// 已提交的搜索关键字:仅当提交(敲回车/点搜索按钮)后用它过滤树。
    search_query: String,
    /// 搜索框是否处于自绘编辑态:键盘走 main.rs 拦截层(同树内行编辑
    /// `tree_edit`/验收意见框,不用 iced 原生 text_input)。为真时 main.rs 把
    /// 按键路由成 `SearchEvent`,不再喂给 PTY——否则在搜索框里打字会同时
    /// 漏进已聚焦的终端(本项目所有文本输入都是自绘,理由一致)。
    search_editing: bool,
}

/// 挂在 App 上的右键菜单浮层状态(屏幕空间单例,不随项目切换各自保留)。
/// 对应现有 `App` 上 `context_menu`/`last_right_click` 两个字段。
#[derive(Default)]
pub struct AppState {
    context_menu: Option<ContextMenu>,
    last_right_click: (f32, f32),
}

/// 对应现在顶层 `Message` 里的 20 个 `ProjectTreeXxx`/`ProjectGitRefreshed`/
/// `RightClickAt` 变体,去前缀原样搬来。
#[derive(Debug, Clone)]
pub enum Message {
    Toggle(PathBuf),
    StatusesRefreshed(i64, HashMap<PathBuf, FileGitStatus>),
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
    /// 点进搜索框开始编辑:置 `search_editing = true`,此后按键交 main.rs
    /// 拦截层路由成 `SearchEvent`(不再漏进终端)。
    SearchEditStart,
    /// 搜索框编辑态下的按键:只动草稿 `tree_search`,不重新过滤(需提交)。
    SearchEvent(AddrEvent),
    /// 提交搜索:把草稿 `tree_search` 落成生效的过滤 `search_query`。
    SearchSubmit,
    /// 切换"显示/隐藏以 `.` 开头的文件/目录"(搜索框后的眼睛按钮)。翻转
    /// 后调用 `file_tree.set_show_dotfiles` 重读已缓存目录,让树立刻反映。
    ToggleDotfiles,
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
        self.git_statuses = HashMap::new();
        self.tree_search.clear();
        self.search_query.clear();
        self.search_editing = false;
    }

    /// 供内核判断"删除确认浮层该不该显示"(`App::view()` 顶层互斥浮层
    /// 判断链用,见设计文档 §5)。
    pub fn tree_delete_confirm_is_some(&self) -> bool {
        self.tree_delete_confirm.is_some()
    }

    /// 供内核 `Workspace::blur_inputs`(点击输入框外时退出所有自绘输入的
    /// 编辑态)调用——原逻辑直接 `self.tree_edit = None`,字段私有化后改走
    /// 这个访问器。
    pub fn cancel_tree_edit(&mut self) {
        self.tree_edit = None;
    }

    /// 搜索框失焦退出编辑态(`Workspace::blur_inputs` 用)`:草稿 `tree_search`
    /// 保留,退出后仍作为盒子里的占位/已输入文本继续显示。
    pub fn cancel_search_edit(&mut self) {
        self.search_editing = false;
    }

    /// 供内核 `Workspace::search_editing`(main.rs 键盘路由用)判断搜索框
    /// 是否处于自绘编辑态。
    pub fn search_editing(&self) -> bool {
        self.search_editing
    }

    /// 供内核 `Workspace::tree_editing`(main.rs 键盘路由用,判断项目树是否
    /// 处于行内编辑态)调用。
    pub fn tree_edit_is_some(&self) -> bool {
        self.tree_edit.is_some()
    }

    /// 供内核 `Message::PreviewOpenPath` 处理器调用——打开预览的同时把该
    /// 文件标记为项目树里的"选中"行(点击文件行→打开预览→该行高亮,是
    /// 现状既有的联动效果,不是这次重构新增的)。
    pub fn set_tree_selected(&mut self, path: PathBuf) {
        self.tree_selected = Some(path);
    }

    /// 项目树操作的行内报错文案,供内核测试用作槽位内容的身份标记(见
    /// `workspace.rs` 测试模块 `loaded_slot`)。只有测试会调用,生产代码不
    /// 需要读它(渲染走 `files::view` 内部,不经这个访问器)。
    #[cfg(test)]
    pub fn tree_error(&self) -> Option<&str> {
        self.tree_error.as_deref()
    }

    /// 同上,供测试构造带标记的槽位用。
    #[cfg(test)]
    pub fn set_tree_error(&mut self, e: Option<String>) {
        self.tree_error = e;
    }

    /// 文件树是否已建立(供内核测试断言"占位构造立刻有文件树根,不需要
    /// 等 IO"用)。只有测试会调用。
    #[cfg(test)]
    pub fn file_tree_is_some(&self) -> bool {
        self.file_tree.is_some()
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
    /// 关掉文件树右键菜单(供其它浮层打开时互斥清理,见 `App::update` 的
    /// `PreviewTabContextMenu` 分支——避免两者同时挂着导致 dismiss 串味)。
    pub fn close_context_menu(&mut self) {
        self.context_menu = None;
    }
    /// 最近一次右键落点坐标(屏幕空间,已由 main.rs 钳制在窗口内)。
    /// 预览 tab 右键菜单复用同一份坐标,避免再写一套捕获逻辑。
    pub fn last_right_click(&self) -> (f32, f32) {
        self.last_right_click
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
        Message::StatusesRefreshed(_, statuses) => {
            ws_state.git_statuses = statuses;
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
        Message::SearchEditStart => {
            ws_state.search_editing = true;
        }
        Message::SearchEvent(ev) => {
            // 只在搜索框编辑态处理按键(点击盒子进入编辑态后,main.rs 才把
            // 按键路由成这个变体);未进入时收到属异常,直接忽略。
            if !ws_state.search_editing {
                return;
            }
            match ev {
                AddrEvent::Text(s) => ws_state.tree_search.push_str(&s),
                AddrEvent::Backspace => {
                    ws_state.tree_search.pop();
                }
                AddrEvent::Cancel => ws_state.search_editing = false,
                AddrEvent::Submit => {
                    ws_state.search_query = ws_state.tree_search.clone();
                    ws_state.search_editing = false;
                }
            }
        }
        Message::SearchSubmit => {
            ws_state.search_query = ws_state.tree_search.clone();
        }
        Message::ToggleDotfiles => {
            if let Some(tree) = &mut ws_state.file_tree {
                tree.set_show_dotfiles(!tree.dotfiles_shown());
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

/// 文件树可滚动列表(现有 `workspace.rs::project_pane` 的搬家版本,签名改吃
/// 本模块状态,去掉了不再归属本模块的项目信息卡/底部状态条)。
pub fn view<'a>(
    ws_state: &'a WorkspaceState,
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let region = theme::region::project_pane();
    let mut header = column![].spacing(region.gap).width(Length::Fill);
    let mut tree_col = column![].spacing(region.gap);

    // 文件树搜索框:按文件/目录名称筛选整棵树(大小写不敏感子串匹配)。
    // 不会边输入边过滤——敲回车/点右侧"搜索"按钮后,由 `SearchSubmit` 把
    // 草稿落成为生效的 `search_query`。这是自绘输入(同树内行编辑/验收意见
    // 框):键盘走 main.rs 拦截层路由成 `SearchEvent`,不用 iced 原生
    // text_input——原生输入无法让 main.rs 知道它挂在焦点上,打字会同时漏进
    // 已聚焦的终端(本项目所有文本输入都为此自绘,理由一致)。
    let search_active = !ws_state.search_query.is_empty();
    let search_box = search_box_widget(
        &ws_state.tree_search,
        ws_state.search_editing,
        search_active,
    );
    let search_button = button(icons::view(
        icons::IconKind::FolderSearch,
        crate::theme::icon_size::row(),
        theme::color::CREAM,
    ))
    .on_press(Message::SearchSubmit)
    .padding([6, 8])
    .style(|_t: &iced_widget::Theme, _s| button::Style {
        background: Some(theme::color::CARD.into()),
        border: Border {
            color: theme::color::BORDER,
            width: 1.0,
            radius: 4.0.into(),
        },
        text_color: theme::color::CREAM,
        ..button::Style::default()
    });

    // "显示/隐藏点文件"按钮:切换后 `ToggleDotfiles` 调
    // `set_show_dotfiles` 重读树。图标反映当前口径——正显示(`eye`)时点它
    // 隐藏点文件;隐藏(`eye-off`)时点它恢复显示。点文件被隐藏时按钮边框
    // 用 GOLD 高亮提示当前树缺了点文件(与搜索生效同款"功能开启"信号)。
    let dotfiles_shown = ws_state
        .file_tree
        .as_ref()
        .map(|t| t.dotfiles_shown())
        .unwrap_or(true);
    let dotfiles_button = button(icons::view(
        if dotfiles_shown {
            icons::IconKind::Eye
        } else {
            icons::IconKind::EyeOff
        },
        crate::theme::icon_size::row(),
        if dotfiles_shown {
            theme::color::CREAM
        } else {
            theme::color::GOLD
        },
    ))
    .on_press(Message::ToggleDotfiles)
    .padding([6, 8])
    .style(move |_t: &iced_widget::Theme, _s| button::Style {
        background: Some(theme::color::CARD.into()),
        border: Border {
            color: if dotfiles_shown {
                theme::color::BORDER
            } else {
                theme::color::GOLD
            },
            width: 1.0,
            radius: 4.0.into(),
        },
        text_color: theme::color::CREAM,
        ..button::Style::default()
    });
    header = header.push(
        row![search_box, search_button, dotfiles_button]
            .spacing(6)
            .align_y(iced_widget::core::Alignment::Center),
    );

    // 根目录头部:只显示名称(CREAM 高亮),不再直接显示完整路径;名称前
    // 挂 folder-open-dot 图标(lucide 的展开文件夹 + 圆点,有别于普通展开目录
    // 的 folder-open,特标项目根)。顶部留一点边距,把根目录头和上方
    // 搜索/点文件工具行分隔开。右键根目录打开目录右键菜单(新建文件/文件夹、
    // 复制、粘贴、删除、重命名、在 Finder 打开、从磁盘重新加载…),坐标复用
    // `main.rs` 右键时写入的 `last_right_click`。
    if let Some(tree) = &ws_state.file_tree {
        let root = tree.root();
        let name = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| root.display().to_string());
        let root_header = container(
            row![
                icons::view(
                    icons::IconKind::FolderOpenDot,
                    crate::theme::icon_size::row(),
                    theme::color::CREAM
                ),
                text(name)
                    .size(theme::font::body())
                    .color(theme::color::CREAM),
            ]
            .spacing(6)
            .align_y(iced_widget::core::Alignment::Center),
        )
        .width(Length::Fill)
        .padding([8, 0]);
        header = header.push(MouseArea::new(root_header).on_right_press(
            Message::ContextMenuOpen {
                path: root.to_path_buf(),
                is_dir: true,
            },
        ));
    }

    if let Some(err) = &ws_state.tree_error {
        header = header.push(
            text(format!("⚠ {err}"))
                .size(theme::font::label())
                .color(theme::color::RED),
        );
    }
    if let Some(tree) = &ws_state.file_tree {
        // 搜索激活时走全树搜索(递归遍历含未展开深层目录),否则走当前展开
        // 的可见行。`search_rows` 只读不写缓存/展开态,view 的不变借用即可。
        let rows: Vec<crate::project::TreeRow> = if search_active {
            tree.search_rows(&ws_state.search_query)
        } else {
            tree.visible_rows()
        };
        for row in rows {
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
            // git 状态编码名称颜色:未加入版本=红(最高优先),加入版本未提交
            // 的新文件=绿,修改/删除未提交=青,一般=灰,被忽略=弱灰。目录
            // 聚合取子孙中最高档(`dir_status`),让用户先注意到没加入版本
            // 管理的文件。无任何 git 记录的干净条目(状态 `None`)补成"一般"。
            let state: delivery::TreeState = if row.is_dir {
                delivery::dir_status(&row.path, &ws_state.git_statuses)
                    .unwrap_or(delivery::TreeState::Unchanged)
            } else {
                ws_state
                    .git_statuses
                    .get(&row.path)
                    .copied()
                    .map(delivery::TreeState::from)
                    .unwrap_or(delivery::TreeState::Unchanged)
            };
            let name_color = tree_state_color(state);
            let row_icon: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
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
            let line = row![
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
                iced_renderer::Renderer,
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

    let body = container(
        column![
            crate::homespace::home_panel_head(icons::IconKind::FolderTree, "文件"),
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

    container(body).width(width).height(Length::Fill).into()
}

/// 行内编辑框(新建/重命名共用):自绘输入,尾缀 "▏" 模拟光标,与地址栏/
/// 验收意见框同款风格(键盘走 main.rs 拦截层,不用 iced 原生 text_input)。
fn tree_edit_row(
    depth: usize,
    buffer: &str,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
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

/// 文件树搜索框:自绘输入(键盘走 main.rs 拦截层路由成 `SearchEvent`,不用
/// iced 原生 text_input——原生输入没法让 main.rs 知道它挂着焦点,打字会同步
/// 漏进已聚焦的终端)。整体是 `button`,点击(`SearchEditStart`)进入编辑态;
/// 视觉上是普通输入框,不带按钮的按压/悬停感。
///
/// - 草稿为空且未编辑:显式 DIM 占位符 "搜索文件/目录…"。
/// - 编辑态:草稿文本 + 尾缀 "▏" 光标,边框 GOLD 高亮表示焦点归属搜索框。
/// - 已过滤(`active`):边框 GOLD 常亮,提示当前树被搜索词收窄。
fn search_box_widget(
    draft: &str,
    editing: bool,
    active: bool,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let body = if draft.is_empty() && !editing {
        text("搜索文件/目录…")
            .size(theme::font::body())
            .color(theme::color::DIM)
    } else {
        let caret = if editing { "▏" } else { "" };
        text(format!("{draft}{caret}"))
            .size(theme::font::body())
            .color(theme::color::CREAM)
    };

    button(body)
        .on_press(Message::SearchEditStart)
        .width(Length::Fill)
        .padding([6, 8])
        .style(move |_t: &iced_widget::Theme, _s| button::Style {
            background: Some(theme::color::BG.into()),
            border: Border {
                color: if editing || active {
                    theme::color::GOLD
                } else {
                    theme::color::BORDER
                },
                width: 1.0,
                radius: 4.0.into(),
            },
            text_color: theme::color::CREAM,
            ..button::Style::default()
        })
        .into()
}

/// 右键菜单一项:图标(可选)+文字按钮。默认底色透出容器背景,hover/pressed
/// 切到 `TAB_HOVER`(同顶栏/面板 tab 的 hover 背景 `#152630`);按下即
/// `Pressed` 仍走同款背景,让按住期间有视觉反馈。文本保持 CREAM(在
/// `TAB_HOVER` 深底上可读,与 tab hover 文字色一致)。图标颜色在创建时烘焙,
/// 无法随 hover 切换——保持 CREAM(同文字色,差异不显著,避免过度工程去
/// 重写 `icons::view` 的颜色级联)。
///
/// `icon` 传 `None` 时只渲染文字(用于"复制绝对路径/相对路径"这类不需要
/// 图标的条目),文字起始 x 与有图标项的图标起始 x 对齐。
fn menu_item<'a>(
    icon: Option<icons::IconKind>,
    label: &'static str,
    msg: Message,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let content = match icon {
        Some(icon) => row![
            icons::view(icon, crate::theme::icon_size::row(), theme::color::CREAM),
            text(label).size(theme::font::body()),
        ],
        None => row![text(label).size(theme::font::body())],
    };
    button(
        content
            .spacing(crate::theme::geometry::menu_gap())
            .align_y(iced_widget::core::Alignment::Center),
    )
    .on_press(msg)
    .width(Length::Fixed(crate::theme::geometry::menu_item_width()))
    .padding([
        crate::theme::geometry::menu_pad_v(),
        crate::theme::geometry::menu_pad_h(),
    ])
    .style(|_t, s| {
        let base = button::Style {
            background: None,
            text_color: theme::color::CREAM,
            ..button::Style::default()
        };
        match s {
            button::Status::Hovered | button::Status::Pressed => button::Style {
                background: Some(theme::color::TAB_HOVER.into()),
                text_color: theme::color::CREAM,
                border: Border {
                    color: Color::TRANSPARENT,
                    width: 0.0,
                    radius: 4.0.into(),
                },
                ..base
            },
            _ => base,
        }
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
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let Some(menu) = &app_state.context_menu else {
        return column![].into();
    };
    let mut items: Vec<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>> =
        Vec::new();
    let push_sep =
        |items: &mut Vec<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>>| {
            if !items.is_empty() {
                items.push(menu_separator());
            }
        };
    // 目标是否为项目根:根目录不可删除/重命名(否则会连整个项目目录一起
    // 删/改名),据此从菜单隐去对应项。
    let is_root = ws_state
        .file_tree
        .as_ref()
        .map(|t| t.root() == menu.target.as_path())
        .unwrap_or(false);
    if menu.is_dir {
        items.push(menu_item(
            Some(icons::IconKind::FilePlus),
            "新建文件",
            Message::NewFile(menu.target.clone()),
        ));
        items.push(menu_item(
            Some(icons::IconKind::FolderPlus),
            "新建文件夹",
            Message::NewFolder(menu.target.clone()),
        ));
    }
    push_sep(&mut items);
    items.push(menu_item(
        Some(icons::IconKind::Copy),
        "复制",
        Message::Copy(menu.target.clone(), menu.is_dir),
    ));
    if menu.is_dir {
        let has_clipboard = ws_state.tree_clipboard.is_some();
        let paste_msg = Message::Paste(menu.target.clone());
        items.push(if has_clipboard {
            menu_item(Some(icons::IconKind::ClipboardPaste), "粘贴", paste_msg)
        } else {
            // 剪贴槽为空:置灰且不挂 on_press,真正不可点(同 P1L tab 箭头
            // "到头变灰"的既有处理口径,不是视觉变灰但仍能点)。背景透明
            // 透出容器底,不要 hover 高亮——保持视觉一致的"灰且不可点"。
            // 文字不挂显式 color,让 button style 的 text_color(DIM)接管,
            // 与 `menu_item` 让 text_color 接管 hover 切白的处理同源。
            button(
                row![
                    icons::view(
                        icons::IconKind::ClipboardPaste,
                        crate::theme::icon_size::row(),
                        theme::color::DIM
                    ),
                    text("粘贴").size(theme::font::body()),
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
                background: None,
                text_color: theme::color::DIM,
                ..button::Style::default()
            })
            .into()
        });
    }
    // 项目根不可删除/重命名:从菜单隐去这两项(其余目录均可)。
    if !is_root {
        items.push(menu_item(
            Some(icons::IconKind::Trash),
            "删除",
            Message::DeleteRequest(menu.target.clone(), menu.is_dir),
        ));
        items.push(menu_item(
            Some(icons::IconKind::Rename),
            "重命名",
            Message::RenameStart(menu.target.clone()),
        ));
    }
    push_sep(&mut items);
    items.push(menu_item(
        None,
        "复制绝对路径",
        Message::CopyPath(menu.target.clone(), crate::project::PathKind::Absolute),
    ));
    items.push(menu_item(
        None,
        "复制相对路径",
        Message::CopyPath(menu.target.clone(), crate::project::PathKind::Relative),
    ));
    items.push(menu_item(
        Some(icons::IconKind::FolderOpen),
        "在 Finder 中打开",
        Message::RevealInFinder(menu.target.clone()),
    ));
    items.push(menu_item(
        Some(icons::IconKind::RefreshCw),
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

/// 菜单项之间的细分隔线:1px BORDER 高度,左右各留一点内边距,与 macOS
/// 系统菜单分组线同款。列项之间由 `column.spacing` 控间距,分隔线本身不
/// 再额外加 padding。
fn menu_separator<'a>() -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    container(iced_widget::Space::new())
        .width(Length::Fill)
        .height(Length::Fixed(1.0))
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(theme::color::BORDER.into()),
            ..container::Style::default()
        })
        .into()
}

/// 删除确认框:居中浮层,显示目标文件名 + 确认/取消两个按钮。
pub fn delete_confirm_popup(
    ws_state: &WorkspaceState,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
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

/// 文件树名称颜色编码 git 状态,取代早前 D2 的行尾色点。按
/// `delivery::TreeState` 档位取色:未加入版本 → 红 `RED`;加入版本未提交的
/// 新文件 → 绿 `GREEN`;修改/删除未提交 → 青 `CYAN`;被忽略 → 弱灰
/// `IGNORED`。无改动(状态为 `None`)由调用方给灰色 `BODY`。
fn tree_state_color(state: delivery::TreeState) -> iced_widget::core::Color {
    match state {
        delivery::TreeState::Untracked => theme::color::RED,
        delivery::TreeState::StagedNew => theme::color::GREEN,
        delivery::TreeState::Modified => theme::color::CYAN,
        delivery::TreeState::Unchanged => theme::color::BODY,
        delivery::TreeState::Ignored => theme::color::IGNORED,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws_with_tree(root: PathBuf) -> WorkspaceState {
        WorkspaceState::new(FileTree::new(root))
    }

    #[test]
    fn tree_state_colors() {
        assert_eq!(
            tree_state_color(delivery::TreeState::Untracked),
            theme::color::RED
        );
        assert_eq!(
            tree_state_color(delivery::TreeState::StagedNew),
            theme::color::GREEN
        );
        assert_eq!(
            tree_state_color(delivery::TreeState::Modified),
            theme::color::CYAN
        );
        assert_eq!(
            tree_state_color(delivery::TreeState::Unchanged),
            theme::color::BODY
        );
        assert_eq!(
            tree_state_color(delivery::TreeState::Ignored),
            theme::color::IGNORED
        );
    }

    #[tokio::test]
    async fn search_input_then_submit_commits_draft_and_reset_clears() {
        let dir = tempfile::tempdir().unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        let mut app_state = AppState::default();
        let handle = tokio::runtime::Handle::current();

        // 点进搜索框:进入自绘编辑态。
        update(
            &mut ws_state,
            &mut app_state,
            Message::SearchEditStart,
            1,
            &handle,
            |_| {},
        );
        assert!(ws_state.search_editing());

        // 键入:只进草稿,不触发过滤(生效的 search_query 仍为空)。
        update(
            &mut ws_state,
            &mut app_state,
            Message::SearchEvent(AddrEvent::Text("main".to_string())),
            1,
            &handle,
            |_| {},
        );
        assert_eq!(ws_state.tree_search, "main");
        assert!(ws_state.search_query.is_empty());

        // 提交(点右侧"搜索"按钮):草稿落成为生效过滤词。按钮本身不退出编辑
        // 态——下次 mousedown 的 `blur_inputs` 会清掉;敲回车(SearchEvent 的
        // Submit 分支)才在更新内直接退出编辑态,见断言其后。
        update(
            &mut ws_state,
            &mut app_state,
            Message::SearchSubmit,
            1,
            &handle,
            |_| {},
        );
        assert_eq!(ws_state.search_query, "main");

        // 编辑态敲回车:提交并退出编辑态。
        update(
            &mut ws_state,
            &mut app_state,
            Message::SearchEvent(AddrEvent::Submit),
            1,
            &handle,
            |_| {},
        );
        assert_eq!(ws_state.search_query, "main");
        assert!(!ws_state.search_editing());

        // 认领其它项目时清空草稿、生效词与编辑态,避免旧筛选残留在新项目树上。
        ws_state.reset_for_project(FileTree::new(dir.path().to_path_buf()));
        assert!(ws_state.tree_search.is_empty() && ws_state.search_query.is_empty());
    }

    #[tokio::test]
    async fn search_editing_flag_and_cancel_work() {
        let dir = tempfile::tempdir().unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        let mut app_state = AppState::default();
        let handle = tokio::runtime::Handle::current();

        // 初始未编辑。
        assert!(!ws_state.search_editing());

        // 进入编辑态。
        update(
            &mut ws_state,
            &mut app_state,
            Message::SearchEditStart,
            1,
            &handle,
            |_| {},
        );
        assert!(ws_state.search_editing());

        // blur_inputs 入口 `cancel_search_edit` 退出编辑态。
        ws_state.cancel_search_edit();
        assert!(!ws_state.search_editing());

        // 非编辑态下 SearchEvent 不该做任何事(草稿保持为空)。
        update(
            &mut ws_state,
            &mut app_state,
            Message::SearchEvent(AddrEvent::Text("x".to_string())),
            1,
            &handle,
            |_| {},
        );
        assert!(ws_state.tree_search.is_empty());
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
    async fn statuses_refreshed_updates_git_statuses() {
        let mut ws_state = ws_with_tree(std::env::temp_dir());
        let mut app_state = AppState::default();
        let mut statuses = HashMap::new();
        statuses.insert(
            PathBuf::from("/x"),
            crate::delivery::FileGitStatus {
                kind: crate::delivery::ChangeKind::Modified,
                staged: false,
                unstaged: true,
                ignored: false,
            },
        );
        let handle = tokio::runtime::Handle::current();
        update(
            &mut ws_state,
            &mut app_state,
            Message::StatusesRefreshed(1, statuses.clone()),
            1,
            &handle,
            |_| {},
        );
        assert_eq!(ws_state.git_statuses.len(), 1);
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
