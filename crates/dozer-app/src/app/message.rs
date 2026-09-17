//! `Message` 枚举(130 变体的顶层消息协议)+ 各弹出菜单/选择器的浮层状态
//! 结构体。Phase 2 结构重组时从 `app.rs` 抽出,类型与语义保持原样。

use std::path::PathBuf;

use dozer_core::protocol::{AgentKind, AgentState, ProjectInfo, SessionInfo};

use crate::chrome::homespace::{self, HomeRecentConversation, HomeRecentFile};
use crate::extensions::{
    browser, conversations, database, file_history, files, footbar, git_log, project, search, ssh,
    todo, usage,
};
use crate::git_watch;
use crate::term::terminal;
use crate::transcript::ReviewEntry;
use crate::workspace::{PickerLaunch, RestorePayload, ReviewSource};

use super::layout::{Divider, ProjectId, RowDivider, TabGroup};
use super::state::{HoverId, PanelKind, Side, TextInputTarget};

#[derive(Debug, Clone)]
pub enum Message {
    /// 终端聚焦时的键盘/IME 输入字节（已经过 `keymap` 翻译）。直接写给
    /// 当前激活 tab 对应的 daemon 会话（`client.write`）——不再本地
    /// echo，回显完全走 PTY 真实回路（daemon → attach 流 → `TermOutput`）。
    TermInput(terminal::TermTarget, Vec<u8>),
    /// IME 组字预览(未提交):`None` 表示组字结束/取消,清空预览。不发字节
    /// 给 PTY——只是渲染层叠加,`term_view` 画在光标位置(见其 `draw`)。
    TermImePreedit(terminal::TermTarget, Option<String>),
    /// attach 事件流转发来的输出字节，`usize` 是 tab 的稳定 id
    /// （`SessionTab::tab_id`，不是 vec 位置——关闭 tab 会移动位置，
    /// 但 id 不变，事件流路由必须认 id）。首字段的项目归属见 [`ProjectId`]
    /// ——`tab_id` 只在单个项目内唯一,跨项目会撞。
    TermOutput(ProjectId, usize, Vec<u8>),
    /// 对应 tab 的会话已退出（PTY 子进程退出或 daemon 断连）。
    SessionExited(ProjectId, usize),
    /// attach 流转发来的 agent 状态变更（tab_id, 状态, 该会话最新 transcript 路径）。
    AgentStateChanged(ProjectId, usize, AgentKind, AgentState, Option<String>),
    /// hook 事件驱动的 Agent 卡片元信息刷新结果(tab_id, LLM 型号,
    /// permission mode,transcript 最后活动摘要(兜底"当前工作内容",见
    /// `workspace::agent_card`),工作区分支/脏标覆盖)。`None` 字段表示
    /// 这次没有新值,落地时不覆盖已有值(见
    /// `workspace::apply_agent_card_refresh`)。
    AgentCardRefreshed(
        ProjectId,
        usize,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<crate::workspace::WorkspaceGitInfo>,
    ),
    /// 会话审阅:解析完成（来源, 追加标记, 条目 / 错误文案）。`append`
    /// 为 `true` 时新条目应追加进现有 `entries`(详情"加载更多"),`false`
    /// 时整段替换(首次打开/活会话刷新)。
    ReviewLoaded(
        ProjectId,
        ReviewSource,
        bool,
        Result<Vec<ReviewEntry>, String>,
    ),
    /// `GetTodoDetail` 异步结果:`usize` 是打开弹窗时记录的卡片下标(用来
    /// 校验弹窗还开着同一个任务,不是用 id 找——`items()` 下标和渲染时
    /// 用的下标必须一致,同 `todo::Message` 全线用下标寻址任务的既有约定)。
    TodoDetailLoaded(usize, Vec<dozer_core::protocol::TurnRecord>),
    /// Usage 面板的全部消息,内核只转发不解读——见 `extensions::usage::Message`。
    Usage(usage::Message),
    /// 对话(Conversations)面板列表侧的全部消息,内核只转发不解读——见
    /// `extensions::conversations::Message`。其中 `SessionOpen`/
    /// `DetailLoadMore`/`Hover`/`TextInputMenuOpen` 四种由内核直接拦截处理,
    /// 不会到达 `conversations::update`。
    Conversations(conversations::Message),
    /// 一次"删除项目"执行完成。`Vec<String>` 是文件系统步骤各自独立的
    /// 失败原因(空 = 全部成功);dozerd 侧两步(登记/agent 历史)任一失败
    /// 时这里只会收到那一条错误。项目对应的 tab 在发起删除时已经关掉,
    /// 这个消息到达时已经没有面板可以展示状态,统一走 `self.daemon_error`
    /// (同 `project_tab_opened` 失败路径的既有做法)。
    ProjectDeleteDone(Vec<String>),
    /// 切换当前显示的 tab（这里的 `usize` 是 vec 位置——用户点击的是
    /// "屏幕上第几个 tab"，跟稳定 id 是两回事）。**仅限左侧终端 tab 栏本身
    /// 的按钮**发这条消息——`select_tab()` 顺带把 `tab_drag` 武装成"这一
    /// 页签正被按住",随后光标划过 tab 栏任意条目就会触发换位
    /// (`tab_drag_move`)。任何不是 tab 栏本身、但也想"选中某个 tab"的地方
    /// (比如 Agent 面板右侧卡片列表)必须发 `SelectTabNoDrag`,否则会在
    /// 无关点击后意外武装拖拽状态机,松手前只要划过 tab 栏就会错误换位
    /// (2026-08-17 修的一个真实 bug)。
    SelectTab(usize),
    /// 语义同 `SelectTab`(选中 + 路由键盘焦点),但**不武装拖拽状态机**。
    /// 给"不是 tab 栏本身、但也要切换 tab"的调用方用(目前只有 Agent 面板
    /// 右侧卡片列表 `agent_card`)。
    SelectTabNoDrag(usize),
    /// 关闭 tab = 结束会话：中断转发任务并 kill daemon 侧会话（P1e 验收
    /// 反馈裁决：重开 app 只恢复"关 app 时还开着"的 tab，已关的不还魂）。
    /// "会话存活"保的是关 app/崩溃不掉会话——退 app 才是 detach。
    CloseTab(usize),
    /// Agent 面板"＋"按钮:开/关 agent 选择菜单。
    AgentPickerToggle,
    /// agent 选择菜单:点击菜单外/Esc,关闭不建会话。
    AgentPickerClose,
    /// agent 选择菜单:选中一项(`Agent(None)` = 纯 Shell,`Agent(Some(a))`
    /// = 新建会话后自动键入该 agent 的 CLI 名字,`Git` = 项目根开 git shell)。
    AgentPickerSelect(PickerLaunch),
    /// 顶栏"＋新增项目"按钮:开/关最近项目选择菜单。
    ProjectAddMenuToggle,
    /// 最近项目选择菜单:点击菜单外/Esc,关闭不做任何事。
    ProjectAddMenuClose,
    /// 新建会话完成 attach（tab_id、`SessionInfo`、初始快照、picker 选定的
    /// 目标 agent）。启动时的恢复走同步的 `bootstrap`，不需要过一次消息循环。
    /// 末尾的 `Option<AgentKind>` 是 picker 选择时**已经确定**要键入的 agent
    /// ——不能等 `info.agent`：那个字段在 daemon 侧要靠 hook 上报才会从
    /// 默认值改成真实 agent,而 hook 上报必然晚于 agent CLI 进程启动瞬间
    /// 发出的终端探测查询(如 opencode 的 OSC 10/11),用 `info.agent` 判断
    /// 会永远赶不上趟。SSH 等不经 picker 的 attach 路径传 `None`。
    TabAttached(ProjectId, usize, SessionInfo, Vec<u8>, Option<AgentKind>),
    /// Todo 面板的全部消息(派发到已有/新建会话除外——那两条内核直接
    /// 拦截处理,见 `update()` 对应分支),内核只转发不解读——见
    /// `extensions::todo::Message`。
    Todo(todo::Message),
    /// 数据库面板的全部消息。`TestConnectionResult` 特化分支内核直接拦截
    /// 处理(带 `project_id`,不能按当前聚焦项目路由),其余走通配分发。
    Database(database::Message),
    /// 文件树右键"搜索"弹窗的全部消息,内核只转发不解读——见
    /// `extensions::search::Message`。`SearchResults`(带 `project_id`)按
    /// 项目路由,其余(弹窗常驻 UI 交互)投给当前聚焦项目。
    Search(search::Message),
    /// 终端 pane 像素尺寸变化换算出的新网格尺寸；对所有 tab 生效
    /// （包括当前不可见的），保证切换 tab 时尺寸已经是最新的。
    /// 共享终端 pane 与 SSH 面板内嵌终端并行重算:两个 pane 几何不同,
    /// 必须带着各自的网格一起下发,否则 SSH 终端会沿用共享终端的列数。
    PaneResized {
        cols: u16,
        rows: u16,
        ssh_cols: u16,
        ssh_rows: u16,
    },
    /// 按下某条分隔线,记录"正在拖哪条"(main.rs 后续 CursorMoved 靠这个
    /// 状态决定要不要继续转发拖拽)。构造方为 `divider_bar` 的 `on_press`。
    ColumnDragStart(Divider),
    /// 拖拽中:当前窗口逻辑宽 + 光标逻辑 x(main.rs 换算好传入,`update()`
    /// 统一算+夹取,不与 main.rs 分摊裁剪逻辑)。构造方为 main.rs 的
    /// `CursorMoved` 续传。
    ColumnDrag {
        window_width: f32,
        logical_x: f32,
    },
    /// 松开左键,结束拖拽并触发写盘。构造方为 main.rs 的
    /// `MouseInput{Released}` 分支。
    ColumnDragEnd,
    /// 按下某条纵向(上下)分隔线,记录"正在拖哪条"。构造方为
    /// `horizontal_divider_bar` 的 `on_press`。
    RowDragStart(RowDivider),
    /// 纵向拖拽中:当前窗口逻辑高 + 光标逻辑 y(main.rs 换算好传入)。
    RowDrag {
        window_height: f32,
        logical_y: f32,
    },
    /// 松开左键,结束纵向拖拽并触发写盘。
    RowDragEnd,
    /// 拖拽中,光标进入了 `group` 组的第 `index` 个 tab 上空——拖起的源项
    /// 应移动到这个目标位(换位)。构造方为该组每个 tab 顶层的
    /// `MouseArea::on_move`(仅在 `tab_drag` 命中本组时挂载)。按住页签＝
    /// 准备拖的来源,由各选中处理(`SelectTab`/`PreviewSelectTab`/
    /// `ProjectTabSwitch` 及浏览器 `SelectTab`)在按住瞬间把 `tab_drag` 置位。
    TabDragMove {
        group: TabGroup,
        index: usize,
    },
    /// 松开左键,结束页签拖拽。构造方为 main.rs 的 `MouseInput{Released}`
    /// 分支;项目页签组顺带把新顺序写盘。
    TabDragEnd,
    /// 图标栏面板拖拽,光标进入了 `side` 栏第 `index` 个位置——同栏内是
    /// 重排,跨栏是记录悬停目标。构造方为该栏每个按钮顶层的
    /// `MouseArea::on_move`(仅在 `rail_drag` 命中时挂载)。按住图标＝准备
    /// 拖的来源由 `panel_select` 在按住瞬间武装(`self.rail_drag` 置位)。
    RailDragMove {
        side: Side,
        index: usize,
    },
    /// 松开左键,结束图标栏面板拖拽。构造方为 main.rs 的
    /// `MouseInput{Released}` 分支;跨栏移动此时才提交并写盘。
    RailDragEnd,
    /// Todo 面板拖拽排序结束:松开左键,把新顺序写盘。构造方为 main.rs 的
    /// `MouseInput{Released}` 分支,同 `TabDragEnd`(页签拖拽)那套。拖拽中
    /// 的 `DragMove` 由卡片外层 `MouseArea::on_move` 直接发 `Todo::DragMove`
    /// (走 `Message::Todo` 通道),不需要顶层变体——这里只收尾。
    TodoDragEnd,
    /// 图标栏点击选中某个面板——不区分左右栏,`panel_select` 内部按
    /// `RailLayout::side_of` 查它当前挂在哪条栏。
    PanelSelect(PanelKind),
    /// 文件预览面板右上角"收起/展开文件树"按钮:翻转
    /// `dims.files_tree_collapsed`。只影响文件树列表子栏的显隐,不触碰
    /// `files_split` 比例,展开时按原比例恢复。
    ToggleFileTreeCollapse,
    /// 某两栏面板的列表列收起/展开按钮:翻转该面板对应的 `dims.*_list_collapsed`。
    /// 与 `ToggleFileTreeCollapse` 同一套语义——只改列表子栏显隐、不触碰
    /// split 比例,展开时按原宽度恢复。`PanelKind` 只能是六个两栏面板之一
    /// (Project/Todo/Database/Ssh/Agent/Conversations),不是它们则忽略。
    TogglePanelListCollapse(PanelKind),
    /// 任意 iced 原生输入框(`text_input`/`text_editor`)的右键菜单:在某输入
    /// 框上右键触发(由 byteui 的 `context_menu::wrap` 接线)。携带被右键的
    /// 输入目标,用于本次右键时把焦点移到该输入,让菜单的复制/粘贴作用于
    /// 它。定位坐标复用 `files.last_right_click`(main.rs 右键时已写入)。
    TextInputMenuOpen(TextInputTarget),
    /// 输入框右键菜单关闭(点遮罩 / 按 Esc)。
    TextInputMenuClose,
    /// 输入框右键菜单的动作项:剪切/复制/粘贴/全选。`App::update` 只负责把
    /// 菜单关掉;真正把动作作用到聚焦输入框的是 main.rs——把这些消息合成回
    /// 对应的 ⌘/Ctrl+`x`/`c`/`v`/`a` 键盘事件,喂给下一帧 `interface.update`,
    /// 复用 iced 原生的剪贴板/光标插入逻辑(见 `dispatch`/`editor`)。密码框
    /// 会禁用剪切/复制(同原生快捷键)。
    TextInputMenuCut,
    TextInputMenuCopy,
    TextInputMenuPaste,
    TextInputMenuSelectAll,
    /// 图标栏按钮 hover 进入/离开:进入带 `Some(id)`,离开带 `None`,
    /// 任意按钮的 hover 进入/离开:带按钮标识 `HoverId` 与 `true`/`false`,
    /// 驱动该按钮图标/背景/边框颜色的平滑过渡动画(见 `App::set_hover`/
    /// `advance_hover_anims`/`hover_progress`)。取代原 `RailHover`/
    /// `TopbarHover`/`HomeHover` 三个专为各自按钮写的变体。
    Hover(HoverId, bool),
    /// 点击放大态背后的变暗遮罩:退出放大。
    MaximizeClose,
    /// 双击顶栏空白处(去掉原生标题栏后,原生"双击标题栏缩放窗口"手势
    /// 只在系统认为仍是"标题栏"的那一小条区域生效;顶栏其余空白靠这条
    /// 消息手动补上同样的行为)。真正调用 `window.set_maximized(...)`
    /// 的是 main.rs——`App` 不持有 `winit::window::Window` 句柄,这里只
    /// 记一个待处理标记,由 `take_pending_zoom_toggle` 供 main.rs 轮询。
    TopBarDoubleClick,
    /// 点击顶栏 Dozer(带 home 图标)按钮:进入首页落地页(`AppPage::Home`)。
    /// 打开/切换项目会自动退回 `Workspace`(见 `ProjectTabOpened`/
    /// `ProjectTabSwitch`/`ProjectSelect`)。
    TopBarHome,
    /// 什么也不做。专门给"就地吃掉事件、不让它冒泡到父级"的 `MouseArea`
    /// 用(`MouseArea::on_press`/`on_scroll` 一旦有消息就会
    /// `shell.capture_event()`)。目前唯一用处:放大态浮层里罩在放大内容
    /// 之上的那层——不吃掉的话,点在审阅正文/卡片空白等"自己不消费点击"
    /// 的地方会穿到外层 dim 遮罩的 `MaximizeClose`,一点正文就退出放大;
    /// 滚轮同理会穿到底层那块看不见的终端 canvas 上,把它的历史滚走
    /// (Fix round 2 #4)。
    Noop,
    /// daemon 不可用（启动连接失败，或某次会话操作失败）的错误文案，
    /// 终端区以 RED 文案展示。
    DaemonError(String),
    /// 终端滚轮：视口向历史方向（正数）/活动区方向（负数）滚动的行数。
    /// 只作用于当前激活 tab（滚轮事件来自它的 canvas）。
    TermScroll(terminal::TermTarget, i32),
    /// 终端 tab 栏溢出下拉开关：点 V 按钮切换；打开时把 `App::last_cursor`
    /// 记进 `Workspace::term_tab_overflow_anchor` 作为悬浮定位锚点。
    TermTabOverflowToggle,
    /// 终端 tab 栏溢出下拉：点击外部区域关闭。
    TermTabOverflowDismiss,
    /// 文件预览 tab 栏溢出下拉开关,语义同 `TermTabOverflowToggle`。
    PreviewTabOverflowToggle,
    /// 文件预览 tab 栏溢出下拉:点击外部关闭。
    PreviewTabOverflowDismiss,
    /// 终端左键按下：在视口格 `(col, row)` 起新选区（`right` = 按点在
    /// 格子右半）。
    TermSelStart {
        target: terminal::TermTarget,
        col: usize,
        row: usize,
        right: bool,
    },
    /// 终端拖拽：选区末端更新到视口格 `(col, row)`。
    TermSelUpdate {
        target: terminal::TermTarget,
        col: usize,
        row: usize,
        right: bool,
    },
    /// ⌘V 粘贴剪贴板文本：按会话的 bracketed paste 模式决定是否包裹
    /// `ESC[200~`/`ESC[201~` 后写入 daemon。
    TermPaste(terminal::TermTarget, String),
    /// 预览:打开本地文件为新 tab(路径已由入口侧确认存在,来自项目树点击/
    /// 会话恢复;预览面板本身已不再有"打开文件…"按钮或地址栏)。
    PreviewOpenPath(PathBuf),
    /// 预览:切换 tab(vec 位置).
    PreviewSelectTab(usize),
    /// 预览:关闭 tab(vec 位置).
    PreviewCloseTab(usize),
    /// 预览:点 tab 上的预览/代码切换按钮(只对 `wry_toggle_eligible` 的文件
    /// 画)——`usize` 是 vec 位置,交给 `Workspace::preview_pane_toggle_render_mode`
    /// 落盘 + 切渲染路径。Files 预览面板。
    PreviewToggleRenderMode(usize),
    /// 原生预览 tab 的 `text_editor::Action`,`usize` 是 `PreviewTab.id`。由
    /// `main.rs` 的 dispatch 直接转发给 `App::preview_tab_editor_event`(剪贴
    /// 板由 iced 运行时自己处理,不需要像 vendored `iced-code-editor` 那样手动
    /// 拆 `Task` 桥接)。
    PreviewEditorEvent(usize, iced_widget::text_editor::Action),
    /// 原生预览就地可写后的 ⌘S:把 `kind` 指向面板(`Files`/`Project`)当前激活
    /// 原生 tab 的改动保存到磁盘(仅脏的原生 tab 动作;见
    /// `Workspace::preview_pane_save_active`)。携带 `PanelKind`(可由 main.rs
    /// `FocusIntent::Preview` 直接转发,不必频繁 preview↔panel 双枚举映射)。
    PreviewSaveActive(PanelKind),
    /// 原生预览 tab 就地敲 Tab(裸 Tab、非 ⌘/⌃/⌥ 组合):iced 官方
    /// `text_editor` 默认 Binding 对 Tab 完全不产生动作,必须在这里作为一条
    /// 编辑动作手工插 `\t`(走 `CodeView::perform` 的原生 Content 光标,选区
    /// 替换/撤销都能对)。`kind` 指向当前真正聚焦的原生编辑器所在面板,同
    /// `PreviewSaveActive` 一跳区分、不做 preview↔panel 双枚举映射。构造/调用
    /// 处是 main.rs 原生预览闸门;先拦下,没命中的键才放行给 iced。
    PreviewTabInsertTab(PanelKind),
    /// 原生预览撤销(⌘Z,非 shift):把 `kind` 面板当前激活原生 tab 的 buffer
    /// 回退到上一条编辑命令前状态(见 `Workspace::preview_pane_undo_active` /
    /// `CodeView::undo`)。官方 `text_editor` 没有任何 undo/redo API,历史是
    /// 应用层按编辑命令粒度记的整文本快照栈(code_editor 模块"已知取舍"),因
    /// 此撤销序列不是逐键、光标只近似还原。`kind` 语义同 `PreviewSaveActive`。
    PreviewUndoActive(PanelKind),
    /// 原生预览重做(⌘⇧Z):重放被 `PreviewUndoActive` 撤掉的最后一条编辑,
    /// 语义同 `CodeView::redo`。`kind` 语义同 `PreviewSaveActive`。
    PreviewRedoActive(PanelKind),
    /// 原生预览打开 File-Find(⌘F)。`kind` 指向 `Files` 或 `Project` 面板——
    /// File-Find 对两个面板的原生编辑 tab 语义相同(见
    /// `Workspace::preview_find_open`,⌘F 已开时是重聚焦的 no-op)。跟
    /// `PreviewSaveActive` 一样用 `PanelKind` 一跳区分面板,不强做 preview↔panel
    /// 双枚举映射。
    PreviewFindOpen(PanelKind),
    /// 同 `PreviewFindOpen`(⌘R),但替换行默认展开——查询框前的圆盘箭头也
    /// 展示这个展开态,`PreviewFindReplaceToggle` 再手动翻转。
    PreviewFindOpenWithReplace(PanelKind),
    /// 查询框前的圆盘箭头点击:手动翻转替换行展开/收起,不受 ⌘F/⌘R 影响。
    PreviewFindReplaceToggle(PanelKind),
    /// File-Find 关闭(输入框 × / Esc / 切走文件)。`kind` 语义同
    /// `PreviewFindOpen`。
    PreviewFindClose(PanelKind),
    /// File-Find 输入框每键的 query 落定:同步到面板并让面板当场重算、跳首个命中。
    PreviewFindText(PanelKind, String),
    /// File-Find 下一条 / 上一条。
    PreviewFindGo(PanelKind, bool),
    /// File-Find 大小写敏感开关(`true`=逐字严格、`false`=ASCII 大小写折叠)——
    /// 点条上「Aa」切换钮落定的方向。只翻当轮会话的语义,不改全局默认。
    PreviewFindCase(PanelKind, bool),
    /// File-Find 条的「替换为」输入框每键落定(只写 `FindState::replacement`
    /// 草稿,不触发任何替换;真正动作在点「替…」按钮时发生)。
    PreviewFindReplacement(PanelKind, String),
    /// File-Find 条「替换当前」:把本轮 `current` 指着的那一处清掉换成替换框
    /// 文本。照 Enter/⌘S 外的普通打字语义,只改**原生 buffer 并标脏**等待用户
    /// ⌘S 落盘——替换不隐式写盘([CLAUDE.md 裁决]预览优先渲染/不可逆动作留给
    /// 显式保存)。
    PreviewFindReplaceCurrent(PanelKind),
    /// File-Find 条「替换全部」:与 `PreviewFindReplaceCurrent` 同一 buffer-only
    /// 语义,只是把这轮每一处命中一次性全改、同样标脏等 ⌘S。
    PreviewFindReplaceAll(PanelKind),
    /// Project 面板右配对预览:打开本地文件为新 tab,语义同 `PreviewOpenPath`。
    ProjectPreviewOpenPath(PathBuf),
    /// Project 面板右配对预览:切换 tab(vec 位置)。
    ProjectPreviewSelectTab(usize),
    /// Project 面板右配对预览:关闭 tab(vec 位置)。
    ProjectPreviewCloseTab(usize),
    /// Project 面板右配对预览:预览/代码切换按钮,语义同 `PreviewToggleRenderMode`。
    ProjectPreviewToggleRenderMode(usize),
    /// Project 面板右配对预览 tab 栏溢出下拉开关,语义同上
    /// (`PreviewTabOverflowToggle`)。
    ProjectPreviewTabOverflowToggle,
    /// Project 面板右配对预览 tab 栏溢出下拉:点击外部关闭。
    ProjectPreviewTabOverflowDismiss,
    /// Project 面板右配对预览的原生 `text_editor::Action`，语义同
    /// `PreviewEditorEvent`。
    ProjectPreviewEditorEvent(usize, iced_widget::text_editor::Action),
    /// 浏览器面板的全部消息,内核只转发不解读——见
    /// `extensions::browser::Message`。
    Browser(browser::Message),
    /// 浏览器 webview 渲染进程报回页面 HTML 标题(id = webview/tab id,
    /// title = `document.title`)。浏览器面板有首页全局(`home_browser`)与
    /// 工作区(`ws.browser`)两套、且同时只有一套活跃,按 `is_home()` 路由;
    /// main.rs 的 IPC 分支不知道自己在哪套里,故用独立顶层消息,不开新
    /// `browser::Message::TitleLoaded` 包装。
    BrowserTitle(usize, String),
    /// 浏览器 webview 渲染进程报回网页内超链接/`window.open` 自行导航后的
    /// 真实地址(id = webview/tab id,url = webview 加载到的 URL)。路由口径
    /// 同 `BrowserTitle`:首页全局/工作区两套浏览器按 `is_home()` 分派,而不
    /// 走 `Message::Browser(..)`(那会只落到聚焦工作区)。
    BrowserNavigated(usize, String),
    /// 网页内 `target="_blank"`/`window.open` 请求新窗口。wry 的
    /// `new_window_req` 分支不知道目标归属哪套浏览器,同 `BrowserTitle` 按
    /// `is_home()` 路由,在其内 `open_url`(总开新 tab,不复用激活 tab)。
    BrowserNewWindow(String),
    /// 项目:切换到最近项目。
    ProjectSelect(i64),
    /// 项目:"打开项目…"→ rfd 文件夹选择(main.rs 执行),选中后回送
    /// `ProjectTabOpen`。顶栏"＋"与项目栏那颗"打开项目…"按钮**共用**这
    /// 一条入口——两者都是"我要打开一个项目",都必须落成**新增页签**。
    ///
    /// 此前项目栏那颗按钮走的是另一条 `ProjectPickFolder`→`ProjectOpened`
    /// 的**就地改写**路径(`close_all_tabs_for_switch` + `adopt_project`),
    /// 会把当前聚焦项目的终端会话全杀掉——直接违反设计文档 §2"只有用户
    /// 显式关闭一个页签,才结束该项目下的会话"。整条就地改写路径连同
    /// `retarget_active_slot` 已随之删除,不留第二条会杀会话的打开入口。
    ProjectTabPickFolder,
    /// 单颗"＋"按钮:main.rs 弹 rfd 模态(根目录在项目根),选完后按
    /// `is_dir()` 判 `LinkKind` 回送 `project::Message::LinkAdd`。
    ProjectLinkPick(project::links::LinkTarget),
    /// Project 面板链接行右键菜单关闭(点遮罩 / 按 Esc)。
    ProjectLinkContextMenuClose,
    /// Todo 分类树节点右键菜单关闭(点遮罩 / 按 Esc)。
    CategoryContextMenuClose,
    /// 分类选择器("移动到..." / 任务挂分类)浮层关闭(点遮罩 / 按 Esc)。
    CategoryPickerClose,
    /// 选择器里点了某一项:`None` = "未分类"(仅 `Todo` target 下有效,
    /// `Category` target 选"未分类"表示挪到顶层)。
    CategoryPickerSelect(Option<i64>),
    /// 数据库面板数据源树 header 行右键菜单关闭(点遮罩 / 按 Esc)。
    DatabaseSourceContextMenuClose,
    /// 项目页签:把某路径作为**新页签**打开(不动任何已存在页签的内容)。
    ProjectTabOpen(PathBuf),
    /// 项目页签:`ProjectTabOpen` 异步完成(daemon upsert 结果 + 最近列表)。
    /// `None` = 这次打开失败,只报错、不改任何页签状态。
    ProjectTabOpened(Option<ProjectInfo>, Vec<ProjectInfo>),
    /// H0 项目中心:`Message::TopBarHome` 发起的异步刷新完成(最近改动的文件、
    /// 最近的对话两份列表;D4)。
    HomeRecentsLoaded(Vec<HomeRecentFile>, Vec<HomeRecentConversation>),
    /// 首页左图标栏:切换 `HomeLeftView`(项目列表/Recents)。首页没有
    /// collapse 概念,恒有一个 pane 显示,不像工作区 `PanelSelect` 那样
    /// 需要处理"点已选中图标收起面板区"的分支。
    HomeLeftIconSelect(homespace::HomeLeftView),
    /// 首页"项目列表" pane:点"更多..."再展开 5 个项目。纯面板内状态变更,
    /// 无 IO;只有还有更多项目时才渲染那颗按钮(见
    /// `homespace::home_project_list_view`)。
    HomeMoreProjects,
    /// 首页项目列表搜索框草稿变化(iced `text_input::on_input`)。
    HomeProjectSearchInput(String),
    /// 回车 / 点搜索按钮:把草稿落成生效的 `home_project_search` 过滤词,
    /// 同时把翻页重置回第 1 页(过滤后结果变少,停在旧页码没有意义)。
    HomeProjectSearchSubmit,
    /// 首页右图标栏:切换 `HomeRightView`(目前只有 Browser)。
    HomeRightIconSelect(homespace::HomeRightView),
    /// 首页全局浏览器面板的全部消息,内核只转发不解读——见
    /// `extensions::browser::Message`。路由到 `app.home_browser`,
    /// `project_id` 恒传 `None`;与工作区 `Message::Browser` 路由到
    /// `ws.browser` 是两条独立路径,互不影响。
    HomeBrowser(browser::Message),
    /// 项目页签:点已存在的页签 → 前台化该项目。只改"当前是哪个页签",
    /// 不结束任何会话、不改写任何 `Workspace` 的内容。
    ProjectTabSwitch(i64),
    /// 项目页签:点页签的 × → 关闭该页签,并结束该项目下所有会话。
    ProjectTabClose(i64),
    /// 项目页签:`App::ensure_loaded` 的异步促成完成——素材已取回,由
    /// `update` 在 UI 线程上装配成 `Workspace`,替换掉那份"加载中"占位
    /// (载荷是一次性信封,见 [`RestorePayload`])。
    ProjectSlotLoaded(i64, RestorePayload),
    /// 项目:`git_watch` 监听到工作区/`.git` 引用变化,该重新跑一次 git 刷新
    /// 了(D4)。`Relevance` 决定这次触发要不要顺带做 Plan 2 的 Git Log 快照
    /// 重建。这条消息同时喂给 Files(刷新 git_statuses)、Project(刷新
    /// branch/dirty)和 Git Log(条件触发快照重建)三个独立扩展,
    /// 内核继续拦截、分别转发,不包进任何一个 extension 的 `Message`。
    ProjectFsChanged(ProjectId, git_watch::FsChanges),
    /// Git Log 面板的全部消息,内核只转发不解读——见
    /// `extensions::git_log::Message`。
    GitLog(git_log::Message),
    /// 文件历史对比弹窗的全部消息,内核只转发不解读——见
    /// `extensions::file_history::Message`。
    FileHistory(file_history::Message),
    /// Files 面板的全部消息,内核只转发不解读——见 `extensions::files::Message`。
    Files(files::Message),
    /// Project 信息面板的全部消息,内核只转发不解读——见
    /// `extensions::project::Message`。
    Project(project::Message),
    /// SSH 主机面板的全部消息,内核只转发不解读——见
    /// `extensions::ssh::Message`。
    Ssh(ssh::Message),
    /// Footbar 系统信息条的消息,内核只转发不解读——见
    /// `extensions::footbar::Message`。App 级状态(不挂 `Workspace`),
    /// 路由比其它 extension 简单:不带 `project_id`,不需要
    /// `with_project`/`with_focused_project`,直接 `footbar::update`。
    Footbar(footbar::Message),
    /// UI 整体放大(Ctrl +)：放大/还原的全局 scale 乘一个步近因子,下一帧
    /// 按新 scale 重排全部图标/字号/间距/骨架。
    ZoomIn,
    /// UI 整体缩小(Ctrl -)。
    ZoomOut,
    /// UI 缩放还原(Ctrl+1)：回到启动基准 scale。
    ZoomReset,
    /// 预览/浏览器 webview 收到鼠标点击(JS mousedown → IPC → EventLoopProxy),
    /// 通知 main.rs 调 `view.focus()` 让 WKWebView 成为 first responder。
    /// winit 收不到子 webview 上的 `MouseInput`,这条消息是唯一焦点信号源。
    /// 不区分 Preview/Browser:`left_view` 互斥,`dispatch` 按 `shell_state`
    /// 判断归谁。
    WebViewFocused,
    /// 子 webview 上的鼠标松开(winit 收不到,JS 经 IPC 发来)。目的是结束
    /// 页签拖拽:若用户把 tab 从 iced 表层一路拖进 webview 并在这里松开,
    /// winit 的根本 `MouseInput{Released}` 收不到,`TabDragEnd` 就永不触发,
    /// 拖拽状态会残留、变成"松开还能继续拖"。这条消息统一兜底清掉。
    WebViewMouseUp,
    /// 顶栏设置齿轮:开主题设置弹窗。App 级状态(不挂 `Workspace`)——
    /// 首页/空工作区/项目工作区三种 `view()` 分支都画顶栏,弹窗必须在三者
    /// 之上都能弹出,见 `App::view` 里 `view_inner` 的外层叠加。
    SettingsOpen,
    /// 设置弹窗:点遮罩/关闭按钮收起,不需要"取消"语义——选中主题即时生效
    /// 并已落盘,收起只是隐藏浮层。
    SettingsClose,
    /// 设置弹窗:选中一个配色方案,立即调
    /// `byteui::theme::color::set_scheme` 全局生效并调 `persist_scheme`
    /// 落盘(跨重启记住选择,同 `ZoomIn`/`ZoomOut` 之于 `icon_size::persist_scale`
    /// 的模式)。
    SettingsThemeSelected(byteui::theme::color::ColorScheme),
}
pub(crate) struct ProjectLinkMenu {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) target: project::links::LinkTarget,
    pub(crate) index: usize,
}

/// Todo 分类树节点右键菜单浮层状态,镜像 `ProjectLinkMenu`。`id` 为
/// `None` 表示右键的是"全部"/"未分类"伪节点(菜单只含"新建分类"新建
/// 顶层分类)。
pub(crate) struct CategoryContextMenu {
    pub(crate) x: f32,
    pub(crate) y: f32,
    /// 被右键的节点:`Some` 为真实分类 id,`None` 为全部/未分类伪节点。
    pub(crate) id: Option<i64>,
}

/// 分类选择器要挂靠的目标:给任务挂分类、给分类节点 reparent。二者共用
/// "点树选一个节点";挂任务 → `set_todo_category`,reparent → `reparent_category`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CategoryPickerTarget {
    /// 给某个任务挂分类(任务 id)。
    Todo(i64),
    /// 给某个分类节点 reparent(分类 id)。
    Category(i64),
}

/// 分类选择器浮层状态:定位坐标 + 目标。渲染内容复用
/// `category_tree_nav` 同一份树数据(只读展示,不接展开/右键,选中即
/// 关闭并提交)。
pub(crate) struct CategoryPicker {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) target: CategoryPickerTarget,
}

/// 输入框右键菜单浮层状态:定位坐标(屏幕空间,复用 `files.last_right_click`)
/// 与被右键的输入目标。二者都由 `TextInputMenuOpen(target)` 写入,关闭或
/// 执行某个编辑动作后清空。渲染见 `app.view` 顶层 `text_input_menu_popup`。
pub(crate) struct TextInputMenu {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) target: TextInputTarget,
}

/// 数据库面板数据源树 header 行的右键菜单浮层状态:定位坐标(复用
/// `files.last_right_click`)+ 目标数据源 id。测试连接/编辑/删除/刷新
/// 四个动作见 `database_source_context_menu_popup`。
pub(crate) struct DatabaseSourceMenu {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) source_id: String,
}
