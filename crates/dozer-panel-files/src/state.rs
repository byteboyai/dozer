//! Files 面板的消息与它们的纯数据载荷。

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::fs::PathKind;
use crate::git_status::FileGitStatus;

#[derive(Debug, Clone, Copy)]
pub enum FilesToolbarTarget {
    SearchSubmit,
    Dotfiles,
    BranchSwitch,
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

/// 对应现在顶层 `Message` 里的 20 个 `ProjectTreeXxx`/`ProjectGitRefreshed`/
/// `RightClickAt` 变体,去前缀原样搬来。
#[derive(Debug, Clone)]
pub enum Message {
    /// 立即展开/折叠一个目录并选中它,不经拖拽/双击那套延迟判定——两处
    /// 调用点:①目录行被双击(`TreeRowDoubleClick` 分派,给没精确点在箭头
    /// 上的用户一个整行都能双击的入口);②外部/内部拖拽悬停到折叠目录时
    /// 自动展开(`expand_files_dir_if_collapsed`/`expand_dir_if_collapsed`)。
    /// 不跨内核边界,直接在 `files::update()` 里处理。目录箭头(`>`/`V`)
    /// 本身**不**发这条消息——见 `ToggleNoSelect`(2026-09 用户反馈:点箭头
    /// 只是想看子项,不代表要把这个目录选中,原先箭头也发 `Toggle` 会把它
    /// 一并选中)。
    Toggle(PathBuf),
    /// 只展开/折叠,不改变选中——目录箭头(`>`/`V`)专用,见 `view()` 挂在
    /// 箭头自己的 `MouseArea::on_press` 上(点击会 `shell.capture_event()`,
    /// 不会冒泡触发外层整行的 `TreeRowPress`/`TreeRowDoubleClick`)。与
    /// `Toggle` 的唯一差异就是不写 `tree_selected`。
    ToggleNoSelect(PathBuf),
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
    /// 预览 tab 页签右键"打开菜单":携带该 tab 对应的文件绝对路径与所在预览
    /// 面板种类(Files/Project)。具体坐标复用 `AppState::last_right_click`
    /// (main.rs 右键钳制后的落点),不另行传参。预处理先互斥关掉文件树右键
    /// 菜单与分支切换弹层(`close_context_menu` 已一并清 tab 菜单),避免两个
    /// 浮层同时挂着。
    TabContextMenuOpen {
        kind: crate::adapter::PreviewKind,
        path: PathBuf,
    },
    /// 预览 tab 页签右键菜单的关闭(dismiss)。与 `ContextMenuClose` 同口径,
    /// 由 `close_context_menu()` 统一清掉两类菜单。
    TabContextMenuClose,
    /// 预览 tab 右键菜单"关闭":关掉右键目标所对应的那个预览 tab。携带该 tab
    /// 所在预览面板种类(Files/Project)与其文件的绝对路径——`app/update.rs`
    /// 在两个预览面板里按路径定位 tab、调 `PreviewPane::close` 关掉它(见
    /// `preview.rs::PreviewPane::close` 文档)。`kind` 决定优先命中哪个面板,
    /// 与 `TabContextMenuOpen` 同一口径。
    TabContextMenuCloseTab(crate::adapter::PreviewKind, PathBuf),
    /// 预览 tab 右键"从磁盘重新加载":内核/app 拦截,不进 `files::update`——
    /// 由 `app/update.rs` 在 Files/Project 两个预览面板里按路径找出对应 tab,
    /// 调 `PreviewPane::bump_reload` 重建编辑器/推进 webview 的 `reload_nonce`
    /// 让 webview 重新 `load_url` 读盘最新内容(见 `preview.rs::PreviewPane::
    /// bump_reload` 文档)。携带该 tab 文件的绝对路径。
    TabReloadFromDisk(PathBuf),
    /// 右键"查看此文件历史":`files::update` 先收起右键菜单,再往 `ws_state.host` 提一个
    /// `PanelCommand::ShowFileHistory`;host 的 `run_panel_command` 解析出仓库
    /// 相对路径、组出 `file_history::FileHistoryTarget`,写入
    /// `App::file_history` 并异步跑 `file_history::build`(见
    /// `docs/superpowers/specs/2026-09-17-file-history-popup-design.md`)。
    /// 携带的是右键目标的绝对路径,同其它右键菜单消息(`DeleteRequest`/
    /// `OpenWithDefault` 等)的既有口径。
    FileHistoryOpen(PathBuf),
    /// 右键菜单"回滚到上一版本":内核/app 拦截,不进 `files::update`——
    /// 由 `app/update.rs` 解析出仓库相对路径、算上一版本 Oid、调
    /// `file_history::rollback_to` 把文件还原(同 `FileHistoryOpen` 在 app
    /// 层接线的既有写法,见其注释)。携带右键目标的绝对路径。
    FileHistoryRollbackPrevious(PathBuf),
    /// `FileHistoryRollbackPrevious` 的异步结果:成功即把文件还原到了上一版本,
    /// 失败带错误信息。由 `app/update.rs` 发出、在 `files::update` 里刷新树/
    /// 失败推 Toast(不进 `file_history` 弹窗状态机——这是从文件树右键发起
    /// 的一键动作,与弹窗内的版本挑选回滚是两条独立路径)。
    FileHistoryRollbackDone(i64, PathBuf, Result<(), String>),
    /// 右键菜单"搜索":`files::update` 先收起右键菜单,再往 `ws_state.host` 提一个
    /// `PanelCommand::SearchIn`;host 把它映射成 `search::Message::SearchOpen`
    /// 打开文件树右键作用域的搜索弹窗。
    OpenSearch(PathBuf, bool),
    /// 文件树右键"添加到 Agent 上下文":`bool` = `is_dir`。真正的 PTY 写入
    /// 需要内核顶层的终端句柄,`files::update` 拿不到,同 `CopyPath` 的既有
    /// 模式——`update` 里把拼好的文本经 `emit` 回内核顶层拦截处理
    /// (`RequestSendToAgentTerminal`)。
    SendToAgentContext(PathBuf, bool),
    /// 内核拦截,不进 `update`——`SendToAgentContext` 在 `files::update` 里
    /// 算出项目内相对路径后,经 `emit` 送回内核顶层,由 `App::update` 按当前
    /// locale 拼模板文本并调 `term_paste` 写进当前激活的 agent 终端(真正的
    /// PTY 句柄只有内核有)。同时携带落库所需的 `is_dir`/`relative`。
    RequestSendToAgentTerminal {
        is_dir: bool,
        /// 项目内相对路径,目录不带尾部 `/`,落库用。
        relative: String,
    },
    /// 内核拦截,不进 `update`——真正的系统剪贴板写入需要 `main.rs` 的
    /// `Clipboard` 句柄,`update()` 拿不到(见设计文档"关键语义确认")。
    CopyPath(PathBuf, PathKind),
    /// 右键菜单"用外部软件打开":以系统默认 App 打开右键目标(文件或
    /// 目录)。`app_name` 为 `None` 时退回系统默认打开方式(`open <path>`,
    /// 不带 `-a`);为 `Some` 时按 `App::external_apps` 配置用指定 App 打开
    /// (`open -a <app> <path>`)。`app_name` 在组装菜单时就按扩展名查好、随
    /// 消息一并带下来(菜单项的展示文案也用它,见 `context_menu_spec`),
    /// `files::update` 的 handler 直接按它 spawn,不重复查配置。
    OpenWithDefault(PathBuf, Option<String>),
    /// `OpenWithDefault` 后台等 `open` 退出后的结果(项目 id,路径,App 名,结果)。
    /// `open -a 不存在的App` 的失败在进程退出之后才出现,所以必须等退出状态。
    OpenWithDefaultDone(
        i64,
        PathBuf,
        Option<String>,
        Result<(), crate::external_apps::OpenError>,
    ),
    Copy(PathBuf, bool),
    Paste(PathBuf),
    PasteDone(i64, Result<PathBuf, crate::fs::ProjectFsError>),
    DeleteRequest(PathBuf, bool),
    DeleteConfirm,
    DeleteCancel,
    /// 批量删除确认框"确定"后,`DeleteConfirm` 异步完成全部删除的收尾:
    /// 首字段保留发起项目 id,避免切换项目后把结果投递到当前项目。
    /// `results` 逐项是 `(被删路径, 删除结果)`,成功项刷新其父目录、失败项
    /// 推 Toast。比单文件时用的 `OpDone` 更通用——多文件会命中多个不同父目录,
    /// 需要逐个刷新。
    DeleteApplied(i64, Vec<(PathBuf, std::result::Result<(), String>)>),
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
    TextInputMenuOpen(crate::adapter::TextInputTarget),
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
    /// 完整路径(main.rs 每次原生 `DroppedFile` 事件只带一个路径,一次拖入
    /// 多个文件会拆成多条独立的 `DroppedFile`/`FileDrop`,`paths` 恒长度 1;
    /// `Vec` 类型只是留了口子,不是当前真实会出现多元素的场景),`target`
    /// 是落点目录。由 main.rs 在原生 `DroppedFile` 事件层命中测试后发出。
    /// 单文件(今天恒成立的情况)不立即移动,弹 `PendingMove` 确认框
    /// (见其文档);那条从未被触发过的"多文件"分支保留旧的直接移动行为
    /// 兜底。
    FileDrop {
        paths: Vec<PathBuf>,
        target: PathBuf,
    },
    /// 一次拖入(多文件兜底分支)/`Message::MoveConfirm` 的异步移动结果:
    /// 成功时刷新 `target` 目录,失败时推 Toast(outbox)。
    FileDropDone(i64, PathBuf, Result<(), String>),
    /// 拖拽移动确认框"新名称"输入框内容变化(`byteui::form::input_text`
    /// 的 `on_input`)。
    MoveNameInput(String),
    /// 拖拽移动确认框"到目录"输入框内容变化——手动改写路径文本,或
    /// `MoveDirBrowse` 选完目录后回填。
    MoveDirInput(String),
    /// 拖拽移动确认框"到目录"字段旁边的"..."浏览按钮:要弹原生目录选择器,
    /// `files::update()`(纯状态转换,拿不到原生对话框能力)自己做不了——它往
    /// `ws_state.host` 提一个 `HostRequest::PickDirectory`(起始目录 = 当前草稿),
    /// host 弹完对话框后用 `on_picked` 转发一条 `MoveDirInput` 回填草稿。
    MoveDirBrowse,
    /// 拖拽移动确认框"确定":真正提交移动(改名 + 改目标目录一起生效,见
    /// `PendingMove`/`crate::fs::move_item_to`)。校验失败(名字为空/
    /// 含路径分隔符、目标目录不存在)置 `move_error` 并把对话框放回去
    /// (不关闭,同行内编辑框"已存在同名项"校验失败时的既有口径);校验通过
    /// 异步落盘,完成后复用 `FileDropDone` 刷新目标目录。
    MoveConfirm,
    /// 拖拽移动确认框"取消"/点击框外空白(见 app.rs 顶层浮层的 `dismiss`
    /// 层,同 `DeleteCancel` 的既有套路):整场拖拽作废,不做任何磁盘改动,
    /// 只清空 `pending_move`。
    MoveCancel,
    /// 树内行(不含目录箭头,箭头见 `Message::Toggle`)被按下:武装拖拽,
    /// 同时保留原有点击语义——单击只选中,不展开/折叠目录、不打开文件
    /// 预览,那两个效果都推迟到双击才触发,见 `TreeRowDoubleClick`。这条
    /// 跨内核边界:由 `App::update` 拦截转发,`files::update()` 收到会
    /// `unreachable!`。
    TreeRowPress {
        path: PathBuf,
        is_dir: bool,
    },
    /// 树行被双击:文件打开预览;目录展开/折叠(与点箭头的 `Message::
    /// Toggle` 等效,给没精确点在箭头上的用户一个整行都能双击的入口)。
    /// 单击只选中(见 `TreeRowPress`)。文件那支要跨到 `Message::
    /// PreviewOpenPath`,该面板本身不渲染预览;这条整体跨内核边界:由
    /// `App::update` 按 `is_dir` 分派(目录转发 `Message::Toggle`,文件转
    /// `PreviewOpenPath`),`files::update()` 收到会 `unreachable!`。iced
    /// `MouseArea::on_double_click` 的事件时序是 `on_press -> on_release ->
    /// on_press -> on_double_click -> on_release`,第二次按下仍会重新武装一
    /// 次拖拽(`TreeRowPress`)、松开仍会照常触发 `TreeDragEnd(false)` 选中
    /// 同一路径——都是幂等操作,不需要互斥。
    TreeRowDoubleClick {
        path: PathBuf,
        is_dir: bool,
    },
    /// 树内拖拽悬停到某个目录行(项目内部移动专用,与外部 OS 拖拽的
    /// `FileDragHover` 分开走——内部拖拽还要校验落点合法性,见
    /// `is_valid_move_target`,非法落点不高亮、不记为待定目标)。
    TreeDragOver(PathBuf),
    /// 鼠标进入/离开某个树行:`Some(路径)` = 进入(行 hover 背景,同右键
    /// 菜单项 hover 的 `TAB_HOVER` 底色),`None` = 离开。行 `MouseArea`
    /// 的 `on_enter`/`on_exit` 上报,见 `WorkspaceState::tree_hover` 文档
    /// (为什么不能靠 `button::Status::Hovered`)。不跨内核边界,
    /// `files::update()` 直接处理。
    TreeRowHover(Option<PathBuf>),
    /// 树内拖拽松开左键:main.rs 全局 `MouseInput::Released` 在
    /// `App::dragging_tree_item()` 为真时发出。这条本身跨内核边界(内核要
    /// 拿 `App::last_cursor` 和武装时的 `press_pos` 比对,算出下面
    /// `TreeDragEnd` 要带的 `confirmed`),由 `App::update` 拦截转发,
    /// `files::update()` 收到会 `unreachable!`。
    /// 树内拖拽松开左键:main.rs 全局 `MouseInput::Released` 在
    /// `App::dragging_tree_item()` 为真时发出。`shift` 是松开瞬间的 Shift
    /// 修饰键状态(由 `window_events.rs` 在原生事件层读 `modifiers` 带下来)——
    /// 真正点选那一下要不要用它做区间多选,由下游 `TreeDragEnd` 决定。这条
    /// 本身跨内核边界(内核要拿 `App::last_cursor` 和武装时的 `press_pos`
    /// 比对,算出下面 `TreeDragEnd` 要带的 `confirmed`),由 `App::update`
    /// 拦截转发,`files::update()` 收到会 `unreachable!`。
    TreeDragRelease {
        shift: bool,
        /// 松开瞬间是否按住 Cmd(macOS)/Ctrl(非 mac)——用于点选那一下做
        /// 不连续多选切换(`select_toggle`),见 `TreeDragEnd` 文档。
        command: bool,
    },
    /// 树内拖拽真正的收尾:`confirmed` = 松开时光标是否已越过
    /// [`crate::app`] 的确认阈值(同 tab/rail 拖拽的既有手法);`shift` = 松开
    /// 时是否按住 Shift——仅当 `!confirmed`(这只不过是一次单击)且 `shift` 为真
    /// 时,点选走区间多选(`select_shift_range`),否则单选被点项。只有
    /// `confirmed` 且有合法待定目标才提交移动(复用 `FileDrop` 的
    /// `move_item` + `FileDropDone` 收尾);`!confirmed` 时这其实只是一次
    /// 单击,目录的展开/折叠推迟到这里才执行——`TreeRowPress` 按下不再
    /// 立即展开,避免布局位移把静止光标误判成"划到了旁边那一行"
    /// (2026-09 用户实测反馈:点一个目录就'进入拖拽态',点另一个目录会把
    /// 前者错误地移进去)。
    TreeDragEnd {
        confirmed: bool,
        shift: bool,
        /// 松开瞬间是否按住 Cmd(macOS)/Ctrl(非 mac)——仅当 `!confirmed`
        /// (这只不过是一次单击)时用来做不连续多选切换,否则忽略。见
        /// `Message::TreeDragRelease` 对 `command` 的说明。
        command: bool,
    },
}
