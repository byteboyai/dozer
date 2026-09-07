//! Files(项目文件树)面板:文件树 + 右键菜单/删除确认浮层。阶段 1 扩展化
//! 重构第四个试点,设计见
//! `docs/superpowers/specs/2026-08-07-files-extension-pilot-design.md`。
use crate::delivery::FileGitStatus;
use crate::project::{FileTree, PathKind, TreeRow};
use crate::theme::terminal_font;
use crate::{delivery, theme};
use byteui::interaction::icons;
use iced_widget::core::text::LineHeight;
use iced_widget::core::widget::operation::Focusable;
use iced_widget::core::widget::{Id, Operation};
use iced_widget::core::{Border, Color, Element, Length, Padding, Rectangle};
use iced_widget::{MouseArea, Scrollable, button, column, container, row, scrollable, text};
use std::collections::{HashMap, HashSet};
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

/// 树内拖拽(文件/文件夹在项目树内移动目录)进行中的状态。`target` 是当前
/// 悬停命中的合法落点目录,悬停到非法落点(自身/自身子树)或没悬停到任何
/// 目录行时为 `None`——`TreeDragEnd` 只在 `Some` 时提交移动。
#[derive(Debug, Clone, PartialEq)]
struct TreeDrag {
    source: PathBuf,
    source_is_dir: bool,
    target: Option<PathBuf>,
    /// 按下瞬间的 `App::last_cursor`(窗口逻辑坐标),拖拽期间不更新——
    /// 内核用它和当前光标算位移,判断这是不是"真的在拖"(同
    /// `TabDrag::press_pos`/`RailDrag::press_pos` 的既有用法)。单击时的
    /// 正常光标抖动若不设这道门槛,会被 `TreeDragOver` 当成"拖到了旁边
    /// 那一行"——2026-09 用户实测反馈:点一下就报"不能把目录移到它自己或
    /// 其子目录里",根因正是本消息此前无阈值直接生效。
    press_pos: (f32, f32),
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
    /// 项目树行内编辑框是否持有 iced 内部真实焦点,每帧由 `CaptureTreeEditFocus`
    /// 写入。
    tree_edit_focused_flag: bool,
    /// 一次性标记:`tree_edit` 刚从 `None` 变成 `Some`(新建/重命名刚
    /// 触发)时置真,main.rs 渲染循环取走后用 `operation::focusable::
    /// focus` 强制聚焦真正的 `text_input`——右键菜单点"重命名"/"新建
    /// 文件"这类触发点击落在别的控件上,新出现的输入框不会自动拿到
    /// iced 焦点,需要这一下程序化聚焦(同 `todo::scroll_to_top` 的既有
    /// 一次性位手法)。
    tree_edit_focus_pending: bool,
    /// 搜索框里正在键入的草稿文本(尚未提交时不影响树)。
    tree_search: String,
    /// 已提交的搜索关键字:仅当提交(敲回车/点搜索按钮)后用它过滤树。
    search_query: String,
    /// 搜索框是否持有 iced 内部真实焦点。**不是**应用层手动置位的镜像——
    /// 每帧渲染循环里 `CaptureSearchFocus` 问一遍 iced 真相后立刻写进这里
    /// (`set_search_focused`),`main.rs` 键盘路由读它决定要不要把事件放行
    /// 给标准 iced 管线(同 `search_field_id`/`CaptureSearchFocus` 说明)。
    search_focused: bool,
    /// 文件树 Scrollable 当前滚动偏移(逻辑像素,Y 向下偏移)。由树自身的
    /// `on_scroll` 上报;main.rs 在外部文件拖拽事件层用它与行高几何做命中
    /// 测试,把光标位置换算成命中的目录行。此偏移只影响命中测试,不影响
    /// 渲染(渲染仍由 Scrollable 内部状态驱动)。
    tree_scroll: f32,
    /// 外部文件拖拽悬停时命中的目录集合:命中即整行高亮为"拖入落点",
    /// 由 `FileDragHover` 更新、清空时为空集合。
    drag_hover: HashSet<PathBuf>,
    /// 底部 git 分支栏信息是否已加载(`GitInfoLoaded` 送达前为 false,此时
    /// 分支栏显示中性"…"占位)。
    git_loaded: bool,
    /// 项目根目录是否在 git 仓库内(决定底栏显示"分支切换"还是
    /// "未受 git 保护 / 新建仓库")。
    git_is_repo: bool,
    /// 当前分支名(detached HEAD / 无提交时为 None)。
    current_branch: Option<String>,
    /// 当前分支是否已有至少一次提交(unborn/空仓为 false)。分支菜单据此把
    /// 其余分支置灰禁用 `BranchPickerOpen` 时的渲染用)。
    current_branch_has_commits: bool,
    /// 本地分支列表(仅 git 仓库内有意义;空则无分支可切)。
    git_branches: Vec<String>,
    /// 分支切换弹层是否展开(展开时在底栏上方罗列可切换的本地分支)。
    branch_picker_open: bool,
    /// 底栏 git 操作(切换分支/新建仓库)的最近错误,就地显示在底栏下缘。
    git_error: Option<String>,
    /// 树内拖拽移动进行中的状态,`None` 表示当前没有树内拖拽——见 `TreeDrag`
    /// 文档。按下树行武装(`arm_tree_drag`),`TreeDragOver`/`TreeDragEnd`
    /// 更新/收尾。
    tree_drag: Option<TreeDrag>,
}

/// 一次 git 仓库信息加载的结果(分支栏渲染用)。判"是否在仓库内"靠
/// `repo_root().is_some()`,再取当前分支与本地分支表。
#[derive(Debug, Clone, PartialEq)]
pub struct GitInfo {
    pub is_repo: bool,
    pub current_branch: Option<String>,
    pub current_branch_has_commits: bool,
    pub branches: Vec<String>,
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
    /// 右键菜单"搜索":内核拦截,不进 `update`——由内核映射成
    /// `search::Message::SearchOpen` 打开文件树右键作用域的搜索弹窗。
    OpenSearch(PathBuf, bool),
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
    /// 项目树行内编辑框草稿变化(iced `text_input::on_input`,每次给全量
    /// 当前字符串)。
    EditInput(String),
    /// 回车 / 失焦(由 `set_tree_edit_focused` 的边缘触发,不经过消息):
    /// 提交改名/新建。
    EditSubmit,
    /// 行内编辑框 / 搜索框被右键:内核拦截,不进 `update`——转发成顶层
    /// `Message::TextInputMenuOpen` 弹出通用输入框右键菜单(见 app.rs)。
    TextInputMenuOpen(crate::app::TextInputTarget),
    /// 搜索框草稿变化(iced `text_input::on_input`,每次按键给全量当前
    /// 字符串,不是逐字符追加)。只进草稿,不触发过滤——同现状,过滤词由
    /// `SearchSubmit` 落定。
    SearchInput(String),
    /// 提交搜索:把草稿 `tree_search` 落成生效的过滤 `search_query`。
    SearchSubmit,
    /// 切换"显示/隐藏以 `.` 开头的文件/目录"(搜索框后的眼睛按钮)。翻转
    /// 后调用 `file_tree.set_show_dotfiles` 重读已缓存目录,让树立刻反映。
    ToggleDotfiles,
    /// 异步加载 git 仓库信息的结果:落 `git_is_repo`/`current_branch`/
    /// `git_branches` 并置 `git_loaded`。
    GitInfoLoaded(i64, GitInfo),
    /// 展开/收起底部分支切换弹层。
    BranchPickerOpen,
    BranchPickerClose,
    /// 用户从分支列表选中 `name`:异步 `git checkout` 切换,结果回传
    /// `BranchSwitchDone`。
    BranchSwitch(String),
    /// 切换分支的异步结果;成功后仓库的 HEAD/refs 变化会被 git_watch 拾起、
    /// 触发 git 状态刷新,文件树随之更新。
    BranchSwitchDone(i64, Result<(), String>),
    /// 项目无 git 仓库时点"新建仓库":异步 `git init` 创建,结果回传
    /// `GitInitDone`。
    GitInit,
    GitInitDone(i64, Result<(), String>),
    /// 文件树工具行(搜索按钮/点文件按钮/底部分支切换按钮)的 hover 进入/
    /// 离开。文件树面板只有 `WorkspaceState`,不挂内核 `App` 的 hover 动画
    /// 表,`view` 通过传入的 `hover_t` 参数取动画进度,进入/离开则以本消息
    /// 上报给内核(`Message::Files` 分支里转发到 `HoverId`)。
    ToolbarHover(FilesToolbarTarget, bool),
    /// 文件树滚动偏移上报(逻辑像素 Y):树自身 `on_scroll` 发出,写进
    /// `tree_scroll` 供外部拖拽命中测试用(渲染仍由 Scrollable 内部状态驱动)。
    TreeScroll(f32),
    /// 外部文件被拖拽悬停在文件树上:`dirs` 是命中的目标目录集合(可为空)。
    /// 由 main.rs 在原生 `HoveredFile`/`CursorMoved` 事件层命中测试后发出,
    /// 用于把那些目录行高亮成"拖入落点"(整行高亮),见 `view()`。
    FileDragHover(HashSet<PathBuf>),
    /// 外部文件被松开(落下)在某个目录上:`paths` 是本次拖入的文件/目录
    /// 完整路径,`target` 是落点目录。由 main.rs 在原生 `DroppedFile` 事件层
    /// 命中测试后发出,异步移动(见 `Message::FileDrop` 处理器)。
    FileDrop {
        paths: Vec<PathBuf>,
        target: PathBuf,
    },
    /// 一次拖入的异步移动结果:成功时刷新 `target` 目录,失败时置
    /// `tree_error`。
    FileDropDone(i64, PathBuf, Result<(), String>),
    /// 树内行被按下:武装拖拽,同时保留原有点击语义(展开/折叠目录,或打开
    /// 文件——文件要跨到 `Message::PreviewOpenPath`,该面板本身不渲染预览)。
    /// 这条跨内核边界:由 `App::update` 拦截转发,`files::update()` 收到会
    /// `unreachable!`。
    TreeRowPress {
        path: PathBuf,
        is_dir: bool,
    },
    /// 树内拖拽悬停到某个目录行(项目内部移动专用,与外部 OS 拖拽的
    /// `FileDragHover` 分开走——内部拖拽还要校验落点合法性,见
    /// `is_valid_move_target`,非法落点不高亮、不记为待定目标)。
    TreeDragOver(PathBuf),
    /// 树内拖拽松开左键:main.rs 全局 `MouseInput::Released` 在
    /// `App::dragging_tree_item()` 为真时发出。有合法待定目标就提交移动
    /// (复用 `FileDrop` 的 `move_item` + `FileDropDone` 收尾),否则原地
    /// 清空,不做任何文件系统操作。
    TreeDragEnd,
}

/// 文件树工具行里带 hover 动画的 icon 按钮。与内核 `HoverId` 一一对应
/// (`HoverId::FilesSearchSubmit` / `FilesDotfiles` / `FilesBranchSwitch`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilesToolbarTarget {
    SearchSubmit,
    Dotfiles,
    BranchSwitch,
}

/// 搜索框稳定的 iced widget id:`view()` 里 `.id()` 挂给真正的
/// `text_input`,`CaptureSearchFocus` 每帧靠它在 widget 树里认出这一个
/// (同 `extensions::todo::add_field_id` 的既有手法——`Id::new` 而非
/// `Id::unique()`,保证同一个字符串在两处各自构造出的 `Id` 相等)。
pub fn search_field_id() -> Id {
    Id::new("files-search-box")
}

static SEARCH_FOCUSED: std::sync::LazyLock<std::sync::Mutex<bool>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(false));

/// 读走并复位(消费式)搜索框上一帧是否持有 iced 内部真实焦点。`main.rs`
/// 渲染循环每帧跑完 `CaptureSearchFocus` 后立刻调用本函数,把结果塞进当前
/// `Workspace`(`set_search_focused`)——`static` 只是临时桥接,不是长期
/// 状态存放处(同 `extensions::todo::take_add_field_bounds` 的既有手法)。
/// **必须消费式复位为 `false`**:搜索框不可见时(如收起 Files 面板但
/// `left_view` 未变)本帧 `CaptureSearchFocus` 找不到匹配 id、不会覆盖,
/// 若读了不清就会让上一次的 `true` 一直卡住,永久堵死终端键盘转发
/// (main.rs 键盘路由的 OR 链——`files_search_focused()` 卡真则任何按键都
/// 出不去、送不进 PTY)。消费后下一帧只要搜索框真被找到,`focusable()` 会
/// 立刻拿真值覆盖回来,不会丢真实焦点态。
pub fn take_search_focused() -> bool {
    std::mem::replace(&mut *SEARCH_FOCUSED.lock().unwrap(), false)
}

/// 每帧 `interface.operate()` 跑一遍,把 `search_field_id()` 命中的
/// `text_input` 当前是否持有 iced 焦点写进 `SEARCH_FOCUSED`。`traverse`
/// 必须调用传入的 `operate` 闭包才能继续递归子节点——`Row`/`Column` 等容器的
/// `operate()` 实现把子节点遍历整个包在 `operation.traverse(&mut |op| {..})`
/// 里(iced_widget 0.14.2 `row.rs`/`column.rs`),不调用闭包会导致容器直接
/// 跳过全部子节点,搜索框嵌在 `row!`/`column!` 里永远遍历不到、
/// `focusable()` 永远不会被调用。
pub struct CaptureSearchFocus;
impl Operation<()> for CaptureSearchFocus {
    fn focusable(&mut self, id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Focusable) {
        if id == Some(&search_field_id()) {
            *SEARCH_FOCUSED.lock().unwrap() = state.is_focused();
        }
    }

    fn traverse(&mut self, operate: &mut dyn for<'a> FnMut(&'a mut (dyn Operation<()> + 'a))) {
        operate(self);
    }
}

/// 项目树行内编辑框稳定的 iced widget id。同一时刻 `tree_edit` 只可能是
/// `Some` 一份(新建/重命名互斥,不会有两个编辑框同时存在),固定 id 够用,
/// 不需要按行号/路径动态生成。
pub fn tree_edit_field_id() -> Id {
    Id::new("files-tree-edit-box")
}

static TREE_EDIT_FOCUSED: std::sync::LazyLock<std::sync::Mutex<bool>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(false));

/// 读走并复位(消费式)项目树编辑框上一帧是否持有 iced 内部真实焦点,同
/// `take_search_focused` 的桥接手法——消费式复位的理由同上(避免编辑框
/// 不可见的帧里卡死上一次的 `true`,永久堵死终端键盘转发)。
pub fn take_tree_edit_focused() -> bool {
    std::mem::replace(&mut *TREE_EDIT_FOCUSED.lock().unwrap(), false)
}

/// 每帧 `interface.operate()` 跑一遍,把 `tree_edit_field_id()` 命中的
/// `text_input` 当前是否持有 iced 焦点写进 `TREE_EDIT_FOCUSED`。`traverse`
/// 必须调用传入的 `operate` 闭包才能继续递归子节点(见 `CaptureSearchFocus`
/// 的文档)。
pub struct CaptureTreeEditFocus;
impl Operation<()> for CaptureTreeEditFocus {
    fn focusable(&mut self, id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Focusable) {
        if id == Some(&tree_edit_field_id()) {
            *TREE_EDIT_FOCUSED.lock().unwrap() = state.is_focused();
        }
    }

    fn traverse(&mut self, operate: &mut dyn for<'a> FnMut(&'a mut (dyn Operation<()> + 'a))) {
        operate(self);
    }
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
        self.search_focused = false;
    }

    /// 供内核判断"删除确认浮层该不该显示"(`App::view()` 顶层互斥浮层
    /// 判断链用,见设计文档 §5)。
    pub fn tree_delete_confirm_is_some(&self) -> bool {
        self.tree_delete_confirm.is_some()
    }

    /// 项目树行内编辑框是否持有 iced 真实焦点(main.rs 键盘路由用)。
    /// **不是**应用层手动置位的镜像——每帧渲染循环里 `CaptureTreeEditFocus`
    /// 问一遍 iced 真相后立刻写进这里(`set_tree_edit_focused`)。
    pub fn tree_edit_focused(&self) -> bool {
        self.tree_edit_focused_flag
    }

    /// 每帧渲染循环读走 `CaptureTreeEditFocus` 查到的真实焦点态后写进来。
    /// 这里只更新焦点标志位;焦点从真变假(失焦)时该不该保存由内核
    /// `App::set_tree_edit_focused` 在边缘处直接调 `submit_tree_edit` 落盘
    /// (同 `App::set_project_name_focused` 的既有手法,与回车提交共用同一份
    /// 逻辑,不再像旧 `cancel_tree_edit` 那样失焦即丢)。
    pub fn set_tree_edit_focused(&mut self, focused: bool) {
        self.tree_edit_focused_flag = focused;
    }

    /// 行内编辑框当前是否展开(`App::set_tree_edit_focused` 的边缘判断用:
    /// 只在有编辑在页时失焦才提交,避免空转)。
    pub(crate) fn tree_edit_is_some(&self) -> bool {
        self.tree_edit.is_some()
    }

    /// 读走(消费式)一次性聚焦标记。main.rs 在 `UserInterface::build`
    /// 之前调用(此时还能自由 `&mut app`),同 `todo::take_scroll_to_top`
    /// 的既有调用时机。
    pub fn take_tree_edit_focus_pending(&mut self) -> bool {
        std::mem::take(&mut self.tree_edit_focus_pending)
    }

    /// 搜索框草稿、生效词保留,退出后仍作为盒子里的已输入文本继续显示。
    /// 每帧渲染循环读走 `CaptureSearchFocus` 查到的真实焦点态后写进来。
    pub fn set_search_focused(&mut self, focused: bool) {
        self.search_focused = focused;
    }

    /// 供内核 `Workspace::files_search_focused`(main.rs 键盘路由用)判断
    /// 搜索框是否持有 iced 真实焦点。
    pub fn search_focused(&self) -> bool {
        self.search_focused
    }

    /// 分支切换弹层是否展开(`App::view()` 顶层互斥浮层判断链用)。
    pub fn branch_picker_is_open(&self) -> bool {
        self.branch_picker_open
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

    /// 当前滚动偏移(逻辑像素,内容 Y 向下偏移)。外部拖拽命中测试用——
    /// `Main` 层拿不到 iced 布局,只能靠它与树视口矩形把窗口 Y 换算成可见
    /// 行序号。**不是**渲染驱动的真相(`Scrollable` 内部状态才是),是
    /// `TreeScroll` 消息实时上报的镜像。
    pub fn tree_scroll(&self) -> f32 {
        self.tree_scroll
    }

    /// 当前文件树**全体可见行**(按展开状态折叠后的顺序,与
    /// `files::view` 渲染逐行一一对应)。外部拖拽命中测试 `App::files_drop_target`
    /// 用它 + `tree_scroll` 命中目录行;滚动只改视口不改这份可见行集合。
    pub fn visible_tree_rows(&self) -> Vec<crate::project::TreeRow> {
        self.file_tree
            .as_ref()
            .map(|t| t.visible_rows())
            .unwrap_or_default()
    }

    /// 外部文件系统变化(agent/其他进程改了盘)后,让文件树从磁盘重读已缓存
    /// 目录,即时反映新增/删除/改名(现有 `FileTree::reload_from_disk`:只
    /// 重读已缓存过的目录、不清 `expanded`,当前展开层级保持不变)。由
    /// `App::project_fs_changed` 在 `git_watch` 上报的工作区变更里调用。
    pub fn reload_tree_from_disk(&mut self) {
        if let Some(tree) = &mut self.file_tree {
            tree.reload_from_disk();
        }
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
        self.tree_edit_focus_pending = true;
    }

    /// 行内编辑框提交:回车(`Message::EditSubmit`)与失焦(`App::
    /// set_tree_edit_focused` 的边缘触发)两条路径共用,避免两份重复的重名/
    /// 路径分量校验与落盘调用。逻辑是现有 `Workspace::submit_tree_edit` 的
    /// 搬家版本(`io: &ShellIo` 换成 `handle`/`emit`,`self.project_id()` 换成
    /// 显式传入的 `project_id`),三模式的重名校验/路径分量校验原样照抄。
    pub(crate) fn submit_tree_edit(
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

impl WorkspaceState {
    /// 树内行被按下:武装拖拽(见 `Message::TreeRowPress` 文档)。该消息本身
    /// 由内核拦截(还要跨到 `PreviewOpenPath`),不进 `files::update()`,故
    /// 内核直接调用这个方法而不是发一条 `files::Message`。
    pub(crate) fn arm_tree_drag(
        &mut self,
        source: PathBuf,
        source_is_dir: bool,
        press_pos: (f32, f32),
    ) {
        self.tree_drag = Some(TreeDrag {
            source,
            source_is_dir,
            target: None,
            press_pos,
        });
    }

    /// 树内拖拽是否正在进行(供 `App::dragging_tree_item()` 判断全局左键
    /// 松开是否该收尾这场拖拽——同 `dragging_rail`/`dragging_tab` 的用法)。
    pub(crate) fn is_dragging_tree_item(&self) -> bool {
        self.tree_drag.is_some()
    }

    /// 当前这场树内拖拽按下瞬间的光标位置(`None` = 没有拖拽在进行)——供
    /// 内核在 `TreeDragOver` 到达时和当前光标比对,判断是否已越过阈值、
    /// 真正算作一次拖拽,见 `TreeDrag::press_pos` 文档。
    pub(crate) fn tree_drag_press_pos(&self) -> Option<(f32, f32)> {
        self.tree_drag.as_ref().map(|d| d.press_pos)
    }

    /// `TreeDragEnd` 收尾前问一句:这一按其实只是单击一个文件(没有产生
    /// 合法拖拽目标),该不该由内核补发 `PreviewOpenPath`——`TreeRowPress`
    /// 不再立即打开文件就是为了避免预览 webview 在拖拽途中冒出来挡住画面
    /// (见其文档)。目录不需要这个:它的点击语义(展开/折叠)按下那一刻
    /// 已经立即执行。**必须在 `files::update()` 消费 `tree_drag` 之前调用**
    /// ——那之后状态已被 `TreeDragEnd` 的处理器 `take()` 走。
    pub(crate) fn pending_click_open_file(&self) -> Option<PathBuf> {
        let drag = self.tree_drag.as_ref()?;
        (!drag.source_is_dir && drag.target.is_none()).then(|| drag.source.clone())
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
        Message::StatusesRefreshed(project_id, statuses) => {
            ws_state.git_statuses = statuses;
            // 项目 git 状态更新(git_watch 拾起 HEAD/refs 变化后)常伴随分支
            // 切换,顺手把分支栏的仓库信息一并刷新,让底栏与树保持一致。
            spawn_git_info_load(ws_state, project_id, handle, emit);
        }
        Message::RightClickAt { x, y } => {
            app_state.last_right_click = (x, y);
        }
        Message::ContextMenuOpen { path, is_dir } => {
            let (x, y) = app_state.last_right_click;
            ws_state.tree_selected = Some(path.clone());
            // 与分支切换弹层互斥:开右键菜单时收起分支弹层,避免两个浮层
            // 同时挂着(同 `PreviewTabContextMenu` 关文件树右键菜单的约定)。
            ws_state.branch_picker_open = false;
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
        Message::SearchInput(s) => {
            ws_state.tree_search = s;
        }
        Message::SearchSubmit => {
            ws_state.search_query = ws_state.tree_search.clone();
        }
        Message::ToggleDotfiles => {
            if let Some(tree) = &mut ws_state.file_tree {
                tree.set_show_dotfiles(!tree.dotfiles_shown());
            }
        }
        Message::GitInfoLoaded(_, info) => {
            ws_state.git_loaded = true;
            ws_state.git_is_repo = info.is_repo;
            ws_state.current_branch = info.current_branch;
            ws_state.current_branch_has_commits = info.current_branch_has_commits;
            ws_state.git_branches = info.branches;
        }
        Message::BranchPickerOpen => {
            ws_state.branch_picker_open = true;
            ws_state.git_error = None;
        }
        Message::BranchPickerClose => {
            ws_state.branch_picker_open = false;
        }
        Message::BranchSwitch(name) => {
            ws_state.branch_picker_open = false;
            ws_state.git_error = None;
            let Some(tree) = &ws_state.file_tree else {
                return;
            };
            let root = tree.root().to_path_buf();
            handle.spawn(async move {
                let result = tokio::task::spawn_blocking(move || {
                    crate::delivery::checkout_branch(&root, &name)
                })
                .await
                .unwrap_or_else(|e| Err(e.to_string()));
                emit(Message::BranchSwitchDone(project_id, result));
            });
        }
        Message::BranchSwitchDone(_, result) => {
            match result {
                Ok(()) => {
                    // checkout 成功后 HEAD/refs 变化由 git_watch 拾起,会再次
                    // 触发 StatusesRefreshed → spawn_git_info_load。这里再主动
                    // 刷一次分支信息(脱离 watch 兜底,如未启动 watch 时)。
                    spawn_git_info_load(ws_state, project_id, handle, emit);
                }
                Err(e) => ws_state.git_error = Some(e),
            }
        }
        Message::GitInit => {
            ws_state.git_error = None;
            if let Some(tree) = &ws_state.file_tree {
                let root = tree.root().to_path_buf();
                handle.spawn(async move {
                    let result =
                        tokio::task::spawn_blocking(move || crate::delivery::init_repo(&root))
                            .await
                            .unwrap_or_else(|e| Err(e.to_string()));
                    emit(Message::GitInitDone(project_id, result));
                });
            }
        }
        Message::GitInitDone(_, result) => match result {
            Ok(()) => {
                ws_state.git_loaded = true;
                ws_state.git_is_repo = true;
                // 新建的空仓库没有提交/分支,分支栏仍显示"分支切换"态但当前
                // 分支为空;git status / 文件树颜色等后续由 git_watch 正常驱动。
                spawn_git_info_load(ws_state, project_id, handle, emit);
            }
            Err(e) => ws_state.git_error = Some(e),
        },
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
            ws_state.tree_edit_focus_pending = true;
        }
        Message::EditInput(s) => {
            let Some(edit) = &mut ws_state.tree_edit else {
                return;
            };
            edit.buffer = s;
        }
        Message::EditSubmit => {
            ws_state.submit_tree_edit(project_id, handle, emit);
        }
        Message::TextInputMenuOpen(_) => {
            unreachable!("由内核拦截处理,见 files::Message::TextInputMenuOpen 文档")
        }
        Message::CopyPath(..) => {
            unreachable!("由内核拦截处理,见 files::Message::CopyPath 文档")
        }
        Message::OpenSearch(..) => {
            unreachable!("由内核拦截处理,映射成 search::Message::SearchOpen")
        }
        // 工具行 icon 按钮的 hover 由内核 `Message::Files` 分支转发到
        // `HoverId`(文件树面板不挂 App 的 hover 动画表),`update` 吃不到
        // 这里;保 no-op 分支维持 match 穷尽。
        Message::ToolbarHover(..) => {}
        // 文件树滚动偏移:写进 `tree_scroll` 供外部拖拽命中测试。
        Message::TreeScroll(off) => {
            ws_state.tree_scroll = off;
        }
        // 外部文件拖拽悬停命中目录集合:整行高亮这些目录为"拖入落点"。
        // `view` 读 `drag_hover` 渲染金色高亮边框。
        Message::FileDragHover(dirs) => {
            ws_state.drag_hover = dirs;
        }
        // 外部文件被松开在某目录上:逐个移动(同 Finder 拖拽的 move 语义,
        // 跨文件系统在 `move_item` 里降级为复制+删源)。全部完成后 `emit`
        // `FileDropDone` 刷新目标目录。
        Message::FileDrop { paths, target } => {
            ws_state.tree_error = None;
            ws_state.drag_hover = HashSet::new();
            if paths.is_empty() {
                return;
            }
            // 已经在项目内的文件不许走这条"外部文件拖拽"通路移动路径——核心
            // 裁决要求直接改动项目产物的入口收着给,不能让鼠标拖拽变成一个
            // 可以随手重排项目文件结构的隐藏功能(容易误操作,又绕开 AI 侧
            // 的变更记录)。整批只要有一个源在项目树内就整体拒绝,不做"部分
            // 生效"。
            if let Some(tree) = &ws_state.file_tree
                && let Some(bad) = paths.iter().find(|p| p.starts_with(tree.root()))
            {
                ws_state.tree_error = Some(format!(
                    "{} 已在项目内,不支持用拖拽移动项目内文件",
                    bad.display()
                ));
                return;
            }
            let drop_target = target.clone();
            handle.spawn(async move {
                let result = tokio::task::spawn_blocking(move || {
                    for p in &paths {
                        let is_dir = std::fs::metadata(p).map(|m| m.is_dir()).unwrap_or(false);
                        crate::project::move_item(p, is_dir, &target)?;
                    }
                    Ok::<(), String>(())
                })
                .await
                .unwrap_or_else(|e| Err(e.to_string()));
                emit(Message::FileDropDone(project_id, drop_target, result));
            });
        }
        Message::FileDropDone(_, target, result) => match result {
            Ok(()) => {
                ws_state.tree_error = None;
                if let Some(tree) = &mut ws_state.file_tree {
                    tree.refresh(&target);
                }
            }
            Err(e) => ws_state.tree_error = Some(e),
        },
        Message::TreeRowPress { .. } => {
            unreachable!("由内核拦截处理,见 files::Message::TreeRowPress 文档")
        }
        // 树内拖拽悬停:只在落点合法(非自身/自身子树,见
        // `is_valid_move_target`)时记为待定目标并高亮——复用外部拖拽同一份
        // `drag_hover` 渲染,不用另画一套。悬停到折叠的目录顺带自动展开,
        // 方便继续往深一层拖(同外部 OS 拖拽既有的 `expand_dir_if_collapsed`
        // 行为一致)。
        Message::TreeDragOver(target) => {
            let Some(drag) = &mut ws_state.tree_drag else {
                return;
            };
            if is_valid_move_target(&drag.source, drag.source_is_dir, &target) {
                drag.target = Some(target.clone());
                ws_state.drag_hover = std::iter::once(target.clone()).collect();
                expand_dir_if_collapsed(ws_state, &target);
            } else {
                drag.target = None;
                ws_state.drag_hover = HashSet::new();
            }
        }
        // 树内拖拽松开:有合法待定目标就提交移动,复用 `FileDrop` 那套
        // `move_item` + `spawn_blocking` + `FileDropDone` 收尾逻辑;没有
        // (没悬停到任何目录/悬停到的都是非法落点)就只清状态,不碰文件系统。
        Message::TreeDragEnd => {
            let Some(drag) = ws_state.tree_drag.take() else {
                return;
            };
            ws_state.drag_hover = HashSet::new();
            let Some(target) = drag.target else {
                return;
            };
            let source = drag.source;
            let is_dir = drag.source_is_dir;
            let refresh_target = target.clone();
            handle.spawn(async move {
                let result = tokio::task::spawn_blocking(move || {
                    crate::project::move_item(&source, is_dir, &target)?;
                    Ok::<(), String>(())
                })
                .await
                .unwrap_or_else(|e| Err(e.to_string()));
                emit(Message::FileDropDone(project_id, refresh_target, result));
            });
        }
    }
}

/// 异步加载一次 git 仓库信息:判项目根是否在仓库内、读当前分支、读本地分支
/// 表,结果通过 `emit(GitInfoLoaded(..))` 回投。走 `spawn_blocking` 防止阻塞
/// UI 线程(交付层的 git 调用都是同步阻塞,见 delivery.rs 模块头注释)。
fn spawn_git_info_load(
    ws_state: &WorkspaceState,
    project_id: i64,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    let Some(tree) = &ws_state.file_tree else {
        return;
    };
    let root = tree.root().to_path_buf();
    handle.spawn(async move {
        let info = tokio::task::spawn_blocking(move || {
            use crate::delivery::{branch, current_branch_has_commits, local_branches, repo_root};
            match repo_root(&root) {
                Some(repo) => GitInfo {
                    is_repo: true,
                    current_branch: branch(&repo),
                    current_branch_has_commits: current_branch_has_commits(&repo),
                    branches: local_branches(&repo).unwrap_or_default(),
                },
                None => GitInfo {
                    is_repo: false,
                    current_branch: None,
                    current_branch_has_commits: false,
                    branches: Vec::new(),
                },
            }
        })
        .await
        .unwrap_or_else(|_| GitInfo {
            is_repo: false,
            current_branch: None,
            current_branch_has_commits: false,
            branches: Vec::new(),
        });
        emit(Message::GitInfoLoaded(project_id, info));
    });
}

/// 窗口坐标 (x, y) → 命中的**目录行**路径。外部 OS 文件拖拽的命中测试：
/// main.rs 在原生事件层拿不到 iced 布局，只能靠 `left_files_tree_bounds`
/// 算出的树视口矩形 + `tree_scroll` 偏移 + 行高/行间距，把窗口 Y 换算成
/// 可见行序号，再确认命中行是目录。
///
/// 只返回**目录**（文件不可作落点）；命中视图外 / 非目录行返回 `None`。
/// 行 i 的屏幕上沿 = `bounds.y - scroll + i * (row_h + region.gap)`，行高
/// 与间距必须和渲染侧同源（`tree_row_h()`、`project_pane().gap`）。行间死
/// 区（间距）落在任一相邻目录行之间时按最近目录行吸住。
pub fn tree_drop_target(
    x: f32,
    y: f32,
    bounds: (f32, f32, f32, f32),
    scroll: f32,
    rows: &[TreeRow],
) -> Option<PathBuf> {
    let (bx, by, bw, bh) = bounds;
    if bw <= 0.0 || bh <= 0.0 || !(bx..bx + bw).contains(&x) || !(by..by + bh).contains(&y) {
        return None;
    }
    let row_h = crate::theme::geometry::tree_row_h();
    let gap = crate::theme::region::project_pane().gap;
    let pitch = row_h + gap;
    // 内容坐标(未滚动)下的命中 Y。
    let content_y = (y - by) + scroll;
    // 命中行序号(含行间死区吸附)。
    let idx = (content_y / pitch).floor() as isize;
    let within_row = content_y - idx as f32 * pitch <= row_h;
    let idx = if within_row { idx } else { idx + 1 };
    if idx >= rows.len() as isize || idx < 0 {
        return None;
    }
    let row = &rows[idx as usize];
    if row.is_dir {
        Some(row.path.clone())
    } else {
        None
    }
}

/// 树内拖拽合法落点校验:只拒绝"移到自己"、"目录移进自己子树"(会产生
/// 环/孤儿)。`starts_with` 走路径分量比较(不是字符串前缀),`/proj/ab`
/// 不会被误判成 `/proj/a` 的子路径。
///
/// 拖到"当前所在的父目录"(true no-op)**不**在这里拒绝——2026-09 用户
/// 实测反馈"无法拖到父目录":这条曾经的"帮用户挡掉无意义操作"的好心
/// 拒绝,实际效果只是悬停时不高亮、用户以为拖拽坏了。真无意义时
/// `crate::project::move_item` 自己会安全兜底(目标路径与源相同:文件走
/// "已存在同名项"报错,目录走"不能移到它自己"报错,`std::fs::rename`
/// 都不会被调用),不需要在这一层提前拦。
pub(super) fn is_valid_move_target(
    source: &std::path::Path,
    source_is_dir: bool,
    target: &std::path::Path,
) -> bool {
    if target == source {
        return false;
    }
    if source_is_dir && target.starts_with(source) {
        return false;
    }
    true
}

/// 树内拖拽悬停到折叠的目录时自动展开,方便继续往深一层拖——同外部 OS
/// 拖拽既有的 `App::expand_files_dir_if_collapsed` 行为一致,只是这里操作
/// 对象是 `WorkspaceState` 本身(`files::update()` 已经有 `&mut
/// WorkspaceState`,不需要跨到内核)。已展开则不重复调用 `toggle`(避免
/// 误触发折叠)。
fn expand_dir_if_collapsed(ws_state: &mut WorkspaceState, dir: &std::path::Path) {
    let already_expanded = ws_state
        .visible_tree_rows()
        .iter()
        .any(|r| r.path == dir && r.expanded);
    if !already_expanded && let Some(tree) = &mut ws_state.file_tree {
        tree.toggle(dir);
    }
}

/// 文件树可滚动列表(现有 `workspace.rs::project_pane` 的搬家版本,签名改吃
/// 本模块状态,去掉了不再归属本模块的项目信息卡/底部状态条)。
pub fn view<'a>(
    ws_state: &'a WorkspaceState,
    width: Length,
    outer: Border,
    search_hover_t: f32,
    dotfiles_hover_t: f32,
    branch_hover_t: f32,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let region = theme::region::project_pane();
    let mut header = column![].spacing(region.gap).width(Length::Fill);
    let mut tree_col = column![].spacing(region.gap);

    // 文件树搜索框:按文件/目录名称筛选整棵树(大小写不敏感子串匹配)。
    // 不会边输入边过滤——敲回车/点右侧"搜索"按钮后,由 `SearchSubmit` 把
    // 草稿落成为生效的 `search_query`。Stage 2 迁移成真正的
    // `iced_widget::text_input`:鼠标点击聚焦、方向键/选区/IME 全部走 iced
    // 标准管线自己处理,`main.rs` 只需要每帧问一遍它是否持有真实焦点
    // (`files_search_focused`)决定要不要把键盘事件放行,不再需要点击盒子
    // 手动进入自绘编辑态。样式收敛到 `byteui::form::search_box`(需求:所有
    // 面板搜索框统一成首页项目列表搜索框那一套),不再是各画一套的
    // `input_text` + 独立图标按钮;`highlight`(内部叫 `search_active`)
    // 传真实聚焦态或已生效搜索词非空,即使当前没聚焦,只要树被搜索词
    // 过滤中就持续金框提示。
    let search_active = ws_state.search_focused() || !ws_state.search_query.is_empty();
    let box_len = byteui::theme::icon_size::row() + 12.0;
    let search_box = byteui::form::search_box::view(
        "搜索目录…",
        &ws_state.tree_search,
        Some(search_field_id()),
        search_active,
        Message::SearchInput,
        Message::SearchSubmit,
        search_hover_t,
        |hovered| Message::ToolbarHover(FilesToolbarTarget::SearchSubmit, hovered),
    );
    let search_box = byteui::interaction::context_menu::wrap(
        search_box,
        Some(Message::TextInputMenuOpen(crate::app::TextInputTarget {
            id: search_field_id(),
            secure: false,
        })),
    );

    // "显示/隐藏点文件"按钮:切换后 `ToggleDotfiles` 调
    // `set_show_dotfiles` 重读树。图标反映当前口径——正显示(`eye`)时点它
    // 隐藏点文件;隐藏(`eye-off`)时点它恢复显示。切换只换图标,不套任何
    // "选中生效"的视觉信号(无 GOLD 边框/无点亮图标),保持按钮常驻常态外观。
    let dotfiles_shown = ws_state
        .file_tree
        .as_ref()
        .map(|t| t.dotfiles_shown())
        .unwrap_or(true);
    let dotfiles_button = icons::icon_button_entry(
        if dotfiles_shown {
            icons::IconKind::Eye
        } else {
            icons::IconKind::EyeOff
        },
        byteui::theme::icon_size::row(),
        false,
        false,
        dotfiles_hover_t,
        true,
        box_len,
        true,
        Message::ToggleDotfiles,
        |hovered| Message::ToolbarHover(FilesToolbarTarget::Dotfiles, hovered),
        "切换点文件",
    );
    header = header.push(
        row![search_box, dotfiles_button]
            .spacing(6)
            .align_y(iced_widget::core::Alignment::Center)
            .padding([6, 0]),
    );

    // 根目录头部:只显示名称(CREAM 高亮),不再直接显示完整路径;名称前
    // 挂 folder-open-dot 图标(lucide 的展开文件夹 + 圆点,有别于普通展开目录
    // 的 folder-open,特标项目根)。与上方工具行的间距由搜索行的底部 padding
    // 承担,这里不再额外加顶边距。右键根目录打开目录右键菜单(新建文件/文件夹、
    // 复制、粘贴、删除、重命名、在 Finder 打开、从磁盘重新加载…),坐标复用
    // `main.rs` 右键时写入的 `last_right_click`。
    if let Some(tree) = &ws_state.file_tree {
        let root = tree.root();
        let name = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| root.display().to_string());
        // 根目录名称颜色跟着 git 状态走(与树行同款 `tree_state_color`),
        // 图标恒为灰(`DIM`),不再用 CREAM 高亮。
        let root_state = delivery::dir_status(root, &ws_state.git_statuses)
            .unwrap_or(delivery::TreeState::Unchanged);
        let root_color = tree_state_color(root_state);
        // 根目录本身也是合法的拖拽落点(项目内移动到顶层),但它不在
        // `visible_tree_rows()` 循环里(单独渲成静态头部,见上方注释),
        // 得在这里单独补上同一套"命中即高亮 + 悬停上报"逻辑,否则永远拖不
        // 到根目录(2026-09 用户实测反馈)。
        let root_is_drop_target = ws_state.drag_hover.contains(root);
        let root_header = container(
            row![
                icons::view(
                    icons::IconKind::FolderOpenDot,
                    byteui::theme::icon_size::row(),
                    byteui::theme::color::current().dim
                ),
                text(name)
                    .size(byteui::theme::font::body())
                    .color(root_color),
            ]
            .spacing(6)
            .align_y(iced_widget::core::Alignment::Center),
        )
        .width(Length::Fill)
        .padding([0, 0])
        .style(move |_t: &iced_widget::Theme| container::Style {
            border: if root_is_drop_target {
                Border {
                    color: byteui::theme::color::current().gold,
                    width: 1.0,
                    radius: 6.0.into(),
                }
            } else {
                Border {
                    color: Color::TRANSPARENT,
                    width: 0.0,
                    radius: 0.0.into(),
                }
            },
            ..container::Style::default()
        });
        let mut root_area = MouseArea::new(root_header).on_right_press(Message::ContextMenuOpen {
            path: root.to_path_buf(),
            is_dir: true,
        });
        if ws_state.tree_drag.is_some() {
            let root_target = root.to_path_buf();
            root_area = root_area.on_move(move |_| Message::TreeDragOver(root_target.clone()));
        }
        header = header.push(root_area);
    }

    if let Some(err) = &ws_state.tree_error {
        header = header.push(
            text(format!("⚠ {err}"))
                .size(byteui::theme::font::label())
                .color(byteui::theme::color::current().red),
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
                            byteui::theme::icon_size::chevron(),
                            byteui::theme::color::current().dim
                        ),
                        icons::view(
                            folder,
                            byteui::theme::icon_size::row(),
                            byteui::theme::color::current().dim
                        ),
                    ]
                    .spacing(byteui::theme::icon_size::tree_row_gap())
                    .align_y(iced_widget::core::Alignment::Center)
                    .into()
                } else {
                    row![
                        iced_widget::space::Space::new()
                            .width(Length::Fixed(
                                byteui::theme::icon_size::chevron()
                                    + byteui::theme::icon_size::tree_row_gap(),
                            ))
                            .height(Length::Shrink),
                        icons::view(
                            icons::icon_for_file(&row.name),
                            byteui::theme::icon_size::row(),
                            byteui::theme::color::current().dim
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
            let msg = Message::TreeRowPress {
                path: row.path.clone(),
                is_dir: row.is_dir,
            };
            let is_selected = ws_state.tree_selected.as_deref() == Some(row.path.as_path());
            // 外部文件拖拽落点:目录被命中 → 整行金色描边高亮(仅目录可作落点)。
            let is_drop_target = row.is_dir && ws_state.drag_hover.contains(&row.path);
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
                        Some(byteui::theme::color::current().card.into())
                    } else {
                        None
                    },
                    text_color: byteui::theme::color::current().body,
                    border: if is_drop_target {
                        Border {
                            color: byteui::theme::color::current().gold,
                            width: 1.0,
                            radius: 6.0.into(),
                        }
                    } else {
                        Border {
                            color: Color::TRANSPARENT,
                            width: 0.0,
                            radius: 0.0.into(),
                        }
                    },
                    ..button::Style::default()
                });
            let mut row_area = MouseArea::new(row_btn).on_right_press(Message::ContextMenuOpen {
                path: row.path.clone(),
                is_dir: row.is_dir,
            });
            // 树内拖拽进行中且这一行是目录:光标划过即上报为悬停命中,驱动
            // `TreeDragOver` 校验落点合法性并刷新 `drag_hover` 高亮(同外部
            // OS 拖拽复用的那一圈金色描边)。非目录行/没有拖拽在进行时不挂
            // `on_move`,避免无意义消息。
            if row.is_dir && ws_state.tree_drag.is_some() {
                let drag_target = row.path.clone();
                row_area = row_area.on_move(move |_| Message::TreeDragOver(drag_target.clone()));
            }
            tree_col = tree_col.push(row_area);
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
        // 项目根目录不出现在 `visible_rows()` 里(它只渲染成上方静态头部),
        // 所以上面循环里的 `is_new_target` 永远匹配不到 root。这里单独补一段:
        // 当选中根目录作为新建父目录时,在根头部下方、按子项深度渲染编辑框,
        // 否则在根目录右键"新建文件/文件夹"会"点了菜单却没有任何输入框"。
        let root_new_edit = matches!(
            &ws_state.tree_edit,
            Some(TreeEdit {
                mode: TreeEditMode::NewFile | TreeEditMode::NewFolder,
                parent_dir,
                ..
            }) if parent_dir.as_path() == tree.root()
        );
        if root_new_edit {
            let buffer = ws_state
                .tree_edit
                .as_ref()
                .map(|e| e.buffer.as_str())
                .unwrap_or("");
            tree_col = tree_col.push(tree_edit_row(1, buffer));
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
                    byteui::interaction::scrollbar::scrollbar()
                ))
                .on_scroll(|viewport| { Message::TreeScroll(viewport.absolute_offset().y) })
                .style(|_t, _s| byteui::interaction::scrollbar::scrollbar_style()),
            git_footer_bar(ws_state, branch_hover_t),
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

/// 行内编辑框(新建/重命名共用):真正的 iced `text_input`(`bare: false` 由
/// `byteui::form::input_text` 自己画卡片背景 + 聚焦金框描边)。缩进不再用
/// 等宽空格字符模拟,**改用外层容器真正的左内边距**——旧版把缩进拼进文本
/// 内容,新版用 `Padding::left` 让编辑框整体右移,视觉跟树层级绑定对齐。
fn tree_edit_row(
    depth: usize,
    buffer: &str,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    // 每级缩进逻辑像素:原自绘版每级用两个全角空格字符,换算成像素 =
    // `tree_row_font_size() * 0.6`(ASCII 字符宽经验值,同
    // `extensions::todo::cursor_from_x` 的换算口径)* 2(原来每级两个空格)。
    // 数字来源见 Stage 5 计划 Task 1 Step 8 的说明,不是随手拍脑袋的魔法值。
    let indent_px = depth as f32 * crate::workspace::tree_row_font_size() * 0.6 * 2.0;
    let field: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
        container(byteui::form::input_text::view(
            "",
            buffer,
            false,
            Some(tree_edit_field_id()),
            false,
            Some(Message::EditSubmit),
            false,
            Message::EditInput,
        ))
        .width(Length::Fill)
        .padding(Padding {
            left: indent_px,
            ..Padding::default()
        })
        .into();
    byteui::interaction::context_menu::wrap(
        field,
        Some(Message::TextInputMenuOpen(crate::app::TextInputTarget {
            id: tree_edit_field_id(),
            secure: false,
        })),
    )
}

/// 文件树底部 git 栏三元组(图标 + 文案元素 + 可选操作按钮)。
type GitFooterTriple<'a> = (
    icons::IconKind,
    Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>,
    Option<Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>>,
);

/// 文件树底部 git 栏:项目在仓库内显示
/// `folder-git-2 当前分支名 〔切换按钮〕`;项目无 git 仓库显示
/// `folder-minus 未受Git保护 〔新建Git仓库〕`;仓库信息尚未加载显示中性
/// 占位。最左图标与文字之间、右缘切换/新建按钮始终可见;整栏无底色、
/// 顶部一条 BORDER 分隔线,与上方滚动树区隔。
fn git_footer_bar(
    ws_state: &WorkspaceState,
    branch_hover_t: f32,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let box_len = byteui::theme::icon_size::row() + 12.0;
    let (icon, label, action): GitFooterTriple<'_> = if !ws_state.git_loaded {
        (
            icons::IconKind::GitBranch,
            text("加载仓库信息…")
                .size(byteui::theme::font::label())
                .color(byteui::theme::color::current().cream)
                .into(),
            None,
        )
    } else if ws_state.git_is_repo {
        // 有仓库:当前分支名(detached/无提交时 None → "无分支"),右侧切换按钮。
        // 若工作区有未提交改动,分支名以对应 git 状态色高亮(色即提示;
        // "(Uncommitted)" 文案只在展开的分支下拉菜单里对当前分支追加)。
        let root = ws_state.file_tree.as_ref().map(|t| t.root().to_path_buf());
        let dirty_state = root
            .as_deref()
            .and_then(|r| delivery::dir_status(r, &ws_state.git_statuses))
            // `dir_status` 聚合时忽略被忽略文件,`Some` 即真实未提交改动。
            .filter(|st| *st != delivery::TreeState::Ignored);
        let branch_name = ws_state
            .current_branch
            .clone()
            .unwrap_or_else(|| "无分支".to_string());
        let label_color = if let Some(st) = dirty_state {
            tree_state_color(st)
        } else {
            byteui::theme::color::current().cream
        };
        let switch = icons::icon_button_entry(
            if ws_state.branch_picker_open {
                icons::IconKind::ChevronUp
            } else {
                icons::IconKind::ChevronDown
            },
            byteui::theme::icon_size::row(),
            false,
            false,
            branch_hover_t,
            false,
            box_len,
            true,
            Message::BranchPickerOpen,
            |hovered| Message::ToolbarHover(FilesToolbarTarget::BranchSwitch, hovered),
            "切换分支",
        );
        (
            icons::IconKind::FolderGit2,
            text(branch_name)
                .size(byteui::theme::font::label())
                .color(label_color)
                .into(),
            Some(switch),
        )
    } else {
        // 无 git 仓库:提示未受 git 保护 + 新建仓库按钮。
        let init = button(
            row![
                icons::view(
                    icons::IconKind::FolderMinus,
                    byteui::theme::icon_size::row(),
                    byteui::theme::color::current().cream
                ),
                text("新建Git仓库")
                    .size(byteui::theme::font::label())
                    .color(byteui::theme::color::current().cream),
            ]
            .spacing(6)
            .align_y(iced_widget::core::Alignment::Center),
        )
        .on_press(Message::GitInit)
        .padding([4, 8])
        .style(|_t: &iced_widget::Theme, _s| button::Style {
            background: Some(byteui::theme::color::current().bg.into()),
            border: Border {
                color: byteui::theme::color::current().border,
                width: 1.0,
                radius: 4.0.into(),
            },
            text_color: byteui::theme::color::current().cream,
            ..button::Style::default()
        });
        (
            icons::IconKind::FolderMinus,
            text("未受Git保护")
                .size(byteui::theme::font::label())
                .color(byteui::theme::color::current().cream)
                .into(),
            Some(init.into()),
        )
    };

    let bar = row![
        icons::view(
            icon,
            byteui::theme::icon_size::row(),
            byteui::theme::color::current().cream
        ),
        label,
        iced_widget::space::horizontal(),
        if let Some(btn) = action {
            btn
        } else {
            iced_widget::space::Space::new()
                .height(Length::Fixed(byteui::theme::icon_size::row() + 8.0))
                .into()
        },
    ]
    .spacing(6)
    .align_y(iced_widget::core::Alignment::Center);

    let top_line = container(iced_widget::space::Space::new())
        .width(Length::Fill)
        .height(Length::Fixed(1.0))
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(byteui::theme::color::current().border.into()),
            ..container::Style::default()
        });

    let mut content = column![top_line, bar].spacing(4);
    if let Some(err) = &ws_state.git_error {
        content = content.push(
            text(format!("⚠ {err}"))
                .size(byteui::theme::font::label())
                .color(byteui::theme::color::current().red),
        );
    }

    container(content)
        .width(Length::Fill)
        .padding([6, 0])
        .style(|_t: &iced_widget::Theme| container::Style {
            background: None,
            ..container::Style::default()
        })
        .into()
}

/// 分支切换弹层（窗口级浮层）:底栏"切换按钮"按下(`branch_picker_open`)时在
/// git 底栏上方弹出全部本地分支(当前分支高亮),点某行即 `BranchSwitch(name)`
/// 切换并收起。**以 window-wide overlay 渲染**(`App::view` 的 `stack!` 里,
/// 下层垫一块透明 `MouseArea` 承接"点别处收起")——所以返回的是**占满全窗的
/// 填充容器**,靠 `Padding{bottom, left}` 把下拉框钉到 git 底栏正上方;这与
/// `context_menu_popup` 用 `Padding{top,left}` 手算像素定位是同一套约定。非
/// git/未加载/未展开时返回空(零高度元素)。
///
/// 宽度注意:与右键菜单同款——每行按钮用 `Length::Fixed(menu_item_width())`
/// 固定宽,列容器保持 `Length::Shrink`,于是整个菜单总宽恒定、不会随分支名
/// 长短自动收缩(短分支名时下拉框保持同一宽度)。不能在 Shrink 容器里给按钮
/// `Length::Fill`,否则 Fill 子在无确定宽的 Shrink 轴上会折叠成 0 宽,整个
/// 菜单就消失;`align_y(End)`(配合外层 `Padding`)负责把菜单压在 git 底栏
/// 正上方、并把下沉量交给动画起点,不会让它跑到窗口顶部。
///
/// 视觉与右键菜单(`context_menu_popup` 的 `menu_item`)对齐:同一套
/// `context_menu` 区域底色/描边/内外边距、`TAB_HOVER` hover 底。
/// 当前分支带未提交改动(dirty)时,除当前分支外的其余分支全部置灰且
/// 不可点——dirty 下切分支会被 git 拒绝(checkout 报错),提前禁用避免
/// 触发错误;同时给当前分支行追加 "(Uncommitted)" 提示。
pub fn branch_picker_popup(
    ws_state: &WorkspaceState,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    if !ws_state.git_loaded || !ws_state.git_is_repo || !ws_state.branch_picker_open {
        return iced_widget::space::Space::new().into();
    }
    let current = ws_state.current_branch.as_deref();
    // 当前分支是否带未提交改动(dirty)?是则锁定其余分支(禁用切换)并给
    // 当前分支行追加 "(Uncommitted)"。
    let is_dirty = ws_state
        .file_tree
        .as_ref()
        .map(|t| t.root().to_path_buf())
        .as_deref()
        .and_then(|r| delivery::dir_status(r, &ws_state.git_statuses))
        .filter(|st| *st != delivery::TreeState::Ignored)
        .is_some();
    // dirty → 除当前分支外的其余分支全部置灰禁用。
    let lock_others = is_dirty;
    // 面板项/间隔统一走 `crate::menu`(样式基准即文件树右键菜单)。
    let mut items: Vec<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>> =
        Vec::new();
    if ws_state.git_branches.is_empty() {
        items.push(
            text("暂无本地分支")
                .size(byteui::theme::font::body())
                .color(byteui::theme::color::current().dim)
                .into(),
        );
    }
    for name in &ws_state.git_branches {
        let is_current = Some(name.as_str()) == current;
        // 当前分支 GOLD 高亮 + 指示点;其余分支:dirty 锁定时 DIM 置灰,否则
        // 常规 CREAM(同上下文菜单项文字)。
        let color = if is_current {
            byteui::theme::color::current().gold
        } else if lock_others {
            byteui::theme::color::current().dim
        } else {
            byteui::theme::color::current().cream
        };
        let indicator: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
            if is_current {
                text("● ")
                    .size(byteui::theme::font::body())
                    .color(color)
                    .into()
            } else {
                iced_widget::space::Space::new()
                    .width(Length::Fixed(18.0))
                    .into()
            };
        let label = {
            let mut n = name.clone();
            if is_current && is_dirty {
                n.push_str("(Uncommitted)");
            }
            n
        };
        items.push(crate::menu::item_row(
            Some(indicator),
            label,
            color,
            // dirty 锁定时,非当前分支不可点(不挂 `on_press`)。
            (is_current || !lock_others).then(|| Message::BranchSwitch(name.clone())),
        ));
    }
    let list: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
        crate::menu::shell(items, Length::Shrink);

    // 把下拉框钉到 git 底栏正上方:左缘对齐文件面板(左图标栏 + project_pane
    // 左 padding),底缘对齐 git 底栏顶部(footbar 高 + project_pane 底 padding
    // + git 底栏自身高)。外层容器铺满全窗,靠 `Padding{left,bottom}` + 子原件
    // `align_x(Start)`/`align_y(End)` 把它推到左下角(仅 `bottom` padding 而不
    // `align_y(End)` 时,Shrink 高子原件会落在内容区**顶部**,菜单就跑到窗口
    // 最上方去了——与右键菜单 `top` 定位同源,方向相反)。宽度用 `Shrink` 让
    // 菜单贴合最宽项,不会铺满窗口右缘。
    let (left, bottom) = branch_picker_popup_offset(ws_state);
    iced_widget::Container::new(list)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(Padding {
            top: 0.0,
            right: 0.0,
            bottom,
            left,
        })
        .align_x(iced_widget::core::Alignment::Start)
        .align_y(iced_widget::core::Alignment::End)
        .into()
}

/// `branch_picker_popup` 的基准偏移:左缘=左图标栏宽 + project_pane 左 padding;
/// 底缘=footbar 高 + project_pane 底 padding + git 底栏高。二者都吃全局 scale,
/// 随主题/缩放联动,不写死像素。
fn branch_picker_popup_offset(ws_state: &WorkspaceState) -> (f32, f32) {
    let rail = byteui::theme::geometry::icon_rail_width();
    let pane = theme::region::project_pane();
    let left = rail + pane.padding.left;
    // git 底栏高度:顶部分隔 1px + 栏内容(icon_box + 上下 padding 6) + 栏间
    // spacing 4 + 可能的 git_error 一行;project_pane gap 计入把下拉钉紧底栏。
    let git_bar_top_line = 1.0;
    let git_bar_vpad = 6.0 * 2.0;
    let bar_h = byteui::theme::icon_size::row() + 12.0;
    let error_line = if ws_state.git_error.is_some() {
        18.0
    } else {
        0.0
    };
    let git_bar_h = git_bar_top_line + bar_h + git_bar_vpad + 4.0 + error_line;
    let bottom =
        byteui::theme::geometry::footbar_height() + pane.padding.bottom + git_bar_h + pane.gap;
    (left, bottom)
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
    // "搜索"恒置顶(对目录=全文搜该目录,对文件=搜该文件),与其余项用一条
    // 分隔线隔开。
    items.push(crate::menu::item::<Message>(
        Some(icons::IconKind::Search),
        "搜索",
        Message::OpenSearch(menu.target.clone(), menu.is_dir),
    ));
    push_sep(&mut items);
    if menu.is_dir {
        items.push(crate::menu::item::<Message>(
            Some(icons::IconKind::FilePlus),
            "新建文件",
            Message::NewFile(menu.target.clone()),
        ));
        items.push(crate::menu::item::<Message>(
            Some(icons::IconKind::FolderPlus),
            "新建文件夹",
            Message::NewFolder(menu.target.clone()),
        ));
    }
    push_sep(&mut items);
    items.push(crate::menu::item::<Message>(
        Some(icons::IconKind::Copy),
        "复制",
        Message::Copy(menu.target.clone(), menu.is_dir),
    ));
    if menu.is_dir {
        let has_clipboard = ws_state.tree_clipboard.is_some();
        let paste_msg = Message::Paste(menu.target.clone());
        items.push(if has_clipboard {
            crate::menu::item::<Message>(Some(icons::IconKind::ClipboardPaste), "粘贴", paste_msg)
        } else {
            // 剪贴槽为空:置灰且不挂 on_press,真正不可点(同 P1L tab 箭头
            // "到头变灰"的既有处理口径,不是视觉变灰但仍能点)。背景透明
            // 透出容器底,不要 hover 高亮——保持视觉一致的"灰且不可点"。
            crate::menu::item_locked::<Message>(
                Some(icons::IconKind::ClipboardPaste),
                "粘贴",
                byteui::theme::color::current().dim,
            )
        });
    }
    // 项目根不可删除/重命名:从菜单隐去这两项(其余目录均可)。
    if !is_root {
        items.push(crate::menu::item::<Message>(
            Some(icons::IconKind::Trash),
            "删除",
            Message::DeleteRequest(menu.target.clone(), menu.is_dir),
        ));
        items.push(crate::menu::item::<Message>(
            Some(icons::IconKind::Rename),
            "重命名",
            Message::RenameStart(menu.target.clone()),
        ));
    }
    push_sep(&mut items);
    items.push(crate::menu::item::<Message>(
        None,
        "复制绝对路径",
        Message::CopyPath(menu.target.clone(), crate::project::PathKind::Absolute),
    ));
    items.push(crate::menu::item::<Message>(
        None,
        "复制相对路径",
        Message::CopyPath(menu.target.clone(), crate::project::PathKind::Relative),
    ));
    items.push(crate::menu::item::<Message>(
        Some(icons::IconKind::FolderOpen),
        "在 Finder 中打开",
        Message::RevealInFinder(menu.target.clone()),
    ));
    items.push(crate::menu::item::<Message>(
        Some(icons::IconKind::RefreshCw),
        "从磁盘重新加载",
        Message::ReloadFromDisk,
    ));

    let list: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
        crate::menu::shell(items, Length::Shrink);

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
pub(crate) fn menu_separator<'a>()
-> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    crate::menu::separator::<Message>()
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
                .size(byteui::theme::font::subtitle())
                .color(byteui::theme::color::current().cream),
            text("会移入系统回收站,可从回收站找回。")
                .size(byteui::theme::font::label())
                .color(byteui::theme::color::current().dim),
            row![
                button(
                    text("取消")
                        .size(byteui::theme::font::body())
                        .color(byteui::theme::color::current().cream)
                )
                .on_press(Message::DeleteCancel)
                .padding([6, 12])
                .style(|_t, _s| button::Style {
                    background: Some(byteui::theme::color::current().card.into()),
                    text_color: byteui::theme::color::current().cream,
                    border: Border {
                        color: byteui::theme::color::current().border,
                        width: 1.0,
                        radius: 4.0.into()
                    },
                    ..button::Style::default()
                }),
                button(
                    text("删除")
                        .size(byteui::theme::font::body())
                        .color(byteui::theme::color::current().red)
                )
                .on_press(Message::DeleteConfirm)
                .padding([6, 12])
                .style(|_t, _s| button::Style {
                    background: Some(byteui::theme::color::current().card.into()),
                    text_color: byteui::theme::color::current().red,
                    border: Border {
                        color: byteui::theme::color::current().red,
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
        background: Some(byteui::theme::color::current().card.into()),
        border: Border {
            color: byteui::theme::color::current().border,
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
        delivery::TreeState::Untracked => byteui::theme::color::current().red,
        delivery::TreeState::StagedNew => byteui::theme::color::current().green,
        delivery::TreeState::Modified => byteui::theme::color::current().cyan,
        delivery::TreeState::Unchanged => byteui::theme::color::current().body,
        delivery::TreeState::Ignored => byteui::theme::color::current().ignored,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws_with_tree(root: PathBuf) -> WorkspaceState {
        WorkspaceState::new(FileTree::new(root))
    }

    fn row(path: &str, is_dir: bool) -> TreeRow {
        TreeRow {
            path: PathBuf::from(path),
            name: String::new(),
            depth: 0,
            is_dir,
            expanded: false,
        }
    }

    /// 回归测试:main.rs 键盘路由靠 `files_search_focused()` 决定按键是否
    /// 放行给终端/agent PTY——一旦这个标志卡在 `true` 就永久堵死终端输入
    /// (用户实测反馈,2026-08-31:切到文件树面板、用过输入框之后,右侧
    /// agent 输入框再也打不进字)。根因是搜索框不可见的帧(收起 Files 面板
    /// 但 `left_view` 未变、切到别的面板等)`CaptureSearchFocus` 找不到匹配
    /// id、不会覆盖 `SEARCH_FOCUSED`,旧版 `take_search_focused` 又是非消费
    /// 读法,上一次的 `true` 就会一直卡着。改成消费式复位后,读一次没找到
    /// 真身的 `true` 就应该只生效一帧,不能无限期卡住。
    #[test]
    fn take_search_focused_consumes_stale_true_after_one_read() {
        *SEARCH_FOCUSED.lock().unwrap() = true;
        assert!(take_search_focused());
        // `CaptureSearchFocus` 这一帧没找到搜索框(未运行/未命中),复位后
        // 第二次读必须是 false,不能沿用上一帧的 true。
        assert!(!take_search_focused());
    }

    /// 同上,项目树行内编辑框(`TREE_EDIT_FOCUSED`)同款消费式复位回归测试。
    #[test]
    fn take_tree_edit_focused_consumes_stale_true_after_one_read() {
        *TREE_EDIT_FOCUSED.lock().unwrap() = true;
        assert!(take_tree_edit_focused());
        assert!(!take_tree_edit_focused());
    }

    #[test]
    fn tree_drop_target_hits_only_folders_within_viewport() {
        let rows = vec![
            row("/a", true),
            row("/a/f.rs", false),
            row("/b", true),
            row("/c", true),
        ];
        let bounds = (100.0, 100.0, 400.0, 400.0);
        let scroll = 0.0;
        let row_h = crate::theme::geometry::tree_row_h();
        let gap = crate::theme::region::project_pane().gap;
        let pitch = row_h + gap;
        let by = bounds.1;
        // 第 0 行是目录 → 命中。
        assert_eq!(
            tree_drop_target(200.0, by + row_h / 2.0, bounds, scroll, &rows),
            Some(PathBuf::from("/a"))
        );
        // 第 1 行是文件 → None。
        assert_eq!(
            tree_drop_target(200.0, by + pitch + row_h / 2.0, bounds, scroll, &rows),
            None
        );
        // 第 2、3 行都是目录 → 各自命中。
        assert_eq!(
            tree_drop_target(200.0, by + 2.0 * pitch + row_h / 2.0, bounds, scroll, &rows),
            Some(PathBuf::from("/b"))
        );
        assert_eq!(
            tree_drop_target(200.0, by + 3.0 * pitch + row_h / 2.0, bounds, scroll, &rows),
            Some(PathBuf::from("/c"))
        );
        // 视图上方 / 右侧外 → None。
        assert_eq!(
            tree_drop_target(50.0, by + row_h / 2.0, bounds, scroll, &rows),
            None
        );
        assert_eq!(
            tree_drop_target(200.0, by - 5.0, bounds, scroll, &rows),
            None
        );
        // 行间死区吸到下一行目录(idx1 是文件 /a/f.rs,2*pitch 的空档应在 /b
        // 之上;吸住 /b)。
        assert_eq!(
            tree_drop_target(200.0, by + 2.0 * pitch - 1.0, bounds, scroll, &rows),
            Some(PathBuf::from("/b"))
        );
    }

    #[test]
    fn is_valid_move_target_rejects_moving_dir_into_itself() {
        assert!(!is_valid_move_target(
            &PathBuf::from("/proj/a"),
            true,
            &PathBuf::from("/proj/a"),
        ));
    }

    #[test]
    fn is_valid_move_target_rejects_moving_dir_into_own_descendant() {
        assert!(!is_valid_move_target(
            &PathBuf::from("/proj/a"),
            true,
            &PathBuf::from("/proj/a/b"),
        ));
    }

    /// 2026-09 用户实测反馈"无法拖到父目录"后取消了这条拒绝——真无意义时
    /// `move_item` 自己会安全兜底,不需要在悬停高亮这一层提前挡。
    #[test]
    fn is_valid_move_target_allows_dropping_onto_current_parent_as_true_noop() {
        assert!(is_valid_move_target(
            &PathBuf::from("/proj/a/f.rs"),
            false,
            &PathBuf::from("/proj/a"),
        ));
    }

    #[test]
    fn is_valid_move_target_allows_moving_file_to_unrelated_dir() {
        assert!(is_valid_move_target(
            &PathBuf::from("/proj/a/f.rs"),
            false,
            &PathBuf::from("/proj/c"),
        ));
    }

    #[test]
    fn is_valid_move_target_allows_moving_dir_to_unrelated_dir() {
        assert!(is_valid_move_target(
            &PathBuf::from("/proj/a"),
            true,
            &PathBuf::from("/proj/z"),
        ));
    }

    /// 目录名前缀相同但不是真子路径(`/proj/ab` 不是 `/proj/a` 的子目录)不能
    /// 被 `starts_with` 字符串前缀误判——必须走路径分量比较。
    #[test]
    fn is_valid_move_target_does_not_confuse_sibling_with_shared_prefix() {
        assert!(is_valid_move_target(
            &PathBuf::from("/proj/a"),
            true,
            &PathBuf::from("/proj/ab"),
        ));
    }

    #[test]
    fn tree_drop_target_respects_scroll() {
        let rows = vec![row("/a", true), row("/b", true), row("/c", true)];
        let bounds = (0.0, 0.0, 500.0, 500.0);
        let row_h = crate::theme::geometry::tree_row_h();
        let gap = crate::theme::region::project_pane().gap;
        let pitch = row_h + gap;
        // 滚过一整行：视觉第 0 行其实是内容第 1 行 → 命中 /b。
        let scroll = pitch;
        assert_eq!(
            tree_drop_target(100.0, row_h / 2.0, bounds, scroll, &rows),
            Some(PathBuf::from("/b"))
        );
    }

    #[test]
    fn tree_state_colors() {
        assert_eq!(
            tree_state_color(delivery::TreeState::Untracked),
            byteui::theme::color::current().red
        );
        assert_eq!(
            tree_state_color(delivery::TreeState::StagedNew),
            byteui::theme::color::current().green
        );
        assert_eq!(
            tree_state_color(delivery::TreeState::Modified),
            byteui::theme::color::current().cyan
        );
        assert_eq!(
            tree_state_color(delivery::TreeState::Unchanged),
            byteui::theme::color::current().body
        );
        assert_eq!(
            tree_state_color(delivery::TreeState::Ignored),
            byteui::theme::color::current().ignored
        );
    }

    #[tokio::test]
    async fn search_input_then_submit_commits_draft_and_reset_clears() {
        let dir = tempfile::tempdir().unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        let mut app_state = AppState::default();
        let handle = tokio::runtime::Handle::current();

        // text_input::on_input 每次给全量当前字符串,不是逐字符追加。
        update(
            &mut ws_state,
            &mut app_state,
            Message::SearchInput("main".to_string()),
            1,
            &handle,
            |_| {},
        );
        assert_eq!(ws_state.tree_search, "main");
        assert!(ws_state.search_query.is_empty());

        // 提交(点右侧"搜索"按钮,或 iced text_input::on_submit 触发的
        // Enter):草稿落成为生效过滤词。
        update(
            &mut ws_state,
            &mut app_state,
            Message::SearchSubmit,
            1,
            &handle,
            |_| {},
        );
        assert_eq!(ws_state.search_query, "main");

        // 认领其它项目时清空草稿、生效词与焦点镜像,避免旧筛选残留在新
        // 项目树上。
        ws_state.reset_for_project(FileTree::new(dir.path().to_path_buf()));
        assert!(ws_state.tree_search.is_empty() && ws_state.search_query.is_empty());
        assert!(!ws_state.search_focused());
    }

    #[test]
    fn search_field_id_is_stable_across_calls() {
        // `CaptureSearchFocus` 靠 `search_field_id()` 在两处(view() 的
        // `.id()` 与每帧焦点查询)各自构造出的 `Id` 相等来认出同一个字段,
        // 这个前提必须成立。
        assert_eq!(search_field_id(), search_field_id());
    }

    #[test]
    fn set_search_focused_updates_accessor() {
        let mut ws_state = WorkspaceState::default();
        assert!(!ws_state.search_focused());
        ws_state.set_search_focused(true);
        assert!(ws_state.search_focused());
        ws_state.set_search_focused(false);
        assert!(!ws_state.search_focused());
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

    #[test]
    fn arm_tree_drag_records_source_and_starts_with_no_target() {
        let mut ws_state = WorkspaceState::default();
        assert!(!ws_state.is_dragging_tree_item());
        ws_state.arm_tree_drag(PathBuf::from("/proj/a"), true, (10.0, 20.0));
        assert!(ws_state.is_dragging_tree_item());
    }

    #[test]
    fn tree_drag_press_pos_returns_press_point_recorded_at_arm_time() {
        let mut ws_state = WorkspaceState::default();
        assert_eq!(ws_state.tree_drag_press_pos(), None);
        ws_state.arm_tree_drag(PathBuf::from("/proj/a"), true, (10.0, 20.0));
        assert_eq!(ws_state.tree_drag_press_pos(), Some((10.0, 20.0)));
    }

    #[test]
    fn pending_click_open_file_returns_source_when_file_press_had_no_drag_target() {
        let mut ws_state = WorkspaceState::default();
        ws_state.arm_tree_drag(PathBuf::from("/proj/f.rs"), false, (0.0, 0.0));
        assert_eq!(
            ws_state.pending_click_open_file(),
            Some(PathBuf::from("/proj/f.rs"))
        );
    }

    #[test]
    fn pending_click_open_file_none_once_a_drag_target_is_recorded() {
        let mut ws_state = WorkspaceState::default();
        ws_state.arm_tree_drag(PathBuf::from("/proj/f.rs"), false, (0.0, 0.0));
        ws_state.tree_drag.as_mut().unwrap().target = Some(PathBuf::from("/proj/dst"));
        assert_eq!(ws_state.pending_click_open_file(), None);
    }

    #[test]
    fn pending_click_open_file_none_for_directory_source() {
        let mut ws_state = WorkspaceState::default();
        ws_state.arm_tree_drag(PathBuf::from("/proj/dir"), true, (0.0, 0.0));
        assert_eq!(ws_state.pending_click_open_file(), None);
    }

    #[test]
    fn pending_click_open_file_none_when_no_drag_armed() {
        let ws_state = WorkspaceState::default();
        assert_eq!(ws_state.pending_click_open_file(), None);
    }

    #[tokio::test]
    async fn tree_drag_over_expands_collapsed_target_directory() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("child.txt"), b"hi").unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        let mut app_state = AppState::default();
        let handle = tokio::runtime::Handle::current();
        ws_state.arm_tree_drag(dir.path().join("other.txt"), false, (0.0, 0.0));

        update(
            &mut ws_state,
            &mut app_state,
            Message::TreeDragOver(sub.clone()),
            1,
            &handle,
            |_| {},
        );

        assert!(
            ws_state
                .visible_tree_rows()
                .iter()
                .any(|r| r.path == sub && r.expanded)
        );
    }

    #[tokio::test]
    async fn tree_drag_over_does_not_toggle_an_already_expanded_target() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        // 空目录 `toggle` 不会标记 expanded(无可展开内容,见 `FileTree::
        // toggle` 文档),得放一个子项才能让"已展开"这个前提成立。
        std::fs::write(sub.join("child.txt"), b"hi").unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        ws_state.file_tree.as_mut().unwrap().toggle(&sub);
        assert!(
            ws_state
                .visible_tree_rows()
                .iter()
                .any(|r| r.path == sub && r.expanded)
        );
        let mut app_state = AppState::default();
        let handle = tokio::runtime::Handle::current();
        ws_state.arm_tree_drag(dir.path().join("other.txt"), false, (0.0, 0.0));

        update(
            &mut ws_state,
            &mut app_state,
            Message::TreeDragOver(sub.clone()),
            1,
            &handle,
            |_| {},
        );

        // 已展开的目录不该被再 toggle 一次(那会变成收起)。
        assert!(
            ws_state
                .visible_tree_rows()
                .iter()
                .any(|r| r.path == sub && r.expanded)
        );
    }

    #[tokio::test]
    async fn tree_drag_over_valid_target_sets_target_and_highlight() {
        let mut ws_state = WorkspaceState::default();
        let mut app_state = AppState::default();
        let handle = tokio::runtime::Handle::current();
        ws_state.arm_tree_drag(PathBuf::from("/proj/a/f.rs"), false, (0.0, 0.0));

        update(
            &mut ws_state,
            &mut app_state,
            Message::TreeDragOver(PathBuf::from("/proj/c")),
            1,
            &handle,
            |_| {},
        );

        assert_eq!(
            ws_state.tree_drag.as_ref().and_then(|d| d.target.clone()),
            Some(PathBuf::from("/proj/c"))
        );
        assert!(ws_state.drag_hover.contains(&PathBuf::from("/proj/c")));
    }

    #[tokio::test]
    async fn tree_drag_over_invalid_target_clears_target_and_highlight() {
        let mut ws_state = WorkspaceState::default();
        let mut app_state = AppState::default();
        let handle = tokio::runtime::Handle::current();
        // 目录拖到自己的子目录:非法落点,不该被记为待定目标或高亮。
        ws_state.arm_tree_drag(PathBuf::from("/proj/a"), true, (0.0, 0.0));

        update(
            &mut ws_state,
            &mut app_state,
            Message::TreeDragOver(PathBuf::from("/proj/a/b")),
            1,
            &handle,
            |_| {},
        );

        assert_eq!(
            ws_state.tree_drag.as_ref().and_then(|d| d.target.clone()),
            None
        );
        assert!(ws_state.drag_hover.is_empty());
    }

    #[tokio::test]
    async fn tree_drag_end_without_target_clears_drag_state_without_moving_anything() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f.txt");
        std::fs::write(&file, b"hi").unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        let mut app_state = AppState::default();
        let handle = tokio::runtime::Handle::current();
        ws_state.arm_tree_drag(file.clone(), false, (0.0, 0.0));

        update(
            &mut ws_state,
            &mut app_state,
            Message::TreeDragEnd,
            1,
            &handle,
            |_| panic!("no target armed, TreeDragEnd 不该 emit 任何消息"),
        );

        assert!(!ws_state.is_dragging_tree_item());
        assert!(ws_state.drag_hover.is_empty());
        assert!(file.exists());
    }

    #[tokio::test]
    async fn tree_drag_end_with_valid_target_moves_file_and_refreshes_tree() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        let file = dir.path().join("f.txt");
        std::fs::write(&file, b"hi").unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        let mut app_state = AppState::default();
        let handle = tokio::runtime::Handle::current();
        ws_state.arm_tree_drag(file.clone(), false, (0.0, 0.0));
        update(
            &mut ws_state,
            &mut app_state,
            Message::TreeDragOver(sub.clone()),
            1,
            &handle,
            |_| {},
        );

        let (tx, rx) = tokio::sync::oneshot::channel();
        let tx = std::sync::Mutex::new(Some(tx));
        update(
            &mut ws_state,
            &mut app_state,
            Message::TreeDragEnd,
            1,
            &handle,
            move |msg| {
                if let Some(tx) = tx.lock().unwrap().take() {
                    let _ = tx.send(msg);
                }
            },
        );
        assert!(!ws_state.is_dragging_tree_item());
        let done_msg = rx.await.unwrap();
        update(&mut ws_state, &mut app_state, done_msg, 1, &handle, |_| {});

        assert!(!file.exists());
        assert!(sub.join("f.txt").exists());
        assert!(ws_state.tree_error.is_none());
    }

    #[test]
    fn reload_tree_from_disk_reflects_externally_added_and_deleted_files() {
        let dir = tempfile::tempdir().unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        // 初始只有根目录,展开它让行可见。
        ws_state.file_tree.as_mut().unwrap().toggle(dir.path());
        assert!(ws_state.visible_tree_rows().iter().all(|r| !r.is_dir));

        // 外部(agent/其他进程)新增一个文件:此刻未重载,树看不到。
        let added = dir.path().join("new.txt");
        std::fs::write(&added, "x").unwrap();
        assert!(!ws_state.visible_tree_rows().iter().any(|r| r.path == added));

        // `reload_tree_from_disk` 后即时反映新增。
        ws_state.reload_tree_from_disk();
        assert!(ws_state.visible_tree_rows().iter().any(|r| r.path == added));

        // 外部删除同一文件:重载后从树消失。
        std::fs::remove_file(&added).unwrap();
        ws_state.reload_tree_from_disk();
        assert!(!ws_state.visible_tree_rows().iter().any(|r| r.path == added));
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
    async fn edit_input_replaces_whole_buffer() {
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
            Message::EditInput("ab".to_string()),
            1,
            &handle,
            |_| {},
        );
        assert_eq!(ws_state.tree_edit.as_ref().unwrap().buffer, "ab");
        // iced text_input 每次 on_input 给全量当前字符串,不是逐字符追加。
        update(
            &mut ws_state,
            &mut app_state,
            Message::EditInput("a".to_string()),
            1,
            &handle,
            |_| {},
        );
        assert_eq!(ws_state.tree_edit.as_ref().unwrap().buffer, "a");
    }

    #[test]
    fn set_tree_edit_focused_only_tracks_focus_flag() {
        let dir = tempfile::tempdir().unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        ws_state.tree_edit = Some(TreeEdit {
            parent_dir: dir.path().to_path_buf(),
            mode: TreeEditMode::NewFile,
            buffer: "ab".to_string(),
        });
        // 先置真:进入聚焦态。
        ws_state.set_tree_edit_focused(true);
        assert!(ws_state.tree_edit.is_some());
        assert!(ws_state.tree_edit_focused());
        // 焦点从真变假:这里只落焦点标志位——该不该保存/是否清空由内核
        // `App::set_tree_edit_focused` 在边缘处调 `submit_tree_edit` 决定,
        // `set_tree_edit_focused` 本身不再改动编辑态。
        ws_state.set_tree_edit_focused(false);
        assert!(ws_state.tree_edit.is_some());
        assert!(!ws_state.tree_edit_focused());
    }

    #[tokio::test]
    async fn submit_edit_creates_new_file_on_success() {
        let dir = tempfile::tempdir().unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        let mut app_state = AppState::default();
        ws_state.tree_edit = Some(TreeEdit {
            parent_dir: dir.path().to_path_buf(),
            mode: TreeEditMode::NewFile,
            buffer: "fresh.txt".to_string(),
        });
        let handle = tokio::runtime::Handle::current();
        update(
            &mut ws_state,
            &mut app_state,
            Message::EditSubmit,
            1,
            &handle,
            // 成功路径会 spawn 一个异步落盘任务,这里不能 panic。
            |_| {},
        );
        // 提交即取走编辑态(退出行内编辑框)。
        assert!(ws_state.tree_edit.is_none());
        // 让当前线程的 tokio 运行时有空档轮询刚 spawn 的落盘任务,再断言。
        // `spawn_blocking` 会把文件系统操作丢给阻塞线程池,这里 `.await`
        // 主动让出执行器,等它把结果送回。
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(dir.path().join("fresh.txt").exists());
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
            Message::EditSubmit,
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
            Message::EditSubmit,
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
            Message::EditSubmit,
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
