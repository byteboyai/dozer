//! Files 面板状态类型与状态机:TreeEditMode/TreeEdit/ContextMenu/PendingMove/
//! WorkspaceState/GitInfo/AppState/Message/焦点捕获。

use super::git_status;
use crate::project::FileTree;
use iced_widget::core::Rectangle;
use iced_widget::core::widget::operation::Focusable;
use iced_widget::core::widget::{Id, Operation};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use super::*;

/// 项目树行内编辑的模式:新建文件/新建文件夹/重命名(携带原路径)。
/// 现有 `workspace.rs::TreeEditMode` 的搬家版本,定义不变。
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum TreeEditMode {
    NewFile,
    NewFolder,
    Rename(PathBuf),
}

/// 项目树行内编辑态:新建/重命名共用。现有 `workspace.rs::TreeEdit` 的搬家
/// 版本,定义不变。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TreeEdit {
    pub(crate) parent_dir: PathBuf,
    pub(crate) mode: TreeEditMode,
    pub(crate) buffer: String,
}

/// 项目树右键菜单当前打开状态:定位坐标 + 目标(路径/是否目录)。现有
/// `workspace.rs::ContextMenu` 的搬家版本,定义不变。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ContextMenu {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) target: PathBuf,
    pub(crate) is_dir: bool,
    /// 触发这次右键时"当前项目是否有可见/激活的 agent 终端"——非 mac 的
    /// iced 兜底渲染时用它决定"添加到 Agent 上下文"是否置灰(渲染层拿不到
    /// 顶层终端的可见性,故开菜单时存下来带上)。
    pub(crate) agent_terminal_visible: bool,
}

/// 拖拽移动待确认——内部树拖拽(可一次多文件)落点合法、或外部 OS 文件拖入
/// 命中树上某一行后,不立即移动,而是弹这个确认框:用户可在框里改目标目录
/// (多文件时还会列出全部待移动项),取消则整场拖拽作废(磁盘不会有任何改动),
/// 点确定才真正提交(`Message::MoveConfirm`)。2026-09 用户实测反馈:拖拽
/// 移动不该悄无声息直接改路径,得让用户确认。单文件移动时 `sources` 长度为
/// 1,确认框里还允许在 `name_draft` 改文件名;多文件移动时 `sources` 多于一个,
/// 确认框只展示列表、不改名(各自保留原名)。
#[derive(Debug, Clone, PartialEq)]
pub struct PendingMove {
    /// 全部待移动源(支持一次移动多个选中项):`路径 + 是否目录`。
    pub(crate) sources: Vec<(PathBuf, bool)>,
    /// 编辑框草稿:新文件名(不含路径,默认单文件移动时源文件名)。多文件
    /// 移动时此字段忽略,各源直接用各自原名。
    pub(crate) name_draft: String,
    /// 编辑框草稿:目标目录路径文本(默认落点目录,允许手改或用
    /// `MoveDirBrowse` 选)。
    pub(crate) dir_draft: String,
}

/// 挂在每个 Workspace 上的 Files 面板状态(项目信息卡数据 + 文件树 + 树操作
/// 弹层)。对应现有 `Workspace` 上 11 个字段。
#[derive(Default)]
pub struct WorkspaceState {
    /// 待发提示(失败/被拒等一次性反馈)。`App::update` 的包装函数每次处理完消息后
    /// 统一排空成 Toast,见 `extensions::toast::Outbox`。
    pub(crate) outbox: crate::extensions::toast::Outbox,
    /// 待交给 host 的需求(系统对话框、跨面板命令);`App::update` 每条消息后排空执行。
    pub(crate) host: crate::panel_host::HostOutbox<Message>,
    pub(crate) file_tree: Option<FileTree>,
    pub(crate) git_statuses: HashMap<PathBuf, FileGitStatus>,
    /// 目录 → 聚合 git 状态(由 `git_status::rollup_dir_statuses` 在
    /// `StatusesRefreshed` 时一并算出),文件树渲染 O(1) 查表,取代每行
    /// 一次 `dir_status` 的全表扫描(2026-09-17 性能优化)。
    pub(crate) dir_statuses: HashMap<PathBuf, git_status::TreeState>,
    /// 当前选中的文件/目录集合(支持 Shift 多选,见 `select_shift_range`)。
    /// 单选(普通点击)时这个集合里只有唯一一项;`is_selected`/`root_is_selected`
    /// 都按集合成员判断,不再是比较单一路径。
    pub(crate) tree_selected: HashSet<PathBuf>,
    /// Shift 多选的区间起点(锚点):普通点击把它设成被点的那一项,Shift+点击
    /// 时以它为起点、把锚点到被点项之间的可见行整段纳入选中。锚点不在当前
    /// 可见行里时(比如中途折叠了它所在的目录)`select_shift_range` 退化为单选。
    pub(crate) tree_selection_anchor: Option<PathBuf>,
    pub(crate) tree_clipboard: Option<(PathBuf, bool)>,
    /// **仅**移动对话框(`pending_move`)内的校验错误,在对话框里内联显示(对话框保持
    /// 打开,用户就地改正)。其余文件操作的失败/被拒(粘贴、回滚、新建/重命名/删除、
    /// 拖拽移动、行内命名校验)不进这里,走 `outbox` → Toast。(原名 `tree_error`,
    /// 曾同时承载这两类语义。)
    pub(crate) move_error: Option<dozer_model::LocalizedText>,
    /// 待确认的删除目标(支持一次删除多个选中项):`路径 + 是否目录` 列表。
    /// 单文件删除时长度为 1;右键菜单点"删除"时若被右键项属于当前多选集,
    /// 整组选中项一起进入待删除(项目根会被过滤掉)。见 `delete_confirm_spec`
    /// 的标题/列表渲染与 `Message::DeleteConfirm` 的批量删除逻辑。
    pub(crate) tree_delete_confirm: Option<Vec<(PathBuf, bool)>>,
    pub(crate) tree_edit: Option<TreeEdit>,
    /// 项目树行内编辑框是否持有 iced 内部真实焦点,每帧由 `CaptureTreeEditFocus`
    /// 写入。
    pub(crate) tree_edit_focused_flag: bool,
    /// 一次性标记:`tree_edit` 刚从 `None` 变成 `Some`(新建/重命名刚
    /// 触发)时置真,main.rs 渲染循环取走后用 `operation::focusable::
    /// focus` 强制聚焦真正的 `text_input`——右键菜单点"重命名"/"新建
    /// 文件"这类触发点击落在别的控件上,新出现的输入框不会自动拿到
    /// iced 焦点,需要这一下程序化聚焦(同 `todo::scroll_to_top` 的既有
    /// 一次性位手法)。
    pub(crate) tree_edit_focus_pending: bool,
    /// 搜索框里正在键入的草稿文本(尚未提交时不影响树)。
    pub(crate) tree_search: String,
    /// 已提交的搜索关键字:仅当提交(敲回车/点搜索按钮)后用它过滤树。
    pub(crate) search_query: String,
    /// 搜索框是否持有 iced 内部真实焦点。**不是**应用层手动置位的镜像——
    /// 每帧渲染循环里 `CaptureSearchFocus` 问一遍 iced 真相后立刻写进这里
    /// (`set_search_focused`),`main.rs` 键盘路由读它决定要不要把事件放行
    /// 给标准 iced 管线(同 `search_field_id`/`CaptureSearchFocus` 说明)。
    pub(crate) search_focused: bool,
    /// 文件树 Scrollable 当前滚动偏移(逻辑像素,Y 向下偏移)。由树自身的
    /// `on_scroll` 上报;main.rs 在外部文件拖拽事件层用它与行高几何做命中
    /// 测试,把光标位置换算成命中的目录行。此偏移只影响命中测试,不影响
    /// 渲染(渲染仍由 Scrollable 内部状态驱动)。
    pub(crate) tree_scroll: f32,
    /// 外部文件拖拽悬停时命中的目录集合:命中即整行高亮为"拖入落点",
    /// 由 `FileDragHover` 更新、清空时为空集合。
    pub(crate) drag_hover: HashSet<PathBuf>,
    /// 鼠标当前悬停的树行(行 hover 背景用)。**不能**靠内层 `button` 的
    /// `button::Status::Hovered` 判断——树行按下走外层 `MouseArea`(见
    /// `TreeRowPress` 文档),内层 button 不挂 `on_press`,iced 视其为
    /// 禁用态、永远报 `Disabled` 而非 `Hovered`。改由行 `MouseArea` 的
    /// `on_enter`/`on_exit` 上报 `TreeRowHover` 维护,与
    /// `byteui::icons::icon_button_entry` 的 hover 手法同款。
    pub(crate) tree_hover: Option<PathBuf>,
    /// 底部 git 分支栏信息是否已加载(`GitInfoLoaded` 送达前为 false,此时
    /// 分支栏显示中性"…"占位)。
    pub(crate) git_loaded: bool,
    /// 项目根目录是否在 git 仓库内(决定底栏显示"分支切换"还是
    /// "未受 git 保护 / 新建仓库")。
    pub(crate) git_is_repo: bool,
    /// 当前分支名(detached HEAD / 无提交时为 None)。
    pub(crate) current_branch: Option<String>,
    /// 当前分支是否已有至少一次提交(unborn/空仓为 false)。分支菜单据此把
    /// 其余分支置灰禁用 `BranchPickerOpen` 时的渲染用)。
    pub(crate) current_branch_has_commits: bool,
    /// 本地分支列表(仅 git 仓库内有意义;空则无分支可切)。
    pub(crate) git_branches: Vec<String>,
    /// 分支切换弹层是否展开(展开时在底栏上方罗列可切换的本地分支)。
    pub(crate) branch_picker_open: bool,
    /// 底栏 git 操作(切换分支/新建仓库)的最近错误,就地显示在底栏下缘。
    pub(crate) git_error: Option<String>,
    /// 树内拖拽移动进行中的状态,`None` 表示当前没有树内拖拽——见 `TreeDrag`
    /// 文档。按下树行武装(`arm_tree_drag`),`TreeDragOver`/`TreeDragEnd`
    /// 更新/收尾。
    pub(crate) tree_drag: Option<TreeDrag>,
    /// 拖拽(外部 OS 拖入或树内拖拽,两条路径共用这一份状态)悬停在一个
    /// 仍处于折叠态的目录上时,记录"从什么时候开始悬停"——`arm_drag_expand`
    /// 维护、`drag_expand_ready` 读、`clear_drag_expand` 清空。真正的展开
    /// 推迟到悬停满 `DRAG_HOVER_EXPAND_DELAY` 才由 `App::advance_drag_hover_
    /// expand` 触发(同 `HOVER_TOOLTIP_DELAY` 那套"武装计时→轮询到点才动手"
    /// 手法),不在悬停那一刻立即展开——2026-09 用户实测反馈:拖着划过时
    /// 沿途目录被立即强制展开,布局跟着疯狂跳动,来不及瞄准真正想投放的
    /// 目录。
    pub(crate) drag_expand_pending: Option<(PathBuf, std::time::Instant)>,
    /// 拖拽移动待确认,见 `PendingMove` 文档。
    pub(crate) pending_move: Option<PendingMove>,
    /// 一次性标记:`pending_move` 刚从 `None` 变成 `Some` 时置真,main.rs
    /// 渲染循环取走后程序化聚焦"新名称"输入框(同 `tree_edit_focus_pending`
    /// 的既有手法,见其文档)。
    pub(crate) move_focus_pending: bool,
}

/// 拖拽悬停到折叠目录后等满这么久才自动展开,见 `WorkspaceState::
/// drag_expand_pending` 文档。
pub(crate) const DRAG_HOVER_EXPAND_DELAY: std::time::Duration = std::time::Duration::from_secs(1);

pub use dozer_panel_files::state::GitInfo;

/// 预览 tab 页签右键菜单浮层状态(屏幕空间单例,不随项目切换各自保留)。
/// 与 `ContextMenu`(文件树行右键)互斥——两者不能同挂(见
/// `Message::TabContextMenuOpen` 的清理约定)。`kind` 记下菜单来自哪个预览
/// 面板(Files/Project),供 webview 隐藏判断按面板种类命中(同 `context_menu`
/// 既有的"面板内浮层盖住 webview"场景);`path` 是 tab 对应文件的绝对路径,
/// 作为各菜单项的右键目标。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TabContextMenu {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) kind: crate::app::PanelKind,
    pub(crate) path: PathBuf,
}

/// 挂在 App 上的右键菜单浮层状态(屏幕空间单例,不随项目切换各自保留)。
/// 对应现有 `App` 上 `context_menu`/`last_right_click` 两个字段。
#[derive(Default)]
pub struct AppState {
    pub(crate) context_menu: Option<ContextMenu>,
    pub(crate) tab_context_menu: Option<TabContextMenu>,
    pub(crate) last_right_click: (f32, f32),
}

/// 消息类型已随文件树迁入 `dozer-panel-files`(S5b-1);这里重新导出,
/// 让 `files::Message` 与库内 `update`/`view` 用的是同一个类型。
pub use dozer_panel_files::state::Message;

pub use dozer_panel_files::state::FilesToolbarTarget;

/// 搜索框稳定的 iced widget id:`view()` 里 `.id()` 挂给真正的
/// `text_input`,`CaptureSearchFocus` 每帧靠它在 widget 树里认出这一个
/// (同 `extensions::todo::add_field_id` 的既有手法——`Id::new` 而非
/// `Id::unique()`,保证同一个字符串在两处各自构造出的 `Id` 相等)。
pub fn search_field_id() -> Id {
    Id::new("files-search-box")
}

pub(crate) static SEARCH_FOCUSED: std::sync::LazyLock<std::sync::Mutex<bool>> =
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

/// 拖拽移动确认框"新名称"输入框稳定的 iced widget id——`pending_move`
/// 刚出现时程序化聚焦这个 id(见 `move_focus_pending` 文档),同一时刻
/// 至多一场待确认的移动,固定 id 够用。
pub fn move_name_field_id() -> Id {
    Id::new("files-move-name-box")
}

/// 拖拽移动确认框"到目录"输入框稳定的 iced widget id。
pub fn move_dir_field_id() -> Id {
    Id::new("files-move-dir-box")
}

pub(crate) static TREE_EDIT_FOCUSED: std::sync::LazyLock<std::sync::Mutex<bool>> =
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
    /// 取走待发提示(`App::drain_outboxes` 调用)。
    pub fn take_outbox(&mut self) -> Vec<crate::extensions::toast::Pending> {
        self.outbox.take()
    }

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
    /// 包括现状本来就没重置 `worktrees`/`tree_clipboard`/`move_error`/
    /// `tree_delete_confirm`/`tree_edit` 这一点,纯迁移不新增行为)。
    pub fn reset_for_project(&mut self, file_tree: FileTree) {
        self.file_tree = Some(file_tree);
        self.tree_selected = HashSet::new();
        self.tree_selection_anchor = None;
        self.git_statuses = HashMap::new();
        self.dir_statuses = HashMap::new();
        self.tree_search.clear();
        self.search_query.clear();
        self.search_focused = false;
    }

    /// 供测试断言"删除确认状态是否存在"(生产侧已改由
    /// `platform::confirm_overlay` 的 `desired_confirm` 读取
    /// `tree_delete_confirm` 本体,不再经此访问器)。
    #[cfg(test)]
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

    /// 供内核判断"拖拽移动确认框该不该显示"(`App::view()` 顶层互斥浮层
    /// 判断链用,同 `tree_delete_confirm_is_some` 的用法),以及键盘路由是否
    /// 该整体放行给对话框里的两个原生 `text_input`(粗粒度信号,同 SSH/
    /// Database 表单"整体放行不细分字段"的既有口径——同一时刻框里只有两个
    /// 字段,不需要为哪个字段真正持有焦点单独判断)。
    pub fn pending_move_is_some(&self) -> bool {
        self.pending_move.is_some()
    }

    /// 读走(消费式)一次性聚焦标记,同 `take_tree_edit_focus_pending` 的
    /// 既有手法——main.rs 据此程序化聚焦"新名称"输入框。
    pub fn take_move_focus_pending(&mut self) -> bool {
        std::mem::take(&mut self.move_focus_pending)
    }

    /// `Message::MoveDirBrowse` 弹原生目录选择器时用作起始目录(main.rs
    /// `Runner::dispatch` 拦截处理时调用,见其文档)。没有待确认的移动时
    /// 返回 `None`。
    pub fn move_dir_draft(&self) -> Option<&str> {
        self.pending_move.as_ref().map(|m| m.dir_draft.as_str())
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

    /// 工作区是否有未提交改动——项目根目录的聚合 git 状态非 `Ignored` 即有。
    /// 分支切换菜单用:dirty 时锁定除当前分支外的其余分支(dirty 下切分支会
    /// 被 git 拒绝),并给当前分支追加 "(Uncommitted)" 提示。原生菜单
    /// (`files::update` 的 `BranchPickerOpen`)与 iced 弹层
    /// (`branch_picker_popup`)共用这一份判断。
    pub(crate) fn git_dirty(&self) -> bool {
        self.file_tree
            .as_ref()
            .map(|t| t.root().to_path_buf())
            .as_deref()
            .and_then(|r| self.dir_statuses.get(r).copied())
            // 聚合时忽略被忽略文件,`Some` 即真实未提交改动。
            .filter(|st| *st != git_status::TreeState::Ignored)
            .is_some()
    }

    /// 供内核 `Message::PreviewOpenPath` 处理器调用——打开预览的同时把该
    /// 文件标记为项目树里的"选中"行(点击文件行→打开预览→该行高亮,是
    /// 现状既有的联动效果,不是这次重构新增的)。这里走**单选**:清空现有
    /// 多选,只留 `path` 一项,并把锚点也设到 `path`(后续 Shift+点击以它为
    /// 起点延展区间)。
    pub fn set_tree_selected(&mut self, path: PathBuf) {
        self.select_single(path);
    }

    /// 单选:清空选中集、只保留 `path`,并把多选锚点设成 `path`。普通点击、
    /// 双击目录(`Toggle`)、右键落在选区外、打开预览都用它。
    pub(crate) fn select_single(&mut self, path: PathBuf) {
        self.tree_selected.clear();
        self.tree_selected.insert(path.clone());
        self.tree_selection_anchor = Some(path);
    }

    /// Cmd/Ctrl+点击(非 mac 为 Ctrl)的单键切换:在选中集里"切换"一项——
    /// 命中已选项则移除(剩下空集时退回锚点空),未命中则加入并保持锚点。
    /// 用于挑出任意不连续的多项,配合 `select_shift_range`(连续区间)两类
    /// 多选互补。空点击(点在文件树空白处)由 `App::update` 端清掉选中集,不
    /// 走这里。
    pub(crate) fn select_toggle(&mut self, path: PathBuf) {
        if !self.tree_selected.remove(&path) {
            self.tree_selected.insert(path.clone());
        }
        self.tree_selection_anchor = Some(path);
    }

    /// Shift+点击:以 `tree_selection_anchor` 为起点,把锚点到 `path` 之间的
    /// 全部可见行纳入选中(替换原有选中集,标准文件管理器区间选)。锚点缺失、
    /// 锚点或被点项不在当前可见行里时,退化为单选 `path`(锚点同步更新到
    /// `path`)。锚点本身**保持不动**,这样连续多次 Shift+点击会一直以最初
    /// 那一项为起点延展。
    pub(crate) fn select_shift_range(&mut self, path: PathBuf) {
        let Some(anchor) = self.tree_selection_anchor.clone() else {
            self.select_single(path);
            return;
        };
        let rows = self.visible_tree_rows();
        // 同一行可能既匹配锚点又匹配被点项(两者相同),用 `positions` 收集索引、
        // 用 `len()==2` 判定是否是"两个不同行"。
        let positions: Vec<usize> = rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.path == anchor || r.path == path)
            .map(|(i, _)| i)
            .collect();
        let anchor_visible = rows.iter().any(|r| r.path == anchor);
        if positions.len() != 2 || !anchor_visible {
            self.select_single(path);
            return;
        }
        let (lo, hi) = (
            positions[0].min(positions[1]),
            positions[0].max(positions[1]),
        );
        self.tree_selected.clear();
        for r in &rows[lo..=hi] {
            self.tree_selected.insert(r.path.clone());
        }
    }

    /// 移动对话框内的校验错误文案,供内核测试用作槽位内容的身份标记(见
    /// `workspace.rs` 测试模块 `loaded_slot`)。只有测试会调用,生产代码不
    /// 需要读它(渲染走 `files::view` 内部,不经这个访问器)。
    #[cfg(test)]
    pub fn move_error(&self) -> Option<&str> {
        self.move_error.as_ref().map(|t| t.fallback.as_str())
    }

    /// 同上,供测试构造带标记的槽位用。
    #[cfg(test)]
    pub fn set_move_error(&mut self, e: Option<String>) {
        self.move_error = e.map(|t| dozer_model::LocalizedText::literal(FILES_OWNER, t));
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
            .map(|t| {
                if self.search_query.is_empty() {
                    t.visible_rows()
                } else {
                    t.search_rows(&self.search_query)
                }
            })
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
    pub(crate) fn start_tree_new(&mut self, parent: PathBuf, mode: TreeEditMode) {
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
        let name = edit.buffer.trim();
        if name.is_empty() {
            return;
        }
        if !crate::project::is_single_path_component(name) {
            self.outbox.push(
                LOG,
                crate::extensions::toast::Level::Error,
                crate::i18n::panel_keyed(
                    FILES_OWNER,
                    "edit.error.invalid_name",
                    "Name cannot contain a path separator",
                ),
            );
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
                    self.outbox.push(
                        LOG,
                        crate::extensions::toast::Level::Error,
                        crate::i18n::panel_keyed_args(
                            FILES_OWNER,
                            "edit.error.exists",
                            "{name} already exists",
                            &[("name", new_path.display().to_string().as_str())],
                        ),
                    );
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
                    self.outbox.push(
                        LOG,
                        crate::extensions::toast::Level::Error,
                        crate::i18n::panel_keyed_args(
                            FILES_OWNER,
                            "edit.error.exists",
                            "{name} already exists",
                            &[("name", new_path.display().to_string().as_str())],
                        ),
                    );
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
                    self.outbox.push(
                        LOG,
                        crate::extensions::toast::Level::Error,
                        crate::i18n::panel_keyed_args(
                            FILES_OWNER,
                            "edit.error.exists",
                            "{name} already exists",
                            &[("name", new_path.display().to_string().as_str())],
                        ),
                    );
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
        armed_at: std::time::Instant,
    ) {
        // 多选拖拽:若按下的这一项已在当前多选集里、且选中多于一项,整组一起
        // 拖(用户拖任意选中项都带走整组);否则只拖这一项。项目根永远排除——
        // 不能移动/删除项目根。选中项是否目录取可见行表,取不到(如已折叠的
        // 子树项)时退回 `Path::is_dir()`。
        let root = self.file_tree.as_ref().map(|t| t.root().to_path_buf());
        let sources: Vec<(PathBuf, bool)> =
            if self.tree_selected.contains(&source) && self.tree_selected.len() > 1 {
                let visible: std::collections::HashMap<PathBuf, bool> = self
                    .visible_tree_rows()
                    .into_iter()
                    .map(|r| (r.path, r.is_dir))
                    .collect();
                self.tree_selected
                    .iter()
                    .filter(|p| Some(p.as_path()) != root.as_deref())
                    .map(|p| {
                        let is_dir = visible.get(p).copied().unwrap_or_else(|| p.is_dir());
                        (p.clone(), is_dir)
                    })
                    .collect()
            } else {
                vec![(source.clone(), source_is_dir)]
            };
        // 兜底:整组过滤后若只剩根(极端情况),退回只拖被按下的这一项。
        let sources = if sources.is_empty() {
            vec![(source.clone(), source_is_dir)]
        } else {
            sources
        };
        self.tree_drag = Some(TreeDrag {
            source,
            source_is_dir,
            sources,
            phase: TreeDragPhase::Pending,
            press_pos,
            armed_at,
        });
    }

    /// 拖拽悬停命中一个目录时调用(外部 OS 拖入、内部树拖拽两条路径共用):
    /// `already_expanded` 由调用方算好传入(两条路径判断口径不同,一条读
    /// `visible_tree_rows`,一条还要考虑根目录特例,見各自调用点)。已展开
    /// 直接清空计时(没什么好等的);未展开且悬停的还是同一个目录则**不**
    /// 重置计时(这才是"悬停满 1s"的关键——否则光标在同一行内轻微抖动,
    /// 每帧都重新起计时,永远等不满);悬停切换到别的目录才重新起计时。
    pub(crate) fn arm_drag_expand(&mut self, dir: PathBuf, already_expanded: bool) {
        if already_expanded {
            self.drag_expand_pending = None;
            return;
        }
        if self.drag_expand_pending.as_ref().map(|(p, _)| p) != Some(&dir) {
            self.drag_expand_pending = Some((dir, std::time::Instant::now()));
        }
    }

    /// 悬停离开目标目录、拖拽收尾/取消时调用:清空计时,不留残留状态
    /// (否则一场拖拽结束后,下一场全新拖拽可能被上一场的残留计时提前
    /// 触发展开)。
    pub(crate) fn clear_drag_expand(&mut self) {
        self.drag_expand_pending = None;
    }

    /// 当前悬停的目录是否已经计满 `DRAG_HOVER_EXPAND_DELAY`——满则返回该
    /// 目录路径(克隆一份,调用方据此发 `Message::Toggle` 真正展开),未满
    /// 或没有悬停中的目录都返回 `None`。供 `App::advance_drag_hover_expand`
    /// 每次唤醒时轮询。
    pub(crate) fn drag_expand_ready(&self) -> Option<PathBuf> {
        let (dir, start) = self.drag_expand_pending.as_ref()?;
        (start.elapsed() >= DRAG_HOVER_EXPAND_DELAY).then(|| dir.clone())
    }

    /// 距计时满 1s 的剩余时间——`App::next_drag_hover_expand_wake` 据此给
    /// `about_to_wait` 排精确唤醒(同 `next_tooltip_wake` 手法),不空转也
    /// 不迟到。没有悬停中的目录时返回 `None`。
    pub(crate) fn next_drag_expand_wake(&self) -> Option<std::time::Duration> {
        let (_, start) = self.drag_expand_pending.as_ref()?;
        DRAG_HOVER_EXPAND_DELAY.checked_sub(start.elapsed())
    }

    /// 树内拖拽是否正在进行(供 `App::dragging_tree_item()` 判断全局左键
    /// 松开是否该收尾这场拖拽——同 `dragging_rail`/`dragging_tab` 的用法)。
    /// 无论处于 `Pending` 还是 `Dragging` 都算"在进行":松开必须总能收尾、
    /// 清掉状态,哪怕从未越过确认阈值(那种情况下收尾就是执行点击语义,见
    /// `Message::TreeDragEnd` 文档)。
    pub(crate) fn is_dragging_tree_item(&self) -> bool {
        self.tree_drag.is_some()
    }

    /// 当前这场树内拖拽是否还停留在 `Pending`(尚未越过确认阈值)——供
    /// `App::maybe_confirm_tree_drag` 每次 `CursorMoved` 判断要不要推进到
    /// `Dragging`。已经是 `Dragging` 或根本没有拖拽都返回 `false`(前者
    /// 不需要重复推进,后者无意义)。
    pub(crate) fn tree_drag_is_pending(&self) -> bool {
        matches!(
            self.tree_drag.as_ref().map(|d| &d.phase),
            Some(TreeDragPhase::Pending)
        )
    }

    /// 把 `Pending` 推进到 `Dragging { target: None }`——只应该在越过确认
    /// 阈值(`tree_drag_is_pending()` 为真且距离/时长两道阈值都过)后调用
    /// 一次,见 `TreeDragPhase` 文档。已经是 `Dragging` 或没有拖拽时是
    /// no-op。
    pub(crate) fn confirm_tree_drag(&mut self) {
        if let Some(drag) = &mut self.tree_drag
            && matches!(drag.phase, TreeDragPhase::Pending)
        {
            drag.phase = TreeDragPhase::Dragging { target: None };
        }
    }

    /// 自愈式无条件清空(不做任何点击语义补偿、不高亮、不提交移动)——供
    /// `App::maybe_confirm_tree_drag` 在左键并未物理按住却发现还有残留
    /// `tree_drag` 时调用。正常路径下 `TreeDragEnd` 早该清过一次;还留着
    /// 说明那次收尾出于某种原因没有发生,这已经是异常状态,不该假装它是
    /// 一次合法的单击或拖拽去补执行任何副作用,原地清空最安全。
    pub(crate) fn cancel_tree_drag(&mut self) {
        self.tree_drag = None;
        self.drag_hover = HashSet::new();
    }

    /// 当前这场树内拖拽是否已确认为 `Dragging`(供 `view()` 判断要不要给
    /// 目录行挂 `on_move`/给光标换成抓取图标,以及 `App::tree_drag_ghost`
    /// 判断要不要画跟随光标的幽灵胶囊)。`Pending` 或没有拖拽都返回
    /// `false`——`Pending` 期间必须完全没有任何视觉/交互反应,这正是
    /// 2026-09 用户反馈"点一下就进入拖拽态"要修的东西,见 `TreeDragPhase`
    /// 文档。
    pub(crate) fn tree_drag_confirmed(&self) -> bool {
        matches!(
            self.tree_drag.as_ref().map(|d| &d.phase),
            Some(TreeDragPhase::Dragging { .. })
        )
    }

    /// `Dragging` 中主源的路径 + 是否目录 + 本次拖拽源总数(`None` = 没有
    /// 确认中的拖拽)——供 `App::tree_drag_ghost` 取图标/文件名/项数。多文件
    /// 拖拽时 `count > 1`,胶囊显示"name 等 N 项"。`Pending` 期间返回 `None`:
    /// 幽灵胶囊只在确认后才画,同 `tree_drag_confirmed` 的理由。
    pub(crate) fn tree_drag_ghost_source(&self) -> Option<(&std::path::Path, bool, usize)> {
        let drag = self.tree_drag.as_ref()?;
        matches!(drag.phase, TreeDragPhase::Dragging { .. }).then_some((
            drag.source.as_path(),
            drag.source_is_dir,
            drag.sources.len(),
        ))
    }

    /// 当前这场树内拖拽按下瞬间的光标位置(`None` = 没有拖拽在进行)——供
    /// `App::maybe_confirm_tree_drag` 和当前光标比对,判断是否已越过确认
    /// 阈值,见 `TreeDrag::press_pos` 文档。
    pub(crate) fn tree_drag_press_pos(&self) -> Option<(f32, f32)> {
        self.tree_drag.as_ref().map(|d| d.press_pos)
    }

    /// 当前这场树内拖拽按下瞬间的墙钟时间(`None` = 没有拖拽在进行)——供
    /// `App::maybe_confirm_tree_drag` 算按住时长,见 `TreeDrag::armed_at`
    /// 文档。
    pub(crate) fn tree_drag_armed_at(&self) -> Option<std::time::Instant> {
        self.tree_drag.as_ref().map(|d| d.armed_at)
    }
}

impl AppState {
    /// 供内核判断"右键菜单该不该显示"(`App::view()` 顶层互斥浮层判断链
    /// 用)。
    pub fn context_menu_is_some(&self) -> bool {
        self.context_menu.is_some()
    }
    /// 预览 tab 页签右键菜单是否打开(内核 Esc 键路由 / webview 隐藏判断用)。
    pub fn tab_context_menu_is_some(&self) -> bool {
        self.tab_context_menu.is_some()
    }
    /// 渲染用:返回当前打开的预览 tab 右键菜单(坐标/面板/路径),`None` 表示
    /// 未打开。非 macOS 的 iced 浮层才读它;macOS 走原生 NSMenu 不读。
    pub fn tab_context_menu(&self) -> Option<&TabContextMenu> {
        self.tab_context_menu.as_ref()
    }
    /// 关掉文件树右键菜单(供其它浮层打开时互斥清理,见 `App::update` 的
    /// `PreviewTabContextMenu` 分支——避免两者同时挂着导致 dismiss 串味)。
    /// 顺带关掉预览 tab 右键菜单:两者同属"文件相关右键浮层",互斥清理统一
    /// 在这一个入口收口,各菜单项动作走 `close_context_menu()` 即一并 dismiss。
    pub fn close_context_menu(&mut self) {
        self.context_menu = None;
        self.tab_context_menu = None;
    }
    /// 最近一次右键落点坐标(屏幕空间,已由 main.rs 钳制在窗口内)。
    /// 预览 tab 右键菜单复用同一份坐标,避免再写一套捕获逻辑。
    pub fn last_right_click(&self) -> (f32, f32) {
        self.last_right_click
    }
}

/// 库的中性预览来源 ↔ Dozer 的 `PanelKind`(S5b-1:库不认识 `PanelKind`)。
pub(crate) fn preview_kind_of(
    panel: crate::app::PanelKind,
) -> dozer_panel_files::adapter::PreviewKind {
    match panel {
        crate::app::PanelKind::Project => dozer_panel_files::adapter::PreviewKind::Project,
        _ => dozer_panel_files::adapter::PreviewKind::Files,
    }
}

pub(crate) fn panel_kind_of(
    kind: dozer_panel_files::adapter::PreviewKind,
) -> crate::app::PanelKind {
    match kind {
        dozer_panel_files::adapter::PreviewKind::Project => crate::app::PanelKind::Project,
        dozer_panel_files::adapter::PreviewKind::Files => crate::app::PanelKind::Files,
    }
}
