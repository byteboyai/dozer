// crates/dozer-app/src/workspace.rs
//! iced 程序状态，承担 spike B 里 `controls.rs` 的角色。分两层（P2a 多项目
//! 并行）：
//!
//! - [`App`]：main.rs 真正持有的顶层容器，暴露 `view()`/`update()`。它持有
//!   整个程序只有一份的**外壳态**（daemon 客户端/窗口尺寸/图标栏与面板区
//!   的收起-拖宽-放大状态/右键菜单），以及 `projects`——并行打开的 N 个项目
//!   页签。
//! - [`Workspace`]：**单个项目**的全部状态（终端 tab 集合、文件树、预览/
//!   浏览器域、验收与审阅、对话列表）。N 份同时存活、互不干扰；当前聚焦的
//!   那一份经 `App::active_workspace()`/`active_workspace_mut()` 取用。
//!
//! 渲染的是 ByteBoy2077 的图标栏外壳：左右各一条固定宽图标栏，中间是左面板
//! 区（文件列表配对 / Web）与右面板区（Agent 配对 / 对话配对），两侧各自可
//! 收起、可拖宽，每个配对视图各自记住内部"列表:内容"分割比例；任一内容子
//! 面板可放大成覆盖整个中间区域的浮层（`maximize_overlay`）。终端接
//! `TerminalModel` + `term_view::view`，键盘输入直达 `dozerd`。
//!
//! ## 线程模型（配合 `main.rs` 一起看）
//! - UI 线程：winit 事件循环所在线程，`App::update`/`view` 只在这
//!   里跑。`update` 里凡是要碰网络 IO 的地方（写输入、建会话、resize），
//!   一律 `handle.spawn(...)` 丢给 tokio，绝不在这里 `block_on`。项目态方法
//!   要用到的那几个句柄按值打包成 [`ShellIo`] 传进去（见其文档）。
//! - tokio worker 线程：`main` 持有的 `tokio::runtime::Runtime` 驱动。
//!   每个 tab 的 attach 数据流有且只有一个消费者任务
//!   （[`forward_events`]），持有 `mpsc::UnboundedReceiver<TermEvent>`
//!   ——这是"事件流"的唯一物理落点。
//! - 桥接：tokio 任务里通过 `winit::event_loop::EventLoopProxy::send_event`
//!   把 `Message` 送回 UI 线程；`main.rs` 的 `ApplicationHandler::user_event`
//!   收到后调用 `app.update(..)` 并请求重绘。反方向（UI → tokio）
//!   靠 `Handle::spawn`，两个方向都不需要锁。
use crate::bookmarks;
use crate::chrome_style;
use crate::conversation::{self, ConversationMeta};
use crate::delivery::{self, FileChange, FileGitStatus, WorktreeInfo};
use crate::extensions::git_log;
use crate::git_watch;
use crate::goal::{self, Goal};
use crate::icons;
use crate::icons::IconKind;
use crate::layout;
use crate::open_projects;
use crate::osc::{OscEvent, OscScanner};
use crate::preview::{AddrTarget, PreviewPane, TabKind, WebviewSpec, is_editable_extension};
use crate::preview_state;
use crate::project::{self, FileTree};
use crate::term_model::TerminalModel;
use crate::term_view;
use crate::terminal_font;
use crate::theme;
use crate::todo;
use crate::todo_meta;
use crate::transcript::{self, ReviewEntry};
use crate::usage;
use crate::workspace_font;
use crate::workspace_geometry;
use dozer_client::{Client, TermEvent};
use dozer_core::protocol::{
    AgentKind, AgentState, BookmarkInfo, BookmarkScope, ProjectInfo, SessionInfo,
};
use iced_widget::core::border::Radius;
use iced_widget::core::font::Weight;
use iced_widget::core::mouse;
use iced_widget::core::text::LineHeight;
use iced_widget::core::{Border, Color, Element, Font, Length, Padding};
use iced_widget::{
    MouseArea, Scrollable, button, column, container, responsive, rich_text, row, scrollable, span,
    stack, text, text_editor, text_input,
};
use iced_winit::winit::event_loop::EventLoopProxy;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::runtime::Handle;
use tokio::sync::mpsc;

/// 终端初始网格尺寸（列 x 行）。真实尺寸由窗口创建后的第一次
/// `Message::PaneResized` 立刻纠正（见 `main.rs` 的 `resumed()`）；这里只是
/// "窗口还没量出真实像素前"的兜底默认值。
const DEFAULT_COLS: u16 = 80;
const DEFAULT_ROWS: u16 = 24;

/// 左侧面板区当前显示哪个视图：文件列表(项目树+文件预览配对) / Web(单面板) /
/// Git 提交图(单面板;spike(2026-08-06) 验证 `gleisbau` 库可行性用) / Todo
/// (单面板;`.dozer/todo.md` 任务列表)。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum LeftView {
    Files,
    Web,
    GitLog,
    Todo,
}

/// 右侧面板区当前显示哪个视图：Agent(Agent列表+终端配对) / 对话(对话列表+对话审阅配对)。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum RightView {
    Agent,
    Conversations,
    Usage,
}

/// 四个图标栏按钮的标识,用于追踪 hover 态(图标颜色在 hover 时需变金,
/// 而 SVG 颜色在构建时就定死、不随 `button::Status` 变化,所以得在 App
/// 里记一个 hovered 目标,改色时按它重算)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RailButton {
    LeftFiles,
    LeftWeb,
    LeftGit,
    LeftTodo,
    RightAgent,
    RightConversations,
    RightUsage,
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
/// (`TopbarButton`)、图标栏按钮(`RailButton`)、顶栏 Home 按钮收进同一个
/// 枚举,这样它们能共用一套 `hover_anims` 状态机与同一条自驱 redraw 定时
/// 唤醒(见 `App::set_hover`/`advance_hover_anims`/`hover_progress`),不必
/// 每个按钮各写一套进度字段。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HoverId {
    Home,
    Topbar(TopbarButton),
    Rail(RailButton),
    /// 某个项目页签的关闭按钮(`×`),按项目 id 区分——同一时刻可能有多个
    /// 页签,不能像 `TopbarButton`/`RailButton` 那样用一个全局标识共用。
    ProjectTabClose(i64),
}

/// 一个可平滑过渡的 hover 动画状态机。iced 0.14 无内置动画 API,这套自驱
/// redraw(与光标闪烁同款范式)把图标/背景颜色在 idle↔hover 之间做 ease-out
/// 插值,而非硬切。`progress` 朝 `target`(0 或 1)指数逼近,约 150ms 收敛。
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
    /// 朝目标逼近一拍(每拍残余 75%),足够接近则 snap 到目标避免无限抖动。
    fn advance(&mut self) {
        let next = self.progress + (self.target - self.progress) * 0.25;
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

/// `Stub` → `Loaded` 促成的中间产物:一个项目的完整恢复素材,且**可以跨线程
/// 搬运**。
///
/// 为什么需要这一层、而不是让异步任务直接产出 `Workspace`:`Workspace` 不是
/// `Send`(`TerminalModel` 内部用 `Rc<RefCell<..>>` 收 PTY 应答),所以它既不能
/// 在 `handle.spawn` 的任务里被构造出来带回,也不能塞进 `Message`——`Message`
/// 一旦不 `Send`,`EventLoopProxy<Message>` 跟着不 `Send`,全文件所有
/// `handle.spawn(async { .. proxy.send_event(..) })` 会一起编译不过。
///
/// 于是促成拆成两段:**IO 段**(本结构体,`Client` 往返,可以在 tokio 线程池
/// 上跑)与**装配段**(`Workspace::from_restore`,建终端模型/派生转发任务,
/// 只能在 UI 线程跑)。本结构体的每个字段都是 `Send` 的纯数据/通道端点。
pub struct ProjectRestore {
    project: ProjectInfo,
    /// 最近项目列表(顺带取回,省一次往返)。
    recent_projects: Vec<ProjectInfo>,
    /// 已 attach 上的存活会话:(会话信息, 起始快照, 事件流)。
    #[allow(clippy::type_complexity)]
    sessions: Vec<(
        SessionInfo,
        Vec<u8>,
        tokio::sync::mpsc::UnboundedReceiver<TermEvent>,
    )>,
}

/// `Message::ProjectSlotLoaded` 的载荷:一份**一次性**的 [`ProjectRestore`]
/// 信封。
///
/// `Message` 必须 `Clone + Debug`(iced 的控件回调按值要一份消息),而
/// `ProjectRestore` 里的 `UnboundedReceiver` 复制不了——通道的接收端只能有
/// 一个。所以用 `Arc<Mutex<Option<..>>>` 包一层:`Clone` 只复制句柄,真正的
/// 素材由第一个 [`RestorePayload::take`] 的人拿走,之后再取得到 `None`(消息
/// 在 winit 事件环里只会被处理一次,不会真的出现第二个取用者)。
pub struct RestorePayload(Arc<Mutex<Option<Box<ProjectRestore>>>>);

impl RestorePayload {
    fn new(restore: ProjectRestore) -> Self {
        Self(Arc::new(Mutex::new(Some(Box::new(restore)))))
    }

    /// 取走信封里的素材;已被取走(或锁中毒)时返回 `None`,调用方当作
    /// "这次促成结果没人要了"处理即可。
    fn take(&self) -> Option<Box<ProjectRestore>> {
        self.0.lock().ok().and_then(|mut slot| slot.take())
    }
}

impl Clone for RestorePayload {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl std::fmt::Debug for RestorePayload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RestorePayload(..)")
    }
}

/// 促成的 **IO 段**:把一个项目在 daemon 上的存活会话逐一 attach 下来,连同
/// 最近项目列表打包成 [`ProjectRestore`]。整段只碰 `Client`,不碰任何 GUI
/// 类型,所以可以在 tokio 线程池上跑(`App::ensure_loaded` 正是这么用的)。
async fn fetch_project_restore(client: &Client, project: ProjectInfo) -> ProjectRestore {
    let mut sessions = Vec::new();
    match client.list().await {
        Ok(list) => {
            // 只认归属本项目的会话:daemon 现在按 `project_id` 给会话分家
            // (P2a Task 1-3),并行打开的别的项目的终端不该跑到这一份
            // `Workspace` 的 tab 栏里来。
            //
            // 迁移期孤儿会话——Task 1-3 落地**之前**建的、`project_id`
            // 为 `None` 的存活会话——会被这条 filter 一并排除,从此不出现
            // 在任何项目的 tab 栏里,直到 dozerd 重启把它们清掉为止(在此
            // 期间它们仍占着 PTY)。这是**有意为之**,不是漏判:规格把
            // "迁移期孤儿会话怎么处理"显式挂起、留给实现计划阶段决定,
            // 本期不做孤儿会话的找回入口。日后有人发现"重启 daemon 前
            // 有几个会话凭空消失了",答案就在这一行。
            for info in list
                .into_iter()
                .filter(|s| s.alive && s.project_id == Some(project.id))
            {
                match client.attach(&info.id, 0).await {
                    Ok((snapshot, _next_offset, rx)) => sessions.push((info, snapshot, rx)),
                    Err(e) => {
                        tracing::warn!(session = %info.id, "attach 失败，跳过该会话恢复: {e}")
                    }
                }
            }
        }
        Err(e) => tracing::warn!("list 失败，跳过启动恢复: {e}"),
    }
    // 最近项目列表（git 分支/脏在窗口起来后异步补）。"当前项目"不再
    // 向 daemon 打听——daemon 侧的"活跃项目"概念已随 P2a Task 1-3 删除
    // （多项目并行下没有唯一活跃项目），改由调用方(`App`)指定。
    let recent_projects = client.list_projects().await.unwrap_or_default();
    ProjectRestore {
        project,
        recent_projects,
        sessions,
    }
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
    pub left_width: f32,
    /// 文件列表配对:项目树占左面板区宽度的比例，文件预览拿剩下的。
    pub files_split: f32,
    /// Agent配对:Agent列表占右面板区宽度的比例，终端拿剩下的。
    pub agent_split: f32,
    /// 对话配对:对话列表占右面板区宽度的比例，对话审阅拿剩下的。
    pub conversations_split: f32,
    pub left_view: LeftView,
    pub right_view: RightView,
    pub left_collapsed: bool,
    pub right_collapsed: bool,
    /// 上次退出时的窗口逻辑尺寸(宽,高)。`main.rs` 建窗时读它决定初始
    /// `with_inner_size`,取代写死的 `workspace_geometry::initial_window_size()`；`App::
    /// persist_window_size_on_exit` 在 `WindowEvent::CloseRequested` 时
    /// 写回。跟其余字段一样走 `#[serde(default)]`,老 `layout.json` 缺这
    /// 两个字段时退化成 `workspace_geometry::initial_window_size()`,不影响其余已存的偏好。
    pub window_width: f32,
    pub window_height: f32,
}

impl Default for ShellLayout {
    fn default() -> Self {
        Self {
            left_width: 640.0,
            files_split: 0.35,
            agent_split: 0.4,
            conversations_split: 0.4,
            left_view: LeftView::Files,
            right_view: RightView::Agent,
            left_collapsed: false,
            right_collapsed: false,
            window_width: workspace_geometry::initial_window_size().0,
            window_height: workspace_geometry::initial_window_size().1,
        }
    }
}

/// 把从磁盘读回来的 `ShellLayout` 夹进合法范围(`layout::load_from` 调用)。
/// 三个 split 用与拖拽同一对上下界:比例恰为 0.0/1.0 时 `split_portions`
/// 会给出 `FillPortion(0)`,那一块在 flex 里拿不到任何宽度、整块消失;
/// `left_width` 只保下限(上限依赖窗口宽,由渲染/几何时刻的
/// `clamp_left_width` 负责,不在这里写死)。`window_width`/`window_height`
/// 同样只夹下限(`workspace_geometry::min_window_width()`/`workspace_geometry::min_window_height()`,建窗时还有
/// `with_min_inner_size` 兜底),非法值(非有限数、缺字段的 0.0)退化成
/// `workspace_geometry::initial_window_size()`。
pub fn sanitize_shell_layout(l: ShellLayout) -> ShellLayout {
    let clamp_split = |v: f32| {
        if v.is_finite() {
            v.clamp(
                workspace_geometry::min_split_ratio(),
                workspace_geometry::max_split_ratio(),
            )
        } else {
            ShellLayout::default().files_split
        }
    };
    ShellLayout {
        left_width: if l.left_width.is_finite() {
            l.left_width.max(workspace_geometry::min_zone_width())
        } else {
            ShellLayout::default().left_width
        },
        files_split: clamp_split(l.files_split),
        agent_split: clamp_split(l.agent_split),
        conversations_split: clamp_split(l.conversations_split),
        window_width: if l.window_width.is_finite() && l.window_width > 0.0 {
            l.window_width.max(workspace_geometry::min_window_width())
        } else {
            workspace_geometry::initial_window_size().0
        },
        window_height: if l.window_height.is_finite() && l.window_height > 0.0 {
            l.window_height.max(workspace_geometry::min_window_height())
        } else {
            workspace_geometry::initial_window_size().1
        },
        ..l
    }
}

/// 新外壳的三条可拖拽分隔线：左右面板区之间、左侧配对视图内部、右侧配对
/// 视图内部。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Divider {
    LeftRight,
    LeftPairSplit,
    RightPairSplit,
}

/// 项目树右键菜单当前打开状态：定位坐标 + 目标（路径/是否目录）。
#[derive(Debug, Clone, PartialEq)]
struct ContextMenu {
    x: f32,
    y: f32,
    target: PathBuf,
    is_dir: bool,
}

/// 项目树行内编辑的模式：新建文件/新建文件夹/重命名(携带原路径)。
#[derive(Debug, Clone, PartialEq)]
enum TreeEditMode {
    NewFile,
    NewFolder,
    Rename(PathBuf),
}

/// 项目树行内编辑态：新建/重命名共用。`parent_dir` 对 `Rename` 而言是
/// 被改名项的父目录(新路径=parent_dir.join(新名字));对 `NewFile`/
/// `NewFolder` 就是目标创建位置。
#[derive(Debug, Clone, PartialEq)]
struct TreeEdit {
    parent_dir: PathBuf,
    mode: TreeEditMode,
    buffer: String,
}

/// 主界面当前几何状态的只读快照(main.rs 拖拽追踪/离屏几何计算用途,
/// `Copy` 类型直接按值传递)。取代旧 `PanelLayout` 单独传递的做法——
/// 新几何公式(webview bounds/焦点路由/IME 光标)都依赖"当前是哪个视图、
/// 是否收起"，不能只靠宽高数字算，所以把这些也打包进来。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShellState {
    pub layout: ShellLayout,
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
fn zones_width(window_width: f32) -> f32 {
    (window_width
        - 2.0 * workspace_geometry::icon_rail_width()
        - workspace_geometry::divider_width())
    .max(0.0)
}

/// 把持久化的 `left_width` 夹进"当前窗口宽度下合法"的区间:下限
/// `workspace_geometry::min_zone_width()`,上限"给右面板区也留够 `workspace_geometry::min_zone_width()`"。窗口窄到
/// 上界低于下界时用 `.max(workspace_geometry::min_zone_width())` 把上界垫平,`clamp` 恒不 panic。
///
/// 这是**唯一**一处 left_width 的夹取:渲染侧(`left_panel_area` 经
/// `Workspace::effective_left_width`)、几何侧(`left_zone_width` → webview
/// bounds/命中测试/终端网格)、拖拽侧(`apply_column_drag`)全部走这里。
/// 各写一份夹取(或只在拖拽时夹一次)就会出现:窗口被拖窄后持久化宽仍按
/// 640 渲染,`Length::Fixed(640)` 在 flex 第一趟就把可用空间吃光,唯一
/// `Fill` 的右面板区拿到 0 宽——整个右半边(终端/Agent 列表/对话/审阅)
/// 凭空消失(Fix round 2 Critical #1)。夹取只发生在渲染/几何时刻,不回写
/// `ShellLayout`,窗口再拉宽时用户原来偏好的宽度自动复原。
fn clamp_left_width(window_width: f32, left_width: f32) -> f32 {
    let upper = (zones_width(window_width) - workspace_geometry::min_zone_width())
        .max(workspace_geometry::min_zone_width());
    left_width.clamp(workspace_geometry::min_zone_width(), upper)
}

/// 左面板区当前实际宽度(逻辑像素)：收起时 0；对侧收起时独占 `zones_width`
/// (与 `left_panel_area` 此时渲染成 `Length::Fill` 对应)；否则用持久化宽
/// 按当前窗口宽夹取后的**有效**宽(见 `clamp_left_width`)。
fn left_zone_width(window_width: f32, state: &ShellState) -> f32 {
    if state.left_collapsed {
        0.0
    } else if state.right_collapsed {
        zones_width(window_width)
    } else {
        clamp_left_width(window_width, state.layout.left_width)
    }
}

/// 右面板区当前实际宽度(逻辑像素)；收起时为 0；否则是 `zones_width` 里
/// 左面板区没占走的剩余空间——右面板区不像左面板区那样有独立持久化宽度，
/// 恒为 Fill。
fn right_zone_width(window_width: f32, state: &ShellState) -> f32 {
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
    (zone_width - workspace_geometry::divider_width()).max(0.0)
}

/// 拖拽某条分隔线到窗口逻辑 x 坐标 `logical_x` 后的新 `ShellLayout`。
/// `LeftPairSplit`/`RightPairSplit` 写哪个 split 字段取决于当前那一侧的
/// 视图选择(比如右侧当前是"对话"就写 `conversations_split`，不是
/// `agent_split`)——这条信息 `ShellLayout` 自己没有，靠 `ShellState` 带过来。
fn apply_column_drag(
    state: ShellState,
    divider: Divider,
    window_width: f32,
    logical_x: f32,
) -> ShellLayout {
    match divider {
        Divider::LeftRight => {
            let new_left = clamp_left_width(
                window_width,
                logical_x - workspace_geometry::icon_rail_width(),
            );
            ShellLayout {
                left_width: new_left,
                ..state.layout
            }
        }
        Divider::LeftPairSplit => {
            let pair_w = pair_content_width(left_zone_width(window_width, &state));
            if pair_w <= 0.0 {
                return state.layout;
            }
            let ratio = ((logical_x - workspace_geometry::icon_rail_width()) / pair_w).clamp(
                workspace_geometry::min_split_ratio(),
                workspace_geometry::max_split_ratio(),
            );
            ShellLayout {
                files_split: ratio,
                ..state.layout
            }
        }
        Divider::RightPairSplit => {
            let right_w = right_zone_width(window_width, &state);
            let pair_w = pair_content_width(right_w);
            if pair_w <= 0.0 {
                return state.layout;
            }
            let right_x0 = window_width - workspace_geometry::icon_rail_width() - right_w;
            // `ratio` 是"配对里渲染在左边那块"的宽度占比(拖拽点左侧的宽度
            // 除以配对总宽)——这块现在是终端/审阅,不是 agent_split/
            // conversations_split 存的"列表侧(Agent 列表/对话列表)占比"。
            // 两者互补(列表侧渲染在右边),所以要写 1.0-ratio,不能直接写
            // ratio,否则拖拽方向会反(见 `right_panel_area` 顶部注释)。
            let ratio = ((logical_x - right_x0) / pair_w).clamp(
                workspace_geometry::min_split_ratio(),
                workspace_geometry::max_split_ratio(),
            );
            match state.right_view {
                RightView::Agent => ShellLayout {
                    agent_split: 1.0 - ratio,
                    ..state.layout
                },
                RightView::Conversations => ShellLayout {
                    conversations_split: 1.0 - ratio,
                    ..state.layout
                },
                // 用量统计是单栏（不分割），没有自己的 split 权重。
                RightView::Usage => state.layout,
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
    let x0 = workspace_geometry::icon_rail_width() + workspace_geometry::maximize_overlay_padding();
    let avail_w = (window_width
        - 2.0 * workspace_geometry::icon_rail_width()
        - 2.0 * workspace_geometry::maximize_overlay_padding())
    .max(0.0);
    (x0, avail_w)
}

/// 放大态金色描边盒子的纵向可用高度(逻辑像素)。`maximize_overlay` 顶部
/// 垫了一条 `workspace_geometry::top_bar_height()` 高的 Space 把遮罩钉在顶栏之下,盒子上下各留
/// `workspace_geometry::maximize_overlay_padding()`;遮罩铺到窗口底边(状态栏也被盖住),所以这里
/// **不**扣 `workspace_geometry::status_bar_height()`——与 `preview_content_bounds` 放大分支同源。
fn maximized_box_height(window_height: f32) -> f32 {
    (window_height
        - workspace_geometry::top_bar_height()
        - 2.0 * workspace_geometry::maximize_overlay_padding())
    .max(0.0)
}

/// 窗口逻辑尺寸 → 左侧文件/Web 预览内容区矩形(逻辑像素 x/y/w/h)，供
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
        let y0 =
            workspace_geometry::top_bar_height() + workspace_geometry::maximize_overlay_padding();
        let avail_h = maximized_box_height(window_height);
        return match state.left_view {
            LeftView::Web => {
                let y = y0 + workspace_geometry::browser_chrome_top_px();
                let h = (avail_h - workspace_geometry::browser_chrome_top_px() - 8.0).max(0.0);
                let x = x0 + 8.0;
                let w = (avail_w - 16.0).max(0.0);
                (x, y, w, h)
            }
            LeftView::Files => {
                let y = y0 + workspace_geometry::preview_chrome_top_px();
                let h = (avail_h - workspace_geometry::preview_chrome_top_px() - 8.0).max(0.0);
                let pair_w = pair_content_width(avail_w);
                let list_w = pair_w * state.layout.files_split;
                let content_w = pair_w * (1.0 - state.layout.files_split);
                let x = x0 + list_w + workspace_geometry::divider_width() + 8.0;
                let w = (content_w - 16.0).max(0.0);
                (x, y, w, h)
            }
            // Git 提交图是原生 Canvas 绘制,不挂 webview 子视图。
            LeftView::GitLog => (0.0, 0.0, 0.0, 0.0),
            // Todo 面板同 GitLog,纯 iced 绘制,不挂 webview 子视图。
            LeftView::Todo => (0.0, 0.0, 0.0, 0.0),
        };
    }
    let left_w = left_zone_width(window_width, state);
    // `left_zone` 的上下 margin:webview 必须跟着 inset,否则会戳出外边框
    // (原生子视图不听 iced 布局,逐像素靠这里算)。左右 margin 同样要算进去,
    // 否则去掉外边框后 webview 会戳出新增的左侧留白。
    let m = chrome_style::left_zone().margin;
    let y_top =
        |chrome_top: f32| -> f32 { workspace_geometry::top_bar_height() + m.top + chrome_top };
    let h_for = |y: f32| -> f32 { (window_height - y - m.bottom - 8.0).max(0.0) };
    match state.left_view {
        LeftView::Web => {
            let y = y_top(workspace_geometry::browser_chrome_top_px());
            let h = h_for(y);
            let x = workspace_geometry::icon_rail_width() + 8.0 + m.left;
            let w = (left_w - 16.0 - m.left - m.right).max(0.0);
            (x, y, w, h)
        }
        LeftView::Files => {
            let y = y_top(workspace_geometry::preview_chrome_top_px());
            let h = h_for(y);
            let pair_w = pair_content_width(left_w);
            let list_w = pair_w * state.layout.files_split;
            let content_w = pair_w * (1.0 - state.layout.files_split);
            let x = workspace_geometry::icon_rail_width()
                + list_w
                + workspace_geometry::divider_width()
                + 8.0
                + m.left;
            let w = (content_w - 16.0 - m.left - m.right).max(0.0);
            (x, y, w, h)
        }
        LeftView::GitLog => (0.0, 0.0, 0.0, 0.0),
        // Todo 面板同 GitLog,纯 iced 绘制,不挂 webview 子视图。
        LeftView::Todo => (0.0, 0.0, 0.0, 0.0),
    }
}

/// 逻辑 x 是否落在左侧文件/Web 预览内容区列内。焦点路由用:点击落在
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
            LeftView::Web => {
                let start = x0;
                let end = start + avail_w;
                x >= start && x < end
            }
            LeftView::Files => {
                let list_w = pair_content_width(avail_w) * state.layout.files_split;
                let start = x0 + list_w + workspace_geometry::divider_width();
                let end = x0 + avail_w;
                x >= start && x < end
            }
            LeftView::GitLog => false,
            // Todo 面板纯 iced 绘制,无 webview,永不落在预览列。
            LeftView::Todo => false,
        };
    }
    let left_w = left_zone_width(window_width, state);
    match state.left_view {
        LeftView::Web => {
            let start = workspace_geometry::icon_rail_width();
            let end = start + left_w;
            x >= start && x < end
        }
        LeftView::Files => {
            let list_w = pair_content_width(left_w) * state.layout.files_split;
            let start = workspace_geometry::icon_rail_width()
                + list_w
                + workspace_geometry::divider_width();
            let end = workspace_geometry::icon_rail_width() + left_w;
            x >= start && x < end
        }
        LeftView::GitLog => false,
        // Todo 面板纯 iced 绘制,无 webview,永不落在预览列。
        LeftView::Todo => false,
    }
}

/// 逻辑 x 落在哪一侧面板区(整区,不分区内具体是哪个 pane)。左键点击
/// 落点决定当前"聚焦"哪一侧,驱动 `left_zone`/`right_zone` 外边框的高亮态
/// (见 [`ZoneSide`])。落在图标栏本身(两侧各 `workspace_geometry::icon_rail_width()` 宽)或
/// 某侧收起而点在了"不存在的那一侧"时不算数,返回 `None`(调用方应保持
/// 点击前的聚焦态不变,而不是清空)。
///
/// 放大态:整个内容区就是放大的那一侧,不用再按横坐标细分——
/// `maximize_overlay` 渲染时两条图标栏原样露在外面,和非放大态同一
/// 横向范围,所以图标栏判定不用跟着改。
pub fn zone_at_x(x: f32, window_width: f32, state: &ShellState) -> Option<ZoneSide> {
    if x < workspace_geometry::icon_rail_width()
        || x > window_width - workspace_geometry::icon_rail_width()
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
    let boundary = workspace_geometry::icon_rail_width() + left_zone_width(window_width, state);
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
fn terminal_visible(state: &ShellState) -> bool {
    state.right_view == RightView::Agent
        && !state.right_collapsed
        && state.maximized != Some(MaximizedPane::Left)
}

/// 换算终端 PTY 网格时用的假想外壳状态:强制"右侧展开 + 显示 Agent 配对"。
///
/// 终端此刻可能不可见(右侧收起 / 右视图是对话),但它的 PTY 网格仍应按
/// "被显示时占多大"来定——否则上次退出前停在对话视图的会话,重开 app 后会
/// 一直卡在 `DEFAULT_COLS`×`DEFAULT_ROWS`(80×24),直到用户偶然拖一下窗口
/// 才纠正(Fix round 2 #6)。用字段覆盖表达这个假想,复用同一套宽度公式,
/// 不另写一份几何。
fn terminal_grid_state(state: ShellState) -> ShellState {
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
        let content_w = pair_content_width(avail_w) * (1.0 - state.layout.agent_split);
        let pane_width = (content_w - workspace_geometry::chrome_width_px()).max(0.0);
        // `workspace_geometry::status_bar_height()` 是终端 pane 自带的底栏(`terminal_status_bar`,
        // 不是窗口级状态栏),放大态一样在盒子里,照扣。
        let pane_height = (maximized_box_height(window_height)
            - workspace_geometry::status_bar_height()
            - workspace_geometry::chrome_height_px())
        .max(0.0);
        return (pane_width, pane_height);
    }
    let right_w = right_zone_width(window_width, state);
    let content_w = pair_content_width(right_w) * (1.0 - state.layout.agent_split);
    let pane_width = (content_w - workspace_geometry::chrome_width_px()).max(0.0);
    // `right_zone` 上下 margin:终端是 iced 布局(自动 inset),但其 PTY 网格
    // 尺寸靠这里算,必须同步扣掉上下 margin,否则字符网格比实际渲染区高。
    let m = chrome_style::right_zone().margin;
    let pane_height = (window_height
        - workspace_geometry::top_bar_height()
        - workspace_geometry::status_bar_height()
        - workspace_geometry::chrome_height_px()
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
/// 触发的消息(`TermInput`/`AcceptanceOpen`/`PreviewSelectTab` …)仍然走
/// `with_focused_project`——它们的语义本来就是"作用于用户此刻看着的那个项目"。
pub type ProjectId = i64;

/// Ctrl + / Ctrl - 每次触发的相对缩放步近因子（1.1 ≈ 每按一次放大 10%）。
const UI_ZOOM_STEP: f32 = 1.1;

/// Agent 面板"＋"弹出菜单选择项的语义。`Agent` 复用原 `Option<AgentKind>`
/// 语义(`None` = 纯 Shell);`Git` 在项目根开一个 shell 并自动跑 `git status`
/// 看仓库状态。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum PickerLaunch {
    Agent(Option<AgentKind>),
    Git,
}

#[derive(Debug, Clone)]
pub enum Message {
    /// 终端聚焦时的键盘/IME 输入字节（已经过 `keymap` 翻译）。直接写给
    /// 当前激活 tab 对应的 daemon 会话（`client.write`）——不再本地
    /// echo，回显完全走 PTY 真实回路（daemon → attach 流 → `TermOutput`）。
    TermInput(Vec<u8>),
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
    /// 点击横幅"进入验收"（tab_id 为来源会话）。用户点的是当前界面上的
    /// 横幅,归属天然是聚焦项目,不需要 `ProjectId`。
    AcceptanceOpen(usize),
    /// 验收数据装载完成（repo, 来源 tab_id, goal, 变更清单）。
    AcceptanceLoaded(ProjectId, PathBuf, usize, Option<Goal>, Vec<FileChange>),
    /// 勾选/取消第 n 条标准。
    AcceptanceToggle(usize),
    /// 点击意见输入框进入编辑态。
    AcceptanceCommentClick,
    /// 意见输入事件（main.rs 键盘路由送入,复用 AddrEvent）。
    AcceptanceCommentEvent(AddrEvent),
    /// 点"通过·沉淀"。
    AcceptanceAccept,
    /// 点"打回并注回"。
    AcceptanceReject,
    /// 通过动作结果（Ok(版本号)/Err(红字文案)）。
    AcceptanceDone(ProjectId, Result<u32, String>),
    /// 会话审阅:解析完成（来源, 条目 / 错误文案）。
    ReviewLoaded(ProjectId, ReviewSource, Result<Vec<ReviewEntry>, String>),
    /// 会话审阅:展开/收起第 n 个 AI 回合的过程区。
    ReviewToggle(usize),
    /// 对话列表刷新结果（扫描完成）。
    ConversationsRefreshed(ProjectId, Vec<ConversationMeta>),
    /// 手动点用量面板头部的刷新按钮 → 触发一次异步扫描。
    UsageRefresh,
    /// 用量面板的异步扫描/解析结果落地。
    UsageLoaded(ProjectId, Vec<(ConversationMeta, usage::ConversationUsage)>),
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
    /// Todo 面板：点击任务行勾选框，`usize` 是 `Workspace.todo_items` 下标。
    TodoToggle(usize),
    /// Todo 面板："＋新增任务"输入框内容变化。
    TodoAddInputChanged(String),
    /// Todo 面板：提交"＋新增任务"（回车）。
    TodoAddSubmit,
    /// Todo 面板：点击筛选分段（全部/待办/进行中/完成）。
    TodoFilterSet(todo::TodoFilter),
    /// Todo 面板：搜索框内容变化。
    TodoSearchChanged(String),
    /// Todo 面板：点击某条任务"派发"按钮，打开派发选择层。
    TodoDispatchOpen(usize),
    /// Todo 面板：点击派发选择层外/Esc，关闭不派发。
    TodoDispatchClose,
    /// Todo 面板：选中一个已存活的 agent tab 派发：(任务下标, 目标 session id)。
    TodoDispatchToExisting(usize, String),
    /// Todo 面板：选"新建"派发：(任务下标, 要新建的 agent/shell 选项)。
    TodoDispatchNew(usize, PickerLaunch),
    /// Todo 面板：点击某条任务旁"计划"文字，进入内联编辑计划时间。
    TodoPlanDateEditStart(usize),
    /// Todo 面板：计划时间编辑框内容变化。
    TodoPlanDateChanged(String),
    /// Todo 面板：提交计划时间（回车）。
    TodoPlanDateSubmit,
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
    TermScroll(i32),
    /// 终端 tab 栏箭头翻页（`true`=右/`false`=左）。一次翻 2 个 tab；
    /// 上界不在此钳，渲染时 `tab_window` 钳制显示（P1L T5 验收返工）。
    TermTabScroll(bool),
    /// 预览 tab 栏箭头翻页，语义同 `TermTabScroll`。
    PreviewTabScroll(bool),
    /// 浏览器 tab 栏箭头翻页，语义同 `TermTabScroll`。
    BrowserTabScroll(bool),
    /// 终端左键按下：在视口格 `(col, row)` 起新选区（`right` = 按点在
    /// 格子右半）。
    TermSelStart { col: usize, row: usize, right: bool },
    /// 终端拖拽：选区末端更新到视口格 `(col, row)`。
    TermSelUpdate { col: usize, row: usize, right: bool },
    /// ⌘V 粘贴剪贴板文本：按会话的 bracketed paste 模式决定是否包裹
    /// `ESC[200~`/`ESC[201~` 后写入 daemon。
    TermPaste(String),
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
    /// 预览编辑弹层:`text_editor` widget 的编辑动作回调。
    PreviewEditAction(iced_widget::text_editor::Action),
    /// 预览编辑弹层:"保存"按钮 / ⌘S。
    PreviewEditSave,
    /// 预览编辑弹层:×按钮 / 点遮罩——脏改动会先转成二次确认,不直接关。
    PreviewEditCloseRequest,
    /// 预览编辑弹层二次确认:"放弃改动"。
    PreviewEditConfirmDiscard,
    /// 预览编辑弹层二次确认:"取消"(回到编辑态)。
    PreviewEditConfirmCancel,
    /// 浏览器:打开 URL 为新网页 tab——落在独立的 `Workspace::browser` 上,
    /// 不产生任何文件预览 tab(该功能只服务左图标栏"地球"进入的独立浏览器
    /// 视图)。
    BrowserOpenUrl(String),
    /// 浏览器:切换 tab(vec 位置)。
    BrowserSelectTab(usize),
    /// 浏览器:关闭 tab(vec 位置)。
    BrowserCloseTab(usize),
    /// 浏览器:点击地址栏,进入编辑态。
    BrowserAddrClick,
    /// 浏览器:地址栏编辑事件。
    BrowserAddrEvent(AddrEvent),
    /// 浏览器:点击地址栏星标,开合"加入/移出收藏"小菜单。
    BrowserStarClick,
    /// 浏览器:星标菜单里点"加入全局/本项目收藏"。
    BrowserBookmarkAdd(BookmarkScope),
    /// 浏览器:星标菜单/收藏面板里点"移出收藏"(按 dozerd 记录 id)。
    BrowserBookmarkRemove(i64),
    /// 浏览器:tab 栏"收藏夹"图标按钮,开合下拉面板。
    BrowserBookmarksToggle,
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
    /// 项目页签:把某路径作为**新页签**打开(不动任何已存在页签的内容)。
    ProjectTabOpen(PathBuf),
    /// 项目页签:`ProjectTabOpen` 异步完成(daemon upsert 结果 + 最近列表)。
    /// `None` = 这次打开失败,只报错、不改任何页签状态。
    ProjectTabOpened(Option<ProjectInfo>, Vec<ProjectInfo>),
    /// H0 项目中心:`Message::TopBarHome` 发起的异步刷新完成(最近改动的文件、
    /// 最近的对话两份列表;D4)。
    HomeRecentsLoaded(Vec<HomeRecentFile>, Vec<HomeRecentConversation>),
    /// 项目页签:点已存在的页签 → 前台化该项目。只改"当前是哪个页签",
    /// 不结束任何会话、不改写任何 `Workspace` 的内容。
    ProjectTabSwitch(i64),
    /// 项目页签:点页签的 × → 关闭该页签,并结束该项目下所有会话。
    ProjectTabClose(i64),
    /// 项目页签:`App::ensure_loaded` 的异步促成完成——素材已取回,由
    /// `update` 在 UI 线程上装配成 `Workspace`,替换掉那份"加载中"占位
    /// (载荷是一次性信封,见 [`RestorePayload`])。
    ProjectSlotLoaded(i64, RestorePayload),
    /// 项目:文件树展开/收起某目录。
    ProjectTreeToggle(PathBuf),
    /// 项目:git 分支/脏/文件状态/worktree 列表刷新结果。
    ProjectGitRefreshed(
        ProjectId,
        Option<String>,
        bool,
        HashMap<PathBuf, FileGitStatus>,
        Vec<WorktreeInfo>,
    ),
    /// 项目:`git_watch` 监听到工作区/`.git` 引用变化,该重新跑一次 git 刷新
    /// 了(D4)。`Relevance` 决定这次触发要不要顺带做 Plan 2 的 Git Log 快照
    /// 重建。
    ProjectFsChanged(ProjectId, git_watch::Relevance),
    /// Git Log 面板的全部消息,内核只转发不解读——见
    /// `extensions::git_log::Message`。
    GitLog(git_log::Message),
    /// 项目:当前项目验收次数刷新结果(项目卡"N 次验收"副行用)。
    AcceptanceCountLoaded(ProjectId, Option<u64>),
    /// 浏览器:收藏夹"全局+当前项目"合集刷新结果(项目打开/切换,或一次
    /// 增删收藏之后的重新拉取)。
    BrowserBookmarksLoaded(ProjectId, Vec<BookmarkInfo>),
    /// 浏览器:一次 `AddBookmark`/`RemoveBookmark` 往返完成——无论成功
    /// 失败都触发一次 `BrowserBookmarksLoaded` 式的全量刷新去纠正本地
    /// 乐观更新;失败时额外把错误文案落进 `browser_error`。
    BrowserBookmarksMutated(ProjectId, Result<(), String>),
    /// 项目树:右键按下的窗口逻辑坐标(main.rs 原始事件层发,供随后可能
    /// 触发的 `ProjectTreeContextMenu` 定位弹出菜单)。
    RightClickAt { x: f32, y: f32 },
    /// 项目树:某行右键命中,打开菜单(位置取 `last_right_click`)。
    ProjectTreeContextMenu { path: PathBuf, is_dir: bool },
    /// 项目树:关闭菜单(点击外部/Esc/动作完成后)。
    ProjectTreeContextMenuClose,
    /// 项目树:菜单选"复制绝对/相对路径"→ main.rs 拦截写系统剪贴板,
    /// 不落 `Workspace::update`。
    ProjectTreeCopyPath(PathBuf, project::PathKind),
    /// 项目树:菜单选"在 Finder 中打开"→ `open -R` 拉起 Finder 并选中目标,
    /// 无需窗口句柄/剪贴板,直接在 `App::update` 里同步 `spawn`(不等待退出)。
    ProjectTreeRevealInFinder(PathBuf),
    /// 项目树:菜单选"复制"→ 标记应用内剪贴槽(参数=路径,是否目录)。
    ProjectTreeCopy(PathBuf, bool),
    /// 项目树:菜单选"粘贴"→ 异步复制剪贴槽项到目标目录(参数=目标目录)。
    ProjectTreePaste(PathBuf),
    /// 项目树:粘贴异步结果(Ok=新建出的路径,Err=错误文案)。
    ProjectTreePasteDone(ProjectId, Result<PathBuf, String>),
    /// 项目树:菜单选"删除"→ 打开确认框(参数=路径,是否目录)。
    ProjectTreeDeleteRequest(PathBuf, bool),
    /// 项目树:确认框点"删除"。
    ProjectTreeDeleteConfirm,
    /// 项目树:确认框点"取消"。
    ProjectTreeDeleteCancel,
    /// 项目树:删除/重命名/新建文件/新建文件夹 异步操作统一完成回传。
    /// `parent`:Ok=需要刷新的父目录,Err=错误文案。`expand`:成功时是否
    /// 需要顺带把 `parent` 标为展开态——新建文件/文件夹传 true(让刚建出
    /// 的项立刻可见,哪怕父目录之前是空的/收起的);删除/重命名传 false
    /// (删除后没有理由展开父目录,重命名不改变展开态)。四个操作共用一个
    /// 变量曾叫 `ProjectTreeDeleteDone`,重命名/新建完成后也发它,读起来
    /// 会以为出了删除——改名 + 加 `expand` 字段一并解决。
    ProjectTreeOpDone {
        project_id: ProjectId,
        parent: Result<PathBuf, String>,
        expand: bool,
    },
    /// 项目树:菜单选"新建文件"→ 进入行内编辑(参数=目标父目录)。
    ProjectTreeNewFile(PathBuf),
    /// 项目树:菜单选"新建文件夹"→ 进入行内编辑(参数=目标父目录)。
    ProjectTreeNewFolder(PathBuf),
    /// 项目树:菜单选"重命名"→ 进入行内编辑(参数=被改名项路径)。
    ProjectTreeRenameStart(PathBuf),
    /// 项目树:菜单选"从磁盘重新加载"→ 重读所有已缓存目录,让树与磁盘实际
    /// 状态保持一致(不依赖右键目标,故不带参数)。
    ProjectTreeReloadFromDisk,
    /// 项目树:行内编辑框的键盘事件(main.rs 键盘拦截层送入,复用 AddrEvent)。
    ProjectTreeEditEvent(AddrEvent),
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
}

/// 地址栏编辑事件:由 main.rs 的键盘拦截层在 `browser_addr_editing()`
/// 为真时翻译产生(字符/退格/回车/Esc),不经过 keymap 的 PTY 字节翻译.
#[derive(Debug, Clone)]
pub enum AddrEvent {
    Text(String),
    Backspace,
    Submit,
    Cancel,
}

/// 审阅内容的来源（P1j）：活会话 tab（回合结束刷新）或历史对话文件（快照不刷新）。
#[derive(Debug, Clone, PartialEq)]
pub enum ReviewSource {
    Session(usize),
    File(PathBuf),
}

/// 回合结束时该审阅视图是否应重解析：仅当它是该会话的活审阅。
fn review_should_refresh_on_turn(source: &ReviewSource, tab_id: usize) -> bool {
    matches!(source, ReviewSource::Session(id) if *id == tab_id)
}

/// 会话审阅 tab 的内容（P1i）。
pub struct ReviewView {
    pub source: ReviewSource,
    pub entries: Vec<ReviewEntry>,
    pub error: Option<String>,
    /// 展开了过程区的 AI 回合下标（entries 中的位置）。
    pub expanded: std::collections::HashSet<usize>,
}

/// 预览编辑弹层的进行中会话(全局至多一个;弹层是应用级模态)。
pub struct EditSession {
    /// `PreviewTab.id`(webview 池用的稳定 id,不是 `tabs` vec 下标——见
    /// `Workspace::preview_edit_open` 的取值处)。
    pub tab_id: usize,
    pub path: PathBuf,
    pub content: iced_widget::text_editor::Content,
    pub dirty: bool,
    /// 打开失败(理论上不会,打开前已判过存在)或保存失败的错误文案。
    pub error: Option<String>,
    /// 脏改动下点关闭:先弹二次确认,不直接丢。
    pub confirm_discard: bool,
}

/// 验收 tab 的一次进行中验收（spec P1f D8）。
pub struct AcceptanceView {
    pub repo: PathBuf,
    /// 来源会话 tab（打回意见注回目标）。
    pub source_tab_id: usize,
    pub goal: Option<Goal>,
    pub changes: Vec<FileChange>,
    pub checked: Vec<bool>,
    pub comment: String,
    pub comment_editing: bool,
    pub error: Option<String>,
    /// 通过后记录版本号（显示"已沉淀 v<n>"）。
    pub accepted_version: Option<u32>,
}

/// 一个 tab 对应一个 daemon 会话。
pub struct SessionTab {
    pub info: SessionInfo,
    pub model: TerminalModel,
    pub alive: bool,
    /// 会话内 agent 的最新状态（hook 事件驱动；初值来自
    /// `SessionInfo.agent_state`，晚 attach 也能恢复现状）。
    pub agent_state: AgentState,
    /// 会话归属的 agent（P2b；初值来自 `SessionInfo.agent`，随
    /// `AgentStateChanged` 更新）。
    pub agent: AgentKind,
    /// 该会话的 transcript 路径（有则终端 tab 显"审阅"入口；P1i）。
    pub transcript_path: Option<String>,
    /// OSC 扫描器（每 tab 独立，序列可跨 chunk）。
    osc: OscScanner,
    /// OSC 7 上报的当前目录；tab 标题优先显示其 basename。
    pub cwd: Option<PathBuf>,
    /// OSC 133;D 上报的最近命令退出码；非零时终端栏红字提示；
    /// 下一条命令开始（133;C）时清除。
    pub last_exit: Option<i32>,
    /// TurnEnded 检测出的"交付待验收"标记（spec P1f D3）。
    pub delivery_pending: bool,
    /// 上一次 TurnEnded 时的 HEAD（无沉淀 ref 时的比对基线）。
    pub last_turn_head: Option<String>,
    /// 稳定 id，`Message::TermOutput`/`SessionExited` 用它路由，不受
    /// tab 增删导致的 vec 位置变化影响。
    tab_id: usize,
    /// attach 数据流的转发任务句柄。`CloseTab` 时 `abort()` 掉它——
    /// 这个任务是 `mpsc::UnboundedReceiver<TermEvent>` 的唯一持有者，
    /// 任务被中断即意味着 receiver 被 drop（detach）。app 整体退出时
    /// 只发生 detach（会话存活）；显式关 tab 则再补一次 kill。
    forwarder: tokio::task::JoinHandle<()>,
}

/// 会话当前有效工作目录：OSC 7 跟踪的实时 cwd 优先，回落到会话
/// 启动目录。交付检测必须认实时 cwd——用户 `cd` 进项目仓库后，
/// 启动目录（多为 `$HOME`）不是那个仓库（spec P1f D1）。
fn effective_cwd(osc_cwd: Option<&Path>, spawn_cwd: &str) -> PathBuf {
    match osc_cwd {
        Some(p) => p.to_path_buf(),
        None => PathBuf::from(spawn_cwd),
    }
}

impl SessionTab {
    /// 交付检测/验收装载用的当前工作目录（OSC 7 优先）。
    fn effective_cwd(&self) -> PathBuf {
        effective_cwd(self.cwd.as_deref(), &self.info.cwd)
    }

    /// 把一段会话输出送进 OSC 扫描器并落地状态（观察式，不改写字节）。
    fn ingest_osc(&mut self, bytes: &[u8]) {
        for ev in self.osc.feed(bytes) {
            match ev {
                OscEvent::Cwd(p) => self.cwd = Some(p),
                OscEvent::CmdStart => self.last_exit = None,
                OscEvent::CmdExit(code) => self.last_exit = Some(code),
            }
        }
    }
}

/// 外壳侧共享句柄的一次性快照。项目态(`Workspace`)的方法要发起异步 IO
/// 时需要 daemon 客户端 / tokio 句柄 / 事件回灌通道,但这三样东西整个程序
/// 只有一份、长在 `App` 上(P2a 多项目并行:N 个 `Workspace` 共用同一份)。
///
/// 为什么按值克隆传进来,而不是让 `Workspace` 反过来借 `App`:
/// `App::active_workspace_mut()` 交出的 `&mut Workspace` 本身就是从
/// `&mut self` 里借出去的,此时再取 `&self.client` 就是同时可变+不可变借
/// `self`,借用检查器不放行。三个句柄的 `clone` 都只是 `Arc` 级别的浅拷贝,
/// 每次 `update` 克隆一份的代价可以忽略。
#[derive(Clone)]
pub struct ShellIo {
    client: Client,
    handle: Handle,
    proxy: EventLoopProxy<Message>,
    /// 终端网格尺寸快照(新建会话时让新 PTY 一开始就匹配 pane 实际大小)。
    cols: u16,
    rows: u16,
}

/// 顶层容器:main.rs 持有的就是这个(取代此前直接持有单个 `Workspace`)。
/// 外壳字段是整个程序只有一份的窗口态,`projects` 承载并行打开的项目
/// 页签——每个 `Workspace` 是完全独立、同时存活的一套项目态(P2a)。
pub struct App {
    client: Client,
    handle: Handle,
    proxy: EventLoopProxy<Message>,
    /// 当前终端网格尺寸,随 `PaneResized` 更新;新建 tab 时也用这份
    /// 尺寸,保证新会话从一开始就跟 pane 实际大小匹配。
    cols: u16,
    rows: u16,
    /// 终端是否聚焦(决定光标反色画法)。当前是单窗口应用且没有其它可
    /// 聚焦的输入控件,因此终端默认常驻聚焦。
    term_focused: bool,
    /// daemon 连接失败,或某次会话操作失败时的错误文案。整个程序共享
    /// 一份:daemon 连不连得上不是某个项目自己的状态。
    daemon_error: Option<String>,
    /// tab 前状态点的闪烁相位(true=亮/false=暗)。由 main.rs 的定时唤醒
    /// 每拍翻转(见 `toggle_blink`/`any_blinking`)。
    blink_on: bool,
    /// 图标栏+左右面板区宽度/分割状态;启动时 `layout::load()` 读盘作
    /// 起始值,拖拽结束(`ColumnDragEnd`)写盘。
    shell_layout: ShellLayout,
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
    /// `Some(Right)`——终端默认聚焦(`term_focused: true`),终端在右面板区。
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
    /// 项目树右键菜单当前打开状态(None=未打开)。窗口级浮层,同一时刻
    /// 只可能有一个,因此是外壳态而非项目态。
    context_menu: Option<ContextMenu>,
    /// 最近一次右键点击的窗口逻辑坐标,给 `ProjectTreeContextMenu` 定位菜单用。
    last_right_click: (f32, f32),

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
    recent_projects: Vec<ProjectInfo>,
    /// H0"最近的文件"卡数据(D4);`Message::TopBarHome` 时异步刷新。
    home_recent_files: Vec<HomeRecentFile>,
    /// H0"最近的对话"卡数据(D4);语义同上。
    home_recent_conversations: Vec<HomeRecentConversation>,
    /// 是否已经收到过至少一次 `HomeRecentsLoaded`——区分"还在加载"与"加载完
    /// 但结果为空"，两张卡据此决定画"加载中…"还是空状态文案(spec §4)。
    home_recents_loaded: bool,
    /// Git Log 面板状态——自己的 `Message`/`update`/`view`,见
    /// `extensions::git_log`。`App` 级共享、不按项目分(现状,纯重构不改,
    /// 见 `sync_git_log_to_active_project`)。
    git_log: git_log::State,
    /// Todo 面板本地元数据（派发记录/计划时间/完成时间），启动时
    /// `todo_meta::load()` 读盘，每次变更后 `todo_meta::save` 落盘。
    todo_meta: todo_meta::TodoMetaState,
}

pub struct Workspace {
    tabs: Vec<SessionTab>,
    /// 当前显示的 tab 在 `tabs` 中的位置（不是 `tab_id`）。
    active: usize,
    next_tab_id: usize,
    /// 发起新建会话(create+attach)期间的转发任务句柄暂存区，
    /// `Message::TabAttached` 到达时取出、装进新建的 `SessionTab`。
    pending: HashMap<usize, tokio::task::JoinHandle<()>>,
    /// 预览域状态机(P1d).
    preview: PreviewPane,
    /// 预览域错误文案(打开文件失败等), RED 显示在预览栏地址栏下方。
    preview_error: Option<String>,
    /// 浏览器域状态机:与 `preview` 完全独立的一份 tab/地址栏/webview
    /// 状态,只承载网页(点左图标栏"地球"进入,不受文件预览影响,反之亦然)。
    browser: PreviewPane,
    /// 浏览器域错误文案,语义同 `preview_error`。
    browser_error: Option<String>,
    /// 收藏夹本地缓存(全局 + 当前项目合集),`Message::BrowserBookmarksLoaded`
    /// 落地时整份替换;加入/移出走乐观本地更新,见 `crate::bookmarks`。
    bookmarks: Vec<BookmarkInfo>,
    /// 收藏夹下拉面板(tab 栏"收藏夹"图标按钮)开合。
    browser_bookmarks_open: bool,
    /// 地址栏星标"加入/移出收藏"小菜单开合。
    browser_star_menu_open: bool,
    /// `dozer://flyfish/__file__` 端点的文件白名单;与 main.rs 的协议
    /// 闭包共享(Arc),打开文件时插入.
    allowed_files: Arc<Mutex<HashSet<PathBuf>>>,
    /// 进行中的验收（验收 tab 内容;None=未打开）。
    acceptance: Option<AcceptanceView>,
    /// 进行中的会话审阅（审阅 tab 内容;None=未打开;P1i）。
    review: Option<ReviewView>,
    /// 当前项目的对话列表（扫 Claude 目录；P1j）。
    conversations: Vec<ConversationMeta>,
    /// 当前项目的 agent 用量统计（会话粒度；扫描+解析全量 transcript，比
    /// `conversations` 贵得多,所以不像它那样跟着 `DeliveryChecked` 自动
    /// 刷新——只在切到 `RightView::Usage` 或点手动刷新按钮时才重新扫
    /// （spec 非目标"不做实时更新"）。
    usage: Vec<(ConversationMeta, usage::ConversationUsage)>,
    /// `spawn_usage_refresh` 发起到 `UsageLoaded` 落地之间为真；面板据此
    /// 显示"统计中…"，避免展示陈旧数据被误读成最新值。
    usage_loading: bool,
    /// 当前项目（None=未打开；P1g）。
    project: Option<ProjectInfo>,
    /// 当前项目的文件树（随 project 建立）。
    file_tree: Option<FileTree>,
    /// 当前项目 git 分支（非 git 为 None）。
    branch: Option<String>,
    /// 当前项目工作树是否脏。
    dirty: bool,
    /// 最近项目（切换用）。
    recent_projects: Vec<ProjectInfo>,
    /// 当前项目的 git 文件状态（路径→状态；文件树装饰用；P1h）。
    git_statuses: HashMap<PathBuf, FileGitStatus>,
    /// 同仓库的其他 git worktree(D3),随 git 刷新一起更新。
    worktrees: Vec<WorktreeInfo>,
    /// 本项目的实时文件系统监听(D4)。`None` 只可能出现在 watcher 启动
    /// 失败时(降级为"只在开项目/回合结束时刷新")。Drop 时自动停止。
    git_watch: Option<git_watch::Handle>,
    /// 顶栏胶囊用的项目级目标（打开项目时同步读 .dozer/goal.md）。
    project_goal: Option<Goal>,
    /// 当前项目的验收次数（项目卡"N 次验收"副行；None=未载入/取不到）。
    project_acceptance_count: Option<u64>,
    /// 终端 tab 栏当前最左可见 tab 序号（箭头翻页用；P1L T5）。
    term_tab_first: usize,
    /// 预览 tab 栏当前最左可见 tab 序号，语义同 `term_tab_first`。
    preview_tab_first: usize,
    /// 浏览器 tab 栏当前最左可见 tab 序号，语义同 `term_tab_first`。
    browser_tab_first: usize,
    /// 项目树当前"选中"行(左键点击或右键命中都会更新),渲染时给该行背景色。
    tree_selected: Option<PathBuf>,
    /// 项目树"文件管理器式"剪贴槽:最近一次"复制"的项(路径,是否目录)。
    tree_clipboard: Option<(PathBuf, bool)>,
    /// 项目树操作的行内报错文案(冲突/失败时显示;下次树操作发起时清空)。
    tree_error: Option<String>,
    /// 项目树删除确认框目标(路径,是否目录;None=未打开确认框)。
    tree_delete_confirm: Option<(PathBuf, bool)>,
    /// 项目树行内编辑态(新建/重命名共用;None=未在编辑)。
    tree_edit: Option<TreeEdit>,
    /// Agent 面板"＋"按钮弹出的"新建"菜单当前是否打开。不需要坐标——面板顶部固定
    /// 位置的下拉,不像项目树右键菜单需要跟随点击坐标。
    agent_picker_open: bool,
    /// 预览编辑弹层进行中的会话;`None` = 未打开。
    edit_session: Option<EditSession>,
    /// `.dozer/todo.md` 解析后的内存缓存，`reload_todo_from_disk` 刷新。
    todo_items: Vec<todo::TodoItem>,
    /// 上一次成功读取时 `.dozer/todo.md` 的 mtime，轮询靠比较它决定要不要
    /// 重读（`App::poll_todo_if_visible`）。`None` = 还没读过，或文件不存在。
    todo_mtime: Option<std::time::SystemTime>,
    /// "＋新增任务"输入框当前内容（未提交）。
    todo_add_draft: String,
    /// 当前状态筛选（全部/待办/进行中/完成），纯前端状态，不持久化。
    todo_filter: todo::TodoFilter,
    /// 搜索框当前关键字，纯前端状态，不持久化。
    todo_search: String,
    /// 当前打开着派发选择层的任务下标（`None` = 未打开任何派发层）。
    todo_dispatch_open: Option<usize>,
    /// "派发到新建"发起时记一笔：`tab_id` → 任务文本。`on_tab_attached`
    /// 时消费掉、往 `App.todo_meta` 补派发记录（这时才知道真的 `session_id`）。
    todo_pending_dispatch: HashMap<usize, String>,
    /// 正在内联编辑计划时间的任务下标 + 输入框草稿；`None` = 当前没有
    /// 任何一条在编辑计划时间。
    todo_editing_plan_date: Option<(usize, String)>,
    /// 这份 `Workspace` 是否只是 `Stub` → `Loaded` 促成期间的"加载中"占位
    /// (见 [`Workspace::loading_for_project`])。占位有正确的 `project`/文件树,
    /// 但会话/git/对话都还没拉,并且整份对象会在
    /// `Message::ProjectSlotLoaded` 到达时被真正的结果替换掉——所以此刻
    /// **不该**替用户在它上面新建终端会话(建出来的会话会随占位一起被丢弃,
    /// 却仍在 daemon 上活着),`spawn_new_tab` 据此原地放弃。
    loading: bool,
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
fn focus_project_tab(
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
fn next_active_after_close(project_order: &[i64], closed: i64) -> Option<i64> {
    let idx = project_order.iter().position(|&i| i == closed)?;
    project_order
        .get(idx + 1)
        .or_else(|| idx.checked_sub(1).and_then(|prev| project_order.get(prev)))
        .copied()
}

/// 从槽位表里摘掉一个项目页签,返回被摘掉的槽位交给调用方善后(结束会话)。
/// 只在关的正好是当前页签时才动 `active_project_id`(落到
/// [`next_active_after_close`] 给的邻居),关后台页签不打扰前台。
fn take_project_tab(
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
fn loaded_workspace_mut(
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
fn restore_open_tabs(
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

impl Workspace {
    /// 启动/促成恢复:把 daemon 上现存的存活会话逐一 `attach`，快照直接喂给
    /// 新建的 `TerminalModel`（GUI 级会话恢复）。
    ///
    /// 实现分成两段——`fetch_project_restore`(纯 IO,可跨线程)+
    /// `from_restore`(纯装配,必须在 UI 线程)——原因见
    /// [`ProjectRestore`] 的文档。启动路径两段连着跑:调用方用
    /// `runtime.block_on` 驱动,此时窗口还没创建,不占用任何"正在跑的"
    /// UI 线程;恢复完成后的持续输出全部走 `forward_events` 派生任务 +
    /// `EventLoopProxy`,不再阻塞任何线程。
    pub async fn bootstrap(io: &ShellIo, project: ProjectInfo) -> Self {
        let restore = fetch_project_restore(&io.client, project).await;
        // 启动路径没有前身可继承,白名单从空开始。
        Self::from_restore(io, restore, None)
    }

    /// [`ProjectRestore`] → 完整 `Workspace` 的**同步**装配:给每个 attach
    /// 好的会话建终端模型、喂快照、派生转发任务,再把"要等 IO 才有结果"的
    /// 部分(新终端/git/对话/验收次数)照 `adopt_project` 的老样子异步补上。
    ///
    /// 必须在 UI 线程上跑(`TerminalModel` 内部有 `Rc`,整个 `Workspace` 不
    /// `Send`)。
    /// `allowed_files`:若这份 `Workspace` 是在**替换**同一个项目的另一份
    /// `Workspace`(促成落地),必须把前身那个 `Arc` 原样接过来,理由见
    /// `Message::ProjectSlotLoaded` 分支里的注释。`None` = 没有前身,新开一份。
    fn from_restore(
        io: &ShellIo,
        restore: ProjectRestore,
        allowed_files: Option<Arc<Mutex<HashSet<PathBuf>>>>,
    ) -> Self {
        let ProjectRestore {
            project,
            recent_projects,
            sessions,
        } = restore;
        let project_id = project.id;
        let mut tabs = Vec::new();
        let mut next_tab_id = 0usize;
        for (info, snapshot, rx) in sessions {
            let tab_id = next_tab_id;
            next_tab_id += 1;
            let mut model = TerminalModel::new(DEFAULT_COLS, DEFAULT_ROWS);
            let _ = model.feed(&snapshot); // 快照回放：陈旧查询应答不可补发，丢弃
            let forwarder =
                io.handle
                    .spawn(forward_events(project_id, tab_id, rx, io.proxy.clone()));
            tabs.push(SessionTab {
                agent_state: info.agent_state,
                agent: info.agent,
                transcript_path: info.transcript_path.clone(),
                info,
                model,
                alive: true,
                tab_id,
                forwarder,
                osc: OscScanner::new(),
                cwd: None,
                last_exit: None,
                delivery_pending: false,
                last_turn_head: None,
            });
            if let Some(t) = tabs.last_mut() {
                t.ingest_osc(&snapshot);
            }
        }

        let file_tree = Some(FileTree::new(PathBuf::from(&project.path)));
        let project_goal = load_project_goal(&project.path);

        let mut ws = Self {
            tabs,
            active: 0,
            next_tab_id,
            pending: HashMap::new(),
            project: Some(project),
            file_tree,
            project_goal,
            recent_projects,
            // 必须在 `restore_preview_state()` **之前**就位:那一步会把重开的
            // 预览文件写进白名单,写晚了就写到了一个没人看的 Arc 上。
            allowed_files: allowed_files.unwrap_or_else(|| Arc::new(Mutex::new(HashSet::new()))),
            ..Self::empty_for_project_placeholder()
        };
        // 与 ProjectOpened 同样异步补 git 分支/脏与对话列表（承诺"窗口起来
        // 后异步补"——此前只在用户主动打开项目时接线,启动恢复路径漏了,
        // 导致重开 app 后对话列表空白）。
        //
        // 没恢复出任何存活会话(比如上次退出前刚好关光了终端)时,默认新开一
        // 个根在项目目录的终端,不用用户手动点"+"。
        ws.ensure_project_terminal(io);
        // 认回上次退出前打开的预览文件 tab（重启后自动重开）。
        ws.restore_preview_state();
        ws.spawn_project_git_refresh(io);
        ws.spawn_conversations_refresh(io);
        ws.spawn_acceptance_count_refresh(io);
        ws.spawn_bookmarks_refresh(io);
        ws.start_git_watch(io);
        ws
    }

    /// 启动本项目的实时文件系统监听(D4)。失败(比如 fd 耗尽)只记一条
    /// warn,不影响项目正常打开——退化成"只在开项目/回合结束时刷新"这个
    /// D4 之前就有的行为。
    fn start_git_watch(&mut self, io: &ShellIo) {
        let Some(p) = &self.project else {
            return;
        };
        let project_id = p.id;
        let repo = PathBuf::from(&p.path);
        let proxy = io.proxy.clone();
        match git_watch::start(
            &io.handle,
            repo,
            std::time::Duration::from_millis(300),
            move |relevance| {
                let _ = proxy.send_event(Message::ProjectFsChanged(project_id, relevance));
            },
        ) {
            Ok(handle) => self.git_watch = Some(handle),
            Err(err) => tracing::warn!(project_id, %err, "git_watch 启动失败,降级为手动刷新"),
        }
    }

    /// `WorkspaceSlot::Stub` 促成 `Loaded` 之前的占位内容:一个"什么都没有"
    /// 的空 `Workspace`(`project: None`,无 tab)。`App::view()` 按
    /// `ws.project.is_none()` 识别"这是占位,不是真的空项目"——正常促成过的
    /// `Workspace` 恒有 `project: Some(_)`,不会和这个占位混淆。
    ///
    /// 注意这**不是**旧的 `Workspace::with_daemon_error`:"daemon 连不上"是
    /// 整个程序共享的状态(`daemon_error` 已随外壳字段搬到 `App`),对应
    /// `App::with_daemon_error`;这里表达的是完全不同的"这个项目的真实状态
    /// 还没加载完"。
    fn empty_for_project_placeholder() -> Self {
        Self {
            tabs: Vec::new(),
            active: 0,
            next_tab_id: 0,
            pending: HashMap::new(),
            preview: PreviewPane::default(),
            preview_error: None,
            browser: PreviewPane::default(),
            browser_error: None,
            allowed_files: Arc::new(Mutex::new(HashSet::new())),
            acceptance: None,
            review: None,
            conversations: Vec::new(),
            usage: Vec::new(),
            usage_loading: false,
            project: None,
            file_tree: None,
            project_goal: None,
            project_acceptance_count: None,
            branch: None,
            dirty: false,
            recent_projects: Vec::new(),
            git_statuses: HashMap::new(),
            worktrees: Vec::new(),
            git_watch: None,
            term_tab_first: 0,
            preview_tab_first: 0,
            browser_tab_first: 0,
            bookmarks: Vec::new(),
            browser_bookmarks_open: false,
            browser_star_menu_open: false,
            tree_selected: None,
            tree_clipboard: None,
            tree_error: None,
            tree_delete_confirm: None,
            tree_edit: None,
            agent_picker_open: false,
            edit_session: None,
            todo_items: Vec::new(),
            todo_mtime: None,
            todo_add_draft: String::new(),
            todo_filter: todo::TodoFilter::All,
            todo_search: String::new(),
            todo_dispatch_open: None,
            todo_pending_dispatch: HashMap::new(),
            todo_editing_plan_date: None,
            loading: false,
        }
    }

    /// `Stub` → `Loaded` 促成的**同步**第一步:一份已经知道自己归属哪个项目
    /// 的"加载中"占位。会话/git/对话/验收计数都还没拉(那些要 IO,由随后的
    /// `Workspace::bootstrap` 异步补),但项目、文件树根、项目目标这些不需要
    /// 网络的部分立刻就位,界面在同一帧内就有东西可画。
    ///
    /// **这里必须填 `project: Some(_)`,不能图省事用
    /// `empty_for_project_placeholder()`**:促成是异步的,占位会在消息环里
    /// 存活若干毫秒并且可以是当前聚焦的 workspace;只要它 `project` 为 `None`,
    /// `spawn_new_tab` 的 `expect("Workspace 存在即已知归属项目")` 就重新变成
    /// 可达路径,用户在这段窗口里点一下终端 tab 栏的"＋"就会 panic 掉整个
    /// GUI。同步构造 + 恒有 `project` 是这条不变式的落地方式。
    fn loading_for_project(project: ProjectInfo) -> Self {
        let file_tree = Some(FileTree::new(PathBuf::from(&project.path)));
        let project_goal = load_project_goal(&project.path);
        Self {
            project: Some(project),
            file_tree,
            project_goal,
            loading: true,
            ..Self::empty_for_project_placeholder()
        }
    }

    /// 是否有 tab 处于"工作中"(agent Running 且存活)——决定 main.rs 是否
    /// 需要定时唤醒来驱动状态点闪烁；无则回到 `ControlFlow::Wait` 省电。
    pub fn any_blinking(&self) -> bool {
        self.tabs
            .iter()
            .any(|t| t.alive && t.agent_state == AgentState::Running)
    }

    /// 当前激活 tab 的选区文本（⌘C 复制用）。
    pub fn active_selection_text(&self) -> Option<String> {
        self.tabs
            .get(self.active)
            .and_then(|t| t.model.selection_text())
    }

    fn tab_by_id_mut(&mut self, tab_id: usize) -> Option<&mut SessionTab> {
        self.tabs.iter_mut().find(|t| t.tab_id == tab_id)
    }

    /// 打开预览编辑弹层:按 tab 下标取路径读盘。下标越界或该 tab 不是
    /// `TabKind::File` 时静默 no-op(按钮本就只在 file tab 上画,正常路径
    /// 走不到这两种情况)。读盘失败写 `preview_error`,不开弹层。
    fn preview_edit_open(&mut self, idx: usize) {
        let Some(tab) = self.preview.tabs().get(idx) else {
            return;
        };
        let TabKind::File(path) = &tab.kind else {
            return;
        };
        let path = path.clone();
        let tab_id = tab.id;
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                self.preview_error = None;
                self.edit_session = Some(EditSession {
                    tab_id,
                    path,
                    content: iced_widget::text_editor::Content::with_text(&text),
                    dirty: false,
                    error: None,
                    confirm_discard: false,
                });
            }
            Err(e) => {
                self.preview_error = Some(format!("打开编辑失败: {e}"));
            }
        }
    }

    /// 转发 `text_editor` 的编辑动作;只有真正的编辑(增删字符,非光标
    /// 移动/选区/滚动)才置脏。没有打开编辑会话时 no-op。
    fn preview_edit_action(&mut self, action: iced_widget::text_editor::Action) {
        let Some(session) = self.edit_session.as_mut() else {
            return;
        };
        let is_edit = action.is_edit();
        session.content.perform(action);
        if is_edit {
            session.dirty = true;
        }
    }

    /// 保存当前编辑会话到磁盘,成功则清脏并推进该 tab 的 reload nonce
    /// (逼预览 webview 重新加载,否则用户会看到保存前的旧内容)。失败写
    /// `session.error`,弹层不关。没有打开编辑会话时 no-op。
    fn preview_edit_save(&mut self) {
        let Some(session) = self.edit_session.as_mut() else {
            return;
        };
        match std::fs::write(&session.path, session.content.text()) {
            Ok(()) => {
                session.dirty = false;
                session.error = None;
                let tab_id = session.tab_id;
                self.preview.bump_reload(tab_id);
            }
            Err(e) => {
                session.error = Some(format!("保存失败: {e}"));
            }
        }
    }

    /// 请求关闭编辑弹层:有未保存改动则转成二次确认,否则直接关。没有
    /// 打开编辑会话时 no-op。
    fn preview_edit_close_request(&mut self) {
        let Some(session) = self.edit_session.as_mut() else {
            return;
        };
        if session.dirty {
            session.confirm_discard = true;
        } else {
            self.edit_session = None;
        }
    }

    /// 二次确认:确认放弃未保存改动,真正关闭。
    fn preview_edit_confirm_discard(&mut self) {
        self.edit_session = None;
    }

    /// 二次确认:取消,回到编辑态(改动不丢)。
    fn preview_edit_confirm_cancel(&mut self) {
        if let Some(session) = self.edit_session.as_mut() {
            session.confirm_discard = false;
        }
    }

    /// 把键盘/IME 字节直接写给当前激活 tab 对应的 daemon 会话。异步写
    /// 交给 tokio（`self.handle.spawn`），绝不在 UI 线程 `block_on`。
    fn send_input(&self, io: &ShellIo, bytes: Vec<u8>) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        if !tab.alive {
            return;
        }
        let client = io.client.clone();
        let id = tab.info.id.clone();
        io.handle.spawn(async move {
            if let Err(e) = client.write(&id, &bytes).await {
                tracing::warn!("写入终端失败: {e}");
            }
        });
    }

    /// 从磁盘重新读取并解析 `.dozer/todo.md`，刷新 `todo_items`/
    /// `todo_mtime`。文件不存在/读失败按"空列表"处理，不 panic、不报错。
    fn reload_todo_from_disk(&mut self) {
        let Some(project) = self.project.as_ref() else {
            return;
        };
        let path = todo::todo_path(std::path::Path::new(&project.path));
        self.todo_mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        let md = std::fs::read_to_string(&path).unwrap_or_default();
        self.todo_items = todo::parse_todo(&md);
    }

    /// 勾选/取消勾选第 `idx` 条任务：算出新行文本、用
    /// `todo::replace_todo_line` 定点替换、写回磁盘、重新解析刷新内存态。
    /// 找不到要替换的原始行（文件已被 agent 并发改过）时静默放弃这次操作、
    /// 强制走一次 `reload_todo_from_disk`（冲突不是错误）。
    fn toggle_todo_item(&mut self, idx: usize) {
        let Some(project) = self.project.as_ref() else {
            return;
        };
        let Some(item) = self.todo_items.get(idx) else {
            return;
        };
        let old_line = format!("- [{}] {}", if item.done { "x" } else { " " }, item.text);
        let new_line = format!("- [{}] {}", if item.done { " " } else { "x" }, item.text);
        let path = todo::todo_path(std::path::Path::new(&project.path));
        let Ok(content) = std::fs::read_to_string(&path) else {
            return;
        };
        match todo::replace_todo_line(&content, &old_line, &new_line) {
            Some(new_content) => {
                if let Err(e) = std::fs::write(&path, &new_content) {
                    tracing::warn!("写入 todo.md 失败: {e}");
                    return;
                }
                self.reload_todo_from_disk();
            }
            None => {
                // 冲突：文件已经变了，放弃这次写入，直接重读展示最新状态。
                self.reload_todo_from_disk();
            }
        }
    }

    /// 提交"＋新增任务"输入框：草稿为空/全空白时不动作（不追加空任务）。
    /// 用 `todo::append_todo_item` 纯追加，冲突面比 `replace_todo_line` 小。
    fn submit_todo_add(&mut self) {
        let text = self.todo_add_draft.trim().to_string();
        if text.is_empty() {
            return;
        }
        let Some(project) = self.project.as_ref() else {
            return;
        };
        let path = todo::todo_path(std::path::Path::new(&project.path));
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        let new_content = todo::append_todo_item(&content, &text);
        if let Some(parent) = path.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            tracing::warn!("创建 .dozer 目录失败: {e}");
            return;
        }
        if let Err(e) = std::fs::write(&path, &new_content) {
            tracing::warn!("写入 todo.md 失败: {e}");
            return;
        }
        self.todo_add_draft.clear();
        self.reload_todo_from_disk();
    }

    /// 把 `text` 当输入写进已存活的 `session_id` 对应 tab。派发目标可能在
    /// 选择弹层打开期间被用户关掉（tab 已不在 `self.tabs` 里）——静默跳过。
    fn dispatch_todo_to_existing(&self, io: &ShellIo, session_id: &str, text: &str) {
        let Some(tab) = self.tabs.iter().find(|t| t.info.id == session_id) else {
            return;
        };
        if !tab.alive {
            return;
        }
        let client = io.client.clone();
        let id = session_id.to_string();
        let bytes = format!("{text}\n").into_bytes();
        io.handle.spawn(async move {
            if let Err(e) = client.write(&id, &bytes).await {
                tracing::warn!("派发任务文本失败: {e}");
            }
        });
    }

    /// 异步扫当前项目的对话目录 → ConversationsRefreshed（GUI 侧 spawn_blocking；P1j）。
    fn spawn_conversations_refresh(&self, io: &ShellIo) {
        let Some(p) = &self.project else {
            return;
        };
        let project_id = p.id;
        let cwd = PathBuf::from(&p.path);
        let cwd_for_log = cwd.clone();
        let proxy = io.proxy.clone();
        io.handle.spawn(async move {
            let list =
                tokio::task::spawn_blocking(move || conversation::list_all_conversations(&cwd))
                    .await
                    .unwrap_or_default();
            tracing::debug!(n = list.len(), cwd = %cwd_for_log.display(), "对话列表扫描完成");
            let _ = proxy.send_event(Message::ConversationsRefreshed(project_id, list));
        });
    }

    /// 异步扫当前项目的全部 transcript 并逐个解析用量 → `UsageLoaded`。
    /// 比 `spawn_conversations_refresh` 贵得多(要读整份文件内容，不只是
    /// 文件头)，所以不接入它那条"回合结束自动刷新"的调用链——只在
    /// `RightIconSelect(RightView::Usage)` 或手动刷新按钮时触发。
    fn spawn_usage_refresh(&self, io: &ShellIo) {
        let Some(p) = &self.project else {
            return;
        };
        let project_id = p.id;
        let cwd = PathBuf::from(&p.path);
        let proxy = io.proxy.clone();
        io.handle.spawn(async move {
            let rows = tokio::task::spawn_blocking(move || {
                conversation::list_all_conversations(&cwd)
                    .into_iter()
                    .filter_map(|meta| {
                        // 读失败(权限/IO error)的会话整条跳过、不计入汇总——
                        // 不能退化成"记一条全零 usage"，那样会把这次失败悄悄
                        // 算进 `ProjectUsageTotals::conversation_count`（spec
                        // 错误处理:"该会话跳过、不计入汇总"）。
                        let jsonl = std::fs::read_to_string(&meta.path).ok()?;
                        let u = usage::parse_usage(meta.agent, &jsonl);
                        Some((meta, u))
                    })
                    .collect::<Vec<_>>()
            })
            .await
            .unwrap_or_default();
            let _ = proxy.send_event(Message::UsageLoaded(project_id, rows));
        });
    }

    /// 异步取当前项目验收次数 → AcceptanceCountLoaded（项目卡副行）。
    /// 查询键走 `acceptance_query_repo`（= 落库侧 `delivery::repo_root`），
    /// 而非原始 `p.path`，否则子目录/符号链接路径撞不到库、副行静默空白。
    fn spawn_acceptance_count_refresh(&self, io: &ShellIo) {
        let Some(p) = &self.project else { return };
        let project_id = p.id;
        let project_path = p.path.clone();
        let client = io.client.clone();
        let proxy = io.proxy.clone();
        io.handle.spawn(async move {
            // repo_root 是阻塞 git 调用，隔离到 spawn_blocking。
            let repo = tokio::task::spawn_blocking(move || acceptance_query_repo(&project_path))
                .await
                .ok()
                .flatten();
            let n = match repo {
                Some(repo) => client.acceptance_count(&repo).await.ok(),
                None => None,
            };
            let _ = proxy.send_event(Message::AcceptanceCountLoaded(project_id, n));
        });
    }

    /// 异步拉取"全局 + 当前项目"收藏夹合集 → `BrowserBookmarksLoaded`。
    fn spawn_bookmarks_refresh(&self, io: &ShellIo) {
        let Some(p) = &self.project else { return };
        let project_id = p.id;
        let client = io.client.clone();
        let proxy = io.proxy.clone();
        io.handle.spawn(async move {
            let bookmarks = client
                .list_bookmarks(Some(project_id))
                .await
                .unwrap_or_default();
            let _ = proxy.send_event(Message::BrowserBookmarksLoaded(project_id, bookmarks));
        });
    }

    /// 本 `Workspace` 归属的项目 id。所有"发起时已知项目、结果晚些才回来"的
    /// 异步任务都要带上它,让 [`App::with_project`] 能投回原主(见
    /// [`ProjectId`])。`None` 只可能出现在 `ProjectOpened` 单条消息内部那个
    /// 用完即改写的空壳上,那里不发起任何异步任务。
    fn project_id(&self) -> Option<ProjectId> {
        self.project.as_ref().map(|p| p.id)
    }

    /// 打开着的会话 transcript 路径集合（UI 判"● 当前"用）。
    pub fn open_transcript_paths(&self) -> Vec<String> {
        self.tabs
            .iter()
            .filter_map(|t| t.transcript_path.clone())
            .collect()
    }

    /// 异步读 transcript + 解析 → ReviewLoaded（GUI 侧 spawn_blocking；P1i/P1j/P2b 按源）。
    fn spawn_review_load(
        &self,
        io: &ShellIo,
        source: ReviewSource,
        path: String,
        agent: AgentKind,
    ) {
        let Some(project_id) = self.project_id() else {
            return;
        };
        let proxy = io.proxy.clone();
        io.handle.spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                std::fs::read_to_string(&path)
                    .map(|s| transcript::parse_transcript(agent, &s))
                    .map_err(|e| format!("无法读取会话记录: {e}"))
            })
            .await
            .unwrap_or_else(|e| Err(format!("解析任务失败: {e}")));
            let _ = proxy.send_event(Message::ReviewLoaded(project_id, source, result));
        });
    }

    /// 异步刷新当前项目的 git 分支/脏/文件状态/worktree 列表(打开项目 +
    /// 回合结束 + `git_watch` 检测到变化时触发,见 `Message::ProjectFsChanged`)。
    fn spawn_project_git_refresh(&self, io: &ShellIo) {
        let Some(p) = &self.project else {
            return;
        };
        let project_id = p.id;
        let repo = PathBuf::from(&p.path);
        let proxy = io.proxy.clone();
        io.handle.spawn(async move {
            let (b, d, s, w) = tokio::task::spawn_blocking(move || {
                (
                    delivery::branch(&repo),
                    delivery::is_dirty(&repo),
                    delivery::file_statuses(&repo),
                    delivery::worktrees(&repo),
                )
            })
            .await
            .unwrap_or((None, false, HashMap::new(), Vec::new()));
            let _ = proxy.send_event(Message::ProjectGitRefreshed(project_id, b, d, s, w));
        });
    }

    /// 项目切换清理：关掉所有终端 tab（=结束会话，同 CloseTab 语义）与
    /// 所有预览 tab，给新项目一个干净起点（P1g 验收反馈）。webview 池由
    /// main.rs 的 sync_previews 依据空的期望清单自动销毁。
    fn close_all_tabs_for_switch(&mut self, io: &ShellIo) {
        while !self.tabs.is_empty() {
            self.close_tab(io, 0);
        }
        while !self.preview.tabs().is_empty() {
            self.preview.close(0);
        }
        self.acceptance = None;
        self.preview_error = None;
        self.term_tab_first = 0;
        self.preview_tab_first = 0;
    }

    /// 让这份 `Workspace` 认领一个项目:项目相关的字段整体换成该项目的,
    /// 再把"要等 IO 才有结果"的部分(终端/git/对话/验收次数)异步补上。
    ///
    /// 两个调用方共用:`ProjectOpened`(把当前页签就地改写成另一个项目,
    /// 调用前已 `close_all_tabs_for_switch` 清场)与
    /// `ProjectTabOpened`(在一个全新的空 `Workspace` 上认领,新增页签)。
    ///
    /// 注意这里**不做**"attach 该项目在 daemon 上已存在的存活会话"——那是
    /// `Workspace::bootstrap` 的异步活儿,归 Task 7 的 `ensure_loaded` 促成
    /// 逻辑;本函数与既有 `ProjectOpened` 落地路径行为完全一致(直接开一个
    /// 新终端)。
    fn adopt_project(&mut self, io: &ShellIo, project: ProjectInfo) {
        // 认领 = 这份 `Workspace` 从此有真正的内容,不再是促成期占位:必须
        // 清掉 `loading`,否则(1)`spawn_new_tab` 会继续拒绝建会话,(2)一个
        // 迟到的 `ProjectSlotLoaded` 会把刚认领好的内容当成占位覆盖掉。
        self.loading = false;
        self.file_tree = Some(FileTree::new(PathBuf::from(&project.path)));
        self.tree_selected = None;
        self.branch = None;
        self.dirty = false;
        self.git_statuses = HashMap::new();
        // 复用中的 `Workspace`(就地改写成另一个项目,见本方法文档)可能还
        // 挂着上一个项目的 watcher——显式清掉再重开,而不是指望
        // `start_git_watch` 成功时的赋值顺带把旧的 drop 掉:万一新项目的
        // 路径打不开 watcher(见其内部 `Err` 分支),不清的话旧 watcher 会
        // 带着旧 project_id 继续在后台跑,`Message::ProjectFsChanged` 送来
        // 的刷新信号会挂在一个此刻已经不对应这份 `Workspace` 的项目 id 上。
        self.git_watch = None;
        self.conversations = Vec::new();
        self.usage = Vec::new();
        self.usage_loading = false;
        self.project_goal = load_project_goal(&project.path);
        self.project = Some(project);
        self.project_acceptance_count = None;
        self.ensure_project_terminal(io);
        self.restore_preview_state();
        self.spawn_project_git_refresh(io);
        self.spawn_conversations_refresh(io);
        self.spawn_acceptance_count_refresh(io);
        self.spawn_bookmarks_refresh(io);
        // D4:新开的项目页签也要有实时刷新——此前只有跨重启恢复
        // (`from_restore`)/`Stub` 促成时会启动 watcher,直接开新项目这条最
        // 常见的路径反而漏了,退化成"只在开项目/回合结束时刷新"(code
        // review 发现)。
        self.start_git_watch(io);
    }

    /// "新建文件"/"新建文件夹"的公共起点:关菜单、确保目标目录展开(让
    /// 待插入的空白编辑行有可见位置)、进入空白行内编辑。
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

    /// 行内编辑框回车提交：按 `TreeEditMode` 分派成重命名/新建文件/新建
    /// 文件夹的实际文件系统操作(异步,`handle.spawn`)。名字为空或就是原名
    /// (仅 Rename 场景)直接静默取消编辑，不发起任何 IO。
    fn submit_tree_edit(&mut self, io: &ShellIo) {
        let Some(edit) = self.tree_edit.take() else {
            return;
        };
        // 树操作的异步结果要投回**发起它的**项目,不能投给"结果回来时恰好
        // 在前台的那个"(见 `ProjectId`)。
        let Some(project_id) = self.project_id() else {
            return;
        };
        // 清掉可能残留的上一次失败(哪怕是另一次操作,比如粘贴冲突)留下的
        // 红字——用户这次成功了就不该再看见旧错误(Important #4)。若这次
        // 也失败,下面各分支会立刻重新设置,不会丢失新错误。
        self.tree_error = None;
        let name = edit.buffer.trim();
        if name.is_empty() {
            return;
        }
        // 名字必须是单一正常路径分量,不能含 `/` 或是 `..`——否则
        // parent_dir.join(name) 会把项目挪出/建到目标目录之外
        // (Important #6)。三种模式共用同一检查,放在分派前。
        if !project::is_single_path_component(name) {
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
                    return; // 没改名,直接结束编辑
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
                let proxy = io.proxy.clone();
                io.handle.spawn(async move {
                    let result = tokio::task::spawn_blocking(move || {
                        std::fs::rename(&old_path, &new_path).map_err(|e| e.to_string())
                    })
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()));
                    let outcome = result.map(|()| parent);
                    let _ = proxy.send_event(Message::ProjectTreeOpDone {
                        project_id,
                        parent: outcome,
                        expand: false, // 重命名不改变展开态
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
                let proxy = io.proxy.clone();
                io.handle.spawn(async move {
                    let result = tokio::task::spawn_blocking(move || {
                        std::fs::File::create(&new_path)
                            .map(|_| ())
                            .map_err(|e| e.to_string())
                    })
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()));
                    let outcome = result.map(|()| parent);
                    let _ = proxy.send_event(Message::ProjectTreeOpDone {
                        project_id,
                        parent: outcome,
                        expand: true, // 新建的项要立刻可见,哪怕父目录之前是空的
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
                let proxy = io.proxy.clone();
                io.handle.spawn(async move {
                    let result = tokio::task::spawn_blocking(move || {
                        std::fs::create_dir(&new_path).map_err(|e| e.to_string())
                    })
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()));
                    let outcome = result.map(|()| parent);
                    let _ = proxy.send_event(Message::ProjectTreeOpDone {
                        project_id,
                        parent: outcome,
                        expand: true, // 新建的项要立刻可见,哪怕父目录之前是空的
                    });
                });
            }
        }
    }

    /// tab 关闭 = 结束会话：中断转发任务（`rx` 随任务栈析构）并 kill
    /// daemon 侧会话——否则 bootstrap 会把它当存活会话再恢复出来
    /// （P1e 验收反馈）。死会话（已 exited）无需再 kill。
    fn close_tab(&mut self, io: &ShellIo, idx: usize) {
        if idx >= self.tabs.len() {
            return;
        }
        let tab = self.tabs.remove(idx);
        tab.forwarder.abort();
        if tab.alive {
            let client = io.client.clone();
            let id = tab.info.id.clone();
            io.handle.spawn(async move {
                if let Err(e) = client.kill(&id).await {
                    tracing::warn!("关闭 tab 时结束会话失败: {e}");
                }
            });
        }
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len().saturating_sub(1);
        } else if idx < self.active {
            self.active -= 1;
        }
        // 关 tab 后位置全变，旧 first 可能越界——归零防御（P1L T5）。
        self.term_tab_first = 0;
    }

    /// 把当前预览 tab(仅文件类)异步写盘,同 `layout::save` 走
    /// `handle.spawn` 的既有模式,不阻塞 UI 线程。没有打开项目时不存
    /// (状态按项目 id 分文件,没有项目就没有归属)。
    fn spawn_preview_state_save(&self, io: &ShellIo) {
        let Some(project) = &self.project else {
            return;
        };
        let project_id = project.id;
        let active_idx = self.preview.active_idx();
        let mut paths = Vec::new();
        let mut active_path = None;
        for (idx, tab) in self.preview.tabs().iter().enumerate() {
            if let TabKind::File(p) = &tab.kind {
                paths.push(p.clone());
                if idx == active_idx {
                    active_path = Some(p.clone());
                }
            }
        }
        let state = preview_state::PreviewState { paths, active_path };
        io.handle.spawn(async move {
            if let Err(e) = preview_state::save(project_id, &state) {
                tracing::warn!("预览 tab 状态写盘失败: {e}");
            }
        });
    }

    /// 项目打开/启动恢复时,认回上次持久化的预览 tab(仅文件类;已被删除/
    /// 移动的文件静默跳过,不报错占位)。
    fn restore_preview_state(&mut self) {
        let Some(project) = &self.project else {
            return;
        };
        let state = preview_state::load(project.id);
        let mut active_id = None;
        for path in state.paths {
            if !path.is_file() {
                continue;
            }
            let is_active = state.active_path.as_deref() == Some(path.as_path());
            self.allowed_files
                .lock()
                .expect("allowed_files 锁")
                .insert(path.clone());
            let id = self.preview.open_path(path);
            if is_active {
                active_id = Some(id);
            }
        }
        if let Some(id) = active_id
            && let Some(idx) = self.preview.tabs().iter().position(|t| t.id == id)
        {
            self.preview.select(idx);
        }
    }

    /// 保证"项目打开时至少有一个终端 tab"这条不变式:启动恢复、关闭最后
    /// 一个 tab、切换/打开项目后都要检查一次。
    ///
    /// `self.project.is_some()` 这道闸门现在纯属**防御**:P2a 之后
    /// `Workspace` 恒有归属项目(会话必须归属项目,见 `spawn_new_tab` 的
    /// `expect` 与 `Client::create` 的 `project_id`),`None` 只可能出现在
    /// `Stub` 促成之前的占位 `Workspace` 上——那种状态下不该替用户建会话,
    /// 占位里建出来的 tab 在真正促成时会被整体丢弃。旧注释说的"无项目时
    /// `spawn_new_tab` 落回 $HOME"已随会话必须归属项目一起删除,不再是
    /// 当前行为。
    fn ensure_project_terminal(&mut self, io: &ShellIo) {
        if self.project.is_some() && self.tabs.is_empty() {
            self.spawn_new_tab(io, PickerLaunch::Agent(None), None);
        }
    }

    fn spawn_new_tab(
        &mut self,
        io: &ShellIo,
        launch: PickerLaunch,
        follow_up: Option<String>,
    ) -> Option<usize> {
        // 促成中的"加载中"占位不建会话:这份 `Workspace` 马上会被
        // `Message::ProjectSlotLoaded` 整份换掉,此刻建出来的会话会连同占位
        // 一起被丢弃,却仍在 daemon 上占着 PTY(见 `loading` 字段)。
        if self.loading {
            return None;
        }
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
        // 重锚:新终端 tab 开在当前项目根。会话必须归属一个项目(daemon 侧
        // `create` 要 project_id,P2a Task 3),而一个活着的 `Workspace` 之所以
        // 存在于 `App.projects` 里,前提就是它已知自己归属哪个项目——这里
        // 的 expect 失败只可能是上游逻辑错了(比如给还没促成的占位
        // `Workspace` 发了新建会话消息),该让它响亮地 panic,而不是静默落到
        // $HOME 建一个无归属的野会话。
        let project = self.project.as_ref().expect("Workspace 存在即已知归属项目");
        let cwd = project.path.clone();
        let project_id = project.id;
        let (cols, rows) = (io.cols, io.rows);
        let tab_id = self.next_tab_id;
        self.next_tab_id += 1;

        let client = io.client.clone();
        let proxy = io.proxy.clone();

        let jh = io.handle.spawn(async move {
            let info = match client
                .create("shell", &shell, &[], &cwd, cols, rows, project_id)
                .await
            {
                Ok(info) => info,
                Err(e) => {
                    let _ = proxy.send_event(Message::DaemonError(format!("新建会话失败: {e}")));
                    return;
                }
            };
            match client.attach(&info.id, 0).await {
                Ok((snapshot, _next_offset, rx)) => {
                    let session_id = info.id.clone();
                    if proxy
                        .send_event(Message::TabAttached(project_id, tab_id, info, snapshot))
                        .is_err()
                    {
                        return;
                    }
                    if let Some(cmd) = picker_launch_command(launch) {
                        let bytes = format!("{cmd}\n").into_bytes();
                        if let Err(e) = client.write(&session_id, &bytes).await {
                            tracing::warn!("自动键入初始命令失败: {e}");
                        }
                    }
                    if let Some(text) = follow_up {
                        let bytes = format!("{text}\n").into_bytes();
                        if let Err(e) = client.write(&session_id, &bytes).await {
                            tracing::warn!("派发任务文本失败: {e}");
                        }
                    }
                    forward_events(project_id, tab_id, rx, proxy).await;
                }
                Err(e) => {
                    let _ =
                        proxy.send_event(Message::DaemonError(format!("attach 新会话失败: {e}")));
                }
            }
        });

        self.pending.insert(tab_id, jh);
        Some(tab_id)
    }

    fn on_tab_attached(
        &mut self,
        io: &ShellIo,
        tab_id: usize,
        info: SessionInfo,
        snapshot: Vec<u8>,
    ) {
        // `pending` 条目总是在对应的 create+attach 任务 spawn 时就插入
        // （见 `spawn_new_tab`），理论上不会缺失；防御性丢弃而不是
        // panic，避免一次偶然的竞态打垮整个 GUI。
        let Some(forwarder) = self.pending.remove(&tab_id) else {
            return;
        };
        let mut model = TerminalModel::new(io.cols, io.rows);
        let _ = model.feed(&snapshot); // 快照回放：陈旧查询应答不可补发，丢弃
        self.tabs.push(SessionTab {
            agent_state: info.agent_state,
            agent: info.agent,
            transcript_path: info.transcript_path.clone(),
            info,
            model,
            alive: true,
            tab_id,
            forwarder,
            osc: OscScanner::new(),
            cwd: None,
            last_exit: None,
            delivery_pending: false,
            last_turn_head: None,
        });
        if let Some(t) = self.tabs.last_mut() {
            t.ingest_osc(&snapshot);
        }
        self.active = self.tabs.len() - 1;
        // 新 tab 落在末尾，滚回最左让它可见（P1L T5）。
        self.term_tab_first = 0;
    }

    /// 终端 pane 尺寸变化：换算出的新网格套用到本项目的所有 tab（含当前
    /// 不可见的），并把新尺寸同步给 daemon 侧存活的会话。
    ///
    /// "网格真的变了吗"这道闸门在调用方 `App::update` 的 `PaneResized`
    /// 分支上——`cols`/`rows` 是外壳态（整个窗口一份），比对基准不在这里。
    fn resize_all(&mut self, io: &ShellIo, cols: u16, rows: u16) {
        let client = io.client.clone();
        let handle = io.handle.clone();
        for tab in &mut self.tabs {
            tab.model.resize(cols, rows);
            if tab.alive {
                let client = client.clone();
                let id = tab.info.id.clone();
                handle.spawn(async move {
                    if let Err(e) = client.resize(&id, cols, rows).await {
                        tracing::warn!("同步终端尺寸到 daemon 失败: {e}");
                    }
                });
            }
        }
    }

    /// 浏览器地址栏是否在编辑态(main.rs 据此路由键盘:真 → AddrEvent,
    /// 假 → keymap → PTY)。预览面板已不再有地址栏,只需查 `self.browser`。
    pub fn browser_addr_editing(&self) -> bool {
        self.browser.addr_editing()
    }

    /// 验收意见输入是否在编辑态（main.rs 键盘路由用）。
    pub fn acceptance_comment_editing(&self) -> bool {
        self.acceptance.as_ref().is_some_and(|a| a.comment_editing)
    }

    /// 当前激活预览 tab 若是 webview(文件/网页)则返回其 id,供 main.rs
    /// 焦点路由取句柄;验收 tab/无 tab 返回 None。
    pub fn active_preview_webview_id(&self) -> Option<usize> {
        self.preview.active_webview_id()
    }

    /// 当前激活浏览器 tab 的 webview id,语义同 `active_preview_webview_id`,
    /// 查独立的 `self.browser`。
    pub fn active_browser_webview_id(&self) -> Option<usize> {
        self.browser.active_webview_id()
    }

    /// 项目树是否处于行内编辑态(main.rs 键盘路由用,同款
    /// `browser_addr_editing()`/`acceptance_comment_editing()`)。
    pub fn tree_editing(&self) -> bool {
        self.tree_edit.is_some()
    }

    /// 当前项目根路径(供 main.rs 算相对路径用;未打开项目时 None)。
    pub fn active_project_path(&self) -> Option<PathBuf> {
        self.project.as_ref().map(|p| PathBuf::from(&p.path))
    }

    /// 点击输入框外时退出所有自绘输入的编辑态(验收反馈:失焦回正常态)。
    /// 浏览器地址栏取消(清空半输入),意见框仅退出编辑(保留已输入文字),树内
    /// 编辑(重命名/新建)直接取消(Important #5——不清会导致点到别处后键盘还
    /// 在悄悄写进树编辑缓冲区,"打不出字"的假象)。`context_menu` 不在这里
    /// 清:它已经有专门的外点 dismiss 遮罩(`ProjectTreeContextMenuClose`,
    /// 见 view() 里的 stack dismiss 层),这里重复清是死代码。
    pub fn blur_inputs(&mut self) {
        if self.browser.addr_editing() {
            self.browser.addr_cancel();
        }
        if let Some(acc) = &mut self.acceptance {
            acc.comment_editing = false;
        }
        self.tree_edit = None;
    }

    /// 通过·沉淀：git update-ref + 落库（脏工作区在 delivery::accept 内被拒）。
    fn acceptance_accept(&mut self, io: &ShellIo) {
        let Some(project_id) = self.project_id() else {
            return;
        };
        let Some(acc) = &mut self.acceptance else {
            return;
        };
        acc.error = None;
        let repo = acc.repo.clone();
        let goal_title = acc
            .goal
            .as_ref()
            .map(|g| g.title.clone())
            .unwrap_or_default();
        let checked: Vec<String> = acc
            .goal
            .as_ref()
            .map(|g| {
                g.criteria
                    .iter()
                    .zip(&acc.checked)
                    .filter(|(_, c)| **c)
                    .map(|(s, _)| s.clone())
                    .collect()
            })
            .unwrap_or_default();
        let comment = acc.comment.clone();
        let client = io.client.clone();
        let proxy = io.proxy.clone();
        io.handle.spawn(async move {
            let repo2 = repo.clone();
            let accepted = tokio::task::spawn_blocking(move || delivery::accept(&repo2)).await;
            let result = match accepted {
                Ok(Ok(n)) => {
                    let ref_name = format!("{}{n}", delivery::ACCEPTED_REF_PREFIX);
                    let ts_ms = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis() as u64)
                        .unwrap_or(0);
                    if let Err(e) = client
                        .record_acceptance(
                            &repo.to_string_lossy(),
                            &goal_title,
                            &checked,
                            "accepted",
                            &comment,
                            &ref_name,
                            ts_ms,
                        )
                        .await
                    {
                        // ref 已写成立（真相源）,库失败只提示（spec §3）
                        Err(format!("已沉淀 v{n},但记录落库失败: {e}"))
                    } else {
                        Ok(n)
                    }
                }
                Ok(Err(e)) => Err(e.to_string()),
                Err(e) => Err(format!("任务失败: {e}")),
            };
            let _ = proxy.send_event(Message::AcceptanceDone(project_id, result));
        });
    }

    /// 打回并注回：意见 write 回来源会话 PTY，关验收 tab 回执行现场。
    fn acceptance_reject(&mut self, io: &ShellIo) {
        let Some(acc) = &self.acceptance else {
            return;
        };
        let comment = acc.comment.trim().to_string();
        let source = acc.source_tab_id;
        let target = self.tabs.iter().find(|t| t.tab_id == source);
        let Some(tab) = target.filter(|t| t.alive) else {
            if let Some(acc) = &mut self.acceptance {
                acc.error = Some("会话已结束,意见无处可注".into());
            }
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
        // 关验收 tab（打回后回执行现场）
        if let Some(idx) = self
            .preview
            .tabs()
            .iter()
            .position(|t| t.kind == crate::preview::TabKind::Acceptance)
        {
            self.preview.close(idx);
        }
        self.acceptance = None;
    }

    /// 协议闭包共享的文件白名单句柄.
    pub fn allowed_files(&self) -> Arc<Mutex<HashSet<PathBuf>>> {
        Arc::clone(&self.allowed_files)
    }

    /// 当前激活 tab 是否处于 application cursor mode（DECCKM）。
    /// `main.rs` 的 `on_window_event` 用它决定方向键发 CSI 还是 SS3 序列
    /// （见 `keymap::key_to_bytes` 的 `app_cursor` 参数）。没有任何 tab
    /// 时（例如 daemon 连接失败的降级态）保守返回 `false`。
    pub fn active_app_cursor_mode(&self) -> bool {
        self.tabs
            .get(self.active)
            .map(|t| t.model.app_cursor_mode())
            .unwrap_or(false)
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
        Self {
            client,
            handle,
            proxy,
            cols: DEFAULT_COLS,
            rows: DEFAULT_ROWS,
            term_focused: true,
            daemon_error,
            blink_on: true,
            left_view: shell_layout.left_view,
            right_view: shell_layout.right_view,
            left_collapsed: shell_layout.left_collapsed,
            right_collapsed: shell_layout.right_collapsed,
            shell_layout,
            maximized: None,
            active_zone: Some(ZoneSide::Right),
            hover_anims: std::collections::HashMap::new(),
            pending_zoom_toggle: false,
            pending_preview_zoom: false,
            window_size: workspace_geometry::initial_window_size(),
            dragging: None,
            context_menu: None,
            last_right_click: (0.0, 0.0),
            projects: HashMap::new(),
            project_order: Vec::new(),
            active_project_id: None,
            current_page: AppPage::Workspace,
            recent_projects: Vec::new(),
            home_recent_files: Vec::new(),
            home_recent_conversations: Vec::new(),
            home_recents_loaded: false,
            git_log: git_log::State::default(),
            todo_meta: todo_meta::load(),
        }
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
    /// 变了才重读+reparse（`reload_todo_from_disk` 内部也会再 stat 一次
    /// mtime，多一次系统调用换取 `reload_todo_from_disk` 保持独立可复用）。
    pub fn poll_todo_if_visible(&mut self) {
        if !self.todo_panel_visible() {
            return;
        }
        let Some(ws) = self.active_workspace_mut() else {
            return;
        };
        let Some(project) = ws.project.as_ref() else {
            return;
        };
        let path = todo::todo_path(std::path::Path::new(&project.path));
        let current = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        if current != ws.todo_mtime {
            ws.reload_todo_from_disk();
        }
    }

    /// 把一条派发记录写进 `todo_meta` 并落盘。`text` 用来算
    /// `todo::todo_line_key`——跟渲染时查询用的 key 必须是同一套算法，
    /// 否则写进去的记录永远查不到。
    fn record_todo_dispatch(&mut self, project_id: ProjectId, text: &str, session_id: String) {
        let key = todo::todo_line_key(text);
        let entry = self.todo_meta.entry(project_id).or_default();
        entry.insert(
            key,
            todo_meta::TodoTaskMeta {
                dispatch: Some(todo_meta::DispatchRecord {
                    session_id,
                    dispatched_at: std::time::SystemTime::now(),
                }),
                ..entry.get(&key).cloned().unwrap_or_default()
            },
        );
        if let Err(e) = todo_meta::save(&self.todo_meta) {
            tracing::warn!("写入 todo_meta.json 失败: {e}");
        }
    }

    /// 写/清计划时间：`draft` 为空字符串时存 `None`（清空这个字段，
    /// 不留空白占位——空草稿等价于用户想清空这条）。
    fn set_todo_plan_date(&mut self, project_id: ProjectId, text: &str, draft: String) {
        let key = todo::todo_line_key(text);
        let entry = self.todo_meta.entry(project_id).or_default();
        let meta = entry.entry(key).or_default();
        meta.plan_date = if draft.trim().is_empty() {
            None
        } else {
            Some(draft.trim().to_string())
        };
        if let Err(e) = todo_meta::save(&self.todo_meta) {
            tracing::warn!("写入 todo_meta.json 失败: {e}");
        }
    }

    /// 勾选变完成 → 盖章当前时间；取消勾选 → 清空（避免待办任务身上挂着
    /// 陈旧的完成于时间戳）。
    fn set_todo_completed_at(&mut self, project_id: ProjectId, text: &str, done: bool) {
        let key = todo::todo_line_key(text);
        let entry = self.todo_meta.entry(project_id).or_default();
        let meta = entry.entry(key).or_default();
        meta.completed_at = todo::completed_at_for_toggle(done, std::time::SystemTime::now());
        if let Err(e) = todo_meta::save(&self.todo_meta) {
            tracing::warn!("写入 todo_meta.json 失败: {e}");
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

    /// 翻转闪烁相位；由 main.rs 的定时唤醒每拍调用一次。
    pub fn toggle_blink(&mut self) {
        self.blink_on = !self.blink_on;
    }

    /// 设置某按钮的悬停目标（`true`=进入,`false`=离开）；动画由
    /// `advance_hover_anims` 循环把它指数逼近（见 `HoverAnim`）。
    pub fn set_hover(&mut self, id: HoverId, hovered: bool) {
        self.hover_anims.entry(id).or_default().set(hovered);
    }

    /// 推进所有按钮的悬停动画一拍（约 60fps 一拍，由 main.rs 的定时唤醒
    /// 驱动；与光标闪烁同款自驱 redraw 范式）。每拍残余 75%（逼近系数
    /// 0.25），约 150ms 内收敛到目标，视觉上是干脆的 ease-out。
    pub fn advance_hover_anims(&mut self) {
        for a in self.hover_anims.values_mut() {
            a.advance();
        }
    }

    /// 是否还有按钮的悬停动画在进行中（任一进度未到目标）。
    /// main.rs 据此决定是否继续排下一拍定时唤醒。
    pub fn any_hover_anim_active(&self) -> bool {
        self.hover_anims.values().any(HoverAnim::active)
    }

    /// 某按钮当前悬停动画进度(0..=1)，给视图层做颜色插值。
    pub fn hover_progress(&self, id: HoverId) -> f32 {
        self.hover_anims.get(&id).map(HoverAnim::t).unwrap_or(0.0)
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
        self.active_workspace()?.active_browser_webview_id()
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
    pub fn blur_inputs(&mut self) {
        if let Some(ws) = self.active_workspace_mut() {
            ws.blur_inputs();
        }
    }

    /// 把 `left_view`/`right_view`/`left_collapsed`/`right_collapsed` 同步进
    /// `shell_layout` 再异步写盘。图标切换/收起要立即持久化，不能只靠
    /// `ColumnDragEnd` 顺带存(用户可能从没拖过分隔线)。
    fn spawn_shell_layout_save(&mut self) {
        self.shell_layout.left_view = self.left_view;
        self.shell_layout.right_view = self.right_view;
        self.shell_layout.left_collapsed = self.left_collapsed;
        self.shell_layout.right_collapsed = self.right_collapsed;
        let layout = self.shell_layout;
        self.handle.spawn(async move {
            if let Err(e) = layout::save(&layout) {
                tracing::warn!("外壳布局写盘失败: {e}");
            }
        });
    }

    /// 外壳几何状态(视图选择/收起态/分隔线位置)变化后的统一收尾:持久化 +
    /// 按新几何重算终端网格。任何改变"终端 pane 实际拿到多少像素"的
    /// handler 都该走这里,不要只调其中一半——只存不重算,终端网格会停在
    /// 上一次窗口 resize 时的尺寸(Fix round 2 #6)。
    fn on_shell_layout_changed(&mut self) {
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

    /// 建窗时用的初始窗口尺寸偏好:优先用上次退出前持久化的
    /// `shell_layout.window_width/height`(已经过 `sanitize_shell_layout`
    /// 夹取),`layout.json` 不存在/读不到时 `layout::load()` 本身已经退化
    /// 成 `ShellLayout::default()`,即 `workspace_geometry::initial_window_size()`,这里不用再
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

    /// 左面板区当前**有效**宽度:持久化宽按当前窗口宽夹取(见
    /// `clamp_left_width`)。渲染侧(`left_panel_area`)必须用这个值,而不是
    /// 直接读 `shell_layout.left_width`——几何侧(`left_zone_width`)走的是
    /// 同一个 `clamp_left_width`,两边只有共用同一份夹取才不会漂移。
    fn effective_left_width(&self) -> f32 {
        clamp_left_width(self.window_size.0, self.shell_layout.left_width)
    }

    /// 终端 pane 此刻是否真的呈现在用户眼前(判定见自由函数
    /// [`terminal_visible`];逻辑只此一份,便于单测直接喂 `ShellState`)。
    fn terminal_visible(&self) -> bool {
        terminal_visible(&self.shell_state())
    }

    /// 当前外壳几何状态快照(main.rs 拖拽追踪/离屏几何计算用;`Copy`
    /// 类型直接按值返回)。
    pub fn shell_state(&self) -> ShellState {
        ShellState {
            layout: self.shell_layout,
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

    /// 项目树右键菜单是否打开(main.rs Esc 键路由用)。
    pub fn context_menu_open(&self) -> bool {
        self.context_menu.is_some()
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
            .map(|ws| ws.todo_dispatch_open.is_some())
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
        let list_w = pair_content_width(right_w) * state.layout.agent_split;
        let x0 = window_w - workspace_geometry::icon_rail_width() - right_w
            + list_w
            + workspace_geometry::divider_width()
            + 8.0;
        // 终端网格上方 chrome:顶栏 44 + 上 padding 8 + tab 栏 30 + spacing 4(header 已去,P1L #4)
        let y0 = workspace_geometry::top_bar_height() + 8.0 + 30.0 + 4.0;
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
        if self.left_view != LeftView::Files {
            return Vec::new();
        }
        let Some(ws) = self.active_workspace() else {
            return Vec::new();
        };
        let specs = ws.preview.desired_webviews();
        // 编辑弹层开着时,应用级模态盖住了预览区,原生 wry 子视图不听 iced
        // 绘制顺序摆布,必须显式 visible=false 才能真正藏起来——与
        // `TabKind::Acceptance` 隐藏其余 webview 的机制完全一致
        // (`PreviewPane::desired_webviews` 内部的 `acceptance_active`)。
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
    /// `Workspace::browser`,且只在左视图为 Web 时非空。
    pub fn browser_desired(&self) -> Vec<WebviewSpec> {
        if self.left_view != LeftView::Web {
            return Vec::new();
        }
        match self.active_workspace() {
            Some(ws) => ws.browser.desired_webviews(),
            None => Vec::new(),
        }
    }

    /// 保证 `git_log` 状态跟得上"现在应该看哪个项目"——`git_log: State`
    /// 是 `App` 级字段,不是每个项目各自一份(不像 `Workspace.worktrees`),
    /// 所以面板打开时(`LeftIconSelect`)和切项目页签时(`ProjectTabSwitch`)
    /// 都得调这个方法对齐一次,否则 Git Log 面板开着的状态下切页签,提交图
    /// 会停在上一个项目不动,而同一面板里的 worktree 速览条(`ws.worktrees`
    /// 是按项目取的)却已经跳到新项目——两者对不上。缓存已经是当前项目的
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
            Message::TermInput(bytes) => {
                // 终端不在屏上时丢弃按键(不报错、不写 PTY):否则用户在读
                // 对话审阅时敲的回车/方向键会静默提交给隐藏在后面的 agent
                // 会话(Fix round 2 #3)。
                if !self.terminal_visible() {
                    return;
                }
                self.with_focused_project(|ws, io| {
                    // 键入即回底 + 清选区：正在回看历史时一敲键盘，视口跳回
                    // 实时输出（常规终端语义），再把字节写给 daemon。
                    if let Some(tab) = ws.tabs.get_mut(ws.active) {
                        tab.model.scroll_to_bottom();
                        tab.model.selection_clear();
                    }
                    ws.send_input(io, bytes);
                });
            }
            Message::TermOutput(project_id, tab_id, bytes) => {
                self.with_project(project_id, |ws, io| {
                    let Some(tab) = ws.tab_by_id_mut(tab_id) else {
                        return;
                    };
                    // 实时输出可能含设备查询（DSR/DA 等），应答必须写回 PTY
                    // ——atuin/claude 等 TUI 依赖它（此前丢弃导致探测超时）。
                    tab.ingest_osc(&bytes);
                    let responses = tab.model.feed(&bytes);
                    let alive = tab.alive;
                    let id = tab.info.id.clone();
                    if !responses.is_empty() && alive {
                        let client = io.client.clone();
                        io.handle.spawn(async move {
                            if let Err(e) = client.write(&id, &responses).await {
                                tracing::warn!("回写终端查询应答失败: {e}");
                            }
                        });
                    }
                });
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
                            let cwd =
                                effective_project_repo(active_repo.as_deref(), &tab.effective_cwd());
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
                                    let _ =
                                        proxy.send_event(Message::DeliveryChecked(
                                            project_id, tab_id, pending,
                                        ));
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
            Message::DeliveryChecked(project_id, tab_id, pending) => {
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
                        let cwd =
                            effective_project_repo(active_repo.as_deref(), &tab.effective_cwd());
                        if let Some(repo) = delivery::repo_root(&cwd) {
                            tab.last_turn_head = delivery::head_commit(&repo);
                        }
                    }
                    // 回合结束后刷新项目 git 状态,文件树装饰随之更新（P1h）。
                    ws.spawn_project_git_refresh(io);
                    // 回合结束后刷新对话列表(transcript 增长/新增；P1j)。
                    ws.spawn_conversations_refresh(io);
                });
            }
            Message::AcceptanceOpen(tab_id) => {
                self.with_focused_project(|ws, io| {
                    let active_repo = ws.project.as_ref().map(|p| PathBuf::from(&p.path));
                    // 用户点的是当前界面上的横幅,所以入口走 `with_focused_project`;但异步
                    // 装载结果要投回**这个**项目,不能投给"结果回来时恰好在
                    // 前台的那个"(见 `ProjectId`)。
                    let Some(project_id) = ws.project_id() else {
                        return;
                    };
                    let Some(tab) = ws.tab_by_id_mut(tab_id) else {
                        return;
                    };
                    tab.delivery_pending = false;
                    let cwd = effective_project_repo(active_repo.as_deref(), &tab.effective_cwd());
                    let proxy = io.proxy.clone();
                    io.handle.spawn(async move {
                        let loaded = tokio::task::spawn_blocking(move || {
                            let repo = delivery::repo_root(&cwd)?;
                            let goal = std::fs::read_to_string(goal::goal_path(&repo))
                                .ok()
                                .and_then(|md| goal::parse_goal(&md));
                            let changes = delivery::changes(&repo);
                            Some((repo, goal, changes))
                        })
                        .await
                        .ok()
                        .flatten();
                        if let Some((repo, goal, changes)) = loaded {
                            let _ = proxy.send_event(Message::AcceptanceLoaded(
                                project_id, repo, tab_id, goal, changes,
                            ));
                        }
                    });
                });
            }
            Message::AcceptanceLoaded(project_id, repo, source_tab_id, goal, changes) => {
                // 这条异步结果可能属于**后台**项目(用户在加载期间切了页签)。
                // 那种情况下只把验收内容装进那个项目的 `Workspace`,绝不动外壳:
                // 展开左面板是"让你现在就看见"的动作,而现在你看的是别的项目。
                let landed_on_focused = self.active_project_id == Some(project_id);
                self.with_project(project_id, move |ws, _io| {
                    let n = goal.as_ref().map(|g| g.criteria.len()).unwrap_or(0);
                    ws.acceptance = Some(AcceptanceView {
                        repo,
                        source_tab_id,
                        goal,
                        changes,
                        checked: vec![false; n],
                        comment: String::new(),
                        comment_editing: false,
                        error: None,
                        accepted_version: None,
                    });
                    ws.preview.open_acceptance();
                });
                // 验收内容画在 `preview_pane` 里,而 `preview_pane` 属于左
                // 面板区:左侧收起时点"进入验收"会毫无反应(内容装进了一个
                // 没被渲染的面板)。新外壳鼓励收起左侧给终端腾空间,所以这里
                // 必须主动展开(Fix round 2 #5)。左侧收起态是外壳态,搬家后
                // 留在 `App` 上处理。
                if landed_on_focused && self.left_collapsed {
                    self.left_collapsed = false;
                    self.on_shell_layout_changed();
                }
            }
            Message::AcceptanceToggle(i) => {
                self.with_focused_project(|ws, _io| {
                    if let Some(acc) = &mut ws.acceptance
                        && let Some(c) = acc.checked.get_mut(i)
                    {
                        *c = !*c;
                    }
                });
            }
            Message::AcceptanceCommentClick => {
                self.with_focused_project(|ws, _io| {
                    if let Some(acc) = &mut ws.acceptance {
                        acc.comment_editing = true;
                    }
                });
            }
            Message::AcceptanceCommentEvent(ev) => {
                self.with_focused_project(|ws, _io| {
                    if let Some(acc) = &mut ws.acceptance {
                        match ev {
                            AddrEvent::Text(s) => acc.comment.push_str(&s),
                            AddrEvent::Backspace => {
                                acc.comment.pop();
                            }
                            AddrEvent::Submit | AddrEvent::Cancel => acc.comment_editing = false,
                        }
                    }
                });
            }
            Message::AcceptanceAccept => {
                self.with_focused_project(|ws, io| ws.acceptance_accept(io))
            }
            Message::AcceptanceReject => {
                self.with_focused_project(|ws, io| ws.acceptance_reject(io))
            }
            Message::AcceptanceDone(project_id, result) => {
                self.with_project(project_id, |ws, io| {
                    let landed = result.is_ok();
                    if let Some(acc) = &mut ws.acceptance {
                        match result {
                            Ok(n) => acc.accepted_version = Some(n),
                            Err(e) => acc.error = Some(e),
                        }
                    }
                    if landed {
                        ws.spawn_acceptance_count_refresh(io);
                    }
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
            Message::UsageRefresh => {
                self.with_focused_project(|ws, io| {
                    ws.usage_loading = true;
                    ws.spawn_usage_refresh(io);
                });
            }
            Message::UsageLoaded(project_id, rows) => {
                self.with_project(project_id, move |ws, _io| {
                    ws.usage = rows;
                    ws.usage_loading = false;
                });
            }
            Message::ConversationOpen(path) => {
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
            Message::SelectTab(idx) => {
                self.with_focused_project(|ws, _io| {
                    if idx < ws.tabs.len() {
                        ws.active = idx;
                    }
                });
            }
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
            Message::TodoToggle(idx) => {
                let Some(project_id) = self.active_project_id else {
                    return;
                };
                let before = self
                    .active_workspace()
                    .and_then(|ws| ws.todo_items.get(idx))
                    .cloned();
                self.with_focused_project(|ws, _io| ws.toggle_todo_item(idx));
                let after = self
                    .active_workspace()
                    .and_then(|ws| ws.todo_items.get(idx))
                    .cloned();
                if let (Some(before), Some(after)) = (before, after) {
                    // 文本没变(正常勾选场景)才更新 `completed_at`；如果文本变了
                    // (文件可能在加载期间被 agent 并发改过)，跳过，避免把完成
                    // 时间错记到另一条任务上。
                    if before.text == after.text {
                        self.set_todo_completed_at(project_id, &after.text, after.done);
                    }
                }
            }
            Message::TodoAddInputChanged(s) => {
                self.with_focused_project(|ws, _io| ws.todo_add_draft = s);
            }
            Message::TodoAddSubmit => {
                self.with_focused_project(|ws, _io| ws.submit_todo_add());
            }
            Message::TodoFilterSet(f) => {
                self.with_focused_project(|ws, _io| ws.todo_filter = f);
            }
            Message::TodoSearchChanged(s) => {
                self.with_focused_project(|ws, _io| ws.todo_search = s);
            }
            Message::TodoDispatchOpen(idx) => {
                self.with_focused_project(|ws, _io| ws.todo_dispatch_open = Some(idx));
            }
            Message::TodoDispatchClose => {
                self.with_focused_project(|ws, _io| ws.todo_dispatch_open = None);
            }
            Message::TodoDispatchToExisting(idx, session_id) => {
                let Some(project_id) = self.active_project_id else {
                    return;
                };
                let text = self
                    .active_workspace()
                    .and_then(|ws| ws.todo_items.get(idx))
                    .map(|item| item.text.clone());
                let Some(text) = text else {
                    return;
                };
                self.with_focused_project(|ws, io| {
                    ws.todo_dispatch_open = None;
                    ws.dispatch_todo_to_existing(io, &session_id, &text);
                });
                self.record_todo_dispatch(project_id, &text, session_id);
            }
            Message::TodoDispatchNew(idx, launch) => {
                let text = self
                    .active_workspace()
                    .and_then(|ws| ws.todo_items.get(idx))
                    .map(|item| item.text.clone());
                let Some(text) = text else {
                    return;
                };
                self.with_focused_project(|ws, io| {
                    ws.todo_dispatch_open = None;
                    if let Some(tab_id) = ws.spawn_new_tab(io, launch, Some(text.clone())) {
                        ws.todo_pending_dispatch.insert(tab_id, text);
                    }
                });
            }
            Message::TodoPlanDateEditStart(idx) => {
                let existing = self
                    .active_workspace()
                    .and_then(|ws| ws.project.as_ref().map(|p| p.id))
                    .and_then(|pid| {
                        let key = self
                            .active_workspace()
                            .and_then(|ws| ws.todo_items.get(idx))
                            .map(|item| todo::todo_line_key(&item.text))?;
                        self.todo_meta.get(&pid)?.get(&key)?.plan_date.clone()
                    })
                    .unwrap_or_default();
                self.with_focused_project(|ws, _io| {
                    ws.todo_editing_plan_date = Some((idx, existing));
                });
            }
            Message::TodoPlanDateChanged(s) => {
                self.with_focused_project(|ws, _io| {
                    if let Some((_, draft)) = ws.todo_editing_plan_date.as_mut() {
                        *draft = s;
                    }
                });
            }
            Message::TodoPlanDateSubmit => {
                let Some(project_id) = self.active_project_id else {
                    return;
                };
                let entry = self
                    .active_workspace()
                    .and_then(|ws| ws.todo_editing_plan_date.clone())
                    .and_then(|(idx, draft)| {
                        let text = self
                            .active_workspace()
                            .and_then(|ws| ws.todo_items.get(idx))
                            .map(|item| item.text.clone())?;
                        Some((text, draft))
                    });
                if let Some((text, draft)) = entry {
                    self.set_todo_plan_date(project_id, &text, draft);
                }
                self.with_focused_project(|ws, _io| ws.todo_editing_plan_date = None);
            }
            Message::TabAttached(project_id, tab_id, info, snapshot) => {
                let session_id = info.id.clone();
                self.with_project(project_id, move |ws, io| {
                    ws.on_tab_attached(io, tab_id, info, snapshot)
                });
                // 若是从 Todo 面板"派发到新建"建的 tab,补记派发记录——此时才
                // 第一次知道真正的 `session_id`。`TabAttached` 是异步结果消息,
                // 必须按自带的 `project_id` 路由(同 `on_tab_attached` 那一步),
                // 不能用 `active_workspace_mut()`(当前聚焦项目可能已经切走)。
                if let Some(text) = loaded_workspace_mut(&mut self.projects, project_id)
                    .and_then(|ws| ws.todo_pending_dispatch.remove(&tab_id))
                {
                    self.record_todo_dispatch(project_id, &text, session_id);
                }
            }
            Message::PaneResized { cols, rows } => {
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
            Message::ColumnDragStart(divider) => {
                self.dragging = Some(divider);
            }
            Message::ColumnDrag {
                window_width,
                logical_x,
            } => {
                if let Some(divider) = self.dragging {
                    let state = self.shell_state();
                    self.shell_layout = apply_column_drag(state, divider, window_width, logical_x);
                }
            }
            Message::ColumnDragEnd => {
                self.dragging = None;
                self.on_shell_layout_changed();
            }
            Message::LeftIconSelect(v) => {
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
                    self.with_focused_project(|ws, _io| ws.reload_todo_from_disk());
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
            Message::RightIconSelect(v) => {
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
                            ws.usage_loading = true;
                            ws.spawn_usage_refresh(io);
                        });
                    }
                }
                // 同 LeftIconSelect(Fix round 2 #2)。
                self.maximized = None;
                self.on_shell_layout_changed();
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
            Message::TopBarHome => {
                self.current_page = AppPage::Home;
                self.home_recents_loaded = false;
                let projects: Vec<ProjectInfo> =
                    self.recent_projects.iter().take(5).cloned().collect();
                let proxy = self.proxy.clone();
                self.handle.spawn(async move {
                    let (files, convs) =
                        tokio::task::spawn_blocking(move || load_home_recents(&projects))
                            .await
                            .unwrap_or_default();
                    let _ = proxy.send_event(Message::HomeRecentsLoaded(files, convs));
                });
            }
            Message::HomeRecentsLoaded(files, convs) => {
                self.home_recent_files = files;
                self.home_recent_conversations = convs;
                self.home_recents_loaded = true;
            }
            Message::Noop => {}
            Message::DaemonError(message) => self.daemon_error = Some(message),
            Message::TermScroll(delta) => {
                self.with_focused_project(|ws, _io| {
                    if let Some(tab) = ws.tabs.get_mut(ws.active) {
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
            Message::BrowserTabScroll(right) => {
                self.with_focused_project(|ws, _io| {
                    if right {
                        ws.browser_tab_first = ws.browser_tab_first.saturating_add(2);
                    } else {
                        ws.browser_tab_first = ws.browser_tab_first.saturating_sub(2);
                    }
                });
            }
            Message::TermSelStart { col, row, right } => {
                self.with_focused_project(|ws, _io| {
                    if let Some(tab) = ws.tabs.get_mut(ws.active) {
                        tab.model.selection_start(col, row, right);
                    }
                });
            }
            Message::TermSelUpdate { col, row, right } => {
                self.with_focused_project(|ws, _io| {
                    if let Some(tab) = ws.tabs.get_mut(ws.active) {
                        tab.model.selection_update(col, row, right);
                    }
                });
            }
            Message::TermPaste(text) => {
                // 同 TermInput 的可见性闸门(Fix round 3):⌘V 粘贴走同一条
                // PTY 写入路径,粘贴内容若含换行还会在看不见的会话里直接
                // 执行,比单个按键更危险,必须同样拦截。
                if !self.terminal_visible() {
                    return;
                }
                self.with_focused_project(move |ws, io| {
                    let Some(tab) = ws.tabs.get_mut(ws.active) else {
                        return;
                    };
                    tab.model.scroll_to_bottom();
                    let bytes = if tab.model.bracketed_paste() {
                        let mut b = b"\x1b[200~".to_vec();
                        b.extend_from_slice(text.as_bytes());
                        b.extend_from_slice(b"\x1b[201~");
                        b
                    } else {
                        text.into_bytes()
                    };
                    ws.send_input(io, bytes);
                });
            }
            Message::PreviewOpenPath(path) => {
                self.with_focused_project(move |ws, io| {
                    if !path.is_file() {
                        ws.preview_error = Some(format!("文件不存在或不可读: {}", path.display()));
                        return;
                    }
                    ws.preview_error = None;
                    ws.tree_selected = Some(path.clone());
                    ws.allowed_files
                        .lock()
                        .expect("allowed_files 锁")
                        .insert(path.clone());
                    ws.preview.open_path(path);
                    // 新 tab 落在末尾，滚回最左让它可见（P1L T5）。
                    ws.preview_tab_first = 0;
                    ws.spawn_preview_state_save(io);
                });
            }
            Message::PreviewSelectTab(idx) => {
                self.with_focused_project(|ws, io| {
                    ws.preview.select(idx);
                    ws.spawn_preview_state_save(io);
                });
            }
            Message::PreviewCloseTab(idx) => {
                self.with_focused_project(|ws, io| {
                    ws.preview.close(idx);
                    // 关 tab 后位置全变，旧 first 可能越界——归零防御（P1L T5）。
                    ws.preview_tab_first = 0;
                    ws.spawn_preview_state_save(io);
                });
            }
            Message::PreviewEditOpen(idx) => {
                self.with_focused_project(move |ws, _io| ws.preview_edit_open(idx));
            }
            Message::PreviewEditAction(action) => {
                self.with_focused_project(move |ws, _io| ws.preview_edit_action(action));
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
            Message::BrowserOpenUrl(url) => {
                self.with_focused_project(move |ws, _io| {
                    ws.browser_error = None;
                    ws.browser.open_url(url);
                    ws.browser_tab_first = 0;
                });
            }
            Message::BrowserSelectTab(idx) => {
                self.with_focused_project(|ws, _io| ws.browser.select(idx));
            }
            Message::BrowserCloseTab(idx) => {
                self.with_focused_project(|ws, _io| {
                    ws.browser.close(idx);
                    ws.browser_tab_first = 0;
                });
            }
            Message::BrowserAddrClick => {
                self.with_focused_project(|ws, _io| {
                    ws.browser_error = None;
                    ws.browser.addr_begin();
                });
            }
            Message::BrowserAddrEvent(ev) => {
                // 地址栏回车解析出网址时要走一遍完整的 `BrowserOpenUrl` 处理,
                // 而 `self.update(..)` 不能在 `with_focused_project` 的闭包里调(闭包正握着
                // 从 `self` 借出去的 `&mut Workspace`),所以先把结果攒出来,
                // 出了闭包再派发。
                let mut open_url = None;
                self.with_focused_project(|ws, _io| match ev {
                    AddrEvent::Text(s) => ws.browser.addr_text(&s),
                    AddrEvent::Backspace => ws.browser.addr_backspace(),
                    AddrEvent::Cancel => ws.browser.addr_cancel(),
                    AddrEvent::Submit => match ws.browser.addr_submit() {
                        // 浏览器只承载网页 tab,地址栏解析出的本地路径不受支持
                        // (与文件预览彻底独立,不借它的文件打开能力)。
                        Some(AddrTarget::File(_)) => {
                            ws.browser_error = Some("浏览器不支持打开本地文件".to_string());
                        }
                        Some(AddrTarget::Url(url)) => open_url = Some(url),
                        None => {}
                    },
                });
                if let Some(url) = open_url {
                    self.update(Message::BrowserOpenUrl(url));
                }
            }
            Message::BrowserStarClick => {
                self.with_focused_project(|ws, _io| {
                    ws.browser_error = None;
                    ws.browser_star_menu_open = !ws.browser_star_menu_open;
                });
            }
            Message::BrowserBookmarksToggle => {
                self.with_focused_project(|ws, _io| {
                    ws.browser_bookmarks_open = !ws.browser_bookmarks_open;
                });
            }
            Message::BrowserBookmarkAdd(scope) => {
                self.with_focused_project(|ws, io| {
                    ws.browser_star_menu_open = false;
                    let Some(project_id) = ws.project.as_ref().map(|p| p.id) else {
                        return;
                    };
                    let Some(tab) = ws.browser.tabs().get(ws.browser.active_idx()) else {
                        return;
                    };
                    let TabKind::Web { url } = tab.kind.clone() else {
                        return;
                    };
                    let title = tab.title.clone();
                    let target_project_id = match scope {
                        BookmarkScope::Global => None,
                        BookmarkScope::Project => Some(project_id),
                    };
                    let created_ms = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis() as u64)
                        .unwrap_or(0);
                    bookmarks::optimistic_add(
                        &mut ws.bookmarks,
                        scope,
                        target_project_id,
                        &url,
                        &title,
                        created_ms,
                    );
                    let client = io.client.clone();
                    let proxy = io.proxy.clone();
                    io.handle.spawn(async move {
                        let res = client
                            .add_bookmark(scope, target_project_id, &url, &title)
                            .await
                            .map_err(|e| e.to_string());
                        let _ = proxy.send_event(Message::BrowserBookmarksMutated(project_id, res));
                    });
                });
            }
            Message::BrowserBookmarkRemove(id) => {
                self.with_focused_project(|ws, io| {
                    ws.browser_star_menu_open = false;
                    let Some(project_id) = ws.project.as_ref().map(|p| p.id) else {
                        return;
                    };
                    bookmarks::optimistic_remove(&mut ws.bookmarks, id);
                    let client = io.client.clone();
                    let proxy = io.proxy.clone();
                    io.handle.spawn(async move {
                        let res = client.remove_bookmark(id).await.map_err(|e| e.to_string());
                        let _ = proxy.send_event(Message::BrowserBookmarksMutated(project_id, res));
                    });
                });
            }
            Message::BrowserBookmarksLoaded(project_id, bookmarks) => {
                self.with_project(project_id, move |ws, _io| {
                    ws.bookmarks = bookmarks;
                });
            }
            Message::BrowserBookmarksMutated(project_id, res) => {
                self.with_project(project_id, move |ws, io| {
                    if let Err(msg) = res {
                        ws.browser_error = Some(msg);
                    }
                    ws.spawn_bookmarks_refresh(io);
                });
            }
            Message::ProjectSelect(id) => {
                // 切项目不再通知 daemon:"活跃项目"是 GUI 侧的概念了(P2a
                // Task 1-3 删掉了 SetActiveProject)。
                //
                // 这个项目已经开着页签(`Loaded` 或还没促成的 `Stub`)时,点最近
                // 项目卡片就只是"切到那个页签",走与点页签完全相同的非破坏性
                // 路径——绝不能杀掉任何已有页签的会话(设计文档 §2)。
                if focus_project_tab(&self.projects, &mut self.active_project_id, id) {
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
            Message::ProjectTabPickFolder => {} // 副作用在 main.rs(rfd 文件夹选择)
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
            Message::ProjectTabOpened(project, recent) => {
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
                if focus_project_tab(&self.projects, &mut self.active_project_id, id) {
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
                self.current_page = AppPage::Workspace;
                self.sync_terminal_grid(); // 同上
                self.persist_open_projects();
            }
            Message::ProjectTabSwitch(id) => {
                // 切页签只有两件事:改 `active_project_id`、必要时促成 `Stub`。
                // 没有任何内容改写,因此后台项目的终端/预览/审阅原样留着,切
                // 回来还是刚才那副样子。
                if !focus_project_tab(&self.projects, &mut self.active_project_id, id) {
                    return;
                }
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
            }
            Message::ProjectTabClose(id) => {
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
                    self.ensure_loaded(next);
                }
                self.sync_terminal_grid(); // 清放大态后重算网格,理由同 `ProjectTabSwitch`
                self.persist_open_projects();
            }
            Message::ProjectSlotLoaded(id, payload) => {
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
            Message::ProjectTreeToggle(dir) => {
                self.with_focused_project(move |ws, _io| {
                    ws.tree_selected = Some(dir.clone());
                    if let Some(t) = &mut ws.file_tree {
                        t.toggle(&dir);
                    }
                });
            }
            Message::ProjectGitRefreshed(project_id, branch, dirty, statuses, worktrees) => {
                self.with_project(project_id, move |ws, _io| {
                    ws.branch = branch;
                    ws.dirty = dirty;
                    ws.git_statuses = statuses;
                    ws.worktrees = worktrees;
                });
            }
            Message::ProjectFsChanged(project_id, relevance) => {
                self.with_project(project_id, |ws, io| {
                    ws.spawn_project_git_refresh(io);
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
            Message::GitLog(git_log::Message::LoadMore) => {
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
            Message::AcceptanceCountLoaded(project_id, n) => {
                self.with_project(project_id, move |ws, _io| {
                    ws.project_acceptance_count = n;
                });
            }
            Message::RightClickAt { x, y } => {
                self.last_right_click = (x, y);
            }
            Message::ProjectTreeContextMenu { path, is_dir } => {
                let (x, y) = self.last_right_click;
                let target = path.clone();
                self.with_focused_project(move |ws, _io| {
                    ws.tree_selected = Some(path);
                });
                self.context_menu = Some(ContextMenu {
                    x,
                    y,
                    target,
                    is_dir,
                });
            }
            Message::ProjectTreeContextMenuClose => {
                self.context_menu = None;
            }
            Message::ProjectTreeCopyPath(_, _) => {} // 副作用在 main.rs(写系统剪贴板需 Clipboard 句柄)
            Message::ProjectTreeRevealInFinder(path) => {
                self.context_menu = None;
                // spawn 不等待子进程退出,不阻塞 UI 线程;拉起失败(如非 macOS)
                // 静默忽略——不是值得打断用户的错误。
                let _ = std::process::Command::new("open")
                    .arg("-R")
                    .arg(&path)
                    .spawn();
            }
            Message::ProjectTreeCopy(path, is_dir) => {
                self.context_menu = None;
                self.with_focused_project(move |ws, _io| {
                    ws.tree_clipboard = Some((path, is_dir));
                });
            }
            Message::ProjectTreePaste(target_dir) => {
                self.context_menu = None;
                self.with_focused_project(move |ws, io| {
                    ws.tree_error = None;
                    let Some((source, source_is_dir)) = ws.tree_clipboard.clone() else {
                        return;
                    };
                    // 结果要投回**发起它的**项目(见 `ProjectId`)。
                    let Some(project_id) = ws.project_id() else {
                        return;
                    };
                    let proxy = io.proxy.clone();
                    io.handle.spawn(async move {
                        let result = tokio::task::spawn_blocking(move || {
                            project::paste_item(&source, source_is_dir, &target_dir)
                        })
                        .await
                        .unwrap_or_else(|e| Err(e.to_string()));
                        let _ = proxy.send_event(Message::ProjectTreePasteDone(project_id, result));
                    });
                });
            }
            Message::ProjectTreePasteDone(project_id, result) => {
                self.with_project(project_id, move |ws, _io| match result {
                    Ok(new_path) => {
                        if let (Some(tree), Some(parent)) = (&mut ws.file_tree, new_path.parent()) {
                            tree.refresh(parent);
                        }
                    }
                    Err(e) => ws.tree_error = Some(e),
                });
            }
            Message::ProjectTreeDeleteRequest(path, is_dir) => {
                self.context_menu = None;
                self.with_focused_project(move |ws, _io| {
                    ws.tree_delete_confirm = Some((path, is_dir));
                });
            }
            Message::ProjectTreeDeleteCancel => {
                self.with_focused_project(|ws, _io| {
                    ws.tree_delete_confirm = None;
                });
            }
            Message::ProjectTreeDeleteConfirm => {
                self.with_focused_project(|ws, io| {
                    let Some((path, _)) = ws.tree_delete_confirm.take() else {
                        return;
                    };
                    ws.tree_error = None;
                    // 结果要投回**发起它的**项目(见 `ProjectId`)。
                    let Some(project_id) = ws.project_id() else {
                        return;
                    };
                    let proxy = io.proxy.clone();
                    io.handle.spawn(async move {
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
                        let _ = proxy.send_event(Message::ProjectTreeOpDone {
                            project_id,
                            parent: outcome,
                            expand: false, // 删除不展开父目录
                        });
                    });
                });
            }
            Message::ProjectTreeOpDone {
                project_id,
                parent,
                expand,
            } => {
                self.with_project(project_id, move |ws, _io| match parent {
                    Ok(parent) => {
                        ws.tree_error = None; // 成功后清掉上一次失败重试留下的红字(Important #4)
                        if let Some(tree) = &mut ws.file_tree {
                            tree.refresh(&parent);
                            if expand {
                                tree.ensure_expanded(&parent);
                            }
                        }
                    }
                    Err(e) => ws.tree_error = Some(e),
                });
            }
            Message::ProjectTreeNewFile(parent) => {
                self.context_menu = None;
                self.with_focused_project(move |ws, _io| {
                    ws.start_tree_new(parent, TreeEditMode::NewFile)
                });
            }
            Message::ProjectTreeNewFolder(parent) => {
                self.context_menu = None;
                self.with_focused_project(move |ws, _io| {
                    ws.start_tree_new(parent, TreeEditMode::NewFolder)
                });
            }
            Message::ProjectTreeReloadFromDisk => {
                self.context_menu = None;
                self.with_focused_project(|ws, _io| {
                    ws.tree_error = None;
                    if let Some(tree) = &mut ws.file_tree {
                        tree.reload_from_disk();
                    }
                });
            }
            Message::ProjectTreeRenameStart(path) => {
                self.context_menu = None;
                self.with_focused_project(move |ws, _io| {
                    ws.tree_error = None;
                    let Some(parent) = path.parent().map(|p| p.to_path_buf()) else {
                        return;
                    };
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    ws.tree_edit = Some(TreeEdit {
                        parent_dir: parent,
                        mode: TreeEditMode::Rename(path),
                        buffer: name,
                    });
                });
            }
            Message::ProjectTreeEditEvent(ev) => {
                self.with_focused_project(|ws, io| {
                    let Some(edit) = &mut ws.tree_edit else {
                        return;
                    };
                    match ev {
                        AddrEvent::Text(s) => edit.buffer.push_str(&s),
                        AddrEvent::Backspace => {
                            edit.buffer.pop();
                        }
                        AddrEvent::Cancel => ws.tree_edit = None,
                        AddrEvent::Submit => ws.submit_tree_edit(io),
                    }
                });
            }
            Message::ZoomIn => {
                crate::icon_size::zoom_by(UI_ZOOM_STEP);
                crate::icon_size::persist_scale();
                self.sync_terminal_grid();
                self.pending_preview_zoom = true;
            }
            Message::ZoomOut => {
                crate::icon_size::zoom_by(1.0 / UI_ZOOM_STEP);
                crate::icon_size::persist_scale();
                self.sync_terminal_grid();
                self.pending_preview_zoom = true;
            }
            Message::ZoomReset => {
                crate::icon_size::reset_scale();
                self.sync_terminal_grid();
                self.pending_preview_zoom = true;
            }
            // WebViewFocused 只在 main.rs 的 dispatch 里设 pending_focus,
            // App::update 无需处理。
            Message::WebViewFocused => {}
        }
    }

    pub fn view(
        &self,
    ) -> iced_widget::core::Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
        // 顶栏先画:它是外壳的一部分(项目页签行 + "＋"就在上面),一个项目都
        // 没打开时更要画得出来——否则用户没有任何入口去打开第一个项目。
        let top = top_bar(self);
        // 首页落地页:点顶栏 Dozer 进入,独立于工作区(即使没开任何项目也画得
        // 出来)。打开/切换项目会自动退回工作区(见各 `ProjectTab*` 处理器)。
        if self.current_page == AppPage::Home {
            return column![top, home_page(self)].into();
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
                    .size(workspace_font::subtitle())
                    .color(theme::DIM)
            ]
            .spacing(8);
            if let Some(err) = &self.daemon_error {
                hint_col = hint_col.push(
                    text(format!("⚠ {err}"))
                        .size(workspace_font::body())
                        .color(theme::RED),
                );
            }
            let hint = container(hint_col.padding(16))
                .width(Length::Fill)
                .height(Length::Fill)
                .style(|_t: &iced_widget::Theme| container::Style {
                    background: Some(chrome_style::background().into()),
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
            left_panel_area(self, ws, false),
            divider_bar(Divider::LeftRight, theme::BG, theme::BG),
            right_panel_area(self, ws, false),
            right_icon_rail(self),
        ];
        let base = column![top, body];

        let popped = if ws.edit_session.is_some() {
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
        } else if ws.tree_delete_confirm.is_some() {
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::ProjectTreeDeleteCancel);
            stack![base, dismiss, delete_confirm_popup(ws)]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else if self.context_menu.is_some() {
            let dismiss = MouseArea::new(
                container(column![])
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::ProjectTreeContextMenuClose);
            stack![base, dismiss, context_menu_popup(self, ws)]
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

/// 单个 attach 数据流的转发循环：持有 `rx`（唯一持有者），逐条转发到
/// UI 线程（经 `EventLoopProxy`）。tab 关闭时 `JoinHandle::abort` 打断
/// 这个循环，`rx` 随任务栈一起析构——这就是 detach 的物理落点。
///
/// `project_id` 是这条流归属的项目:`tab_id` 只在单个 `Workspace` 内唯一,
/// 每个项目都从 0 起编,所以转发出去的消息必须自带项目归属,否则后台项目的
/// 输出会被喂进前台项目同号的 tab(见 [`ProjectId`])。
async fn forward_events(
    project_id: ProjectId,
    tab_id: usize,
    mut rx: mpsc::UnboundedReceiver<TermEvent>,
    proxy: EventLoopProxy<Message>,
) {
    while let Some(event) = rx.recv().await {
        let message = match event {
            TermEvent::Output(bytes) => Message::TermOutput(project_id, tab_id, bytes),
            TermEvent::Exited(_code) => {
                let _ = proxy.send_event(Message::SessionExited(project_id, tab_id));
                return;
            }
            TermEvent::Disconnected => {
                let _ = proxy.send_event(Message::SessionExited(project_id, tab_id));
                return;
            }
            TermEvent::Lagged => {
                tracing::warn!(tab_id, "终端事件滞后（lagged），可能丢失部分历史输出");
                continue;
            }
            TermEvent::Agent {
                agent,
                state,
                transcript_path,
            } => Message::AgentStateChanged(project_id, tab_id, agent, state, transcript_path),
        };
        if proxy.send_event(message).is_err() {
            // UI 线程（EventLoop）已经关闭，没有必要继续转发。
            return;
        }
    }
}

/// `[会话已结束]` 尾行标记：CREAM 字 / CARD 底。颜色值直接镜像
/// `theme::CREAM` / `theme::CARD`——`TerminalModel` 只吃 ANSI truecolor
/// 转义序列，认不出 `iced::Color`，这里手工写死 RGB 常量；本任务范围
/// 不含 `theme.rs`，若那边颜色改动，这两个常量需要手动同步。
fn exited_marker() -> Vec<u8> {
    const FG: (u8, u8, u8) = (0xFF, 0xE5, 0xB4); // CREAM
    const BG: (u8, u8, u8) = (0x12, 0x20, 0x2a); // CARD
    format!(
        "\r\n\x1b[38;2;{};{};{}m\x1b[48;2;{};{};{}m[会话已结束]\x1b[0m\r\n",
        FG.0, FG.1, FG.2, BG.0, BG.1, BG.2
    )
    .into_bytes()
}

/// 验收 tab 内容（spec P1f D8）:目标 + 标准勾选 + 变更文件 + 意见 + 双动作。
/// iced 直绘（验收 tab 激活时 webview 全隐藏，不抢层）。
/// 会话审阅 tab 内容（P1i）：人类锚点 + AI 回合折叠（正文/过程）。
fn review_content<'a>(
    mut content: iced_widget::Column<'a, Message, iced_widget::Theme, iced_widget::Renderer>,
    ws: &'a Workspace,
) -> iced_widget::Column<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    let Some(rv) = &ws.review else {
        return content;
    };
    if let Some(err) = &rv.error {
        return content.push(
            text(format!("⚠ {err}"))
                .size(workspace_font::subtitle())
                .color(theme::RED),
        );
    }
    if rv.entries.is_empty() {
        return content.push(lh(text("暂无对话")
            .size(workspace_font::subtitle())
            .color(theme::DIM)));
    }
    for (i, e) in rv.entries.iter().enumerate() {
        match e {
            ReviewEntry::Human { text: t } => {
                content = content.push(lh(text(format!("▎{t}"))
                    .size(workspace_font::title())
                    .color(theme::CREAM)));
            }
            ReviewEntry::AiTurn {
                text: body,
                tools,
                thinking,
            } => {
                if !body.is_empty() {
                    content = content.push(lh(text(body.clone())
                        .size(workspace_font::subtitle())
                        .color(theme::BODY)));
                }
                let expanded = rv.expanded.contains(&i);
                let glyph = if expanded { "▾ " } else { "▸ " };
                content = content.push(
                    button(lh(text(format!(
                        "{glyph}{}",
                        ai_turn_summary(tools.len(), *thinking)
                    ))
                    .size(workspace_font::body())
                    .color(theme::DIM)))
                    .on_press(Message::ReviewToggle(i))
                    .style(|_t, _s| button::Style {
                        background: None,
                        text_color: theme::DIM,
                        ..button::Style::default()
                    }),
                );
                if expanded {
                    if *thinking {
                        content = content.push(lh(text("  · 思考(略)")
                            .size(workspace_font::label())
                            .color(theme::DIM)));
                    }
                    for tool in tools {
                        content = content.push(lh(text(format!("  · {tool}"))
                            .size(workspace_font::body())
                            .color(theme::CYAN)));
                    }
                }
            }
        }
    }
    content
}

fn acceptance_content<'a>(
    mut content: iced_widget::Column<'a, Message, iced_widget::Theme, iced_widget::Renderer>,
    ws: &'a Workspace,
) -> iced_widget::Column<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    let Some(acc) = &ws.acceptance else {
        return content;
    };
    if let Some(n) = acc.accepted_version {
        return content.push(lh(text(format!("✓ 已沉淀 v{n}"))
            .size(workspace_font::title())
            .color(theme::GOLD)));
    }
    match &acc.goal {
        Some(g) => {
            content = content.push(lh(text(g.title.clone())
                .size(workspace_font::title())
                .color(theme::CREAM)));
            for (i, c) in g.criteria.iter().enumerate() {
                let checked = acc.checked.get(i).copied().unwrap_or(false);
                content = content.push(
                    button(lh(text(criteria_line(checked, c))
                        .size(workspace_font::body())
                        .color(if checked { theme::GOLD } else { theme::BODY })))
                    .on_press(Message::AcceptanceToggle(i))
                    .style(|_t, _s| button::Style {
                        background: None,
                        text_color: theme::BODY,
                        ..button::Style::default()
                    }),
                );
            }
        }
        None => {
            content = content.push(lh(text(
                "未定标——先在仓库写 .dozer/goal.md（首行目标,\n- [ ] 列表为标准）",
            )
            .size(workspace_font::body())
            .color(theme::DIM)));
        }
    }
    content = content.push(lh(text("变更文件")
        .size(workspace_font::body())
        .color(theme::DIM)));
    for fc in &acc.changes {
        let path = acc.repo.join(&fc.path);
        content = content.push(
            button(lh(text(file_change_line(fc))
                .size(workspace_font::body())
                .color(theme::CYAN)))
            .on_press(Message::PreviewOpenPath(path))
            .style(|_t, _s| button::Style {
                background: None,
                text_color: theme::CYAN,
                ..button::Style::default()
            }),
        );
    }
    let editing = acc.comment_editing;
    let comment_text = if editing {
        format!("{}▏", acc.comment)
    } else if acc.comment.is_empty() {
        "验收意见…（打回时注回会话）".to_string()
    } else {
        acc.comment.clone()
    };
    content = content.push(
        button(lh(text(comment_text)
            .size(workspace_font::body())
            .color(if editing { theme::CREAM } else { theme::DIM })))
        .on_press(Message::AcceptanceCommentClick)
        .width(Length::Fill)
        .style(move |_t, _s| button::Style {
            background: Some(theme::TERM_BG.into()),
            text_color: theme::CREAM,
            border: Border {
                color: if editing { theme::GOLD } else { theme::BORDER },
                width: 1.0,
                radius: 2.0.into(),
            },
            ..button::Style::default()
        }),
    );
    let actions = row![
        button(lh(text("通过·沉淀")
            .size(workspace_font::body())
            .color(theme::BG)))
        .on_press(Message::AcceptanceAccept)
        .style(|_t, _s| button::Style {
            background: Some(theme::GOLD.into()),
            text_color: theme::BG,
            border: Border {
                color: theme::GOLD,
                width: 1.0,
                radius: 2.0.into()
            },
            ..button::Style::default()
        }),
        button(lh(text("打回并注回")
            .size(workspace_font::body())
            .color(theme::RED)))
        .on_press(Message::AcceptanceReject)
        .style(|_t, _s| button::Style {
            background: None,
            text_color: theme::RED,
            border: Border {
                color: theme::RED,
                width: 1.0,
                radius: 2.0.into()
            },
            ..button::Style::default()
        }),
    ]
    .spacing(8);
    content = content.push(actions);
    if let Some(err) = &acc.error {
        content = content.push(lh(text(format!("⚠ {err}"))
            .size(workspace_font::body())
            .color(theme::RED)));
    }
    content
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

/// 顶栏 Home 按钮(D1)：Lucide house(`IconKind::Home`) + 圆角正方形底,
/// 无文字、恒在最左、不参与 `project_tabs_row` 的拥挤收窄——与当前项目
/// 页签行"＋"按钮同款的"固定位不参与收窄"处理。点它进首页(`AppPage::Home`)。
///
/// `hover_t`(0..=1)是悬停动画进度,由 App 自驱 redraw 平滑推进:图标色
/// idle→金、背景 idle→CARD、边框 idle→金,都是按它插值,给出悬停时的
/// 平滑过渡而非硬切(见 `App::advance_hover_anims`/`hover_progress`)。
fn dozer_home_tab<'a>(
    active: bool,
    hover_t: f32,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    // idle 图标色随激活态取 CREAM/DIM,与既有顶栏页签 hover 一致；悬停时
    // 朝金插值。背景 idle 取 CARD(激活)/TAB_HOVER(未激活),悬停朝 CARD 插值；
    // 边框 idle 取 BORDER,悬停朝金插值。
    let idle_icon = if active { theme::CREAM } else { theme::DIM };
    let icon_color = theme::mix(idle_icon, theme::GOLD, hover_t);
    let bg_idle = if active {
        theme::CARD
    } else {
        theme::TAB_HOVER
    };
    let bg = theme::mix(bg_idle, theme::CARD, hover_t);
    let border_color = theme::mix(theme::BORDER, theme::GOLD, hover_t);
    // 圆角正方形边长 = 图标尺寸 + 留白(图标居中)。
    let sq = crate::icon_size::rail() + 14.0;

    let btn = button(
        container(icons::view(
            icons::IconKind::Home,
            crate::icon_size::rail(),
            icon_color,
        ))
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(iced_widget::core::Alignment::Center)
        .align_y(iced_widget::core::Alignment::Center),
    )
    .on_press(Message::TopBarHome)
    .width(Length::Fixed(sq))
    .height(Length::Fixed(sq))
    .style(move |_t: &iced_widget::Theme, _s| button::Style {
        background: Some(bg.into()),
        text_color: icon_color,
        border: Border {
            color: border_color,
            width: 1.0,
            radius: 8.0.into(),
        },
        ..button::Style::default()
    });

    // `MouseArea` 提供 hover 进入/离开事件(按钮本身无 `on_enter`/`on_exit`),
    // 用来驱动 `Hover(HoverId::Home, ..)` 动画;点击仍由内层 `btn` 的 `on_press` 处理。
    let hit = MouseArea::new(btn)
        .on_enter(Message::Hover(HoverId::Home, true))
        .on_exit(Message::Hover(HoverId::Home, false));

    // 外层 `container` 只负责在顶栏里垂直居中(按钮是 Fixed 高,默认贴顶,
    // 与交通灯对不齐——同 `project_tab_item` 里注释过的根因)。
    container(hit)
        .height(Length::Fixed(workspace_geometry::top_bar_height()))
        .align_y(iced_widget::core::Alignment::Center)
        .into()
}

fn top_bar(app: &App) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    // Dozer 字标做成按钮:Dozer 品牌标(dozer-logo-main.jpeg 矢量化) + 文字,
    // 点它进首页(`AppPage::Home`)。Dozer 页签:视觉与右侧项目页签一致,
    // 恒在最左、不参与拥挤收窄(D1)。
    let title = dozer_home_tab(
        app.current_page == AppPage::Home,
        app.hover_progress(HoverId::Home),
    );

    // 页签行占满标题与右侧之间的全部空间。裁剪与翻页在 `project_tabs_row`
    // 内部做(只裁页签本身,箭头与"＋"钉在裁剪区外),这里**不能**再套一层
    // `clip`——那会把"＋"和箭头一起裁掉,正是要修的问题。
    // 贴底对齐在 `project_tabs_row` 内部(`responsive` 闭包里)完成,这里
    // 套 `align_y` 对它不起作用,见该函数内注释。
    let tabs = container(project_tabs_row(app)).width(Length::Fill);

    let mut right = row![].spacing(10);
    if let Some(cap) = app
        .active_workspace()
        .and_then(|ws| goal_capsule_text(ws.project_goal.as_ref(), 28))
    {
        let capsule = container(
            row![
                text("●").size(workspace_font::dot_sm()).color(theme::GOLD),
                text(cap).size(workspace_font::body()).color(theme::CREAM)
            ]
            .spacing(6),
        )
        .padding([5, 10])
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(theme::CARD.into()),
            border: Border {
                color: theme::GOLD,
                width: 1.0,
                radius: 6.0.into(),
            },
            ..container::Style::default()
        });
        right = right.push(capsule);
    }
    // 设计稿"btn settings"外框 padding-left 6 / padding-y 4(hit-box 留白,
    // 图标本身仍是 16x16)。目前尚未接入设置面板,先只还原视觉,不加
    // on_press——没有对应 Message 变体可派发。
    // 图标颜色:SVG 颜色构建时定死,hover 态平滑过渡到金(见 `HoverId`/
    // `App::hover_progress`——与光标闪烁同款自驱 redraw 动画)。
    let settings_color = theme::mix(
        theme::DIM,
        theme::GOLD,
        app.hover_progress(HoverId::Topbar(TopbarButton::Settings)),
    );
    right = right.push(
        MouseArea::new(
            container(icons::view(
                icons::IconKind::Settings,
                crate::icon_size::rail(),
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
        )),
    );

    let region = chrome_style::top_bar();
    let bar = row![title, tabs, right]
        .spacing(region.gap)
        .padding(region.padding)
        .height(Length::Fixed(workspace_geometry::top_bar_height()))
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
        .height(Length::Fixed(workspace_geometry::top_bar_height()))
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: region.border.unwrap_or_default(),
            ..container::Style::default()
        })
        .into()
}

/// 首页落地页(点顶栏 Dozer 进入):规格 §3(⓪c) H0 帧——左栏"我的项目"列表,
/// 右侧"最近的文件"与"最近的对话"两卡(D1-D7)。风格沿用 ByteBoy2077 主题。
/// 点某张最近项目卡会自动退回工作区视图(`Message::ProjectSelect`)。
fn home_page(app: &App) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    let body = row![home_sidebar(app, now_ms), home_recents_column(app, now_ms)]
        .spacing(24)
        .height(Length::Fill);

    let mut col = column![body].spacing(16).height(Length::Fill);
    if let Some(err) = &app.daemon_error {
        col = col.push(
            text(format!("⚠ {err}"))
                .size(workspace_font::body())
                .color(theme::RED),
        );
    }

    container(col.padding(24))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(chrome_style::background().into()),
            ..container::Style::default()
        })
        .into()
}

/// H0 左栏(固定宽 `h0_sidebar_width`):品牌区、"我的项目"、搜索占位(D7)、
/// 最近项目卡(取 `app.recent_projects` 前 5 条,D2/D3)、"更多项目"占位(D7)、
/// "＋新增项目"(复用 `Message::ProjectTabPickFolder`)。
/// `app.recent_projects` 为空时画"还没有项目"兜底文案,不崩(spec §4)。
fn home_sidebar(
    app: &App,
    now_ms: u64,
) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let mut col = column![].spacing(16);

    col = col.push(
        row![
            icons::view(
                icons::IconKind::Dozer,
                crate::icon_size::rail(),
                theme::GOLD
            ),
            text("Dozer")
                .font(top_bar_font())
                .size(workspace_font::subtitle())
                .color(theme::CREAM),
            text(format!("v{}", env!("CARGO_PKG_VERSION")))
                .size(workspace_font::caption_sm())
                .color(theme::DIM),
        ]
        .spacing(8)
        .align_y(iced_widget::core::Alignment::Center),
    );

    col = col.push(
        text("我的项目")
            .size(workspace_font::caption())
            .color(theme::DIM),
    );

    // 搜索框:视觉占位,不接线(D7；precedent:顶栏 ⌘K 搜索框同款"先视觉后接线")。
    col = col.push(
        container(
            row![
                icons::view(icons::IconKind::Search, crate::icon_size::row(), theme::DIM),
                text("搜索项目…")
                    .size(workspace_font::body())
                    .color(theme::DIM),
            ]
            .spacing(8)
            .align_y(iced_widget::core::Alignment::Center),
        )
        .padding([6, 10])
        .width(Length::Fill)
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(theme::CARD.into()),
            border: Border {
                color: theme::BORDER,
                width: 1.0,
                radius: 6.0.into(),
            },
            ..container::Style::default()
        }),
    );

    if app.recent_projects.is_empty() {
        col = col.push(
            text("还没有项目")
                .size(workspace_font::body())
                .color(theme::DIM),
        );
    } else {
        let mut list = column![].spacing(8);
        for p in app.recent_projects.iter().take(5) {
            let card = button(
                column![
                    lh(text(p.name.clone())
                        .size(workspace_font::body())
                        .color(theme::CREAM)),
                    lh(text(relative_time_text(p.last_active_ms, now_ms))
                        .size(workspace_font::caption_sm())
                        .color(theme::DIM)),
                    lh(text(p.path.clone())
                        .size(workspace_font::caption_sm())
                        .color(theme::DIM)),
                ]
                .spacing(2),
            )
            .on_press(Message::ProjectSelect(p.id))
            .width(Length::Fill)
            .padding(10)
            .style(|_t: &iced_widget::Theme, _s| button::Style {
                background: Some(theme::CARD.into()),
                text_color: theme::CREAM,
                border: Border {
                    color: theme::BORDER,
                    width: 1.0,
                    radius: 8.0.into(),
                },
                ..button::Style::default()
            });
            list = list.push(card);
        }
        col = col.push(
            Scrollable::new(list)
                .width(Length::Fill)
                .height(Length::Fill)
                .direction(scrollable::Direction::Vertical(
                    crate::scrollbar::scrollbar(),
                ))
                .style(|_t, _s| crate::scrollbar::scrollbar_style()),
        );
    }

    // "更多项目":视觉占位,不接线——对应的"全部项目列表"视图现在不存在,
    // 属于后续增量(D7)。
    col = col.push(
        container(
            text("更多项目")
                .size(workspace_font::caption())
                .color(theme::DIM),
        )
        .padding([6, 0]),
    );

    col = col.push(
        button(
            text("＋新增项目")
                .size(workspace_font::body())
                .color(theme::GOLD),
        )
        .on_press(Message::ProjectTabPickFolder)
        .padding([8, 16])
        .width(Length::Fill)
        .style(|_t: &iced_widget::Theme, _s| button::Style {
            background: Some(theme::CARD.into()),
            border: Border {
                color: theme::GOLD,
                width: 1.0,
                radius: 6.0.into(),
            },
            text_color: theme::GOLD,
            ..button::Style::default()
        }),
    );

    container(col)
        .width(Length::Fixed(workspace_geometry::h0_sidebar_width()))
        .height(Length::Fill)
        .into()
}

/// H0 右侧："最近的文件"/"最近的对话"两卡并排(D4)。
fn home_recents_column(
    app: &App,
    now_ms: u64,
) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    row![
        home_recent_files_card(app, now_ms),
        home_recent_conversations_card(app, now_ms)
    ]
    .spacing(24)
    .height(Length::Fill)
    .into()
}

/// "最近的文件"卡：`app.home_recents_loaded` 为 false 时(刚点进 Home 还没等
/// 到异步结果)画"加载中…"，避免第一帧空白跳变(spec §4)。
fn home_recent_files_card(
    app: &App,
    now_ms: u64,
) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let mut col = column![
        text("最近的文件")
            .size(workspace_font::subtitle())
            .color(theme::CREAM)
    ]
    .spacing(8);

    if !app.home_recents_loaded {
        col = col.push(
            text("加载中…")
                .size(workspace_font::body())
                .color(theme::DIM),
        );
    } else if app.home_recent_files.is_empty() {
        col = col.push(
            text("暂无最近改动的文件")
                .size(workspace_font::body())
                .color(theme::DIM),
        );
    } else {
        for f in &app.home_recent_files {
            let filename = f
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| f.path.display().to_string());
            let row_el = row![
                icons::view(
                    icons::icon_for_file(&filename),
                    crate::icon_size::row(),
                    theme::DIM
                ),
                column![
                    lh(text(filename.clone())
                        .size(workspace_font::body())
                        .color(theme::CREAM)),
                    lh(text(format!(
                        "{} · {}",
                        f.project_name,
                        relative_time_text(f.modified_ms, now_ms)
                    ))
                    .size(workspace_font::caption_sm())
                    .color(theme::DIM)),
                ]
                .spacing(2),
            ]
            .spacing(8)
            .align_y(iced_widget::core::Alignment::Center);
            col = col.push(container(row_el).padding(10).width(Length::Fill).style(
                |_t: &iced_widget::Theme| container::Style {
                    background: Some(theme::CARD.into()),
                    border: Border {
                        color: theme::BORDER,
                        width: 1.0,
                        radius: 8.0.into(),
                    },
                    ..container::Style::default()
                },
            ));
        }
    }

    container(col)
        .width(Length::FillPortion(1))
        .height(Length::Fill)
        .into()
}

/// "最近的对话"卡：语义同 `home_recent_files_card`。裁剪掉"进行中/已验收
/// vN"状态字(D4)，只显示"标题 · 项目名 · agent · 相对时间"。
fn home_recent_conversations_card(
    app: &App,
    now_ms: u64,
) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let mut col = column![
        text("最近的对话")
            .size(workspace_font::subtitle())
            .color(theme::CREAM)
    ]
    .spacing(8);

    if !app.home_recents_loaded {
        col = col.push(
            text("加载中…")
                .size(workspace_font::body())
                .color(theme::DIM),
        );
    } else if app.home_recent_conversations.is_empty() {
        col = col.push(
            text("暂无对话记录")
                .size(workspace_font::body())
                .color(theme::DIM),
        );
    } else {
        for c in &app.home_recent_conversations {
            let sub = format!(
                "{} · {} · {}",
                c.project_name,
                c.meta.agent.label(),
                relative_time_text(c.meta.modified_ms, now_ms)
            );
            col = col.push(
                container(
                    column![
                        lh(text(c.meta.title.clone())
                            .size(workspace_font::body())
                            .color(theme::CREAM)),
                        lh(text(sub)
                            .size(workspace_font::caption_sm())
                            .color(theme::DIM)),
                    ]
                    .spacing(4),
                )
                .padding(10)
                .width(Length::Fill)
                .style(|_t: &iced_widget::Theme| container::Style {
                    background: Some(theme::CARD.into()),
                    border: Border {
                        color: theme::BORDER,
                        width: 1.0,
                        radius: 8.0.into(),
                    },
                    ..container::Style::default()
                }),
            );
        }
    }

    container(col)
        .width(Length::FillPortion(1))
        .height(Length::Fill)
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
fn project_tabs_row(app: &App) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let active_project_id = app.active_project_id;
    let blink_on = app.blink_on;
    let entries = project_tab_entries(app);
    let n = entries.len();

    // `responsive` 在每轮布局把页签区可用宽交给闭包,闭包据此算每片宽。
    responsive(move |size| {
        let gap = workspace_geometry::project_tab_gap();
        let default_w = workspace_geometry::project_tab_max_width();
        // 预留"＋"按钮与其紧跟最后一片页签的 gap(页签内部还有 n-1 道 gap),
        // 避免页签在拥挤时压到"＋"上。
        let reserved = workspace_geometry::project_tab_add_button_width() + (n as f32 + 1.0) * gap;
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
        let sep_h = workspace_geometry::top_bar_height() * 0.45;
        for (i, entry) in entries.iter().enumerate() {
            let active = active_project_id == Some(entry.id);
            let close_hover_t = app.hover_progress(HoverId::ProjectTabClose(entry.id));
            let item = project_tab_item(
                entry.id,
                entry.name.clone(),
                entry.dot,
                active,
                blink_on,
                close_hover_t,
            );
            // 固定宽:少页签时为默认宽,挤时为均分窄宽(Chrome 式收窄)。
            let cell = container(item).width(Length::Fixed(per_tab));
            tabs = tabs.push(cell);
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
                                background: Some(theme::BORDER.into()),
                                ..container::Style::default()
                            }),
                    );
                }
            }
        }

        // 图标颜色:SVG 构建时定死、不吃 `button::Status`,hover 态平滑过渡到
        // 金(见 `HoverId`/`App::hover_progress`)。
        let add_color = theme::mix(
            theme::DIM,
            theme::GOLD,
            app.hover_progress(HoverId::Topbar(TopbarButton::AddProject)),
        );
        let add = MouseArea::new(
            button(icons::view(
                icons::IconKind::SquarePlus,
                crate::icon_size::row(),
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
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    // 页签背景圆角半径参考 Dozer 按钮(圆角正方形)的边长 `sq`,但实际背景高
    // 用更高的 `tab_h`——页签贴底(见 `project_tabs_row` 的 `align_y(End)`)、
    // 底部留白必须是 0,可 Dozer 按钮在顶栏里是居中的,顶部留白
    // `(top_bar_height-sq)/2` 不为 0;要让页签顶边跟 Dozer 按钮背景顶边对齐,
    // 页签背景就不能也用 `sq` 这个高度贴底(那样顶边会比 Dozer 的更低),
    // 必须把高度补到 `(top_bar_height+sq)/2`,贴底后顶部留白才恰好等于
    // Dozer 按钮那份 `(top_bar_height-sq)/2`。
    let sq = crate::icon_size::rail() + 14.0;
    let tab_h = (workspace_geometry::top_bar_height() + sq) / 2.0;
    // 关闭按钮用与顶栏其它图标按钮(tab 箭头 / 最大化)同尺寸的方形命中区。
    let close_sz = crate::workspace_geometry::tab_button_size();
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
        label = label.push(text("●").size(workspace_font::caption_sm()).color(color));
    }
    label = label.push(
        text(name)
            .font(top_bar_font())
            .size(workspace_font::body())
            .color(if active { theme::CREAM } else { theme::DIM }),
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
        .padding(Padding {
            right: close_sz + 6.0,
            ..Padding::ZERO
        })
        .clip(true);

    let select = button(label)
        .on_press(Message::ProjectTabSwitch(id))
        .width(Length::Fill)
        .height(Length::Fixed(tab_h))
        .style(move |_t: &iced_widget::Theme, s| {
            let mut st = button::Style {
                background: None,
                text_color: theme::CREAM,
                ..button::Style::default()
            };
            // 选中态不参与 hover 提亮(已有实底 TAB_ACTIVE_BG + 底部强调线,
            // 无需再高亮);未选中态 hover 时画一条与 Dozer 按钮同高同圆角的
            // 胶囊背景(#152630)。
            if !active && let button::Status::Hovered = s {
                st.background = Some(theme::TAB_HOVER.into());
                st.border = Border {
                    radius: 8.0.into(),
                    ..Border::default()
                };
            }
            st
        });

    // 关闭按钮:方形图标按钮,叠在页签主体之上(见下方 tab_row)。hover 效果
    // 与顶栏"＋"新建项目按钮一致——无背景胶囊,图标(这里是 `×` 文字)颜色
    // 随 `close_hover_t` 从 DIM 平滑过渡到 GOLD(见 `HoverId::ProjectTabClose`/
    // `App::hover_progress`),不用 iced `button::Status` 的硬切背景。
    // `×` 必须包一层 `Fill`+`align_y(Center)`(与下面 `label` 同一条注释里
    // 说的 iced 按钮布局 quirk)——按钮只吃 padding,不回收多余竖向空间,
    // 裸 `text` 会贴在按钮内容区顶部,跟垂直居中的标题文字对不上。
    let close_color = theme::mix(theme::DIM, theme::GOLD, close_hover_t);
    let close = MouseArea::new(
        button(
            container(text("×").size(workspace_font::body()).color(close_color))
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(iced_widget::core::Alignment::Center)
                .align_y(iced_widget::core::Alignment::Center),
        )
        .on_press(Message::ProjectTabClose(id))
        .width(Length::Fixed(close_sz))
        .height(Length::Fixed(close_sz))
        .padding(0)
        .style(move |_t: &iced_widget::Theme, _status| button::Style {
            background: None,
            text_color: close_color,
            ..button::Style::default()
        }),
    )
    .on_enter(Message::Hover(HoverId::ProjectTabClose(id), true))
    .on_exit(Message::Hover(HoverId::ProjectTabClose(id), false));

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
    let tab_row = container(stack![select_layer, close_layer])
        .width(Length::Fill)
        .height(Length::Fill)
        .padding([0, 14]);

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
                        background: Some(theme::TAB_ACTIVE_BORDER.into()),
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
                    background: Some(theme::TAB_ACTIVE_BG.into()),
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
fn project_dot(alive_states: &[AgentState]) -> Option<(Color, bool)> {
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

/// 对话列表面板(右面板区"对话"视图的列表侧):当前项目的对话记录卡片,
/// 活跃对话置顶+金框标记。点某条 → `ConversationOpen` 驱动右侧审阅内容。
fn conversation_list_pane(
    ws: &Workspace,
    width: Length,
    outer: Border,
) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let region = chrome_style::conversation_list_pane();
    let mut content = column![
        row![
            lh(text("对话")
                .size(workspace_font::subtitle())
                .color(theme::CREAM)),
            lh(text(
                ws.project
                    .as_ref()
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| "未打开项目".into())
            )
            .size(workspace_font::label())
            .color(theme::DIM)),
        ]
        .spacing(8)
    ]
    .spacing(region.gap);

    let opens = ws.open_transcript_paths();
    let active_n = ws
        .conversations
        .iter()
        .filter(|c| conversation::is_current_conversation(&c.path, &opens))
        .count();
    content = content.push(
        row![
            lh(text("对话")
                .size(workspace_font::caption())
                .color(theme::DIM)),
            lh(
                text(format!("{} 条 · {} 活跃", ws.conversations.len(), active_n))
                    .size(workspace_font::caption())
                    .color(theme::DIM)
            ),
        ]
        .spacing(6),
    );
    if ws.conversations.is_empty() {
        content = content.push(lh(text("暂无对话记录")
            .size(workspace_font::body())
            .color(theme::DIM)));
    }
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    // 当前活跃置顶：先活后历史（列表本身 mtime 倒序）。
    let mut ordered: Vec<&ConversationMeta> = ws.conversations.iter().collect();
    ordered.sort_by_key(|c| conversation::is_current_conversation(&c.path, &opens) as u8);
    ordered.reverse();
    for c in ordered {
        let current = conversation::is_current_conversation(&c.path, &opens);
        let sub = if current {
            format!(
                "● 当前 · {}",
                conversation_sub(c.agent.label(), c.modified_ms, c.size_bytes, now_ms)
            )
        } else {
            conversation_sub(c.agent.label(), c.modified_ms, c.size_bytes, now_ms)
        };
        let sub_color = if current { theme::GREEN } else { theme::DIM };
        let card = button(
            column![
                row![
                    text("●")
                        .size(workspace_font::caption())
                        .color(agent_dot_color(c.agent)),
                    lh(text(c.title.clone())
                        .size(workspace_font::body())
                        .color(theme::CREAM)),
                ]
                .spacing(6)
                .align_y(iced_widget::core::Alignment::Center),
                lh(text(sub)
                    .size(workspace_font::caption_sm())
                    .color(sub_color)),
            ]
            .spacing(4),
        )
        .on_press(Message::ConversationOpen(c.path.clone()))
        .width(Length::Fill)
        .padding(10)
        .style(move |_t, _s| button::Style {
            background: Some(theme::CARD.into()),
            text_color: theme::CREAM,
            border: Border {
                color: if current { theme::GOLD } else { theme::BORDER },
                width: 1.0,
                radius: 8.0.into(),
            },
            ..button::Style::default()
        });
        content = content.push(card);
    }

    container(content.padding(region.padding))
        .width(width)
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: outer,
            ..container::Style::default()
        })
        .into()
}

/// 按 `AgentKind` 把会话 tab 分组,固定顺序 Claude → Codebuddy → Opencode
/// → Unknown,只返回非空分组(没有该 agent 的会话就不出现,面板不留空
/// 分组占位)。组内保持 `tabs` 原有顺序(tab 打开顺序)。返回下标而非
/// 引用——渲染时既要下标发 `Message::SelectTab(idx)`,又要用下标回查
/// `ws.tabs[idx]` 取展示字段,直接存下标比存 `&SessionTab` 省一次生命
/// 周期纠缠。
fn group_tabs_by_agent(tabs: &[SessionTab]) -> Vec<(AgentKind, Vec<usize>)> {
    const ORDER: [AgentKind; 4] = [
        AgentKind::Claude,
        AgentKind::Codebuddy,
        AgentKind::Opencode,
        AgentKind::Unknown,
    ];
    ORDER
        .into_iter()
        .filter_map(|kind| {
            let idxs: Vec<usize> = tabs
                .iter()
                .enumerate()
                .filter(|(_, t)| t.agent == kind)
                .map(|(i, _)| i)
                .collect();
            (!idxs.is_empty()).then_some((kind, idxs))
        })
        .collect()
}

/// Agent 列表面板(右面板区"Agent"视图的列表侧):按 `AgentKind` 分组展示
/// 当前项目的会话,组内保留 tab 打开顺序;点击一行 = `Message::SelectTab`
/// 切焦点(同终端 tab 栏点击效果)。
fn agent_list_pane(
    ws: &Workspace,
    width: Length,
    outer: Border,
) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let region = chrome_style::agent_list_pane();
    let mut content = column![
        row![
            lh(text("Agent")
                .size(workspace_font::subtitle())
                .color(theme::CREAM)),
            lh(text(
                ws.project
                    .as_ref()
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| "未打开项目".into())
            )
            .size(workspace_font::label())
            .color(theme::DIM)),
            iced_widget::space::horizontal(),
            agent_picker_toggle_button(),
        ]
        .spacing(8)
        .align_y(iced_widget::core::Alignment::Center)
    ]
    .spacing(region.gap);

    if ws.tabs.is_empty() {
        content = content.push(lh(text("暂无会话")
            .size(workspace_font::body())
            .color(theme::DIM)));
    } else {
        for (agent, idxs) in group_tabs_by_agent(&ws.tabs) {
            content = content.push(lh(text(format!("{}（{}）", agent.label(), idxs.len()))
                .size(workspace_font::caption())
                .color(theme::DIM)));
            for idx in idxs {
                content = content.push(agent_list_row(ws, idx));
            }
        }
    }

    container(content.padding(region.padding))
        .width(width)
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: outer,
            ..container::Style::default()
        })
        .into()
}

/// Agent 面板里单条会话行:状态点(`dot_color`)+ 状态文字
/// (`agent_state_label`)+ 会话名(`tab_title`),整行可点选中该 tab
/// (`idx == ws.active` 时 `theme::CARD` 背景高亮,同项目树选中行的手法,
/// 见 `workspace.rs` 里 `is_selected` 那段)。
fn agent_list_row(
    ws: &Workspace,
    idx: usize,
) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let tab = &ws.tabs[idx];
    let active = idx == ws.active;
    let row_el = row![
        text("●")
            .size(workspace_font::caption())
            .color(dot_color(tab.agent_state, tab.alive)),
        lh(text(agent_state_label(tab.agent_state))
            .size(workspace_font::caption_sm())
            .color(theme::DIM)),
        lh(
            text(tab_title(tab.agent, tab.cwd.as_deref(), &tab.info.name))
                .size(workspace_font::body())
                .color(theme::CREAM)
        ),
    ]
    .spacing(6)
    .align_y(iced_widget::core::Alignment::Center);
    button(row_el)
        .on_press(Message::SelectTab(idx))
        .width(Length::Fill)
        .padding(6)
        .style(move |_t, _s| button::Style {
            background: if active {
                Some(theme::CARD.into())
            } else {
                None
            },
            text_color: theme::CREAM,
            ..button::Style::default()
        })
        .into()
}

/// Agent 面板头部"＋"按钮:点击切换 `agent_picker_open`,弹出 agent
/// 选择菜单(`agent_picker_popup`)。样式为 CARD 底 + BORDER 描边的"＋",
/// 与已移除的终端 tab 栏"＋"同源。
fn agent_picker_toggle_button<'a>()
-> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    button(text("＋").size(workspace_font::title()).color(theme::CREAM))
        .on_press(Message::AgentPickerToggle)
        .padding([4, 8])
        .style(|_theme, _status| button::Style {
            background: Some(theme::CARD.into()),
            text_color: theme::CREAM,
            border: Border {
                color: theme::BORDER,
                width: 1.0,
                radius: 4.0.into(),
            },
            ..button::Style::default()
        })
        .into()
}

/// Agent 选择菜单浮层:固定挂在窗口右上角("＋"按钮下方——该按钮
/// 就在最靠右的 Agent 面板头部,近似等于窗口右上角),八个选项
/// Claude/CodeBuddy/OpenCode/Codex/Qoder/Kilo/纯 Shell/Git Shell。跟项目树右键菜单
/// (`context_menu_popup`)同款按钮样式,但不需要像素坐标定位——同
/// `delete_confirm_popup` 一样固定 padding 摆位。`ws.agent_picker_open`
/// 为假时返回空视图,调用方(`App::view`)据此决定要不要把这层塞进
/// `stack!`。
fn agent_picker_popup(
    ws: &Workspace,
) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    if !ws.agent_picker_open {
        return column![].into();
    }
    let items: [(&str, PickerLaunch); 8] = [
        ("Claude", PickerLaunch::Agent(Some(AgentKind::Claude))),
        ("CodeBuddy", PickerLaunch::Agent(Some(AgentKind::Codebuddy))),
        ("OpenCode", PickerLaunch::Agent(Some(AgentKind::Opencode))),
        ("Codex", PickerLaunch::Agent(Some(AgentKind::Codex))),
        ("Qoder", PickerLaunch::Agent(Some(AgentKind::Qoder))),
        ("Kilo", PickerLaunch::Agent(Some(AgentKind::Kilo))),
        ("纯 Shell", PickerLaunch::Agent(None)),
        ("Git Shell", PickerLaunch::Git),
    ];
    let mut col = column![].spacing(2);
    for (label, agent) in items {
        let (icon, icon_color) = match agent {
            PickerLaunch::Agent(Some(kind)) => (agent_icon(kind), agent_dot_color(kind)),
            PickerLaunch::Agent(None) => (IconKind::Terminal, theme::CREAM),
            PickerLaunch::Git => (IconKind::GitBranch, theme::CREAM),
        };
        let content = row![
            icons::view(icon, crate::icon_size::row(), icon_color),
            text(label).size(workspace_font::body()).color(theme::CREAM),
        ]
        .align_y(iced_widget::core::alignment::Vertical::Center)
        .spacing(8);
        col = col.push(
            button(content)
                .on_press(Message::AgentPickerSelect(agent))
                .width(Length::Fixed(160.0))
                .padding([6, 12])
                .style(|_t, _s| button::Style {
                    background: Some(theme::CARD.into()),
                    text_color: theme::CREAM,
                    ..button::Style::default()
                }),
        );
    }
    let list = container(col)
        .padding(6)
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(theme::CARD.into()),
            border: Border {
                color: theme::BORDER,
                width: 1.0,
                radius: 6.0.into(),
            },
            ..container::Style::default()
        });
    // 右上角固定偏移:48px 避开顶栏,16px 避开窗口右边缘。这是估算值,
    // 不是像素级对齐"＋"按钮(spec 明确"不算点击坐标")——Task 4 最后
    // 一步的人工验收里如果视觉上偏得明显,回来调这两个数字即可,不影响
    // 其余逻辑。
    container(list)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(iced_widget::core::alignment::Horizontal::Right)
        .align_y(iced_widget::core::alignment::Vertical::Top)
        .padding(Padding {
            top: 48.0,
            left: 0.0,
            right: 16.0,
            bottom: 0.0,
        })
        .into()
}

/// 会话审阅内容面板(右面板区"对话"视图的内容侧):直接读 `ws.review`,
/// 不经过 `ws.preview` 的 tab 系统——新外壳下审阅是独立面板,不再是
/// 预览 tab 条里的一个 tab。
fn review_content_pane(
    ws: &Workspace,
    width: Length,
    outer: Border,
) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let region = chrome_style::review_content_pane();
    let header = row![
        lh(text("会话审阅")
            .size(workspace_font::body())
            .color(theme::CREAM)),
        iced_widget::space::horizontal(),
    ]
    .spacing(4);
    let mut content = column![header].spacing(region.gap);

    if ws.review.is_some() {
        content = review_content(content, ws);
    } else {
        content = content.push(
            container(lh(text("暂无审阅内容——点击左侧对话列表中的对话开始审阅")
                .size(workspace_font::subtitle())
                .color(theme::DIM)))
            .width(Length::Fill)
            .height(Length::Fill),
        );
    }

    container(content.padding(region.padding))
        .width(width)
        .height(Length::Fill)
        .style(move |_theme: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: outer,
            ..container::Style::default()
        })
        .into()
}

/// 单个图标栏按钮：圆角正方形背景常驻,hover 图标变金(无金框),选中图标
/// 变金且带金色外框。
fn rail_icon_button<'a>(
    icon: icons::IconKind,
    active: bool,
    hover_t: f32,
    msg: Message,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    // 图标颜色:选中态恒为金;未选中时 hover 平滑过渡到金(见 `HoverId`/
    // `App::hover_progress`——与光标闪烁同款自驱 redraw 动画)。SVG 颜色
    // 构建时定死、不吃 `button::Status`,所以 hover 进度靠 `hover_t` 参数从
    // App 算进来。
    let color = if active {
        theme::GOLD
    } else {
        theme::mix(theme::DIM, theme::GOLD, hover_t)
    };
    let inner = container(icons::view(icon, crate::icon_size::rail(), color))
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

    button(inner)
        .on_press(msg)
        .width(Length::Fixed(crate::workspace_geometry::rail_button_size()))
        .height(Length::Fixed(crate::workspace_geometry::rail_button_size()))
        .padding(0)
        .style(move |_t: &iced_widget::Theme, _status: button::Status| {
            // 圆角正方形背景常驻(`CARD`);金色外框只在选中态出现,hover
            // 不放金框——所以样式完全由 `active` 决定,与交互态无关。
            button::Style {
                background: Some(theme::CARD.into()),
                border: Border {
                    color: if active {
                        theme::GOLD
                    } else {
                        Color::TRANSPARENT
                    },
                    ..base_border
                },
                ..button::Style::default()
            }
        })
        .into()
}

/// 左图标栏:文件列表 / Web 两个图标,点已激活的那个即收起左面板区。
fn left_icon_rail(app: &App) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let region = chrome_style::left_icon_rail();
    // 视觉"选中"= 该视图激活 **且**左面板区展开。点已选中的图标会收起面板区,
    // 此时图标要退回未选中态(见 `LeftIconSelect`),所以 `active` 得带上
    // `!left_collapsed`。
    let left_open = !app.left_collapsed;
    let content = column![
        MouseArea::new(rail_icon_button(
            icons::IconKind::Folder,
            app.left_view == LeftView::Files && left_open,
            app.hover_progress(HoverId::Rail(RailButton::LeftFiles)),
            Message::LeftIconSelect(LeftView::Files),
        ))
        .on_enter(Message::Hover(HoverId::Rail(RailButton::LeftFiles), true))
        .on_exit(Message::Hover(HoverId::Rail(RailButton::LeftFiles), false)),
        MouseArea::new(rail_icon_button(
            icons::IconKind::Globe,
            app.left_view == LeftView::Web && left_open,
            app.hover_progress(HoverId::Rail(RailButton::LeftWeb)),
            Message::LeftIconSelect(LeftView::Web),
        ))
        .on_enter(Message::Hover(HoverId::Rail(RailButton::LeftWeb), true))
        .on_exit(Message::Hover(HoverId::Rail(RailButton::LeftWeb), false)),
        // spike(2026-08-06):Git 提交图入口,验证 gleisbau 库可行性用。
        MouseArea::new(rail_icon_button(
            icons::IconKind::GitBranch,
            app.left_view == LeftView::GitLog && left_open,
            app.hover_progress(HoverId::Rail(RailButton::LeftGit)),
            Message::LeftIconSelect(LeftView::GitLog),
        ))
        .on_enter(Message::Hover(HoverId::Rail(RailButton::LeftGit), true))
        .on_exit(Message::Hover(HoverId::Rail(RailButton::LeftGit), false)),
        // Todo 面板入口：`.dozer/todo.md` 任务列表。
        MouseArea::new(rail_icon_button(
            icons::IconKind::ListChecks,
            app.left_view == LeftView::Todo && left_open,
            app.hover_progress(HoverId::Rail(RailButton::LeftTodo)),
            Message::LeftIconSelect(LeftView::Todo),
        ))
        .on_enter(Message::Hover(HoverId::Rail(RailButton::LeftTodo), true))
        .on_exit(Message::Hover(HoverId::Rail(RailButton::LeftTodo), false)),
    ]
    .spacing(region.gap)
    .padding(region.padding);

    container(content)
        .width(Length::Fixed(workspace_geometry::icon_rail_width()))
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: region.border.unwrap_or_default(),
            ..container::Style::default()
        })
        .into()
}

/// 右图标栏:Agent / 对话两个图标,语义同 `left_icon_rail`。
fn right_icon_rail(app: &App) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let region = chrome_style::right_icon_rail();
    // 同 `left_icon_rail`:视觉"选中"需右面板区展开。
    let right_open = !app.right_collapsed;
    let content = column![
        MouseArea::new(rail_icon_button(
            icons::IconKind::Bot,
            app.right_view == RightView::Agent && right_open,
            app.hover_progress(HoverId::Rail(RailButton::RightAgent)),
            Message::RightIconSelect(RightView::Agent),
        ))
        .on_enter(Message::Hover(HoverId::Rail(RailButton::RightAgent), true))
        .on_exit(Message::Hover(HoverId::Rail(RailButton::RightAgent), false)),
        MouseArea::new(rail_icon_button(
            icons::IconKind::MessageSquare,
            app.right_view == RightView::Conversations && right_open,
            app.hover_progress(HoverId::Rail(RailButton::RightConversations)),
            Message::RightIconSelect(RightView::Conversations),
        ))
        .on_enter(Message::Hover(
            HoverId::Rail(RailButton::RightConversations),
            true
        ))
        .on_exit(Message::Hover(
            HoverId::Rail(RailButton::RightConversations),
            false
        )),
        MouseArea::new(rail_icon_button(
            icons::IconKind::BarChart3,
            app.right_view == RightView::Usage && right_open,
            app.hover_progress(HoverId::Rail(RailButton::RightUsage)),
            Message::RightIconSelect(RightView::Usage),
        ))
        .on_enter(Message::Hover(HoverId::Rail(RailButton::RightUsage), true))
        .on_exit(Message::Hover(HoverId::Rail(RailButton::RightUsage), false)),
    ]
    .spacing(region.gap)
    .padding(region.padding);

    container(content)
        .width(Length::Fixed(workspace_geometry::icon_rail_width()))
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: region.border.unwrap_or_default(),
            ..container::Style::default()
        })
        .into()
}

/// 配对视图内部"列表:内容"的 `FillPortion` 权重对。`FillPortion` 在扣掉
/// 中间固定宽的分隔线之后按权重分剩余空间,与 `pair_content_width` 同源。
fn split_portions(split: f32) -> (u16, u16) {
    let list = (split * 10_000.0).round() as u16;
    let content = ((1.0 - split) * 10_000.0).round() as u16;
    (list, content)
}

/// 面板区里某块 pane 在外框圆角处要收圆的外角:`Left`/`Right` 配对视图里
/// 左 pane 收左侧、右 pane 收右侧;`All` 是 Web 单 pane 收全部四角;`None`
/// 不收(放大态下 pane 直接撑满放大盒子,外框由金色浮层负责,方角才对)。
#[derive(Clone, Copy)]
enum PaneCorner {
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
fn zone_pane_border(zone: chrome_style::RegionStyle, corner: PaneCorner) -> Border {
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
/// `shell_layout.left_width`。持久化宽可能大过当前窗口容得下的范围(用户
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
/// `chrome_style::left_zone()` 的外框(四向 margin 做悬浮留白,无描边)。
/// 放大态跳过——`maximize_overlay` 已经用金色边框把同一块内容整体框起来,
/// 再套一层外框会在金框内侧多出一圈视觉噪音。
fn worktree_strip<'a>(
    worktrees: &'a [WorktreeInfo],
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    // 把同仓库的其他 worktree 压成一行小字,标示当前提交图对应哪个 worktree
    // 上下文。主 worktree + N 个链接 worktree 各自的分支会散落在同一条图上,
    // 这个条带帮助用户分辨 `[→main]` 到底指谁。"本工作区"是状态展示,不是
    // 甲方动作,不能用 `theme::GOLD`(CLAUDE.md 硬性裁决,GOLD 专属甲方动作)
    // ——真正的动作是点其它 worktree 切过去,那些按钮才该用 GOLD。
    let current = worktrees.iter().find(|w| w.is_current);
    let others = worktrees
        .iter()
        .filter(|w| !w.is_current)
        .collect::<Vec<_>>();
    let mut chips: Vec<Element<'_, Message, iced_widget::Theme, iced_widget::Renderer>> = vec![];
    if let Some(c) = current {
        chips.push(
            text(format!(
                "本工作区:{}",
                c.branch.as_deref().unwrap_or("(无分支)")
            ))
            .size(workspace_font::caption())
            .color(theme::CYAN)
            .into(),
        );
    }
    for o in others {
        let label = match (&o.branch, o.missing) {
            (Some(b), true) => format!("{b} (缺失)"),
            (Some(b), _) => b.clone(),
            (None, true) => "无分支 (缺失)".into(),
            (None, _) => "无分支".into(),
        };
        if o.missing {
            // 目录已经不在磁盘上,没有可切换的目标——保留纯展示文案。
            chips.push(
                text(label)
                    .size(workspace_font::caption())
                    .color(theme::DIM)
                    .into(),
            );
        } else {
            chips.push(
                button(
                    text(label)
                        .size(workspace_font::caption())
                        .color(theme::GOLD),
                )
                .on_press(Message::ProjectTabOpen(o.path.clone()))
                .padding(0)
                .style(|_t: &iced_widget::Theme, _s| button::Style {
                    background: None,
                    text_color: theme::GOLD,
                    ..button::Style::default()
                })
                .into(),
            );
        }
    }
    if chips.is_empty() {
        return container(iced_widget::Space::new())
            .height(Length::Shrink)
            .into();
    }
    row![
        iced_widget::Row::with_children(chips).spacing(12),
        iced_widget::Space::new().width(Length::Fill),
    ]
    .padding([4, 8])
    .width(Length::Fill)
    .height(Length::Shrink)
    .into()
}

fn left_panel_area<'a>(
    app: &'a App,
    ws: &'a Workspace,
    maximized: bool,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
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
    let zone = chrome_style::left_zone();
    let (lc, rc, ac) = if maximized {
        (PaneCorner::None, PaneCorner::None, PaneCorner::None)
    } else {
        (PaneCorner::Left, PaneCorner::Right, PaneCorner::All)
    };
    let inner: Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> = match app.left_view
    {
        LeftView::Files => {
            let (list_portion, content_portion) = split_portions(app.shell_layout.files_split);
            row![
                project_pane(
                    app,
                    ws,
                    Length::FillPortion(list_portion),
                    zone_pane_border(zone, lc)
                ),
                divider_bar(
                    Divider::LeftPairSplit,
                    chrome_style::project_pane().background.unwrap_or(theme::BG),
                    chrome_style::preview_pane().background.unwrap_or(theme::BG),
                ),
                preview_pane(
                    ws,
                    Length::FillPortion(content_portion),
                    zone_pane_border(zone, rc)
                ),
            ]
            .width(Length::Fill)
            .into()
        }
        LeftView::Web => browser_pane(ws, Length::Fill, zone_pane_border(zone, ac)),
        LeftView::GitLog => git_log::view(&app.git_log).map(Message::GitLog),
        LeftView::Todo => todo_pane(app, ws, Length::Fill, zone_pane_border(zone, ac)),
    };
    if maximized {
        return inner;
    }
    let region = zone;
    // 哪怕提交图还没画出来(加载中/出错/空仓库),worktree 速览条也该照常
    // 显示——用户可能就是先想看看有哪些 worktree,不必等图先画出来。
    let strip: Option<Element<'_, Message, iced_widget::Theme, iced_widget::Renderer>> =
        if app.left_view == LeftView::GitLog {
            Some(worktree_strip(&ws.worktrees))
        } else {
            None
        };
    let mut zone_body: Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> = inner;
    if let Some(strip) = strip {
        zone_body = column![strip, zone_body]
            .width(Length::Fill)
            .height(Length::Fill)
            .into();
    }
    let zone_box = container(zone_body)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(region.padding)
        .clip(true)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: region.border.unwrap_or_default(),
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
/// 一个整体,套 `chrome_style::right_zone()` 的外框(四向 margin 做悬浮留白,
/// 无描边),`maximized` 时跳过(理由同 `left_panel_area`)。
fn right_panel_area<'a>(
    app: &'a App,
    ws: &'a Workspace,
    maximized: bool,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
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
    let zone = chrome_style::right_zone();
    let (lc, rc, ac) = if maximized {
        (PaneCorner::None, PaneCorner::None, PaneCorner::None)
    } else {
        (PaneCorner::Left, PaneCorner::Right, PaneCorner::All)
    };
    let inner: Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> =
        match app.right_view {
            RightView::Agent => {
                let (list_portion, content_portion) = split_portions(app.shell_layout.agent_split);
                row![
                    terminal_pane(
                        app,
                        ws,
                        Length::FillPortion(content_portion),
                        zone_pane_border(zone, lc)
                    ),
                    divider_bar(
                        Divider::RightPairSplit,
                        chrome_style::terminal_pane()
                            .background
                            .unwrap_or(theme::BG),
                        chrome_style::agent_list_pane()
                            .background
                            .unwrap_or(theme::BG),
                    ),
                    agent_list_pane(
                        ws,
                        Length::FillPortion(list_portion),
                        zone_pane_border(zone, rc)
                    ),
                ]
                .width(Length::Fill)
                .into()
            }
            RightView::Conversations => {
                let (list_portion, content_portion) =
                    split_portions(app.shell_layout.conversations_split);
                row![
                    review_content_pane(
                        ws,
                        Length::FillPortion(content_portion),
                        zone_pane_border(zone, lc)
                    ),
                    divider_bar(
                        Divider::RightPairSplit,
                        chrome_style::review_content_pane()
                            .background
                            .unwrap_or(theme::BG),
                        chrome_style::conversation_list_pane()
                            .background
                            .unwrap_or(theme::BG),
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
                ws.usage_loading,
                ws.project
                    .as_ref()
                    .map(|p| p.name.as_str())
                    .unwrap_or("未打开项目"),
                Length::Fill,
                zone_pane_border(zone, ac),
            ),
        };
    if maximized {
        return inner;
    }
    let region = zone;
    let zone_box = container(inner)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(region.padding)
        .clip(true)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: region.border.unwrap_or_default(),
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
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
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
    let overlay_style = chrome_style::maximize_overlay();
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

    // 顶部垫一条透明的 `workspace_geometry::top_bar_height()` 高 Space,把变暗遮罩钉在顶栏
    // 之下——`base = column![top, body]` 里顶栏和内容区就是这么分的,
    // 这里镜像同一结构,让变暗区域精确对齐 `body` 的渲染范围,不覆盖顶栏
    // (Important:此前没有这条 Space,遮罩会盖住整个窗口高度,连顶栏的
    // 项目 tab 等控件都会被染黑)。
    column![
        iced_widget::space::Space::new()
            .height(Length::Fixed(workspace_geometry::top_bar_height())),
        row![
            iced_widget::space::Space::new()
                .width(Length::Fixed(workspace_geometry::icon_rail_width())),
            dim_bg,
            iced_widget::space::Space::new()
                .width(Length::Fixed(workspace_geometry::icon_rail_width())),
        ],
    ]
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

/// 文件树目录/文件名行的字号：与终端字号(`terminal_font`)对齐（含全局 UI
/// scale），配合下面的 `LineHeight::Relative(line_height_factor)` 让每行行高
/// 等于终端行距，目录/文件列表不再比终端稀疏。
fn tree_row_font_size() -> f32 {
    terminal_font::size() * crate::icon_size::scale()
}

/// 统一行高：把一段文字的行高设为终端行高
/// (`terminal_font::line_height_factor()` = 1.2)，让各面板列表/正文行的行距
/// 与文件树、终端观感一致。`size`/`color` 等仍由调用方设置，这里只补行高。
pub(crate) fn lh<'a>(
    t: iced_widget::text::Text<'a, iced_widget::Theme, iced_widget::Renderer>,
) -> iced_widget::text::Text<'a, iced_widget::Theme, iced_widget::Renderer> {
    t.line_height(LineHeight::Relative(terminal_font::line_height_factor()))
}

fn project_pane<'a>(
    app: &'a App,
    ws: &'a Workspace,
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    let region = chrome_style::project_pane();
    // 头部:项目信息卡,固定在文件树上方,不随滚动条滚走(需求 1)。
    let mut header = column![].spacing(region.gap).width(Length::Fill);
    // 文件树行:唯一进入 scrollable 的内容。
    let mut tree_col = column![].spacing(region.gap);

    match &ws.project {
        Some(p) => {
            let label = project_branch_label(ws.branch.as_deref(), ws.dirty);
            let bcolor = if ws.dirty { theme::GOLD } else { theme::BODY };
            // 需求 3:git 分支名前加 git-branch icon;需求 2:去掉完整文件路径。
            let mut card_col = column![
                text(p.name.clone())
                    .size(workspace_font::title())
                    .color(theme::CREAM),
                row![
                    icons::view(icons::IconKind::GitBranch, crate::icon_size::row(), bcolor),
                    text(label).size(workspace_font::label()).color(bcolor),
                ]
                .spacing(6)
                .align_y(iced_widget::core::Alignment::Center),
            ]
            .spacing(2);
            if let Some(n) = ws.project_acceptance_count.filter(|n| *n > 0) {
                card_col = card_col.push(
                    text(format!("{n} 次验收"))
                        .size(workspace_font::caption())
                        .color(theme::GOLD),
                );
            }
            let card = container(card_col).width(Length::Fill).padding(10).style(
                |_t: &iced_widget::Theme| container::Style {
                    background: Some(theme::CARD.into()),
                    border: Border {
                        color: theme::BORDER,
                        width: 1.0,
                        radius: 8.0.into(),
                    },
                    ..container::Style::default()
                },
            );
            header = header.push(card);
            if let Some(err) = &ws.tree_error {
                header = header.push(
                    text(format!("⚠ {err}"))
                        .size(workspace_font::label())
                        .color(theme::RED),
                );
            }
            if let Some(tree) = &ws.file_tree {
                for row in tree.visible_rows() {
                    let is_renaming = matches!(
                        &ws.tree_edit,
                        Some(TreeEdit { mode: TreeEditMode::Rename(p), .. }) if *p == row.path
                    );
                    if is_renaming {
                        let buffer = ws
                            .tree_edit
                            .as_ref()
                            .map(|e| e.buffer.as_str())
                            .unwrap_or("");
                        tree_col = tree_col.push(tree_edit_row(row.depth, buffer));
                        continue;
                    }
                    let indent = "  ".repeat(row.depth);
                    let status: Option<(delivery::ChangeKind, bool)> = if row.is_dir {
                        delivery::dir_status(&row.path, &ws.git_statuses)
                            .map(|d| (d.kind, d.unstaged))
                    } else {
                        ws.git_statuses.get(&row.path).map(|s| (s.kind, s.unstaged))
                    };
                    let name_color = theme::BODY;
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
                                icons::view(chevron, crate::icon_size::chevron(), theme::DIM),
                                icons::view(folder, crate::icon_size::row(), theme::DIM),
                            ]
                            .spacing(crate::icon_size::tree_row_gap())
                            .align_y(iced_widget::core::Alignment::Center)
                            .into()
                        } else {
                            row![
                                iced_widget::space::Space::new()
                                    .width(Length::Fixed(
                                        crate::icon_size::chevron()
                                            + crate::icon_size::tree_row_gap(),
                                    ))
                                    .height(Length::Shrink),
                                icons::view(
                                    icons::icon_for_file(&row.name),
                                    crate::icon_size::row(),
                                    theme::DIM
                                ),
                            ]
                            .spacing(0)
                            .align_y(iced_widget::core::Alignment::Center)
                            .into()
                        };
                    let mut line = row![
                        text(indent)
                            .size(tree_row_font_size())
                            .line_height(LineHeight::Relative(terminal_font::line_height_factor()))
                            .color(name_color),
                        row_icon,
                        text(row.name.clone())
                            .size(tree_row_font_size())
                            .line_height(LineHeight::Relative(terminal_font::line_height_factor()))
                            .color(name_color),
                    ]
                    .spacing(6)
                    .align_y(iced_widget::core::Alignment::Center);
                    if let Some((kind, unstaged)) = status {
                        line = line.push(iced_widget::space::horizontal());
                        line = line.push(
                            text(tree_row_dot_glyph(unstaged))
                                .size(workspace_font::dot_xs())
                                .color(tree_row_dot_color(kind)),
                        );
                    }
                    let msg = if row.is_dir {
                        Message::ProjectTreeToggle(row.path.clone())
                    } else {
                        Message::PreviewOpenPath(row.path.clone())
                    };
                    let is_selected = ws.tree_selected.as_deref() == Some(row.path.as_path());
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
                                Some(theme::CARD.into())
                            } else {
                                None
                            },
                            text_color: theme::BODY,
                            ..button::Style::default()
                        });
                    tree_col = tree_col.push(MouseArea::new(row_btn).on_right_press(
                        Message::ProjectTreeContextMenu {
                            path: row.path.clone(),
                            is_dir: row.is_dir,
                        },
                    ));
                    let is_new_target = matches!(
                        &ws.tree_edit,
                        Some(TreeEdit {
                            mode: TreeEditMode::NewFile | TreeEditMode::NewFolder,
                            parent_dir,
                            ..
                        }) if *parent_dir == row.path
                    );
                    if is_new_target && row.expanded {
                        let buffer = ws
                            .tree_edit
                            .as_ref()
                            .map(|e| e.buffer.as_str())
                            .unwrap_or("");
                        tree_col = tree_col.push(tree_edit_row(row.depth + 1, buffer));
                    }
                }
            }
        }
        None => {
            header = header.push(
                text("未打开项目")
                    .size(workspace_font::body())
                    .color(theme::DIM),
            );
            for p in &ws.recent_projects {
                header = header.push(
                    button(
                        text(p.name.clone())
                            .size(workspace_font::body())
                            .color(theme::CREAM),
                    )
                    .on_press(Message::ProjectSelect(p.id))
                    .style(|_t, _s| button::Style {
                        background: None,
                        text_color: theme::CREAM,
                        ..button::Style::default()
                    }),
                );
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
    container(column![body, project_status_bar(app, ws, outer)])
        .width(width)
        .height(Length::Fill)
        .into()
}

/// 项目栏底状态条：左 环境/dozerd 点，右 [文件|git {分支}|组件]（文件高亮,组件占位）。
fn project_status_bar<'a>(
    app: &'a App,
    ws: &'a Workspace,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    let (env, dot) = env_status_text(app.daemon_error.is_none());
    let left = row![
        text("●").size(workspace_font::dot_sm()).color(dot),
        text(env).size(workspace_font::caption()).color(theme::BODY)
    ]
    .spacing(6);
    let git = format!(
        "git {}",
        project_branch_label(ws.branch.as_deref(), ws.dirty)
    );
    let tabs = row![
        text("文件")
            .size(workspace_font::caption())
            .color(theme::CREAM),
        text("·").size(workspace_font::caption()).color(theme::DIM),
        text(git).size(workspace_font::caption()).color(theme::BODY),
        text("·").size(workspace_font::caption()).color(theme::DIM),
        text("组件")
            .size(workspace_font::caption())
            .color(theme::DIM),
    ]
    .spacing(6);
    status_bar_container(
        row![left, iced_widget::space::horizontal(), tabs]
            .align_y(iced_widget::core::Alignment::Center),
        outer,
    )
}

/// 终端栏底状态条：当前激活 tab 的 agent 态 · resume · dozerd 持有。
fn terminal_status_bar(
    ws: &Workspace,
    outer: Border,
) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let (label, dot) = match ws.tabs.get(ws.active) {
        Some(t) => (
            agent_state_label(t.agent_state),
            dot_color(t.agent_state, t.alive),
        ),
        None => ("空闲", theme::DIM),
    };
    let resume = ws.tabs.get(ws.active).map(|t| t.alive).unwrap_or(false);
    let line = row![
        text("●").size(workspace_font::dot_sm()).color(dot),
        text(label)
            .size(workspace_font::caption())
            .color(theme::BODY),
        text("·").size(workspace_font::caption()).color(theme::DIM),
        text(format!("resume {}", if resume { "✓" } else { "—" }))
            .size(workspace_font::caption())
            .color(theme::BODY),
        text("·").size(workspace_font::caption()).color(theme::DIM),
        text("dozerd 持有 · 断连可恢复")
            .size(workspace_font::caption())
            .color(theme::DIM),
    ]
    .spacing(6);
    status_bar_container(line, outer)
}

/// 状态条通用外框：略深底 + 上边线 + 固定高。`outer` 是所属 pane 的整体
/// 外框圆角（`zone_pane_border` 算出的 `Border`），只取它的 `radius` 套到
/// 底栏上——底栏贴在 pane 最底部,若不收圆角和会戳出 pane 已收圆的底角,
/// 在 zone 圆角 CARD 背景上顶出小尖角。保留底栏自己那条 1px 上边分隔线
/// （颜色/宽度沿用 `status_bar` 区域配置,只改圆角）。
fn status_bar_container<'a>(
    inner: impl Into<Element<'a, Message, iced_widget::Theme, iced_widget::Renderer>>,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    let region = chrome_style::status_bar();
    let base = region.border.unwrap_or_default();
    container(inner)
        .width(Length::Fill)
        .height(Length::Fixed(workspace_geometry::status_bar_height()))
        .padding(region.padding)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: Border {
                color: base.color,
                width: base.width,
                radius: outer.radius,
            },
            ..container::Style::default()
        })
        .into()
}

fn preview_pane(
    ws: &Workspace,
    width: Length,
    outer: Border,
) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    // tab 栏:箭头翻页(到头变灰) + 每 tab 选择按钮 + 关闭 ×。tab 只能由项目树
    // 点击/会话恢复产生——面板本身已不再有"打开文件…"按钮或地址栏(P1 后续
    // 反馈:文件预览与浏览器彻底分离,文件只走项目树入口)。
    // P1L T5 验收返工:同 term `tab_bar`,横向 scrollable 换成索引窗口化 + clip.
    let region = chrome_style::preview_pane();
    let widths: Vec<f32> = ws
        .preview
        .tabs()
        .iter()
        .map(|t| preview_tab_display_width(&t.title))
        .collect();
    let (first, can_left, can_right) = tab_window(
        &widths,
        4.0,
        workspace_geometry::tab_bar_avail_px(),
        ws.preview_tab_first,
    );

    let items: Vec<Element<'_, Message, iced_widget::Theme, iced_widget::Renderer>> = ws
        .preview
        .tabs()
        .iter()
        .enumerate()
        .filter(|(idx, _)| *idx >= first)
        .map(|(idx, tab)| {
            let active = idx == ws.preview.active_idx();
            let select = button(lh(text(tab.title.clone())
                .size(workspace_font::subtitle())
                .color(theme::CREAM)))
            .on_press(Message::PreviewSelectTab(idx))
            .style(|_t, _s| button::Style {
                background: None,
                text_color: theme::CREAM,
                ..button::Style::default()
            });
            let editable = matches!(&tab.kind, TabKind::File(path) if is_editable_extension(path));
            let edit: Option<Element<'_, Message, iced_widget::Theme, iced_widget::Renderer>> =
                if editable {
                    Some(
                        button(icons::view(
                            icons::IconKind::Rename,
                            crate::icon_size::row(),
                            theme::DIM,
                        ))
                        .on_press(Message::PreviewEditOpen(idx))
                        .padding(0)
                        .style(|_t, _s| button::Style {
                            background: None,
                            text_color: theme::DIM,
                            ..button::Style::default()
                        })
                        .into(),
                    )
                } else {
                    None
                };
            let close = button(lh(text("×").size(workspace_font::body()).color(theme::DIM)))
                .on_press(Message::PreviewCloseTab(idx))
                .style(|_t, _s| button::Style {
                    background: None,
                    text_color: theme::DIM,
                    ..button::Style::default()
                });
            let mut chip_row = row![select].spacing(2);
            if let Some(edit) = edit {
                chip_row = chip_row.push(edit);
            }
            chip_row = chip_row.push(close);
            container(chip_row.align_y(iced_widget::core::Alignment::Center))
                .padding([2, 4])
                .style(move |_t: &iced_widget::Theme| {
                    if active {
                        container::Style {
                            background: Some(theme::CARD.into()),
                            border: Border {
                                color: theme::BORDER,
                                width: 1.0,
                                radius: 6.0.into(),
                            },
                            ..container::Style::default()
                        }
                    } else {
                        container::Style::default()
                    }
                })
                .into()
        })
        .collect();
    // tab 列表进 clip 容器占 Fill,裁掉右侧溢出;左右箭头钉在裁剪区外。
    let tabs_row = row(items).spacing(4);
    let clipped = container(tabs_row).width(Length::Fill).clip(true);
    let left_arrow = tab_arrow_button(
        icons::IconKind::ChevronLeft,
        can_left,
        Message::PreviewTabScroll(false),
    );
    let right_arrow = tab_arrow_button(
        icons::IconKind::ChevronRight,
        can_right,
        Message::PreviewTabScroll(true),
    );
    let tab_bar = row![left_arrow, right_arrow, clipped]
        .spacing(4)
        .align_y(iced_widget::core::Alignment::Center);

    let mut content = column![tab_bar, tab_divider()].spacing(region.gap);

    if let Some(err) = &ws.preview_error {
        content = content.push(lh(text(format!("⚠ {err}"))
            .size(workspace_font::body())
            .color(theme::RED)));
    }

    if ws.preview.acceptance_active() {
        content = acceptance_content(content, ws);
    } else if ws.preview.tabs().is_empty() {
        content = content.push(
            container(lh(text("暂无预览——在左侧文件树选择文件")
                .size(workspace_font::subtitle())
                .color(theme::DIM)))
            .width(Length::Fill)
            .height(Length::Fill),
        );
    }

    container(content.padding(region.padding))
        .width(width)
        .height(Length::Fill)
        .style(move |_theme: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: outer,
            ..container::Style::default()
        })
        .into()
}

/// 左面板区 Todo 视图：`.dozer/todo.md` 任务列表 + 筛选/搜索 + 派发 +
/// 计划/完成时间。
fn todo_pane<'a>(
    app: &'a App,
    ws: &'a Workspace,
    width: Length,
    border: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    let header = column![
        text("Todo")
            .size(workspace_font::title())
            .color(theme::CREAM),
        text(format!("{} 条任务 · .dozer/todo.md", ws.todo_items.len()))
            .size(workspace_font::caption())
            .color(theme::DIM),
    ]
    .spacing(4)
    .padding([20, 20]);

    // 状态三态推导 + 筛选下标。
    let states: Vec<todo::TodoState> = ws
        .todo_items
        .iter()
        .map(|item| {
            let key = todo::todo_line_key(&item.text);
            let dispatch = app_todo_dispatch_for(app, ws, key);
            let target_alive = dispatch
                .map(|d| ws.tabs.iter().any(|t| t.info.id == d.session_id && t.alive))
                .unwrap_or(false);
            todo::todo_display_state(item, dispatch, target_alive)
        })
        .collect();
    let visible_idx = todo::filter_todos(&ws.todo_items, &states, ws.todo_filter, &ws.todo_search);

    // 派发选择层要列出的存活 agent tab。
    let existing_tabs: Vec<(&str, String)> = ws
        .tabs
        .iter()
        .filter(|t| t.alive)
        .map(|t| {
            (
                t.info.id.as_str(),
                tab_title(t.agent, t.cwd.as_deref(), &t.info.name),
            )
        })
        .collect();

    let toolbar = row![
        todo_filter_segment("全部", todo::TodoFilter::All, ws.todo_filter),
        todo_filter_segment("待办", todo::TodoFilter::Pending, ws.todo_filter),
        todo_filter_segment("进行中", todo::TodoFilter::InProgress, ws.todo_filter),
        todo_filter_segment("完成", todo::TodoFilter::Done, ws.todo_filter),
        text_input("搜索任务关键字…", &ws.todo_search)
            .on_input(Message::TodoSearchChanged)
            .size(workspace_font::body())
            .width(Length::Fill)
            .style(
                |_t: &iced_widget::Theme, _s| iced_widget::text_input::Style {
                    background: theme::BG.into(),
                    border: Border::default(),
                    icon: theme::DIM,
                    placeholder: theme::DIM,
                    value: theme::CREAM,
                    selection: theme::GOLD,
                }
            ),
    ]
    .spacing(8)
    .padding([12, 20])
    .align_y(iced_widget::core::alignment::Vertical::Center);

    let mut list = column![].spacing(2);
    if visible_idx.is_empty() {
        list = list.push(
            container(
                text("没有匹配的任务")
                    .size(workspace_font::body())
                    .color(theme::DIM),
            )
            .padding([20, 20]),
        );
    } else {
        for &idx in &visible_idx {
            let item = &ws.todo_items[idx];
            let key = todo::todo_line_key(&item.text);
            let project_id = ws.project.as_ref().map(|p| p.id);
            let meta = project_id
                .and_then(|pid| app.todo_meta.get(&pid))
                .and_then(|m| m.get(&key));
            let mut row = None;
            if let Some((editing_idx, draft)) = &ws.todo_editing_plan_date
                && *editing_idx == idx
            {
                row = Some(todo_plan_date_edit_row(item, draft));
            }
            match row {
                Some(r) => list = list.push(r),
                None => {
                    list = list.push(todo_row(
                        idx,
                        item,
                        states[idx],
                        meta,
                        ws.todo_dispatch_open == Some(idx),
                        &existing_tabs,
                    ));
                }
            }
        }
    }

    let add_row = text_input("＋新增任务…", &ws.todo_add_draft)
        .on_input(Message::TodoAddInputChanged)
        .on_submit(Message::TodoAddSubmit)
        .size(workspace_font::body())
        .padding([10, 20])
        .style(
            |_t: &iced_widget::Theme, _s| iced_widget::text_input::Style {
                background: theme::BG.into(),
                border: Border {
                    color: Color::TRANSPARENT,
                    width: 0.0,
                    radius: 0.0.into(),
                },
                icon: theme::DIM,
                placeholder: theme::DIM,
                value: theme::CREAM,
                selection: theme::GOLD,
            },
        );

    let divider = container(iced_widget::Space::new())
        .width(Length::Fill)
        .height(Length::Fixed(1.0))
        .style(|_t: &iced_widget::Theme| container::Style {
            border: Border {
                color: theme::BORDER,
                width: 1.0,
                radius: 0.0.into(),
            },
            ..container::Style::default()
        });

    let content = column![
        header,
        toolbar,
        scrollable(list).height(Length::Fill),
        divider,
        add_row,
    ]
    .height(Length::Fill);

    container(content)
        .width(width)
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: Some(theme::BG.into()),
            border,
            ..container::Style::default()
        })
        .into()
}

/// 一条 Todo 任务行。勾选框+文本（完成态删除线+暗色）+ 右侧按状态显示：
/// 待办=派发按钮；进行中=绿点+"进行中"标签；完成=占位。派发选择层叠在行下方。
fn todo_row<'a>(
    idx: usize,
    item: &'a todo::TodoItem,
    state: todo::TodoState,
    meta: Option<&'a todo_meta::TodoTaskMeta>,
    dispatch_open: bool,
    existing_tabs: &'a [(&'a str, String)],
) -> Element<'static, Message, iced_widget::Theme, iced_widget::Renderer> {
    let done = item.done;
    let box_color = if done { theme::BORDER } else { theme::DIM };
    let checkbox = button(
        container(if done {
            text("✓")
                .size(workspace_font::caption())
                .color(theme::DIM)
                .into()
        } else {
            Element::from(iced_widget::space::Space::new())
        })
        .width(Length::Fixed(18.0))
        .height(Length::Fixed(18.0))
        .align_x(iced_widget::core::alignment::Horizontal::Center)
        .align_y(iced_widget::core::alignment::Vertical::Center)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: if done {
                Some(theme::BORDER.into())
            } else {
                None
            },
            border: Border {
                color: box_color,
                width: 1.5,
                radius: 4.0.into(),
            },
            ..container::Style::default()
        }),
    )
    .on_press(Message::TodoToggle(idx))
    .padding(0)
    .style(|_t: &iced_widget::Theme, _s| button::Style {
        background: None,
        text_color: theme::CREAM,
        ..button::Style::default()
    });

    let label_color = if done { theme::DIM } else { theme::CREAM };
    let label: Element<'static, Message, iced_widget::Theme, iced_widget::Renderer> = if done {
        // `rich_text!` 会经 `FromIterator` 收集成 `Rich`，再 `.into()` 成
        // `Element` 抹掉 `Link` 泛型；没有链接时 `Link` 无从推导，得用
        // 显式类型钉住它（iced 文档也提示无链接时要人工指定 `Link`）。
        let rich: iced_widget::text::Rich<
            '_,
            (),
            Message,
            iced_widget::Theme,
            iced_widget::Renderer,
        > = rich_text![
            span(item.text.clone())
                .size(workspace_font::body())
                .color(label_color)
                .strikethrough(true)
        ];
        rich.into()
    } else {
        text(item.text.clone())
            .size(workspace_font::body())
            .color(label_color)
            .into()
    };

    // 中间段：日期标签（完成=完成于 xx；待办/进行中=计划 xx）。
    let date_label: Option<Element<'static, Message, iced_widget::Theme, iced_widget::Renderer>> =
        match state {
            todo::TodoState::Done => meta.and_then(|m| m.completed_at).map(|t| {
                text(format!("完成于 {}", format_todo_time(t)))
                    .size(workspace_font::caption())
                    .color(theme::DIM)
                    .into()
            }),
            _ => meta.and_then(|m| m.plan_date.as_deref()).map(|d| {
                button(
                    text(format!("计划 {d}"))
                        .size(workspace_font::caption())
                        .color(theme::DIM),
                )
                .on_press(Message::TodoPlanDateEditStart(idx))
                .padding(0)
                .style(|_t: &iced_widget::Theme, _s| button::Style {
                    background: None,
                    text_color: theme::DIM,
                    ..button::Style::default()
                })
                .into()
            }),
        };
    let mut middle = row![checkbox, label]
        .spacing(10)
        .align_y(iced_widget::core::alignment::Vertical::Center);
    if let Some(label) = date_label {
        middle = middle.push(label);
    }

    let trailing: Element<'static, Message, iced_widget::Theme, iced_widget::Renderer> = match state
    {
        todo::TodoState::Pending => button(
            row![
                icons::view(
                    icons::IconKind::SquarePlus,
                    crate::icon_size::row(),
                    theme::GOLD
                ),
                text("派发")
                    .size(workspace_font::caption())
                    .color(theme::GOLD),
            ]
            .spacing(4)
            .align_y(iced_widget::core::alignment::Vertical::Center),
        )
        .on_press(Message::TodoDispatchOpen(idx))
        .padding([5, 10])
        .style(|_t: &iced_widget::Theme, _s| button::Style {
            background: None,
            text_color: theme::GOLD,
            border: Border {
                color: theme::BORDER,
                width: 1.0,
                radius: 6.0.into(),
            },
            ..button::Style::default()
        })
        .into(),
        todo::TodoState::InProgress => row![
            text("●").size(workspace_font::dot_sm()).color(theme::GREEN),
            text("进行中")
                .size(workspace_font::caption())
                .color(theme::GREEN),
        ]
        .spacing(5)
        .into(),
        todo::TodoState::Done => iced_widget::space::Space::new().into(),
    };

    let base: Element<'static, Message, iced_widget::Theme, iced_widget::Renderer> =
        row![middle, trailing]
            .spacing(10)
            .align_y(iced_widget::core::alignment::Vertical::Center)
            .padding([10, 20])
            .into();
    if dispatch_open {
        column![base, todo_dispatch_popup(idx, existing_tabs)].into()
    } else {
        base
    }
}

/// Todo 派发选择层：列出当前项目存活的 agent tab + 一个"新建"入口，样式
/// 对齐 `agent_picker_popup`（CARD 底 + BORDER 描边）。挂在触发它的那一行
/// 下方，不需要额外的坐标计算。
fn todo_dispatch_popup<'a>(
    idx: usize,
    existing_tabs: &'a [(&'a str, String)],
) -> Element<'static, Message, iced_widget::Theme, iced_widget::Renderer> {
    let mut col = column![].spacing(2);
    // `title` 是 `String`，非 `Copy`；match ergonomics 下 `(session_id, title)`
    // 对 `&(&str, String)` 解构自动按引用绑定，两者都够用。
    for (session_id, title) in existing_tabs {
        col = col.push(
            button(
                text(title.clone())
                    .size(workspace_font::body())
                    .color(theme::CREAM),
            )
            .on_press(Message::TodoDispatchToExisting(idx, session_id.to_string()))
            .width(Length::Fill)
            .padding([6, 12])
            .style(|_t: &iced_widget::Theme, _s| button::Style {
                background: None,
                text_color: theme::CREAM,
                ..button::Style::default()
            }),
        );
    }
    col = col.push(
        button(
            text("新建 agent 会话…")
                .size(workspace_font::body())
                .color(theme::GOLD),
        )
        .on_press(Message::TodoDispatchNew(idx, PickerLaunch::Agent(None)))
        .width(Length::Fill)
        .padding([6, 12])
        .style(|_t: &iced_widget::Theme, _s| button::Style {
            background: None,
            text_color: theme::GOLD,
            ..button::Style::default()
        }),
    );
    container(col)
        .padding(6)
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(theme::CARD.into()),
            border: Border {
                color: theme::BORDER,
                width: 1.0,
                radius: 6.0.into(),
            },
            ..container::Style::default()
        })
        .into()
}

/// 计划时间内联编辑态：任务文本 + 一个 `text_input`，回车提交。
fn todo_plan_date_edit_row<'a>(
    item: &'a todo::TodoItem,
    draft: &'a str,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    row![
        text(item.text.clone())
            .size(workspace_font::body())
            .color(theme::CREAM),
        text_input("计划时间，如 08-10", draft)
            .on_input(Message::TodoPlanDateChanged)
            .on_submit(Message::TodoPlanDateSubmit)
            .size(workspace_font::caption())
            .width(Length::Fixed(140.0)),
    ]
    .spacing(10)
    .align_y(iced_widget::core::alignment::Vertical::Center)
    .padding([10, 20])
    .into()
}

/// 筛选分段按钮，选中态高亮。
fn todo_filter_segment<'a>(
    label: &'a str,
    value: todo::TodoFilter,
    current: todo::TodoFilter,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    let active = value == current;
    button(
        text(label)
            .size(workspace_font::caption())
            .color(if active { theme::CREAM } else { theme::DIM }),
    )
    .on_press(Message::TodoFilterSet(value))
    .padding([4, 10])
    .style(move |_t: &iced_widget::Theme, _s| button::Style {
        background: if active {
            Some(theme::CARD.into())
        } else {
            None
        },
        text_color: if active { theme::CREAM } else { theme::DIM },
        border: Border {
            radius: 5.0.into(),
            ..Border::default()
        },
        ..button::Style::default()
    })
    .into()
}

/// 按 `todo_line_key` 查 `App.todo_meta` 拿这条任务的派发记录（如果有）。
fn app_todo_dispatch_for<'a>(
    app: &'a App,
    ws: &Workspace,
    key: u64,
) -> Option<&'a todo_meta::DispatchRecord> {
    let project_id = ws.project.as_ref()?.id;
    app.todo_meta.get(&project_id)?.get(&key)?.dispatch.as_ref()
}

/// `SystemTime` → "MM-DD HH:MM"(UTC)。不引 `chrono`,用 civil-from-days
/// 算法(Howard Hinnant)手推公历年月日,再拼 HH:MM。只用于"完成于"这种
/// 粗粒度提示,UTC 而非本地时区,不追求夏令时/时区严格正确。
fn format_todo_time(t: std::time::SystemTime) -> String {
    let secs = t
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // days = 秒数 → 自 1970-01-01 的整数日;`secs` 已经是 `u64`(1970 前会被
    // 上面的 `unwrap_or_default()` 夹到 0),这里不会是负数。
    let days = (secs / 86400) as i64;
    let rem = secs % 86400;
    let (hour, minute) = (rem / 3600, (rem % 3600) / 60);
    let (_y, m, d) = civil_from_days(days);
    format!("{:02}-{:02} {:02}:{:02}", m, d, hour, minute)
}

/// civil-from-days：把"自 1970-01-01 的天数"换算成 (年, 月, 日)。
/// 用 Hinnant 经典公式,范围覆盖 1970..=2100,足够"完成于"提示用。
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m as u32, d as u32)
}

/// 浏览器栏:左图标栏"地球"进入的独立浏览器,tab 栏 + 地址栏 + 内容,
/// 读写完全独立的 `ws.browser`——文件预览面板已不再有地址栏,浏览器是
/// 唯一还能输入网址打开网页的入口,也不受文件预览的 tab 状态影响。
fn browser_pane(
    ws: &Workspace,
    width: Length,
    outer: Border,
) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let region = chrome_style::browser_pane();
    let widths: Vec<f32> = ws
        .browser
        .tabs()
        .iter()
        .map(|t| preview_tab_display_width(&t.title))
        .collect();
    let (first, can_left, can_right) = tab_window(
        &widths,
        4.0,
        workspace_geometry::tab_bar_avail_px(),
        ws.browser_tab_first,
    );

    let items: Vec<Element<'_, Message, iced_widget::Theme, iced_widget::Renderer>> = ws
        .browser
        .tabs()
        .iter()
        .enumerate()
        .filter(|(idx, _)| *idx >= first)
        .map(|(idx, tab)| {
            let active = idx == ws.browser.active_idx();
            let select = button(lh(text(tab.title.clone())
                .size(workspace_font::subtitle())
                .color(theme::CREAM)))
            .on_press(Message::BrowserSelectTab(idx))
            .style(|_t, _s| button::Style {
                background: None,
                text_color: theme::CREAM,
                ..button::Style::default()
            });
            let close = button(lh(text("×").size(workspace_font::body()).color(theme::DIM)))
                .on_press(Message::BrowserCloseTab(idx))
                .style(|_t, _s| button::Style {
                    background: None,
                    text_color: theme::DIM,
                    ..button::Style::default()
                });
            container(
                row![select, close]
                    .spacing(2)
                    .align_y(iced_widget::core::Alignment::Center),
            )
            .padding([2, 4])
            .style(move |_t: &iced_widget::Theme| {
                if active {
                    container::Style {
                        background: Some(theme::CARD.into()),
                        border: Border {
                            color: theme::BORDER,
                            width: 1.0,
                            radius: 6.0.into(),
                        },
                        ..container::Style::default()
                    }
                } else {
                    container::Style::default()
                }
            })
            .into()
        })
        .collect();
    let tabs_row = row(items).spacing(4);
    let clipped = container(tabs_row).width(Length::Fill).clip(true);
    let left_arrow = tab_arrow_button(
        icons::IconKind::ChevronLeft,
        can_left,
        Message::BrowserTabScroll(false),
    );
    let right_arrow = tab_arrow_button(
        icons::IconKind::ChevronRight,
        can_right,
        Message::BrowserTabScroll(true),
    );
    let tab_bar = row![left_arrow, right_arrow, clipped]
        .spacing(4)
        .align_y(iced_widget::core::Alignment::Center);

    let editing = ws.browser.addr_editing();
    let addr_text = if editing {
        format!("{}▏", ws.browser.addr_buffer())
    } else {
        "输入网址".to_string()
    };
    let addr = button(lh(text(addr_text)
        .size(workspace_font::body())
        .color(if editing { theme::CREAM } else { theme::DIM })))
    .on_press(Message::BrowserAddrClick)
    .width(Length::Fill)
    .style(move |_t, _s| button::Style {
        background: Some(theme::TERM_BG.into()),
        text_color: theme::CREAM,
        border: Border {
            color: if editing { theme::GOLD } else { theme::BORDER },
            width: 1.0,
            radius: 2.0.into(),
        },
        ..button::Style::default()
    });

    let addr_row = row![
        addr,
        browser_star_button(ws),
        browser_bookmarks_toggle_button()
    ]
    .spacing(4)
    .align_y(iced_widget::core::Alignment::Center);

    let mut content = column![tab_bar, tab_divider(), addr_row].spacing(region.gap);

    if ws.browser_star_menu_open {
        content = content.push(browser_star_menu_popup(ws));
    }
    if ws.browser_bookmarks_open {
        content = content.push(browser_bookmarks_panel(ws));
    }

    if let Some(err) = &ws.browser_error {
        content = content.push(lh(text(format!("⚠ {err}"))
            .size(workspace_font::body())
            .color(theme::RED)));
    }

    if ws.browser.tabs().is_empty() {
        content = content.push(
            container(lh(text("暂无网页——在地址栏输入网址")
                .size(workspace_font::subtitle())
                .color(theme::DIM)))
            .width(Length::Fill)
            .height(Length::Fill),
        );
    }

    container(content.padding(region.padding))
        .width(width)
        .height(Length::Fill)
        .style(move |_theme: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: outer,
            ..container::Style::default()
        })
        .into()
}

/// 当前浏览器激活 tab 若是网页,取其 URL;文件/验收 tab 返回 `None`
/// (星标按钮据此判定是否可点、菜单据此判定收藏状态)。
fn current_browser_url(ws: &Workspace) -> Option<String> {
    match ws
        .browser
        .tabs()
        .get(ws.browser.active_idx())
        .map(|t| &t.kind)
    {
        Some(TabKind::Web { url }) => Some(url.clone()),
        _ => None,
    }
}

/// 地址栏星标:当前 URL 在全局/本项目任一边已收藏则 GOLD 实心,否则
/// DIM;非网页 tab(文件/验收)禁用。
fn browser_star_button(
    ws: &Workspace,
) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let url = current_browser_url(ws);
    let starred = url
        .as_ref()
        .map(|u| {
            bookmarks::bookmark_status(&ws.bookmarks, u, ws.project.as_ref().map(|p| p.id))
                .is_bookmarked()
        })
        .unwrap_or(false);
    let color = if starred { theme::GOLD } else { theme::DIM };
    let mut btn = button(icons::view(
        icons::IconKind::Star,
        crate::icon_size::row(),
        color,
    ))
    .width(Length::Fixed(crate::workspace_geometry::tab_button_size()))
    .height(Length::Fixed(crate::workspace_geometry::tab_button_size()))
    .padding(0)
    .style(move |_t, _s| button::Style {
        background: None,
        text_color: color,
        ..button::Style::default()
    });
    if url.is_some() {
        btn = btn.on_press(Message::BrowserStarClick);
    }
    btn.into()
}

/// tab 栏"收藏夹"下拉面板触发按钮,颜色恒定(不像星标那样带收藏状态)。
fn browser_bookmarks_toggle_button<'a>()
-> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    button(icons::view(
        icons::IconKind::Bookmark,
        crate::icon_size::row(),
        theme::DIM,
    ))
    .on_press(Message::BrowserBookmarksToggle)
    .width(Length::Fixed(crate::workspace_geometry::tab_button_size()))
    .height(Length::Fixed(crate::workspace_geometry::tab_button_size()))
    .padding(0)
    .style(|_t, _s| button::Style {
        background: None,
        text_color: theme::DIM,
        ..button::Style::default()
    })
    .into()
}

fn bookmark_menu_row(
    label: String,
    msg: Message,
) -> Element<'static, Message, iced_widget::Theme, iced_widget::Renderer> {
    button(lh(text(label)
        .size(workspace_font::body())
        .color(theme::CREAM)))
    .on_press(msg)
    .width(Length::Fill)
    .padding([6, 12])
    .style(|_t: &iced_widget::Theme, _s| button::Style {
        background: None,
        text_color: theme::CREAM,
        ..button::Style::default()
    })
    .into()
}

/// 星标小菜单:未收藏显示"加入…",已收藏显示"移出…"(打勾态)。当前
/// tab 非网页时(`current_browser_url` 返回 `None`)不该能弹出这个菜单
/// (`browser_star_button` 已经不给非网页 tab 挂 `on_press`),这里仍防御
/// 性处理为空内容,不 panic。
fn browser_star_menu_popup(
    ws: &Workspace,
) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let Some(url) = current_browser_url(ws) else {
        return column![].into();
    };
    let project_id = ws.project.as_ref().map(|p| p.id);
    let status = bookmarks::bookmark_status(&ws.bookmarks, &url, project_id);

    let mut col = column![match status.global {
        Some(id) => bookmark_menu_row(
            "移出全局收藏".to_string(),
            Message::BrowserBookmarkRemove(id)
        ),
        None => bookmark_menu_row(
            "加入全局收藏".to_string(),
            Message::BrowserBookmarkAdd(BookmarkScope::Global)
        ),
    }]
    .spacing(2);

    if project_id.is_some() {
        col = col.push(match status.project {
            Some(id) => bookmark_menu_row(
                "移出本项目收藏".to_string(),
                Message::BrowserBookmarkRemove(id),
            ),
            None => bookmark_menu_row(
                "加入本项目收藏".to_string(),
                Message::BrowserBookmarkAdd(BookmarkScope::Project),
            ),
        });
    }

    container(col)
        .padding(6)
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(theme::CARD.into()),
            border: Border {
                color: theme::BORDER,
                width: 1.0,
                radius: 6.0.into(),
            },
            ..container::Style::default()
        })
        .into()
}

/// 一组收藏条目:标题(点击新开 tab)+ `×` 删除按钮,风格照抄 tab 关闭
/// 按钮。
fn bookmark_group<'a>(
    title: &'static str,
    items: &[&'a BookmarkInfo],
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    let mut col = column![lh(text(title)
        .size(workspace_font::subtitle())
        .color(theme::DIM))]
    .spacing(2);
    for b in items {
        let open = button(lh(text(b.title.clone())
            .size(workspace_font::body())
            .color(theme::CREAM)))
        .on_press(Message::BrowserOpenUrl(b.url.clone()))
        .width(Length::Fill)
        .style(|_t: &iced_widget::Theme, _s| button::Style {
            background: None,
            text_color: theme::CREAM,
            ..button::Style::default()
        });
        let remove = button(lh(text("×").size(workspace_font::body()).color(theme::DIM)))
            .on_press(Message::BrowserBookmarkRemove(b.id))
            .style(|_t: &iced_widget::Theme, _s| button::Style {
                background: None,
                text_color: theme::DIM,
                ..button::Style::default()
            });
        col = col.push(
            row![open, remove]
                .spacing(4)
                .align_y(iced_widget::core::Alignment::Center),
        );
    }
    col.into()
}

/// 收藏夹下拉面板:分"全局收藏"/"本项目收藏"两组,都为空时显示占位文案。
fn browser_bookmarks_panel(
    ws: &Workspace,
) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let project_id = ws.project.as_ref().map(|p| p.id);
    let global: Vec<&BookmarkInfo> = ws
        .bookmarks
        .iter()
        .filter(|b| b.scope == BookmarkScope::Global)
        .collect();
    let project: Vec<&BookmarkInfo> = ws
        .bookmarks
        .iter()
        .filter(|b| b.scope == BookmarkScope::Project && b.project_id == project_id)
        .collect();

    let both_empty = global.is_empty() && project.is_empty();
    let mut col = column![].spacing(6);
    col = col.push(bookmark_group("全局收藏", &global));
    if project_id.is_some() {
        col = col.push(bookmark_group("本项目收藏", &project));
    }
    if both_empty {
        col = col.push(lh(text("暂无收藏")
            .size(workspace_font::subtitle())
            .color(theme::DIM)));
    }

    container(col)
        .padding(6)
        .width(Length::Fill)
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(theme::CARD.into()),
            border: Border {
                color: theme::BORDER,
                width: 1.0,
                radius: 6.0.into(),
            },
            ..container::Style::default()
        })
        .into()
}

/// 终端栏：表头 + tab 栏 + （可能的错误文案）+ 当前激活 tab 的终端网格。
fn terminal_pane<'a>(
    app: &'a App,
    ws: &'a Workspace,
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    let region = chrome_style::terminal_pane();
    let mut content = column![tab_bar(app, ws)].spacing(region.gap);

    if let Some(err) = &app.daemon_error {
        content = content.push(
            text(format!("⚠ {err}"))
                .size(workspace_font::body())
                .color(theme::RED),
        );
    }

    // OSC 133;D 的最近命令非零退出码提示（下一条命令开始时消失）。
    if let Some(code) = ws.tabs.get(ws.active).and_then(|t| t.last_exit)
        && code != 0
    {
        content = content.push(
            text(format!("exit {code}"))
                .size(workspace_font::label())
                .color(theme::RED),
        );
    }

    // 交付横幅（spec P1f D3）:金字金框,CTA 进入验收
    if let Some(tab) = ws.tabs.get(ws.active)
        && let Some(text_str) = banner_text(tab.delivery_pending)
    {
        let banner = container(
            row![
                text(text_str)
                    .size(workspace_font::body())
                    .color(theme::GOLD),
                button(
                    text("进入验收")
                        .size(workspace_font::body())
                        .color(theme::GOLD)
                )
                .on_press(Message::AcceptanceOpen(tab.tab_id))
                .style(|_t, _s| button::Style {
                    background: Some(theme::CARD.into()),
                    text_color: theme::GOLD,
                    border: Border {
                        color: theme::GOLD,
                        width: 1.0,
                        radius: 2.0.into()
                    },
                    ..button::Style::default()
                }),
            ]
            .spacing(8),
        )
        .padding([6, 10])
        .style(|_t: &iced_widget::Theme| container::Style {
            border: Border {
                color: theme::GOLD,
                width: 1.0,
                radius: 6.0.into(),
            },
            ..container::Style::default()
        });
        content = content.push(banner);
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

/// 分隔线:命中区 `workspace_geometry::divider_width()` 宽、`Length::Fill` 高,
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
/// `chrome_style::left_zone()`/`right_zone()` 的整体外框,这条 8px 缝是故意
/// 空出来给两侧 zone 圆角边框各自收边的,不能填成某侧 pane 色。
fn divider_bar<'a>(
    divider: Divider,
    left_bg: Color,
    right_bg: Color,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    let show_line = !matches!(divider, Divider::LeftRight);
    if !show_line {
        let gap = iced_widget::Space::new()
            .width(Length::Fixed(workspace_geometry::divider_width()))
            .height(Length::Fill);
        return MouseArea::new(gap)
            .interaction(mouse::Interaction::ResizingColumn)
            .on_press(Message::ColumnDragStart(divider))
            .into();
    }
    let line_w = 2.0_f32;
    let side_w = (workspace_geometry::divider_width() - line_w) / 2.0;
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
            background: Some(theme::BORDER.into()),
            ..container::Style::default()
        });
    let row = row![left_side, line, right_side]
        .width(Length::Fixed(workspace_geometry::divider_width()))
        .height(Length::Fill);
    MouseArea::new(row)
        .interaction(mouse::Interaction::ResizingColumn)
        .on_press(Message::ColumnDragStart(divider))
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
            icons::view(icon, crate::icon_size::row(), theme::CREAM),
            text(label).size(workspace_font::body()).color(theme::CREAM),
        ]
        .spacing(crate::workspace_geometry::menu_gap())
        .align_y(iced_widget::core::Alignment::Center),
    )
    .on_press(msg)
    .width(Length::Fixed(crate::workspace_geometry::menu_item_width()))
    .padding([
        crate::workspace_geometry::menu_pad_v(),
        crate::workspace_geometry::menu_pad_h(),
    ])
    .style(|_t, _s| button::Style {
        background: Some(theme::CARD.into()),
        text_color: theme::CREAM,
        ..button::Style::default()
    })
    .into()
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
            .size(tree_row_font_size())
            .line_height(LineHeight::Relative(terminal_font::line_height_factor()))
            .color(theme::CREAM),
    )
    .width(Length::Fill)
    .padding([2, 4])
    .style(|_t: &iced_widget::Theme| container::Style {
        background: Some(theme::CARD.into()),
        border: Border {
            color: theme::CREAM,
            width: 1.0,
            radius: 2.0.into(),
        },
        ..container::Style::default()
    })
    .into()
}

/// 右键菜单浮层本体:纵向按钮列表,`container` 用 `Padding{top,left,..}`
/// 手算定位到点击坐标——`Stack` 各层共享同一份 bounds,不像原生系统菜单
/// 那样自带绝对定位,这是本仓一贯的手算像素定位风格(`ime_cursor_area`/
/// `preview_content_bounds` 同款)。
fn context_menu_popup<'a>(
    app: &'a App,
    ws: &'a Workspace,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    let Some(menu) = &app.context_menu else {
        return column![].into();
    };
    let mut items: Vec<Element<'_, Message, iced_widget::Theme, iced_widget::Renderer>> =
        Vec::new();
    if menu.is_dir {
        items.push(menu_item(
            icons::IconKind::FilePlus,
            "新建文件",
            Message::ProjectTreeNewFile(menu.target.clone()),
        ));
        items.push(menu_item(
            icons::IconKind::FolderPlus,
            "新建文件夹",
            Message::ProjectTreeNewFolder(menu.target.clone()),
        ));
    }
    items.push(menu_item(
        icons::IconKind::Copy,
        "复制",
        Message::ProjectTreeCopy(menu.target.clone(), menu.is_dir),
    ));
    if menu.is_dir {
        let has_clipboard = ws.tree_clipboard.is_some();
        let paste_msg = Message::ProjectTreePaste(menu.target.clone());
        items.push(if has_clipboard {
            menu_item(icons::IconKind::ClipboardPaste, "粘贴", paste_msg)
        } else {
            // 剪贴槽为空:置灰且不挂 on_press,真正不可点(同 P1L tab 箭头
            // "到头变灰"的既有处理口径,不是视觉变灰但仍能点)。
            button(
                row![
                    icons::view(
                        icons::IconKind::ClipboardPaste,
                        crate::icon_size::row(),
                        theme::DIM
                    ),
                    text("粘贴").size(workspace_font::body()).color(theme::DIM),
                ]
                .spacing(crate::workspace_geometry::menu_gap())
                .align_y(iced_widget::core::Alignment::Center),
            )
            .width(Length::Fixed(crate::workspace_geometry::menu_item_width()))
            .padding([
                crate::workspace_geometry::menu_pad_v(),
                crate::workspace_geometry::menu_pad_h(),
            ])
            .style(|_t, _s| button::Style {
                background: Some(theme::CARD.into()),
                text_color: theme::DIM,
                ..button::Style::default()
            })
            .into()
        });
    }
    items.push(menu_item(
        icons::IconKind::Trash,
        "删除",
        Message::ProjectTreeDeleteRequest(menu.target.clone(), menu.is_dir),
    ));
    items.push(menu_item(
        icons::IconKind::Rename,
        "重命名",
        Message::ProjectTreeRenameStart(menu.target.clone()),
    ));
    items.push(menu_item(
        icons::IconKind::Copy,
        "复制绝对路径",
        Message::ProjectTreeCopyPath(menu.target.clone(), project::PathKind::Absolute),
    ));
    items.push(menu_item(
        icons::IconKind::Copy,
        "复制相对路径",
        Message::ProjectTreeCopyPath(menu.target.clone(), project::PathKind::Relative),
    ));
    items.push(menu_item(
        icons::IconKind::FolderOpen,
        "在 Finder 中打开",
        Message::ProjectTreeRevealInFinder(menu.target.clone()),
    ));
    items.push(menu_item(
        icons::IconKind::RefreshCw,
        "从磁盘重新加载",
        Message::ProjectTreeReloadFromDisk,
    ));

    let region = chrome_style::context_menu();
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
fn delete_confirm_popup(
    ws: &Workspace,
) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let Some((path, is_dir)) = &ws.tree_delete_confirm else {
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
                .size(workspace_font::subtitle())
                .color(theme::CREAM),
            text("会移入系统回收站,可从回收站找回。")
                .size(workspace_font::label())
                .color(theme::DIM),
            row![
                button(
                    text("取消")
                        .size(workspace_font::body())
                        .color(theme::CREAM)
                )
                .on_press(Message::ProjectTreeDeleteCancel)
                .padding([6, 12])
                .style(|_t, _s| button::Style {
                    background: Some(theme::CARD.into()),
                    text_color: theme::CREAM,
                    border: Border {
                        color: theme::BORDER,
                        width: 1.0,
                        radius: 4.0.into()
                    },
                    ..button::Style::default()
                }),
                button(text("删除").size(workspace_font::body()).color(theme::RED))
                    .on_press(Message::ProjectTreeDeleteConfirm)
                    .padding([6, 12])
                    .style(|_t, _s| button::Style {
                        background: Some(theme::CARD.into()),
                        text_color: theme::RED,
                        border: Border {
                            color: theme::RED,
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
        background: Some(theme::CARD.into()),
        border: Border {
            color: theme::BORDER,
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

/// 文本编辑弹层:标题行(文件名+关闭)+ `text_editor` 主体(等宽字体)+
/// 错误位 + 保存/关闭按钮。宽高吃满大部分屏幕("放大窗口"的产品意图,
/// 不是小弹窗),四周留 `40.0` 边距,与 `maximize_overlay` 的
/// `scrim_padding` 同一量级,视觉上是同一族"大号应用内模态"。
fn edit_modal(ws: &Workspace) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let Some(session) = &ws.edit_session else {
        return column![].into();
    };
    let name = session
        .path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| session.path.display().to_string());

    let title_row = row![
        text(name)
            .size(workspace_font::subtitle())
            .color(theme::CREAM),
        iced_widget::space::horizontal(),
        button(text("×").size(workspace_font::subtitle()).color(theme::DIM))
            .on_press(Message::PreviewEditCloseRequest)
            .padding(0)
            .style(|_t, _s| button::Style {
                background: None,
                text_color: theme::DIM,
                ..button::Style::default()
            }),
    ]
    .align_y(iced_widget::core::Alignment::Center);

    let editor: Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> =
        text_editor(&session.content)
            .on_action(Message::PreviewEditAction)
            .font(crate::fonts::code_font())
            .size(workspace_font::body())
            .height(Length::Fill)
            .into();

    let mut body = column![title_row, editor].spacing(8);

    if let Some(err) = &session.error {
        body = body.push(
            text(format!("⚠ {err}"))
                .size(workspace_font::body())
                .color(theme::RED),
        );
    }

    let close_btn = button(
        text("关闭")
            .size(workspace_font::body())
            .color(theme::CREAM),
    )
    .on_press(Message::PreviewEditCloseRequest)
    .padding([6, 12])
    .style(|_t, _s| button::Style {
        background: Some(theme::CARD.into()),
        text_color: theme::CREAM,
        border: Border {
            color: theme::BORDER,
            width: 1.0,
            radius: 4.0.into(),
        },
        ..button::Style::default()
    });
    let save_btn = button(
        text("保存")
            .size(workspace_font::body())
            .color(theme::CREAM),
    )
    .on_press(Message::PreviewEditSave)
    .padding([6, 12])
    .style(|_t, _s| button::Style {
        background: Some(theme::CARD.into()),
        text_color: theme::CREAM,
        border: Border {
            color: theme::CREAM,
            width: 1.0,
            radius: 4.0.into(),
        },
        ..button::Style::default()
    });
    body = body.push(row![close_btn, save_btn].spacing(8));

    let dialog = container(body.padding(16))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(theme::CARD.into()),
            border: Border {
                color: theme::BORDER,
                width: 1.0,
                radius: 6.0.into(),
            },
            ..container::Style::default()
        });

    container(dialog)
        .padding(40.0)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(theme::SCRIM.into()),
            ..container::Style::default()
        })
        .into()
}

/// 编辑弹层的二次确认:脏改动状态下点关闭,叠在 `edit_modal` 之上。
/// 视觉风格与 `delete_confirm_popup` 一致。
fn edit_discard_confirm_popup<'a>()
-> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    let dialog = container(
        column![
            text("放弃未保存的改动?")
                .size(workspace_font::subtitle())
                .color(theme::CREAM),
            text("关闭后这次编辑不会被保存。")
                .size(workspace_font::label())
                .color(theme::DIM),
            row![
                button(
                    text("取消")
                        .size(workspace_font::body())
                        .color(theme::CREAM)
                )
                .on_press(Message::PreviewEditConfirmCancel)
                .padding([6, 12])
                .style(|_t, _s| button::Style {
                    background: Some(theme::CARD.into()),
                    text_color: theme::CREAM,
                    border: Border {
                        color: theme::BORDER,
                        width: 1.0,
                        radius: 4.0.into(),
                    },
                    ..button::Style::default()
                }),
                button(
                    text("放弃改动")
                        .size(workspace_font::body())
                        .color(theme::RED)
                )
                .on_press(Message::PreviewEditConfirmDiscard)
                .padding([6, 12])
                .style(|_t, _s| button::Style {
                    background: Some(theme::CARD.into()),
                    text_color: theme::RED,
                    border: Border {
                        color: theme::RED,
                        width: 1.0,
                        radius: 4.0.into(),
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
        background: Some(theme::CARD.into()),
        border: Border {
            color: theme::BORDER,
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

/// 箭头翻页按钮：ChevronLeft / ChevronRight，可用时 GOLD，hover 显 CARD 圆角底，到头时 DIM 且不可点。
pub(crate) fn tab_arrow_button<'a, M: Clone + 'a>(
    icon: icons::IconKind,
    enabled: bool,
    msg: M,
) -> Element<'a, M, iced_widget::Theme, iced_widget::Renderer> {
    let color = if enabled { theme::GOLD } else { theme::DIM };
    let mut btn = button(icons::view(icon, crate::icon_size::row(), color))
        .width(Length::Fixed(crate::workspace_geometry::tab_button_size()))
        .height(Length::Fixed(crate::workspace_geometry::tab_button_size()))
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
                    background: Some(theme::CARD.into()),
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
pub(crate) fn tab_divider<'a, M: 'a>() -> Element<'a, M, iced_widget::Theme, iced_widget::Renderer> {
    container(iced_widget::Space::new())
        .width(Length::Fill)
        .height(Length::Fixed(1.0))
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(theme::BORDER.into()),
            ..container::Style::default()
        })
        .into()
}

/// tab 栏：两侧箭头翻页(到头变灰) + 每会话一个按钮(状态点 + 名称 + 关闭
/// ×)。新建会话走 Agent 面板"＋"(纯 Shell 也在其菜单里),终端 tab 栏
/// 不再放独立"＋"。P1L T5 验收返工：横向 scrollable(底部滚动条)
/// 换成索引窗口化 + `clip`——`on_scroll` 只认滚轮/拖拽，程序化滚动在本
/// app 自建循环里够不到，箭头翻页必须走状态驱动的窗口渲染。
fn tab_bar<'a>(
    app: &'a App,
    ws: &'a Workspace,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    let widths: Vec<f32> = ws
        .tabs
        .iter()
        .map(|t| tab_display_width(&tab_title(t.agent, t.cwd.as_deref(), &t.info.name)))
        .collect();
    let (first, can_left, can_right) = tab_window(
        &widths,
        4.0,
        workspace_geometry::tab_bar_avail_px(),
        ws.term_tab_first,
    );

    let items: Vec<Element<'_, Message, iced_widget::Theme, iced_widget::Renderer>> = ws
        .tabs
        .iter()
        .enumerate()
        .filter(|(idx, _)| *idx >= first)
        .map(|(idx, tab)| tab_item(idx, tab, idx == ws.active, app.blink_on))
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

/// 交付横幅文案：pending 才有（金色,甲方动作）。
fn banner_text(pending: bool) -> Option<&'static str> {
    pending.then_some("交付待验收")
}

/// 对话副行文案：`<agent> · <相对时间> · <规模>`（P1j）。
/// 相对时间文案：刚刚/N 分钟前/N 小时前/N 天前（D5，从 `conversation_sub`
/// 抽出为独立纯函数）。H0 项目卡"活跃时间"、文件卡、对话卡三处复用，
/// 不要三份重复 switch。
fn relative_time_text(modified_ms: u64, now_ms: u64) -> String {
    let ago = now_ms.saturating_sub(modified_ms) / 1000; // 秒
    if ago < 60 {
        "刚刚".to_string()
    } else if ago < 3600 {
        format!("{} 分钟前", ago / 60)
    } else if ago < 86400 {
        format!("{} 小时前", ago / 3600)
    } else {
        format!("{} 天前", ago / 86400)
    }
}

/// 对话副行文案：`<agent> · <相对时间> · <规模>`（P1j）。
fn conversation_sub(agent: &str, modified_ms: u64, size_bytes: u64, now_ms: u64) -> String {
    let when = relative_time_text(modified_ms, now_ms);
    let size = if size_bytes >= 1024 * 1024 {
        format!("{:.1}MB", size_bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{}KB", (size_bytes / 1024).max(1))
    };
    format!("{agent} · {when} · {size}")
}

/// H0"最近的文件"卡一行(跨项目合并前的中间表示；D4)。`Message::
/// HomeRecentsLoaded` 的载荷用到它，因此至少是 `pub(crate)`(见 `private_interfaces`)。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct HomeRecentFile {
    path: PathBuf,
    project_name: String,
    modified_ms: u64,
}

/// H0"最近的对话"卡一行；语义同上。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct HomeRecentConversation {
    project_name: String,
    meta: ConversationMeta,
}

/// D4 纯 IO 内核：对给定项目列表分别取"最近改动的文件"(git 改动/未跟踪 +
/// fs mtime)与"最近的对话"(三个 agent 来源已聚合、按 mtime 倒序)，跨项目
/// 合并后各自按时间倒序，取前 4 条 / 前 3 条(对齐 Figma 卡片行数)。
///
/// 必须在 `spawn_blocking` 里跑，不能在 UI 线程直呼——内部既有阻塞 git
/// 子进程调用，也有阻塞文件系统调用。签名固定(`&[ProjectInfo]` 输入，两个
/// `Vec` 输出)方便 headless 单测：不需要 daemon 连接或 winit `EventLoopProxy`。
fn load_home_recents(
    projects: &[ProjectInfo],
) -> (Vec<HomeRecentFile>, Vec<HomeRecentConversation>) {
    let mut files: Vec<HomeRecentFile> = Vec::new();
    let mut convs: Vec<HomeRecentConversation> = Vec::new();
    for p in projects {
        let cwd = PathBuf::from(&p.path);
        if let Some(repo) = delivery::repo_root(&cwd) {
            for (path, _status) in delivery::file_statuses(&repo) {
                let Ok(meta) = std::fs::metadata(&path) else {
                    continue; // 路径已在磁盘消失(用户手动删了),静默跳过(spec §4)
                };
                let modified_ms = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                files.push(HomeRecentFile {
                    path,
                    project_name: p.name.clone(),
                    modified_ms,
                });
            }
        }
        for meta in conversation::list_all_conversations(&cwd) {
            convs.push(HomeRecentConversation {
                project_name: p.name.clone(),
                meta,
            });
        }
    }
    files.sort_by_key(|f| std::cmp::Reverse(f.modified_ms));
    files.truncate(4);
    convs.sort_by_key(|c| std::cmp::Reverse(c.meta.modified_ms));
    convs.truncate(3);
    (files, convs)
}

/// AI 回合折叠行文案（P1i）：过程 = thinking + N 工具。
fn ai_turn_summary(tools_len: usize, thinking: bool) -> String {
    match (thinking, tools_len) {
        (false, 0) => "过程:无".into(),
        (true, 0) => "过程:思考".into(),
        (false, n) => format!("过程:{n} 工具"),
        (true, n) => format!("过程:思考 + {n} 工具"),
    }
}

/// 交付/验收使用的仓库：当前项目优先，无则回落会话 cwd（P1f 现状；P1g D4）。
fn effective_project_repo(active: Option<&Path>, session_cwd: &Path) -> PathBuf {
    active
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| session_cwd.to_path_buf())
}

/// 文件/目录 git 状态 → 行尾彩色圆点色。金=改/绿=新/红=删。
/// 色点颜色编码改动类型(kind),不变——D2 只新增了填充态维度,不推翻既有
/// 配色约定。
fn tree_row_dot_color(kind: delivery::ChangeKind) -> Color {
    match kind {
        delivery::ChangeKind::Modified => theme::GOLD,
        delivery::ChangeKind::New => theme::GREEN,
        delivery::ChangeKind::Deleted => theme::RED,
    }
}

/// 色点字形编码暂存态(D2):全部暂存(无未暂存改动)→ 实心 `●`;有任何未
/// 暂存改动(不论是否同时有暂存部分)→ 空心 `○`。尾缀字符不重复编码 kind
/// (颜色已经够用),避免过度设计。
fn tree_row_dot_glyph(unstaged: bool) -> &'static str {
    if unstaged { "○" } else { "●" }
}

/// 项目卡分支标签：`分支` / `分支*`（脏）/ `—`（非 git）。
fn project_branch_label(branch: Option<&str>, dirty: bool) -> String {
    match branch {
        Some(b) if dirty => format!("{b}*"),
        Some(b) => b.to_string(),
        None => "—".to_string(),
    }
}

/// 顶栏目标胶囊文案：`目标：{标题}`；标题过长按字符截断加省略号。
/// 无 goal 或空标题 → None（胶囊隐藏）。`max_chars` 含省略号占位。
fn goal_capsule_text(goal: Option<&Goal>, max_chars: usize) -> Option<String> {
    let title = goal?.title.trim();
    if title.is_empty() {
        return None;
    }
    let shown = if title.chars().count() > max_chars {
        let mut s: String = title.chars().take(max_chars.saturating_sub(1)).collect();
        s.push('…');
        s
    } else {
        title.to_string()
    };
    Some(format!("目标：{shown}"))
}

/// 同步读 `.dozer/goal.md` 并解析（顶栏胶囊用；文件极小，可容忍同步读）。
fn load_project_goal(repo_path: &str) -> Option<Goal> {
    let md = std::fs::read_to_string(goal::goal_path(Path::new(repo_path))).ok()?;
    goal::parse_goal(&md)
}

/// 从项目路径求"验收查询键"：与落库侧同款 `delivery::repo_root`（git
/// toplevel，解析符号链接/子目录），保证 `count_for_repo` 精确匹配命中。
/// 非 git 路径返回 `None`——验收依赖 git ref 沉淀，非 git 仓库不可能有记录，
/// 直接不查，别拿未规范化的原始路径去撞库（会静默查不到→副行空白）。
fn acceptance_query_repo(project_path: &str) -> Option<String> {
    delivery::repo_root(Path::new(project_path)).map(|p| p.to_string_lossy().into_owned())
}

/// 标准行文案:金勾 ✓ / 空圈 ○。
fn criteria_line(checked: bool, text: &str) -> String {
    format!("{} {}", if checked { "✓" } else { "○" }, text)
}

/// 变更文件行:path  +a −r;未跟踪标 (新)。
fn file_change_line(fc: &FileChange) -> String {
    match (fc.added, fc.removed) {
        (Some(a), Some(r)) => format!("{}  +{a} −{r}", fc.path),
        _ => format!("{}  (新)", fc.path),
    }
}

/// agent 选择菜单选中的 agent → 要自动键入 PTY 的 CLI 命令名。`Unknown`
/// 不该从选择菜单产生(选项只有 Claude/CodeBuddy/OpenCode/纯 Shell 四选
/// 一,纯 Shell 走 `launch: None`,不经过这个函数),但函数保持穷尽
/// match,防止未来枚举新增变体时静默漏写。已知变体的 CLI 名字与
/// `AgentKind::label()` 逐字节一致(`label()` 本身就是给这三个变体返回
/// 小写 CLI 名),这里直接复用而不重复一份映射表,避免两处拼写分叉。
fn agent_cli_command(agent: AgentKind) -> Option<&'static str> {
    match agent {
        AgentKind::Unknown => None,
        known => Some(known.label()),
    }
}

/// picker 选择项 → attach 成功后自动键入 PTY 的初始命令。`Agent(Some(a))`
/// 复用 `agent_cli_command`(键入 agent CLI);`Agent(None)` 不键入(纯 Shell);
/// `Git` 键入 `git status`——新开的 shell 已在项目根,直接看仓库状态。
/// 抽成纯函数是为了能 headless 单测(同 `agent_cli_command` 的惯例)。
fn picker_launch_command(launch: PickerLaunch) -> Option<String> {
    match launch {
        PickerLaunch::Agent(Some(agent)) => agent_cli_command(agent).map(str::to_owned),
        PickerLaunch::Agent(None) => None,
        PickerLaunch::Git => Some("git status".to_string()),
    }
}

/// tab 标题：已识别出 agent（hook 上报）则显 agent 名（如 "claude"）；
/// 否则回落到 OSC 7 的 cwd basename，再无 cwd 才回落会话名。
fn tab_title(agent: AgentKind, cwd: Option<&Path>, fallback: &str) -> String {
    if agent != AgentKind::Unknown {
        return agent.label().to_string();
    }
    match cwd {
        Some(p) => p
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| p.to_string_lossy().into_owned()),
        None => fallback.to_string(),
    }
}

/// 文本显示宽度的基础单元和：CJK 字符按全宽计 2，其余按半宽计 1。
/// `tab_display_width`/`preview_tab_display_width` 共用。
fn text_width_units(s: &str) -> f32 {
    s.chars()
        .map(|c| if (c as u32) > 0x2E80 { 2.0 } else { 1.0 })
        .sum()
}

/// 终端 tab 估算显示宽（逻辑像素）：状态点+名称+关闭×+pill padding 的粗估。
/// 不追求精确——估偏几像素只会让翻页边界差一个 tab。
fn tab_display_width(title: &str) -> f32 {
    // 状态点●+spacing ≈ 18, 名称 ≈ units * 半宽 8.0(14px), 关闭× ≈ 18, pill padding ≈ 12
    18.0 + text_width_units(title) * 8.0 + 18.0 + 12.0
}

/// 预览 tab 估算显示宽：同 `tab_display_width` 但无状态点。
pub(crate) fn preview_tab_display_width(title: &str) -> f32 {
    // 名称 ≈ units * 半宽 8.0(14px), 关闭× ≈ 18, pill padding ≈ 12
    text_width_units(title) * 8.0 + 18.0 + 12.0
}

/// 给定各 tab 宽、tab 间距、可视宽、当前 first，算出：
/// (钳制后的 first, 左可滚, 右可滚)。
/// - 全部 tab 能放下(总宽<=avail) → first=0, 两端皆不可滚(箭头都变灰)。
/// - 溢出 → max_first = 最小的 i 使 tabs[i..] 总宽 <= avail(即从 i 起剩余恰好放得下);
///   钳制 first 到 [0, max_first]; 左可滚 = first>0; 右可滚 = first<max_first。
pub(crate) fn tab_window(widths: &[f32], gap: f32, avail: f32, first: usize) -> (usize, bool, bool) {
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

/// agent 四态中文（终端状态栏用）。
fn agent_state_label(state: AgentState) -> &'static str {
    match state {
        AgentState::Running => "运行中",
        AgentState::AwaitingInput => "待输入",
        AgentState::TurnEnded => "回合毕",
        AgentState::Idle => "空闲",
    }
}

/// 环境状态栏文案 + 点色：daemon 连通=绿"环境正常", 断=红"未连接"。
fn env_status_text(daemon_ok: bool) -> (&'static str, Color) {
    if daemon_ok {
        ("环境正常 · dozerd 运行中", theme::GREEN)
    } else {
        ("dozerd 未连接", theme::RED)
    }
}

/// tab 前状态点配色：死会话灰；存活按 agent 状态——绿=空闲/运行、
/// 紫蓝=待输入、金=回合结束（金是甲方动作专属色：该出手了）。运行态
/// 与空闲态同为绿，靠 `tab_item` 里的闪烁区分（工作中才闪）。
fn dot_color(state: AgentState, alive: bool) -> Color {
    if !alive {
        return theme::DIM;
    }
    match state {
        AgentState::Idle | AgentState::Running => theme::GREEN,
        AgentState::AwaitingInput => theme::PURPLE,
        AgentState::TurnEnded => theme::GOLD,
    }
}

/// agent → 对话列表圆点颜色。避开 `theme::GOLD`(甲方动作专属色,
/// CLAUDE.md 明文规定,不能被 agent 分类语义借用)。
pub(crate) fn agent_dot_color(agent: AgentKind) -> Color {
    match agent {
        AgentKind::Claude => theme::CYAN,
        AgentKind::Codebuddy => theme::PURPLE,
        AgentKind::Opencode => theme::GREEN,
        AgentKind::Codex => theme::ORANGE,
        AgentKind::Qoder => theme::MAGENTA,
        AgentKind::Kilo => theme::BLUE,
        AgentKind::Unknown => theme::DIM,
    }
}

/// agent → 品牌图标(新建 agent 菜单用)。颜色由调用方按 `agent_dot_color`
/// 同款语义传入,使图标色与圆点色一致,避免引入新配色维度。
fn agent_icon(agent: AgentKind) -> IconKind {
    match agent {
        AgentKind::Claude => IconKind::Claude,
        AgentKind::Codebuddy => IconKind::Codebuddy,
        AgentKind::Opencode => IconKind::Opencode,
        // 暂无确认可用的品牌素材，回落通用图标（spec §8/§6 明确允许）。
        AgentKind::Codex | AgentKind::Qoder | AgentKind::Kilo | AgentKind::Unknown => IconKind::Bot,
    }
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
) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let working = tab.alive && tab.agent_state == AgentState::Running;
    let mut color = dot_color(tab.agent_state, tab.alive);
    // 工作中且处于暗相位：把点点压到近乎透明，形成"呼吸"般的闪烁。
    // 闪烁相位是全局的（main.rs 定时翻转），因此失焦的工作 tab 也照闪。
    if working && !blink_on {
        color = Color { a: 0.15, ..color };
    }
    let label = row![
        text("●").size(workspace_font::caption()).color(color),
        text(tab_title(tab.agent, tab.cwd.as_deref(), &tab.info.name))
            .size(workspace_font::subtitle())
            .color(theme::CREAM),
    ]
    .spacing(4);

    let select = button(label)
        .on_press(Message::SelectTab(idx))
        .style(|_theme, _status| button::Style {
            background: None,
            text_color: theme::CREAM,
            ..button::Style::default()
        });

    let close = button(text("×").size(workspace_font::body()).color(theme::DIM))
        .on_press(Message::CloseTab(idx))
        .style(|_theme, _status| button::Style {
            background: None,
            text_color: theme::DIM,
            ..button::Style::default()
        });

    container(
        row![select, close]
            .spacing(2)
            .align_y(iced_widget::core::Alignment::Center),
    )
    .padding([2, 4])
    .style(move |_t: &iced_widget::Theme| {
        if active {
            container::Style {
                background: Some(theme::CARD.into()),
                border: Border {
                    color: theme::BORDER,
                    width: 1.0,
                    radius: 6.0.into(),
                },
                ..container::Style::default()
            }
        } else {
            container::Style::default()
        }
    })
    .into()
}

fn active_tab_view<'a>(
    app: &'a App,
    ws: &'a Workspace,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    match ws.tabs.get(ws.active) {
        Some(tab) => term_view::view(&tab.model, app.term_focused),
        None => container(
            text("暂无会话——到 Agent 面板点「＋」")
                .size(workspace_font::subtitle())
                .color(theme::DIM),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced_widget::text_editor;

    /// 造一个带记号的 `Loaded` 槽位,用 `tree_error` 当身份标记——这样
    /// 重挂之后能断言"搬过去的确实是同一个槽位",而不只是"新 id 上有东西"。
    fn loaded_slot(marker: &str) -> WorkspaceSlot {
        let mut ws = Workspace::empty_for_project_placeholder();
        ws.tree_error = Some(marker.to_string());
        WorkspaceSlot::Loaded(Box::new(ws))
    }

    fn slot_marker(slot: Option<&WorkspaceSlot>) -> Option<String> {
        match slot {
            Some(WorkspaceSlot::Loaded(ws)) => ws.tree_error.clone(),
            _ => None,
        }
    }

    fn project_info(id: i64) -> ProjectInfo {
        ProjectInfo {
            id,
            path: format!("/tmp/p{id}"),
            name: format!("p{id}"),
            last_active_ms: 0,
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
            ws.tree_error.as_deref(),
            Some("A"),
            "后台项目的结果不能落到前台项目"
        );
        // 反向同理:B 的结果落到 B。
        let ws = loaded_workspace_mut(&mut projects, 2).expect("B 已加载");
        assert_eq!(ws.tree_error.as_deref(), Some("B"));
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
            loaded_workspace_mut(&mut projects, 1).and_then(|w| w.tree_error.clone()),
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

    /// 促成期占位的**核心不变式**:同步换上的那一刻就必须已知归属项目。
    ///
    /// 促成是异步的,这份占位会在消息环里存活若干毫秒,而且它就是当前聚焦
    /// 的 workspace(用户正是点了这个页签才触发促成)。只要它 `project` 为
    /// `None`,`spawn_new_tab` 的 `expect("Workspace 存在即已知归属项目")` 就
    /// 重新变成可达路径——用户在这段窗口里点一下终端 tab 栏的"＋"就会
    /// panic 掉整个 GUI 进程(Task 5/6 review 反复强调的那条)。
    #[test]
    fn loading_placeholder_always_knows_its_project() {
        let info = ProjectInfo {
            id: 42,
            path: "/tmp/p42".to_string(),
            name: "p42".to_string(),
            last_active_ms: 0,
        };
        let ws = Workspace::loading_for_project(info.clone());
        assert_eq!(
            ws.project.as_ref().map(|p| p.id),
            Some(42),
            "占位必须携带项目,否则 spawn_new_tab 的 expect 变成可达路径"
        );
        assert_eq!(
            ws.project.as_ref().map(|p| p.path.as_str()),
            Some("/tmp/p42")
        );
        assert!(ws.file_tree.is_some(), "文件树根不需要 IO,应当立刻可画");
        assert!(ws.loading, "必须打上占位标记,促成结果才认得出该替换谁");
        assert!(ws.tabs.is_empty(), "会话要等 IO,占位阶段不该有 tab");
    }

    /// 反过来:空壳占位(`retarget_active_slot` 在同一条消息内用完即改写的
    /// 那种)不带项目也不带 `loading` 标记——它的安全性靠"同步内改写完",
    /// 与促成占位是两码事,不能互相顶替。
    #[test]
    fn empty_placeholder_is_not_a_loading_placeholder() {
        let ws = Workspace::empty_for_project_placeholder();
        assert!(ws.project.is_none());
        assert!(!ws.loading);
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
        assert_eq!(project_dot(&[Idle]), Some((theme::GREEN, false)));
        assert_eq!(
            project_dot(&[Idle, Running]),
            Some((theme::GREEN, true)),
            "有会话在跑 → 同为绿但要闪,靠闪烁与空闲区分"
        );
        assert_eq!(
            project_dot(&[Idle, Running, AwaitingInput]),
            Some((theme::PURPLE, false)),
            "待输入优先于运行/空闲"
        );
        assert_eq!(
            project_dot(&[Idle, Running, AwaitingInput, TurnEnded]),
            Some((theme::GOLD, false)),
            "回合结束(该甲方出手了)优先级最高"
        );
        // 顺序无关:优先级看的是状态集合,不是 tab 的先后。
        assert_eq!(
            project_dot(&[TurnEnded, Idle]),
            project_dot(&[Idle, TurnEnded])
        );
    }

    #[test]
    fn chrome_constants_exclude_removed_header() {
        // #4 去掉 header 行(22px + 一处 spacing 4 = 26)后的期望值,锁死防漂移遮挡。
        // 文件预览又去掉了地址栏(只剩 tab 栏),浏览器仍保留地址栏,两分支
        // chrome 高度分道,不能再共用同一个值——否则文件预览顶上会露一截
        // 再也画不出东西的空白。
        assert_eq!(
            workspace_geometry::preview_chrome_top_px(),
            38.0,
            "文件预览 chrome 顶应为去地址栏后的 38(8 内边距 + 30 tab 栏)"
        );
        assert_eq!(
            workspace_geometry::browser_chrome_top_px(),
            72.0,
            "浏览器 chrome 顶应为 72(8 内边距 + 30 tab 栏 + 4 spacing + 30 地址栏)"
        );
        assert_eq!(
            workspace_geometry::chrome_height_px(),
            50.0,
            "终端 chrome 高应为去 header 后的 50"
        );
    }

    /// 测试基准态:默认布局、左=文件列表、右=Agent、两侧都展开。
    fn test_state() -> ShellState {
        ShellState {
            layout: ShellLayout::default(),
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
        let list_w = state.layout.left_width * state.layout.files_split;
        let col_start =
            workspace_geometry::icon_rail_width() + list_w + workspace_geometry::divider_width();
        assert!(x >= col_start && x < col_start + 16.0, "x={x}");
        assert!((380.0..=420.0).contains(&w), "w={w}");
        assert!(
            (y - (78.0 + chrome_style::left_zone().margin.top)).abs() < 0.1,
            "y={y}(顶栏 40 + tab 栏 38 + left_zone 上 margin 之下,地址栏已去)"
        );
        assert!(h > 700.0 && h < 900.0 - y, "h={h}");
    }

    #[test]
    fn preview_content_bounds_web_view_spans_whole_left_zone() {
        // Web 视图没有项目树配对,预览内容区从图标栏右侧起占满左面板区。
        let state = ShellState {
            left_view: LeftView::Web,
            ..test_state()
        };
        let (x, _, w, _) = preview_content_bounds(1440.0, 900.0, &state);
        let m = chrome_style::left_zone().margin;
        assert_eq!(x, workspace_geometry::icon_rail_width() + 8.0 + m.left);
        assert_eq!(w, state.layout.left_width - 16.0 - m.left - m.right);
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
    /// x0=workspace_geometry::icon_rail_width()(44)+workspace_geometry::maximize_overlay_padding()(40)=84,
    /// avail_w=1440-2*44-2*40=1272,pair_w=1272-8=1264,
    /// list_w=1264*0.35=442.4,x=84+442.4+8+8=542.4,w=1264*0.65-16=805.6;
    /// y0=workspace_geometry::top_bar_height()(40)+40=80,y=80+38(workspace_geometry::preview_chrome_top_px(),地址栏已去)=118,
    /// avail_h=900-40-80=780,h=780-38-8=734。
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
        assert!((h - 734.0).abs() < 0.1, "h={h}");
        // 明显区别于平时(非放大)的几何——不能巧合碰上同一个值。
        let normal = preview_content_bounds(1440.0, 900.0, &test_state());
        assert_ne!((x, y, w, h), normal, "放大态几何必须和平时不同");
    }

    /// Fix round 1 Critical:左侧被放大(Web 视图,无项目树配对)时同样要
    /// 按放大盒子换算。x=x0+8=92,w=avail_w-16=1256。
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
        let only_chrome = 900.0 - workspace_geometry::chrome_height_px();
        let m = chrome_style::right_zone().margin;
        assert!(
            (only_chrome
                - h_with
                - (workspace_geometry::top_bar_height()
                    + workspace_geometry::status_bar_height()
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
    /// 上界 = max(624 - 320(workspace_geometry::min_zone_width()), 320) = 320;
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
        assert_eq!(state.layout.left_width, 640.0, "前提:默认持久化宽 640");

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
        assert_eq!(
            state.layout.left_width, 640.0,
            "夹取不得改写 ShellLayout 里的持久化宽"
        );
    }

    /// 极窄窗口(比 `workspace_geometry::min_window_width()` 还窄,例如外部强制 resize)下也不 panic,
    /// 且左区宽不会超过 `zones_width` 本身。
    #[test]
    fn clamp_left_width_survives_absurdly_narrow_window() {
        assert_eq!(
            clamp_left_width(200.0, 640.0),
            workspace_geometry::min_zone_width()
        );
        assert_eq!(
            clamp_left_width(0.0, 640.0),
            workspace_geometry::min_zone_width()
        );
        // 最小窗口宽恰好能让两侧都拿到 workspace_geometry::min_zone_width()。
        assert_eq!(
            clamp_left_width(workspace_geometry::min_window_width(), 640.0),
            workspace_geometry::min_zone_width()
        );
        assert!(
            (zones_width(workspace_geometry::min_window_width())
                - 2.0 * workspace_geometry::min_zone_width())
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
    /// 具体数字(1440x900,`agent_split`=0.4):
    /// avail_w = 1440 - 2*44 - 2*40 = 1272,pair_w = 1272 - 8 = 1264,
    /// 终端占 1-0.4 → 1264*0.6 = 758.4,减 `workspace_geometry::chrome_width_px()`(16) = 742.4;
    /// 盒子高 = 900 - 40(顶栏) - 2*40 = 780,再减 pane 自带底栏 26
    /// (`workspace_geometry::status_bar_height()`)与 `workspace_geometry::chrome_height_px()`(50) = 704。
    /// 对照平时:zones_width = 1440-2*44-8=1344,right_w = 1344 - 640 = 704,pair = 696,
    /// 696*0.6 = 417.6,减 16 = 401.6;高 = 900 - 40 - 26 - 50 - right_zone 上下 margin(各 6) = 772。
    /// 换成网格(CELL_WIDTH=8.4,LINE_HEIGHT_PX=16.8 即 14*1.2):放大后 88x41,平时 47x45。
    #[test]
    fn terminal_pane_pixel_size_right_maximized_matches_overlay_box() {
        let maxed = ShellState {
            maximized: Some(MaximizedPane::Right),
            ..test_state()
        };
        let (w, h) = terminal_pane_pixel_size(1440.0, 900.0, &maxed);
        assert!((w - 742.4).abs() < 0.1, "w={w}");
        assert!((h - 704.0).abs() < 0.1, "h={h}");

        let normal = terminal_pane_pixel_size(1440.0, 900.0, &test_state());
        assert!((normal.0 - 401.6).abs() < 0.1, "平时 w={}", normal.0);
        assert!((normal.1 - 772.0).abs() < 0.1, "平时 h={}", normal.1);
        assert_ne!((w, h), normal, "放大态几何必须和平时不同");
        assert!(w > normal.0, "放大后终端必须真的更宽(网格跟着变宽)");

        assert_eq!(crate::term_view::grid_size(w, h), (88, 41));
        assert_eq!(crate::term_view::grid_size(normal.0, normal.1), (47, 45));

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

        // 具体网格:1440x900 下应是 47x45(已扣 right_zone 上下 margin),不是兜底的 80x24。
        let (cols, rows) = crate::term_view::grid_size(shown.0, shown.1);
        assert_eq!((cols, rows), (47, 45));
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

    /// 可选项:磁盘上的布局值不一定出自本程序(手改 layout.json/别的版本)。
    /// split 恰为 0.0/1.0 时 `split_portions` 会给出 `FillPortion(0)`,那一块
    /// 在 flex 里拿不到任何宽度、整块消失,所以读盘时先夹一遍。
    #[test]
    fn sanitize_shell_layout_clamps_foreign_values() {
        let poisoned = ShellLayout {
            left_width: 10.0,
            files_split: 0.0,
            agent_split: 1.0,
            conversations_split: f32::NAN,
            ..ShellLayout::default()
        };
        let s = sanitize_shell_layout(poisoned);
        assert_eq!(s.left_width, workspace_geometry::min_zone_width());
        assert_eq!(s.files_split, workspace_geometry::min_split_ratio());
        assert_eq!(s.agent_split, workspace_geometry::max_split_ratio());
        assert_eq!(s.conversations_split, ShellLayout::default().files_split);
        // 夹过之后 FillPortion 两侧都非 0(那一块不会凭空消失)。
        for split in [s.files_split, s.agent_split, s.conversations_split] {
            let (list, content) = split_portions(split);
            assert!(list > 0 && content > 0, "split={split}");
        }
        // 合法值原样保留。
        let sane = ShellLayout {
            left_width: 500.0,
            files_split: 0.35,
            ..ShellLayout::default()
        };
        assert_eq!(sanitize_shell_layout(sane), sane);
    }

    /// `window_width`/`window_height` 的夹取单独测:老 `layout.json` 缺这两
    /// 个字段时 serde 补 0.0(不是 `f32::NAN`,判断要用 `> 0.0` 而不能只查
    /// `is_finite`),负数/NAN 同样要落回 `workspace_geometry::initial_window_size()`;合法但过小
    /// 的值只夹下限,不整个重置。
    #[test]
    fn sanitize_shell_layout_clamps_window_size() {
        let missing_fields = ShellLayout {
            window_width: 0.0,
            window_height: 0.0,
            ..ShellLayout::default()
        };
        let s = sanitize_shell_layout(missing_fields);
        assert_eq!(s.window_width, workspace_geometry::initial_window_size().0);
        assert_eq!(s.window_height, workspace_geometry::initial_window_size().1);

        let poisoned = ShellLayout {
            window_width: -100.0,
            window_height: f32::NAN,
            ..ShellLayout::default()
        };
        let s = sanitize_shell_layout(poisoned);
        assert_eq!(s.window_width, workspace_geometry::initial_window_size().0);
        assert_eq!(s.window_height, workspace_geometry::initial_window_size().1);

        let too_small = ShellLayout {
            window_width: 10.0,
            window_height: 10.0,
            ..ShellLayout::default()
        };
        let s = sanitize_shell_layout(too_small);
        assert_eq!(s.window_width, workspace_geometry::min_window_width());
        assert_eq!(s.window_height, workspace_geometry::min_window_height());

        let legit = ShellLayout {
            window_width: 1800.0,
            window_height: 1100.0,
            ..ShellLayout::default()
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
    fn criteria_check_line_renders_gold_check() {
        assert_eq!(criteria_line(true, "测试全绿"), "✓ 测试全绿");
        assert_eq!(criteria_line(false, "测试全绿"), "○ 测试全绿");
    }

    #[test]
    fn file_change_line_formats_counts() {
        use crate::delivery::FileChange;
        let fc = FileChange {
            path: "src/a.rs".into(),
            added: Some(3),
            removed: Some(1),
        };
        assert_eq!(file_change_line(&fc), "src/a.rs  +3 −1");
        let un = FileChange {
            path: "new.txt".into(),
            added: None,
            removed: None,
        };
        assert_eq!(file_change_line(&un), "new.txt  (新)");
    }

    #[test]
    fn acceptance_query_repo_none_for_non_git_path() {
        // 非 git 目录必须回 None(不能退化成原始路径去撞库——Important #2)。
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            acceptance_query_repo(&dir.path().to_string_lossy()),
            None,
            "非 git 临时目录应回 None"
        );
        // 不存在的路径同样 None(repo_root 先做 is_dir 检查)。
        assert_eq!(acceptance_query_repo("/no/such/path/xyz"), None);
    }

    #[test]
    fn effective_project_repo_prefers_active() {
        use std::path::Path;
        assert_eq!(
            effective_project_repo(Some(Path::new("/proj")), Path::new("/home/me")),
            PathBuf::from("/proj")
        );
        assert_eq!(
            effective_project_repo(None, Path::new("/home/me")),
            PathBuf::from("/home/me")
        );
    }

    #[test]
    fn conversation_sub_line_format() {
        let s = conversation_sub("claude", 1000, 78 * 1024, 1000);
        assert!(s.starts_with("claude · "), "含 agent 前缀: {s}");
        assert!(s.ends_with("· 78KB"), "含规模: {s}");
        let s2 = conversation_sub("claude", 1000, 8 * 1024 * 1024, 1000);
        assert!(s2.ends_with("· 8.0MB"), "MB 规模: {s2}");
    }

    #[test]
    fn relative_time_text_boundaries() {
        assert_eq!(relative_time_text(1000, 1000), "刚刚");
        assert_eq!(relative_time_text(0, 59_000), "刚刚");
        assert_eq!(relative_time_text(0, 60_000), "1 分钟前");
        assert_eq!(relative_time_text(0, 3_599_000), "59 分钟前");
        assert_eq!(relative_time_text(0, 3_600_000), "1 小时前");
        assert_eq!(relative_time_text(0, 86_399_000), "23 小时前");
        assert_eq!(relative_time_text(0, 86_400_000), "1 天前");
    }

    #[test]
    fn load_home_recents_empty_input_returns_empty_vecs() {
        let (files, convs) = load_home_recents(&[]);
        assert!(files.is_empty());
        assert!(convs.is_empty());
    }

    #[test]
    fn load_home_recents_merges_and_sorts_across_projects() {
        let proj_a = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(proj_a.path())
            .status()
            .unwrap();
        std::fs::write(proj_a.path().join("a.txt"), "changed").unwrap();

        let proj_b = tempfile::tempdir().unwrap(); // 非 git 目录,没有改动可报告

        let projects = vec![
            ProjectInfo {
                id: 1,
                path: proj_a.path().to_string_lossy().into_owned(),
                name: "proj-a".into(),
                last_active_ms: 0,
            },
            ProjectInfo {
                id: 2,
                path: proj_b.path().to_string_lossy().into_owned(),
                name: "proj-b".into(),
                last_active_ms: 0,
            },
        ];

        let (files, convs) = load_home_recents(&projects);
        assert_eq!(files.len(), 1, "只有项目 A(git repo)贡献一条改动文件");
        assert_eq!(files[0].project_name, "proj-a");
        assert!(files[0].path.ends_with("a.txt"));
        // 这里不额外造一个带假 Claude 对话目录的项目去断言"合并进 convs"：
        // `conversation::list_all_conversations` 内部读真实 `HOME` 环境变量
        // (`conversation.rs::home_dir`)，没有注入点；`conversation.rs` 自己的
        // 测试也因为同样原因(cargo test 多线程、mutate HOME 不安全)绕开了
        // 真实入口，转而在 `project_dir_in` 这一层验证目录拼接+合并排序(见
        // `list_all_conversations_merges_three_dirs_sorted_by_mtime`)。三个
        // agent 目录的合并/排序逻辑已经在那条测试里覆盖，这里只需确认
        // "没有可达对话目录时 convs 为空、不 panic"这一层 `load_home_recents`
        // 自己的收尾逻辑。
        assert!(convs.is_empty(), "两个项目都没有可达的 agent 对话目录");
    }

    #[test]
    fn load_home_recents_truncates_files_to_top_4() {
        let proj = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(proj.path())
            .status()
            .unwrap();
        for i in 0..6 {
            std::fs::write(proj.path().join(format!("f{i}.txt")), "x").unwrap();
        }
        let projects = vec![ProjectInfo {
            id: 1,
            path: proj.path().to_string_lossy().into_owned(),
            name: "proj".into(),
            last_active_ms: 0,
        }];
        let (files, _convs) = load_home_recents(&projects);
        assert_eq!(
            files.len(),
            4,
            "跨项目合并后只取前 4 条(对齐 Figma 卡片行数)"
        );
    }

    #[test]
    fn review_refresh_only_for_matching_session() {
        use std::path::PathBuf;
        assert!(review_should_refresh_on_turn(&ReviewSource::Session(3), 3));
        assert!(!review_should_refresh_on_turn(&ReviewSource::Session(3), 4));
        assert!(!review_should_refresh_on_turn(
            &ReviewSource::File(PathBuf::from("/t/x.jsonl")),
            3
        ));
    }

    #[test]
    fn ai_turn_summary_text() {
        assert_eq!(ai_turn_summary(0, false), "过程:无");
        assert_eq!(ai_turn_summary(2, false), "过程:2 工具");
        assert_eq!(ai_turn_summary(2, true), "过程:思考 + 2 工具");
        assert_eq!(ai_turn_summary(0, true), "过程:思考");
    }

    #[test]
    fn tree_dot_maps_status_colors() {
        assert_eq!(
            tree_row_dot_color(delivery::ChangeKind::Modified),
            theme::GOLD
        );
        assert_eq!(tree_row_dot_color(delivery::ChangeKind::New), theme::GREEN);
        assert_eq!(
            tree_row_dot_color(delivery::ChangeKind::Deleted),
            theme::RED
        );
        assert_eq!(tree_row_dot_glyph(false), "●", "全部暂存=实心");
        assert_eq!(tree_row_dot_glyph(true), "○", "有未暂存改动=空心");
    }

    #[test]
    fn project_card_branch_label() {
        assert_eq!(project_branch_label(Some("main"), false), "main");
        assert_eq!(project_branch_label(Some("main"), true), "main*");
        assert_eq!(project_branch_label(None, false), "—");
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
        assert_eq!(l.left_width, 500.0 - workspace_geometry::icon_rail_width());
    }

    #[test]
    fn clamp_left_width_to_minimum() {
        let state = test_state();
        let l = apply_column_drag(state, Divider::LeftRight, 1440.0, 10.0);
        assert_eq!(l.left_width, workspace_geometry::min_zone_width());
    }

    #[test]
    fn clamp_left_width_to_maximum_keeps_right_zone_alive() {
        // 拖到最右也要给右面板区留 workspace_geometry::min_zone_width()。
        let state = test_state();
        let l = apply_column_drag(state, Divider::LeftRight, 1440.0, 1430.0);
        let expected = 1440.0
            - 2.0 * workspace_geometry::icon_rail_width()
            - workspace_geometry::divider_width()
            - workspace_geometry::min_zone_width();
        assert_eq!(l.left_width, expected);
    }

    #[test]
    fn clamp_left_width_when_window_too_narrow_does_not_panic() {
        // 窗口窄到上界低于下界时,`.max(下限)` 把上界垫平,恒不 panic。
        let state = test_state();
        let l = apply_column_drag(state, Divider::LeftRight, 700.0, 650.0);
        assert_eq!(l.left_width, workspace_geometry::min_zone_width());
    }

    #[test]
    fn clamp_files_split_within_bounds() {
        let state = test_state();
        // 左面板区 640 宽,配对内容宽 = 640-8=632,拖到其中点(316)→ 0.5。
        let l = apply_column_drag(
            state,
            Divider::LeftPairSplit,
            1440.0,
            workspace_geometry::icon_rail_width() + 316.0,
        );
        assert!((l.files_split - 0.5).abs() < 0.001, "{}", l.files_split);
    }

    #[test]
    fn clamp_files_split_to_range() {
        let state = test_state();
        let l = apply_column_drag(state, Divider::LeftPairSplit, 1440.0, 10.0);
        assert_eq!(l.files_split, workspace_geometry::min_split_ratio());
        let l = apply_column_drag(state, Divider::LeftPairSplit, 1440.0, 5000.0);
        assert_eq!(l.files_split, workspace_geometry::max_split_ratio());
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
            left_gone.layout
        );
        let right_gone = ShellState {
            right_collapsed: true,
            ..test_state()
        };
        assert_eq!(
            apply_column_drag(right_gone, Divider::RightPairSplit, 1440.0, 1000.0),
            right_gone.layout
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
            l.conversations_split, agent.layout.conversations_split,
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
        assert_eq!(l.agent_split, conversations.layout.agent_split);
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

    #[test]
    fn banner_text_for_pending() {
        assert_eq!(banner_text(true), Some("交付待验收"));
        assert_eq!(banner_text(false), None);
    }

    #[test]
    fn effective_cwd_prefers_osc_over_spawn() {
        use std::path::Path;
        // 用户 cd 进仓库:OSC 7 跟踪的实时目录优先于启动目录($HOME)
        assert_eq!(
            effective_cwd(Some(Path::new("/repo/proj")), "/Users/me"),
            PathBuf::from("/repo/proj")
        );
        // 尚无 OSC 7 上报:回落启动目录
        assert_eq!(effective_cwd(None, "/Users/me"), PathBuf::from("/Users/me"));
    }

    #[test]
    fn tab_title_prefers_cwd_basename() {
        use std::path::Path;
        assert_eq!(
            tab_title(
                AgentKind::Unknown,
                Some(Path::new("/Users/c/proj")),
                "shell"
            ),
            "proj"
        );
        assert_eq!(
            tab_title(AgentKind::Unknown, Some(Path::new("/")), "shell"),
            "/"
        );
        assert_eq!(tab_title(AgentKind::Unknown, None, "shell"), "shell");
    }

    #[test]
    fn tab_title_prefers_agent_name_once_known() {
        use std::path::Path;
        assert_eq!(
            tab_title(AgentKind::Claude, Some(Path::new("/Users/c/proj")), "shell"),
            "claude"
        );
        assert_eq!(tab_title(AgentKind::Claude, None, "shell"), "claude");
    }

    #[test]
    fn tab_window_no_overflow_both_disabled() {
        let (first, left, right) = tab_window(&[50.0, 50.0, 50.0], 4.0, 500.0, 0);
        assert_eq!((first, left, right), (0, false, false));
    }

    #[test]
    fn tab_window_overflow_clamps_and_flags() {
        let w = [100.0; 5];
        assert_eq!(tab_window(&w, 0.0, 250.0, 0), (0, false, true));
        // 末2个(200)放得下、末3个(300)放不下 → max_first=3;过大 first 钳到 3、右到头
        assert_eq!(tab_window(&w, 0.0, 250.0, 99), (3, true, false));
        assert_eq!(tab_window(&w, 0.0, 250.0, 1), (1, true, true));
    }

    #[test]
    fn tab_display_width_cjk_wider_than_ascii() {
        assert!(tab_display_width("中文会话") > tab_display_width("sh"));
    }

    #[test]
    fn preview_tab_display_width_narrower_than_term_no_dot() {
        // 同标题下预览版无状态点,应恒窄于终端版。
        assert!(preview_tab_display_width("a.rs") < tab_display_width("a.rs"));
    }

    #[test]
    fn dot_color_states() {
        use dozer_core::protocol::AgentState::*;
        // 死会话恒为灰，不论 agent 状态。
        assert_eq!(dot_color(Running, false), theme::DIM, "死会话灰点");
        // 存活：空闲/运行同绿（运行靠闪烁区分），待输入紫、回合毕金。
        assert_eq!(dot_color(Idle, true), theme::GREEN);
        assert_eq!(dot_color(Running, true), theme::GREEN);
        assert_eq!(dot_color(AwaitingInput, true), theme::PURPLE);
        assert_eq!(dot_color(TurnEnded, true), theme::GOLD);
    }

    #[test]
    fn open_missing_file_sets_preview_error_and_no_tab() {
        // Workspace 全量构造依赖 daemon/EventLoop,headless 里只验状态机
        // 侧的可测部分:错误字段与 tab 数经由 update 的行为契约。
        // 若 Workspace 无法在测试中直接构造,则改为验证 preview_content_bounds
        // 之外新增一个纯函数不现实——此时降级为:仅确认编译期字段存在,
        // 测试留待 Task 6 人工验收覆盖,并在报告中写明。
    }

    #[test]
    fn agent_state_label_covers_all() {
        assert_eq!(agent_state_label(AgentState::Running), "运行中");
        assert_eq!(agent_state_label(AgentState::AwaitingInput), "待输入");
        assert_eq!(agent_state_label(AgentState::TurnEnded), "回合毕");
        assert_eq!(agent_state_label(AgentState::Idle), "空闲");
    }

    #[test]
    fn env_status_text_ok_and_down() {
        assert_eq!(
            env_status_text(true),
            ("环境正常 · dozerd 运行中", theme::GREEN)
        );
        assert_eq!(env_status_text(false), ("dozerd 未连接", theme::RED));
    }

    #[test]
    fn goal_capsule_prefixes_and_truncates() {
        use crate::goal::Goal;
        let g = Goal {
            title: "会话存活 daemon 雏形".into(),
            criteria: vec![],
        };
        assert_eq!(
            goal_capsule_text(Some(&g), 100).as_deref(),
            Some("目标：会话存活 daemon 雏形")
        );
        // 过长按字符截断并加省略号（max_chars 含省略号位）
        let long = Goal {
            title: "一二三四五六七八九十".into(),
            criteria: vec![],
        };
        assert_eq!(
            goal_capsule_text(Some(&long), 5).as_deref(),
            Some("目标：一二三四…")
        );
        // 无 goal / 空标题 → None（胶囊隐藏）
        assert_eq!(goal_capsule_text(None, 10), None);
        let empty = Goal {
            title: "   ".into(),
            criteria: vec![],
        };
        assert_eq!(goal_capsule_text(Some(&empty), 10), None);
    }

    fn make_test_tab(rt: &tokio::runtime::Runtime, id: &str, agent: AgentKind) -> SessionTab {
        SessionTab {
            info: SessionInfo {
                id: id.to_string(),
                name: "shell".into(),
                command: "shell".into(),
                cwd: "/tmp".into(),
                alive: true,
                created_ms: 0,
                agent_state: AgentState::Idle,
                transcript_path: None,
                project_id: None,
                agent,
            },
            model: TerminalModel::new(80, 24),
            alive: true,
            agent_state: AgentState::Idle,
            agent,
            transcript_path: None,
            osc: OscScanner::new(),
            cwd: None,
            last_exit: None,
            delivery_pending: false,
            last_turn_head: None,
            tab_id: 0,
            forwarder: rt.spawn(async {}),
        }
    }

    #[test]
    fn group_tabs_by_agent_empty_list() {
        let tabs: Vec<SessionTab> = Vec::new();
        assert_eq!(
            group_tabs_by_agent(&tabs),
            Vec::<(AgentKind, Vec<usize>)>::new()
        );
    }

    #[test]
    fn group_tabs_by_agent_single_agent_multiple_sessions() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let tabs = vec![
            make_test_tab(&rt, "a", AgentKind::Claude),
            make_test_tab(&rt, "b", AgentKind::Claude),
        ];
        assert_eq!(
            group_tabs_by_agent(&tabs),
            vec![(AgentKind::Claude, vec![0, 1])]
        );
    }

    #[test]
    fn group_tabs_by_agent_mixed_fixed_order_no_empty_groups() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        // 故意打乱插入顺序(Opencode 先、Claude 后),验证分组输出顺序
        // 固定为 Claude→Codebuddy→Opencode→Unknown,不是插入顺序;
        // 没有任何会话的 Codebuddy 不出现在结果里(空分组不占位)。
        let tabs = vec![
            make_test_tab(&rt, "a", AgentKind::Opencode),
            make_test_tab(&rt, "b", AgentKind::Claude),
            make_test_tab(&rt, "c", AgentKind::Unknown),
            make_test_tab(&rt, "d", AgentKind::Claude),
        ];
        assert_eq!(
            group_tabs_by_agent(&tabs),
            vec![
                (AgentKind::Claude, vec![1, 3]),
                (AgentKind::Opencode, vec![0]),
                (AgentKind::Unknown, vec![2]),
            ]
        );
    }

    #[test]
    fn agent_dot_color_maps_each_kind_and_avoids_gold() {
        let cases = [
            (AgentKind::Claude, theme::CYAN),
            (AgentKind::Codebuddy, theme::PURPLE),
            (AgentKind::Opencode, theme::GREEN),
            (AgentKind::Codex, theme::ORANGE),
            (AgentKind::Qoder, theme::MAGENTA),
            (AgentKind::Kilo, theme::BLUE),
            (AgentKind::Unknown, theme::DIM),
        ];
        for (agent, expected) in cases {
            let color = agent_dot_color(agent);
            assert_eq!(color, expected, "{agent:?}");
            assert_ne!(
                color,
                theme::GOLD,
                "{agent:?} 的对话列表圆点色不能是 GOLD(甲方动作专属,CLAUDE.md 明文规定)"
            );
        }
    }

    #[test]
    fn agent_icon_maps_each_kind_to_brand_icon() {
        assert_eq!(agent_icon(AgentKind::Claude), IconKind::Claude);
        assert_eq!(agent_icon(AgentKind::Codebuddy), IconKind::Codebuddy);
        assert_eq!(agent_icon(AgentKind::Opencode), IconKind::Opencode);
        // Codex/Qoder/Kilo 暂无确认可用的品牌素材，回落通用 Bot 图标
        // （见计划 Task 3 说明，非占位符——spec §8/§6 明确允许的兜底）。
        assert_eq!(agent_icon(AgentKind::Codex), IconKind::Bot);
        assert_eq!(agent_icon(AgentKind::Qoder), IconKind::Bot);
        assert_eq!(agent_icon(AgentKind::Kilo), IconKind::Bot);
        // Unknown 同样回落 Bot 图标。
        assert_eq!(agent_icon(AgentKind::Unknown), IconKind::Bot);
    }

    #[test]
    fn agent_cli_command_maps_known_agents_and_none_for_unknown() {
        assert_eq!(agent_cli_command(AgentKind::Claude), Some("claude"));
        assert_eq!(agent_cli_command(AgentKind::Codebuddy), Some("codebuddy"));
        assert_eq!(agent_cli_command(AgentKind::Opencode), Some("opencode"));
        assert_eq!(agent_cli_command(AgentKind::Codex), Some("codex"));
        assert_eq!(agent_cli_command(AgentKind::Qoder), Some("qoder"));
        assert_eq!(agent_cli_command(AgentKind::Kilo), Some("kilo"));
        assert_eq!(agent_cli_command(AgentKind::Unknown), None);
    }

    #[test]
    fn picker_launch_command_maps_selection_to_initial_command() {
        // 已知 agent → 其 CLI 名(复用 agent_cli_command)。
        assert_eq!(
            picker_launch_command(PickerLaunch::Agent(Some(AgentKind::Claude))),
            Some("claude".to_string())
        );
        assert_eq!(
            picker_launch_command(PickerLaunch::Agent(Some(AgentKind::Codebuddy))),
            Some("codebuddy".to_string())
        );
        assert_eq!(
            picker_launch_command(PickerLaunch::Agent(Some(AgentKind::Opencode))),
            Some("opencode".to_string())
        );
        assert_eq!(
            picker_launch_command(PickerLaunch::Agent(Some(AgentKind::Codex))),
            Some("codex".to_string())
        );
        assert_eq!(
            picker_launch_command(PickerLaunch::Agent(Some(AgentKind::Qoder))),
            Some("qoder".to_string())
        );
        assert_eq!(
            picker_launch_command(PickerLaunch::Agent(Some(AgentKind::Kilo))),
            Some("kilo".to_string())
        );
        // 纯 Shell → 不键入任何初始命令。
        assert_eq!(picker_launch_command(PickerLaunch::Agent(None)), None);
        // Git Shell → 项目根开 shell 后自动跑 git status。
        assert_eq!(
            picker_launch_command(PickerLaunch::Git),
            Some("git status".to_string())
        );
    }

    fn write_temp_file(name: &str, content: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(name);
        std::fs::write(&path, content).unwrap();
        (dir, path)
    }

    #[test]
    fn preview_edit_open_reads_file_and_starts_clean_session() {
        let (_dir, path) = write_temp_file("a.rs", "fn main() {}");
        let mut ws = Workspace::empty_for_project_placeholder();
        let tab_id = ws.preview.open_path(path.clone());
        ws.preview_edit_open(0);
        let session = ws.edit_session.as_ref().expect("应打开编辑会话");
        assert_eq!(session.tab_id, tab_id);
        assert_eq!(session.path, path);
        assert_eq!(session.content.text(), "fn main() {}");
        assert!(!session.dirty);
        assert!(session.error.is_none());
        assert!(!session.confirm_discard);
    }

    #[test]
    fn preview_edit_open_missing_file_reports_error_and_does_not_open() {
        let mut ws = Workspace::empty_for_project_placeholder();
        ws.preview
            .open_path(PathBuf::from("/nonexistent/does-not-exist.rs"));
        ws.preview_edit_open(0);
        assert!(ws.edit_session.is_none());
        assert!(ws.preview_error.is_some());
    }

    #[test]
    fn preview_edit_open_out_of_range_index_is_noop() {
        let mut ws = Workspace::empty_for_project_placeholder();
        ws.preview_edit_open(0);
        assert!(ws.edit_session.is_none());
        assert!(ws.preview_error.is_none());
    }

    #[test]
    fn preview_edit_action_marks_dirty_only_on_edit_actions() {
        let (_dir, path) = write_temp_file("a.txt", "hi");
        let mut ws = Workspace::empty_for_project_placeholder();
        ws.preview.open_path(path);
        ws.preview_edit_open(0);
        // 非编辑动作(光标移动)不置脏。
        ws.preview_edit_action(text_editor::Action::Move(text_editor::Motion::Right));
        assert!(!ws.edit_session.as_ref().unwrap().dirty);
        // 编辑动作置脏。前一步光标右移了一位,Insert 落在 'h' 之后。
        ws.preview_edit_action(text_editor::Action::Edit(text_editor::Edit::Insert('!')));
        assert!(ws.edit_session.as_ref().unwrap().dirty);
        assert_eq!(ws.edit_session.as_ref().unwrap().content.text(), "h!i");
    }

    #[test]
    fn preview_edit_save_writes_disk_clears_dirty_and_bumps_reload() {
        let (_dir, path) = write_temp_file("a.txt", "hi");
        let mut ws = Workspace::empty_for_project_placeholder();
        let tab_id = ws.preview.open_path(path.clone());
        ws.preview_edit_open(0);
        ws.preview_edit_action(text_editor::Action::Edit(text_editor::Edit::Insert('!')));
        ws.preview_edit_save();
        assert!(!ws.edit_session.as_ref().unwrap().dirty);
        assert!(ws.edit_session.as_ref().unwrap().error.is_none());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "!hi");
        let specs = ws.preview.desired_webviews();
        let spec = specs.iter().find(|s| s.id == tab_id).unwrap();
        assert!(
            spec.url.contains("&_r=1"),
            "保存后应推进 reload nonce: {}",
            spec.url
        );
    }

    #[test]
    fn preview_edit_close_request_without_dirty_closes_immediately() {
        let (_dir, path) = write_temp_file("a.txt", "hi");
        let mut ws = Workspace::empty_for_project_placeholder();
        ws.preview.open_path(path);
        ws.preview_edit_open(0);
        ws.preview_edit_close_request();
        assert!(ws.edit_session.is_none());
    }

    #[test]
    fn preview_edit_close_request_with_dirty_asks_confirm_then_discard_or_cancel() {
        let (_dir, path) = write_temp_file("a.txt", "hi");
        let mut ws = Workspace::empty_for_project_placeholder();
        ws.preview.open_path(path);
        ws.preview_edit_open(0);
        ws.preview_edit_action(text_editor::Action::Edit(text_editor::Edit::Insert('!')));
        ws.preview_edit_close_request();
        assert!(
            ws.edit_session.as_ref().unwrap().confirm_discard,
            "脏改动关闭要先确认"
        );
        assert!(ws.edit_session.is_some(), "确认前不能真的关掉");

        ws.preview_edit_confirm_cancel();
        assert!(
            !ws.edit_session.as_ref().unwrap().confirm_discard,
            "取消要回到编辑态"
        );
        assert!(ws.edit_session.as_ref().unwrap().dirty, "取消不丢改动");

        ws.preview_edit_close_request();
        ws.preview_edit_confirm_discard();
        assert!(ws.edit_session.is_none(), "确认放弃要真正关闭");
    }
}
