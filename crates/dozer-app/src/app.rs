// crates/dozer-app/src/app.rs
//! 整个程序只有一份的外壳态:daemon 客户端、窗口尺寸、图标栏/面板区的
//! 收起-拖宽-放大状态、并行打开的 N 个项目页签。`App::view()`/`update()`
//! 是 iced 应用的顶层入口,`Message` 是它的消息协议。
//!
//! 与 `Workspace`(单个项目的全部状态,见 `crate::workspace`)的拆分边界:
//! `impl Workspace` 的方法零处依赖 `App`(`ShellIo` 就是为此存在的解耦
//! 值类型),因此依赖方向是单向的——本文件 `use crate::workspace::
//! Workspace;`,`workspace.rs` 只反向借用本文件的 `Message` 一个类型
//! (`Workspace` 的异步方法要构造 `Message` 变体经 `EventLoopProxy`
//! 回传)。拆分细节见
//! `docs/superpowers/specs/2026-08-08-app-workspace-file-split-design.md`.

use crate::conversation::ConversationMeta;
use crate::delivery;
use crate::extensions::acceptance;
use crate::extensions::browser;
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
use crate::icons;
use crate::layout;
use crate::open_projects;
use crate::panel_layouts;
use crate::preview::WebviewSpec;
use crate::tabs;
use crate::term_view;
use crate::theme;
use crate::transcript::ReviewEntry;
use crate::workspace::{
    PickerLaunch, RestorePayload, ReviewSource, ReviewView, SessionTab, ShellIo, SshOut,
    TabBackend, Workspace, agent_list_pane, agent_picker_popup, conversation_list_pane, dot_color,
    edit_discard_confirm_popup, edit_modal, effective_project_repo, exited_marker,
    fetch_project_restore, no_project_placeholder, preview_pane, project_preview_pane,
    review_content_pane, review_should_refresh_on_turn, spawn_disk_usage_refresh,
    spawn_project_git_refresh, split_portions, tab_display_width, tab_title, terminal_status_bar,
};
use dozer_client::Client;
use dozer_core::protocol::{AgentKind, AgentState, BookmarkInfo, ProjectInfo, SessionInfo};
use iced_widget::core::border::Radius;
use iced_widget::core::font::Weight;
use iced_widget::core::mouse;
use iced_widget::core::{Border, Color, Element, Font, Length, Padding};
use iced_widget::{MouseArea, button, column, container, responsive, row, stack, text};
use iced_winit::winit::event_loop::EventLoopProxy;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::runtime::Handle;

/// 终端初始网格尺寸（列 x 行）。真实尺寸由窗口创建后的第一次
/// `Message::PaneResized` 立刻纠正（见 `main.rs` 的 `resumed()`）；这里只是
/// "窗口还没量出真实像素前"的兜底默认值。
pub(crate) const DEFAULT_COLS: u16 = 80;

pub(crate) const DEFAULT_ROWS: u16 = 24;

/// 左侧面板区当前显示哪个视图：文件列表(项目树+文件预览配对) /
/// Git 提交图(单面板;spike(2026-08-06) 验证 `gleisbau` 库可行性用) / Todo
/// (单面板;`.dozer/todo.md` 任务列表) / 浏览器(Web 单面板,左图标栏最底部
/// 的 Globe 按钮切换;2026-08-11 曾短暂迁至右栏,同日按用户
/// 要求移回左栏)。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum LeftView {
    Files,
    GitLog,
    Todo,
    Project,
    Database,
    /// SSH 远程主机面板(Lucide server)。阶段 1:主机连接管理 + 认证 +
    /// 连接测试(含 host key 验证)。
    Ssh,
    /// 浏览器(Web)单面板:左图标栏最底部的 Globe 按钮切换。
    Web,
}

/// 右侧面板区当前显示哪个视图：Agent(Agent列表+终端配对) / 对话(对话列表+
/// 对话审阅配对) / 用量 / 验收。浏览器已移回左栏(见 `LeftView::Web`)。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum RightView {
    Agent,
    Conversations,
    Usage,
    Acceptance,
}

/// 四个图标栏按钮的标识,用于追踪 hover 态(图标颜色在 hover 时需变金,
/// 而 SVG 颜色在构建时就定死、不随 `button::Status` 变化,所以得在 App
/// 里记一个 hovered 目标,改色时按它重算)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RailButton {
    LeftFiles,
    LeftGit,
    LeftTodo,
    LeftProject,
    LeftDatabase,
    /// SSH 主机面板入口。
    LeftSsh,
    /// 浏览器面板入口(左图标栏最底部 Globe 按钮)。
    LeftWeb,
    RightAgent,
    RightConversations,
    RightUsage,
    RightAcceptance,
    /// 首页左栏"项目列表" pane 图标。
    HomeProjectList,
    /// 首页左栏"Recents" pane 图标。
    HomeRecents,
    /// 首页右栏"浏览器" pane 图标。
    HomeBrowser,
}

/// 顶栏右侧按钮(添加项目 / 设置)的标识,用于追踪 hover 态(图标颜色在
/// hover 时需变金,而 SVG 颜色在构建时就定死、不随 `button::Status` 变化,
/// 所以得在 App 里记一个 hovered 目标,改色时按它重算)。语义同
/// `RailButton`,只是作用域在顶栏而非图标栏。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TopbarButton {
    AddProject,
    Settings,
}

/// 所有需要"悬停平滑过渡动画"的按钮的统一标识。把顶栏右侧按钮
/// (`TopbarButton`)、图标栏按钮(`RailButton`)收进同一个
/// 枚举,这样它们能共用一套 `hover_anims` 状态机与同一条自驱 redraw 定时
/// 唤醒(见 `App::set_hover`/`advance_hover_anims`/`hover_progress`),不必
/// 每个按钮各写一套进度字段。顶栏 Home 品牌页签的标题文字 hover 时也走这个
/// 动画表(从 DIM 平滑过渡到 GOLD),与项目页签一致;选中态恒为 GOLD(见
/// `dozer_home_tab`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HoverId {
    Topbar(TopbarButton),
    Rail(RailButton),
    /// 某个项目页签的关闭按钮(`×`),按项目 id 区分——同一时刻可能有多个
    /// 页签,不能像 `TopbarButton`/`RailButton` 那样用一个全局标识共用。
    ProjectTabClose(i64),
    /// 某个项目页签的标题文字,按项目 id 区分(同 `ProjectTabClose` 的考量):
    /// 未选中态 hover 时标题从 DIM 平滑过渡到 GOLD(见 `project_tab_item`)。
    ProjectTabItem(i64),
    /// 终端面板某个会话 tab 的标题文字,按会话序号区分——未选中态 hover 时
    /// 标题从 DIM 平滑过渡到 GOLD(见 `panel_tab`)。
    TermTabItem(usize),
    /// 终端面板某个会话 tab 的关闭按钮(×),按会话序号区分——hover 时颜色从
    /// DIM 平滑过渡到 GOLD(见 `panel_tab`)。
    TermTabClose(usize),
    /// 预览面板某个文件 tab 的标题文字,按 tab 序号区分(同 `TermTabItem`)。
    PreviewTabItem(usize),
    /// 预览面板某个文件 tab 的关闭按钮(×),按 tab 序号区分(同 `TermTabClose`)。
    PreviewTabClose(usize),
    /// Project 面板右配对预览某个文件 tab 的标题文字,按 tab 序号区分。
    ProjectPreviewTabItem(usize),
    /// Project 面板右配对预览某个文件 tab 的关闭按钮(×),按 tab 序号区分。
    ProjectPreviewTabClose(usize),
    /// SSH 面板自己 tab 条上某个 tab 的标题文字,按 host_id 的哈希区分
    /// (SSH tab 没有稳定的数字序号——按身份是 `(host_id, SshTabKind)`,
    /// `HoverId` 整体 `derive(Copy)`,`String` 不是 `Copy`,不能直接塞
    /// `host_id.clone()`,用哈希值退化成 `u64`,同 `todo_line_key` 的
    /// 既有精度取舍)。
    SshTabItem(u64),
    /// SSH 面板自己 tab 条上某个 tab 的关闭按钮(×),同上按 host_id 哈希区分。
    SshTabClose(u64),
    /// 顶栏 Dozer Home 品牌页签的标题文字(图标 + "Dozer"):未选中态 hover 时
    /// 从 DIM 平滑过渡到 GOLD,选中态恒为 GOLD——与 `ProjectTabItem` 同一手法
    /// (见 `dozer_home_tab`)。
    HomeTab,
    /// Agent 面板头部"＋"按钮:无背景的 `SquarePlus` 图标,未选中态静止 DIM,
    /// hover 平滑过渡到 GOLD(见 `agent_picker_toggle_button`)。
    AgentPickerToggle,
    /// Usage 用量面板"手动刷新"按钮(`RefreshCw`):静止 DIM,hover 平滑过渡
    /// 到 GOLD(见 `extensions/usage.rs`)。
    UsageRefresh,
    /// 文件树搜索提交按钮(`FolderSearch`):静止 DIM,hover 过渡到 GOLD
    /// (见 `extensions/files.rs` 的搜索按钮)。
    FilesSearchSubmit,
    /// 文件树"显示隐藏文件"切换按钮(`Eye`/`EyeOff`):静止 DIM,hover 过渡
    /// 到 GOLD;已开启(隐藏文件可见)恒金(见 `extensions/files.rs`)。
    FilesDotfiles,
    /// 文件树底栏 git 分支切换按钮(`ChevronDown`):静止 DIM,hover 过渡到
    /// GOLD(见 `extensions/files.rs` 的 `git_footer_bar`)。
    FilesBranchSwitch,
    /// 数据库面板 schema 树"返回"按钮(`ChevronLeft`):静止 DIM,hover 过渡
    /// 到 GOLD(见 `extensions/database.rs`)。
    DatabaseSchemaBack,
}

/// 一个可平滑过渡的 hover 动画状态机。iced 0.14 无内置动画 API,这套自驱
/// redraw(与光标闪烁同款范式)把图标/背景颜色在 idle↔hover 之间做 ease-out
/// 插值,而非硬切。`progress` 朝 `target`(0 或 1)指数逼近,约 80ms 收敛。
#[derive(Debug, Clone, Copy, Default)]
struct HoverAnim {
    /// 当前帧插值系数(0..=1)。
    progress: f32,
    /// 动画目标(0=未悬停,1=悬停)。
    target: f32,
}

impl HoverAnim {
    /// 设置悬停目标(`true`=进入,`false`=离开);动画由 `advance` 循环逼近。
    fn set(&mut self, hovered: bool) {
        self.target = if hovered { 1.0 } else { 0.0 };
    }
    /// 朝目标逼近一拍(每拍残余 50%),足够接近则 snap 到目标避免无限抖动。
    fn advance(&mut self) {
        let next = self.progress + (self.target - self.progress) * 0.5;
        self.progress = if (next - self.target).abs() < 0.01 {
            self.target
        } else {
            next
        };
    }
    /// 动画是否仍在进行中(进度未到目标)。
    fn active(&self) -> bool {
        (self.progress - self.target).abs() > 0.001
    }
    /// 当前插值系数,给视图层做颜色插值。
    fn t(&self) -> f32 {
        self.progress
    }
}

/// 顶层级页面：工作区(默认,左右面板区+页签) / 首页落地页(点顶栏 Dozer 进入)。
/// 默认 `Workspace`——程序启动照常进工作区,Home 是用户主动点击 Dozer 才进。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AppPage {
    #[default]
    Workspace,
    Home,
}

/// 当前放大态：放大的是左面板区的内容子面板，还是右面板区的。`None` = 未放大。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MaximizedPane {
    Left,
    Right,
}

/// 当前"聚焦"在哪一侧面板区:左键点击落点决定(见 [`zone_at_x`]),用于
/// `left_zone`/`right_zone` 外边框的高亮态——点哪侧,哪侧的边框就变亮
/// (GOLD),不区分具体点中区内哪个 pane(项目树/预览/终端/Agent 列表等）。
/// 点在图标栏/分隔线上不改变当前态(`zone_at_x` 返回 `None`)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ZoneSide {
    Left,
    Right,
}

/// 单个项目页签的加载状态：懒加载用。`Stub` 只有页签渲染需要的最小信息
/// （启动恢复时,还没被聚焦过的页签停在这一态）,`Loaded` 是完整的
/// `Workspace`（P2a 多项目并行）。
pub enum WorkspaceSlot {
    Stub {
        info: ProjectInfo,
        /// 该项目在**启动恢复那一刻**的后台活动状态(见 [`stub_activity`])。
        /// `None` = 那时没有存活会话,不画指示点。
        ///
        /// 这份快照之后不会再更新——`Stub` 没有任何会话事件流,真正的实时
        /// 指示点要等它被促成成 `Loaded`。写成"restart 时已知"而不是空白,
        /// 是因为重启后**所有**后台页签都是 `Stub`,指示点全空等于整个功能
        /// 在最常见的场景下不工作(最终审查 Required Fix #5)。
        activity: Option<AgentState>,
    },
    Loaded(Box<Workspace>),
}

/// 图标栏+左右面板区的宽度/分割状态。取代 `PanelLayout`——不再有"项目栏/AI栏
/// 固定宽+预览终端共享比例"这套四栏几何，改成"左面板区总宽(可拖) + 三个
/// 配对视图各自独立记住的内部列表:内容分割比例"。右面板区总宽不持久化，
/// 恒为剩余空间(`Length::Fill`)——只有一条 LeftRight 分隔线，不需要像旧
/// 模型那样两个固定宽度各自夹一条。
///
/// `#[serde(default)]`:今后加字段时,老 `layout.json` 里缺的字段用
/// `Default` 补齐,而不是整份反序列化失败 → `unwrap_or_default()` 把用户
/// 攒下来的宽度/比例全部重置。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShellLayout {
    /// 上次退出时的窗口逻辑尺寸(宽,高)。`main.rs` 建窗时读它决定初始
    /// `with_inner_size`,取代写死的 `theme::geometry::initial_window_size()`；`App::
    /// persist_window_size_on_exit` 在 `WindowEvent::CloseRequested` 时
    /// 写回。跟其余字段一样走 `#[serde(default)]`,老 `layout.json` 缺这
    /// 两个字段时退化成 `theme::geometry::initial_window_size()`,不影响其余已存的偏好。
    ///
    /// 左右面板区的宽度/分割比例(`left_width` 与四个 split)已迁进每项目
    /// `PanelLayout`(见 `PanelDims`/`panel_layouts.json`),`ShellLayout`
    /// 不再持有——它们是 per-project 偏好,切项目要各自换,放这里会全局
    /// 共享(见切换项目 bug)。只有窗口尺寸是全局的,留在这里。
    pub window_width: f32,
    pub window_height: f32,
}

impl Default for ShellLayout {
    fn default() -> Self {
        Self {
            window_width: theme::geometry::initial_window_size().0,
            window_height: theme::geometry::initial_window_size().1,
        }
    }
}

/// 左右面板区各维度尺寸(每项目一份)。原来是 `ShellLayout` 的字段(全局共享),
/// 迁进 `PanelDims` 后挂在每项目 `PanelLayout` 上,按项目 id 记到
/// `panel_layouts.json`。`left_width` 是左面板区宽度;四个 split 是各配对视图
/// 内部的分割比例。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PanelDims {
    pub left_width: f32,
    /// 文件列表配对:项目树占左面板区宽度的比例，文件预览拿剩下的。
    pub files_split: f32,
    /// Project 面板配对:信息面板占左面板区宽度的比例，项目预览(右配对)拿剩下的。
    pub project_split: f32,
    /// SSH 面板"主机列表 | 内嵌终端"两栏的分屏比例,镜像 `project_split`。
    pub ssh_split: f32,
    /// Todo 面板配对:分类导航占左面板区宽度的比例，列表/看板/MARKDOWN 内容
    /// (右配对)拿剩下的。
    pub todo_split: f32,
    /// Agent配对:Agent列表占右面板区宽度的比例，终端拿剩下的。
    pub agent_split: f32,
    /// 对话配对:对话列表占右面板区宽度的比例，对话审阅拿剩下的。
    pub conversations_split: f32,
}

/// 每项目尺寸的默认值(数值来源统一从这取,迁走的 `ShellLayout::default()`
/// 就是这组)。作为 `PanelDims::default()`。
/// 四个 split 用同一个 `default_split_ratio()`:所有左右双栏 zone(文件/项目/
/// Agent/对话)的初始宽度分配统一。
fn default_panel_dims() -> PanelDims {
    PanelDims {
        left_width: 640.0,
        files_split: theme::geometry::default_split_ratio(),
        project_split: theme::geometry::default_split_ratio(),
        ssh_split: theme::geometry::default_split_ratio(),
        todo_split: theme::geometry::default_split_ratio(),
        agent_split: theme::geometry::default_split_ratio(),
        conversations_split: theme::geometry::default_split_ratio(),
    }
}

impl Default for PanelDims {
    fn default() -> Self {
        default_panel_dims()
    }
}

/// 每个项目各自记住的面板布局:左右面板区当前显示的配对视图、以及左右
/// 面板区是否收起。原来这几项是全局 `ShellLayout` 的字段,所有项目共享同一
/// 份,导致"在项目 A 改完面板,切到项目 B 时 B 被 A 的面板状态盖掉"(切换
/// 项目 bug)。改成按项目 id 记到 `panel_layouts.json`(见 `panel_layouts`
/// 模块),切换项目只换当前这份、绝不碰别的项目那份。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct PanelLayout {
    pub left_view: LeftView,
    pub right_view: RightView,
    pub left_collapsed: bool,
    pub right_collapsed: bool,
    /// 本项目的面板区尺寸(左宽 + 四个 split)。`#[serde(default)]` 对老
    /// `panel_layouts.json` 缺尺寸字段时补 `PanelDims::default()`;是否用
    /// 全局旧值/12% 回填见 `panel_layouts::load_from`。
    pub dims: PanelDims,
}

impl Default for PanelLayout {
    fn default() -> Self {
        Self {
            left_view: LeftView::Files,
            right_view: RightView::Agent,
            left_collapsed: false,
            right_collapsed: false,
            dims: PanelDims::default(),
        }
    }
}

/// 把从磁盘读回来的 `ShellLayout` 夹进合法范围(`layout::load_from` 调用)。
/// 迁走面板尺寸后只剩窗口尺寸:夹下限(`theme::geometry::min_window_width()`/
/// `theme::geometry::min_window_height()`,建窗时还有 `with_min_inner_size`
/// 兜底),非法值(非有限数、缺字段的 0.0)退化成
/// `theme::geometry::initial_window_size()`。
pub fn sanitize_shell_layout(l: ShellLayout) -> ShellLayout {
    ShellLayout {
        window_width: if l.window_width.is_finite() && l.window_width > 0.0 {
            l.window_width.max(theme::geometry::min_window_width())
        } else {
            theme::geometry::initial_window_size().0
        },
        window_height: if l.window_height.is_finite() && l.window_height > 0.0 {
            l.window_height.max(theme::geometry::min_window_height())
        } else {
            theme::geometry::initial_window_size().1
        },
    }
}

/// 把从磁盘读回来(或首次默认算出)的面板区尺寸夹进合法范围(`panel_layouts::
/// load_from` 与每项目 adopt 调用)。四个 split 用与拖拽同一对上下界:比例恰为
/// 0.0/1.0 时 `split_portions` 会给出 `FillPortion(0)`,那一块在 flex 里拿不到
/// 任何宽度、整块消失;`left_width` 只保下限(上限依赖窗口宽,由渲染/几何时刻
/// 的 `clamp_left_width` 负责,不在这里写死)。
pub fn sanitize_panel_dims(d: PanelDims) -> PanelDims {
    let clamp_split = |v: f32| {
        if v.is_finite() {
            v.clamp(
                theme::geometry::min_split_ratio(),
                theme::geometry::max_split_ratio(),
            )
        } else {
            PanelDims::default().files_split
        }
    };
    PanelDims {
        left_width: if d.left_width.is_finite() {
            d.left_width.max(theme::geometry::min_zone_width())
        } else {
            PanelDims::default().left_width
        },
        files_split: clamp_split(d.files_split),
        project_split: clamp_split(d.project_split),
        ssh_split: clamp_split(d.ssh_split),
        todo_split: clamp_split(d.todo_split),
        agent_split: clamp_split(d.agent_split),
        conversations_split: clamp_split(d.conversations_split),
    }
}

/// 新外壳的三条可拖拽分隔线：左右面板区之间、左侧配对视图内部、右侧配对
/// 视图内部。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Divider {
    LeftRight,
    LeftPairSplit,
    /// Project 面板内部的配对分隔线:左边信息面板、右边项目预览。
    ProjectSplit,
    /// SSH 面板内部的分隔线:左边主机列表、右边内嵌终端。
    SshSplit,
    /// Todo 面板内部的配对分隔线:左边分类导航、右边列表/看板/MARKDOWN 内容。
    TodoSplit,
    RightPairSplit,
}

/// 参与拖拽换位的四种 tab 组：顶栏项目页签、终端会话页签、预览页签、浏览器
/// 页签。`main.rs` 在拖拽中只知道"当前在拖哪个组"，据此转发光标位置；真正
/// 的换位发生在 `App::update`（持有各组的状态与纯函数算宽度的能力）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabGroup {
    Project,
    Terminal,
    Preview,
    /// Project 面板右配对的预览 tab——与 `Preview`(Files 预览)是两套独立
    /// 状态,拖拽换位不能混用,得单独一个组区分。
    ProjectPreview,
    Browser,
}

/// 正在进行的页签拖拽换位。`source` 记拖起时该组里的源下标，换位过程中源
/// 下标会随 `Vec` 移动而更新（移动后源跑到新位置，续拖以新位置为准）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TabDrag {
    pub group: TabGroup,
    pub source: usize,
}

/// 主界面当前几何状态的只读快照(main.rs 拖拽追踪/离屏几何计算用途,
/// `Copy` 类型直接按值传递)。取代旧 `PanelLayout` 单独传递的做法——
/// 新几何公式(webview bounds/焦点路由/IME 光标)都依赖"当前是哪个视图、
/// 是否收起"，不能只靠宽高数字算，所以把这些也打包进来。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShellState {
    pub layout: ShellLayout,
    /// 当前活跃项目面板区尺寸(左宽 + 四个 split)。每项目一份,由
    /// `App::adopt_panel_layout` 切项目时灌入,`apply_column_drag` 拖拽后写回。
    pub dims: PanelDims,
    pub left_view: LeftView,
    pub left_collapsed: bool,
    pub right_view: RightView,
    pub right_collapsed: bool,
    /// 当前放大态。`preview_content_bounds`/`is_in_preview_column`/
    /// `terminal_pane_pixel_size` 靠这个字段才能感知"这块内容其实被放大
    /// 遮罩盖住了/放大到了整个 maximize 区域"——没有它,离屏 webview 摆位、
    /// 焦点路由、终端 PTY 网格都会对着放大前的旧几何算,和 `maximize_overlay`
    /// 实际渲染的画面对不上。
    pub maximized: Option<MaximizedPane>,
}

/// 两条图标栏与那条恒在的 `LeftRight` 分隔线之外，留给左右两个面板区的
/// 总宽。`view()` 无条件渲染 `divider_bar(Divider::LeftRight)`(收起某侧也
/// 保留拖拽手柄)，所以这 8px 恒扣，不看收起态——早先版本只在两侧都可见时
/// 扣，导致收起一侧后几何比实际渲染宽 8px 且原点左偏 8px。
pub(crate) fn zones_width(window_width: f32) -> f32 {
    (window_width - 2.0 * theme::geometry::icon_rail_width() - theme::geometry::divider_width())
        .max(0.0)
}

/// 把持久化的 `left_width` 夹进"当前窗口宽度下合法"的区间:下限
/// `theme::geometry::min_zone_width()`,上限"给右面板区也留够 `theme::geometry::min_zone_width()`"。窗口窄到
/// 上界低于下界时用 `.max(theme::geometry::min_zone_width())` 把上界垫平,`clamp` 恒不 panic。
///
/// 这是**唯一**一处 left_width 的夹取:渲染侧(`left_panel_area` 经
/// `Workspace::effective_left_width`)、几何侧(`left_zone_width` → webview
/// bounds/命中测试/终端网格)、拖拽侧(`apply_column_drag`)全部走这里。
/// 各写一份夹取(或只在拖拽时夹一次)就会出现:窗口被拖窄后持久化宽仍按
/// 640 渲染,`Length::Fixed(640)` 在 flex 第一趟就把可用空间吃光,唯一
/// `Fill` 的右面板区拿到 0 宽——整个右半边(终端/Agent 列表/对话/审阅)
/// 凭空消失(Fix round 2 Critical #1)。夹取只发生在渲染/几何时刻,不回写
/// `ShellLayout`,窗口再拉宽时用户原来偏好的宽度自动复原。
pub(crate) fn clamp_left_width(window_width: f32, left_width: f32) -> f32 {
    let upper = (zones_width(window_width) - theme::geometry::min_zone_width())
        .max(theme::geometry::min_zone_width());
    left_width.clamp(theme::geometry::min_zone_width(), upper)
}

/// 左面板区当前实际宽度(逻辑像素)：收起时 0；对侧收起时独占 `zones_width`
/// (与 `left_panel_area` 此时渲染成 `Length::Fill` 对应)；否则用持久化宽
/// 按当前窗口宽夹取后的**有效**宽(见 `clamp_left_width`)。
pub(crate) fn left_zone_width(window_width: f32, state: &ShellState) -> f32 {
    if state.left_collapsed {
        0.0
    } else if state.right_collapsed {
        zones_width(window_width)
    } else {
        clamp_left_width(window_width, state.dims.left_width)
    }
}

/// 右面板区当前实际宽度(逻辑像素)；收起时为 0；否则是 `zones_width` 里
/// 左面板区没占走的剩余空间——右面板区不像左面板区那样有独立持久化宽度，
/// 恒为 Fill。
pub(crate) fn right_zone_width(window_width: f32, state: &ShellState) -> f32 {
    if state.right_collapsed {
        return 0.0;
    }
    (zones_width(window_width) - left_zone_width(window_width, state)).max(0.0)
}

/// 配对视图内部可按 split 比例分配的宽度 = 区宽减去中间那条分隔线。
/// 两侧配对都用 `FillPortion` 渲染内部分割，而 `FillPortion` 是在扣掉固定
/// 宽的分隔线之后才按比例分剩余空间的，所以比例的分母必须是这个值，不是
/// 区宽本身。
fn pair_content_width(zone_width: f32) -> f32 {
    (zone_width - theme::geometry::divider_width()).max(0.0)
}

/// 配对视图内部"列表侧"与"内容侧"的宽度,按 `split`(列表侧占比)从
/// `pair_w` 分出。四个左右双栏 zone(左:文件/项目,右:Agent/对话)统一走
/// 这一份公式:`split` 恒代表列表侧占比,内容侧拿剩下的 `1 - split`。
/// 渲染/几何/预览 bounds 三侧都要共用,不许各写各的 `pair_w * split` /
/// `pair_w * (1.0 - split)`,否则一处改动、别处漂移(见 `pair_content_width`
/// 同条原则)。
fn pair_list_content_width(pair_w: f32, split: f32) -> (f32, f32) {
    (pair_w * split, pair_w * (1.0 - split))
}

/// 拖拽某条分隔线到窗口逻辑 x 坐标 `logical_x` 后的新 `ShellLayout`。
/// `LeftPairSplit`/`RightPairSplit` 写哪个 split 字段取决于当前那一侧的
/// 视图选择(比如右侧当前是"对话"就写 `conversations_split`，不是
/// `agent_split`)——这条信息 `ShellLayout` 自己没有，靠 `ShellState` 带过来。
pub(crate) fn apply_column_drag(
    state: ShellState,
    divider: Divider,
    window_width: f32,
    logical_x: f32,
) -> PanelDims {
    match divider {
        Divider::LeftRight => {
            let new_left =
                clamp_left_width(window_width, logical_x - theme::geometry::icon_rail_width());
            PanelDims {
                left_width: new_left,
                ..state.dims
            }
        }
        Divider::LeftPairSplit => {
            let pair_w = pair_content_width(left_zone_width(window_width, &state));
            if pair_w <= 0.0 {
                return state.dims;
            }
            let ratio = ((logical_x - theme::geometry::icon_rail_width()) / pair_w).clamp(
                theme::geometry::min_split_ratio(),
                theme::geometry::max_split_ratio(),
            );
            PanelDims {
                files_split: ratio,
                ..state.dims
            }
        }
        Divider::ProjectSplit => {
            let pair_w = pair_content_width(left_zone_width(window_width, &state));
            if pair_w <= 0.0 {
                return state.dims;
            }
            let ratio = ((logical_x - theme::geometry::icon_rail_width()) / pair_w).clamp(
                theme::geometry::min_split_ratio(),
                theme::geometry::max_split_ratio(),
            );
            PanelDims {
                project_split: ratio,
                ..state.dims
            }
        }
        Divider::SshSplit => {
            let pair_w = pair_content_width(left_zone_width(window_width, &state));
            if pair_w <= 0.0 {
                return state.dims;
            }
            let ratio = ((logical_x - theme::geometry::icon_rail_width()) / pair_w).clamp(
                theme::geometry::min_split_ratio(),
                theme::geometry::max_split_ratio(),
            );
            PanelDims {
                ssh_split: ratio,
                ..state.dims
            }
        }
        Divider::TodoSplit => {
            let pair_w = pair_content_width(left_zone_width(window_width, &state));
            if pair_w <= 0.0 {
                return state.dims;
            }
            let ratio = ((logical_x - theme::geometry::icon_rail_width()) / pair_w).clamp(
                theme::geometry::min_split_ratio(),
                theme::geometry::max_split_ratio(),
            );
            PanelDims {
                todo_split: ratio,
                ..state.dims
            }
        }
        Divider::RightPairSplit => {
            let right_w = right_zone_width(window_width, &state);
            let pair_w = pair_content_width(right_w);
            if pair_w <= 0.0 {
                return state.dims;
            }
            let right_x0 = window_width - theme::geometry::icon_rail_width() - right_w;
            // `ratio` 是"配对里渲染在左边那块"的宽度占比(拖拽点左侧的宽度
            // 除以配对总宽)——这块现在是终端/审阅,不是 agent_split/
            // conversations_split 存的"列表侧(Agent 列表/对话列表)占比"。
            // 两者互补(列表侧渲染在右边),所以要写 1.0-ratio,不能直接写
            // ratio,否则拖拽方向会反(见 `right_panel_area` 顶部注释)。
            let ratio = ((logical_x - right_x0) / pair_w).clamp(
                theme::geometry::min_split_ratio(),
                theme::geometry::max_split_ratio(),
            );
            match state.right_view {
                RightView::Agent => PanelDims {
                    agent_split: 1.0 - ratio,
                    ..state.dims
                },
                RightView::Conversations => PanelDims {
                    conversations_split: 1.0 - ratio,
                    ..state.dims
                },
                // 用量统计是单栏（不分割）,没有自己的 split 权重。
                RightView::Usage => state.dims,
                // 验收面板同用量统计是单栏,不分割。
                RightView::Acceptance => state.dims,
            }
        }
    }
}

/// 放大态金色描边盒子在窗口坐标系里的横向范围 (x0, 可用宽度)。放大左侧
/// 还是右侧都是同一个盒子(`maximize_overlay` 的 dim_bg 铺满两条图标栏
/// 之间,`bordered` 再铺满其内边距之内),所以这一份公式两侧共用:三层留白
/// 累加 = 图标栏宽 + `maximize_overlay` 里 dim_bg 的内边距——`bordered`
/// 容器本身无内边距、宽度铺满,所以到这里为止。
/// `preview_content_bounds`/`is_in_preview_column`/`terminal_pane_pixel_size`
/// 都靠它换算放大态几何,不能各写各的字面量,否则和 `maximize_overlay`
/// 实际渲染的画面对不上。
fn maximized_box_x_range(window_width: f32) -> (f32, f32) {
    let x0 = theme::geometry::icon_rail_width() + theme::geometry::maximize_overlay_padding();
    let avail_w = (window_width
        - 2.0 * theme::geometry::icon_rail_width()
        - 2.0 * theme::geometry::maximize_overlay_padding())
    .max(0.0);
    (x0, avail_w)
}

/// 放大态金色描边盒子的纵向可用高度(逻辑像素)。`maximize_overlay` 顶部
/// 垫了一条 `theme::geometry::top_bar_height()` 高的 Space 把遮罩钉在顶栏之下,盒子上下各留
/// `theme::geometry::maximize_overlay_padding()`;遮罩铺到窗口底边(状态栏也被盖住),所以这里
/// **不**扣 `theme::geometry::status_bar_height()`——与 `preview_content_bounds` 放大分支同源。
fn maximized_box_height(window_height: f32) -> f32 {
    (window_height
        - theme::geometry::top_bar_height()
        - 2.0 * theme::geometry::maximize_overlay_padding())
    .max(0.0)
}

/// 窗口逻辑尺寸 → 左侧文件预览内容区矩形(逻辑像素 x/y/w/h)，供
/// main.rs 摆放 wry webview 用。左侧收起时返回零尺寸矩形。
///
/// 放大态(Task 5):右侧被放大时左侧内容被 `maximize_overlay` 的变暗遮罩
/// 整片盖住——但 wry webview 是原生子视图,不听 iced 的绘制顺序摆布,会
/// 无视遮罩径直叠在最上面,必须用零尺寸矩形把它真正藏起来(与
/// `left_collapsed` 分支同一手法)。左侧被放大时,矩形要按
/// `maximize_overlay` 实际渲染的更大盒子重新换算,不能再用平时的
/// `left_zone_width`。
pub fn preview_content_bounds(
    window_width: f32,
    window_height: f32,
    state: &ShellState,
) -> (f32, f32, f32, f32) {
    if state.left_collapsed {
        return (0.0, 0.0, 0.0, 0.0);
    }
    if state.maximized == Some(MaximizedPane::Right) {
        return (0.0, 0.0, 0.0, 0.0);
    }
    // 两分支 chrome 高度不同(浏览器仍有地址栏,文件预览已去掉),必须各用
    // 各的常量——共用一个会在文件预览顶上留出一截再也画不出东西的空白
    // (webview 摆位比实际渲染的 tab 栏低了一整个地址栏的高度)。
    if state.maximized == Some(MaximizedPane::Left) {
        let (x0, avail_w) = maximized_box_x_range(window_width);
        let y0 = theme::geometry::top_bar_height() + theme::geometry::maximize_overlay_padding();
        // 同样扣掉 footbar 高度,让放大态 webview 底部也不戳到 footbar。
        let avail_h =
            (maximized_box_height(window_height) - theme::geometry::status_bar_height()).max(0.0);
        return match state.left_view {
            LeftView::Files => {
                let y = y0 + theme::geometry::preview_chrome_top_px();
                let h = (avail_h - theme::geometry::preview_chrome_top_px() - 8.0).max(0.0);
                let pair_w = pair_content_width(avail_w);
                let (list_w, content_w) = pair_list_content_width(pair_w, state.dims.files_split);
                let x = x0 + list_w + theme::geometry::divider_width() + 8.0;
                let w = (content_w - 16.0).max(0.0);
                (x, y, w, h)
            }
            // 浏览器(Web)是单栏(无配对),放大态占满整条放大盒子。
            LeftView::Web => {
                let y = y0 + theme::geometry::browser_chrome_top_px();
                let h = (avail_h - theme::geometry::browser_chrome_top_px() - 8.0).max(0.0);
                let x = x0 + 8.0;
                let w = (avail_w - 16.0).max(0.0);
                (x, y, w, h)
            }
            // Git 提交图是原生 Canvas 绘制,不挂 webview 子视图。
            LeftView::GitLog => (0.0, 0.0, 0.0, 0.0),
            // Todo 面板同 GitLog,纯 iced 绘制,不挂 webview 子视图。
            LeftView::Todo => (0.0, 0.0, 0.0, 0.0),
            // Project 面板的右配对(项目预览)是 Files 同款预览 chrome,按
            // `project_split` 算出右配对那条 webview 的矩形。
            LeftView::Project => {
                let y = y0 + theme::geometry::preview_chrome_top_px();
                let h = (avail_h - theme::geometry::preview_chrome_top_px() - 8.0).max(0.0);
                let pair_w = pair_content_width(avail_w);
                let (list_w, content_w) = pair_list_content_width(pair_w, state.dims.project_split);
                let x = x0 + list_w + theme::geometry::divider_width() + 8.0;
                let w = (content_w - 16.0).max(0.0);
                (x, y, w, h)
            }
            // Database 面板同 Project,纯 iced 绘制,不挂 webview 子视图。
            LeftView::Database => (0.0, 0.0, 0.0, 0.0),
            // SSH 面板同 Project,纯 iced 绘制,阶段 1 不挂 webview 子视图。
            LeftView::Ssh => (0.0, 0.0, 0.0, 0.0),
        };
    }
    let left_w = left_zone_width(window_width, state);
    // `left_zone` 的上下 margin:webview 必须跟着 inset,否则会戳出外边框
    // (原生子视图不听 iced 布局,逐像素靠这里算)。左右 margin 同样要算进去,
    // 否则去掉外边框后 webview 会戳出新增的左侧留白。
    let m = theme::region::left_zone().margin;
    let y_top = |chrome_top: f32| -> f32 { theme::geometry::top_bar_height() + m.top + chrome_top };
    // 底部扣 footbar(`extensions::footbar::view` 的固定高度
    // = `theme::geometry::status_bar_height()`)——wry webview 不听 iced
    // 布局,若不扣会把 footbar 文字盖在底下。不留额外 8px 间隙,让
    // webview 底部紧贴 footbar 顶部(只留 left_zone 的下 margin)。
    let h_for = |y: f32| -> f32 {
        (window_height - y - m.bottom - theme::geometry::status_bar_height()).max(0.0)
    };
    match state.left_view {
        LeftView::Files => {
            let y = y_top(theme::geometry::preview_chrome_top_px());
            let h = h_for(y);
            let pair_w = pair_content_width(left_w);
            let (list_w, content_w) = pair_list_content_width(pair_w, state.dims.files_split);
            let x = theme::geometry::icon_rail_width()
                + list_w
                + theme::geometry::divider_width()
                + 8.0
                + m.left;
            let w = (content_w - 16.0 - m.left - m.right).max(0.0);
            (x, y, w, h)
        }
        // 浏览器(Web)是单栏,左图标栏右侧 + 左 margin + 8 起,占满左面板区。
        LeftView::Web => {
            let y = y_top(theme::geometry::browser_chrome_top_px());
            let h = h_for(y);
            let x = theme::geometry::icon_rail_width() + 8.0 + m.left;
            let w = (left_w - 16.0 - m.left - m.right).max(0.0);
            (x, y, w, h)
        }
        LeftView::GitLog => (0.0, 0.0, 0.0, 0.0),
        // Todo 面板同 GitLog,纯 iced 绘制,不挂 webview 子视图。
        LeftView::Todo => (0.0, 0.0, 0.0, 0.0),
        // Project 面板右配对(项目预览)是 Files 同款预览 chrome,按
        // `project_split` 算出右配对那条 webview 矩形。
        LeftView::Project => {
            let y = y_top(theme::geometry::preview_chrome_top_px());
            let h = h_for(y);
            let pair_w = pair_content_width(left_w);
            let (list_w, content_w) = pair_list_content_width(pair_w, state.dims.project_split);
            let x = theme::geometry::icon_rail_width()
                + list_w
                + theme::geometry::divider_width()
                + 8.0
                + m.left;
            let w = (content_w - 16.0 - m.left - m.right).max(0.0);
            (x, y, w, h)
        }
        // Database 面板同 Project,纯 iced 绘制,不挂 webview 子视图。
        LeftView::Database => (0.0, 0.0, 0.0, 0.0),
        // SSH 面板同 Project,纯 iced 绘制,阶段 1 不挂 webview 子视图。
        LeftView::Ssh => (0.0, 0.0, 0.0, 0.0),
    }
}

/// 左侧文件树的**目录列表 Scrollable** 在窗口坐标系里的矩形（上/左/宽/高，
/// 逻辑像素），供 main.rs 做外部文件拖拽命中测试。返回的矩形只覆盖列表
/// 视口本身——命中测试据此把窗口 Y 换算成 `tree_scroll` 偏移下的"可见行
/// 序号"，再推出那行是不是目录。
///
/// 与 `preview_content_bounds` 同源（外层）但其目标是**配对里左侧那一栏**
/// （树），不是右侧的 webview 列，所以横向起点去掉了分隔线+8px、纵向起点
/// 换用 `tree_chrome_top_px`（面板头+搜索/工具栏），底部扣 `git 脚注栏`
/// 而非 footbar 专用常量。表单汇总：
///
/// - 外层：左栏位于 `icon_rail_width + m.left`，纵向从 `top_bar_height +
///   m.top` 起、到 `window_height - m.bottom - status_bar_height` 止；
///   栏宽 = `pair_content_width(left_w) * files_split`。
/// - 内层：`project_pane().padding` 给容器留内边距；`tree_chrome_top_px()`
///   盖掉上方（面板头+搜索行+两段间距），`tree_chrome_bottom_px()` 盖掉
///   下方（git 脚注栏+间距+下内边距）。
///
/// 不可命中（左侧收起 / 右侧放大 / 不在文件树视图）时返回零尺寸矩形。
/// 放大态左侧(`MaximizedPane::Left`)按 `maximize_overlay` 的实际盒子换算。
pub fn left_files_tree_bounds(
    window_width: f32,
    window_height: f32,
    state: &ShellState,
) -> (f32, f32, f32, f32) {
    let zero = || (0.0, 0.0, 0.0, 0.0);
    if state.left_collapsed || state.left_view != LeftView::Files {
        return zero();
    }
    let m = theme::region::left_zone().margin;
    let p = theme::region::project_pane();
    if state.maximized == Some(MaximizedPane::Right) {
        return zero();
    }
    if state.maximized == Some(MaximizedPane::Left) {
        let (x0, avail_w) = maximized_box_x_range(window_width);
        let y_top = theme::geometry::top_bar_height() + theme::geometry::maximize_overlay_padding();
        let avail_h =
            (maximized_box_height(window_height) - theme::geometry::status_bar_height()).max(0.0);
        let pair_w = pair_content_width(avail_w);
        let (list_w, _content_w) = pair_list_content_width(pair_w, state.dims.files_split);
        let x = x0 + m.left + p.padding.left;
        let w = (list_w - p.padding.left - p.padding.right).max(0.0);
        let y = y_top + m.top + p.padding.top + theme::geometry::tree_chrome_top_px();
        let h = (avail_h
            - m.top
            - m.bottom
            - p.padding.top
            - p.padding.bottom
            - theme::geometry::tree_chrome_top_px()
            - theme::geometry::tree_chrome_bottom_px())
        .max(0.0);
        return (x, y, w, h);
    }
    let left_w = left_zone_width(window_width, state);
    let (list_w, _) = pair_list_content_width(pair_content_width(left_w), state.dims.files_split);
    let x = theme::geometry::icon_rail_width() + m.left + p.padding.left;
    let w = (list_w - p.padding.left - p.padding.right).max(0.0);
    let y_pane = theme::geometry::top_bar_height() + m.top;
    let y = y_pane + p.padding.top + theme::geometry::tree_chrome_top_px();
    let h = ((window_height - m.bottom - theme::geometry::status_bar_height())
        - (y_pane + p.padding.top + theme::geometry::tree_chrome_top_px())
        - p.padding.bottom
        - theme::geometry::tree_chrome_bottom_px())
    .max(0.0);
    (x, y, w, h)
}

/// 逻辑 x 是否落在左侧文件预览内容区列内。焦点路由用:点击落在
/// 该列 → 键盘交给 webview;落在别处 → 交回窗口(终端)。
///
/// 放大态(Task 5):右侧被放大时左侧内容不可见,恒不落在预览列;左侧被
/// 放大时按 `maximize_overlay` 实际渲染的更大盒子重新换算横向范围。
pub fn is_in_preview_column(x: f32, window_width: f32, state: &ShellState) -> bool {
    if state.left_collapsed {
        return false;
    }
    if state.maximized == Some(MaximizedPane::Right) {
        return false;
    }
    if state.maximized == Some(MaximizedPane::Left) {
        let (x0, avail_w) = maximized_box_x_range(window_width);
        return match state.left_view {
            LeftView::Files => {
                let (list_w, _) =
                    pair_list_content_width(pair_content_width(avail_w), state.dims.files_split);
                let start = x0 + list_w + theme::geometry::divider_width();
                let end = x0 + avail_w;
                x >= start && x < end
            }
            // 浏览器(Web)是单栏,放大态占满整条放大盒子横向范围。
            LeftView::Web => {
                let start = x0;
                let end = start + avail_w;
                x >= start && x < end
            }
            LeftView::GitLog => false,
            // Todo 面板纯 iced 绘制,无 webview,永不落在预览列。
            LeftView::Todo => false,
            // Project 面板右配对(项目预览)是 Files 同款预览列,按 `project_split` 算。
            LeftView::Project => {
                let (list_w, _) =
                    pair_list_content_width(pair_content_width(avail_w), state.dims.project_split);
                let start = x0 + list_w + theme::geometry::divider_width();
                let end = x0 + avail_w;
                x >= start && x < end
            }
            // Database 面板同 Project,纯 iced 绘制,永无 webview。
            LeftView::Database => false,
            // SSH 面板同 Project,纯 iced 绘制,永无 webview。
            LeftView::Ssh => false,
        };
    }
    let left_w = left_zone_width(window_width, state);
    match state.left_view {
        LeftView::Files => {
            let (list_w, _) =
                pair_list_content_width(pair_content_width(left_w), state.dims.files_split);
            let start =
                theme::geometry::icon_rail_width() + list_w + theme::geometry::divider_width();
            let end = theme::geometry::icon_rail_width() + left_w;
            x >= start && x < end
        }
        // 浏览器(Web)是单栏,占满整条左面板区横向范围。
        LeftView::Web => {
            let start = theme::geometry::icon_rail_width();
            let end = start + left_w;
            x >= start && x < end
        }
        LeftView::GitLog => false,
        // Todo 面板纯 iced 绘制,无 webview,永不落在预览列。
        LeftView::Todo => false,
        // Project 面板右配对(项目预览)是 Files 同款预览列,按 `project_split` 算。
        LeftView::Project => {
            let (list_w, _) =
                pair_list_content_width(pair_content_width(left_w), state.dims.project_split);
            let start =
                theme::geometry::icon_rail_width() + list_w + theme::geometry::divider_width();
            let end = theme::geometry::icon_rail_width() + left_w;
            x >= start && x < end
        }
        // Database 面板同 Project,纯 iced 绘制,永无 webview。
        LeftView::Database => false,
        // SSH 面板同 Project,纯 iced 绘制,永无 webview。
        LeftView::Ssh => false,
    }
}

/// 逻辑 x 落在哪一侧面板区(整区,不分区内具体是哪个 pane)。左键点击
/// 落点决定当前"聚焦"哪一侧,驱动 `left_zone`/`right_zone` 外边框的高亮态
/// (见 [`ZoneSide`])。落在图标栏本身(两侧各 `theme::geometry::icon_rail_width()` 宽)或
/// 某侧收起而点在了"不存在的那一侧"时不算数,返回 `None`(调用方应保持
/// 点击前的聚焦态不变,而不是清空)。
///
/// 放大态:整个内容区就是放大的那一侧,不用再按横坐标细分——
/// `maximize_overlay` 渲染时两条图标栏原样露在外面,和非放大态同一
/// 横向范围,所以图标栏判定不用跟着改。
pub fn zone_at_x(x: f32, window_width: f32, state: &ShellState) -> Option<ZoneSide> {
    if x < theme::geometry::icon_rail_width()
        || x > window_width - theme::geometry::icon_rail_width()
    {
        return None;
    }
    if let Some(which) = state.maximized {
        return Some(match which {
            MaximizedPane::Left => ZoneSide::Left,
            MaximizedPane::Right => ZoneSide::Right,
        });
    }
    if state.left_collapsed {
        return if state.right_collapsed {
            None
        } else {
            Some(ZoneSide::Right)
        };
    }
    if state.right_collapsed {
        return Some(ZoneSide::Left);
    }
    let boundary = theme::geometry::icon_rail_width() + left_zone_width(window_width, state);
    Some(if x < boundary {
        ZoneSide::Left
    } else {
        ZoneSide::Right
    })
}

/// 终端 pane 此刻是否真的呈现在用户眼前。键盘输入(`Message::TermInput`)
/// 必须以此为闸门:右侧收起、右视图切到对话、或左侧被放大(右半被变暗遮罩
/// 整片盖住)时,敲下的回车/方向键会静默提交给一个看不见的 agent 会话——这条
/// 在旧四栏布局里不存在(终端恒在屏上),是新外壳带出来的新风险
/// (Fix round 2 #3)。放大的正是右侧时终端**是**可见的(只是更大),算可见。
pub(crate) fn terminal_visible(state: &ShellState) -> bool {
    state.right_view == RightView::Agent
        && !state.right_collapsed
        && state.maximized != Some(MaximizedPane::Left)
}

/// SSH 面板内嵌终端此刻是否真的呈现在用户眼前——镜像 `terminal_visible`,
/// 判定对象换成左侧:SSH 面板必须是当前左视图,且没有被"右侧放大"盖住
/// (角色与 `terminal_visible` 的 `MaximizedPane::Left` 判断对调:终端在
/// 右、被左侧放大遮住;SSH 面板在左、被右侧放大遮住)。
pub(crate) fn ssh_terminal_visible(state: &ShellState) -> bool {
    state.left_view == LeftView::Ssh && state.maximized != Some(MaximizedPane::Right)
}

/// 键盘/粘贴事件此刻该写给右侧共享终端条还是 SSH 面板自己的内嵌终端。
/// 复用既有 `active_zone`(点击左右面板区任意位置就会更新,已经在驱动
/// `left_zone`/`right_zone` 的高亮边框,见 `App::set_active_zone`)——
/// SSH 面板在左侧且左侧是当前聚焦区时走 SSH 面板,否则走现状的共享
/// 终端条(不需要新增专门的终端焦点状态)。
pub(crate) fn keyboard_term_target(
    left_view: LeftView,
    active_zone: Option<ZoneSide>,
) -> TermTarget {
    if left_view == LeftView::Ssh && active_zone == Some(ZoneSide::Left) {
        TermTarget::SshPanel
    } else {
        TermTarget::Shared
    }
}

/// 换算终端 PTY 网格时用的假想外壳状态:强制"右侧展开 + 显示 Agent 配对"。
///
/// 终端此刻可能不可见(右侧收起 / 右视图是对话),但它的 PTY 网格仍应按
/// "被显示时占多大"来定——否则上次退出前停在对话视图的会话,重开 app 后会
/// 一直卡在 `DEFAULT_COLS`×`DEFAULT_ROWS`(80×24),直到用户偶然拖一下窗口
/// 才纠正(Fix round 2 #6)。用字段覆盖表达这个假想,复用同一套宽度公式,
/// 不另写一份几何。
pub(crate) fn terminal_grid_state(state: ShellState) -> ShellState {
    // `maximized == Some(Right)` 只有在真实 `right_view` 本来就是 `Agent`
    // 时才代表"终端被放大"——对话视图下点"放大"是放大审阅 pane
    // (`review_content_pane` 自己的放大按钮),不是终端。不做这个过滤会让
    // "对话视图下放大审阅"被这里误判成"终端被放大",按放大格算出一个终端
    // 实际不可见、也不是那个尺寸的网格,给所有存活 PTY 发一次错的 SIGWINCH
    // (Fix round 3,scoped re-review 发现)。
    let maximized = state
        .maximized
        .filter(|m| *m != MaximizedPane::Right || state.right_view == RightView::Agent);
    ShellState {
        right_collapsed: false,
        right_view: RightView::Agent,
        maximized,
        ..state
    }
}

/// 窗口整体逻辑像素尺寸 → 终端 pane 的可用像素尺寸。终端只在右侧视图是
/// `Agent` 且未收起时可见；否则返回零尺寸(调用方在这种情况下本就不会真的
/// 用这个尺寸去 resize 一个不可见的终端，返回零是安全兜底；要按"若显示则
/// 多大"换算 PTY 网格的场合见 `Workspace::sync_terminal_grid`)。
///
/// 放大态(Fix round 2 #6):右侧被放大时终端所在的整个右面板区被
/// `maximize_overlay` 渲染成金色描边盒子那么大,网格必须跟着变大,否则
/// "放大终端"只放大了外框、字符网格还是放大前那么小(放大等于白放)。
/// 左侧被放大时右面板区在遮罩之下、几何不变,沿用常规分支。
pub fn terminal_pane_pixel_size(
    window_width: f32,
    window_height: f32,
    state: &ShellState,
) -> (f32, f32) {
    if state.right_collapsed || state.right_view != RightView::Agent {
        return (0.0, 0.0);
    }
    if state.maximized == Some(MaximizedPane::Right) {
        let (_x0, avail_w) = maximized_box_x_range(window_width);
        let (_list_w, content_w) =
            pair_list_content_width(pair_content_width(avail_w), state.dims.agent_split);
        let pane_width = (content_w - theme::geometry::chrome_width_px()).max(0.0);
        // `theme::geometry::status_bar_height()` 是终端 pane 自带的底栏(`terminal_status_bar`,
        // 不是窗口级状态栏),放大态一样在盒子里,照扣。
        let pane_height = (maximized_box_height(window_height)
            - theme::geometry::status_bar_height()
            - theme::geometry::chrome_height_px())
        .max(0.0);
        return (pane_width, pane_height);
    }
    let right_w = right_zone_width(window_width, state);
    let (_list_w, content_w) =
        pair_list_content_width(pair_content_width(right_w), state.dims.agent_split);
    let pane_width = (content_w - theme::geometry::chrome_width_px()).max(0.0);
    // `right_zone` 上下 margin:终端是 iced 布局(自动 inset),但其 PTY 网格
    // 尺寸靠这里算,必须同步扣掉上下 margin,否则字符网格比实际渲染区高。
    let m = theme::region::right_zone().margin;
    let pane_height = (window_height
        - theme::geometry::top_bar_height()
        - theme::geometry::status_bar_height()
        - theme::geometry::chrome_height_px()
        - m.top
        - m.bottom)
        .max(0.0);
    (pane_width, pane_height)
}

/// 一批**异步结果**消息共同的首个字段:它们归属哪个项目。
///
/// 为什么必须显式带上、不能"投给当时聚焦的那个项目"(P2a Task 7 fix round 1
/// 的 Critical):`SessionTab::tab_id` 来自每个 `Workspace` 自己的
/// `next_tab_id`,**每个项目都从 0 起编**。多项目并行之后"项目 A 和项目 B
/// 各有活着的会话"是常态而不是边角情况,两边的第一个 tab 都是 id 0。若按
/// `App::with_focused_project`(投给当前聚焦项目)路由,后台项目 A 的 agent 每吐一次输出,
/// `TermOutput(0, ..)` 就会被喂进前台项目 B 的 tab 0 里——这不是竞态窗口,
/// 是只要两个项目都有会话就持续发生。`SessionExited` 会把错误的 tab 标成
/// 已死,`TabAttached` 会让 B 认领本属于 A 的会话。
///
/// 所以凡是"发起时就已知归属项目、结果晚些才回来"的消息,一律带上
/// `project_id`,由 [`App::with_project`] 直接投递到对应槽位;投递不到
/// (项目已被关掉/还没促成)就静默丢弃。反过来,由用户点击当前界面直接
/// 触发的消息(`TermInput`/`Acceptance(..::Open(..))`/`PreviewSelectTab` …)仍然走
/// `with_focused_project`——它们的语义本来就是"作用于用户此刻看着的那个项目"。
pub type ProjectId = i64;

/// Ctrl + / Ctrl - 每次触发的相对缩放步近因子（1.1 ≈ 每按一次放大 10%）。
const UI_ZOOM_STEP: f32 = 1.1;

/// 终端相关消息(键盘/滚轮/选区/粘贴)该写去右侧共享终端条还是 SSH 面板
/// 自己的内嵌终端——两者可能同时在屏幕上,裸消息本身不带这个信息,靠
/// canvas 渲染时(`term_view::view`)烘焙进它构造的每条消息里。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TermTarget {
    Shared,
    SshPanel,
}

#[derive(Debug, Clone)]
pub enum Message {
    /// 终端聚焦时的键盘/IME 输入字节（已经过 `keymap` 翻译）。直接写给
    /// 当前激活 tab 对应的 daemon 会话（`client.write`）——不再本地
    /// echo，回显完全走 PTY 真实回路（daemon → attach 流 → `TermOutput`）。
    TermInput(TermTarget, Vec<u8>),
    /// attach 事件流转发来的输出字节，`usize` 是 tab 的稳定 id
    /// （`SessionTab::tab_id`，不是 vec 位置——关闭 tab 会移动位置，
    /// 但 id 不变，事件流路由必须认 id）。首字段的项目归属见 [`ProjectId`]
    /// ——`tab_id` 只在单个项目内唯一,跨项目会撞。
    TermOutput(ProjectId, usize, Vec<u8>),
    /// 对应 tab 的会话已退出（PTY 子进程退出或 daemon 断连）。
    SessionExited(ProjectId, usize),
    /// attach 流转发来的 agent 状态变更（tab_id, 状态, 该会话最新 transcript 路径）。
    AgentStateChanged(ProjectId, usize, AgentKind, AgentState, Option<String>),
    /// TurnEnded 触发的交付检测结果（tab_id, 是否有待验收交付）。
    DeliveryChecked(ProjectId, usize, bool),
    /// 验收面板的全部消息,内核只转发不解读——见
    /// `extensions::acceptance::Message`。
    Acceptance(acceptance::Message),
    /// 会话审阅:解析完成（来源, 条目 / 错误文案）。
    ReviewLoaded(ProjectId, ReviewSource, Result<Vec<ReviewEntry>, String>),
    /// 会话审阅:展开/收起第 n 个 AI 回合的过程区。
    ReviewToggle(usize),
    /// 对话列表刷新结果（扫描完成）。
    ConversationsRefreshed(ProjectId, Vec<ConversationMeta>),
    /// Usage 面板的全部消息,内核只转发不解读——见 `extensions::usage::Message`。
    Usage(usage::Message),
    /// 点对话列表某条 → 审阅该对话（当前会话用 Session 源以便回合刷新,历史用 File）。
    ConversationOpen(PathBuf),
    /// 切换当前显示的 tab（这里的 `usize` 是 vec 位置——用户点击的是
    /// "屏幕上第几个 tab"，跟稳定 id 是两回事）。
    SelectTab(usize),
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
    /// 新建会话完成 attach（tab_id、`SessionInfo`、初始快照）。
    /// 启动时的恢复走同步的 `bootstrap`，不需要过一次消息循环。
    TabAttached(ProjectId, usize, SessionInfo, Vec<u8>),
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
    PaneResized { cols: u16, rows: u16 },
    /// 按下某条分隔线,记录"正在拖哪条"(main.rs 后续 CursorMoved 靠这个
    /// 状态决定要不要继续转发拖拽)。构造方为 `divider_bar` 的 `on_press`。
    ColumnDragStart(Divider),
    /// 拖拽中:当前窗口逻辑宽 + 光标逻辑 x(main.rs 换算好传入,`update()`
    /// 统一算+夹取,不与 main.rs 分摊裁剪逻辑)。构造方为 main.rs 的
    /// `CursorMoved` 续传。
    ColumnDrag { window_width: f32, logical_x: f32 },
    /// 松开左键,结束拖拽并触发写盘。构造方为 main.rs 的
    /// `MouseInput{Released}` 分支。
    ColumnDragEnd,
    /// 拖拽中,光标进入了 `group` 组的第 `index` 个 tab 上空——拖起的源项
    /// 应移动到这个目标位(换位)。构造方为该组每个 tab 顶层的
    /// `MouseArea::on_move`(仅在 `tab_drag` 命中本组时挂载)。按住页签＝
    /// 准备拖的来源,由各选中处理(`SelectTab`/`PreviewSelectTab`/
    /// `ProjectTabSwitch` 及浏览器 `SelectTab`)在按住瞬间把 `tab_drag` 置位。
    TabDragMove { group: TabGroup, index: usize },
    /// 松开左键,结束页签拖拽。构造方为 main.rs 的 `MouseInput{Released}`
    /// 分支;项目页签组顺带把新顺序写盘。
    TabDragEnd,
    /// 点击左图标栏某图标:已是当前视图则切换收起态,否则切到该视图并展开。
    LeftIconSelect(LeftView),
    /// 同上,右图标栏。
    RightIconSelect(RightView),
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
    TermScroll(TermTarget, i32),
    /// 终端 tab 栏箭头翻页（`true`=右/`false`=左）。一次翻 2 个 tab；
    /// 上界不在此钳，渲染时 `tab_window` 钳制显示（P1L T5 验收返工）。
    TermTabScroll(bool),
    /// 预览 tab 栏箭头翻页，语义同 `TermTabScroll`。
    PreviewTabScroll(bool),
    /// 终端左键按下：在视口格 `(col, row)` 起新选区（`right` = 按点在
    /// 格子右半）。
    TermSelStart {
        target: TermTarget,
        col: usize,
        row: usize,
        right: bool,
    },
    /// 终端拖拽：选区末端更新到视口格 `(col, row)`。
    TermSelUpdate {
        target: TermTarget,
        col: usize,
        row: usize,
        right: bool,
    },
    /// ⌘V 粘贴剪贴板文本：按会话的 bracketed paste 模式决定是否包裹
    /// `ESC[200~`/`ESC[201~` 后写入 daemon。
    TermPaste(TermTarget, String),
    /// 预览:打开本地文件为新 tab(路径已由入口侧确认存在,来自项目树点击/
    /// 会话恢复;预览面板本身已不再有"打开文件…"按钮或地址栏)。
    PreviewOpenPath(PathBuf),
    /// 预览:切换 tab(vec 位置).
    PreviewSelectTab(usize),
    /// 预览:关闭 tab(vec 位置).
    PreviewCloseTab(usize),
    /// 预览:点 tab chip 上的"编辑"按钮,携带 tab 下标(渲染时发出,和
    /// `PreviewSelectTab`/`PreviewCloseTab` 同一约定)。
    PreviewEditOpen(usize),
    /// 预览:原生编辑器右键菜单里的"编辑"项(`Message::OpenInEditor`),
    /// 携带 `PreviewTab.id`。由 `main.rs` 从 `PreviewEditorEvent` 前置拦截,
    /// 转成这条,打开对应文件的编辑浮层。
    PreviewEditOpenByTab(usize),
    /// 预览编辑弹层:`iced-code-editor` 的内部消息。由 `main.rs` 的 Task
    /// 桥接器消费(编辑产生的 `iced::Task` 在此执行剪贴板/聚焦等副作用),
    /// 不经 `App::update`。
    EditorEvent(iced_code_editor::Message),
    /// 原生预览 tab 的 `iced-code-editor` 内部消息,`usize` 是 `PreviewTab.id`。
    /// 与 `EditorEvent`(编辑弹层专用)是两条独立路径,互不路由串台——见
    /// `preview_tab_editor_event` 的文档。
    PreviewEditorEvent(usize, iced_code_editor::Message),
    /// 预览编辑弹层:"保存"按钮 / ⌘S。
    PreviewEditSave,
    /// 预览编辑弹层:×按钮 / 点遮罩——脏改动会先转成二次确认,不直接关。
    PreviewEditCloseRequest,
    /// 预览编辑弹层二次确认:"放弃改动"。
    PreviewEditConfirmDiscard,
    /// 预览编辑弹层二次确认:"取消"(回到编辑态)。
    PreviewEditConfirmCancel,
    /// 预览 tab 右键菜单:在 `preview_pane` 某 tab 上右键打开,携带 tab 下标
    /// 与该文件是否可编辑(仅文本类文件,决定菜单"编辑"项是否出现)。定位
    /// 坐标复用 `files.last_right_click`(main.rs 右键时写入)。
    PreviewTabContextMenu { idx: usize, editable: bool },
    /// 预览 tab 右键菜单关闭(点遮罩 / 按 Esc)。
    PreviewTabContextMenuClose,
    /// Project 面板右配对预览:打开本地文件为新 tab,语义同 `PreviewOpenPath`。
    ProjectPreviewOpenPath(PathBuf),
    /// Project 面板右配对预览:切换 tab(vec 位置)。
    ProjectPreviewSelectTab(usize),
    /// Project 面板右配对预览:关闭 tab(vec 位置)。
    ProjectPreviewCloseTab(usize),
    /// Project 面板右配对预览:tab 栏箭头翻页(语义同 `PreviewTabScroll`)。
    ProjectPreviewTabScroll(bool),
    /// Project 面板右配对预览的原生 `iced-code-editor` 内部消息，语义同
    /// `PreviewEditorEvent`。
    ProjectPreviewEditorEvent(usize, iced_code_editor::Message),
    /// Project 面板右配对预览:右键菜单里的"编辑"项,语义同 `PreviewEditOpen`。
    ProjectPreviewEditOpen(usize),
    /// Project 面板右配对预览:原生编辑器里的"编辑"项
    /// (`Message::OpenInEditor`),按 `PreviewTab.id` 路由,语义同
    /// `PreviewEditOpenByTab`。
    ProjectPreviewEditOpenByTab(usize),
    /// Project 面板右配对预览 tab 右键菜单,语义同 `PreviewTabContextMenu`。
    ProjectPreviewTabContextMenu { idx: usize, editable: bool },
    /// Project 面板右配对预览 tab 右键菜单关闭。
    ProjectPreviewTabContextMenuClose,
    /// 浏览器面板的全部消息,内核只转发不解读——见
    /// `extensions::browser::Message`。
    Browser(browser::Message),
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
    /// 项目页签:把某路径作为**新页签**打开(不动任何已存在页签的内容)。
    ProjectTabOpen(PathBuf),
    /// 项目页签:`ProjectTabOpen` 异步完成(daemon upsert 结果 + 最近列表)。
    /// `None` = 这次打开失败,只报错、不改任何页签状态。
    ProjectTabOpened(Option<ProjectInfo>, Vec<ProjectInfo>),
    /// H0 项目中心:`Message::TopBarHome` 发起的异步刷新完成(最近改动的文件、
    /// 最近的对话两份列表;D4)。
    HomeRecentsLoaded(Vec<HomeRecentFile>, Vec<HomeRecentConversation>),
    /// 首页左图标栏:切换 `HomeLeftView`(项目列表/Recents)。首页没有
    /// collapse 概念,恒有一个 pane 显示,不像工作区 `LeftIconSelect` 那样
    /// 需要处理"点已选中图标收起面板区"的分支。
    HomeLeftIconSelect(homespace::HomeLeftView),
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
    /// branch/dirty/worktrees)和 Git Log(条件触发快照重建)三个独立扩展,
    /// 内核继续拦截、分别转发,不包进任何一个 extension 的 `Message`。
    ProjectFsChanged(ProjectId, git_watch::Relevance),
    /// Git Log 面板的全部消息,内核只转发不解读——见
    /// `extensions::git_log::Message`。
    GitLog(git_log::Message),
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
}

/// 顶层容器:main.rs 持有的就是这个(取代此前直接持有单个 `Workspace`)。
/// 外壳字段是整个程序只有一份的窗口态,`projects` 承载并行打开的项目
/// 页签——每个 `Workspace` 是完全独立、同时存活的一套项目态(P2a)。
/// 文件预览 tab 的右键菜单浮层状态:定位坐标(屏幕空间,复用 `files`
/// 右键落点)+ 目标 tab 下标 + 该文件是否可编辑(仅文本类文件可编辑,
/// 决定菜单里"编辑"项是否出现)。见 `preview_pane` / 顶层 `view`。
struct PreviewTabMenu {
    x: f32,
    y: f32,
    idx: usize,
    editable: bool,
}

/// Project 面板「项目文档 / Agent 记忆」虚拟链接行的右键菜单浮层状态:
/// 定位坐标(屏幕空间,复用 `files` 右键落点)+ 目标区 + 下标。删除动作回
/// `project::Message::LinkRemove`。见 `project::links_section`。
struct ProjectLinkMenu {
    x: f32,
    y: f32,
    target: project::links::LinkTarget,
    index: usize,
}

pub struct App {
    client: Client,
    handle: Handle,
    proxy: EventLoopProxy<Message>,
    /// 当前终端网格尺寸,随 `PaneResized` 更新;新建 tab 时也用这份
    /// 尺寸,保证新会话从一开始就跟 pane 实际大小匹配。
    cols: u16,
    rows: u16,
    /// daemon 连接失败,或某次会话操作失败时的错误文案。整个程序共享
    /// 一份:daemon 连不连得上不是某个项目自己的状态。
    pub(crate) daemon_error: Option<String>,
    /// tab 前状态点的闪烁相位(true=亮/false=暗)。由 main.rs 的定时唤醒
    /// 每拍翻转(见 `toggle_blink`/`any_blinking`)。
    blink_on: bool,
    /// 上次真正翻转 `blink_on` 的时刻,`toggle_blink` 据此把自己限速到
    /// `BLINK_INTERVAL` 一拍——main.rs 的定时唤醒并不专属闪烁:悬停动画
    /// 期间(`HOVER_ANIM_INTERVAL`=16ms)会把唤醒频率提到闪烁本该的 450ms
    /// 的近 30 倍,若 `toggle_blink` 对"被叫到"照单全收,状态点就会跟着
    /// hover 的那份高频唤醒一起快速明灭,观感是"悬停 icon 按钮,别处的点
    /// 跟着闪"。
    last_blink_at: std::time::Instant,
    /// 上次真正执行 Todo 面板磁盘轮询(`poll_todo_if_visible`)的时刻,
    /// 用法与 `last_blink_at` 一致:按 `TODO_POLL_INTERVAL` 自限速,未到
    /// 点的调用直接 no-op。现在靠 mtime 检查已经安全(没变化就早退,见
    /// 该方法文档),这里补上限速是为了让"周期性函数自己对被更快唤醒
    /// 节奏带跑免疫"这条约定对全部三个周期性关注点(闪烁/悬停动画/
    /// Todo 轮询)都显式成立,不留一个"靠巧合安全"的例外。
    last_todo_poll_at: std::time::Instant,
    /// 全局窗口尺寸;启动时 `layout::load()` 读盘作起始值,退出前写盘。
    /// 只存窗口尺寸——左右面板区的宽度/分割比例(**每个项目各自**的偏好)
    /// 已迁进每项目 `dims`(见 `panel_layouts`),不放在这里。
    shell_layout: ShellLayout,
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
    left_view: LeftView,
    /// 右面板区当前显示的配对视图(右图标栏点击切换)。
    right_view: RightView,
    /// 左面板区是否折叠(再点一次当前已激活的图标即收起)。
    left_collapsed: bool,
    /// 右面板区是否折叠,语义同 `left_collapsed`。
    right_collapsed: bool,
    /// 当前放大的内容子面板(`None`=未放大)。
    maximized: Option<MaximizedPane>,
    /// 左键点击落点决定的当前"聚焦"面板区,驱动 `left_zone`/`right_zone`
    /// 外边框的高亮态(见 `set_active_zone`/`zone_at_x`)。启动默认
    /// `Some(Right)`——终端处默认焦点区,终端在右面板区。
    active_zone: Option<ZoneSide>,
    /// 所有按钮的悬停动画状态机(顶栏 Home / 顶栏右侧 / 图标栏),key 为
    /// `HoverId`。iced 0.14 无内置动画 API,这套自驱 redraw(与光标闪烁同款)
    /// 把图标/背景颜色在 idle↔hover 间 ease-out 过渡。进度由 main.rs 的定时
    /// 唤醒经 `advance_hover_anims` 指数逼近各自 `target`(见 `HoverAnim`)。
    hover_anims: std::collections::HashMap<HoverId, HoverAnim>,
    /// 双击顶栏空白处待处理标记,见 `Message::TopBarDoubleClick`/
    /// `take_pending_zoom_toggle`。`App` 不持有 `winit::window::Window`
    /// 句柄,真正切换最大化态由 main.rs 轮询这个标记后调用。
    pending_zoom_toggle: bool,
    /// 全局 UI 缩放(⌘/Ctrl +/-)改变后,预览/浏览器 webview 的
    /// `WebView::zoom` 也要同步——但 `App` 不持有 webview 句柄,只能
    /// 置这个标记,由 main.rs 轮询 `take_pending_preview_zoom` 后逐个
    /// 应用。新 webview 在 `sync_webview_pool` 建出来时直接按当前 scale
    /// 初始化,所以本标记只管"已存在 webview 的缩放变更"这一增量。
    pending_preview_zoom: bool,
    /// 当前窗口逻辑尺寸(宽,高)。由 main.rs 建窗口/`WindowEvent::Resized`
    /// 时经 `set_window_size` 写入。
    window_size: (f32, f32),
    /// 正在拖拽的分隔线;`None` 表示未在拖拽。
    dragging: Option<Divider>,
    /// 正在拖拽的页签(换位);`None` 表示未在拖拽页签。与 `dragging` 分隔线
    /// 互斥(一次左键拖拽只能是一件事)。
    tab_drag: Option<TabDrag>,
    /// Files 面板右键菜单浮层状态——见 `extensions::files::AppState`。
    files: files::AppState,
    /// 文件预览 tab 右键菜单浮层状态(屏幕空间单例,不随项目切换各自保留);
    /// 定位坐标复用 `files.last_right_click`(main.rs 右键时已写入)。
    preview_tab_menu: Option<PreviewTabMenu>,
    /// Project 面板右配对预览 tab 的右键菜单浮层状态,语义同
    /// `preview_tab_menu`,定位坐标同样复用 `files.last_right_click`。
    project_preview_tab_menu: Option<PreviewTabMenu>,
    /// Project 面板「项目文档 / Agent 记忆」链接行的右键菜单浮层状态,坐标
    /// 同样复用 `files.last_right_click`。
    project_link_menu: Option<ProjectLinkMenu>,

    /// 并行打开的项目页签:project id → 该项目的完整/占位状态。
    projects: HashMap<i64, WorkspaceSlot>,
    /// 页签顺序(`projects` 是 HashMap,顺序另存;Task 6 的页签栏按它渲染)。
    project_order: Vec<i64>,
    /// 当前聚焦的项目页签(`None`=一个项目都没打开)。
    active_project_id: Option<i64>,
    /// 当前顶层页面(工作区 / 首页)。默认 `Workspace`;点顶栏 Dozer 切到
    /// `Home`,打开/切换项目切回 `Workspace`。
    current_page: AppPage,
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
    /// 首页右栏当前显示哪个 pane(目前只有 Browser)。语义同上。
    pub(crate) home_right_view: homespace::HomeRightView,
    /// 首页全局浏览器面板状态,不挂在任何 `Workspace` 上;`view`/`update`
    /// 调用时 `project_id` 恒传 `None`(全局收藏夹作用域)。
    pub(crate) home_browser: browser::State,
    /// Git Log 面板状态——自己的 `Message`/`update`/`view`,见
    /// `extensions::git_log`。`App` 级共享、不按项目分(现状,纯重构不改,
    /// 见 `sync_git_log_to_active_project`)。
    git_log: git_log::State,
    /// Todo 面板 App 级状态(派发记录/计划时间/完成时间,按项目分桶,
    /// 启动时读盘、每次变更落盘)——见 `extensions::todo::AppState`。
    todo: todo::AppState,
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
            cols: DEFAULT_COLS,
            rows: DEFAULT_ROWS,
            daemon_error,
            blink_on: true,
            last_blink_at: std::time::Instant::now(),
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
            pending_zoom_toggle: false,
            pending_preview_zoom: false,
            window_size: theme::geometry::initial_window_size(),
            dragging: None,
            tab_drag: None,
            files: files::AppState::default(),
            preview_tab_menu: None,
            project_preview_tab_menu: None,
            project_link_menu: None,
            projects: HashMap::new(),
            project_order: Vec::new(),
            active_project_id: None,
            current_page: AppPage::Workspace,
            recent_projects: Vec::new(),
            home_recent_files: Vec::new(),
            home_recent_conversations: Vec::new(),
            home_recents_loaded: false,
            home_left_view: homespace::HomeLeftView::default(),
            home_right_view: homespace::HomeRightView::default(),
            home_browser: browser::State::with_initial_url("https://byteboy.ai"),
            git_log: git_log::State::default(),
            todo: todo::AppState::load(),
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

    /// 外部 OS 文件拖拽命中测试：窗口坐标 (x, y) 在**当前**左侧是否落在某个
    /// 目录行上，返回该目录路径。main.rs 在 winit 原生事件层的
    /// `CursorMoved`/`DroppedFile` 上调用它——只要不在文件树目录行上就返回
    /// `None`（此时拖入按现状落给终端）。
    ///
    /// 不做任何像素布局复制：文件树列的矩形由 [`left_files_tree_bounds`]
    /// 按 `preview_content_bounds` 同源的谱系换算，可见行集合从当前项目
    /// `WorkspaceState` 现取现算，二者与渲染侧 `files::view` 同源。
    pub fn files_drop_target(
        &self,
        window_w: f32,
        window_h: f32,
        x: f32,
        y: f32,
    ) -> Option<PathBuf> {
        if self.left_collapsed || self.left_view != LeftView::Files {
            return None;
        }
        let ws = self.active_workspace()?;
        let bounds = left_files_tree_bounds(window_w, window_h, &self.shell_state());
        let rows = ws.files.visible_tree_rows();
        files::tree_drop_target(x, y, bounds, ws.files.tree_scroll(), &rows)
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
    /// 唤醒的判断条件（`main.rs::about_to_wait`），跟 `any_blinking`/
    /// `any_hover_anim_active` 同一层级。
    pub fn todo_panel_visible(&self) -> bool {
        self.left_view == LeftView::Todo && self.active_workspace().is_some()
    }

    /// `main.rs` 定时唤醒调用：只在 `todo_panel_visible()` 时才真的
    /// `stat` 一下 `.dozer/todo.md` 的 mtime；没变就是一次系统调用，
    /// 变了才重读+reparse（`todo::reload_from_disk` 内部也会再 stat 一次
    /// mtime，多一次系统调用换取它保持独立可复用）。按 `last_todo_poll_at`
    /// 自限速到 `TODO_POLL_INTERVAL`——悬停动画等更快节奏把唤醒带密时
    /// 不会跟着高频重复 `stat`(2026-08-12 解耦重构,同 `toggle_blink` 的
    /// 处理)。
    pub fn poll_todo_if_visible(&mut self) {
        if !self.todo_panel_visible() {
            return;
        }
        let now = std::time::Instant::now();
        if now.duration_since(self.last_todo_poll_at) < crate::TODO_POLL_INTERVAL {
            return;
        }
        self.last_todo_poll_at = now;
        let Some(ws) = self.active_workspace_mut() else {
            return;
        };
        let Some(project) = ws.project.as_ref() else {
            return;
        };
        let path = todo::todo_path(std::path::Path::new(&project.path));
        let current = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        if current != ws.todo.mtime() {
            todo::reload_from_disk(&mut ws.todo, std::path::Path::new(&project.path));
        }
    }

    /// 是否有 tab 处于"工作中"——决定 main.rs 是否需要定时唤醒来驱动状态点
    /// 闪烁。任一并行项目里有在跑的会话就得继续闪(页签上也要显示状态点)。
    pub fn any_blinking(&self) -> bool {
        self.projects.values().any(|slot| match slot {
            WorkspaceSlot::Loaded(ws) => ws.any_blinking(),
            WorkspaceSlot::Stub { .. } => false,
        })
    }

    /// 翻转闪烁相位；由 main.rs 的定时唤醒每拍调用,但唤醒节奏不专属闪烁
    /// (悬停动画期间会被提到 16ms 一拍),这里按 `last_blink_at` 自己限速
    /// 到 `BLINK_INTERVAL`,未到点的调用直接是 no-op——否则悬停 icon 按钮
    /// 时别处的状态点会跟着高频唤醒一起快速明灭。
    pub fn toggle_blink(&mut self) {
        let now = std::time::Instant::now();
        if now.duration_since(self.last_blink_at) < crate::BLINK_INTERVAL {
            return;
        }
        self.blink_on = !self.blink_on;
        self.last_blink_at = now;
    }

    /// 设置某按钮的悬停目标（`true`=进入,`false`=离开）；动画由
    /// `advance_hover_anims` 循环把它指数逼近（见 `HoverAnim`）。
    pub fn set_hover(&mut self, id: HoverId, hovered: bool) {
        self.hover_anims.entry(id).or_default().set(hovered);
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
    }

    /// 某按钮当前悬停动画进度(0..=1)，给视图层做颜色插值。
    pub fn hover_progress(&self, id: HoverId) -> f32 {
        self.hover_anims.get(&id).map(HoverAnim::t).unwrap_or(0.0)
    }

    /// 拖拽换位:把当前拖起的源项(`self.tab_drag.source`)移到 `group` 组的
    /// `to` 处。源与目标同址/越界/不在拖拽中均 no-op。换位后把 `self.tab_drag
    /// .source` 更新成新位置(续拖以新位置为准),并顺带修正受影响的 index-
    /// keyed hover 键/激活项。
    fn tab_drag_move(&mut self, group: TabGroup, to: usize) {
        let Some(drag) = self.tab_drag else {
            return;
        };
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
                self.tab_drag = Some(TabDrag { group, source: to });
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
                self.tab_drag = Some(TabDrag { group, source: to });
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
                self.tab_drag = Some(TabDrag { group, source: to });
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
                self.tab_drag = Some(TabDrag { group, source: to });
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
                self.tab_drag = Some(TabDrag { group, source: to });
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

    /// 浏览器地址栏是否在编辑态(main.rs 据此路由键盘)。
    pub fn browser_addr_editing(&self) -> bool {
        self.active_workspace()
            .is_some_and(|ws| ws.browser_addr_editing())
    }

    /// 验收意见输入是否在编辑态（main.rs 键盘路由用）。
    pub fn acceptance_comment_editing(&self) -> bool {
        self.active_workspace()
            .is_some_and(|ws| ws.acceptance_comment_editing())
    }

    /// 项目树是否处于行内编辑态(main.rs 键盘路由用)。
    pub fn tree_editing(&self) -> bool {
        self.active_workspace().is_some_and(|ws| ws.tree_editing())
    }

    /// 文件树搜索框是否处于自绘编辑态(main.rs 键盘路由用)。为真时按键改
    /// 路由成 `files::Message::SearchEvent`,不再喂 PTY。
    pub fn search_editing(&self) -> bool {
        self.active_workspace()
            .is_some_and(|ws| ws.search_editing())
    }

    /// 右键文件树"搜索"弹窗是否打开(main.rs 键盘路由 + App view 浮层用)。
    pub fn search_popup_open(&self) -> bool {
        self.active_workspace()
            .is_some_and(|ws| ws.search_popup_open())
    }

    /// 右键文件树"搜索"弹窗查询框是否处于编辑态(main.rs 键盘路由用)。
    pub fn search_popup_editing(&self) -> bool {
        self.active_workspace()
            .is_some_and(|ws| ws.search_popup_editing())
    }

    /// 项目信息面板名称是否处于自绘编辑态(main.rs 键盘路由用)。
    pub fn project_name_editing(&self) -> bool {
        self.active_workspace()
            .is_some_and(|ws| ws.project_name_editing())
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

    /// 当前激活预览 tab 的 webview id(main.rs 焦点路由用)。
    pub fn active_preview_webview_id(&self) -> Option<usize> {
        self.active_workspace()?.active_preview_webview_id()
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

    /// 点击输入框外时退出所有自绘输入的编辑态(验收反馈:失焦回正常态)。
    /// 项目名称编辑走"失焦保存":取出半输入缓冲,改动且非空时发起 daemon
    /// 改名(与回车提交同一路径),未改动/空名则直接丢弃编辑框,与描述字段
    /// "失焦写盘"行为对齐——修复之前失焦把改名直接丢弃、看起来"无法保存"。
    pub fn blur_inputs(&mut self) {
        let Some(ws) = self.active_workspace_mut() else {
            return;
        };
        // 先取出名称编辑缓冲,再交给 `Workspace::blur_inputs` 清其它编辑态,
        // 避免顺序问题丢失半输入。
        let pending_name = ws.project_panel.take_name_edit();
        let project = ws.project.clone();
        ws.blur_inputs();
        if let (Some(p), Some(raw)) = (project, pending_name) {
            let name = raw.trim().to_string();
            let project_id = p.id;
            let current_name = p.name.clone();
            if !name.is_empty() && name != current_name {
                let client = self.client.clone();
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let emit = move |m: project::Message| {
                    let _ = proxy.send_event(Message::Project(m));
                };
                handle.spawn(async move {
                    let result = client
                        .rename_project(project_id, &name)
                        .await
                        .map_err(|e| e.to_string())
                        .and_then(|opt| opt.ok_or_else(|| "项目不存在".to_string()));
                    emit(project::Message::NameRenamed(project_id, result));
                });
            }
        }
    }

    /// 把几何状态(宽度/分割比例/窗口尺寸,**不含**左右视图选择与收起态——
    /// 那些按项目分,见 `panel_layouts`)写盘。图标切换/收起要立即持久化几何,
    /// 不能只靠 `ColumnDragEnd` 顺带存(用户可能从没拖过分隔线)。
    fn spawn_shell_layout_save(&mut self) {
        let layout = self.shell_layout;
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
    pub(crate) fn keyboard_term_target(&self) -> TermTarget {
        crate::app::keyboard_term_target(self.left_view, self.active_zone)
    }

    /// 建窗时用的初始窗口尺寸偏好:优先用上次退出前持久化的
    /// `shell_layout.window_width/height`(已经过 `sanitize_shell_layout`
    /// 夹取),`layout.json` 不存在/读不到时 `layout::load()` 本身已经退化
    /// 成 `ShellLayout::default()`,即 `theme::geometry::initial_window_size()`,这里不用再
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

    /// 按当前窗口尺寸+外壳状态重算终端网格并同步给所有 tab / daemon。
    ///
    /// 用于换算的是"右侧展开且显示 Agent 配对"这个假想状态,而不是当前
    /// 真实状态:终端此刻可能不可见(右侧收起 / 右视图是对话),但它的 PTY
    /// 网格仍应按"被显示时占多大"来定——否则上次退出前停在对话视图的会话,
    /// 重开 app 后会一直卡在 `DEFAULT_COLS`×`DEFAULT_ROWS`(80×24),直到用户
    /// 偶然拖一下窗口才纠正(Fix round 2 #6)。假想状态用 `ShellState`
    /// 的字段覆盖表达,不另写一套宽度公式,避免两份几何漂移。
    fn sync_terminal_grid(&mut self) {
        let (w, h) = self.window_size;
        let shown = terminal_grid_state(self.shell_state());
        let (pane_w, pane_h) = terminal_pane_pixel_size(w, h, &shown);
        let (cols, rows) = crate::term_view::grid_size(pane_w, pane_h);
        if cols > 0 && rows > 0 {
            self.update(Message::PaneResized {
                cols: cols as u16,
                rows: rows as u16,
            });
        }
    }

    /// Ctrl ± / 重置缩放后,把全局 scale 变化同步到所有已打开的原生编辑器
    /// (预览 tab + 编辑弹层),使其字号随终端/图标一起缩放——否则编辑器字号
    /// 冻结在打开时刻(见 `preview::dozer_editor_font_metrics` 注释)。
    fn resync_editor_font_metrics(&mut self) {
        let ids: Vec<i64> = self.projects.keys().copied().collect();
        for pid in ids {
            if let Some(ws) = loaded_workspace_mut(&mut self.projects, pid) {
                ws.resync_editor_font_metrics();
            }
        }
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

    /// 当前外壳几何状态快照(main.rs 拖拽追踪/离屏几何计算用;`Copy`
    /// 类型直接按值返回)。
    pub fn shell_state(&self) -> ShellState {
        ShellState {
            layout: self.shell_layout,
            dims: self.dims,
            left_view: self.left_view,
            left_collapsed: self.left_collapsed,
            right_view: self.right_view,
            right_collapsed: self.right_collapsed,
            maximized: self.maximized,
        }
    }

    /// 当前正在拖拽的分隔线(main.rs 拖拽追踪用,调用方为
    /// `on_window_event` 的 `CursorMoved`/`MouseInput{Released}` 分支)。
    pub fn dragging_divider(&self) -> Option<Divider> {
        self.dragging
    }

    /// 当前正在拖拽的页签(main.rs 拖拽追踪用,调用方同 `dragging_divider`)。
    pub fn dragging_tab(&self) -> Option<TabDrag> {
        self.tab_drag
    }

    /// 当前是否正按住 `group` 组的页签(渲染侧据此把光标改成"抓取"把手)。
    pub fn dragging_group(&self, group: TabGroup) -> bool {
        self.tab_drag.is_some_and(|d| d.group == group)
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
            || self.preview_tab_menu.is_some()
            || self.project_preview_tab_menu.is_some()
            || self.project_link_menu.is_some()
    }

    /// 预览 tab 右键菜单是否打开(main.rs Esc 键路由用)。
    pub fn preview_tab_context_menu_open(&self) -> bool {
        self.preview_tab_menu.is_some()
    }

    /// Project 面板右配对预览 tab 右键菜单是否打开(main.rs Esc 键路由用)。
    pub fn project_preview_tab_context_menu_open(&self) -> bool {
        self.project_preview_tab_menu.is_some()
    }

    /// Project 面板链接行右键菜单是否打开(main.rs Esc 键路由用)。
    pub fn project_link_context_menu_open(&self) -> bool {
        self.project_link_menu.is_some()
    }

    /// 打开 Project 面板「项目文档 / Agent 记忆」链接行的删除右键菜单。与
    /// 文件树右键菜单互斥(坐标复用 `files.last_right_click`)。
    fn project_link_context_menu(&mut self, target: project::links::LinkTarget, index: usize) {
        let (x, y) = self.files.last_right_click();
        self.files.close_context_menu();
        self.project_link_menu = Some(ProjectLinkMenu {
            x,
            y,
            target,
            index,
        });
    }

    /// Agent 选择菜单是否打开(main.rs Esc 键路由用)。
    pub fn agent_picker_open(&self) -> bool {
        self.active_workspace()
            .map(|ws| ws.agent_picker_open)
            .unwrap_or(false)
    }

    /// Todo 派发选择层是否打开(给 main.rs 的 Esc 关闭用)。
    pub fn todo_dispatch_open(&self) -> bool {
        self.active_workspace()
            .map(|ws| ws.todo.dispatch_popup_open())
            .unwrap_or(false)
    }

    /// Todo 状态 pill 菜单是否打开(给 main.rs 的 Esc 关闭用,同
    /// `todo_dispatch_open` 的既有模式)。
    pub fn todo_state_pill_open(&self) -> bool {
        self.active_workspace()
            .map(|ws| ws.todo.state_pill_menu_open())
            .unwrap_or(false)
    }

    /// 预览编辑弹层是否打开(main.rs 键盘路由用)。打开期间键盘必须走
    /// 弹层的文本编辑器,不能落进终端 PTY——弹层挂在左侧预览面板,不影响
    /// `terminal_visible()` 的判断条件(右侧展开与否),不加这道闸门的话,
    /// 默认布局(右侧终端可见)下编辑弹层里敲的每个字符,包括回车,都会
    /// 同时写进背后那个终端/agent 会话。
    pub fn edit_session_open(&self) -> bool {
        self.active_workspace()
            .map(|ws| ws.edit_session.is_some())
            .unwrap_or(false)
    }

    /// 当前激活预览 tab 是否走原生渲染。main.rs 键盘路由用,同 `edit_session_open`
    /// 那道闸门的道理——但原生预览不是模态弹层,还要求键盘焦点确实在预览列
    /// (`current_focus == FocusIntent::Preview`),否则用户正在打字给终端时,
    /// 只因为预览列背景里开着一个原生 tab 就会把按键错误地拦下来。
    pub fn active_preview_tab_has_native_editor(&self) -> bool {
        self.active_workspace()
            .map(|ws| ws.active_preview_tab_has_native_editor())
            .unwrap_or(false)
    }

    /// 转发 `iced-code-editor` 的内部消息到当前聚焦项目的编辑器,返回编辑器
    /// 产生的 `iced::Task`(剪贴板读写/搜索框聚焦等),交由 `main.rs` 的 Task
    /// 桥接器执行。`iced_code_editor::Message` 经 `Message::EditorEvent` 进入
    /// `main.rs::dispatch` 后才会走到这里。
    pub fn preview_edit_event(
        &mut self,
        event: iced_code_editor::Message,
    ) -> iced_winit::runtime::Task<iced_code_editor::Message> {
        if let Some(ws) = self.active_workspace_mut() {
            ws.preview_edit_event(event)
        } else {
            iced_winit::runtime::Task::none()
        }
    }

    /// 转发到聚焦项目里某个原生预览 tab 的 editor,语义同 `preview_edit_event`
    /// 但按 `tab_id` 定位而不是"当前编辑弹层"。`Message::PreviewEditorEvent` 经
    /// `main.rs::dispatch` 进入后走到这里。
    pub fn preview_tab_editor_event(
        &mut self,
        tab_id: usize,
        event: iced_code_editor::Message,
    ) -> iced_winit::runtime::Task<iced_code_editor::Message> {
        let io = self.shell_io();
        if let Some(ws) = self.active_workspace_mut() {
            let task = ws.preview_tab_editor_event(tab_id, event);
            ws.spawn_preview_context_push(&io);
            task
        } else {
            iced_winit::runtime::Task::none()
        }
    }

    /// Project 面板右配对预览 tab 的 `iced-code-editor` 内部消息转发,语义同
    /// `preview_tab_editor_event`,作用于 `ws.project_preview`。
    pub fn project_preview_tab_editor_event(
        &mut self,
        tab_id: usize,
        event: iced_code_editor::Message,
    ) -> iced_winit::runtime::Task<iced_code_editor::Message> {
        if let Some(ws) = self.active_workspace_mut() {
            ws.project_preview_tab_editor_event(tab_id, event)
        } else {
            iced_winit::runtime::Task::none()
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

    /// 当前文本光标的窗口逻辑坐标 `(x, y_底, 行高)`,给 main.rs 设 IME
    /// 候选窗位置(让选词窗落在光标右下,而非窗口左上)。地址栏/意见框编辑
    /// 态用预览列上部近似(iced 立即模式拿不到精确控件屏坐标);否则用终端
    /// 光标——单元格尺寸由 pane 像素 ÷ 网格推出,不依赖字号常量。
    pub fn ime_cursor_area(&self, window_w: f32, window_h: f32) -> (f32, f32, f32) {
        let state = self.shell_state();
        if self.browser_addr_editing() || self.acceptance_comment_editing() {
            let (bx, by, _bw, _bh) = preview_content_bounds(window_w, window_h, &state);
            return (bx + 4.0, by, 20.0);
        }
        let (pane_w, pane_h) = terminal_pane_pixel_size(window_w, window_h, &state);
        let cell_w = pane_w / self.cols.max(1) as f32;
        let line_h = pane_h / self.rows.max(1) as f32;
        let right_w = right_zone_width(window_w, &state);
        let (list_w, _) =
            pair_list_content_width(pair_content_width(right_w), state.dims.agent_split);
        let x0 = window_w - theme::geometry::icon_rail_width() - right_w
            + list_w
            + theme::geometry::divider_width()
            + 8.0;
        // 终端网格上方 chrome:顶栏 44 + 上 padding 8 + tab 栏 30 + spacing 4(header 已去,P1L #4)
        let y0 = theme::geometry::top_bar_height() + 8.0 + 30.0 + 4.0;
        let (col, row) = self
            .active_workspace()
            .and_then(|ws| ws.tabs.get(ws.active))
            .map(|t| t.model.cursor())
            .unwrap_or((0, 0));
        let x = x0 + col as f32 * cell_w;
        let y = y0 + (row as f32 + 1.0) * line_h; // 光标格底部,候选窗落其下方
        (x, y, line_h)
    }

    /// 当前应存在的 webview 清单(main.rs 差集同步)。不在文件视图时整体
    /// 清空:`preview_pane` 此刻根本不在屏上,若不清空,其原生 wry 子视图会
    /// 无视 iced 绘制顺序,径直叠在浏览器视图之上(与 `browser_desired` 互斥
    /// 同理)。webview 池是窗口级的,所以只认当前聚焦项目的清单——后台项目
    /// 的预览 tab 不该把自己的原生子视图画到别人的界面上。
    pub fn preview_desired(&self) -> Vec<WebviewSpec> {
        // 进首页(Dozer Home)时,预览区根本不在屏上——原生 wry 子视图无视
        // iced 绘制顺序,若不主动清空,会径直叠在 homespace 页面之上。
        // 与 `browser_desired`(app.rs:1896 已对 Home 重定向到 `home_browser`)
        // 保持一致:首页时不返回任何文件预览 webview,让 main.rs 的差集同步
        // 把残留的那个销毁掉。
        if self.current_page == AppPage::Home {
            return Vec::new();
        }
        if !matches!(self.left_view, LeftView::Files | LeftView::Project) {
            return Vec::new();
        }
        let Some(ws) = self.active_workspace() else {
            return Vec::new();
        };
        // Files 预览取 `ws.preview`,Project 面板右配对取 `ws.project_preview`——
        // 两者都是"预览区",复用同一支几何/可见性逻辑,只是状态源不同。
        let specs = match self.left_view {
            LeftView::Files => ws.preview.desired_webviews(),
            LeftView::Project => ws.project_preview.desired_webviews(),
            _ => Vec::new(),
        };
        // 编辑弹层开着时,应用级模态盖住了预览区,原生 wry 子视图不听 iced
        // 绘制顺序摆布,必须显式 visible=false 才能真正藏起来。
        // (预览 tab 右键菜单不藏 webview——它向上弹出,落在 tab 栏上方的
        // iced 区域,根本不压到下方 webview,见 `preview_tab_context_menu_popup`。)
        if ws.edit_session.is_some() {
            specs
                .into_iter()
                .map(|mut s| {
                    s.visible = false;
                    s
                })
                .collect()
        } else {
            specs
        }
    }

    /// 浏览器域的 webview 清单,语义同 `preview_desired`,查独立的
    /// `Workspace::browser`,且只在左视图为 `Web`(非首页)或首页时非空。
    pub fn browser_desired(&self) -> Vec<WebviewSpec> {
        // 首页右栏恒为全局浏览器(`home_browser`),与 `left_view` 无关——
        // 进首页就让它成为浏览器 webview 池的唯一来源,否则默认 URL 的 tab
        // 建了却永远等不到 webview(见 `sync_webview_pool`)。
        if self.current_page == AppPage::Home {
            return self.home_browser.desired_webviews();
        }
        if self.left_view != LeftView::Web {
            return Vec::new();
        }
        match self.active_workspace() {
            Some(ws) => ws.browser.desired_webviews(),
            None => Vec::new(),
        }
    }

    /// 保证 `git_log` 状态跟得上"现在应该看哪个项目"——`git_log: State`
    /// 是 `App` 级字段,不是每个项目各自一份(不像 `Workspace.files`),
    /// 所以面板打开时(`LeftIconSelect`)和切项目页签时(`ProjectTabSwitch`)
    /// 都得调这个方法对齐一次,否则 Git Log 面板开着的状态下切页签,提交图
    /// 会停在上一个项目不动,而同一面板里的 worktree 速览条(`ws.files
    /// .worktrees()` 是按项目取的)却已经跳到新项目——两者对不上。缓存已经是当前项目的
    /// 路径就不动(避免每次切页签都重算一遍),路径不一致就重建,没有项目
    /// 就清空。只在 `left_view == LeftView::GitLog` 时调用才有意义。
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
            Message::DeliveryChecked(project_id, tab_id, pending) => {
                self.delivery_checked(project_id, tab_id, pending)
            }
            Message::Acceptance(acceptance::Message::Open(tab_id)) => self.acceptance_open(tab_id),
            Message::Acceptance(acceptance::Message::Reject) => self.acceptance_reject(),
            Message::Acceptance(
                msg @ (acceptance::Message::Loaded(project_id, ..)
                | acceptance::Message::DiffLoaded(project_id, ..)
                | acceptance::Message::Done(project_id, ..)),
            ) => self.acceptance_result(project_id, msg),
            Message::Acceptance(msg) => {
                let Some(project_id) = self.active_project_id else {
                    return;
                };
                self.with_focused_project(|ws, io| {
                    let client = io.client.clone();
                    let handle = io.handle.clone();
                    let proxy = io.proxy.clone();
                    let emit = move |m| {
                        let _ = proxy.send_event(Message::Acceptance(m));
                    };
                    acceptance::update(&mut ws.acceptance, msg, project_id, &client, &handle, emit);
                });
            }
            Message::ReviewLoaded(project_id, source, result) => {
                self.with_project(project_id, |ws, _io| {
                    if let Some(rv) = &mut ws.review
                        && rv.source == source
                    {
                        match result {
                            Ok(entries) => {
                                rv.entries = entries;
                                rv.error = None;
                            }
                            Err(e) => rv.error = Some(e),
                        }
                    }
                });
            }
            Message::ReviewToggle(i) => {
                self.with_focused_project(|ws, _io| {
                    if let Some(rv) = &mut ws.review
                        && !rv.expanded.remove(&i)
                    {
                        rv.expanded.insert(i);
                    }
                });
            }
            Message::ConversationsRefreshed(project_id, list) => {
                self.with_project(project_id, move |ws, _io| {
                    ws.conversations = list;
                });
            }
            Message::Usage(msg @ usage::Message::Loaded(project_id, ..)) => {
                self.with_project(project_id, move |ws, io| {
                    let Some(project) = &ws.project else { return };
                    let project_path = PathBuf::from(&project.path);
                    let handle = io.handle.clone();
                    let proxy = io.proxy.clone();
                    let emit = move |m| {
                        let _ = proxy.send_event(Message::Usage(m));
                    };
                    usage::update(&mut ws.usage, msg, project_id, project_path, &handle, emit);
                });
            }
            Message::Usage(msg @ usage::Message::Hover(_)) => {
                let usage::Message::Hover(h) = msg else {
                    unreachable!()
                };
                self.set_hover(HoverId::UsageRefresh, h);
            }
            Message::Usage(msg) => {
                self.with_focused_project(|ws, io| {
                    let Some(project) = &ws.project else { return };
                    let project_id = project.id;
                    let project_path = PathBuf::from(&project.path);
                    let handle = io.handle.clone();
                    let proxy = io.proxy.clone();
                    let emit = move |m| {
                        let _ = proxy.send_event(Message::Usage(m));
                    };
                    usage::update(&mut ws.usage, msg, project_id, project_path, &handle, emit);
                });
            }
            Message::ConversationOpen(path) => self.conversation_open(path),

            Message::SelectTab(idx) => self.select_tab(idx),
            Message::CloseTab(idx) => {
                self.with_focused_project(|ws, io| {
                    ws.close_tab(io, idx);
                    ws.ensure_project_terminal(io);
                });
            }
            Message::AgentPickerToggle => {
                self.with_focused_project(|ws, _io| {
                    ws.agent_picker_open = !ws.agent_picker_open;
                });
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
            Message::Todo(todo::Message::DispatchToExisting(idx, session_id)) => {
                self.todo_dispatch_to_existing(idx, session_id)
            }
            Message::Todo(todo::Message::DispatchNew(idx, launch)) => {
                self.todo_dispatch_new(idx, launch)
            }
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
            Message::Database(database::Message::ToolbarHover(target, hovered)) => {
                // 数据库面板 schema 树头部 icon 按钮的悬停:本面板不挂 App 的
                // hover 动画表,把进入/离开转发成 `HoverId` 由内核统一驱动动画。
                let id = match target {
                    database::DatabaseToolbarTarget::SchemaBack => HoverId::DatabaseSchemaBack,
                };
                self.set_hover(id, hovered);
            }
            Message::Database(msg) => self.database_message(msg),
            Message::Todo(msg) => self.todo_message(msg),
            // 文件树右键"搜索"弹窗:`SearchResults` 带 `project_id`,异步结果
            // 按所属项目路由(用户可能已切走);其余交互投当前聚焦项目。
            Message::Search(search::Message::SearchResults(project_id, result)) => {
                self.search_results(project_id, result)
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
            Message::TabAttached(project_id, tab_id, info, snapshot) => {
                let session_id = info.id.clone();
                self.with_project(project_id, move |ws, io| {
                    ws.on_tab_attached(io.cols, io.rows, tab_id, info, snapshot)
                });
                // 若是从 Todo 面板"派发到新建"建的 tab,补记派发记录——此时才
                // 第一次知道真正的 `session_id`。`TabAttached` 是异步结果消息,
                // 必须按自带的 `project_id` 路由(同 `on_tab_attached` 那一步),
                // 不能用 `active_workspace_mut()`(当前聚焦项目可能已经切走)。
                if let Some(text) = loaded_workspace_mut(&mut self.projects, project_id)
                    .and_then(|ws| ws.todo.take_pending_dispatch(tab_id))
                {
                    self.todo.record_dispatch(project_id, &text, session_id);
                }
            }
            Message::PaneResized { cols, rows } => self.pane_resized(cols, rows),
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
            Message::TabDragMove { group, index } => {
                self.tab_drag_move(group, index);
            }
            Message::TabDragEnd => {
                self.end_tab_drag();
            }
            Message::LeftIconSelect(v) => self.left_icon_select(v),
            Message::RightIconSelect(v) => self.right_icon_select(v),
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
            Message::Noop => {}
            Message::DaemonError(message) => self.daemon_error = Some(message),
            Message::TermScroll(target, delta) => {
                self.with_focused_project(|ws, _io| {
                    let tab = match target {
                        TermTarget::Shared => ws.tabs.get_mut(ws.active),
                        TermTarget::SshPanel => ws.ssh_active_tab_mut(),
                    };
                    if let Some(tab) = tab {
                        tab.model.scroll_display(delta);
                    }
                });
            }
            Message::TermTabScroll(right) => {
                self.with_focused_project(|ws, _io| {
                    if right {
                        ws.term_tab_first = ws.term_tab_first.saturating_add(2);
                    } else {
                        ws.term_tab_first = ws.term_tab_first.saturating_sub(2);
                    }
                });
            }
            Message::PreviewTabScroll(right) => {
                self.with_focused_project(|ws, _io| {
                    if right {
                        ws.preview_tab_first = ws.preview_tab_first.saturating_add(2);
                    } else {
                        ws.preview_tab_first = ws.preview_tab_first.saturating_sub(2);
                    }
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
                        TermTarget::Shared => ws.tabs.get_mut(ws.active),
                        TermTarget::SshPanel => ws.ssh_active_tab_mut(),
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
                        TermTarget::Shared => ws.tabs.get_mut(ws.active),
                        TermTarget::SshPanel => ws.ssh_active_tab_mut(),
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
                self.preview_tab_menu = None;
                self.with_focused_project(|ws, io| {
                    ws.preview.close(idx);
                    // 关 tab 后位置全变，旧 first 可能越界——归零防御（P1L T5）。
                    ws.preview_tab_first = 0;
                    ws.spawn_preview_state_save(io);
                    ws.spawn_preview_context_push(io);
                });
            }
            Message::PreviewEditOpen(idx) => {
                self.preview_tab_menu = None;
                self.with_focused_project(move |ws, _io| ws.preview_edit_open(idx));
            }
            Message::PreviewEditOpenByTab(tab_id) => {
                self.with_focused_project(move |ws, _io| ws.preview_edit_open_by_id(tab_id));
            }
            Message::EditorEvent(_event) => {
                // `iced-code-editor` 的内部消息走 `main.rs` 的 Task 桥接器
                // (需要 `Clipboard` 句柄执行剪贴板副作用),这里不处理——若
                // 真到达 `App::update` 说明事件未走 dispatch 拦截,直接忽略。
            }
            Message::PreviewEditorEvent(_tab_id, _event) => {
                // 原生预览 tab 的 `iced-code-editor` 内部消息同样走 `main.rs`
                // 的 Task 桥接器(`run_preview_tab_editor_task`),与 `EditorEvent`
                // 同口径:到达 `App::update` 说明未走 dispatch 拦截,直接忽略。
            }
            Message::PreviewEditSave => {
                self.with_focused_project(|ws, _io| ws.preview_edit_save());
            }
            Message::PreviewEditCloseRequest => {
                self.with_focused_project(|ws, _io| ws.preview_edit_close_request());
            }
            Message::PreviewEditConfirmDiscard => {
                self.with_focused_project(|ws, _io| ws.preview_edit_confirm_discard());
            }
            Message::PreviewEditConfirmCancel => {
                self.with_focused_project(|ws, _io| ws.preview_edit_confirm_cancel());
            }
            Message::PreviewTabContextMenu { idx, editable } => {
                let (x, y) = self.files.last_right_click();
                // 与文件树右键菜单互斥——避免两者同时挂着,关掉一个把另一个
                // 意外顶出来。
                self.files.close_context_menu();
                self.preview_tab_menu = Some(PreviewTabMenu {
                    x,
                    y,
                    idx,
                    editable,
                });
            }
            Message::PreviewTabContextMenuClose => {
                self.preview_tab_menu = None;
            }
            Message::ProjectPreviewOpenPath(path) => self.project_preview_open_path(path),
            Message::ProjectPreviewSelectTab(idx) => self.project_preview_select_tab(idx),
            Message::ProjectPreviewCloseTab(idx) => {
                self.project_preview_tab_menu = None;
                self.with_focused_project(|ws, _io| {
                    ws.project_preview.close(idx);
                    ws.project_preview_tab_first = 0;
                });
            }
            Message::ProjectPreviewTabScroll(right) => {
                self.with_focused_project(|ws, _io| {
                    if right {
                        ws.project_preview_tab_first =
                            ws.project_preview_tab_first.saturating_add(2);
                    } else {
                        ws.project_preview_tab_first =
                            ws.project_preview_tab_first.saturating_sub(2);
                    }
                });
            }
            Message::ProjectPreviewEditorEvent(_tab_id, _event) => {
                // 与 `PreviewEditorEvent` 同口径:到达 `App::update` 说明未走
                // main.rs 的 Task 桥接器,直接忽略。
            }
            Message::ProjectPreviewEditOpen(idx) => {
                self.project_preview_tab_menu = None;
                self.with_focused_project(move |ws, _io| ws.project_preview_edit_open(idx));
            }
            Message::ProjectPreviewEditOpenByTab(tab_id) => {
                self.with_focused_project(move |ws, _io| {
                    ws.project_preview_edit_open_by_id(tab_id)
                });
            }
            Message::ProjectPreviewTabContextMenu { idx, editable } => {
                let (x, y) = self.files.last_right_click();
                self.project_preview_tab_menu = Some(PreviewTabMenu {
                    x,
                    y,
                    idx,
                    editable,
                });
            }
            Message::ProjectPreviewTabContextMenuClose => {
                self.project_preview_tab_menu = None;
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
            Message::Browser(msg) => self.browser_message(msg),
            Message::ProjectSelect(id) => self.project_select(id),
            Message::ProjectTabPickFolder => {} // 副作用在 main.rs(rfd 文件夹选择)
            // rfd 弹窗在 main.rs 里同步处理,选中后转成 project::Message::LinkAdd
            // 再回送到这里;这条顶层消息本身不需要 App::update 处理任何东西。
            Message::ProjectLinkPick(_) => {}
            Message::ProjectLinkContextMenuClose => {
                self.project_link_menu = None;
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
            Message::ProjectFsChanged(project_id, relevance) => {
                self.project_fs_changed(project_id, relevance)
            }
            Message::GitLog(git_log::Message::ProjectTabOpen(p)) => {
                // worktree 条带里点其它 worktree,转成内核的切项目消息。
                self.update(Message::ProjectTabOpen(p));
            }
            Message::GitLog(git_log::Message::LoadMore) => self.git_log_load_more(),
            Message::GitLog(msg) => {
                let handle = self.handle.clone();
                let proxy = self.proxy.clone();
                let emit = move |m| {
                    let _ = proxy.send_event(Message::GitLog(m));
                };
                if let Some(next) = git_log::update(&mut self.git_log, msg, &handle, emit) {
                    self.update(Message::GitLog(next));
                }
            }
            Message::Files(files::Message::CopyPath(path, kind)) => {
                let _ = (path, kind); // main.rs 拦截处理写剪贴板,这里维持现状空分支
            }
            Message::Files(files::Message::OpenFile(path)) => {
                // 单击文件行打开预览——`files` 模块不认识预览域,这条消息由
                // 内核拦截转发成核心的 `PreviewOpenPath`(同
                // `files::Message::CopyPath`,不能落进下面的兜底分支,否则会
                // 命中 `files::update` 里的 `unreachable!`)。
                self.update(Message::PreviewOpenPath(path));
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
                | project::Message::AcceptanceCountLoaded(project_id, ..)
                | project::Message::NameRenamed(project_id, ..)
                | project::Message::DiskUsageLoaded(project_id, ..)),
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
            Message::Project(project::Message::OpenLink(path)) => {
                // 项目链接打开的文件进 Project 面板右配对的预览(`ws.project_preview`),
                // 不冲进 Files 预览——两条预览各自独立,互相不打扰。
                self.update(Message::ProjectPreviewOpenPath(path));
            }
            Message::Project(project::Message::Pick(target)) => {
                self.update(Message::ProjectLinkPick(target));
            }
            Message::Project(project::Message::LinkContextMenu { target, index }) => {
                self.project_link_context_menu(target, index);
            }
            Message::Project(project::Message::LinkRemove { target, index }) => {
                // 删除来自行内右键菜单:落 `LinkRemove` 时把菜单浮层一并收起。
                self.project_link_menu = None;
                self.update(Message::Project(project::Message::LinkRemove {
                    target,
                    index,
                }));
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
                        ws.spawn_ssh_tab(io, host_id);
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
                });
            }
            // SFTP tab 内部交互:按 host_id 路由到 `sftp::route`,真正的
            // 处理逻辑在那边(sftp::Message 有 7+ 个变体,内容又都操作
            // `ws.sftp_tabs`,摊平会让这里的大 match 更难读)。
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
            Message::Ssh(msg) => {
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
                crate::theme::icon_size::zoom_by(UI_ZOOM_STEP);
                crate::theme::icon_size::persist_scale();
                self.sync_terminal_grid();
                self.resync_editor_font_metrics();
                self.pending_preview_zoom = true;
            }
            Message::ZoomOut => {
                crate::theme::icon_size::zoom_by(1.0 / UI_ZOOM_STEP);
                crate::theme::icon_size::persist_scale();
                self.sync_terminal_grid();
                self.resync_editor_font_metrics();
                self.pending_preview_zoom = true;
            }
            Message::ZoomReset => {
                crate::theme::icon_size::reset_scale();
                self.sync_terminal_grid();
                self.resync_editor_font_metrics();
                self.pending_preview_zoom = true;
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
        if self.left_view == LeftView::GitLog {
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
            self.handle.spawn(async move {
                for sid in ids {
                    if let Err(e) = client.kill(&sid).await {
                        tracing::warn!("丢弃过期促成结果时结束会话失败: {e}");
                    }
                }
            });
            return;
        }
        let io = self.shell_io();
        let mut ws = Workspace::from_restore(&io, *restore, inherited);
        // 重挂出来的会话,终端模型是按 `DEFAULT_COLS`×`DEFAULT_ROWS`
        // 建的,得按当前窗口几何纠正一次。这里**不能**指望
        // `sync_terminal_grid`:它算出来的网格与 `self.cols/rows` 相同
        // 时 `PaneResized` 会原地返回(去重),于是这份新装配的
        // `Workspace` 会一直停在 80×24。直接对它自己 resize 一次。
        ws.resize_all(&io, io.cols, io.rows);
        self.projects
            .insert(id, WorkspaceSlot::Loaded(Box::new(ws)));
    }

    fn project_fs_changed(&mut self, project_id: ProjectId, relevance: git_watch::Relevance) {
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
        if relevance == git_watch::Relevance::GitRefs
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

    fn acceptance_open(&mut self, tab_id: usize) {
        self.with_focused_project(|ws, io| {
            let active_repo = ws.project.as_ref().map(|p| PathBuf::from(&p.path));
            let Some(project_id) = ws.project_id() else {
                return;
            };
            let Some(tab) = ws.tab_by_id_mut(tab_id) else {
                return;
            };
            tab.delivery_pending = false;
            let cwd = effective_project_repo(active_repo.as_deref(), &tab.effective_cwd());
            let handle = io.handle.clone();
            let proxy = io.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::Acceptance(m));
            };
            acceptance::spawn_open(project_id, tab_id, cwd, &handle, emit);
        });
    }

    fn acceptance_reject(&mut self) {
        self.with_focused_project(|ws, io| {
            let Some(session) = ws.acceptance.session() else {
                return;
            };
            let comment = session.comment().trim().to_string();
            let source = session.source_tab_id();
            let target = ws.tabs.iter().find(|t| t.tab_id == source);
            let Some(tab) = target.filter(|t| t.alive) else {
                // 会话已结束,意见无处可注——留住当前 session,不清空,让用户
                // 看到错误(现有 `acceptance_reject` 的降级路径)。
                ws.acceptance
                    .set_error("会话已结束,意见无处可注".to_string());
                return;
            };
            let id = tab.info.id.clone();
            let client = io.client.clone();
            let text_out = format!("[Dozer 验收打回] {comment}\n");
            io.handle.spawn(async move {
                if let Err(e) = client.write(&id, text_out.as_bytes()).await {
                    tracing::warn!("打回注回失败: {e}");
                }
            });
            ws.acceptance.clear_session();
        });
    }

    fn acceptance_result(&mut self, project_id: i64, msg: acceptance::Message) {
        // 判断"这次是不是通过成功"要在 `msg` 被 `move` 进闭包之前算好
        // (用 `&msg` 引用匹配,不消耗它;闭包里 `acceptance::update` 会真正
        // 拿走 `msg` 的所有权),否则会撞上"用后借用"的编译错误。
        let is_accept_ok = matches!(&msg, acceptance::Message::Done(_, Ok(_)));
        self.with_project(project_id, move |ws, io| {
            let client = io.client.clone();
            let handle = io.handle.clone();
            let proxy = io.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::Acceptance(m));
            };
            acceptance::update(&mut ws.acceptance, msg, project_id, &client, &handle, emit);
            if is_accept_ok {
                ws.spawn_acceptance_count_refresh(io);
            }
        });
    }

    fn todo_dispatch_to_existing(&mut self, idx: usize, session_id: String) {
        let Some(project_id) = self.active_project_id else {
            return;
        };
        let text = self
            .active_workspace()
            .and_then(|ws| ws.todo.items().get(idx))
            .map(|item| item.text.clone());
        let Some(text) = text else {
            return;
        };
        self.with_focused_project(|ws, io| {
            ws.todo.close_dispatch_popup();
            ws.dispatch_todo_to_existing(io, &session_id, &text);
        });
        self.todo.record_dispatch(project_id, &text, session_id);
    }

    fn todo_dispatch_new(&mut self, idx: usize, launch: crate::workspace::PickerLaunch) {
        let text = self
            .active_workspace()
            .and_then(|ws| ws.todo.items().get(idx))
            .map(|item| item.text.clone());
        let Some(text) = text else {
            return;
        };
        self.with_focused_project(|ws, io| {
            ws.todo.close_dispatch_popup();
            if let Some(tab_id) = ws.spawn_new_tab(io, launch, Some(text.clone())) {
                ws.todo.insert_pending_dispatch(tab_id, text);
            }
        });
    }

    fn todo_message(&mut self, msg: todo::Message) {
        let Some(project_id) = self.active_project_id else {
            return;
        };
        // `self.todo`(App 级)和某个 `Workspace` 要同时可变借用,
        // `todo::update` 才能一次处理完两块状态——不能套用
        // `with_focused_project(|ws, _io| ..)` 那种单参数闭包(它只
        // 借出 `ws`,拿不到 `self.todo`)。改用 `loaded_workspace_mut`
        // 直接从 `self.projects` 借 `&mut Workspace`,跟 `&mut self.todo`
        // 是结构体的两个不同字段,互不冲突,Rust 借用检查器允许分别
        // 借用。
        let app_todo = &mut self.todo;
        let Some(ws) = loaded_workspace_mut(&mut self.projects, project_id) else {
            return;
        };
        let Some(project) = ws.project.as_ref() else {
            return;
        };
        let project_path = std::path::PathBuf::from(&project.path);
        todo::update(&mut ws.todo, app_todo, msg, project_id, &project_path);
    }

    fn browser_bookmarks_loaded(&mut self, pid: i64, bookmarks: Vec<BookmarkInfo>) {
        self.with_project(pid, move |ws, io| {
            let handle = io.handle.clone();
            let client = io.client.clone();
            let proxy = io.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::Browser(m));
            };
            browser::update(
                &mut ws.browser,
                browser::Message::BookmarksLoaded(pid, bookmarks),
                Some(pid),
                &client,
                &handle,
                emit,
            );
        });
    }

    fn browser_bookmarks_mutated(&mut self, pid: i64, res: Result<(), String>) {
        self.with_project(pid, move |ws, io| {
            let handle = io.handle.clone();
            let client = io.client.clone();
            let proxy = io.proxy.clone();
            let emit = move |m| {
                let _ = proxy.send_event(Message::Browser(m));
            };
            browser::update(
                &mut ws.browser,
                browser::Message::BookmarksMutated(pid, res),
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
            });
        }
    }

    fn term_input(&mut self, target: TermTarget, bytes: Vec<u8>) {
        // 终端不在屏上时丢弃按键(不报错、不写 PTY):否则用户在读
        // 对话审阅时敲的回车/方向键会静默提交给隐藏在后面的 agent
        // 会话(Fix round 2 #3)。
        let visible = match target {
            TermTarget::Shared => self.terminal_visible(),
            TermTarget::SshPanel => self.ssh_terminal_visible(), // Task 12 新增
        };
        if !visible {
            return;
        }
        self.with_focused_project(|ws, io| {
            match target {
                TermTarget::Shared => {
                    // 键入即回底 + 清选区：正在回看历史时一敲键盘，视口跳回
                    // 实时输出（常规终端语义），再把字节写给 daemon。
                    if let Some(tab) = ws.tabs.get_mut(ws.active) {
                        tab.model.scroll_to_bottom();
                        tab.model.selection_clear();
                    }
                    ws.send_input(io, bytes);
                }
                TermTarget::SshPanel => {
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

    fn term_paste(&mut self, target: TermTarget, text: String) {
        // 同 TermInput 的可见性闸门(Fix round 3):⌘V 粘贴走同一条
        // PTY 写入路径,粘贴内容若含换行还会在看不见的会话里直接
        // 执行,比单个按键更危险,必须同样拦截。
        let visible = match target {
            TermTarget::Shared => self.terminal_visible(),
            TermTarget::SshPanel => self.ssh_terminal_visible(),
        };
        if !visible {
            return;
        }
        self.with_focused_project(move |ws, io| {
            let bracketed = match target {
                TermTarget::Shared => ws.tabs.get(ws.active).map(|t| t.model.bracketed_paste()),
                TermTarget::SshPanel => ws
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
                TermTarget::Shared => {
                    if let Some(tab) = ws.tabs.get_mut(ws.active) {
                        tab.model.scroll_to_bottom();
                    }
                }
                TermTarget::SshPanel => {
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
                TermTarget::Shared => ws.send_input(io, bytes),
                TermTarget::SshPanel => ws.ssh_send_input(io, bytes),
            }
        });
    }

    fn select_tab(&mut self, idx: usize) {
        // 按下页签＝选中＋准备被拖走:选中仍是唯一的语义,但顺带记下
        // "这一页签正被按住",随后鼠标划过其它页签时 `on_move` 触发
        // `TabDragMove` 完成换位;松开时 main.rs `TabDragEnd` 收尾。
        self.with_focused_project(|ws, _io| {
            if idx < ws.tabs.len() {
                ws.active = idx;
            }
        });
        if let Some(ws) = self.active_workspace()
            && idx < ws.tabs.len()
        {
            self.tab_drag = Some(TabDrag {
                group: TabGroup::Terminal,
                source: idx,
            });
        }
    }

    fn pane_resized(&mut self, cols: u16, rows: u16) {
        if cols == 0 || rows == 0 || (cols, rows) == (self.cols, self.rows) {
            return;
        }
        self.cols = cols;
        self.rows = rows;
        let io = self.shell_io();
        // 终端网格是窗口级的:并行打开的每个项目各有一套终端 tab,
        // 但它们共用同一块终端 pane。只改当前项目的话,切回后台项目
        // 会看到一个停在旧网格、和 pane 对不上的画面,直到用户偶然
        // 再拖一次窗口才纠正——所以这里对所有已加载项目一起改
        // (`Stub` 还没有任何 tab,促成时自然按当时的 `io.cols/rows`)。
        for slot in self.projects.values_mut() {
            if let WorkspaceSlot::Loaded(ws) = slot {
                ws.resize_all(&io, cols, rows);
            }
        }
    }

    fn left_icon_select(&mut self, v: LeftView) {
        if self.left_view == v {
            // 点的是已选中(激活)的图标:应退回未选中并收起左面板区。
            // 但若右面板区也已经收起了,左就是最后一个还开着的 zone,
            // 不能关——保持展开、图标维持选中态(什么都不做)。
            if !self.right_collapsed {
                self.left_collapsed = !self.left_collapsed;
            }
        } else {
            self.left_view = v;
            self.left_collapsed = false;
        }
        // 切进 Git 提交图视图时,若缓存为空或不属于当前项目,同步跑
        // 一次 `gleisbau` 布局。失败/未打开项目都落成文案,交给
        // `git_log::view` 画出来,不 panic、不静默吞掉。
        if self.left_view == LeftView::GitLog {
            self.sync_git_log_to_active_project();
        }
        // Todo 面板：切入即从磁盘重读一次 `.dozer/todo.md`，保证切进来
        // 立刻是最新内容（轮询只负责"停留期间"的同步，切换本身不算）。
        if self.left_view == LeftView::Todo {
            self.with_focused_project(|ws, _io| {
                if let Some(project) = ws.project.as_ref() {
                    todo::reload_from_disk(&mut ws.todo, std::path::Path::new(&project.path));
                }
            });
        }
        // 数据库面板：切入即从磁盘重读一次 `.dozer/database.json`，
        // 保证切进来立刻是最新内容(同 Todo 面板的切换时语义)。
        if self.left_view == LeftView::Database {
            self.with_focused_project(|ws, _io| {
                if let Some(project) = ws.project.as_ref() {
                    database::reload_from_disk(
                        &mut ws.database,
                        std::path::Path::new(&project.path),
                    );
                }
            });
        }
        // SSH 面板：切入即从磁盘重读一次 `.dozer/ssh_hosts.json`，语义
        // 同 Todo 面板(切换本身触发重读,停留期间的同步靠别的机制)。
        if self.left_view == LeftView::Ssh {
            self.with_focused_project(|ws, _io| {
                if let Some(project) = ws.project.as_ref() {
                    ssh::reload_from_disk(&mut ws.ssh, std::path::Path::new(&project.path));
                }
            });
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

    fn right_icon_select(&mut self, v: RightView) {
        if self.right_view == v {
            // 同上,对称:右是最后开着的 zone 时不收起。
            if !self.left_collapsed {
                self.right_collapsed = !self.right_collapsed;
            }
        } else {
            self.right_view = v;
            self.right_collapsed = false;
            if v == RightView::Usage {
                self.with_focused_project(|ws, io| {
                    ws.usage.set_loading(true);
                    ws.spawn_usage_refresh(io);
                });
            } else if v == RightView::Acceptance {
                let tab_id = self
                    .active_workspace()
                    .and_then(|ws| ws.tabs.get(ws.active))
                    .map(|t| t.tab_id);
                if let Some(tab_id) = tab_id {
                    self.update(Message::Acceptance(acceptance::Message::Open(tab_id)));
                }
            }
        }
        // 同 LeftIconSelect(Fix round 2 #2)。
        self.maximized = None;
        self.on_shell_layout_changed();
    }

    fn top_bar_home(&mut self) {
        self.current_page = AppPage::Home;
        self.home_recents_loaded = false;
        self.home_left_view = homespace::HomeLeftView::default();
        self.home_right_view = homespace::HomeRightView::default();
        let projects: Vec<ProjectInfo> = self.recent_projects.iter().take(5).cloned().collect();
        let proxy = self.proxy.clone();
        self.handle.spawn(async move {
            let (files, convs) = tokio::task::spawn_blocking(move || load_home_recents(&projects))
                .await
                .unwrap_or_default();
            let _ = proxy.send_event(Message::HomeRecentsLoaded(files, convs));
        });
    }

    fn preview_open_path(&mut self, path: PathBuf) {
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
            // 新 tab 落在末尾，滚回最左让它可见（P1L T5）。
            ws.preview_tab_first = 0;
            ws.spawn_preview_state_save(io);
            ws.spawn_preview_context_push(io);
        });
    }

    fn preview_select_tab(&mut self, idx: usize) {
        let arming = self
            .active_workspace()
            .map(|ws| idx < ws.preview.tabs().len())
            .unwrap_or(false);
        self.with_focused_project(|ws, io| {
            ws.preview.select(idx);
            ws.spawn_preview_state_save(io);
            ws.spawn_preview_context_push(io);
        });
        if arming {
            self.tab_drag = Some(TabDrag {
                group: TabGroup::Preview,
                source: idx,
            });
        }
    }

    /// Project 面板右配对预览打开文件:写入 `ws.project_preview`(独立的
    /// `PreviewPane`),完全不碰 Files 预览的 `ws.preview`/`ws.files`。
    /// tab 是项目链接点开产生的会话期状态,不持久化、也不向 daemon 推上下文,
    /// 避免与 Files 预览那份持久化 `preview_state` 互相覆盖。
    fn project_preview_open_path(&mut self, path: PathBuf) {
        self.with_focused_project(|ws, _io| {
            if !path.is_file() {
                ws.project_preview_error = Some(format!("文件不存在或不可读: {}", path.display()));
                return;
            }
            ws.project_preview_error = None;
            ws.allowed_files
                .lock()
                .expect("allowed_files 锁")
                .insert(path.clone());
            ws.project_preview.open_path(path);
            // 新 tab 落在末尾,滚回最左让它可见(同 Files 预览)。
            ws.project_preview_tab_first = 0;
        });
    }

    fn project_preview_select_tab(&mut self, idx: usize) {
        let arming = self
            .active_workspace()
            .map(|ws| idx < ws.project_preview.tabs().len())
            .unwrap_or(false);
        self.with_focused_project(|ws, _io| {
            ws.project_preview.select(idx);
        });
        if arming {
            self.tab_drag = Some(TabDrag {
                group: TabGroup::ProjectPreview,
                source: idx,
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
            if let Some(tab) = ws.tab_by_id_mut(tab_id) {
                tab.agent_state = state;
                tab.agent = agent;
                if let Some(tp) = transcript_path {
                    tab.transcript_path = Some(tp);
                }
                tracing::info!(tab_id, ?state, "agent 状态变更");
                if state == AgentState::TurnEnded {
                    // git 检测不许在 UI 线程跑：丢 tokio,结果经 proxy 回来
                    let cwd = effective_project_repo(active_repo.as_deref(), &tab.effective_cwd());
                    let last_turn = tab.last_turn_head.clone();
                    let proxy = io.proxy.clone();
                    tracing::info!(tab_id, cwd = %cwd.display(), "回合结束,开始交付检测");
                    io.handle.spawn(async move {
                        let pending = tokio::task::spawn_blocking(move || {
                            let Some(repo) = delivery::repo_root(&cwd) else {
                                tracing::info!(cwd = %cwd.display(), "非 git 仓库,不参与闭环");
                                return None;
                            };
                            let dirty = delivery::is_dirty(&repo);
                            let head = delivery::head_commit(&repo);
                            let accepted = delivery::last_accepted(&repo).map(|(_, c)| c);
                            let pending = delivery::delivery_pending(
                                dirty,
                                head.as_deref(),
                                accepted.as_deref(),
                                last_turn.as_deref(),
                            );
                            tracing::info!(
                                repo = %repo.display(),
                                dirty,
                                has_accepted = accepted.is_some(),
                                pending,
                                "交付检测完成"
                            );
                            Some(pending)
                        })
                        .await
                        .ok()
                        .flatten();
                        if let Some(pending) = pending {
                            let _ = proxy
                                .send_event(Message::DeliveryChecked(project_id, tab_id, pending));
                        }
                    });
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
                ws.spawn_review_load(io, ReviewSource::Session(tab_id), path, tab_agent);
            }
        });
    }

    fn delivery_checked(&mut self, project_id: ProjectId, tab_id: usize, pending: bool) {
        self.with_project(project_id, |ws, io| {
            let active_id = ws.tabs.get(ws.active).map(|t| t.tab_id);
            let is_active = active_id == Some(tab_id);
            let active_repo = ws.project.as_ref().map(|p| PathBuf::from(&p.path));
            tracing::info!(
                tab_id,
                pending,
                is_active,
                "交付检测结果落地(pending 写入该 tab;仅当前激活 tab 显示横幅)"
            );
            if let Some(tab) = ws.tab_by_id_mut(tab_id) {
                tab.delivery_pending = pending;
                // 记录本回合 HEAD 供下回合比对（同步读一次可容忍:仅 rev-parse）
                let cwd = effective_project_repo(active_repo.as_deref(), &tab.effective_cwd());
                if let Some(repo) = delivery::repo_root(&cwd) {
                    tab.last_turn_head = delivery::head_commit(&repo);
                }
            }
            // 回合结束后刷新项目 git 状态,文件树装饰随之更新（P1h）。
            if let Some(project) = &ws.project {
                let repo_path = PathBuf::from(&project.path);
                spawn_project_git_refresh(project_id, repo_path.clone(), io);
                spawn_disk_usage_refresh(project_id, repo_path, io);
            }
            // 回合结束后刷新对话列表(transcript 增长/新增；P1j)。
            ws.spawn_conversations_refresh(io);
        });
    }

    fn conversation_open(&mut self, path: PathBuf) {
        self.with_focused_project(move |ws, io| {
            // 若点开的是某活会话的当前对话 → Session 源(回合结束刷新);否则 File 快照。
            let path_s = path.to_string_lossy().into_owned();
            let session_tab = ws
                .tabs
                .iter()
                .find(|t| t.transcript_path.as_deref() == Some(path_s.as_str()));
            // 活会话 tab 的 agent 若还是 Unknown（hook 事件还没到，或
            // 老装的 hook 一直上报 Unknown）不该盖掉从对话历史扫描
            // 位置推断出的已知 agent——优先取“已知”的那个。
            let agent = session_tab
                .map(|t| t.agent)
                .filter(|a| *a != AgentKind::Unknown)
                .or_else(|| {
                    ws.conversations
                        .iter()
                        .find(|c| c.path == path)
                        .map(|c| c.agent)
                })
                .unwrap_or_default();
            let source = session_tab
                .map(|t| ReviewSource::Session(t.tab_id))
                .unwrap_or_else(|| ReviewSource::File(path.clone()));
            ws.review = Some(ReviewView {
                source: source.clone(),
                entries: Vec::new(),
                error: None,
                expanded: std::collections::HashSet::new(),
            });
            ws.spawn_review_load(io, source, path_s, agent);
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

    fn git_log_load_more(&mut self) {
        let Some(path) = self
            .active_workspace()
            .and_then(|ws| ws.active_project_path())
        else {
            return;
        };
        let next = self.git_log.next_load_more_count();
        // `request_refresh` 内部会把 `selected` 清空,所以必须在调用它之前
        // 先读出来,落地新快照后(`update()` 处理 `SnapshotLoaded` 那支)才能
        // 据此还原选中态——镜像现有 `Message::GitLogLoadMore` 分支"先记
        // selected,刷新,再把 restore_after_load 设回去"的顺序。
        let selected = self.git_log.selected();
        let handle = self.handle.clone();
        let proxy = self.proxy.clone();
        let emit = move |m| {
            let _ = proxy.send_event(Message::GitLog(m));
        };
        git_log::request_refresh(&mut self.git_log, path, next, &handle, emit);
        self.git_log.set_restore_after_load(selected);
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

    /// 文件预览 tab 右键菜单浮层:含"编辑"(仅可编辑文本文件)与"关闭"两项。
    /// 定位坐标由 `PreviewTabContextMenu` 打开时记录,风格与文件树右键菜单
    /// 一致(`files::context_menu_popup`/`menu_item`)。
    fn preview_tab_context_menu_popup<'a>(
        &self,
    ) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
        let menu = match &self.preview_tab_menu {
            Some(m) => m,
            None => return column![].into(),
        };
        let mut items: Vec<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>> =
            Vec::new();
        // 仅可编辑文本文件显示"编辑"(见 `is_editable_extension`)。
        if menu.editable {
            items.push(Self::preview_menu_item(
                Some(icons::IconKind::Rename),
                "编辑",
                Message::PreviewEditOpen(menu.idx),
            ));
        }
        items.push(Self::preview_menu_item(
            None,
            "关闭",
            Message::PreviewCloseTab(menu.idx),
        ));

        let region = theme::region::context_menu();
        let list = container(column(items).spacing(region.gap))
            .padding(region.padding)
            .style(move |_t: &iced_widget::Theme| container::Style {
                background: region.background.map(Into::into),
                border: region.border.unwrap_or_default(),
                ..container::Style::default()
            });
        // 文件预览的 webview 只铺在 tab 栏**下方**的内容区(这就是 tab 栏本身
        // 始终以 iced 显示、不被 webview 盖住的原因)。右键菜单若向下弹会压到
        // webview、被原生子视图挡住;故改为**向上弹**——以光标为底边、向上展开,
        // 整片落在 tab 栏上方的 iced 区域,既不被 webview 遮、也不用在菜单期间
        // 藏掉预览内容(那个方案会让预览整片消失,体验更差)。
        let window_h = self.window_size.1;
        let bottom = (window_h - menu.y).max(0.0);
        container(list)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_y(iced_widget::core::alignment::Vertical::Bottom)
            .padding(Padding {
                top: 0.0,
                left: menu.x,
                right: 0.0,
                bottom,
            })
            .into()
    }

    /// Project 面板右配对预览 tab 右键菜单浮层,语义同
    /// `preview_tab_context_menu_popup`。定位坐标同样复用 `files.last_right_click`
    /// (main.rs 任意右键都会先写入),"编辑"项落 `ProjectPreviewEditOpen`(编辑
    /// project 预览的 tab)、"关闭"落 `ProjectPreviewCloseTab`。
    fn project_preview_tab_context_menu_popup<'a>(
        &self,
    ) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
        let menu = match &self.project_preview_tab_menu {
            Some(m) => m,
            None => return column![].into(),
        };
        let mut items: Vec<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>> =
            Vec::new();
        if menu.editable {
            items.push(Self::preview_menu_item(
                Some(icons::IconKind::Rename),
                "编辑",
                Message::ProjectPreviewEditOpen(menu.idx),
            ));
        }
        items.push(Self::preview_menu_item(
            None,
            "关闭",
            Message::ProjectPreviewCloseTab(menu.idx),
        ));

        let region = theme::region::context_menu();
        let list = container(column(items).spacing(region.gap))
            .padding(region.padding)
            .style(move |_t: &iced_widget::Theme| container::Style {
                background: region.background.map(Into::into),
                border: region.border.unwrap_or_default(),
                ..container::Style::default()
            });
        let window_h = self.window_size.1;
        let bottom = (window_h - menu.y).max(0.0);
        container(list)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_y(iced_widget::core::alignment::Vertical::Bottom)
            .padding(Padding {
                top: 0.0,
                left: menu.x,
                right: 0.0,
                bottom,
            })
            .into()
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
            vec![Self::preview_menu_item(
                Some(icons::IconKind::Trash),
                "删除",
                Message::Project(project::Message::LinkRemove {
                    target: menu.target,
                    index: menu.index,
                }),
            )];

        let region = theme::region::context_menu();
        let list = container(column(items).spacing(region.gap))
            .width(Length::Shrink)
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

    /// 预览 tab 右键菜单单项(图标可选 + 文字按钮)。hover/pressed 切到
    /// `TAB_HOVER` 背景,与文件树右键菜单 `menu_item` 同款。
    fn preview_menu_item<'a>(
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

    pub fn view(
        &self,
    ) -> iced_widget::core::Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
        // 顶栏先画:它是外壳的一部分(项目页签行 + "＋"就在上面),一个项目都
        // 没打开时更要画得出来——否则用户没有任何入口去打开第一个项目。
        let top = top_bar(self);
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
                    .size(theme::font::subtitle())
                    .color(theme::color::DIM)
            ]
            .spacing(8);
            if let Some(err) = &self.daemon_error {
                hint_col = hint_col.push(
                    text(format!("⚠ {err}"))
                        .size(theme::font::body())
                        .color(theme::color::RED),
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
            left_icon_rail(self),
            column![
                row![
                    left_panel_area(self, ws, false),
                    divider_bar(Divider::LeftRight, theme::color::BG, theme::color::BG),
                    right_panel_area(self, ws, false),
                ]
                .height(Length::Fill),
                footbar::view(&self.footbar).map(Message::Footbar),
            ]
            .width(Length::Fill),
            right_icon_rail(self),
        ];
        let base = column![top, body];

        let popped = if ws.search_popup_open() {
            // 文件树右键"搜索"弹窗:窗口级浮层。遮罩"点点即关"由
            // `search_modal` 内部自己处理(整窗 `SCRIM` 做成可点击目标,卡片
            // 是兄弟元素盖在上面),这里只需把弹窗叠在 `base` 之上。
            let project_root = ws.project.as_ref().map(|p| std::path::Path::new(&p.path));
            stack![
                base,
                search::search_modal(&ws.search, project_root).map(Message::Search)
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else if ws.edit_session.is_some() {
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::PreviewEditCloseRequest);
            let confirm_discard = ws.edit_session.as_ref().is_some_and(|s| s.confirm_discard);
            if confirm_discard {
                stack![base, dismiss, edit_modal(ws), edit_discard_confirm_popup()]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into()
            } else {
                stack![base, dismiss, edit_modal(ws)]
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into()
            }
        } else if ws.files.tree_delete_confirm_is_some() {
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::Files(files::Message::DeleteCancel));
            stack![
                base,
                dismiss,
                files::delete_confirm_popup(&ws.files).map(Message::Files)
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
        } else if self.preview_tab_menu.is_some() {
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::PreviewTabContextMenuClose);
            stack![base, dismiss, self.preview_tab_context_menu_popup()]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else if self.project_preview_tab_menu.is_some() {
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::ProjectPreviewTabContextMenuClose);
            stack![base, dismiss, self.project_preview_tab_context_menu_popup()]
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

        if let Some(which) = self.maximized {
            stack![popped, maximize_overlay(self, ws, which)]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else {
            popped
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
fn top_bar_font() -> Font {
    Font {
        weight: Weight::Medium,
        ..Font::default()
    }
}

/// 顶栏 Home 按钮(D1)：Lucide house(`IconKind::Home`) + "Dozer"文字,视觉、
/// 高度、选中态样式与右侧项目页签(`project_tab_item`)完全一致——同一份
/// `tab_h`、同一套 hover 胶囊/选中态实底+底部强调线,只是没有状态点和关闭
/// 按钮。恒在最左、不参与 `project_tabs_row` 的拥挤收窄——与当前项目页签
/// 行"＋"按钮同款的"固定位不参与收窄"处理。点它进首页(`AppPage::Home`)。
///
/// `active` 由调用方传入 `current_page == AppPage::Home`,与项目页签的
/// `active_project_id == Some(id)` 是两套独立状态,靠调用方各自互斥地计算
/// (见 `top_bar`/`project_tabs_row`),不然会出现两边同时"选中"的视觉冲突。
fn dozer_home_tab<'a>(
    active: bool,
    title_hover_t: f32,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    // 与 `project_tab_item` 用同一份高度公式,保证两者视觉同高、顶边对齐。
    let sq = crate::theme::icon_size::rail() + 14.0;
    let tab_h = (theme::geometry::top_bar_height() + sq) / 2.0;

    // 标题(图标 + "Dozer" 文字)颜色:选中态恒为金 `#F2D94E`(甲方动作专属色,
    // 与项目页签一致);未选中态静止 DIM,hover 时随 `title_hover_t` 平滑过渡
    // 到金(同一套悬停动画,见 `HoverId::HomeTab`)。
    let title_color = if active {
        theme::color::GOLD
    } else {
        theme::color::mix(theme::color::DIM, theme::color::GOLD, title_hover_t)
    };

    // `height(Fill)` + `align_y(Center)` 缺一不可:与 `project_tab_item` 同一处
    // iced 按钮布局 quirk——`button` 只加 padding、不回收多余竖向空间,内层
    // `row` 的 `align_y(Center)` 因此形同虚设,必须让这层 `container` 撑满按钮
    // 内容区、自己吃掉那截空间才能真正居中,否则 icon + "Dozer" 贴顶。这里
    // `width(Fill)` 与该项目页签同款,内层 icon / 文字才会稳稳落在按钮垂直中线。
    let label = container(
        row![
            icons::view(
                icons::IconKind::Home,
                crate::theme::icon_size::home(),
                title_color
            ),
            text("Dozer")
                .font(top_bar_font())
                .size(theme::font::body())
                .color(title_color),
        ]
        .spacing(6)
        .align_y(iced_widget::core::Alignment::Center),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .align_y(iced_widget::core::Alignment::Center);

    // 未选中态 hover 时画一条与项目页签同款的胶囊背景(`TAB_HOVER`);选中态
    // 不参与 hover 提亮,同 `project_tab_item::select`。`width(Shrink)` 让这枚
    // 品牌页签只包住 icon + "Dozer" 本身,不抢顶栏横向空间(项目页签是
    // `Fill` 因为它要均分页签行宽度)。
    let select = button(label)
        .on_press(Message::TopBarHome)
        .width(Length::Shrink)
        .height(Length::Fixed(tab_h))
        .padding([0, 14])
        .style(move |_t: &iced_widget::Theme, s| {
            let mut st = button::Style {
                background: None,
                text_color: title_color,
                ..button::Style::default()
            };
            if !active && let button::Status::Hovered = s {
                st.background = Some(theme::color::TAB_HOVER.into());
                st.border = Border {
                    radius: 8.0.into(),
                    ..Border::default()
                };
            }
            st
        });
    // 标题文字的 hover 变色走 `MouseArea` + `HoverId::HomeTab`(与项目页签的
    // `ProjectTabItem` 同款叠层:`MouseArea` 只抓 enter/exit 事件,按下仍由
    // 底层 `select` 按钮处理),驱动 `title_color` 从 DIM 平滑过渡到 GOLD。
    let select = MouseArea::new(select)
        .on_enter(Message::Hover(HoverId::HomeTab, true))
        .on_exit(Message::Hover(HoverId::HomeTab, false));

    // 选中态:实底背景(左上/右上圆角) + 底部 1px 强调线,与 `project_tab_item`
    // 同一手法——`stack!` 叠加而非 `column!`,避免强调线瓜分 `select` 的
    // `Fixed` 高度导致文字居中基准跟项目页签错位(见该函数同一处注释)。
    let inner: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> = if active {
        container(stack![
            select,
            container(
                container(iced_widget::space::Space::new())
                    .width(Length::Fill)
                    .height(Length::Fixed(1.0))
                    .style(|_t: &iced_widget::Theme| container::Style {
                        background: Some(theme::color::TAB_ACTIVE_BORDER.into()),
                        ..container::Style::default()
                    }),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .align_y(iced_widget::core::alignment::Vertical::Bottom),
        ])
        .into()
    } else {
        select.into()
    };

    let tab_box = container(inner)
        .height(Length::Fixed(tab_h))
        .clip(true)
        .style(move |_t: &iced_widget::Theme| {
            if active {
                container::Style {
                    background: Some(theme::color::TAB_ACTIVE_BG.into()),
                    border: Border {
                        radius: Radius {
                            top_left: 8.0,
                            top_right: 8.0,
                            ..Radius::default()
                        },
                        ..Border::default()
                    },
                    ..container::Style::default()
                }
            } else {
                container::Style::default()
            }
        });

    // 外层贴底对齐,与项目页签在 `project_tabs_row` 里的贴底方式一致
    // (那边靠 `responsive` 闭包最外层 `container(...).height(Fill).align_y(End)`,
    // 见该函数注释),这样两者的顶边才能真正对齐,而不是像旧版那样一个居中
    // 一个贴底、靠公式凑巧对齐。
    container(tab_box)
        .height(Length::Fill)
        .align_y(iced_widget::core::alignment::Vertical::Bottom)
        .into()
}

fn top_bar(app: &App) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    // Dozer 字标做成按钮:house 图标(`IconKind::Home`) + "Dozer"文字,
    // 点它进首页(`AppPage::Home`)。Dozer 页签:视觉与右侧项目页签一致,
    // 恒在最左、不参与拥挤收窄(D1)。
    let title = dozer_home_tab(
        app.current_page == AppPage::Home,
        app.hover_progress(HoverId::HomeTab),
    );

    // 页签行占满标题与右侧之间的全部空间。裁剪与翻页在 `project_tabs_row`
    // 内部做(只裁页签本身,箭头与"＋"钉在裁剪区外),这里**不能**再套一层
    // `clip`——那会把"＋"和箭头一起裁掉,正是要修的问题。
    // 贴底对齐在 `project_tabs_row` 内部(`responsive` 闭包里)完成,这里
    // 套 `align_y` 对它不起作用,见该函数内注释。
    let tabs = container(project_tabs_row(app)).width(Length::Fill);

    let mut right = row![].spacing(10);
    // 设计稿"btn settings"外框 padding-left 6 / padding-y 4(hit-box 留白,
    // 图标本身仍是 16x16)。目前尚未接入设置面板,先只还原视觉,不加
    // on_press——没有对应 Message 变体可派发。
    // 图标颜色:SVG 颜色构建时定死,hover 态平滑过渡到金(见 `HoverId`/
    // `App::hover_progress`——与光标闪烁同款自驱 redraw 动画)。
    let settings_color = theme::color::mix(
        theme::color::DIM,
        theme::color::GOLD,
        app.hover_progress(HoverId::Topbar(TopbarButton::Settings)),
    );
    right = right.push(
        MouseArea::new(
            container(icons::view(
                icons::IconKind::Settings,
                crate::theme::icon_size::rail(),
                settings_color,
            ))
            .padding(Padding {
                top: 4.0,
                right: 0.0,
                bottom: 4.0,
                left: 6.0,
            }),
        )
        .on_enter(Message::Hover(
            HoverId::Topbar(TopbarButton::Settings),
            true,
        ))
        .on_exit(Message::Hover(
            HoverId::Topbar(TopbarButton::Settings),
            false,
        ))
        .interaction(mouse::Interaction::Pointer),
    );

    let region = theme::region::top_bar();
    let bar = row![title, tabs, right]
        .spacing(region.gap)
        .padding(region.padding)
        .height(Length::Fixed(theme::geometry::top_bar_height()))
        .align_y(iced_widget::core::Alignment::Center);

    // 双击顶栏空白处缩放窗口(原生标题栏没了之后,系统"双击标题栏缩放"
    // 手势只在它认为仍是标题栏的那一条区域生效,顶栏其余空白靠这层背景
    // MouseArea 手动补上)。放在 `bar` 下面这一层——iced 的点击命中是
    // 子先父后/上先下后,`bar` 里真正的按钮(页签/加号/箭头)会先吃掉
    // 落在它们身上的点击,双击事件只有落在没有任何控件的空白处才会穿透
    // 到这层背景,不会误吞正常的页签交互。
    let background = MouseArea::new(
        container(iced_widget::Space::new())
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .on_double_click(Message::TopBarDoubleClick);

    container(stack![background, bar])
        .width(Length::Fill)
        .height(Length::Fixed(theme::geometry::top_bar_height()))
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: region.border.unwrap_or_default(),
            ..container::Style::default()
        })
        .into()
}

/// 一个项目页签渲染需要的三样东西:名字、状态点、id。抽出来是为了让宽度
/// 估算(`tab_window` 要各页签宽)与渲染读同一份数据,不各遍历一次
/// `project_order` 走岔。
fn project_tab_entries(app: &App) -> Vec<ProjectTabEntry> {
    app.project_order
        .iter()
        // `project_order` 与 `projects` 理论上恒一致;真出现孤儿 id 时跳过
        // 渲染而不是 panic——顺序表是要写盘的,不值得为一条脏数据崩掉 GUI。
        .filter_map(|id| app.projects.get(id).map(|slot| (*id, slot)))
        .map(|(id, slot)| match slot {
            // `Stub` 没有会话列表,但启动恢复时按 daemon 的会话快照算过一次
            // 状态点(见 [`stub_activity`]),用那份"重启时已知"的结果。
            WorkspaceSlot::Stub { info, activity } => ProjectTabEntry {
                id,
                name: info.name.clone(),
                dot: activity.map(agent_state_dot),
            },
            WorkspaceSlot::Loaded(ws) => ProjectTabEntry {
                id,
                name: ws
                    .project
                    .as_ref()
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| "加载中…".to_string()),
                dot: project_tab_dot(ws),
            },
        })
        .collect()
}

/// 见 [`project_tab_entries`]。
struct ProjectTabEntry {
    id: i64,
    name: String,
    /// `(颜色, 是否闪烁)`;`None` = 不画状态点。
    dot: Option<(Color, bool)>,
}

/// 顶栏项目页签行:固定默认宽 + 拥挤时均分收窄的页签 + 紧跟最后一片页签之后的"＋"。
///
/// 每片页签的宽度按如下规则算(`project_tab_max_width()` 即"默认/合适宽"):
/// 页签少、每片都能容下默认宽时,统一用默认宽(左对齐,右侧留白,不撑爆);
/// 页签多到塞不下默认宽时,按可用宽均分,每片窄于默认宽(随实际拥挤程度收窄)。
/// 这样少数页签始终是固定的"默认宽度",只有真挤了才缩。可用宽在布局期由
///
/// `responsive` 实时拿到(不引入窗口尺寸依赖),再扣掉"＋"按钮与各处 gap。
fn project_tabs_row(
    app: &App,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    // `active_project_id` 记的是"最后聚焦的项目",跟 Dozer Home 页签是否被
    // 选中的 `current_page` 是两套独立状态(见 `dozer_home_tab` 注释)——切去
    // Home 时 `active_project_id` 不会被清空(方便切回来时记得原项目),所以
    // 页签的"选中"视觉要额外拿 `current_page` 挡一道,否则 Home 和某个项目
    // 页签会同时高亮。
    let active_project_id = (app.current_page == AppPage::Workspace)
        .then_some(app.active_project_id)
        .flatten();
    let blink_on = app.blink_on;
    let entries = project_tab_entries(app);
    let n = entries.len();

    // `responsive` 在每轮布局把页签区可用宽交给闭包,闭包据此算每片宽。
    responsive(move |size| {
        let gap = theme::geometry::project_tab_gap();
        let default_w = theme::geometry::project_tab_max_width();
        // 预留"＋"按钮与其紧跟最后一片页签的 gap(页签内部还有 n-1 道 gap),
        // 避免页签在拥挤时压到"＋"上。
        let reserved = theme::geometry::project_tab_add_button_width() + (n as f32 + 1.0) * gap;
        let avail = (size.width - reserved).max(0.0);
        // 每片目标宽:少页签用默认宽(固定);多到塞不下默认宽才均分收窄。
        let per_tab = if n == 0 {
            default_w
        } else {
            let fit = avail / n as f32;
            if fit >= default_w { default_w } else { fit }
        };
        let per_tab = per_tab.max(0.0);

        let mut tabs = row![]
            .spacing(gap)
            .align_y(iced_widget::core::Alignment::Center)
            .width(Length::Shrink); // 固定宽,不撑满;右侧留白把"＋"顶到最右
        // 分割竖线高度:顶栏高的约 45%,在行内 `align_y(Center)` 自然垂直居中。
        let sep_h = theme::geometry::top_bar_height() * 0.45;
        for (i, entry) in entries.iter().enumerate() {
            let active = active_project_id == Some(entry.id);
            let close_hover_t = app.hover_progress(HoverId::ProjectTabClose(entry.id));
            let title_hover_t = app.hover_progress(HoverId::ProjectTabItem(entry.id));
            let item = project_tab_item(
                entry.id,
                entry.name.clone(),
                entry.dot,
                active,
                blink_on,
                close_hover_t,
                title_hover_t,
            );
            // 固定宽:少页签时为默认宽,挤时为均分窄宽(Chrome 式收窄)。
            let cell = container(item).width(Length::Fixed(per_tab));
            // 拖拽换位:按住页签(选中处理已把 `tab_drag` 置位)后光标扫过哪个
            // 页签,这个 `on_move` 就按它发 `TabDragMove`,完成换位。
            let armed = app.dragging_group(TabGroup::Project);
            let mut surface = MouseArea::new(cell).on_move(move |_| Message::TabDragMove {
                group: TabGroup::Project,
                index: i,
            });
            if armed {
                surface = surface.interaction(mouse::Interaction::Grabbing);
            }
            tabs = tabs.push(surface);
            // 仅当"当前"与"下一个"页签都未选中时,二者之间插一条小竖线做
            // 分割;只要相邻任意一侧是选中态,那一侧就不画(选中页签左右都
            // 干净,既不被竖线打断,也把"当前页签"在视觉上独立出来)。
            if i + 1 < n {
                let next_active = active_project_id == Some(entries[i + 1].id);
                if !active && !next_active {
                    tabs = tabs.push(
                        container(iced_widget::space::Space::new())
                            .width(Length::Fixed(1.0))
                            .height(Length::Fixed(sep_h))
                            .style(|_t: &iced_widget::Theme| container::Style {
                                background: Some(theme::color::BORDER.into()),
                                ..container::Style::default()
                            }),
                    );
                }
            }
        }

        // 图标颜色:SVG 构建时定死、不吃 `button::Status`,hover 态平滑过渡到
        // 金(见 `HoverId`/`App::hover_progress`)。
        let add_color = theme::color::mix(
            theme::color::DIM,
            theme::color::GOLD,
            app.hover_progress(HoverId::Topbar(TopbarButton::AddProject)),
        );
        let add = MouseArea::new(
            button(icons::view(
                icons::IconKind::SquarePlus,
                crate::theme::icon_size::row(),
                add_color,
            ))
            .on_press(Message::ProjectTabPickFolder)
            .padding([6, 8])
            .style(move |_t: &iced_widget::Theme, _s| button::Style {
                background: None,
                text_color: add_color,
                ..button::Style::default()
            }),
        )
        .on_enter(Message::Hover(
            HoverId::Topbar(TopbarButton::AddProject),
            true,
        ))
        .on_exit(Message::Hover(
            HoverId::Topbar(TopbarButton::AddProject),
            false,
        ));

        // 页签(固定宽,左对齐) + "＋"紧邻最后一片页签之后(不再用弹性留白把
        // 它顶到最右——它隶属于页签区,跟在最后一片页签后面,像浏览器新建
        // 页签的 ＋)。
        // 贴底必须在这里(闭包*内部*)包一层 `Length::Fill` + `align_y(End)`
        // 才生效——`responsive` 自身默认已是 Fill×Fill,闭包返回的内容在
        // `Responsive::layout` 里直接贴 (0,0) 摆放,外层 `top_bar()` 包多少层
        // `container(...).align_y(..)` 都摸不到它,曾经这样试过没用。
        container(
            row![tabs, add]
                .spacing(gap)
                .align_y(iced_widget::core::Alignment::Center),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .align_y(iced_widget::core::Alignment::End)
        .into()
    })
    .into()
}

/// 单个项目页签:状态点(可选)+ 项目名的切换按钮 + 关闭按钮。结构与终端
/// `tab_item` 一致(两个平级按钮包在一个 container 里,不做按钮套按钮)。
fn project_tab_item<'a>(
    id: i64,
    name: String,
    dot: Option<(Color, bool)>,
    active: bool,
    blink_on: bool,
    close_hover_t: f32,
    title_hover_t: f32,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    // 页签背景圆角半径参考 Dozer 按钮(圆角正方形)的边长 `sq`,但实际背景高
    // 用更高的 `tab_h`——页签贴底(见 `project_tabs_row` 的 `align_y(End)`)、
    // 底部留白必须是 0,可 Dozer 按钮在顶栏里是居中的,顶部留白
    // `(top_bar_height-sq)/2` 不为 0;要让页签顶边跟 Dozer 按钮背景顶边对齐,
    // 页签背景就不能也用 `sq` 这个高度贴底(那样顶边会比 Dozer 的更低),
    // 必须把高度补到 `(top_bar_height+sq)/2`,贴底后顶部留白才恰好等于
    // Dozer 按钮那份 `(top_bar_height-sq)/2`。
    let sq = crate::theme::icon_size::rail() + 14.0;
    let tab_h = (theme::geometry::top_bar_height() + sq) / 2.0;
    // 关闭按钮用与顶栏其它图标按钮(tab 箭头 / 最大化)同尺寸的方形命中区。
    let close_sz = crate::theme::geometry::tab_button_size();
    // 组合 hover:鼠标悬停标题或关闭按钮任一,都应让胶囊背景浮现、× 显形。
    // 不能只依赖 select 按钮的 `button::Status::Hovered`——× 叠在 select 之上,
    // 悬停 × 时底层 select 拿不到 `Hovered`,胶囊会凭空消失。
    let hover = title_hover_t.max(close_hover_t).clamp(0.0, 1.0);
    let hovered = hover > 0.001;
    let mut label = row![]
        .spacing(4)
        .align_y(iced_widget::core::Alignment::Center);
    if let Some((color, blinking)) = dot {
        // 工作中且处于暗相位:点点压到近乎透明,与终端 tab 同一套"呼吸"。
        let color = if blinking && !blink_on {
            Color { a: 0.15, ..color }
        } else {
            color
        };
        label = label.push(text("●").size(theme::font::caption_sm()).color(color));
    }
    label = label.push(
        text(name)
            .font(top_bar_font())
            .size(theme::font::body())
            .color(if active {
                // 选中态标题恒为金 `#F2D94E`(甲方动作专属色)。
                theme::color::GOLD
            } else {
                // 未选中态:静止 DIM,hover 时平滑过渡到金(见 `ProjectTabItem`)。
                theme::color::mix(theme::color::DIM, theme::color::GOLD, title_hover_t)
            }),
    );
    // 标签行撑满并裁剪:页签被 `FillPortion` 压窄时长名在此截断(Chrome 式
    // 无限收窄),不会把关闭按钮挤出去。
    // `height(Fill)` + `align_y(Center)` 缺一不可:`button` 的布局只加
    // padding、不回收多余竖向空间(iced_widget::button::layout 用
    // `layout::padded`,内容按 padding 定位后剩余空间原样留在下方),内层
    // `row` 的 `align_y(Center)` 因此形同虚设——必须让这层 `container` 撑满
    // 按钮内容区、自己吃掉那截空间才能真正居中,否则页签文字贴顶,与
    // `main.rs::center_traffic_lights` 摆在顶栏正中的交通灯对不齐。
    let label = container(label.align_y(iced_widget::core::Alignment::Center))
        .width(Length::Fill)
        .height(Length::Fill)
        .align_y(iced_widget::core::Alignment::Center)
        // 右侧留白给叠在页签之上的关闭按钮:长名在此截断,不会跑到 × 底下。
        // 左侧留白是这里单独加的,不是靠下面 `tab_row` 的外层 padding——
        // `capsule`(悬停胶囊背景)和这层 `label` 是 `stack!` 里的平级层,共用
        // `tab_row` 那份外层 padding 定的同一个起点,只调外层 padding 只会让
        // 胶囊和文字**一起**往右挪,两者间距不变;点点因此贴着胶囊圆角左缘
        // (验收反馈截图)。真正拉开点点与胶囊边缘间距,得单独加在 `label`
        // 自己的 padding 上。
        .padding(Padding {
            right: close_sz + 4.0,
            left: 6.0,
            ..Padding::ZERO
        })
        .clip(true);

    // 选中/关闭的接线逻辑收在 `tabs::tab_core`(2026-08-12 抽取)——mousedown
    // 即选中+备拖、关闭按钮仅悬停时可点这两条规则只在一处维护。`hovered`
    // 已经在上方算好(`let hovered = hover > 0.001;`),就是原来关闭按钮挂
    // `on_press` 的判定条件,直接复用。宽高原来靠 `MouseArea` 里的 `container`
    // 撑(Fill + Fixed(tab_h)),`MouseArea` 自身不认宽高,照抄内容尺寸。
    let close_base = theme::color::mix(theme::color::DIM, theme::color::GOLD, close_hover_t);
    let close_color = Color {
        a: hover,
        ..close_base
    };
    let (select, close) = tabs::tab_core(
        container(label)
            .width(Length::Fill)
            .height(Length::Fixed(tab_h))
            .into(),
        close_sz,
        close_color,
        hovered,
        Message::ProjectTabSwitch(id),
        Message::ProjectTabClose(id),
        move |hovered| Message::Hover(HoverId::ProjectTabItem(id), hovered),
        move |hovered| Message::Hover(HoverId::ProjectTabClose(id), hovered),
    );

    // 页签主体(select)为底层、关闭按钮为上层叠在其右:关闭按钮视觉上落在
    // 页签背景里,而非独立的相邻按钮。两层都 `Fill` 撑满整条顶栏高,select
    // 用容器垂直居中、close 用容器靠右居中;横向内缩 14。
    let select_layer = container(select)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_y(iced_widget::core::alignment::Vertical::Center);
    let close_layer = container(close)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(iced_widget::core::alignment::Horizontal::Right)
        .align_y(iced_widget::core::alignment::Vertical::Center);
    // 悬停胶囊:与 select 按钮同高同圆角、铺满整片页签(含右缘 × 区),由组合
    // hover 进度驱动透明度——只在悬停页签时浮现,且 × 落在其内部。选中态已有
    // 实底背景(TAB_ACTIVE_BG),不再叠胶囊。
    let capsule = container(iced_widget::space::Space::new())
        .width(Length::Fill)
        .height(Length::Fixed(tab_h))
        .align_y(iced_widget::core::alignment::Vertical::Center)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: if !active && hover > 0.0 {
                Some(
                    Color {
                        a: hover,
                        ..theme::color::TAB_HOVER
                    }
                    .into(),
                )
            } else {
                None
            },
            border: if !active && hover > 0.0 {
                Border {
                    radius: 8.0.into(),
                    ..Border::default()
                }
            } else {
                Border::default()
            },
            ..container::Style::default()
        });
    let capsule_layer = container(capsule)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_y(iced_widget::core::alignment::Vertical::Center);
    let tab_row = container(stack![capsule_layer, select_layer, close_layer])
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(Padding {
            top: 0.0,
            right: 8.0,
            bottom: 0.0,
            left: 12.0,
        });

    // 激活态:实底背景(左上/右上圆角) + 底部 1px 强调线
    // (`#dcc9a3` = `TAB_ACTIVE_BORDER`),不要外边框;未激活态:无背景、无边框
    // (仅 hover 时画胶囊,见上)。强调线用 `stack!` 叠在 `tab_row` 之上(贴底
    // 对齐),不能用 `column!` 把它当 `tab_row` 的兄弟项——`column!` 会从
    // `tab_row` 的 `Fill` 高度里瓜分掉这 1px,导致选中页签的 `select`
    // 按钮比未选中页签矮 1px,标题文字的居中基准跟着偏,与未选中页签的
    // 标题对不上(貌似"没对齐"的根因)。`stack!` 的每一层都吃满同一块
    // 区域,不会互相抢空间。
    let inner = if active {
        container(stack![
            tab_row,
            container(
                container(iced_widget::space::Space::new())
                    .width(Length::Fill)
                    .height(Length::Fixed(1.0))
                    .style(|_t: &iced_widget::Theme| container::Style {
                        background: Some(theme::color::TAB_ACTIVE_BORDER.into()),
                        ..container::Style::default()
                    }),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .align_y(iced_widget::core::alignment::Vertical::Bottom),
        ])
        .width(Length::Fill)
        .height(Length::Fill)
    } else {
        tab_row
    };

    // 背景高 `tab_h`(见上,比 `sq` 高),贴底放进 `project_tabs_row` 的行里后
    // 顶部留白与 Dozer 按钮背景顶部留白相等,视觉上两者顶边对齐,底部则贴到
    // 顶栏下沿(页签式,与内容区无缝衔接)。未激活态同样高 `tab_h`、无背景;
    // 这里的 `align_y` 对贴底本身不起作用(那层在 `project_tabs_row` 的
    // `container(...).align_y(End)` 完成),留着只是 iced `container` 布局
    // 惯例、无空间可分配时是无操作。
    container(inner)
        .height(Length::Fixed(tab_h))
        .width(Length::Fill)
        .align_y(iced_widget::core::alignment::Vertical::Center)
        .clip(true)
        .style(move |_t: &iced_widget::Theme| {
            if active {
                container::Style {
                    background: Some(theme::color::TAB_ACTIVE_BG.into()),
                    // 仅左上/右上圆角,底部 1px 强调线由 inner 承载(见上)。
                    border: Border {
                        radius: Radius {
                            top_left: 8.0,
                            top_right: 8.0,
                            ..Radius::default()
                        },
                        ..Border::default()
                    },
                    ..container::Style::default()
                }
            } else {
                container::Style::default()
            }
        })
        .into()
}

/// 一个项目页签的后台活动指示点:取该项目所有**存活**会话里最值得关注的
/// 那个状态。返回 `(颜色, 是否闪烁)`;`None` = 没有存活会话,不画点。
fn project_tab_dot(ws: &Workspace) -> Option<(Color, bool)> {
    let alive: Vec<AgentState> = ws
        .tabs
        .iter()
        .filter(|t| t.alive)
        .map(|t| t.agent_state)
        .collect();
    project_dot(&alive)
}

/// 上面那个的纯逻辑内核(可单测:构造 `Workspace` 需要 daemon + EventLoop,
/// headless 测试里造不出来,与本文件既有约定一致)。
pub(crate) fn project_dot(alive_states: &[AgentState]) -> Option<(Color, bool)> {
    winning_agent_state(alive_states).map(agent_state_dot)
}

/// 一组存活会话状态里"最值得关注"的那个(设计文档 §6 的优先级):
/// TurnEnded(该甲方出手了)> AwaitingInput(agent 在等人)> Running(还在跑)
/// > Idle > 无存活会话(`None`,不画点)。
///
/// 与 [`agent_state_dot`] 分家是为了让 `Stub` 页签也能用:启动恢复时那些还没
/// 促成的页签手上只有 daemon 的 `SessionInfo` 列表,没有 `Workspace`,但"哪个
/// 状态优先"这条规则必须与 `Loaded` 页签**完全一致**,不能各写一份
/// (最终审查 Required Fix #5)。
fn winning_agent_state(alive_states: &[AgentState]) -> Option<AgentState> {
    [
        AgentState::TurnEnded,
        AgentState::AwaitingInput,
        AgentState::Running,
        AgentState::Idle,
    ]
    .into_iter()
    .find(|candidate| alive_states.contains(candidate))
}

/// 胜出状态 → `(颜色, 是否闪烁)`。颜色不另造一套表,直接问既有 `dot_color`
/// ——页签点与 tab 点讲的是同一种语言,两份颜色表迟早会漂。
fn agent_state_dot(state: AgentState) -> (Color, bool) {
    (
        dot_color(state, true),
        state == AgentState::Running, // 只有"在跑"才闪
    )
}

/// 启动恢复时给每个 `Stub` 页签算后台活动状态:从 daemon 一次性吐出的全量
/// 会话列表里,挑出属于该项目的**存活**会话,套用与 `Loaded` 页签相同的优先级。
///
/// 为什么非要有这一步:重启后除了上次聚焦的那一个,**所有**页签都是 `Stub`,
/// 而 `Stub` 手上没有会话列表 → 指示点恒为空。也就是说"切走了还想知道另一个
/// 项目有没有在动"这个整套功能存在的核心理由(设计文档 §6),在最常见的
/// "刚打开 app"场景下完全不工作。这里只额外花一次 `list()` 往返、不促成任何
/// `Workspace`,懒加载照旧(最终审查 Required Fix #5)。
fn stub_activity(sessions: &[SessionInfo], project_id: i64) -> Option<AgentState> {
    let alive: Vec<AgentState> = sessions
        .iter()
        .filter(|s| s.alive && s.project_id == Some(project_id))
        .map(|s| s.agent_state)
        .collect();
    winning_agent_state(&alive)
}

/// 单个图标栏按钮：圆角正方形背景常驻,hover 图标变金(无金框),选中图标
/// 变金且带金色外框。
pub(crate) fn rail_icon_button<'a>(
    icon: icons::IconKind,
    active: bool,
    hover_t: f32,
    msg: Message,
    tooltip: &'a str,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    // 图标颜色:选中态恒为金;未选中时 hover 平滑过渡到金(见 `HoverId`/
    // `App::hover_progress`——与光标闪烁同款自驱 redraw 动画)。SVG 颜色
    // 构建时定死、不吃 `button::Status`,所以 hover 进度靠 `hover_t` 参数从
    // App 算进来。
    let color = if active {
        theme::color::GOLD
    } else {
        theme::color::mix(theme::color::DIM, theme::color::GOLD, hover_t)
    };
    let inner = container(icons::view(icon, crate::theme::icon_size::rail(), color))
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(iced_widget::core::alignment::Horizontal::Center)
        .align_y(iced_widget::core::alignment::Vertical::Center);

    let radius = 8.0;
    let base_border = Border {
        color: Color::TRANSPARENT,
        width: 1.0,
        radius: radius.into(),
    };

    let content: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> = button(inner)
        .on_press(msg)
        .width(Length::Fixed(crate::theme::geometry::rail_button_size()))
        .height(Length::Fixed(crate::theme::geometry::rail_button_size()))
        .padding(0)
        .style(move |_t: &iced_widget::Theme, _status: button::Status| {
            // 圆角正方形背景常驻(`CARD`);金色外框只在选中态出现,hover
            // 不放金框——所以样式完全由 `active` 决定,与交互态无关。
            button::Style {
                background: Some(theme::color::CARD.into()),
                border: Border {
                    color: if active {
                        theme::color::GOLD
                    } else {
                        Color::TRANSPARENT
                    },
                    ..base_border
                },
                ..button::Style::default()
            }
        })
        .into();
    icons::with_tooltip(content, tooltip)
}

/// 左图标栏:文件列表 / Web 两个图标,点已激活的那个即收起左面板区。
fn left_icon_rail(app: &App) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let region = theme::region::left_icon_rail();
    // 视觉"选中"= 该视图激活 **且**左面板区展开。点已选中的图标会收起面板区,
    // 此时图标要退回未选中态(见 `LeftIconSelect`),所以 `active` 得带上
    // `!left_collapsed`。
    let left_open = !app.left_collapsed;
    let content = column![
        // Project 信息面板入口：项目名 / git 分支+脏标 / 验收次数 / 可编辑目标。
        // 置顶(用户 2026-08-11 指定)。
        icons::icon_button_entry(
            icons::IconKind::Briefcase,
            crate::theme::icon_size::rail(),
            app.left_view == LeftView::Project && left_open,
            app.hover_progress(HoverId::Rail(RailButton::LeftProject)),
            true,
            crate::theme::geometry::rail_button_size(),
            true,
            Message::LeftIconSelect(LeftView::Project),
            |hovered| Message::Hover(HoverId::Rail(RailButton::LeftProject), hovered),
            "项目",
        ),
        // Todo 面板入口：`.dozer/todo.md` 任务列表。第二顺位(用户 2026-08-11
        // 指定)。
        icons::icon_button_entry(
            icons::IconKind::ListTodo,
            crate::theme::icon_size::rail(),
            app.left_view == LeftView::Todo && left_open,
            app.hover_progress(HoverId::Rail(RailButton::LeftTodo)),
            true,
            crate::theme::geometry::rail_button_size(),
            true,
            Message::LeftIconSelect(LeftView::Todo),
            |hovered| Message::Hover(HoverId::Rail(RailButton::LeftTodo), hovered),
            "待办",
        ),
        // 文件列表入口：项目树 + 文件预览配对。
        icons::icon_button_entry(
            icons::IconKind::FolderTree,
            crate::theme::icon_size::rail(),
            app.left_view == LeftView::Files && left_open,
            app.hover_progress(HoverId::Rail(RailButton::LeftFiles)),
            true,
            crate::theme::geometry::rail_button_size(),
            true,
            Message::LeftIconSelect(LeftView::Files),
            |hovered| Message::Hover(HoverId::Rail(RailButton::LeftFiles), hovered),
            "文件",
        ),
        // spike(2026-08-06):Git 提交图入口,验证 gleisbau 库可行性用。
        icons::icon_button_entry(
            icons::IconKind::GitGraph,
            crate::theme::icon_size::rail(),
            app.left_view == LeftView::GitLog && left_open,
            app.hover_progress(HoverId::Rail(RailButton::LeftGit)),
            true,
            crate::theme::geometry::rail_button_size(),
            true,
            Message::LeftIconSelect(LeftView::GitLog),
            |hovered| Message::Hover(HoverId::Rail(RailButton::LeftGit), hovered),
            "Git 提交",
        ),
        // 数据库面板入口:数据源管理 + 连接测试。
        icons::icon_button_entry(
            icons::IconKind::Database,
            crate::theme::icon_size::rail(),
            app.left_view == LeftView::Database && left_open,
            app.hover_progress(HoverId::Rail(RailButton::LeftDatabase)),
            true,
            crate::theme::geometry::rail_button_size(),
            true,
            Message::LeftIconSelect(LeftView::Database),
            |hovered| Message::Hover(HoverId::Rail(RailButton::LeftDatabase), hovered),
            "数据库",
        ),
        // SSH 主机面板入口。
        icons::icon_button_entry(
            icons::IconKind::Server,
            crate::theme::icon_size::rail(),
            app.left_view == LeftView::Ssh && left_open,
            app.hover_progress(HoverId::Rail(RailButton::LeftSsh)),
            true,
            crate::theme::geometry::rail_button_size(),
            true,
            Message::LeftIconSelect(LeftView::Ssh),
            |hovered| Message::Hover(HoverId::Rail(RailButton::LeftSsh), hovered),
            "SSH 主机",
        ),
        // 浏览器面板入口:左图标栏最底部 Globe 按钮(2026-08-11 从右栏移回)。
        icons::icon_button_entry(
            icons::IconKind::Globe,
            crate::theme::icon_size::rail(),
            app.left_view == LeftView::Web && left_open,
            app.hover_progress(HoverId::Rail(RailButton::LeftWeb)),
            true,
            crate::theme::geometry::rail_button_size(),
            true,
            Message::LeftIconSelect(LeftView::Web),
            |hovered| Message::Hover(HoverId::Rail(RailButton::LeftWeb), hovered),
            "浏览器",
        ),
    ]
    .spacing(region.gap)
    .padding(region.padding);

    container(content)
        .width(Length::Fixed(theme::geometry::icon_rail_width()))
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: region.border.unwrap_or_default(),
            ..container::Style::default()
        })
        .into()
}

/// 右图标栏:Agent / 对话两个图标,语义同 `left_icon_rail`。
fn right_icon_rail(app: &App) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let region = theme::region::right_icon_rail();
    // 同 `left_icon_rail`:视觉"选中"需右面板区展开。
    let right_open = !app.right_collapsed;
    let content = column![
        icons::icon_button_entry(
            icons::IconKind::Brain,
            crate::theme::icon_size::rail(),
            app.right_view == RightView::Agent && right_open,
            app.hover_progress(HoverId::Rail(RailButton::RightAgent)),
            true,
            crate::theme::geometry::rail_button_size(),
            true,
            Message::RightIconSelect(RightView::Agent),
            |hovered| Message::Hover(HoverId::Rail(RailButton::RightAgent), hovered),
            "代理",
        ),
        icons::icon_button_entry(
            icons::IconKind::BotMessageSquare,
            crate::theme::icon_size::rail(),
            app.right_view == RightView::Conversations && right_open,
            app.hover_progress(HoverId::Rail(RailButton::RightConversations)),
            true,
            crate::theme::geometry::rail_button_size(),
            true,
            Message::RightIconSelect(RightView::Conversations),
            |hovered| Message::Hover(HoverId::Rail(RailButton::RightConversations), hovered),
            "对话",
        ),
        icons::icon_button_entry(
            icons::IconKind::BarChart3,
            crate::theme::icon_size::rail(),
            app.right_view == RightView::Usage && right_open,
            app.hover_progress(HoverId::Rail(RailButton::RightUsage)),
            true,
            crate::theme::geometry::rail_button_size(),
            true,
            Message::RightIconSelect(RightView::Usage),
            |hovered| Message::Hover(HoverId::Rail(RailButton::RightUsage), hovered),
            "用量",
        ),
        {
            let pending = app
                .active_workspace()
                .and_then(|ws| ws.tabs.get(ws.active))
                .map(|t| t.delivery_pending)
                .unwrap_or(false);
            let base: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
                icons::icon_button_entry(
                    icons::IconKind::BadgeCheck,
                    crate::theme::icon_size::rail(),
                    app.right_view == RightView::Acceptance && right_open,
                    app.hover_progress(HoverId::Rail(RailButton::RightAcceptance)),
                    true,
                    crate::theme::geometry::rail_button_size(),
                    true,
                    Message::RightIconSelect(RightView::Acceptance),
                    |hovered| Message::Hover(HoverId::Rail(RailButton::RightAcceptance), hovered),
                    "验收",
                );
            if pending {
                let badge: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
                    stack![
                        base,
                        container(iced_widget::Space::new())
                            .width(Length::Fixed(8.0))
                            .height(Length::Fixed(8.0))
                            .style(|_t: &iced_widget::Theme| container::Style {
                                background: Some(theme::color::GOLD.into()),
                                border: Border {
                                    radius: 4.0.into(),
                                    ..Border::default()
                                },
                                ..container::Style::default()
                            }),
                    ]
                    .into();
                badge
            } else {
                base
            }
        },
    ]
    .spacing(region.gap)
    .padding(region.padding);

    container(content)
        .width(Length::Fixed(theme::geometry::icon_rail_width()))
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: region.border.unwrap_or_default(),
            ..container::Style::default()
        })
        .into()
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
/// 会被 `LeftIconSelect`/`RightIconSelect` 无条件清掉,所以这个组合不会
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
    let inner: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
        match app.left_view {
            LeftView::Files => {
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
                row![
                    list_pane,
                    divider_bar(
                        Divider::LeftPairSplit,
                        theme::region::project_pane()
                            .background
                            .unwrap_or(theme::color::BG),
                        theme::region::preview_pane()
                            .background
                            .unwrap_or(theme::color::BG),
                    ),
                    preview_pane(
                        app,
                        ws,
                        Length::FillPortion(content_portion),
                        zone_pane_border(zone, rc)
                    ),
                ]
                .width(Length::Fill)
                .into()
            }
            LeftView::GitLog => {
                git_log::view(&app.git_log, ws.project_panel.worktrees()).map(Message::GitLog)
            }
            LeftView::Todo => {
                let Some(project_id) = ws.project.as_ref().map(|p| p.id) else {
                    return column![].into();
                };
                let tabs: Vec<todo::SessionTabSummary> = ws
                    .tabs
                    .iter()
                    .chain(ws.ssh_tabs.iter())
                    .map(|t| todo::SessionTabSummary {
                        session_id: t.info.id.clone(),
                        title: tab_title(t.agent, t.cwd.as_deref(), &t.info.name),
                        alive: t.alive,
                    })
                    .collect();
                let project_path = ws
                    .project
                    .as_ref()
                    .map(|p| std::path::PathBuf::from(&p.path));
                let (list_portion, content_portion) = split_portions(app.dims.todo_split);
                let (sidebar_pane, content_pane) = todo::view(
                    &app.todo,
                    &ws.todo,
                    project_id,
                    &tabs,
                    project_path.as_deref(),
                    Length::FillPortion(list_portion),
                    zone_pane_border(zone, lc),
                    Length::FillPortion(content_portion),
                    zone_pane_border(zone, rc),
                );
                row![
                    sidebar_pane.map(Message::Todo),
                    divider_bar(
                        Divider::TodoSplit,
                        theme::region::project_pane()
                            .background
                            .unwrap_or(theme::color::BG),
                        theme::region::preview_pane()
                            .background
                            .unwrap_or(theme::color::BG),
                    ),
                    content_pane.map(Message::Todo),
                ]
                .width(Length::Fill)
                .into()
            }
            LeftView::Project => {
                let (list_portion, content_portion) = split_portions(app.dims.project_split);
                let info_pane: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
                    project::view(
                        &ws.project_panel,
                        ws.project.as_ref(),
                        Length::FillPortion(list_portion),
                        zone_pane_border(zone, lc),
                    )
                    .map(Message::Project);
                row![
                    info_pane,
                    divider_bar(
                        Divider::ProjectSplit,
                        theme::region::project_pane()
                            .background
                            .unwrap_or(theme::color::BG),
                        theme::region::preview_pane()
                            .background
                            .unwrap_or(theme::color::BG),
                    ),
                    project_preview_pane(
                        app,
                        ws,
                        Length::FillPortion(content_portion),
                        zone_pane_border(zone, rc)
                    ),
                ]
                .width(Length::Fill)
                .into()
            }
            LeftView::Database => {
                // 数据库面板需要项目已打开才能读写 `.dozer/database.json`。
                if ws.project.is_none() {
                    return column![].into();
                }
                database::view(
                    &app.database,
                    &ws.database,
                    Length::Fill,
                    zone_pane_border(zone, ac),
                    app.hover_progress(HoverId::DatabaseSchemaBack),
                )
                .map(Message::Database)
            }
            LeftView::Ssh => {
                // 同 Files/Database 面板:`ws.project.is_none()` 是 Stub→Loaded
                // 促成期间的占位态,这时不该渲染出一个看似可点、实际上
                // `Message::Ssh` 分发会被内核静默吞掉(无 project 时直接
                // return)的"＋新增主机"按钮。
                if ws.project.is_none() {
                    return column![].into();
                }
                let (list_portion, content_portion) = split_portions(app.dims.ssh_split);
                let list_pane = ssh::view(
                    &ws.ssh,
                    Length::FillPortion(list_portion),
                    zone_pane_border(zone, lc),
                )
                .map(Message::Ssh);
                row![
                    list_pane,
                    divider_bar(
                        Divider::SshSplit,
                        theme::region::project_pane()
                            .background
                            .unwrap_or(theme::color::BG),
                        theme::region::preview_pane()
                            .background
                            .unwrap_or(theme::color::BG),
                    ),
                    ssh_terminal_pane(
                        app,
                        ws,
                        Length::FillPortion(content_portion),
                        zone_pane_border(zone, rc)
                    ),
                ]
                .width(Length::Fill)
                .into()
            }
            LeftView::Web => browser::view(
                &ws.browser,
                ws.project.as_ref().map(|p| p.id),
                Length::Fill,
                zone_pane_border(zone, ac),
            )
            .map(Message::Browser),
        };
    if maximized {
        return inner;
    }
    let region = zone;
    // worktree 速览条("本工作区：xxx" + 其它 worktree 切换)现在由 `git_log::view`
    // 自己渲染在文件夹路径下方,不再在这里额外包一层,避免盖在面板标题上方。
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
                    color: theme::color::GOLD,
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
    let inner: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
        match app.right_view {
            RightView::Agent => {
                let (list_portion, content_portion) = split_portions(app.dims.agent_split);
                row![
                    terminal_pane(
                        app,
                        ws,
                        Length::FillPortion(content_portion),
                        zone_pane_border(zone, lc)
                    ),
                    divider_bar(
                        Divider::RightPairSplit,
                        theme::region::terminal_pane()
                            .background
                            .unwrap_or(theme::color::BG),
                        theme::region::agent_list_pane()
                            .background
                            .unwrap_or(theme::color::BG),
                    ),
                    agent_list_pane(
                        app,
                        ws,
                        Length::FillPortion(list_portion),
                        zone_pane_border(zone, rc)
                    ),
                ]
                .width(Length::Fill)
                .into()
            }
            RightView::Conversations => {
                let (list_portion, content_portion) = split_portions(app.dims.conversations_split);
                row![
                    review_content_pane(
                        ws,
                        Length::FillPortion(content_portion),
                        zone_pane_border(zone, lc)
                    ),
                    divider_bar(
                        Divider::RightPairSplit,
                        theme::region::review_content_pane()
                            .background
                            .unwrap_or(theme::color::BG),
                        theme::region::conversation_list_pane()
                            .background
                            .unwrap_or(theme::color::BG),
                    ),
                    conversation_list_pane(
                        ws,
                        Length::FillPortion(list_portion),
                        zone_pane_border(zone, rc)
                    ),
                ]
                .width(Length::Fill)
                .into()
            }
            RightView::Usage => usage::view(
                &ws.usage,
                Length::Fill,
                zone_pane_border(zone, ac),
                app.hover_progress(HoverId::UsageRefresh),
            )
            .map(Message::Usage),
            RightView::Acceptance => {
                acceptance::view(&ws.acceptance, Length::Fill, zone_pane_border(zone, ac))
                    .map(Message::Acceptance)
            }
        };
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
                    color: theme::color::GOLD,
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

    // 顶部垫一条透明的 `theme::geometry::top_bar_height()` 高 Space,把变暗遮罩钉在顶栏
    // 之下——`base = column![top, body]` 里顶栏和内容区就是这么分的,
    // 这里镜像同一结构,让变暗区域精确对齐 `body` 的渲染范围,不覆盖顶栏
    // (Important:此前没有这条 Space,遮罩会盖住整个窗口高度,连顶栏的
    // 项目 tab 等控件都会被染黑)。
    column![
        iced_widget::space::Space::new().height(Length::Fixed(theme::geometry::top_bar_height())),
        row![
            iced_widget::space::Space::new()
                .width(Length::Fixed(theme::geometry::icon_rail_width())),
            dim_bg,
            iced_widget::space::Space::new()
                .width(Length::Fixed(theme::geometry::icon_rail_width())),
        ],
    ]
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

/// 终端栏：表头 + tab 栏 + （可能的错误文案）+ 当前激活 tab 的终端网格。
fn terminal_pane<'a>(
    app: &'a App,
    ws: &'a Workspace,
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let region = theme::region::terminal_pane();
    let mut content = column![tab_bar(app, ws)].spacing(region.gap);

    if let Some(err) = &app.daemon_error {
        content = content.push(
            text(format!("⚠ {err}"))
                .size(theme::font::body())
                .color(theme::color::RED),
        );
    }

    // OSC 133;D 的最近命令非零退出码提示（下一条命令开始时消失）。
    if let Some(code) = ws.tabs.get(ws.active).and_then(|t| t.last_exit)
        && code != 0
    {
        content = content.push(
            text(format!("exit {code}"))
                .size(theme::font::label())
                .color(theme::color::RED),
        );
    }

    // P1j 收敛：终端"审阅"按钮移除，会话审阅入口统一到右一对话列表。

    content = content.push(active_tab_view(app, ws));

    let body = container(content.spacing(region.gap).padding(region.padding))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_theme: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: outer,
            ..container::Style::default()
        });

    // 底栏(`terminal_status_bar`)是贴在 `body` 下方的独立元素,同
    // `project_status_bar` 一样需要 `outer` 的圆角半径收底角,否则左下角
    // 顶出小尖角。
    container(column![body, terminal_status_bar(ws, outer)])
        .width(width)
        .height(Length::Fill)
        .into()
}

/// 分隔线:命中区 `theme::geometry::divider_width()` 宽、`Length::Fill` 高,
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
fn divider_bar<'a>(
    divider: Divider,
    left_bg: Color,
    right_bg: Color,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let show_line = !matches!(divider, Divider::LeftRight);
    if !show_line {
        let gap = iced_widget::Space::new()
            .width(Length::Fixed(theme::geometry::divider_width()))
            .height(Length::Fill);
        return MouseArea::new(gap)
            .interaction(mouse::Interaction::ResizingColumn)
            .on_press(Message::ColumnDragStart(divider))
            .into();
    }
    let line_w = 2.0_f32;
    let side_w = (theme::geometry::divider_width() - line_w) / 2.0;
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
            background: Some(theme::color::BORDER.into()),
            ..container::Style::default()
        });
    let row = row![left_side, line, right_side]
        .width(Length::Fixed(theme::geometry::divider_width()))
        .height(Length::Fill);
    MouseArea::new(row)
        .interaction(mouse::Interaction::ResizingColumn)
        .on_press(Message::ColumnDragStart(divider))
        .into()
}

/// 箭头翻页按钮：ChevronLeft / ChevronRight，可用时 GOLD，hover 显 CARD 圆角底，到头时 DIM 且不可点。
pub(crate) fn tab_arrow_button<'a, M: Clone + 'a>(
    icon: icons::IconKind,
    enabled: bool,
    msg: M,
) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer> {
    // 激活(可点)态用 `#dcc9a3`(同顶栏选中页签描边 `TAB_ACTIVE_BORDER`),
    // 静止不再用金;hover 再跳到金 `#F2D94E` 提亮。
    let color = if enabled {
        theme::color::TAB_ACTIVE_BORDER
    } else {
        theme::color::DIM
    };
    let mut btn = button(icons::view(
        icon,
        crate::theme::icon_size::tab_arrow(),
        color,
    ))
    .width(Length::Fixed(
        crate::theme::geometry::tab_arrow_button_size(),
    ))
    .height(Length::Fixed(
        crate::theme::geometry::tab_arrow_button_size(),
    ))
    .padding(0)
    .style(move |_theme, status| {
        let base = button::Style {
            background: None,
            text_color: color,
            ..button::Style::default()
        };
        if !enabled {
            return base;
        }
        match status {
            button::Status::Hovered | button::Status::Pressed => button::Style {
                background: Some(theme::color::CARD.into()),
                text_color: theme::color::GOLD,
                border: Border {
                    color: Color::TRANSPARENT,
                    width: 1.0,
                    radius: 4.0.into(),
                },
                ..base
            },
            _ => base,
        }
    });
    if enabled {
        btn = btn.on_press(msg);
    }
    btn.into()
}

/// tab 栏下方的 1px 分割线。
pub(crate) fn tab_divider<'a, M: 'a>() -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>
{
    container(iced_widget::Space::new())
        .width(Length::Fill)
        .height(Length::Fixed(1.0))
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(theme::color::BORDER.into()),
            ..container::Style::default()
        })
        .into()
}

/// 给一块 tab 内容包上"拖拽换位"的感应层:内容本身仍是原来的交互(点标题
/// 选中/点 × 关闭全在内部),外层只补一个 `on_move`——因为子按钮只会吞掉
/// **按下**事件,光标在页签上移动的 `CursorMoved` 不会被吞,`on_move` 照常
/// 触发,据此发出 `TabDragMove`。真正"按住页签＝准备拖"由各选中处理
/// (`SelectTab`/`PreviewSelectTab`/`ProjectTabSwitch`)在按住瞬间把
/// `tab_drag` 置位,这里的 `on_move` 只认"当前拖的是本组"的时刻(见
/// `App::tab_drag_move` 的组校验),松开由 main.rs 发 `TabDragEnd`。这样
/// 点击选中与拖拽换位互不干扰,也和 `ColumnDrag` 同一套原始事件后端。
pub(crate) fn tab_drag_surface(
    content: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>,
    group: TabGroup,
    index: usize,
    armed: bool,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let area = MouseArea::new(content).on_move(move |_| Message::TabDragMove { group, index });
    if armed {
        let area = area.interaction(mouse::Interaction::Grabbing);
        return area.into();
    }
    area.into()
}

/// 面板内 tab（终端 / 预览 / 浏览器三处共用）的渲染器，样式对齐顶栏未选中
/// 页签：标题 `body()`(13px) + `top_bar_font()`，静止 `DIM`、hover 动画
/// `DIM→金`；关闭 `×` 静止 `DIM`、hover `DIM→金`、24×24 命中框；未选中
/// hover 显 `TAB_HOVER` 胶囊背景（radius 8）。tab 宽度随标题适配
/// (`Length::Shrink`)，超过 `PANEL_TAB_MAX_W` 时标题省略号截断。激活态外观
/// (CREAM 标题 + CARD 实底 + 1px 边框)由本函数统一绘制，未选中态额外画
/// hover 细节。
///
/// 泛型 over 消息类型 `M`：终端/预览传 `app::Message`，浏览器传
/// `browser::Message`，保证三处渲染完全一致。`hover_t`/`close_hover_t` 是
/// 调用方动画源给的插值进度(0..=1)；`prefix` 承载终端状态点(标题左侧)，
/// `suffix` 承载预览编辑图标(标题右侧、关闭按钮前，仍是独立可点元素)。
/// (iced 0.14 的 `Text` 无原生省略号，截断靠 `fit_title` 手动补 `…`。)
// 共享的 panel tab 渲染器,被终端/预览/browser 三处复用;参数多是刻意保留的
// 单一职责接口(标题/激活态/两组 hover 进度与回调/前后缀),拆结构体反而要
// 引入 `Box<dyn Fn>`,得不偿失。
#[allow(clippy::too_many_arguments)]
pub(crate) fn panel_tab<'a, M: Clone + 'a>(
    title: String,
    active: bool,
    hover_t: f32,
    close_hover_t: f32,
    prefix: Option<Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>>,
    suffix: Option<Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>>,
    on_select: M,
    on_close: M,
    title_hover: impl Fn(bool) -> M + 'a,
    close_hover: impl Fn(bool) -> M + 'a,
) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer> {
    let close_sz = crate::theme::geometry::tab_button_size();
    // 组合 hover:悬停标题或 × 任一,胶囊背景都浮现、× 显形。
    let hover = hover_t.max(close_hover_t).clamp(0.0, 1.0);
    let hovered = hover > 0.001;
    // 标题区域最大宽 = 整 tab 上限 - 左右 padding - 与关闭按钮的间距 - 关闭按钮。
    let title_max = PANEL_TAB_MAX_W - PANEL_TAB_PAD_LEFT - PANEL_TAB_PAD_X - 2.0 - close_sz;
    let title_color = if active {
        theme::color::CREAM
    } else {
        // 未选中态:静止 DIM,hover 时平滑过渡到金(与顶栏页签同一套动画)。
        theme::color::mix(theme::color::DIM, theme::color::GOLD, hover_t)
    };

    let mut title_row = row![]
        .spacing(4)
        .align_y(iced_widget::core::Alignment::Center);
    if let Some(p) = prefix {
        title_row = title_row.push(p);
    }
    title_row = title_row.push(
        container(
            text(fit_title(&title, title_max))
                .font(top_bar_font())
                .size(theme::font::body())
                .color(title_color),
        )
        // 随标题长度适配,超宽则截到 title_max 并靠 `fit_title` 补省略号。
        .width(Length::Shrink)
        .max_width(title_max)
        .clip(true),
    );

    // 选中/关闭的接线逻辑收在 `tabs::tab_core`(2026-08-12 抽取,试点已
    // 验证过——项目页签早已用它;这里是把面板 tab 自己那份原版实现换
    // 成同一个共享内核)。四个现有参数 title_hover/close_hover/on_select/
    // on_close 与 tab_core 的 on_select_hover/on_close_hover/on_select/
    // on_close 逐个对应,直接透传。
    let close_base = theme::color::mix(theme::color::DIM, theme::color::GOLD, close_hover_t);
    let close_color = Color {
        a: hover,
        ..close_base
    };
    let (select, close) = tabs::tab_core(
        title_row.into(),
        close_sz,
        close_color,
        hovered,
        on_select,
        on_close,
        title_hover,
        close_hover,
    );

    let mut tab_row = row![select]
        .spacing(2)
        .align_y(iced_widget::core::Alignment::Center);
    if let Some(s) = suffix {
        tab_row = tab_row.push(s);
    }
    tab_row = tab_row.push(close);
    container(tab_row)
        .padding(Padding {
            top: PANEL_TAB_PAD_Y,
            right: PANEL_TAB_PAD_X,
            bottom: PANEL_TAB_PAD_Y,
            left: PANEL_TAB_PAD_LEFT,
        })
        .width(Length::Shrink)
        .max_width(PANEL_TAB_MAX_W)
        .style(move |_t: &iced_widget::Theme| {
            if active {
                container::Style {
                    background: Some(theme::color::CARD.into()),
                    border: Border {
                        color: theme::color::BORDER,
                        width: 1.0,
                        radius: 6.0.into(),
                    },
                    ..container::Style::default()
                }
            } else if hover > 0.0 {
                // 悬停胶囊铺满整片 tab(含 × 区),× 落在其内部。
                container::Style {
                    background: Some(
                        Color {
                            a: hover,
                            ..theme::color::TAB_HOVER
                        }
                        .into(),
                    ),
                    border: Border {
                        radius: 6.0.into(),
                        ..Border::default()
                    },
                    ..container::Style::default()
                }
            } else {
                container::Style::default()
            }
        })
        .into()
}

/// 面板 tab 统一上限宽（对齐顶栏 `project_tab_max_width`）。标题超宽时省略号
/// 截断,正常情况下 tab 宽度随标题适配。导出给 `workspace.rs`/`browser.rs`
/// 的翻页宽度估算共用,避免各处硬编码 160。
pub(crate) const PANEL_TAB_MAX_W: f32 = 160.0;
/// 面板 tab 内边距:横向留白给 hover 胶囊,纵向收紧以缩小高度。左侧单独
/// 放大(原先与右侧同为 4,标题贴左缘太紧),右侧维持贴近关闭按钮的窄距。
const PANEL_TAB_PAD_LEFT: f32 = 10.0;
const PANEL_TAB_PAD_X: f32 = 4.0;
const PANEL_TAB_PAD_Y: f32 = 1.0;

/// 按 `max_w` 把标题裁到能放下的长度,截掉的部分用 `…` 替代(iced 0.14 的
/// `Text` 无原生省略号)。粗估每字符宽:CJK 全宽 16、其余半宽 8,留 8px 给
/// `…` 自身。估偏只会让省略号早/晚一个字符,不影响布局。
fn fit_title(title: &str, max_w: f32) -> String {
    const ELLIPSIS_W: f32 = 8.0;
    let mut out = String::new();
    let mut used: f32 = 0.0;
    for c in title.chars() {
        let w = if (c as u32) > 0x2E80 { 16.0 } else { 8.0 };
        // 放不下当前字(且还需为 `…` 留位)就截断并补省略号。
        if used + w > max_w - ELLIPSIS_W {
            out.push('…');
            break;
        }
        out.push(c);
        used += w;
    }
    out
}

/// tab 栏：两侧箭头翻页(到头变灰) + 每会话一个按钮(状态点 + 名称 + 关闭
/// ×)。新建会话走 Agent 面板"＋"(纯 Shell 也在其菜单里),终端 tab 栏
/// 不再放独立"＋"。P1L T5 验收返工：横向 scrollable(底部滚动条)
/// 换成索引窗口化 + `clip`——`on_scroll` 只认滚轮/拖拽，程序化滚动在本
/// app 自建循环里够不到，箭头翻页必须走状态驱动的窗口渲染。
fn tab_bar<'a>(
    app: &'a App,
    ws: &'a Workspace,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let widths: Vec<f32> = ws
        .tabs
        .iter()
        .map(|t| tab_display_width(&tab_title(t.agent, t.cwd.as_deref(), &t.info.name)))
        .collect();
    let (first, can_left, can_right) = tab_window(
        &widths,
        4.0,
        theme::geometry::tab_bar_avail_px(),
        ws.term_tab_first,
    );

    let items: Vec<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>> = ws
        .tabs
        .iter()
        .enumerate()
        .filter(|(idx, _)| *idx >= first)
        .map(|(idx, tab)| {
            let title_hover_t = app.hover_progress(HoverId::TermTabItem(idx));
            let close_hover_t = app.hover_progress(HoverId::TermTabClose(idx));
            let armed = app.dragging_group(TabGroup::Terminal);
            tab_drag_surface(
                tab_item(
                    idx,
                    tab,
                    idx == ws.active,
                    app.blink_on,
                    title_hover_t,
                    close_hover_t,
                ),
                TabGroup::Terminal,
                idx,
                armed,
            )
        })
        .collect();

    // tab 列表进 clip 容器占 Fill,裁掉右侧溢出;左右箭头钉在裁剪区外.
    let tabs_row = row(items).spacing(4);
    let clipped = container(tabs_row).width(Length::Fill).clip(true);
    let left_arrow = tab_arrow_button(
        icons::IconKind::ChevronLeft,
        can_left,
        Message::TermTabScroll(false),
    );
    let right_arrow = tab_arrow_button(
        icons::IconKind::ChevronRight,
        can_right,
        Message::TermTabScroll(true),
    );

    let tab_row = row![left_arrow, right_arrow, clipped]
        .spacing(4)
        .align_y(iced_widget::core::Alignment::Center);

    column![tab_row, tab_divider()].spacing(4).into()
}

/// 给定各 tab 宽、tab 间距、可视宽、当前 first，算出：
/// (钳制后的 first, 左可滚, 右可滚)。
/// - 全部 tab 能放下(总宽<=avail) → first=0, 两端皆不可滚(箭头都变灰)。
/// - 溢出 → max_first = 最小的 i 使 tabs[i..] 总宽 <= avail(即从 i 起剩余恰好放得下);
///   钳制 first 到 [0, max_first]; 左可滚 = first>0; 右可滚 = first<max_first。
pub(crate) fn tab_window(
    widths: &[f32],
    gap: f32,
    avail: f32,
    first: usize,
) -> (usize, bool, bool) {
    let n = widths.len();
    if n == 0 {
        return (0, false, false);
    }
    let total: f32 = widths.iter().sum::<f32>() + gap * (n.saturating_sub(1)) as f32;
    if total <= avail {
        return (0, false, false);
    }
    // 求 max_first：从右往左累加，找最大的窗口起点使 tails 放得下。
    let mut max_first = n - 1;
    let mut acc = 0.0;
    for i in (0..n).rev() {
        let w = widths[i] + if i < n - 1 { gap } else { 0.0 };
        if acc + w <= avail {
            acc += w;
            max_first = i;
        } else {
            break;
        }
    }
    let clamped = first.min(max_first);
    (clamped, clamped > 0, clamped < max_first)
}

/// 单个 tab：状态点（颜色见 `dot_color`）+ 名称的选中按钮，紧跟一个关闭
/// 按钮（点击 = detach，见 `Message::CloseTab` 的文档）。状态不再用文字
/// 胶囊表达，全部收敛到点点的颜色与闪烁（goal.md）：工作中(Running)的
/// 点点随 `blink_on` 一明一暗地闪，其余状态常亮。
fn tab_item(
    idx: usize,
    tab: &SessionTab,
    active: bool,
    blink_on: bool,
    title_hover_t: f32,
    close_hover_t: f32,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let working = tab.alive && tab.agent_state == AgentState::Running;
    let mut color = dot_color(tab.agent_state, tab.alive);
    // 工作中且处于暗相位：把点点压到近乎透明，形成"呼吸"般的闪烁。
    // 闪烁相位是全局的（main.rs 定时翻转），因此失焦的工作 tab 也照闪。
    if working && !blink_on {
        color = Color { a: 0.15, ..color };
    }
    // 状态点作 `panel_tab` 的 prefix（颜色/呼吸逻辑不变）。
    let dot = text("●").size(theme::font::caption_sm()).color(color);

    panel_tab(
        tab_title(tab.agent, tab.cwd.as_deref(), &tab.info.name),
        active,
        title_hover_t,
        close_hover_t,
        Some(dot.into()),
        None,
        Message::SelectTab(idx),
        Message::CloseTab(idx),
        move |h| Message::Hover(HoverId::TermTabItem(idx), h),
        move |h| Message::Hover(HoverId::TermTabClose(idx), h),
    )
}

/// `host_id` → `HoverId::SshTab{Item,Close}` 用的哈希键(`HoverId` 整体
/// `derive(Copy)`,`String` 不是 `Copy`,退化成 `u64`,同 `todo_line_key`
/// 的既有精度取舍——碰撞在同一台主机的 tab hover 高亮场景下不构成实际
/// 风险)。
fn ssh_tab_hover_key(host_id: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    host_id.hash(&mut hasher);
    hasher.finish()
}

/// SSH 面板自己的 tab 条:遍历 `ws.ssh_tabs`,每个渲染一个可关闭 tab。
/// 直接复用 `panel_tab`(右侧共享终端条 `tab_item` 用的同一个函数)而不是
/// 自己拼容器样式,视觉/hover 动画与全应用其它 tab 完全一致——不需要
/// `tabs::tab_core` 手动接线。前缀图标固定用 `IconKind::Terminal`(阶段
/// 4 只有这一种;阶段 3 加 `Sftp` 变体后按 tab 的种类换图标,`SessionTab`
/// 本身不带 `SshTabKind` 字段,种类信息只在 `ws.ssh_active` 里——阶段 4
/// 全部 `ssh_tabs` 里的 tab 都是 `Terminal` 种类,这里暂时不需要按 tab
/// 查种类,阶段 3 扩展这个函数时才需要处理"同一个 host_id 可能对应两个
/// 不同种类的 tab,要分别渲染两个 tab 条目"这件事)。
fn ssh_tab_bar<'a>(
    app: &'a App,
    ws: &'a Workspace,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let mut bar = row![].spacing(4);
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
            crate::theme::icon_size::row(),
            theme::color::DIM,
        );
        let select_id = host_id.clone();
        let close_id = host_id.clone();
        let title_hover_id = host_id.clone();
        let close_hover_id = host_id;
        bar = bar.push(panel_tab(
            tab_title(tab.agent, tab.cwd.as_deref(), &tab.info.name),
            is_active,
            title_hover_t,
            close_hover_t,
            Some(icon),
            None,
            Message::Ssh(ssh::Message::SelectSshTab(
                select_id,
                ssh::SshTabKind::Terminal,
            )),
            Message::Ssh(ssh::Message::CloseSshTab(
                close_id,
                ssh::SshTabKind::Terminal,
            )),
            move |h| Message::Hover(HoverId::SshTabItem(ssh_tab_hover_key(&title_hover_id)), h),
            move |h| Message::Hover(HoverId::SshTabClose(ssh_tab_hover_key(&close_hover_id)), h),
        ));
    }
    bar.into()
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
    let active_tab = ws.ssh_active.as_ref().and_then(|(host_id, _kind)| {
        ws.ssh_tabs
            .iter()
            .find(|t| t.info.id.strip_prefix("ssh:") == Some(host_id.as_str()))
    });
    let body: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> = match active_tab {
        Some(tab) => term_view::view(
            &tab.model,
            keyboard_term_target(app.left_view, app.active_zone) == TermTarget::SshPanel,
            TermTarget::SshPanel,
        ),
        None => container(
            text("点主机卡片的终端/文件传输图标开始")
                .size(theme::font::body())
                .color(theme::color::DIM),
        )
        .padding(20)
        .into(),
    };
    container(column![ssh_tab_bar(app, ws), body].height(Length::Fill))
        .width(width)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(theme::color::BG.into()),
            border: outer,
            ..container::Style::default()
        })
        .into()
}

fn active_tab_view<'a>(
    app: &'a App,
    ws: &'a Workspace,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    match ws.tabs.get(ws.active) {
        Some(tab) => term_view::view(
            &tab.model,
            keyboard_term_target(app.left_view, app.active_zone) == TermTarget::Shared,
            TermTarget::Shared,
        ),
        None => container(
            text("暂无会话——到 Agent 面板点「＋」")
                .size(theme::font::subtitle())
                .color(theme::color::DIM),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// 页签指示点的优先级:金 > 紫 > 绿闪 > 绿常亮 > 不画点。
    #[test]
    fn project_dot_color_priority() {
        use dozer_core::protocol::AgentState::*;

        assert_eq!(project_dot(&[]), None, "无存活会话不画点");
        assert_eq!(project_dot(&[Idle]), Some((theme::color::GREEN, false)));
        assert_eq!(
            project_dot(&[Idle, Running]),
            Some((theme::color::GREEN, true)),
            "有会话在跑 → 同为绿但要闪,靠闪烁与空闲区分"
        );
        assert_eq!(
            project_dot(&[Idle, Running, AwaitingInput]),
            Some((theme::color::PURPLE, false)),
            "待输入优先于运行/空闲"
        );
        assert_eq!(
            project_dot(&[Idle, Running, AwaitingInput, TurnEnded]),
            Some((theme::color::GOLD, false)),
            "回合结束(该甲方出手了)优先级最高"
        );
        // 顺序无关:优先级看的是状态集合,不是 tab 的先后。
        assert_eq!(
            project_dot(&[TurnEnded, Idle]),
            project_dot(&[Idle, TurnEnded])
        );
    }

    /// 测试基准态:默认布局、左=文件列表、右=Agent、两侧都展开。
    fn test_state() -> ShellState {
        ShellState {
            layout: ShellLayout::default(),
            dims: PanelDims::default(),
            left_view: LeftView::Files,
            left_collapsed: false,
            right_view: RightView::Agent,
            right_collapsed: false,
            maximized: None,
        }
    }

    #[test]
    fn preview_content_bounds_is_inside_left_content_column() {
        // 左面板区 640 宽,项目树占 0.35(=224),预览内容区在其右侧(过分隔线)。
        let state = test_state();
        let (x, y, w, h) = preview_content_bounds(1440.0, 900.0, &state);
        let list_w = state.dims.left_width * state.dims.files_split;
        let col_start =
            theme::geometry::icon_rail_width() + list_w + theme::geometry::divider_width();
        assert!(x >= col_start && x < col_start + 16.0, "x={x}");
        assert!((380.0..=420.0).contains(&w), "w={w}");
        assert!(
            (y - (78.0 + theme::region::left_zone().margin.top)).abs() < 0.1,
            "y={y}(顶栏 40 + tab 栏 38 + left_zone 上 margin 之下,地址栏已去)"
        );
        assert!(h > 700.0 && h < 900.0 - y, "h={h}");
    }

    #[test]
    fn preview_content_bounds_web_view_spans_whole_left_zone() {
        // Web 视图(左栏最底部 Globe 按钮)没有配对,预览内容区从图标栏右侧起
        // 占满左面板区。
        let state = ShellState {
            left_view: LeftView::Web,
            ..test_state()
        };
        let (x, _, w, _) = preview_content_bounds(1440.0, 900.0, &state);
        let m = theme::region::left_zone().margin;
        let left_w = left_zone_width(1440.0, &state);
        assert_eq!(x, theme::geometry::icon_rail_width() + 8.0 + m.left);
        assert_eq!(w, left_w - 16.0 - m.left - m.right);
    }

    #[test]
    fn preview_content_bounds_zero_when_left_collapsed() {
        let state = ShellState {
            left_collapsed: true,
            ..test_state()
        };
        assert_eq!(
            preview_content_bounds(1440.0, 900.0, &state),
            (0.0, 0.0, 0.0, 0.0)
        );
    }

    /// Fix round 1 Critical:右侧被放大时,左侧 webview 必须归零——它是原生
    /// wry 子视图,不听 iced 的绘制顺序摆布,不归零会无视变暗遮罩径直叠在
    /// 最上面。
    #[test]
    fn preview_content_bounds_zero_when_right_maximized() {
        let state = ShellState {
            maximized: Some(MaximizedPane::Right),
            ..test_state()
        };
        assert_eq!(
            preview_content_bounds(1440.0, 900.0, &state),
            (0.0, 0.0, 0.0, 0.0)
        );
    }

    /// Fix round 1 Critical:左侧被放大(Files 配对)时,webview 矩形必须
    /// 按 `maximize_overlay` 实际渲染的更大盒子换算,不能再用平时的
    /// `left_zone_width`(640)。用具体数字核对,不只看"落在范围内"：
    /// x0=theme::geometry::icon_rail_width()(44)+theme::geometry::maximize_overlay_padding()(40)=84,
    /// avail_w=1440-2*44-2*40=1272,pair_w=1272-8=1264,
    /// list_w=1264*0.35=442.4,x=84+442.4+8+8=542.4,w=1264*0.65-16=805.6;
    /// y0=theme::geometry::top_bar_height()(40)+40=80,y=80+38(theme::geometry::preview_chrome_top_px(),地址栏已去)=118,
    /// avail_h=900-40-80=780 - status_bar_height()(26,扣 footbar)=754,
    /// h=754-38-8=708。
    #[test]
    fn preview_content_bounds_left_maximized_files_matches_overlay_geometry() {
        let state = ShellState {
            maximized: Some(MaximizedPane::Left),
            ..test_state()
        };
        let (x, y, w, h) = preview_content_bounds(1440.0, 900.0, &state);
        assert!((x - 542.4).abs() < 0.1, "x={x}");
        assert!((y - 118.0).abs() < 0.1, "y={y}");
        assert!((w - 805.6).abs() < 0.1, "w={w}");
        assert!((h - 708.0).abs() < 0.1, "h={h}");
        // 明显区别于平时(非放大)的几何——不能巧合碰上同一个值。
        let normal = preview_content_bounds(1440.0, 900.0, &test_state());
        assert_ne!((x, y, w, h), normal, "放大态几何必须和平时不同");
    }

    /// Fix round 1 Critical:左侧被放大(Web 视图,无配对)时同样要按放大盒子
    /// 换算。x=x0+8=92,w=avail_w-16=1256。
    #[test]
    fn preview_content_bounds_left_maximized_web_spans_whole_overlay_box() {
        let state = ShellState {
            left_view: LeftView::Web,
            maximized: Some(MaximizedPane::Left),
            ..test_state()
        };
        let (x, _, w, _) = preview_content_bounds(1440.0, 900.0, &state);
        assert!((x - 92.0).abs() < 0.1, "x={x}");
        assert!((w - 1256.0).abs() < 0.1, "w={w}");
    }

    /// Fix round 1 Critical:焦点路由与 webview 摆位必须用同一份放大态
    /// 几何——右侧放大时左侧列恒不可点中;左侧放大时命中范围要按放大盒子
    /// 的横向范围([534.4, 1356))判定,不是平时的 [280, 688)。
    #[test]
    fn preview_column_hit_test_respects_maximized_state() {
        let right_max = ShellState {
            maximized: Some(MaximizedPane::Right),
            ..test_state()
        };
        assert!(!is_in_preview_column(500.0, 1440.0, &right_max));

        let left_max = ShellState {
            maximized: Some(MaximizedPane::Left),
            ..test_state()
        };
        assert!(
            !is_in_preview_column(500.0, 1440.0, &left_max),
            "500 在平时的预览列内,但放大盒子的列起点在 534.4 之后"
        );
        assert!(is_in_preview_column(600.0, 1440.0, &left_max));
        assert!(is_in_preview_column(1300.0, 1440.0, &left_max));
        assert!(!is_in_preview_column(1400.0, 1440.0, &left_max));
    }

    #[test]
    fn terminal_pane_height_excludes_top_and_status_bars() {
        let state = test_state();
        let (_, h_with) = terminal_pane_pixel_size(1440.0, 900.0, &state);
        let only_chrome = 900.0 - theme::geometry::chrome_height_px();
        let m = theme::region::right_zone().margin;
        assert!(
            (only_chrome
                - h_with
                - (theme::geometry::top_bar_height()
                    + theme::geometry::status_bar_height()
                    + m.top
                    + m.bottom))
                .abs()
                < 0.01,
            "终端 pane 高度必须再扣顶栏+状态栏+right_zone 上下 margin"
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
            right_view: RightView::Conversations,
            ..test_state()
        };
        assert_eq!(
            terminal_pane_pixel_size(1440.0, 900.0, &conversations),
            (0.0, 0.0)
        );
    }

    /// Fix round 2 Critical #1:窗口被缩到比持久化 `left_width` 还窄时,
    /// 左面板区的**有效**宽必须重新夹取,否则右面板区一寸不剩。
    ///
    /// 具体数字(窗口逻辑宽 720——1440pt 屏上把窗口贴半屏就是这个宽度):
    /// `zones_width(720)` = 720 - 2*44(图标栏) - 8(LeftRight 分隔线) = 624;
    /// 上界 = max(624 - 320(theme::geometry::min_zone_width()), 320) = 320;
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

    /// 极窄窗口(比 `theme::geometry::min_window_width()` 还窄,例如外部强制 resize)下也不 panic,
    /// 且左区宽不会超过 `zones_width` 本身。
    #[test]
    fn clamp_left_width_survives_absurdly_narrow_window() {
        assert_eq!(
            clamp_left_width(200.0, 640.0),
            theme::geometry::min_zone_width()
        );
        assert_eq!(
            clamp_left_width(0.0, 640.0),
            theme::geometry::min_zone_width()
        );
        // 最小窗口宽恰好能让两侧都拿到 theme::geometry::min_zone_width()。
        assert_eq!(
            clamp_left_width(theme::geometry::min_window_width(), 640.0),
            theme::geometry::min_zone_width()
        );
        assert!(
            (zones_width(theme::geometry::min_window_width())
                - 2.0 * theme::geometry::min_zone_width())
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
                right_view: RightView::Conversations,
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
            right_view: RightView::Conversations,
            maximized: Some(MaximizedPane::Right),
            ..test_state()
        };
        let grid_state = terminal_grid_state(review_maximized);
        assert_eq!(
            grid_state.maximized, None,
            "对话视图下的 MaximizedPane::Right 指的是审阅 pane,换算终端网格时不该当成终端被放大"
        );

        let terminal_maximized = ShellState {
            right_view: RightView::Agent,
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
    /// 终端占 1-0.35 → 1264*0.65 = 821.6,减 `theme::geometry::chrome_width_px()`(16) = 805.6;
    /// 盒子高 = 900 - 40(顶栏) - 2*40 = 780,再减 pane 自带底栏 26
    /// (`theme::geometry::status_bar_height()`)与 `theme::geometry::chrome_height_px()`(50) = 704。
    /// 对照平时:zones_width = 1440-2*44-8=1344,right_w = 1344 - 640 = 704,pair = 696,
    /// 696*0.65 = 452.4,减 16 = 436.4;高 = 900 - 40 - 26 - 50 - right_zone 上下 margin = 778。
    /// 换成网格(CELL_WIDTH=8.4,LINE_HEIGHT_PX=16.8 即 14*1.2):放大后 95x41,平时 51x46。
    #[test]
    fn terminal_pane_pixel_size_right_maximized_matches_overlay_box() {
        let maxed = ShellState {
            maximized: Some(MaximizedPane::Right),
            ..test_state()
        };
        let (w, h) = terminal_pane_pixel_size(1440.0, 900.0, &maxed);
        assert!((w - 805.6).abs() < 0.1, "w={w}");
        assert!((h - 704.0).abs() < 0.1, "h={h}");

        let normal = terminal_pane_pixel_size(1440.0, 900.0, &test_state());
        assert!((normal.0 - 436.4).abs() < 0.1, "平时 w={}", normal.0);
        assert!((normal.1 - 778.0).abs() < 0.1, "平时 h={}", normal.1);
        assert_ne!((w, h), normal, "放大态几何必须和平时不同");
        assert!(w > normal.0, "放大后终端必须真的更宽(网格跟着变宽)");

        assert_eq!(crate::term_view::grid_size(w, h), (95, 41));
        assert_eq!(crate::term_view::grid_size(normal.0, normal.1), (51, 46));

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
                right_view: RightView::Conversations,
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

        // 具体网格:1440x900 下应是 51x46(已扣 right_zone 上下 margin),不是兜底的 80x24。
        let (cols, rows) = crate::term_view::grid_size(shown.0, shown.1);
        assert_eq!((cols, rows), (51, 46));
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
        assert_eq!(s.left_width, theme::geometry::min_zone_width());
        assert_eq!(s.files_split, theme::geometry::min_split_ratio());
        assert_eq!(s.agent_split, theme::geometry::max_split_ratio());
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

    /// `window_width`/`window_height` 的夹取单独测:老 `layout.json` 缺这两
    /// 个字段时 serde 补 0.0(不是 `f32::NAN`,判断要用 `> 0.0` 而不能只查
    /// `is_finite`),负数/NAN 同样要落回 `theme::geometry::initial_window_size()`;合法但过小
    /// 的值只夹下限,不整个重置。
    #[test]
    fn sanitize_shell_layout_clamps_window_size() {
        let missing_fields = ShellLayout {
            window_width: 0.0,
            window_height: 0.0,
        };
        let s = sanitize_shell_layout(missing_fields);
        assert_eq!(s.window_width, theme::geometry::initial_window_size().0);
        assert_eq!(s.window_height, theme::geometry::initial_window_size().1);

        let poisoned = ShellLayout {
            window_width: -100.0,
            window_height: f32::NAN,
        };
        let s = sanitize_shell_layout(poisoned);
        assert_eq!(s.window_width, theme::geometry::initial_window_size().0);
        assert_eq!(s.window_height, theme::geometry::initial_window_size().1);

        let too_small = ShellLayout {
            window_width: 10.0,
            window_height: 10.0,
        };
        let s = sanitize_shell_layout(too_small);
        assert_eq!(s.window_width, theme::geometry::min_window_width());
        assert_eq!(s.window_height, theme::geometry::min_window_height());

        let legit = ShellLayout {
            window_width: 1800.0,
            window_height: 1100.0,
        };
        assert_eq!(sanitize_shell_layout(legit), legit);
    }

    #[test]
    fn preview_content_bounds_never_negative() {
        let state = test_state();
        let (_, _, w, h) = preview_content_bounds(100.0, 50.0, &state);
        assert!(w >= 0.0 && h >= 0.0);
    }

    #[test]
    fn preview_column_hit_test() {
        // 窗口宽 1440:左图标栏 44 + 左面板区 640(项目树 0.35=224 + 分隔线 8)。
        // 预览内容列 = [273.2, 684)。
        let state = test_state();
        assert!(!is_in_preview_column(100.0, 1440.0, &state), "落在项目树列");
        assert!(
            is_in_preview_column(273.2, 1440.0, &state),
            "预览列左边界(过配对分隔线)"
        );
        assert!(is_in_preview_column(500.0, 1440.0, &state), "预览列内");
        assert!(!is_in_preview_column(700.0, 1440.0, &state), "已进右面板区");
        assert!(!is_in_preview_column(1200.0, 1440.0, &state), "右面板区内");
    }

    #[test]
    fn preview_column_hit_test_left_collapsed_is_never_hit() {
        let state = ShellState {
            left_collapsed: true,
            ..test_state()
        };
        assert!(!is_in_preview_column(300.0, 1440.0, &state));
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
        assert_eq!(l.left_width, 500.0 - theme::geometry::icon_rail_width());
    }

    #[test]
    fn clamp_left_width_to_minimum() {
        let state = test_state();
        let l = apply_column_drag(state, Divider::LeftRight, 1440.0, 10.0);
        assert_eq!(l.left_width, theme::geometry::min_zone_width());
    }

    #[test]
    fn clamp_left_width_to_maximum_keeps_right_zone_alive() {
        // 拖到最右也要给右面板区留 theme::geometry::min_zone_width()。
        let state = test_state();
        let l = apply_column_drag(state, Divider::LeftRight, 1440.0, 1430.0);
        let expected = 1440.0
            - 2.0 * theme::geometry::icon_rail_width()
            - theme::geometry::divider_width()
            - theme::geometry::min_zone_width();
        assert_eq!(l.left_width, expected);
    }

    #[test]
    fn clamp_left_width_when_window_too_narrow_does_not_panic() {
        // 窗口窄到上界低于下界时,`.max(下限)` 把上界垫平,恒不 panic。
        let state = test_state();
        let l = apply_column_drag(state, Divider::LeftRight, 700.0, 650.0);
        assert_eq!(l.left_width, theme::geometry::min_zone_width());
    }

    #[test]
    fn clamp_files_split_within_bounds() {
        let state = test_state();
        // 左面板区 640 宽,配对内容宽 = 640-8=632,拖到其中点(316)→ 0.5。
        let l = apply_column_drag(
            state,
            Divider::LeftPairSplit,
            1440.0,
            theme::geometry::icon_rail_width() + 316.0,
        );
        assert!((l.files_split - 0.5).abs() < 0.001, "{}", l.files_split);
    }

    #[test]
    fn clamp_files_split_to_range() {
        let state = test_state();
        let l = apply_column_drag(state, Divider::LeftPairSplit, 1440.0, 10.0);
        assert_eq!(l.files_split, theme::geometry::min_split_ratio());
        let l = apply_column_drag(state, Divider::LeftPairSplit, 1440.0, 5000.0);
        assert_eq!(l.files_split, theme::geometry::max_split_ratio());
    }

    #[test]
    fn split_drag_skips_update_on_zero_zone_width() {
        // 该侧已收起 → 区宽 0,除零防御:原样返回不 panic。
        let left_gone = ShellState {
            left_collapsed: true,
            ..test_state()
        };
        assert_eq!(
            apply_column_drag(left_gone, Divider::LeftPairSplit, 1440.0, 500.0),
            left_gone.dims
        );
        let right_gone = ShellState {
            right_collapsed: true,
            ..test_state()
        };
        assert_eq!(
            apply_column_drag(right_gone, Divider::RightPairSplit, 1440.0, 1000.0),
            right_gone.dims
        );
    }

    #[test]
    fn right_pair_split_writes_field_of_current_right_view() {
        // 右面板区宽 = 1440 - 2*44 - 640 - 8 = 704,左边缘 x = 1440-44-704 = 692;
        // 配对内容宽 = 704-8=696,其中点 348 处拖动(692+348=1040)→ 0.5。
        let agent = test_state();
        let l = apply_column_drag(agent, Divider::RightPairSplit, 1440.0, 696.0 + 344.0);
        assert!((l.agent_split - 0.5).abs() < 0.001, "{}", l.agent_split);
        assert_eq!(
            l.conversations_split, agent.dims.conversations_split,
            "不该串写另一配对的比例"
        );

        let conversations = ShellState {
            right_view: RightView::Conversations,
            ..test_state()
        };
        let l = apply_column_drag(
            conversations,
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
            right_view: RightView::Conversations,
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
}
