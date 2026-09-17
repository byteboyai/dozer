//! Workspace 内核:ProjectRestore/RestorePayload/ReviewSource/ReviewView/SshOut/
//! TabBackend/SessionTab/ShellIo/`struct Workspace` + `impl Workspace`,以及
//! 项目恢复、agent 卡片刷新、git/磁盘用量轮询、review 内容等辅助函数。

use crate::app::{
    DEFAULT_COLS, DEFAULT_ROWS, Message, PROJECT_PREVIEW_ID_OFFSET, PanelKind, ProjectId,
};
use crate::chrome::tab_widget::tab_window_reveal;
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
use crate::osc::{OscEvent, OscScanner};
use crate::preview::{PreviewPane, TabKind};
use crate::preview_state;
use crate::project::FileTree;
use crate::term::term_model::TerminalModel;
use crate::transcript::{self, ReviewEntry};
use dozer_client::{Client, TermEvent};
use dozer_core::protocol::{AgentKind, AgentState, ProjectInfo, SessionInfo};
use iced_widget::text;
use iced_widget::text_editor::Action as EditorAction;
use iced_winit::winit::event_loop::EventLoopProxy;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::runtime::Handle;
use tokio::sync::mpsc;

use super::*;

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
    pub(crate) fn resize_one(
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
