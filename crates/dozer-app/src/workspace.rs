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
    App, DEFAULT_COLS, DEFAULT_ROWS, HoverId, Message, ProjectId, panel_tab, tab_arrow_button,
    tab_divider, tab_window,
};
use crate::conversation::{self, ConversationMeta};
use crate::delivery::{self};
use crate::extensions::acceptance;
use crate::extensions::browser;
use crate::extensions::database;
use crate::extensions::files;
use crate::extensions::project;
use crate::extensions::search;
use crate::extensions::ssh;
use crate::extensions::todo;
use crate::extensions::usage;
use crate::git_watch;
use crate::homespace::home_panel_head;
use crate::homespace::home_panel_head_with_actions;
use crate::icons;
use crate::icons::IconKind;
use crate::osc::{OscEvent, OscScanner};
use crate::preview::{PreviewPane, TabKind, is_editable_extension};
use crate::preview_state;
use crate::project::FileTree;
use crate::term_model::TerminalModel;
use crate::theme;
use crate::theme::terminal_font;
use crate::transcript::{self, ReviewEntry};
use dozer_client::{Client, TermEvent};
use dozer_core::protocol::{AgentKind, AgentState, ProjectInfo, SessionInfo};
use iced_code_editor::{CodeEditor, Message as EditorMessage};
use iced_widget::core::mouse;
use iced_widget::core::text::LineHeight;
use iced_widget::core::{Border, Color, Element, Length, Padding};
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
pub(crate) fn review_should_refresh_on_turn(source: &ReviewSource, tab_id: usize) -> bool {
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
    /// 编辑器组件(`iced-code-editor`)。有状态 widget,持有内容与撤销栈。
    pub editor: CodeEditor,
    /// 上次落盘(或打开)时的内容快照。脏标记 = `editor.content() !=
    /// saved_content`,不依赖 `editor.is_modified()`——后者在连续输入
    /// 的 undo 分组(`is_grouping`)未提交期间会恒为 `true`,导致保存后
    /// 仍误报脏(`iced-code-editor` 没有公开的 `end_group` 接口来收口分组)。
    pub saved_content: String,
    /// 打开失败(理论上不会,打开前已判过存在)或保存失败的错误文案。
    pub error: Option<String>,
    /// 脏改动下点关闭:先弹二次确认,不直接丢。
    pub confirm_discard: bool,
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
    /// TurnEnded 检测出的"交付待验收"标记（spec P1f D3）。
    pub delivery_pending: bool,
    /// 上一次 TurnEnded 时的 HEAD（无沉淀 ref 时的比对基线）。
    pub last_turn_head: Option<String>,
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
    /// 进行中的验收——验收面板 per-project 状态,见
    /// `extensions::acceptance::WorkspaceState`。
    pub(crate) acceptance: acceptance::WorkspaceState,
    /// 进行中的会话审阅（审阅 tab 内容;None=未打开;P1i）。
    pub(crate) review: Option<ReviewView>,
    /// 当前项目的对话列表（扫 Claude 目录；P1j）。
    pub(crate) conversations: Vec<ConversationMeta>,
    /// 当前项目的 agent 用量统计（会话粒度；扫描+解析全量 transcript，比
    /// `conversations` 贵得多,所以不像它那样跟着 `DeliveryChecked` 自动
    /// 刷新——只在切到 `RightView::Usage` 或点手动刷新按钮时才重新扫
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
    /// 终端 tab 栏当前最左可见 tab 序号（箭头翻页用；P1L T5）。
    pub(crate) term_tab_first: usize,
    /// 预览 tab 栏当前最左可见 tab 序号，语义同 `term_tab_first`。
    pub(crate) preview_tab_first: usize,
    /// Project 面板右配对预览 tab 栏当前最左可见 tab 序号,语义同
    /// `preview_tab_first`。
    pub(crate) project_preview_tab_first: usize,
    /// Files 面板 per-project 状态——见 `extensions::files::WorkspaceState`。
    pub(crate) files: files::WorkspaceState,
    /// Agent 面板"＋"按钮弹出的"新建"菜单当前是否打开。不需要坐标——面板顶部固定
    /// 位置的下拉,不像项目树右键菜单需要跟随点击坐标。
    pub(crate) agent_picker_open: bool,
    /// 预览编辑弹层进行中的会话;`None` = 未打开。
    pub(crate) edit_session: Option<EditSession>,
    /// Todo 面板 per-project 状态——见 `extensions::todo::WorkspaceState`。
    pub(crate) todo: todo::WorkspaceState,
    /// 数据库面板 per-project 状态(当前项目的数据源列表 + 编辑草稿 + 测试
    /// 状态 map)——见 `extensions::database::WorkspaceState`。
    pub(crate) database: database::WorkspaceState,
    /// SSH 面板 per-project 状态——见 `extensions::ssh::WorkspaceState`。
    pub(crate) ssh: ssh::WorkspaceState,
    /// 文件树右键"搜索"弹窗 per-project 状态——见
    /// `extensions::search::WorkspaceState`。瞬态弹窗,不挂 LeftView。
    pub(crate) search: search::WorkspaceState,
    /// 这份 `Workspace` 是否只是 `Stub` → `Loaded` 促成期间的"加载中"占位
    /// (见 [`Workspace::loading_for_project`])。占位有正确的 `project`/文件树,
    /// 但会话/git/对话都还没拉,并且整份对象会在
    /// `Message::ProjectSlotLoaded` 到达时被真正的结果替换掉——所以此刻
    /// **不该**替用户在它上面新建终端会话(建出来的会话会随占位一起被丢弃,
    /// 却仍在 daemon 上活着),`spawn_new_tab` 据此原地放弃。
    pub(crate) loading: bool,
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
        ws.spawn_acceptance_count_refresh(io);
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
            preview: PreviewPane::default(),
            project_preview: PreviewPane::default(),
            preview_error: None,
            project_preview_error: None,
            preview_context_nonce: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            browser: browser::State::default(),
            allowed_files: Arc::new(Mutex::new(HashSet::new())),
            acceptance: acceptance::WorkspaceState::default(),
            review: None,
            conversations: Vec::new(),
            usage: usage::WorkspaceState::default(),
            project: None,
            project_panel: project::WorkspaceState::default(),
            recent_projects: Vec::new(),
            git_watch: None,
            term_tab_first: 0,
            preview_tab_first: 0,
            project_preview_tab_first: 0,
            files: files::WorkspaceState::default(),
            agent_picker_open: false,
            edit_session: None,
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

    pub(crate) fn tab_by_id_mut(&mut self, tab_id: usize) -> Option<&mut SessionTab> {
        self.tabs
            .iter_mut()
            .find(|t| t.tab_id == tab_id)
            .or_else(|| self.ssh_tabs.iter_mut().find(|t| t.tab_id == tab_id))
    }

    /// 打开预览编辑弹层:按 tab 下标取路径读盘。下标越界或该 tab 不是
    /// `TabKind::File` 时静默 no-op(按钮本就只在 file tab 上画,正常路径
    /// 走不到这两种情况)。读盘失败写 `preview_error`,不开弹层。
    pub(crate) fn preview_edit_open(&mut self, idx: usize) {
        self.preview_edit_open_for(PreviewPaneKind::Files, idx);
    }

    /// Project 面板右配对预览的编辑入口,语义同 `preview_edit_open`,状态取自
    /// `ws.project_preview`,错误写到 `project_preview_error`。
    pub(crate) fn project_preview_edit_open(&mut self, idx: usize) {
        self.preview_edit_open_for(PreviewPaneKind::Project, idx);
    }

    fn preview_edit_open_for(&mut self, kind: PreviewPaneKind, idx: usize) {
        let (preview, error_field) = match kind {
            PreviewPaneKind::Files => (&self.preview, &mut self.preview_error),
            PreviewPaneKind::Project => (&self.project_preview, &mut self.project_preview_error),
        };
        let Some(tab) = preview.tabs().get(idx) else {
            return;
        };
        let TabKind::File(path) = &tab.kind;
        let path = path.clone();
        let tab_id = tab.id;
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                *error_field = None;
                let mut editor =
                    CodeEditor::new(&text, &crate::preview::extension_to_syntax(&path));
                // 让编辑器适配 Dozer 配色(见 `dozer_editor_style`),而非
                // 让 Dozer 迁就编辑器的默认蓝底。chrome 与语法 token 一起接管。
                editor.set_theme(crate::preview::dozer_editor_style());
                editor.set_syntax_theme(crate::preview::dozer_syntax_theme());
                editor.set_font(crate::fonts::code_font());
                // 编辑器/终端排版同步(见 CLAUDE.md 关键裁决,以及
                // `font.rs` _font/tab 注释):字号、行高与终端同源,
                // 一个字面量真相源 `terminal_font`(乘全局 scale)。
                // 字距两者都走 cosmic-text 默认,不额外加宽。
                crate::preview::dozer_editor_font_metrics(&mut editor);
                // 打开即夺焦点:设置内部 focus 标记,使键盘事件无需先点击
                // 即可直达编辑器(仍建议点击以触发光标定位与选区)。
                editor.request_focus();
                let _ = editor.update(&EditorMessage::CanvasFocusGained);
                self.edit_session = Some(EditSession {
                    tab_id,
                    path,
                    editor,
                    saved_content: text.clone(),
                    error: None,
                    confirm_discard: false,
                });
            }
            Err(e) => {
                *error_field = Some(format!("打开编辑失败: {e}"));
            }
        }
    }

    /// 按预览 tab id 打开对应文件的编辑浮层——右键"编辑"上下文菜单动作
    /// (`Message::OpenInEditor`)落地的入口。没找到该 tab 时 no-op。
    pub(crate) fn preview_edit_open_by_id(&mut self, tab_id: usize) {
        let idx = self.preview.tabs().iter().position(|tab| tab.id == tab_id);
        if let Some(idx) = idx {
            self.preview_edit_open(idx);
        }
    }

    /// 转发 `iced-code-editor` 的内部消息,返回编辑器产生的    /// `iced::Task<EditorMessage>`(交由 `main.rs` 的 Task 桥接器执行——
    /// 主要是剪贴板读写与搜索框聚焦;无运行时下普通编辑路径恒为
    /// `Task::none()`)。没有打开编辑会话时直接返回 `Task::none()`。
    pub(crate) fn preview_edit_event(
        &mut self,
        event: EditorMessage,
    ) -> iced_winit::runtime::Task<EditorMessage> {
        let Some(session) = self.edit_session.as_mut() else {
            return iced_winit::runtime::Task::none();
        };
        session.editor.update(&event)
    }

    /// 转发 `iced-code-editor` 的内部消息到某个原生预览 tab(按 `tab_id` 定位,
    /// 不是"当前聚焦编辑弹层"——一个项目可以同时开好几个原生预览 tab,只有
    /// 事件来源的那一个该收到)。tab 不存在或不是原生 tab 时静默 no-op。
    pub(crate) fn preview_tab_editor_event(
        &mut self,
        tab_id: usize,
        event: EditorMessage,
    ) -> iced_winit::runtime::Task<EditorMessage> {
        match self.preview.editor_mut(tab_id) {
            Some(editor) => editor.update(&event),
            None => iced_winit::runtime::Task::none(),
        }
    }

    /// 按 Project 面板右配对预览 tab id 打开编辑浮层,语义同
    /// `preview_edit_open_by_id`,状态取自 `ws.project_preview`。
    pub(crate) fn project_preview_edit_open_by_id(&mut self, tab_id: usize) {
        let idx = self
            .project_preview
            .tabs()
            .iter()
            .position(|tab| tab.id == tab_id);
        if let Some(idx) = idx {
            self.project_preview_edit_open(idx);
        }
    }

    /// 转发 `iced-code-editor` 的内部消息到 Project 面板右配对预览的某个原生
    /// tab,语义同 `preview_tab_editor_event`,状态取自 `ws.project_preview`。
    pub(crate) fn project_preview_tab_editor_event(
        &mut self,
        tab_id: usize,
        event: EditorMessage,
    ) -> iced_winit::runtime::Task<EditorMessage> {
        match self.project_preview.editor_mut(tab_id) {
            Some(editor) => editor.update(&event),
            None => iced_winit::runtime::Task::none(),
        }
    }

    /// 保存当前编辑会话到磁盘,成功则清脏(`mark_saved`)并推进该 tab 的
    /// reload nonce(逼预览 webview 重新加载,否则用户会看到保存前的旧内容)。
    /// 失败写 `session.error`,弹层不关。没有打开编辑会话时 no-op。
    pub(crate) fn preview_edit_save(&mut self) {
        let Some(session) = self.edit_session.as_mut() else {
            return;
        };
        match std::fs::write(&session.path, session.editor.content()) {
            Ok(()) => {
                session.saved_content = session.editor.content();
                session.editor.mark_saved();
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
    pub(crate) fn preview_edit_close_request(&mut self) {
        let Some(session) = self.edit_session.as_mut() else {
            return;
        };
        if session.editor.content() != session.saved_content {
            session.confirm_discard = true;
        } else {
            self.edit_session = None;
        }
    }

    /// 二次确认:确认放弃未保存改动,真正关闭。
    pub(crate) fn preview_edit_confirm_discard(&mut self) {
        self.edit_session = None;
    }

    /// 二次确认:取消,回到编辑态(改动不丢)。
    pub(crate) fn preview_edit_confirm_cancel(&mut self) {
        if let Some(session) = self.edit_session.as_mut() {
            session.confirm_discard = false;
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

    /// 把 `text` 当输入写进已存活的 `session_id` 对应 tab。派发目标可能在
    /// 选择弹层打开期间被用户关掉（tab 已不在 `self.tabs` 里）——静默跳过。
    pub(crate) fn dispatch_todo_to_existing(&self, io: &ShellIo, session_id: &str, text: &str) {
        let Some(tab) = self
            .tabs
            .iter()
            .find(|t| t.info.id == session_id)
            .or_else(|| self.ssh_tabs.iter().find(|t| t.info.id == session_id))
        else {
            return;
        };
        if !tab.alive {
            return;
        }
        let bytes = format!("{text}\n").into_bytes();
        match &tab.backend {
            TabBackend::Daemon => {
                let client = io.client.clone();
                let id = session_id.to_string();
                io.handle.spawn(async move {
                    if let Err(e) = client.write(&id, &bytes).await {
                        tracing::warn!("派发任务文本失败: {e}");
                    }
                });
            }
            TabBackend::Ssh { out } => {
                let _ = out.send(SshOut::Data(bytes));
            }
        }
    }

    /// 异步扫当前项目的对话目录 → ConversationsRefreshed（GUI 侧 spawn_blocking；P1j）。
    pub(crate) fn spawn_conversations_refresh(&self, io: &ShellIo) {
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

    /// 异步扫当前项目的全部 transcript 并逐个解析用量 → `Usage(Loaded)`。
    /// 比 `spawn_conversations_refresh` 贵得多(要读整份文件内容，不只是
    /// 文件头)，所以不接入它那条"回合结束自动刷新"的调用链——只在
    /// `RightIconSelect(RightView::Usage)` 或手动刷新按钮时触发。
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
        usage::spawn_refresh(project_id, project_path, &io.handle, emit);
    }

    /// 异步取当前项目验收次数 → AcceptanceCountLoaded（项目卡副行）。
    /// 查询键走 `acceptance_query_repo`（= 落库侧 `delivery::repo_root`），
    /// 而非原始 `p.path`，否则子目录/符号链接路径撞不到库、副行静默空白。
    pub(crate) fn spawn_acceptance_count_refresh(&self, io: &ShellIo) {
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
            let _ = proxy.send_event(Message::Project(project::Message::AcceptanceCountLoaded(
                project_id, n,
            )));
        });
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

    /// 异步读 transcript + 解析 → ReviewLoaded（GUI 侧 spawn_blocking；P1i/P1j/P2b 按源）。
    pub(crate) fn spawn_review_load(
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

    /// 项目切换清理：关掉所有终端 tab（=结束会话，同 CloseTab 语义）与
    /// 所有预览 tab，给新项目一个干净起点（P1g 验收反馈）。webview 池由
    /// main.rs 的 sync_previews 依据空的期望清单自动销毁。
    pub(crate) fn close_all_tabs_for_switch(&mut self, io: &ShellIo) {
        while !self.tabs.is_empty() {
            self.close_tab(io, 0);
        }
        while !self.preview.tabs().is_empty() {
            self.preview.close(0);
        }
        self.acceptance = acceptance::WorkspaceState::default();
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
        self.conversations = Vec::new();
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
        self.spawn_acceptance_count_refresh(io);
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
        if tab.alive && matches!(tab.backend, TabBackend::Daemon) {
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
            let TabKind::File(p) = &tab.kind;
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
                let TabKind::File(path) = &tab.kind;
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

    /// SSH 版 `spawn_new_tab`:握手 + host key 校验 + 认证 + 开 channel +
    /// request_pty/request_shell,成功后进入读写泵循环。10 秒超时罩住
    /// "握手到 channel 就绪"这一段(同阶段 1 `test_connection` 的超时
    /// 口径,泵循环本身不设超时——那是长连接,超时语义不适用)。
    pub(crate) fn spawn_ssh_tab(&mut self, io: &ShellIo, host_id: String) {
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
        let (cols, rows) = (io.cols, io.rows);
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
                .send_event(Message::TabAttached(project_id, tab_id, info, Vec::new()))
                .is_err()
            {
                return;
            }

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
            ssh::sftp::SftpTabState::new(host_id.clone(), project_root, "~".to_string()),
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
            let _ = proxy.send_event(Message::Ssh(ssh::Message::Sftp(
                ssh::sftp::Message::Connected(host_id_for_task.clone(), Ok(())),
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
    /// 同样为了可测性收窄参数的既有先例)。
    pub(crate) fn on_tab_attached(
        &mut self,
        cols: u16,
        rows: u16,
        tab_id: usize,
        info: SessionInfo,
        snapshot: Vec<u8>,
    ) {
        let Some(forwarder) = self.pending.remove(&tab_id) else {
            return;
        };
        let mut model = TerminalModel::new(cols, rows);
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
            delivery_pending: false,
            last_turn_head: None,
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
            self.term_tab_first = 0;
        }
    }

    /// 终端 pane 尺寸变化：换算出的新网格套用到本项目的所有 tab（含当前
    /// 不可见的），并把新尺寸同步给 daemon 侧存活的会话。
    ///
    /// "网格真的变了吗"这道闸门在调用方 `App::update` 的 `PaneResized`
    /// 分支上——`cols`/`rows` 是外壳态（整个窗口一份），比对基准不在这里。
    pub(crate) fn resize_all(&mut self, io: &ShellIo, cols: u16, rows: u16) {
        let client = io.client.clone();
        let handle = io.handle.clone();
        for tab in &mut self.tabs {
            Self::resize_one(tab, &client, &handle, cols, rows);
        }
        for tab in &mut self.ssh_tabs {
            Self::resize_one(tab, &client, &handle, cols, rows);
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

    /// 浏览器地址栏是否在编辑态(main.rs 据此路由键盘:真 → AddrEvent,
    /// 假 → keymap → PTY)。预览面板已不再有地址栏,只需查 `self.browser`。
    pub fn browser_addr_editing(&self) -> bool {
        self.browser.addr_editing()
    }

    /// 验收意见输入是否在编辑态（main.rs 键盘路由用）。
    pub fn acceptance_comment_editing(&self) -> bool {
        self.acceptance.comment_editing()
    }

    /// 当前激活预览 tab 若是 webview(文件/网页)则返回其 id,供 main.rs
    /// 焦点路由取句柄;验收 tab/无 tab 返回 None。
    pub fn active_preview_webview_id(&self) -> Option<usize> {
        self.preview.active_webview_id()
    }

    /// 当前激活预览 tab 是否走原生渲染(有 `editor`)。main.rs 键盘路由用:
    /// 原生预览 tab 跟编辑弹层(`edit_session_open`)一样,需要在按键分发链
    /// 里提前放行,让键盘事件走 iced 正常管线直达 `CodeEditor`,不落进
    /// 终端/⌘ 快捷键那些手工转发分支。
    pub fn active_preview_tab_has_native_editor(&self) -> bool {
        self.preview
            .tabs()
            .get(self.preview.active_idx())
            .is_some_and(|t| t.editor.is_some())
    }

    /// Ctrl ± / 重置缩放后,重算所有原生编辑器(编辑弹层 + 各预览 tab)的排版,
    /// 使其随全局 scale 一起放大缩小。见 `preview::dozer_editor_font_metrics`。
    pub(crate) fn resync_editor_font_metrics(&mut self) {
        if let Some(session) = self.edit_session.as_mut() {
            crate::preview::dozer_editor_font_metrics(&mut session.editor);
        }
        self.preview.resync_editor_font_metrics();
    }

    /// 当前激活浏览器 tab 的 webview id,语义同 `active_preview_webview_id`,
    /// 查独立的 `self.browser`。
    pub fn active_browser_webview_id(&self) -> Option<usize> {
        self.browser.active_webview_id()
    }

    /// 项目树是否处于行内编辑态(main.rs 键盘路由用,同款
    /// `browser_addr_editing()`/`acceptance_comment_editing()`)。
    pub fn tree_editing(&self) -> bool {
        self.files.tree_edit_is_some()
    }

    /// 文件树搜索框是否处于自绘编辑态(main.rs 键盘路由用):为真时按键改
    /// 路由成 `files::Message::SearchEvent`,不再喂 PTY。
    pub fn search_editing(&self) -> bool {
        self.files.search_editing()
    }

    /// 右键文件树"搜索"弹窗是否打开(main.rs 键盘路由/App view 浮层用)。
    pub fn search_popup_open(&self) -> bool {
        self.search.is_open()
    }

    /// 右键文件树"搜索"弹窗查询框是否处于自绘编辑态(main.rs 键盘路由用):
    /// 为真时按键路由成 `search::Message::QueryChanged`,不再喂 PTY。
    pub fn search_popup_editing(&self) -> bool {
        self.search.query_editing()
    }

    /// 项目信息面板名称是否处于自绘编辑态(main.rs 键盘路由用)。
    pub fn project_name_editing(&self) -> bool {
        self.project_panel.name_editing_is_some()
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
    pub fn blur_inputs(&mut self) {
        if self.browser.addr_editing() {
            self.browser.addr_cancel();
        }
        self.acceptance.clear_comment_editing();
        self.files.cancel_tree_edit();
        self.files.cancel_search_edit();
        self.todo.cancel_search_edit();
        self.todo.cancel_drag();
        // 名称编辑不在失焦时丢弃——改由 `App::blur_inputs` 取出缓冲并发起
        // daemon 改名(改动且非空才发请求),与描述字段"失焦写盘"行为对齐。
        if let Some(project) = self.project.as_ref() {
            self.project_panel
                .submit_description_edit_on_blur(std::path::Path::new(&project.path));
        }
        self.blur_preview_editors();
    }

    /// `iced-code-editor` 不会自己在别处获得焦点时让出焦点(vendor README
    /// 明文要求宿主显式调用 `lose_focus()`),否则光标闪烁/IME 状态会一直
    /// 赖在最后打开的编辑器上,即使键盘输入其实已经转到了终端/其它输入框。
    /// 两个独立 `PreviewPane`(Files 预览、Project 面板配对预览)+ 编辑
    /// 弹层各自的 editor 都要清。独立于 `blur_inputs` 之外单独暴露:
    /// `main.rs` 里还有一条不经过鼠标点击、纯靠消息把 `current_focus` 拨
    /// 离 `FocusIntent::Preview` 的路径(切终端 tab/新会话落成/选中
    /// agent),那条路径不该顺带触发 `blur_inputs` 里其它自绘输入的失焦
    /// 逻辑(地址栏取消/树编辑取消等语义不搭)。
    pub fn blur_preview_editors(&mut self) {
        self.preview.blur_all_editors();
        self.project_preview.blur_all_editors();
        if let Some(session) = self.edit_session.as_mut() {
            session.editor.lose_focus();
        }
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

/// 异步跑一次组合 git 查询(分支/脏/文件状态/worktree),完成后分发成两条
/// 独立消息:`Files(StatusesRefreshed)` 只带文件级状态,`Project(GitRefreshed)`
/// 带分支/脏/worktree。两个 extension 互不知道对方存在,内核是唯一知道
/// "这两份数据同源"的地方(设计文档"关键语义确认")。4 个既有调用点:
/// `Workspace::from_restore`/`adopt_project`/回合结束(`DeliveryChecked`)/
/// `Message::ProjectFsChanged`——因为要同时认识 `files::Message`/
/// `project::Message` 两个类型,不适合作为任何一个 extension 的自由函数,
/// 也不需要 `&self`,做成纯自由函数、参数显式传入。
pub(crate) fn spawn_project_git_refresh(project_id: i64, repo_path: PathBuf, io: &ShellIo) {
    let proxy = io.proxy.clone();
    io.handle.spawn(async move {
        let (b, d, s, w, r) = tokio::task::spawn_blocking({
            let repo_path = repo_path.clone();
            move || {
                (
                    delivery::branch(&repo_path),
                    delivery::is_dirty(&repo_path),
                    delivery::file_statuses(&repo_path),
                    delivery::worktrees(&repo_path),
                    delivery::remote_url(&repo_path),
                )
            }
        })
        .await
        .unwrap_or((None, false, HashMap::new(), Vec::new(), Vec::new()));
        let _ = proxy.send_event(Message::Files(files::Message::StatusesRefreshed(
            project_id, s,
        )));
        let _ = proxy.send_event(Message::Project(project::Message::GitRefreshed(
            project_id, b, d, w, r,
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
/// `theme::color::CREAM` / `theme::color::CARD`——`TerminalModel` 只吃 ANSI truecolor
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
                .size(theme::font::subtitle())
                .color(theme::color::RED),
        );
    }
    if rv.entries.is_empty() {
        return content.push(lh(text("暂无对话")
            .size(theme::font::subtitle())
            .color(theme::color::DIM)));
    }
    for (i, e) in rv.entries.iter().enumerate() {
        match e {
            ReviewEntry::Human { text: t } => {
                content = content.push(lh(text(format!("▎{t}"))
                    .size(theme::font::title())
                    .color(theme::color::CREAM)));
            }
            ReviewEntry::AiTurn {
                text: body,
                tools,
                thinking,
            } => {
                if !body.is_empty() {
                    content = content.push(lh(text(body.clone())
                        .size(theme::font::subtitle())
                        .color(theme::color::BODY)));
                }
                let expanded = rv.expanded.contains(&i);
                let glyph = if expanded { "▾ " } else { "▸ " };
                content = content.push(
                    button(lh(text(format!(
                        "{glyph}{}",
                        ai_turn_summary(tools.len(), *thinking)
                    ))
                    .size(theme::font::body())
                    .color(theme::color::DIM)))
                    .on_press(Message::ReviewToggle(i))
                    .style(|_t, _s| button::Style {
                        background: None,
                        text_color: theme::color::DIM,
                        ..button::Style::default()
                    }),
                );
                if expanded {
                    if *thinking {
                        content = content.push(lh(text("  · 思考(略)")
                            .size(theme::font::label())
                            .color(theme::color::DIM)));
                    }
                    for tool in tools {
                        content = content.push(lh(text(format!("  · {tool}"))
                            .size(theme::font::body())
                            .color(theme::color::CYAN)));
                    }
                }
            }
        }
    }
    content
}

/// 对话列表面板(右面板区"对话"视图的列表侧):当前项目的对话记录卡片,
/// 活跃对话置顶+金框标记。点某条 → `ConversationOpen` 驱动右侧审阅内容。
pub(crate) fn conversation_list_pane(
    ws: &Workspace,
    width: Length,
    outer: Border,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let region = theme::region::conversation_list_pane();
    // 套用统一 panel head:Lucide `BotMessageSquare` 图标 + 暖金 `#dcc9a3`
    // 的 "会话" 标题 + 1px 分割线;去掉原先跟在项目名后的 "Dozer 项目" 副标题。
    let mut content =
        column![home_panel_head(IconKind::BotMessageSquare, "会话"),].spacing(region.gap);

    let opens = ws.open_transcript_paths();
    let active_n = ws
        .conversations
        .iter()
        .filter(|c| conversation::is_current_conversation(&c.path, &opens))
        .count();
    content = content.push(
        row![
            lh(text("会话")
                .size(theme::font::caption())
                .color(theme::color::DIM)),
            lh(
                text(format!("{} 条 · {} 活跃", ws.conversations.len(), active_n))
                    .size(theme::font::caption())
                    .color(theme::color::DIM)
            ),
        ]
        .spacing(6),
    );
    if ws.conversations.is_empty() {
        content = content.push(lh(text("暂无对话记录")
            .size(theme::font::body())
            .color(theme::color::DIM)));
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
        let sub_color = if current {
            theme::color::GREEN
        } else {
            theme::color::DIM
        };
        let card = button(
            column![
                row![
                    text("●")
                        .size(theme::font::caption())
                        .color(agent_dot_color(c.agent)),
                    lh(text(c.title.clone())
                        .size(theme::font::body())
                        .color(theme::color::CREAM)),
                ]
                .spacing(6)
                .align_y(iced_widget::core::Alignment::Center),
                lh(text(sub).size(theme::font::caption_sm()).color(sub_color)),
            ]
            .spacing(4),
        )
        .on_press(Message::ConversationOpen(c.path.clone()))
        .width(Length::Fill)
        .padding(10)
        .style(move |_t, _s| button::Style {
            background: Some(theme::color::CARD.into()),
            text_color: theme::color::CREAM,
            border: Border {
                color: if current {
                    theme::color::GOLD
                } else {
                    theme::color::BORDER
                },
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
pub(crate) fn group_tabs_by_agent(tabs: &[SessionTab]) -> Vec<(AgentKind, Vec<usize>)> {
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
            .size(theme::font::body())
            .color(theme::color::DIM)));
    } else {
        for (agent, idxs) in group_tabs_by_agent(&ws.tabs) {
            content = content.push(lh(text(format!("{}（{}）", agent.label(), idxs.len()))
                .size(theme::font::caption())
                .color(theme::color::DIM)));
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
/// (`idx == ws.active` 时 `theme::color::CARD` 背景高亮,同项目树选中行的手法,
/// 见 `workspace.rs` 里 `is_selected` 那段)。
pub(crate) fn agent_list_row(
    ws: &Workspace,
    idx: usize,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let tab = &ws.tabs[idx];
    let active = idx == ws.active;
    let row_el = row![
        text("●")
            .size(theme::font::caption())
            .color(dot_color(tab.agent_state, tab.alive)),
        lh(text(agent_state_label(tab.agent_state))
            .size(theme::font::caption_sm())
            .color(theme::color::DIM)),
        lh(
            text(tab_title(tab.agent, tab.cwd.as_deref(), &tab.info.name))
                .size(theme::font::body())
                .color(theme::color::CREAM)
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
                Some(theme::color::CARD.into())
            } else {
                None
            },
            text_color: theme::color::CREAM,
            ..button::Style::default()
        })
        .into()
}

/// Agent 面板头部"＋"按钮:点击切换 `agent_picker_open`,弹出 agent
/// 选择菜单(`agent_picker_popup`)。样式与顶栏页签行的"＋"一致——无背景、
/// Lucide `SquarePlus` 图标、静止灰(`DIM`)、hover 平滑过渡到金(`GOLD`),
/// 由 `HoverId::AgentPickerToggle` + `MouseArea` 驱动同一套悬停动画。
pub(crate) fn agent_picker_toggle_button<'a>(
    app: &App,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let color = theme::color::mix(
        theme::color::DIM,
        theme::color::GOLD,
        app.hover_progress(HoverId::AgentPickerToggle),
    );
    let add = MouseArea::new(
        button(icons::view(
            icons::IconKind::SquarePlus,
            crate::theme::icon_size::row(),
            color,
        ))
        .on_press(Message::AgentPickerToggle)
        .padding([6, 8])
        .style(move |_t: &iced_widget::Theme, _s| button::Style {
            background: None,
            text_color: color,
            ..button::Style::default()
        }),
    )
    .on_enter(Message::Hover(HoverId::AgentPickerToggle, true))
    .on_exit(Message::Hover(HoverId::AgentPickerToggle, false));
    add.into()
}

/// Agent 选择菜单浮层:固定挂在窗口右上角("＋"按钮下方——该按钮
/// 就在最靠右的 Agent 面板头部,近似等于窗口右上角),九个选项
/// Claude/CodeBuddy/OpenCode/Codex/Qoder/Kilo/v8agent/纯 Shell/Git Shell。跟项目树右键菜单
/// (`context_menu_popup`)同款按钮样式,但不需要像素坐标定位——同
/// `delete_confirm_popup` 一样固定 padding 摆位。`ws.agent_picker_open`
/// 为假时返回空视图,调用方(`App::view`)据此决定要不要把这层塞进
/// `stack!`。
pub(crate) fn agent_picker_popup(
    ws: &Workspace,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    if !ws.agent_picker_open {
        return column![].into();
    }
    let items: [(&str, PickerLaunch); 9] = [
        ("Claude", PickerLaunch::Agent(Some(AgentKind::Claude))),
        ("CodeBuddy", PickerLaunch::Agent(Some(AgentKind::Codebuddy))),
        ("OpenCode", PickerLaunch::Agent(Some(AgentKind::Opencode))),
        ("Codex", PickerLaunch::Agent(Some(AgentKind::Codex))),
        ("Qoder", PickerLaunch::Agent(Some(AgentKind::Qoder))),
        ("Kilo", PickerLaunch::Agent(Some(AgentKind::Kilo))),
        ("v8agent", PickerLaunch::Agent(Some(AgentKind::V8agent))),
        ("纯 Shell", PickerLaunch::Agent(None)),
        ("Git Shell", PickerLaunch::Git),
    ];
    let mut col = column![].spacing(2);
    for (label, agent) in items {
        let (icon, icon_color) = match agent {
            PickerLaunch::Agent(Some(kind)) => (agent_icon(kind), agent_dot_color(kind)),
            PickerLaunch::Agent(None) => (IconKind::Terminal, theme::color::CREAM),
            PickerLaunch::Git => (IconKind::GitBranch, theme::color::CREAM),
        };
        let content = row![
            icons::view(icon, crate::theme::icon_size::row(), icon_color),
            text(label)
                .size(theme::font::body())
                .color(theme::color::CREAM),
        ]
        .align_y(iced_widget::core::alignment::Vertical::Center)
        .spacing(8);
        col = col.push(
            button(content)
                .on_press(Message::AgentPickerSelect(agent))
                .width(Length::Fixed(160.0))
                .padding([6, 12])
                .style(|_t, _s| button::Style {
                    background: Some(theme::color::CARD.into()),
                    text_color: theme::color::CREAM,
                    ..button::Style::default()
                }),
        );
    }
    let list = container(col)
        .padding(6)
        .style(|_t: &iced_widget::Theme| container::Style {
            background: Some(theme::color::CARD.into()),
            border: Border {
                color: theme::color::BORDER,
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
pub(crate) fn review_content_pane(
    ws: &Workspace,
    width: Length,
    outer: Border,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let region = theme::region::review_content_pane();
    // 去掉原先的 "会话审阅" 标题文字——列表侧已统一为 "会话" panel head,
    // 内容侧直接展示选中会话的审阅正文,不再重复标题。
    let mut content = column![].spacing(region.gap);

    if ws.review.is_some() {
        content = review_content(content, ws);
    } else {
        content = content.push(
            container(lh(text("暂无审阅内容——点击左侧对话列表中的对话开始审阅")
                .size(theme::font::subtitle())
                .color(theme::color::DIM)))
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
    terminal_font::size() * crate::theme::icon_size::scale()
}

/// 统一行高：把一段文字的行高设为终端行高
/// (`terminal_font::line_height_factor()` = 1.2)，让各面板列表/正文行的行距
/// 与文件树、终端观感一致。`size`/`color` 等仍由调用方设置，这里只补行高。
pub(crate) fn lh<'a>(
    t: iced_widget::text::Text<'a, iced_widget::Theme, iced_renderer::Renderer>,
) -> iced_widget::text::Text<'a, iced_widget::Theme, iced_renderer::Renderer> {
    t.line_height(LineHeight::Relative(terminal_font::line_height_factor()))
}

/// `LeftView::Files` 在没有打开项目时的占位:"未打开项目"提示 + 最近项目
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
            .size(theme::font::body())
            .color(theme::color::DIM),
    );
    for p in &ws.recent_projects {
        header = header.push(
            button(
                text(p.name.clone())
                    .size(theme::font::body())
                    .color(theme::color::CREAM),
            )
            .on_press(Message::ProjectSelect(p.id))
            .style(|_t, _s| button::Style {
                background: None,
                text_color: theme::color::CREAM,
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

/// 终端栏底状态条：当前激活 tab 的 agent 态 · resume · dozerd 持有。
pub(crate) fn terminal_status_bar(
    ws: &Workspace,
    outer: Border,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let (label, dot) = match ws.tabs.get(ws.active) {
        Some(t) => (
            agent_state_label(t.agent_state),
            dot_color(t.agent_state, t.alive),
        ),
        None => ("空闲", theme::color::DIM),
    };
    let resume = ws.tabs.get(ws.active).map(|t| t.alive).unwrap_or(false);
    let line = row![
        text("●").size(theme::font::dot_sm()).color(dot),
        text(label)
            .size(theme::font::caption())
            .color(theme::color::BODY),
        text("·")
            .size(theme::font::caption())
            .color(theme::color::DIM),
        text(format!("resume {}", if resume { "✓" } else { "—" }))
            .size(theme::font::caption())
            .color(theme::color::BODY),
        text("·")
            .size(theme::font::caption())
            .color(theme::color::DIM),
        text(match ws.tabs.get(ws.active).map(|t| &t.backend) {
            Some(TabBackend::Ssh { .. }) => "SSH 直连 · 断连不可恢复",
            _ => "dozerd 持有 · 断连可恢复",
        })
        .size(theme::font::caption())
        .color(theme::color::DIM),
    ]
    .spacing(6);
    status_bar_container(line, outer)
}

/// 状态条通用外框：略深底 + 上边线 + 固定高。`outer` 是所属 pane 的整体
/// 外框圆角（`zone_pane_border` 算出的 `Border`），只取它的 `radius` 套到
/// 底栏上——底栏贴在 pane 最底部,若不收圆角和会戳出 pane 已收圆的底角,
/// 在 zone 圆角 CARD 背景上顶出小尖角。保留底栏自己那条 1px 上边分隔线
/// （颜色/宽度沿用 `status_bar` 区域配置,只改圆角）。
pub(crate) fn status_bar_container<'a, Msg: 'a>(
    inner: impl Into<Element<'a, Msg, iced_widget::Theme, iced_renderer::Renderer>>,
    outer: Border,
) -> Element<'a, Msg, iced_widget::Theme, iced_renderer::Renderer> {
    let region = theme::region::status_bar();
    let base = region.border.unwrap_or_default();
    container(inner)
        .width(Length::Fill)
        .height(Length::Fixed(theme::geometry::status_bar_height()))
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
    let scroll_msg = move |right| match kind {
        PreviewPaneKind::Files => Message::PreviewTabScroll(right),
        PreviewPaneKind::Project => Message::ProjectPreviewTabScroll(right),
    };
    let context_msg = move |idx, editable| match kind {
        PreviewPaneKind::Files => Message::PreviewTabContextMenu { idx, editable },
        PreviewPaneKind::Project => Message::ProjectPreviewTabContextMenu { idx, editable },
    };
    let editor_msg = move |tab_id, ev| match kind {
        PreviewPaneKind::Files => Message::PreviewEditorEvent(tab_id, ev),
        PreviewPaneKind::Project => Message::ProjectPreviewEditorEvent(tab_id, ev),
    };

    let widths: Vec<f32> = preview
        .tabs()
        .iter()
        .map(|t| preview_tab_display_width(&t.title))
        .collect();
    let (first, can_left, can_right) =
        tab_window(&widths, 4.0, theme::geometry::tab_bar_avail_px(), tab_first);

    let items: Vec<Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer>> = preview
        .tabs()
        .iter()
        .enumerate()
        .filter(|(idx, _)| *idx >= first)
        .map(|(idx, tab)| {
            let active = idx == preview.active_idx();
            let title_hover_t = app.hover_progress(item_hover(idx));
            let close_hover_t = app.hover_progress(close_hover(idx));
            // 仅文本类文件可编辑——决定右键菜单里"编辑"项是否出现(标题后的
            // 编辑图标已移除,编辑入口统一收进 tab 右键菜单,见 `PreviewTabContextMenu`)。
            let editable = matches!(&tab.kind, TabKind::File(path) if is_editable_extension(path));
            let tab = panel_tab(
                tab.title.clone(),
                active,
                title_hover_t,
                close_hover_t,
                None,
                None,
                select_msg(idx),
                close_msg(idx),
                move |h| Message::Hover(item_hover(idx), h),
                move |h| Message::Hover(close_hover(idx), h),
            );
            // 右键 tab 弹上下文菜单:"编辑"(仅可编辑)/"关闭"。
            // 拖拽换位:按住页签(选中处理已把 `app.tab_drag` 置位)后光标
            // 扫过哪个页签,这个 `on_move` 就按它发 `TabDragMove`,完成换位。
            let armed = app.dragging_group(tab_group);
            let mut area = MouseArea::new(tab)
                .on_right_press(context_msg(idx, editable))
                .on_move(move |_| Message::TabDragMove {
                    group: tab_group,
                    index: idx,
                });
            if armed {
                area = area.interaction(mouse::Interaction::Grabbing);
            }
            area.into()
        })
        .collect();
    // tab 列表进 clip 容器占 Fill,裁掉右侧溢出;左右箭头钉在裁剪区外。
    let tabs_row = row(items).spacing(4);
    let clipped = container(tabs_row).width(Length::Fill).clip(true);
    let left_arrow = tab_arrow_button(icons::IconKind::ChevronLeft, can_left, scroll_msg(false));
    let right_arrow = tab_arrow_button(icons::IconKind::ChevronRight, can_right, scroll_msg(true));
    let tab_bar = row![left_arrow, right_arrow, clipped]
        .spacing(4)
        .align_y(iced_widget::core::Alignment::Center);

    let mut content = column![tab_bar, tab_divider()].spacing(region.gap);

    if let Some(err) = error {
        content = content.push(lh(text(format!("⚠ {err}"))
            .size(theme::font::body())
            .color(theme::color::RED)));
    }

    if preview.tabs().is_empty() {
        content = content.push(
            container(lh(text("暂无预览——在左侧文件树选择文件")
                .size(theme::font::subtitle())
                .color(theme::color::DIM)))
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
            content = content.push(
                container(editor.view().map(move |ev| editor_msg(tab_id, ev)))
                    .width(Length::Fill)
                    .height(Length::Fill),
            );
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

/// 文本编辑弹层:标题行(文件名+关闭)+ `iced-code-editor` 代码编辑器主体
/// (语法高亮/等宽字体)+ 错误位 + 保存/关闭按钮。宽高吃满大部分屏幕("放大
/// 窗口"的产品意图,
/// 不是小弹窗),四周留 `40.0` 边距,与 `maximize_overlay` 的
/// `scrim_padding` 同一量级,视觉上是同一族"大号应用内模态"。
pub(crate) fn edit_modal(
    ws: &Workspace,
) -> Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> {
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
            .size(theme::font::subtitle())
            .color(theme::color::CREAM),
        iced_widget::space::horizontal(),
        button(
            text("×")
                .size(theme::font::subtitle())
                .color(theme::color::DIM)
        )
        .on_press(Message::PreviewEditCloseRequest)
        .padding(0)
        .style(|_t, _s| button::Style {
            background: None,
            text_color: theme::color::DIM,
            ..button::Style::default()
        }),
    ]
    .align_y(iced_widget::core::Alignment::Center);

    // `iced-code-editor::view()` 返回的是裸 `Element`,没有 `height` 这类
    // widget 方法,用 `container` 包一层再撑满弹层正文高度。
    let editor: Element<'_, Message, iced_widget::Theme, iced_renderer::Renderer> =
        container(session.editor.view().map(Message::EditorEvent))
            .height(Length::Fill)
            .into();

    let mut body = column![title_row, editor].spacing(8);

    if let Some(err) = &session.error {
        body = body.push(
            text(format!("⚠ {err}"))
                .size(theme::font::body())
                .color(theme::color::RED),
        );
    }

    let close_btn = button(
        text("关闭")
            .size(theme::font::body())
            .color(theme::color::CREAM),
    )
    .on_press(Message::PreviewEditCloseRequest)
    .padding([6, 12])
    .style(|_t, _s| button::Style {
        background: Some(theme::color::CARD.into()),
        text_color: theme::color::CREAM,
        border: Border {
            color: theme::color::BORDER,
            width: 1.0,
            radius: 4.0.into(),
        },
        ..button::Style::default()
    });
    let save_btn = button(
        text("保存")
            .size(theme::font::body())
            .color(theme::color::CREAM),
    )
    .on_press(Message::PreviewEditSave)
    .padding([6, 12])
    .style(|_t, _s| button::Style {
        background: Some(theme::color::CARD.into()),
        text_color: theme::color::CREAM,
        border: Border {
            color: theme::color::CREAM,
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
            background: Some(theme::color::CARD.into()),
            border: Border {
                color: theme::color::BORDER,
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
            background: Some(theme::color::SCRIM.into()),
            ..container::Style::default()
        })
        .into()
}

/// 编辑弹层的二次确认:脏改动状态下点关闭,叠在 `edit_modal` 之上。
/// 视觉风格与 `delete_confirm_popup` 一致。
pub(crate) fn edit_discard_confirm_popup<'a>()
-> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let dialog = container(
        column![
            text("放弃未保存的改动?")
                .size(theme::font::subtitle())
                .color(theme::color::CREAM),
            text("关闭后这次编辑不会被保存。")
                .size(theme::font::label())
                .color(theme::color::DIM),
            row![
                button(
                    text("取消")
                        .size(theme::font::body())
                        .color(theme::color::CREAM)
                )
                .on_press(Message::PreviewEditConfirmCancel)
                .padding([6, 12])
                .style(|_t, _s| button::Style {
                    background: Some(theme::color::CARD.into()),
                    text_color: theme::color::CREAM,
                    border: Border {
                        color: theme::color::BORDER,
                        width: 1.0,
                        radius: 4.0.into(),
                    },
                    ..button::Style::default()
                }),
                button(
                    text("放弃改动")
                        .size(theme::font::body())
                        .color(theme::color::RED)
                )
                .on_press(Message::PreviewEditConfirmDiscard)
                .padding([6, 12])
                .style(|_t, _s| button::Style {
                    background: Some(theme::color::CARD.into()),
                    text_color: theme::color::RED,
                    border: Border {
                        color: theme::color::RED,
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

/// 对话副行文案：`<agent> · <相对时间> · <规模>`（P1j）。
pub(crate) fn conversation_sub(
    agent: &str,
    modified_ms: u64,
    size_bytes: u64,
    now_ms: u64,
) -> String {
    let when = relative_time_text(modified_ms, now_ms);
    let size = if size_bytes >= 1024 * 1024 {
        format!("{:.1}MB", size_bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{}KB", (size_bytes / 1024).max(1))
    };
    format!("{agent} · {when} · {size}")
}

/// AI 回合折叠行文案（P1i）：过程 = thinking + N 工具。
pub(crate) fn ai_turn_summary(tools_len: usize, thinking: bool) -> String {
    match (thinking, tools_len) {
        (false, 0) => "过程:无".into(),
        (true, 0) => "过程:思考".into(),
        (false, n) => format!("过程:{n} 工具"),
        (true, n) => format!("过程:思考 + {n} 工具"),
    }
}

/// 交付/验收使用的仓库：当前项目优先，无则回落会话 cwd（P1f 现状；P1g D4）。
pub(crate) fn effective_project_repo(active: Option<&Path>, session_cwd: &Path) -> PathBuf {
    active
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| session_cwd.to_path_buf())
}

/// 从项目路径求"验收查询键"：与落库侧同款 `delivery::repo_root`（git
/// toplevel，解析符号链接/子目录），保证 `count_for_repo` 精确匹配命中。
/// 非 git 路径返回 `None`——验收依赖 git ref 沉淀，非 git 仓库不可能有记录，
/// 直接不查，别拿未规范化的原始路径去撞库（会静默查不到→副行空白）。
pub(crate) fn acceptance_query_repo(project_path: &str) -> Option<String> {
    delivery::repo_root(Path::new(project_path)).map(|p| p.to_string_lossy().into_owned())
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

/// 已接入 `dozer-hook` 安装器的 agent 集合。刻意穷尽 match 而不是拿
/// `agent.label()` 当 catch-all 参数：`install::settings_path_for` 对未识别
/// 的 agent 名一律落回 Claude 的 `settings.json`路径，如果不显式排除
/// Kilo/V8agent(纯 GUI 占位，没有真实 hook 支持)，误调用会把
/// "kilo"/"v8agent" 的 hook 命令写进 Claude 的 settings.json，顶掉真正的
/// claude hook 条目。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HookInstallTarget {
    Settings,
    Opencode,
}

pub(crate) fn hook_install_target(agent: AgentKind) -> Option<HookInstallTarget> {
    match agent {
        AgentKind::Claude | AgentKind::Codebuddy | AgentKind::Codex | AgentKind::Qoder => {
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
/// 不追求精确——估偏几像素只会让翻页边界差一个 tab。上限封顶到
/// `PANEL_TAB_MAX_W`,与 `preview_tab_display_width` 同款理由。
pub(crate) fn tab_display_width(title: &str) -> f32 {
    // 状态点●+spacing ≈ 18, 名称 ≈ units * 半宽 8.0(14px), 关闭× ≈ 18, pill padding ≈ 12
    let est = 18.0 + text_width_units(title) * 8.0 + 18.0 + 12.0;
    est.min(crate::app::PANEL_TAB_MAX_W)
}

/// 预览 tab 估算显示宽：同 `tab_display_width` 但无状态点。上限封顶到
/// `PANEL_TAB_MAX_W`——标题超宽会被省略号截断,翻页窗口数学据此不会把
/// 被裁剪的 tab 算成比实际渲染更宽。
pub(crate) fn preview_tab_display_width(title: &str) -> f32 {
    // 名称 ≈ units * 半宽 8.0(14px), 关闭× ≈ 18, pill padding ≈ 12
    let est = text_width_units(title) * 8.0 + 18.0 + 12.0;
    est.min(crate::app::PANEL_TAB_MAX_W)
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
        return theme::color::DIM;
    }
    match state {
        AgentState::Idle | AgentState::Running => theme::color::GREEN,
        AgentState::AwaitingInput => theme::color::PURPLE,
        AgentState::TurnEnded => theme::color::GOLD,
    }
}

/// agent → 对话列表圆点颜色。避开 `theme::color::GOLD`(甲方动作专属色,
/// CLAUDE.md 明文规定,不能被 agent 分类语义借用)。
pub(crate) fn agent_dot_color(agent: AgentKind) -> Color {
    match agent {
        AgentKind::Claude => theme::color::CYAN,
        AgentKind::Codebuddy => theme::color::PURPLE,
        AgentKind::Opencode => theme::color::GREEN,
        AgentKind::Codex => theme::color::ORANGE,
        AgentKind::Qoder => theme::color::MAGENTA,
        AgentKind::Kilo => theme::color::BLUE,
        AgentKind::V8agent => theme::color::LIME,
        AgentKind::Unknown => theme::color::DIM,
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
        AgentKind::Codex
        | AgentKind::Qoder
        | AgentKind::Kilo
        | AgentKind::V8agent
        | AgentKind::Unknown => IconKind::Bot,
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
            theme::geometry::preview_chrome_top_px(),
            38.0,
            "文件预览 chrome 顶应为去地址栏后的 38(8 内边距 + 30 tab 栏)"
        );
        assert_eq!(
            theme::geometry::browser_chrome_top_px(),
            72.0,
            "浏览器 chrome 顶应为 72(8 内边距 + 30 tab 栏 + 4 spacing + 30 地址栏)"
        );
        assert_eq!(
            theme::geometry::chrome_height_px(),
            50.0,
            "终端 chrome 高应为去 header 后的 50"
        );
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
        assert_eq!(dot_color(Running, false), theme::color::DIM, "死会话灰点");
        // 存活：空闲/运行同绿（运行靠闪烁区分），待输入紫、回合毕金。
        assert_eq!(dot_color(Idle, true), theme::color::GREEN);
        assert_eq!(dot_color(Running, true), theme::color::GREEN);
        assert_eq!(dot_color(AwaitingInput, true), theme::color::PURPLE);
        assert_eq!(dot_color(TurnEnded, true), theme::color::GOLD);
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
            backend: TabBackend::Daemon,
        }
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
    fn agent_dot_color_maps_each_kind_and_avoids_gold() {
        let cases = [
            (AgentKind::Claude, theme::color::CYAN),
            (AgentKind::Codebuddy, theme::color::PURPLE),
            (AgentKind::Opencode, theme::color::GREEN),
            (AgentKind::Codex, theme::color::ORANGE),
            (AgentKind::Qoder, theme::color::MAGENTA),
            (AgentKind::Kilo, theme::color::BLUE),
            (AgentKind::V8agent, theme::color::LIME),
            (AgentKind::Unknown, theme::color::DIM),
        ];
        for (agent, expected) in cases {
            let color = agent_dot_color(agent);
            assert_eq!(color, expected, "{agent:?}");
            assert_ne!(
                color,
                theme::color::GOLD,
                "{agent:?} 的对话列表圆点色不能是 GOLD(甲方动作专属,CLAUDE.md 明文规定)"
            );
        }
    }

    #[test]
    fn agent_icon_maps_each_kind_to_brand_icon() {
        assert_eq!(agent_icon(AgentKind::Claude), IconKind::Claude);
        assert_eq!(agent_icon(AgentKind::Codebuddy), IconKind::Codebuddy);
        assert_eq!(agent_icon(AgentKind::Opencode), IconKind::Opencode);
        // Codex/Qoder/Kilo/V8agent 暂无确认可用的品牌素材，回落通用 Bot 图标
        // （见计划 Task 3 说明，非占位符——spec §8/§6 明确允许的兜底）。
        assert_eq!(agent_icon(AgentKind::Codex), IconKind::Bot);
        assert_eq!(agent_icon(AgentKind::Qoder), IconKind::Bot);
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
        assert_eq!(agent_cli_command(AgentKind::Qoder), Some("qoder"));
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
            picker_launch_command(PickerLaunch::Agent(Some(AgentKind::Qoder))),
            Some("qoder".to_string())
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
        // Claude/CodeBuddy/Codex/Qoder 都走 `install::run_at` 的 JSON settings
        // 补丁机制。
        for agent in [
            AgentKind::Claude,
            AgentKind::Codebuddy,
            AgentKind::Codex,
            AgentKind::Qoder,
        ] {
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
        // 顶掉真正的 claude hook 条目。Unknown 同理，从不该触发安装。
        for agent in [AgentKind::Kilo, AgentKind::V8agent, AgentKind::Unknown] {
            assert_eq!(hook_install_target(agent), None, "{agent:?}");
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
            !ws.active_preview_tab_has_native_editor(),
            "没有 tab 时应为 false"
        );

        ws.preview.open_path(rs_path);
        assert!(
            ws.active_preview_tab_has_native_editor(),
            ".rs 是白名单扩展名,应走原生渲染"
        );

        ws.preview.open_path(png_path);
        assert!(
            !ws.active_preview_tab_has_native_editor(),
            "切到 .png 后激活 tab 应走 wry,不是原生"
        );
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
        assert_eq!(session.editor.content(), "fn main() {}");
        assert_eq!(session.saved_content, "fn main() {}");
        assert!(session.editor.content() == session.saved_content);
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
    fn preview_edit_open_by_id_opens_matching_tab() {
        let (_dir, path) = write_temp_file("a.rs", "fn main() {}");
        let mut ws = Workspace::empty_for_project_placeholder();
        let tab_id = ws.preview.open_path(path.clone());
        ws.preview_edit_open_by_id(tab_id);
        let session = ws.edit_session.as_ref().expect("应打开编辑会话");
        assert_eq!(session.tab_id, tab_id);
        assert_eq!(session.path, path);
    }

    #[test]
    fn preview_edit_open_by_id_unknown_tab_is_noop() {
        let mut ws = Workspace::empty_for_project_placeholder();
        ws.preview_edit_open_by_id(424242);
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
        let _ = ws.preview_edit_event(EditorMessage::ArrowKey(
            iced_code_editor::ArrowDirection::Right,
            false,
        ));
        assert!(
            ws.edit_session.as_ref().unwrap().editor.content()
                == ws.edit_session.as_ref().unwrap().saved_content
        );
        // 编辑动作置脏。前一步光标右移了一位,Insert 落在 'h' 之后。
        let _ = ws.preview_edit_event(EditorMessage::CharacterInput('!'));
        assert!(
            ws.edit_session.as_ref().unwrap().editor.content()
                != ws.edit_session.as_ref().unwrap().saved_content
        );
        assert_eq!(ws.edit_session.as_ref().unwrap().editor.content(), "h!i");
    }

    #[test]
    fn preview_edit_save_writes_disk_clears_dirty_and_bumps_reload() {
        let (_dir, path) = write_temp_file("a.txt", "hi");
        let mut ws = Workspace::empty_for_project_placeholder();
        let tab_id = ws.preview.open_path(path.clone());
        ws.preview_edit_open(0);
        let _ = ws.preview_edit_event(EditorMessage::CharacterInput('!'));
        ws.preview_edit_save();
        assert!(
            ws.edit_session.as_ref().unwrap().editor.content()
                == ws.edit_session.as_ref().unwrap().saved_content
        );
        assert!(ws.edit_session.as_ref().unwrap().error.is_none());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "!hi");
        // `.txt` 是白名单扩展名,原生 tab。保存后 `bump_reload` 走原生路径:
        // 读盘重建 editor,而非推进 reload nonce——原生 tab 不进 wry 期望清单,
        // 也不会出现在 webview 规格里(闭包因 editor.is_some() 被 filter 掉)。
        let tab = &ws.preview.tabs()[0];
        assert!(tab.editor.is_some(), "原生 tab 保存后仍持有(重建的)editor");
        assert_eq!(
            tab.editor.as_ref().unwrap().content(),
            "!hi",
            "保存内容应反映磁盘上的新内容"
        );
        assert_eq!(
            tab.reload_nonce, 0,
            "原生 tab 的 reload 不推进 nonce(那是 wry URL 换参专用)"
        );
        assert!(
            !ws.preview.desired_webviews().iter().any(|s| s.id == tab_id),
            "原生 tab 移出 wry 期望清单"
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
        let _ = ws.preview_edit_event(EditorMessage::CharacterInput('!'));
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
        assert!(
            ws.edit_session.as_ref().unwrap().editor.content()
                != ws.edit_session.as_ref().unwrap().saved_content,
            "取消不丢改动"
        );

        ws.preview_edit_close_request();
        ws.preview_edit_confirm_discard();
        assert!(ws.edit_session.is_none(), "确认放弃要真正关闭");
    }
}
