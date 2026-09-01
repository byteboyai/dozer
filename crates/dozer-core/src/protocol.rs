use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::collections::BTreeSet;

/// agent 会话状态（hook 事件驱动的四态机；spec P1e D6）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    #[default]
    Idle,
    Running,
    AwaitingInput,
    TurnEnded,
}

/// 会话当前归属的 agent（协议层从 P1e-P1j 时代的"隐式恒 Claude"升级为
/// 显式字段；P2b 多 agent 支持第一步）。`Unknown` 是首个 hook 事件到达前
/// 的默认值，也是老协议帧缺该字段时的回落值。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    #[default]
    Unknown,
    Claude,
    Codebuddy,
    Opencode,
    Codex,
    Kilo,
    V8agent,
}

impl AgentKind {
    /// 展示用短标签（对话历史副行、GUI 角标）。同时也是启动器菜单键入
    /// 的 CLI 命令名——三家均已核实与官方命令名一致（见计划 Global
    /// Constraints）。
    pub fn label(&self) -> &'static str {
        match self {
            AgentKind::Unknown => "shell",
            AgentKind::Claude => "claude",
            AgentKind::Codebuddy => "codebuddy",
            AgentKind::Opencode => "opencode",
            AgentKind::Codex => "codex",
            AgentKind::Kilo => "kilo",
            AgentKind::V8agent => "v8agent",
        }
    }
}

/// 一次会话总结的产出状态:agent 真的经 dozer-mcp 交回,还是超时后由
/// dozerd 从已摄取对话数据算的启发式兜底(spec 2026-08-27)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SummaryStatus {
    AiGenerated,
    HeuristicFallback,
}

/// 一份持久化的会话总结(`dozerd` 的 `session_summaries` 表一行)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionSummaryPayload {
    pub session_id: String,
    pub agent_kind: AgentKind,
    /// 该会话对应的 transcript conversation_id(取自 `transcript_path` 的
    /// `file_stem()`)。`None` 表示这个会话直到总结产出时都没收到过任何
    /// hook 事件(纯 shell/agent 没配好 hook),启发式兜底也查不到任何数据。
    /// `session_id`(dozerd 自己生成的 PTY 会话 id)与这里的 `conversation_id`
    /// 是两个不相关的 id 空间,不能混用去查 `conversation_turns`。
    pub conversation_id: Option<String>,
    pub title: String,
    pub summary: String,
    pub status: SummaryStatus,
    pub created_ts_ms: u64,
}

/// 单个历史会话(=一份 agent transcript 文件)的索引摘要;由 dozerd 的
/// `TranscriptStore` 摄取落库维护(spec 2026-08-20)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConversationSummary {
    pub conversation_id: String,
    pub agent: AgentKind,
    pub file_path: String,
    pub title: String,
    pub first_ts: u64,
    pub last_ts: u64,
    pub turn_count: u32,
}

/// 会话内一个回合(人类发言 / AI 回复 / 工具执行结果)的明细;`role` 取值
/// `"human"`/`"ai"`/`"tool_result"`(不用枚举是为了跟 sqlite 存储列直接
/// 对应,减一层转换)。`is_error` 只对 `role == "tool_result"` 有意义,
/// 标记这次工具调用是否失败。
/// 一次工具调用的结构化明细：`summary` 是既有的一行摘要（`tool_summary()`
/// 产出，如 "Edit README.md"），`input_json` 是完整 `input`/`arguments`
/// pretty-print 后的 JSON 字符串，`None` 表示没有参数或解析失败。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCallInfo {
    pub summary: String,
    pub input_json: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TurnRecord {
    pub turn_index: i64,
    pub role: String,
    pub content: String,
    /// 一次工具调用的结构化明细 (2026-08-22 起由 `raw_json` 读时解析补上)。
    /// 老协议帧/无工具调用时为 `None`。
    #[serde(default)]
    pub tool_calls: Vec<ToolCallInfo>,
    pub thinking: bool,
    /// 真实思考文本（2026-08-22 起读时解析补上，见
    /// `dozerd::transcripts::parse::extract_turn_trace_detail`）；老协议帧/
    /// 无思考内容时为 `None`。
    #[serde(default)]
    pub thinking_text: Option<String>,
    pub ts: Option<u64>,
    #[serde(default)]
    pub is_error: bool,
    /// 本行(单次 API 调用)的 token 用量,跟 `UsagePayload` 同款四个字段
    /// 同名(2026-08-23 起补上,供审阅面板"轨迹"展示统计用)。`ai` 角色外
    /// 恒为 0。老协议帧缺该字段时回落 0,不是"确实为 0"。
    #[serde(default)]
    pub tokens_in: u64,
    #[serde(default)]
    pub tokens_out: u64,
    #[serde(default)]
    pub tokens_cache_read: u64,
    #[serde(default)]
    pub tokens_cache_write: u64,
}

/// 单个会话的用量统计(token/工具调用/改动文件),已按 `message_key` 做过
/// fork/resume 去重(spec"用量去重"一节)。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UsagePayload {
    pub turns: u32,
    pub tool_calls: u32,
    pub mutating_tool_calls: u32,
    pub files_touched: BTreeSet<String>,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub tokens_cache_read: u64,
    pub tokens_cache_write: u64,
}

/// 预览面板当前上下文：文件路径 + 光标/选区（1-indexed，见 spec
/// "1-indexed 行列" 一节）。无选区时 `start == end` 为光标位置。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PreviewContext {
    pub path: String,
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
    pub has_selection: bool,
    /// 这份上下文被推送时的 Unix 纪元毫秒。没有它的话，一个陈旧的缓存值
    /// （GUI 已退出但 dozerd 还活着、或用户几小时没碰过预览面板）和刚刚
    /// 更新的值长得一模一样，调 MCP tool 的 agent 无从判断新鲜度。
    pub updated_at_ms: u64,
}

/// 项目（甲方资产域的根；P1g）。id 为 dozerd SQLite 主键。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectInfo {
    pub id: i64,
    pub path: String,
    pub name: String,
    /// 用户在 Dozer 里最后一次打开/激活该项目的时间（严格单调活跃戳，
    /// 见 `dozerd::projects`）。仅用于排序/判定"最近用过"，不再作为列表
    /// 展示的更新时间。
    pub last_active_ms: u64,
    /// 创建时间：项目在 Dozer 中首次被新建（首次 `open`）时的时间。
    pub created_ms: u64,
    /// 更新时间（git 感知）：优先取项目仓库最新 commit 时间；若该仓库没有
    /// commit，或 commit 时间早于 `last_active_ms`，则回落为 `last_active_ms`。
    /// 由 dozerd 在 `list()` 时实时计算（不入库），保证每次打开或新提交后
    /// 都能拿到最新值。
    pub updated_ms: u64,
}

/// 收藏夹范围:全局(跨项目共享)或挂靠某个项目(`project_id` 必填)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BookmarkScope {
    Global,
    Project,
}

/// 一条收藏记录。`project_id`:`scope=Global` 时恒为 `None`,
/// `scope=Project` 时是该项目在 `projects` 表里的 `id`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BookmarkInfo {
    pub id: i64,
    pub scope: BookmarkScope,
    pub project_id: Option<i64>,
    pub url: String,
    pub title: String,
    pub created_ms: u64,
}

/// 一条任务(`dozerd` 的 `todos` 表一行)。`id` 是稳定身份,取代 v1 时
/// 靠文本哈希关联元数据的做法(2026-09-01 SQLite 迁移)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TodoInfo {
    pub id: i64,
    pub project_id: i64,
    pub text: String,
    pub done: bool,
    /// 排序键:同一 project 下,展示查询固定 `ORDER BY done, rank`——待办
    /// 按 rank 升序在前,已完成按 rank 升序沉底,同一 done 分组内比较才
    /// 有意义,跨分组数值不保证可比。
    pub rank: i64,
    pub created_ms: u64,
    pub completed_at_ms: Option<u64>,
    /// "MM-DD" 格式,GUI 日历选择器写入。
    pub plan_date: Option<String>,
    pub dispatch_session_id: Option<String>,
    pub dispatch_at_ms: Option<u64>,
    /// 所属分类节点 id,`None` = 未分类(2026-09-01 分类树设计新增)。
    #[serde(default)]
    pub category_id: Option<i64>,
}

/// 一个分类树节点(`dozerd` 的 `todo_categories` 表一行)。`parent_id ==
/// None` 表示顶层节点。作用域按 `project_id` 隔离,不跨项目共享
/// (2026-09-01 分类树设计)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CategoryInfo {
    pub id: i64,
    pub project_id: i64,
    pub parent_id: Option<i64>,
    pub name: String,
    /// 同一 `parent_id` 下的兄弟排序键,升序展示。
    pub rank: i64,
    pub created_ms: u64,
}

/// `MoveCategorySibling` 的方向:与同一 `parent_id` 下相邻的前一个/
/// 后一个兄弟节点交换 `rank`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CategoryMoveDirection {
    Up,
    Down,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub name: String,
    pub command: String,
    pub cwd: String,
    pub alive: bool,
    pub created_ms: u64,
    /// 会话内 agent 的最新状态；旧协议帧无此字段时回落 Idle。
    #[serde(default)]
    pub agent_state: AgentState,
    /// 当前会话 agent 的 transcript 文件路径（Claude Code JSONL；hook 携带）。
    #[serde(default)]
    pub transcript_path: Option<String>,
    /// 会话归属的项目 id（多项目并行；P2a）。`None` 表示迁移期孤儿会话——
    /// daemon 重启前已存活、早于本字段引入时创建的会话，首次读出时没有
    /// 归属信息，不强行捏造一个。
    #[serde(default)]
    pub project_id: Option<i64>,
    /// 会话归属的 agent；首个 hook 事件到达前恒 `Unknown`（P2b）。
    #[serde(default)]
    pub agent: AgentKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    ListSessions,
    CreateSession {
        name: String,
        command: String,
        args: Vec<String>,
        cwd: String,
        cols: u16,
        rows: u16,
        /// 这个会话属于哪个项目——GUI 侧发起 CreateSession 时必须显式指定，
        /// 不再像单项目时代那样只靠一个隐式全局"当前项目"（P2a）。
        project_id: i64,
    },
    Attach {
        session_id: String,
        from_offset: u64,
    },
    Write {
        session_id: String,
        data_b64: String,
    },
    Resize {
        session_id: String,
        cols: u16,
        rows: u16,
    },
    Kill {
        session_id: String,
    },
    /// dozer-hook 单向上报的 agent hook 事件；data 原样透传（P1f 消费）。
    HookEvent {
        session_id: String,
        /// 触发这次事件的 agent；由 dozer-hook/opencode 插件在装的时候
        /// 写死，不是猜出来的（spec §3）。
        #[serde(default)]
        agent: AgentKind,
        event: String,
        ts_ms: u64,
        data: serde_json::Value,
    },
    /// 列出某 cwd 下的历史对话(跨 Claude/CodeBuddy/OpenCode 三家合并;
    /// `agent` 非空时只查该家)。spec 2026-08-20。
    ListConversations {
        cwd: String,
        agent: Option<AgentKind>,
        limit: u32,
        offset: u32,
    },
    /// 单个会话的回合明细,keyset 分页(`after_turn_index=-1` 表示从头)。
    GetConversationTurns {
        conversation_id: String,
        after_turn_index: i64,
        limit: u32,
    },
    /// 某 cwd 下会话列表，每行附上该会话的总结(来自 `session_summaries`，
    /// 无总结的会话第二项为 `None`)。对话面板 session 列表/详情导航用
    /// (spec 2026-08-27)。
    ListConversationsWithSummaries {
        cwd: String,
        agent: Option<AgentKind>,
        limit: u32,
        offset: u32,
    },
    /// 某 cwd 下按会话分组的用量统计。
    GetUsageSummary {
        cwd: String,
        since_ts: Option<u64>,
    },
    /// 验收通过的结构性记录（spec P1f D5）；acceptor 由 daemon 侧补 "user"。
    RecordAcceptance {
        repo: String,
        goal: String,
        criteria_checked: Vec<String>,
        verdict: String,
        comment: String,
        ref_name: String,
        ts_ms: u64,
    },
    /// 打开一个目录为项目（已存在则更新活跃时间，返回该项目信息）。
    /// P2a 起不再有"顺带置为当前项目"的副作用——daemon 不维护活跃项目
    /// 概念，"当前显示哪个项目"完全是 GUI 侧的本地状态。
    OpenProject {
        path: String,
    },
    /// 补录一个项目目录下、dozerd 还没摄取过的 agent transcript 历史
    /// (以及追平任何已摄取文件里新增的尾部内容)。`cwd` 是项目根目录的
    /// 绝对路径字符串，跟 `OpenProject.path` 同一种形状。
    BackfillProjectTranscripts {
        cwd: String,
    },
    /// 列出所有项目（按活跃时间倒序）。
    ListProjects,
    /// 项目改名。`id` 不存在或 `name` trim 后为空 → `Reply::Error`。成功复用
    /// `Reply::Project { project: Some(更新后的项目) }`。
    RenameProject {
        id: i64,
        name: String,
    },
    /// 从 dozerd 登记里彻底移除这个项目(层级 `DozerOnly` 起都会发)。
    /// `id` 不存在 → `Reply::Error`。成功 → 裸 `Reply::Ok`。
    RemoveProject {
        id: i64,
    },
    /// 删掉这个项目在三家 agent 存储目录下已摄取的 conversations/
    /// conversation_turns 数据(层级 `WithAgentCache` 起才发)。`cwd` 跟
    /// `OpenProject.path` 同一种形状——原始项目根目录绝对路径,不是
    /// agent 存储目录本身(那个由 dozerd 内部用 `agent_paths` 算)。
    DeleteProjectTranscripts {
        cwd: String,
    },
    /// 取某仓库的验收次数（项目卡"N 次验收"用）。
    GetAcceptanceCount {
        repo: String,
    },
    /// 加入收藏(幂等:同 scope+project_id+url 已存在则 no-op)。
    AddBookmark {
        scope: BookmarkScope,
        project_id: Option<i64>,
        url: String,
        title: String,
    },
    /// 移除收藏(按记录 id;不存在则 no-op)。
    RemoveBookmark {
        id: i64,
    },
    /// 列出"全局 + 指定项目"的收藏合集;`project_id: None` 时只返回全局。
    ListBookmarks {
        project_id: Option<i64>,
    },
    /// `dozer-app` 预览面板变化时推送最新上下文；`context: None` 表示当前
    /// 无活动文本预览。`dozerd` 侧纯内存缓存，同一 `project_id` 后写覆盖
    /// 前写。
    UpdatePreviewContext {
        project_id: i64,
        context: Option<PreviewContext>,
    },
    /// `dozer-mcp` 按需查询某项目当前的预览上下文。
    GetPreviewContext {
        project_id: i64,
    },
    /// `dozer-mcp` 的写工具提交一份会话总结;`dozerd` 只做"session_id 是否
    /// 存在于 registry"的存在性检查,不做权限校验(与 `Write`/`HookEvent`
    /// 同等信任本机调用方)。主键 `session_id`,重复提交后到覆盖先到。
    RecordSessionSummary {
        session_id: String,
        title: String,
        summary: String,
    },
    /// 查询某会话是否已有总结(`None` 表示尚未生成或本会话不适用)。
    GetSessionSummary {
        session_id: String,
    },
    /// 关闭 tab 时触发"总结后再 kill":dozerd 立即返回 Ok,实际注入 prompt/
    /// 轮询/超时兜底/kill 全部在后台异步完成,调用方不等待(spec
    /// 2026-08-27)。
    CloseWithSummary {
        session_id: String,
    },
    /// 补录一个项目目录下缺失 `session_summaries` 行的历史会话总结(项目
    /// "修复"按钮触发)。立即返回 `Reply::Ok`——实际处理在 dozerd 后台异步
    /// 完成,`total` 在返回 Ok 之前已同步算好并写进内存态进度表,调用方拿到
    /// Ok 后即可放心轮询 `GetSessionSummaryBackfillStatus` 不会撞见"还没算出
    /// total"的空窗期(spec 2026-08-28)。
    BackfillSessionSummaries {
        cwd: String,
    },
    /// 查询某 cwd 下补总结后台任务的进度。找不到对应记录(还没发起过/
    /// dozerd 重启后内存态丢失)时约定回 `total=0, completed=0`,调用方据此
    /// 判定"无进行中任务"。
    GetSessionSummaryBackfillStatus {
        cwd: String,
    },
    /// 列出某项目全部任务,`ORDER BY done, rank` 排好序返回。
    ListTodos {
        project_id: i64,
    },
    /// 新增一条待办,置顶(rank 小于当前最小待办 rank)。
    AddTodo {
        project_id: i64,
        text: String,
    },
    /// 勾选/取消勾选;`done` 从假变真时服务端顺带写 `completed_at_ms`,
    /// 真变假时清空。`id` 不存在 → `Reply::Error`。
    ToggleTodo {
        id: i64,
        done: bool,
    },
    /// 改任务文字。`id` 不存在 → `Reply::Error`。
    EditTodoText {
        id: i64,
        text: String,
    },
    /// 把 `id` 挪到 `after_id` 之后(`None` = 待办块最前);只在待办子集内
    /// 生效,服务端对该 project 的待办子集做一次完整 rank 重编号。
    ReorderTodo {
        id: i64,
        after_id: Option<i64>,
    },
    /// 写/清计划日期,`None` 表示清空。
    SetTodoPlanDate {
        id: i64,
        plan_date: Option<String>,
    },
    /// 记录一次派发(指派到已有会话)。
    RecordTodoDispatch {
        id: i64,
        session_id: String,
    },
    /// 列出某项目全部分类节点,扁平返回(不分页,数据量小)。
    ListCategories {
        project_id: i64,
    },
    /// 新增分类,追加到 `parent_id` 下兄弟节点末尾。
    AddCategory {
        project_id: i64,
        parent_id: Option<i64>,
        name: String,
    },
    /// 重命名。`id` 不存在 → `Reply::Error`。
    RenameCategory {
        id: i64,
        name: String,
    },
    /// 删除该节点及其全部子孙节点;原本挂在这棵子树下的任务全部降级为
    /// 未分类(`category_id = NULL`),任务本身不删除。`id` 不存在 →
    /// `Reply::Error`。
    DeleteCategory {
        id: i64,
    },
    /// 重新挂到 `new_parent_id` 下(`None` = 顶层),追加到新父节点子级
    /// 末尾。目标是自己或自己的子孙时 → `Reply::Error`(防止成环)。
    ReparentCategory {
        id: i64,
        new_parent_id: Option<i64>,
    },
    /// 与同一 `parent_id` 下相邻的前一个/后一个兄弟节点交换 `rank`。已经
    /// 在最前/最后时对应方向是 no-op(仍返回 `Reply::Category`,不报错)。
    MoveCategorySibling {
        id: i64,
        direction: CategoryMoveDirection,
    },
    /// 挂/摘任务的分类。`category_id: None` = 摘掉分类,变回未分类。
    SetTodoCategory {
        id: i64,
        category_id: Option<i64>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Reply {
    Sessions {
        sessions: Vec<SessionInfo>,
    },
    Created {
        session: SessionInfo,
    },
    Attached {
        session_id: String,
        snapshot_b64: String,
        next_offset: u64,
    },
    Output {
        session_id: String,
        data_b64: String,
        offset: u64,
    },
    Exited {
        session_id: String,
        code: Option<i32>,
    },
    Ok,
    Error {
        message: String,
    },
    /// hook 事件引起的状态变更，随 attach 流广播给该会话的订阅者。
    AgentEvent {
        session_id: String,
        /// 该事件所属 agent；旧协议帧无此字段时回落 Unknown。
        #[serde(default)]
        agent: AgentKind,
        state: AgentState,
        event: String,
        ts_ms: u64,
        /// 该会话最新已知的 transcript 路径（hook data 携带；无则 None）。
        transcript_path: Option<String>,
    },
    /// 项目列表。
    Projects {
        projects: Vec<ProjectInfo>,
    },
    /// 单个/当前项目（无则 None）。
    Project {
        project: Option<ProjectInfo>,
    },
    /// `BackfillProjectTranscripts` 应答:这次实际导入(全新摄取)的文件数。
    /// 0 表示这个项目的三家 agent 目录里没有还没摄取过的文件——不代表
    /// 出错。
    BackfillDone {
        imported_files: u32,
    },
    /// `DeleteProjectTranscripts` 应答:这次实际删掉的 conversations 行数
    /// (三家 agent 加总)。0 不代表出错——可能这个项目本来就没被摄取过。
    DeletedTranscripts {
        conversations: u32,
    },
    /// 验收次数。
    AcceptanceCount {
        count: u64,
    },
    /// 收藏夹列表。
    Bookmarks {
        bookmarks: Vec<BookmarkInfo>,
    },
    /// `ListConversations` 应答。
    Conversations {
        conversations: Vec<ConversationSummary>,
    },
    /// `GetConversationTurns` 应答。
    ConversationTurns {
        conversation_id: String,
        turns: Vec<TurnRecord>,
    },
    /// `ListConversationsWithSummaries` 应答；`rows` 已按 `last_ts` 倒序排好。
    ConversationsWithSummaries {
        rows: Vec<(ConversationSummary, Option<SessionSummaryPayload>)>,
    },
    /// `GetUsageSummary` 应答。
    UsageSummary {
        rows: Vec<(ConversationSummary, UsagePayload)>,
    },
    /// `GetPreviewContext` 的应答；`context: None` 表示当前无活动文本预览
    /// 或该 `project_id` 从未收到过推送。
    PreviewContext {
        context: Option<PreviewContext>,
    },
    /// `GetSessionSummary` 应答。
    SessionSummary {
        summary: Option<SessionSummaryPayload>,
    },
    /// `GetSessionSummaryBackfillStatus` 应答。`completed >= total` 表示
    /// 已处理完(`total == 0` 表示这个项目本来就没有缺总结的会话)。
    BackfillStatus {
        total: u32,
        completed: u32,
    },
    Todos {
        todos: Vec<TodoInfo>,
    },
    Todo {
        todo: TodoInfo,
    },
    Categories {
        categories: Vec<CategoryInfo>,
    },
    Category {
        category: CategoryInfo,
    },
}

pub fn encode_line<T: Serialize>(value: &T) -> String {
    let mut s = serde_json::to_string(value).expect("protocol types always serialize");
    s.push('\n');
    s
}

pub fn decode_line<T: DeserializeOwned>(line: &str) -> anyhow::Result<T> {
    Ok(serde_json::from_str(line)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversation_protocol_types_roundtrip() {
        let req = Request::ListConversations {
            cwd: "/proj".into(),
            agent: Some(AgentKind::Claude),
            limit: 50,
            offset: 0,
        };
        let line = encode_line(&req);
        let back: Request = decode_line(&line).unwrap();
        assert_eq!(req, back);

        let turns_req = Request::GetConversationTurns {
            conversation_id: "abc".into(),
            after_turn_index: -1,
            limit: 100,
        };
        let line = encode_line(&turns_req);
        let back: Request = decode_line(&line).unwrap();
        assert_eq!(turns_req, back);

        let usage_req = Request::GetUsageSummary {
            cwd: "/proj".into(),
            since_ts: None,
        };
        let line = encode_line(&usage_req);
        let back: Request = decode_line(&line).unwrap();
        assert_eq!(usage_req, back);

        let summary = ConversationSummary {
            conversation_id: "abc".into(),
            agent: AgentKind::Claude,
            file_path: "/h/.claude/projects/x/abc.jsonl".into(),
            title: "标题".into(),
            first_ts: 1,
            last_ts: 2,
            turn_count: 3,
        };
        let turn = TurnRecord {
            turn_index: 0,
            role: "human".into(),
            content: "你好".into(),
            tool_calls: vec![ToolCallInfo {
                summary: "Edit README.md".into(),
                input_json: Some("{\"file_path\":\"README.md\"}".into()),
            }],
            thinking: true,
            thinking_text: Some("先看看现有实现".into()),
            ts: Some(42),
            is_error: false,
            ..Default::default()
        };
        let usage = UsagePayload {
            turns: 2,
            tool_calls: 1,
            mutating_tool_calls: 1,
            files_touched: std::collections::BTreeSet::from(["README.md".to_string()]),
            tokens_in: 10,
            tokens_out: 20,
            tokens_cache_read: 0,
            tokens_cache_write: 0,
        };
        let reply = Reply::UsageSummary {
            rows: vec![(summary.clone(), usage)],
        };
        let line = encode_line(&reply);
        let back: Reply = decode_line(&line).unwrap();
        assert_eq!(reply, back);

        let reply2 = Reply::ConversationTurns {
            conversation_id: "abc".into(),
            turns: vec![turn],
        };
        let line = encode_line(&reply2);
        let back2: Reply = decode_line(&line).unwrap();
        assert_eq!(reply2, back2);

        let reply3 = Reply::Conversations {
            conversations: vec![summary],
        };
        let line = encode_line(&reply3);
        let back3: Reply = decode_line(&line).unwrap();
        assert_eq!(reply3, back3);
    }

    #[test]
    fn turn_record_is_error_field_roundtrips() {
        let turn = TurnRecord {
            turn_index: 0,
            role: "tool_result".into(),
            content: "boom".into(),
            tool_calls: vec![],
            thinking: false,
            thinking_text: None,
            ts: Some(1),
            is_error: true,
            ..Default::default()
        };
        let json = serde_json::to_string(&turn).unwrap();
        let back: TurnRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(back, turn);
        assert!(json.contains("\"is_error\":true"));
    }

    #[test]
    fn list_conversations_with_summaries_protocol_types_roundtrip() {
        let req = Request::ListConversationsWithSummaries {
            cwd: "/home/x/proj".into(),
            agent: Some(AgentKind::Claude),
            limit: 500,
            offset: 0,
        };
        let line = encode_line(&req);
        let back: Request = decode_line(&line).unwrap();
        assert_eq!(req, back);

        let summary = ConversationSummary {
            conversation_id: "abc".into(),
            agent: AgentKind::Claude,
            file_path: "/home/x/proj/.dozer/transcripts/abc.md".into(),
            title: "标题".into(),
            first_ts: 1,
            last_ts: 100,
            turn_count: 3,
        };
        let payload = SessionSummaryPayload {
            session_id: "s1".into(),
            agent_kind: AgentKind::Claude,
            conversation_id: Some("abc".into()),
            title: "总结标题".into(),
            summary: "总结全文".into(),
            status: SummaryStatus::AiGenerated,
            created_ts_ms: 200,
        };
        let reply = Reply::ConversationsWithSummaries {
            rows: vec![(summary.clone(), Some(payload.clone())), (summary, None)],
        };
        let line = encode_line(&reply);
        let back: Reply = decode_line(&line).unwrap();
        assert_eq!(reply, back);
    }

    #[test]
    fn backfill_project_transcripts_protocol_types_roundtrip() {
        let req = Request::BackfillProjectTranscripts {
            cwd: "/home/x/proj".into(),
        };
        let line = encode_line(&req);
        let back: Request = decode_line(&line).unwrap();
        assert_eq!(req, back);

        let reply = Reply::BackfillDone { imported_files: 3 };
        let line = encode_line(&reply);
        let back: Reply = decode_line(&line).unwrap();
        assert_eq!(reply, back);
    }

    #[test]
    fn remove_project_protocol_types_roundtrip() {
        let req = Request::RemoveProject { id: 7 };
        let line = encode_line(&req);
        let back: Request = decode_line(&line).unwrap();
        assert_eq!(req, back);
    }

    #[test]
    fn delete_project_transcripts_protocol_types_roundtrip() {
        let req = Request::DeleteProjectTranscripts {
            cwd: "/home/x/proj".into(),
        };
        let line = encode_line(&req);
        let back: Request = decode_line(&line).unwrap();
        assert_eq!(req, back);

        let reply = Reply::DeletedTranscripts { conversations: 5 };
        let line = encode_line(&reply);
        let back: Reply = decode_line(&line).unwrap();
        assert_eq!(reply, back);
    }

    #[test]
    fn request_roundtrips_as_single_json_line() {
        let req = Request::CreateSession {
            name: "主线".into(),
            command: "/bin/sh".into(),
            args: vec!["-c".into(), "cat".into()],
            cwd: "/tmp".into(),
            cols: 80,
            rows: 24,
            project_id: 1,
        };
        let line = encode_line(&req);
        assert!(line.ends_with('\n'));
        assert_eq!(line.matches('\n').count(), 1);
        let back: Request = decode_line(line.trim()).unwrap();
        assert_eq!(back, req);
    }

    #[test]
    fn reply_tag_is_snake_case() {
        let line = encode_line(&Reply::Attached {
            session_id: "s1".into(),
            snapshot_b64: "aGk=".into(),
            next_offset: 2,
        });
        assert!(line.contains(r#""type":"attached""#));
    }

    #[test]
    fn decode_rejects_garbage() {
        assert!(decode_line::<Request>("not json").is_err());
    }

    #[test]
    fn turn_record_missing_tool_calls_decodes_empty_vec() {
        // 2026-08-22 起 `TurnRecord` 新增了 `tool_calls`,但老 daemon 进程
        // 仍可能发来不带该字段的协议帧(它按旧代码序列化)。`tool_calls` 有
        // `#[serde(default)]`,缺字段时应当解成空 vec,而不是报
        // "missing field `tool_calls`" 让 session 明细整个打不开。
        let old_frame = TurnRecord {
            turn_index: 0,
            role: "human".into(),
            content: "你好".into(),
            tool_calls: vec![],
            thinking: true,
            thinking_text: Some("先想想".into()),
            ts: Some(42),
            is_error: false,
            ..Default::default()
        };
        let line = encode_line(&old_frame);
        // 去掉 `tool_calls` 字段,模拟老 daemon 发来的帧。
        let mut v: serde_json::Value = serde_json::from_str(&line).unwrap();
        v.as_object_mut().unwrap().remove("tool_calls");
        let stripped = serde_json::to_string(&v).unwrap();

        let back: TurnRecord = serde_json::from_str(&stripped).unwrap();
        assert_eq!(back.tool_calls, vec![]);
    }

    #[test]
    fn hook_event_roundtrips() {
        let req = Request::HookEvent {
            session_id: "s1".into(),
            agent: AgentKind::Claude,
            event: "Stop".into(),
            ts_ms: 123,
            data: serde_json::json!({"transcript_path": "/tmp/t.jsonl"}),
        };
        let line = encode_line(&req);
        let back: Request = decode_line(line.trim()).unwrap();
        assert_eq!(back, req);
    }

    #[test]
    fn old_hook_event_without_agent_decodes_unknown() {
        let old = r#"{"type":"hook_event","session_id":"s1","event":"Stop","ts_ms":1,"data":null}"#;
        match decode_line::<Request>(old).unwrap() {
            Request::HookEvent { agent, .. } => assert_eq!(agent, AgentKind::Unknown),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn agent_event_reply_tags_snake_case() {
        let line = encode_line(&Reply::AgentEvent {
            session_id: "s1".into(),
            agent: AgentKind::Claude,
            state: AgentState::AwaitingInput,
            event: "Notification".into(),
            ts_ms: 5,
            transcript_path: None,
        });
        assert!(line.contains(r#""type":"agent_event""#));
        assert!(line.contains(r#""state":"awaiting_input""#));
        assert!(line.contains(r#""agent":"claude""#));
    }

    #[test]
    fn project_messages_roundtrip() {
        let req = Request::OpenProject {
            path: "/repo/x".into(),
        };
        assert_eq!(
            decode_line::<Request>(encode_line(&req).trim()).unwrap(),
            req
        );

        let reply = Reply::Projects {
            projects: vec![ProjectInfo {
                id: 1,
                path: "/repo/x".into(),
                name: "x".into(),
                last_active_ms: 5,
                created_ms: 0,
                updated_ms: 5,
            }],
        };
        let line = encode_line(&reply);
        assert!(line.contains(r#""type":"projects""#));
        assert_eq!(decode_line::<Reply>(line.trim()).unwrap(), reply);

        let reply = Reply::Project { project: None };
        assert_eq!(
            decode_line::<Reply>(encode_line(&reply).trim()).unwrap(),
            reply
        );
    }

    #[test]
    fn rename_project_request_roundtrips() {
        let req = Request::RenameProject {
            id: 1,
            name: "新名字".into(),
        };
        assert_eq!(
            decode_line::<Request>(encode_line(&req).trim()).unwrap(),
            req
        );
    }

    #[test]
    fn record_acceptance_roundtrips() {
        let req = Request::RecordAcceptance {
            repo: "/r".into(),
            goal: "目标".into(),
            criteria_checked: vec!["测试全绿".into()],
            verdict: "accepted".into(),
            comment: "".into(),
            ref_name: "refs/dozer/accepted/1".into(),
            ts_ms: 9,
        };
        let back: Request = decode_line(encode_line(&req).trim()).unwrap();
        assert_eq!(back, req);
    }

    #[test]
    fn agent_event_carries_transcript_path() {
        let line = encode_line(&Reply::AgentEvent {
            session_id: "s".into(),
            agent: AgentKind::Codebuddy,
            state: AgentState::Running,
            event: "UserPromptSubmit".into(),
            ts_ms: 1,
            transcript_path: Some("/t/x.jsonl".into()),
        });
        assert!(line.contains("/t/x.jsonl"));
        match decode_line::<Reply>(line.trim()).unwrap() {
            Reply::AgentEvent {
                agent,
                transcript_path,
                ..
            } => {
                assert_eq!(agent, AgentKind::Codebuddy);
                assert_eq!(transcript_path.as_deref(), Some("/t/x.jsonl"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn old_session_info_without_transcript_path_decodes_none() {
        let old =
            r#"{"id":"a","name":"n","command":"/bin/sh","cwd":"/tmp","alive":true,"created_ms":1}"#;
        let info: SessionInfo = decode_line(old).unwrap();
        assert_eq!(info.transcript_path, None);
    }

    #[test]
    fn acceptance_count_request_roundtrips() {
        let req = Request::GetAcceptanceCount { repo: "/r".into() };
        let back: Request = decode_line(&encode_line(&req)).unwrap();
        assert_eq!(back, req);
        let rep = Reply::AcceptanceCount { count: 12 };
        let back: Reply = decode_line(&encode_line(&rep)).unwrap();
        assert_eq!(back, rep);
    }

    #[test]
    fn old_session_info_without_agent_state_decodes_as_idle() {
        // P1b-d 时代的 SessionInfo JSON（无 agent_state 字段）必须可解
        let old =
            r#"{"id":"a","name":"n","command":"/bin/sh","cwd":"/tmp","alive":true,"created_ms":1}"#;
        let info: SessionInfo = decode_line(old).unwrap();
        assert_eq!(info.agent_state, AgentState::Idle);
    }

    #[test]
    fn session_info_carries_project_id() {
        let info = SessionInfo {
            id: "a".into(),
            name: "n".into(),
            command: "/bin/sh".into(),
            cwd: "/tmp".into(),
            alive: true,
            created_ms: 1,
            agent_state: AgentState::Idle,
            transcript_path: None,
            project_id: Some(7),
            agent: AgentKind::Unknown,
        };
        let line = encode_line(&info);
        let back: SessionInfo = decode_line(line.trim()).unwrap();
        assert_eq!(back.project_id, Some(7));
    }

    #[test]
    fn old_session_info_without_project_id_decodes_none() {
        // 迁移期：daemon 重启前已存活的会话首次读出时没有 project_id 字段。
        let old =
            r#"{"id":"a","name":"n","command":"/bin/sh","cwd":"/tmp","alive":true,"created_ms":1}"#;
        let info: SessionInfo = decode_line(old).unwrap();
        assert_eq!(info.project_id, None);
    }

    #[test]
    fn create_session_request_carries_project_id() {
        let req = Request::CreateSession {
            name: "主线".into(),
            command: "/bin/sh".into(),
            args: vec!["-c".into(), "cat".into()],
            cwd: "/tmp".into(),
            cols: 80,
            rows: 24,
            project_id: 3,
        };
        let line = encode_line(&req);
        let back: Request = decode_line(line.trim()).unwrap();
        assert_eq!(back, req);
    }

    #[test]
    fn agent_kind_defaults_to_unknown() {
        assert_eq!(AgentKind::default(), AgentKind::Unknown);
    }

    #[test]
    fn agent_kind_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&AgentKind::Codebuddy).unwrap(),
            "\"codebuddy\""
        );
        assert_eq!(
            serde_json::to_string(&AgentKind::Opencode).unwrap(),
            "\"opencode\""
        );
        assert_eq!(
            serde_json::to_string(&AgentKind::Codex).unwrap(),
            "\"codex\""
        );
        assert_eq!(serde_json::to_string(&AgentKind::Kilo).unwrap(), "\"kilo\"");
        assert_eq!(
            serde_json::from_str::<AgentKind>("\"claude\"").unwrap(),
            AgentKind::Claude
        );
    }

    #[test]
    fn summary_status_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&SummaryStatus::AiGenerated).unwrap(),
            "\"ai_generated\""
        );
        assert_eq!(
            serde_json::to_string(&SummaryStatus::HeuristicFallback).unwrap(),
            "\"heuristic_fallback\""
        );
    }

    #[test]
    fn record_session_summary_request_roundtrips() {
        let req = Request::RecordSessionSummary {
            session_id: "s1".into(),
            title: "改了个函数".into(),
            summary: "用户让改 README,agent 改完了".into(),
        };
        let line = encode_line(&req);
        let back: Request = decode_line(&line).unwrap();
        assert_eq!(req, back);
    }

    #[test]
    fn close_with_summary_request_roundtrips() {
        let req = Request::CloseWithSummary {
            session_id: "s1".into(),
        };
        let line = encode_line(&req);
        assert_eq!(decode_line::<Request>(&line).unwrap(), req);
    }

    #[test]
    fn backfill_session_summaries_request_roundtrips() {
        let req = Request::BackfillSessionSummaries {
            cwd: "/repo/x".into(),
        };
        let line = encode_line(&req);
        assert_eq!(decode_line::<Request>(&line).unwrap(), req);
    }

    #[test]
    fn get_session_summary_backfill_status_roundtrips() {
        let req = Request::GetSessionSummaryBackfillStatus {
            cwd: "/repo/x".into(),
        };
        let line = encode_line(&req);
        assert_eq!(decode_line::<Request>(&line).unwrap(), req);

        let reply = Reply::BackfillStatus {
            total: 5,
            completed: 2,
        };
        let line = encode_line(&reply);
        assert_eq!(decode_line::<Reply>(&line).unwrap(), reply);
    }

    #[test]
    fn get_session_summary_reply_roundtrips_with_payload() {
        let reply = Reply::SessionSummary {
            summary: Some(SessionSummaryPayload {
                session_id: "s1".into(),
                agent_kind: AgentKind::Claude,
                conversation_id: Some("c1".into()),
                title: "t".into(),
                summary: "s".into(),
                status: SummaryStatus::AiGenerated,
                created_ts_ms: 42,
            }),
        };
        let line = encode_line(&reply);
        let back: Reply = decode_line(&line).unwrap();
        assert_eq!(reply, back);
    }

    #[test]
    fn get_session_summary_reply_roundtrips_with_no_conversation_id() {
        let reply = Reply::SessionSummary {
            summary: Some(SessionSummaryPayload {
                session_id: "s1".into(),
                agent_kind: AgentKind::Claude,
                conversation_id: None,
                title: "t".into(),
                summary: "s".into(),
                status: SummaryStatus::AiGenerated,
                created_ts_ms: 42,
            }),
        };
        let line = encode_line(&reply);
        let back: Reply = decode_line(&line).unwrap();
        assert_eq!(reply, back);
    }

    #[test]
    fn get_session_summary_reply_roundtrips_with_none() {
        let reply = Reply::SessionSummary { summary: None };
        let line = encode_line(&reply);
        let back: Reply = decode_line(&line).unwrap();
        assert_eq!(reply, back);
    }

    #[test]
    fn agent_kind_label_matches_variant() {
        assert_eq!(AgentKind::Unknown.label(), "shell");
        assert_eq!(AgentKind::Claude.label(), "claude");
        assert_eq!(AgentKind::Codebuddy.label(), "codebuddy");
        assert_eq!(AgentKind::Opencode.label(), "opencode");
        assert_eq!(AgentKind::Codex.label(), "codex");
        assert_eq!(AgentKind::Kilo.label(), "kilo");
    }

    #[test]
    fn session_info_carries_agent() {
        let info = SessionInfo {
            id: "a".into(),
            name: "n".into(),
            command: "/bin/sh".into(),
            cwd: "/tmp".into(),
            alive: true,
            created_ms: 1,
            agent_state: AgentState::Idle,
            transcript_path: None,
            project_id: Some(7),
            agent: AgentKind::Claude,
        };
        let line = encode_line(&info);
        let back: SessionInfo = decode_line(line.trim()).unwrap();
        assert_eq!(back.agent, AgentKind::Claude);
    }

    #[test]
    fn old_session_info_without_agent_decodes_unknown() {
        let old =
            r#"{"id":"a","name":"n","command":"/bin/sh","cwd":"/tmp","alive":true,"created_ms":1}"#;
        let info: SessionInfo = decode_line(old).unwrap();
        assert_eq!(info.agent, AgentKind::Unknown);
    }

    #[test]
    fn bookmark_scope_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&BookmarkScope::Global).unwrap(),
            "\"global\""
        );
        assert_eq!(
            serde_json::to_string(&BookmarkScope::Project).unwrap(),
            "\"project\""
        );
    }

    #[test]
    fn bookmark_messages_roundtrip() {
        let req = Request::AddBookmark {
            scope: BookmarkScope::Project,
            project_id: Some(7),
            url: "https://example.com".into(),
            title: "example".into(),
        };
        assert_eq!(decode_line::<Request>(&encode_line(&req)).unwrap(), req);

        let req = Request::RemoveBookmark { id: 3 };
        assert_eq!(decode_line::<Request>(&encode_line(&req)).unwrap(), req);

        let req = Request::ListBookmarks {
            project_id: Some(7),
        };
        assert_eq!(decode_line::<Request>(&encode_line(&req)).unwrap(), req);

        let req = Request::ListBookmarks { project_id: None };
        assert_eq!(decode_line::<Request>(&encode_line(&req)).unwrap(), req);

        let reply = Reply::Bookmarks {
            bookmarks: vec![BookmarkInfo {
                id: 1,
                scope: BookmarkScope::Global,
                project_id: None,
                url: "https://example.com".into(),
                title: "example".into(),
                created_ms: 5,
            }],
        };
        let line = encode_line(&reply);
        assert!(line.contains(r#""type":"bookmarks""#));
        assert_eq!(decode_line::<Reply>(line.trim()).unwrap(), reply);
    }

    #[test]
    fn old_agent_event_without_agent_decodes_unknown() {
        let old =
            r#"{"type":"agent_event","session_id":"s","state":"running","event":"test","ts_ms":1}"#;
        match decode_line::<Reply>(old).unwrap() {
            Reply::AgentEvent { agent, .. } => assert_eq!(agent, AgentKind::Unknown),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn agent_kind_recognizes_v8agent() {
        assert_eq!(
            serde_json::to_string(&AgentKind::V8agent).unwrap(),
            "\"v8agent\""
        );
        assert_eq!(
            serde_json::from_str::<AgentKind>("\"v8agent\"").unwrap(),
            AgentKind::V8agent
        );
        assert_eq!(AgentKind::V8agent.label(), "v8agent");
    }

    #[test]
    fn preview_context_round_trips() {
        let ctx = PreviewContext {
            path: "/repo/src/main.rs".into(),
            start_line: 12,
            start_col: 3,
            end_line: 14,
            end_col: 1,
            has_selection: true,
            updated_at_ms: 1_700_000_000_000,
        };
        let req = Request::UpdatePreviewContext {
            project_id: 7,
            context: Some(ctx.clone()),
        };
        let line = encode_line(&req);
        assert_eq!(decode_line::<Request>(&line).unwrap(), req);

        let req_none = Request::UpdatePreviewContext {
            project_id: 7,
            context: None,
        };
        let line = encode_line(&req_none);
        assert_eq!(decode_line::<Request>(&line).unwrap(), req_none);

        let get = Request::GetPreviewContext { project_id: 7 };
        let line = encode_line(&get);
        assert_eq!(decode_line::<Request>(&line).unwrap(), get);

        let reply = Reply::PreviewContext { context: Some(ctx) };
        let line = encode_line(&reply);
        assert_eq!(decode_line::<Reply>(&line).unwrap(), reply);
    }

    #[test]
    fn todo_protocol_types_roundtrip() {
        let add_req = Request::AddTodo {
            project_id: 1,
            text: "写完 spec".into(),
        };
        let line = encode_line(&add_req);
        let back: Request = decode_line(&line).unwrap();
        assert_eq!(add_req, back);

        let reorder_req = Request::ReorderTodo {
            id: 5,
            after_id: Some(3),
        };
        let line = encode_line(&reorder_req);
        let back: Request = decode_line(&line).unwrap();
        assert_eq!(reorder_req, back);

        let todo = TodoInfo {
            id: 1,
            project_id: 1,
            text: "写完 spec".into(),
            done: false,
            rank: 0,
            created_ms: 1_700_000_000_000,
            completed_at_ms: None,
            plan_date: Some("08-10".into()),
            dispatch_session_id: None,
            dispatch_at_ms: None,
            category_id: None,
        };
        let reply = Reply::Todo { todo: todo.clone() };
        let line = encode_line(&reply);
        let back: Reply = decode_line(&line).unwrap();
        assert_eq!(reply, back);

        let list_reply = Reply::Todos { todos: vec![todo] };
        let line = encode_line(&list_reply);
        let back: Reply = decode_line(&line).unwrap();
        assert_eq!(list_reply, back);
    }

    #[test]
    fn category_protocol_types_roundtrip() {
        let req = Request::AddCategory {
            project_id: 1,
            parent_id: Some(2),
            name: "前端".into(),
        };
        let line = encode_line(&req);
        let decoded: Request = decode_line(&line).unwrap();
        assert_eq!(req, decoded);

        let category = CategoryInfo {
            id: 10,
            project_id: 1,
            parent_id: Some(2),
            name: "前端".into(),
            rank: 0,
            created_ms: 1_700_000_000_000,
        };
        let reply = Reply::Category {
            category: category.clone(),
        };
        let line = encode_line(&reply);
        let decoded: Reply = decode_line(&line).unwrap();
        assert_eq!(reply, decoded);

        let list_reply = Reply::Categories {
            categories: vec![category],
        };
        let line = encode_line(&list_reply);
        let decoded: Reply = decode_line(&line).unwrap();
        assert_eq!(list_reply, decoded);
    }

    #[test]
    fn todo_info_category_id_defaults_to_none_when_absent_from_json() {
        // 老协议帧没有 category_id 字段,新增字段要能优雅缺省,不报错。
        let json = r#"{"id":1,"project_id":1,"text":"任务","done":false,"rank":0,
            "created_ms":0,"completed_at_ms":null,"plan_date":null,
            "dispatch_session_id":null,"dispatch_at_ms":null}"#;
        let todo: TodoInfo = serde_json::from_str(json).unwrap();
        assert_eq!(todo.category_id, None);
    }
}
