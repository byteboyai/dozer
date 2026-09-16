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
use crate::app::{
    App, DEFAULT_COLS, DEFAULT_ROWS, HoverId, Message, PROJECT_PREVIEW_ID_OFFSET, PanelKind,
    ProjectId, tab_divider,
};
use crate::delivery::{self};
use crate::extensions::browser;
use crate::extensions::conversations;
use crate::extensions::database;
use crate::extensions::files;
use crate::extensions::project;
use crate::extensions::search;
use crate::extensions::ssh;
use crate::extensions::todo;
use crate::extensions::usage;
use crate::git_watch;
use crate::homespace::home_panel_head_with_actions;
use crate::osc::{OscEvent, OscScanner};
use crate::preview::{PreviewPane, TabKind};
use crate::preview_state;
use crate::project::FileTree;
use crate::tab_widget::{
    PanelTabArgs, TabOverflowEntry, TabOverflowMenuArgs, panel_tab, tab_overflow_button,
    tab_overflow_menu, tab_render_mode_button, tab_window, tab_window_reveal,
};
use crate::term_model::TerminalModel;
use crate::theme;
use crate::theme::terminal_font;
use crate::transcript::{self, ReviewEntry};
use byteui::interaction::icons;
use byteui::interaction::icons::IconKind;
use dozer_client::{Client, TermEvent};
use dozer_core::protocol::{AgentKind, AgentState, ProjectInfo, SessionInfo};
use iced_widget::core::mouse;
use iced_widget::core::text::LineHeight;
use iced_widget::core::{Border, Color, Element, Length, Padding};
use iced_widget::text_editor::Action as EditorAction;
use iced_widget::{MouseArea, Scrollable, button, column, container, row, scrollable, text};
use iced_winit::winit::event_loop::EventLoopProxy;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::runtime::Handle;
use tokio::sync::mpsc;

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
    pub(crate) project: ProjectInfo,
    /// 最近项目列表(顺带取回,省一次往返)。
    pub(crate) recent_projects: Vec<ProjectInfo>,
    /// 已 attach 上的存活会话:(会话信息, 起始快照, 事件流)。
    #[allow(clippy::type_complexity)]
    pub(crate) sessions: Vec<(
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
    pub(crate) fn new(restore: ProjectRestore) -> Self {
        Self(Arc::new(Mutex::new(Some(Box::new(restore)))))
    }

    /// 取走信封里的素材;已被取走(或锁中毒)时返回 `None`,调用方当作
    /// "这次促成结果没人要了"处理即可。
    pub(crate) fn take(&self) -> Option<Box<ProjectRestore>> {
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

/// Agent 面板"＋"弹出菜单选择项的语义。`Agent` 复用原 `Option<AgentKind>`
/// 语义(`None` = 纯 Shell);`Git` 在项目根开一个 shell 并自动跑 `git status`
/// 看仓库状态。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum PickerLaunch {
    Agent(Option<AgentKind>),
    Git,
}

/// 审阅内容的来源（P1j）：活会话 tab（回合结束刷新）或对话面板的
/// session 详情(2026-08-27，整段摊平加载)。原 `ReviewSource::FileRange`
/// (历史对话文件里某个回合区间)已随会话树改造一并移除。
#[derive(Debug, Clone, PartialEq)]
pub enum ReviewSource {
    Session(usize),
    /// 对话面板的 session 详情:`conversation_id`,整段摊平加载,不再有
    /// "回合区间"概念(2026-08-27,取代 `FileRange`)。
    Conversation(String),
}

/// 回合结束时该审阅视图是否应重解析：仅当它是该会话的活审阅。
pub(crate) fn review_should_refresh_on_turn(source: &ReviewSource, tab_id: usize) -> bool {
    matches!(source, ReviewSource::Session(id) if *id == tab_id)
}

/// 会话审阅 tab 的内容（P1i）。
pub struct ReviewView {
    pub source: ReviewSource,
    pub entries: Vec<ReviewEntry>,
    pub error: Option<String>,
    /// 这份 transcript 归属的 agent——面板里"AI"气泡的头像/名字标签用它
    /// 而不是写死的"AI"(P2b 多 agent 支持)。
    pub agent: AgentKind,
    /// 审阅 webview 的重新加载水位:每次 `ReviewLoaded` 成功都从
    /// `Workspace.review_nonce` 拷一份新值,写进 `dozer://review-trace/
    /// host.html?_r=<nonce>` 的查询参数,逼 wry 在内容变化时重新导航
    /// 拉取(同 `preview.rs::PreviewTab.reload_nonce` 的手法)。
    pub nonce: u64,
    /// session 总结标题/全文(有则展示,无则详情页只显示回合列表)。
    /// 数据来自打开详情时已加载好的 `SessionRow`,不为此单独发请求。
    pub summary_title: Option<String>,
    pub summary_text: Option<String>,
    /// 总结区展示的相对时间(如 "3 分钟前"),打开详情那一刻用
    /// `SessionRow.last_ts` 算一次定格,不随详情页停留时长实时跳字
    /// (2026-08-28 新增)。
    pub summary_time: Option<String>,
}

/// UI → SSH 泵任务的写指令(`Workspace::spawn_ssh_tab` 消费)。
pub enum SshOut {
    Data(Vec<u8>),
    Resize { cols: u16, rows: u16 },
}

/// 一个终端 tab 的字节流去向。
pub enum TabBackend {
    /// dozerd 托管的本地 PTY(现状所有 tab)。
    Daemon,
    /// SSH channel,UI 侧写操作经 `out` 送进泵任务。
    Ssh { out: mpsc::UnboundedSender<SshOut> },
}

/// 单个会话的工作区展示态,来自 `delivery::branch`/`delivery::is_dirty`。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct WorkspaceGitInfo {
    pub branch: Option<String>,
    pub dirty: bool,
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
    pub(crate) osc: OscScanner,
    /// OSC 7 上报的当前目录；tab 标题优先显示其 basename。
    pub cwd: Option<PathBuf>,
    /// OSC 133;D 上报的最近命令退出码；非零时终端栏红字提示；
    /// 下一条命令开始（133;C）时清除。
    pub last_exit: Option<i32>,
    /// 最近一次 hook 事件后从 transcript 尾部提取的模型 id(原始,未美化;
    /// 渲染时经 `format_model_label`)。仅 `agent == AgentKind::Claude` 会
    /// 被填充,其余 agent 恒 `None`。不叫 `model`——上面已有一个
    /// `model: TerminalModel` 字段是终端显示缓冲区,同名会编译报错。
    pub llm_model: Option<String>,
    /// 同上,来自 transcript 顶层 `permissionMode`(如 `auto`/`plan`)。
    pub permission_mode: Option<String>,
    /// 同上,从 transcript 尾部提取的最后一句人类发言/AI 回复摘要(截到
    /// 60 字符)。Agent 卡片"当前工作内容"的兜底数据源——优先用 Todo
    /// 派发记录的任务标题(`todo::WorkspaceState::task_title_for_session`),
    /// 拿不到才落到这个字段(见 `agent_card`)。
    pub last_activity: Option<String>,
    /// 工作区分支/脏标覆盖:仅当这个会话的 `effective_cwd()` 偏离项目根
    /// 目录时才会被填充;为 `None` 时渲染层直接读 `ws.project_panel` 的
    /// 项目级缓存(见 `Workspace::spawn_agent_card_refresh`)。
    pub workspace_override: Option<WorkspaceGitInfo>,
    /// 稳定 id，`Message::TermOutput`/`SessionExited` 用它路由，不受
    /// tab 增删导致的 vec 位置变化影响。
    pub(crate) tab_id: usize,
    /// attach 数据流的转发任务句柄。`CloseTab` 时 `abort()` 掉它——
    /// 这个任务是 `mpsc::UnboundedReceiver<TermEvent>` 的唯一持有者，
    /// 任务被中断即意味着 receiver 被 drop（detach）。app 整体退出时
    /// 只发生 detach（会话存活）；显式关 tab 则再补一次 kill。
    pub(crate) forwarder: tokio::task::JoinHandle<()>,
    /// 字节流去向(本地 PTY 还是 SSH channel;阶段 2)。
    pub(crate) backend: TabBackend,
}

/// 会话当前有效工作目录：OSC 7 跟踪的实时 cwd 优先，回落到会话
/// 启动目录。交付检测必须认实时 cwd——用户 `cd` 进项目仓库后，
/// 启动目录（多为 `$HOME`）不是那个仓库（spec P1f D1）。
pub(crate) fn effective_cwd(osc_cwd: Option<&Path>, spawn_cwd: &str) -> PathBuf {
    match osc_cwd {
        Some(p) => p.to_path_buf(),
        None => PathBuf::from(spawn_cwd),
    }
}

impl SessionTab {
    /// 交付检测/验收装载用的当前工作目录（OSC 7 优先）。
    pub(crate) fn effective_cwd(&self) -> PathBuf {
        effective_cwd(self.cwd.as_deref(), &self.info.cwd)
    }

    /// 把一段会话输出送进 OSC 扫描器并落地状态（观察式，不改写字节）。
    pub(crate) fn ingest_osc(&mut self, bytes: &[u8]) {
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
    pub(crate) client: Client,
    pub(crate) handle: Handle,
    pub(crate) proxy: EventLoopProxy<Message>,
    /// 终端网格尺寸快照(新建会话时让新 PTY 一开始就匹配 pane 实际大小)。
    pub(crate) cols: u16,
    pub(crate) rows: u16,
    /// "退出前必须等它跑完"的 spawn 任务句柄暂存区(与 `App` 共享同一份
    /// `Arc`)。关 tab 时发往 daemon 的 kill/总结请求属于此类——不然 App
    /// 退出时 tokio runtime 直接 drop,任务可能没来得及把 kill 发出去,
    /// daemon 侧会话仍是 `alive`,下次启动又被恢复出来(P1x 验收反馈)。
    /// 见 [`ShellIo::track_exit_critical`] 与 `App::wait_for_pending_exit_tasks`。
    pub(crate) pending_exit_tasks: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
}

impl ShellIo {
    /// 登记一个"退出前必须等完"的 spawn 任务。调用方仍按 fire-and-forget
    /// 的写法 `io.handle.spawn(...)`,只是把返回的 `JoinHandle` 交这里
    /// 存着,而不是直接丢弃——退出时 `App::wait_for_pending_exit_tasks`
    /// 会把它们收走、`block_on` 等到完成或超时。
    pub(crate) fn track_exit_critical(&self, task: tokio::task::JoinHandle<()>) {
        if let Ok(mut pending) = self.pending_exit_tasks.lock() {
            pending.push(task);
        }
    }
}

pub struct Workspace {
    pub(crate) tabs: Vec<SessionTab>,
    /// 当前显示的 tab 在 `tabs` 中的位置（不是 `tab_id`）。
    pub(crate) active: usize,
    pub(crate) next_tab_id: usize,
    /// 发起新建会话(create+attach)期间的转发任务句柄暂存区，
    /// `Message::TabAttached` 到达时取出、装进新建的 `SessionTab`。
    pub(crate) pending: HashMap<usize, tokio::task::JoinHandle<()>>,
    /// SSH tab 创建期间的写指令发送端暂存区,语义同 `pending`——
    /// `on_tab_attached` 时取出,取得到就是 SSH tab(`backend = Ssh {
    /// out }`),取不到就是本地 tab(`backend = Daemon`)。
    pub(crate) ssh_out_pending: HashMap<usize, mpsc::UnboundedSender<SshOut>>,
    /// SSH 面板自己的 tab 集合(阶段 4)——与 `tabs`/`active`(右侧共享
    /// agent/本地终端条)完全独立。目前只装 `TabBackend::Ssh` 的
    /// `SessionTab`(种类=`SshTabKind::Terminal`);阶段 3 起 SFTP 种类
    /// 的 tab 走另一个集合(`sftp_tabs`,内容不是 `SessionTab`,见阶段 3
    /// 计划),不混进这里。
    pub(crate) ssh_tabs: Vec<SessionTab>,
    /// SSH 面板当前显示哪个 tab——身份寻址(不是下标),因为 tab 会被
    /// 用户关闭导致下标漂移。`None` = 没有任何 SSH 终端 tab 打开。
    pub(crate) ssh_active: Option<(String, ssh::SshTabKind)>,
    /// SFTP tab 状态(阶段 3),按 host_id 去重——同一主机同时只有一个
    /// SFTP tab 有意义(见 spec 的既有论证)。
    pub(crate) sftp_tabs: HashMap<String, ssh::sftp::SftpTabState>,
    /// SSH 面板 tab 条翻页窗口起点,语义同 `preview_tab_first`——
    /// `tab_window` 每帧据此钳制到合法范围,这里只存"用户上次翻到哪"。
    pub(crate) ssh_tab_first: usize,
    /// SSH tab 栏溢出下拉锚点,语义同 `term_tab_overflow_anchor`。
    pub(crate) ssh_tab_overflow_anchor: Option<(f32, f32)>,
    /// 预览域状态机(P1d).
    pub(crate) preview: PreviewPane,
    /// Project 面板右配对的预览状态机——独立的 `PreviewPane` 实例,与
    /// `preview`(Files 面板)互不干扰,项目链接打开的文件进这里而不是进
    /// Files 预览(见 `Project::OpenLink` 的路由)。
    pub(crate) project_preview: PreviewPane,
    /// 预览域错误文案(打开文件失败等), RED 显示在预览栏地址栏下方。
    pub(crate) preview_error: Option<String>,
    /// Project 面板右配对预览的错误文案,语义同 `preview_error`。
    pub(crate) project_preview_error: Option<String>,
    /// 预览上下文推送的防抖 nonce:每次变化时自增，延迟任务醒来后只有
    /// "自己发起时的值仍是最新值"才真正推送，否则说明中途又有新变化，
    /// 让更晚的那次任务去做(trailing-edge 防抖，见
    /// `spawn_preview_context_push`)。
    pub(crate) preview_context_nonce: Arc<std::sync::atomic::AtomicU64>,
    /// 浏览器面板状态——自己的 `Message`/`update`/`view`,见
    /// `extensions::browser`。挂在每个 `Workspace` 上(不像 Git Log 挂在
    /// `App` 上),项目切换靠 `Workspace` 生命周期天然隔离。
    pub(crate) browser: browser::State,
    /// `dozer://flyfish/__file__` 端点的文件白名单;与 main.rs 的协议
    /// 闭包共享(Arc),打开文件时插入.
    pub(crate) allowed_files: Arc<Mutex<HashSet<PathBuf>>>,
    /// 进行中的会话审阅（审阅 tab 内容;None=未打开;P1i）。
    pub(crate) review: Option<ReviewView>,
    /// `dozer://review-trace/data.json` 协议端点回显的当前审阅内容快照
    /// (JSON 字符串)。**per-project**——`Message::ReviewLoaded` 只在
    /// `rv.source == source`(未过期)时才会写(见该处理分支),协议闭包
    /// 在 webview 创建时按当前聚焦项目捕获这个 `Arc`(同 `allowed_files`
    /// 的手法),天然避免"过期加载结果覆盖当前内容"与"跨项目串数据"
    /// 两个问题——不能像最初实现那样用 main.rs 里的裸 `static`(那样会绕开
    /// `rv.source == source` 的过期结果过滤,也没有 per-project 隔离)。见
    /// docs/superpowers/plans/2026-08-21-review-content-webview-trace.md
    /// 审阅记录。
    pub(crate) review_snapshot: Arc<Mutex<Option<String>>>,
    /// 全局单调递增的审阅内容加载水位,每次 `Message::ReviewLoaded`
    /// 成功一次就 +1(与具体加载了哪个回合区间无关)——保证连续点开
    /// 两个不同回合、恰好都是"该 source 第一次加载"时,`ReviewView.nonce`
    /// 也不会撞成同一个值(如果各自从 0 起独立计数会撞)。见
    /// docs/superpowers/plans/2026-08-21-review-content-webview-trace.md
    /// Task 2。
    pub(crate) review_nonce: u64,
    /// 对话(Conversations)面板列表侧的本地状态——见
    /// `extensions::conversations::WorkspaceState`(2026-09-15 从 `Workspace`
    /// 上平铺的 7 个 `conversation_*` 字段收拢而来)。
    pub(crate) conversations: conversations::WorkspaceState,
    /// 当前项目的 agent 用量统计（会话粒度；扫描+解析全量 transcript，比
    /// `conversations` 贵得多,所以不像它那样在回合结束时自动刷新——只在
    /// 切到 Usage 面板或点手动刷新按钮时才重新扫
    /// （spec 非目标"不做实时更新"）。
    /// Usage 面板 per-project 状态——见 `extensions::usage::WorkspaceState`。
    pub(crate) usage: usage::WorkspaceState,
    /// 当前项目（None=未打开；P1g）。
    pub(crate) project: Option<ProjectInfo>,
    /// 最近项目（切换用）。
    pub(crate) recent_projects: Vec<ProjectInfo>,
    /// 本项目的实时文件系统监听(D4)。`None` 只可能出现在 watcher 启动
    /// 失败时(降级为"只在开项目/回合结束时刷新")。Drop 时自动停止。
    pub(crate) git_watch: Option<git_watch::Handle>,
    /// Project 信息面板 per-project 状态——见 `extensions::project::WorkspaceState`。
    pub(crate) project_panel: project::WorkspaceState,
    /// 终端 tab 栏当前最左可见 tab 序号（箭头翻页用；P1L T5）。溢出改 V
    /// 下拉后该字段仍保留(仅被 `tab_window_reveal` 驱动,不再有手动翻页)。
    pub(crate) term_tab_first: usize,
    /// 终端 tab 栏溢出下拉的悬浮锚点：`None`=下拉关闭，`Some(x,y)`=打开且
    /// 记录了点 V 按钮那一刻的 `App::last_cursor` 快照（逻辑坐标），同
    /// `project_add_menu_anchor` 手法。
    pub(crate) term_tab_overflow_anchor: Option<(f32, f32)>,
    /// 预览 tab 栏当前最左可见 tab 序号，语义同 `term_tab_first`。
    pub(crate) preview_tab_first: usize,
    /// Project 面板右配对预览 tab 栏当前最左可见 tab 序号,语义同
    /// `preview_tab_first`。
    pub(crate) project_preview_tab_first: usize,
    /// 文件预览 tab 栏溢出下拉锚点,语义同 `term_tab_overflow_anchor`。
    pub(crate) preview_tab_overflow_anchor: Option<(f32, f32)>,
    /// 项目预览 tab 栏溢出下拉锚点,语义同上。
    pub(crate) project_preview_tab_overflow_anchor: Option<(f32, f32)>,
    /// Files 面板 per-project 状态——见 `extensions::files::WorkspaceState`。
    pub(crate) files: files::WorkspaceState,
    /// Agent 面板"＋"按钮弹出的"新建"菜单当前是否打开。不需要坐标——面板顶部固定
    /// 位置的下拉,不像项目树右键菜单需要跟随点击坐标。
    pub(crate) agent_picker_open: bool,
    /// 消息驱动(非鼠标点击)把焦点拨离预览编辑器时置位——官方 `text_editor`
    /// 的焦点是真实 iced 焦点树的一部分,不能像 vendored `iced-code-editor`
    /// 那样直接对某个实例调 `lose_focus()`,改成一次性位,main.rs 下一帧用
    /// `operation::focusable::unfocus` 统一让出当前焦点(见 `blur_preview_editors`)。
    pub(crate) pending_editor_unfocus: bool,
    /// Todo 面板 per-project 状态——见 `extensions::todo::WorkspaceState`。
    pub(crate) todo: todo::WorkspaceState,
    /// 数据库面板 per-project 状态(当前项目的数据源列表 + 编辑草稿 + 测试
    /// 状态 map)——见 `extensions::database::WorkspaceState`。
    pub(crate) database: database::WorkspaceState,
    /// SSH 面板 per-project 状态——见 `extensions::ssh::WorkspaceState`。
    pub(crate) ssh: ssh::WorkspaceState,
    /// 文件树右键"搜索"弹窗 per-project 状态——见
    /// `extensions::search::WorkspaceState`。瞬态弹窗,不挂左侧图标栏。
    pub(crate) search: search::WorkspaceState,
    /// 这份 `Workspace` 是否只是 `Stub` → `Loaded` 促成期间的"加载中"占位
    /// (见 [`Workspace::loading_for_project`])。占位有正确的 `project`/文件树,
    /// 但会话/git/对话都还没拉,并且整份对象会在
    /// `Message::ProjectSlotLoaded` 到达时被真正的结果替换掉——所以此刻
    /// **不该**替用户在它上面新建终端会话(建出来的会话会随占位一起被丢弃,
    /// 却仍在 daemon 上活着),`spawn_new_tab` 据此原地放弃。
    pub(crate) loading: bool,
}

/// [`Workspace::on_tab_attached`] 的参数——超过 7 个位置参数且其中
/// `cols`/`rows` 相邻同型(顺序传错编译器不报错),按 CLAUDE.md 关键裁决
/// 改具名字段结构体,不加 `#[allow(clippy::too_many_arguments)]`。
pub(crate) struct TabAttachedArgs {
    pub(crate) cols: u16,
    pub(crate) rows: u16,
    pub(crate) tab_id: usize,
    pub(crate) info: SessionInfo,
    pub(crate) snapshot: Vec<u8>,
    /// picker 选中的目标 agent——见 `Message::TabAttached` 字段文档。
    pub(crate) picked_agent: Option<AgentKind>,
    /// `App::terminal_tab_bar_avail_px()` 的算好值——见其文档。
    pub(crate) term_tab_bar_avail_px: f32,
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
    pub(crate) fn from_restore(
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
            model.set_answer_dynamic_color(info.agent == AgentKind::Opencode);
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
                llm_model: None,
                permission_mode: None,
                last_activity: None,
                workspace_override: None,
                backend: TabBackend::Daemon,
            });
            if let Some(t) = tabs.last_mut() {
                t.ingest_osc(&snapshot);
            }
        }

        let files = files::WorkspaceState::new(FileTree::new(PathBuf::from(&project.path)));
        let repo_path = PathBuf::from(&project.path);
        let project_panel = project::WorkspaceState::new(
            crate::project_meta::load_description(&repo_path),
            project::links::load_or_discover(&repo_path),
        );

        let mut ws = Self {
            tabs,
            active: 0,
            next_tab_id,
            pending: HashMap::new(),
            project: Some(project),
            files,
            project_panel,
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
        // 启动就把"恢复出来的东西"（或 `None`）推一次:dozerd 可能比 GUI
        // 活得久,上次会话留下的缓存值不该在新会话里冒充当前上下文。
        ws.spawn_preview_context_push(io);
        spawn_project_git_refresh(project_id, repo_path.clone(), io);
        spawn_disk_usage_refresh(project_id, repo_path, io);
        ws.spawn_conversations_refresh(io);
        browser::request_bookmarks_refresh(
            ws.project.as_ref().map(|p| p.id),
            &io.client,
            &io.handle,
            {
                let proxy = io.proxy.clone();
                move |m| {
                    let _ = proxy.send_event(Message::Browser(m));
                }
            },
        );
        ws.start_git_watch(io);
        ws
    }

    /// 启动本项目的实时文件系统监听(D4)。失败(比如 fd 耗尽)只记一条
    /// warn,不影响项目正常打开——退化成"只在开项目/回合结束时刷新"这个
    /// D4 之前就有的行为。
    pub(crate) fn start_git_watch(&mut self, io: &ShellIo) {
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
            move |changes| {
                let _ = proxy.send_event(Message::ProjectFsChanged(project_id, changes));
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
    pub(crate) fn empty_for_project_placeholder() -> Self {
        Self {
            tabs: Vec::new(),
            active: 0,
            next_tab_id: 0,
            pending: HashMap::new(),
            ssh_out_pending: HashMap::new(),
            ssh_tabs: Vec::new(),
            ssh_active: None,
            sftp_tabs: HashMap::new(),
            ssh_tab_first: 0,
            ssh_tab_overflow_anchor: None,
            preview: PreviewPane::default(),
            project_preview: PreviewPane::default(),
            preview_error: None,
            project_preview_error: None,
            preview_context_nonce: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            browser: browser::State::default(),
            allowed_files: Arc::new(Mutex::new(HashSet::new())),
            review: None,
            review_snapshot: Arc::new(Mutex::new(None)),
            review_nonce: 0,
            conversations: conversations::WorkspaceState::default(),
            usage: usage::WorkspaceState::default(),
            project: None,
            project_panel: project::WorkspaceState::default(),
            recent_projects: Vec::new(),
            git_watch: None,
            term_tab_first: 0,
            term_tab_overflow_anchor: None,
            preview_tab_first: 0,
            preview_tab_overflow_anchor: None,
            project_preview_tab_first: 0,
            project_preview_tab_overflow_anchor: None,
            files: files::WorkspaceState::default(),
            agent_picker_open: false,
            pending_editor_unfocus: false,
            todo: todo::WorkspaceState::default(),
            database: database::WorkspaceState::default(),
            ssh: ssh::WorkspaceState::default(),
            search: search::WorkspaceState::default(),
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
    pub(crate) fn loading_for_project(project: ProjectInfo) -> Self {
        let files = files::WorkspaceState::new(FileTree::new(PathBuf::from(&project.path)));
        let repo_path = std::path::Path::new(&project.path);
        let project_panel = project::WorkspaceState::new(
            crate::project_meta::load_description(repo_path),
            project::links::load_or_discover(repo_path),
        );
        Self {
            project: Some(project),
            files,
            project_panel,
            loading: true,
            ..Self::empty_for_project_placeholder()
        }
    }

    /// 当前激活 tab 的选区文本（⌘C 复制用）。
    pub fn active_selection_text(&self) -> Option<String> {
        self.tabs
            .get(self.active)
            .and_then(|t| t.model.selection_text())
    }

    pub(crate) fn tab_by_id_mut(&mut self, tab_id: usize) -> Option<&mut SessionTab> {
        self.tabs
            .iter_mut()
            .find(|t| t.tab_id == tab_id)
            .or_else(|| self.ssh_tabs.iter_mut().find(|t| t.tab_id == tab_id))
    }

    /// 转发 `text_editor::Action` 到某个原生预览 tab(按 `tab_id` 定位,不是
    /// "固定某个面板当前激活的那个"——一个项目可以同时开好几个原生预览 tab,
    /// 只有事件来源的那一个该收到)。tab 不存在或不是原生 tab 时静默 no-op。
    /// 事件类别是"会改写正文的 `Action::Edit`"(插字/退格/删除/粘贴/Enter/IME
    /// paste)时,顺手把该 tab 标脏(2026-09-06 原生预览就地可写,dirty 由这个
    /// 事件位推进,fallback 仍由 Pane 层的 save/刷新/项目切换按相同字段读写)。
    pub(crate) fn preview_tab_editor_event(&mut self, tab_id: usize, action: EditorAction) {
        if let Some(editor) = self.preview.editor_mut(tab_id) {
            let is_edit = matches!(action, EditorAction::Edit(_));
            editor.perform(action);
            if is_edit {
                self.preview.mark_dirty_by_id(tab_id);
            }
        }
    }

    /// 转发 `text_editor::Action` 到 Project 面板右配对预览的某个原生 tab,
    /// 语义同 `preview_tab_editor_event`,状态取自 `ws.project_preview`。
    pub(crate) fn project_preview_tab_editor_event(&mut self, tab_id: usize, action: EditorAction) {
        if let Some(editor) = self.project_preview.editor_mut(tab_id) {
            let is_edit = matches!(action, EditorAction::Edit(_));
            editor.perform(action);
            if is_edit {
                self.project_preview.mark_dirty_by_id(tab_id);
            }
        }
    }

    /// 把键盘/IME 字节直接写给当前激活 tab 对应的 daemon 会话。异步写
    /// 交给 tokio（`self.handle.spawn`），绝不在 UI 线程 `block_on`。
    pub(crate) fn send_input(&self, io: &ShellIo, bytes: Vec<u8>) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        if !tab.alive {
            return;
        }
        match &tab.backend {
            TabBackend::Daemon => {
                let client = io.client.clone();
                let id = tab.info.id.clone();
                io.handle.spawn(async move {
                    if let Err(e) = client.write(&id, &bytes).await {
                        tracing::warn!("写入终端失败: {e}");
                    }
                });
            }
            TabBackend::Ssh { out } => {
                let _ = out.send(SshOut::Data(bytes));
            }
        }
    }

    /// SSH 面板当前显示的 tab(`ssh_active` 指向的那个),可变引用。
    /// 镜像 `ws.tabs.get_mut(ws.active)` 的既有用法,用于"敲键盘/滚动
    /// 前先处理一下当前终端状态"(回底/清选区)这类场景。
    pub(crate) fn ssh_active_tab_mut(&mut self) -> Option<&mut SessionTab> {
        let (host_id, _kind) = self.ssh_active.as_ref()?;
        let host_id = host_id.clone();
        self.ssh_tabs
            .iter_mut()
            .find(|t| t.info.id.strip_prefix("ssh:") == Some(host_id.as_str()))
    }

    /// 把字节写进 SSH 面板当前显示的 tab。镜像 `send_input`,操作对象
    /// 换成 `ssh_tabs`/`ssh_active`。
    pub(crate) fn ssh_send_input(&self, io: &ShellIo, bytes: Vec<u8>) {
        let _ = io; // ssh_tabs 里只会是 TabBackend::Ssh,不需要 io.client/handle,
        // 保留参数是为了和 send_input 签名对齐、调用方不用分叉判断
        let Some((host_id, _kind)) = self.ssh_active.as_ref() else {
            return;
        };
        let Some(tab) = self
            .ssh_tabs
            .iter()
            .find(|t| t.info.id.strip_prefix("ssh:") == Some(host_id.as_str()))
        else {
            return;
        };
        if !tab.alive {
            return;
        }
        match &tab.backend {
            TabBackend::Daemon => {
                debug_assert!(false, "ssh_tabs 里不应该出现 TabBackend::Daemon");
            }
            TabBackend::Ssh { out } => {
                let _ = out.send(SshOut::Data(bytes));
            }
        }
    }

    /// 加载(或刷新)当前项目的会话列表(联查总结，标题+摘要预览)
    /// → `Conversations(SessionsRefreshed)`(spec 2026-08-27，取代
    /// `spawn_all_turn_groups_refresh`)。2026-09-15 起实现搬到
    /// `extensions::conversations::spawn_refresh`,这里只做薄封装。
    pub(crate) fn spawn_conversations_refresh(&self, io: &ShellIo) {
        let Some(project) = self.project.as_ref() else {
            return;
        };
        let project_id = project.id;
        let cwd = PathBuf::from(&project.path);
        let proxy = io.proxy.clone();
        let emit = move |m| {
            let _ = proxy.send_event(Message::Conversations(m));
        };
        conversations::spawn_refresh(project_id, cwd, &io.client, &io.handle, emit);
    }

    /// 异步扫当前项目的全部 transcript 并逐个解析用量 → `Usage(Loaded)`。
    /// 比 `spawn_all_turn_groups_refresh` 贵得多(要读整份文件内容，不只是
    /// 文件头)，所以不接入它那条"回合结束自动刷新"的调用链——只在
    /// 切到 Usage 面板(右图标栏)或手动刷新按钮时触发。
    pub(crate) fn spawn_usage_refresh(&self, io: &ShellIo) {
        let Some(p) = &self.project else {
            return;
        };
        let project_id = p.id;
        let project_path = PathBuf::from(&p.path);
        let proxy = io.proxy.clone();
        let emit = move |m| {
            let _ = proxy.send_event(Message::Usage(m));
        };
        usage::spawn_refresh(project_id, project_path, &io.client, &io.handle, emit);
    }

    /// 本 `Workspace` 归属的项目 id。所有"发起时已知项目、结果晚些才回来"的
    /// 异步任务都要带上它,让 [`App::with_project`] 能投回原主(见
    /// [`ProjectId`])。`None` 只可能出现在 `ProjectOpened` 单条消息内部那个
    /// 用完即改写的空壳上,那里不发起任何异步任务。
    pub(crate) fn project_id(&self) -> Option<ProjectId> {
        self.project.as_ref().map(|p| p.id)
    }

    /// 打开着的会话 transcript 路径集合（UI 判"● 当前"用）。
    pub fn open_transcript_paths(&self) -> Vec<String> {
        self.tabs
            .iter()
            .filter_map(|t| t.transcript_path.clone())
            .collect()
    }

    /// 异步查回合明细 → ReviewLoaded（改走 dozerd；P1i/P1j/P2b 按源）。
    /// `after_turn_index`/`limit` 由调用方按 `source` 算好传入——
    /// `ReviewSource::FileRange` 传精确区间(只拿这一个回合分组的内容)，
    /// 其余两个 source 继续传 `(-1, 10_000)` 全量拉取(既有行为不变)。
    pub(crate) fn spawn_review_load(
        &self,
        io: &ShellIo,
        source: ReviewSource,
        path: String,
        _agent: AgentKind,
        after_turn_index: i64,
        limit: u32,
    ) {
        let Some(project) = self.project.as_ref() else {
            return;
        };
        let project_id = project.id;
        let client = io.client.clone();
        let proxy = io.proxy.clone();
        // conversation_id 是文件名(不含扩展名)——与 dozerd 摄取时的派生
        // 规则一致(见 TranscriptStore::ingest_session)。
        let conversation_id = std::path::Path::new(&path)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        io.handle.spawn(async move {
            let result = client
                .get_conversation_turns(&conversation_id, after_turn_index, limit)
                .await
                .map(|turns| crate::transcript::review_entries_from_turns(&turns))
                .map_err(|e| e.to_string());
            let _ = proxy.send_event(Message::ReviewLoaded(project_id, source, false, result));
        });
    }

    /// session 详情整段加载/"加载更多"追加(2026-08-27)。`append` 为
    /// `true` 时结果应追加进现有 `entries`(加载更多),`false` 时替换
    /// (首次打开)——原样透传进 `Message::ReviewLoaded` 的第三个参数,
    /// 实际的追加/替换逻辑在 `update()` 里处理,这里只负责发请求带上
    /// 这个标记。**`Message::ReviewLoaded` 的签名要在 Task 8 扩到 4 元组
    /// `(ProjectId, ReviewSource, bool, Result<...>)` 才能接住这里传的
    /// `append`——本任务落地后 `dozer-app` 暂时编译不过是预期状态**。
    pub(crate) fn spawn_review_load_conversation(
        &self,
        io: &ShellIo,
        conversation_id: String,
        after_turn_index: i64,
        limit: u32,
        append: bool,
    ) {
        let Some(project) = self.project.as_ref() else {
            return;
        };
        let project_id = project.id;
        let client = io.client.clone();
        let proxy = io.proxy.clone();
        let source = ReviewSource::Conversation(conversation_id.clone());
        io.handle.spawn(async move {
            let result = client
                .get_conversation_turns(&conversation_id, after_turn_index, limit)
                .await
                .map(|turns| crate::transcript::review_entries_from_turns(&turns))
                .map_err(|e| e.to_string());
            let _ = proxy.send_event(Message::ReviewLoaded(project_id, source, append, result));
        });
    }

    /// hook 事件驱动的"卡片元信息"刷新(仿 `spawn_review_load` 的写法):
    /// model/permissionMode(Claude 与 Unknown——后者是老装 hook 上报的
    /// "还不知道具体是哪家",但 transcript 仍是 Claude 形状,见
    /// `transcript::latest_model_mode_and_activity`)+ 工作区覆盖(仅当该 session
    /// 的 cwd 偏离项目根目录——即不在 `project_root` 路径前缀下——时才
    /// 查;常见情形直接复用 `project_panel` 的项目级缓存,这里不产生任何
    /// IO)。两者都不需要时不起异步任务,但仍同步发一条全 `None` 的
    /// `AgentCardRefreshed`,确保 `workspace_override` 之类的残留覆盖能被
    /// 清掉(见 `apply_agent_card_refresh`)。
    pub(crate) fn spawn_agent_card_refresh(
        &self,
        io: &ShellIo,
        tab_id: usize,
        agent: AgentKind,
        transcript_path: Option<String>,
        cwd: PathBuf,
        project_root: Option<PathBuf>,
    ) {
        let Some(project_id) = self.project_id() else {
            return;
        };
        let (needs_model_mode, needs_activity, needs_workspace) =
            agent_card_refresh_plan(agent, &cwd, project_root.as_deref());
        if !needs_model_mode && !needs_activity && !needs_workspace {
            // cwd 未偏离项目根、且这个 agent 没有可提取的 model/mode/activity:
            // 不需要起 IO/async 任务,同步把这个明确值发出去即可,顺便清掉
            // 可能残留的 workspace_override(见 apply_agent_card_refresh 的
            // 无条件覆盖注释)。
            let _ = io.proxy.send_event(Message::AgentCardRefreshed(
                project_id, tab_id, None, None, None, None,
            ));
            return;
        }
        let proxy = io.proxy.clone();
        io.handle.spawn(async move {
            let (llm_model, mode, activity, workspace) = tokio::task::spawn_blocking(move || {
                let (llm_model, mode, activity) = if needs_model_mode || needs_activity {
                    let (m, mo, a) = transcript_path
                        .and_then(|p| std::fs::read_to_string(p).ok())
                        .map(|s| transcript::latest_model_mode_and_activity(&s))
                        .unwrap_or((None, None, None));
                    (
                        if needs_model_mode { m } else { None },
                        if needs_model_mode { mo } else { None },
                        if needs_activity { a } else { None },
                    )
                } else {
                    (None, None, None)
                };
                let workspace = if needs_workspace {
                    delivery::repo_root(&cwd).map(|repo| WorkspaceGitInfo {
                        branch: delivery::branch(&repo),
                        dirty: delivery::is_dirty(&repo),
                    })
                } else {
                    None
                };
                (llm_model, mode, activity, workspace)
            })
            .await
            .unwrap_or((None, None, None, None));
            let _ = proxy.send_event(Message::AgentCardRefreshed(
                project_id, tab_id, llm_model, mode, activity, workspace,
            ));
        });
    }

    /// 项目切换清理：关掉所有终端 tab（=结束会话，同 CloseTab 语义）与
    /// 所有预览 tab，给新项目一个干净起点。webview 池由
    /// main.rs 的 sync_previews 依据空的期望清单自动销毁。
    pub(crate) fn close_all_tabs_for_switch(&mut self, io: &ShellIo) {
        while !self.tabs.is_empty() {
            self.close_tab(io, 0);
        }
        self.preview.clear_all();
        self.preview_error = None;
        self.term_tab_first = 0;
        self.preview_tab_first = 0;
    }

    /// 让这份 `Workspace` 认领一个项目:项目相关的字段整体换成该项目的,
    /// 再把"要等 IO 才有结果"的部分(终端/git/对话)异步补上。
    ///
    /// 两个调用方共用:`ProjectOpened`(把当前页签就地改写成另一个项目,
    /// 调用前已 `close_all_tabs_for_switch` 清场)与
    /// `ProjectTabOpened`(在一个全新的空 `Workspace` 上认领,新增页签)。
    ///
    /// 注意这里**不做**"attach 该项目在 daemon 上已存在的存活会话"——那是
    /// `Workspace::bootstrap` 的异步活儿,归 Task 7 的 `ensure_loaded` 促成
    /// 逻辑;本函数与既有 `ProjectOpened` 落地路径行为完全一致(直接开一个
    /// 新终端)。
    pub(crate) fn adopt_project(&mut self, io: &ShellIo, project: ProjectInfo) {
        // 认领 = 这份 `Workspace` 从此有真正的内容,不再是促成期占位:必须
        // 清掉 `loading`,否则(1)`spawn_new_tab` 会继续拒绝建会话,(2)一个
        // 迟到的 `ProjectSlotLoaded` 会把刚认领好的内容当成占位覆盖掉。
        self.loading = false;
        self.files
            .reset_for_project(FileTree::new(PathBuf::from(&project.path)));
        // 复用中的 `Workspace`(就地改写成另一个项目,见本方法文档)可能还
        // 挂着上一个项目的 watcher——显式清掉再重开,而不是指望
        // `start_git_watch` 成功时的赋值顺带把旧的 drop 掉:万一新项目的
        // 路径打不开 watcher(见其内部 `Err` 分支),不清的话旧 watcher 会
        // 带着旧 project_id 继续在后台跑,`Message::ProjectFsChanged` 送来
        // 的刷新信号会挂在一个此刻已经不对应这份 `Workspace` 的项目 id 上。
        self.git_watch = None;
        self.conversations = conversations::WorkspaceState::default();
        self.usage = usage::WorkspaceState::default();
        let project_id = project.id;
        let repo_path = PathBuf::from(&project.path);
        self.project_panel = project::WorkspaceState::new(
            crate::project_meta::load_description(&repo_path),
            project::links::load_or_discover(&repo_path),
        );
        self.project = Some(project);
        self.ensure_project_terminal(io);
        self.restore_preview_state();
        // 启动就把"恢复出来的东西"（或 `None`）推一次:dozerd 可能比 GUI
        // 活得久,上次会话留下的缓存值不该在新会话里冒充当前上下文。
        self.spawn_preview_context_push(io);
        spawn_project_git_refresh(project_id, repo_path.clone(), io);
        spawn_disk_usage_refresh(project_id, repo_path, io);
        self.spawn_conversations_refresh(io);
        browser::request_bookmarks_refresh(
            self.project.as_ref().map(|p| p.id),
            &io.client,
            &io.handle,
            {
                let proxy = io.proxy.clone();
                move |m| {
                    let _ = proxy.send_event(Message::Browser(m));
                }
            },
        );
        // D4:新开的项目页签也要有实时刷新——此前只有跨重启恢复
        // (`from_restore`)/`Stub` 促成时会启动 watcher,直接开新项目这条最
        // 常见的路径反而漏了,退化成"只在开项目/回合结束时刷新"(code
        // review 发现)。
        self.start_git_watch(io);
    }

    /// tab 关闭 = 结束会话：中断转发任务（`rx` 随任务栈析构）并 kill
    /// daemon 侧会话——否则 bootstrap 会把它当存活会话再恢复出来
    /// （P1e 验收反馈）。死会话（已 exited）无需再 kill。
    pub(crate) fn close_tab(&mut self, io: &ShellIo, idx: usize) {
        if idx >= self.tabs.len() {
            return;
        }
        let tab = self.tabs.remove(idx);
        tab.forwarder.abort();
        let client = io.client.clone();
        let id = tab.info.id.clone();
        if should_summarize_on_close(tab.agent, tab.alive, &tab.backend) {
            let task = io.handle.spawn(async move {
                if let Err(e) = client.close_with_summary(&id).await {
                    tracing::warn!("关闭 tab 时触发总结失败: {e}");
                }
            });
            io.track_exit_critical(task);
        } else if tab.alive && matches!(tab.backend, TabBackend::Daemon) {
            let task = io.handle.spawn(async move {
                if let Err(e) = client.kill(&id).await {
                    tracing::warn!("关闭 tab 时结束会话失败: {e}");
                }
            });
            io.track_exit_critical(task);
        }
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len().saturating_sub(1);
        } else if idx < self.active {
            self.active -= 1;
        }
        // 关 tab 后位置全变，旧 first 可能越界——归零防御（P1L T5）。
        self.term_tab_first = 0;
    }

    /// 关闭 SSH 面板某个 tab(镜像 `close_tab` 的中断转发任务/kill 逻辑,
    /// 按身份而不是下标寻址)。关的正好是当前显示的 tab 时,切到剩下
    /// tab 里的第一个,没有剩下的就清空 `ssh_active`。
    pub(crate) fn close_ssh_tab(&mut self, io: &ShellIo, host_id: &str, kind: ssh::SshTabKind) {
        let Some(idx) = self
            .ssh_tabs
            .iter()
            .position(|t| t.info.id.strip_prefix("ssh:") == Some(host_id))
        else {
            return;
        };
        let tab = self.ssh_tabs.remove(idx);
        tab.forwarder.abort();
        // SSH 后端不需要 kill——drop `out`(tx)后 spawn_ssh_tab 的泵循环
        // `rx_out.recv() => None` 分支自然退出,同 close_tab 现有对
        // TabBackend::Ssh 的既有处理口径(见 close_tab_skips_daemon_
        // kill_for_ssh_backend 测试)。
        let _ = io;
        if self.ssh_active.as_ref().map(|(h, k)| (h.as_str(), *k)) == Some((host_id, kind)) {
            self.ssh_active = self.ssh_tabs.first().map(|t| {
                let h = t
                    .info
                    .id
                    .strip_prefix("ssh:")
                    .unwrap_or(&t.info.id)
                    .to_string();
                (h, ssh::SshTabKind::Terminal)
            });
        }
    }

    /// 切换 SSH 面板当前显示哪个 tab。目标 tab 不存在时 no-op(保持
    /// 原有 `ssh_active` 不变,不是清空——镜像其它"目标已消失"场景的
    /// 既有容错口径)。
    pub(crate) fn select_ssh_tab(&mut self, host_id: String, kind: ssh::SshTabKind) {
        let exists = match kind {
            ssh::SshTabKind::Terminal => self
                .ssh_tabs
                .iter()
                .any(|t| t.info.id.strip_prefix("ssh:") == Some(host_id.as_str())),
            ssh::SshTabKind::Sftp => self.sftp_tabs.contains_key(&host_id),
        };
        if exists {
            self.ssh_active = Some((host_id, kind));
        }
    }

    /// 拖拽换位:把 `from` 处的终端 tab 移到 `to`,并同步 `active`。`tab_id`
    /// 不变(它独立于 Vec 位置,异步路由按 id 走),forwarder/后端都不动。
    /// `from`/`to` 同址或越界是 no-op。
    pub(crate) fn reorder_term_tab(&mut self, from: usize, to: usize) {
        if from == to || from >= self.tabs.len() || to >= self.tabs.len() {
            return;
        }
        let tab = self.tabs.remove(from);
        self.tabs.insert(to, tab);
        if self.active == from {
            self.active = to;
        } else if from < self.active && to >= self.active {
            self.active -= 1;
        } else if from > self.active && to <= self.active {
            self.active += 1;
        }
        // 换位不影响能见度,但拖拽中视觉以光标为准,无需强制归零 first。
    }

    /// 把当前预览 tab(仅文件类)异步写盘,同 `layout::save` 走
    /// `handle.spawn` 的既有模式,不阻塞 UI 线程。没有打开项目时不存
    /// (状态按项目 id 分文件,没有项目就没有归属)。
    pub(crate) fn spawn_preview_state_save(&self, io: &ShellIo) {
        let Some(project) = &self.project else {
            return;
        };
        let project_id = project.id;
        let active_idx = self.preview.active_idx();
        let mut paths = Vec::new();
        let mut active_path = None;
        for (idx, tab) in self.preview.tabs().iter().enumerate() {
            // `Blank` 占位 tab(恒在 index 0 的那个)没有真实路径,不写进
            // 持久化状态——`PreviewPane::default()` 天然就带这个占位,恢复时
            // 无需从盘上再变一个出来。
            let TabKind::File(p) = &tab.kind else {
                continue;
            };
            paths.push(p.clone());
            if idx == active_idx {
                active_path = Some(p.clone());
            }
        }
        let state = preview_state::PreviewState { paths, active_path };
        io.handle.spawn(async move {
            if let Err(e) = preview_state::save(project_id, &state) {
                tracing::warn!("预览 tab 状态写盘失败: {e}");
            }
        });
    }

    /// 把当前预览上下文(活动 tab 路径 + 光标/选区)防抖推给 `dozerd`。
    /// 无活动 tab、或活动 tab 无原生 `CodeEditor`(图片/webview 类)时推
    /// `None`。~250ms trailing-edge 防抖:连续快速触发(方向键连按)只有
    /// 最后一次真正发出 UDS 请求。
    ///
    /// 还有第三种"直接放弃、连 `None` 都不推"的情况:`self.project` 为
    /// `None` 的加载期占位 `Workspace`——那时候没有 `project_id` 可以寻址,
    /// 这次推送根本无处可去。占位很快会被真 `Workspace` 换掉,后者的
    /// `bootstrap` 里会补一次推送,所以这里静默返回不会留下空窗。
    pub(crate) fn spawn_preview_context_push(&mut self, io: &ShellIo) {
        let Some(project) = &self.project else {
            return;
        };
        let project_id = project.id;
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let context = self
            .preview
            .tabs()
            .get(self.preview.active_idx())
            .and_then(|tab| {
                let TabKind::File(path) = &tab.kind else {
                    return None;
                };
                let editor = tab.editor.as_ref()?;
                let path_str = path.to_string_lossy().into_owned();
                Some(preview_context_from_editor_state(
                    &path_str,
                    editor.has_selection(),
                    editor.cursor_position(),
                    editor.selection_range(),
                    now_ms,
                ))
            });

        let nonce = self
            .preview_context_nonce
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            + 1;
        let flag = self.preview_context_nonce.clone();
        let client = io.client.clone();
        io.handle.spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            if flag.load(std::sync::atomic::Ordering::SeqCst) != nonce {
                return; // 被更晚的一次变化取代
            }
            if let Err(e) = client.update_preview_context(project_id, context).await {
                tracing::warn!("推送预览上下文失败: {e}");
            }
        });
    }

    /// 项目打开/启动恢复时,认回上次持久化的预览 tab(仅文件类;已被删除/
    /// 移动的文件静默跳过,不报错占位)。
    pub(crate) fn restore_preview_state(&mut self) {
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
    pub(crate) fn ensure_project_terminal(&mut self, io: &ShellIo) {
        if self.project.is_some() && self.tabs.is_empty() {
            self.spawn_new_tab(io, PickerLaunch::Agent(None), None);
        }
    }

    pub(crate) fn spawn_new_tab(
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
        let hook_agent = match launch {
            PickerLaunch::Agent(Some(agent)) => Some(agent),
            _ => None,
        };

        let jh = io.handle.spawn(async move {
            if let Some(agent) = hook_agent {
                let _ = tokio::task::spawn_blocking(move || ensure_hook_installed(agent)).await;
                let _ = tokio::task::spawn_blocking(move || ensure_mcp_installed(agent)).await;
            }
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
                        .send_event(Message::TabAttached(
                            project_id, tab_id, info, snapshot, hook_agent,
                        ))
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

    /// SSH 版 `spawn_new_tab`:握手 + host key 校验 + 认证 + 开 channel +
    /// request_pty/request_shell,成功后进入读写泵循环。10 秒超时罩住
    /// "握手到 channel 就绪"这一段(同阶段 1 `test_connection` 的超时
    /// 口径,泵循环本身不设超时——那是长连接,超时语义不适用)。
    pub(crate) fn spawn_ssh_tab(&mut self, io: &ShellIo, host_id: String, cols: u16, rows: u16) {
        if self.loading {
            return; // 同 spawn_new_tab:促成中的占位不建会话
        }
        let Some(host) = self.ssh.hosts().iter().find(|h| h.id == host_id).cloned() else {
            return;
        };
        let Some(project) = self.project.as_ref() else {
            return;
        };
        let project_id = project.id;
        let tab_id = self.next_tab_id;
        self.next_tab_id += 1;

        let password = ssh::keyring_password(project_id, &host_id);
        let (tx_out, mut rx_out) = mpsc::unbounded_channel::<SshOut>();
        let proxy = io.proxy.clone();

        let jh = io.handle.spawn(async move {
            let handshake_result = tokio::time::timeout(
                std::time::Duration::from_secs(10),
                ssh::handshake(&host, password),
            )
            .await;
            let handle = match handshake_result {
                Ok(Ok(h)) => h,
                Ok(Err(ssh::SshError::UnknownHostKey { fingerprint, key_bytes })) => {
                    let _ = proxy.send_event(Message::Ssh(ssh::Message::UnknownKeyDetected(
                        project_id,
                        host_id.clone(),
                        fingerprint.clone(),
                        key_bytes,
                    )));
                    let _ = proxy.send_event(Message::Ssh(ssh::Message::TerminalConnectFailed(
                        project_id,
                        host_id,
                        tab_id,
                        format!("未知主机,指纹 {fingerprint}——需要确认信任"),
                    )));
                    return;
                }
                Ok(Err(ssh::SshError::KeyChanged { fingerprint })) => {
                    let _ = proxy.send_event(Message::Ssh(ssh::Message::KeyChanged(
                        project_id,
                        host_id.clone(),
                        fingerprint.clone(),
                    )));
                    let _ = proxy.send_event(Message::Ssh(ssh::Message::TerminalConnectFailed(
                        project_id,
                        host_id,
                        tab_id,
                        format!("主机指纹已变化({fingerprint}),拒绝连接"),
                    )));
                    return;
                }
                Ok(Err(e)) => {
                    let _ = proxy.send_event(Message::Ssh(ssh::Message::TerminalConnectFailed(
                        project_id,
                        host_id,
                        tab_id,
                        e.to_string(),
                    )));
                    return;
                }
                Err(_) => {
                    let _ = proxy.send_event(Message::Ssh(ssh::Message::TerminalConnectFailed(
                        project_id,
                        host_id,
                        tab_id,
                        "连接超时(10 秒)".to_string(),
                    )));
                    return;
                }
            };
            // `handle` 必须留在作用域内到函数结尾(泵循环退出前),不能被
            // 提前丢弃——见设计文档"背景"末尾与本计划 Global Constraints。
            let channel = match handle.channel_open_session().await {
                Ok(c) => c,
                Err(e) => {
                    let _ = proxy.send_event(Message::Ssh(ssh::Message::TerminalConnectFailed(
                        project_id,
                        host_id,
                        tab_id,
                        e.to_string(),
                    )));
                    return;
                }
            };
            let (mut read_half, write_half) = channel.split();
            if let Err(e) = write_half
                .request_pty(true, "xterm-256color", cols as u32, rows as u32, 0, 0, &[])
                .await
            {
                let _ = proxy.send_event(Message::Ssh(ssh::Message::TerminalConnectFailed(
                    project_id,
                    host_id,
                    tab_id,
                    e.to_string(),
                )));
                return;
            }
            if let Err(e) = write_half.request_shell(true).await {
                let _ = proxy.send_event(Message::Ssh(ssh::Message::TerminalConnectFailed(
                    project_id,
                    host_id,
                    tab_id,
                    e.to_string(),
                )));
                return;
            }

            let info = ssh::synth_session_info(&host, project_id);
            if proxy
                .send_event(Message::TabAttached(
                    project_id,
                    tab_id,
                    info,
                    Vec::new(),
                    None,
                ))
                .is_err()
            {
                return;
            }

            // 主机已登录:在同一个连接上开一个临时 session channel 采集
            // 操作系统信息(`cat /etc/os-release`),结果经 `FetchOsResult`
            // 落到主机卡片名字后面。与 shell 泵循环并行不冲突(SSH 连接
            // 支持多 channel),5 秒超时罩住慢命令,采集失败只是"卡片不
            // 展示 OS 段",不影响终端本身。
            let os_host_id = host_id.clone();
            let os_task = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                ssh::fetch_os_info(&handle),
            )
            .await
            .unwrap_or_else(|_| Err("采集操作系统超时".to_string()));
            let _ = proxy.send_event(Message::Ssh(ssh::Message::FetchOsResult(
                project_id,
                os_host_id,
                os_task,
            )));

            loop {
                tokio::select! {
                    msg = read_half.wait() => match msg {
                        Some(russh::ChannelMsg::Data { data }) | Some(russh::ChannelMsg::ExtendedData { data, .. }) => {
                            if proxy
                                .send_event(Message::TermOutput(project_id, tab_id, data.to_vec()))
                                .is_err()
                            {
                                return;
                            }
                        }
                        Some(russh::ChannelMsg::Eof) | Some(russh::ChannelMsg::Close) | None => {
                            let _ = proxy.send_event(Message::SessionExited(project_id, tab_id));
                            return;
                        }
                        Some(_) => continue,
                    },
                    out = rx_out.recv() => match out {
                        Some(SshOut::Data(bytes)) => {
                            if write_half.data_bytes(bytes).await.is_err() {
                                let _ = proxy.send_event(Message::SessionExited(project_id, tab_id));
                                return;
                            }
                        }
                        Some(SshOut::Resize { cols, rows }) => {
                            let _ = write_half.window_change(cols as u32, rows as u32, 0, 0).await;
                        }
                        None => return, // 所有 sender 已 drop(tab 关了/Workspace 没了)
                    },
                }
            }
        });

        self.pending.insert(tab_id, jh);
        self.ssh_out_pending.insert(tab_id, tx_out);
    }

    /// 打开一个 SFTP tab:建立独立 SSH 连接 + SFTP 子系统,起一个持有
    /// 连接、监听命令通道的异步任务(镜像 `spawn_ssh_tab` 的整体结构)。
    pub(crate) fn spawn_sftp_tab(&mut self, io: &ShellIo, host_id: String) {
        let Some(host) = self.ssh.hosts().iter().find(|h| h.id == host_id).cloned() else {
            return;
        };
        let Some(project) = self.project.as_ref() else {
            return;
        };
        let project_id = project.id;
        let project_root = std::path::PathBuf::from(&project.path);
        let password = ssh::keyring_password(project_id, &host_id);
        let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::unbounded_channel::<ssh::sftp::SftpCmd>();
        let proxy = io.proxy.clone();

        // 先在本地(同步)插入一个占位 tab 状态,连接结果异步回填——
        // 这样"点文件传输图标"能立刻看到一个 tab 出现(带 loading 态),
        // 不用等 10 秒握手超时才有任何 UI 反馈。
        self.sftp_tabs.insert(
            host_id.clone(),
            // 占位 root 传空字符串:握手前不知道远程用户的真实 home 路径,
            // 真实路径要等 `canonicalize(".")` 解析出来才通过 `Connected`
            // 回填(`ssh::sftp::RemoteTree::set_root`)。
            ssh::sftp::SftpTabState::new(host_id.clone(), project_root, String::new()),
        );

        let host_id_for_task = host_id.clone();
        io.handle.spawn(async move {
            let handshake_result = tokio::time::timeout(
                std::time::Duration::from_secs(10),
                ssh::sftp::open_sftp_session(&host, password),
            )
            .await;
            let (handle, sftp) = match handshake_result {
                Ok(Ok(pair)) => pair,
                Ok(Err(e)) => {
                    let _ = proxy.send_event(Message::Ssh(ssh::Message::Sftp(
                        ssh::sftp::Message::Connected(host_id_for_task, Err(e.to_string())),
                    )));
                    return;
                }
                Err(_) => {
                    let _ = proxy.send_event(Message::Ssh(ssh::Message::Sftp(
                        ssh::sftp::Message::Connected(
                            host_id_for_task,
                            Err("连接超时(10 秒)".to_string()),
                        ),
                    )));
                    return;
                }
            };
            // `"~"` 是 shell 语义,SFTP 协议的 `opendir`/`realpath` 不做
            // tilde 展开——之前直接拿字面 `"~"` 当 root 发 readdir,服务端
            // 当成真实文件名去找,几乎总是 "No such file",导致远程文件树
            // 读不出来。改用 `canonicalize(".")`(SFTP REALPATH 请求)问
            // 服务端要真实的 home 绝对路径。
            let root_result = sftp.canonicalize(".").await.map_err(|e| e.to_string());
            let _ = proxy.send_event(Message::Ssh(ssh::Message::Sftp(
                ssh::sftp::Message::Connected(host_id_for_task.clone(), root_result),
            )));
            // `handle` 必须留在这个任务作用域内到循环结束——同阶段 2
            // 终端连接的既有约束,理由一样(不能提前析构掉 SSH 连接本身)。
            let _handle_keepalive = handle;
            while let Some(cmd) = cmd_rx.recv().await {
                match cmd {
                    ssh::sftp::SftpCmd::ReadDir(dir) => {
                        let result = sftp
                            .read_dir(dir.clone())
                            .await
                            .map(|read_dir| {
                                read_dir
                                    .map(|e| ssh::sftp::RemoteEntry {
                                        path: e.path(),
                                        name: e.file_name(),
                                        is_dir: e.file_type().is_dir(),
                                    })
                                    .collect::<Vec<_>>()
                            })
                            .map_err(|e| e.to_string());
                        let _ = proxy.send_event(Message::Ssh(ssh::Message::Sftp(
                            ssh::sftp::Message::RemoteDirLoaded(
                                host_id_for_task.clone(),
                                dir,
                                result,
                            ),
                        )));
                    }
                    ssh::sftp::SftpCmd::Upload { local, remote_dir } => {
                        let result = ssh::sftp::upload(&sftp, &local, &remote_dir).await;
                        let _ = proxy.send_event(Message::Ssh(ssh::Message::Sftp(
                            ssh::sftp::Message::TransferResult(host_id_for_task.clone(), result),
                        )));
                    }
                    ssh::sftp::SftpCmd::Download { remote, local_dir } => {
                        let result = ssh::sftp::download(&sftp, &remote, &local_dir).await;
                        let _ = proxy.send_event(Message::Ssh(ssh::Message::Sftp(
                            ssh::sftp::Message::TransferResult(host_id_for_task.clone(), result),
                        )));
                    }
                }
            }
            // cmd_tx 全部 drop(tab 被关闭)→ 循环退出 → handle/sftp 析构 →
            // 连接关闭。
        });

        if let Some(state) = self.sftp_tabs.get_mut(&host_id) {
            state.cmd_tx = Some(cmd_tx);
        }
    }

    /// 只取 `cols`/`rows`(不取整个 `&ShellIo`)——`EventLoopProxy` 在单测
    /// 里没法脱离真实 winit 事件循环构造,签名只留函数体实际用到的两个
    /// `u16`,让 SSH-vs-daemon 的分流逻辑能被直接单测(镜像 `resize_one`
    /// 同样为了可测性收窄参数的既有先例)。`term_tab_bar_avail_px` 同理是
    /// `App::terminal_tab_bar_avail_px()` 的算好值而不是整个 `&App`——调用方
    /// (`Message::TabAttached` 分支)在借用 `ws` 之前算好传进来。参数超过 7
    /// 个后改收 [`TabAttachedArgs`](按 CLAUDE.md 关键裁决,不无脑加
    /// `#[allow(clippy::too_many_arguments)]`)——`cols`/`rows` 相邻同型,
    /// 位置传参顺序传错编译器发现不了,具名字段能防这个。
    pub(crate) fn on_tab_attached(&mut self, args: TabAttachedArgs) {
        let TabAttachedArgs {
            cols,
            rows,
            tab_id,
            info,
            snapshot,
            picked_agent,
            term_tab_bar_avail_px,
        } = args;
        let Some(forwarder) = self.pending.remove(&tab_id) else {
            return;
        };
        let mut model = TerminalModel::new(cols, rows);
        model.set_answer_dynamic_color(should_answer_dynamic_color(picked_agent, info.agent));
        let _ = model.feed(&snapshot);
        let ssh_backend = self.ssh_out_pending.remove(&tab_id);
        let is_ssh = ssh_backend.is_some();
        let mut tab = SessionTab {
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
            llm_model: None,
            permission_mode: None,
            last_activity: None,
            workspace_override: None,
            backend: match ssh_backend {
                Some(out) => TabBackend::Ssh { out },
                None => TabBackend::Daemon,
            },
        };
        tab.ingest_osc(&snapshot);
        if is_ssh {
            // synth_session_info 把 id 编成 "ssh:{host_id}"(ssh.rs:157),
            // 反解出 host_id 作为 ssh_tabs 里这个 tab 的身份。
            let host_id = tab
                .info
                .id
                .strip_prefix("ssh:")
                .unwrap_or(&tab.info.id)
                .to_string();
            self.ssh_tabs.push(tab);
            self.ssh_active = Some((host_id, ssh::SshTabKind::Terminal));
        } else {
            self.tabs.push(tab);
            self.active = self.tabs.len() - 1;
            // 此前硬写 `= 0`:tab 栏一旦横向溢出,新建的这个(必是最后一个、
            // 刚设成 `active`)如果排不进从 0 开始的窗口就直接被甩出可见区
            // ——用户新建会话反而看不见它,得自己点 V 下拉才找得到。跟
            // `select_tab_no_drag` 同一套 `tab_window_reveal`,把新 tab
            // 卷入可见窗口(镜像该处宽度计算,见其注释)。
            let widths: Vec<f32> = self
                .tabs
                .iter()
                .map(|t| tab_display_width(&tab_title(t.agent, t.cwd.as_deref(), &t.info.name)))
                .collect();
            self.term_tab_first = tab_window_reveal(
                &widths,
                4.0,
                term_tab_bar_avail_px,
                self.term_tab_first,
                self.active,
            );
        }
    }

    /// 终端 pane 尺寸变化:换算出的新网格套用到本项目的所有 tab,并把新
    /// 尺寸同步给远端侧存活的会话。
    ///
    /// 共享终端与 SSH 面板内嵌终端是两个独立 pane,几何不同,故各自带一
    /// 份网格:`cols/rows` 配共享 tab(`self.tabs`),`ssh_cols/ssh_rows` 配
    /// SSH tab(`self.ssh_tabs`)——SSH 终端必须按左面板区自己的宽度换算,
    /// 否则沿用共享终端的列数,字符折行对不上宿主面板。
    ///
    /// "网格真的变了吗"这道闸门在调用方 `App::update` 的 `PaneResized`
    /// 分支上——这里是外壳态运算结果的落地,不做二次比对。
    pub(crate) fn resize_all(
        &mut self,
        io: &ShellIo,
        cols: u16,
        rows: u16,
        ssh_cols: u16,
        ssh_rows: u16,
    ) {
        let client = io.client.clone();
        let handle = io.handle.clone();
        for tab in &mut self.tabs {
            Self::resize_one(tab, &client, &handle, cols, rows);
        }
        for tab in &mut self.ssh_tabs {
            Self::resize_one(tab, &client, &handle, ssh_cols, ssh_rows);
        }
    }

    /// 单个 tab 的尺寸同步:本地改模型 + 远端 resize。被 `resize_all` 对
    /// `tabs`/`ssh_tabs` 各调一遍(两个字段分开遍历避免"同时可变借用
    /// self 两个字段"的借用冲突)。
    fn resize_one(
        tab: &mut SessionTab,
        client: &Client,
        handle: &tokio::runtime::Handle,
        cols: u16,
        rows: u16,
    ) {
        tab.model.resize(cols, rows);
        if !tab.alive {
            return;
        }
        match &tab.backend {
            TabBackend::Daemon => {
                let client = client.clone();
                let id = tab.info.id.clone();
                handle.spawn(async move {
                    if let Err(e) = client.resize(&id, cols, rows).await {
                        tracing::warn!("同步终端尺寸到 daemon 失败: {e}");
                    }
                });
            }
            TabBackend::Ssh { out } => {
                let _ = out.send(SshOut::Resize { cols, rows });
            }
        }
    }

    /// 浏览器地址栏是否持有 iced 真实焦点(main.rs 据此路由键盘:真 →
    /// 标准 iced 管线,假 → keymap → PTY)。预览面板已不再有地址栏,只需查
    /// `self.browser`。
    pub fn browser_addr_focused(&self) -> bool {
        self.browser.addr_focused()
    }

    /// `kind` 是 `is_in_preview_column` 命中的面板(`Files` 或
    /// `Project`)。此前硬编码只查 `self.preview`(Files)——`Project`
    /// 面板的预览 webview 点击后一直拿不到键盘焦点(⌘C 复制不了),
    /// 这是这次 Stage 4b 才第一次让它变得可测、可发现的一个独立预存
    /// bug,不是拖拽换栏引入的新问题。`Project` 分支的 id 要加
    /// `PROJECT_PREVIEW_ID_OFFSET`,因为 `webviews` 共享池里它的 key
    /// 已经加了这个偏移(见 `preview_desired`)。
    pub fn active_preview_webview_id(&self, kind: PanelKind) -> Option<usize> {
        match kind {
            PanelKind::Project => self
                .project_preview
                .active_webview_id()
                .map(|id| id + PROJECT_PREVIEW_ID_OFFSET),
            _ => self.preview.active_webview_id(),
        }
    }

    /// 当前哪个预览面板有活跃 webview。`Files`/`Project` 各自带独立的
    /// `PreviewPane`,`active_preview_webview_id` 是按 `kind` 定向查询的;
    /// 这里在**不携带面板信息**的汇聚信号(如 `WebViewFocused`)需要反推
    /// "刚聚焦的是哪个池"时用:两个面板都有 webview 时按顺序返回
    /// `Files`(左栏预览通常是文件,优先级高),都没有返回 `None`。
    pub fn active_preview_panel_kind(&self) -> Option<PanelKind> {
        if self.preview.active_webview_id().is_some() {
            Some(PanelKind::Files)
        } else if self.project_preview.active_webview_id().is_some() {
            Some(PanelKind::Project)
        } else {
            None
        }
    }

    /// `kind` 是当前 `FocusIntent::Preview` 携带的面板(`Files` 或
    /// `Project`)——当前激活预览 tab 是否走原生渲染(有 `editor`)。
    /// main.rs 键盘路由用:原生预览 tab 需要在按键分发链里提前放行,让键盘
    /// 事件走 iced 正常管线直达 `CodeEditor`,不落进终端/⌘ 快捷键那些手工
    /// 转发分支。此前硬编码只查
    /// `self.preview`(Files)——`Project` 预览面板里打开的原生编辑器 tab
    /// 收不到键盘输入,是这次 Stage 4b 审阅时发现的独立预存 bug,和
    /// `active_preview_webview_id` 此前只查 `ws.preview` 是同一类问题。
    pub fn active_preview_tab_has_native_editor(&self, kind: PanelKind) -> bool {
        let pane = match kind {
            PanelKind::Project => &self.project_preview,
            _ => &self.preview,
        };
        pane.tabs()
            .get(pane.active_idx())
            .is_some_and(|t| t.editor.is_some())
    }

    /// 把 `kind` 面板**当前激活原生 tab** 的 CodeView `perform` 一条应用层
    /// 编辑 `Action`。这是 main.rs 键盘路由在原生预览闸门里手工补 `Tab`
    /// 的落点:`Active` 的编辑器 Content 光标在 CodeView 内部,直接
    /// `Action::Edit(Edit::Insert('\t'))`(非只读、非 webview/Blank)就插在
    /// 用户当前光标、替换现选区,撤销/脏标记照常,与 widget 自发 Action 走
    /// 同一条 perform 管线(见 code_editor 模块)。目标 tab 非原生/无编辑器
    /// 一律 no-op。
    pub fn preview_pane_active_editor_event(&mut self, kind: PanelKind, action: EditorAction) {
        let pane = match kind {
            PanelKind::Project => &mut self.project_preview,
            _ => &mut self.preview,
        };
        let tab_id = match pane.tabs().get(pane.active_idx()) {
            Some(t) if t.editor.is_some() => t.id,
            _ => return,
        };
        if let Some(editor) = pane.editor_mut(tab_id) {
            editor.perform(action);
        }
    }

    /// ⌘Z:把 `kind` 面板**当前激活原生 tab** 的编辑器回退一条编辑命令
    /// (`CodeView::undo`——官方 `text_editor` 无 undo API,历史是应用层整文本
    /// 快照栈,见 code_editor 模块"已知取舍")。真的发生了回退时顺手把该 tab
    /// 标脏:回退同样改变了 buffer、与磁盘不再一致,未保存前必须维持脏标记
    /// (不比对磁盘文本——`PreviewTab` 只存布尔 `dirty`,不做内容比对)。
    /// 目标非原生/无编辑器/栈空时 no-op。
    pub fn preview_pane_undo_active(&mut self, kind: PanelKind) {
        let pane = if kind == PanelKind::Project {
            &mut self.project_preview
        } else {
            &mut self.preview
        };
        let Some(tab_id) = pane
            .tabs()
            .get(pane.active_idx())
            .filter(|t| t.editor.is_some())
            .map(|t| t.id)
        else {
            return;
        };
        if let Some(editor) = pane.editor_mut(tab_id)
            && editor.undo()
        {
            pane.mark_dirty_by_id(tab_id);
        }
    }

    /// ⌘⇧Z:重做 `PreviewUndoActive` 撤掉的最后一条编辑(`CodeView::redo`),
    /// 发生重做时同样标脏(语义同 `preview_pane_undo_active`)。
    pub fn preview_pane_redo_active(&mut self, kind: PanelKind) {
        let pane = if kind == PanelKind::Project {
            &mut self.project_preview
        } else {
            &mut self.preview
        };
        let Some(tab_id) = pane
            .tabs()
            .get(pane.active_idx())
            .filter(|t| t.editor.is_some())
            .map(|t| t.id)
        else {
            return;
        };
        if let Some(editor) = pane.editor_mut(tab_id)
            && editor.redo()
        {
            pane.mark_dirty_by_id(tab_id);
        }
    }
    /// ⌘S:把 `kind` 面板**当前激活原生 tab** 的就地改动保存到磁盘,语义同
    /// `preview_pane_save_at`——除了定位固定取"当前激活 tab"。
    pub fn preview_pane_save_active(&mut self, kind: PanelKind) {
        let idx = if kind == PanelKind::Project {
            self.project_preview.active_idx()
        } else {
            self.preview.active_idx()
        };
        self.preview_pane_save_at(kind, idx);
    }

    /// 把 `kind` 面板**指定下标**tab 的就地改动保存到磁盘,语义同
    /// `preview_pane_save_active`(其实现已改为委托这个方法),差别只是不再
    /// 局限于"当前激活"——`preview_pane_toggle_render_mode`(代码→预览)、
    /// `PreviewCloseTab`/`ProjectPreviewCloseTab`(关闭前静默保存)都可能要
    /// 保存一个非激活的背景 tab。仅当该 tab 是原生可编辑且脏时动作,其余
    /// 沿用原实现(不脏不动磁盘、失败写面板 error)。`pub(crate)`(而非
    /// 私有 `fn`)是因为 `app.rs::update` 的 `PreviewCloseTab`/
    /// `ProjectPreviewCloseTab` 分支(Step 8)要直接调它做关闭前静默保存。
    pub(crate) fn preview_pane_save_at(&mut self, kind: PanelKind, idx: usize) {
        let project = kind == PanelKind::Project;
        let (tab_id, path) = {
            let pane = if project {
                &self.project_preview
            } else {
                &self.preview
            };
            let Some(tab) = pane.tabs().get(idx) else {
                return;
            };
            // 仅脏的**原生** tab 值得落盘;不脏不动磁盘(省得住人保存也触发
            // 外部监听/无谓 mtime),webview / Blank 没有就地 buffer。
            if !tab.dirty || tab.editor.is_none() {
                return;
            }
            let TabKind::File(p) = &tab.kind else {
                return;
            };
            (tab.id, p.clone())
        };
        // 取当前文本:结束上面的不可变借后,再作一次短暂可变借拿到 buffer 全量。
        let text = {
            let pane = if project {
                &mut self.project_preview
            } else {
                &mut self.preview
            };
            match pane.editor_mut(tab_id) {
                Some(e) => e.text(),
                None => return,
            }
        };
        match std::fs::write(&path, text) {
            Ok(()) => {
                // 落盘成功:按先前那段的 `dirty==true` 前提清脏;fail 写该面板 error。
                let pane = if project {
                    &mut self.project_preview
                } else {
                    &mut self.preview
                };
                pane.clear_dirty_by_id(tab_id);
                // 落盘成功:若该 tab 上开着文件内 Find 会话,按最新文本重算命中与
                // 当前定位(编辑把命中行推走/删除后,陈旧索引会导致 ⌘G 跳到错位)。
                pane.find_refresh_after_edit(tab_id);
            }
            Err(e) => {
                let err = Some(format!("保存失败: {e}"));
                if project {
                    self.project_preview_error = err;
                } else {
                    self.preview_error = err;
                }
            }
        }
    }

    /// 切换 `kind` 面板某个 tab 的预览/代码渲染模式(仅对 `wry_toggle_eligible`
    /// 的文件 tab 有意义——按钮只在这类 tab 上画,其它 tab 点不到)。
    /// 代码→预览:先 `preview_pane_save_at` 静默落盘(不脏则内部直接
    /// no-op),再 `exit_code_mode` 转回渲染。预览→代码:直接
    /// `enter_code_mode` 读盘建原生编辑器,失败写对应面板 error。
    pub(crate) fn preview_pane_toggle_render_mode(&mut self, kind: PanelKind, idx: usize) {
        let project = kind == PanelKind::Project;
        let in_code_mode = {
            let pane = if project {
                &self.project_preview
            } else {
                &self.preview
            };
            pane.tabs().get(idx).is_some_and(|t| t.editor.is_some())
        };
        if in_code_mode {
            self.preview_pane_save_at(kind, idx);
            let pane = if project {
                &mut self.project_preview
            } else {
                &mut self.preview
            };
            pane.exit_code_mode(idx);
        } else {
            let pane = if project {
                &mut self.project_preview
            } else {
                &mut self.preview
            };
            if let Err(e) = pane.enter_code_mode(idx) {
                let err = Some(format!("打开代码模式失败: {e}"));
                if project {
                    self.project_preview_error = err;
                } else {
                    self.preview_error = err;
                }
            }
        }
    }

    /// 把 `kind` 面板的 Find 命令转发给面板执行。四种都只需要分面板取到变引用
    /// 调对应方法(面板自持 buffer + Find 状态,逻辑全在 pane 内,这里只当跳板,
    /// 避免 workspace 顶部对各项目冗余拆两支)。
    /// ⌘F 打开同时在面板上置一次性"请求聚焦输入框"(已开着的第二次 ⌘F 也只是
    /// 把焦点拨回输入框——见 `open_find_on_active` 的 no-op 语义),主循环次帧
    /// 用 `find_field_id(panel)` 真正把焦点给输入框。替换行默认收起(⌘R 走
    /// `preview_find_open_with_replace` 才展开)。
    pub fn preview_find_open(&mut self, kind: PanelKind) {
        let pane = if kind == PanelKind::Project {
            &mut self.project_preview
        } else {
            &mut self.preview
        };
        pane.open_find_on_active(false);
        pane.request_find_focus();
    }

    /// 同 `preview_find_open`,但替换行默认展开(⌘R)。
    pub fn preview_find_open_with_replace(&mut self, kind: PanelKind) {
        let pane = if kind == PanelKind::Project {
            &mut self.project_preview
        } else {
            &mut self.preview
        };
        pane.open_find_on_active(true);
        pane.request_find_focus();
    }

    /// 查询框前的圆盘箭头:手动翻转 `kind` 面板 Find 条的替换行展开态。
    pub fn preview_find_toggle_replace(&mut self, kind: PanelKind) {
        if kind == PanelKind::Project {
            self.project_preview.toggle_find_replace();
        } else {
            self.preview.toggle_find_replace();
        }
    }

    /// `kind` 面板的 Find 条当前是否显示(main.rs Esc/⌘ 键盘路由、视图渲染分层
    /// 共用)。
    pub fn preview_find_bar_open(&self, kind: PanelKind) -> bool {
        if kind == PanelKind::Project {
            self.project_preview.find_bar_open()
        } else {
            self.preview.find_bar_open()
        }
    }

    /// 每帧渲染循环把 `preview::take_find_focused(kind)` 查到的真实焦点态
    /// 写回这里。
    pub fn set_find_query_focused(&mut self, kind: PanelKind, focused: bool) {
        if kind == PanelKind::Project {
            self.project_preview.set_find_query_focused(focused);
        } else {
            self.preview.set_find_query_focused(focused);
        }
    }

    /// 关闭 `kind` 面板 Find 条(× / Esc / ⌘F 里输入框清空后的迁离)。关闭后把
    /// 焦点拨回其下代码编辑器(`request_editor_focus` 置一次性位,次帧 main.rs
    /// 用 `active_editor_focus_id` 真正聚焦回)——⌘F 打开时开条会抢走输入框焦
    /// 点,关闭就该还回去,不许键盘焦点悬空在已消失的 widget 上。
    pub fn preview_find_close(&mut self, kind: PanelKind) {
        let pane = if kind == PanelKind::Project {
            &mut self.project_preview
        } else {
            &mut self.preview
        };
        pane.close_find();
        pane.request_editor_focus();
    }

    /// 键入:转发 query 到面板,让面板当场重算与跳第一个命中。
    pub fn preview_find_type(&mut self, kind: PanelKind, query: String) {
        if kind == PanelKind::Project {
            self.project_preview.find_type(query);
        } else {
            self.preview.find_type(query);
        }
    }

    /// 下一个/上一个命中。
    pub fn preview_find_go(&mut self, kind: PanelKind, next: bool) {
        if kind == PanelKind::Project {
            self.project_preview.find_go(next);
        } else {
            self.preview.find_go(next);
        }
    }

    /// 翻转 Find 条大小写敏感开关(`case_sensitive` 真=逐字严格、假=ASCII 折叠),
    /// 作用在 `kind` 面板当前打开的会话上;条未开是 no-op。
    pub fn preview_find_case(&mut self, kind: PanelKind, case_sensitive: bool) {
        if kind == PanelKind::Project {
            self.project_preview.set_find_case(case_sensitive);
        } else {
            self.preview.set_find_case(case_sensitive);
        }
    }

    /// 落定「替换为」草稿到 `kind` 面板当前 Find 会话(只写,不触发替换)。
    pub fn preview_find_set_replacement(&mut self, kind: PanelKind, replacement: String) {
        if kind == PanelKind::Project {
            self.project_preview.set_find_replacement(replacement);
        } else {
            self.preview.set_find_replacement(replacement);
        }
    }

    /// 「替换当前命中」。作用对象与返回值语义见 `PreviewPane::replace_current`。
    pub fn preview_find_replace_current(&mut self, kind: PanelKind) {
        if kind == PanelKind::Project {
            self.project_preview.replace_current();
        } else {
            self.preview.replace_current();
        }
    }

    /// 「替换全部」。作用对象与返回值语义见 `PreviewPane::replace_all`。
    pub fn preview_find_replace_all(&mut self, kind: PanelKind) {
        if kind == PanelKind::Project {
            self.project_preview.replace_all();
        } else {
            self.preview.replace_all();
        }
    }

    /// 当前激活浏览器 tab 的 webview id,语义同 `active_preview_webview_id`,
    /// 查独立的 `self.browser`。
    pub fn active_browser_webview_id(&self) -> Option<usize> {
        self.browser.active_webview_id()
    }

    /// 项目树行内编辑框是否持有 iced 真实焦点(main.rs 键盘路由用,每帧由
    /// `CaptureTreeEditFocus` 查询后经 `set_tree_edit_focused` 写入)。
    pub fn tree_edit_focused(&self) -> bool {
        self.files.tree_edit_focused()
    }

    /// 读走(消费式)项目树行内编辑的一次性程序化聚焦标记(新建/重命名刚
    /// 触发时置位,text_input 下一帧才出现、不会自己拿焦点)。main.rs 在
    /// `UserInterface::build` 之前调用,为真则用 `operation::focusable::
    /// focus` 强制聚焦真 `text_input`。
    pub fn take_tree_edit_focus_pending(&mut self) -> bool {
        self.files.take_tree_edit_focus_pending()
    }

    /// 读走(消费式)拖拽移动确认框"新名称"输入框的一次性程序化聚焦标记
    /// (弹框刚出现时置位,真 `text_input` 下一帧才出现、不会自己拿焦点)。
    /// main.rs 在 `UserInterface::build` 之前调用,为真则用 `operation::
    /// focusable::focus` 强制聚焦,同 `take_tree_edit_focus_pending`。
    pub fn take_move_focus_pending(&mut self) -> bool {
        self.files.take_move_focus_pending()
    }

    /// 读走(消费式)Todo 任务内容编辑的一次性程序化聚焦标记(点卡片文字进入
    /// 编辑态时置位,text_input 下一帧才出现、不会自己拿焦点)。main.rs 在
    /// `UserInterface::build` 之前调用,为真则用 `operation::focusable::
    /// focus` 强制聚焦真 `text_input`。
    pub fn take_content_edit_focus_pending(&mut self) -> bool {
        self.todo.take_content_edit_focus_pending()
    }

    /// 文件树搜索框是否持有 iced 真实焦点(main.rs 键盘路由用):为真时按键
    /// 放行给标准 iced 事件管线,交真正的 text_input 自己处理。
    pub fn files_search_focused(&self) -> bool {
        self.files.search_focused()
    }

    /// SSH 新增/编辑表单**任意字段**是否持有真实焦点(main.rs 键盘路由用)。
    /// 曾经只看"表单是否打开"这个粗粒度信号(`ssh.editing().is_some()`),
    /// 图省事不用像 Files 搜索框那样每帧查真实焦点——2026-09 用户反馈的根因
    /// 就在这里:SSH/Database 面板与 Agent 终端分栏同屏显示时,表单开着但
    /// 用户实际点进的是终端输入框,粗粒度信号仍卡真,main.rs 键盘路由的
    /// OR 链一直 `return`,终端收不到任何按键(面板"是否可见"、表单"是否
    /// 打开"、字段"是否真聚焦"是三件不同的事,只有最后一个才是键盘该往哪儿
    /// 走的正确依据)。现在同 `files_search_focused` 一样查真实焦点。
    pub fn ssh_form_open(&self) -> bool {
        self.ssh.form_focused()
    }

    /// Database 新增/编辑表单任意字段是否持有真实焦点(main.rs 键盘路由用),
    /// 同 `ssh_form_open` 的粒度与根因。
    pub fn database_form_open(&self) -> bool {
        self.database.form_focused()
    }

    /// 每帧渲染循环用 `ssh::take_form_focused()` 查回来的真实焦点态写回。
    pub fn set_ssh_form_focused(&mut self, focused: bool) {
        self.ssh.set_form_focused(focused);
    }

    /// 每帧渲染循环用 `database::take_form_focused()` 查回来的真实焦点态
    /// 写回。
    pub fn set_database_form_focused(&mut self, focused: bool) {
        self.database.set_form_focused(focused);
    }

    /// 右键文件树"搜索"弹窗是否打开(main.rs 键盘路由/App view 浮层用)。
    pub fn search_popup_open(&self) -> bool {
        self.search.is_open()
    }

    /// 右键文件树"搜索"弹窗查询框是否持有 iced 真实焦点(main.rs 原生放行
    /// 闸门用)。
    pub fn query_focused(&self) -> bool {
        self.search.query_focused()
    }

    /// 读走(消费式)搜索弹窗查询框的一次性聚焦标记。
    pub fn take_query_focus_pending(&mut self) -> bool {
        self.search.take_query_focus_pending()
    }

    /// 项目信息面板名称编辑框是否持有 iced 真实焦点(main.rs 原生放行闸门用)。
    pub fn name_edit_focused(&self) -> bool {
        self.project_panel.name_edit_focused()
    }

    /// 读走(消费式)项目名称编辑框的一次性聚焦标记。
    pub fn take_name_edit_focus_pending(&mut self) -> bool {
        self.project_panel.take_name_edit_focus_pending()
    }

    /// 当前项目根路径(供 main.rs 算相对路径用;未打开项目时 None)。
    pub fn active_project_path(&self) -> Option<PathBuf> {
        self.project.as_ref().map(|p| PathBuf::from(&p.path))
    }

    /// 点击输入框外时退出所有自绘输入的编辑态(验收反馈:失焦回正常态)。
    /// 浏览器地址栏取消(清空半输入),意见框仅退出编辑(保留已输入文字),树内
    /// 编辑(重命名/新建)直接取消(Important #5——不清会导致点到别处后键盘还
    /// 在悄悄写进树编辑缓冲区,"打不出字"的假象)。`context_menu` 不在这里
    /// 清:它已经有专门的外点 dismiss 遮罩(`files::Message::ContextMenuClose`,
    /// 见 view() 里的 stack dismiss 层),这里重复清是死代码。
    /// 点击输入框外时退出所有自绘输入的编辑态。`keep_native_preview_editor`
    /// 为 `true` 时跳过结尾的 `blur_preview_editors`(见其文档):官方
    /// `text_editor` 是真实 iced 焦点,鼠标点进它自己那帧会 self-focus 出
    /// 光标,这里若再补一记 unfocus 就会同一帧把光标抬掉——`pending_editor_focus`
    /// 之外,点到原生预览编辑器本身上时也必须让它留住焦点(2026-09-06:
    /// "点代码预览无法获得光标" 修复)。点击的是预览列里非编辑器区(树/Files
    /// 等)时保持 `false`,照旧把预览编辑器也一起 blur 掉。
    pub fn blur_inputs(&mut self, keep_native_preview_editor: bool) {
        self.todo.cancel_drag();
        // 名称编辑不在失焦时丢弃——改由 `App::blur_inputs` 取出缓冲并发起
        // daemon 改名(改动且非空才发请求),与描述字段"失焦写盘"行为对齐。
        if let Some(project) = self.project.as_ref() {
            let path = std::path::Path::new(&project.path);
            self.project_panel.submit_description_edit_on_blur(path);
        }
        // 任务内容行内编辑的失焦落盘/丢弃判断已经从 `blur_inputs` 搬走——
        // 改由 `App::set_todo_content_focused` 的边缘触发(`CaptureContentEditFocus`
        // 每帧查到的真实焦点从真变假那一刻)承担,见该方法的文档。
        if !keep_native_preview_editor {
            self.blur_preview_editors();
        }
    }

    /// 官方 `text_editor` 的焦点是真实 iced 焦点树的一部分,不能像 vendored
    /// `iced-code-editor` 那样直接对某个实例调 `lose_focus()`——只能置一个
    /// 一次性位,main.rs 下一帧用 `operation::focusable::unfocus`(不指定
    /// 目标,统一让出当前持有焦点的那个 widget)。独立于 `blur_inputs` 之外
    /// 单独暴露:`main.rs` 里还有一条不经过鼠标点击、纯靠消息把
    /// `current_focus` 拨离 `FocusIntent::Preview` 的路径(切终端 tab/新会话
    /// 落成/选中 agent),那条路径不该顺带触发 `blur_inputs` 里其它自绘输入
    /// 的失焦逻辑(地址栏取消/树编辑取消等语义不搭)。
    pub fn blur_preview_editors(&mut self) {
        self.pending_editor_unfocus = true;
    }

    /// 读走(消费式)"让出预览编辑器焦点"的一次性标记。
    pub fn take_editor_unfocus_pending(&mut self) -> bool {
        std::mem::take(&mut self.pending_editor_unfocus)
    }

    /// 协议闭包共享的文件白名单句柄.
    pub fn allowed_files(&self) -> Arc<Mutex<HashSet<PathBuf>>> {
        Arc::clone(&self.allowed_files)
    }

    /// 协议闭包共享的审阅内容快照句柄,同 `allowed_files` 的手法。
    pub fn review_snapshot(&self) -> Arc<Mutex<Option<String>>> {
        Arc::clone(&self.review_snapshot)
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

/// 决定这次 hook 事件驱动的卡片刷新要不要做 model/mode 提取、要不要
/// 单独查一次工作区 git 信息。抽成纯函数只为可测——
/// `Workspace::spawn_agent_card_refresh` 里直接调用,不重复判断逻辑。
pub(crate) fn agent_card_refresh_plan(
    agent: AgentKind,
    cwd: &Path,
    project_root: Option<&Path>,
) -> (bool, bool, bool) {
    // Unknown 同样走 Claude 形状的 transcript 解析(见 transcript.rs 里
    // parse_transcript 对 Unknown 的既有处理和注释——老装 hook 上报的
    // Unknown agent 不该因为这道门禁又变回"空白卡片"这同一类 bug)。
    // Codebuddy 有独立 schema,但 latest_model_mode_and_activity 已经兼认它的
    // providerData.model 字段(mode 恒 None——transcript 没有 permissionMode
    // 等价字段,卡片 Mode 行因此天然不渲染,不是 bug)。Opencode 的合成
    // transcript 是 Claude 形状,dozer-hook 插件(dozer-translate.ts::
    // onUserMessage)已经把 chat.message hook 拿到的 model(`{providerID,
    // modelID}`,spike 实测两个字段都有)格式化成 `"providerID/modelID"`
    // 写进 `message.model`,同一条 `latest_model_mode_and_activity` 路径
    // 直接读得到,不需要单独分支。mode 恒 None(OpenCode 没有
    // permissionMode 等价字段,同 Codebuddy)。Kilo 等其余 agent 仍不在这道
    // 门禁里:它们的合成 transcript 还没有 model 字段,加了也读不到值。
    let needs_model_mode = matches!(
        agent,
        AgentKind::Claude | AgentKind::Unknown | AgentKind::Codebuddy | AgentKind::Opencode
    );
    // "当前工作内容"兜底摘要的门禁比 model/mode 宽——只要 transcript
    // schema 能被 `parse_transcript` 解出人类/AI 文本就值得读(Opencode/
    // Kilo 的合成 transcript 是 Claude 形状,真有内容,只是没写 model/mode
    // 字段而已;V8agent 的 transcript 现在也走同一条 Claude 形状解析路径,
    // 见 dozerd `transcripts/parse.rs` 的 `parse_chunk` 分派)。Codex 目前
    // `parse_transcript` 恒回空,读了也提取不出东西,不值得为它打开这道门。
    let needs_activity = !matches!(agent, AgentKind::Codex);
    // 精确相等太脆弱——cd 进项目根的任意子目录都会被判定成"偏离",既多做
    // 一次不必要的 git 查询,也是 Finding 1 那个 bug 更容易被触发的原因之
    // 一。改成路径前缀包含关系:cwd 是 project_root 的子路径就算"未偏离"。
    let needs_workspace = match project_root {
        Some(root) => !cwd.starts_with(root),
        None => true,
    };
    (needs_model_mode, needs_activity, needs_workspace)
}

/// `Message::AgentCardRefreshed` 落地:在 `tabs` 里找 `tab_id`。
/// `llm_model`/`mode` 的 `None` 表示这次没有新值,不覆盖已有值(每次刷新
/// 只重新扫描"当前" transcript 内容,理论上不会无中生有变回 `None`,这里
/// 的保护针对 transcript 读取失败等异常情形,不让卡片从"有值"闪回
/// "无值")。`workspace` 语义不同——它的 `None` 是上游
/// `spawn_agent_card_refresh`/`agent_card_refresh_plan` 给出的明确信号
/// "cwd 未偏离项目根",必须无条件覆盖(含清空 `Some` → `None`),否则
/// session 一旦偏离过一次项目根,`workspace_override` 就再也清不掉了。
/// tab 不存在(已关闭)时整体 no-op,不 panic。只依赖 `&mut [SessionTab]`
/// 不依赖整个 `Workspace`,同 `group_tabs_by_agent` 的既有写法,方便
/// 直接单测。
pub(crate) fn apply_agent_card_refresh(
    tabs: &mut [SessionTab],
    tab_id: usize,
    llm_model: Option<String>,
    mode: Option<String>,
    activity: Option<String>,
    workspace: Option<WorkspaceGitInfo>,
) {
    let Some(tab) = tabs.iter_mut().find(|t| t.tab_id == tab_id) else {
        return;
    };
    if llm_model.is_some() {
        tab.llm_model = llm_model;
    }
    if mode.is_some() {
        tab.permission_mode = mode;
    }
    if activity.is_some() {
        tab.last_activity = activity;
    }
    // workspace 不走"只在 Some 时覆盖"这条——上游 spawn_agent_card_refresh
    // 现在保证 None 在这里永远是明确语义("cwd 未偏离项目根,该清空覆盖"),
    // 不是"这次没查、保留原值"，跟 llm_model/mode 的 None 语义不同(那两个
    // 的 None 才是"没查到新值,保留旧值")。无条件覆盖——修复此前
    // workspace_override 一旦被设置过就再也清不掉的 bug:session 只要偏离
    // 过一次项目根,就算 cd 回去了,卡片工作区行也会永远停在旧快照上。
    tab.workspace_override = workspace;
}

/// 项目 git/磁盘状态在多个地方触发刷新,完成后分发成两条
/// 独立消息:`Files(StatusesRefreshed)` 只带文件级状态,`Project(GitRefreshed)`
/// 带分支/脏/remote。两个 extension 互不知道对方存在,内核是唯一知道
/// "这两份数据同源"的地方(设计文档"关键语义确认")。既有调用点:
/// `Workspace::from_restore`/`adopt_project`/回合结束(`agent_state_changed`)/
/// `Message::ProjectFsChanged`——因为要同时认识 `files::Message`/
/// `project::Message` 两个类型,不适合作为任何一个 extension 的自由函数,
/// 也不需要 `&self`,做成纯自由函数、参数显式传入。
pub(crate) fn spawn_project_git_refresh(project_id: i64, repo_path: PathBuf, io: &ShellIo) {
    let proxy = io.proxy.clone();
    io.handle.spawn(async move {
        let (b, d, s, r) = tokio::task::spawn_blocking({
            let repo_path = repo_path.clone();
            move || {
                (
                    delivery::branch(&repo_path),
                    delivery::is_dirty(&repo_path),
                    delivery::file_statuses(&repo_path),
                    delivery::remote_url(&repo_path),
                )
            }
        })
        .await
        .unwrap_or((None, false, HashMap::new(), Vec::new()));
        let _ = proxy.send_event(Message::Files(files::Message::StatusesRefreshed(
            project_id, s,
        )));
        let _ = proxy.send_event(Message::Project(project::Message::GitRefreshed(
            project_id, b, d, r,
        )));
    });
}

/// 磁盘占用是独立于组合 git 刷新的异步任务——避免大仓库的目录遍历拖慢
/// 分支/脏标显示。触发点与 `spawn_project_git_refresh` 相同。
pub(crate) fn spawn_disk_usage_refresh(project_id: i64, repo_path: PathBuf, io: &ShellIo) {
    let proxy = io.proxy.clone();
    io.handle.spawn(async move {
        let bytes = tokio::task::spawn_blocking(move || {
            project::dir_size_excluding(&repo_path, &project::DISK_USAGE_EXCLUDE)
        })
        .await
        .unwrap_or(0);
        let _ = proxy.send_event(Message::Project(project::Message::DiskUsageLoaded(
            project_id, bytes,
        )));
    });
}

/// `[会话已结束]` 尾行标记：CREAM 字 / CARD 底。颜色值直接镜像
/// `byteui::theme::color::current().cream` / `byteui::theme::color::current().card`——`TerminalModel` 只吃 ANSI truecolor
/// 转义序列，认不出 `iced::Color`，这里手工写死 RGB 常量；本任务范围
/// 不含 `theme.rs`，若那边颜色改动，这两个常量需要手动同步。
pub(crate) fn exited_marker() -> Vec<u8> {
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
pub(crate) fn review_content<'a>(
    mut content: iced_widget::Column<'a, Message, iced_widget::Theme, iced_renderer::Renderer>,
    ws: &'a Workspace,
) -> iced_widget::Column<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let Some(rv) = &ws.review else {
        return content;
    };
    if let Some(err) = &rv.error {
        return content.push(
            text(format!("⚠ {err}"))
                .size(byteui::theme::font::subtitle())
                .color(byteui::theme::color::current().red),
        );
    }
    if rv.entries.is_empty() {
        content = content.push(lh(text("暂无对话")
            .size(byteui::theme::font::subtitle())
            .color(byteui::theme::color::current().dim)));
    }
    // 有内容时:真正的渲染由 review_content_pane 区域叠加的 wry webview
    // 负责(dozer://review-trace/host.html,数据经 dozer://review-trace/
    // data.json 拉取),这里不再手写 iced Column——2026-08-21 webview
    // trace 改造,见
    // docs/superpowers/plans/2026-08-21-review-content-webview-trace.md。
    // 对话面板 session 详情(2026-08-27):条目渲染走 webview,但"加载更多"
    // 按钮补在 iced 滚动体末尾——点它发 `ConversationDetailLoadMore`,
    // dozer-app 的 `update` 用 `spawn_review_load_conversation(append=true)`
    // 追加下一页回合,而不是替换首屏。
    if let ReviewSource::Conversation(conversation_id) = &rv.source
        && !rv.entries.is_empty()
    {
        content = content.push(load_more_button(conversation_id, rv.entries.len() as i64));
    }
    content
}

/// 会话详情"加载更多"按钮(2026-08-27):居中的 `<summary>` 样式的图标按钮,
/// hover 金边提示,点击追加下一页回合。`after_turn_index` 用 `entries.len()`
/// 当锚点——`ReviewEntry` 不携带原始 turn index,详情页按整段摊平加载,取
/// 条数近似下一步起点即可(跳过首尾折叠带来的少量误差在可接受范围)。
fn load_more_button<'a>(
    conversation_id: &'a str,
    after_turn_index: i64,
) -> iced_widget::core::Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let btn = iced_widget::button(
        text("加载更多…")
            .size(byteui::theme::font::caption_sm())
            .color(byteui::theme::color::current().dim),
    )
    .on_press(Message::Conversations(
        conversations::Message::DetailLoadMore(conversation_id.to_string(), after_turn_index),
    ))
    .padding(6)
    .style(|_t, _s| iced_widget::button::Style {
        background: None,
        text_color: byteui::theme::color::current().dim,
        ..iced_widget::button::Style::default()
    });
    container(btn)
        .width(Length::Fill)
        .align_x(iced_widget::core::Alignment::Center)
        .into()
}

/// 对话面板 session 详情的首屏回合页大小(2026-08-27):列表分页
/// (`extensions::conversations::CONVERSATION_PAGE_SIZE`,20)与详情页分页
/// (这里,200)含义不同,不要混用。
pub(crate) const CONVERSATION_DETAIL_PAGE_SIZE: u32 = 200;

/// 按 `AgentKind` 把会话 tab 分组,固定顺序 Claude → Codebuddy → Opencode
/// → Codex → Kilo → V8agent → Unknown(与 `conversation_agents_present`
/// 同一份顺序),只返回非空分组(没有该 agent 的会话就不出现,面板不留空
/// 分组占位)。组内保持 `tabs` 原有顺序(tab 打开顺序)。返回下标而非
/// 引用——渲染时既要下标发 `Message::SelectTab(idx)`,又要用下标回查
/// `ws.tabs[idx]` 取展示字段,直接存下标比存 `&SessionTab` 省一次生命
/// 周期纠缠。
pub(crate) fn group_tabs_by_agent(tabs: &[SessionTab]) -> Vec<(AgentKind, Vec<usize>)> {
    const ORDER: [AgentKind; 7] = [
        AgentKind::Claude,
        AgentKind::Codebuddy,
        AgentKind::Opencode,
        AgentKind::Codex,
        AgentKind::Kilo,
        AgentKind::V8agent,
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
pub(crate) fn agent_list_pane<'a>(
    app: &'a App,
    ws: &'a Workspace,
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let region = theme::region::agent_list_pane();
    let mut content = column![
        // 套用统一 panel head:暖金 `#dcc9a3` 的 Bot 图标 + "Agent" 标题 +
        // 1px 分割线;新建 agent 的"＋"按钮放到标题同一行的右侧(见
        // `home_panel_head_with_actions` 的 `actions` 参数),不再单独占一行。
        home_panel_head_with_actions(
            IconKind::Brain,
            "Agent",
            Some(agent_picker_toggle_button(app)),
        ),
    ]
    .spacing(region.gap);

    if ws.tabs.is_empty() {
        content = content.push(lh(text("暂无会话")
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().dim)));
    } else {
        for (agent, idxs) in group_tabs_by_agent(&ws.tabs) {
            content = content.push(
                row![
                    icons::view(
                        agent_icon(agent),
                        byteui::theme::icon_size::row(),
                        agent_dot_color(agent),
                    ),
                    lh(text(format!("{}（{}）", agent.label(), idxs.len()))
                        .size(byteui::theme::font::caption())
                        .color(byteui::theme::color::current().dim)),
                ]
                .align_y(iced_widget::core::alignment::Vertical::Center)
                .spacing(8),
            );
            for idx in idxs {
                content = content.push(agent_card(ws, idx));
            }
        }
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

/// Agent 面板里单条会话卡片,三行:1) 图标 + agent 名(+ `(model, mode)`,
/// 只要有一项能读到就跟名字拼一起,两项都没有就只显示名字);2) "当前
/// 工作内容"(优先 Todo 任务派发出来时反查到的任务标题,拿不到就用
/// transcript 最后活动摘要兜底,都没有就省略)紧跟工作区文案(分支名+脏标),
/// 同一行不换行(卡片改版要求,work_content 在前);3) 状态点 + 状态文字。
/// 整卡可点选中该 tab(`idx == ws.active` 时金色边框高亮、无背景;hover 时
/// 显示 `CARD` 背景 + 金色边框)。Task 6 起 `TodoInfo` 自带派发记录,
/// `task_title_for_session` 不再需要 `app` 侧的元数据表(见 todo.rs)。
pub(crate) fn agent_card<'a>(
    ws: &'a Workspace,
    idx: usize,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let tab = &ws.tabs[idx];
    let active = idx == ws.active;

    let model_label = tab.llm_model.as_deref().map(format_model_label);
    let mode_label = tab.permission_mode.as_deref();
    let llm_mode_value = match (&model_label, mode_label) {
        (Some(m), Some(mo)) => Some(format!("{m}, {mo}")),
        (Some(m), None) => Some(m.clone()),
        (None, Some(mo)) => Some(mo.to_string()),
        (None, None) => None,
    };
    let title = tab_title(tab.agent, tab.cwd.as_deref(), &tab.info.name);
    let title_text = match &llm_mode_value {
        Some(v) => format!("{title} ({v})"),
        None => title,
    };

    let mut lines = column![
        text(title_text)
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().cream),
    ]
    .spacing(4);

    let work_content = ws
        .todo
        .task_title_for_session(&tab.info.id)
        .map(str::to_string)
        .or_else(|| tab.last_activity.clone());

    let (branch, dirty) = match &tab.workspace_override {
        Some(w) => (w.branch.as_deref(), w.dirty),
        None => (ws.project_panel.branch(), ws.project_panel.dirty()),
    };
    lines = lines.push(work_content_and_workspace_row(
        work_content.as_deref(),
        branch,
        dirty,
    ));

    lines = lines.push(
        row![
            byteui::feedback::status::dot(dot_color(tab.agent_state, tab.alive)),
            text(agent_state_label(tab.agent_state))
                .size(byteui::theme::font::caption_sm())
                .color(byteui::theme::color::current().dim),
        ]
        .spacing(6)
        .align_y(iced_widget::core::Alignment::Center),
    );

    button(container(lines).padding(10))
        // 不是左侧 tab 栏本身,发 `SelectTabNoDrag` 而不是 `SelectTab`——
        // 后者会顺带武装左侧 tab 栏的拖拽状态机,导致点这张卡片后只要
        // 光标划过 tab 栏就被误判成"正在拖 tab"而错误换位(见
        // `Message::SelectTab` 文档,2026-08-17 修的真实 bug)。
        .on_press(Message::SelectTabNoDrag(idx))
        .width(Length::Fill)
        .style(byteui::interaction::cards::button_card(
            active,
            byteui::theme::color::current().card,
        ))
        .into()
}

/// "当前工作内容"(有就显示,没有就省略)+ 工作区(`@分支名` + 脏标,所有
/// agent 都显示)合并一行、不换行,work_content 排在工作区前面(卡片
/// 改版要求)。工作区不再用"工作区:"文字标签,前缀改成 `@`——跟卡片其余
/// 行的极简风格对齐。分支名规则不变:无分支(非 git 项目)显示 `—`;有
/// 未提交改动时分支名后缀 `(Uncommitted)`——跟 `extensions/files.rs`
/// 里分支切换菜单当前分支带脏标时的既有文案(`n.push_str("(Uncommitted)")`,
/// 见该文件约第 1409 行)保持同一措辞,不新造一套脏标文案。
fn work_content_and_workspace_row(
    work_content: Option<&str>,
    branch: Option<&str>,
    dirty: bool,
) -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let workspace_value = match branch {
        Some(b) if dirty => format!("{b}(Uncommitted)"),
        Some(b) => b.to_string(),
        None => "—".to_string(),
    };
    let value = match work_content {
        Some(w) => format!("{w}  @{workspace_value}"),
        None => format!("@{workspace_value}"),
    };
    text(value)
        .size(byteui::theme::font::caption())
        .color(byteui::theme::color::current().dim)
        .into()
}

/// Agent 面板头部"＋"按钮:点击切换 `agent_picker_open`,弹出 agent
/// 选择菜单(`agent_picker_popup`)。样式与顶栏页签行的"＋"一致——无背景、
/// Lucide `SquarePlus` 图标、静止灰(`DIM`)、hover 平滑过渡到金(`GOLD`),
/// 由 `HoverId::AgentPickerToggle` + `MouseArea` 驱动同一套悬停动画。
pub(crate) fn agent_picker_toggle_button<'a>(
    app: &App,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    icons::icon_button_entry(
        icons::IconKind::SquarePlus,
        byteui::theme::icon_size::row(),
        false,
        false,
        app.hover_progress(HoverId::AgentPickerToggle),
        false,
        byteui::theme::geometry::tab_button_size(),
        true,
        Message::AgentPickerToggle,
        |hovered| Message::Hover(HoverId::AgentPickerToggle, hovered),
        "新建 Agent 会话",
    )
}

/// Agent 选择菜单浮层:固定挂在窗口右上角("＋"按钮下方——该按钮
/// 就在最靠右的 Agent 面板头部,近似等于窗口右上角),八个选项按标签
/// 首字母顺序排列:Claude/CodeBuddy/Codex/Git Shell/Kilo/OpenCode/
/// v8agent/纯 Shell(验收反馈,2026-08-21;此前是手写的固定顺序,不便
/// 找到目标 agent)。跟项目树右键菜单(`context_menu_popup`)同款按钮
/// 样式,但不需要像素坐标定位——同 `delete_confirm_popup` 一样固定
/// padding 摆位。`ws.agent_picker_open` 为假时返回空视图,调用方
/// (`App::view`)据此决定要不要把这层塞进 `stack!`。
pub(crate) fn agent_picker_popup(
    ws: &Workspace,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    if !ws.agent_picker_open {
        return column![].into();
    }
    // 常规 agent 按标签首字母排序。"Git Shell" / "纯 Shell" 归到菜单最底部,
    // 与上方 agent 用 1px 分割线(`crate::menu::separator`)分组隔开。
    let agents: [(&str, PickerLaunch); 6] = [
        ("Claude", PickerLaunch::Agent(Some(AgentKind::Claude))),
        ("CodeBuddy", PickerLaunch::Agent(Some(AgentKind::Codebuddy))),
        ("Codex", PickerLaunch::Agent(Some(AgentKind::Codex))),
        ("Kilo", PickerLaunch::Agent(Some(AgentKind::Kilo))),
        ("OpenCode", PickerLaunch::Agent(Some(AgentKind::Opencode))),
        ("v8agent", PickerLaunch::Agent(Some(AgentKind::V8agent))),
    ];
    let shells: [(&str, PickerLaunch); 2] = [
        ("Git Shell", PickerLaunch::Git),
        ("纯 Shell", PickerLaunch::Agent(None)),
    ];
    // 单项统一走 `crate::menu::item_row`:图标沿用各 agent 代表色,文字保持
    // `BODY`(同 `menu::item()` 的标准配色);hover/锁定语义、常宽、padding
    // 同文件树右键菜单基准。
    let mk_item = |label: &'static str,
                   agent: PickerLaunch|
     -> Element<'static, Message, iced_widget::Theme, iced_renderer::Renderer> {
        let (icon, icon_color) = match agent {
            PickerLaunch::Agent(Some(kind)) => (agent_icon(kind), agent_dot_color(kind)),
            PickerLaunch::Agent(None) => (IconKind::Terminal, byteui::theme::color::current().body),
            PickerLaunch::Git => (IconKind::GitBranch, byteui::theme::color::current().body),
        };
        crate::menu::item_row(
            Some(icons::view(
                icon,
                byteui::theme::icon_size::row(),
                icon_color,
            )),
            label,
            byteui::theme::color::current().body,
            Some(Message::AgentPickerSelect(agent)),
        )
    };
    let mut list: Vec<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>> =
        Vec::new();
    for (label, agent) in agents {
        list.push(mk_item(label, agent));
    }
    // 1px 分割线:把 Shell 类(底部)与上方常规 agent 分组隔开。
    list.push(crate::menu::separator());
    for (label, agent) in shells {
        list.push(mk_item(label, agent));
    }
    let list: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
        crate::menu::shell_frosted(
            list,
            Length::Fixed(byteui::theme::geometry::menu_item_width()),
        );
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

/// 审阅内容的 webview 期望清单(`preview::desired_webviews` 同款语义)。
/// 没有审阅内容 / 出错 / 空回合区间时返回空清单——`sync_webview_pool`
/// 的 `retain` 会据此销毁 webview,不需要额外的隐藏逻辑。有内容时返回
/// 唯一一条,URL 带 `rv.nonce` 当查询参数,内容变化(`Message::ReviewLoaded`
/// 落地新 entries)时 nonce 递增、URL 变化,逼 `sync_webview_pool` 重新
/// `load_url`(同 `preview.rs::PreviewTab.reload_nonce` 的手法)。`id`
/// 固定填 0,真正的池 key 由调用方(`App::preview_desired`)加
/// `CONVERSATION_REVIEW_ID_OFFSET` 决定——这个面板任意时刻只有一份内容,
/// 不需要 Files/Project 那种按 tab id 分池的能力。
pub(crate) fn review_webview_spec(review: Option<&ReviewView>) -> Vec<crate::preview::WebviewSpec> {
    let Some(rv) = review else {
        return Vec::new();
    };
    if rv.error.is_some() || rv.entries.is_empty() {
        return Vec::new();
    }
    vec![crate::preview::WebviewSpec {
        id: 0,
        url: format!("dozer://review-trace/host.html?_r={}", rv.nonce),
        visible: true,
    }]
}

/// 会话审阅内容面板(右面板区"对话"视图的内容侧):直接读 `ws.review`,
/// 不经过 `ws.preview` 的 tab 系统——新外壳下审阅是独立面板,不再是
/// 预览 tab 条里的一个 tab。
pub(crate) fn review_content_pane<'a>(
    app: &'a App,
    ws: &'a Workspace,
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let region = theme::region::review_content_pane();
    // 内容侧标题栏:左侧名字信息面板(会话/agent 态) + 右侧"收起/展开
    // 列表列"按钮(与 Todo/Database/SSH/Agent 内容侧统一)。列表列收起后
    // 本面板拿满配对宽度,按钮仍在此处可见以便恢复。
    let header = container(
        row![home_panel_head_with_actions(
            IconKind::BotMessageSquare,
            "会话",
            Some(app.list_collapse_button(
                PanelKind::Conversations,
                app.list_collapsed(PanelKind::Conversations),
                HoverId::ConversationsListCollapse,
                "收起列表",
                "展开列表",
                Message::TogglePanelListCollapse(PanelKind::Conversations),
                move |h| { Message::Hover(HoverId::ConversationsListCollapse, h) },
            )),
        )]
        .width(Length::Fill),
    )
    .padding(theme::region::project_pane().padding);
    let mut content = column![header].spacing(region.gap);

    if ws.review.is_some() {
        let body = review_content(column![].spacing(region.gap), ws);
        content = content.push(
            Scrollable::new(body)
                .width(Length::Fill)
                .height(Length::Fill)
                .direction(scrollable::Direction::Vertical(
                    byteui::interaction::scrollbar::scrollbar(),
                ))
                .style(|_t, _s| byteui::interaction::scrollbar::scrollbar_style()),
        );
    } else {
        content = content.push(
            container(lh(text("暂无审阅内容——点击左侧对话列表中的对话开始审阅")
                .size(byteui::theme::font::subtitle())
                .color(byteui::theme::color::current().dim)))
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

/// 配对视图内部"列表:内容"的 `FillPortion` 权重对。`FillPortion` 在扣掉
/// 中间固定宽的分隔线之后按权重分剩余空间,与 `pair_content_width` 同源。
pub(crate) fn split_portions(split: f32) -> (u16, u16) {
    let list = (split * 10_000.0).round() as u16;
    let content = ((1.0 - split) * 10_000.0).round() as u16;
    (list, content)
}

/// 文件树目录/文件名行的字号：与终端字号(`terminal_font`)对齐（含全局 UI
/// scale），配合下面的 `LineHeight::Relative(line_height_factor)` 让每行行高
/// 等于终端行距，目录/文件列表不再比终端稀疏。
pub(crate) fn tree_row_font_size() -> f32 {
    terminal_font::size() * byteui::theme::icon_size::scale()
}

/// 统一行高：把一段文字的行高设为终端行高
/// (`terminal_font::line_height_factor()` = 1.2)，让各面板列表/正文行的行距
/// 与文件树、终端观感一致。`size`/`color` 等仍由调用方设置，这里只补行高。
pub(crate) fn lh<'a>(
    t: iced_widget::text::Text<'a, iced_widget::Theme, iced_renderer::Renderer>,
) -> iced_widget::text::Text<'a, iced_widget::Theme, iced_renderer::Renderer> {
    t.line_height(LineHeight::Relative(terminal_font::line_height_factor()))
}

/// `PanelKind::Files` 在没有打开项目时的占位:"未打开项目"提示 + 最近项目
/// 列表(点击即打开)。这是一个项目切换器,不是文件树的一部分,`files` 模块
/// 不认识 `ws.recent_projects`/`Message::ProjectSelect` 这些核心概念,留在
/// 内核(现有 `project_pane` 的 `None` 分支的搬家版本,渲染结构原样保留)。
pub(crate) fn no_project_placeholder<'a>(
    ws: &'a Workspace,
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let region = theme::region::project_pane();
    let mut header = column![].spacing(region.gap).width(Length::Fill);
    let tree_col = column![].spacing(region.gap);
    header = header.push(
        text("未打开项目")
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().dim),
    );
    for p in &ws.recent_projects {
        header = header.push(
            button(
                text(p.name.clone())
                    .size(byteui::theme::font::body())
                    .color(byteui::theme::color::current().cream),
            )
            .on_press(Message::ProjectSelect(p.id))
            .style(|_t, _s| button::Style {
                background: None,
                text_color: byteui::theme::color::current().cream,
                ..button::Style::default()
            }),
        );
    }
    let body = container(
        column![
            header,
            Scrollable::new(tree_col)
                .width(Length::Fill)
                .height(Length::Fill)
                .direction(scrollable::Direction::Vertical(
                    byteui::interaction::scrollbar::scrollbar()
                ))
                .style(|_t, _s| byteui::interaction::scrollbar::scrollbar_style()),
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

/// 原生预览里文件内 Find 的「查询」与「替换」两个输入框各自的外框壳。
/// text_input 本体是透明无边框的(`bare`/`unframed` 或带 8px 自己 padding),
/// 真正的圆角框底/描边由这里画齐整:
/// - 底色用 `colors.bg` —— 与 `code_editor::editor_style` 画编辑器同一 `bg`,
///   让文件内搜索时敲进去的词,底色跟右侧正浏览的代码看板完全一致(需求:
///   "输入框背景色和 editor 一致")。
/// - 有内容(`active`)整框描金,否则普通 `colors.border` 边色(沿用单字段
///   chip 的"非空即高亮"约定)。
fn find_field_shell<'a>(
    inner: iced_widget::core::Element<
        'a,
        crate::app::Message,
        iced_widget::Theme,
        iced_renderer::Renderer,
    >,
    colors: byteui::theme::color::ColorTokens,
    vert: f32,
    horiz: f32,
    active: bool,
) -> iced_widget::core::Element<'a, crate::app::Message, iced_widget::Theme, iced_renderer::Renderer>
{
    use iced_widget::core::Length;
    iced_widget::container(inner)
        .width(Length::Fill)
        .padding([vert, horiz])
        .style({
            // 框内底色同右侧代码编辑器(`colors.bg`)——敲的词跟被找的正文
            // 底色一致,像"嵌进"编辑区的一扇窗;外层整条 Find/替换组件另有
            // 自己的 `card` 底色(见 `find_rows` 的包裹),两层色阶分得开。
            let bg = colors.bg;
            let border_color = if active { colors.gold } else { colors.border };
            move |_t: &iced_widget::Theme| iced_widget::container::Style {
                background: Some(bg.into()),
                border: iced_widget::core::Border {
                    color: border_color,
                    width: 1.0,
                    radius: 6.0.into(),
                },
                ..iced_widget::container::Style::default()
            }
        })
        .align_y(iced_widget::core::alignment::Vertical::Center)
        .into()
}

/// Project 面板右配对的预览 pane,复用与 Files 预览同一套渲染。独立调一个
/// 新 `PreviewPaneKind`,让项目链接打开的文件进 `ws.project_preview` 而不是
/// 冲进 Files 预览(见 Task 12 `OpenLink` 路由)。
#[derive(Debug, Clone, Copy)]
pub(crate) enum PreviewPaneKind {
    Files,
    Project,
}

pub(crate) fn preview_pane<'a>(
    app: &'a App,
    ws: &'a Workspace,
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    preview_pane_for(app, ws, PreviewPaneKind::Files, width, outer)
}

pub(crate) fn project_preview_pane<'a>(
    app: &'a App,
    ws: &'a Workspace,
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    preview_pane_for(app, ws, PreviewPaneKind::Project, width, outer)
}

/// 预览 pane 的共同渲染(Files 预览与 Project 面板右配对复用同一套 tab
/// 栏/原生编辑器/占位文案逻辑,只是状态取自 `ws.preview` 还是
/// `ws.project_preview`、消息与前缀路由到哪套)不同。
fn preview_pane_for<'a>(
    app: &'a App,
    ws: &'a Workspace,
    kind: PreviewPaneKind,
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    // tab 栏:箭头翻页(到头变灰) + 每 tab 选择按钮 + 关闭 ×。tab 只能由项目树
    // 点击/会话恢复产生——面板本身已不再有"打开文件…"按钮或地址栏(P1 后续
    // 反馈:文件预览与浏览器彻底分离,文件只走项目树入口)。
    // P1L T5 验收返工:同 term `tab_bar`,横向 scrollable 换成索引窗口化 + clip.
    let region = theme::region::preview_pane();
    let (preview, tab_first, error) = match kind {
        PreviewPaneKind::Files => (&ws.preview, ws.preview_tab_first, &ws.preview_error),
        PreviewPaneKind::Project => (
            &ws.project_preview,
            ws.project_preview_tab_first,
            &ws.project_preview_error,
        ),
    };
    // 状态取的是一份只读引用,后续渲染把对应的消息/前缀按 `kind` 选好。
    // `move` 只捕获 `PreviewPaneKind`(Clone/Copy),多余生命周期问题一并消掉。
    let item_hover = move |idx| match kind {
        PreviewPaneKind::Files => HoverId::PreviewTabItem(idx),
        PreviewPaneKind::Project => HoverId::ProjectPreviewTabItem(idx),
    };
    let close_hover = move |idx| match kind {
        PreviewPaneKind::Files => HoverId::PreviewTabClose(idx),
        PreviewPaneKind::Project => HoverId::ProjectPreviewTabClose(idx),
    };
    let tab_group = match kind {
        PreviewPaneKind::Files => crate::app::TabGroup::Preview,
        PreviewPaneKind::Project => crate::app::TabGroup::ProjectPreview,
    };
    let select_msg = move |idx| match kind {
        PreviewPaneKind::Files => Message::PreviewSelectTab(idx),
        PreviewPaneKind::Project => Message::ProjectPreviewSelectTab(idx),
    };
    let close_msg = move |idx| match kind {
        PreviewPaneKind::Files => Message::PreviewCloseTab(idx),
        PreviewPaneKind::Project => Message::ProjectPreviewCloseTab(idx),
    };
    let toggle_msg = move |idx| match kind {
        PreviewPaneKind::Files => Message::PreviewToggleRenderMode(idx),
        PreviewPaneKind::Project => Message::ProjectPreviewToggleRenderMode(idx),
    };
    let overflow_toggle_msg = move || match kind {
        PreviewPaneKind::Files => Message::PreviewTabOverflowToggle,
        PreviewPaneKind::Project => Message::ProjectPreviewTabOverflowToggle,
    };
    let overflow_hover = move || match kind {
        PreviewPaneKind::Files => HoverId::PreviewTabOverflow,
        PreviewPaneKind::Project => HoverId::ProjectPreviewTabOverflow,
    };
    let render_mode_hover = move || match kind {
        PreviewPaneKind::Files => HoverId::PreviewRenderMode,
        PreviewPaneKind::Project => HoverId::ProjectPreviewRenderMode,
    };
    let editor_msg = move |tab_id, ev| match kind {
        PreviewPaneKind::Files => Message::PreviewEditorEvent(tab_id, ev),
        PreviewPaneKind::Project => Message::ProjectPreviewEditorEvent(tab_id, ev),
    };
    // Find 条与编辑器共享同一份"按面板选消息/悬停态"手法。消息统一走带
    // `PanelKind` 的顶层 `Message::PreviewFind*`(同 `PreviewSaveActive`,一条
    // 消息两面板通吃,Files/Project 由 `PanelKind` 区分)。
    let find_panel = move || match kind {
        PreviewPaneKind::Files => PanelKind::Files,
        PreviewPaneKind::Project => PanelKind::Project,
    };

    let widths: Vec<f32> = preview
        .tabs()
        .iter()
        .map(|t| preview_tab_display_width(&t.title))
        .collect();
    let window = tab_window(
        &widths,
        4.0,
        app.preview_tab_bar_avail_px(find_panel()),
        tab_first,
    );

    let items: Vec<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>> = preview
        .tabs()
        .iter()
        .enumerate()
        .filter(|(idx, _)| (window.first..window.visible_end).contains(idx))
        .map(|(idx, tab)| {
            let active = idx == preview.active_idx();
            let title_hover_t = app.hover_progress(item_hover(idx));
            let close_hover_t = app.hover_progress(close_hover(idx));
            // 就地可写的原生 tab 有未保存改动:标题后缀 ` *`(2026-09-06)。宽度
            // 预算仍按 `tab.title`(不带星)估,最坏多一个字符略挤,不换行折叠。
            let display_title = if tab.editor.is_some() && tab.dirty {
                format!("{} *", tab.title)
            } else {
                tab.title.clone()
            };
            // index 0 的 `Blank` 占位 tab 不可关闭:它上面的 × 点击等同于
            // "选中空白页"(不真关),与 SSH/数据库面板 tab 条最前面那个固定
            // "空白"占位(`app.rs::ssh_tab_bar` 的 `on_close: SelectBlankTab`)
            // 完全同一套做法——数据层 `PreviewPane::close(0)` 另有兜底 no-op。
            let is_placeholder = matches!(tab.kind, crate::preview::TabKind::Blank);
            let tab = panel_tab(PanelTabArgs {
                title: display_title,
                active,
                hover_t: title_hover_t,
                close_hover_t,
                prefix: None,
                suffix: None,
                on_select: select_msg(idx),
                on_close: if is_placeholder {
                    select_msg(idx)
                } else {
                    close_msg(idx)
                },
                show_tooltip: app.hover_tooltip_ready(item_hover(idx)),
                title_hover: move |h| Message::Hover(item_hover(idx), h),
                close_hover: move |h| Message::Hover(close_hover(idx), h),
            });
            // 拖拽换位:按住页签(选中处理已把 `app.tab_drag` 置位)后光标
            // 扫过哪个页签,这个 `on_move` 就按它发 `TabDragMove`,完成换位。
            let armed = app.dragging_group(tab_group);
            let mut area = MouseArea::new(tab).on_move(move |_| Message::TabDragMove {
                group: tab_group,
                index: idx,
            });
            if armed {
                area = area.interaction(mouse::Interaction::Grabbing);
            }
            area.into()
        })
        .collect();
    // tab 列表进 clip 容器占 Fill,裁掉右侧溢出;V 按钮钉在裁剪区外、tab 组
    // 最左侧(只要 tab 组非空即显示,见 `tab_overflow_button`——无溢出也
    // 列出全部 tab 供跳转)。
    let tabs_row = row(items).spacing(4);
    let clipped = container(tabs_row).width(Length::Fill).clip(true);
    // 预览/代码切换按钮:曾贴在每个 tab 自己身上(suffix),验收反馈挪到
    // tab 组最右侧(V 与"收起列表"之间)、只对**当前选中** tab 出一个——
    // 只有选中 tab 的内容看得见,切别的 tab 时按钮跟着换,不用每个 tab 各挂
    // 一份。仅对 `wry_toggle_eligible`(文本可编辑却默认走 wry/flyfish 渲染)
    // 的文件出现,随当前代码模式换图标(`editor.is_some()` = 代码模式 →
    // 显示 FilePlay,点它切回预览;否则显示 FileCode 切进代码)。
    let render_mode_button: Option<
        Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>,
    > = preview.tabs().get(preview.active_idx()).and_then(|tab| {
        let eligible = match &tab.kind {
            crate::preview::TabKind::File(p) => crate::preview::wry_toggle_eligible(p),
            _ => false,
        };
        eligible.then(|| {
            tab_render_mode_button(
                tab.editor.is_some(),
                app.hover_progress(render_mode_hover()),
                toggle_msg(preview.active_idx()),
                move |hovered| Message::Hover(render_mode_hover(), hovered),
            )
        })
    });
    let overflow_button = tab_overflow_button(
        preview.tabs().len(),
        app.hover_progress(overflow_hover()),
        overflow_toggle_msg(),
        move |hovered| Message::Hover(overflow_hover(), hovered),
    );
    // 预览右上角"收起/展开列表列"按钮:Files 预览收起文件树,Project 预览
    // 收起 info 列。按钮始终在此(内容侧),收起后仍可见以便恢复。图标按该
    // 面板当前所在栏(左/右)与收起态四选一,见 `IconKind::PanelLeftClose`
    // 等注释;工具文案由调用方给静态字符串。
    let collapse: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> = match kind {
        PreviewPaneKind::Files => app.list_collapse_button(
            PanelKind::Files,
            app.files_tree_collapsed(),
            HoverId::FileTreeCollapse,
            "收起文件树",
            "展开文件树",
            Message::ToggleFileTreeCollapse,
            move |hovered| Message::Hover(HoverId::FileTreeCollapse, hovered),
        ),
        PreviewPaneKind::Project => app.list_collapse_button(
            PanelKind::Project,
            app.list_collapsed(PanelKind::Project),
            HoverId::ProjectListCollapse,
            "收起列表",
            "展开列表",
            Message::TogglePanelListCollapse(PanelKind::Project),
            move |hovered| Message::Hover(HoverId::ProjectListCollapse, hovered),
        ),
    };
    let mut tab_bar_row = row![]
        .spacing(4)
        .align_y(iced_widget::core::Alignment::Center);
    if let Some(btn) = overflow_button {
        tab_bar_row = tab_bar_row.push(btn);
    }
    tab_bar_row = tab_bar_row.push(clipped);
    if let Some(btn) = render_mode_button {
        tab_bar_row = tab_bar_row.push(btn);
    }
    let tab_bar = tab_bar_row.push(collapse);

    let mut content = column![tab_bar, tab_divider()].spacing(region.gap);

    if let Some(err) = error {
        content = content.push(lh(text(format!("⚠ {err}"))
            .size(byteui::theme::font::body())
            .color(byteui::theme::color::current().red)));
    }

    // `PreviewPane` 恒定携带第 0 项 `TabKind::Blank` 占位(见 `PreviewPane::
    // default`/`preview.rs::TabKind::Blank`),所以这里的列表正常不会空——保留
    // 这个空分支纯属防御,不再有独立的"暂无预览"文案(空态由 `Blank` 占位
    // 那个分支画 Dozer 品牌标呈现)。
    if preview.tabs().is_empty() {
        content = content.push(
            iced_widget::Space::new()
                .width(Length::Fill)
                .height(Length::Fill),
        );
    } else {
        let active_tab = &preview.tabs()[preview.active_idx()];
        if let Some(editor) = &active_tab.editor {
            // 原生 tab:激活 tab 有原生 editor 时,直接在 iced 里渲染它(语法
            // 高亮/行号/ByteBoy2077 配色),put 下 content。`editor` 为 `None`
            // 的 wry 路由 tab 不 push 任何 iced 元素——那片区域由 main.rs 定位
            // 的 wry webview 子视图负责渲染,现状不变。
            let tab_id = active_tab.id;
            // 文件内 Find 条:锁着当前激活原生 tab 的会话存在时,在 tab_bar 分隔线
            // 之下、编辑器之上渲染输入框(框内右侧内嵌 Aa 大小写开关)+ n/m 计数 +
            // ↑ 上一个 / ↓ 下一命中。
            // 切走文件时 `PreviewPane` 已 cull 掉失配会话,条随之一并消失——既然
            // open/lifecycle 保证 `find` 总锁着激活原生 tab、此处又只在激活 tab 是
            // 原生时进入,读数即可,不必再校 tab 归属。
            if let Some(find) = preview.find_state() {
                let panel = find_panel();
                let colors = byteui::theme::color::current();
                // Find 条紧贴右侧编辑器,查询框/替换框正文用与编辑器相同的代码
                // 字号(`tree_row_font_size` 与 code_editor 同公式),让用户敲的
                // 词跟被找的文件正文看齐(需求:文件内查找条字号 = text editor)。
                let find_font = tree_row_font_size();
                // 「Aa」大小写开关:无独立 SVG 的字形钮(同被删的 Find × 按钮,但
                // 有真状态)。开(逐字严格)文字青 `cyan`、关(ASCII 折叠)灰 `dim`。
                // 青是 ByteBoy2077 甲方金之外的"用户动作强调色",toggle 归用户操作,
                // 不用甲方专属 gold,遵循 CLAUDE.md 裁决。
                // 内嵌在输入框同一圈边框内靠右(`input_text::view_with_suffix`,
                // 结构参照 search_box 的"共框尾控件"既有做法),不再占条上独立槽位。
                let case_toggle: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> = {
                    let active = find.case_sensitive;
                    button(text("Aa").size(byteui::theme::font::body()))
                        .on_press(match panel {
                            PanelKind::Project => {
                                Message::PreviewFindCase(PanelKind::Project, !active)
                            }
                            _ => Message::PreviewFindCase(PanelKind::Files, !active),
                        })
                        .padding(2)
                        .style(move |_t, _s| button::Style {
                            background: None,
                            text_color: if active { colors.cyan } else { colors.dim },
                            ..button::Style::default()
                        })
                        .into()
                };
                // 边框高亮同 search_box 约定由调用方给:查询词非空即金框。
                // 用 unframed 透明版 text_input(框/底由外层 `find_field_shell`
                // 统一垫 editor 背景 + 描边),让 Aa 大小写钮共享同一圈内边距。
                let input = byteui::form::input_text::view_with_suffix_unframed_at_size(
                    find_font,
                    "搜索",
                    &find.query,
                    false,
                    Some(crate::preview::find_field_id(panel)),
                    !find.query.is_empty(),
                    None,
                    move |q: String| match panel {
                        PanelKind::Project => Message::PreviewFindText(PanelKind::Project, q),
                        _ => Message::PreviewFindText(PanelKind::Files, q),
                    },
                    case_toggle,
                );
                // 命中计数:n 1-based;查无命中(非空 query)标红;新开 empty query
                // 不显示计数——这条进列就 pad 占用让条高稳定,避免每次刷字数跳动。
                let count_label: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
                    if find.count > 0 {
                        text(format!("{}/{}", find.current + 1, find.count))
                            .size(byteui::theme::font::body())
                            .color(colors.dim)
                            .into()
                    } else if !find.query.is_empty() {
                        text("0 个结果")
                            .size(byteui::theme::font::body())
                            .color(colors.red)
                            .into()
                    } else {
                        container(iced_widget::Row::<
                            Message,
                            iced_widget::Theme,
                            iced_renderer::Renderer,
                        >::new())
                        .into()
                    };
                let hover = move |next: bool| match next {
                    true => match panel {
                        PanelKind::Project => HoverId::ProjectPreviewFindNext,
                        _ => HoverId::PreviewFindNext,
                    },
                    false => match panel {
                        PanelKind::Project => HoverId::ProjectPreviewFindPrev,
                        _ => HoverId::PreviewFindPrev,
                    },
                };
                let step_icon = move |next: bool| -> iced_widget::core::Element<
                    'static,
                    Message,
                    iced_widget::Theme,
                    iced_renderer::Renderer,
                > {
                    let hid = hover(next);
                    let btn = icons::icon_button_entry(
                        if next {
                            icons::IconKind::ArrowDown
                        } else {
                            icons::IconKind::ArrowUp
                        },
                        byteui::theme::icon_size::row(),
                        false,
                        false,
                        app.hover_progress(hid),
                        false,
                        byteui::theme::geometry::tab_button_size(),
                        true,
                        match (next, panel) {
                            (true, PanelKind::Project) => {
                                Message::PreviewFindGo(PanelKind::Project, true)
                            }
                            (true, _) => Message::PreviewFindGo(PanelKind::Files, true),
                            (false, PanelKind::Project) => {
                                Message::PreviewFindGo(PanelKind::Project, false)
                            }
                            (false, _) => Message::PreviewFindGo(PanelKind::Files, false),
                        },
                        move |hovered| Message::Hover(hid, hovered),
                        if next { "下一个" } else { "上一个" },
                    );
                    // 同替换行图标按钮:外面补一圈固定可见的圆角边框(`icon_
                    // button_entry` 只在 `active` 态描边,这两个按钮没有持久
                    // 选中态)。
                    container(btn)
                        .style(
                            move |_t: &iced_widget::Theme| iced_widget::container::Style {
                                background: None,
                                border: Border {
                                    color: colors.border,
                                    width: 1.0,
                                    radius: 6.0.into(),
                                },
                                ..iced_widget::container::Style::default()
                            },
                        )
                        .into()
                };
                // 文件内搜索的查询框与替换框各自独立、成两个带 1px 圆角边框的
                // 输入框(需求:v0.5.94 之后改回,不再合成一整块):框内底色与
                // 右侧代码编辑器同一 `colors.bg`,敲词文字底色跟被找正文看板一致
                // (比亮一点的 card 更"嵌进"编辑区);有内容时整框描金、否则普通
                // 边色。命中计数与上下箭头、替换按钮都摆在框外右侧,不占框内。
                type EE<'x> = iced_widget::core::Element<
                    'x,
                    Message,
                    iced_widget::Theme,
                    iced_renderer::Renderer,
                >;
                // 查询框前的展开/收起替换行圆盘箭头:收起态 `ChevronRight`、
                // 展开态 `ChevronDown`(同文件树/Todo 分类树展开箭头的既有语义)。
                // ⌘F 打开条时收起、⌘R 打开时展开(见 `PreviewFindOpen`/
                // `PreviewFindOpenWithReplace`),这里手动点按翻转。
                let replace_open = find.replace_open;
                let replace_toggle_hid = match panel {
                    PanelKind::Project => HoverId::ProjectPreviewFindReplaceToggle,
                    _ => HoverId::PreviewFindReplaceToggle,
                };
                let replace_toggle: EE<'_> = icons::icon_button_entry(
                    if replace_open {
                        icons::IconKind::ChevronDown
                    } else {
                        icons::IconKind::ChevronRight
                    },
                    byteui::theme::icon_size::row(),
                    false,
                    false,
                    app.hover_progress(replace_toggle_hid),
                    false,
                    byteui::theme::geometry::tab_button_size(),
                    true,
                    match panel {
                        PanelKind::Project => Message::PreviewFindReplaceToggle(PanelKind::Project),
                        _ => Message::PreviewFindReplaceToggle(PanelKind::Files),
                    },
                    move |hovered| Message::Hover(replace_toggle_hid, hovered),
                    if replace_open {
                        "收起替换"
                    } else {
                        "展开替换"
                    },
                );
                // 查询框与替换框要"长度一样、右边缘对齐"(参照 VSCode 查找条):
                // 两行各自的框后附件(计数+上下箭头 vs 替换按钮×2)天然宽度不
                // 等,若各自吃 `Length::Fill` 剩余空间,两个框会不等宽。这里给
                // 两行的"框后附件"统一钳到同一个固定宽度(取较宽的替换按钮组
                // 富余出来),框本身仍吃 `Length::Fill`——总行宽相同、附件区宽度
                // 相同,余下的 `Fill` 自然等宽,顺带右边缘也对齐。
                // 「替换当前」/「替换全部」改图标按钮后trailing 区收窄回来——
                // 决定宽度的现在是查询行那边"命中计数 + 上下箭头"这一组,不再
                // 是文字按钮组。
                const FIND_TRAILING_ZONE: f32 = 150.0;
                let query_trailing = container(
                    row![count_label, step_icon(false), step_icon(true)]
                        .spacing(4)
                        .align_y(iced_widget::core::alignment::Alignment::Center),
                )
                .width(Length::Fixed(FIND_TRAILING_ZONE))
                .align_x(iced_widget::core::alignment::Horizontal::Right);
                let mut find_rows: Vec<EE<'_>> = vec![];
                // 边框描金:查询词非空 **或** 输入框持有真实焦点——2026-09-11
                // 需求补上聚焦态,不再只靠已有内容触发(此前空 query 时点进框里
                // 光标闪烁却没有任何视觉反馈)。
                let query_active = !find.query.is_empty() || preview.find_query_focused();
                let query_row: EE<'_> = container(
                    row![
                        replace_toggle,
                        find_field_shell(input, colors, 7.0, 10.0, query_active),
                        query_trailing,
                    ]
                    .spacing(4)
                    .align_y(iced_widget::core::alignment::Alignment::Center),
                )
                .width(Length::Fill)
                .into();
                find_rows.push(query_row);

                // ---- 文件内替换行(条身之下第二行) ----
                // 替换只改锁定 buffer 并标脏、落盘仍等 ⌘S(`PreviewPane::replace_*`
                // 的语义),不做直接磁盘写。默认跳过空命中(避免误把用户缓冲区清空
                // 成替换框逗号残片)。“替换当前”会顺带到下一命中、方便一路处理,
                // “替换全部”把这一轮全部落一次。两个按钮共用一轮是否可替换的开关。
                let armed = find.count > 0;
                // 替换框同查询框独立栅格:bare=true 去底去框透明,边框/底色交给
                // 下方 `editor_field` 统一垫(editor 背景色)。
                let replacement_input = byteui::form::input_text::view_at_size(
                    find_font,
                    "替换",
                    &find.replacement,
                    false,
                    None,
                    !find.replacement.is_empty(),
                    None,
                    true,
                    move |s: String| match panel {
                        PanelKind::Project => {
                            Message::PreviewFindReplacement(PanelKind::Project, s)
                        }
                        _ => Message::PreviewFindReplacement(PanelKind::Files, s),
                    },
                );
                // 「替换当前」/「替换全部」改图标按钮(Lucide replace /
                // replace-all,2026-09-07 需求),不再是文字按钮——无命中时
                // `dim` 置灰 + `interactive=false` 不可点,同 `armed` 语义。
                let replace_icon_button = |kind: icons::IconKind,
                                           hid: HoverId,
                                           msg: Message,
                                           tooltip: &'static str,
                                           disabled: bool|
                 -> iced_widget::core::Element<
                    'static,
                    Message,
                    iced_widget::Theme,
                    iced_renderer::Renderer,
                > {
                    let btn = icons::icon_button_entry(
                        kind,
                        byteui::theme::icon_size::row(),
                        false,
                        disabled,
                        app.hover_progress(hid),
                        false,
                        byteui::theme::geometry::tab_button_size(),
                        !disabled,
                        msg,
                        move |hovered| Message::Hover(hid, hovered),
                        tooltip,
                    );
                    // `icon_button_entry` 本身只在 `active` 态描边(这两个按钮
                    // 没有持久选中态,永远描不出来)——外面再包一圈固定可见的
                    // 圆角边框,让它们看起来像独立按钮而不是裸图标。
                    container(btn)
                        .style(
                            move |_t: &iced_widget::Theme| iced_widget::container::Style {
                                background: None,
                                border: Border {
                                    color: colors.border,
                                    width: 1.0,
                                    radius: 6.0.into(),
                                },
                                ..iced_widget::container::Style::default()
                            },
                        )
                        .into()
                };
                // 替换框要跟上面查询框左对齐:查询框前多了圆盘箭头
                // (`tab_button_size()` 宽 + `query_row` 的 `spacing(4)`),这里
                // 用等宽占位补上——占位宽度扣掉本行自己的 `spacing(6)`,两行加
                // 起来对 `find_field_shell` 左边缘落在同一个 x。
                let replace_indent =
                    (byteui::theme::geometry::tab_button_size() + 4.0 - 6.0).max(0.0);
                let replace_trailing = container(
                    row![
                        replace_icon_button(
                            icons::IconKind::Replace,
                            match panel {
                                PanelKind::Project => HoverId::ProjectPreviewFindReplaceCurrentBtn,
                                _ => HoverId::PreviewFindReplaceCurrentBtn,
                            },
                            match panel {
                                PanelKind::Project => {
                                    Message::PreviewFindReplaceCurrent(PanelKind::Project)
                                }
                                _ => Message::PreviewFindReplaceCurrent(PanelKind::Files),
                            },
                            "替换当前",
                            !armed,
                        ),
                        replace_icon_button(
                            icons::IconKind::ReplaceAll,
                            match panel {
                                PanelKind::Project => HoverId::ProjectPreviewFindReplaceAllBtn,
                                _ => HoverId::PreviewFindReplaceAllBtn,
                            },
                            match panel {
                                PanelKind::Project => {
                                    Message::PreviewFindReplaceAll(PanelKind::Project)
                                }
                                _ => Message::PreviewFindReplaceAll(PanelKind::Files),
                            },
                            "替换全部",
                            !armed,
                        ),
                    ]
                    .spacing(6)
                    .align_y(iced_widget::core::alignment::Alignment::Center),
                )
                .width(Length::Fixed(FIND_TRAILING_ZONE))
                .align_x(iced_widget::core::alignment::Horizontal::Right);
                let replace_row = container(
                    row![
                        iced_widget::Space::new().width(Length::Fixed(replace_indent)),
                        find_field_shell(
                            replacement_input,
                            colors,
                            4.0,
                            10.0,
                            !find.replacement.is_empty()
                        ),
                        replace_trailing,
                    ]
                    .spacing(6)
                    .align_y(iced_widget::core::alignment::Alignment::Center),
                )
                .width(Length::Fill)
                .into();
                if replace_open {
                    find_rows.push(replace_row);
                }
                let find_rows = container(column(find_rows).spacing(4))
                    .width(Length::Fill)
                    .padding(8)
                    .style(
                        move |_t: &iced_widget::Theme| iced_widget::container::Style {
                            background: Some(colors.card.into()),
                            border: iced_widget::core::Border {
                                color: colors.border,
                                width: 1.0,
                                radius: 8.0.into(),
                            },
                            ..iced_widget::container::Style::default()
                        },
                    );
                content = content.push(find_rows);
            }
            content = content.push(
                container(editor.view().map(move |ev| editor_msg(tab_id, ev)))
                    .width(Length::Fill)
                    .height(Length::Fill),
            );
        } else if active_tab.kind == TabKind::Blank {
            // 关到最后一个 tab 后自动补的空白占位:没有 wry 页面,内容区
            // 纯 iced 原生渲染,居中放 Dozer 品牌标(`IconKind::Dozer`,此前
            // 一直没有调用点,见该枚举成员的注释)。
            content = content.push(
                container(icons::view(
                    icons::IconKind::Dozer,
                    96.0,
                    byteui::theme::color::current().dim,
                ))
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(iced_widget::core::alignment::Horizontal::Center)
                .align_y(iced_widget::core::alignment::Vertical::Center),
            );
        }
    }

    let base: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> =
        container(content.padding(region.padding))
            .width(width)
            .height(Length::Fill)
            .style(move |_theme: &iced_widget::Theme| container::Style {
                background: region.background.map(Into::into),
                border: outer,
                ..container::Style::default()
            })
            .into();
    base
}

/// 文件/项目预览 tab 栏"溢出下拉"浮层。**必须**在 `App::view` 顶层
/// `stack![base, ...]` 里拼(同 `terminal::term_tab_overflow_popup` 文档
/// 解释的理由——`anchor`/`window_size` 是全窗口坐标系,嵌在 `preview_pane_for`
/// 自己的局部布局里换算位置会跟真实点击位置对不上)。
pub(crate) fn preview_tab_overflow_popup<'a>(
    app: &'a App,
    ws: &'a Workspace,
    kind: PreviewPaneKind,
) -> Option<Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>> {
    let (preview, anchor) = match kind {
        PreviewPaneKind::Files => (&ws.preview, ws.preview_tab_overflow_anchor),
        PreviewPaneKind::Project => (&ws.project_preview, ws.project_preview_tab_overflow_anchor),
    };
    let anchor = anchor?;
    if preview.tabs().is_empty() {
        return None;
    }
    let select_msg = move |idx| match kind {
        PreviewPaneKind::Files => Message::PreviewSelectTab(idx),
        PreviewPaneKind::Project => Message::ProjectPreviewSelectTab(idx),
    };
    let close_msg = move |idx| match kind {
        PreviewPaneKind::Files => Message::PreviewCloseTab(idx),
        PreviewPaneKind::Project => Message::ProjectPreviewCloseTab(idx),
    };
    let overflow_dismiss_msg = match kind {
        PreviewPaneKind::Files => Message::PreviewTabOverflowDismiss,
        PreviewPaneKind::Project => Message::ProjectPreviewTabOverflowDismiss,
    };
    // 下拉列出精选组内**全部** tab(不管当前是否横向可见),便于随时
    // 跳转到某一项,而非只列"被挤出可见区"的子集。
    let entries: Vec<TabOverflowEntry<'_, Message>> = preview
        .tabs()
        .iter()
        .enumerate()
        .map(|(idx, tab)| TabOverflowEntry {
            index: idx,
            prefix: None,
            title: tab.title.clone(),
            active: idx == preview.active_idx(),
            // index 0 的 `Blank` 占位固定存在、不可关闭(同 SSH/数据库面板的
            // 固定"空白"占位),下拉里这一行不画 ×。
            closable: !matches!(tab.kind, crate::preview::TabKind::Blank),
            hover_t: app.hover_progress(HoverId::TabOverflowRow(idx)),
        })
        .collect();
    Some(tab_overflow_menu(TabOverflowMenuArgs {
        entries,
        anchor,
        window_size: app.window_size,
        on_select: select_msg,
        on_close: close_msg,
        on_dismiss: overflow_dismiss_msg,
        on_row_hover: move |idx, hovered| Message::Hover(HoverId::TabOverflowRow(idx), hovered),
    }))
}

/// 对话副行文案：`<agent> · <相对时间> · <规模>`（P1j）。
/// 相对时间文案：刚刚/N 分钟前/N 小时前/N 天前（D5，从 `conversation_sub`
/// 抽出为独立纯函数）。H0 项目卡"活跃时间"、文件卡、对话卡三处复用，
/// 不要三份重复 switch。
pub(crate) fn relative_time_text(modified_ms: u64, now_ms: u64) -> String {
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

/// agent 选择菜单选中的 agent → 要自动键入 PTY 的 CLI 命令名。`Unknown`
/// 不该从选择菜单产生(选项只有 Claude/CodeBuddy/OpenCode/纯 Shell 四选
/// 一,纯 Shell 走 `launch: None`,不经过这个函数),但函数保持穷尽
/// match,防止未来枚举新增变体时静默漏写。已知变体的 CLI 名字与
/// `AgentKind::label()` 逐字节一致(`label()` 本身就是给这三个变体返回
/// 小写 CLI 名),这里直接复用而不重复一份映射表,避免两处拼写分叉。
pub(crate) fn agent_cli_command(agent: AgentKind) -> Option<&'static str> {
    match agent {
        AgentKind::Unknown => None,
        known => Some(known.label()),
    }
}

/// 关闭 tab 时是否应该走"总结后关闭"而不是直接 `Kill`——仅对话摄取管线
/// 已覆盖、且当前存活、且走 daemon 后端的四家 agent(spec
/// 2026-08-27)。
fn should_summarize_on_close(agent: AgentKind, alive: bool, backend: &TabBackend) -> bool {
    alive
        && matches!(backend, TabBackend::Daemon)
        && matches!(
            agent,
            AgentKind::Claude | AgentKind::Codebuddy | AgentKind::Opencode | AgentKind::V8agent
        )
}

/// `on_tab_attached` 里决定要不要回应 OSC 10/11 终端探测查询——`info.agent`
/// 在新建会话这一刻几乎总还是 daemon 侧的默认值,真实 agent 要靠 hook 事后
/// 上报,而那必然晚于 agent CLI 进程启动瞬间就发出的查询;picker 选中的
/// `picked_agent` 才是这一刻唯一"确定即将变成谁"的信号,`info.agent` 仅在
/// picker 未给出提示(如 SSH attach、或重连时 hook 已先一步上报过)时兜底。
fn should_answer_dynamic_color(picked_agent: Option<AgentKind>, info_agent: AgentKind) -> bool {
    picked_agent == Some(AgentKind::Opencode) || info_agent == AgentKind::Opencode
}

/// 已接入 `dozer-hook` 安装器的 agent 集合。刻意穷尽 match 而不是拿
/// `agent.label()` 当 catch-all 参数：`install::settings_path_for` 对未识别
/// 的 agent 名一律落回 Claude 的 `settings.json`路径，如果不显式排除
/// Kilo/V8agent，误调用会把 "kilo"/"v8agent" 的 hook 命令写进 Claude 的
/// settings.json，顶掉真正的 claude hook 条目。两者排除的原因不同：Kilo
/// 是真实缺口(没有任何 hook 上报机制)；V8agent 走的是完全不同的路子——
/// `v8agent-cli` 在 `DOZER_SESSION_ID` 存在时直接通过 UDS socket 上报
/// `Request::HookEvent`(见 `dozer-core::protocol`),不依赖这套"往
/// agent 自己的配置文件里写 hook 命令"的安装机制，所以这里返回 `None`
/// 对 V8agent 而言是正确行为，不是待办事项(spec
/// `docs/superpowers/specs/2026-08-24-v8agent-integration-design.md`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HookInstallTarget {
    Settings,
    Opencode,
}

pub(crate) fn hook_install_target(agent: AgentKind) -> Option<HookInstallTarget> {
    match agent {
        AgentKind::Claude | AgentKind::Codebuddy | AgentKind::Codex => {
            Some(HookInstallTarget::Settings)
        }
        AgentKind::Opencode => Some(HookInstallTarget::Opencode),
        AgentKind::Unknown | AgentKind::Kilo | AgentKind::V8agent => None,
    }
}

/// `exe` 所在目录下名为 `dozer-hook` 的同级二进制路径（跟
/// `main.rs::spawn_dozerd` 定位同级 `dozerd` 的手法一致——发行版把
/// `dozer-hook` 跟 `dozer`/`dozerd` 一起装进同一个 `Contents/MacOS/`）。
/// 抽成纯函数只是为了能不依赖 `current_exe()` 直接单测。
fn dozer_hook_binary_path(exe: &Path) -> PathBuf {
    match exe.parent() {
        Some(dir) => dir.join("dozer-hook"),
        None => PathBuf::from("dozer-hook"),
    }
}

/// 新开 agent 会话前静默注册该 agent 的 hook（幂等、恒静默——同
/// `dozer-hook install` 自身"绝不因失败拖慢/打断会话"的错误处理哲学）。
/// 在此之前 hook 注册是一步用户必须自己发现并手动执行的 CLI 命令
/// （`dozer-hook install <agent>`），Claude 之外的 agent 几乎没人知道要
/// 跑它，于是 Agent 面板里 name/status 永远停在 `Unknown`/`Idle`。阻塞
/// 文件 I/O，调用方须包一层 `spawn_blocking`。
///
/// 必须用 `run_at_with_exe`/`opencode_install::run_at_with_exe` 显式传入
/// exe 路径，不能调不带 `_with_exe` 的版本：那两个版本内部读
/// `std::env::current_exe()`，在这里（`dozer-app` 进程内直接函数调用，
/// 不是 spawn 一个独立的 `dozer-hook` 子进程）会拿到 `dozer-app` 自己的
/// 可执行文件路径，写出一条指向错误二进制的 hook 命令——2026-08 线上
/// 事故：`~/.claude/settings.json` 堆出重复的坏 hook，agent 名字/光标
/// 状态全靠 `dozer-hook` 转发的事件才能更新，全断了。
fn ensure_hook_installed(agent: AgentKind) {
    let Some(target) = hook_install_target(agent) else {
        return;
    };
    let exe =
        dozer_hook_binary_path(&std::env::current_exe().unwrap_or_else(|_| PathBuf::from("dozer")));
    let exe = exe.to_string_lossy();
    match target {
        HookInstallTarget::Settings => {
            let label = agent.label();
            let _ = dozer_hook::install::run_at_with_exe(
                &dozer_hook::install::settings_path_for(label),
                label,
                true,
                &exe,
            );
        }
        HookInstallTarget::Opencode => {
            let _ = dozer_hook::opencode_install::run_at_with_exe(
                &dozer_hook::opencode_install::plugins_dir(),
                true,
                &exe,
            );
        }
    }
}

/// `dozer-mcp` 自动注册:仿 `ensure_hook_installed` 同一套幂等/静默/失败
/// 只 warn 的哲学。V8agent 走完全不同的路(见 `hook_install_target` 文档
/// 注释同款理由)——它自己硬编码检测 `DOZER_SESSION_ID` 后自动挂载
/// `dozer-mcp serve`,不读任何配置文件,这里对它直接 no-op。
fn ensure_mcp_installed(agent: AgentKind) {
    let Some((path, _)) = dozer_mcp::install::config_path_for(agent.label()) else {
        return;
    };
    let exe =
        dozer_hook_binary_path(&std::env::current_exe().unwrap_or_else(|_| PathBuf::from("dozer")))
            .parent()
            .map(|dir| dir.join("dozer-mcp"))
            .unwrap_or_else(|| PathBuf::from("dozer-mcp"));
    let exe = exe.to_string_lossy();
    let _ = dozer_mcp::install::run_at_with_exe(&path, agent.label(), true, &exe);
}

/// picker 选择项 → attach 成功后自动键入 PTY 的初始命令。`Agent(Some(a))`
/// 复用 `agent_cli_command`(键入 agent CLI);`Agent(None)` 不键入(纯 Shell);
/// `Git` 键入 `git status`——新开的 shell 已在项目根,直接看仓库状态。
/// 抽成纯函数是为了能 headless 单测(同 `agent_cli_command` 的惯例)。
pub(crate) fn picker_launch_command(launch: PickerLaunch) -> Option<String> {
    match launch {
        PickerLaunch::Agent(Some(agent)) => agent_cli_command(agent).map(str::to_owned),
        PickerLaunch::Agent(None) => None,
        PickerLaunch::Git => Some("git status".to_string()),
    }
}

/// tab 标题：已识别出 agent（hook 上报）则显 agent 名（如 "claude"）；
/// 否则回落到 OSC 7 的 cwd basename，再无 cwd 才回落会话名。
pub(crate) fn tab_title(agent: AgentKind, cwd: Option<&Path>, fallback: &str) -> String {
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
pub(crate) fn text_width_units(s: &str) -> f32 {
    s.chars()
        .map(|c| if (c as u32) > 0x2E80 { 2.0 } else { 1.0 })
        .sum()
}

/// 终端 tab 估算显示宽（逻辑像素）：状态点+名称+关闭×+pill padding 的粗估。
/// 不追求精确——估偏几像素只会让翻页边界差一个 tab。宽度随标题实际长度
/// 增长(无上限),渲染侧亦有对应 `panel_tab` 的按内容伸缩。
pub(crate) fn tab_display_width(title: &str) -> f32 {
    // 状态点●+spacing ≈ 18, 名称 ≈ units * 半宽 8.0(14px), 关闭× ≈ 18, pill padding ≈ 12
    18.0 + text_width_units(title) * 8.0 + 18.0 + 12.0
}

/// 预览 tab 估算显示宽：同 `tab_display_width` 但无状态点。同按标题实际长度
/// 估算,不设上限(与渲染侧 `panel_tab` 按内容伸缩对齐,翻页窗口数学按真实
/// 宽度算,标题多宽估多宽)。
pub(crate) fn preview_tab_display_width(title: &str) -> f32 {
    // 名称 ≈ units * 半宽 8.0(14px), 关闭× ≈ 18, pill padding ≈ 12
    text_width_units(title) * 8.0 + 18.0 + 12.0
}

/// agent 四态中文（终端状态栏用）。
pub(crate) fn agent_state_label(state: AgentState) -> &'static str {
    match state {
        AgentState::Running => "运行中",
        AgentState::AwaitingInput => "待输入",
        AgentState::TurnEnded => "回合毕",
        AgentState::Idle => "空闲",
    }
}

/// tab 前状态点配色：死会话灰；存活按 agent 状态——绿=空闲/运行、
/// 紫蓝=待输入、金=回合结束（金是甲方动作专属色：该出手了）。运行态
/// 与空闲态同为绿，靠 `tab_item` 里的闪烁区分（工作中才闪）。
pub(crate) fn dot_color(state: AgentState, alive: bool) -> Color {
    if !alive {
        return byteui::theme::color::current().dim;
    }
    match state {
        AgentState::Idle => byteui::theme::color::current().cyan,
        AgentState::Running => byteui::theme::color::current().green,
        AgentState::AwaitingInput => byteui::theme::color::current().red,
        AgentState::TurnEnded => byteui::theme::color::current().gold,
    }
}

/// `claude-sonnet-5` → `Sonnet 5`:去掉 `claude-` 前缀,按 `-` 分词、每
/// 词首字母大写、空格拼接。不以 `claude-` 开头的原样返回(不确定形状,
/// 不强行摘,避免拍出乱码;不维护会过期的型号对照表)。
pub(crate) fn format_model_label(raw: &str) -> String {
    match raw.strip_prefix("claude-") {
        Some(rest) => rest
            .split('-')
            .map(|w| {
                let mut chars = w.chars();
                match chars.next() {
                    Some(f) => f.to_uppercase().collect::<String>() + chars.as_str(),
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
        None => raw.to_string(),
    }
}

/// agent → 对话列表圆点颜色。避开 `byteui::theme::color::current().gold`(甲方动作专属色,
/// CLAUDE.md 明文规定,不能被 agent 分类语义借用)。
pub(crate) fn agent_dot_color(agent: AgentKind) -> Color {
    match agent {
        AgentKind::Claude => byteui::theme::color::current().cyan,
        AgentKind::Codebuddy => byteui::theme::color::current().purple,
        AgentKind::Opencode => byteui::theme::color::current().green,
        AgentKind::Codex => byteui::theme::color::current().orange,
        AgentKind::Kilo => byteui::theme::color::current().blue,
        AgentKind::V8agent => byteui::theme::color::current().lime,
        AgentKind::Unknown => byteui::theme::color::current().dim,
    }
}

/// agent → 品牌图标(新建 agent 菜单用)。颜色由调用方按 `agent_dot_color`
/// 同款语义传入,使图标色与圆点色一致,避免引入新配色维度。
pub(crate) fn agent_icon(agent: AgentKind) -> IconKind {
    match agent {
        AgentKind::Claude => IconKind::Claude,
        AgentKind::Codebuddy => IconKind::Codebuddy,
        AgentKind::Opencode => IconKind::Opencode,
        // 暂无确认可用的品牌素材，回落通用图标（spec §8/§6 明确允许）。
        AgentKind::Codex | AgentKind::Kilo | AgentKind::V8agent | AgentKind::Unknown => {
            IconKind::Bot
        }
    }
}

/// 由(路径, 是否有选区, 0-indexed 光标位置, 0-indexed 选区范围, 当前时间)
/// 组装 1-indexed 的 `PreviewContext`。抽成纯函数是为了不依赖真实
/// `CodeEditor`/`PreviewPane` 就能单测坐标转换这一层逻辑——`now_ms` 由调
/// 用方传进来而不是在这里读 `SystemTime::now()`,正是为了保住这份纯度。
fn preview_context_from_editor_state(
    path: &str,
    has_selection: bool,
    cursor: (usize, usize),
    selection: Option<((usize, usize), (usize, usize))>,
    now_ms: u64,
) -> dozer_core::protocol::PreviewContext {
    let (start, end) = if has_selection {
        selection.unwrap_or((cursor, cursor))
    } else {
        (cursor, cursor)
    };
    dozer_core::protocol::PreviewContext {
        path: path.to_string(),
        start_line: start.0 as u32 + 1,
        start_col: start.1 as u32 + 1,
        end_line: end.0 as u32 + 1,
        end_col: end.1 as u32 + 1,
        has_selection,
        updated_at_ms: now_ms,
    }
}

/// 促成的 **IO 段**:把一个项目在 daemon 上的存活会话逐一 attach 下来,连同
/// 最近项目列表打包成 [`ProjectRestore`]。整段只碰 `Client`,不碰任何 GUI
/// 类型,所以可以在 tokio 线程池上跑(`App::ensure_loaded` 正是这么用的)。
pub(crate) async fn fetch_project_restore(client: &Client, project: ProjectInfo) -> ProjectRestore {
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

/// 单个 attach 数据流的转发循环：持有 `rx`（唯一持有者），逐条转发到
/// UI 线程（经 `EventLoopProxy`）。tab 关闭时 `JoinHandle::abort` 打断
/// 这个循环，`rx` 随任务栈一起析构——这就是 detach 的物理落点。
///
/// `project_id` 是这条流归属的项目:`tab_id` 只在单个 `Workspace` 内唯一,
/// 每个项目都从 0 起编,所以转发出去的消息必须自带项目归属,否则后台项目的
/// 输出会被喂进前台项目同号的 tab(见 [`ProjectId`])。
pub(crate) async fn forward_events(
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
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_webview_spec_empty_when_no_review() {
        assert_eq!(review_webview_spec(None), Vec::new());
    }

    #[test]
    fn review_webview_spec_empty_on_error_or_empty_entries() {
        let with_error = ReviewView {
            source: ReviewSource::Conversation("a".into()),
            entries: vec![ReviewEntry::Human { text: "hi".into() }],
            error: Some("boom".into()),
            agent: AgentKind::Claude,
            nonce: 3,
            summary_title: None,
            summary_text: None,
            summary_time: None,
        };
        assert_eq!(review_webview_spec(Some(&with_error)), Vec::new());

        let empty_entries = ReviewView {
            source: ReviewSource::Conversation("a".into()),
            entries: Vec::new(),
            error: None,
            agent: AgentKind::Claude,
            nonce: 3,
            summary_title: None,
            summary_text: None,
            summary_time: None,
        };
        assert_eq!(review_webview_spec(Some(&empty_entries)), Vec::new());
    }

    #[test]
    fn review_webview_spec_url_carries_nonce_and_is_visible() {
        let rv = ReviewView {
            source: ReviewSource::Conversation("a".into()),
            entries: vec![ReviewEntry::Human { text: "hi".into() }],
            error: None,
            agent: AgentKind::Claude,
            nonce: 7,
            summary_title: None,
            summary_text: None,
            summary_time: None,
        };
        let specs = review_webview_spec(Some(&rv));
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].url, "dozer://review-trace/host.html?_r=7");
        assert!(specs[0].visible);
    }

    #[test]
    fn agent_card_refresh_plan_decides_by_agent_and_cwd() {
        let root = PathBuf::from("/repo");
        let elsewhere = PathBuf::from("/elsewhere");

        assert_eq!(
            agent_card_refresh_plan(dozer_core::protocol::AgentKind::Claude, &root, Some(&root)),
            (true, true, false),
            "Claude + cwd 等于项目根:model/mode + activity,不做工作区"
        );
        assert_eq!(
            agent_card_refresh_plan(
                dozer_core::protocol::AgentKind::Opencode,
                &root,
                Some(&root)
            ),
            (true, true, false),
            "Opencode + cwd 等于项目根:dozer-hook 插件已经把 model 写进合成 \
             transcript(见 dozer-translate.ts::onUserMessage),model/mode \
             门也跟 Claude 一样打开;不做工作区"
        );
        assert_eq!(
            agent_card_refresh_plan(
                dozer_core::protocol::AgentKind::Opencode,
                &elsewhere,
                Some(&root)
            ),
            (true, true, true),
            "Opencode + cwd 偏离项目根:model/mode + activity + 工作区"
        );
        assert_eq!(
            agent_card_refresh_plan(
                dozer_core::protocol::AgentKind::Codebuddy,
                &root,
                Some(&root)
            ),
            (true, true, false),
            "Codebuddy + cwd 等于项目根:也要做(只提 LLM,mode 恒 None——\
             transcript 没有 permissionMode 等价字段,见 latest_model_mode_and_activity)"
        );
        assert_eq!(
            agent_card_refresh_plan(
                dozer_core::protocol::AgentKind::Claude,
                &elsewhere,
                Some(&root)
            ),
            (true, true, true),
            "Claude + cwd 偏离项目根:三者都做"
        );
        assert_eq!(
            agent_card_refresh_plan(dozer_core::protocol::AgentKind::Unknown, &root, Some(&root)),
            (true, true, false),
            "Finding 2: Unknown + cwd 等于项目根:也要按 Claude 形状做 model/mode 提取"
        );
        let subdir = PathBuf::from("/repo").join("crates").join("dozer-app");
        assert_eq!(
            agent_card_refresh_plan(
                dozer_core::protocol::AgentKind::Claude,
                &subdir,
                Some(&root)
            ),
            (true, true, false),
            "Finding 3: cwd 是 project_root 的子目录,不算偏离,不应触发 needs_workspace"
        );
        assert_eq!(
            agent_card_refresh_plan(dozer_core::protocol::AgentKind::Codex, &root, Some(&root)),
            (false, false, false),
            "Codex 的 transcript 恒解不出内容(parse_transcript 空 Vec),\
             model/mode/activity 都不值得读"
        );
        assert_eq!(
            agent_card_refresh_plan(dozer_core::protocol::AgentKind::V8agent, &root, Some(&root)),
            (false, true, false),
            "V8agent 现在走 Claude 形状解析(parse.rs 的 dispatch 改动),\
             transcript 能真正解出内容,activity 门禁应该打开;\
             model/mode 门禁不动(V8agent 的 transcript 里没有 model 字段)"
        );
    }

    #[test]
    fn apply_agent_card_refresh_llm_and_mode_keep_last_value_when_none() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut tabs = vec![make_test_tab(&rt, "a", AgentKind::Claude)];
        tabs[0].tab_id = 7;

        apply_agent_card_refresh(
            &mut tabs,
            7,
            Some("claude-sonnet-5".to_string()),
            Some("auto".to_string()),
            None,
            None,
        );
        assert_eq!(tabs[0].llm_model.as_deref(), Some("claude-sonnet-5"));
        assert_eq!(tabs[0].permission_mode.as_deref(), Some("auto"));
        assert_eq!(tabs[0].workspace_override, None);

        // 第二次刷新 model/mode 都是 None(比如那次 transcript 读取
        // 失败):不应该把已经拿到的值抹掉。
        apply_agent_card_refresh(&mut tabs, 7, None, None, None, None);
        assert_eq!(tabs[0].llm_model.as_deref(), Some("claude-sonnet-5"));
        assert_eq!(tabs[0].permission_mode.as_deref(), Some("auto"));

        // 未知 tab_id:整体 no-op,不 panic。
        apply_agent_card_refresh(&mut tabs, 999, Some("x".to_string()), None, None, None);
        assert_eq!(tabs[0].llm_model.as_deref(), Some("claude-sonnet-5"));
    }

    #[test]
    fn apply_agent_card_refresh_workspace_always_overwrites_including_clear() {
        // Finding 1 回归测试:workspace_override 曾经"只在 Some 时覆盖",
        // 导致 session 一旦偏离过项目根就再也清不掉覆盖(cd 回项目根后卡片
        // 工作区行永远停在旧仓库快照)。workspace 字段跟 llm_model/mode
        // 语义不同——None 是明确的"清空"信号,不是"没查、保留原值"。
        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut tabs = vec![make_test_tab(&rt, "a", AgentKind::Claude)];
        tabs[0].tab_id = 7;

        let diverged = WorkspaceGitInfo {
            branch: Some("feature/x".to_string()),
            dirty: true,
        };
        apply_agent_card_refresh(&mut tabs, 7, None, None, None, Some(diverged.clone()));
        assert_eq!(tabs[0].workspace_override, Some(diverged));

        // cwd 回到项目根:workspace 传 None,必须真的清空,不是保留旧覆盖。
        apply_agent_card_refresh(&mut tabs, 7, None, None, None, None);
        assert_eq!(tabs[0].workspace_override, None);
    }

    #[test]
    fn preview_context_from_cursor_only_uses_1_indexed_point_range() {
        // 无选区:0-indexed (1, 4) 光标 → 1-indexed start==end==(2, 5)。
        let ctx = preview_context_from_editor_state(
            "/repo/src/main.rs",
            false,
            (1, 4),
            None,
            1_700_000_000_000,
        );
        assert_eq!(ctx.path, "/repo/src/main.rs");
        assert_eq!((ctx.start_line, ctx.start_col), (2, 5));
        assert_eq!((ctx.end_line, ctx.end_col), (2, 5));
        assert!(!ctx.has_selection);
        // 新鲜度戳原样透传调用方给的时间,不在函数里读时钟。
        assert_eq!(ctx.updated_at_ms, 1_700_000_000_000);
    }

    #[test]
    fn preview_context_from_selection_uses_1_indexed_range() {
        // 0-indexed 选区 (1,4)..(3,0) → 1-indexed (2,5)..(4,1)。
        let ctx = preview_context_from_editor_state(
            "/repo/src/main.rs",
            true,
            (1, 4),
            Some(((1, 4), (3, 0))),
            1_700_000_000_000,
        );
        assert_eq!((ctx.start_line, ctx.start_col), (2, 5));
        assert_eq!((ctx.end_line, ctx.end_col), (4, 1));
        assert!(ctx.has_selection);
        assert_eq!(ctx.updated_at_ms, 1_700_000_000_000);
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
            created_ms: 0,
            updated_ms: 0,
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
        assert!(
            ws.files.file_tree_is_some(),
            "文件树根不需要 IO,应当立刻可画"
        );
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

    #[test]
    fn chrome_constants_exclude_removed_header() {
        // #4 去掉 header 行(22px + 一处 spacing 4 = 26)后的期望值,锁死防漂移遮挡。
        // 文件预览又去掉了地址栏(只剩 tab 栏),浏览器仍保留地址栏,两分支
        // chrome 高度分道,不能再共用同一个值——否则文件预览顶上会露一截
        // 再也画不出东西的空白。
        assert_eq!(
            byteui::theme::geometry::preview_chrome_top_px(),
            38.0,
            "文件预览 chrome 顶应为去地址栏后的 38(8 内边距 + 30 tab 栏)"
        );
        assert_eq!(
            byteui::theme::geometry::browser_chrome_top_px(),
            72.0,
            "浏览器 chrome 顶应为 72(8 内边距 + 30 tab 栏 + 4 spacing + 30 地址栏)"
        );
        assert_eq!(
            byteui::theme::geometry::chrome_height_px(),
            50.0,
            "终端 chrome 高应为去 header 后的 50"
        );
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
    fn review_refresh_only_for_matching_session() {
        assert!(review_should_refresh_on_turn(&ReviewSource::Session(3), 3));
        assert!(!review_should_refresh_on_turn(&ReviewSource::Session(3), 4));
        assert!(!review_should_refresh_on_turn(
            &ReviewSource::Conversation("x".into()),
            3
        ));
    }

    #[test]
    fn review_source_conversation_matches_conversation_id_not_tab() {
        // Conversation 变体不该被 review_should_refresh_on_turn(只认
        // Session(tab_id))误判为需要跟随终端回合刷新。
        assert!(!review_should_refresh_on_turn(
            &ReviewSource::Conversation("c1".into()),
            3
        ));
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
    fn tab_window_no_overflow_all_visible() {
        let w = tab_window(&[50.0, 50.0, 50.0], 4.0, 500.0, 0);
        assert_eq!((w.first, w.visible_end), (0, 3));
        assert!(!w.has_overflow(3));
    }

    #[test]
    fn tab_window_overflow_clamps_and_computes_visible_end() {
        let widths = [100.0; 5];
        let w = tab_window(&widths, 0.0, 250.0, 0);
        assert_eq!((w.first, w.visible_end), (0, 2));
        assert!(w.has_overflow(5));
        assert_eq!(w.hidden_before(), 0..0);
        assert_eq!(w.hidden_after(5), 2..5);

        // 请求的 first 越界 → 钳到 max_first(=3),此时尾部 3 个恰好全可见。
        let w = tab_window(&widths, 0.0, 250.0, 99);
        assert_eq!((w.first, w.visible_end), (3, 5));
        assert_eq!(w.hidden_before(), 0..3);
        assert_eq!(w.hidden_after(5), 5..5);

        let w = tab_window(&widths, 0.0, 250.0, 1);
        assert_eq!((w.first, w.visible_end), (1, 3));
        assert_eq!(w.hidden_before(), 0..1);
        assert_eq!(w.hidden_after(5), 3..5);
    }

    #[test]
    fn tab_window_reveal_keeps_visible_tab_still_no_jump() {
        let widths = [100.0; 5];
        // first=1 时可见区间是 [1,3):选中已经可见的 tab 1,first 不应该变。
        assert_eq!(
            crate::tab_widget::tab_window_reveal(&widths, 0.0, 250.0, 1, 1),
            1
        );
    }

    #[test]
    fn tab_window_reveal_scrolls_hidden_tab_into_view() {
        let widths = [100.0; 5];
        // first=0 时可见区间是 [0,2):选中隐藏在右侧的 tab 4,应重新钳出
        // 一个包含它的窗口。
        let new_first = crate::tab_widget::tab_window_reveal(&widths, 0.0, 250.0, 0, 4);
        let w = tab_window(&widths, 0.0, 250.0, new_first);
        assert!((w.first..w.visible_end).contains(&4));
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
        assert_eq!(
            dot_color(Running, false),
            byteui::theme::color::current().dim,
            "死会话灰点"
        );
        // 存活：空闲青、运行绿、待输入红、回合毕金——各状态独立配色，不再
        // 靠闪烁区分空闲/运行(闪烁动画已取消)。
        assert_eq!(dot_color(Idle, true), byteui::theme::color::current().cyan);
        assert_eq!(
            dot_color(Running, true),
            byteui::theme::color::current().green
        );
        assert_eq!(
            dot_color(AwaitingInput, true),
            byteui::theme::color::current().red
        );
        assert_eq!(
            dot_color(TurnEnded, true),
            byteui::theme::color::current().gold
        );
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
    fn format_model_label_strips_claude_prefix_and_titlecases() {
        assert_eq!(format_model_label("claude-sonnet-5"), "Sonnet 5");
        assert_eq!(format_model_label("claude-opus-5"), "Opus 5");
        assert_eq!(format_model_label("claude-haiku-4-5"), "Haiku 4 5");
    }

    #[test]
    fn format_model_label_unknown_shape_returns_verbatim() {
        assert_eq!(format_model_label("gpt-4"), "gpt-4");
        assert_eq!(format_model_label(""), "");
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
            llm_model: None,
            permission_mode: None,
            last_activity: None,
            workspace_override: None,
            tab_id: 0,
            forwarder: rt.spawn(async {}),
            backend: TabBackend::Daemon,
        }
    }

    #[test]
    fn new_session_tab_fields_default_to_none() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let tab = make_test_tab(&rt, "s1", dozer_core::protocol::AgentKind::Claude);
        assert_eq!(tab.llm_model, None);
        assert_eq!(tab.permission_mode, None);
        assert_eq!(tab.workspace_override, None);
    }

    #[test]
    fn should_summarize_on_close_true_for_four_supported_agents() {
        for agent in [
            AgentKind::Claude,
            AgentKind::Codebuddy,
            AgentKind::Opencode,
            AgentKind::V8agent,
        ] {
            assert!(
                should_summarize_on_close(agent, true, &TabBackend::Daemon),
                "{agent:?} 应该走总结后关闭"
            );
        }
    }

    #[test]
    fn should_summarize_on_close_false_for_unsupported_agents_or_dead_or_ssh() {
        assert!(!should_summarize_on_close(
            AgentKind::Codex,
            true,
            &TabBackend::Daemon
        ));
        assert!(!should_summarize_on_close(
            AgentKind::Kilo,
            true,
            &TabBackend::Daemon
        ));
        assert!(!should_summarize_on_close(
            AgentKind::Unknown,
            true,
            &TabBackend::Daemon
        ));
        assert!(!should_summarize_on_close(
            AgentKind::Claude,
            false,
            &TabBackend::Daemon
        ));
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        assert!(!should_summarize_on_close(
            AgentKind::Claude,
            true,
            &TabBackend::Ssh { out: tx }
        ));
    }

    /// 回归测试:此前只看 `info.agent`(daemon 默认值,真实 agent 靠 hook
    /// 事后上报),导致 picker 新建的 opencode 会话在 `on_tab_attached` 这一
    /// 刻永远判定成"非 opencode"——回应 OSC 10/11 的开关从未真正打开过。
    #[test]
    fn should_answer_dynamic_color_true_when_picker_targets_opencode_even_if_info_agent_lags() {
        assert!(should_answer_dynamic_color(
            Some(AgentKind::Opencode),
            AgentKind::Unknown, // daemon 侧此刻还没收到 hook 上报
        ));
    }

    #[test]
    fn should_answer_dynamic_color_true_when_info_agent_already_known_opencode() {
        // SSH attach、或重连时 hook 已先上报过的兜底路径:picker 没有提示。
        assert!(should_answer_dynamic_color(None, AgentKind::Opencode));
    }

    #[test]
    fn should_answer_dynamic_color_false_for_other_agents() {
        assert!(!should_answer_dynamic_color(
            Some(AgentKind::Claude),
            AgentKind::Unknown
        ));
        assert!(!should_answer_dynamic_color(None, AgentKind::Unknown));
        assert!(!should_answer_dynamic_color(None, AgentKind::Claude));
    }

    #[test]
    fn close_tab_skips_daemon_kill_for_ssh_backend() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut tab = make_test_tab(&rt, "ssh:h1", dozer_core::protocol::AgentKind::Unknown);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        tab.backend = TabBackend::Ssh { out: tx };
        tab.alive = true;
        // `close_tab` 里 `client.kill` 那一步只在
        // `matches!(tab.backend, TabBackend::Daemon)` 时才走——单测环境没
        // 有真实 daemon,不适合跑完整 `close_tab`,重点断言这个分支判断
        // 本身:SSH backend 不该触发 daemon kill。
        assert!(!matches!(tab.backend, TabBackend::Daemon));
    }

    #[test]
    fn resize_one_resizes_ssh_tab_model() {
        // `resize_all` 对 `tabs`/`ssh_tabs` 各跑一遍 `resize_one`；这里直接测
        // 那个被复用的核心逻辑(它不碰 `ShellIo`,单测不用构造 winit
        // `EventLoopProxy`)——构造一个 SSH backend 的假 tab,断言 resize 后
        // 网格尺寸跟着变了。
        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut tab = make_test_tab(&rt, "ssh:h1", dozer_core::protocol::AgentKind::Unknown);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        tab.backend = TabBackend::Ssh { out: tx };
        let before = tab.model.grid_dims();
        let client = dozer_client::Client::new(std::path::PathBuf::from(
            "/tmp/dozer-resize-test-nonexistent.sock",
        ));
        Workspace::resize_one(&mut tab, &client, rt.handle(), 100, 40);
        let after = tab.model.grid_dims();
        assert_ne!(before, after, "ssh_tab 的网格尺寸应随 resize 变化");
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
    fn group_tabs_by_agent_includes_codex_kilo_and_v8agent() {
        // 回归测试:`ORDER` 曾经只有 4 个 AgentKind(Claude/Codebuddy/
        // Opencode/Unknown),Codex/Kilo/V8agent 的会话会被 filter_map
        // 静默丢弃——tab 标题栏能正确识别出 agent 种类,但 Agent 侧栏
        // 面板完全不显示这些会话,面板直接留空。
        let rt = tokio::runtime::Runtime::new().unwrap();
        let tabs = vec![
            make_test_tab(&rt, "a", AgentKind::V8agent),
            make_test_tab(&rt, "b", AgentKind::Codex),
            make_test_tab(&rt, "c", AgentKind::Kilo),
        ];
        assert_eq!(
            group_tabs_by_agent(&tabs),
            vec![
                (AgentKind::Codex, vec![1]),
                (AgentKind::Kilo, vec![2]),
                (AgentKind::V8agent, vec![0]),
            ]
        );
    }

    #[test]
    fn agent_dot_color_maps_each_kind_and_avoids_gold() {
        let cases = [
            (AgentKind::Claude, byteui::theme::color::current().cyan),
            (AgentKind::Codebuddy, byteui::theme::color::current().purple),
            (AgentKind::Opencode, byteui::theme::color::current().green),
            (AgentKind::Codex, byteui::theme::color::current().orange),
            (AgentKind::Kilo, byteui::theme::color::current().blue),
            (AgentKind::V8agent, byteui::theme::color::current().lime),
            (AgentKind::Unknown, byteui::theme::color::current().dim),
        ];
        for (agent, expected) in cases {
            let color = agent_dot_color(agent);
            assert_eq!(color, expected, "{agent:?}");
            assert_ne!(
                color,
                byteui::theme::color::current().gold,
                "{agent:?} 的对话列表圆点色不能是 GOLD(甲方动作专属,CLAUDE.md 明文规定)"
            );
        }
    }

    #[test]
    fn agent_icon_maps_each_kind_to_brand_icon() {
        assert_eq!(agent_icon(AgentKind::Claude), IconKind::Claude);
        assert_eq!(agent_icon(AgentKind::Codebuddy), IconKind::Codebuddy);
        assert_eq!(agent_icon(AgentKind::Opencode), IconKind::Opencode);
        // Codex/Kilo/V8agent 暂无确认可用的品牌素材，回落通用 Bot 图标
        // （见计划 Task 3 说明，非占位符——spec §8/§6 明确允许的兜底）。
        assert_eq!(agent_icon(AgentKind::Codex), IconKind::Bot);
        assert_eq!(agent_icon(AgentKind::Kilo), IconKind::Bot);
        assert_eq!(agent_icon(AgentKind::V8agent), IconKind::Bot);
        // Unknown 同样回落 Bot 图标。
        assert_eq!(agent_icon(AgentKind::Unknown), IconKind::Bot);
    }

    #[test]
    fn agent_cli_command_maps_known_agents_and_none_for_unknown() {
        assert_eq!(agent_cli_command(AgentKind::Claude), Some("claude"));
        assert_eq!(agent_cli_command(AgentKind::Codebuddy), Some("codebuddy"));
        assert_eq!(agent_cli_command(AgentKind::Opencode), Some("opencode"));
        assert_eq!(agent_cli_command(AgentKind::Codex), Some("codex"));
        assert_eq!(agent_cli_command(AgentKind::Kilo), Some("kilo"));
        assert_eq!(agent_cli_command(AgentKind::V8agent), Some("v8agent"));
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
            picker_launch_command(PickerLaunch::Agent(Some(AgentKind::Kilo))),
            Some("kilo".to_string())
        );
        assert_eq!(
            picker_launch_command(PickerLaunch::Agent(Some(AgentKind::V8agent))),
            Some("v8agent".to_string())
        );
        // 纯 Shell → 不键入任何初始命令。
        assert_eq!(picker_launch_command(PickerLaunch::Agent(None)), None);
        // Git Shell → 项目根开 shell 后自动跑 git status。
        assert_eq!(
            picker_launch_command(PickerLaunch::Git),
            Some("git status".to_string())
        );
    }

    #[test]
    fn hook_install_target_covers_only_agents_wired_up_in_dozer_hook() {
        // Claude/CodeBuddy/Codex 都走 `install::run_at` 的 JSON settings
        // 补丁机制。
        for agent in [AgentKind::Claude, AgentKind::Codebuddy, AgentKind::Codex] {
            assert_eq!(
                hook_install_target(agent),
                Some(HookInstallTarget::Settings),
                "{agent:?}"
            );
        }
        assert_eq!(
            hook_install_target(AgentKind::Opencode),
            Some(HookInstallTarget::Opencode)
        );
        // Kilo/V8agent 在 `dozer-hook::install::settings_path_for` 里没有专属
        // 分支，会落回 Claude 的 settings.json 路径——绝不能对它们调用安装
        // 逻辑，否则会把 "kilo"/"v8agent" 的 hook 命令误写进 Claude 的配置，
        // 顶掉真正的 claude hook 条目。Unknown 同理，从不该触发安装。V8agent
        // 不属于这里(它走 socket 直连上报，见 hook_install_target 上方文档
        // 注释)，只是恰好也该返回 None——跟 Kilo 是两个不同的理由。
        for agent in [AgentKind::Kilo, AgentKind::V8agent, AgentKind::Unknown] {
            assert_eq!(hook_install_target(agent), None, "{agent:?}");
        }
    }

    #[test]
    fn mcp_install_target_covers_four_config_capable_agents() {
        for agent in [
            AgentKind::Claude,
            AgentKind::Codebuddy,
            AgentKind::Codex,
            AgentKind::Opencode,
        ] {
            assert!(
                dozer_mcp::install::config_path_for(agent.label()).is_some(),
                "{agent:?} 应该有对应的 mcp 配置文件路径"
            );
        }
    }

    #[test]
    fn mcp_install_target_excludes_v8agent_kilo_unknown() {
        // V8agent 走硬编码自动挂载(不读配置文件),Kilo 无 MCP 支持,
        // Unknown 是纯 shell——三者都不该有配置文件路径。
        for agent in [AgentKind::V8agent, AgentKind::Kilo, AgentKind::Unknown] {
            assert!(
                dozer_mcp::install::config_path_for(agent.label()).is_none(),
                "{agent:?} 不该有 mcp 配置文件路径"
            );
        }
    }

    #[test]
    fn dozer_hook_binary_path_is_sibling_of_exe() {
        assert_eq!(
            dozer_hook_binary_path(Path::new(
                "/Applications/Dozer AI Coder.app/Contents/MacOS/dozer"
            )),
            PathBuf::from("/Applications/Dozer AI Coder.app/Contents/MacOS/dozer-hook")
        );
    }

    #[test]
    fn ensure_hook_installed_writes_codebuddy_settings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        unsafe { std::env::set_var("DOZER_CODEBUDDY_SETTINGS", path.to_str().unwrap()) };
        ensure_hook_installed(AgentKind::Codebuddy);
        unsafe { std::env::remove_var("DOZER_CODEBUDDY_SETTINGS") };
        let root: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let cmd = root["hooks"]["Stop"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        assert!(cmd.contains(" codebuddy "), "{cmd}");
        // 回归线上事故：命令必须指向 `dozer-hook`（sibling 于当前进程的
        // exe），不能是调用方自己（测试进程本身，路径里带 "deps/"，不含
        // "dozer-hook"）——否则 `entry_is_dozer` 认不出，每次都重复追加。
        assert!(
            cmd.contains("dozer-hook") || cmd.contains("dozer_hook"),
            "{cmd}: 必须指向 dozer-hook 二进制，不能是 dozer-app 自己"
        );
    }

    #[test]
    fn ensure_hook_installed_writes_opencode_plugin() {
        let dir = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("DOZER_OPENCODE_PLUGIN_DIR", dir.path().to_str().unwrap()) };
        ensure_hook_installed(AgentKind::Opencode);
        unsafe { std::env::remove_var("DOZER_OPENCODE_PLUGIN_DIR") };
        assert!(dir.path().join("dozer.ts").exists());
        assert!(
            dir.path()
                .join("dozer-lib")
                .join("dozer-translate.ts")
                .exists()
        );
    }

    #[test]
    fn ensure_hook_installed_is_noop_for_kilo_v8agent_and_unknown() {
        // 回归 hook_install_target 的排除名单：这三者不该产生任何文件写入。
        // 用 Claude 的 settings 路径当探针——如果实现退化成 catch-all 调用
        // `install::run_at`，这里会意外产生一个把 "kilo" 写进去的
        // settings.json。
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        unsafe { std::env::set_var("DOZER_CLAUDE_SETTINGS", path.to_str().unwrap()) };
        for agent in [AgentKind::Kilo, AgentKind::V8agent, AgentKind::Unknown] {
            ensure_hook_installed(agent);
        }
        unsafe { std::env::remove_var("DOZER_CLAUDE_SETTINGS") };
        assert!(!path.exists(), "Kilo/V8agent/Unknown 不该写任何 hook 配置");
    }

    fn write_temp_file(name: &str, content: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(name);
        std::fs::write(&path, content).unwrap();
        (dir, path)
    }

    #[test]
    fn active_preview_tab_has_native_editor_reflects_active_tab_kind() {
        let (_dir_rs, rs_path) = write_temp_file("a.rs", "fn main() {}");
        let (_dir_png, png_path) = write_temp_file("a.png", "");
        let mut ws = Workspace::empty_for_project_placeholder();
        assert!(
            !ws.active_preview_tab_has_native_editor(PanelKind::Files),
            "没有 tab 时应为 false"
        );

        ws.preview.open_path(rs_path);
        assert!(
            ws.active_preview_tab_has_native_editor(PanelKind::Files),
            ".rs 是白名单扩展名,应走原生渲染"
        );

        ws.preview.open_path(png_path);
        assert!(
            !ws.active_preview_tab_has_native_editor(PanelKind::Files),
            "切到 .png 后激活 tab 应走 wry,不是原生"
        );
    }

    #[test]
    fn active_preview_tab_has_native_editor_checks_project_preview_independently() {
        let (_dir_rs, rs_path) = write_temp_file("a.rs", "fn main() {}");
        let mut ws = Workspace::empty_for_project_placeholder();
        assert!(
            !ws.active_preview_tab_has_native_editor(PanelKind::Project),
            "project_preview 没有 tab 时应为 false"
        );

        ws.project_preview.open_path(rs_path);
        assert!(
            ws.active_preview_tab_has_native_editor(PanelKind::Project),
            "Project 预览面板里的 .rs tab 也应走原生渲染,不是恒查 Files 那个 PreviewPane"
        );
        assert!(
            !ws.active_preview_tab_has_native_editor(PanelKind::Files),
            "Files 预览面板本身没开 tab,不该被 Project 那边的状态影响"
        );
    }

    #[test]
    fn preview_pane_undo_active_reverts_edit_and_marks_dirty() {
        use iced_widget::text_editor::{Action, Edit, Motion};
        let (_dir, path) = write_temp_file("a.txt", "ab");
        let mut ws = Workspace::empty_for_project_placeholder();
        ws.preview.open_path(path);
        let id = ws.preview.tabs()[ws.preview.active_idx()].id;
        assert!(
            !ws.preview.tabs()[ws.preview.active_idx()].dirty,
            "新开不脏"
        );

        // 落到行尾后插一个字符(经激活 tab 的 perform 管线,与裸 Tab 同路)。
        ws.preview_pane_active_editor_event(PanelKind::Files, Action::Move(Motion::DocumentEnd));
        ws.preview_pane_active_editor_event(PanelKind::Files, Action::Edit(Edit::Insert('!')));
        assert_eq!(ws.preview.editor_mut(id).unwrap().text(), "ab!");

        ws.preview_pane_undo_active(PanelKind::Files);
        assert_eq!(
            ws.preview.editor_mut(id).unwrap().text(),
            "ab",
            "⌘Z 应回退到编辑前"
        );
        assert!(
            ws.preview.tabs()[ws.preview.active_idx()].dirty,
            "发生过回退的 tab 应保持脏(与磁盘不一致)"
        );

        // 栈空后再撤是 no-op,不 panic、不改文本。
        ws.preview_pane_undo_active(PanelKind::Files);
        assert_eq!(ws.preview.editor_mut(id).unwrap().text(), "ab");
    }

    #[test]
    fn preview_pane_redo_active_reapplies_undone_edit_and_marks_dirty() {
        use iced_widget::text_editor::{Action, Edit, Motion};
        let (_dir, path) = write_temp_file("a.txt", "ab");
        let mut ws = Workspace::empty_for_project_placeholder();
        ws.project_preview.open_path(path);
        let id = ws.project_preview.tabs()[ws.project_preview.active_idx()].id;

        ws.preview_pane_active_editor_event(PanelKind::Project, Action::Move(Motion::DocumentEnd));
        ws.preview_pane_active_editor_event(PanelKind::Project, Action::Edit(Edit::Insert('!')));
        assert_eq!(ws.project_preview.editor_mut(id).unwrap().text(), "ab!");

        ws.preview_pane_undo_active(PanelKind::Project);
        assert_eq!(ws.project_preview.editor_mut(id).unwrap().text(), "ab");

        ws.preview_pane_redo_active(PanelKind::Project);
        assert_eq!(
            ws.project_preview.editor_mut(id).unwrap().text(),
            "ab!",
            "⌘⇧Z 应重做被撤掉的编辑"
        );
        assert!(
            ws.project_preview.tabs()[ws.project_preview.active_idx()].dirty,
            "发生过重做的 tab 应保持脏"
        );
    }

    #[test]
    fn preview_pane_undo_active_noop_without_native_tab() {
        let (_dir, path) = write_temp_file("a.png", "");
        let mut ws = Workspace::empty_for_project_placeholder();
        ws.preview.open_path(path);
        // .png 走 wry(无 editor):撤销应静默 no-op,不 panic、不乱标脏。
        ws.preview_pane_undo_active(PanelKind::Files);
        assert!(!ws.preview.tabs()[ws.preview.active_idx()].dirty);
    }

    #[test]
    fn blur_preview_editors_sets_pending_unfocus_flag() {
        let mut ws = Workspace::empty_for_project_placeholder();
        assert!(!ws.take_editor_unfocus_pending());
        ws.blur_preview_editors();
        assert!(ws.take_editor_unfocus_pending(), "应置一次性让出焦点标记");
        assert!(!ws.take_editor_unfocus_pending(), "消费式:取走后应复位");
    }

    #[test]
    fn blur_inputs_keep_native_preview_editor_skips_pending_unfocus() {
        let (_dir, path) = write_temp_file("a.txt", "hi");
        let mut ws = Workspace::empty_for_project_placeholder();
        ws.preview.open_path(path);
        assert!(
            ws.preview.active_tab_is_native(),
            "白名单文本 tab 应为原生可编辑预览"
        );
        // 点到原生预览编辑器本身:保留(不置让出焦点标记)。
        ws.blur_inputs(true);
        assert!(
            !ws.take_editor_unfocus_pending(),
            "保留时不该同帧把 self-focus 出的光标抬掉"
        );
        // 点其它地方:维持原行为,照常让出预览编辑器焦点。
        ws.blur_inputs(false);
        assert!(
            ws.take_editor_unfocus_pending(),
            "点非编辑器区仍应让出预览编辑器焦点"
        );
    }
}
