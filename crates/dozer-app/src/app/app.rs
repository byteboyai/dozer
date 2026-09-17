//! `struct App` 外壳态本体 + `impl App`(生命周期/accessor/布局持久化/
//! update/view)+ 自由 view 函数 + 测试。Phase 2 结构重组时从 `app.rs` 整体
//! 平移进来,Phase 3 再按 update/view 拆开。

use crate::extensions::browser;
use crate::extensions::conversations;
use crate::extensions::database;
use crate::extensions::files;
use crate::extensions::footbar;
use crate::extensions::git_log;
use crate::extensions::project;
use crate::extensions::search;
use crate::extensions::ssh;
use crate::extensions::todo;
use crate::extensions::usage;
use crate::git_watch;
use crate::homespace::{self, HomeRecentConversation, HomeRecentFile, load_home_recents};
use crate::layout;
use crate::open_projects;
use crate::panel_layouts;
use crate::preview::WebviewSpec;
use crate::rail;
use crate::settings;
use crate::tab_widget;
use crate::term_view;
use crate::terminal;
use crate::theme;
use crate::topbar;
use crate::transcript::ReviewEntry;
use crate::webview_geometry;
use crate::workspace::{
    CONVERSATION_DETAIL_PAGE_SIZE, PreviewPaneKind, RestorePayload, ReviewSource, ReviewView,
    ShellIo, SshOut, TabAttachedArgs, TabBackend, Workspace, agent_list_pane, agent_picker_popup,
    dot_color, exited_marker, fetch_project_restore, no_project_placeholder, preview_pane,
    preview_tab_display_width, preview_tab_overflow_popup, project_preview_pane,
    relative_time_text, review_content_pane, review_should_refresh_on_turn,
    spawn_disk_usage_refresh, spawn_project_git_refresh, split_portions, tab_display_width,
    tab_title,
};
use byteui::interaction::icons;
use dozer_client::Client;
use dozer_core::protocol::{AgentKind, AgentState, BookmarkInfo, ProjectInfo, SessionInfo};
use iced_widget::core::border::Radius;
use iced_widget::core::font::Weight;
use iced_widget::core::mouse;
use iced_widget::core::{Border, Color, Element, Font, Length, Padding};
use iced_widget::tooltip::{Position, Tooltip};
use iced_widget::{MouseArea, button, column, container, row, stack, text};
use iced_winit::winit::event_loop::EventLoopProxy;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::runtime::Handle;

use super::*;

/// 终端初始网格尺寸（列 x 行）。真实尺寸由窗口创建后的第一次
/// `Message::PaneResized` 立刻纠正（见 `main.rs` 的 `resumed()`）；这里只是
/// "窗口还没量出真实像素前"的兜底默认值。
pub(crate) const DEFAULT_COLS: u16 = 80;

pub(crate) const DEFAULT_ROWS: u16 = 24;

/// `dozer://review-trace/data.json` 的响应体形状——`review_trace.html` 按
/// 这个结构消费(`entries`/`agent_label`/`summary_title`/`summary_text`/
/// `summary_time` 顶层字段)。总结区(标题/全文/时间)只在本会话详情里非空,
/// 活会话 `None`。
#[derive(serde::Serialize)]
struct ReviewSnapshot<'a> {
    entries: &'a [ReviewEntry],
    /// `AgentKind::label()`(如 `"claude"`)——AI 气泡的头像名字标签用它
    /// 替代写死的"AI"。
    agent_label: &'static str,
    /// session 详情顶部总结区标题/全文/相对时间(2026-08-27 起标题/全文;
    /// 2026-08-28 加时间;活会话审阅为 `None`)。
    summary_title: Option<String>,
    summary_text: Option<String>,
    summary_time: Option<String>,
}

/// `Message::ReviewLoaded` 落地新一页回合时,决定是替换还是追加进已有
/// `entries`。`append == true`("加载更多")必须真的累加,不能让调用方
/// 在这一步完成之前就拿刚到手的这一页去建 webview 快照——那样会把之前
/// 已经展示的内容整个换掉,只剩最新这一页(2026-08-27 修正的真实 bug,
/// 抽成纯函数方便 headless 单测覆盖这条语义,不依赖 App 级测试夹具)。
fn merge_review_entries(existing: &mut Vec<ReviewEntry>, new: Vec<ReviewEntry>, append: bool) {
    if append {
        existing.extend(new);
    } else {
        *existing = new;
    }
}
#[cfg(target_os = "macos")]
fn project_link_menu_items(
    target: project::links::LinkTarget,
    index: usize,
) -> Vec<crate::native_menu::Item<Message>> {
    vec![crate::native_menu::Item::Entry {
        icon: Some(icons::IconKind::Trash),
        icon_color: None,
        label: "删除".into(),
        color: byteui::theme::color::current().body,
        enabled: true,
        msg: Message::Project(project::Message::LinkRemove { target, index }),
    }]
}

/// `text_input_menu_popup` 的原生菜单版本,纯数据组装——密码框场景锁定
/// 剪切/复制,同旧版语义。仅 macOS 编译,非 mac 平台继续走 iced 弹层。
#[cfg(target_os = "macos")]
fn text_input_menu_items(target: &TextInputTarget) -> Vec<crate::native_menu::Item<Message>> {
    use crate::native_menu::Item;
    let body = byteui::theme::color::current().body;
    let dim = byteui::theme::color::current().dim;
    let (cut_copy_color, cut_copy_enabled) = if target.secure {
        (dim, false)
    } else {
        (body, true)
    };
    vec![
        Item::Entry {
            icon: Some(icons::IconKind::Scissors),
            icon_color: None,
            label: "剪切".into(),
            color: cut_copy_color,
            enabled: cut_copy_enabled,
            msg: Message::TextInputMenuCut,
        },
        Item::Entry {
            icon: Some(icons::IconKind::Copy),
            icon_color: None,
            label: "复制".into(),
            color: cut_copy_color,
            enabled: cut_copy_enabled,
            msg: Message::TextInputMenuCopy,
        },
        Item::Entry {
            icon: Some(icons::IconKind::ClipboardPaste),
            icon_color: None,
            label: "粘贴".into(),
            color: body,
            enabled: true,
            msg: Message::TextInputMenuPaste,
        },
        Item::Entry {
            icon: Some(icons::IconKind::SelectAll),
            icon_color: None,
            label: "全选".into(),
            color: body,
            enabled: true,
            msg: Message::TextInputMenuSelectAll,
        },
    ]
}

/// `database_source_context_menu_popup` 的原生菜单版本,纯数据组装——数据源
/// 未展开时"刷新"置灰(同旧版 `item_locked` 语义)。仅 macOS 编译。
#[cfg(target_os = "macos")]
fn database_source_menu_items(
    source_id: &str,
    expanded: bool,
) -> Vec<crate::native_menu::Item<Message>> {
    use crate::native_menu::Item;
    let dim = byteui::theme::color::current().dim;
    let id = source_id.to_string();
    vec![
        Item::entry(
            Some(icons::IconKind::RefreshCw),
            "测试连接",
            Message::Database(database::Message::TestConnection(id.clone())),
        ),
        Item::entry(
            Some(icons::IconKind::Settings),
            "编辑",
            Message::Database(database::Message::EditSourceStart(id.clone())),
        ),
        Item::entry(
            Some(icons::IconKind::Trash),
            "删除",
            Message::Database(database::Message::DeleteSourceRequest(id.clone())),
        ),
        Item::Entry {
            icon: Some(icons::IconKind::RotateCw),
            icon_color: None,
            label: "刷新".into(),
            color: if expanded {
                byteui::theme::color::current().body
            } else {
                dim
            },
            enabled: expanded,
            msg: Message::Database(database::Message::SchemaRefresh(id)),
        },
    ]
}

/// `category_context_menu_popup` 的原生菜单版本,纯数据组装。仅 macOS 编译。
#[cfg(target_os = "macos")]
fn category_context_menu_items(
    id: Option<i64>,
    sibling_parent_id: Option<i64>,
) -> Vec<crate::native_menu::Item<Message>> {
    use crate::native_menu::Item;
    match id {
        None => vec![Item::entry(
            Some(icons::IconKind::SquarePlus),
            "新建分类",
            Message::Todo(todo::Message::CategoryNewSibling(None)),
        )],
        Some(id) => vec![
            Item::entry(
                Some(icons::IconKind::SquarePlus),
                "新建子分类",
                Message::Todo(todo::Message::CategoryNewChild(id)),
            ),
            Item::entry(
                Some(icons::IconKind::SquarePlus),
                "新建同级分类",
                Message::Todo(todo::Message::CategoryNewSibling(sibling_parent_id)),
            ),
            Item::entry(
                Some(icons::IconKind::ChevronUp),
                "上移",
                Message::Todo(todo::Message::CategoryMoveSibling(
                    id,
                    dozer_core::protocol::CategoryMoveDirection::Up,
                )),
            ),
            Item::entry(
                Some(icons::IconKind::ChevronDown),
                "下移",
                Message::Todo(todo::Message::CategoryMoveSibling(
                    id,
                    dozer_core::protocol::CategoryMoveDirection::Down,
                )),
            ),
            Item::entry(
                Some(icons::IconKind::FolderOpen),
                "移动到...",
                Message::Todo(todo::Message::CategoryReparentPickerOpen(id)),
            ),
            Item::entry(
                Some(icons::IconKind::Rename),
                "重命名",
                Message::Todo(todo::Message::CategoryRenameStart(id)),
            ),
            Item::entry(
                Some(icons::IconKind::Trash),
                "删除",
                Message::Todo(todo::Message::CategoryDelete(id)),
            ),
        ],
    }
}
pub struct App {
    client: Client,
    handle: Handle,
    proxy: EventLoopProxy<Message>,
    /// 关 tab/丢弃过期促成结果时发往 daemon 的 kill/总结请求句柄——退出前
    /// `wait_for_pending_exit_tasks` 要等它们跑完,不然请求可能因为 tokio
    /// runtime 随进程退出被中途丢弃,daemon 侧会话仍是 `alive`,下次启动
    /// 又被恢复出来。见 `ShellIo::track_exit_critical`。
    pending_exit_tasks: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
    /// 当前终端网格尺寸,随 `PaneResized` 更新;新建 tab 时也用这份
    /// 尺寸,保证新会话从一开始就跟 pane 实际大小匹配。
    cols: u16,
    rows: u16,
    /// SSH 面板内嵌终端的网格尺寸,与 `cols/rows`(共享终端)分开记——两个
    /// pane 几何不同,`PaneResized` 各带一份,无法互相替代。
    ssh_cols: u16,
    ssh_rows: u16,
    /// 当前终端 IME 组字预览(未提交,`Ime::Preedit` 驱动):不进 PTY,只在
    /// `term_view::draw` 里叠一层带下划线的预览文字。`Ime::Commit`/组字
    /// 取消(空 preedit)时清空。只对 `App::keyboard_term_target()` 当前
    /// 指向的那个终端 pane 生效(见两处 `term_view::view` 调用处按
    /// `focused` 决定是否传入)。
    term_ime_preedit: Option<String>,
    /// daemon 连接失败,或某次会话操作失败时的错误文案。整个程序共享
    /// 一份:daemon 连不连得上不是某个项目自己的状态。
    pub(crate) daemon_error: Option<String>,
    /// 上次真正执行 Todo 面板磁盘轮询(`poll_todo_if_visible`)的时刻:
    /// 按 `TODO_POLL_INTERVAL` 自限速,未到点的调用直接 no-op。现在靠
    /// mtime 检查已经安全(没变化就早退,见该方法文档),这里补上限速是为了
    /// 让"周期性函数自己对被更快唤醒节奏带跑免疫"这条约定对周期性关注点
    /// (悬停动画/Todo 轮询)都显式成立,不留一个"靠巧合安全"的例外。
    last_todo_poll_at: std::time::Instant,
    /// 全局窗口尺寸;启动时 `layout::load()` 读盘作起始值,退出前写盘。
    /// 只存窗口尺寸——左右面板区的宽度/分割比例(**每个项目各自**的偏好)
    /// 已迁进每项目 `dims`(见 `panel_layouts`),不放在这里。
    pub(crate) shell_layout: ShellLayout,
    /// 当前活跃项目的面板区尺寸(左宽 + 四个 split)活值。切项目前经
    /// `current_panel_layout` 回填进 `PanelLayout.dims` stash,切过去由
    /// `adopt_panel_layout` 灌回来。拖拽(`ColumnDragEnd`)写回这份。
    dims: PanelDims,
    /// 每个项目各自的面板布局(左右视图选择 + 收起态),按项目 id 索引;
    /// 启动时从 `panel_layouts::load()` 读回,切换/改面板时写回。当前正
    /// 显示的项目的布局由 `left_view`/`right_view`/`left_collapsed`/
    /// `right_collapsed` 这几份"活值"承载,切换项目前先 `stash` 回这里、
    /// 切过去再 `adopt` 出来,别的项目那份绝不被当前项目盖掉。
    panel_layouts: HashMap<i64, PanelLayout>,
    /// 左面板区当前显示的配对视图(左图标栏点击切换)。
    pub(crate) left_view: PanelKind,
    /// 右面板区当前显示的配对视图(右图标栏点击切换)。
    pub(crate) right_view: PanelKind,
    /// 左面板区是否折叠(再点一次当前已激活的图标即收起)。
    pub(crate) left_collapsed: bool,
    /// 右面板区是否折叠,语义同 `left_collapsed`。
    pub(crate) right_collapsed: bool,
    /// 当前放大的内容子面板(`None`=未放大)。
    maximized: Option<MaximizedPane>,
    /// 左键点击落点决定的当前"聚焦"面板区,驱动 `left_zone`/`right_zone`
    /// 外边框的高亮态(见 `set_active_zone`/`zone_at_x`)。启动默认
    /// `Some(Right)`——终端处默认焦点区,终端在右面板区。
    pub(crate) active_zone: Option<ZoneSide>,
    /// 所有按钮的悬停动画状态机(顶栏 Home / 顶栏右侧 / 图标栏),key 为
    /// `HoverId`。iced 0.14 无内置动画 API,这套自驱 redraw(与光标闪烁同款)
    /// 把图标/背景颜色在 idle↔hover 间 ease-out 过渡。进度由 main.rs 的定时
    /// 唤醒经 `advance_hover_anims` 指数逼近各自 `target`(见 `HoverAnim`)。
    hover_anims: std::collections::HashMap<HoverId, HoverAnim>,
    /// 页签标题 tooltip 的悬停计时起点:key 复用 `HoverId`(与 `hover_anims`
    /// 同源),进入页签记 `Instant::now()`,离开即清除。悬停满
    /// `HOVER_TOOLTIP_DELAY` 后视图层据此弹出标题全称 tooltip(见
    /// `hover_tooltip_ready`)。浏览器面板的页签走自己那套 hover 状态机,
    /// 计时另存于 `browser::State::tooltip_starts`,本表只覆盖顶栏页签与
    /// 终端/预览/SSH 面板页签。
    hover_tooltip_starts: std::collections::HashMap<HoverId, std::time::Instant>,
    /// 图标栏拖拽换位/换栏时,每个面板按钮的动画槽位状态机——同栏重排让
    /// 让位的相邻按钮平滑滑动到新槽位,而不是瞬间跳变。key 为
    /// `PanelKind`,与 `hover_anims` 同款自驱 redraw 节奏(`advance_hover_anims`
    /// 顺带推进,见 `RailSlotAnim`)。跨栏移动的目标侧与来源侧是两条完全
    /// 不同的物理列,不追求跨列平滑滑动,该面板在新一侧直接按新槽位
    /// snap(`RailSlotAnim::retarget` 检测到侧变化即重置,不生成动画)。
    rail_slot_anims: std::collections::HashMap<PanelKind, rail::RailSlotAnim>,
    /// 双击顶栏空白处待处理标记,见 `Message::TopBarDoubleClick`/
    /// `take_pending_zoom_toggle`。`App` 不持有 `winit::window::Window`
    /// 句柄,真正切换最大化态由 main.rs 轮询这个标记后调用。
    pending_zoom_toggle: bool,
    /// 顶栏"＋新增项目"按钮的最近项目选择菜单是否打开(见
    /// `Message::ProjectAddMenuToggle`/`project_add_menu_popup`)。挂在
    /// `App` 而不是某个 `Workspace` 上——这个按钮本身就在顶栏、不属于任何
    /// 单个项目,与 `ws.agent_picker_open`(Agent 面板"＋",项目内状态)是
    /// 两个不同归属层级的同类开关。
    pub(crate) project_add_menu_open: bool,
    /// 打开菜单那一刻的光标逻辑坐标,菜单弹出锚点——"＋"按钮的 x 随已开
    /// 页签数量浮动(`project_tabs_row` 页签 `Shrink` 宽、"＋"紧跟最后一片
    /// 页签之后),没有固定 padding 能蒙对,改用 `todo::set_calendar_anchor`
    /// /`set_dispatch_anchor` 同款"记下点击时的 `App::last_cursor`"手法。
    pub(crate) project_add_menu_anchor: (f32, f32),
    /// 顶栏设置齿轮的主题设置弹窗是否打开。挂在 `App` 而不是某个
    /// `Workspace` 上——设置按钮本身就在顶栏,不属于任何单个项目,同
    /// `project_add_menu_open` 的归属考量。
    pub(crate) settings_modal_open: bool,
    /// 全局 UI 缩放(⌘/Ctrl +/-)改变后,预览/浏览器 webview 的
    /// `WebView::zoom` 也要同步——但 `App` 不持有 webview 句柄,只能
    /// 置这个标记,由 main.rs 轮询 `take_pending_preview_zoom` 后逐个
    /// 应用。新 webview 在 `sync_webview_pool` 建出来时直接按当前 scale
    /// 初始化,所以本标记只管"已存在 webview 的缩放变更"这一增量。
    pending_preview_zoom: bool,
    /// 当前窗口逻辑尺寸(宽,高)。由 main.rs 建窗口/`WindowEvent::Resized`
    /// 时经 `set_window_size` 写入。
    pub(crate) window_size: (f32, f32),
    /// 最近一次 `CursorMoved` 的光标逻辑坐标(宽,高),由 main.rs 每帧更新。
    /// Todo 日历浮层用它当弹出锚点(点日历按钮时光标就在按钮上,等价于"按钮
    /// 旁边"),镜像 `files.last_right_click` 的坐标复用套路。
    pub(crate) last_cursor: (f32, f32),
    /// 正在拖拽的分隔线;`None` 表示未在拖拽。
    dragging: Option<Divider>,
    /// 正在拖拽的纵向(上下)分隔线;`None` 表示未在拖拽。
    dragging_row: Option<RowDivider>,
    /// 正在拖拽的页签(换位);`None` 表示未在拖拽页签。与 `dragging` 分隔线
    /// 互斥(一次左键拖拽只能是一件事)。
    tab_drag: Option<TabDrag>,
    /// 正在进行的图标栏面板拖拽(同栏重排/跨栏移动);`None` 表示未在拖拽。
    /// 与 `tab_drag`/`dragging` 互斥(一次左键拖拽只能是一件事)。
    pub(crate) rail_drag: Option<rail::RailDrag>,
    /// Files 面板右键菜单浮层状态——见 `extensions::files::AppState`。
    files: files::AppState,
    /// Project 面板「项目文档 / Agent 记忆」链接行的右键菜单浮层状态,坐标
    /// 同样复用 `files.last_right_click`。
    project_link_menu: Option<ProjectLinkMenu>,
    /// Todo 分类树节点右键菜单浮层状态,坐标复用 `files.last_right_click`。
    category_context_menu: Option<CategoryContextMenu>,
    /// 分类选择器("移动到..." / 任务挂分类)浮层状态:定位坐标 + 目标。
    category_picker: Option<CategoryPicker>,
    /// 通用输入框右键菜单浮层状态(屏幕空间单例)。`TextInputMenuOpen` 时
    /// 写入、`TextInputMenuClose`/动作后清空。同一时刻最多挂一个。
    text_input_menu: Option<TextInputMenu>,
    /// mac 原生菜单选中剪切/复制/粘贴/全选后,要合成的 `⌘+x/c/v/a` 字符 +
    /// 目标输入 id——`native_menu::show` 同步阻塞返回时那一帧的
    /// `UserInterface` 已经不在了,main.rs 在 `dispatch` 循环之后另起一次
    /// 补上(见 `main.rs::window_event` 对应处)。非 mac 平台恒 `None`。
    pending_native_menu_edit_key: Option<(char, iced_widget::core::widget::Id)>,
    /// 数据库面板数据源树 header 行的右键菜单浮层状态,坐标同样复用
    /// `files.last_right_click`。
    database_source_menu: Option<DatabaseSourceMenu>,
    /// 待处理的"输入框右键菜单要作用的输入"焦点:载入 `TextInputMenuOpen`
    /// 携带的 `TextInputTarget.id`,由 main.rs 的 `apply_pending_focus` 在
    /// 本帧后移至该输入,供菜单的复制/粘贴作用到被右键的输入。
    pending_text_input_focus: Option<iced_widget::core::widget::Id>,

    /// 并行打开的项目页签:project id → 该项目的完整/占位状态。
    pub(crate) projects: HashMap<i64, WorkspaceSlot>,
    /// 页签顺序(`projects` 是 HashMap,顺序另存;Task 6 的页签栏按它渲染)。
    pub(crate) project_order: Vec<i64>,
    /// 当前聚焦的项目页签(`None`=一个项目都没打开)。
    pub(crate) active_project_id: Option<i64>,
    /// 当前顶层页面(工作区 / 首页)。默认 `Workspace`;点顶栏 Dozer 切到
    /// `Home`,打开/切换项目切回 `Workspace`。
    pub(crate) current_page: AppPage,
    /// H0 项目中心侧栏用的"最近项目"列表(D2)。与 `Workspace.recent_projects`
    /// 语义相同但字段独立——避免为了 H0 牵连项目栏"未打开项目"兜底列表那条
    /// 无关路径。`App::bootstrap()`/`Message::ProjectTabOpened` 处理函数负责
    /// 让它跟 daemon 的 `list_projects()` 结果保持同步。
    pub(crate) recent_projects: Vec<ProjectInfo>,
    /// H0"最近的文件"卡数据(D4);`Message::TopBarHome` 时异步刷新。
    pub(crate) home_recent_files: Vec<HomeRecentFile>,
    /// H0"最近的对话"卡数据(D4);语义同上。
    pub(crate) home_recent_conversations: Vec<HomeRecentConversation>,
    /// 是否已经收到过至少一次 `HomeRecentsLoaded`——区分"还在加载"与"加载完
    /// 但结果为空"，两张卡据此决定画"加载中…"还是空状态文案(spec §4)。
    pub(crate) home_recents_loaded: bool,
    /// 首页左栏当前显示哪个 pane(项目列表/Recents)。不持久化,每次
    /// `Message::TopBarHome` 进首页都重置为默认值——见
    /// `homespace::HomeLeftView`。
    pub(crate) home_left_view: homespace::HomeLeftView,
    /// 首页"项目列表" pane 已经展开的项目页数(点一次"更多..." +1)。首屏
    /// 只显示前 `PAGE_SIZE`(5)个,翻页后显示 `pages * 5` 个。不持久化,每次
    /// `Message::TopBarHome` 进首页重置回 1——与 `home_left_view` 同套
    /// "进首页即重置"语义;别的 pane(Recents)不读它。
    pub(crate) home_project_pages: usize,
    /// 首页"项目列表"搜索框已提交生效的过滤词(空串 = 不过滤)。不持久化,
    /// 与 `home_project_pages` 同套"进首页即重置"语义。
    pub(crate) home_project_search: String,
    /// 搜索框编辑态草稿——同 `todo::search_draft`,打字期间只改草稿,
    /// 回车/点搜索按钮才落成 `home_project_search`。
    pub(crate) home_project_search_draft: String,
    /// 是否持有 iced 真实焦点。**不是**应用层手动置位的镜像——每帧渲染
    /// 循环里 `CaptureHomeSearchFocus` 问一遍 iced 真相后立刻写进这里
    /// (`set_home_project_search_focused`)。
    pub(crate) home_project_search_focused: bool,
    /// 首页右栏当前显示哪个 pane(目前只有 Browser)。语义同上。
    pub(crate) home_right_view: homespace::HomeRightView,
    /// 首页全局浏览器面板状态,不挂在任何 `Workspace` 上;`view`/`update`
    /// 调用时 `project_id` 恒传 `None`(全局收藏夹作用域)。
    pub(crate) home_browser: browser::State,
    /// Git Log 面板状态——自己的 `Message`/`update`/`view`,见
    /// `extensions::git_log`。`App` 级共享、不按项目分(现状,纯重构不改,
    /// 见 `sync_git_log_to_active_project`)。
    git_log: git_log::State,
    /// 数据库面板 App 级状态(哪些驱动类型在"新增数据源"下拉里可选,
    /// 启动时读盘)——见 `extensions::database::AppState`。
    database: database::AppState,
    /// Footbar 系统信息条 App 级状态——跨所有项目页签共享(见
    /// `extensions::footbar::AppState`)。`spawn_sampler` 在 `new_shell`
    /// 阶段启动一个长生命周期 tokio 任务,每 1s/300s 采样一次发回
    /// `Message::Footbar(Message::Sampled)`,UI 即刻刷新。
    footbar: footbar::AppState,
}

/// 前台化一个**已经存在**的项目页签:只改"当前是哪个页签",一个槽位的内容
/// 都不碰。返回 false = 没有这个页签(调用方原地放弃)。
///
/// 这是**唯一**一条"换到另一个项目"的路。此前还有一条
/// `retarget_active_slot` 的**就地改写**路(`ProjectOpened`:把当前槽位的内容
/// 整体改写成另一个项目,先 `close_all_tabs_for_switch` 关光终端、再
/// `adopt_project`),它与设计文档 §2"只有用户显式关闭一个页签,才结束该项目
/// 下的会话"直接冲突,已随最终审查一并删除。**不要**把它请回来:打开项目一律
/// 是"新增/前台化一个页签",绝不是"把手上这个换掉"。
pub(crate) fn focus_project_tab(
    projects: &HashMap<i64, WorkspaceSlot>,
    active_project_id: &mut Option<i64>,
    id: i64,
) -> bool {
    if !projects.contains_key(&id) {
        return false;
    }
    *active_project_id = Some(id);
    true
}

/// 关掉某个页签后,焦点该落到谁身上:原位置的右邻优先,没有右邻取左邻,
/// 一个都不剩则 `None`(退回"未打开任何项目"的空外壳)。
///
/// 传入的是**删除之前**的顺序表——右邻/左邻要按被关页签的原位置算,删完
/// 再算就分不清"右邻"了。
pub(crate) fn next_active_after_close(project_order: &[i64], closed: i64) -> Option<i64> {
    let idx = project_order.iter().position(|&i| i == closed)?;
    project_order
        .get(idx + 1)
        .or_else(|| idx.checked_sub(1).and_then(|prev| project_order.get(prev)))
        .copied()
}

/// 从槽位表里摘掉一个项目页签,返回被摘掉的槽位交给调用方善后(结束会话)。
/// 只在关的正好是当前页签时才动 `active_project_id`(落到
/// [`next_active_after_close`] 给的邻居),关后台页签不打扰前台。
pub(crate) fn take_project_tab(
    projects: &mut HashMap<i64, WorkspaceSlot>,
    project_order: &mut Vec<i64>,
    active_project_id: &mut Option<i64>,
    id: i64,
) -> Option<WorkspaceSlot> {
    let slot = projects.remove(&id)?;
    if *active_project_id == Some(id) {
        *active_project_id = next_active_after_close(project_order, id);
    }
    project_order.retain(|pid| *pid != id);
    Some(slot)
}

/// 按 `project_id` 取一份**已加载**的 `Workspace`,与"此刻聚焦的是哪个项目"
/// 完全无关——这就是 [`App::with_project`] 的全部路由逻辑。
///
/// 拆成自由函数是为了能 headless 单测(`App` 要 daemon 连接 + winit
/// `EventLoopProxy` 才构造得出来),与本文件其余纯逻辑一致。这条路由是多项目
/// 并行下最要紧的一条不变式,理由见 [`ProjectId`]。
pub(crate) fn loaded_workspace_mut(
    projects: &mut HashMap<i64, WorkspaceSlot>,
    project_id: ProjectId,
) -> Option<&mut Workspace> {
    match projects.get_mut(&project_id)? {
        WorkspaceSlot::Loaded(ws) => Some(ws),
        // `Stub` 从没促成过,不可能有指向它的在飞会话结果(促成期的"加载中"
        // 占位是 `Loaded`)。
        WorkspaceSlot::Stub { .. } => None,
    }
}

/// 启动恢复的纯逻辑:把盘上记的"上次开着哪些页签"(`open_projects.json`)与
/// daemon 现在还认识的项目列表对一遍,给出该恢复成页签的项目顺序表 + 该聚焦
/// 哪一个。
///
/// 规则(设计文档 §5):
/// - daemon 已经不认识的 id 直接跳过——项目可能在上次退出后被删了,给它开个
///   点不动的空页签只会碍事;
/// - 盘上出现重复 id(理论上不该有,但文件是用户可编辑的普通 JSON)去重,
///   否则 `project_order` 会带出两个指向同一个槽位的页签;
/// - 一个都没恢复出来(首次启动/文件缺失/项目全被删)回落到 `known` 的第一个
///   ——`list_projects()` 按 `last_active_ms` 倒序,第一个就是最近用过的那个,
///   保持"打开 app 就能干活"的既有行为;
/// - 聚焦项:记着的那个若已不在恢复出的页签集合里,回落到第一个页签。
///
/// 抽成自由函数是为了能 headless 单测(`App` 要有 daemon 连接 + winit
/// `EventLoopProxy` 才构造得出来),与本文件其余纯逻辑的处理一致。
pub(crate) fn restore_open_tabs(
    known: &[ProjectInfo],
    state: &open_projects::OpenProjectsState,
) -> (Vec<i64>, Option<i64>) {
    let mut order: Vec<i64> = Vec::new();
    for id in &state.project_ids {
        if !known.iter().any(|p| p.id == *id) {
            continue;
        }
        if order.contains(id) {
            continue;
        }
        order.push(*id);
    }
    if order.is_empty()
        && let Some(p) = known.first()
    {
        order.push(p.id);
    }
    let active = state
        .active_project_id
        .filter(|id| order.contains(id))
        .or_else(|| order.first().copied());
    (order, active)
}

/// 项目信息面板切入时的 README 保障(纯函数,方便单测):保证项目根目录有
/// 一份可读的 `README.md`。已有就**原样保留不重写**(返回其路径);没有则
/// 用项目名当一级标题、`.dozer/description.md` 的描述(`load_description`,
/// 无描述则省略)生成一份再返回路径。
///
/// 只有 README 最终存在且可读时才返回 `Some(readme 路径)`;创建失败(目录
/// 不可写等)返回 `None`——绝不拿一个空文件或半截文件去占预览,也绝不让面
/// 板切入失败。
///
/// 描述固定读磁盘权威来源,不用 `WorkspaceState.description` 缓存字段——
/// 那可能滞后于磁盘,而 README 一旦生成就固化,必须用写入时的真实描述。
fn ensure_project_readme(repo: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
    let readme = repo.join("README.md");
    if !readme.exists() {
        let description = crate::project_meta::load_description(repo);
        let mut content = format!("# {}\n\n", name);
        if let Some(desc) = description {
            content.push_str(&desc);
            content.push('\n');
        }
        std::fs::write(&readme, content).ok()?;
    }
    // 已存在 → 跳过写入原样返回;刚生成 → 写入成功才走到这。最终都以"文件
    // 确实可读"为准——存在但读不了(如权限)就回 `None`,不拿去打开。
    if std::fs::read_to_string(&readme).is_ok() {
        Some(readme)
    } else {
        None
    }
}

/// `ws.preview`(Files)与 `ws.project_preview`(Project)是两个独立
/// `PreviewPane`,各自 `next_id` 从 0 起数——两者的 webview 一旦同时
/// 进同一个 `webviews` 池(`Files` 在左栏、`Project` 在右栏同时活跃时
/// 就会发生),原始 id 会撞(两边都可能是 0/1/2...)。给 `Project` 那
/// 一侧的 id 统一加这个偏移,`ws.preview` 侧不动——量级远超真实 tab
/// 数(几十个封顶),不会反向撞回 `ws.preview` 的 id 区间。main.rs 里
/// 任何按 id 反查 `ws.project_preview` webview(`active_preview_webview_id`
/// 的 Project 分支)都要用同一个偏移量加/减,两处不同步会导致查错池。
pub(crate) const PROJECT_PREVIEW_ID_OFFSET: usize = 1_000_000;

/// `Conversations` 面板的审阅 webview 只有唯一一份内容,不需要 Files/
/// Project 那种按 tab id 分池——固定用这一个 id(经 `review_webview_spec`
/// 的 `id: 0` 加这个偏移得到),与另两个偏移空间(`0` 起、`PROJECT_
/// PREVIEW_ID_OFFSET` 起)互不相撞。
pub(crate) const CONVERSATION_REVIEW_ID_OFFSET: usize = 2_000_000;

/// `wait_for_pending_exit_tasks` 允许在飞的关 tab 收尾请求跑完的总预算。
/// 本地 UDS 往返通常亚毫秒级,留 2 秒是给 daemon 偶尔卡顿的余量,而不是
/// 期望真正用满——超时后放弃等待,不能让退出被一个卡死的 daemon 拖住。
const EXIT_TASK_BUDGET: std::time::Duration = std::time::Duration::from_secs(2);

/// `App::wait_for_pending_exit_tasks` 的核心逻辑,拆成独立函数以便不依赖
/// `ShellIo`/`EventLoopProxy`(单测环境构造不出真实 winit 事件循环)直接
/// 测试:一批 spawn 任务在预算内全部跑完就正常返回,超预算就放弃等待
/// 并打日志,但**不会**无限期挂住调用方。
async fn join_pending_exit_tasks(
    tasks: Vec<tokio::task::JoinHandle<()>>,
    budget: std::time::Duration,
) {
    let joined = futures::future::join_all(tasks);
    if tokio::time::timeout(budget, joined).await.is_err() {
        tracing::warn!("退出前等待关 tab 的收尾请求超时,放弃等待");
    }
}

impl App {
    /// 启动序列成功路径:建好外壳态,再把上次退出时开着的**整份**项目页签
    /// 集合恢复出来。
    ///
    /// 恢复策略是"页签全恢复、内容懒加载"(设计文档 §2/§5):
    /// `open_projects.json` 里记着的每个项目都落一个 `Stub` 槽位——这一步不发
    /// 起任何网络请求,只是把页签栏画全,所以开着十个项目也不会拖慢启动;
    /// 只有上次聚焦的那一个立刻 `Workspace::bootstrap` 出完整状态(会话重挂/
    /// 文件树/git/对话),其余留在 `Stub`,用户点开时才由
    /// [`App::ensure_loaded`] 促成。
    ///
    /// 聚焦的那个这里**直接 `await`**、不走 `ensure_loaded` 的异步路径:窗口
    /// 还没建出来,此刻多等一个 UDS 往返不占用任何 UI 线程,换来的是第一帧
    /// 就是完整界面,而不是先闪一下空的"加载中"再补内容。
    ///
    /// 除了 `list_projects()`,这里还多做一次 `list()`(全量会话快照):`Stub`
    /// 页签靠它算出各自的后台活动指示点(见 [`stub_activity`])。一次协议往返
    /// 换"重启后后台页签的状态点不是空白",而且**不促成**任何 `Workspace`,
    /// 懒加载策略原样保留。
    pub async fn bootstrap(client: Client, handle: Handle, proxy: EventLoopProxy<Message>) -> Self {
        let mut app = Self::new_shell(client, handle, proxy, None);
        let io = app.shell_io();
        // daemon 不再记"活跃项目"(P2a Task 1-3 删掉了这个概念),开着哪些
        // 项目改由 GUI 侧的 open_projects.json 记(Task 4/6 写,这里读回)。
        let known = io.client.list_projects().await.unwrap_or_default();
        app.recent_projects = known.clone();
        let sessions = io.client.list().await.unwrap_or_default();
        let saved = open_projects::load();
        let (order, active) = restore_open_tabs(&known, &saved);
        for id in &order {
            let Some(info) = known.iter().find(|p| p.id == *id) else {
                continue; // restore_open_tabs 已过滤,这里只是让类型收敛
            };
            app.projects.insert(
                *id,
                WorkspaceSlot::Stub {
                    info: info.clone(),
                    activity: stub_activity(&sessions, *id),
                },
            );
        }
        let pruned = order != saved.project_ids || active != saved.active_project_id;
        app.project_order = order;
        app.active_project_id = active;
        // 启动恢复出的活跃项目,把它的面板布局换上来(没存过就退化成默认)。
        if let Some(id) = active {
            app.adopt_panel_layout(id);
        }
        if let Some(id) = active
            && let Some(WorkspaceSlot::Stub { info, .. }) = app.projects.get(&id)
        {
            let ws = Workspace::bootstrap(&io, info.clone()).await;
            app.projects.insert(id, WorkspaceSlot::Loaded(Box::new(ws)));
        }
        // `restore_open_tabs` 会剔掉 daemon 已经不认识的 id、去重、必要时回落
        // 到最近项目——这些都是对盘上那份记录的修正,不写回去的话每次启动都要
        // 重算一遍,而且下次崩在写盘之前时盘上还是那份脏数据。
        if pruned {
            app.persist_open_projects();
        }
        app
    }

    /// daemon 连接失败（自动拉起 + 重试后仍不可用）时的降级构造：不做任何
    /// 会话/项目恢复，只记下错误文案，交给 `view()` 画 RED 文案。
    ///
    /// 这是旧 `Workspace::with_daemon_error` 的正确归宿——"daemon 连不上"
    /// 是整个程序共享的状态（`daemon_error` 现在长在 `App` 上），从来就不是
    /// 某一个项目自己的状态。
    pub fn with_daemon_error(
        client: Client,
        handle: Handle,
        proxy: EventLoopProxy<Message>,
        message: String,
    ) -> Self {
        Self::new_shell(client, handle, proxy, Some(message))
    }

    /// 两个构造函数共用的"只有外壳、一个项目都没打开"的起点。
    fn new_shell(
        client: Client,
        handle: Handle,
        proxy: EventLoopProxy<Message>,
        daemon_error: Option<String>,
    ) -> Self {
        let shell_layout = layout::load();
        let panel_layouts = panel_layouts::load();
        let shell = Self {
            client,
            handle,
            proxy,
            pending_exit_tasks: Arc::new(Mutex::new(Vec::new())),
            cols: DEFAULT_COLS,
            rows: DEFAULT_ROWS,
            ssh_cols: DEFAULT_COLS,
            ssh_rows: DEFAULT_ROWS,
            term_ime_preedit: None,
            daemon_error,
            last_todo_poll_at: std::time::Instant::now(),
            left_view: PanelLayout::default().left_view,
            right_view: PanelLayout::default().right_view,
            left_collapsed: PanelLayout::default().left_collapsed,
            right_collapsed: PanelLayout::default().right_collapsed,
            shell_layout,
            dims: PanelDims::default(),
            panel_layouts,
            maximized: None,
            active_zone: Some(ZoneSide::Right),
            hover_anims: std::collections::HashMap::new(),
            hover_tooltip_starts: std::collections::HashMap::new(),
            rail_slot_anims: std::collections::HashMap::new(),
            pending_zoom_toggle: false,
            project_add_menu_open: false,
            project_add_menu_anchor: (0.0, 0.0),
            settings_modal_open: false,
            pending_preview_zoom: false,
            window_size: byteui::theme::geometry::initial_window_size(),
            last_cursor: (0.0, 0.0),
            dragging: None,
            dragging_row: None,
            tab_drag: None,
            rail_drag: None,
            files: files::AppState::default(),
            project_link_menu: None,
            category_context_menu: None,
            category_picker: None,
            text_input_menu: None,
            database_source_menu: None,
            pending_text_input_focus: None,
            pending_native_menu_edit_key: None,
            projects: HashMap::new(),
            project_order: Vec::new(),
            active_project_id: None,
            current_page: AppPage::Workspace,
            recent_projects: Vec::new(),
            home_recent_files: Vec::new(),
            home_recent_conversations: Vec::new(),
            home_recents_loaded: false,
            home_left_view: homespace::HomeLeftView::default(),
            home_project_pages: 1,
            home_project_search: String::new(),
            home_project_search_draft: String::new(),
            home_project_search_focused: false,
            home_right_view: homespace::HomeRightView::default(),
            home_browser: browser::State::with_initial_url("https://byteboy.ai"),
            git_log: git_log::State::default(),
            database: database::AppState::load(),
            footbar: footbar::AppState::default(),
        };
        // 启动 footbar 采样任务(fire-and-forget):runtime drop 时任务自然取消。
        let io = shell.shell_io();
        footbar::spawn_sampler(&io);
        shell
    }

    /// 外壳侧共享句柄的快照,交给项目态方法发起异步 IO(见 [`ShellIo`])。
    fn shell_io(&self) -> ShellIo {
        ShellIo {
            client: self.client.clone(),
            handle: self.handle.clone(),
            proxy: self.proxy.clone(),
            cols: self.cols,
            rows: self.rows,
            pending_exit_tasks: self.pending_exit_tasks.clone(),
        }
    }

    /// 按 `active_project_id` 取当前项目的 `Workspace` 只读引用；`Stub`
    /// 态和"没有任何页签"都返回 `None`（只读场景不促成加载）。
    pub fn active_workspace(&self) -> Option<&Workspace> {
        let id = self.active_project_id?;
        match self.projects.get(&id)? {
            WorkspaceSlot::Loaded(ws) => Some(ws),
            WorkspaceSlot::Stub { .. } => None,
        }
    }

    /// 外部 OS 文件拖拽命中测试：窗口坐标 (x, y) 在**当前**左侧是否落在
    /// 文件树某一行上，返回该行的落点结果(见 `files::DropHit` 文档:命中
    /// 文件行时 `target` 会退到其父目录)。main.rs 在 winit 原生事件层的
    /// `CursorMoved`/`DroppedFile` 上调用它——只要不在文件树上就返回
    /// `None`（此时拖入按现状落给终端）。
    ///
    /// 不做任何像素布局复制：文件树列的矩形由 [`left_files_tree_bounds_for`]
    /// 按 `preview_content_bounds_for` 同源的谱系换算，可见行集合从当前项目
    /// `WorkspaceState` 现取现算，二者与渲染侧 `files::view` 同源。
    pub fn files_drop_target(
        &self,
        window_w: f32,
        window_h: f32,
        x: f32,
        y: f32,
    ) -> Option<files::DropHit> {
        let side = self
            .shell_state()
            .layout
            .rail_layout
            .side_of(PanelKind::Files);
        let kind = match side {
            Side::Left => self.left_view,
            Side::Right => self.right_view,
        };
        let collapsed = match side {
            Side::Left => self.left_collapsed,
            Side::Right => self.right_collapsed,
        };
        if collapsed || kind != PanelKind::Files {
            return None;
        }
        let ws = self.active_workspace()?;
        let bounds = webview_geometry::left_files_tree_bounds_for(
            side,
            window_w,
            window_h,
            &self.shell_state(),
        );
        let rows = ws.files.visible_tree_rows();
        files::tree_drop_target(x, y, bounds, ws.files.tree_scroll(), &rows)
    }

    /// 拖拽(外部 OS 拖入/内部树拖拽共用)悬停命中一个仍处于折叠态的目录时
    /// 调用:不立即展开,武装 `files::DRAG_HOVER_EXPAND_DELAY` 计时——真正
    /// 展开推迟到 `advance_drag_hover_expand` 满时才做,见 `WorkspaceState::
    /// drag_expand_pending` 文档(立即展开会让拖着划过沿途目录疯狂跳动
    /// 布局,2026-09 用户实测反馈)。已展开(或查不到该行,当已展开处理)
    /// 直接清空计时。
    pub fn arm_drag_hover_expand(&mut self, dir: &std::path::Path) {
        let Some(ws) = self.active_workspace() else {
            return;
        };
        let already_expanded = ws
            .files
            .visible_tree_rows()
            .iter()
            .find(|r| r.path == dir)
            .is_none_or(|r| r.expanded);
        let Some(ws) = self.active_workspace_mut() else {
            return;
        };
        ws.files
            .arm_drag_expand(dir.to_path_buf(), already_expanded);
    }

    /// 悬停离开文件树、拖拽收尾/取消时调用:清空展开计时,不留残留状态。
    pub fn clear_drag_hover_expand(&mut self) {
        if let Some(ws) = self.active_workspace_mut() {
            ws.files.clear_drag_expand();
        }
    }

    /// `about_to_wait`/`ResumeTimeReached` 每次唤醒都调:当前项目悬停中的
    /// 目录若已计满 `DRAG_HOVER_EXPAND_DELAY`,发 `Message::Toggle` 真正
    /// 展开并清空计时;未满或没有悬停中的目录都是 no-op。
    pub fn advance_drag_hover_expand(&mut self) {
        let Some(dir) = self
            .active_workspace()
            .and_then(|ws| ws.files.drag_expand_ready())
        else {
            return;
        };
        self.update(Message::Files(files::Message::Toggle(dir)));
        if let Some(ws) = self.active_workspace_mut() {
            ws.files.clear_drag_expand();
        }
    }

    /// 距当前项目的展开计时满 1s 的剩余时间,`about_to_wait` 据此排精确
    /// 唤醒(同 `next_tooltip_wake` 手法)。没有悬停中的目录时返回 `None`。
    pub fn next_drag_hover_expand_wake(&self) -> Option<std::time::Duration> {
        self.active_workspace()
            .and_then(|ws| ws.files.next_drag_expand_wake())
    }

    /// 同上，可变引用版本；如果对应槽位是 `Stub`，就地促成 `Loaded`
    /// （拉取该项目的完整状态）后再返回引用。
    pub fn active_workspace_mut(&mut self) -> Option<&mut Workspace> {
        let id = self.active_project_id?;
        self.ensure_loaded(id);
        match self.projects.get_mut(&id)? {
            WorkspaceSlot::Loaded(ws) => Some(ws),
            WorkspaceSlot::Stub { .. } => None,
        }
    }

    /// `Stub` → `Loaded` 的促成:两步走。
    ///
    /// 1. **同步**把槽位换成 [`Workspace::loading_for_project`] 的"加载中"占位
    ///    ——用户点开一个 `Stub` 页签的那一刻画面就该有反应(项目名/文件树根
    ///    立刻出来),不能等异步任务跑完才有任何视觉变化。这一步之所以必须
    ///    是**带 `project` 的**占位而不是空壳,见 `loading_for_project` 的
    ///    文档:异步窗口期里这个 `Workspace` 是可达的当前项目,`project` 为
    ///    `None` 会让 `spawn_new_tab` 的 `expect` 重新变成可达路径。
    /// 2. `handle.spawn` 一个任务跑促成的 IO 段
    ///    [`fetch_project_restore`](重挂该项目的存活会话 + 最近项目列表),
    ///    完成后经 `EventLoopProxy` 回送 `Message::ProjectSlotLoaded`;装配段
    ///    (`Workspace::from_restore`)在 `update` 里、UI 线程上跑,把占位换成
    ///    真正的结果。两段之所以不能合成一段,见 [`ProjectRestore`]。
    ///
    /// 已经是 `Loaded`(含正在促成的占位)或 id 根本不在槽位表里时都是 no-op
    /// ——尤其"占位已在"这条保证了同一个页签连点几下不会重复发起促成。
    fn ensure_loaded(&mut self, id: i64) {
        let Some(WorkspaceSlot::Stub { info, .. }) = self.projects.get(&id) else {
            return;
        };
        let info = info.clone();
        let client = self.client.clone();
        let proxy = self.proxy.clone();
        self.projects.insert(
            id,
            WorkspaceSlot::Loaded(Box::new(Workspace::loading_for_project(info.clone()))),
        );
        self.handle.spawn(async move {
            let restore = fetch_project_restore(&client, info).await;
            let _ = proxy.send_event(Message::ProjectSlotLoaded(id, RestorePayload::new(restore)));
        });
    }

    /// 当前聚焦的项目 id(main.rs 的 webview 池按它分家,见
    /// `sync_previews`)。`None` = 一个项目页签都没开。
    pub fn active_project_id(&self) -> Option<i64> {
        self.active_project_id
    }

    /// 把"现在开着哪些项目页签、什么顺序、哪个在前台"写盘(Task 4 的
    /// `open_projects`)。页签集合每次变动都调一次——写的是一个几十字节的
    /// JSON,和 `preview_state`/`layout` 走同一套 `handle.spawn` 异步落盘,
    /// 不阻塞 UI 线程。
    ///
    /// 读回来的一侧目前只有 `App::bootstrap` 用了 `active_project_id`(启动
    /// 恢复上次聚焦的那个项目);把 `project_ids` 整份恢复成多页签是 Task 7
    /// 的活儿,那时这里已经有正确的数据可读。
    fn persist_open_projects(&self) {
        let state = open_projects::OpenProjectsState {
            project_ids: self.project_order.clone(),
            active_project_id: self.active_project_id,
        };
        self.handle.spawn(async move {
            if let Err(e) = open_projects::save(&state) {
                tracing::warn!("项目页签集合写盘失败: {e}");
            }
        });
    }

    /// `update()` 里"这条消息只动项目态"的统一入口:先取一份外壳句柄快照
    /// （[`ShellIo`]），再拿当前项目的 `Workspace`，一并交给闭包。没有任何
    /// 项目打开时整条消息丢弃——项目态消息没有归属就无处可落。
    ///
    /// 闭包里的 `return` 就是"提前结束这条消息的处理"，与搬家前写在
    /// `match` 分支里的 `return` 语义一致。
    /// **只用于用户点击当前界面直接触发的消息**——"作用于我此刻看着的那个
    /// 项目"正是它们的语义。异步结果消息一律**不能**走这里,必须用
    /// [`App::with_project`] 按消息自带的 `project_id` 投递,理由见
    /// [`ProjectId`]。
    ///
    /// 名字从早先的 `with_ws` 改成现在这个,是因为它与 [`App::with_project`]
    /// 的安全性质**相反**(一个投给"此刻聚焦的",一个投给"指定 id 的"),而
    /// 更短、看起来更像默认选项的那个偏偏是用错了会串项目的那个。名字必须
    /// 一眼看出差别,不能靠读文档才知道选哪个。
    fn with_focused_project(&mut self, f: impl FnOnce(&mut Workspace, &ShellIo)) {
        let io = self.shell_io();
        let Some(ws) = self.active_workspace_mut() else {
            return;
        };
        f(ws, &io);
    }

    /// `update()` 里"这条异步结果属于**某个指定项目**"的统一入口:按
    /// `project_id` 直接投递到对应槽位,与"此刻聚焦的是谁"完全无关。
    ///
    /// 这是多项目并行的核心路由不变式(见 [`ProjectId`]):后台项目的会话
    /// 输出/退出/交付检测结果绝不能落到前台项目身上。
    ///
    /// 投不到时静默丢弃,不报错:
    /// - 槽位不存在 = 用户在结果回来之前把这个页签关掉了,他已经不关心了;
    /// - 槽位是 `Stub` = 这个项目还没促成过,不可能有任何在飞的会话结果指向
    ///   它(促成期的"加载中"占位是 `Loaded`,不落进这一支)。
    fn with_project(&mut self, project_id: ProjectId, f: impl FnOnce(&mut Workspace, &ShellIo)) {
        let io = self.shell_io();
        let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
            return;
        };
        f(ws, &io);
    }

    /// 当前是否"正看着"某个项目的 Todo 面板——轮询是否要继续排下一拍
    /// 唤醒的判断条件（`main.rs::about_to_wait`），跟
    /// `any_hover_anim_active` 同一层级。
    pub fn todo_panel_visible(&self) -> bool {
        self.left_view == PanelKind::Todo && self.active_workspace().is_some()
    }

    /// `main.rs` 定时唤醒调用：只在 `todo_panel_visible()` 时真的拉取一次
    /// `dozerd` 侧的任务列表(经 `request_todos_refresh` 异步 `Loaded` 落回
    /// `ws.todo.items`),兼顾响应与省电。按 `last_todo_poll_at` 自限速到
    /// `TODO_POLL_INTERVAL`——悬停动画等更快节奏把唤醒带密时不会跟着高频
    /// 重复发请求(2026-08-12 解耦重构)。
    pub fn poll_todo_if_visible(&mut self) {
        if !self.todo_panel_visible() {
            return;
        }
        let now = std::time::Instant::now();
        if now.duration_since(self.last_todo_poll_at) < crate::TODO_POLL_INTERVAL {
            return;
        }
        self.last_todo_poll_at = now;
        let Some(project_id) = self.active_project_id else {
            return;
        };
        let client = self.client.clone();
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let emit = move |m: todo::Message| {
            let _ = proxy.send_event(Message::Todo(m));
        };
        let emit_todos = emit.clone();
        todo::request_todos_refresh(project_id, &client, &handle, emit_todos);
        todo::request_categories_refresh(project_id, &client, &handle, emit);
    }

    /// 设置某按钮的悬停目标（`true`=进入,`false`=离开）；动画由
    /// `advance_hover_anims` 循环把它指数逼近（见 `HoverAnim`）。
    pub fn set_hover(&mut self, id: HoverId, hovered: bool) {
        self.hover_anims.entry(id).or_default().set(hovered);
        // 标题 tooltip 计时:进入即记起点,离开即清(计时满 2s 由视图层
        // `hover_tooltip_ready` 判断,本函数只负责起止)。
        if hovered {
            self.hover_tooltip_starts
                .insert(id, std::time::Instant::now());
        } else {
            self.hover_tooltip_starts.remove(&id);
        }
    }

    /// 推进所有按钮的悬停动画一拍（约 60fps 一拍，由 main.rs 的定时唤醒
    /// 驱动；与光标闪烁同款自驱 redraw 范式）。每拍残余 50%（逼近系数
    /// 0.5），约 80ms 内收敛到目标，视觉上是干脆的 ease-out。
    pub fn advance_hover_anims(&mut self) {
        for a in self.hover_anims.values_mut() {
            a.advance();
        }
        // 浏览器面板有独立 hover 进度机(无法复用全局 `hover_anims`),这里
        // 一并推进:首页 `home_browser` 与每个已加载工作区的浏览器。
        self.home_browser.advance_hover_anims();
        for slot in self.projects.values_mut() {
            if let WorkspaceSlot::Loaded(ws) = slot {
                ws.browser.advance_hover_anims();
            }
        }
        self.advance_rail_slot_anims();
    }

    /// 是否还有按钮的悬停动画在进行中（任一进度未到目标）。
    /// main.rs 据此决定是否继续排下一拍定时唤醒。
    pub fn any_hover_anim_active(&self) -> bool {
        self.hover_anims.values().any(HoverAnim::active)
            || self.home_browser.any_hover_active()
            || self
                .projects
                .values()
                .any(|s| matches!(s, WorkspaceSlot::Loaded(ws) if ws.browser.any_hover_active()))
            || self.any_rail_slot_anim_active()
    }

    /// 推进图标栏按钮的槽位动画一拍——两侧各自按 `rail_layout` 当前顺序
    /// 现算每个面板的目标槽位号,`RailSlotAnim::retarget` 朝它逼近。折进
    /// `advance_hover_anims` 同一个调用点,复用同一套 60fps 自驱 redraw
    /// 节奏,不需要 main.rs 另开一条唤醒源。
    fn advance_rail_slot_anims(&mut self) {
        rail::advance_slot_anims(&self.shell_layout.rail_layout, &mut self.rail_slot_anims);
    }

    /// 是否还有图标栏按钮的槽位动画在进行中。
    fn any_rail_slot_anim_active(&self) -> bool {
        rail::any_slot_anim_active(&self.shell_layout.rail_layout, &self.rail_slot_anims)
    }

    /// `kind` 在 `side` 栏当前应渲染的动画槽位号(浮点,逼近中的
    /// `rail_layout` 下标)。渲染层据此算按钮的 y 偏移,取代直接用
    /// `rail_layout` 下标瞬间跳变。没有动画记录(刚出现在这一侧,还没被
    /// `advance_rail_slot_anims` 追上)或记录的 `side` 跟当前不符(刚跨栏
    /// 落地那一帧)时,直接返回目标槽位号本身,不插值。
    pub(crate) fn rail_slot_position(&self, side: Side, kind: PanelKind, target_idx: usize) -> f32 {
        rail::slot_position(&self.rail_slot_anims, side, kind, target_idx)
    }

    /// 某按钮当前悬停动画进度(0..=1)，给视图层做颜色插值。
    pub fn hover_progress(&self, id: HoverId) -> f32 {
        self.hover_anims.get(&id).map(HoverAnim::t).unwrap_or(0.0)
    }

    /// 某元素当前是否处于 hover **目标态**(0/1,不做平滑插值)。卡片填充/描边
    /// 这类二元视觉用这个:与 `button` 卡的原生 `button::Status::Hovered` 同
    /// 语义(瞬时切换),不像 `hover_progress` 那样带 ease-out 淡入淡出——图标
    /// 颜色过渡需要平滑,卡片背景/边框切换需要干脆,避免 hover 离开后边框还
    /// 拖着淡出一段(观感像"动画停了一下")。
    pub fn hover_target(&self, id: HoverId) -> bool {
        self.hover_anims
            .get(&id)
            .map(|a| a.target > 0.5)
            .unwrap_or(false)
    }

    /// 某页签悬停是否已持续满 `HOVER_TOOLTIP_DELAY`:满则应在视图层弹出标题
    /// 全称 tooltip(`controlled_tooltip` 据此驱动 `Tooltip::show`)。
    pub fn hover_tooltip_ready(&self, id: HoverId) -> bool {
        self.hover_tooltip_starts
            .get(&id)
            .is_some_and(|start| start.elapsed() >= HOVER_TOOLTIP_DELAY)
    }

    /// 距下一个 tooltip 计时满 2s 的最短剩余时间:main.rs 据此排下次唤醒,做到
    /// "恰好满 2s 才重绘",不空转也不延迟。浏览器面板页签的计时一并纳入。
    pub fn next_tooltip_wake(&self) -> Option<std::time::Duration> {
        let mut next = self
            .hover_tooltip_starts
            .values()
            .filter_map(|start| HOVER_TOOLTIP_DELAY.checked_sub(start.elapsed()))
            .min();
        let browser_next = self
            .home_browser
            .next_tooltip_wake()
            .into_iter()
            .chain(self.projects.values().filter_map(|s| match s {
                WorkspaceSlot::Loaded(ws) => ws.browser.next_tooltip_wake(),
                _ => None,
            }))
            .min();
        next = match (next, browser_next) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        };
        next
    }

    /// 拖拽换位:把当前拖起的源项(`self.tab_drag.source`)移到 `group` 组的
    /// `to` 处。源与目标同址/越界/不在拖拽中/未越过 [`tab_drag_past_threshold`]
    /// 均 no-op(阈值过滤见其文档——挡的是单击途中的抖动,不是真实拖拽)。
    /// 换位后把 `self.tab_drag.source` 更新成新位置(续拖以新位置为准),并
    /// 顺带修正受影响的 index-keyed hover 键/激活项。
    fn tab_drag_move(&mut self, group: TabGroup, to: usize) {
        let Some(drag) = self.tab_drag else {
            return;
        };
        if !tab_drag_past_threshold(drag.press_pos, self.last_cursor) {
            return;
        }
        if drag.group != group {
            return;
        }
        let from = drag.source;
        match group {
            TabGroup::Project => {
                if from == to || from >= self.project_order.len() || to >= self.project_order.len()
                {
                    return;
                }
                let id = self.project_order.remove(from);
                self.project_order.insert(to, id);
                self.tab_drag = Some(TabDrag {
                    group,
                    source: to,
                    press_pos: drag.press_pos,
                });
                // 项目页签 hover 存的是 id-keyed 键(`HoverId::ProjectTabItem`/
                // `ProjectTabClose`),值本身不会因换位错配到别的项目——但换位
                // 让页签在**树里的位置**跟着挪，`MouseArea` 自己那份按位置续存
                // 的悬停内部状态（`is_hovered` 等）跟这份 id-keyed 记录对不上
                // 了：原来悬停中的那个 id，它的 `MouseArea` 实例已经换到别的
                // 树位置，不会再收到 `on_exit`，`hover_anims` 里的值就砸在原地
                // 出不来，页签松手后一直亮着金色胶囊（换位越频繁越容易撞上）。
                // 没有更细粒度的续存机制（`MouseArea` 不支持 `.id()`），换位
                // 时索性把两类项目页签 hover 全部清零最省事——真实悬停哪个,
                // 下一帧鼠标移动会立刻重新点亮,观感上无感知。
                self.hover_anims.retain(|k, _| {
                    !matches!(k, HoverId::ProjectTabItem(_) | HoverId::ProjectTabClose(_))
                });
            }
            TabGroup::Terminal => {
                if from == to {
                    return;
                }
                {
                    let Some(ws) = self.active_workspace_mut() else {
                        return;
                    };
                    if to >= ws.tabs.len() {
                        return;
                    }
                    ws.reorder_term_tab(from, to);
                }
                self.tab_drag = Some(TabDrag {
                    group,
                    source: to,
                    press_pos: drag.press_pos,
                });
                self.rekey_hover_range(HoverId::TermTabItem, HoverId::TermTabClose, from, to);
            }
            TabGroup::Preview => {
                if from == to {
                    return;
                }
                {
                    let Some(ws) = self.active_workspace_mut() else {
                        return;
                    };
                    if to >= ws.preview.tabs().len() {
                        return;
                    }
                    ws.preview.reorder(from, to);
                }
                self.tab_drag = Some(TabDrag {
                    group,
                    source: to,
                    press_pos: drag.press_pos,
                });
                self.rekey_hover_range(HoverId::PreviewTabItem, HoverId::PreviewTabClose, from, to);
            }
            TabGroup::ProjectPreview => {
                if from == to {
                    return;
                }
                {
                    let Some(ws) = self.active_workspace_mut() else {
                        return;
                    };
                    if to >= ws.project_preview.tabs().len() {
                        return;
                    }
                    ws.project_preview.reorder(from, to);
                }
                self.tab_drag = Some(TabDrag {
                    group,
                    source: to,
                    press_pos: drag.press_pos,
                });
                self.rekey_hover_range(
                    HoverId::ProjectPreviewTabItem,
                    HoverId::ProjectPreviewTabClose,
                    from,
                    to,
                );
            }
            TabGroup::Browser => {
                if from == to {
                    return;
                }
                {
                    let Some(ws) = self.active_workspace_mut() else {
                        return;
                    };
                    if to >= ws.browser.tab_count() {
                        return;
                    }
                    ws.browser.reorder_tab(from, to);
                }
                self.tab_drag = Some(TabDrag {
                    group,
                    source: to,
                    press_pos: drag.press_pos,
                });
            }
        }
    }

    /// 给 `from..=to` 区间的 index-keyed hover 键整体顺移一位,使其跟上拖拽换位
    /// 后的条目位置:`Item(i)` 与 `Close(i)` 两种键都必须跟着槽位移。`item_f`/
    /// `close_f` 是构造 `HoverId` 的两个构造器(终端/预览各自的 `Item`/`Close`
    /// 变体)。键在拖拽期间通常无动画在跑(拖走即离开),把它们重排到正确槽位
    /// 即可,不追求平滑。
    fn rekey_hover_range(
        &mut self,
        item_f: fn(usize) -> HoverId,
        close_f: fn(usize) -> HoverId,
        from: usize,
        to: usize,
    ) {
        let lo = from.min(to);
        let hi = from.max(to);
        // 取旧槽上每个键的当前动画值,再按换位后的新槽写回。
        let mut remap = Vec::new();
        for i in lo..=hi {
            for f in [item_f, close_f] {
                if let Some(h) = self.hover_anims.remove(&f(i)) {
                    let new_i = if i == from {
                        to
                    } else if from < to {
                        // 向右拖:中间 from+1..=to 全左移一位。
                        i - 1
                    } else {
                        // 向左拖:中间 to..from 全右移一位。
                        i + 1
                    };
                    remap.push((f(new_i), h));
                }
            }
        }
        for (id, h) in remap {
            self.hover_anims.insert(id, h);
        }
    }

    /// 当前激活 tab 的选区文本（⌘C 复制用）。
    pub fn active_selection_text(&self) -> Option<String> {
        self.active_workspace()?.active_selection_text()
    }

    /// 浏览器地址栏是否持有 iced 真实焦点(main.rs 据此路由键盘)。首页
    /// (`is_home()`)展示的是 `home_browser`(全局浏览器,`App` 级独立
    /// 实例,不属于任何 `Workspace`),跟工作区里打开的 `ws.browser` 是
    /// 两个不同的地址栏——两者共用同一个 `addr_field_id()`(不会同时渲染,
    /// 不会冲突),但读写目标必须按当前在首页还是工作区分流,否则首页地址栏
    /// 的焦点态永远查不到(`active_workspace()` 在首页上恒为 `None`)。
    pub fn browser_addr_focused(&self) -> bool {
        if self.is_home() {
            return self.home_browser.addr_focused();
        }
        self.active_workspace()
            .is_some_and(|ws| ws.browser_addr_focused())
    }

    /// 每帧渲染循环调用:把 `extensions::browser::CaptureAddrFocus` 问到
    /// 的真实焦点态写进正确的浏览器实例(首页 `home_browser` 或当前工作区
    /// `ws.browser`,见 `browser_addr_focused` 的说明)。
    pub fn set_browser_addr_focused(&mut self, focused: bool) {
        if self.is_home() {
            self.home_browser.set_addr_focused(focused);
            return;
        }
        if let Some(ws) = self.active_workspace_mut() {
            ws.browser.set_addr_focused(focused);
        }
    }

    /// 取走"地址栏需全选"的一次性标记(消费即复位),路由同
    /// `set_browser_addr_focused`(首页 `home_browser` / 当前工作区
    /// `ws.browser` 二选一)。
    pub fn take_addr_select_all_pending(&mut self) -> bool {
        if self.is_home() {
            return self.home_browser.take_addr_select_all_pending();
        }
        self.active_workspace_mut()
            .is_some_and(|ws| ws.browser.take_addr_select_all_pending())
    }

    /// 项目树行内编辑框是否持有 iced 真实焦点(main.rs 键盘路由用)。为真时
    /// 按键放行给标准 iced 事件管线,交真正的 text_input 自己处理。
    pub fn tree_edit_focused(&self) -> bool {
        self.active_workspace()
            .is_some_and(|ws| ws.tree_edit_focused())
    }

    /// 每帧渲染循环调用:把 `extensions::files::CaptureTreeEditFocus` 问到的
    /// 真实焦点态写进当前工作区(`main.rs` 键盘路由随后读
    /// `tree_edit_focused` 消费)。焦点从真变假(失焦)时在边缘处落盘——
    /// 项目树重命名/新建是"点别处就该保存"的一次性行内编辑,直接调
    /// `ws.files.submit_tree_edit` 走与回车提交(`Message::EditSubmit`)同一
    /// 份逻辑(空名字/未改动会自行退出编辑态,不产生文件/重命名)。
    pub fn set_tree_edit_focused(&mut self, focused: bool) {
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let Some(project_id) = self.active_project_id else {
            return;
        };
        let Some(ws) = self.active_workspace_mut() else {
            return;
        };
        let was_focused = ws.tree_edit_focused();
        if was_focused && !focused && ws.files.tree_edit_is_some() {
            let emit = move |m: files::Message| {
                let _ = proxy.send_event(Message::Files(m));
            };
            ws.files.submit_tree_edit(project_id, &handle, emit);
        }
        ws.files.set_tree_edit_focused(focused);
    }

    /// 文件树搜索框是否持有 iced 真实焦点(main.rs 键盘路由用)。为真时按键
    /// 放行给标准 iced 事件管线,交真正的 text_input 自己处理。
    pub fn files_search_focused(&self) -> bool {
        self.active_workspace()
            .is_some_and(|ws| ws.files_search_focused())
    }

    /// SSH 新增/编辑表单任意字段是否持有真实焦点(main.rs 键盘路由用)。
    /// 早先只看"表单是否打开"这个粗粒度信号,漏了两种"字段其实没有真焦点"
    /// 的场景:①切走 SSH 面板去用别的面板(`editing` 草稿只在 `DraftSave`/
    /// `DraftCancel` 时才清,不会因为切面板自动关掉);②SSH 面板与 Agent
    /// 终端分栏同屏显示,表单开着但用户实际点进的是终端输入框(2026-09
    /// 用户反馈的两次根因)。现在 `ws.ssh_form_open()` 直接查真实焦点(同
    /// `files_search_focused`),两种场景天然都对,不需要再叠"面板是否
    /// 可见"这道闸门。
    pub fn ssh_form_open(&self) -> bool {
        self.active_workspace().is_some_and(|ws| ws.ssh_form_open())
    }

    /// Database 新增/编辑表单任意字段是否持有真实焦点(main.rs 键盘路由用),
    /// 同 `ssh_form_open` 的根因与做法。
    pub fn database_form_open(&self) -> bool {
        self.active_workspace()
            .is_some_and(|ws| ws.database_form_open())
    }

    /// 每帧渲染循环读走 `ssh::CaptureFormFocus` 查到的真实焦点态后写进
    /// 当前工作区。
    pub fn set_ssh_form_focused(&mut self, focused: bool) {
        if let Some(ws) = self.active_workspace_mut() {
            ws.set_ssh_form_focused(focused);
        }
    }

    /// 每帧渲染循环读走 `database::CaptureFormFocus` 查到的真实焦点态后写进
    /// 当前工作区。
    pub fn set_database_form_focused(&mut self, focused: bool) {
        if let Some(ws) = self.active_workspace_mut() {
            ws.set_database_form_focused(focused);
        }
    }

    /// 拖拽移动确认框是否打开(main.rs 键盘路由用)。同 SSH/Database 表单
    /// 的粗粒度口径——框里只有两个字段,整体放行不细分哪个字段真正持有
    /// 焦点,见 `WorkspaceState::pending_move_is_some` 文档。
    pub fn files_move_confirm_open(&self) -> bool {
        self.active_workspace()
            .is_some_and(|ws| ws.files.pending_move_is_some())
    }

    /// 每帧渲染循环调用:把 `extensions::files::CaptureSearchFocus` 问到
    /// 的真实焦点态写进当前工作区(`main.rs` 键盘路由随后读
    /// `files_search_focused` 消费)。
    pub fn set_files_search_focused(&mut self, focused: bool) {
        if let Some(ws) = self.active_workspace_mut() {
            ws.files.set_search_focused(focused);
        }
    }

    /// 右键文件树"搜索"弹窗是否打开(main.rs 键盘路由 + App view 浮层用)。
    pub fn search_popup_open(&self) -> bool {
        self.active_workspace()
            .is_some_and(|ws| ws.search_popup_open())
    }

    /// 右键文件树"搜索"弹窗查询框是否持有 iced 真实焦点(main.rs 原生放行
    /// 闸门用)。
    pub fn query_focused(&self) -> bool {
        self.active_workspace().is_some_and(|ws| ws.query_focused())
    }

    /// 每帧渲染循环读走 `CaptureQueryFocus` 查到的真实焦点态后写进当前
    /// 工作区。
    pub fn set_query_focused(&mut self, focused: bool) {
        if let Some(ws) = self.active_workspace_mut() {
            ws.search.set_query_focused(focused);
        }
    }

    /// 项目信息面板名称编辑框是否持有 iced 真实焦点(main.rs 原生放行闸门用)。
    pub fn project_name_focused(&self) -> bool {
        self.active_workspace()
            .is_some_and(|ws| ws.name_edit_focused())
    }

    /// 每帧渲染循环调用:把 `CaptureNameEditFocus` 问到的真实焦点态写进
    /// 当前工作区,并在"焦点从真变假"的那一刻做落盘判断(同 `App::
    /// set_todo_content_focused` 的既有手法,`submit_name_edit` 走这条
    /// 而不是重新手写一份 `rename_project` 调用)。
    pub fn set_project_name_focused(&mut self, focused: bool) {
        let client = self.client.clone();
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let Some(ws) = self.active_workspace_mut() else {
            return;
        };
        let was_focused = ws.name_edit_focused();
        if was_focused
            && !focused
            && let Some(project) = ws.project.clone()
        {
            let emit = move |m: project::Message| {
                let _ = proxy.send_event(Message::Project(m));
            };
            project::submit_name_edit(
                &mut ws.project_panel,
                project.id,
                &project.name,
                client,
                &handle,
                emit,
            );
        }
        ws.project_panel.set_name_edit_focused_flag(focused);
    }

    /// 项目信息面板描述编辑框是否持有 iced 真实焦点(main.rs 原生放行闸门用)。
    pub fn project_description_focused(&self) -> bool {
        self.active_workspace()
            .is_some_and(|ws| ws.project_panel.description_edit_focused())
    }

    /// 每帧渲染循环调用:把 `CaptureDescriptionEditFocus` 问到的真实焦点态
    /// 写进当前工作区(落盘走既有的 `blur_inputs`→
    /// `submit_description_edit_on_blur` 鼠标点击失焦路径,这里只负责给
    /// main.rs 键盘路由闸门提供信号,同 `set_project_name_focused` 但不需要
    /// 重复一份落盘判断)。
    pub fn set_project_description_focused(&mut self, focused: bool) {
        if let Some(ws) = self.active_workspace_mut() {
            ws.project_panel.set_description_edit_focused_flag(focused);
        }
    }

    /// Todo 面板搜索框是否持有 iced 真实焦点(main.rs 键盘路由用)。
    pub fn todo_search_focused(&self) -> bool {
        self.active_workspace()
            .is_some_and(|ws| ws.todo.search_focused())
    }

    /// 每帧渲染循环调用:把 `extensions::todo::CaptureTodoSearchFocus` 问到
    /// 的真实焦点态写进当前工作区的 Todo(`main.rs` 键盘路由随后读
    /// `todo_search_focused` 消费)。
    pub fn set_todo_search_focused(&mut self, focused: bool) {
        if let Some(ws) = self.active_workspace_mut() {
            ws.todo.set_search_focused(focused);
        }
    }

    /// 会话列表搜索框是否持有 iced 真实焦点(main.rs 键盘路由用)。
    pub fn conversation_search_focused(&self) -> bool {
        self.active_workspace()
            .is_some_and(|ws| ws.conversations.search_focused())
    }

    /// 每帧渲染循环调用:把 `extensions::conversations::CaptureConversationSearchFocus`
    /// 问到的真实焦点态写进当前工作区(`main.rs` 键盘路由随后读
    /// `conversation_search_focused` 消费)。
    pub fn set_conversation_search_focused(&mut self, focused: bool) {
        if let Some(ws) = self.active_workspace_mut() {
            ws.conversations.set_search_focused(focused);
        }
    }

    /// Git Log 面板 commit 搜索框是否持有 iced 真实焦点(main.rs 键盘路由
    /// 用)。`git_log::State` 不按项目分(同 `home_project_search_focused`
    /// 直接挂在 `App` 上,不用走 `active_workspace` 那套间接)。
    pub fn git_log_search_focused(&self) -> bool {
        self.git_log.search_focused()
    }

    /// 每帧渲染循环调用:把 `extensions::git_log::CaptureSearchFocus` 问到
    /// 的真实焦点态写进 `git_log::State`(`main.rs` 键盘路由随后读
    /// `git_log_search_focused` 消费)。
    pub fn set_git_log_search_focused(&mut self, focused: bool) {
        self.git_log.set_search_focused(focused);
    }

    /// 首页项目搜索框是否持有 iced 真实焦点(main.rs 键盘路由用)。
    pub fn home_project_search_focused(&self) -> bool {
        self.home_project_search_focused
    }

    /// 每帧渲染循环调用:把 `CaptureHomeSearchFocus` 问到的真实焦点态
    /// 写进来。
    pub fn set_home_project_search_focused(&mut self, focused: bool) {
        self.home_project_search_focused = focused;
    }

    /// 搜索框草稿落成为生效的 `home_project_search` 过滤词,并把翻页
    /// 重置回第 1 页(同 `todo::WorkspaceState::commit_search`)。
    fn commit_home_project_search(&mut self) {
        self.home_project_search = self.home_project_search_draft.clone();
        self.home_project_pages = 1;
    }

    /// Todo 面板新增任务框是否持有 iced 真实焦点(main.rs 键盘路由用)。
    pub fn todo_add_focused(&self) -> bool {
        self.active_workspace()
            .is_some_and(|ws| ws.todo.add_focused())
    }

    /// 每帧渲染循环调用:把 `extensions::todo::CaptureAddFocus` 问到的真实
    /// 焦点态写进当前工作区的 Todo(`main.rs` 键盘路由随后读
    /// `todo_add_focused` 消费)。
    pub fn set_todo_add_focused(&mut self, focused: bool) {
        if let Some(ws) = self.active_workspace_mut() {
            ws.todo.set_add_focused(focused);
        }
    }

    /// Todo 任务内容编辑框是否持有 iced 真实焦点(main.rs 键盘路由用)。
    pub fn todo_content_focused(&self) -> bool {
        self.active_workspace()
            .is_some_and(|ws| ws.todo.content_edit_focused())
    }

    /// 每帧渲染循环调用:把 `CaptureContentEditFocus` 问到的真实焦点态
    /// 写进当前工作区的 Todo,并在"焦点从真变假"的那一刻做落盘判断
    /// (同 `Workspace::blur_inputs` 原先的"有项目就 commit、没项目就
    /// cancel"逻辑,只是触发时机从"点击别处"改成"真实焦点丢失")。
    pub fn set_todo_content_focused(&mut self, focused: bool) {
        let Some(ws) = self.active_workspace_mut() else {
            return;
        };
        let was_focused = ws.todo.content_edit_focused();
        // 失焦回退:取走待提交的草稿改动(`commit_content_edit` 返回
        // `Some((id, new_text))` 表示草稿确有改动,且会消费 `editing_content`);
        // 无改动/空草稿返回 `None`,到此随 `editing_content` 一并丢弃。
        let pending_commit = if was_focused && !focused {
            ws.todo.commit_content_edit()
        } else {
            None
        };
        ws.todo.set_content_edit_focused_flag(focused);
        // 先把 `ws` 的借用放掉,再经 self 的 client/handle/proxy 发起异步提交
        // (否则 `active_workspace_mut` 对 `self` 的可变借用会挡住 `self.client`)。
        if let Some((id, new_text)) = pending_commit {
            let client = self.client.clone();
            let handle = self.handle.clone();
            let proxy = self.proxy.clone();
            handle.spawn(async move {
                let res = client
                    .edit_todo_text(id, &new_text)
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string());
                let _ = proxy.send_event(Message::Todo(todo::Message::Mutated(res)));
            });
        }
    }

    /// 分类改名框是否持有 iced 真实焦点(main.rs 键盘路由用)。
    pub fn category_rename_focused(&self) -> bool {
        self.active_workspace()
            .is_some_and(|ws| ws.todo.category_rename_focused())
    }

    /// 分类改名框真实焦点态每帧写回;失焦边缘(`was_focused && !focused`)
    /// 触发一次提交(镜像 `set_todo_content_focused`,只是落盘方法换成
    /// `rename_category`,成功/失败都触发 `CategoryMutated` 刷新)。
    pub fn set_category_rename_focused(&mut self, focused: bool) {
        let Some(ws) = self.active_workspace_mut() else {
            return;
        };
        let was_focused = ws.todo.category_rename_focused();
        let pending_commit = if was_focused && !focused {
            ws.todo.commit_category_rename_for_blur()
        } else {
            None
        };
        ws.todo.set_category_rename_focused_flag(focused);
        if let Some((id, new_name)) = pending_commit {
            let client = self.client.clone();
            let handle = self.handle.clone();
            let proxy = self.proxy.clone();
            handle.spawn(async move {
                let res = client
                    .rename_category(id, &new_name)
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string());
                let _ = proxy.send_event(Message::Todo(todo::Message::CategoryMutated(res)));
            });
        }
    }

    /// 详情弹窗回复框是否持有 iced 真实焦点(main.rs 键盘路由用)。
    pub fn detail_reply_focused(&self) -> bool {
        self.active_workspace()
            .is_some_and(|ws| ws.todo.detail_reply_focused())
    }

    /// 每帧渲染循环调用:把 `CaptureDetailReplyFocus` 问到的真实焦点态
    /// 写进当前工作区的 Todo(`main.rs` 键盘路由随后读 `detail_reply_focused`
    /// 消费)。这个输入没有"失焦提交"的语义(提交靠点"处理"按钮,不是
    /// 失焦/回车),所以不需要 `category_rename_focused`/`todo_content_focused`
    /// 那种"失焦边缘触发落盘"的逻辑,直接写回标记位即可。
    pub fn set_detail_reply_focused(&mut self, focused: bool) {
        if let Some(ws) = self.active_workspace_mut() {
            ws.todo.set_detail_reply_focused_flag(focused);
        }
    }

    /// 当前左栏显示哪个面板(main.rs 每帧 `interface.operate` 捕获 Todo 自绘
    /// 输入字段 bounds 时用来判断是否要遍历,避免无谓开销)。
    pub fn left_view(&self) -> PanelKind {
        self.left_view
    }

    /// 同 `left_view`,右栏当前显示的面板种类——Files 面板可以被拖到右栏
    /// (`relocate_to_right`),main.rs 的键盘焦点捕获闸门(`CaptureSearchFocus`/
    /// `CaptureTreeEditFocus`)必须两栏都查,只查 `left_view` 会在 Files 挪到
    /// 右栏后让搜索框/树内编辑框永远捕不到真实焦点(2026-09 用户反馈:文件树
    /// 搜索框点了也打不进字)。
    pub fn right_view(&self) -> PanelKind {
        self.right_view
    }

    /// 是否正在拖拽 Todo 任务排序(main.rs 鼠标释放路由 + about_to_wait
    /// 持续重绘用;同 `dragging_tab` 那套)。
    pub fn todo_dragging(&self) -> bool {
        self.active_workspace()
            .is_some_and(|ws| ws.todo.drag_active())
    }

    /// 距新增闪光自动清除的剩余时间:main.rs 据此排下次唤醒,恰好到点重绘
    /// 一次清除高亮(同 `next_tooltip_wake` 的定时范式)。
    pub fn next_todo_flash_wake(&self) -> Option<std::time::Duration> {
        self.active_workspace()?.todo.next_flash_wake()
    }

    /// 推进新增闪光倒计时(每帧 `new_events` 调用):到点且用户未手动改选则
    /// 自动清除选中高亮。
    pub fn advance_todo_flash(&mut self) {
        if let Some(ws) = self.active_workspace_mut() {
            ws.todo.advance_flash();
        }
    }

    /// 取走"Todo 列表滚回顶部"的一次性滚动位(main.rs 渲染循环消费)。
    pub fn take_todo_scroll_to_top(&mut self) -> bool {
        self.active_workspace_mut()
            .is_some_and(|ws| ws.todo.take_scroll_to_top())
    }

    /// 当前项目根路径(供 main.rs 算相对路径用;未打开项目时 None)。
    pub fn active_project_path(&self) -> Option<PathBuf> {
        self.active_workspace()?.active_project_path()
    }

    /// 当前激活 tab 是否处于 application cursor mode（DECCKM）。
    pub fn active_app_cursor_mode(&self) -> bool {
        self.active_workspace()
            .is_some_and(|ws| ws.active_app_cursor_mode())
    }

    /// `kind` 是 `is_in_preview_column` 命中的面板(`Files` 或 `Project`),
    /// 据此查 `ws.preview`(Files)还是 `ws.project_preview`(Project)——后者
    /// 的 id 已加 `PROJECT_PREVIEW_ID_OFFSET`(见 workspace.rs 同名方法)。
    /// main.rs 焦点路由取句柄用。
    pub fn active_preview_webview_id(&self, kind: PanelKind) -> Option<usize> {
        self.active_workspace()?.active_preview_webview_id(kind)
    }

    /// 当前哪个预览面板有活跃 webview(`Files`/`Project`/`None`),
    /// 语义见 workspace.rs 同名方法。`WebViewFocused` 这种不携带面板
    /// 信息的信号需要反推池身份时用。
    pub fn active_preview_panel_kind(&self) -> Option<PanelKind> {
        self.active_workspace()?.active_preview_panel_kind()
    }

    /// 当前激活浏览器 tab 的 webview id,语义同 `active_preview_webview_id`。
    pub fn active_browser_webview_id(&self) -> Option<usize> {
        if self.current_page == AppPage::Home {
            return self.home_browser.active_webview_id();
        }
        self.active_workspace()?.active_browser_webview_id()
    }

    /// 当前是否在首页(`AppPage::Home`)——内核(main.rs 的 webview 池同步)
    /// 据此判断浏览器 webview 该用右面板区边界还是工作区左面板预览边界。
    pub(crate) fn is_home(&self) -> bool {
        self.current_page == AppPage::Home
    }

    /// 协议闭包共享的文件白名单句柄(当前项目的那一份)。没有项目打开时
    /// `preview_desired`/`browser_desired` 也必然为空、不会有 webview 去查
    /// 这个白名单,给一个空的即可。
    pub fn allowed_files(&self) -> Arc<Mutex<HashSet<PathBuf>>> {
        match self.active_workspace() {
            Some(ws) => ws.allowed_files(),
            None => Arc::new(Mutex::new(HashSet::new())),
        }
    }

    /// 协议闭包共享的审阅内容快照句柄(当前项目的那一份)——同
    /// `allowed_files` 的手法,webview 创建时按聚焦项目捕获,天然做到
    /// per-project 隔离,不会跨项目串数据。
    pub fn review_snapshot(&self) -> Arc<Mutex<Option<String>>> {
        match self.active_workspace() {
            Some(ws) => ws.review_snapshot(),
            None => Arc::new(Mutex::new(None)),
        }
    }

    /// 点击输入框外时退出所有自绘输入的编辑态(验收反馈:失焦回正常态)。
    /// 项目名称编辑的"失焦保存"已搬进 `set_project_name_focused` 的边缘触发
    /// (与回车提交共用 `extensions::project::submit_name_edit`),这里只交
    /// 给 `Workspace::blur_inputs` 清其它编辑态。
    ///
    /// `keep_native_preview_editor` 为 `true` 时保留原生预览编辑器的焦点(见
    /// `Workspace::blur_inputs`/`active_tab_is_native` 的说明):这次左键按下
    /// 若落在原生 `text_editor` 上,编辑器自己那帧会 self-focus 出光标,不能再
    /// 顺带 `blur`——2026-09-06 "点代码预览无法获得光标" 修复。
    pub fn blur_inputs(&mut self, keep_native_preview_editor: bool) {
        let Some(ws) = self.active_workspace_mut() else {
            return;
        };
        ws.blur_inputs(keep_native_preview_editor);
    }

    /// 当前指定 `PanelKind`(Files/Project)预览列是否"本身是原生可编辑预览"
    /// (激活 tab 是 `CodeView`)。main.rs 在左键按下路由到该列时据此决定
    /// `blur_inputs` 要不要保留预览编辑器(-->)。
    pub fn preview_active_tab_is_native(&self, kind: PanelKind) -> bool {
        let Some(ws) = self.active_workspace() else {
            return false;
        };
        match kind {
            PanelKind::Files => ws.preview.active_tab_is_native(),
            PanelKind::Project => ws.project_preview.active_tab_is_native(),
            _ => false,
        }
    }

    /// 键盘焦点被消息(非鼠标点击)拨离预览列时调用——`main.rs` 里切终端
    /// tab/新会话落成/选中 agent 都走这条路径,不经过 `WindowEvent::
    /// MouseInput` 那次 `blur_inputs()`,原生预览编辑器不会自己让出焦点
    /// （见 `Workspace::blur_preview_editors` 的说明），得单独补一次。
    pub fn blur_preview_editors(&mut self) {
        if let Some(ws) = self.active_workspace_mut() {
            ws.blur_preview_editors();
        }
    }

    /// 把几何状态(宽度/分割比例/窗口尺寸,**不含**左右视图选择与收起态——
    /// 那些按项目分,见 `panel_layouts`)写盘。图标切换/收起要立即持久化几何,
    /// 不能只靠 `ColumnDragEnd` 顺带存(用户可能从没拖过分隔线)。
    fn spawn_shell_layout_save(&mut self) {
        let layout = self.shell_layout.clone();
        self.handle.spawn(async move {
            if let Err(e) = layout::save(&layout) {
                tracing::warn!("外壳布局写盘失败: {e}");
            }
        });
    }

    /// 当前正显示项目的面板布局(活值)打包成 `PanelLayout`。
    fn current_panel_layout(&self) -> PanelLayout {
        PanelLayout {
            left_view: self.left_view,
            right_view: self.right_view,
            left_collapsed: self.left_collapsed,
            right_collapsed: self.right_collapsed,
            dims: self.dims,
        }
    }

    /// 把当前活值存回"当前活跃项目"在 `panel_layouts` 里的那份,并异步写盘。
    /// 切换项目**之前**调:此时 `active_project_id` 还指着老项目,于是老项目
    /// 的面板状态被原样记下,绝不会被接下来要切过去的新项目盖掉。
    fn stash_active_panel_layout(&mut self) {
        if let Some(id) = self.active_project_id {
            self.panel_layouts.insert(id, self.current_panel_layout());
            self.spawn_panel_layouts_save();
        }
    }

    /// 把 `id` 项目自己存的面板布局取出来灌进活值,让界面切到它的样子。
    /// `id` 还没存过(layout.json 升级前/第一次开)时退化成 `PanelLayout::
    /// default()`,跟旧行为一致。
    fn adopt_panel_layout(&mut self, id: i64) {
        let pl = self.panel_layouts.get(&id).copied().unwrap_or_default();
        self.left_view = pl.left_view;
        self.right_view = pl.right_view;
        // 磁盘数据可能来自 `end_rail_drag` 修复前写入的坏状态:两侧
        // active view 撞成同一个 `kind`,渲染时同一面板画两遍,复现过
        // wgpu StagingBelt "still mapped" panic(2026-08-20 崩溃排查)。
        // 落盘数据不可信,载入时兜底一次。
        if self.right_view == self.left_view {
            self.right_view = self
                .shell_layout
                .rail_layout
                .side(Side::Right)
                .iter()
                .find(|k| **k != self.left_view)
                .copied()
                .unwrap_or(self.right_view);
        }
        self.left_collapsed = pl.left_collapsed;
        self.right_collapsed = pl.right_collapsed;
        self.dims = pl.dims;
    }

    /// 把整份 `panel_layouts`(所有项目的面板布局)异步写盘。
    fn spawn_panel_layouts_save(&mut self) {
        let map = self.panel_layouts.clone();
        self.handle.spawn(async move {
            if let Err(e) = panel_layouts::save(&map) {
                tracing::warn!("面板布局写盘失败: {e}");
            }
        });
    }

    /// 外壳几何状态(视图选择/收起态/分隔线位置)变化后的统一收尾:持久化 +
    /// 按新几何重算终端网格。任何改变"终端 pane 实际拿到多少像素"的
    /// handler 都该走这里,不要只调其中一半——只存不重算,终端网格会停在
    /// 上一次窗口 resize 时的尺寸(Fix round 2 #6)。
    fn on_shell_layout_changed(&mut self) {
        // 视图选择/收起态变了:先记进当前活跃项目自己的那份(别的项目不动),
        // 再存几何、重算网格。
        self.stash_active_panel_layout();
        self.spawn_shell_layout_save();
        self.sync_terminal_grid();
    }

    /// main.rs 建窗/`WindowEvent::Resized` 时告知当前窗口逻辑尺寸:记下来
    /// (`view()`/几何公式都要用),并按新尺寸重算终端网格。
    pub fn set_window_size(&mut self, width: f32, height: f32) {
        self.window_size = (width, height);
        self.sync_terminal_grid();
    }

    /// main.rs 左键按下时告知点击落点(逻辑坐标):按 `zone_at_x` 换算落在
    /// 哪一侧面板区,命中就更新 `active_zone`(驱动 `left_zone`/
    /// `right_zone` 外边框高亮)。落在图标栏/两侧都收起等 `None` 场景保持
    /// 原有聚焦态不变——点导航图标不该清空"上一次在哪侧干活"的高亮。
    pub fn set_active_zone(&mut self, x: f32, window_width: f32) {
        if let Some(zone) = zone_at_x(x, window_width, &self.shell_state()) {
            self.active_zone = Some(zone);
        }
    }

    /// main.rs 键盘/粘贴路由用的目标终端(`Shared`/`SshPanel`)。委托给
    /// 纯函数 `keyboard_term_target`(单测用),这里只补上 `App` 私有字段的
    /// 读取。
    pub(crate) fn keyboard_term_target(&self) -> terminal::TermTarget {
        terminal::keyboard_term_target(self.left_view, self.active_zone)
    }

    /// 当前终端 IME 组字预览文本(`term_view` 渲染 + `ime_cursor_area` 算
    /// 候选窗位置共用同一份状态)。
    pub(crate) fn term_ime_preedit(&self) -> Option<&str> {
        self.term_ime_preedit.as_deref()
    }

    /// 建窗时用的初始窗口尺寸偏好:优先用上次退出前持久化的
    /// `shell_layout.window_width/height`(已经过 `sanitize_shell_layout`
    /// 夹取),`layout.json` 不存在/读不到时 `layout::load()` 本身已经退化
    /// 成 `ShellLayout::default()`,即 `byteui::theme::geometry::initial_window_size()`,这里不用再
    /// 单独处理"没存过"的分支。
    pub fn window_size_pref(&self) -> (f32, f32) {
        (
            self.shell_layout.window_width,
            self.shell_layout.window_height,
        )
    }

    /// 退出前把当前窗口尺寸并入 `shell_layout` 落盘(main.rs 在
    /// `WindowEvent::CloseRequested` 时调用,`event_loop.exit()` 之前)。
    /// 不走 `spawn_shell_layout_save` 的异步落盘——进程马上退出,spawn 的
    /// tokio 任务不保证能在进程终止前跑完;这里退化成同步写,反正只在
    /// 退出这一刻触发一次,不占渲染帧预算。
    pub fn persist_window_size_on_exit(&mut self) {
        self.shell_layout.window_width = self.window_size.0;
        self.shell_layout.window_height = self.window_size.1;
        if let Err(e) = layout::save(&self.shell_layout) {
            tracing::warn!("退出前窗口尺寸写盘失败: {e}");
        }
    }

    /// 退出前等"关 tab 时发往 daemon 的 kill/总结请求"真正跑完(main.rs 在
    /// `WindowEvent::CloseRequested` 时调用,`event_loop.exit()` 之前)。
    ///
    /// 这些请求走 `io.handle.spawn` fire-and-forget,只需一次本地 UDS
    /// 往返(通常亚毫秒级)。但如果用户关 tab 后紧接着退出,`event_loop.exit()`
    /// 后 `run_app` 返回、tokio `Runtime` 被 drop——drop 不保证在飞的
    /// spawn 任务跑完,kill 请求可能根本没发出去,daemon 侧会话仍是
    /// `alive`,下次启动就被恢复策略(`s.alive && s.project_id == ...`)
    /// 当成"还开着"重新挂回来(用户报告的验收反馈:显式关闭的 tab 不该
    /// 在重启后还魂)。
    ///
    /// 这里是继启动序列 `runtime.block_on(build_app(..))` 之后,UI 线程
    /// 上第二处、也是唯一一处允许 `block_on` 的地方——同样的理由:窗口
    /// 马上要关,不存在"占用正在渲染的 UI 线程"的问题。有限超时防止
    /// daemon 卡死/socket 挂起时把退出拖住。
    pub fn wait_for_pending_exit_tasks(&self) {
        let tasks: Vec<_> = match self.pending_exit_tasks.lock() {
            Ok(mut pending) => std::mem::take(&mut *pending),
            Err(_) => return,
        };
        if tasks.is_empty() {
            return;
        }
        self.handle
            .block_on(join_pending_exit_tasks(tasks, EXIT_TASK_BUDGET));
    }

    /// 按当前窗口尺寸+外壳状态重算终端网格并同步给所有 tab / daemon。
    ///
    /// 用于换算的是"右侧展开且显示 Agent 配对"这个假想状态,而不是当前
    /// 真实状态:终端此刻可能不可见(右侧收起 / 右视图是对话),但它的 PTY
    /// 网格仍应按"被显示时占多大"来定——否则上次退出前停在对话视图的会话,
    /// 重开 app 后会一直卡在 `DEFAULT_COLS`×`DEFAULT_ROWS`(80×24),直到用户
    /// 偶然拖一下窗口才纠正(Fix round 2 #6)。假想状态用 `ShellState`
    /// 的字段覆盖表达,不另写一套宽度公式,避免两份几何漂移。
    /// 按当前窗口尺寸+外壳状态重算终端网格并同步给所有 tab / daemon。
    ///
    /// 用于换算共享终端的是"右侧展开且显示 Agent 配对"这个假想状态,而不是
    /// 当前真实状态:终端此刻可能不可见(右侧收起 / 右视图是对话),但它的
    /// PTY 网格仍应按"被显示时占多大"来定——否则上次退出前停在对话视图的
    /// 会话,重开 app 后会一直卡在 `DEFAULT_COLS`×`DEFAULT_ROWS`(80×24),
    /// 直到用户偶然拖一下窗口才纠正(Fix round 2 #6)。假想状态用
    /// `ShellState` 的字段覆盖表达,不另写一套宽度公式,避免两份几何漂移。
    fn sync_terminal_grid(&mut self) {
        let (w, h) = self.window_size;
        let shown = terminal_grid_state(self.shell_state());
        let (pane_w, pane_h) = terminal_pane_pixel_size(w, h, &shown);
        let (cols, rows) = crate::term_view::grid_size(pane_w, pane_h);
        // SSH 面板内嵌终端挂在左面板区,几何与共享终端完全不同,由
        // `ssh_terminal_pane_pixel_size` 按左栏 `主机列表|终端` 配对换算一份
        // 独立网格——否则 SSH 终端永远套用共享终端的列数,窗口/分隔条一动
        // 宽度就跟不上宿主面板(见该函数注释)。
        let (ssh_pane_w, ssh_pane_h) = ssh_terminal_pane_pixel_size(w, h, &self.shell_state());
        let (ssh_cols, ssh_rows) = crate::term_view::grid_size(ssh_pane_w, ssh_pane_h);
        // 共享终端与 SSH 终端各自只在当前可见时才有可测量的 pane。某个 pane
        // 此刻不可换算(右侧收起 / 左面板区收起)时,它的终端可能仍挂在后台
        // (SSH tab 跨左视图常驻),这时沿用上一次跟踪的网格、发一个等值尺寸
        // 给 `PaneResized`——`pane_resized` 内部去重,套用后网格保持正确。
        let shared_ok = cols > 0 && rows > 0;
        let ssh_ok = ssh_cols > 0 && ssh_rows > 0;
        let cols = if shared_ok { cols } else { self.cols as usize };
        let rows = if shared_ok { rows } else { self.rows as usize };
        let ssh_cols = if ssh_ok {
            ssh_cols
        } else {
            self.ssh_cols as usize
        };
        let ssh_rows = if ssh_ok {
            ssh_rows
        } else {
            self.ssh_rows as usize
        };
        self.update(Message::PaneResized {
            cols: cols as u16,
            rows: rows as u16,
            ssh_cols: ssh_cols as u16,
            ssh_rows: ssh_rows as u16,
        });
    }

    /// 左面板区当前**有效**宽度:每项目持久化宽按当前窗口宽夹取(见
    /// `clamp_left_width`)。渲染侧(`left_panel_area`)必须用这个值,而不是
    /// 直接读 `self.dims.left_width`——几何侧(`left_zone_width`)走的是
    /// 同一个 `clamp_left_width`,两边只有共用同一份夹取才不会漂移。
    fn effective_left_width(&self) -> f32 {
        clamp_left_width(self.window_size.0, self.dims.left_width)
    }

    /// 终端 pane 此刻是否真的呈现在用户眼前(判定见自由函数
    /// [`terminal_visible`];逻辑只此一份,便于单测直接喂 `ShellState`)。
    fn terminal_visible(&self) -> bool {
        terminal_visible(&self.shell_state())
    }

    /// 镜像 `terminal_visible` 的方法包装:SSH 面板内嵌终端是否可见。
    fn ssh_terminal_visible(&self) -> bool {
        ssh_terminal_visible(&self.shell_state())
    }

    /// 当前外壳几何状态快照(main.rs 拖拽追踪/离屏几何计算用;`Clone`
    /// 类型,按值返回快照)。
    pub fn shell_state(&self) -> ShellState {
        ShellState {
            layout: self.shell_layout.clone(),
            dims: self.dims,
            left_view: self.left_view,
            left_collapsed: self.left_collapsed,
            right_view: self.right_view,
            right_collapsed: self.right_collapsed,
            browser_bookmarks_open: self
                .active_workspace()
                .map(|ws| ws.browser.bookmarks_open())
                .unwrap_or(false),
            maximized: self.maximized,
        }
    }

    /// 当前正在拖拽的分隔线(main.rs 拖拽追踪用,调用方为
    /// `on_window_event` 的 `CursorMoved`/`MouseInput{Released}` 分支)。
    pub fn dragging_divider(&self) -> Option<Divider> {
        self.dragging
    }

    /// 当前正在拖拽的纵向分隔线(main.rs 拖拽追踪用,调用方同
    /// `dragging_divider`)。
    pub fn dragging_row(&self) -> Option<RowDivider> {
        self.dragging_row
    }

    /// 当前正在拖拽的页签(main.rs 拖拽追踪用,调用方同 `dragging_divider`)。
    pub fn dragging_tab(&self) -> Option<TabDrag> {
        self.tab_drag
    }

    /// 当前是否正按住 `group` 组的页签(渲染侧据此把光标改成"抓取"把手)。
    pub fn dragging_group(&self, group: TabGroup) -> bool {
        self.tab_drag.is_some_and(|d| d.group == group)
    }

    /// 图标栏按钮拖拽悬停到 `side` 栏的第 `to` 个位置。同栏内是重排
    /// (立即生效,`Vec::remove`+`insert`);跨栏只记悬停目标,交给
    /// `end_rail_drag` 统一提交——避免每帧 `CursorMoved` 都触发一次
    /// `Vec` 搬移和后续的布局存盘。
    fn rail_drag_move(&mut self, side: Side, to: usize) {
        let Some(mut drag) = self.rail_drag else {
            return;
        };
        rail::rail_drag_move_into(&mut self.shell_layout.rail_layout, &mut drag, side, to);
        self.rail_drag = Some(drag);
    }

    /// 结束图标栏拖拽:松手即"锁定"——
    ///
    /// - 纯同栏重排(悬停目标期间 `rail_drag_move` 已把 `rail_layout` 实时
    ///   改到位,这里只有清空拖拽态;`source_index != origin_index` 说明真
    ///   发生了重排,据此把新顺序落盘)。
    /// - 跨栏悬停过另一栏:提交跨栏移动并落盘。
    /// - 两者皆无(按住后原地松开):只清拖拽态,不动布局、不落盘。
    ///
    /// 无论如何拖拽态都在此终止(`take()`),松手后不会再被任何残留的
    /// `RailDragMove` 驱动。
    fn end_rail_drag(&mut self) {
        let Some(drag) = self.rail_drag.take() else {
            return;
        };
        let Some((target_side, target_index)) = drag.pending_cross_side else {
            // 纯同栏重排路径:若真重排过(源下标偏离起始值),把新顺序落盘。
            if drag.source_index != drag.origin_index {
                self.on_shell_layout_changed();
            }
            return;
        };
        let Some(kind) = rail::rail_cross_apply(
            &mut self.shell_layout.rail_layout,
            drag.source_side,
            drag.source_index,
            target_side,
            target_index,
        ) else {
            // 源栏只剩这一个面板,搬走会清空——挡住,状态已经在上面
            // `take()` 时清空,这里直接返回即可,`RailLayout` 未改动。
            return;
        };
        // 面板搬走后,若源栏原本正显示的就是它,那个 active view 就悬空了
        // ——不补救的话会跟目标栏同时显示同一个 `kind`,两侧渲染出重复的
        // 面板实例(重复的 widget id/图片纹理请求),曾在拖回来回几次后
        // 稳定复现 wgpu `StagingBelt` "still mapped" panic(见
        // 2026-08-20 崩溃排查)。源栏移除后必然还剩至少一个面板(`rail_
        // cross_apply` 不允许栏清空),落到它现在的第一个面板上。
        let source_view = match drag.source_side {
            Side::Left => &mut self.left_view,
            Side::Right => &mut self.right_view,
        };
        if *source_view == kind {
            *source_view = *self
                .shell_layout
                .rail_layout
                .side(drag.source_side)
                .first()
                .expect("rail_cross_apply 保证源栏搬空前至少剩一个面板");
        }

        // 被移动面板成为目标栏新 active,跟随"移动后在按钮所在一侧打开
        // 面板"的要求。跨栏移动结束后统一走 `on_shell_layout_changed`
        // (存盘 + 重算网格)。
        match target_side {
            Side::Left => {
                self.left_view = kind;
                self.left_collapsed = false;
            }
            Side::Right => {
                self.right_view = kind;
                self.right_collapsed = false;
            }
        }
        self.on_shell_layout_changed();
    }

    /// 当前是否正按住某个图标栏按钮(渲染侧据此把光标改成"抓取"把手,
    /// 同 `dragging_group` 对 `TabDrag` 的用法)。
    pub fn dragging_rail(&self) -> bool {
        self.rail_drag.is_some()
    }

    /// 当前是否正在拖拽文件树内的某一行(main.rs 全局左键松开靠它判断该不
    /// 该发 `TreeDragEnd` 收尾——同 `dragging_rail`/`dragging_tab` 的用法)。
    pub(crate) fn dragging_tree_item(&self) -> bool {
        self.active_workspace()
            .is_some_and(|ws| ws.files.is_dragging_tree_item())
    }

    /// 树内拖拽是否已确认(`Dragging` 阶段,见 `TreeDragPhase` 文档)——
    /// 供顶层 `view()` 判断要不要叠加 `files::tree_drag_ghost` 幽灵胶囊
    /// (同 `rail_drag_confirmed()` 驱动 `rail_drag_ghost` 的用法)。
    pub(crate) fn tree_drag_confirmed(&self) -> bool {
        self.active_workspace()
            .is_some_and(|ws| ws.files.tree_drag_confirmed())
    }

    /// 每次 `CursorMoved`(main.rs 调用)都判断一次:当前树内拖拽若还在
    /// `Pending`、且光标位移越过 [`tree_drag_past_threshold`]、按住时长
    /// 越过 [`tree_drag_held_long_enough`],就推进到 `Dragging`——这个
    /// 转换只发生一次(`confirm_tree_drag` 对已是 `Dragging` 的状态是
    /// no-op),之后 `TreeDragEnd` 的 `confirmed` 直接读这个结果,不必在
    /// 松开时重新算(见 `TreeDragPhase`/`Message::TreeDragRelease` 文档)。
    /// 返回是否真的发生了这次转换,供调用方决定要不要 `request_redraw`
    /// (转换会让行开始挂 `on_move`/显示抓取光标/幽灵胶囊,不重绘看不出来)。
    /// `left_mouse_down` 是硬性前提(见 `main.rs` 里 `left_mouse_down` 字段
    /// 文档):不管存好的按下坐标/时间戳算出来是否越过阈值,左键这一刻没
    /// 有物理按住就绝不确认——顺手把任何残留的 `tree_drag` 自愈清空(正常
    /// 路径下 `TreeDragRelease` 早该清过一次;还留着只可能是那次收尾因为
    /// 某种原因没触发,不是一次合法的、仍在进行的拖拽)。
    pub(crate) fn maybe_confirm_tree_drag(&mut self, left_mouse_down: bool) -> bool {
        if !left_mouse_down {
            if let Some(ws) = self.active_workspace_mut() {
                ws.files.cancel_tree_drag();
            }
            return false;
        }
        let should_confirm = self.active_workspace().is_some_and(|ws| {
            ws.files.tree_drag_is_pending()
                && ws
                    .files
                    .tree_drag_press_pos()
                    .is_some_and(|p| tree_drag_past_threshold(p, self.last_cursor))
                && ws
                    .files
                    .tree_drag_armed_at()
                    .is_some_and(|t| tree_drag_held_long_enough(t.elapsed()))
        });
        if should_confirm && let Some(ws) = self.active_workspace_mut() {
            ws.files.confirm_tree_drag();
        }
        should_confirm
    }

    /// 当前正被拖拽的面板种类(`None` = 未在拖拽)——视图层(`icon_rail`
    /// 源图标变淡 / `rail_drag_ghost` 幽灵图标取图标)据此判断"这是不是
    /// 我"。薄包装 `dragged_panel_kind` 自由函数(同 `rail_drag_move`
    /// 包装 `rail_drag_move_into` 的既有手法),不重复实现逻辑。
    pub fn dragged_panel_kind(&self) -> Option<PanelKind> {
        rail::dragged_panel_kind(&self.shell_layout.rail_layout, self.rail_drag)
    }

    /// `dragging_rail()` 为真(已按下武装)且光标已相对按下点位移超过
    /// [`RAIL_DRAG_VISUAL_THRESHOLD_PX`]——只有这时才算"确认是一次拖拽,
    /// 不是单击",视图层(`icon_rail` 源图标变淡/抓手光标、`rail_drag_
    /// ghost` 幽灵图标)一律看这个而不是 `dragging_rail()`,避免快速单击
    /// 也闪一下拖拽视觉(见 `RailDrag::press_pos` 文档)。`RailDragEnd` 的
    /// 收尾逻辑(`main.rs`/`end_rail_drag`)不受影响,继续按 `dragging_rail()`
    /// 判断,因为松手清理拖拽态这件事无论有没有越过阈值都要做。
    pub fn rail_drag_confirmed(&self) -> bool {
        self.rail_drag
            .is_some_and(|d| rail::rail_drag_past_threshold(d, self.last_cursor))
    }

    /// 结束页签拖拽:清掉拖拽态,若是项目页签组还把新顺序写盘。松开左键的
    /// 两条路径都会走到这里——winit 的 `MouseInput{Released}`(`TabDragEnd`)
    /// 与子 webview 上 JS 上报的 `mouseup`(`WebViewMouseUp`)——保证拖拽态
    /// 在任何情况下都不会残留。
    fn end_tab_drag(&mut self) {
        if let Some(drag) = self.tab_drag.take()
            && drag.group == TabGroup::Project
        {
            // 项目页签顺序变了——写盘(同打开项目那条持久化路径)。
            self.persist_open_projects();
        }
    }

    /// 项目树右键菜单是否打开(main.rs Esc 键路由用)。
    pub fn context_menu_open(&self) -> bool {
        self.files.context_menu_is_some()
            || self.project_link_menu.is_some()
            || self.text_input_menu.is_some()
            || self.database_source_menu.is_some()
    }

    /// 输入框右键菜单是否打开(main.rs Esc 键路由用)。
    pub fn text_input_menu_open(&self) -> bool {
        self.text_input_menu.is_some()
    }

    /// main.rs 在 `dispatch` 循环之后取走一次,取到即消费——见
    /// `pending_native_menu_edit_key` 字段文档。
    pub(crate) fn take_pending_native_menu_edit_key(
        &mut self,
    ) -> Option<(char, iced_widget::core::widget::Id)> {
        self.pending_native_menu_edit_key.take()
    }

    /// main.rs 读取"本帧若产生右上角输入框右键菜单动作,要作用到的输入
    /// 焦点",清空后返回。`TextInputMenuOpen` 时写入,供复制/粘贴作用于
    /// 被右键的输入。
    pub fn take_pending_text_input_focus(&mut self) -> Option<iced_widget::core::widget::Id> {
        self.pending_text_input_focus.take()
    }

    /// main.rs 读取"输入框右键菜单当前要作用的输入 id"。菜单展开时
    /// `TextInputMenuOpen` 已写入 `target`,main.rs 在合成复制/粘贴键盘事件前
    /// 用它把焦点再补一次到被右键的输入(保证作用于它而不是别的)。
    pub fn text_input_menu_target_id(&self) -> Option<iced_widget::core::widget::Id> {
        self.text_input_menu.as_ref().map(|m| m.target.id.clone())
    }

    /// Project 面板链接行右键菜单是否打开(main.rs Esc 键路由用)。
    pub fn project_link_context_menu_open(&self) -> bool {
        self.project_link_menu.is_some()
    }

    /// 数据库面板数据源树 header 行右键菜单是否打开(main.rs Esc 键路由用)。
    pub fn database_source_context_menu_open(&self) -> bool {
        self.database_source_menu.is_some()
    }

    /// 打开数据库面板数据源树 header 行的右键菜单(测试连接/编辑/删除/
    /// 刷新)。与文件树右键菜单互斥(坐标复用 `files.last_right_click`)。
    fn database_source_context_menu(&mut self, source_id: String) {
        self.files.close_context_menu();
        #[cfg(target_os = "macos")]
        {
            let (x, y) = self.files.last_right_click();
            let expanded = self
                .active_workspace()
                .is_some_and(|ws| ws.database.is_expanded(&source_id));
            let items = database_source_menu_items(&source_id, expanded);
            if let Some(msg) = crate::native_menu::show(items, (x, y)) {
                self.update(msg);
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let (x, y) = self.files.last_right_click();
            self.database_source_menu = Some(DatabaseSourceMenu { x, y, source_id });
        }
    }

    /// 打开 Project 面板「项目文档 / Agent 记忆」链接行的删除右键菜单。与
    /// 文件树右键菜单互斥(坐标复用 `files.last_right_click`)。
    fn project_link_context_menu(&mut self, target: project::links::LinkTarget, index: usize) {
        self.files.close_context_menu();
        // 右击即选中该行:从对应链接列表取下标项路径,标记到
        // `project_panel.selected_link`(参考文件树 `ContextMenuOpen` 同时选中)。
        if let Some(path) = self
            .active_workspace()
            .and_then(|ws| ws.project_panel.link_path_at(target, index))
        {
            self.with_focused_project(|ws, _io| {
                ws.project_panel.set_selected_link(path);
            });
        }
        #[cfg(target_os = "macos")]
        {
            let (x, y) = self.files.last_right_click();
            let items = project_link_menu_items(target, index);
            if let Some(msg) = crate::native_menu::show(items, (x, y)) {
                self.update(msg);
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let (x, y) = self.files.last_right_click();
            self.project_link_menu = Some(ProjectLinkMenu {
                x,
                y,
                target,
                index,
            });
        }
    }

    /// 打开分类树节点的右键菜单。坐标复用 `files.last_right_click()`
    /// (同 `project_link_context_menu` 的既有接线方式)。
    fn todo_category_context_menu(&mut self, id: Option<i64>) {
        self.files.close_context_menu();
        #[cfg(target_os = "macos")]
        {
            let (x, y) = self.files.last_right_click();
            let sibling_parent_id = id.and_then(|id| {
                self.active_workspace().and_then(|ws| {
                    ws.todo
                        .categories()
                        .iter()
                        .find(|c| c.id == id)
                        .and_then(|c| c.parent_id)
                })
            });
            let items = category_context_menu_items(id, sibling_parent_id);
            if let Some(msg) = crate::native_menu::show(items, (x, y)) {
                self.update(msg);
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let (x, y) = self.files.last_right_click();
            self.category_context_menu = Some(CategoryContextMenu { x, y, id });
        }
    }

    /// 分类选择器是否打开(main.rs Esc 键路由用)。
    pub fn category_picker_open(&self) -> bool {
        self.category_picker.is_some()
    }

    /// Todo 分类树节点右键菜单是否打开(main.rs Esc 键路由用)。
    pub fn category_context_menu_open(&self) -> bool {
        self.category_context_menu.is_some()
    }

    /// 打开分类选择器浮层,挂到给定目标(任务挂分类 / 分类 reparent)。
    fn todo_category_picker_open(&mut self, target: CategoryPickerTarget) {
        let (x, y) = self.last_cursor;
        self.category_picker = Some(CategoryPicker { x, y, target });
    }

    /// Agent 选择菜单是否打开(main.rs Esc 键路由用)。
    pub fn agent_picker_open(&self) -> bool {
        self.active_workspace()
            .map(|ws| ws.agent_picker_open)
            .unwrap_or(false)
    }

    /// 顶栏新增项目菜单是否打开(main.rs Esc 键路由用)。
    pub fn project_add_menu_open(&self) -> bool {
        self.project_add_menu_open
    }

    /// Todo 派发选择层是否打开(给 main.rs 的 Esc 关闭用)。
    pub fn todo_dispatch_open(&self) -> bool {
        self.active_workspace()
            .map(|ws| ws.todo.dispatch_popup_open())
            .unwrap_or(false)
    }

    /// Todo 日历日期选择器是否打开(给 main.rs 的 Esc 关闭用,同
    /// `todo_dispatch_open` 的既有模式)。
    pub fn todo_calendar_open(&self) -> bool {
        self.active_workspace()
            .map(|ws| ws.todo.calendar_popup_open())
            .unwrap_or(false)
    }

    /// Todo 状态下拉选择层是否打开(给 main.rs 的 Esc 关闭用,同
    /// `todo_dispatch_open` 的既有模式)。
    pub fn todo_status_open(&self) -> bool {
        self.active_workspace()
            .map(|ws| ws.todo.status_popup_open())
            .unwrap_or(false)
    }

    /// Todo **搜索框状态筛选**浮层是否打开(给 main.rs 的 Esc 关闭用,同
    /// `todo_dispatch_open` 的既有模式)。
    pub fn todo_status_filter_open(&self) -> bool {
        self.active_workspace()
            .map(|ws| ws.todo.status_filter_popup_open())
            .unwrap_or(false)
    }

    /// 当前激活预览 tab 是否走原生渲染。main.rs 键盘路由用:原生预览是就地可
    /// 写的非模态编辑器,还要求键盘焦点确实在预览列(`current_focus ==
    /// FocusIntent::Preview`),否则用户正在打字给终端时,只因为预览列背景里
    /// 开着一个原生 tab 就会把按键错误地拦下来。
    pub fn active_preview_tab_has_native_editor(&self, kind: PanelKind) -> bool {
        self.active_workspace()
            .map(|ws| ws.active_preview_tab_has_native_editor(kind))
            .unwrap_or(false)
    }

    /// `kind` 预览面板的 Find 条当前是否显示。main.rs Esc/⌘ 键盘路由据此决定在
    /// 原生预览闸门里先吃哪些键(Esc 关条/⌘G 步进只在有条时可行动)。
    pub fn preview_find_bar_open(&self, kind: PanelKind) -> bool {
        self.active_workspace()
            .map(|ws| ws.preview_find_bar_open(kind))
            .unwrap_or(false)
    }

    /// 每帧渲染循环读走 `preview::CaptureFindFocus` 查到的真实焦点态后写进
    /// 当前工作区。
    pub fn set_find_query_focused(&mut self, kind: PanelKind, focused: bool) {
        if let Some(ws) = self.active_workspace_mut() {
            ws.set_find_query_focused(kind, focused);
        }
    }

    /// 转发到聚焦项目里某个原生预览 tab 的 editor,按 `tab_id` 定位(带面板语
    /// 义的兄弟在 `preview_tab_editor_event` / `project_preview_tab_editor_event`,
    /// 语义同文)。官方 `text_editor` 的剪贴板读写由 iced 运行时经
    /// `Widget::update` 拿到的 `Clipboard` 直接处理。
    pub fn preview_tab_editor_event(
        &mut self,
        tab_id: usize,
        action: iced_widget::text_editor::Action,
    ) {
        let io = self.shell_io();
        if let Some(ws) = self.active_workspace_mut() {
            ws.preview_tab_editor_event(tab_id, action);
            ws.spawn_preview_context_push(&io);
        }
    }

    /// Project 面板右配对预览 tab 的 `text_editor::Action` 转发,语义同
    /// `preview_tab_editor_event`,作用于 `ws.project_preview`。
    pub fn project_preview_tab_editor_event(
        &mut self,
        tab_id: usize,
        action: iced_widget::text_editor::Action,
    ) {
        if let Some(ws) = self.active_workspace_mut() {
            ws.project_preview_tab_editor_event(tab_id, action);
        }
    }

    /// 取走"双击顶栏空白处"待处理标记(取走即清零)。main.rs 在派发完
    /// 消息后轮询这个方法,命中就调用 `window.set_maximized(!window.
    /// is_maximized())`——`App` 自己不持有 `Window` 句柄,做不到这一步。
    pub fn take_pending_zoom_toggle(&mut self) -> bool {
        std::mem::take(&mut self.pending_zoom_toggle)
    }

    /// 全局 UI 缩放改了之后,main.rs 轮询这个标记把新 scale 应用到所有
    /// 已存在的预览/浏览器 webview(新的 webview 由 `sync_webview_pool`
    /// 在创建时按当前 scale 初始化,不依赖本标记)。
    pub fn take_pending_preview_zoom(&mut self) -> bool {
        std::mem::take(&mut self.pending_preview_zoom)
    }

    /// 终端光标的窗口逻辑坐标 `(x, y_底, 行高)`,给 main.rs 在没有任何原生
    /// `text_input` 要 IME 时(`InputMethod::Disabled`,即终端聚焦——终端
    /// 不是 iced 控件,没有这套机制)设候选窗位置用。原生 `text_input`
    /// 聚焦要 IME 的情况(地址栏/意见框/搜索框/……)现在统一由 main.rs 直接
    /// 读 `UserInterface::update()` 返回的 `InputMethod::Enabled { cursor,
    /// .. }` 处理,不再靠这里手写特判(2026-08-21 修复:`input_method` 字段
    /// 此前被丢弃,原生控件的候选窗一律钉在这份终端光标算法给出的位置,
    /// 组字预览文字也压根没地方画出来)。单元格尺寸由 pane 像素 ÷ 网格
    /// 推出,不依赖字号常量。
    pub fn ime_cursor_area(&self, window_w: f32, window_h: f32) -> (f32, f32, f32) {
        let state = self.shell_state();
        let (pane_w, pane_h) = terminal_pane_pixel_size(window_w, window_h, &state);
        let cell_w = pane_w / self.cols.max(1) as f32;
        let line_h = pane_h / self.rows.max(1) as f32;
        let right_w = right_zone_width(window_w, &state);
        // `PanelKind::Agent` 面板未镜像时,实际渲染顺序是 `[terminal,
        // divider, list]`(内容在前,见 `app.rs` 的 `PanelKind::Agent`
        // 分支 `else` 臂)——跟 `pair_columns` 的内建默认("mirrored=false
        // → list 先")相反,必须传 `!mirrored`,否则算出来的内容列起点会
        // 多算上一整个 list 宽度(2026-08-22 实测调试日志确认:候选窗
        // x 坐标比实际光标多偏了正好一个 `list_w` 的量,同 `webview_
        // geometry.rs` 的 `Conversations`/`Web` 两处同款手法)。
        let mirrored =
            state.layout.rail_layout.side_of(PanelKind::Agent) != PanelKind::Agent.default_side();
        let cols = pair_columns(
            pair_content_width(right_w),
            state.dims.agent_split,
            !mirrored,
        );
        let m = theme::region::right_zone().margin;
        let x0 = window_w - byteui::theme::geometry::icon_rail_width() - right_w
            + cols.content_x
            + 8.0
            + m.left;
        // 终端网格上方 chrome:顶栏 44 + 上 padding 8 + tab 栏 30 + spacing 4(header 已去,P1L #4)
        let y0 = byteui::theme::geometry::top_bar_height() + 8.0 + 30.0 + 4.0;
        let (col, row) = self
            .active_workspace()
            .and_then(|ws| ws.tabs.get(ws.active))
            .map(|t| {
                // 光标不可信(全屏重绘型 TUI 未发 `?25h` 显示光标,实测
                // CodeBuddy CLI——见 `term_view.rs` 画方块光标那处同款
                // `cursor_visible()` 判断的文档)或正在回看历史
                // (`display_offset() > 0`)时,`model.cursor()` 只是一堆
                // 重绘期间移动/清行序列扫过后留下的陈旧坐标,跟视觉上
                // 光标实际所在毫无关系——不同 CLI 的终端更新习惯不同,
                // 表现就是"候选窗偏移量因 agent 而异"(2026-08-21 用户
                // 实测反馈)。退回"面板底部一行、列 0"这个粗略但不离谱的
                // 默认位置,好过让候选窗跳到跟视觉毫不相关的地方。
                if t.model.cursor_visible() && t.model.display_offset() == 0 {
                    t.model.cursor()
                } else {
                    (0usize, self.rows.max(1) as usize - 1)
                }
            })
            .unwrap_or((0, 0));
        // 组字预览期间 PTY 收不到字节,`model.cursor()` 原地不动——候选窗
        // 要跟着预览文字的末尾走(与 `term_view` 画预览的落点算法一致),
        // 否则用户敲得越多,候选窗越是钉在组字开始前的旧光标位置不跟手。
        let preedit_cols: usize = self
            .term_ime_preedit
            .as_deref()
            .map(|s| {
                s.chars()
                    .map(|c| {
                        unicode_width::UnicodeWidthChar::width(c)
                            .unwrap_or(1)
                            .max(1)
                    })
                    .sum()
            })
            .unwrap_or(0);
        let x = x0 + (col + preedit_cols) as f32 * cell_w;
        let y = y0 + (row as f32 + 1.0) * line_h; // 光标格底部,候选窗落其下方
        (x, y, line_h)
    }

    /// `kind`(`Files`/`Project`)预览面板当前**实际渲染宽度**(逻辑像素),
    /// 供 tab 栏分页(`tab_widget::tab_window`)当可用宽度预算——复用 webview
    /// 定位同源的 `preview_content_bounds_for`,不再用
    /// `byteui::theme::geometry::tab_bar_avail_px()` 那个跟真实面板宽度
    /// 完全无关的静态估算常量(2026-09-14 用户反馈:明明面板里还有大把
    /// 空白,tab 栏却只显示第一个、其余全甩进溢出下拉——`tab_bar_avail_px`
    /// 原本按"顶栏项目页签"这类窄场景标定,被预览/终端/数据库/SSH 好几处
    /// tab 栏共用后,对通常宽得多的面板严重低估)。`kind` 当前不在左右任一
    /// 栏(极短暂的过渡态)时退回旧的静态估算,不 panic。
    ///
    /// 返回值要再扣掉 tab 组右侧那几颗按钮(V 溢出 / 预览·代码切换 / 收起
    /// 列表)——渲染侧 `tab_row` 容器只吃 `content_w` 剩下的那一份
    /// `Length::Fill`(`preview_pane_for` 里 `row![overflow_button?,
    /// clipped_tab_row, render_mode_button?, collapse]`),真正留给 tab
    /// 组的宽度天然比整块内容区窄。不扣的话 `tab_window` 会算出比真实能
    /// 放下的还多一点,多出来的量被 `clipped` 容器的 `.clip(true)` 硬裁
    /// (2026-09-14 用户反馈:新开的文件 tab 标题被裁掉一截"bum"——就是
    /// 这个偏差)。三颗按钮里"预览·代码切换"是可选的(只对 wry 可切换
    /// 的文件类型出现),这里按恒出现的最坏情况扣,宁可窄一点提前进溢出
    /// 下拉,也不能宽出来被裁字。
    pub(crate) fn preview_tab_bar_avail_px(&self, kind: PanelKind) -> f32 {
        /// 三颗按钮(V 溢出/预览切换/收起列表)的命中区宽度粗估,均出自
        /// `byteui::theme::icon_size::row()` 驱动的 `icons::icon_button_entry`,
        /// 跟 `rail_button_size`(32px)同量级;`tab_bar_row` 的
        /// `.spacing(4)` 在按钮之间、以及按钮与 tab 组之间各占一份。
        const CHROME_BUTTON_PX: f32 = 32.0;
        const CHROME_RESERVE_PX: f32 = CHROME_BUTTON_PX * 3.0 + 4.0 * 3.0;
        let side = if self.left_view == kind {
            Side::Left
        } else if self.right_view == kind {
            Side::Right
        } else {
            return byteui::theme::geometry::tab_bar_avail_px();
        };
        let (_, _, w, _) = webview_geometry::preview_content_bounds_for(
            side,
            self.window_size.0,
            self.window_size.1,
            &self.shell_state(),
        );
        (w - CHROME_RESERVE_PX).max(0.0)
    }

    /// 终端 tab 栏当前**实际渲染宽度**(逻辑像素),同
    /// `preview_tab_bar_avail_px` 的理由——`terminal.rs::tab_bar` 与
    /// `App::select_tab_no_drag`/`Workspace::on_tab_attached` 此前都用
    /// `byteui::theme::geometry::tab_bar_avail_px()` 那个跟真实面板宽度
    /// 无关的静态估算常量,同一类反馈(2026-09-16:文件预览已用
    /// `preview_tab_bar_avail_px` 修过,终端 tab 栏这条漏了,新建会话一旦
    /// 超过静态估算能塞下的个数就总有一个排不进可见窗口)。复用给
    /// PTY 网格换算用的 `terminal_pane_pixel_size`——那正是这块 pane
    /// 去掉左右 `theme::region::terminal_pane().padding` 之后的内容宽,
    /// `tab_bar` 是这块内容里的第一个元素,天然同宽。终端目前只在
    /// `PanelKind::Agent` 挂在右栏时才有实际渲染尺寸可言(见
    /// `terminal_pane_pixel_size` 的同一假设),不在右栏/右栏收起时退回
    /// 静态估算。
    pub(crate) fn terminal_tab_bar_avail_px(&self) -> f32 {
        /// 两颗按钮(V 溢出/收起列表)的命中区宽度粗估,同
        /// `preview_tab_bar_avail_px` 里 `CHROME_BUTTON_PX` 的取法;`tab_row`
        /// 的 `.spacing(4)` 在这两颗按钮与 tab 组之间各占一份(见
        /// `terminal.rs::tab_bar`:`row![overflow_button?, clipped, list_collapse_button]`,
        /// 比预览版少一颗"预览·代码切换",按钮数/间隙数各减一)。
        const CHROME_BUTTON_PX: f32 = 32.0;
        const CHROME_RESERVE_PX: f32 = CHROME_BUTTON_PX * 2.0 + 4.0 * 2.0;
        let state = self.shell_state();
        if state.right_collapsed || state.right_view != PanelKind::Agent {
            return byteui::theme::geometry::tab_bar_avail_px();
        }
        let (pane_w, _pane_h) =
            terminal_pane_pixel_size(self.window_size.0, self.window_size.1, &state);
        (pane_w - CHROME_RESERVE_PX).max(0.0)
    }

    /// 当前应存在的"文件/项目预览"webview 清单(main.rs 差集同步用),
    /// 每条自带按其所在侧算好的矩形。左右两侧各自独立判断——`Files` 在
    /// 左栏、`Project` 在右栏可以同时非空(见 spec"webview 面板的镜像
    /// bounds(2026-08-19 Stage 4a 审阅后修订)"一节)。不在文件视图时
    /// 该侧整体不产出;进首页时两侧都不产出(原因见旧版注释:首页时
    /// 预览区根本不在屏上)。
    pub fn preview_desired(
        &self,
        window_width: f32,
        window_height: f32,
    ) -> Vec<(WebviewSpec, (f32, f32, f32, f32))> {
        if self.current_page == AppPage::Home {
            return Vec::new();
        }
        let Some(ws) = self.active_workspace() else {
            return Vec::new();
        };
        // `search_modal` 是满窗 SCRIM+卡片形制(见
        // `extensions/search.rs::search_modal` 注释),打开时要隐藏 webview,
        // 否则 webview 会盖住遮罩和弹窗卡片。`text_input_menu`(输入框右键
        // 剪切/复制/粘贴菜单)是屏幕空间单例、不区分左右哪一侧,和
        // `search_modal` 一样按"两侧都可能被盖住"从宽处理——比如文件树
        // 搜索框右键时,菜单向下弹出恰好压在下方的预览 webview 上。
        let app_modal_open = ws.search.is_open() || self.text_input_menu.is_some();
        let mut out = Vec::new();
        for side in [Side::Left, Side::Right] {
            let kind = match side {
                Side::Left => self.left_view,
                Side::Right => self.right_view,
            };
            let (specs, id_offset): (Vec<WebviewSpec>, usize) = match kind {
                PanelKind::Files => (ws.preview.desired_webviews(), 0),
                PanelKind::Project => (
                    ws.project_preview.desired_webviews(),
                    PROJECT_PREVIEW_ID_OFFSET,
                ),
                PanelKind::Conversations => (
                    crate::workspace::review_webview_spec(ws.review.as_ref()),
                    CONVERSATION_REVIEW_ID_OFFSET,
                ),
                _ => continue,
            };
            // tab 栏"溢出下拉"(V 按钮)向下弹,原生浮层会被本侧 webview
            // 盖住(webview 恒在 iced 内容之上)——按该侧对应的
            // `*_tab_overflow_anchor` 是否展开,同 `app_modal_open` 一并
            // 强制隐藏(见 `tab_widget::tab_overflow_menu` 文档)。
            let tab_overflow_open = match kind {
                PanelKind::Files => ws.preview_tab_overflow_anchor.is_some(),
                PanelKind::Project => ws.project_preview_tab_overflow_anchor.is_some(),
                _ => false,
            };
            // Files 右键菜单/Project 链接右键菜单/Conversations agent
            // 筛选下拉——同款"面板内浮层盖住 webview"场景,按当前面板种类
            // 分别判断(见 `webview_hidden_by_panel_popup` 文档)。
            let panel_popup_open = webview_hidden_by_panel_popup(
                kind,
                self.files.context_menu_is_some(),
                self.project_link_menu.is_some(),
                ws.conversations.agent_picker_open(),
            );
            let bounds = webview_geometry::preview_content_bounds_for(
                side,
                window_width,
                window_height,
                &self.shell_state(),
            );
            out.extend(specs.into_iter().map(|mut s| {
                s.id += id_offset;
                // 搜索弹窗/tab 溢出下拉/面板内浮层开着时,原生浮层盖住了
                // 预览区,原生 wry 子视图不听 iced 绘制顺序摆布,必须显式
                // visible=false 才能真正藏起来。
                if app_modal_open || tab_overflow_open || panel_popup_open {
                    s.visible = false;
                }
                (s, bounds)
            }));
        }
        out
    }

    /// 浏览器域的 webview 清单,语义同 `preview_desired`,查独立的
    /// `Workspace::browser`。首页时矩形留空(main.rs 用 `home_browser_bounds`
    /// 单独覆盖,见调用处),工作区内按 `Web` 当前所在侧现算矩形。
    pub fn browser_desired(
        &self,
        window_width: f32,
        window_height: f32,
    ) -> Vec<(WebviewSpec, (f32, f32, f32, f32))> {
        // 地址栏右键"剪切/复制/粘贴"菜单向下弹出,恰好压在下方的浏览器
        // webview 内容区上——同 `preview_desired` 里 `text_input_menu` 的
        // 处理,原生 wry 子视图不听 iced 绘制顺序摆布,必须显式
        // visible=false 才能真正藏起来。首页(`home_browser`)和工作区内
        // (`ws.browser`)两条分支共用这一个判断。
        let text_input_menu_open = self.text_input_menu.is_some();
        // 首页右栏恒为全局浏览器(`home_browser`),与 `left_view` 无关——
        // 进首页就让它成为浏览器 webview 池的唯一来源,否则默认 URL 的 tab
        // 建了却永远等不到 webview(见 `sync_webview_pool`)。
        if self.current_page == AppPage::Home {
            return self
                .home_browser
                .desired_webviews()
                .into_iter()
                .map(|mut s| {
                    if text_input_menu_open {
                        s.visible = false;
                    }
                    (s, (0.0, 0.0, 0.0, 0.0))
                })
                .collect();
        }
        let side = if self.left_view == PanelKind::Web {
            Side::Left
        } else if self.right_view == PanelKind::Web {
            Side::Right
        } else {
            return Vec::new();
        };
        let Some(ws) = self.active_workspace() else {
            return Vec::new();
        };
        let bounds = webview_geometry::preview_content_bounds_for(
            side,
            window_width,
            window_height,
            &self.shell_state(),
        );
        ws.browser
            .desired_webviews()
            .into_iter()
            .map(|mut s| {
                if text_input_menu_open {
                    s.visible = false;
                }
                (s, bounds)
            })
            .collect()
    }

    /// 保证 `git_log` 状态跟得上"现在应该看哪个项目"——`git_log: State`
    /// 是 `App` 级字段,不是每个项目各自一份(不像 `Workspace.files`),
    /// 所以面板打开时(`PanelSelect`)和切项目页签时(`ProjectTabSwitch`)
    /// 都得调这个方法对齐一次,否则 Git Log 面板开着的状态下切页签,提交图
    /// 会停在上一个项目不动,而同一面板里的 worktree 速览条(`ws.files
    /// .worktrees()` 是按项目取的)却已经跳到新项目——两者对不上。缓存已经是当前项目的
    /// 路径就不动(避免每次切页签都重算一遍),路径不一致就重建,没有项目
    /// 就清空。只在 `left_view == PanelKind::GitLog` 时调用才有意义。
    fn sync_git_log_to_active_project(&mut self) {
        let path = self
            .active_workspace()
            .and_then(|ws| ws.active_project_path());
        match path {
            Some(p) if self.git_log.cache_repo_path() != Some(p.as_path()) => {
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let emit = move |m| {
                    let _ = proxy.send_event(Message::GitLog(m));
                };
                git_log::request_refresh(
                    &mut self.git_log,
                    p,
                    git_log::DEFAULT_MAX_COMMITS,
                    &handle,
                    emit,
                );
            }
            None => {
                self.git_log = git_log::State::default();
            }
            _ => {}
        }
    }

    pub fn update(&mut self, message: Message) {
        match message {
            Message::TermInput(target, bytes) => self.term_input(target, bytes),
            Message::TermImePreedit(target, text) => {
                if target == self.keyboard_term_target() {
                    self.term_ime_preedit = text;
                }
            }
            Message::TermOutput(project_id, tab_id, bytes) => {
                self.term_output(project_id, tab_id, bytes)
            }
            Message::SessionExited(project_id, tab_id) => {
                self.with_project(project_id, |ws, _io| {
                    if let Some(tab) = ws.tab_by_id_mut(tab_id) {
                        tab.alive = false;
                        // 本地标记行，非会话真实输出；应答无处可写，丢弃。
                        let _ = tab.model.feed(&exited_marker());
                    }
                });
            }
            Message::AgentStateChanged(project_id, tab_id, agent, state, transcript_path) => {
                self.agent_state_changed(project_id, tab_id, agent, state, transcript_path)
            }
            Message::AgentCardRefreshed(
                project_id,
                tab_id,
                llm_model,
                mode,
                activity,
                workspace,
            ) => {
                self.with_project(project_id, |ws, _io| {
                    crate::workspace::apply_agent_card_refresh(
                        &mut ws.tabs,
                        tab_id,
                        llm_model,
                        mode,
                        activity,
                        workspace,
                    );
                });
            }
            Message::ReviewLoaded(project_id, source, append, result) => {
                self.with_project(project_id, |ws, _io| {
                    if result.is_ok() {
                        ws.review_nonce = ws.review_nonce.wrapping_add(1);
                    }
                    let nonce = ws.review_nonce;
                    if let Some(rv) = &mut ws.review
                        && rv.source == source
                    {
                        match result {
                            Ok(entries) => {
                                // 快照写入必须放在 `rv.source == source` 判断
                                // 通过之后——这是它跟旧实现(main.rs 里的裸
                                // `static`,过期/乱序结果也会无条件覆盖)的
                                // 关键区别,过期加载结果到这里已经被
                                // 上面的守卫挡在外面,不会再污染快照。
                                merge_review_entries(&mut rv.entries, entries, append);
                                let snapshot = ReviewSnapshot {
                                    entries: &rv.entries,
                                    agent_label: rv.agent.label(),
                                    summary_title: rv.summary_title.clone(),
                                    summary_text: rv.summary_text.clone(),
                                    summary_time: rv.summary_time.clone(),
                                };
                                let json = serde_json::to_string(&snapshot).unwrap_or_default();
                                *ws.review_snapshot.lock().expect("review snapshot 锁") =
                                    Some(json);
                                rv.nonce = nonce;
                                rv.error = None;
                            }
                            Err(e) => rv.error = Some(e),
                        }
                    }
                });
            }
            Message::Conversations(conversations::Message::SessionsRefreshed(
                project_id,
                result,
            )) => {
                self.with_project(project_id, move |ws, _io| {
                    conversations::update(
                        &mut ws.conversations,
                        conversations::Message::SessionsRefreshed(project_id, result),
                    );
                });
            }
            Message::Conversations(conversations::Message::SessionOpen(conversation_id, agent)) => {
                self.conversation_session_open(conversation_id, agent);
            }
            Message::Conversations(conversations::Message::DetailLoadMore(
                conversation_id,
                after_turn_index,
            )) => {
                self.with_focused_project(|ws, io| {
                    ws.spawn_review_load_conversation(
                        io,
                        conversation_id,
                        after_turn_index,
                        CONVERSATION_DETAIL_PAGE_SIZE,
                        true,
                    );
                });
            }
            Message::Conversations(conversations::Message::Hover(id, h)) => self.set_hover(id, h),
            Message::Conversations(conversations::Message::TextInputMenuOpen(target)) => {
                self.update(Message::TextInputMenuOpen(target));
            }
            Message::Conversations(conversations::Message::AgentPickerOpen) => {
                #[cfg(target_os = "macos")]
                {
                    let last_cursor = self.last_cursor;
                    let items = self
                        .active_workspace()
                        .map(|ws| conversations::agent_picker_items(&ws.conversations))
                        .unwrap_or_default();
                    if let Some(msg) = crate::native_menu::show(items, last_cursor) {
                        self.update(Message::Conversations(msg));
                    }
                }
                #[cfg(not(target_os = "macos"))]
                {
                    self.with_focused_project(|ws, _io| {
                        conversations::update(
                            &mut ws.conversations,
                            conversations::Message::AgentPickerOpen,
                        );
                    });
                }
            }
            Message::Conversations(msg) => {
                self.with_focused_project(|ws, _io| {
                    conversations::update(&mut ws.conversations, msg);
                });
            }
            Message::ProjectDeleteDone(errors) => {
                if !errors.is_empty() {
                    self.daemon_error = Some(format!("删除项目未完全成功: {}", errors.join("; ")));
                }
            }
            Message::Usage(msg @ usage::Message::Loaded(project_id, ..)) => {
                self.with_project(project_id, move |ws, _io| {
                    usage::update(&mut ws.usage, msg);
                });
            }
            Message::Usage(usage::Message::ToggleListCollapse) => {
                self.toggle_panel_list_collapse(PanelKind::Usage);
            }
            Message::Usage(usage::Message::Hover(id, h)) => self.set_hover(id, h),
            Message::Usage(msg) => {
                self.with_focused_project(|ws, _io| {
                    usage::update(&mut ws.usage, msg);
                });
            }
            Message::SelectTab(idx) => self.select_tab(idx),
            Message::SelectTabNoDrag(idx) => self.select_tab_no_drag(idx),
            Message::CloseTab(idx) => {
                self.with_focused_project(|ws, io| {
                    ws.close_tab(io, idx);
                    ws.ensure_project_terminal(io);
                });
            }
            Message::AgentPickerToggle => {
                #[cfg(target_os = "macos")]
                {
                    let (x, y) = self.last_cursor;
                    let items = crate::workspace::agent_picker_items();
                    if let Some(msg) = crate::native_menu::show(items, (x, y)) {
                        self.update(msg);
                    }
                }
                #[cfg(not(target_os = "macos"))]
                {
                    self.with_focused_project(|ws, _io| {
                        ws.agent_picker_open = !ws.agent_picker_open;
                    });
                }
            }
            Message::AgentPickerClose => {
                self.with_focused_project(|ws, _io| {
                    ws.agent_picker_open = false;
                });
            }
            Message::AgentPickerSelect(agent) => {
                self.with_focused_project(|ws, io| {
                    ws.agent_picker_open = false;
                    ws.spawn_new_tab(io, agent, None);
                });
            }
            Message::ProjectAddMenuToggle => {
                #[cfg(target_os = "macos")]
                {
                    let last_cursor = self.last_cursor;
                    let open_ids: std::collections::HashSet<i64> =
                        self.projects.keys().copied().collect();
                    let items =
                        crate::topbar::project_add_menu_items(&self.recent_projects, &open_ids);
                    if let Some(msg) = crate::native_menu::show(items, last_cursor) {
                        self.update(msg);
                    }
                }
                #[cfg(not(target_os = "macos"))]
                {
                    self.project_add_menu_open = !self.project_add_menu_open;
                    if self.project_add_menu_open {
                        // 点"＋"时的光标逻辑坐标,作为菜单弹出锚点——同
                        // `todo::set_calendar_anchor`/`set_dispatch_anchor` 手法。
                        self.project_add_menu_anchor = self.last_cursor;
                    }
                }
            }
            Message::ProjectAddMenuClose => {
                self.project_add_menu_open = false;
            }
            Message::Todo(todo::Message::AssignAgent(idx, agent)) => {
                self.todo_assign_agent(idx, agent)
            }
            Message::Todo(todo::Message::DetailOpen(idx)) => self.todo_detail_open(idx),
            Message::Todo(todo::Message::DetailReplySubmit) => self.todo_detail_process(),
            // 数据库连接测试的异步结果带显式 `project_id`——用户可能在等待
            // 期间切走了项目页签,必须按自带 id 路由,不能用当前聚焦项目
            // (同 `TabAttached`/`ProjectSlotLoaded` 那批异步消息的约定,见设计
            // 文档"结果经 `emit` 回传"一节)。特化分支必须排在通配
            // `Message::Database(msg)` **之前**,否则永远匹配不到。
            Message::Database(database::Message::TestConnectionResult(
                project_id,
                source_id,
                result,
            )) => self.database_test_connection_result(project_id, source_id, result),
            // schema 树两个异步结果同 `TestConnectionResult` 口径:自带 project_id,
            // 按自带 id 路由,不能用当前聚焦项目。特化分支必须排在通配
            // `Message::Database(msg)` 之前,否则永远匹配不到。
            Message::Database(database::Message::TablesLoaded(project_id, source_id, result)) => {
                self.database_tables_loaded(project_id, source_id, result)
            }
            Message::Database(database::Message::ColumnsLoaded {
                project_id,
                source_id,
                schema,
                table,
                result,
            }) => self.database_columns_loaded(project_id, source_id, schema, table, result),
            Message::Database(database::Message::BrowseResult(
                project_id,
                tab_id,
                run_seq,
                result,
            )) => self.database_browse_result(project_id, tab_id, run_seq, result),
            Message::Database(database::Message::QueryResult(
                project_id,
                tab_id,
                run_seq,
                result,
            )) => self.database_query_result(project_id, tab_id, run_seq, result),
            Message::Database(database::Message::SourceContextMenu(source_id)) => {
                self.database_source_context_menu(source_id);
            }
            Message::Database(database::Message::TabHover(target, idx, hovered)) => {
                // 内容窗格 tab 本体/关闭按钮的悬停,转发成 `HoverId`(同
                // `ToolbarHover` 的口径)。
                let id = match target {
                    database::DatabaseTabHoverTarget::Title => HoverId::DatabaseTabItem(idx),
                    database::DatabaseTabHoverTarget::Close => HoverId::DatabaseTabClose(idx),
                };
                self.set_hover(id, hovered);
            }
            Message::Database(database::Message::TextInputMenuOpen(target)) => {
                self.update(Message::TextInputMenuOpen(target));
            }
            Message::Database(database::Message::ToggleListCollapse) => {
                self.toggle_panel_list_collapse(PanelKind::Database);
            }
            Message::Database(database::Message::TabOverflowToggle) => {
                #[cfg(target_os = "macos")]
                {
                    let last_cursor = self.last_cursor;
                    let items = self
                        .active_workspace()
                        .map(|ws| database::tab_overflow_items(&ws.database))
                        .unwrap_or_default();
                    if let Some(msg) = crate::native_menu::show(items, last_cursor) {
                        self.update(Message::Database(msg));
                    }
                }
                #[cfg(not(target_os = "macos"))]
                {
                    let last_cursor = self.last_cursor;
                    self.with_focused_project(|ws, _io| {
                        ws.database.content_mut().toggle_tab_overflow(last_cursor);
                    });
                }
            }
            Message::Database(database::Message::TabOverflowDismiss) => {
                self.with_focused_project(|ws, _io| {
                    ws.database.content_mut().dismiss_tab_overflow();
                });
            }
            Message::Database(database::Message::Hover(id, h)) => self.set_hover(id, h),
            Message::Database(msg) => self.database_message(msg),
            Message::Todo(msg) => match msg {
                todo::Message::Hover(id, h) => self.set_hover(id, h),
                todo::Message::TextInputMenuOpen(target) => {
                    self.update(Message::TextInputMenuOpen(target));
                }
                todo::Message::ToggleListCollapse => {
                    self.toggle_panel_list_collapse(PanelKind::Todo);
                }
                todo::Message::CategoryContextMenuOpen(id) => {
                    self.todo_category_context_menu(id);
                }
                // 以下几种分类动作都是从右键菜单里点出来的:先关掉菜单本
                // 身(浮层 if-else 链里 `category_context_menu` 分支排在
                // `category_picker` 之前,不关会导致"移动到..."开了选择器
                // 却永远被菜单盖住),镜像 `files.rs::RenameStart` 落盘动作
                // 时 `app_state.context_menu = None;` 的既有口径。
                todo::Message::CategoryReparentPickerOpen(id) => {
                    self.category_context_menu = None;
                    self.todo_category_picker_open(CategoryPickerTarget::Category(id));
                }
                todo::Message::CategoryNewChild(_)
                | todo::Message::CategoryNewSibling(_)
                | todo::Message::CategoryDelete(_)
                | todo::Message::CategoryRenameStart(_)
                | todo::Message::CategoryMoveSibling(_, _) => {
                    self.category_context_menu = None;
                    self.todo_message(msg);
                }
                todo::Message::CategoryPickerOpenForTodo(todo_id) => {
                    self.todo_category_picker_open(CategoryPickerTarget::Todo(todo_id));
                }
                todo::Message::DispatchOpen(idx) => {
                    #[cfg(target_os = "macos")]
                    {
                        let last_cursor = self.last_cursor;
                        let items = todo::dispatch_items(idx);
                        if let Some(msg) = crate::native_menu::show(items, last_cursor) {
                            self.update(Message::Todo(msg));
                        }
                    }
                    #[cfg(not(target_os = "macos"))]
                    {
                        self.todo_message(todo::Message::DispatchOpen(idx));
                    }
                }
                todo::Message::StatusOpen(idx) => {
                    #[cfg(target_os = "macos")]
                    {
                        let last_cursor = self.last_cursor;
                        let items = todo::status_items(idx);
                        if let Some(msg) = crate::native_menu::show(items, last_cursor) {
                            self.update(Message::Todo(msg));
                        }
                    }
                    #[cfg(not(target_os = "macos"))]
                    {
                        self.todo_message(todo::Message::StatusOpen(idx));
                    }
                }
                other => self.todo_message(other),
            },
            Message::TodoDetailLoaded(idx, turns) => {
                self.with_focused_project(move |ws, _io| {
                    if ws.todo.detail_open_idx() == Some(idx) {
                        ws.todo.replace_detail_turns(turns);
                    }
                });
            }
            // 文件树右键"搜索"弹窗:`SearchResults` 带 `project_id`,异步结果
            // 按所属项目路由(用户可能已切走);其余交互投当前聚焦项目。
            Message::Search(search::Message::SearchResults(project_id, result)) => {
                self.search_results(project_id, result)
            }
            Message::Search(search::Message::TextInputMenuOpen(target)) => {
                self.update(Message::TextInputMenuOpen(target));
            }
            Message::Search(msg) => {
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let Some(project_id) = self.active_project_id else {
                    return;
                };
                self.with_focused_project(move |ws, _io| {
                    let emit = move |m| {
                        let _ = proxy.send_event(Message::Search(m));
                    };
                    search::update(&mut ws.search, msg, project_id, &handle, emit);
                });
            }
            Message::TabAttached(project_id, tab_id, info, snapshot, picked_agent) => {
                let term_tab_bar_avail_px = self.terminal_tab_bar_avail_px();
                self.with_project(project_id, move |ws, io| {
                    ws.on_tab_attached(TabAttachedArgs {
                        cols: io.cols,
                        rows: io.rows,
                        tab_id,
                        info,
                        snapshot,
                        picked_agent,
                        term_tab_bar_avail_px,
                    })
                });
            }
            Message::PaneResized {
                cols,
                rows,
                ssh_cols,
                ssh_rows,
            } => self.pane_resized(cols, rows, ssh_cols, ssh_rows),
            Message::ColumnDragStart(divider) => {
                self.dragging = Some(divider);
            }
            Message::ColumnDrag {
                window_width,
                logical_x,
            } => {
                if let Some(divider) = self.dragging {
                    let state = self.shell_state();
                    self.dims = apply_column_drag(state, divider, window_width, logical_x);
                }
            }
            Message::ColumnDragEnd => {
                self.dragging = None;
                self.on_shell_layout_changed();
            }
            Message::RowDragStart(divider) => {
                self.dragging_row = Some(divider);
            }
            Message::RowDrag {
                window_height,
                logical_y,
            } => {
                if let Some(divider) = self.dragging_row {
                    match divider {
                        RowDivider::GitLogFileDiffSplit => {
                            let state = self.shell_state();
                            self.dims = apply_row_drag(state, divider, window_height, logical_y);
                        }
                        // 新增任务框高度:基线 = 框底 = 左面板区底 =
                        // `window_height - footbar_height`(顶栏在 `base`
                        // 之上,不参与);高度 = 基线 - 光标 y,向上拉变高。
                        // 上限再夹一道,避免列表区被压没(留约 140px)。
                        RowDivider::TodoAddGrow => {
                            let baseline =
                                window_height - byteui::theme::geometry::footbar_height();
                            let max_h =
                                (baseline - byteui::theme::geometry::top_bar_height() - 140.0)
                                    .max(todo::ADD_INPUT_MIN_HEIGHT);
                            let h = (baseline - logical_y).clamp(todo::ADD_INPUT_MIN_HEIGHT, max_h);
                            if let Some(ws) = self.active_workspace_mut() {
                                ws.todo.set_add_input_height(h);
                            }
                        }
                    }
                }
            }
            Message::RowDragEnd => {
                self.dragging_row = None;
                self.on_shell_layout_changed();
            }
            Message::TabDragMove { group, index } => {
                self.tab_drag_move(group, index);
            }
            Message::TabDragEnd => {
                self.end_tab_drag();
            }
            Message::RailDragMove { side, index } => {
                self.rail_drag_move(side, index);
            }
            Message::RailDragEnd => {
                self.end_rail_drag();
            }
            Message::TodoDragEnd => {
                self.todo_message(todo::Message::DragEnd);
            }
            Message::PanelSelect(v) => self.panel_select(v),
            Message::ToggleFileTreeCollapse => self.toggle_files_tree_collapse(),
            Message::TogglePanelListCollapse(kind) => self.toggle_panel_list_collapse(kind),
            Message::TextInputMenuOpen(target) => {
                #[cfg(target_os = "macos")]
                {
                    let (x, y) = self.files.last_right_click();
                    let items = text_input_menu_items(&target);
                    if let Some(msg) = crate::native_menu::show(items, (x, y))
                        && let Some(ch) = crate::menu_edit_key(&msg)
                    {
                        self.pending_native_menu_edit_key = Some((ch, target.id.clone()));
                    }
                }
                #[cfg(not(target_os = "macos"))]
                {
                    // 与其它右键菜单互斥——关掉别的,只留本菜单(同时避免互相顶)。
                    self.files.close_context_menu();
                    self.project_link_menu = None;
                    let (x, y) = self.files.last_right_click();
                    self.text_input_menu = Some(TextInputMenu {
                        x,
                        y,
                        target: target.clone(),
                    });
                    // 右键不聚焦 iced 输入框(只有左键会),菜单的复制/粘贴需要通过
                    // `interface.operate` 把焦点移到目标输入,否则合成回的 ⌘+c/v
                    // 事件作用不到它。记录待聚焦 id,本帧后由 `apply_pending_focus`
                    // 应用(main.rs)。
                    self.pending_text_input_focus = Some(target.id);
                }
            }
            Message::TextInputMenuClose => {
                self.text_input_menu = None;
            }
            Message::TextInputMenuCut => {
                self.text_input_menu = None;
            }
            Message::TextInputMenuCopy => {
                self.text_input_menu = None;
            }
            Message::TextInputMenuPaste => {
                self.text_input_menu = None;
            }
            Message::TextInputMenuSelectAll => {
                self.text_input_menu = None;
            }
            Message::Hover(id, h) => {
                self.set_hover(id, h);
            }
            Message::MaximizeClose => {
                self.maximized = None;
                self.sync_terminal_grid();
            }
            Message::TopBarDoubleClick => {
                self.pending_zoom_toggle = true;
            }
            Message::TopBarHome => self.top_bar_home(),
            Message::HomeRecentsLoaded(files, convs) => {
                self.home_recent_files = files;
                self.home_recent_conversations = convs;
                self.home_recents_loaded = true;
            }
            Message::HomeLeftIconSelect(v) => {
                self.home_left_view = v;
            }
            Message::HomeMoreProjects => self.home_project_pages += 1,
            Message::HomeProjectSearchInput(s) => self.home_project_search_draft = s,
            Message::HomeProjectSearchSubmit => self.commit_home_project_search(),
            Message::HomeRightIconSelect(v) => {
                self.home_right_view = v;
            }
            Message::HomeBrowser(msg) => {
                let client = self.client.clone();
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let emit = move |m| {
                    let _ = proxy.send_event(Message::HomeBrowser(m));
                };
                browser::update(&mut self.home_browser, msg, None, &client, &handle, emit);
            }
            Message::BrowserTitle(id, title) => {
                let msg = browser::Message::TitleLoaded(id, title);
                if self.is_home() {
                    // 首页全局浏览器:webview 属于 `home_browser`,直接落地。
                    browser::update(
                        &mut self.home_browser,
                        msg,
                        None,
                        &self.client,
                        &self.handle,
                        |_| {},
                    );
                } else {
                    // 工作区浏览器:webview id 落在当前聚焦工作区的 `ws.browser`。
                    self.with_focused_project(|ws, io| {
                        browser::update(
                            &mut ws.browser,
                            msg,
                            ws.project.as_ref().map(|p| p.id),
                            &io.client,
                            &io.handle,
                            |_| {},
                        );
                    });
                }
            }
            Message::BrowserNavigated(id, url) => {
                let msg = browser::Message::Loaded(id, url);
                if self.is_home() {
                    browser::update(
                        &mut self.home_browser,
                        msg,
                        None,
                        &self.client,
                        &self.handle,
                        |_| {},
                    );
                } else {
                    self.with_focused_project(|ws, io| {
                        browser::update(
                            &mut ws.browser,
                            msg,
                            ws.project.as_ref().map(|p| p.id),
                            &io.client,
                            &io.handle,
                            |_| {},
                        );
                    });
                }
            }
            Message::BrowserNewWindow(url) => {
                let msg = browser::Message::OpenUrl(url);
                if self.is_home() {
                    browser::update(
                        &mut self.home_browser,
                        msg,
                        None,
                        &self.client,
                        &self.handle,
                        |_| {},
                    );
                } else {
                    self.with_focused_project(|ws, io| {
                        browser::update(
                            &mut ws.browser,
                            msg,
                            ws.project.as_ref().map(|p| p.id),
                            &io.client,
                            &io.handle,
                            |_| {},
                        );
                    });
                }
            }
            Message::Noop => {}
            Message::DaemonError(message) => self.daemon_error = Some(message),
            Message::TermScroll(target, delta) => {
                self.with_focused_project(|ws, _io| {
                    let tab = match target {
                        terminal::TermTarget::Shared => ws.tabs.get_mut(ws.active),
                        terminal::TermTarget::SshPanel => ws.ssh_active_tab_mut(),
                    };
                    if let Some(tab) = tab {
                        tab.model.scroll_display(delta);
                    }
                });
            }
            Message::TermTabOverflowToggle => {
                #[cfg(target_os = "macos")]
                {
                    let last_cursor = self.last_cursor;
                    let items = match self.active_workspace() {
                        Some(ws) => {
                            let entries: Vec<(usize, String, bool)> = ws
                                .tabs
                                .iter()
                                .enumerate()
                                .map(|(idx, tab)| {
                                    (
                                        idx,
                                        tab_title(tab.agent, tab.cwd.as_deref(), &tab.info.name),
                                        idx == ws.active,
                                    )
                                })
                                .collect();
                            tab_widget::tab_overflow_items(&entries, Message::SelectTab)
                        }
                        None => Vec::new(),
                    };
                    if let Some(msg) = crate::native_menu::show(items, last_cursor) {
                        self.update(msg);
                    }
                }
                #[cfg(not(target_os = "macos"))]
                {
                    let last_cursor = self.last_cursor;
                    self.with_focused_project(|ws, _io| {
                        ws.term_tab_overflow_anchor = if ws.term_tab_overflow_anchor.is_some() {
                            None
                        } else {
                            Some(last_cursor)
                        };
                    });
                }
            }
            Message::TermTabOverflowDismiss => {
                self.with_focused_project(|ws, _io| {
                    ws.term_tab_overflow_anchor = None;
                });
            }
            Message::PreviewTabOverflowToggle => {
                #[cfg(target_os = "macos")]
                {
                    let last_cursor = self.last_cursor;
                    let items = match self.active_workspace() {
                        Some(ws) => {
                            let entries: Vec<(usize, String, bool)> = ws
                                .preview
                                .tabs()
                                .iter()
                                .enumerate()
                                .map(|(idx, tab)| {
                                    (idx, tab.title.clone(), idx == ws.preview.active_idx())
                                })
                                .collect();
                            tab_widget::tab_overflow_items(&entries, Message::PreviewSelectTab)
                        }
                        None => Vec::new(),
                    };
                    if let Some(msg) = crate::native_menu::show(items, last_cursor) {
                        self.update(msg);
                    }
                }
                #[cfg(not(target_os = "macos"))]
                {
                    let last_cursor = self.last_cursor;
                    self.with_focused_project(|ws, _io| {
                        ws.preview_tab_overflow_anchor = if ws.preview_tab_overflow_anchor.is_some()
                        {
                            None
                        } else {
                            Some(last_cursor)
                        };
                    });
                }
            }
            Message::PreviewTabOverflowDismiss => {
                self.with_focused_project(|ws, _io| {
                    ws.preview_tab_overflow_anchor = None;
                });
            }
            Message::TermSelStart {
                target,
                col,
                row,
                right,
            } => {
                self.with_focused_project(|ws, _io| {
                    let tab = match target {
                        terminal::TermTarget::Shared => ws.tabs.get_mut(ws.active),
                        terminal::TermTarget::SshPanel => ws.ssh_active_tab_mut(),
                    };
                    if let Some(tab) = tab {
                        tab.model.selection_start(col, row, right);
                    }
                });
            }
            Message::TermSelUpdate {
                target,
                col,
                row,
                right,
            } => {
                self.with_focused_project(|ws, _io| {
                    let tab = match target {
                        terminal::TermTarget::Shared => ws.tabs.get_mut(ws.active),
                        terminal::TermTarget::SshPanel => ws.ssh_active_tab_mut(),
                    };
                    if let Some(tab) = tab {
                        tab.model.selection_update(col, row, right);
                    }
                });
            }
            Message::TermPaste(target, text) => self.term_paste(target, text),
            Message::PreviewOpenPath(path) => self.preview_open_path(path),
            Message::PreviewSelectTab(idx) => self.preview_select_tab(idx),
            Message::PreviewCloseTab(idx) => {
                self.with_focused_project(|ws, io| {
                    // 关闭前静默保存该 tab 的就地改动(仅当它是脏的原生 tab 才
                    // 动作;不脏/走 wry 的 tab 内部直接 no-op)——复用
                    // `preview_pane_save_at` 的落盘 + 清脏 + 面板 error 管线,抵掉
                    // 关闭即丢改动。顺序:先 `save_at`(取的是关闭前的下标 + buffer)
                    // 再 `close`。
                    ws.preview_pane_save_at(PanelKind::Files, idx);
                    ws.preview.close(idx);
                    // 关 tab 后位置全变，旧 first 可能越界——归零防御（P1L T5）。
                    ws.preview_tab_first = 0;
                    ws.spawn_preview_state_save(io);
                    ws.spawn_preview_context_push(io);
                });
            }
            Message::PreviewToggleRenderMode(idx) => {
                self.with_focused_project(|ws, _io| {
                    ws.preview_pane_toggle_render_mode(PanelKind::Files, idx);
                });
            }
            Message::PreviewEditorEvent(_tab_id, _action) => {
                // main.rs 直接调 `App::preview_tab_editor_event`,不经过这里。
            }
            Message::PreviewSaveActive(kind) => {
                self.with_focused_project(move |ws, _io| ws.preview_pane_save_active(kind));
            }
            Message::PreviewTabInsertTab(kind) => {
                self.with_focused_project(move |ws, _io| {
                    ws.preview_pane_active_editor_event(
                        kind,
                        iced_widget::text_editor::Action::Edit(
                            iced_widget::text_editor::Edit::Insert('\t'),
                        ),
                    );
                });
            }
            Message::PreviewUndoActive(kind) => {
                self.with_focused_project(move |ws, _io| ws.preview_pane_undo_active(kind));
            }
            Message::PreviewRedoActive(kind) => {
                self.with_focused_project(move |ws, _io| ws.preview_pane_redo_active(kind));
            }
            Message::PreviewFindOpen(kind) => {
                self.with_focused_project(move |ws, _io| ws.preview_find_open(kind));
            }
            Message::PreviewFindOpenWithReplace(kind) => {
                self.with_focused_project(move |ws, _io| ws.preview_find_open_with_replace(kind));
            }
            Message::PreviewFindReplaceToggle(kind) => {
                self.with_focused_project(move |ws, _io| ws.preview_find_toggle_replace(kind));
            }
            Message::PreviewFindClose(kind) => {
                self.with_focused_project(move |ws, _io| ws.preview_find_close(kind));
            }
            Message::PreviewFindText(kind, query) => {
                self.with_focused_project(move |ws, _io| ws.preview_find_type(kind, query));
            }
            Message::PreviewFindGo(kind, next) => {
                self.with_focused_project(move |ws, _io| ws.preview_find_go(kind, next));
            }
            Message::PreviewFindCase(kind, sensitive) => {
                self.with_focused_project(move |ws, _io| ws.preview_find_case(kind, sensitive));
            }
            Message::PreviewFindReplacement(kind, repl) => {
                self.with_focused_project(move |ws, _io| {
                    ws.preview_find_set_replacement(kind, repl);
                });
            }
            Message::PreviewFindReplaceCurrent(kind) => {
                self.with_focused_project(move |ws, _io| {
                    ws.preview_find_replace_current(kind);
                });
            }
            Message::PreviewFindReplaceAll(kind) => {
                self.with_focused_project(move |ws, _io| {
                    ws.preview_find_replace_all(kind);
                });
            }
            Message::ProjectPreviewOpenPath(path) => self.project_preview_open_path(path),
            Message::ProjectPreviewSelectTab(idx) => self.project_preview_select_tab(idx),
            Message::ProjectPreviewCloseTab(idx) => {
                self.with_focused_project(|ws, _io| {
                    // 关闭前静默保存,语义同 `PreviewCloseTab`(先 `save_at` 再 close)。
                    ws.preview_pane_save_at(PanelKind::Project, idx);
                    ws.project_preview.close(idx);
                    ws.project_preview_tab_first = 0;
                });
            }
            Message::ProjectPreviewToggleRenderMode(idx) => {
                self.with_focused_project(|ws, _io| {
                    ws.preview_pane_toggle_render_mode(PanelKind::Project, idx);
                });
            }
            Message::ProjectPreviewTabOverflowToggle => {
                #[cfg(target_os = "macos")]
                {
                    let last_cursor = self.last_cursor;
                    let items = match self.active_workspace() {
                        Some(ws) => {
                            let entries: Vec<(usize, String, bool)> = ws
                                .project_preview
                                .tabs()
                                .iter()
                                .enumerate()
                                .map(|(idx, tab)| {
                                    (
                                        idx,
                                        tab.title.clone(),
                                        idx == ws.project_preview.active_idx(),
                                    )
                                })
                                .collect();
                            tab_widget::tab_overflow_items(
                                &entries,
                                Message::ProjectPreviewSelectTab,
                            )
                        }
                        None => Vec::new(),
                    };
                    if let Some(msg) = crate::native_menu::show(items, last_cursor) {
                        self.update(msg);
                    }
                }
                #[cfg(not(target_os = "macos"))]
                {
                    let last_cursor = self.last_cursor;
                    self.with_focused_project(|ws, _io| {
                        ws.project_preview_tab_overflow_anchor =
                            if ws.project_preview_tab_overflow_anchor.is_some() {
                                None
                            } else {
                                Some(last_cursor)
                            };
                    });
                }
            }
            Message::ProjectPreviewTabOverflowDismiss => {
                self.with_focused_project(|ws, _io| {
                    ws.project_preview_tab_overflow_anchor = None;
                });
            }
            Message::ProjectPreviewEditorEvent(_tab_id, _event) => {
                // 与 `PreviewEditorEvent` 同口径:到达 `App::update` 说明未走
                // main.rs 的 Task 桥接器,直接忽略。
            }
            Message::Browser(browser::Message::BookmarksLoaded(pid, bookmarks)) => {
                self.browser_bookmarks_loaded(pid, bookmarks)
            }
            Message::Browser(browser::Message::BookmarksMutated(pid, res)) => {
                self.browser_bookmarks_mutated(pid, res)
            }
            Message::Browser(browser::Message::DragHover(idx)) => {
                // 浏览器 tab 脱的换位:光标扫过 `idx` 页签 → 走共同换位逻辑。
                self.tab_drag_move(TabGroup::Browser, idx);
            }
            Message::Browser(browser::Message::ColumnDragStart) => {
                self.update(Message::ColumnDragStart(Divider::BrowserBookmarksSplit));
            }
            Message::Browser(browser::Message::TextInputMenuOpen(target)) => {
                self.update(Message::TextInputMenuOpen(target));
            }
            Message::Browser(msg) => self.browser_message(msg),
            Message::ProjectSelect(id) => self.project_select(id),
            Message::ProjectTabPickFolder => {
                // 副作用在 main.rs(rfd 文件夹选择);从新增项目菜单触发时顺带
                // 关掉菜单,同 `project_select` 的处理口径。
                self.project_add_menu_open = false;
            }
            // rfd 弹窗在 main.rs 里同步处理,选中后转成 project::Message::LinkAdd
            // 再回送到这里;这条顶层消息本身不需要 App::update 处理任何东西。
            Message::ProjectLinkPick(_) => {}
            Message::ProjectLinkContextMenuClose => {
                self.project_link_menu = None;
            }
            Message::CategoryContextMenuClose => {
                self.category_context_menu = None;
            }
            Message::CategoryPickerClose => {
                self.category_picker = None;
            }
            Message::CategoryPickerSelect(chosen) => {
                let Some(picker) = self.category_picker.take() else {
                    return;
                };
                match picker.target {
                    CategoryPickerTarget::Todo(todo_id) => {
                        let client = self.client.clone();
                        let handle = self.handle.clone();
                        let proxy = self.proxy.clone();
                        handle.spawn(async move {
                            let res = client
                                .set_todo_category(todo_id, chosen)
                                .await
                                .map(|_| ())
                                .map_err(|e| e.to_string());
                            let _ = proxy
                                .send_event(Message::Todo(todo::Message::CategoryMutated(res)));
                        });
                    }
                    CategoryPickerTarget::Category(category_id) => {
                        let client = self.client.clone();
                        let handle = self.handle.clone();
                        let proxy = self.proxy.clone();
                        handle.spawn(async move {
                            let res = client
                                .reparent_category(category_id, chosen)
                                .await
                                .map(|_| ())
                                .map_err(|e| e.to_string());
                            let _ = proxy
                                .send_event(Message::Todo(todo::Message::CategoryMutated(res)));
                        });
                    }
                }
            }
            Message::DatabaseSourceContextMenuClose => {
                self.database_source_menu = None;
            }
            Message::ProjectTabOpen(path) => {
                let client = self.client.clone();
                let proxy = self.proxy.clone();
                let path_s = path.to_string_lossy().into_owned();
                self.handle.spawn(async move {
                    let opened = client.open_project(&path_s).await.ok().flatten();
                    let recent = client.list_projects().await.unwrap_or_default();
                    let _ = proxy.send_event(Message::ProjectTabOpened(opened, recent));
                });
            }
            Message::ProjectTabOpened(project, recent) => self.project_tab_opened(project, recent),
            Message::ProjectTabSwitch(id) => self.project_tab_switch(id),
            Message::ProjectTabClose(id) => self.project_tab_close(id),
            Message::ProjectSlotLoaded(id, payload) => self.project_slot_loaded(id, payload),
            Message::ProjectFsChanged(project_id, changes) => {
                self.project_fs_changed(project_id, changes)
            }
            Message::GitLog(git_log::Message::ColumnDragStart) => {
                // Git Log 三栏布局里左右分割线开始拖拽——扩展发不了 app 级
                // 拖拽消息,由内核代发。
                self.update(Message::ColumnDragStart(Divider::GitLogSplit));
            }
            Message::GitLog(git_log::Message::RowDragStart) => {
                self.update(Message::RowDragStart(RowDivider::GitLogFileDiffSplit));
            }
            Message::GitLog(git_log::Message::BranchPickerOpen) => {
                // 先把"展开"这个状态位落地(纯状态机部分仍走 update,不跳过),
                // 首次展开且还没缓存过分支列表时,顺带异步查一次本地分支。
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let emit = move |m| {
                    let _ = proxy.send_event(Message::GitLog(m));
                };
                let needs_fetch = self.git_log.branches_is_empty();
                git_log::update(
                    &mut self.git_log,
                    git_log::Message::BranchPickerOpen,
                    &handle,
                    emit.clone(),
                );
                if needs_fetch
                    && let Some(repo_path) = self
                        .active_workspace()
                        .and_then(|ws| ws.active_project_path())
                {
                    self.handle.spawn(async move {
                        let repo_path2 = repo_path.clone();
                        let (branches, dirty) = tokio::task::spawn_blocking(move || {
                            let branches =
                                crate::delivery::local_branches(&repo_path2).unwrap_or_default();
                            let dirty = crate::delivery::is_dirty(&repo_path2);
                            (branches, dirty)
                        })
                        .await
                        .unwrap_or_default();
                        emit(git_log::Message::BranchesLoaded(repo_path, branches, dirty));
                    });
                }
            }
            Message::GitLog(git_log::Message::BranchSwitch(name)) => {
                let Some(repo_path) = self
                    .active_workspace()
                    .and_then(|ws| ws.active_project_path())
                else {
                    return;
                };
                self.git_log.set_branch_switch_pending(true);
                let proxy = self.proxy.clone();
                self.handle.spawn(async move {
                    let repo_path2 = repo_path.clone();
                    let result = tokio::task::spawn_blocking(move || {
                        crate::delivery::checkout_branch(&repo_path2, &name)
                    })
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()));
                    let _ = proxy
                        .send_event(Message::GitLog(git_log::Message::BranchSwitchDone(result)));
                });
            }
            Message::GitLog(git_log::Message::TextInputMenuOpen(target)) => {
                self.update(Message::TextInputMenuOpen(target));
            }
            Message::GitLog(msg) => {
                if let git_log::Message::Hover(id, h) = msg {
                    self.set_hover(id, h);
                    return;
                }
                // 分支切换成功后(checkout 改了 HEAD/工作区),commit 列表要重拉。
                let is_branch_switch_success =
                    matches!(&msg, git_log::Message::BranchSwitchDone(Ok(())));
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let emit = move |m| {
                    let _ = proxy.send_event(Message::GitLog(m));
                };
                if let Some(next) = git_log::update(&mut self.git_log, msg, &handle, emit.clone()) {
                    self.update(Message::GitLog(next));
                }
                if is_branch_switch_success
                    && let Some(repo_path) = self
                        .active_workspace()
                        .and_then(|ws| ws.active_project_path())
                {
                    let max_count = self.git_log.cache_max_count();
                    git_log::request_refresh(
                        &mut self.git_log,
                        repo_path,
                        max_count,
                        &handle,
                        emit,
                    );
                }
            }
            Message::Files(files::Message::CopyPath(path, kind)) => {
                let _ = (path, kind); // main.rs 拦截处理写剪贴板,这里维持现状空分支
            }
            Message::Files(files::Message::OpenSearch(path, is_dir)) => {
                // 右键菜单"搜索":跨 `files::Message` 边界,由内核把它映射成
                // `search::Message::SearchOpen`。先关右键菜单(否则搜索弹窗
                // dismiss 一关,旧菜单又冒回来),作用域由 `is_dir` 决定——目录
                // 按目录递归搜,文件只搜单文件。
                self.files.close_context_menu();
                let scope = if is_dir {
                    search::Scope::Dir(path)
                } else {
                    search::Scope::File(path)
                };
                self.update(Message::Search(search::Message::SearchOpen(scope)));
            }
            Message::Files(
                msg @ (files::Message::StatusesRefreshed(project_id, ..)
                | files::Message::PasteDone(project_id, ..)
                | files::Message::OpDone { project_id, .. }
                | files::Message::GitInfoLoaded(project_id, ..)
                | files::Message::BranchSwitchDone(project_id, ..)
                | files::Message::GitInitDone(project_id, ..)
                | files::Message::FileDropDone(project_id, ..)),
            ) => self.files_project_message(project_id, msg),

            Message::Files(files::Message::ToolbarHover(target, hovered)) => {
                // 文件树工具行 icon 按钮的 hover:本面板不挂 App 的 hover 动画
                // 表,把进入/离开转发成 `HoverId` 由内核统一驱动动画进度。
                let id = match target {
                    files::FilesToolbarTarget::SearchSubmit => HoverId::FilesSearchSubmit,
                    files::FilesToolbarTarget::Dotfiles => HoverId::FilesDotfiles,
                    files::FilesToolbarTarget::BranchSwitch => HoverId::FilesBranchSwitch,
                };
                self.set_hover(id, hovered);
            }
            Message::Files(files::Message::TextInputMenuOpen(target)) => {
                self.update(Message::TextInputMenuOpen(target));
            }
            // 树内行被按下:只武装拖拽(供 main.rs 全局松开左键时收尾),
            // **不**立即执行任何点击语义(既不展开/折叠目录,也不打开文件)。
            // 两者都推迟到真正松开、且确认这其实只是一次单击(未越过拖拽
            // 确认阈值)时才在 `TreeDragEnd` 里补做——展开目录会让下方行
            // 布局位移,若按下就立即展开,静止不动的光标可能被 iced 判定成
            // 树内行被按下:只武装 `Pending`(见 `TreeDragPhase` 文档)——
            // `Pending` 期间完全没有任何反应,不挂 `on_move`、不展开目录、
            // 不打开文件。真正推进到 `Dragging`(越过距离+时长两道阈值)由
            // `maybe_confirm_tree_drag` 在每次 `CursorMoved` 时判断(main.rs
            // 调用),不在这里做。
            Message::Files(files::Message::TreeRowPress { path, is_dir }) => {
                let press_pos = self.last_cursor;
                if let Some(ws) = self.active_workspace_mut() {
                    ws.files
                        .arm_tree_drag(path, is_dir, press_pos, std::time::Instant::now());
                }
            }
            // 树内拖拽松开左键:main.rs 发这条。`confirmed` 就是"这场拖拽有
            // 没有走到 `Dragging` 阶段"——由 `maybe_confirm_tree_drag` 在
            // 越过阈值那一刻就已经推进过一次,这里直接读结果,不重新算
            // 距离/时长(2026-09 用户实测反馈带诊断日志实锤过纯距离阈值挡
            // 不住 trackpad 快速点按的真实位移,才改成阈值判断只在
            // `Pending → Dragging` 转换时做一次、结果落进状态机里的这个
            // 设计,见 `TreeDragPhase` 文档)。
            Message::Files(files::Message::TreeDragRelease) => {
                let confirmed = self
                    .active_workspace()
                    .is_some_and(|ws| ws.files.tree_drag_confirmed());
                self.update(Message::Files(files::Message::TreeDragEnd(confirmed)));
            }
            // 树行双击:目录复用 `Message::Toggle` 那条本地消息直接切换展开
            // 态(同点箭头效果),文件跨到 `PreviewOpenPath`——该面板本身
            // 不认识这条消息,见 `files::Message::TreeRowDoubleClick` 文档。
            Message::Files(files::Message::TreeRowDoubleClick { path, is_dir }) => {
                if is_dir {
                    self.update(Message::Files(files::Message::Toggle(path)));
                } else {
                    self.update(Message::PreviewOpenPath(path));
                }
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
            Message::Project(
                msg @ (project::Message::GitRefreshed(project_id, ..)
                | project::Message::NameRenamed(project_id, ..)
                | project::Message::DiskUsageLoaded(project_id, ..)
                | project::Message::ScaffoldStepStarted(project_id, ..)
                | project::Message::ScaffoldStepFinished(project_id, ..)
                | project::Message::TranscriptBackfillStarted(project_id, ..)
                | project::Message::TranscriptBackfillFinished(project_id, ..)
                | project::Message::SummaryBackfillProgress(project_id, ..)),
            ) => {
                let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
                    return;
                };
                let Some(project) = ws.project.as_ref() else {
                    return;
                };
                let current_name = project.name.clone();
                let repo_path = std::path::PathBuf::from(&project.path);
                // `NameRenamed(Ok(updated))` 要把顶栏项目页签等读的 `ws.project`
                // 缓存一并更新——这是这个面板第一次出现需要内核介入(而不是纯
                // 委托给 `project::update`)的消息。
                if let project::Message::NameRenamed(_, Ok(updated)) = &msg {
                    ws.project = Some(updated.clone());
                }
                // 补总结进度追到 Done 时,弹窗外面缓存的 `conversation_sessions`
                // (`spawn_conversations_refresh` 唯一写入点)不会自动感知
                // `session_summaries` 表的新增行——不重新拉一次,对话列表会一直
                // 显示"未总结"直到用户重开项目 tab 或触发别的回合结束刷新。
                let refresh_conversations = matches!(
                    &msg,
                    project::Message::SummaryBackfillProgress(_, completed, total)
                        if completed >= total
                );
                let client = self.client.clone();
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let emit = move |m| {
                    let _ = proxy.send_event(Message::Project(m));
                };
                project::update(
                    &mut ws.project_panel,
                    msg,
                    project_id,
                    &current_name,
                    &repo_path,
                    &client,
                    &handle,
                    emit,
                );
                if refresh_conversations {
                    self.with_project(project_id, |ws, io| {
                        ws.spawn_conversations_refresh(io);
                    });
                }
            }
            Message::Project(project::Message::OpenLink(path)) => {
                // 项目链接打开的文件进 Project 面板右配对的预览(`ws.project_preview`),
                // 不冲进 Files 预览——两条预览各自独立,互相不打扰。
                self.update(Message::ProjectPreviewOpenPath(path));
            }
            Message::Project(project::Message::Pick(target)) => {
                // 文件/目录选择器依赖 macOS 主线程原生能力(rfd/NSOpenPanel 模态,
                // 见 main.rs `pick_file_or_dir`),必须由 main.rs 的 winit 事件循环
                // 里 `dispatch` 拦截同步执行。这里只用代理把这条消息回灌回事件循环
                // ——不能 `self.update(Message::ProjectLinkPick(..))` 直调:那是同步
                // 递归,只会命中 `App::update` 里那格 no-op,绝不会触发文件选择弹窗。
                let _ = self.proxy.send_event(Message::ProjectLinkPick(target));
            }
            Message::Project(project::Message::LinkContextMenu { target, index }) => {
                self.project_link_context_menu(target, index);
            }
            Message::Project(project::Message::LinkRemove { target, index }) => {
                // 删除来自行内右键菜单:落 `LinkRemove` 时把菜单浮层一并收起,
                // 然后委托 `project::update` 真正执行删除(含越界校验与保存失败
                // 回滚,见 `project.rs` 的 `Message::LinkRemove`)。不能
                // `self.update(同一条 LinkRemove)` 直调——那会命中本分支自身
                // 再次匹配 `LinkRemove`,无限递归爆栈。
                self.project_link_menu = None;
                let Some(project_id) = self.active_project_id else {
                    return;
                };
                let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
                    return;
                };
                let Some(project) = ws.project.as_ref() else {
                    return;
                };
                let current_name = project.name.clone();
                let repo_path = std::path::PathBuf::from(&project.path);
                let client = self.client.clone();
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let emit = move |m| {
                    let _ = proxy.send_event(Message::Project(m));
                };
                project::update(
                    &mut ws.project_panel,
                    project::Message::LinkRemove { target, index },
                    project_id,
                    &current_name,
                    &repo_path,
                    &client,
                    &handle,
                    emit,
                );
            }
            Message::Project(project::Message::DeleteProjectConfirm) => {
                self.project_delete_confirm();
            }
            Message::Project(project::Message::TextInputMenuOpen(target)) => {
                self.update(Message::TextInputMenuOpen(target));
            }
            Message::Project(msg) => {
                let Some(project_id) = self.active_project_id else {
                    return;
                };
                let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
                    return;
                };
                let Some(project) = ws.project.as_ref() else {
                    return;
                };
                let current_name = project.name.clone();
                let repo_path = std::path::PathBuf::from(&project.path);
                let client = self.client.clone();
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let emit = move |m| {
                    let _ = proxy.send_event(Message::Project(m));
                };
                project::update(
                    &mut ws.project_panel,
                    msg,
                    project_id,
                    &current_name,
                    &repo_path,
                    &client,
                    &handle,
                    emit,
                );
            }
            Message::Ssh(ssh::Message::TestConnectionResult(project_id, host_id, result)) => {
                self.ssh_test_connection_result(project_id, host_id, result)
            }
            Message::Ssh(ssh::Message::UnknownKeyDetected(
                project_id,
                host_id,
                fingerprint,
                key_bytes,
            )) => self.ssh_unknown_key_detected(project_id, host_id, fingerprint, key_bytes),
            Message::Ssh(ssh::Message::KeyChanged(project_id, host_id, fingerprint)) => {
                self.ssh_key_changed(project_id, host_id, fingerprint)
            }
            // 点"终端"按钮:与既有 `TestConnection`/其它同步交互消息不同,
            // 这个消息不走 `ssh::update`(它要新建一个 tab,需要 `&mut
            // Workspace` 整体,`ssh::update` 只拿得到 `&mut ws.ssh`)——
            // 拦截在通配 `Message::Ssh(msg)` 之前,直接调 `Workspace::
            // spawn_ssh_tab`。
            Message::Ssh(ssh::Message::OpenSshTab(host_id, ssh::SshTabKind::Terminal)) => {
                // 新 SSH 终端从一开始就用 SSH 面板自己的网格(不是共享终端的
                // 列数)——在闭包里再借 `self` 会与 `with_focused_project` 的
                // `&mut self` 冲突,先取到局变量。
                let ssh_cols = self.ssh_cols;
                let ssh_rows = self.ssh_rows;
                self.with_focused_project(|ws, io| {
                    // 已经开着这台主机的终端 tab 就直接切过去,不重新握手
                    // 连一遍(阶段 3 SFTP 决定"每个 tab 独立新建连接",但
                    // 终端 tab 本来就是"一台主机一条常驻连接",重复点
                    // "终端"图标应该是切换焦点而不是叠加新连接)。
                    let already_open = ws
                        .ssh_tabs
                        .iter()
                        .any(|t| t.info.id.strip_prefix("ssh:") == Some(host_id.as_str()));
                    if already_open {
                        ws.select_ssh_tab(host_id, ssh::SshTabKind::Terminal);
                    } else {
                        ws.ssh.record_reopen_after_trust(host_id.clone());
                        ws.spawn_ssh_tab(io, host_id, ssh_cols, ssh_rows);
                    }
                });
            }
            // Sftp 阶段 3:真实打开一个 SFTP tab(独立连接 + 命令通道)。
            Message::Ssh(ssh::Message::OpenSshTab(host_id, ssh::SshTabKind::Sftp)) => {
                self.with_focused_project(|ws, io| {
                    if ws.sftp_tabs.contains_key(&host_id) {
                        ws.select_ssh_tab(host_id, ssh::SshTabKind::Sftp);
                    } else {
                        ws.spawn_sftp_tab(io, host_id.clone());
                        ws.select_ssh_tab(host_id, ssh::SshTabKind::Sftp);
                    }
                });
            }
            Message::Ssh(ssh::Message::CloseSshTab(host_id, kind)) => {
                self.with_focused_project(|ws, io| match kind {
                    ssh::SshTabKind::Terminal => ws.close_ssh_tab(io, &host_id, kind),
                    ssh::SshTabKind::Sftp => {
                        ws.sftp_tabs.remove(&host_id);
                        if ws.ssh_active.as_ref().map(|(h, k)| (h.as_str(), *k))
                            == Some((host_id.as_str(), ssh::SshTabKind::Sftp))
                        {
                            ws.ssh_active = None; // 简化处理:关掉 SFTP tab 后不自动
                            // 切到其它 tab,和终端 tab 关闭后的
                            // "切到剩下第一个"逻辑不强行统一,
                            // 因为 ssh_tabs/sftp_tabs 是两个不同
                            // 集合,统一切换逻辑收益不大,YAGNI。
                        }
                    }
                });
            }
            Message::Ssh(ssh::Message::SelectSshTab(host_id, kind)) => {
                self.with_focused_project(|ws, _io| {
                    ws.select_ssh_tab(host_id, kind);
                    let mut widths: Vec<f32> = vec![tab_display_width("空白")];
                    widths.extend(ws.ssh_tabs.iter().map(|t| {
                        tab_display_width(&tab_title(t.agent, t.cwd.as_deref(), &t.info.name))
                    }));
                    widths.extend(ws.sftp_tabs.keys().map(|host_id| {
                        tab_display_width(
                            &ws.ssh
                                .hosts()
                                .iter()
                                .find(|h| &h.id == host_id)
                                .map(|h| h.name.clone())
                                .unwrap_or_else(|| host_id.clone()),
                        )
                    }));
                    let target = match &ws.ssh_active {
                        None => 0,
                        Some((active_host, ssh::SshTabKind::Terminal)) => ws
                            .ssh_tabs
                            .iter()
                            .position(|t| {
                                t.info.id.strip_prefix("ssh:") == Some(active_host.as_str())
                            })
                            .map(|i| i + 1)
                            .unwrap_or(0),
                        Some((active_host, ssh::SshTabKind::Sftp)) => ws
                            .sftp_tabs
                            .keys()
                            .position(|h| h == active_host)
                            .map(|i| i + 1 + ws.ssh_tabs.len())
                            .unwrap_or(0),
                    };
                    ws.ssh_tab_first = tab_widget::tab_window_reveal(
                        &widths,
                        4.0,
                        byteui::theme::geometry::tab_bar_avail_px(),
                        ws.ssh_tab_first,
                        target,
                    );
                    ws.ssh_tab_overflow_anchor = None;
                });
            }
            // 点固定的"空白"占位 tab:它不对应 `ssh_tabs`/`sftp_tabs` 里
            // 任何一条记录,选中态就是 `ssh_active == None`。
            Message::Ssh(ssh::Message::SelectBlankTab) => {
                self.with_focused_project(|ws, _io| {
                    ws.ssh_active = None;
                    let mut widths: Vec<f32> = vec![tab_display_width("空白")];
                    widths.extend(ws.ssh_tabs.iter().map(|t| {
                        tab_display_width(&tab_title(t.agent, t.cwd.as_deref(), &t.info.name))
                    }));
                    widths.extend(ws.sftp_tabs.keys().map(|host_id| {
                        tab_display_width(
                            &ws.ssh
                                .hosts()
                                .iter()
                                .find(|h| &h.id == host_id)
                                .map(|h| h.name.clone())
                                .unwrap_or_else(|| host_id.clone()),
                        )
                    }));
                    ws.ssh_tab_first = tab_widget::tab_window_reveal(
                        &widths,
                        4.0,
                        byteui::theme::geometry::tab_bar_avail_px(),
                        ws.ssh_tab_first,
                        0,
                    );
                    ws.ssh_tab_overflow_anchor = None;
                });
            }
            Message::Ssh(ssh::Message::TabOverflowToggle) => {
                #[cfg(target_os = "macos")]
                {
                    let last_cursor = self.last_cursor;
                    let items = match self.active_workspace() {
                        Some(ws) => {
                            let mut entries: Vec<(usize, String, bool)> = Vec::new();
                            for (i, tab) in ws.ssh_tabs.iter().enumerate() {
                                let idx = i + 1;
                                let host_id = tab
                                    .info
                                    .id
                                    .strip_prefix("ssh:")
                                    .unwrap_or(&tab.info.id)
                                    .to_string();
                                entries.push((
                                    idx,
                                    tab_title(tab.agent, tab.cwd.as_deref(), &tab.info.name),
                                    ws.ssh_active.as_ref().is_some_and(|(h, k)| {
                                        h == &host_id && *k == ssh::SshTabKind::Terminal
                                    }),
                                ));
                            }
                            for (i, (host_id, _)) in ws.sftp_tabs.iter().enumerate() {
                                let idx = i + 1 + ws.ssh_tabs.len();
                                let label = ws
                                    .ssh
                                    .hosts()
                                    .iter()
                                    .find(|h| &h.id == host_id)
                                    .map(|h| h.name.clone())
                                    .unwrap_or_else(|| host_id.clone());
                                entries.push((
                                    idx,
                                    label,
                                    ws.ssh_active.as_ref()
                                        == Some(&(host_id.clone(), ssh::SshTabKind::Sftp)),
                                ));
                            }
                            tab_widget::tab_overflow_items(&entries, |idx| {
                                ssh_tab_overflow_select_message(ws, idx)
                            })
                        }
                        None => Vec::new(),
                    };
                    if let Some(msg) = crate::native_menu::show(items, last_cursor) {
                        self.update(msg);
                    }
                }
                #[cfg(not(target_os = "macos"))]
                {
                    let last_cursor = self.last_cursor;
                    self.with_focused_project(|ws, _io| {
                        ws.ssh_tab_overflow_anchor = if ws.ssh_tab_overflow_anchor.is_some() {
                            None
                        } else {
                            Some(last_cursor)
                        };
                    });
                }
            }
            Message::Ssh(ssh::Message::TabOverflowDismiss) => {
                self.with_focused_project(|ws, _io| {
                    ws.ssh_tab_overflow_anchor = None;
                });
            }
            // SFTP tab 内部交互:按 host_id 路由到 `sftp::route`,真正的
            // 处理逻辑在那边(sftp::Message 有 7+ 个变体,内容又都操作
            // `ws.sftp_tabs`,摊平会让这里的大 match 更难读)。
            Message::Ssh(ssh::Message::Sftp(ssh::sftp::Message::ContextMenuOpen {
                host_id,
                is_local,
                path,
            })) => {
                #[cfg(target_os = "macos")]
                {
                    // 先落选中态(同 `sftp::route` 的 ContextMenuOpen 分支),
                    // 再同步弹原生菜单,结果经 `Ssh(Sftp(msg))` 回路由。
                    self.with_focused_project(|ws, io| {
                        ssh::sftp::route(
                            ws,
                            io,
                            ssh::sftp::Message::ContextMenuOpen {
                                host_id: host_id.clone(),
                                is_local,
                                path: path.clone(),
                            },
                        );
                    });
                    let (x, y) = self.files.last_right_click();
                    let items = ssh::sftp::context_menu_items(&host_id, is_local);
                    if let Some(msg) = crate::native_menu::show(items, (x, y)) {
                        self.update(Message::Ssh(ssh::Message::Sftp(msg)));
                    }
                }
                #[cfg(not(target_os = "macos"))]
                {
                    self.with_focused_project(|ws, io| {
                        ssh::sftp::route(
                            ws,
                            io,
                            ssh::sftp::Message::ContextMenuOpen {
                                host_id,
                                is_local,
                                path,
                            },
                        );
                    });
                }
            }
            Message::Ssh(ssh::Message::Sftp(msg)) => {
                self.with_focused_project(|ws, io| {
                    ssh::sftp::route(ws, io, msg);
                });
            }
            // 终端连接失败:先做内核层面的清理(pending/ssh_out_pending
            // 两处暂存——这次连接没能走到 `TabAttached`,不清理会一直占着
            // 这两个 map 的位置),再转给 `ssh::update` 落卡片状态(同
            // `TestConnectionResult` 的路由口径,带显式 project_id,套用
            // 一模一样的 `with_project` 外壳)。
            Message::Ssh(ssh::Message::TerminalConnectFailed(project_id, host_id, tab_id, err)) => {
                self.ssh_terminal_connect_failed(project_id, host_id, tab_id, err)
            }
            Message::Ssh(ssh::Message::TextInputMenuOpen(target)) => {
                self.update(Message::TextInputMenuOpen(target));
            }
            Message::Ssh(msg) => {
                if let ssh::Message::Hover(id, h) = msg {
                    self.set_hover(id, h);
                    return;
                }
                self.with_focused_project(|ws, io| {
                    let Some(project) = ws.project.as_ref() else {
                        return;
                    };
                    let project_id = project.id;
                    let repo_path = PathBuf::from(&project.path);
                    let handle = io.handle.clone();
                    let proxy = io.proxy.clone();
                    let emit = move |m| {
                        let _ = proxy.send_event(Message::Ssh(m));
                    };
                    ssh::update(&mut ws.ssh, msg, project_id, &repo_path, &handle, emit);
                });
            }
            Message::Footbar(msg) => {
                // App 级 + 纯展示,不带 project_id,不需要
                // `with_project`/`with_focused_project`,直接更新。
                footbar::update(&mut self.footbar, msg);
            }
            Message::ZoomIn => {
                byteui::theme::icon_size::zoom_by(UI_ZOOM_STEP);
                byteui::theme::icon_size::persist_scale(&crate::theme::ui_scale_path());
                self.sync_terminal_grid();
                self.pending_preview_zoom = true;
            }
            Message::ZoomOut => {
                byteui::theme::icon_size::zoom_by(1.0 / UI_ZOOM_STEP);
                byteui::theme::icon_size::persist_scale(&crate::theme::ui_scale_path());
                self.sync_terminal_grid();
                self.pending_preview_zoom = true;
            }
            Message::ZoomReset => {
                byteui::theme::icon_size::reset_scale(&crate::theme::ui_scale_path());
                self.sync_terminal_grid();
                self.pending_preview_zoom = true;
            }
            Message::SettingsOpen => {
                self.settings_modal_open = true;
            }
            Message::SettingsClose => {
                self.settings_modal_open = false;
            }
            Message::SettingsThemeSelected(scheme) => {
                byteui::theme::color::set_scheme(scheme);
                byteui::theme::color::persist_scheme(&crate::theme::color_theme_path());
            }
            // WebViewFocused 只在 main.rs 的 dispatch 里设 pending_focus,
            // App::update 无需处理。
            Message::WebViewFocused => {}
            // 鼠标在子 webview 上松开(见 `WebViewMouseUp` 文档):一并结束页签
            // 拖拽,避免"松开还能继续拖"。
            Message::WebViewMouseUp => self.end_tab_drag(),
        }
    }

    fn project_select(&mut self, id: i64) {
        // 从新增项目菜单点选时顺带关掉菜单(菜单本来就该在选中后消失);
        // 从其它入口(首页最近项目卡片)调用时这里恒为 false,no-op。
        self.project_add_menu_open = false;
        // 切项目不再通知 daemon:"活跃项目"是 GUI 侧的概念了(P2a
        // Task 1-3 删掉了 SetActiveProject)。
        //
        // 这个项目已经开着页签(`Loaded` 或还没促成的 `Stub`)时,点最近
        // 项目卡片就只是"切到那个页签",走与点页签完全相同的非破坏性
        // 路径——绝不能杀掉任何已有页签的会话(设计文档 §2)。
        // 切走前先把当前(老)项目的面板布局原样存下,再换成新项目的。
        self.stash_active_panel_layout();
        if focus_project_tab(&self.projects, &mut self.active_project_id, id) {
            self.adopt_panel_layout(id);
            self.maximized = None;
            self.current_page = AppPage::Workspace;
            self.ensure_loaded(id);
            // 清放大态改变了终端 pane 的像素尺寸,网格必须跟着重算:
            // `terminal_grid_state` 把 `maximized` 算进去,不重算的话
            // PTY 会一直停在放大时的 cols/rows,直到某个无关的几何事件
            // 偶然触发一次重算(最终审查 Required Fix #2)。
            self.sync_terminal_grid();
            self.persist_open_projects();
            return;
        }
        // 还没开着:作为**新页签**打开(与顶栏"＋"同一条 `ProjectTabOpened`
        // 落地路径),而不是把当前页签的内容换掉——多页签下"点一张最近
        // 项目卡片"的直觉是"再开一个",不是"把手上这个换掉"。
        let client = self.client.clone();
        let proxy = self.proxy.clone();
        self.handle.spawn(async move {
            let recent = client.list_projects().await.unwrap_or_default();
            let opened = recent.iter().find(|p| p.id == id).cloned();
            let _ = proxy.send_event(Message::ProjectTabOpened(opened, recent));
        });
    }

    fn project_tab_opened(&mut self, project: Option<ProjectInfo>, recent: Vec<ProjectInfo>) {
        self.recent_projects = recent.clone();
        // `None` = 这次打开失败(daemon 不通/回 `Reply::Error`)。硬性
        // 要求:失败绝不能落进任何 `Workspace`,否则会留下"有界面、没
        // 归属项目"的破状态,用户一点 tab 栏的"＋"就 panic
        // (`spawn_new_tab` 的 expect)。失败文案挂到 App 级的
        // `daemon_error` 上——它不依赖任何 `Workspace` 存在,一个项目
        // 都没打开时空态视图也画得出来(Required Fix #1)。
        let Some(project) = project else {
            tracing::warn!("打开项目页签失败,页签集合保持不变");
            self.daemon_error = Some("打开项目失败,请确认 dozerd 正常后重试".to_string());
            self.with_focused_project(move |ws, _io| {
                ws.recent_projects = recent;
            });
            return;
        };
        self.daemon_error = None;
        // 放大态是外壳态,换页签后留着只会挡住新页签的界面。
        self.maximized = None;
        let id = project.id;
        // 切走前先把当前(老)项目的面板布局原样存下,再换成新项目的。
        self.stash_active_panel_layout();
        if focus_project_tab(&self.projects, &mut self.active_project_id, id) {
            self.adopt_panel_layout(id);
            // 这个项目已经开着页签了:只前台化,绝不改写它的内容——
            // 那会把这个页签既有的终端全关掉、文件树对话列表全清空重来。
            self.current_page = AppPage::Workspace;
            self.ensure_loaded(id);
            self.with_focused_project(move |ws, _io| {
                ws.recent_projects = recent;
            });
            self.sync_terminal_grid(); // 清放大态后重算网格,理由见 `ProjectSelect`
            self.persist_open_projects();
            return;
        }
        let io = self.shell_io();
        let repo_path = PathBuf::from(&project.path);
        let mut ws = Workspace::empty_for_project_placeholder();
        ws.recent_projects = recent;
        ws.adopt_project(&io, project);
        self.projects
            .insert(id, WorkspaceSlot::Loaded(Box::new(ws)));
        self.project_order.push(id);
        self.active_project_id = Some(id);
        // 换成新项目的面板布局(它自己没存过就退化成默认)。
        self.adopt_panel_layout(id);
        self.current_page = AppPage::Workspace;
        self.sync_terminal_grid(); // 同上
        self.persist_open_projects();
        // 新开的项目 tab(区别于"已开着、只是前台化"那条 `focus_project_tab`
        // 早退分支):静默跑一次 ensure(README/.dozer/git/agent 历史),不
        // 展示结果(见 project_scaffold 设计"打开即 ensure"一节)。
        let client = self.client.clone();
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let emit = move |m| {
            let _ = proxy.send_event(Message::Project(m));
        };
        project::spawn_scaffold_run(repo_path, client, &handle, emit);
    }

    fn project_tab_switch(&mut self, id: i64) {
        // 切页签只有两件事:改 `active_project_id`、必要时促成 `Stub`。
        // 没有任何内容改写,因此后台项目的终端/预览/审阅原样留着,切
        // 回来还是刚才那副样子。
        // 切走前先把当前(老)项目的面板布局原样存下,再换成新项目的。
        self.stash_active_panel_layout();
        if !focus_project_tab(&self.projects, &mut self.active_project_id, id) {
            return;
        }
        self.adopt_panel_layout(id);
        self.maximized = None;
        self.current_page = AppPage::Workspace;
        self.ensure_loaded(id);
        // Git Log 面板已经开着的话,提交图缓存是 `App` 级的、不随项目
        // 页签走(见 `sync_git_log_to_active_project` 文档),不补这一
        // 下切页签会让提交图停在上一个项目,跟同一面板里已经按新项目
        // 刷新的 worktree 速览条对不上。
        if self.left_view == PanelKind::GitLog {
            self.sync_git_log_to_active_project();
        }
        // 清放大态后必须重算终端网格。`PaneResized` 那条分支只在**窗口
        // 几何变化**时触发,清 `maximized` 不会自己走到那里;而
        // `terminal_grid_state` 把 `maximized` 算进公式,不重算的话
        // "在项目 A 放大终端 → 切到 B"会让 A 的 PTY 停在放大时的
        // cols/rows(最终审查 Required Fix #2)。重算是幂等的:算出来
        // 与当前 `cols/rows` 相同时 `PaneResized` 的去重会原地返回。
        self.sync_terminal_grid();
        self.persist_open_projects();
        // 按下项目页签＝选中＋准备被拖走(同终端/预览页签)。
        if let Some(idx) = self.project_order.iter().position(|p| *p == id) {
            self.tab_drag = Some(TabDrag {
                group: TabGroup::Project,
                source: idx,
                press_pos: self.last_cursor,
            });
        }
    }

    fn project_tab_close(&mut self, id: i64) {
        // 关掉当前页签前先把它的面板布局原样存下(焦点还在它身上,
        // `stash` 会记进 `id` 那份),以后重开还能恢复。
        self.stash_active_panel_layout();
        let io = self.shell_io();
        let Some(slot) = take_project_tab(
            &mut self.projects,
            &mut self.project_order,
            &mut self.active_project_id,
            id,
        ) else {
            return;
        };
        if let WorkspaceSlot::Loaded(mut ws) = slot {
            // 关页签 = 结束该项目下所有会话(abort 转发任务 + kill
            // daemon 侧会话)。不 kill 的话会话会继续在 daemon 上跑,
            // 还会被下次 bootstrap 恢复出来。
            ws.close_all_tabs_for_switch(&io);
        }
        self.maximized = None;
        // 焦点被 `take_project_tab` 挪到了邻居页签上,而那个邻居可能还
        // 是个懒加载 `Stub`——`view()` 走的是只读的 `active_workspace()`,
        // 它**不促成** `Stub`,于是界面会画成"未打开任何项目",尽管顶栏
        // 那个页签明明高亮着。必须在这里显式促成(最终审查 Required
        // Fix #3)。
        if let Some(next) = self.active_project_id {
            // 焦点被挪到了邻居页签,把它的面板布局换上来。
            self.adopt_panel_layout(next);
            self.ensure_loaded(next);
        }
        self.sync_terminal_grid(); // 清放大态后重算网格,理由同 `ProjectTabSwitch`
        self.persist_open_projects();
    }

    /// "删除项目"确认弹窗的"删除"按钮触发,由 `Message::Project(project::
    /// Message::DeleteProjectConfirm)` 拦截调用(见该分支注释)。这个操作
    /// 一定作用在当前聚焦的项目上——删除按钮本来就在那个项目自己的面板
    /// 里,不存在"删除一个没打开的项目"这回事。
    fn project_delete_confirm(&mut self) {
        let Some(project_id) = self.active_project_id else {
            return;
        };
        let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
            return;
        };
        let Some(scope) = ws.project_panel.delete_pending.take() else {
            return;
        };
        let Some(project) = ws.project.as_ref() else {
            return;
        };
        let repo_path = PathBuf::from(&project.path);
        // 关 tab 必须在发起删除请求之前——删除一旦成功,这个项目在
        // dozerd/磁盘上都可能已经不存在了,`Workspace` 不该继续留着。
        self.project_tab_close(project_id);
        let client = self.client.clone();
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let on_done = move |errors: Vec<String>| {
            let _ = proxy.send_event(Message::ProjectDeleteDone(errors));
        };
        project::delete::spawn_delete_project(
            project_id, repo_path, scope, client, &handle, on_done,
        );
    }

    fn project_slot_loaded(&mut self, id: i64, payload: RestorePayload) {
        let Some(restore) = payload.take() else {
            return; // 信封已被取走(理论上不会发生),没有素材可落地
        };
        // 只在槽位仍是那份"加载中"占位时落地。两种落空情形:
        // - 页签在促成完成前被用户关掉了(槽位已不存在);
        // - 槽位已经被别的路径换成了真正的内容(比如
        //   `ProjectTabOpened` 的 `adopt_project`)。
        // 两种情形下这份素材都没人要了,但它已经 attach 上了该项目在
        // daemon 上的存活会话——直接 drop 只是断开事件流,daemon 侧
        // 会话仍在跑,会变成"没有任何页签持有、却还占着 PTY"的野会话。
        // 所以按关页签的语义结束掉它们(`ProjectTabClose` 同款处理)。
        // 落地的同时把占位那份 `allowed_files` 句柄接过来:main.rs 的
        // webview 池只在 `active_project_id` **变化**时才清空,它看不见
        // "同一个项目换了一份 `Workspace` 对象"。促成窗口期里用户点开
        // 的文件预览已经建出一个 id 0 的 webview,其 `dozer://` 协议
        // 闭包捕获的是**占位那一个** `Arc`;新 `Workspace` 若另起一个
        // `Arc`,`restore_preview_state` 重开的 id 0 会被
        // `sync_webview_pool` 认成"这个 id 已经有 webview 了"而只调
        // `load_url`,于是文件请求走的还是旧 `Arc` 的白名单 → 对不上
        // → 空白预览。这与 Required Fix #3 是同一个失效模式,只是触发
        // 点从"切项目"变成"促成换对象"。共用同一个 `Arc` 即可,而且
        // 不损失已经建好的 webview(比清空池更省一次导航)。
        let inherited = match self.projects.get(&id) {
            Some(WorkspaceSlot::Loaded(cur)) if cur.loading => Some(cur.allowed_files()),
            _ => None,
        };
        let landed = inherited.is_some();
        if !landed {
            let client = self.client.clone();
            let ids: Vec<String> = restore
                .sessions
                .iter()
                .map(|(info, _, _)| info.id.clone())
                .collect();
            let task = self.handle.spawn(async move {
                for sid in ids {
                    if let Err(e) = client.kill(&sid).await {
                        tracing::warn!("丢弃过期促成结果时结束会话失败: {e}");
                    }
                }
            });
            if let Ok(mut pending) = self.pending_exit_tasks.lock() {
                pending.push(task);
            }
            return;
        }
        let io = self.shell_io();
        let mut ws = Workspace::from_restore(&io, *restore, inherited);
        // 重挂出来的会话,终端模型是按 `DEFAULT_COLS`×`DEFAULT_ROWS`
        // 建的,得按当前窗口几何纠正一次。这里**不能**指望
        // `sync_terminal_grid`:它算出来的网格与 `self.cols/rows` 相同
        // 时 `PaneResized` 会原地返回(去重),于是这份新装配的
        // `Workspace` 会一直停在 80×24。直接对它自己 resize 一次——
        // 共享与 SSH 两个 pane 各按自己跟踪的网格分别纠正。
        ws.resize_all(&io, io.cols, io.rows, self.ssh_cols, self.ssh_rows);
        self.projects
            .insert(id, WorkspaceSlot::Loaded(Box::new(ws)));
    }

    fn project_fs_changed(&mut self, project_id: ProjectId, changes: git_watch::FsChanges) {
        // 工作区类变更:文件树 + 打开的 webview 预览即时跟进。`notify` 递上
        // 的是具体变更路径 `changes.paths`,文件树按"受影响即相关"整棵从盘重
        // 读已缓存目录(`reload_tree_from_disk`,只重读已展开/缓存过的层,开销
        // 小),预览则只重载路径命中的 webview tab。
        if changes.relevance == Some(git_watch::Relevance::Workdir)
            || changes.relevance == Some(git_watch::Relevance::GitRefs)
        {
            self.with_project(project_id, |ws, _io| {
                ws.files.reload_tree_from_disk();
                ws.preview.reload_webviews_for(&changes.paths);
                ws.project_preview.reload_webviews_for(&changes.paths);
            });
        }
        self.with_project(project_id, |ws, io| {
            let Some(project) = &ws.project else { return };
            let repo_path = PathBuf::from(&project.path);
            spawn_project_git_refresh(project_id, repo_path.clone(), io);
            spawn_disk_usage_refresh(project_id, repo_path, io);
        });
        // 只有 `.git` 引用类变化(分支切换/外部提交/其他 worktree
        // 提交)才值得重建 Git Log 快照——纯工作区文件编辑不影响
        // 提交历史,重算是纯浪费。`git_log_cache` 是 `App` 级、不是
        // 按项目分的(见 `sync_git_log_to_active_project`),所以这里
        // 必须先核实这条事件本来就是"当前聚焦项目"发出的
        // (`project_id == self.active_project_id`)——否则后台项目
        // 的引用变化会拿"缓存路径恰好等于前台项目路径"这个巧合当
        // 通行证,把前台正打开的详情/选中态平白清掉,而其实什么都
        // 没变。项目 id 匹配之外再核一次路径,双保险防状态漂移。
        if changes.relevance == Some(git_watch::Relevance::GitRefs)
            && self.active_project_id == Some(project_id)
            && let Some(repo_path) = self.git_log.cache_repo_path().map(|p| p.to_path_buf())
            && self
                .active_workspace()
                .and_then(|ws| ws.active_project_path())
                .as_deref()
                == Some(repo_path.as_path())
        {
            // 引用变化只是要"内容不变、重新拉一遍",窗口大小维持原样——
            // 用 `cache_max_count()`(读当前缓存的 max_count),不是"加载
            // 更多"专用、会 `+LOAD_MORE_STEP` 的 `next_load_more_count()`。
            let max = self.git_log.cache_max_count();
            let handle = self.handle.clone();
            let proxy = self.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::GitLog(m));
            };
            git_log::request_refresh(&mut self.git_log, repo_path, max, &handle, emit);
        }
    }

    fn database_test_connection_result(
        &mut self,
        project_id: i64,
        source_id: String,
        result: Result<(), String>,
    ) {
        let app_db = &mut self.database;
        let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
            return;
        };
        let Some(project) = ws.project.as_ref() else {
            return;
        };
        let repo_path = std::path::PathBuf::from(&project.path);
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let emit = move |m| {
            let _ = proxy.send_event(Message::Database(m));
        };
        database::update(
            &mut ws.database,
            app_db,
            database::Message::TestConnectionResult(project_id, source_id, result),
            project_id,
            &repo_path,
            &handle,
            emit,
        );
    }

    fn database_browse_result(
        &mut self,
        project_id: i64,
        tab_id: usize,
        run_seq: u64,
        result: Result<database::BrowsePage, String>,
    ) {
        let app_db = &mut self.database;
        let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
            return;
        };
        let Some(project) = ws.project.as_ref() else {
            return;
        };
        let repo_path = std::path::PathBuf::from(&project.path);
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let emit = move |m| {
            let _ = proxy.send_event(Message::Database(m));
        };
        database::update(
            &mut ws.database,
            app_db,
            database::Message::BrowseResult(project_id, tab_id, run_seq, result),
            project_id,
            &repo_path,
            &handle,
            emit,
        );
    }

    fn database_query_result(
        &mut self,
        project_id: i64,
        tab_id: usize,
        run_seq: u64,
        result: Result<database::QueryOutcome, String>,
    ) {
        let app_db = &mut self.database;
        let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
            return;
        };
        let Some(project) = ws.project.as_ref() else {
            return;
        };
        let repo_path = std::path::PathBuf::from(&project.path);
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let emit = move |m| {
            let _ = proxy.send_event(Message::Database(m));
        };
        database::update(
            &mut ws.database,
            app_db,
            database::Message::QueryResult(project_id, tab_id, run_seq, result),
            project_id,
            &repo_path,
            &handle,
            emit,
        );
    }

    fn database_tables_loaded(
        &mut self,
        project_id: i64,
        source_id: String,
        result: Result<Vec<database::TableRef>, String>,
    ) {
        let app_db = &mut self.database;
        let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
            return;
        };
        let Some(project) = ws.project.as_ref() else {
            return;
        };
        let repo_path = std::path::PathBuf::from(&project.path);
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let emit = move |m| {
            let _ = proxy.send_event(Message::Database(m));
        };
        database::update(
            &mut ws.database,
            app_db,
            database::Message::TablesLoaded(project_id, source_id, result),
            project_id,
            &repo_path,
            &handle,
            emit,
        );
    }

    fn database_columns_loaded(
        &mut self,
        project_id: i64,
        source_id: String,
        schema: Option<String>,
        table: String,
        result: Result<Vec<database::ColumnInfo>, String>,
    ) {
        let app_db = &mut self.database;
        let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
            return;
        };
        let Some(project) = ws.project.as_ref() else {
            return;
        };
        let repo_path = std::path::PathBuf::from(&project.path);
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let emit = move |m| {
            let _ = proxy.send_event(Message::Database(m));
        };
        database::update(
            &mut ws.database,
            app_db,
            database::Message::ColumnsLoaded {
                project_id,
                source_id,
                schema,
                table,
                result,
            },
            project_id,
            &repo_path,
            &handle,
            emit,
        );
    }

    fn database_message(&mut self, msg: database::Message) {
        // 四个动作都可能来自数据源树 header 行的右键菜单——菜单本体没有
        // "点了就自动收起"的行为(popup 盖在 dismiss 遮罩之上,点菜单项本身
        // 吃不到遮罩的点击),落地时顺手收掉,不来自菜单时该字段本就是
        // `None`,无副作用。
        if matches!(
            msg,
            database::Message::TestConnection(_)
                | database::Message::EditSourceStart(_)
                | database::Message::DeleteSourceRequest(_)
                | database::Message::SchemaRefresh(_)
        ) {
            self.database_source_menu = None;
        }
        let Some(project_id) = self.active_project_id else {
            return;
        };
        let app_db = &mut self.database;
        let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
            return;
        };
        let Some(project) = ws.project.as_ref() else {
            return;
        };
        let repo_path = std::path::PathBuf::from(&project.path);
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let emit = move |m| {
            let _ = proxy.send_event(Message::Database(m));
        };
        database::update(
            &mut ws.database,
            app_db,
            msg,
            project_id,
            &repo_path,
            &handle,
            emit,
        );
    }

    fn ssh_test_connection_result(
        &mut self,
        project_id: i64,
        host_id: String,
        result: Result<(), String>,
    ) {
        self.with_project(project_id, move |ws, io| {
            let handle = io.handle.clone();
            let proxy = io.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::Ssh(m));
            };
            let repo_path = ws
                .project
                .as_ref()
                .map(|p| PathBuf::from(&p.path))
                .unwrap_or_default();
            ssh::update(
                &mut ws.ssh,
                ssh::Message::TestConnectionResult(project_id, host_id, result),
                project_id,
                &repo_path,
                &handle,
                emit,
            );
        });
    }

    fn ssh_unknown_key_detected(
        &mut self,
        project_id: i64,
        host_id: String,
        fingerprint: String,
        key_bytes: Vec<u8>,
    ) {
        self.with_project(project_id, move |ws, io| {
            let handle = io.handle.clone();
            let proxy = io.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::Ssh(m));
            };
            let repo_path = ws
                .project
                .as_ref()
                .map(|p| PathBuf::from(&p.path))
                .unwrap_or_default();
            ssh::update(
                &mut ws.ssh,
                ssh::Message::UnknownKeyDetected(project_id, host_id, fingerprint, key_bytes),
                project_id,
                &repo_path,
                &handle,
                emit,
            );
        });
    }

    fn ssh_key_changed(&mut self, project_id: i64, host_id: String, fingerprint: String) {
        self.with_project(project_id, move |ws, io| {
            let handle = io.handle.clone();
            let proxy = io.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::Ssh(m));
            };
            let repo_path = ws
                .project
                .as_ref()
                .map(|p| PathBuf::from(&p.path))
                .unwrap_or_default();
            ssh::update(
                &mut ws.ssh,
                ssh::Message::KeyChanged(project_id, host_id, fingerprint),
                project_id,
                &repo_path,
                &handle,
                emit,
            );
        });
    }

    fn ssh_terminal_connect_failed(
        &mut self,
        project_id: i64,
        host_id: String,
        tab_id: usize,
        err: String,
    ) {
        self.with_project(project_id, move |ws, io| {
            ws.pending.remove(&tab_id);
            ws.ssh_out_pending.remove(&tab_id);
            let handle = io.handle.clone();
            let proxy = io.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::Ssh(m));
            };
            let repo_path = ws
                .project
                .as_ref()
                .map(|p| PathBuf::from(&p.path))
                .unwrap_or_default();
            ssh::update(
                &mut ws.ssh,
                ssh::Message::TerminalConnectFailed(project_id, host_id, tab_id, err),
                project_id,
                &repo_path,
                &handle,
                emit,
            );
        });
    }

    /// 指派任务给某个 agent 种类,纯记录,不触发任何执行。异步确认经
    /// `Mutated` 刷新列表——与其它写操作同一条乐观更新链路。
    fn todo_assign_agent(&mut self, idx: usize, agent: dozer_core::protocol::AgentKind) {
        let Some(id) = self
            .active_workspace()
            .and_then(|ws| ws.todo.items().get(idx))
            .map(|item| item.id)
        else {
            return;
        };
        self.with_focused_project(|ws, _io| {
            ws.todo.close_dispatch_popup();
        });
        let handle = self.handle.clone();
        let client = self.client.clone();
        let proxy = self.proxy.clone();
        handle.spawn(async move {
            let res = client
                .assign_todo_agent(id, agent)
                .await
                .map(|_| ())
                .map_err(|e| e.to_string());
            let _ = proxy.send_event(Message::Todo(todo::Message::Mutated(res)));
        });
    }

    /// 打开任务详情弹窗:先本地记下 `idx`(弹窗定位/后续"处理"要用),再
    /// 异步拉 `GetTodoDetail`。RPC 结果经专门的 `Message::TodoDetailLoaded`
    /// 落地——`todo::Message::Mutated` 那条通用刷新链路只刷 `items`/
    /// `categories`,不携带回合数据,不能复用。
    fn todo_detail_open(&mut self, idx: usize) {
        let Some(id) = self
            .active_workspace()
            .and_then(|ws| ws.todo.items().get(idx))
            .map(|item| item.id)
        else {
            return;
        };
        self.with_focused_project(|ws, _io| {
            ws.todo.open_detail(idx, Vec::new());
        });
        let handle = self.handle.clone();
        let client = self.client.clone();
        let proxy = self.proxy.clone();
        handle.spawn(async move {
            if let Ok((_, turns)) = client.get_todo_detail(id).await {
                let _ = proxy.send_event(Message::TodoDetailLoaded(idx, turns));
            }
        });
    }

    /// 详情弹窗"处理"按钮:乐观插入已经在 `todo::update`(`DetailReplySubmit`
    /// 分支)做过,这里只管发 `ProcessTodoNow` RPC 并在结果回来后用服务端
    /// 权威回合列表刷新。耗时可能到 10 分钟,走 `handle.spawn` 不阻塞 UI。
    fn todo_detail_process(&mut self) {
        let Some((idx, id, reply_text)) = self.active_workspace().and_then(|ws| {
            let idx = ws.todo.detail_open_idx()?;
            let id = ws.todo.items().get(idx)?.id;
            Some((idx, id, ws.todo.last_reply_text()))
        }) else {
            return;
        };
        let handle = self.handle.clone();
        let client = self.client.clone();
        let proxy = self.proxy.clone();
        handle.spawn(async move {
            let _ = client.process_todo_now(id, reply_text.as_deref()).await;
            if let Ok((_, turns)) = client.get_todo_detail(id).await {
                let _ = proxy.send_event(Message::TodoDetailLoaded(idx, turns));
            }
        });
    }

    fn todo_message(&mut self, msg: todo::Message) {
        // 新增任务框高度拖拽:只在 app 层接管,置 `dragging_row`,后续
        // `CursorMoved` → `RowDrag` 由 `update` 统一换算高度写回
        // `ws.todo`(见 `RowDrag` 的 `TodoAddGrow` 分支)。这条不到
        // `todo::update`(那里有 no-op arm 保持 match 穷尽)。
        if let todo::Message::AddResizeStart = msg {
            self.dragging_row = Some(RowDivider::TodoAddGrow);
            return;
        }
        let Some(project_id) = self.active_project_id else {
            return;
        };
        let client = self.client.clone();
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let last_cursor = self.last_cursor;
        // 异步结果/写确认透过 `proxy` 重发回主循环,回调里会借用 `self`
        // 的 client/handle ——闭包捕获是 move 出来的副本,行得通(同
        // `Message::Search` 分支的既有手法)。
        let emit = move |m: todo::Message| {
            let _ = proxy.send_event(Message::Todo(m));
        };
        self.with_focused_project(move |ws, _io| {
            // 点日历按钮时的光标逻辑坐标,作为窗口级 overlay 的弹出锚点——
            // 先记下再交给 `todo::update` 展开(它只管 `calendar_open`/`calendar_view`)。
            if matches!(msg, todo::Message::CalendarOpen(_)) {
                ws.todo.set_calendar_anchor(last_cursor);
            }
            // 点"指派"按钮时的光标逻辑坐标,作为派发选择层 overlay 的弹出锚点。
            if matches!(msg, todo::Message::DispatchOpen(_)) {
                ws.todo.set_dispatch_anchor(last_cursor);
            }
            // 点卡片左下"状态"按钮的光标逻辑坐标,作为状态下拉选择层 overlay
            // 的弹出锚点。`StatusOpen` 自身交给 `todo::update` 展开(它只改
            // `status_open`)。
            if matches!(msg, todo::Message::StatusOpen(_)) {
                ws.todo.set_status_anchor(last_cursor);
            }
            // 点搜索框左前"状态"segment 按钮时的光标逻辑坐标,作为搜索框状态
            // 筛选浮层 overlay 的弹出锚点。`StatusFilterOpen` 自身交给
            // `todo::update` 展开(它只改 `status_filter_open`)。
            if matches!(msg, todo::Message::StatusFilterOpen) {
                ws.todo.set_status_filter_anchor(last_cursor);
            }
            todo::update(&mut ws.todo, msg, project_id, &client, &handle, emit);
        });
    }

    fn browser_bookmarks_loaded(&mut self, pid: Option<i64>, bookmarks: Vec<BookmarkInfo>) {
        if pid.is_none() {
            // 首页全局浏览器(或无项目工作区)的收藏列表刷新。
            let handle = self.handle.clone();
            let client = self.client.clone();
            let proxy = self.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::HomeBrowser(m));
            };
            browser::update(
                &mut self.home_browser,
                browser::Message::BookmarksLoaded(None, bookmarks),
                None,
                &client,
                &handle,
                emit,
            );
            return;
        }
        self.with_project(pid.unwrap(), move |ws, io| {
            let pid = pid.unwrap();
            let handle = io.handle.clone();
            let client = io.client.clone();
            let proxy = io.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::Browser(m));
            };
            browser::update(
                &mut ws.browser,
                browser::Message::BookmarksLoaded(Some(pid), bookmarks),
                Some(pid),
                &client,
                &handle,
                emit,
            );
        });
    }

    fn browser_bookmarks_mutated(&mut self, pid: Option<i64>, res: Result<(), String>) {
        if pid.is_none() {
            // 首页全局浏览器(或无项目工作区)的添加/删除结果回调。
            let handle = self.handle.clone();
            let client = self.client.clone();
            let proxy = self.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::HomeBrowser(m));
            };
            browser::update(
                &mut self.home_browser,
                browser::Message::BookmarksMutated(None, res),
                None,
                &client,
                &handle,
                emit,
            );
            return;
        }
        self.with_project(pid.unwrap(), move |ws, io| {
            let pid = pid.unwrap();
            let handle = io.handle.clone();
            let client = io.client.clone();
            let proxy = io.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::Browser(m));
            };
            browser::update(
                &mut ws.browser,
                browser::Message::BookmarksMutated(Some(pid), res),
                Some(pid),
                &client,
                &handle,
                emit,
            );
        });
    }

    fn browser_message(&mut self, msg: browser::Message) {
        // 按下浏览器页签＝选中＋准备被拖走(`SelectTab` 在
        // `browser::update` 里真正选中为 `active`,这里按它记下拖起源)。
        let was_select = matches!(msg, browser::Message::SelectTab(_));
        self.with_focused_project(|ws, io| {
            let project_id = ws.project.as_ref().map(|p| p.id);
            let client = io.client.clone();
            let handle = io.handle.clone();
            let proxy = io.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::Browser(m));
            };
            browser::update(&mut ws.browser, msg, project_id, &client, &handle, emit);
        });
        if was_select
            && let Some(ws) = self.active_workspace()
            && ws.browser.active_tab_idx() < ws.browser.tab_count()
        {
            let active = ws.browser.active_tab_idx();
            self.tab_drag = Some(TabDrag {
                group: TabGroup::Browser,
                source: active,
                press_pos: self.last_cursor,
            });
        }
    }

    fn term_input(&mut self, target: terminal::TermTarget, bytes: Vec<u8>) {
        // 终端不在屏上时丢弃按键(不报错、不写 PTY):否则用户在读
        // 对话审阅时敲的回车/方向键会静默提交给隐藏在后面的 agent
        // 会话(Fix round 2 #3)。
        let visible = match target {
            terminal::TermTarget::Shared => self.terminal_visible(),
            terminal::TermTarget::SshPanel => self.ssh_terminal_visible(), // Task 12 新增
        };
        if !visible {
            return;
        }
        self.with_focused_project(|ws, io| {
            match target {
                terminal::TermTarget::Shared => {
                    // 键入即回底 + 清选区：正在回看历史时一敲键盘，视口跳回
                    // 实时输出（常规终端语义），再把字节写给 daemon。
                    if let Some(tab) = ws.tabs.get_mut(ws.active) {
                        tab.model.scroll_to_bottom();
                        tab.model.selection_clear();
                    }
                    ws.send_input(io, bytes);
                }
                terminal::TermTarget::SshPanel => {
                    if let Some(tab) = ws.ssh_active_tab_mut() {
                        tab.model.scroll_to_bottom();
                        tab.model.selection_clear();
                    }
                    ws.ssh_send_input(io, bytes);
                }
            }
        });
    }

    fn term_output(&mut self, project_id: ProjectId, tab_id: usize, bytes: Vec<u8>) {
        self.with_project(project_id, |ws, io| {
            let Some(tab) = ws.tab_by_id_mut(tab_id) else {
                return;
            };
            // 实时输出可能含设备查询（DSR/DA 等），应答必须写回 PTY
            // ——atuin/claude 等 TUI 依赖它（此前丢弃导致探测超时）。
            tab.ingest_osc(&bytes);
            let responses = tab.model.feed(&bytes);
            if responses.is_empty() || !tab.alive {
                return;
            }
            match &tab.backend {
                TabBackend::Daemon => {
                    let client = io.client.clone();
                    let id = tab.info.id.clone();
                    io.handle.spawn(async move {
                        if let Err(e) = client.write(&id, &responses).await {
                            tracing::warn!("回写终端查询应答失败: {e}");
                        }
                    });
                }
                TabBackend::Ssh { out } => {
                    let _ = out.send(SshOut::Data(responses));
                }
            }
        });
    }

    fn term_paste(&mut self, target: terminal::TermTarget, text: String) {
        // 同 TermInput 的可见性闸门(Fix round 3):⌘V 粘贴走同一条
        // PTY 写入路径,粘贴内容若含换行还会在看不见的会话里直接
        // 执行,比单个按键更危险,必须同样拦截。
        let visible = match target {
            terminal::TermTarget::Shared => self.terminal_visible(),
            terminal::TermTarget::SshPanel => self.ssh_terminal_visible(),
        };
        if !visible {
            return;
        }
        self.with_focused_project(move |ws, io| {
            let bracketed = match target {
                terminal::TermTarget::Shared => {
                    ws.tabs.get(ws.active).map(|t| t.model.bracketed_paste())
                }
                terminal::TermTarget::SshPanel => ws
                    .ssh_tabs
                    .iter()
                    .find(|t| {
                        ws.ssh_active.as_ref().is_some_and(|(h, _)| {
                            t.info.id.strip_prefix("ssh:") == Some(h.as_str())
                        })
                    })
                    .map(|t| t.model.bracketed_paste()),
            };
            let Some(bracketed) = bracketed else {
                return;
            };
            match target {
                terminal::TermTarget::Shared => {
                    if let Some(tab) = ws.tabs.get_mut(ws.active) {
                        tab.model.scroll_to_bottom();
                    }
                }
                terminal::TermTarget::SshPanel => {
                    if let Some(tab) = ws.ssh_active_tab_mut() {
                        tab.model.scroll_to_bottom();
                    }
                }
            }
            let bytes = if bracketed {
                let mut b = b"\x1b[200~".to_vec();
                b.extend_from_slice(text.as_bytes());
                b.extend_from_slice(b"\x1b[201~");
                b
            } else {
                text.into_bytes()
            };
            match target {
                terminal::TermTarget::Shared => ws.send_input(io, bytes),
                terminal::TermTarget::SshPanel => ws.ssh_send_input(io, bytes),
            }
        });
    }

    fn select_tab(&mut self, idx: usize) {
        // 按下页签＝选中＋准备被拖走:选中仍是唯一的语义,但顺带记下
        // "这一页签正被按住",随后鼠标划过其它页签时 `on_move` 触发
        // `TabDragMove` 完成换位;松开时 main.rs `TabDragEnd` 收尾。
        // 只应该被 tab 栏本身的按钮调用——见 `Message::SelectTab` 文档。
        self.select_tab_no_drag(idx);
        if let Some(ws) = self.active_workspace()
            && idx < ws.tabs.len()
        {
            self.tab_drag = Some(TabDrag {
                group: TabGroup::Terminal,
                source: idx,
                press_pos: self.last_cursor,
            });
        }
    }

    /// `select_tab` 去掉"武装拖拽状态机"那部分,给非 tab 栏的调用方
    /// (Agent 面板右侧卡片列表)用——见 `Message::SelectTabNoDrag` 文档。
    fn select_tab_no_drag(&mut self, idx: usize) {
        // `with_focused_project` 的闭包里借的是 `ws`,拿不到 `self`——真实
        // 可用宽度得在借用开始前算好(同 `preview_select_tab` 那批调用方的
        // 既有先例)。
        let avail_px = self.terminal_tab_bar_avail_px();
        self.with_focused_project(move |ws, _io| {
            if idx < ws.tabs.len() {
                ws.active = idx;
                // 从 tab 栏的 V 下拉里选中某一项:选中后把主条滚入可见窗口
                // (若该项仍横向可见则不受影响,见 `tab_window_reveal`),并收起
                // 下拉——避免"选中了却看不见在哪"。下拉列的是组内全部 tab,
                // 高亮常驻在可见宽度内时不会多跳一行。
                let widths: Vec<f32> = ws
                    .tabs
                    .iter()
                    .map(|t| tab_display_width(&tab_title(t.agent, t.cwd.as_deref(), &t.info.name)))
                    .collect();
                ws.term_tab_first =
                    tab_widget::tab_window_reveal(&widths, 4.0, avail_px, ws.term_tab_first, idx);
                ws.term_tab_overflow_anchor = None;
            }
        });
        // `term_ime_preedit` 是 `App` 上唯一一份、不按 tab 分的组字预览态
        // (见该字段文档),只在真正 `Ime::Commit`/组字取消时才清空——切
        // tab 不会清。`TermTarget::Shared` 这道"该不该显示"闸门只判断
        // "键盘现在归不归共享终端条",不区分具体哪个 tab,于是新切过去的
        // tab 会直接"继承"上一个 tab 还没提交完的组字预览文字/候选词
        // (2026-08-21 用户实测反馈:切 tab 后串台,选完字对方也不会真的
        // 收到那些字——因为提交字节确实是发给切换后的新 tab 的 PTY,只是
        // 预览视觉是借来的)。切 tab 时无条件清掉(即使 idx 越界导致上面
        // 没真的切,清掉一份陈旧组字预览也没有副作用),避免这份陈旧状态
        // 被新激活的 tab 误当成自己的组字预览渲染出来。
        self.term_ime_preedit = None;
    }

    fn pane_resized(&mut self, cols: u16, rows: u16, ssh_cols: u16, ssh_rows: u16) {
        if cols == 0 || rows == 0 {
            return;
        }
        // 共享与 SSH 两个网格各自带独立去重:任一真变了都要往 dev 文件里
        // propagate,不能因为共享网格没动就跳掉 SSH 网格的同步。
        let shared_changed = (cols, rows) != (self.cols, self.rows);
        let ssh_changed = (ssh_cols, ssh_rows) != (self.ssh_cols, self.ssh_rows);
        if !shared_changed && !ssh_changed {
            return;
        }
        self.cols = cols;
        self.rows = rows;
        self.ssh_cols = ssh_cols;
        self.ssh_rows = ssh_rows;
        let io = self.shell_io();
        // 终端网格是窗口级的:并行打开的每个项目各有一套终端 tab,但它们
        // 共用同一批 pane。只改当前项目的话,切回后台项目会看到一个停在
        // 旧网格、和 pane 对不上的画面,直到用户偶然再拖一次窗口才纠正——
        // 所以这里对所有已加载项目一起改(`Stub` 还没有任何 tab,促成时
        // 自然按当时的 `io.cols/rows`)。共享/SSH 各自带独立网格下发。
        for slot in self.projects.values_mut() {
            if let WorkspaceSlot::Loaded(ws) = slot {
                ws.resize_all(&io, cols, rows, ssh_cols, ssh_rows);
            }
        }
    }

    fn panel_select(&mut self, kind: PanelKind) {
        let side = self.shell_layout.rail_layout.side_of(kind);
        // 武装拖拽态:按住图标＝准备拖(同 `TabDrag` 的"按下即武装"手法)。
        // 同栏重排 / 跨栏移动都是靠渲染层挂在图标上的 `on_move` 驱动
        // (`Message::RailDragMove`),`MouseMotion` 期间逐帧上报；这里只记下
        // "从哪栏的哪个位置开始拖"。`RailLayout` 的不变式(sanitize 已保证
        // 10 个面板不重不漏分到两栏)确保 `kind` 一定能在 `side_of` 返回的
        // 那一栏里被 `position` 找到。
        let source_index = self
            .shell_layout
            .rail_layout
            .side(side)
            .iter()
            .position(|&k| k == kind)
            .expect("kind 应该在 side_of 返回的那一侧里,sanitize 已保证不变式");
        self.rail_drag = Some(rail::RailDrag {
            source_side: side,
            source_index,
            origin_index: source_index,
            pending_cross_side: None,
            press_pos: self.last_cursor,
        });
        // 点当前已激活的图标:退回未选中并收起对应面板区;但若对侧面板区
        // 也已收起,当前侧就是最后一个还开着的 zone,不能关(两侧对称)。
        let switched = match side {
            Side::Left => {
                if self.left_view == kind {
                    if !self.right_collapsed {
                        self.left_collapsed = !self.left_collapsed;
                    }
                    false
                } else {
                    self.left_view = kind;
                    self.left_collapsed = false;
                    true
                }
            }
            Side::Right => {
                if self.right_view == kind {
                    if !self.left_collapsed {
                        self.right_collapsed = !self.right_collapsed;
                    }
                    false
                } else {
                    self.right_view = kind;
                    self.right_collapsed = false;
                    true
                }
            }
        };
        // 面板专属的"切入时动作"。原左栏处理器把 GitLog/Todo/
        // Database/Project/Ssh 的触发放在 if/else 之后的无条件
        // `if self.left_view == PanelKind::X` 里——收起/展开当前激活的特殊
        // 面板也会跑一遍;原右栏处理器把 Usage 放在 else(真正
        // 切换)分支里——只有切换时才触发。为保持逐像素零差异,左侧面板恒
        // 触发、右侧面板仅在真正切换时触发(默认布局下它们恰好按这个分侧;
        // Stage 4 拖拽换栏后这里再按 `rail_layout.side_of` 重新对齐各面板
        // 的触发语义)。
        let fire = match side {
            Side::Left => true,
            Side::Right => switched,
        };
        if fire {
            match kind {
                PanelKind::GitLog => self.sync_git_log_to_active_project(),
                PanelKind::Todo => {
                    if let Some(project_id) = self.active_project_id {
                        let client = self.client.clone();
                        let handle = self.handle.clone();
                        let proxy = self.proxy.clone();
                        let emit = move |m: todo::Message| {
                            let _ = proxy.send_event(Message::Todo(m));
                        };
                        let emit_todos = emit.clone();
                        todo::request_todos_refresh(project_id, &client, &handle, emit_todos);
                        todo::request_categories_refresh(project_id, &client, &handle, emit);
                    }
                }
                PanelKind::Database => self.with_focused_project(|ws, _io| {
                    if let Some(project) = ws.project.as_ref() {
                        database::reload_from_disk(
                            &mut ws.database,
                            std::path::Path::new(&project.path),
                        );
                    }
                }),
                PanelKind::Project => self.ensure_project_readme_and_reveal(),
                PanelKind::Ssh => self.with_focused_project(|ws, _io| {
                    if let Some(project) = ws.project.as_ref() {
                        ssh::reload_from_disk(&mut ws.ssh, std::path::Path::new(&project.path));
                    }
                }),
                PanelKind::Usage => self.with_focused_project(|ws, io| {
                    ws.usage.set_loading(true);
                    ws.spawn_usage_refresh(io);
                }),
                // 会话列表原本只在项目打开时和回合结束时刷新,切进这个面板时
                // 没有任何补救手段——离开一段时间再切回来看到的还是上次的
                // 快照。补一次切入即刷新,同 `Usage` 面板的既有口径。
                PanelKind::Conversations => self.with_focused_project(|ws, io| {
                    ws.spawn_conversations_refresh(io);
                }),
                PanelKind::Files | PanelKind::Web | PanelKind::Agent => {}
            }
        }
        // 图标栏点击一律退出放大态。放大态浮层不拦图标栏上的点击
        // (遮罩两侧垫的是无交互 Space,点击穿到下层图标按钮),所以
        // "放大左侧 → 点文件夹图标收起左侧"是可达的:不清 `maximized`
        // 就会留下一个空的金色描边浮层,只能点变暗区才能脱身
        // (Fix round 2 #2)。切换本侧显示什么内容时,放大态本也不该
        // 存活,无条件清最简单也最不容易出意外。
        self.maximized = None;
        self.on_shell_layout_changed();
    }

    /// 文件预览右上角按钮:翻转文件树列表子栏的展开/收起。只改一个布尔
    /// (`dims.files_tree_collapsed`),不动 `files_split` 比例(展开时按原比例
    /// 恢复)。收起态下文件树列表不渲染、预览拿满整个配对宽度。与
    /// `panel_select` 一样退出放大态并落盘/重算网格。
    fn toggle_files_tree_collapse(&mut self) {
        self.dims.files_tree_collapsed = !self.dims.files_tree_collapsed;
        self.maximized = None;
        self.on_shell_layout_changed();
    }

    /// 该面板当前是否偏离了默认栏——8 个有内部两栏布局的面板据此决定
    /// 渲染顺序要不要反转。这个 Stage 结束时 `RailLayout` 只可能是
    /// `default()`,所以这个函数在正常运行时恒返回 `false`;它的分支
    /// 靠单元测试直接构造非默认 `RailLayout` 来触发验证,不依赖 GUI
    /// 能不能拖拽出这个状态(Stage 4 才有拖拽)。
    pub(crate) fn panel_mirrored(&self, kind: PanelKind) -> bool {
        rail::panel_mirrored_in(&self.shell_layout.rail_layout, kind)
    }

    /// 文件树列表子栏当前是否被收起(文件预览右上角按钮切换)。暴露只读
    /// 的 `dims.files_tree_collapsed` 给 workspace 层渲染收起按钮时用,
    /// `dims` 字段本身保持模块私有。
    pub(crate) fn files_tree_collapsed(&self) -> bool {
        self.dims.files_tree_collapsed
    }

    /// 七个两栏面板(Project/Todo/Database/Ssh/Agent/Conversations/Usage)的
    /// 列表列当前是否被收起。语义同 `files_tree_collapsed`:列表不渲染、内容
    /// 拿满配对宽度,split 比例保留(展开时按原宽度恢复)。
    pub(crate) fn list_collapsed(&self, kind: PanelKind) -> bool {
        match kind {
            PanelKind::Project => self.dims.project_list_collapsed,
            PanelKind::Todo => self.dims.todo_list_collapsed,
            PanelKind::Database => self.dims.database_list_collapsed,
            PanelKind::Ssh => self.dims.ssh_list_collapsed,
            PanelKind::Agent => self.dims.agent_list_collapsed,
            PanelKind::Conversations => self.dims.conversations_list_collapsed,
            PanelKind::Usage => self.dims.usage_list_collapsed,
            _ => false,
        }
    }

    /// 翻转某两栏面板列表列的展开/收起(`Message::TogglePanelListCollapse` 的
    /// 处理)。只改对应布尔、不动 split 比例,并像 `panel_select` 一样退出
    /// 放大态 + 落盘/重算网格。非两栏面板(`Files` 走独立的
    /// `files_tree_collapsed`,其余单/两栏面板无此能力)直接忽略。
    fn toggle_panel_list_collapse(&mut self, kind: PanelKind) {
        let flag = match kind {
            PanelKind::Project => &mut self.dims.project_list_collapsed,
            PanelKind::Todo => &mut self.dims.todo_list_collapsed,
            PanelKind::Database => &mut self.dims.database_list_collapsed,
            PanelKind::Ssh => &mut self.dims.ssh_list_collapsed,
            PanelKind::Agent => &mut self.dims.agent_list_collapsed,
            PanelKind::Conversations => &mut self.dims.conversations_list_collapsed,
            PanelKind::Usage => &mut self.dims.usage_list_collapsed,
            _ => return,
        };
        *flag = !*flag;
        self.maximized = None;
        self.on_shell_layout_changed();
    }

    /// 内容侧"收起/展开列表列"按钮:用户点击某面板内容区的按钮翻转其列表列
    /// 显隐。语义完全对齐文件预览的 `FileTreeCollapse` 按钮(见 `preview_pane_for`),
    /// 只是图标按该面板当前所在栏(左/右)与收起态四选一、tooltip 由调用方
    /// 给静态文案。泛型 `M` 兼容顶层 `Message` 与各扩展模块的本地 `Message`。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn list_collapse_button<'a, M: Clone + 'a>(
        &self,
        kind: PanelKind,
        collapsed: bool,
        hover_id: HoverId,
        tooltip_collapse: &'a str,
        tooltip_expand: &'a str,
        on_select: M,
        on_hover: impl Fn(bool) -> M + 'a,
    ) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer> {
        let side = self.shell_layout.rail_layout.side_of(kind);
        let (icon, tooltip) = match (side, collapsed) {
            (Side::Left, false) => (icons::IconKind::PanelLeftClose, tooltip_collapse),
            (Side::Left, true) => (icons::IconKind::PanelLeftOpen, tooltip_expand),
            (Side::Right, false) => (icons::IconKind::PanelRightClose, tooltip_collapse),
            (Side::Right, true) => (icons::IconKind::PanelRightOpen, tooltip_expand),
        };
        icons::icon_button_entry(
            icon,
            byteui::theme::icon_size::row(),
            false,
            false,
            self.hover_progress(hover_id),
            false,
            byteui::theme::icon_size::row() + 6.0,
            true,
            on_select,
            on_hover,
            tooltip,
        )
    }

    fn top_bar_home(&mut self) {
        self.current_page = AppPage::Home;
        self.home_recents_loaded = false;
        self.home_left_view = homespace::HomeLeftView::default();
        self.home_project_pages = 1;
        self.home_project_search.clear();
        self.home_project_search_draft.clear();
        self.home_project_search_focused = false;
        self.home_right_view = homespace::HomeRightView::default();
        let projects: Vec<ProjectInfo> = self.recent_projects.iter().take(5).cloned().collect();
        let client = self.client.clone();
        let proxy = self.proxy.clone();
        self.handle.spawn(async move {
            let (files, convs) = load_home_recents(&client, &projects).await;
            let _ = proxy.send_event(Message::HomeRecentsLoaded(files, convs));
        });
    }

    fn preview_open_path(&mut self, path: PathBuf) {
        // 同 `preview_select_tab`:`preview_tab_bar_avail_px` 要 `&self`,
        // 得在 `with_focused_project` 的 `&mut self` 借用之前先算好。
        let avail_w = self.preview_tab_bar_avail_px(PanelKind::Files);
        self.with_focused_project(move |ws, io| {
            if !path.is_file() {
                ws.preview_error = Some(format!("文件不存在或不可读: {}", path.display()));
                return;
            }
            ws.preview_error = None;
            ws.files.set_tree_selected(path.clone());
            ws.allowed_files
                .lock()
                .expect("allowed_files 锁")
                .insert(path.clone());
            ws.preview.open_path(path);
            // 新 tab 落在末尾(复用已开的文件则落在该文件原来的位置)——
            // 用跟 `preview_select_tab` 同一套 `tab_window_reveal`,把窗口
            // 起点钳到"包含这个新激活 tab"的位置,而不是无脑滚回最左
            // (此前 `= 0` 的写法:tab 一多,新开的文件反而被滚出可见区,
            // 2026-09-14 用户反馈"新打开文件时 tab 应该跳转到对应位置")。
            let active = ws.preview.active_idx();
            let widths: Vec<f32> = ws
                .preview
                .tabs()
                .iter()
                .map(|t| preview_tab_display_width(&t.title))
                .collect();
            ws.preview_tab_first =
                tab_widget::tab_window_reveal(&widths, 4.0, avail_w, ws.preview_tab_first, active);
            ws.spawn_preview_state_save(io);
            ws.spawn_preview_context_push(io);
        });
    }

    fn preview_select_tab(&mut self, idx: usize) {
        let arming = self
            .active_workspace()
            .map(|ws| idx < ws.preview.tabs().len())
            .unwrap_or(false);
        // 得在借用 `ws` 之前算好——`preview_tab_bar_avail_px` 要 `&self`,
        // 跟下面 `with_focused_project` 内部的 `&mut self` 借用冲突,必须
        // 提前拿到这个值再原样传进闭包(同渲染侧 `preview_pane_for` 用的
        // 是同一个真实宽度,窗口起点算法两边才不会算出不一致的结果)。
        let avail_w = self.preview_tab_bar_avail_px(PanelKind::Files);
        self.with_focused_project(|ws, io| {
            ws.preview.select(idx);
            if idx < ws.preview.tabs().len() {
                let widths: Vec<f32> = ws
                    .preview
                    .tabs()
                    .iter()
                    .map(|t| preview_tab_display_width(&t.title))
                    .collect();
                ws.preview_tab_first =
                    tab_widget::tab_window_reveal(&widths, 4.0, avail_w, ws.preview_tab_first, idx);
                ws.preview_tab_overflow_anchor = None;
            }
            ws.spawn_preview_state_save(io);
            ws.spawn_preview_context_push(io);
        });
        if arming {
            self.tab_drag = Some(TabDrag {
                group: TabGroup::Preview,
                source: idx,
                press_pos: self.last_cursor,
            });
        }
    }

    /// Project 面板右配对预览打开文件:写入 `ws.project_preview`(独立的
    /// `PreviewPane`),完全不碰 Files 预览的 `ws.preview`/`ws.files`。
    /// tab 是项目链接点开产生的会话期状态,不持久化、也不向 daemon 推上下文,
    /// 避免与 Files 预览那份持久化 `preview_state` 互相覆盖。
    fn project_preview_open_path(&mut self, path: PathBuf) {
        let avail_w = self.preview_tab_bar_avail_px(PanelKind::Project);
        self.with_focused_project(|ws, _io| {
            if !path.is_file() {
                ws.project_preview_error = Some(format!("文件不存在或不可读: {}", path.display()));
                return;
            }
            ws.project_preview_error = None;
            ws.project_panel.set_selected_link(path.clone());
            ws.allowed_files
                .lock()
                .expect("allowed_files 锁")
                .insert(path.clone());
            ws.project_preview.open_path(path);
            // 新 tab 落在末尾(或复用已开文件原位),用 `tab_window_reveal`
            // 钳出包含它的窗口起点,不再无脑滚回最左(同 Files 预览)。
            let active = ws.project_preview.active_idx();
            let widths: Vec<f32> = ws
                .project_preview
                .tabs()
                .iter()
                .map(|t| preview_tab_display_width(&t.title))
                .collect();
            ws.project_preview_tab_first = tab_widget::tab_window_reveal(
                &widths,
                4.0,
                avail_w,
                ws.project_preview_tab_first,
                active,
            );
        });
    }

    /// 项目信息面板切入时调用:确保项目根目录有一份 `README.md`(没有就按
    /// 项目名 + 描述生成,已有则原样保留),然后**一律**在右侧配套预览窗打
    /// 开这份 README(首次切进来就让它展示项目文档)。
    ///
    /// 打开/生成依赖同一份"可读"保障——README 创建失败或不可读时静默返回,
    /// 绝不拿一个空文件去占预览,也绝不让面板切入失败。
    fn ensure_project_readme_and_reveal(&mut self) {
        let Some(project) = self.active_workspace().and_then(|ws| ws.project.clone()) else {
            return;
        };
        let Some(readme) =
            ensure_project_readme(&std::path::PathBuf::from(&project.path), &project.name)
        else {
            return;
        };
        self.project_preview_open_path(readme);
    }

    fn project_preview_select_tab(&mut self, idx: usize) {
        let arming = self
            .active_workspace()
            .map(|ws| idx < ws.project_preview.tabs().len())
            .unwrap_or(false);
        // 同 `preview_select_tab`:提前算好真实可用宽度,避免跟
        // `with_focused_project` 的 `&mut self` 借用冲突。
        let avail_w = self.preview_tab_bar_avail_px(PanelKind::Project);
        self.with_focused_project(|ws, _io| {
            ws.project_preview.select(idx);
            if idx < ws.project_preview.tabs().len() {
                let widths: Vec<f32> = ws
                    .project_preview
                    .tabs()
                    .iter()
                    .map(|t| preview_tab_display_width(&t.title))
                    .collect();
                ws.project_preview_tab_first = tab_widget::tab_window_reveal(
                    &widths,
                    4.0,
                    avail_w,
                    ws.project_preview_tab_first,
                    idx,
                );
                ws.project_preview_tab_overflow_anchor = None;
            }
        });
        if arming {
            self.tab_drag = Some(TabDrag {
                group: TabGroup::ProjectPreview,
                source: idx,
                press_pos: self.last_cursor,
            });
        }
    }

    fn agent_state_changed(
        &mut self,
        project_id: ProjectId,
        tab_id: usize,
        agent: AgentKind,
        state: AgentState,
        transcript_path: Option<String>,
    ) {
        self.with_project(project_id, |ws, io| {
            // 当前项目路径先取出（下面要 &mut 借 tab，冲突）；重锚:项目优先。
            let active_repo = ws.project.as_ref().map(|p| PathBuf::from(&p.path));
            // 卡片刷新要传的数据在这里先摘出来（`Option` 同时充当"tab 是否
            // 存在"的哨兵）：`tab`（来自 `ws.tab_by_id_mut`）借的是整个
            // `ws`，下面 TurnEnded 分支还要再用 `tab`，中间插一句
            // `ws.spawn_agent_card_refresh`（借 `&ws`）会跟这个 `&mut ws`
            // 借用重叠、过不了借用检查；摘成局部变量、挪到这个 `if let`
            // 块结束、`tab` 借用已经释放之后再调用，规避这个冲突,语义不变
            // (仍然是"tab 存在就必调用一次,不进 TurnEnded 条件分支")。
            let mut card_refresh_args: Option<(Option<String>, PathBuf)> = None;
            if let Some(tab) = ws.tab_by_id_mut(tab_id) {
                tab.agent_state = state;
                tab.agent = agent;
                // 补一道:picker 不是唯一入口(比如在已开着的纯 shell tab 里
                // 手打 `opencode`),那种情况下 `on_tab_attached` 拿不到
                // picker 提示,只能靠这条 hook 上报事后补上——虽然大概率已经
                // 错过了 agent 进程启动瞬间那波终端探测查询,但仍是"跟真实
                // agent 保持同步"的唯一后备信号。
                tab.model
                    .set_answer_dynamic_color(agent == AgentKind::Opencode);
                if let Some(tp) = transcript_path {
                    tab.transcript_path = Some(tp);
                }
                tracing::info!(tab_id, ?state, "agent 状态变更");
                card_refresh_args = Some((tab.transcript_path.clone(), tab.effective_cwd()));
            }
            if let Some((transcript_path, cwd)) = card_refresh_args {
                ws.spawn_agent_card_refresh(
                    io,
                    tab_id,
                    agent,
                    transcript_path,
                    cwd,
                    active_repo.clone(),
                );
            }
            // 回合结束后刷新会话列表(transcript 增长/新增；P1j)与项目 git/
            // 磁盘占用状态(文件树装饰随之更新；P1h)。这两项刷新原先分别挂在
            // 会话列表自己的耦合链路、以及已删除的验收检测异步回调
            // (`delivery_checked`)上——验收闭环删除后,后者连带的刷新触发点
            // 也没了,这里改成回合结束就无条件触发,不再依赖任何验收检测结果
            // (前半"会话列表"这条 P1j 当年就已经这样修过一次,这次是把后半
            // "git/磁盘占用"也补齐同样的处理)。
            if state == AgentState::TurnEnded {
                ws.spawn_conversations_refresh(io);
                if let Some(project) = &ws.project {
                    let repo_path = PathBuf::from(&project.path);
                    spawn_project_git_refresh(project_id, repo_path.clone(), io);
                    spawn_disk_usage_refresh(project_id, repo_path, io);
                }
            }
            // 审阅 tab 若开着且属本会话,回合结束重解析 transcript（P1i）。
            if state == AgentState::TurnEnded
                && let Some(rv) = &ws.review
                && review_should_refresh_on_turn(&rv.source, tab_id)
                && let Some((path, tab_agent)) = ws
                    .tabs
                    .iter()
                    .find(|t| t.tab_id == tab_id)
                    .and_then(|t| t.transcript_path.clone().map(|p| (p, t.agent)))
            {
                ws.spawn_review_load(
                    io,
                    ReviewSource::Session(tab_id),
                    path,
                    tab_agent,
                    -1,
                    10_000,
                );
            }
        });
    }

    /// 点击对话面板扁平列表里的某一行——只把审阅面板加载到这一个回合
    /// 点对话面板会话列表某一行 → 打开该 session 的详情审阅:整段回合
    /// 列表 + 总结展示区(标题/全文)。`agent` 由调用方随行内数据一并传入。
    /// 总结数据直接取自已加载的 `SessionRow`,不为此单独发请求(2026-08-27)。
    fn conversation_session_open(&mut self, conversation_id: String, agent: AgentKind) {
        self.with_focused_project(move |ws, io| {
            let Some(row) = ws
                .conversations
                .sessions()
                .and_then(|rows| rows.iter().find(|r| r.conversation_id == conversation_id))
            else {
                // 列表刷新与点击之间的竞态(极小概率):这一行已经不在当前
                // 列表里了,直接不打开详情,不 panic、不报错弹窗。
                return;
            };
            let summary_title = row.display_title.clone();
            let summary_text = row.summary.clone();
            let last_ts = row.last_ts;
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            let source = ReviewSource::Conversation(conversation_id.clone());
            ws.review = Some(ReviewView {
                source: source.clone(),
                entries: Vec::new(),
                error: None,
                agent,
                nonce: 0,
                summary_title: Some(summary_title),
                summary_text,
                summary_time: Some(relative_time_text(last_ts, now_ms)),
            });
            ws.spawn_review_load_conversation(
                io,
                conversation_id,
                -1,
                CONVERSATION_DETAIL_PAGE_SIZE,
                false,
            );
        });
    }

    fn search_results(
        &mut self,
        project_id: i64,
        result: Result<Vec<(String, Vec<search::SearchHit>)>, String>,
    ) {
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        self.with_project(project_id, move |ws, _io| {
            let emit = move |m| {
                let _ = proxy.send_event(Message::Search(m));
            };
            search::update(
                &mut ws.search,
                search::Message::SearchResults(project_id, result),
                project_id,
                &handle,
                emit,
            );
        });
    }

    fn files_project_message(&mut self, project_id: i64, msg: files::Message) {
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

    /// Project 面板链接行右键菜单浮层:当前只含"删除"。定位坐标复用
    /// `files.last_right_click`(main.rs 任意右键都会先写入),"删除"回
    /// `project::Message::LinkRemove`。
    fn project_link_context_menu_popup<'a>(
        &self,
    ) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
        let menu = match &self.project_link_menu {
            Some(m) => m,
            None => return column![].into(),
        };
        let items: Vec<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>> =
            vec![crate::menu::item::<Message>(
                Some(icons::IconKind::Trash),
                "删除",
                Message::Project(project::Message::LinkRemove {
                    target: menu.target,
                    index: menu.index,
                }),
            )];

        let list: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
            crate::menu::shell_frosted(items, Length::Shrink);
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

    /// Todo 分类树节点右键菜单浮层:新建子/同级分类、重命名、删除、上移/
    /// 下移、移动到...。定位坐标复用 `files.last_right_click`。
    fn category_context_menu_popup<'a>(
        &self,
    ) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
        let menu = match &self.category_context_menu {
            Some(m) => m,
            None => return column![].into(),
        };
        // 右键的是"全部"/"未分类"伪节点:菜单只含"新建分类"(新建顶层
        // 分类,不挂在任何真实名字下——那两个只是视图桶)。真实节点才给
        // 完整节点操作集。
        let items: Vec<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>> =
            match menu.id {
                None => vec![crate::menu::item::<Message>(
                    Some(icons::IconKind::SquarePlus),
                    "新建分类",
                    Message::Todo(todo::Message::CategoryNewSibling(None)),
                )],
                Some(id) => {
                    // 被右键节点的 parent_id,给"新建同级分类"用(同级 =
                    // 挂在同一个 parent_id 下)。取不到就退化为顶层。
                    let sibling_parent_id = self.active_workspace().and_then(|ws| {
                        ws.todo
                            .categories()
                            .iter()
                            .find(|c| c.id == id)
                            .and_then(|c| c.parent_id)
                    });
                    vec![
                        crate::menu::item::<Message>(
                            Some(icons::IconKind::SquarePlus),
                            "新建子分类",
                            Message::Todo(todo::Message::CategoryNewChild(id)),
                        ),
                        crate::menu::item::<Message>(
                            Some(icons::IconKind::SquarePlus),
                            "新建同级分类",
                            Message::Todo(todo::Message::CategoryNewSibling(sibling_parent_id)),
                        ),
                        crate::menu::item::<Message>(
                            Some(icons::IconKind::ChevronUp),
                            "上移",
                            Message::Todo(todo::Message::CategoryMoveSibling(
                                id,
                                dozer_core::protocol::CategoryMoveDirection::Up,
                            )),
                        ),
                        crate::menu::item::<Message>(
                            Some(icons::IconKind::ChevronDown),
                            "下移",
                            Message::Todo(todo::Message::CategoryMoveSibling(
                                id,
                                dozer_core::protocol::CategoryMoveDirection::Down,
                            )),
                        ),
                        crate::menu::item::<Message>(
                            Some(icons::IconKind::FolderOpen),
                            "移动到...",
                            Message::Todo(todo::Message::CategoryReparentPickerOpen(id)),
                        ),
                        crate::menu::item::<Message>(
                            Some(icons::IconKind::Rename),
                            "重命名",
                            Message::Todo(todo::Message::CategoryRenameStart(id)),
                        ),
                        crate::menu::item::<Message>(
                            Some(icons::IconKind::Trash),
                            "删除",
                            Message::Todo(todo::Message::CategoryDelete(id)),
                        ),
                    ]
                }
            };

        let list: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
            crate::menu::shell_frosted(items, Length::Shrink);
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

    /// 分类选择器浮层:列出当前项目的全部分类节点(全展开按 depth 缩进
    /// 平铺),点"未分类"或某个节点即把 `category_picker` 目标落盘并关闭
    /// (Category → reparent,Todo → set_todo_category)。浮层只服务这两类
    /// "把一个节点挂到某个分类"的动作 —— 搜索框的分类速滤已移除(改为
    /// 按状态过滤),不再有 `Filter` 目标。
    fn category_picker_popup<'a>(
        &self,
    ) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
        let Some(picker) = &self.category_picker else {
            return column![].into();
        };
        let categories = self
            .active_workspace()
            .map(|ws| ws.todo.categories().to_vec())
            .unwrap_or_default();
        let mut list = column![].spacing(2);

        // "未分类"钉在树顶(`None` = 未分类)。
        list = list.push(
            button(text("未分类").size(byteui::theme::font::body()))
                .on_press(Message::CategoryPickerSelect(None))
                .width(Length::Fill)
                .padding([6, 10]),
        );
        // 复用一份没有展开态(全展开)的拍平——选择器只做单次选择,不需要
        // 折叠交互,直接把整棵树按 depth 缩进平铺出来最简单。
        let all_expanded: std::collections::HashSet<i64> =
            categories.iter().map(|c| c.id).collect();
        for row in todo::visible_category_rows(&categories, &all_expanded) {
            let id = row.id;
            let click = Message::CategoryPickerSelect(Some(id));
            list = list.push(
                button(
                    text(format!("{}{}", "  ".repeat(row.depth), row.name))
                        .size(byteui::theme::font::body()),
                )
                .on_press(click)
                .width(Length::Fill)
                .padding([6, 10]),
            );
        }
        let list =
            container(list.width(Length::Fixed(220.0))).style(move |_t: &iced_widget::Theme| {
                container::Style {
                    background: Some(byteui::theme::color::current().card.into()),
                    border: Border {
                        color: byteui::theme::color::current().border,
                        width: 1.0,
                        radius: 6.0.into(),
                    },
                    ..container::Style::default()
                }
            });
        container(list)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(Padding {
                top: picker.y,
                left: picker.x,
                right: 0.0,
                bottom: 0.0,
            })
            .into()
    }

    /// 任务详情弹窗:原生 iced 渲染(不复用会话面板的 webview trace——
    /// 那套渲染实际内容在 `dozer://review-trace/host.html` 里,任务详情
    /// 只需要看人类/agent 往来文本,不需要工具调用折叠/trace 可视化,
    /// 塞进一个跟随光标定位、随时开合的原生弹窗里没有必要也不合适)。
    fn todo_detail_popup<'a>(
        &self,
    ) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
        let Some(ws) = self.active_workspace() else {
            return column![].into();
        };
        let Some(idx) = ws.todo.detail_open_idx() else {
            return column![].into();
        };
        let Some(item) = ws.todo.items().get(idx) else {
            return column![].into();
        };

        let header = column![
            text(item.text.clone()).size(byteui::theme::font::subtitle()),
            text(
                item.assigned_agent
                    .map(|a| format!("指派给:{}", a.label()))
                    .unwrap_or_else(|| "未指派".to_string())
            )
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().dim),
        ]
        .spacing(4);

        let mut turns_col = column![].spacing(8);
        for turn in ws.todo.detail_turns() {
            let label = if turn.role == "human" {
                "你".to_string()
            } else {
                item.assigned_agent
                    .map(|a| a.label().to_string())
                    .unwrap_or_else(|| "AI".to_string())
            };
            turns_col = turns_col.push(
                column![
                    text(label)
                        .size(byteui::theme::font::caption())
                        .color(byteui::theme::color::current().gold),
                    text(turn.content.clone())
                        .size(byteui::theme::font::body())
                        .width(Length::Fill),
                ]
                .spacing(2),
            );
        }
        let turns_scroll = iced_widget::Scrollable::new(turns_col)
            .width(Length::Fill)
            .height(Length::Fixed(320.0))
            .direction(iced_widget::scrollable::Direction::Vertical(
                byteui::interaction::scrollbar::scrollbar(),
            ))
            .style(|_t, _s| byteui::interaction::scrollbar::scrollbar_style());

        let reply_box = container(byteui::form::input_text::view(
            "回复...",
            ws.todo.detail_reply_draft(),
            false,
            Some(todo::detail_reply_field_id()),
            false,
            None,
            false,
            |s| Message::Todo(todo::Message::DetailReplyInput(s)),
        ))
        .width(Length::Fill);
        let submit_label = if ws.todo.detail_processing() {
            "处理中…"
        } else {
            "处理"
        };
        let submit = button(text(submit_label))
            .on_press_maybe(
                (!ws.todo.detail_processing())
                    .then_some(Message::Todo(todo::Message::DetailReplySubmit)),
            )
            .padding([6, 12]);

        // 宽度改用 `dialog::width`(整窗 1/3,2026-09-15 统一约定),取代此前
        // 写死的 480px。
        let card = column![header, turns_scroll, row![reply_box, submit].spacing(8)]
            .spacing(12)
            .padding(16)
            .width(crate::dialog::width(self.window_size.0));
        let card = container(card).style(crate::dialog::card_style);

        container(card)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(iced_widget::core::alignment::Horizontal::Center)
            .align_y(iced_widget::core::alignment::Vertical::Center)
            .into()
    }

    /// 数据库面板数据源树 header 行右键菜单浮层:测试连接/编辑/删除/刷新。
    /// 定位坐标复用 `files.last_right_click`。"刷新"只有该数据源当前已
    /// 展开(有 schema 树数据)才可点,未展开时置灰——展开动作本身走左键
    /// 点 header,不进这个菜单。
    fn database_source_context_menu_popup<'a>(
        &self,
    ) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
        let menu = match &self.database_source_menu {
            Some(m) => m,
            None => return column![].into(),
        };
        let source_id = menu.source_id.clone();
        let expanded = self
            .active_workspace()
            .is_some_and(|ws| ws.database.is_expanded(&source_id));
        let dim = byteui::theme::color::current().dim;
        let mut items: Vec<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>> = vec![
            crate::menu::item::<Message>(
                Some(icons::IconKind::RefreshCw),
                "测试连接",
                Message::Database(database::Message::TestConnection(source_id.clone())),
            ),
            crate::menu::item::<Message>(
                Some(icons::IconKind::Settings),
                "编辑",
                Message::Database(database::Message::EditSourceStart(source_id.clone())),
            ),
            crate::menu::item::<Message>(
                Some(icons::IconKind::Trash),
                "删除",
                Message::Database(database::Message::DeleteSourceRequest(source_id.clone())),
            ),
        ];
        items.push(if expanded {
            crate::menu::item::<Message>(
                Some(icons::IconKind::RotateCw),
                "刷新",
                Message::Database(database::Message::SchemaRefresh(source_id.clone())),
            )
        } else {
            crate::menu::item_locked(Some(icons::IconKind::RotateCw), "刷新", dim)
        });

        let list: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
            crate::menu::shell_frosted(
                items,
                Length::Fixed(byteui::theme::geometry::menu_item_width()),
            );
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

    /// 输入框右键菜单浮层:固定四项(剪切/复制/粘贴/全选),定位坐标复用
    /// `files.last_right_click`(main.rs 任意右键都会先写入,`TextInputMenuOpen`
    /// 已用它填好 `x/y`)。动作消息回 main.rs——由它合成回 ⌘/Ctrl+`x`/`c`/`v`/`a`
    /// 键盘事件作用到被右键的输入。密码框(`secure`)禁用剪切/复制(置灰)。
    fn text_input_menu_popup<'a>(
        &self,
    ) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
        let menu = match &self.text_input_menu {
            Some(m) => m,
            None => return column![].into(),
        };
        let dim = byteui::theme::color::current().dim;
        let mut items: Vec<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>> =
            Vec::new();
        if menu.target.secure {
            // 密码框:复制/剪切同原生快捷键一样被禁用,置灰不可点。
            items.push(crate::menu::item_locked(
                Some(icons::IconKind::Scissors),
                "剪切",
                dim,
            ));
            items.push(crate::menu::item_locked(
                Some(icons::IconKind::Copy),
                "复制",
                dim,
            ));
        } else {
            items.push(crate::menu::item(
                Some(icons::IconKind::Scissors),
                "剪切",
                Message::TextInputMenuCut,
            ));
            items.push(crate::menu::item(
                Some(icons::IconKind::Copy),
                "复制",
                Message::TextInputMenuCopy,
            ));
        }
        items.push(crate::menu::item(
            Some(icons::IconKind::ClipboardPaste),
            "粘贴",
            Message::TextInputMenuPaste,
        ));
        items.push(crate::menu::item(
            Some(icons::IconKind::SelectAll),
            "全选",
            Message::TextInputMenuSelectAll,
        ));

        let list: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
            crate::menu::shell_frosted(
                items,
                Length::Fixed(byteui::theme::geometry::menu_item_width()),
            );
        // 常规右键菜单就地向下/向上弹即可,这里输入框多用在面板内容区,直接
        // 以光标为左上锚弹出(必要时可在下方再夹窗口高度,留待需要时加)。
        container(list)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_y(iced_widget::core::alignment::Vertical::Top)
            .padding(Padding {
                top: menu.y,
                left: menu.x,
                right: 0.0,
                bottom: 0.0,
            })
            .into()
    }

    pub fn view(
        &self,
    ) -> iced_widget::core::Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
        let content = self.view_inner();
        // 设置弹窗是 App 级浮层(见 `settings_modal_open` 字段文档),必须能
        // 盖在首页/空工作区/项目工作区三种 `view_inner` 分支之上——所以放在
        // 最外层统一叠加,而不是塞进 `view_inner` 内部某个分支。
        if self.settings_modal_open {
            stack![
                content,
                crate::dialog::scrim(Message::SettingsClose),
                settings::settings_modal(self.window_size.0)
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else {
            content
        }
    }

    fn view_inner(
        &self,
    ) -> iced_widget::core::Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
        // 顶栏先画:它是外壳的一部分(项目页签行 + "＋"就在上面),一个项目都
        // 没打开时更要画得出来——否则用户没有任何入口去打开第一个项目。
        let top = topbar::top_bar(self);
        // 首页落地页:点顶栏 Dozer 进入,独立于工作区(即使没开任何项目也画得
        // 出来)。打开/切换项目会自动退回工作区(见各 `ProjectTab*` 处理器)。
        if self.current_page == AppPage::Home {
            return column![top, homespace::home_page(self, &self.footbar)].into();
        }
        // 一个项目页签都没有(或当前页签还停在 `Stub` 没促成)时的占位正文。
        let Some(ws) = self.active_workspace() else {
            // `daemon_error` 必须在这里也画:它平时挂在 `terminal_pane`/
            // `project_status_bar` 上,而那两处都在"有 `Workspace` 才走到"的
            // 分支里。偏偏 daemon 连不上时(`App::with_daemon_error`)一个项目
            // 都恢复不出来,恰恰只会走到这条空态分支——错误文案于是在最需要它
            // 的时候恰好隐身,用户只看到"点 ＋ 打开一个",点了又静默失败
            // (`ProjectTabOpened(None, ..)` 只是再写一遍 `daemon_error`)。
            // 配色沿用 `terminal_pane` 那条同源文案的 RED(最终审查
            // Required Fix #1)。
            let mut hint_col = column![
                text("未打开任何项目——点顶栏的 ＋ 打开一个")
                    .size(byteui::theme::font::subtitle())
                    .color(byteui::theme::color::current().dim)
            ]
            .spacing(8);
            if let Some(err) = &self.daemon_error {
                hint_col = hint_col.push(
                    text(format!("⚠ {err}"))
                        .size(byteui::theme::font::body())
                        .color(byteui::theme::color::current().red),
                );
            }
            let hint = container(hint_col.padding(16))
                .width(Length::Fill)
                .height(Length::Fill)
                .style(|_t: &iced_widget::Theme| container::Style {
                    background: Some(theme::region::background().into()),
                    ..container::Style::default()
                });
            return column![top, hint].into();
        };
        // 用 `row!`(经 `Row::push`/`enclose`)构造:只要子元素里有一个声明了
        // `Length::Fill`/`FillPortion`(如某侧收起时的 `left_panel_area`),
        // 这条 row 自身的宽度就会被自动升级成 `Fill`,从而在 flex 布局里正确
        // 撑满窗口。换成 `Row::from_vec`(其文档明确说明不会检视子元素)或
        // 手动 `.width(Length::Shrink)` 会让 flex 第三阶段(fill 分配)不再
        // 执行,右图标栏就会缩到窗口中间——不要在不理解这个前提的情况下改写。
        let body = row![
            rail::icon_rail(self, Side::Left),
            column![
                row![
                    left_panel_area(self, ws, false),
                    divider_bar(
                        Divider::LeftRight,
                        byteui::theme::color::current().bg,
                        byteui::theme::color::current().bg,
                        Message::ColumnDragStart(Divider::LeftRight),
                    ),
                    right_panel_area(self, ws, false),
                ]
                .height(Length::Fill),
                footbar::view(&self.footbar).map(Message::Footbar),
            ]
            .width(Length::Fill),
            rail::icon_rail(self, Side::Right),
        ];
        let base = column![top, body];

        let popped = if ws.search_popup_open() {
            // 文件树右键"搜索"弹窗:窗口级浮层。遮罩"点点即关"由
            // `search_modal` 内部自己处理(整窗 `SCRIM` 做成可点击目标,卡片
            // 是兄弟元素盖在上面),这里只需把弹窗叠在 `base` 之上。
            let project_root = ws.project.as_ref().map(|p| std::path::Path::new(&p.path));
            stack![
                base,
                search::search_modal(&ws.search, project_root, self.window_size.0)
                    .map(Message::Search)
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else if ws.files.tree_delete_confirm_is_some() {
            let dismiss = crate::dialog::scrim(Message::Files(files::Message::DeleteCancel));
            stack![
                base,
                dismiss,
                files::delete_confirm_popup(&ws.files, self.window_size.0).map(Message::Files)
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else if ws.files.pending_move_is_some() {
            let dismiss = crate::dialog::scrim(Message::Files(files::Message::MoveCancel));
            stack![
                base,
                dismiss,
                files::move_confirm_popup(&ws.files, self.window_size.0).map(Message::Files)
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else if self.files.context_menu_is_some() {
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::Files(files::Message::ContextMenuClose));
            stack![
                base,
                dismiss,
                files::context_menu_popup(&self.files, &ws.files).map(Message::Files)
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else if ws.files.branch_picker_is_open() {
            // 分支切换弹层:窗口级浮层。下层铺一块透明 `MouseArea` 承接
            // "点弹层外的任何地方收起"(与右键菜单同款 dismiss 约定),弹层
            // 本体(`branch_picker_popup`)只占 git 底栏上方一隅。
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::Files(files::Message::BranchPickerClose));
            stack![
                base,
                dismiss,
                files::branch_picker_popup(&ws.files).map(Message::Files)
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else if self.project_link_menu.is_some() {
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::ProjectLinkContextMenuClose);
            stack![base, dismiss, self.project_link_context_menu_popup()]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else if self.category_context_menu.is_some() {
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::CategoryContextMenuClose);
            stack![base, dismiss, self.category_context_menu_popup()]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else if self.category_picker.is_some() {
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::CategoryPickerClose);
            stack![base, dismiss, self.category_picker_popup()]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else if self.text_input_menu.is_some() {
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::TextInputMenuClose);
            stack![base, dismiss, self.text_input_menu_popup()]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else if self.database_source_menu.is_some() {
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::DatabaseSourceContextMenuClose);
            stack![base, dismiss, self.database_source_context_menu_popup()]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else if ws.project_panel.delete_pending.is_some() {
            // 项目面板「删除项目」确认框:窗口级 overlay,同其它面板弹窗
            // 的既有口径(2026-09-15 起——此前是 panel-level `stack!`,只在
            // 本面板宽度范围内居中,不是整个软件窗体)。
            let dismiss =
                crate::dialog::scrim(Message::Project(project::Message::DeleteProjectCancel));
            stack![
                base,
                dismiss,
                project::project_delete_confirm_popup(&ws.project_panel, self.window_size.0)
                    .map(Message::Project)
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else if ws.project_panel.scaffold_run.is_some() {
            // 项目面板「修复项目」进度弹窗:窗口级 overlay。进行中不可通过
            // 点遮罩关闭(`scrim_blocking` 不挂 `on_press`),同 panel-level
            // 版本的既有约定(spec"弹窗可取消性"一节)。
            let scrim = crate::dialog::scrim_blocking();
            stack![
                base,
                scrim,
                project::scaffold_progress_popup(&ws.project_panel, self.window_size.0)
                    .map(Message::Project)
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else if ws.database.delete_confirm().is_some() {
            // 数据库面板「删除数据源」确认框:窗口级 overlay,同上。三个
            // 数据库弹窗互斥优先级(同一时刻只显示一个):待确认删除 >
            // 新增/编辑表单 > 驱动管理。
            let source_id = ws.database.delete_confirm().unwrap();
            let dismiss =
                crate::dialog::scrim(Message::Database(database::Message::DeleteSourceCancel));
            stack![
                base,
                dismiss,
                database::delete_confirm_popup(&ws.database, source_id, self.window_size.0)
                    .map(Message::Database)
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else if ws.database.editing().is_some() {
            // 数据库面板「新增/编辑数据源」表单:窗口级 overlay,同上。
            let draft = ws.database.editing().unwrap();
            let dismiss = crate::dialog::scrim(Message::Database(database::Message::DraftCancel));
            stack![
                base,
                dismiss,
                database::source_form(
                    draft,
                    &self.database,
                    ws.database.draft_test_status(),
                    self.window_size.0,
                )
                .map(Message::Database)
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else if self.database.drivers_popup_open() {
            // 数据库面板「管理驱动」弹窗:窗口级 overlay,同上。
            let dismiss =
                crate::dialog::scrim(Message::Database(database::Message::DriversPopupToggle));
            stack![
                base,
                dismiss,
                database::drivers_popup(&self.database, self.window_size.0).map(Message::Database)
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else if ws.ssh.delete_confirm().is_some() {
            // 主机面板「删除主机」确认框:窗口级 overlay,同上。两个主机
            // 弹窗互斥优先级(同一时刻只显示一个):待确认删除 > 新增/编辑
            // 表单。
            let host_id = ws.ssh.delete_confirm().unwrap();
            let dismiss = crate::dialog::scrim(Message::Ssh(ssh::Message::DeleteHostCancel));
            stack![
                base,
                dismiss,
                ssh::delete_confirm_popup(&ws.ssh, host_id, self.window_size.0).map(Message::Ssh)
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else if ws.ssh.editing().is_some() {
            // 主机面板「添加/编辑主机」表单:窗口级 overlay,同上。
            let draft = ws.ssh.editing().unwrap();
            let status = draft
                .id
                .as_deref()
                .map(|id| ws.ssh.test_status(id))
                .unwrap_or(&ssh::TestStatus::Idle);
            let dismiss = crate::dialog::scrim(Message::Ssh(ssh::Message::DraftCancel));
            stack![
                base,
                dismiss,
                ssh::host_form(draft, status, self.window_size.0).map(Message::Ssh)
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else if ws.agent_picker_open {
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::AgentPickerClose);
            stack![base, dismiss, agent_picker_popup(ws)]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else if self.project_add_menu_open {
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::ProjectAddMenuClose);
            stack![base, dismiss, topbar::project_add_menu_popup(self)]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else if ws.todo.status_popup_open() {
            // 状态下拉选择层:窗口级 overlay。点弹层外任意处经 dismiss 收起
            // (与右键菜单/分支切换同款约定),弹层本体定位到点击"状态"按钮时
            // 的光标锚点。
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::Todo(todo::Message::StatusClose));
            match todo::todo_status_overlay(ws, self.window_size) {
                Some(popup) => stack![base, dismiss, popup.map(Message::Todo)]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
                None => stack![base, dismiss]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
            }
        } else if ws.todo.calendar_popup_open() {
            // 日历浮层:窗口级 overlay。点弹层外任意处经 dismiss 收起(与右键
            // 菜单/分支切换同款约定),弹层本体定位到点击按钮时的光标锚点。
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::Todo(todo::Message::CalendarClose));
            match todo::todo_calendar_overlay(ws, self.window_size) {
                Some(popup) => stack![base, dismiss, popup.map(Message::Todo)]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
                None => stack![base, dismiss]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
            }
        } else if ws.todo.dispatch_popup_open() {
            // 派发选择层:窗口级 overlay。点弹层外任意处经 dismiss 收起(与
            // 右键菜单/分支切换同款约定),弹层本体列出可指派的 agent(带图标),
            // 定位到点击"指派"按钮时的光标锚点。
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::Todo(todo::Message::DispatchClose));
            match todo::todo_dispatch_overlay(ws, self.window_size) {
                Some(popup) => stack![base, dismiss, popup.map(Message::Todo)]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
                None => stack![base, dismiss]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
            }
        } else if ws.todo.clear_confirm_open() {
            // Todo"清空列表"确认弹窗:窗口级 overlay,与其它 Todo 浮层同款
            // "点遮罩即收起"约定。
            let dismiss = crate::dialog::scrim(Message::Todo(todo::Message::ClearListCancel));
            stack![
                base,
                dismiss,
                todo::clear_confirm_popup(&ws.todo, self.window_size.0).map(Message::Todo)
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else if ws.todo.detail_popup_open() {
            // 任务详情弹窗:窗口级 overlay,原生渲染(不走 wry webview)。
            // 点弹层外任意处经 dismiss 收起,与其它 Todo 浮层同款约定。
            let dismiss = crate::dialog::scrim(Message::Todo(todo::Message::DetailClose));
            stack![base, dismiss, self.todo_detail_popup()]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else if ws.todo.status_filter_popup_open() {
            // 搜索框左前"状态"筛选浮层:窗口级 overlay。点弹层外任意处经
            // dismiss 收起(与右键菜单/分支切换同款约定),弹层本体的每一项
            // (全部/待办/进行中/搁置/已完成)emit `StatusFilterPick`,选中
            // 浮层即收。
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::Todo(todo::Message::StatusFilterClose));
            match todo::todo_status_filter_overlay(ws, self.window_size) {
                Some(popup) => stack![base, dismiss, popup.map(Message::Todo)]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
                None => stack![base, dismiss]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
            }
        } else if ws.term_tab_overflow_anchor.is_some() {
            // 终端 tab 栏"溢出下拉"(V 按钮):窗口级 overlay,理由见
            // `terminal::term_tab_overflow_popup` 文档——必须在这里(顶层)
            // 拼,`anchor`/`window_size` 才与全窗口坐标系一致,否则位置算错
            // (验收反馈"菜单错位了")。
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::TermTabOverflowDismiss);
            match terminal::term_tab_overflow_popup(self, ws) {
                Some(popup) => stack![base, dismiss, popup]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
                None => stack![base, dismiss]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
            }
        } else if ws.preview_tab_overflow_anchor.is_some() {
            // 文件预览 tab 栏"溢出下拉",处理方式同上。
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::PreviewTabOverflowDismiss);
            match preview_tab_overflow_popup(self, ws, PreviewPaneKind::Files) {
                Some(popup) => stack![base, dismiss, popup]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
                None => stack![base, dismiss]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
            }
        } else if ws.project_preview_tab_overflow_anchor.is_some() {
            // Project 面板配对预览 tab 栏"溢出下拉",处理方式同上。
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::ProjectPreviewTabOverflowDismiss);
            match preview_tab_overflow_popup(self, ws, PreviewPaneKind::Project) {
                Some(popup) => stack![base, dismiss, popup]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
                None => stack![base, dismiss]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
            }
        } else if ws.ssh_tab_overflow_anchor.is_some() {
            // SSH 面板自己 tab 条"溢出下拉",处理方式同上。
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::Ssh(ssh::Message::TabOverflowDismiss));
            match ssh_tab_overflow_popup(self, ws) {
                Some(popup) => stack![base, dismiss, popup]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
                None => stack![base, dismiss]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
            }
        } else if ws.database.content().tab_overflow_anchor().is_some() {
            // Database 面板内容窗格 tab 栏"溢出下拉",处理方式同上;弹层本身
            // 用的是 database 扩展自己的 `Message`,`.map` 回顶层。
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::Database(database::Message::TabOverflowDismiss));
            match database::tab_overflow_popup(self, &ws.database) {
                Some(popup) => stack![base, dismiss, popup.map(Message::Database)]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
                None => stack![base, dismiss]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into(),
            }
        } else {
            // 始终用 `Stack` 作根,与上面两个分支(删确认弹窗 / 右键菜单)保持一致:
            // 右键菜单开关会把根 widget 类型在 `Column`(`base.into()`)与 `Stack`
            // 之间切换,而 iced 的 `Tree::diff` 在根 tag 变化时(见
            // `iced_core::widget::tree::Tree::diff`)会整体重建整棵树、丢掉所有
            // 嵌套状态——文件树 scrollable 的滚动偏移就在其中,于是右键后滚动条
            // 跳回顶部。统一成 `Stack` 后根 tag 恒定,`base` 子树被 reconcile 原地
            // 保留,滚动位置不再丢失。
            stack![base].into()
        };

        let with_maximize: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
            if let Some(which) = self.maximized {
                stack![popped, maximize_overlay(self, ws, which)]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into()
            } else {
                popped
            };

        if self.rail_drag_confirmed() {
            stack![with_maximize, rail::rail_drag_ghost(self)]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else if self.tree_drag_confirmed() {
            stack![with_maximize, files::tree_drag_ghost(self)]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else {
            with_maximize
        }
    }
}

/// 左二预览 pane:表头 + tab 栏 + 地址栏;内容区本体是 wry webview
/// 子视图(不在 iced 树里),这里只留占位背景——无 tab 时显示提示文案。
/// 左一项目栏：项目卡（名称 + git 分支/脏 + 路径）+ 文件树；无项目时"打开项目…" + 最近。
/// 右一 AI 栏（P1j）：视图切换 [对话|Agents] + 对话列表（当前行金框高亮）。
/// 顶栏：左 Dozer 标题、中 并行项目页签行 + "＋"、右 金色目标胶囊 + 设置齿轮。
///
/// 参数从 `&Workspace` 改成 `&App`：页签行要读的是**外壳级**的
/// `projects`/`project_order`/`active_project_id`（哪些项目开着、什么顺序、
/// 谁在前台），单个 `Workspace` 里没有这份信息。目标胶囊仍只讲当前项目，
/// 从 `app.active_workspace()` 取——没有项目在前台时它自然不画。
///
/// 原先中间的 ⌘K 搜索框是视觉占位（没有任何交互接线），让位给页签行；
/// 搜索入口日后回来时应另找位置，不要再把页签挤掉。
/// 顶栏内容行直接吃满 `top_bar_height()` 并 `align_y(Center)` 垂直居中——
/// 高度由 `workspace.json` 的 `geometry.top_bar_height` 单一来源驱动。
/// (`MACOS_TRAFFIC_LIGHT_BAND_HEIGHT` 的 28px 顶对齐约定已废弃:用户要
/// 求顶栏用自身高度居中内容,不再贴 macOS 交通灯基准。)
///
/// Figma 设计稿(Dozer Phase 1 UI,node-id=87:31)里顶栏标题/页签/加号
/// 文字标的都是 Inter Medium——应用没绑定 Inter,用系统默认字体的
/// Medium 档位贴近这个字重意图,不引入新字体文件。
pub(crate) fn top_bar_font() -> Font {
    Font {
        weight: Weight::Medium,
        ..Font::default()
    }
}

/// 上面那个的纯逻辑内核(可单测:构造 `Workspace` 需要 daemon + EventLoop,
/// headless 测试里造不出来,与本文件既有约定一致)。`agent == Unknown` 的
/// 会话(纯 shell/git shell/hook 还没上报过——`SessionInfo::agent` 文档:
/// "首个 hook 事件到达前恒 Unknown")不参与正常优先级竞争,它们的
/// `AgentState` 只是从未被真实 hook 改写过的默认值,不代表真实"空闲"——
/// 混进竞争会让纯 shell 页签显示成跟真实 agent 完成一轮工作同款的
/// cyan"空闲"点,分不清"agent 真空下来了"和"这压根不是 agent 会话"。
/// 若项目里**还有**真实 agent 存活,优先级/颜色照旧只看那些;若存活会话
/// **全是** Unknown,显示"死会话"灰点(不是"没有点"——用户仍要看得出这个
/// 项目有存活会话,只是状态不可知)。
pub(crate) fn project_dot(alive: &[(AgentState, AgentKind)]) -> Option<Color> {
    let real_states: Vec<AgentState> = alive
        .iter()
        .filter(|(_, agent)| *agent != AgentKind::Unknown)
        .map(|(state, _)| *state)
        .collect();
    if let Some(state) = winning_agent_state(&real_states) {
        return Some(agent_state_dot(state));
    }
    if alive.iter().any(|(_, agent)| *agent == AgentKind::Unknown) {
        return Some(byteui::theme::color::current().dim);
    }
    None
}

/// 一组存活会话状态里"最值得关注"的那个(2026-08-17 用户重新定案的优先级):
/// AwaitingInput(agent 在等你)> Running(还在跑)> TurnEnded(该你出手了)
/// > Idle > 无存活会话(`None`,不画点)。
///
/// 与 [`agent_state_dot`] 分家是为了让 `Stub` 页签也能用:启动恢复时那些还没
/// 促成的页签手上只有 daemon 的 `SessionInfo` 列表,没有 `Workspace`,但"哪个
/// 状态优先"这条规则必须与 `Loaded` 页签**完全一致**,不能各写一份
/// (最终审查 Required Fix #5)。
fn winning_agent_state(alive_states: &[AgentState]) -> Option<AgentState> {
    [
        AgentState::AwaitingInput,
        AgentState::Running,
        AgentState::TurnEnded,
        AgentState::Idle,
    ]
    .into_iter()
    .find(|candidate| alive_states.contains(candidate))
}

/// 胜出状态 → 颜色。不另造一套表,直接问既有 `dot_color`——页签点与 tab
/// 点讲的是同一种语言,两份颜色表迟早会漂。
fn agent_state_dot(state: AgentState) -> Color {
    dot_color(state, true)
}

/// 启动恢复时给每个 `Stub` 页签算后台活动状态:从 daemon 一次性吐出的全量
/// 会话列表里,挑出属于该项目的**存活**会话,套用与 `Loaded` 页签相同的优先级。
///
/// 为什么非要有这一步:重启后除了上次聚焦的那一个,**所有**页签都是 `Stub`,
/// 而 `Stub` 手上没有会话列表 → 指示点恒为空。也就是说"切走了还想知道另一个
/// 项目有没有在动"这个整套功能存在的核心理由(设计文档 §6),在最常见的
/// "刚打开 app"场景下完全不工作。这里只额外花一次 `list()` 往返、不促成任何
/// `Workspace`,懒加载照旧(最终审查 Required Fix #5)。
fn stub_activity(sessions: &[SessionInfo], project_id: i64) -> Option<Color> {
    let alive: Vec<(AgentState, AgentKind)> = sessions
        .iter()
        .filter(|s| s.alive && s.project_id == Some(project_id))
        .map(|s| (s.agent_state, s.agent))
        .collect();
    project_dot(&alive)
}

/// 面板区里某块 pane 在外框圆角处要收圆的外角:`Left`/`Right` 配对视图里
/// 左 pane 收左侧、右 pane 收右侧;`All` 是 Web 单 pane 收全部四角;`None`
/// 不收(放大态下 pane 直接撑满放大盒子,外框由金色浮层负责,方角才对)。
#[derive(Clone, Copy)]
pub(crate) enum PaneCorner {
    None,
    Left,
    Right,
    All,
}

/// 把 `left_zone`/`right_zone` 的圆角背景"透"到内部 pane 上:iced 的
/// `Container::clip(true)` 只把子元素裁成**矩形**,裁不出圆角,所以 pane
/// 自己的方角会戳出 zone 的圆角 CARD 背景,在四角形成小尖角。让 pane 的外
/// 圆角跟随 zone 圆角(半径减掉 zone 内边距),方角就被收进圆角里,只在外
/// 侧那一边收(`corner` 决定),配对的内部接缝仍是方角(本来就藏在 zone 内)。
pub(crate) fn zone_pane_border(zone: theme::region::RegionStyle, corner: PaneCorner) -> Border {
    let r = zone.border.map(|b| b.radius.top_left).unwrap_or(0.0);
    let r = (r - zone.padding.top).max(0.0);
    let radius = match corner {
        PaneCorner::None => Radius::from(0.0),
        PaneCorner::All => Radius::from(r),
        PaneCorner::Left => Radius {
            top_left: r,
            bottom_left: r,
            ..Radius::from(0.0)
        },
        PaneCorner::Right => Radius {
            top_right: r,
            bottom_right: r,
            ..Radius::from(0.0)
        },
    };
    Border {
        color: Color::TRANSPARENT,
        width: 0.0,
        radius,
    }
}

/// 渲染"某一种面板"的内容——`left_panel_area`/`right_panel_area` 共用。
///
/// Stage 4a 给图标栏面板加了跨栏拖拽后,`left_view` 可以是原先挂右栏的
/// `Agent`/`Conversations`/`Usage`/`Acceptance`,`right_view` 也可以是原先
/// 挂左栏的 `Files`/`GitLog`/`Todo`/`Project`/`Database`/`Ssh`/`Web`——
/// 之前两侧各自 `match` 里那行 `_ => unreachable!("Stage 1 ... 面板还固定
/// 在各自原侧")` 已经不成立,再碰到跨栏后的对侧面板会在渲染期直接 abort
/// (GUI 拖拽核对抓到的崩溃)。
///
/// 所以把"渲染一个面板"抽到这里做穷尽 `match`。每个分支只依赖
/// `zone`(外框主题与内部分割线配色)和 `lc/rc/ac`(pane 圆角朝向),由两侧
/// 各自传入自己那一侧的主题——除此之外同一面板在左/右栏渲染完全一致
/// (内部分割线用的 `Divider` variant 是面板固有属性,和挂哪条栏无关;
/// `panel_mirrored(kind)` 已经按当前实际所在栏算出是否镜像、自行翻转
/// `row!` 顺序)。各面板分割线的几何(`apply_column_drag`)已经由
/// Task 5/6 做成 side+镜像感知,这里只需正确渲染,无需再按左/右分支。
fn panel_body<'a>(
    app: &'a App,
    ws: &'a Workspace,
    kind: PanelKind,
    zone: theme::region::RegionStyle,
    lc: PaneCorner,
    rc: PaneCorner,
    ac: PaneCorner,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    match kind {
        PanelKind::Files => {
            if app.dims.files_tree_collapsed {
                // 收起文件树:整个配对宽度都交给预览,项目树列表与分隔线都不
                // 渲染。`files_split` 比例保留,展开时按原比例恢复。
                return preview_pane(app, ws, Length::Fill, zone_pane_border(zone, ac));
            }
            let (list_portion, content_portion) = split_portions(app.dims.files_split);
            let list_pane: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
                if ws.project.is_some() {
                    files::view(
                        &ws.files,
                        Length::FillPortion(list_portion),
                        zone_pane_border(zone, lc),
                        app.hover_progress(HoverId::FilesSearchSubmit),
                        app.hover_progress(HoverId::FilesDotfiles),
                        app.hover_progress(HoverId::FilesBranchSwitch),
                    )
                    .map(Message::Files)
                } else {
                    no_project_placeholder(
                        ws,
                        Length::FillPortion(list_portion),
                        zone_pane_border(zone, lc),
                    )
                };
            let preview = preview_pane(
                app,
                ws,
                Length::FillPortion(content_portion),
                zone_pane_border(zone, rc),
            );
            let list_bg = theme::region::project_pane()
                .background
                .unwrap_or(byteui::theme::color::current().bg);
            let preview_bg = theme::region::preview_pane()
                .background
                .unwrap_or(byteui::theme::color::current().bg);
            if app.panel_mirrored(PanelKind::Files) {
                row![
                    preview,
                    divider_bar(
                        Divider::LeftPairSplit,
                        preview_bg,
                        list_bg,
                        Message::ColumnDragStart(Divider::LeftPairSplit),
                    ),
                    list_pane,
                ]
                .width(Length::Fill)
                .into()
            } else {
                row![
                    list_pane,
                    divider_bar(
                        Divider::LeftPairSplit,
                        list_bg,
                        preview_bg,
                        Message::ColumnDragStart(Divider::LeftPairSplit),
                    ),
                    preview,
                ]
                .width(Length::Fill)
                .into()
            }
        }
        PanelKind::GitLog => git_log::view(
            app,
            &app.git_log,
            app.dims.git_log_split,
            app.dims.git_log_file_diff_split,
            app.panel_mirrored(PanelKind::GitLog),
        )
        .map(Message::GitLog),
        PanelKind::Todo => {
            let collapsed = app.list_collapsed(PanelKind::Todo);
            // 列表列收起:内容拿满整个配对宽度,侧栏不渲染。
            if collapsed {
                return todo::view(
                    app,
                    &ws.todo,
                    ws,
                    Length::Fixed(0.0),
                    Border::default(),
                    Length::Fill,
                    zone_pane_border(zone, ac),
                )
                .1
                .map(Message::Todo);
            }
            let (list_portion, content_portion) = split_portions(app.dims.todo_split);
            let (sidebar_pane, content_pane) = todo::view(
                app,
                &ws.todo,
                ws,
                Length::FillPortion(list_portion),
                zone_pane_border(zone, lc),
                Length::FillPortion(content_portion),
                zone_pane_border(zone, rc),
            );
            let sidebar = sidebar_pane.map(Message::Todo);
            let content = content_pane.map(Message::Todo);
            let sidebar_bg = theme::region::project_pane()
                .background
                .unwrap_or(byteui::theme::color::current().bg);
            let content_bg = theme::region::preview_pane()
                .background
                .unwrap_or(byteui::theme::color::current().bg);
            if app.panel_mirrored(PanelKind::Todo) {
                row![
                    content,
                    divider_bar(
                        Divider::TodoSplit,
                        content_bg,
                        sidebar_bg,
                        Message::ColumnDragStart(Divider::TodoSplit),
                    ),
                    sidebar,
                ]
                .width(Length::Fill)
                .into()
            } else {
                row![
                    sidebar,
                    divider_bar(
                        Divider::TodoSplit,
                        sidebar_bg,
                        content_bg,
                        Message::ColumnDragStart(Divider::TodoSplit),
                    ),
                    content,
                ]
                .width(Length::Fill)
                .into()
            }
        }
        PanelKind::Project => {
            if app.list_collapsed(PanelKind::Project) {
                return project_preview_pane(app, ws, Length::Fill, zone_pane_border(zone, ac));
            }
            let (list_portion, content_portion) = split_portions(app.dims.project_split);
            let info_pane: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
                project::view(
                    &ws.project_panel,
                    ws.project.as_ref(),
                    Length::FillPortion(list_portion),
                    zone_pane_border(zone, lc),
                )
                .map(Message::Project);
            let preview = project_preview_pane(
                app,
                ws,
                Length::FillPortion(content_portion),
                zone_pane_border(zone, rc),
            );
            let info_bg = theme::region::project_pane()
                .background
                .unwrap_or(byteui::theme::color::current().bg);
            let preview_bg = theme::region::preview_pane()
                .background
                .unwrap_or(byteui::theme::color::current().bg);
            if app.panel_mirrored(PanelKind::Project) {
                row![
                    preview,
                    divider_bar(
                        Divider::ProjectSplit,
                        preview_bg,
                        info_bg,
                        Message::ColumnDragStart(Divider::ProjectSplit),
                    ),
                    info_pane,
                ]
                .width(Length::Fill)
                .into()
            } else {
                row![
                    info_pane,
                    divider_bar(
                        Divider::ProjectSplit,
                        info_bg,
                        preview_bg,
                        Message::ColumnDragStart(Divider::ProjectSplit),
                    ),
                    preview,
                ]
                .width(Length::Fill)
                .into()
            }
        }
        PanelKind::Database => {
            // 数据库面板需要项目已打开才能读写 `.dozer/database.json`。
            if ws.project.is_none() {
                return column![].into();
            }
            if app.list_collapsed(PanelKind::Database) {
                return database::content_pane(
                    app,
                    &ws.database,
                    Length::Fill,
                    zone_pane_border(zone, ac),
                )
                .map(Message::Database);
            }
            let (list_portion, content_portion) = split_portions(app.dims.database_split);
            let list_pane = database::view(
                &ws.database,
                Length::FillPortion(list_portion),
                zone_pane_border(zone, lc),
            )
            .map(Message::Database);
            let content_pane = database::content_pane(
                app,
                &ws.database,
                Length::FillPortion(content_portion),
                zone_pane_border(zone, rc),
            )
            .map(Message::Database);
            let list_bg = theme::region::project_pane()
                .background
                .unwrap_or(byteui::theme::color::current().bg);
            let content_bg = theme::region::preview_pane()
                .background
                .unwrap_or(byteui::theme::color::current().bg);
            if app.panel_mirrored(PanelKind::Database) {
                row![
                    content_pane,
                    divider_bar(
                        Divider::DatabaseSplit,
                        content_bg,
                        list_bg,
                        Message::ColumnDragStart(Divider::DatabaseSplit),
                    ),
                    list_pane,
                ]
                .width(Length::Fill)
                .into()
            } else {
                row![
                    list_pane,
                    divider_bar(
                        Divider::DatabaseSplit,
                        list_bg,
                        content_bg,
                        Message::ColumnDragStart(Divider::DatabaseSplit),
                    ),
                    content_pane,
                ]
                .width(Length::Fill)
                .into()
            }
        }
        PanelKind::Ssh => {
            // 同 Files/Database 面板:`ws.project.is_none()` 是 Stub→Loaded
            // 促成期间的占位态。
            if ws.project.is_none() {
                return column![].into();
            }
            if app.list_collapsed(PanelKind::Ssh) {
                return ssh_terminal_pane(app, ws, Length::Fill, zone_pane_border(zone, ac));
            }
            let (list_portion, content_portion) = split_portions(app.dims.ssh_split);
            let list_pane = ssh::view(
                app,
                &ws.ssh,
                Length::FillPortion(list_portion),
                zone_pane_border(zone, lc),
            )
            .map(Message::Ssh);
            let terminal = ssh_terminal_pane(
                app,
                ws,
                Length::FillPortion(content_portion),
                zone_pane_border(zone, rc),
            );
            let list_bg = theme::region::project_pane()
                .background
                .unwrap_or(byteui::theme::color::current().bg);
            let terminal_bg = theme::region::preview_pane()
                .background
                .unwrap_or(byteui::theme::color::current().bg);
            if app.panel_mirrored(PanelKind::Ssh) {
                row![
                    terminal,
                    divider_bar(
                        Divider::SshSplit,
                        terminal_bg,
                        list_bg,
                        Message::ColumnDragStart(Divider::SshSplit),
                    ),
                    list_pane,
                ]
                .width(Length::Fill)
                .into()
            } else {
                row![
                    list_pane,
                    divider_bar(
                        Divider::SshSplit,
                        list_bg,
                        terminal_bg,
                        Message::ColumnDragStart(Divider::SshSplit),
                    ),
                    terminal,
                ]
                .width(Length::Fill)
                .into()
            }
        }
        PanelKind::Web => browser::view(
            &ws.browser,
            ws.project.as_ref().map(|p| p.id),
            app.dims.browser_bookmarks_split,
            Length::Fill,
            zone_pane_border(zone, ac),
            app.panel_mirrored(PanelKind::Web),
        )
        .map(Message::Browser),
        PanelKind::Agent => {
            if app.list_collapsed(PanelKind::Agent) {
                return terminal::terminal_pane(app, ws, Length::Fill, zone_pane_border(zone, ac));
            }
            let (list_portion, content_portion) = split_portions(app.dims.agent_split);
            let terminal = terminal::terminal_pane(
                app,
                ws,
                Length::FillPortion(content_portion),
                zone_pane_border(zone, lc),
            );
            let list = agent_list_pane(
                app,
                ws,
                Length::FillPortion(list_portion),
                zone_pane_border(zone, rc),
            );
            let terminal_bg = theme::region::terminal_pane()
                .background
                .unwrap_or(byteui::theme::color::current().bg);
            let list_bg = theme::region::agent_list_pane()
                .background
                .unwrap_or(byteui::theme::color::current().bg);
            if app.panel_mirrored(PanelKind::Agent) {
                row![
                    list,
                    divider_bar(
                        Divider::RightPairSplit,
                        list_bg,
                        terminal_bg,
                        Message::ColumnDragStart(Divider::RightPairSplit),
                    ),
                    terminal,
                ]
                .width(Length::Fill)
                .into()
            } else {
                row![
                    terminal,
                    divider_bar(
                        Divider::RightPairSplit,
                        terminal_bg,
                        list_bg,
                        Message::ColumnDragStart(Divider::RightPairSplit),
                    ),
                    list,
                ]
                .width(Length::Fill)
                .into()
            }
        }
        PanelKind::Conversations => {
            if app.list_collapsed(PanelKind::Conversations) {
                return review_content_pane(app, ws, Length::Fill, zone_pane_border(zone, ac));
            }
            let (list_portion, content_portion) = split_portions(app.dims.conversations_split);
            let review = review_content_pane(
                app,
                ws,
                Length::FillPortion(content_portion),
                zone_pane_border(zone, lc),
            );
            let list = conversations::view(
                app,
                &ws.conversations,
                ws.todo.items(),
                &ws.open_transcript_paths(),
                Length::FillPortion(list_portion),
                zone_pane_border(zone, rc),
            )
            .map(Message::Conversations);
            let review_bg = theme::region::review_content_pane()
                .background
                .unwrap_or(byteui::theme::color::current().bg);
            let list_bg = theme::region::conversation_list_pane()
                .background
                .unwrap_or(byteui::theme::color::current().bg);
            if app.panel_mirrored(PanelKind::Conversations) {
                row![
                    list,
                    divider_bar(
                        Divider::RightPairSplit,
                        list_bg,
                        review_bg,
                        Message::ColumnDragStart(Divider::RightPairSplit),
                    ),
                    review,
                ]
                .width(Length::Fill)
                .into()
            } else {
                row![
                    review,
                    divider_bar(
                        Divider::RightPairSplit,
                        review_bg,
                        list_bg,
                        Message::ColumnDragStart(Divider::RightPairSplit),
                    ),
                    list,
                ]
                .width(Length::Fill)
                .into()
            }
        }
        PanelKind::Usage => {
            // 加载中/还没数据时没有 agent 筛选栏可拼(同改造前
            // `sidebar: Option<..>` 为 `None` 时的行为),内容侧独占全宽。
            if !ws.usage.has_agent_filter() {
                return usage::content_pane(
                    app,
                    &ws.usage,
                    Length::Fill,
                    zone_pane_border(zone, ac),
                )
                .map(Message::Usage);
            }
            if app.list_collapsed(PanelKind::Usage) {
                return usage::content_pane(
                    app,
                    &ws.usage,
                    Length::Fill,
                    zone_pane_border(zone, ac),
                )
                .map(Message::Usage);
            }
            let (list_portion, content_portion) = split_portions(app.dims.usage_split);
            let content_pane = usage::content_pane(
                app,
                &ws.usage,
                Length::FillPortion(content_portion),
                zone_pane_border(zone, lc),
            )
            .map(Message::Usage);
            let list_pane = usage::list_pane(
                &ws.usage,
                Length::FillPortion(list_portion),
                zone_pane_border(zone, rc),
            )
            .map(Message::Usage);
            let content_bg = byteui::theme::color::current().panel;
            let list_bg = byteui::theme::color::current().bg;
            if app.panel_mirrored(PanelKind::Usage) {
                row![
                    list_pane,
                    divider_bar(
                        Divider::UsageSplit,
                        list_bg,
                        content_bg,
                        Message::ColumnDragStart(Divider::UsageSplit),
                    ),
                    content_pane,
                ]
                .width(Length::Fill)
                .into()
            } else {
                row![
                    content_pane,
                    divider_bar(
                        Divider::UsageSplit,
                        content_bg,
                        list_bg,
                        Message::ColumnDragStart(Divider::UsageSplit),
                    ),
                    list_pane,
                ]
                .width(Length::Fill)
                .into()
            }
        }
    }
}

/// 左面板区:按当前左视图组合"项目树+文件预览"配对或单个 Web 预览面板;
/// 收起时渲染成空元素(不占宽度)。
///
/// 宽度语义与 `left_zone_width` 严格对应:对侧收起时本区 `Fill` 独占
/// `zones_width`(否则整行会缩到"两条图标栏+一条分隔线"那么宽,右图标栏
/// 跑到窗口中间去);两侧都收起时由本区出一个 `Fill` 空白把窗口撑满;
/// 常规态用 `Workspace::effective_left_width()`——**不是**直接读持久化的
/// `dims.left_width`。持久化宽可能大过当前窗口容得下的范围(用户
/// 在大窗口拖宽后把窗口缩小),那样这条 `Length::Fixed` 会在 flex 第一趟
/// 把可用空间吃光,唯一 `Fill` 的右面板区拿到 0 宽(Fix round 2 Critical #1)。
///
/// `maximized`(放大态用):为 `true` 时强制 `Length::Fill`,不看持久化宽/
/// 对侧收起态——`maximize_overlay` 需要这块区域真正撑满整个放大盒子,而不是
/// 停在平时拖拽出来的 `left_width` 那么宽。放大态下 `left_collapsed` 仍可能
/// 为真(旧注释断言"恒为 false"是错的:放大浮层不拦图标栏点击,先放大再点
/// 图标收起本侧是可达路径),此时上面那条收起分支返回空元素;`maximized`
/// 会被 `PanelSelect` 无条件清掉,所以这个组合不会
/// 停留超过一帧(Fix round 2 #2)。
/// 非放大态下,左1(项目树/Web)+左2(预览)两栏被视觉框成一个整体,套
/// `theme::region::left_zone()` 的外框(四向 margin 做悬浮留白,无描边)。
/// 放大态跳过——`maximize_overlay` 已经用金色边框把同一块内容整体框起来,
/// 再套一层外框会在金框内侧多出一圈视觉噪音。
fn left_panel_area<'a>(
    app: &'a App,
    ws: &'a Workspace,
    maximized: bool,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    if app.left_collapsed {
        return if app.right_collapsed {
            iced_widget::space::horizontal().into()
        } else {
            column![].into()
        };
    }
    let total = if maximized || app.right_collapsed {
        Length::Fill
    } else {
        Length::Fixed(app.effective_left_width())
    };
    let zone = theme::region::left_zone();
    let (lc, rc, ac) = if maximized {
        (PaneCorner::None, PaneCorner::None, PaneCorner::None)
    } else {
        (PaneCorner::Left, PaneCorner::Right, PaneCorner::All)
    };
    let inner = panel_body(app, ws, app.left_view, zone, lc, rc, ac);
    if maximized {
        return inner;
    }
    let region = zone;
    // 其它 worktree 切换条已从 Git Log 面板移除(用户需求),这里不再包任何
    // 额外层,直接透传面板本体。
    let zone_body: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> = inner;
    // 聚焦态外框:本 zone 拿到焦点(= `active_zone`)时描 GOLD 边,否则沿用
    // region 的默认(无描边)外框。半径保持与默认外框一致。
    let left_focused = app.active_zone == Some(ZoneSide::Left);
    let base_border = region.border.unwrap_or_default();
    let zone_box = container(zone_body)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(region.padding)
        .clip(true)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: if left_focused {
                Border {
                    color: byteui::theme::color::current().gold,
                    width: 2.0,
                    radius: base_border.radius,
                }
            } else {
                base_border
            },
            ..container::Style::default()
        });
    // 四向 margin:把整块外边框从顶栏/窗口底/图标栏/对侧分隔条各推开一段,
    // 做出悬浮留白。左右 margin 来自 `left_zone` 配置(默认左 8、右 0)。
    let m = region.margin;
    container(zone_box)
        .width(total)
        .height(Length::Fill)
        .padding(Padding {
            top: m.top,
            right: m.right,
            bottom: m.bottom,
            left: m.left,
        })
        .into()
}

/// 右面板区:按当前右视图组合"Agent 列表+终端"或"对话列表+对话审阅"配对;
/// 收起时渲染成空元素。总宽恒为剩余空间(`Fill`),不像左面板区那样有持久化
/// 的固定像素宽——所以内部分割只能用 `FillPortion` 表达,不能预先算像素。
///
/// 两块 pane 的宽度直接由它们自己的外层容器声明成 `FillPortion`,不再套一层
/// 包装容器:`Limits::width(Fixed(w))` 会把子元素的 min/max 都钉成 `w`,父级
/// 的 `FillPortion` 只约束包装容器本身、传不进子元素,曾导致这四块 pane 全部
/// 以 0 宽布局(右半边整片空白)。
///
/// 非放大态下,右1(Agent 列表/对话列表)+右2(终端/审阅)两栏被视觉框成
/// 一个整体,套 `theme::region::right_zone()` 的外框(四向 margin 做悬浮留白,
/// 无描边),`maximized` 时跳过(理由同 `left_panel_area`)。
fn right_panel_area<'a>(
    app: &'a App,
    ws: &'a Workspace,
    maximized: bool,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    if app.right_collapsed {
        return column![].into();
    }
    // 两个配对都是"内容侧渲染在左、列表侧渲染在右"——终端在左/Agent 列表
    // 在右,审阅在左/对话列表在右。`agent_split`/`conversations_split` 仍是
    // "列表侧(Agent 列表/对话列表)占右面板区宽度的比例"这个原有语义不变
    // (`terminal_pane_pixel_size` 等既有几何公式全靠它,不能跟着挪);只是
    // `content_portion`(∝ 1-split)现在给左边那块、`list_portion`(∝ split)
    // 给右边那块,单纯是 `row!` 里两个 pane 的先后顺序换了。`apply_column_drag`
    // 的 `RightPairSplit` 分支要相应把算出来的 ratio 取反再写回,否则拖拽
    // 方向感会反过来(见该函数注释)。
    let zone = theme::region::right_zone();
    let (lc, rc, ac) = if maximized {
        (PaneCorner::None, PaneCorner::None, PaneCorner::None)
    } else {
        (PaneCorner::Left, PaneCorner::Right, PaneCorner::All)
    };
    let inner = panel_body(app, ws, app.right_view, zone, lc, rc, ac);
    if maximized {
        return inner;
    }
    let region = zone;
    // 聚焦态外框:本 zone 拿到焦点(= `active_zone`)时描 GOLD 边,否则沿用
    // region 的默认(无描边)外框。半径保持与默认外框一致。
    let right_focused = app.active_zone == Some(ZoneSide::Right);
    let base_border = region.border.unwrap_or_default();
    let zone_box = container(inner)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(region.padding)
        .clip(true)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: if right_focused {
                Border {
                    color: byteui::theme::color::current().gold,
                    width: 2.0,
                    radius: base_border.radius,
                }
            } else {
                base_border
            },
            ..container::Style::default()
        });
    // 四向 margin:同 `left_panel_area`,左右 margin 来自 `right_zone` 配置
    // (默认左 0、右 8)。
    let m = region.margin;
    container(zone_box)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(Padding {
            top: m.top,
            right: m.right,
            bottom: m.bottom,
            left: m.left,
        })
        .into()
}

/// 放大态浮层:两条图标栏之间的整个内容区变暗+背景虚化，放大的那一侧
/// 内容(左/右面板区，含其内部列表:内容子分隔线，原样渲染，只是占满整个
/// 中间区域)金色描边突出。点变暗区域(放大内容之外的部分)退出放大。
fn maximize_overlay<'a>(
    app: &'a App,
    ws: &'a Workspace,
    which: MaximizedPane,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let inner = match which {
        MaximizedPane::Left => left_panel_area(app, ws, true),
        MaximizedPane::Right => right_panel_area(app, ws, true),
    };
    // `bordered` 显式给 `Length::Fill`(不留给默认 `Length::Shrink`)——
    // iced 0.14 的 `Limits` 有个"compression"传染机制:一个 `Shrink` 容器
    // 包一个 `Fill`/`FillPortion` 子元素时,子元素的 Fill 不会展开到可用
    // 空间,而是退化成"贴着内容收缩"(`Limits::resolve` 对 Fill 的展开分支
    // 要求 `!compression`,`Shrink` 会把 compression 设 true 并一路往下传,
    // 直到遇到一个显式 `Length::Fixed` 才重置)。这里如果不显式给 Fill,
    // `inner`(`left_panel_area`/`right_panel_area` 内部大量 FillPortion
    // 组成)会整体收缩成远小于放大盒子的intrinsic 尺寸,金色描边就会贴着
    // 一小块内容而不是撑满两条图标栏之间的放大区域。
    let overlay_style = theme::region::maximize_overlay();
    let bordered = container(inner)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            border: overlay_style.border,
            ..container::Style::default()
        });
    // 放大内容自己再罩一层"吃掉点击/滚轮"的 MouseArea,拦住它们冒泡到外层
    // dim 遮罩的 `MaximizeClose`(Fix round 2 #4)。iced 的事件是子先父后:
    // 内容里真正可交互的控件(按钮、终端 canvas)会先自己 capture,压根到不了
    // 这一层;到得了这一层的正是"不消费点击的地方"——审阅正文、卡片下方空白、
    // 列表空处——此前它们会一路穿到 dim 遮罩上,点一下正文就把放大退掉,而
    // 放大审阅恰恰是这个功能存在的理由。滚轮同理:内部 scrollable 真滚动了
    // 会自己 capture,滚不动时旧行为是穿到下层(基础层那块被遮住的终端
    // canvas)去滚终端历史,这里一并吃掉。
    let content_guard = MouseArea::new(bordered)
        .on_press(Message::Noop)
        .on_scroll(|_delta| Message::Noop);
    let dim_bg = MouseArea::new(
        container(content_guard)
            .padding(overlay_style.scrim_padding)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_t: &iced_widget::Theme| container::Style {
                background: Some(overlay_style.scrim_background.into()),
                ..container::Style::default()
            }),
    )
    .on_press(Message::MaximizeClose);

    // 顶部垫一条透明的 `byteui::theme::geometry::top_bar_height()` 高 Space,把变暗遮罩钉在顶栏
    // 之下——`base = column![top, body]` 里顶栏和内容区就是这么分的,
    // 这里镜像同一结构,让变暗区域精确对齐 `body` 的渲染范围,不覆盖顶栏
    // (Important:此前没有这条 Space,遮罩会盖住整个窗口高度,连顶栏的
    // 项目 tab 等控件都会被染黑)。
    column![
        iced_widget::space::Space::new()
            .height(Length::Fixed(byteui::theme::geometry::top_bar_height())),
        row![
            iced_widget::space::Space::new()
                .width(Length::Fixed(byteui::theme::geometry::icon_rail_width())),
            dim_bg,
            iced_widget::space::Space::new()
                .width(Length::Fixed(byteui::theme::geometry::icon_rail_width())),
        ],
    ]
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

/// 分隔线:命中区 `byteui::theme::geometry::divider_width()` 宽、`Length::Fill` 高,
/// 中间一条 2px BORDER 竖线。悬停变 resize 光标走 `MouseArea::interaction` →
/// iced 既有的 `mouse_interaction` → `window.set_cursor` 管线(main.rs:808-816
/// 已有),不必另起一套光标代码。`on_press` 只发起拖拽状态,不指望 `MouseArea`
/// 的 `on_move`/`on_release`——它们要求光标不离开这条窄带才触发,快速拖拽会
/// 在光标移出后"断掉";持续追踪交给 `main.rs` 原始事件层。
///
/// 配对视图(左1左2 / 右1右2)内部:`left_bg`/`right_bg` 是分隔线两侧紧贴的
/// pane 底色。命中区左右两半(各 `(divider_width-2)/2`)分别填上这两色,只留
/// 中间 2px BORDER 竖线——否则 8px 命中区是透明的,会露出 zone 的 CARD 底色,
/// 在两块 pane 之间顶出一条浅色"沟",看起来像多了 padding/margin。填色后两块
/// pane 视觉贴合、只剩一条分割线,命中区宽度(拖拽手感)不变。
///
/// `Divider::LeftRight` 不画那条 2px 竖线、也不填色——它两侧各自套了
/// `theme::region::left_zone()`/`right_zone()` 的整体外框,这条 8px 缝是故意
/// 空出来给两侧 zone 圆角边框各自收边的,不能填成某侧 pane 色。
pub(crate) fn divider_bar<'a, M: Clone + 'a>(
    divider: Divider,
    left_bg: Color,
    right_bg: Color,
    on_drag: M,
) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer> {
    let show_line = !matches!(divider, Divider::LeftRight);
    let body: Element<'_, M, iced_widget::Theme, iced_renderer::Renderer> = if !show_line {
        iced_widget::Space::new()
            .width(Length::Fixed(byteui::theme::geometry::divider_width()))
            .height(Length::Fill)
            .into()
    } else {
        let line_w = 2.0_f32;
        let side_w = (byteui::theme::geometry::divider_width() - line_w) / 2.0;
        let left_side = container(iced_widget::Space::new())
            .width(Length::Fixed(side_w))
            .height(Length::Fill)
            .style(move |_t: &iced_widget::Theme| container::Style {
                background: Some(left_bg.into()),
                ..container::Style::default()
            });
        let right_side = container(iced_widget::Space::new())
            .width(Length::Fixed(side_w))
            .height(Length::Fill)
            .style(move |_t: &iced_widget::Theme| container::Style {
                background: Some(right_bg.into()),
                ..container::Style::default()
            });
        let line = container(iced_widget::Space::new())
            .width(Length::Fixed(line_w))
            .height(Length::Fill)
            .style(|_t: &iced_widget::Theme| container::Style {
                background: Some(byteui::theme::color::current().border.into()),
                ..container::Style::default()
            });
        row![left_side, line, right_side]
            .width(Length::Fixed(byteui::theme::geometry::divider_width()))
            .height(Length::Fill)
            .into()
    };
    MouseArea::new(body)
        .interaction(mouse::Interaction::ResizingColumn)
        .on_press(on_drag)
        .into()
}

/// `divider_bar` 的纵向(上下)镜像:一条水平分割线,`row!`→`column!`、
/// `width`↔`height` 互换,鼠标样式 `ResizingRow`(对应横向的
/// `ResizingColumn`)。目前只有 Git Log 面板右侧"文件列表 | diff 内容"这条
/// 纵向拖拽线用它。粗细复用 `byteui::theme::geometry::divider_width()`,与横向一致。
pub(crate) fn horizontal_divider_bar<'a, M: Clone + 'a>(
    top_bg: Color,
    bottom_bg: Color,
    on_drag: M,
) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer> {
    let line_h = 2.0_f32;
    let side_h = (byteui::theme::geometry::divider_width() - line_h) / 2.0;
    let top_side = container(iced_widget::Space::new())
        .height(Length::Fixed(side_h))
        .width(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(top_bg.into()),
            ..container::Style::default()
        });
    let bottom_side = container(iced_widget::Space::new())
        .height(Length::Fixed(side_h))
        .width(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(bottom_bg.into()),
            ..container::Style::default()
        });
    let line = container(iced_widget::Space::new())
        .height(Length::Fixed(line_h))
        .width(Length::Fill)
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(byteui::theme::color::current().border.into()),
            ..container::Style::default()
        });
    let col = column![top_side, line, bottom_side]
        .height(Length::Fixed(byteui::theme::geometry::divider_width()))
        .width(Length::Fill);
    MouseArea::new(col)
        .interaction(mouse::Interaction::ResizingRow)
        .on_press(on_drag)
        .into()
}

/// tab 栏下方的 1px 分割线。
pub(crate) fn tab_divider<'a, M: 'a>() -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>
{
    byteui::layout::divider::horizontal()
}

/// 受控 tooltip:iced 0.14 的 `Tooltip` 没有"延迟显示"开关(它一悬停就弹),
/// 所以这里不靠 `Tooltip` 自带的 hover 检测,而是**仅在 `show` 为真时才把
/// `content` 包进 `Tooltip`**——调用方按"悬停满 2s"算好 `show`(见
/// `App::hover_tooltip_ready` / `browser::State::hover_tooltip_ready`),满 2s
/// 那一刻视图层才挂载 `Tooltip`,气泡随即弹出;离开即 `show` 为假,直接返回
/// 裸 `content`,气泡消失。`position` 由调用方按页签位置定(顶栏页签用
/// `Bottom`、底部面板页签用 `Top`,免得气泡出屏)。`label` 收 `String`(拥有
/// 所有权),使气泡 `Element` 寿命不受调用方局部借用牵制,`Tooltip` 才能正常
/// 把它当 overlay 渲染。
pub(crate) fn controlled_tooltip<'a, M, R>(
    content: Element<'a, M, iced_widget::Theme, R>,
    label: String,
    position: Position,
    show: bool,
) -> Element<'a, M, iced_widget::Theme, R>
where
    M: Clone + 'a,
    R: iced_widget::core::text::Renderer + 'a,
{
    // 未悬停满 2s:不包 tooltip,直接返回裸内容,避免一悬停就弹气泡打扰。
    if !show {
        return content;
    }
    let bubble = container(
        text(label)
            .size(12)
            .color(byteui::theme::color::current().cream),
    )
    .padding([5, 9]);
    Tooltip::new(content, bubble, position)
        .gap(4)
        .style(icons::tooltip_bubble_style())
        .into()
}

/// `host_id` → `HoverId::SshTab{Item,Close}` 用的哈希键(`HoverId` 整体
/// `derive(Copy)`,`String` 不是 `Copy`,退化成 `u64`,不要求无碰撞——
/// 碰撞在同一台主机的 tab hover 高亮场景下不构成实际风险)。
pub(crate) fn ssh_tab_hover_key(host_id: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    host_id.hash(&mut hasher);
    hasher.finish()
}

/// SSH 面板自己的 tab 条:固定一个"空白"占位 tab 打头,后面遍历
/// `ws.ssh_tabs`/`ws.sftp_tabs`,每个渲染一个可关闭 tab。直接复用
/// `panel_tab`(右侧共享终端条 `tab_item` 用的同一个函数)而不是自己拼
/// 容器样式,视觉/hover 动画与全应用其它 tab 完全一致——不需要
/// `tabs::tab_core` 手动接线。前缀图标固定用 `IconKind::Terminal`(阶段
/// 4 只有这一种;阶段 3 加 `Sftp` 变体后按 tab 的种类换图标,`SessionTab`
/// 本身不带 `SshTabKind` 字段,种类信息只在 `ws.ssh_active` 里——阶段 4
/// 全部 `ssh_tabs` 里的 tab 都是 `Terminal` 种类,这里暂时不需要按 tab
/// 查种类,阶段 3 扩展这个函数时才需要处理"同一个 host_id 可能对应两个
/// 不同种类的 tab,要分别渲染两个 tab 条目"这件事)。
///
/// 翻页箭头 + 窗口化裁剪(P1L T5 那套 `tab_window` 索引窗口)镜像
/// `workspace.rs::preview_pane_for`:先把全部 tab 元素连同估算宽度收进
/// `entries`,再用 `tab_window` 算出可视窗口起点 `first`,只渲染
/// `entries[first..]`,左右箭头到头置灰。原先没有这套窗口化,tab 一多
/// 就会被右侧"收起列表"按钮的 `clip` 直接裁没、连滚动入口都没有。
fn ssh_tab_bar<'a>(
    app: &'a App,
    ws: &'a Workspace,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let mut entries: Vec<(
        f32,
        Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>,
    )> = Vec::new();
    // "空白"占位 tab:不对应 `ssh_tabs`/`sftp_tabs` 里任何一条记录,选中
    // 态即 `ssh_active == None`(未开任何主机 tab,或关到最后一个后的
    // 默认落点)。跟文件预览面板 `preview.rs::TabKind::Blank` 是同一个
    // 产品概念,但这边没有对应的轻量 tab 数据可插进 `ssh_tabs`,所以只在
    // 这里画一个固定条目,内容侧靠 `ssh_active == None` 分支渲染
    // `ssh_empty_state()`,不需要真的建一个 tab 结构体。用 `""` 当 hover
    // key(真实 host_id 是 UUID,不会是空串,不会撞)。
    let blank_key = ssh_tab_hover_key("");
    let blank_active = ws.ssh_active.is_none();
    entries.push((
        tab_display_width("空白"),
        tab_widget::panel_tab(tab_widget::PanelTabArgs {
            title: "空白".to_string(),
            active: blank_active,
            hover_t: app.hover_progress(HoverId::SshTabItem(blank_key)),
            close_hover_t: app.hover_progress(HoverId::SshTabClose(blank_key)),
            prefix: None,
            suffix: None,
            on_select: Message::Ssh(ssh::Message::SelectBlankTab),
            on_close: Message::Ssh(ssh::Message::SelectBlankTab),
            show_tooltip: app.hover_tooltip_ready(HoverId::SshTabItem(blank_key)),
            title_hover: move |h| Message::Hover(HoverId::SshTabItem(blank_key), h),
            close_hover: move |h| Message::Hover(HoverId::SshTabClose(blank_key), h),
        }),
    ));
    for tab in &ws.ssh_tabs {
        let host_id = tab
            .info
            .id
            .strip_prefix("ssh:")
            .unwrap_or(&tab.info.id)
            .to_string();
        let is_active = ws
            .ssh_active
            .as_ref()
            .is_some_and(|(h, k)| h == &host_id && *k == ssh::SshTabKind::Terminal);
        let key = ssh_tab_hover_key(&host_id);
        let title_hover_t = app.hover_progress(HoverId::SshTabItem(key));
        let close_hover_t = app.hover_progress(HoverId::SshTabClose(key));
        let icon = icons::view(
            icons::IconKind::Terminal,
            byteui::theme::icon_size::row(),
            byteui::theme::color::current().dim,
        );
        let select_id = host_id.clone();
        let close_id = host_id.clone();
        let title_hover_id = host_id.clone();
        let close_hover_id = host_id;
        let title = tab_title(tab.agent, tab.cwd.as_deref(), &tab.info.name);
        entries.push((
            tab_display_width(&title),
            tab_widget::panel_tab(tab_widget::PanelTabArgs {
                title,
                active: is_active,
                hover_t: title_hover_t,
                close_hover_t,
                prefix: Some(icon),
                suffix: None,
                on_select: Message::Ssh(ssh::Message::SelectSshTab(
                    select_id,
                    ssh::SshTabKind::Terminal,
                )),
                on_close: Message::Ssh(ssh::Message::CloseSshTab(
                    close_id,
                    ssh::SshTabKind::Terminal,
                )),
                show_tooltip: app.hover_tooltip_ready(HoverId::SshTabItem(key)),
                title_hover: move |h| {
                    Message::Hover(HoverId::SshTabItem(ssh_tab_hover_key(&title_hover_id)), h)
                },
                close_hover: move |h| {
                    Message::Hover(HoverId::SshTabClose(ssh_tab_hover_key(&close_hover_id)), h)
                },
            }),
        ));
    }
    // SFTP tab(阶段 3):`sftp_tabs` 按 host_id 去重,渲染形状跟终端 tab
    // 一致(复用 `panel_tab`/`tab_core`),只是图标用 FolderSync、标题用主机名。
    for (host_id, state) in &ws.sftp_tabs {
        let is_active = ws.ssh_active.as_ref() == Some(&(host_id.clone(), ssh::SshTabKind::Sftp));
        let label = ws
            .ssh
            .hosts()
            .iter()
            .find(|h| &h.id == host_id)
            .map(|h| h.name.clone())
            .unwrap_or_else(|| host_id.clone());
        let key = ssh_tab_hover_key(host_id);
        let title_hover_t = app.hover_progress(HoverId::SshTabItem(key));
        let close_hover_t = app.hover_progress(HoverId::SshTabClose(key));
        let icon = icons::view(
            icons::IconKind::FolderSync,
            byteui::theme::icon_size::row(),
            byteui::theme::color::current().dim,
        );
        let select_id = host_id.clone();
        let close_id = host_id.clone();
        let title_hover_id = host_id.clone();
        let close_hover_id = host_id.clone();
        entries.push((
            tab_display_width(&label),
            tab_widget::panel_tab(tab_widget::PanelTabArgs {
                title: label,
                active: is_active,
                hover_t: title_hover_t,
                close_hover_t,
                prefix: Some(icon),
                suffix: None,
                on_select: Message::Ssh(ssh::Message::SelectSshTab(
                    select_id,
                    ssh::SshTabKind::Sftp,
                )),
                on_close: Message::Ssh(ssh::Message::CloseSshTab(close_id, ssh::SshTabKind::Sftp)),
                show_tooltip: app.hover_tooltip_ready(HoverId::SshTabItem(key)),
                title_hover: move |h| {
                    Message::Hover(HoverId::SshTabItem(ssh_tab_hover_key(&title_hover_id)), h)
                },
                close_hover: move |h| {
                    Message::Hover(HoverId::SshTabClose(ssh_tab_hover_key(&close_hover_id)), h)
                },
            }),
        ));
        let _ = state;
    }
    let widths: Vec<f32> = entries.iter().map(|(w, _)| *w).collect();
    let window = tab_widget::tab_window(
        &widths,
        4.0,
        byteui::theme::geometry::tab_bar_avail_px(),
        ws.ssh_tab_first,
    );
    let items: Vec<_> = entries
        .into_iter()
        .enumerate()
        .filter(|(idx, _)| (window.first..window.visible_end).contains(idx))
        .map(|(_, (_, el))| el)
        .collect();
    // tab 条本身占 Fill、裁掉右侧溢出,让"收起列表"钉在裁剪区外的最右侧
    // (镜像 `workspace.rs::preview_pane_for` 的 `clipped`/`collapse` 布局
    // ——之前 `bar` 整体是 `Shrink`,收起按钮只是跟在最后一个 tab 后面,
    // tab 少时会贴在中间而不是面板右边缘,验收反馈要求钉死在右侧)。
    let clipped = container(row(items).spacing(4))
        .width(Length::Fill)
        .clip(true);
    // V 只数**真实** tab(终端 + SFTP),不算"空白"占位——下拉本就不列空白
    // (点开也没什么可跳的,见 `ssh_tab_overflow_popup`),只剩空白页时 V
    // 本身也不该显示(验收反馈)。
    let ssh_tab_total = ws.ssh_tabs.len() + ws.sftp_tabs.len();
    let overflow_button = tab_widget::tab_overflow_button(
        ssh_tab_total,
        app.hover_progress(HoverId::SshTabOverflow),
        Message::Ssh(ssh::Message::TabOverflowToggle),
        move |hovered| Message::Hover(HoverId::SshTabOverflow, hovered),
    );
    // 内容侧"收起/展开列表列"按钮(收起左列主机列表后仍在此可见以便恢复)。
    let collapse = app.list_collapse_button(
        PanelKind::Ssh,
        app.list_collapsed(PanelKind::Ssh),
        HoverId::SshListCollapse,
        "收起列表",
        "展开列表",
        Message::TogglePanelListCollapse(PanelKind::Ssh),
        move |hovered| Message::Hover(HoverId::SshListCollapse, hovered),
    );
    let mut tab_bar_row = row![]
        .spacing(4)
        .align_y(iced_widget::core::Alignment::Center);
    if let Some(btn) = overflow_button {
        tab_bar_row = tab_bar_row.push(btn);
    }
    let tab_bar = tab_bar_row.push(clipped).push(collapse);

    let base: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> = tab_bar.into();
    base
}

/// SSH 面板 tab 栏"溢出下拉"浮层。**必须**在 `App::view` 顶层
/// `stack![base, ...]` 里拼(同 `terminal::term_tab_overflow_popup` 文档
/// 解释的理由——`anchor`/`window_size` 是全窗口坐标系,嵌在 `ssh_tab_bar`
/// 自己的局部布局里换算位置会跟真实点击位置对不上)。
fn ssh_tab_overflow_popup<'a>(
    app: &'a App,
    ws: &'a Workspace,
) -> Option<Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>> {
    let anchor = ws.ssh_tab_overflow_anchor?;
    // 下拉列出该 SSH 面板内**真实** tab(终端 + SFTP),即 horizontal tab
    // 条之上的完整视图——点 V 不是为了翻越隐藏项,而是一览/跳到任意 tab。
    // "空白"占位不进列表(点开也没什么可跳的);下标沿用
    // `ssh_tab_overflow_select_message`/`_close_message` 既有的 1 起步方案
    // (0 留给空白,虽然它现在不会出现在列表里,翻译函数不用跟着改)。
    let mut entries: Vec<tab_widget::TabOverflowEntry<'_, Message>> = Vec::new();
    for (i, tab) in ws.ssh_tabs.iter().enumerate() {
        let idx = i + 1;
        let host_id = tab
            .info
            .id
            .strip_prefix("ssh:")
            .unwrap_or(&tab.info.id)
            .to_string();
        entries.push(tab_widget::TabOverflowEntry {
            index: idx,
            prefix: Some(icons::view(
                icons::IconKind::Terminal,
                byteui::theme::icon_size::row(),
                byteui::theme::color::current().dim,
            )),
            title: tab_title(tab.agent, tab.cwd.as_deref(), &tab.info.name),
            active: ws
                .ssh_active
                .as_ref()
                .is_some_and(|(h, k)| h == &host_id && *k == ssh::SshTabKind::Terminal),
            closable: true,
            hover_t: app.hover_progress(HoverId::TabOverflowRow(idx)),
        });
    }
    for (i, (host_id, _)) in ws.sftp_tabs.iter().enumerate() {
        let idx = i + 1 + ws.ssh_tabs.len();
        let label = ws
            .ssh
            .hosts()
            .iter()
            .find(|h| &h.id == host_id)
            .map(|h| h.name.clone())
            .unwrap_or_else(|| host_id.clone());
        entries.push(tab_widget::TabOverflowEntry {
            index: idx,
            prefix: Some(icons::view(
                icons::IconKind::FolderSync,
                byteui::theme::icon_size::row(),
                byteui::theme::color::current().dim,
            )),
            title: label,
            active: ws.ssh_active.as_ref() == Some(&(host_id.clone(), ssh::SshTabKind::Sftp)),
            closable: true,
            hover_t: app.hover_progress(HoverId::TabOverflowRow(idx)),
        });
    }
    Some(tab_widget::tab_overflow_menu(
        tab_widget::TabOverflowMenuArgs {
            entries,
            anchor,
            window_size: app.window_size,
            on_select: |idx| ssh_tab_overflow_select_message(ws, idx),
            on_close: |idx| ssh_tab_overflow_close_message(ws, idx),
            on_dismiss: Message::Ssh(ssh::Message::TabOverflowDismiss),
            on_row_hover: move |idx, hovered| Message::Hover(HoverId::TabOverflowRow(idx), hovered),
        },
    ))
}

/// 把溢出下拉的扁平下标翻回 SSH 的 `(host_id, kind)`：下标 0 = 空白占位
/// tab；`1..=ssh_tabs.len()` 是终端段；再往后是 SFTP 段。越界兜底回空白态。
fn ssh_tab_overflow_select_message(ws: &Workspace, idx: usize) -> Message {
    if idx == 0 {
        return Message::Ssh(ssh::Message::SelectBlankTab);
    }
    let terminal_count = ws.ssh_tabs.len();
    if idx <= terminal_count {
        let tab = &ws.ssh_tabs[idx - 1];
        let host_id = tab
            .info
            .id
            .strip_prefix("ssh:")
            .unwrap_or(&tab.info.id)
            .to_string();
        return Message::Ssh(ssh::Message::SelectSshTab(
            host_id,
            ssh::SshTabKind::Terminal,
        ));
    }
    match ws.sftp_tabs.keys().nth(idx - 1 - terminal_count) {
        Some(host_id) => Message::Ssh(ssh::Message::SelectSshTab(
            host_id.clone(),
            ssh::SshTabKind::Sftp,
        )),
        None => Message::Ssh(ssh::Message::SelectBlankTab),
    }
}

/// `ssh_tab_overflow_select_message` 的关闭版。空白占位 `closable: false`
/// 保证下拉里它没有 x，走到这只能是兜底，发顶层 no-op。
fn ssh_tab_overflow_close_message(ws: &Workspace, idx: usize) -> Message {
    if idx == 0 {
        return Message::Noop;
    }
    let terminal_count = ws.ssh_tabs.len();
    if idx <= terminal_count {
        let tab = &ws.ssh_tabs[idx - 1];
        let host_id = tab
            .info
            .id
            .strip_prefix("ssh:")
            .unwrap_or(&tab.info.id)
            .to_string();
        return Message::Ssh(ssh::Message::CloseSshTab(
            host_id,
            ssh::SshTabKind::Terminal,
        ));
    }
    match ws.sftp_tabs.keys().nth(idx - 1 - terminal_count) {
        Some(host_id) => Message::Ssh(ssh::Message::CloseSshTab(
            host_id.clone(),
            ssh::SshTabKind::Sftp,
        )),
        None => Message::Noop,
    }
}

/// SSH 面板内嵌终端区:tab 条 + 终端画布(或空态)。镜像 `preview_pane`/
/// `project_preview_pane` 的既有模式——渲染函数不属于 `extensions::ssh`
/// 模块,因为它要用顶层 `Message` 直接操作 `ws.ssh_tabs`。
fn ssh_terminal_pane<'a>(
    app: &'a App,
    ws: &'a Workspace,
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let region = theme::region::terminal_pane();
    let body: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
        match ws.ssh_active.as_ref() {
            Some((host_id, ssh::SshTabKind::Terminal)) => {
                let tab = ws
                    .ssh_tabs
                    .iter()
                    .find(|t| t.info.id.strip_prefix("ssh:") == Some(host_id.as_str()));
                match tab {
                    Some(tab) => {
                        let focused =
                            terminal::keyboard_term_target(app.left_view, app.active_zone)
                                == terminal::TermTarget::SshPanel;
                        term_view::view(
                            &tab.model,
                            focused,
                            terminal::TermTarget::SshPanel,
                            focused.then(|| app.term_ime_preedit()).flatten(),
                        )
                    }
                    None => ssh_empty_state(),
                }
            }
            Some((host_id, ssh::SshTabKind::Sftp)) => match ws.sftp_tabs.get(host_id) {
                Some(state) => {
                    ssh::sftp::sftp_pane_view(state).map(|m| Message::Ssh(ssh::Message::Sftp(m)))
                }
                None => ssh_empty_state(),
            },
            None => ssh_empty_state(),
        };
    container(
        column![ssh_tab_bar(app, ws), tab_divider(), body]
            .spacing(region.gap)
            .height(Length::Fill),
    )
    .width(width)
    .padding(region.padding)
    .style(move |_t: &iced_widget::Theme| container::Style {
        background: region.background.map(Into::into),
        border: outer,
        ..container::Style::default()
    })
    .into()
}

/// "空白" tab(`ssh_active == None`)选中时的内容:跟文件预览面板
/// `preview.rs::TabKind::Blank` 是同一套视觉语言——居中放 Dozer 品牌标 +
/// 引导文案。SSH 这边每个真实 tab 都对应一条 PTY/SFTP 连接
/// (`SessionTab`/`SftpTabState`),没有轻量数据能塞进 `ws.ssh_tabs` 去
/// 表示"空白",所以"空白" tab 只在 `ssh_tab_bar()` 里画一个固定条目,
/// 内容侧靠 `ssh_active == None` 这个分支渲染,不是真的建一个 tab 结构体
/// (对照 preview 那边"tab 数据里有一个 `TabKind::Blank` 变体"的做法)。
fn ssh_empty_state<'a>() -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    container(
        column![
            icons::view(
                icons::IconKind::Dozer,
                72.0,
                byteui::theme::color::current().dim,
            ),
            text("点主机卡片的终端/文件传输图标开始")
                .size(byteui::theme::font::body())
                .color(byteui::theme::color::current().dim),
        ]
        .spacing(14)
        .align_x(iced_widget::core::alignment::Horizontal::Center),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .align_x(iced_widget::core::alignment::Horizontal::Center)
    .align_y(iced_widget::core::alignment::Vertical::Center)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 光标没动(或只在阈值内小幅抖动)不算越过阈值——普通单击场景,同
    /// `rail::rail_drag_past_threshold` 的对应用例。这是 2026-09-04
    /// 用户反馈"agent tab 偶尔两个同时看起来被选中"的根因防回归测试:
    /// 单击 tab 时按下瞬间到抬起前的亚像素抖动不该被当成一次拖拽换位。
    #[test]
    fn tab_drag_past_threshold_false_when_cursor_has_not_moved() {
        assert!(!tab_drag_past_threshold((100.0, 100.0), (100.0, 100.0)));
        assert!(!tab_drag_past_threshold((100.0, 100.0), (101.0, 100.0)));
    }

    /// 恰好等于阈值(平方比较是 `>` 不是 `>=`)不算越过,严格大于才算。
    #[test]
    fn tab_drag_past_threshold_false_when_exactly_at_threshold() {
        assert!(!tab_drag_past_threshold(
            (0.0, 0.0),
            (TAB_DRAG_CONFIRM_THRESHOLD_PX, 0.0)
        ));
    }

    /// 光标越过阈值(任意方向,这里用纯 x 位移)判定为真的拖拽,真实拖拽
    /// 不受这道阈值影响。
    #[test]
    fn tab_drag_past_threshold_true_once_cursor_moves_past_it() {
        assert!(tab_drag_past_threshold(
            (0.0, 0.0),
            (TAB_DRAG_CONFIRM_THRESHOLD_PX + 1.0, 0.0)
        ));
    }

    /// `tree_drag_past_threshold` 同款三条用例(同 `tab_drag_past_threshold`
    /// 的对应测试)——2026-09 用户实测反馈"点一下目录就报'不能把目录移到
    /// 它自己或其子目录里'"的根因防回归测试:单击时按下瞬间到抬起前的
    /// 亚像素抖动不该被当成一次真实拖拽。
    #[test]
    fn tree_drag_past_threshold_false_when_cursor_has_not_moved() {
        assert!(!tree_drag_past_threshold((100.0, 100.0), (100.0, 100.0)));
        assert!(!tree_drag_past_threshold((100.0, 100.0), (101.0, 100.0)));
    }

    #[test]
    fn tree_drag_past_threshold_false_when_exactly_at_threshold() {
        assert!(!tree_drag_past_threshold(
            (0.0, 0.0),
            (TREE_DRAG_CONFIRM_THRESHOLD_PX, 0.0)
        ));
    }

    #[test]
    fn tree_drag_past_threshold_true_once_cursor_moves_past_it() {
        assert!(tree_drag_past_threshold(
            (0.0, 0.0),
            (TREE_DRAG_CONFIRM_THRESHOLD_PX + 1.0, 0.0)
        ));
    }

    /// 2026-09 用户实测反馈(带日志实锤):快速点两下目录,第二下按下到
    /// 松开之间光标真的位移了 ~32px(trackpad 一次快速点按/移开的正常
    /// 抖动量级,超过了 [`TREE_DRAG_CONFIRM_THRESHOLD_PX`]),被判定为一次
    /// "确认的拖拽"并真的把目录移走了——纯距离阈值挡不住这类快速划动。
    /// 加一道"按住时长"门槛:`tree_drag_held_long_enough` 要求按下到松开
    /// 之间至少过了 [`TREE_DRAG_MIN_HOLD_DURATION`],配合距离阈值(两者都要
    /// 满足)才判定为真实拖拽——真正拖拽文件到目标目录、看着高亮再松手,
    /// 耗时远比一次快速点按长。
    #[test]
    fn tree_drag_held_long_enough_false_for_a_quick_flick() {
        assert!(!tree_drag_held_long_enough(
            std::time::Duration::from_millis(30)
        ));
    }

    #[test]
    fn tree_drag_held_long_enough_false_exactly_at_threshold() {
        assert!(!tree_drag_held_long_enough(TREE_DRAG_MIN_HOLD_DURATION));
    }

    #[test]
    fn tree_drag_held_long_enough_true_once_held_past_threshold() {
        assert!(tree_drag_held_long_enough(
            TREE_DRAG_MIN_HOLD_DURATION + std::time::Duration::from_millis(1)
        ));
    }

    /// Files 面板右键菜单开着时该隐藏该侧 webview——同 `tab_overflow_open`
    /// 的既有口径,只是浮层换成了文件树右键菜单(`crate::menu.rs` 文档里
    /// "唯一基准"的那个)。
    #[test]
    fn webview_hidden_by_panel_popup_files_context_menu_open() {
        assert!(webview_hidden_by_panel_popup(
            PanelKind::Files,
            true,
            false,
            false,
        ));
    }

    /// Project 面板链接行右键菜单开着时该隐藏该侧 webview。
    #[test]
    fn webview_hidden_by_panel_popup_project_link_menu_open() {
        assert!(webview_hidden_by_panel_popup(
            PanelKind::Project,
            false,
            true,
            false,
        ));
    }

    /// Conversations 面板 agent 筛选下拉开着时该隐藏该侧 webview。
    #[test]
    fn webview_hidden_by_panel_popup_conversations_agent_picker_open() {
        assert!(webview_hidden_by_panel_popup(
            PanelKind::Conversations,
            false,
            false,
            true,
        ));
    }

    /// 标志位为真,但当前面板种类对不上——不该被误伤隐藏(比如 Project
    /// 链接菜单开着,但这一侧现在显示的是 Files)。
    #[test]
    fn webview_hidden_by_panel_popup_false_when_kind_mismatches_the_open_flag() {
        assert!(!webview_hidden_by_panel_popup(
            PanelKind::Files,
            false,
            true,
            false,
        ));
    }

    /// 没有 webview 的面板种类(如 Todo)恒不隐藏,即便三个标志全为真——
    /// 这几个标志本就不该对这类面板产生任何效果。
    #[test]
    fn webview_hidden_by_panel_popup_false_for_panel_kinds_without_a_webview() {
        assert!(!webview_hidden_by_panel_popup(
            PanelKind::Todo,
            true,
            true,
            true,
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn project_link_menu_items_has_single_delete_entry() {
        let items = project_link_menu_items(project::links::LinkTarget::Docs, 2);
        assert_eq!(items.len(), 1);
        assert!(matches!(
            items[0],
            crate::native_menu::Item::Entry {
                msg: Message::Project(project::Message::LinkRemove { index: 2, .. }),
                ..
            }
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn text_input_menu_items_locks_cut_copy_for_secure_field() {
        let target = TextInputTarget {
            id: iced_widget::core::widget::Id::new("x"),
            secure: true,
        };
        let items = text_input_menu_items(&target);
        let cut_enabled = items.iter().find_map(|i| match i {
            crate::native_menu::Item::Entry {
                msg: Message::TextInputMenuCut,
                enabled,
                ..
            } => Some(*enabled),
            _ => None,
        });
        assert_eq!(cut_enabled, Some(false));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn text_input_menu_items_enables_cut_copy_for_normal_field() {
        let target = TextInputTarget {
            id: iced_widget::core::widget::Id::new("x"),
            secure: false,
        };
        let items = text_input_menu_items(&target);
        let cut_enabled = items.iter().find_map(|i| match i {
            crate::native_menu::Item::Entry {
                msg: Message::TextInputMenuCut,
                enabled,
                ..
            } => Some(*enabled),
            _ => None,
        });
        assert_eq!(cut_enabled, Some(true));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn text_input_menu_items_always_has_four_entries() {
        let target = TextInputTarget {
            id: iced_widget::core::widget::Id::new("x"),
            secure: false,
        };
        assert_eq!(text_input_menu_items(&target).len(), 4);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn database_source_menu_items_locks_refresh_when_not_expanded() {
        let items = database_source_menu_items("pg-main", false);
        let refresh_enabled = items.iter().find_map(|i| match i {
            crate::native_menu::Item::Entry {
                msg: Message::Database(database::Message::SchemaRefresh(_)),
                enabled,
                ..
            } => Some(*enabled),
            _ => None,
        });
        assert_eq!(refresh_enabled, Some(false), "未展开时刷新该锁定");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn database_source_menu_items_enables_refresh_when_expanded() {
        let items = database_source_menu_items("pg-main", true);
        let refresh_enabled = items.iter().find_map(|i| match i {
            crate::native_menu::Item::Entry {
                msg: Message::Database(database::Message::SchemaRefresh(_)),
                enabled,
                ..
            } => Some(*enabled),
            _ => None,
        });
        assert_eq!(refresh_enabled, Some(true));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn database_source_menu_items_always_has_four_entries() {
        assert_eq!(database_source_menu_items("pg-main", false).len(), 4);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn category_context_menu_items_root_pseudo_node_has_only_new_category() {
        let items = category_context_menu_items(None, None);
        assert_eq!(items.len(), 1);
        assert!(matches!(
            items[0],
            crate::native_menu::Item::Entry {
                msg: Message::Todo(todo::Message::CategoryNewSibling(None)),
                ..
            }
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn category_context_menu_items_real_node_has_full_action_set() {
        let items = category_context_menu_items(Some(7), Some(3));
        assert_eq!(
            items.len(),
            7,
            "新建子/同级、上移、下移、移动到、重命名、删除"
        );
        let has_sibling_with_parent = items.iter().any(|i| {
            matches!(
                i,
                crate::native_menu::Item::Entry {
                    msg: Message::Todo(todo::Message::CategoryNewSibling(Some(3))),
                    ..
                }
            )
        });
        assert!(
            has_sibling_with_parent,
            "新建同级分类该挂到 sibling_parent_id"
        );
    }

    /// 复现验收反馈的核心机制:关 tab 后立刻退出,`event_loop.exit()` 不该
    /// 让在飞的 kill/总结 spawn 任务半路被丢弃——`join_pending_exit_tasks`
    /// 必须真的等它们跑完(而不是像修复前那样直接 drop `JoinHandle` 不管)。
    #[test]
    fn join_pending_exit_tasks_waits_for_spawned_tasks_to_finish() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let ran = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let ran_clone = ran.clone();
        let task = rt.spawn(async move {
            // 模拟一次真实的本地 UDS 往返耗时。
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            ran_clone.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        rt.block_on(join_pending_exit_tasks(
            vec![task],
            std::time::Duration::from_secs(2),
        ));
        assert!(
            ran.load(std::sync::atomic::Ordering::SeqCst),
            "预算充足时必须等 spawn 任务真正跑完,而不是提前放弃"
        );
    }

    /// 超预算的任务(daemon 卡死等极端情况)不能把退出流程无限期挂住——
    /// 放弃等待即可,不要求任务本身被中止。
    #[test]
    fn join_pending_exit_tasks_gives_up_after_budget_without_hanging() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let task = rt.spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        });
        let start = std::time::Instant::now();
        rt.block_on(join_pending_exit_tasks(
            vec![task],
            std::time::Duration::from_millis(50),
        ));
        assert!(
            start.elapsed() < std::time::Duration::from_secs(2),
            "超预算必须尽快放弃等待,不能拖住退出流程"
        );
    }

    /// 没有任何在飞任务时(没关过 tab,或都已跑完)应该是零成本的直接返回。
    #[test]
    fn join_pending_exit_tasks_empty_list_returns_immediately() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let start = std::time::Instant::now();
        rt.block_on(join_pending_exit_tasks(
            Vec::new(),
            std::time::Duration::from_secs(2),
        ));
        assert!(start.elapsed() < std::time::Duration::from_millis(100));
    }

    #[test]
    fn merge_review_entries_append_true_accumulates_instead_of_replacing() {
        let mut existing = vec![ReviewEntry::Human {
            text: "第一页第一条".into(),
        }];
        let new = vec![ReviewEntry::Human {
            text: "第二页第一条".into(),
        }];
        merge_review_entries(&mut existing, new, true);
        assert_eq!(existing.len(), 2, "加载更多应该追加,不该丢掉已有内容");
        assert_eq!(
            existing[0],
            ReviewEntry::Human {
                text: "第一页第一条".into()
            }
        );
        assert_eq!(
            existing[1],
            ReviewEntry::Human {
                text: "第二页第一条".into()
            }
        );
    }

    #[test]
    fn merge_review_entries_append_false_replaces() {
        let mut existing = vec![ReviewEntry::Human {
            text: "旧内容".into(),
        }];
        let new = vec![ReviewEntry::Human {
            text: "首次加载的新内容".into(),
        }];
        merge_review_entries(&mut existing, new, false);
        assert_eq!(existing.len(), 1);
        assert_eq!(
            existing[0],
            ReviewEntry::Human {
                text: "首次加载的新内容".into()
            }
        );
    }

    fn loaded_slot(marker: &str) -> WorkspaceSlot {
        let mut ws = Workspace::empty_for_project_placeholder();
        ws.files.set_tree_error(Some(marker.to_string()));
        WorkspaceSlot::Loaded(Box::new(ws))
    }

    fn slot_marker(slot: Option<&WorkspaceSlot>) -> Option<String> {
        match slot {
            Some(WorkspaceSlot::Loaded(ws)) => ws.files.tree_error().map(String::from),
            _ => None,
        }
    }

    fn project_info(id: i64) -> ProjectInfo {
        ProjectInfo {
            id,
            path: format!("/tmp/p{id}"),
            name: format!("p{id}"),
            last_active_ms: 0,
            created_ms: 0,
            updated_ms: 0,
        }
    }

    fn stub_slot(id: i64) -> WorkspaceSlot {
        WorkspaceSlot::Stub {
            info: project_info(id),
            activity: None,
        }
    }

    /// 切页签的核心不变式:只改"当前是哪个页签",两个槽位的内容一个字节都
    /// 不动。设计文档 §2 的"切换永不销毁状态,只有关闭才销毁"整条就落在这
    /// 一个函数上——它若开始改写槽位内容,用户点一下别的页签就丢了工作现场。
    #[test]
    fn focus_project_tab_never_touches_slot_contents() {
        let mut projects = HashMap::new();
        projects.insert(1, loaded_slot("A"));
        projects.insert(2, loaded_slot("B"));
        let order = vec![1, 2];
        let mut active = Some(1);

        assert!(focus_project_tab(&projects, &mut active, 2));

        assert_eq!(active, Some(2));
        assert_eq!(slot_marker(projects.get(&1)).as_deref(), Some("A"));
        assert_eq!(slot_marker(projects.get(&2)).as_deref(), Some("B"));
        assert_eq!(order, vec![1, 2], "切页签不重排页签顺序");
        assert_eq!(projects.len(), 2, "切页签不新增/不删除槽位");
    }

    /// 点一个不存在的页签(脏顺序表/竞态)时原地放弃,不把 `active` 指到一个
    /// 没有槽位的 id 上——那会让 `active_workspace()` 恒为 `None`,界面空白。
    #[test]
    fn focus_project_tab_rejects_unknown_id() {
        let mut projects = HashMap::new();
        projects.insert(1, loaded_slot("A"));
        let mut active = Some(1);

        assert!(!focus_project_tab(&projects, &mut active, 42));
        assert_eq!(active, Some(1));
    }

    /// 关页签后焦点落到右邻;没有右邻取左邻;关光了就回"没有任何项目"。
    #[test]
    fn next_active_after_close_prefers_right_then_left() {
        assert_eq!(next_active_after_close(&[1, 2, 3], 2), Some(3), "右邻优先");
        assert_eq!(
            next_active_after_close(&[1, 2, 3], 3),
            Some(2),
            "末页签取左邻"
        );
        assert_eq!(next_active_after_close(&[1, 2, 3], 1), Some(2));
        assert_eq!(next_active_after_close(&[1], 1), None, "关光了没有下一个");
        assert_eq!(next_active_after_close(&[1, 2], 9), None, "不在表里");
    }

    /// 多项目并行最要紧的一条路由不变式:异步结果按**消息自带的**
    /// `project_id` 落地,与"此刻聚焦的是谁"完全无关。
    ///
    /// 场景就是 review 指出的那个持续性 bug:项目 A 的 agent 在后台跑,用户
    /// 正看着项目 B。A 的 `TermOutput(A, 0, ..)` 到达时,若按"投给当前聚焦的
    /// 项目"路由,`tab_by_id_mut(0)` 会命中 **B 的 tab 0**(每个项目的
    /// `next_tab_id` 都从 0 起编,两边的第一个 tab 必然同号),A 的输出就被
    /// 喂进了 B 的终端。这里断言两个方向都投对了人。
    #[test]
    fn async_results_route_by_project_id_not_by_focus() {
        let mut projects = HashMap::new();
        projects.insert(1, loaded_slot("A"));
        projects.insert(2, loaded_slot("B"));
        // "当前聚焦 B"——路由不该看这个值,这里只是把场景写全。
        let active = Some(2);

        // A 的异步结果(A 在后台)必须落到 A 身上。
        let ws = loaded_workspace_mut(&mut projects, 1).expect("A 已加载");
        assert_eq!(
            ws.files.tree_error(),
            Some("A"),
            "后台项目的结果不能落到前台项目"
        );
        // 反向同理:B 的结果落到 B。
        let ws = loaded_workspace_mut(&mut projects, 2).expect("B 已加载");
        assert_eq!(ws.files.tree_error(), Some("B"));
        assert_eq!(active, Some(2), "路由全程没有读过 active_project_id");
    }

    /// 投不到时静默丢弃,不 panic、不误投给别人:页签在结果回来前被关掉
    /// (槽位不存在),或目标还是没促成过的 `Stub`(不可能有在飞的会话结果)。
    #[test]
    fn async_results_are_dropped_when_target_is_gone_or_unpromoted() {
        let mut projects = HashMap::new();
        projects.insert(1, loaded_slot("A"));
        projects.insert(9, stub_slot(9));

        assert!(
            loaded_workspace_mut(&mut projects, 42).is_none(),
            "页签已关 → 丢弃"
        );
        assert!(
            loaded_workspace_mut(&mut projects, 9).is_none(),
            "Stub 没促成过,不可能有指向它的会话结果"
        );
        // 丢弃的那两条没有波及仍在的槽位。
        assert_eq!(
            loaded_workspace_mut(&mut projects, 1)
                .and_then(|w| w.files.tree_error().map(String::from)),
            Some("A".to_string())
        );
    }

    fn known(ids: &[i64]) -> Vec<ProjectInfo> {
        ids.iter()
            .map(|id| ProjectInfo {
                id: *id,
                path: format!("/tmp/p{id}"),
                name: format!("p{id}"),
                last_active_ms: 0,
                created_ms: 0,
                updated_ms: 0,
            })
            .collect()
    }

    fn state(ids: &[i64], active: Option<i64>) -> open_projects::OpenProjectsState {
        open_projects::OpenProjectsState {
            project_ids: ids.to_vec(),
            active_project_id: active,
        }
    }

    /// 启动恢复的主干:盘上记着的**整份**页签集合都恢复出来(不是只恢复
    /// 聚焦的那一个),顺序按盘上的顺序,聚焦项就是上次退出前聚焦的那个。
    #[test]
    fn restore_open_tabs_restores_whole_tab_set_in_order() {
        let (order, active) = restore_open_tabs(&known(&[1, 2, 3]), &state(&[3, 1, 2], Some(1)));
        assert_eq!(
            order,
            vec![3, 1, 2],
            "页签顺序按盘上记的,不按 daemon 的排序"
        );
        assert_eq!(active, Some(1));
    }

    /// daemon 已经不认识的 id(项目在上次退出后被删了)直接跳过:给它开一个
    /// 点不动的空页签只会碍事。
    #[test]
    fn restore_open_tabs_skips_ids_daemon_no_longer_knows() {
        let (order, active) = restore_open_tabs(&known(&[1, 3]), &state(&[1, 2, 3], Some(3)));
        assert_eq!(order, vec![1, 3]);
        assert_eq!(active, Some(3));
    }

    /// 记着的聚焦项自己就是被删掉的那个 → 回落到第一个页签,而不是留一个
    /// 指向空槽位的 `active_project_id`(那会让界面恒空白)。
    #[test]
    fn restore_open_tabs_falls_back_when_active_is_stale() {
        let (order, active) = restore_open_tabs(&known(&[1, 3]), &state(&[1, 3], Some(2)));
        assert_eq!(order, vec![1, 3]);
        assert_eq!(active, Some(1));
    }

    /// 首次启动(没有 open_projects.json)/上次开着的项目全被删:回落到
    /// `list_projects()` 的第一个——它按 last_active_ms 倒序,就是最近用过的
    /// 那个,保持"打开 app 就能干活"的既有行为。
    #[test]
    fn restore_open_tabs_falls_back_to_most_recent_project() {
        let (order, active) = restore_open_tabs(&known(&[7, 8]), &state(&[], None));
        assert_eq!(order, vec![7]);
        assert_eq!(active, Some(7));

        let (order, active) = restore_open_tabs(&known(&[7, 8]), &state(&[99], Some(99)));
        assert_eq!(order, vec![7], "记着的项目全没了也要回落,不能留空页签栏");
        assert_eq!(active, Some(7));
    }

    /// daemon 上一个项目都没有(全新机器):什么都恢复不出来,`App` 停在
    /// "未打开任何项目"的空外壳,而不是造一个指向不存在项目的页签。
    #[test]
    fn restore_open_tabs_yields_nothing_when_daemon_has_no_projects() {
        let (order, active) = restore_open_tabs(&[], &state(&[1, 2], Some(1)));
        assert!(order.is_empty());
        assert_eq!(active, None);
    }

    /// `open_projects.json` 是用户可编辑的普通 JSON,重复 id 要去重——否则
    /// `project_order` 会带出两个指向同一个槽位的页签(点其中一个,两个一起
    /// 高亮)。
    #[test]
    fn restore_open_tabs_dedups_repeated_ids() {
        let (order, active) = restore_open_tabs(&known(&[1, 2]), &state(&[1, 2, 1], Some(2)));
        assert_eq!(order, vec![1, 2]);
        assert_eq!(active, Some(2));
    }

    /// 关**后台**页签不打扰前台:被关的槽位摘掉、顺序表去掉它,
    /// `active_project_id` 原样不动。
    #[test]
    fn take_project_tab_keeps_active_when_closing_background_tab() {
        let mut projects = HashMap::new();
        projects.insert(1, loaded_slot("A"));
        projects.insert(2, loaded_slot("B"));
        let mut order = vec![1, 2];
        let mut active = Some(1);

        let taken = take_project_tab(&mut projects, &mut order, &mut active, 2);

        assert_eq!(
            slot_marker(taken.as_ref()).as_deref(),
            Some("B"),
            "摘出来的正是那个槽位"
        );
        assert_eq!(active, Some(1), "关后台页签不该改前台");
        assert_eq!(order, vec![1]);
        assert_eq!(slot_marker(projects.get(&1)).as_deref(), Some("A"));
    }

    /// 关的正好是前台页签时焦点顺延到邻居;关光最后一个则回到"未打开任何
    /// 项目"的空外壳(`active_project_id = None`)。
    #[test]
    fn take_project_tab_moves_active_to_neighbor_then_none() {
        let mut projects = HashMap::new();
        projects.insert(1, loaded_slot("A"));
        projects.insert(2, loaded_slot("B"));
        let mut order = vec![1, 2];
        let mut active = Some(1);

        take_project_tab(&mut projects, &mut order, &mut active, 1);
        assert_eq!(active, Some(2));
        assert_eq!(order, vec![2]);

        take_project_tab(&mut projects, &mut order, &mut active, 2);
        assert_eq!(active, None);
        assert!(order.is_empty());
        assert!(projects.is_empty());
    }

    /// 关一个不存在的页签是彻底的空操作(不改 active、不改顺序表)。
    #[test]
    fn take_project_tab_unknown_id_is_noop() {
        let mut projects = HashMap::new();
        projects.insert(1, loaded_slot("A"));
        let mut order = vec![1];
        let mut active = Some(1);

        assert!(take_project_tab(&mut projects, &mut order, &mut active, 42).is_none());
        assert_eq!(active, Some(1));
        assert_eq!(order, vec![1]);
    }

    /// 页签指示点的优先级(2026-08-17 重新定案):红(AwaitingInput,agent 在
    /// 等你)> 绿(Running,还在跑)> 金(TurnEnded,该你出手了)> 青(Idle)
    /// > 不画点。各状态固定配色,不再有闪烁区分。
    ///
    /// 用 `AgentKind::Claude` 代表真实 agent,与下面
    /// `project_dot_unknown_agent_*` 系列(Unknown agent 的死会话灰点)分开测。
    #[test]
    fn project_dot_color_priority() {
        use dozer_core::protocol::AgentKind::Claude;
        use dozer_core::protocol::AgentState::*;

        assert_eq!(project_dot(&[]), None, "无存活会话不画点");
        assert_eq!(
            project_dot(&[(Idle, Claude)]),
            Some(byteui::theme::color::current().cyan)
        );
        assert_eq!(
            project_dot(&[(Idle, Claude), (TurnEnded, Claude)]),
            Some(byteui::theme::color::current().gold),
            "回合结束优先于空闲"
        );
        assert_eq!(
            project_dot(&[(Idle, Claude), (TurnEnded, Claude), (Running, Claude)]),
            Some(byteui::theme::color::current().green),
            "还在跑优先于回合结束/空闲"
        );
        assert_eq!(
            project_dot(&[
                (Idle, Claude),
                (TurnEnded, Claude),
                (Running, Claude),
                (AwaitingInput, Claude)
            ]),
            Some(byteui::theme::color::current().red),
            "agent 在等你优先级最高"
        );
        // 顺序无关:优先级看的是状态集合,不是 tab 的先后。
        assert_eq!(
            project_dot(&[(TurnEnded, Claude), (Idle, Claude)]),
            project_dot(&[(Idle, Claude), (TurnEnded, Claude)])
        );
    }

    /// Unknown agent(纯 shell/git shell/hook 还没上报过)不该显示成跟真实
    /// agent 完成一轮工作同款的 cyan"空闲"——应该显示灰色"死会话"点。
    #[test]
    fn project_dot_unknown_agent_shows_dead_session_gray_not_idle_cyan() {
        use dozer_core::protocol::AgentKind::Unknown;
        use dozer_core::protocol::AgentState::Idle;

        assert_eq!(
            project_dot(&[(Idle, Unknown)]),
            Some(byteui::theme::color::current().dim),
            "只有纯 shell/git shell 存活时应显示死会话灰点,不是空闲青点"
        );
    }

    /// 项目里同时有真实 agent 和纯 shell 存活时,真实 agent 的状态照旧
    /// 优先决定颜色——Unknown 会话不参与竞争,也不会把真实状态"拉低"。
    #[test]
    fn project_dot_real_agent_wins_over_unknown_when_both_alive() {
        use dozer_core::protocol::AgentKind::{Claude, Unknown};
        use dozer_core::protocol::AgentState::{Idle, Running};

        assert_eq!(
            project_dot(&[(Idle, Unknown), (Running, Claude)]),
            Some(byteui::theme::color::current().green),
            "真实 agent 在跑,应该显示绿点,不受纯 shell 的 Idle 干扰"
        );
    }

    /// 测试基准态:默认布局、左=文件列表、右=Agent、两侧都展开。
    fn test_state() -> ShellState {
        ShellState {
            layout: ShellLayout::default(),
            dims: PanelDims::default(),
            left_view: PanelKind::Files,
            left_collapsed: false,
            right_view: PanelKind::Agent,
            right_collapsed: false,
            browser_bookmarks_open: false,
            maximized: None,
        }
    }

    #[test]
    fn terminal_pane_height_excludes_top_bar_and_chrome() {
        // 共享终端自带的 `terminal_status_bar` 已经去掉，高度公式不再扣
        // `status_bar_height()`——只扣顶栏 + chrome + right_zone 上下
        // margin，跟 SSH 终端那边的公式形状一致。
        let state = test_state();
        let (_, h_with) = terminal_pane_pixel_size(1440.0, 900.0, &state);
        let only_chrome = 900.0 - byteui::theme::geometry::chrome_height_px();
        let m = theme::region::right_zone().margin;
        assert!(
            (only_chrome - h_with - (byteui::theme::geometry::top_bar_height() + m.top + m.bottom))
                .abs()
                < 0.01,
            "终端 pane 高度必须再扣顶栏+right_zone 上下 margin"
        );
    }

    #[test]
    fn terminal_pane_zero_when_terminal_not_visible() {
        // 终端只在右侧=Agent 且未收起时可见,否则零尺寸(不 resize 不可见终端)。
        let collapsed = ShellState {
            right_collapsed: true,
            ..test_state()
        };
        assert_eq!(
            terminal_pane_pixel_size(1440.0, 900.0, &collapsed),
            (0.0, 0.0)
        );
        let conversations = ShellState {
            right_view: PanelKind::Conversations,
            ..test_state()
        };
        assert_eq!(
            terminal_pane_pixel_size(1440.0, 900.0, &conversations),
            (0.0, 0.0)
        );
    }

    #[test]
    fn ssh_terminal_pane_matches_left_content_panel_width() {
        // SSH 终端挂在左面板区 `主机列表|终端` 配对的内容侧:宽 = 左区内容
        // 宽 × (1 - ssh_split),再扣左右 chrome。必须跟渲染侧 `left_panel_area`
        // 的 `row![list(FillPortion), divider, content(FillPortion)]` 布局对得上,
        // 否则终端列数套的是共享终端的,拉分隔条就跟不上宿主面板。
        let state = test_state();
        let (w, _h) = ssh_terminal_pane_pixel_size(1440.0, 900.0, &state);
        let left_w = left_zone_width(1440.0, &state);
        let pair_w = pair_content_width(left_w);
        let (_list_w, content_w) = pair_list_content_width(pair_w, state.dims.ssh_split);
        let expected = content_w - byteui::theme::geometry::chrome_width_px();
        assert!(
            (w - expected).abs() < 0.01,
            "SSH 终端 pane 宽必须等于左栏内容侧宽减 chrome: w={w}, expected={expected}"
        );
        assert!(
            w > 0.0 && w < left_w,
            "SSH 终端应占左区一部分宽度、且小于整块左区: w={w}, left_w={left_w}"
        );
        // 隔板越往终端一侧拖(ssh_split 越大),终端越窄——网格必须跟着变。
        let mut tall = state;
        tall.dims.ssh_split = 0.8;
        let (w_tall, _) = ssh_terminal_pane_pixel_size(1440.0, 900.0, &tall);
        assert!(
            w_tall < w,
            "ssh_split 增大后 SSH 终端 pane 应变窄: w_tall={w_tall}, w={w}"
        );
    }

    #[test]
    fn ssh_terminal_pane_height_excludes_top_bar_and_chrome_without_status_bar() {
        // SSH 终端没有 `terminal_status_bar`,高度只扣顶栏 + chrome + 左区
        // 上下 margin(不像共享终端那样再额外扣状态栏高)。
        let state = test_state();
        let (_, h) = ssh_terminal_pane_pixel_size(1440.0, 900.0, &state);
        let m = theme::region::left_zone().margin;
        let expected = 900.0
            - byteui::theme::geometry::top_bar_height()
            - byteui::theme::geometry::chrome_height_px()
            - m.top
            - m.bottom;
        assert!(
            (h - expected).abs() < 0.01,
            "SSH 终端 pane 高度:{h}, expected:{expected}"
        );
    }

    #[test]
    fn ssh_terminal_pane_zero_when_left_collapsed() {
        let collapsed = ShellState {
            left_collapsed: true,
            ..test_state()
        };
        assert_eq!(
            ssh_terminal_pane_pixel_size(1440.0, 900.0, &collapsed),
            (0.0, 0.0),
            "左面板区收起时 SSH 终端不可见,应返回零尺寸"
        );
    }

    /// Fix round 2 Critical #1:窗口被缩到比持久化 `left_width` 还窄时,
    /// 左面板区的**有效**宽必须重新夹取,否则右面板区一寸不剩。
    ///
    /// 具体数字(窗口逻辑宽 720——1440pt 屏上把窗口贴半屏就是这个宽度):
    /// `zones_width(720)` = 720 - 2*44(图标栏) - 8(LeftRight 分隔线) = 624;
    /// 上界 = max(624 - 320(byteui::theme::geometry::min_zone_width()), 320) = 320;
    /// 默认 `left_width`=640 夹取后 = 320,右面板区 = 624 - 320 = 304(>0)。
    ///
    /// 修复前的 flex 追账(iced_core flex.rs `resolve` 第一趟按顺序给
    /// 非流体子元素分配、`available` 递减):available=720 →左图标栏 Fixed(44)
    /// →676 →左面板区 `Length::Fixed(640)` 全额吃下 →36 →分隔线 Fixed(8)
    /// →28 →右图标栏 Fixed(44) 被 `Limits::resolve` 夹到 28 →0,
    /// `remaining`=0,唯一 `Fill` 的右面板区第三趟拿到 0 宽:终端/Agent 列表/
    /// 对话/审阅整片消失,右图标栏还被压成半宽。
    #[test]
    fn left_zone_width_reclamped_when_window_narrower_than_persisted_width() {
        let state = test_state();
        assert_eq!(state.dims.left_width, 640.0, "前提:默认持久化宽 640");

        assert_eq!(clamp_left_width(720.0, 640.0), 320.0);
        assert_eq!(left_zone_width(720.0, &state), 320.0);
        let right = right_zone_width(720.0, &state);
        assert!(
            (right - 304.0).abs() < 0.01,
            "右面板区必须仍有宽度: {right}"
        );
        assert!(right >= 1.0, "右半边不能塌成 0 宽");

        // 宽窗口下不受影响(不能为了修窄窗把正常情形也改坏)。
        assert_eq!(clamp_left_width(1440.0, 640.0), 640.0);
        assert_eq!(left_zone_width(1440.0, &state), 640.0);
        assert_ne!(
            left_zone_width(720.0, &state),
            left_zone_width(1440.0, &state),
            "窄窗与宽窗必须得出不同的有效宽"
        );

        // 夹取是**渲染/几何时刻**的临时行为:不回写持久化值,窗口再拉宽
        // 时用户原来偏好的 640 自动复原。
        assert_eq!(state.dims.left_width, 640.0, "夹取不得改写各项目持久化宽");
    }

    /// 极窄窗口(比 `byteui::theme::geometry::min_window_width()` 还窄,例如外部强制 resize)下也不 panic,
    /// 且左区宽不会超过 `zones_width` 本身。
    #[test]
    fn clamp_left_width_survives_absurdly_narrow_window() {
        assert_eq!(
            clamp_left_width(200.0, 640.0),
            byteui::theme::geometry::min_zone_width()
        );
        assert_eq!(
            clamp_left_width(0.0, 640.0),
            byteui::theme::geometry::min_zone_width()
        );
        // 最小窗口宽恰好能让两侧都拿到 byteui::theme::geometry::min_zone_width()。
        assert_eq!(
            clamp_left_width(byteui::theme::geometry::min_window_width(), 640.0),
            byteui::theme::geometry::min_zone_width()
        );
        assert!(
            (zones_width(byteui::theme::geometry::min_window_width())
                - 2.0 * byteui::theme::geometry::min_zone_width())
            .abs()
                < 0.01
        );
    }

    /// Fix round 2 #3:终端可见性判定。旧四栏布局里终端恒在屏上,新外壳有三
    /// 条路径能把它藏起来,藏着时不能再把按键写进 PTY。
    #[test]
    fn terminal_visible_only_when_agent_pair_on_screen() {
        assert!(
            terminal_visible(&test_state()),
            "基准态:右侧 Agent 配对可见"
        );
        assert!(
            !terminal_visible(&ShellState {
                right_view: PanelKind::Conversations,
                ..test_state()
            }),
            "右视图切到对话:终端不在屏上"
        );
        assert!(
            !terminal_visible(&ShellState {
                right_collapsed: true,
                ..test_state()
            }),
            "右侧收起:终端不在屏上"
        );
        assert!(
            !terminal_visible(&ShellState {
                maximized: Some(MaximizedPane::Left),
                ..test_state()
            }),
            "左侧放大:右半边被变暗遮罩整片盖住"
        );
        assert!(
            terminal_visible(&ShellState {
                maximized: Some(MaximizedPane::Right),
                ..test_state()
            }),
            "放大的正是终端那一侧:终端更大更可见,算可见"
        );
    }

    /// Fix round 3(scoped re-review 发现):对话视图下放大的是审阅 pane
    /// 自己的放大按钮(同样发 `MaximizedPane::Right`),不是终端——
    /// `terminal_grid_state` 如果不过滤这种情况,会把"审阅被放大"误判成
    /// "终端被放大",按放大格给一个实际不可见、也不是那个尺寸的终端算网格,
    /// 给存活 PTY 发一次错的 SIGWINCH。
    #[test]
    fn terminal_grid_state_ignores_maximized_review_not_terminal() {
        let review_maximized = ShellState {
            right_view: PanelKind::Conversations,
            maximized: Some(MaximizedPane::Right),
            ..test_state()
        };
        let grid_state = terminal_grid_state(review_maximized);
        assert_eq!(
            grid_state.maximized, None,
            "对话视图下的 MaximizedPane::Right 指的是审阅 pane,换算终端网格时不该当成终端被放大"
        );

        let terminal_maximized = ShellState {
            right_view: PanelKind::Agent,
            maximized: Some(MaximizedPane::Right),
            ..test_state()
        };
        assert_eq!(
            terminal_grid_state(terminal_maximized).maximized,
            Some(MaximizedPane::Right),
            "右视图本来就是 Agent 时,MaximizedPane::Right 才真的是终端被放大,要保留"
        );
    }

    /// Fix round 2 #6a:放大终端时 PTY 网格必须按 `maximize_overlay` 实际
    /// 渲染的金色描边盒子重算,不能停在放大前的尺寸(否则只放大了外框)。
    ///
    /// 具体数字(1440x900,`agent_split`=默认统一 split 0.35):
    /// avail_w = 1440 - 2*44 - 2*40 = 1272,pair_w = 1272 - 8 = 1264,
    /// 终端占 1-0.35 → 1264*0.65 = 821.6,减 `byteui::theme::geometry::chrome_width_px()`(16) = 805.6;
    /// 盒子高 = 900 - 40(顶栏) - 2*40 = 780,再减
    /// `byteui::theme::geometry::chrome_height_px()`(50) = 730(不扣状态栏高——
    /// 终端 pane 自带的 `terminal_status_bar` 已经去掉)。
    /// 对照平时:zones_width = 1440-2*44-8=1344,right_w = 1344 - 640 = 704,pair = 696,
    /// 696*0.65 = 452.4,减 16 = 436.4;高 = 900 - 40 - 50 - right_zone 上下 margin = 804。
    /// 换成网格(CELL_WIDTH=8.4,LINE_HEIGHT_PX=16.8 即 14*1.2):放大后 95x43,平时 51x47。
    #[test]
    fn terminal_pane_pixel_size_right_maximized_matches_overlay_box() {
        let maxed = ShellState {
            maximized: Some(MaximizedPane::Right),
            ..test_state()
        };
        let (w, h) = terminal_pane_pixel_size(1440.0, 900.0, &maxed);
        assert!((w - 805.6).abs() < 0.1, "w={w}");
        assert!((h - 730.0).abs() < 0.1, "h={h}");

        let normal = terminal_pane_pixel_size(1440.0, 900.0, &test_state());
        assert!((normal.0 - 436.4).abs() < 0.1, "平时 w={}", normal.0);
        assert!((normal.1 - 804.0).abs() < 0.1, "平时 h={}", normal.1);
        assert_ne!((w, h), normal, "放大态几何必须和平时不同");
        assert!(w > normal.0, "放大后终端必须真的更宽(网格跟着变宽)");

        assert_eq!(crate::term_view::grid_size(w, h), (95, 43));
        assert_eq!(crate::term_view::grid_size(normal.0, normal.1), (51, 47));

        // 左侧放大不改变右面板区几何(右半只是被遮罩盖住)。
        let left_maxed = ShellState {
            maximized: Some(MaximizedPane::Left),
            ..test_state()
        };
        assert_eq!(terminal_pane_pixel_size(1440.0, 900.0, &left_maxed), normal);
    }

    /// Fix round 2 #6b:终端当前不可见时,网格换算走"若显示则多大"的假想
    /// 状态。否则上次退出时右侧停在对话视图 → 启动时 `terminal_pane_pixel_size`
    /// 返回 (0,0) → main.rs 的 `cols>0 && rows>0` 守卫跳过 → 恢复出来的会话
    /// 一直卡在 80x24(`DEFAULT_COLS`/`DEFAULT_ROWS`),哪怕屏幕很大。
    #[test]
    fn terminal_grid_state_sizes_hidden_terminal_as_if_shown() {
        let shown = terminal_pane_pixel_size(1440.0, 900.0, &test_state());

        for hidden in [
            ShellState {
                right_view: PanelKind::Conversations,
                ..test_state()
            },
            ShellState {
                right_collapsed: true,
                ..test_state()
            },
        ] {
            // 真实状态下仍是零尺寸(不 resize 一个不可见的终端)。
            assert_eq!(terminal_pane_pixel_size(1440.0, 900.0, &hidden), (0.0, 0.0));
            // 网格换算用的假想状态下,尺寸与"右侧展开显示 Agent"完全一致。
            let for_grid = terminal_grid_state(hidden);
            assert_eq!(terminal_pane_pixel_size(1440.0, 900.0, &for_grid), shown);
        }

        // 具体网格:1440x900 下应是 51x47(已扣 right_zone 上下 margin),不是兜底的 80x24。
        let (cols, rows) = crate::term_view::grid_size(shown.0, shown.1);
        assert_eq!((cols, rows), (51, 47));
        assert_ne!(
            (cols as u16, rows as u16),
            (DEFAULT_COLS, DEFAULT_ROWS),
            "启动时必须算出真实网格,不能停在 80x24 兜底值"
        );

        // 假想状态只覆盖右侧收起/右视图,不篡改放大态与左侧状态。
        let left_max = ShellState {
            maximized: Some(MaximizedPane::Left),
            left_collapsed: true,
            right_collapsed: true,
            ..test_state()
        };
        let g = terminal_grid_state(left_max);
        assert_eq!(g.maximized, Some(MaximizedPane::Left));
        assert!(g.left_collapsed);
        assert!(!g.right_collapsed);
    }

    /// 可选项:磁盘上的面板尺寸值不一定出自本程序(手改 panel_layouts.json/
    /// 别的版本)。split 恰为 0.0/1.0 时 `split_portions` 会给出 `FillPortion(0)`,
    /// 那一块在 flex 里拿不到任何宽度、整块消失,所以读盘时先夹一遍。
    #[test]
    fn sanitize_panel_dims_clamps_foreign_values() {
        let poisoned = PanelDims {
            left_width: 10.0,
            files_split: 0.0,
            agent_split: 1.0,
            conversations_split: f32::NAN,
            ..PanelDims::default()
        };
        let s = sanitize_panel_dims(poisoned);
        assert_eq!(s.left_width, byteui::theme::geometry::min_zone_width());
        assert_eq!(s.files_split, byteui::theme::geometry::min_split_ratio());
        assert_eq!(s.agent_split, byteui::theme::geometry::max_split_ratio());
        assert_eq!(s.conversations_split, PanelDims::default().files_split);
        // 夹过之后 FillPortion 两侧都非 0(那一块不会凭空消失)。
        for split in [
            s.files_split,
            s.project_split,
            s.agent_split,
            s.conversations_split,
        ] {
            let (list, content) = split_portions(split);
            assert!(list > 0 && content > 0, "split={split}");
        }
        // 合法值原样保留。
        let sane = PanelDims {
            left_width: 500.0,
            files_split: 0.35,
            ..PanelDims::default()
        };
        assert_eq!(sanitize_panel_dims(sane), sane);
    }

    #[test]
    fn apply_column_drag_updates_git_log_split_ratio() {
        let state = test_state();
        let window_width = 1600.0;
        let result = apply_column_drag(state, Divider::GitLogSplit, window_width, 300.0);
        assert!(result.git_log_split >= byteui::theme::geometry::min_split_ratio());
        assert!(result.git_log_split <= byteui::theme::geometry::max_split_ratio());
    }

    #[test]
    fn apply_column_drag_updates_browser_bookmarks_split_ratio() {
        let state = test_state();
        let window_width = 1600.0;
        // 拖拽点在左面板区靠右侧,网页内容(拖拽点左侧)占比应偏大。
        let result = apply_column_drag(state, Divider::BrowserBookmarksSplit, window_width, 500.0);
        assert!(result.browser_bookmarks_split >= byteui::theme::geometry::min_split_ratio());
        assert!(result.browser_bookmarks_split <= byteui::theme::geometry::max_split_ratio());
    }

    #[test]
    fn apply_column_drag_browser_bookmarks_split_direction_matches_content_side() {
        // 方向性回归:拖拽点越靠右,网页内容(左侧)占比应该越大——
        // browser_bookmarks_split 存的是内容占比,不是收藏夹占比。
        let state = test_state();
        let window_width = 1600.0;
        let near = apply_column_drag(
            state.clone(),
            Divider::BrowserBookmarksSplit,
            window_width,
            100.0,
        );
        let far = apply_column_drag(state, Divider::BrowserBookmarksSplit, window_width, 500.0);
        assert!(
            far.browser_bookmarks_split > near.browser_bookmarks_split,
            "near={} far={}",
            near.browser_bookmarks_split,
            far.browser_bookmarks_split
        );
    }

    #[test]
    fn sanitize_panel_dims_clamps_git_log_split() {
        let dims = PanelDims {
            git_log_split: 5.0,
            ..PanelDims::default()
        };
        let sanitized = sanitize_panel_dims(dims);
        assert!(sanitized.git_log_split <= byteui::theme::geometry::max_split_ratio());

        let dims = PanelDims {
            git_log_split: f32::NAN,
            ..PanelDims::default()
        };
        let sanitized = sanitize_panel_dims(dims);
        assert_eq!(sanitized.git_log_split, PanelDims::default().files_split);
    }

    #[test]
    fn default_panel_dims_includes_browser_bookmarks_split() {
        let dims = PanelDims::default();
        assert_eq!(
            dims.browser_bookmarks_split,
            byteui::theme::geometry::default_split_ratio()
        );
    }

    #[test]
    fn sanitize_panel_dims_clamps_browser_bookmarks_split() {
        let dims = PanelDims {
            browser_bookmarks_split: 5.0,
            ..PanelDims::default()
        };
        let sanitized = sanitize_panel_dims(dims);
        assert!(sanitized.browser_bookmarks_split <= byteui::theme::geometry::max_split_ratio());

        let dims = PanelDims {
            browser_bookmarks_split: f32::NAN,
            ..PanelDims::default()
        };
        let sanitized = sanitize_panel_dims(dims);
        assert_eq!(
            sanitized.browser_bookmarks_split,
            PanelDims::default().files_split
        );
    }

    #[test]
    fn apply_row_drag_clamps_ratio_within_valid_range() {
        let state = test_state();
        let window_height = 1000.0;

        // 光标在窗口中间——应该落在合法比例区间内。
        let mid = apply_row_drag(
            state.clone(),
            RowDivider::GitLogFileDiffSplit,
            window_height,
            500.0,
        );
        assert!(mid.git_log_file_diff_split >= byteui::theme::geometry::min_split_ratio());
        assert!(mid.git_log_file_diff_split <= byteui::theme::geometry::max_split_ratio());

        // 光标远超窗口顶部/底部——应该被 clamp,不产生非法比例。
        let top = apply_row_drag(
            state.clone(),
            RowDivider::GitLogFileDiffSplit,
            window_height,
            -500.0,
        );
        assert_eq!(
            top.git_log_file_diff_split,
            byteui::theme::geometry::min_split_ratio()
        );
        let bottom = apply_row_drag(
            state,
            RowDivider::GitLogFileDiffSplit,
            window_height,
            5000.0,
        );
        assert_eq!(
            bottom.git_log_file_diff_split,
            byteui::theme::geometry::max_split_ratio()
        );
    }

    #[test]
    fn sanitize_panel_dims_clamps_git_log_file_diff_split() {
        let dims = PanelDims {
            git_log_file_diff_split: -1.0,
            ..PanelDims::default()
        };
        let sanitized = sanitize_panel_dims(dims);
        assert!(sanitized.git_log_file_diff_split >= byteui::theme::geometry::min_split_ratio());
    }

    /// `window_width`/`window_height` 的夹取单独测:老 `layout.json` 缺这两
    /// 个字段时 serde 补 0.0(不是 `f32::NAN`,判断要用 `> 0.0` 而不能只查
    /// `is_finite`),负数/NAN 同样要落回 `byteui::theme::geometry::initial_window_size()`;合法但过小
    /// 的值只夹下限,不整个重置。
    #[test]
    fn sanitize_shell_layout_clamps_window_size() {
        let missing_fields = ShellLayout {
            window_width: 0.0,
            window_height: 0.0,
            ..ShellLayout::default()
        };
        let s = sanitize_shell_layout(missing_fields);
        assert_eq!(
            s.window_width,
            byteui::theme::geometry::initial_window_size().0
        );
        assert_eq!(
            s.window_height,
            byteui::theme::geometry::initial_window_size().1
        );

        let poisoned = ShellLayout {
            window_width: -100.0,
            window_height: f32::NAN,
            ..ShellLayout::default()
        };
        let s = sanitize_shell_layout(poisoned);
        assert_eq!(
            s.window_width,
            byteui::theme::geometry::initial_window_size().0
        );
        assert_eq!(
            s.window_height,
            byteui::theme::geometry::initial_window_size().1
        );

        let too_small = ShellLayout {
            window_width: 10.0,
            window_height: 10.0,
            ..ShellLayout::default()
        };
        let s = sanitize_shell_layout(too_small);
        assert_eq!(s.window_width, byteui::theme::geometry::min_window_width());
        assert_eq!(
            s.window_height,
            byteui::theme::geometry::min_window_height()
        );

        let legit = ShellLayout {
            window_width: 1800.0,
            window_height: 1100.0,
            ..ShellLayout::default()
        };
        assert_eq!(sanitize_shell_layout(legit.clone()), legit);
    }

    #[test]
    fn zone_at_x_splits_left_and_right_at_zone_boundary() {
        // 窗口宽 1440:左图标栏 44 + 左面板区 640 → 边界在 x=684。
        let state = test_state();
        assert_eq!(
            zone_at_x(100.0, 1440.0, &state),
            Some(ZoneSide::Left),
            "左面板区内"
        );
        assert_eq!(
            zone_at_x(683.9, 1440.0, &state),
            Some(ZoneSide::Left),
            "边界前一发"
        );
        assert_eq!(
            zone_at_x(684.0, 1440.0, &state),
            Some(ZoneSide::Right),
            "边界本身归右面板区"
        );
        assert_eq!(
            zone_at_x(1200.0, 1440.0, &state),
            Some(ZoneSide::Right),
            "右面板区内"
        );
    }

    #[test]
    fn zone_at_x_icon_rail_returns_none() {
        let state = test_state();
        assert_eq!(zone_at_x(20.0, 1440.0, &state), None, "左图标栏本身");
        assert_eq!(zone_at_x(1400.0, 1440.0, &state), None, "右图标栏本身");
    }

    #[test]
    fn zone_at_x_collapsed_side_routes_to_the_other() {
        let left_collapsed = ShellState {
            left_collapsed: true,
            ..test_state()
        };
        assert_eq!(
            zone_at_x(300.0, 1440.0, &left_collapsed),
            Some(ZoneSide::Right),
            "左侧收起,内容区恒归右侧"
        );

        let right_collapsed = ShellState {
            right_collapsed: true,
            ..test_state()
        };
        assert_eq!(
            zone_at_x(1200.0, 1440.0, &right_collapsed),
            Some(ZoneSide::Left),
            "右侧收起,内容区恒归左侧"
        );

        let both_collapsed = ShellState {
            left_collapsed: true,
            right_collapsed: true,
            ..test_state()
        };
        assert_eq!(
            zone_at_x(700.0, 1440.0, &both_collapsed),
            None,
            "两侧都收起,没有哪一侧可点"
        );
    }

    #[test]
    fn zone_at_x_maximized_ignores_x_within_content_range() {
        let left_max = ShellState {
            maximized: Some(MaximizedPane::Left),
            ..test_state()
        };
        // 放大态下整个内容区都算放大的那一侧,不再按横坐标细分。
        assert_eq!(zone_at_x(100.0, 1440.0, &left_max), Some(ZoneSide::Left));
        assert_eq!(zone_at_x(1200.0, 1440.0, &left_max), Some(ZoneSide::Left));

        let right_max = ShellState {
            maximized: Some(MaximizedPane::Right),
            ..test_state()
        };
        assert_eq!(zone_at_x(100.0, 1440.0, &right_max), Some(ZoneSide::Right));
    }

    #[test]
    fn clamp_left_width_within_bounds() {
        let state = test_state();
        let l = apply_column_drag(state, Divider::LeftRight, 1440.0, 500.0);
        assert_eq!(
            l.left_width,
            500.0 - byteui::theme::geometry::icon_rail_width()
        );
    }

    #[test]
    fn clamp_left_width_to_minimum() {
        let state = test_state();
        let l = apply_column_drag(state, Divider::LeftRight, 1440.0, 10.0);
        assert_eq!(l.left_width, byteui::theme::geometry::min_zone_width());
    }

    #[test]
    fn clamp_left_width_to_maximum_keeps_right_zone_alive() {
        // 拖到最右也要给右面板区留 byteui::theme::geometry::min_zone_width()。
        let state = test_state();
        let l = apply_column_drag(state, Divider::LeftRight, 1440.0, 1430.0);
        let expected = 1440.0
            - 2.0 * byteui::theme::geometry::icon_rail_width()
            - byteui::theme::geometry::divider_width()
            - byteui::theme::geometry::min_zone_width();
        assert_eq!(l.left_width, expected);
    }

    #[test]
    fn clamp_left_width_when_window_too_narrow_does_not_panic() {
        // 窗口窄到上界低于下界时,`.max(下限)` 把上界垫平,恒不 panic。
        let state = test_state();
        let l = apply_column_drag(state, Divider::LeftRight, 700.0, 650.0);
        assert_eq!(l.left_width, byteui::theme::geometry::min_zone_width());
    }

    #[test]
    fn left_right_drag_keeps_list_pane_pixel_width_fixed() {
        // 默认 `test_state()`:left_view = Files,files_split 默认 0.35。
        // 拖外层 zone 分隔线把 left_width 从默认 640 拉到 800,文件树的
        // 绝对像素宽应该保持不变(只有比例跟着 zone 变宽而回调)。
        let state = test_state();
        let window_width = 1440.0;
        let old_pair_w = pair_content_width(clamp_left_width(window_width, state.dims.left_width));
        let old_list_px = old_pair_w * state.dims.files_split;

        let logical_x = 800.0 + byteui::theme::geometry::icon_rail_width();
        let new = apply_column_drag(state.clone(), Divider::LeftRight, window_width, logical_x);
        assert!(
            (new.left_width - 800.0).abs() < 0.01,
            "拖拽目标本身要生效: {}",
            new.left_width
        );

        let new_pair_w = pair_content_width(clamp_left_width(window_width, new.left_width));
        let new_list_px = new_pair_w * new.files_split;
        assert!(
            (new_list_px - old_list_px).abs() < 0.5,
            "文件树列表像素宽应保持不变: old={old_list_px}, new={new_list_px}"
        );
        // 反解出的新比例必须比默认值小——zone 变宽了,同样的像素宽占比更低。
        assert!(new.files_split < state.dims.files_split);
    }

    #[test]
    fn left_right_drag_also_compensates_right_zone_active_panel() {
        // right_view 默认是 Agent(agent_split)。左边变宽会挤压右边 zone,
        // 右侧配对面板(这里是 Agent 列表)同样不该被连带缩放。
        let state = test_state();
        let window_width = 1440.0;
        let old_right_zone_w = right_zone_width(window_width, &state);
        let old_pair_w = pair_content_width(old_right_zone_w);
        let old_list_px = old_pair_w * state.dims.agent_split;

        let logical_x = 800.0 + byteui::theme::geometry::icon_rail_width();
        let new = apply_column_drag(state.clone(), Divider::LeftRight, window_width, logical_x);
        let new_probe = ShellState { dims: new, ..state };
        let new_right_zone_w = right_zone_width(window_width, &new_probe);
        let new_pair_w = pair_content_width(new_right_zone_w);
        let new_list_px = new_pair_w * new_probe.dims.agent_split;
        assert!(
            (new_list_px - old_list_px).abs() < 0.5,
            "Agent 列表像素宽应保持不变: old={old_list_px}, new={new_list_px}"
        );
    }

    #[test]
    fn clamp_files_split_within_bounds() {
        let state = test_state();
        // 左面板区 640 宽,配对内容宽 = 640-8=632,拖到其中点(316)→ 0.5。
        let l = apply_column_drag(
            state,
            Divider::LeftPairSplit,
            1440.0,
            byteui::theme::geometry::icon_rail_width() + 316.0,
        );
        assert!((l.files_split - 0.5).abs() < 0.001, "{}", l.files_split);
    }

    #[test]
    fn clamp_files_split_to_range() {
        let state = test_state();
        // 默认 640 宽面板区下,`min_split_ratio`(0.2)算出的像素宽已经小于
        // `project::footer_min_width`——拖到最左现在直接收起文件树列表子栏
        // (`files_tree_collapsed`),不再是单纯把 `files_split` 夹到 0.2,
        // 见 `Divider::LeftPairSplit` 分支;冻结的 `files_split` 应保持
        // `state` 原值不变。
        let l = apply_column_drag(state.clone(), Divider::LeftPairSplit, 1440.0, 10.0);
        assert!(l.files_tree_collapsed);
        assert_eq!(l.files_split, state.dims.files_split);
        let l = apply_column_drag(state, Divider::LeftPairSplit, 1440.0, 5000.0);
        assert_eq!(l.files_split, byteui::theme::geometry::max_split_ratio());
    }

    #[test]
    fn split_drag_skips_update_on_zero_zone_width() {
        // 该侧已收起 → 区宽 0,除零防御:原样返回不 panic。
        let left_gone = ShellState {
            left_collapsed: true,
            ..test_state()
        };
        assert_eq!(
            apply_column_drag(left_gone.clone(), Divider::LeftPairSplit, 1440.0, 500.0),
            left_gone.dims
        );
        let right_gone = ShellState {
            right_collapsed: true,
            ..test_state()
        };
        assert_eq!(
            apply_column_drag(right_gone.clone(), Divider::RightPairSplit, 1440.0, 1000.0),
            right_gone.dims
        );
    }

    #[test]
    fn right_pair_split_writes_field_of_current_right_view() {
        // 右面板区宽 = 1440 - 2*44 - 640 - 8 = 704,左边缘 x = 1440-44-704 = 692;
        // 配对内容宽 = 704-8=696,其中点 348 处拖动(692+348=1040)→ 0.5。
        let agent = test_state();
        let l = apply_column_drag(
            agent.clone(),
            Divider::RightPairSplit,
            1440.0,
            696.0 + 344.0,
        );
        assert!((l.agent_split - 0.5).abs() < 0.001, "{}", l.agent_split);
        assert_eq!(
            l.conversations_split, agent.dims.conversations_split,
            "不该串写另一配对的比例"
        );

        let conversations = ShellState {
            right_view: PanelKind::Conversations,
            ..test_state()
        };
        let l = apply_column_drag(
            conversations.clone(),
            Divider::RightPairSplit,
            1440.0,
            696.0 + 344.0,
        );
        assert!(
            (l.conversations_split - 0.5).abs() < 0.001,
            "{}",
            l.conversations_split
        );
        assert_eq!(l.agent_split, conversations.dims.agent_split);
    }

    /// 中点(0.5)拖到哪都是 0.5,取反前后数值一样,不能证明真的取反了。
    /// 这里挑一个偏离中点的落点单独验证方向:配对渲染顺序是"终端/审阅在
    /// 左、Agent 列表/对话列表在右"(见 `right_panel_area`),拖拽点落在
    /// 离左边缘 1/4 处 → 左边(终端/审阅)只分到 25% 宽 → 右边(列表侧,
    /// `agent_split`/`conversations_split` 存的量)理应分到 75%,而不是 25%。
    #[test]
    fn right_pair_split_ratio_is_inverted_for_swapped_visual_order() {
        // 同上一个测试:右面板区左边缘 x=692,配对内容宽=696。落点在左边缘
        // 往右 174(=696/4)处,即左侧(终端)拿到 1/4 宽。
        let agent = test_state();
        let l = apply_column_drag(agent, Divider::RightPairSplit, 1440.0, 692.0 + 174.0);
        assert!(
            (l.agent_split - 0.75).abs() < 0.001,
            "左侧(终端)占 1/4 时,右侧(Agent 列表)该占 3/4,实得 {}",
            l.agent_split
        );

        let conversations = ShellState {
            right_view: PanelKind::Conversations,
            ..test_state()
        };
        let l = apply_column_drag(
            conversations,
            Divider::RightPairSplit,
            1440.0,
            692.0 + 174.0,
        );
        assert!(
            (l.conversations_split - 0.75).abs() < 0.001,
            "左侧(审阅)占 1/4 时,右侧(对话列表)该占 3/4,实得 {}",
            l.conversations_split
        );
    }

    // ---- README 自动生成 ---- //

    /// 没有 README、没写描述:只生成一个"标题+空行"的最小文档,带项目名。
    #[test]
    fn readme_created_from_name_without_description() {
        let dir = tempfile::tempdir().unwrap();
        let created = ensure_project_readme(dir.path(), "我的项目").unwrap();
        assert_eq!(created, dir.path().join("README.md"));
        let body = std::fs::read_to_string(&created).unwrap();
        assert!(body.starts_with("# 我的项目\n\n"), "实际: {body:?}");
    }

    /// 带有 `.dozer/description.md`:一级标题用项目名,正文接描述。
    #[test]
    fn readme_embeds_description_from_dozer_dir() {
        let dir = tempfile::tempdir().unwrap();
        crate::project_meta::write_description(dir.path(), "这是一段中文描述").unwrap();
        let created = ensure_project_readme(dir.path(), "Demo").unwrap();
        let body = std::fs::read_to_string(&created).unwrap();
        assert_eq!(body, "# Demo\n\n这是一段中文描述\n");
    }

    /// 已存在 README:直接返回它的路径(送到预览窗去展示),绝不重写、绝不
    /// 覆盖用户已有内容。
    #[test]
    fn readme_exists_is_left_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let existing = dir.path().join("README.md");
        std::fs::write(&existing, "用户手写的内容\n").unwrap();
        assert_eq!(
            ensure_project_readme(dir.path(), "D"),
            Some(existing.clone())
        );
        assert_eq!(
            std::fs::read_to_string(&existing).unwrap(),
            "用户手写的内容\n"
        );
    }

    /// 目标是文件而非目录时的拒绝语义,等价于 repo 根不可写/不可用。
    #[test]
    fn readme_missing_on_unwritable_root_returns_none() {
        let file_as_repo = tempfile::tempdir().unwrap();
        let repo_path = file_as_repo.path().join("not_a_dir");
        std::fs::write(&repo_path, "我是文件").unwrap();
        assert_eq!(ensure_project_readme(&repo_path, "D"), None);
    }

    /// `apply_column_drag` 里 Ssh/Todo/GitLog 三个左栏默认面板的 side+镜像
    /// 感知改造测试。默认栏(左)下方向应与改造前固定行为逐字节一致(防回归
    /// 锚);挪到右栏后方向要反转(镜像态下"列表在后")。用 near/far 方向性
    /// 比较,不手算精确数值(同既有
    /// `apply_column_drag_browser_bookmarks_split_direction_matches_content_side`)。
    mod apply_column_drag_ssh_todo_gitlog_mirror_tests {
        use super::*;

        fn right_x0_inside(window_width: f32, state: &ShellState) -> f32 {
            window_width
                - byteui::theme::geometry::icon_rail_width()
                - right_zone_width(window_width, state)
        }

        fn relocate_to_right(state: &mut ShellState, kind: PanelKind) {
            state.layout.rail_layout.left.retain(|&k| k != kind);
            state.layout.rail_layout.right.push(kind);
        }

        #[test]
        fn ssh_split_direction_on_default_side_matches_pre_migration_behavior() {
            let state = test_state();
            let window_width = 1600.0;
            let near = apply_column_drag(state.clone(), Divider::SshSplit, window_width, 300.0);
            let far = apply_column_drag(state, Divider::SshSplit, window_width, 500.0);
            assert!(
                far.ssh_split > near.ssh_split,
                "near={} far={}",
                near.ssh_split,
                far.ssh_split
            );
        }

        #[test]
        fn ssh_split_direction_flips_when_relocated_to_right_side() {
            let mut state = test_state();
            relocate_to_right(&mut state, PanelKind::Ssh);
            let window_width = 1600.0;
            let x0 = right_x0_inside(window_width, &state);
            let near = apply_column_drag(state.clone(), Divider::SshSplit, window_width, x0 + 50.0);
            let far = apply_column_drag(state, Divider::SshSplit, window_width, x0 + 250.0);
            assert!(
                far.ssh_split < near.ssh_split,
                "镜像态下方向应反转:near={} far={}",
                near.ssh_split,
                far.ssh_split
            );
        }

        #[test]
        fn todo_split_direction_on_default_side_matches_pre_migration_behavior() {
            let state = test_state();
            let window_width = 1600.0;
            let near = apply_column_drag(state.clone(), Divider::TodoSplit, window_width, 300.0);
            let far = apply_column_drag(state, Divider::TodoSplit, window_width, 500.0);
            assert!(
                far.todo_split > near.todo_split,
                "near={} far={}",
                near.todo_split,
                far.todo_split
            );
        }

        #[test]
        fn todo_split_direction_flips_when_relocated_to_right_side() {
            let mut state = test_state();
            relocate_to_right(&mut state, PanelKind::Todo);
            let window_width = 1600.0;
            let x0 = right_x0_inside(window_width, &state);
            let near =
                apply_column_drag(state.clone(), Divider::TodoSplit, window_width, x0 + 50.0);
            let far = apply_column_drag(state, Divider::TodoSplit, window_width, x0 + 250.0);
            assert!(
                far.todo_split < near.todo_split,
                "镜像态下方向应反转:near={} far={}",
                near.todo_split,
                far.todo_split
            );
        }

        #[test]
        fn git_log_split_direction_on_default_side_matches_pre_migration_behavior() {
            let state = test_state();
            let window_width = 1600.0;
            let near = apply_column_drag(state.clone(), Divider::GitLogSplit, window_width, 300.0);
            let far = apply_column_drag(state, Divider::GitLogSplit, window_width, 500.0);
            assert!(
                far.git_log_split > near.git_log_split,
                "near={} far={}",
                near.git_log_split,
                far.git_log_split
            );
        }

        #[test]
        fn git_log_split_direction_flips_when_relocated_to_right_side() {
            let mut state = test_state();
            relocate_to_right(&mut state, PanelKind::GitLog);
            let window_width = 1600.0;
            let x0 = right_x0_inside(window_width, &state);
            let near =
                apply_column_drag(state.clone(), Divider::GitLogSplit, window_width, x0 + 50.0);
            let far = apply_column_drag(state, Divider::GitLogSplit, window_width, x0 + 250.0);
            assert!(
                far.git_log_split < near.git_log_split,
                "镜像态下方向应反转:near={} far={}",
                near.git_log_split,
                far.git_log_split
            );
        }
    }

    /// `apply_column_drag` 里 `LeftPairSplit`(Files)/`ProjectSplit`/
    /// `BrowserBookmarksSplit` 三个本 Stage 改过 base 的分支的 side+镜像
    /// 感知测试(此前硬编码 `left_zone_width` + `icon_rail_width()`,Project
    /// /Web 挪到右栏后拖拽方向直接错乱)。Files/Project 默认"列表在前",
    /// Web 默认"内容在前"且 `browser_bookmarks_split` 存内容占比——方向翻转
    /// 的判定彼此相反,分开写清楚。
    mod apply_column_drag_files_project_web_mirror_tests {
        use super::*;

        fn right_x0_inside(window_width: f32, state: &ShellState) -> f32 {
            window_width
                - byteui::theme::geometry::icon_rail_width()
                - right_zone_width(window_width, state)
        }

        fn relocate_to_right(state: &mut ShellState, kind: PanelKind) {
            state.layout.rail_layout.left.retain(|&k| k != kind);
            state.layout.rail_layout.right.push(kind);
        }

        #[test]
        fn files_split_direction_on_default_side_matches_pre_migration_behavior() {
            let state = test_state();
            let window_width = 1600.0;
            let near =
                apply_column_drag(state.clone(), Divider::LeftPairSplit, window_width, 300.0);
            let far = apply_column_drag(state, Divider::LeftPairSplit, window_width, 500.0);
            assert!(
                far.files_split > near.files_split,
                "near={} far={}",
                near.files_split,
                far.files_split
            );
        }

        #[test]
        fn files_split_direction_flips_when_relocated_to_right_side() {
            let mut state = test_state();
            relocate_to_right(&mut state, PanelKind::Files);
            let window_width = 1600.0;
            let x0 = right_x0_inside(window_width, &state);
            let near = apply_column_drag(
                state.clone(),
                Divider::LeftPairSplit,
                window_width,
                x0 + 50.0,
            );
            let far = apply_column_drag(state, Divider::LeftPairSplit, window_width, x0 + 250.0);
            assert!(
                far.files_split < near.files_split,
                "镜像态下方向应反转:near={} far={}",
                near.files_split,
                far.files_split
            );
        }

        #[test]
        fn project_split_direction_on_default_side_matches_pre_migration_behavior() {
            let state = test_state();
            let window_width = 1600.0;
            let near = apply_column_drag(state.clone(), Divider::ProjectSplit, window_width, 300.0);
            let far = apply_column_drag(state, Divider::ProjectSplit, window_width, 500.0);
            assert!(
                far.project_split > near.project_split,
                "near={} far={}",
                near.project_split,
                far.project_split
            );
        }

        #[test]
        fn project_split_direction_flips_when_relocated_to_right_side() {
            let mut state = test_state();
            relocate_to_right(&mut state, PanelKind::Project);
            let window_width = 1600.0;
            let x0 = right_x0_inside(window_width, &state);
            let near = apply_column_drag(
                state.clone(),
                Divider::ProjectSplit,
                window_width,
                x0 + 50.0,
            );
            let far = apply_column_drag(state, Divider::ProjectSplit, window_width, x0 + 250.0);
            assert!(
                far.project_split < near.project_split,
                "镜像态下方向应反转:near={} far={}",
                near.project_split,
                far.project_split
            );
        }

        /// 拖 `ProjectSplit` 把信息面板拖到比 footer 两个按钮(`修复项目`/
        /// `删除项目`)还窄时,应直接收起而不是继续缩小,并冻结上一次的
        /// `project_split`(不写入挤爆按钮的小比例)——等同点了收起按钮,
        /// 见 `apply_column_drag` 该分支的文档。
        #[test]
        fn project_split_collapses_when_dragged_narrower_than_footer_buttons() {
            let state = test_state();
            let window_width = 1600.0;
            let (x0, pair_w) = pair_x0_and_width(
                state.layout.rail_layout.side_of(PanelKind::Project),
                window_width,
                &state,
            );
            let target_ratio = (project::footer_min_width() - 10.0) / pair_w;
            let logical_x = x0 + target_ratio * pair_w;
            let result = apply_column_drag(
                state.clone(),
                Divider::ProjectSplit,
                window_width,
                logical_x,
            );
            assert!(
                result.project_list_collapsed,
                "低于两按钮宽度应直接收起面板"
            );
            assert_eq!(
                result.project_split, state.dims.project_split,
                "收起时应冻结上一次的 project_split,不写入挤爆按钮的小比例"
            );
        }

        /// 对称场景:拖回比两按钮宽度更宽时应保持/恢复展开。
        #[test]
        fn project_split_stays_expanded_when_wider_than_footer_buttons() {
            let state = test_state();
            let window_width = 1600.0;
            let (x0, pair_w) = pair_x0_and_width(
                state.layout.rail_layout.side_of(PanelKind::Project),
                window_width,
                &state,
            );
            let target_ratio = (project::footer_min_width() + 20.0) / pair_w;
            let logical_x = x0 + target_ratio * pair_w;
            let result = apply_column_drag(state, Divider::ProjectSplit, window_width, logical_x);
            assert!(!result.project_list_collapsed, "宽于两按钮宽度不应收起面板");
        }

        #[test]
        fn browser_bookmarks_split_direction_on_default_side_matches_pre_migration_behavior() {
            // Web 默认"内容在前"且字段存内容占比:默认栏(左)下拖拽点越靠右,
            // 内容占比越大(与既有 `browser_bookmarks_split_direction_matches_content_side`
            // 一致的方向锚)。
            let state = test_state();
            let window_width = 1600.0;
            let near = apply_column_drag(
                state.clone(),
                Divider::BrowserBookmarksSplit,
                window_width,
                100.0,
            );
            let far = apply_column_drag(state, Divider::BrowserBookmarksSplit, window_width, 500.0);
            assert!(
                far.browser_bookmarks_split > near.browser_bookmarks_split,
                "near={} far={}",
                near.browser_bookmarks_split,
                far.browser_bookmarks_split
            );
        }

        #[test]
        fn browser_bookmarks_split_direction_flips_when_relocated_to_right_side() {
            let mut state = test_state();
            relocate_to_right(&mut state, PanelKind::Web);
            let window_width = 1600.0;
            let x0 = right_x0_inside(window_width, &state);
            let near = apply_column_drag(
                state.clone(),
                Divider::BrowserBookmarksSplit,
                window_width,
                x0 + 50.0,
            );
            let far = apply_column_drag(
                state,
                Divider::BrowserBookmarksSplit,
                window_width,
                x0 + 250.0,
            );
            assert!(
                far.browser_bookmarks_split < near.browser_bookmarks_split,
                "镜像态下方向应反转:near={} far={}",
                near.browser_bookmarks_split,
                far.browser_bookmarks_split
            );
        }
    }

    /// `apply_column_drag` 里 `RightPairSplit`(Agent/Conversations)的
    /// side+镜像感知改造测试。默认在右栏、默认"内容在前"——默认栏下拖拽点
    /// 越靠右,内容占比越大、列表占比越*小*;挪到左栏后镜像成"列表在前",
    /// 方向反转。
    mod apply_column_drag_right_pair_mirror_tests {
        use super::*;

        #[test]
        fn agent_split_direction_on_default_side_matches_pre_migration_behavior() {
            let state = test_state(); // right_view 已经是 Agent
            let window_width = 1600.0;
            let right_x0 = window_width
                - byteui::theme::geometry::icon_rail_width()
                - right_zone_width(window_width, &state);
            let near = apply_column_drag(
                state.clone(),
                Divider::RightPairSplit,
                window_width,
                right_x0 + 50.0,
            );
            let far = apply_column_drag(
                state,
                Divider::RightPairSplit,
                window_width,
                right_x0 + 250.0,
            );
            assert!(
                far.agent_split < near.agent_split,
                "near={} far={}",
                near.agent_split,
                far.agent_split
            );
        }

        #[test]
        fn agent_split_direction_flips_when_relocated_to_left_side() {
            let mut state = test_state();
            state
                .layout
                .rail_layout
                .right
                .retain(|&k| k != PanelKind::Agent);
            state.layout.rail_layout.left.push(PanelKind::Agent);
            let window_width = 1600.0;
            let near = apply_column_drag(
                state.clone(),
                Divider::RightPairSplit,
                window_width,
                byteui::theme::geometry::icon_rail_width() + 50.0,
            );
            let far = apply_column_drag(
                state,
                Divider::RightPairSplit,
                window_width,
                byteui::theme::geometry::icon_rail_width() + 250.0,
            );
            assert!(
                far.agent_split > near.agent_split,
                "镜像态下方向应反转:near={} far={}",
                near.agent_split,
                far.agent_split
            );
        }

        #[test]
        fn conversations_split_direction_on_default_side_matches_pre_migration_behavior() {
            let mut state = test_state();
            state.right_view = PanelKind::Conversations;
            let window_width = 1600.0;
            let right_x0 = window_width
                - byteui::theme::geometry::icon_rail_width()
                - right_zone_width(window_width, &state);
            let near = apply_column_drag(
                state.clone(),
                Divider::RightPairSplit,
                window_width,
                right_x0 + 50.0,
            );
            let far = apply_column_drag(
                state,
                Divider::RightPairSplit,
                window_width,
                right_x0 + 250.0,
            );
            assert!(
                far.conversations_split < near.conversations_split,
                "near={} far={}",
                near.conversations_split,
                far.conversations_split
            );
        }

        #[test]
        fn conversations_split_direction_flips_when_relocated_to_left_side() {
            let mut state = test_state();
            state.right_view = PanelKind::Conversations;
            state
                .layout
                .rail_layout
                .right
                .retain(|&k| k != PanelKind::Conversations);
            state.layout.rail_layout.left.push(PanelKind::Conversations);
            let window_width = 1600.0;
            let near = apply_column_drag(
                state.clone(),
                Divider::RightPairSplit,
                window_width,
                byteui::theme::geometry::icon_rail_width() + 50.0,
            );
            let far = apply_column_drag(
                state,
                Divider::RightPairSplit,
                window_width,
                byteui::theme::geometry::icon_rail_width() + 250.0,
            );
            assert!(
                far.conversations_split > near.conversations_split,
                "镜像态下方向应反转:near={} far={}",
                near.conversations_split,
                far.conversations_split
            );
        }
    }

    /// 拖各面板分隔线把列表侧拖到比 `project::footer_min_width`(所有可
    /// 收纳面板统一共用这一份估算——按用户要求"所有面板最小宽度都和项目
    /// 面板一样,就是两个按钮的宽度",不是各面板各按自己的 footer/头部再
    /// 单独估一套)还窄时应直接收起、冻结上一次的 split 比例,拖回超过阈值
    /// 时对称展开——同 `apply_column_drag_files_project_web_mirror_tests`
    /// 里 `project_split_collapses_when_dragged_narrower_than_footer_buttons`
    /// 一套口径,这里补齐其余面板(Files 有专门的 `clamp_files_split_to_range`
    /// 覆盖,这里不重复)。默认 640 宽面板区下 `min_split_ratio()`(0.2)
    /// 算出的像素宽(0.2×632≈126px)已经小于这份阈值(≈198px),所以直接用
    /// `test_state()`/1600 窗口宽就能触发,不需要额外收窄面板区。
    mod apply_column_drag_collapse_tests {
        use super::*;

        #[test]
        fn ssh_split_collapses_when_narrower_than_footer_button() {
            let state = test_state();
            let window_width = 1600.0;
            let (x0, pair_w) = pair_x0_and_width(
                state.layout.rail_layout.side_of(PanelKind::Ssh),
                window_width,
                &state,
            );
            let target_ratio = (project::footer_min_width() - 10.0) / pair_w;
            let logical_x = x0 + target_ratio * pair_w;
            let result =
                apply_column_drag(state.clone(), Divider::SshSplit, window_width, logical_x);
            assert!(result.ssh_list_collapsed);
            assert_eq!(result.ssh_split, state.dims.ssh_split);
        }

        #[test]
        fn ssh_split_stays_expanded_when_wider_than_footer_button() {
            let state = test_state();
            let window_width = 1600.0;
            let (x0, pair_w) = pair_x0_and_width(
                state.layout.rail_layout.side_of(PanelKind::Ssh),
                window_width,
                &state,
            );
            let target_ratio = (project::footer_min_width() + 20.0) / pair_w;
            let logical_x = x0 + target_ratio * pair_w;
            let result = apply_column_drag(state, Divider::SshSplit, window_width, logical_x);
            assert!(!result.ssh_list_collapsed);
        }

        #[test]
        fn todo_split_collapses_when_narrower_than_footer_button() {
            let state = test_state();
            let window_width = 1600.0;
            let (x0, pair_w) = pair_x0_and_width(
                state.layout.rail_layout.side_of(PanelKind::Todo),
                window_width,
                &state,
            );
            let target_ratio = (project::footer_min_width() - 10.0) / pair_w;
            let logical_x = x0 + target_ratio * pair_w;
            let result =
                apply_column_drag(state.clone(), Divider::TodoSplit, window_width, logical_x);
            assert!(result.todo_list_collapsed);
            assert_eq!(result.todo_split, state.dims.todo_split);
        }

        #[test]
        fn todo_split_stays_expanded_when_wider_than_footer_button() {
            let state = test_state();
            let window_width = 1600.0;
            let (x0, pair_w) = pair_x0_and_width(
                state.layout.rail_layout.side_of(PanelKind::Todo),
                window_width,
                &state,
            );
            let target_ratio = (project::footer_min_width() + 20.0) / pair_w;
            let logical_x = x0 + target_ratio * pair_w;
            let result = apply_column_drag(state, Divider::TodoSplit, window_width, logical_x);
            assert!(!result.todo_list_collapsed);
        }

        #[test]
        fn database_split_collapses_when_narrower_than_footer_buttons() {
            let state = test_state();
            let window_width = 1600.0;
            let (x0, pair_w) = pair_x0_and_width(
                state.layout.rail_layout.side_of(PanelKind::Database),
                window_width,
                &state,
            );
            let target_ratio = (project::footer_min_width() - 10.0) / pair_w;
            let logical_x = x0 + target_ratio * pair_w;
            let result = apply_column_drag(
                state.clone(),
                Divider::DatabaseSplit,
                window_width,
                logical_x,
            );
            assert!(result.database_list_collapsed);
            assert_eq!(result.database_split, state.dims.database_split);
        }

        #[test]
        fn database_split_stays_expanded_when_wider_than_footer_buttons() {
            let state = test_state();
            let window_width = 1600.0;
            let (x0, pair_w) = pair_x0_and_width(
                state.layout.rail_layout.side_of(PanelKind::Database),
                window_width,
                &state,
            );
            let target_ratio = (project::footer_min_width() + 20.0) / pair_w;
            let logical_x = x0 + target_ratio * pair_w;
            let result = apply_column_drag(state, Divider::DatabaseSplit, window_width, logical_x);
            assert!(!result.database_list_collapsed);
        }

        /// Usage 默认"内容在前、筛选栏(list)在后",list 占比是
        /// `1 - raw_ratio`(见 `Divider::UsageSplit` 分支),所以要把 list
        /// 拖窄得把 `logical_x` 推得更靠右——不是简单套用 Ssh/Todo/Database
        /// 那套"list 在前"的坐标公式;Agent/Conversations(`RightPairSplit`)
        /// 同一个方向。
        #[test]
        fn usage_split_collapses_when_narrower_than_header() {
            let state = test_state();
            let window_width = 1600.0;
            let (x0, pair_w) = pair_x0_and_width(
                state.layout.rail_layout.side_of(PanelKind::Usage),
                window_width,
                &state,
            );
            let target_list_ratio = (project::footer_min_width() - 10.0) / pair_w;
            let logical_x = x0 + (1.0 - target_list_ratio) * pair_w;
            let result =
                apply_column_drag(state.clone(), Divider::UsageSplit, window_width, logical_x);
            assert!(result.usage_list_collapsed);
            assert_eq!(result.usage_split, state.dims.usage_split);
        }

        #[test]
        fn usage_split_stays_expanded_when_wider_than_header() {
            let state = test_state();
            let window_width = 1600.0;
            let (x0, pair_w) = pair_x0_and_width(
                state.layout.rail_layout.side_of(PanelKind::Usage),
                window_width,
                &state,
            );
            let target_list_ratio = (project::footer_min_width() + 20.0) / pair_w;
            let logical_x = x0 + (1.0 - target_list_ratio) * pair_w;
            let result = apply_column_drag(state, Divider::UsageSplit, window_width, logical_x);
            assert!(!result.usage_list_collapsed);
        }

        #[test]
        fn agent_split_collapses_when_narrower_than_header() {
            let state = test_state();
            let window_width = 1600.0;
            let (x0, pair_w) = pair_x0_and_width(
                state.layout.rail_layout.side_of(PanelKind::Agent),
                window_width,
                &state,
            );
            let target_list_ratio = (project::footer_min_width() - 10.0) / pair_w;
            let logical_x = x0 + (1.0 - target_list_ratio) * pair_w;
            let result = apply_column_drag(
                state.clone(),
                Divider::RightPairSplit,
                window_width,
                logical_x,
            );
            assert!(result.agent_list_collapsed);
            assert_eq!(result.agent_split, state.dims.agent_split);
        }

        #[test]
        fn agent_split_stays_expanded_when_wider_than_header() {
            let state = test_state();
            let window_width = 1600.0;
            let (x0, pair_w) = pair_x0_and_width(
                state.layout.rail_layout.side_of(PanelKind::Agent),
                window_width,
                &state,
            );
            let target_list_ratio = (project::footer_min_width() + 20.0) / pair_w;
            let logical_x = x0 + (1.0 - target_list_ratio) * pair_w;
            let result = apply_column_drag(state, Divider::RightPairSplit, window_width, logical_x);
            assert!(!result.agent_list_collapsed);
        }

        #[test]
        fn conversations_split_collapses_when_narrower_than_footer() {
            let mut state = test_state();
            state.right_view = PanelKind::Conversations;
            let window_width = 1600.0;
            let (x0, pair_w) = pair_x0_and_width(
                state.layout.rail_layout.side_of(PanelKind::Conversations),
                window_width,
                &state,
            );
            let target_list_ratio = (project::footer_min_width() - 10.0) / pair_w;
            let logical_x = x0 + (1.0 - target_list_ratio) * pair_w;
            let result = apply_column_drag(
                state.clone(),
                Divider::RightPairSplit,
                window_width,
                logical_x,
            );
            assert!(result.conversations_list_collapsed);
            assert_eq!(result.conversations_split, state.dims.conversations_split);
        }

        #[test]
        fn conversations_split_stays_expanded_when_wider_than_footer() {
            let mut state = test_state();
            state.right_view = PanelKind::Conversations;
            let window_width = 1600.0;
            let (x0, pair_w) = pair_x0_and_width(
                state.layout.rail_layout.side_of(PanelKind::Conversations),
                window_width,
                &state,
            );
            let target_list_ratio = (project::footer_min_width() + 20.0) / pair_w;
            let logical_x = x0 + (1.0 - target_list_ratio) * pair_w;
            let result = apply_column_drag(state, Divider::RightPairSplit, window_width, logical_x);
            assert!(!result.conversations_list_collapsed);
        }
    }
}
