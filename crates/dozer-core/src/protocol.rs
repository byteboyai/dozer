use serde::{Deserialize, Serialize, de::DeserializeOwned};

pub use dozer_model::{
    AgentKind, AgentState, BookmarkInfo, BookmarkScope, CategoryInfo, CategoryMoveDirection,
    CodeHealthReportInfo, ContextItemInfo, ConversationSummary, ConversationSummaryResult,
    FileEditHistoryInfo, GroupAuthor, GroupCancelScope, GroupInfo, GroupMemberInfo,
    GroupMessageInfo, GroupMessageStatus, Locale, LocalePreference, LocaleSnapshot,
    LocaleWriteOutcome, LocateMatch, MemoryDetail, MemoryHistoryEntry, MemoryInfo, MutationOutcome,
    PREVIEW_COMMAND_TIMEOUT_MS, PREVIEW_SELECTED_TEXT_MAX_CHARS, PreviewCommand,
    PreviewCommandAction, PreviewCommandOutcome, PreviewCommandTarget, PreviewContext,
    PreviewTabularContext, ProjectInfo, SessionInfo, SessionSummaryPayload, SummaryBatchInfo,
    SummaryErrorKind, SummaryJobInfo, SummaryJobStatus, SummaryProviderInfo, SummaryStatus,
    SummaryTrigger, TodoInfo, TodoStoredStatus, ToolCallInfo, TurnRecord, UsagePayload,
};

/// C/S 协议版本。客户端与守护进程用它握手:不一致时给出可读错误而不是
/// 莫名其妙的反序列化失败。只在协议帧形状变化(加/改变体、改字段语义)
/// 时递增;纯加字段且带 serde default 的兼容变更不必递增。
pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    /// 协议版本握手:任何业务请求前,客户端先发它确认双方版本一致。
    /// 旧守护进程不认识这个变体时只会回 `Reply::Error`,客户端据此
    /// 报"版本过旧"而不是反序列化炸裂。hook 的单向上报同样先发 Hello,
    /// 版本不符就静默放弃(不影响 agent)。
    Hello {
        protocol: u32,
    },
    /// bytehost 应用宿主的请求(安装/启动/停止/列表……),原样交给 dozerd 里的 `AppService`。
    /// 用结构体变体包一层,避免"内部带标签的枚举套内部带标签的枚举"。
    App {
        request: bytehost_apps::proto::AppRequest,
    },
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
        /// 创建者已知的 agent 身份。旧客户端没有此字段时兼容为
        /// `Unknown`；后续 hook 事件仍可更新它。
        #[serde(default)]
        agent: AgentKind,
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
    /// 请求 daemon 对所有存活会话完成"总结后关闭"收尾,再退出进程自身。
    /// 与 `Kill`(杀单个会话)、`CloseWithSummary`(单会话总结后关闭)不同,
    /// 这是唯一一个"daemon 进程整体退出"的请求。
    Shutdown,
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
    /// 保存一次代码健康度扫描的落盘结果（最新一份 + 历史快照）。
    SaveCodeHealthReport {
        project_id: i64,
        report_json: String,
        total_loc: u64,
        total_functions: u64,
        critical_functions: u64,
        overall_tier: String,
        /// 报告 schema 版本；旧客户端不传时回落 0。
        #[serde(default)]
        schema_version: u32,
        #[serde(default)]
        git_head: Option<String>,
        #[serde(default)]
        git_branch: Option<String>,
        #[serde(default)]
        git_dirty: bool,
    },
    /// 查询某项目上次代码健康度扫描结果；`None` = 还没扫描过。
    GetCodeHealthReport {
        project_id: i64,
    },
    /// 列出某项目最近 N 份报告快照（按时间倒序）。`limit` 上限由服务端钳制。
    ListCodeHealthReports {
        project_id: i64,
        limit: u32,
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
    /// T13:daemon → app 的下行预览命令(导航 / revision 守卫的写入)。dozerd 只
    /// 转发给在线的 app;app 处理后回 `Reply::PreviewCommandResult`。app 不在线
    /// 或超时由 dozerd 判定并回 `PreviewCommandOutcome::Timeout`。
    ///
    /// 权限:与本机其它写工具同信任级(不额外鉴权),但 **replace 必须携带
    /// `expected_revision`**,失配一律拒绝,绝不猜增量。
    RunPreviewCommand {
        command: PreviewCommand,
    },
    /// T13:app 轮询取走某项目待处理的预览命令(dozerd 队列 → app)。
    TakePendingPreviewCommands {
        project_id: i64,
    },
    /// T13:app 把某条命令的终态回报给 dozerd(投递给等待中的
    /// `RunPreviewCommand` 调用方)。
    ReportPreviewCommandOutcome {
        outcome: PreviewCommandOutcome,
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
    /// V2 总结协议:提交单条总结任务(持久队列),返回 `job_id`。`trigger`
    /// 记录来源(关闭/修复/手动/退出)。`provider`/`model` 为 None 时由
    /// daemon 按 summary 配置解析。
    SubmitSummaryJob {
        conversation_id: String,
        #[serde(default)]
        source_session_id: Option<String>,
        trigger: SummaryTrigger,
        #[serde(default)]
        provider: Option<AgentKind>,
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        force: bool,
    },
    /// 提交一批修复任务(按选择策略筛选缺失/legacy/stale/失败项),返回
    /// `batch_id` 与选中数。
    SubmitSummaryBatch {
        cwd: String,
        #[serde(default)]
        provider: Option<AgentKind>,
        #[serde(default)]
        model: Option<String>,
    },
    /// 查询单条任务状态。
    GetSummaryJob {
        job_id: i64,
    },
    /// 查询批次状态。
    GetSummaryBatch {
        batch_id: i64,
    },
    /// 重试一条失败任务(新建 attempt,复用原 conversation/触发来源)。
    RetrySummaryJob {
        job_id: i64,
    },
    /// 取消批次(把批次内未完成的任务置 cancelled)。
    CancelSummaryBatch {
        batch_id: i64,
    },
    /// 查询 summary provider 解析结果(UI 展示"用哪个总结器/来源")。
    GetSummaryProvider {
        #[serde(default)]
        ui_choice: Option<AgentKind>,
        #[serde(default)]
        ui_model: Option<String>,
    },
    /// 查询某 conversation 的规范总结结果(新版结果表)。
    GetSummaryResult {
        conversation_id: String,
    },
    /// 列出某项目全部任务,`ORDER BY done, paused, rank` 排好序返回。
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
    /// 指派任务给某个 agent 种类。纯记录,不触发任何执行、不写 PTY——
    /// dozerd 只更新 `assigned_agent`/`dispatch_at_ms`。
    AssignTodoAgent {
        id: i64,
        agent: AgentKind,
    },
    /// 打开/关闭某分类下任务的自动轮询处理。
    SetCategoryAutoPoll {
        id: i64,
        enabled: bool,
    },
    /// 立即触发一次该任务的 headless 处理,不等轮询。`human_reply` 为
    /// `None` 表示这次触发没有新增人类文本(指派后从未回复过就先点了
    /// "处理")。命中"该任务正在处理中"时应答 `Reply::Error`。
    ProcessTodoNow {
        id: i64,
        human_reply: Option<String>,
    },
    /// 列出某项目全部共享记忆(不含正文,列表用)。按 `updated_ms` 倒序。
    ListMemories {
        project_id: i64,
    },
    /// 按 `(project_id, title)` upsert:标题已存在则更新,否则新建。
    /// `actor` 是 `AgentKind::as_str()` 或人工操作时的字面量 `"user"`,
    /// 由调用方(`dozer-mcp`/`dozer-app`)决定,不在 `dozerd` 里推导。
    WriteMemory {
        project_id: i64,
        title: String,
        kind: String,
        description: String,
        body: String,
        actor: String,
    },
    /// 查一条记忆的完整正文 + 最近历史。`id` 不属于 `project_id` 时视同
    /// 不存在(项目隔离)。
    GetMemory {
        project_id: i64,
        id: i64,
    },
    /// 在项目内某文本文件里搜索 `query`,返回全部匹配的坐标。只读。
    LocateInFile {
        project_id: i64,
        path: String,
        query: String,
    },
    /// 精确替换项目内某文本文件的 `[start_line,start_col]`~`[end_line,end_col]`
    /// 区间(1-based,含端点)。`expected_text` 是调用方认为该区间当前的原样
    /// 内容,用于 Conflict Detection。`actor`/`session_id` 供历史记录署名。
    ApplyPreciseEdit {
        project_id: i64,
        path: String,
        start_line: u32,
        start_col: u32,
        end_line: u32,
        end_col: u32,
        expected_text: String,
        new_text: String,
        summary: String,
        actor: String,
        session_id: String,
    },
    /// 把一个实体加入项目的 Agent 上下文列表。重复添加幂等(返回既有项)。
    /// 路径越出项目根目录时应答 `Reply::Error`。
    AddContextItem {
        project_id: i64,
        entity_kind: String,
        entity_ref: String,
    },
    /// 移除一项;目标不存在也视为成功。
    RemoveContextItem {
        project_id: i64,
        id: i64,
    },
    /// 按添加时间升序列出项目的上下文项。
    ListContextItems {
        project_id: i64,
    },
    /// 按时间倒序列出项目的 agent 修改历史。`path_filter` 命中规则:
    /// `target_path` 等于该值,或位于该目录之下;`None` 不过滤。
    ListFileEditHistory {
        project_id: i64,
        path_filter: Option<String>,
        limit: u32,
    },
    /// 人工删除一条记忆(`dozer-mcp` 不暴露对应工具,只有 `dozer-app` UI
    /// 会发这个请求)。删除前的最后状态会被写进一条 `deleted` 历史。
    DeleteMemory {
        project_id: i64,
        id: i64,
        actor: String,
    },
    /// 详情弹窗打开时一次性拿任务信息 + 关联会话的完整回合列表。
    /// `dispatch_session_id` 为 `None`(从未处理过)时 `turns` 返回空数组。
    GetTodoDetail {
        id: i64,
    },
    /// 把任务设为某个已存储逻辑状态(`TodoStoredStatus`,见其注释里三值各
    /// 自落哪些 `done`/`paused`/`dispatch_*` 组合)。服务端一次 UPDATE 原子
    /// 完成;`id` 不存在 → `Reply::Error`。
    SetTodoStatus {
        id: i64,
        status: TodoStoredStatus,
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
    /// 新建群聊。`topic` 非空。
    CreateGroup {
        project_id: i64,
        topic: String,
    },
    ListGroups {
        project_id: i64,
    },
    /// 删除群(含成员与消息);会先取消该群进行中的发言。
    DeleteGroup {
        group_id: i64,
    },
    AddGroupMember {
        group_id: i64,
        agent: AgentKind,
        handle: String,
        role_prompt: String,
    },
    UpdateGroupMember {
        member_id: i64,
        handle: String,
        role_prompt: String,
    },
    RemoveGroupMember {
        member_id: i64,
    },
    /// human 发一条消息;其中 `@handle` 触发对应成员按首次出现顺序串行发言。
    PostGroupMessage {
        group_id: i64,
        text: String,
    },
    /// 增量取消息:返回 `rev > after_rev` 的消息(含状态变更过的老消息)。
    ListGroupMessages {
        group_id: i64,
        after_rev: i64,
        limit: u32,
    },
    CancelGroup {
        group_id: i64,
        scope: GroupCancelScope,
    },
    /// 重试一条 `Failed`/`Cancelled` 的 agent 消息(只重跑这一位)。
    RetryGroupMessage {
        message_id: i64,
    },
    /// 把一条消息推送到 Todo(新建一条待办)。**不做任何指派**——任务分配
    /// 由 Todo 负责。`text` 是用户在对话框里编辑后的内容。
    PushGroupMessageToTodo {
        message_id: i64,
        text: String,
    },
    /// 读取界面语言设置(spec 2026-10-12-dozer-i18n-design §4.2)。daemon 返回
    /// 持久化的 `LocaleSnapshot`;无文件时返回默认快照(system/en/revision 0)。
    GetLocale,
    /// 写入界面语言设置。`expected_revision` 做 CAS:失配不回 `Error`,而是回
    /// `Reply::LocaleWrite { outcome: Conflict }` 并带上最新快照。`system_candidates`
    /// 仅在 System 偏好下用于解析生效语言,不改变显式偏好。该字段为后期追加,
    /// 旧客户端不传时按空候选处理。
    SetLocale {
        preference: LocalePreference,
        expected_revision: u64,
        #[serde(default)]
        system_candidates: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Reply {
    /// `Request::Hello` 的应答:守护进程的协议版本与产品名。
    Hello {
        protocol: u32,
        product: String,
    },
    /// 应用宿主请求的应答。失败走通用的 `Error`。
    App {
        reply: bytehost_apps::proto::AppReply,
    },
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
    /// 收藏夹列表。
    Bookmarks {
        bookmarks: Vec<BookmarkInfo>,
    },
    /// `GetCodeHealthReport` 应答。`None` = 这个项目还没扫描过。
    CodeHealthReport {
        report: Option<CodeHealthReportInfo>,
    },
    /// `ListCodeHealthReports` 应答（按时间倒序）。
    CodeHealthReports {
        reports: Vec<CodeHealthReportInfo>,
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
    /// T13:`RunPreviewCommand` 的应答。无论成功失败都回一条,携带同一
    /// `request_id`,app 不在线/超时由 daemon 判定后回 `Timeout`。
    PreviewCommandResult {
        outcome: PreviewCommandOutcome,
    },
    /// T13:`TakePendingPreviewCommands` 的应答(可能为空)。
    PendingPreviewCommands {
        commands: Vec<PreviewCommand>,
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
    /// V2:`SubmitSummaryJob` 应答。
    SummaryJobSubmitted {
        job_id: i64,
    },
    /// V2:`SubmitSummaryBatch` 应答。
    SummaryBatchSubmitted {
        batch_id: i64,
        total: u32,
    },
    /// V2:`GetSummaryJob` 应答。`None` 表示 job 不存在。
    SummaryJob {
        job: Option<SummaryJobInfo>,
    },
    /// V2:`GetSummaryBatch` 应答。`None` 表示 batch 不存在。
    SummaryBatch {
        batch: Option<SummaryBatchInfo>,
    },
    /// V2:`GetSummaryProvider` 应答。
    SummaryProvider {
        info: SummaryProviderInfo,
    },
    /// V2:`GetSummaryResult` 应答。`None` 表示无规范结果。
    SummaryResult {
        result: Option<ConversationSummaryResult>,
    },
    Todos {
        todos: Vec<TodoInfo>,
    },
    Todo {
        todo: TodoInfo,
    },
    /// `ListMemories` 应答。
    Memories {
        memories: Vec<MemoryInfo>,
    },
    /// `WriteMemory`/`GetMemory` 应答。
    MemoryDetail {
        detail: MemoryDetail,
    },
    /// `LocateInFile` 应答。
    LocateMatches {
        matches: Vec<LocateMatch>,
    },
    /// `ApplyPreciseEdit` 应答。
    MutationResult {
        outcome: MutationOutcome,
    },
    /// `AddContextItem` 应答。
    ContextItem {
        item: ContextItemInfo,
    },
    /// `ListContextItems` 应答。
    ContextItems {
        items: Vec<ContextItemInfo>,
    },
    /// `ListFileEditHistory` 应答。
    FileEditHistory {
        entries: Vec<FileEditHistoryInfo>,
    },
    Categories {
        categories: Vec<CategoryInfo>,
    },
    Category {
        category: CategoryInfo,
    },
    /// `GetTodoDetail` 应答。
    TodoDetail {
        info: TodoInfo,
        turns: Vec<TurnRecord>,
    },
    /// `CreateGroup`/`ListGroups` 用的单个群(含成员)。
    Group {
        group: GroupInfo,
    },
    Groups {
        groups: Vec<GroupInfo>,
    },
    GroupMember {
        member: GroupMemberInfo,
    },
    /// `PostGroupMessage` 应答:用户这条 human 消息本身。
    GroupMessage {
        message: GroupMessageInfo,
    },
    /// `PostGroupMessage` 应答:human 消息 + 为每个 `@` 成员一次性创建的排队占位 +
    /// 未识别的 handle。
    GroupPosted {
        human: GroupMessageInfo,
        placeholders: Vec<GroupMessageInfo>,
        unknown_handles: Vec<String>,
    },
    /// `ListGroupMessages` 应答。`latest_rev` 是本群当前最大 rev,GUI 下次拿它
    /// 当 `after_rev` 继续轮询。
    GroupMessages {
        messages: Vec<GroupMessageInfo>,
        latest_rev: i64,
    },
    /// `GetLocale` 应答:当前权威快照。
    Locale {
        snapshot: LocaleSnapshot,
    },
    /// `SetLocale` 应答:Applied(已落盘)或 Conflict(带最新快照)。
    LocaleWrite {
        outcome: LocaleWriteOutcome,
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
    fn hello_round_trips_through_serde() {
        let req = Request::Hello {
            protocol: PROTOCOL_VERSION,
        };
        let line = encode_line(&req);
        assert!(line.contains("\"type\":\"hello\""), "{line}");
        assert!(line.contains("\"protocol\":1"), "{line}");
        assert_eq!(decode_line::<Request>(&line).unwrap(), req);

        let reply = Reply::Hello {
            protocol: PROTOCOL_VERSION,
            product: "dozer".into(),
        };
        let line = encode_line(&reply);
        assert!(line.contains("\"type\":\"hello\""), "{line}");
        assert!(line.contains("\"product\":\"dozer\""), "{line}");
        assert_eq!(decode_line::<Reply>(&line).unwrap(), reply);
    }

    /// 新增变体不得改变既有变体的字节编码:每个既有变体各取一个
    /// 样例,与改造前的 JSON 逐字比较(改造前该测试先在旧代码上
    /// 记录期望值)。
    #[test]
    fn existing_variants_keep_their_exact_wire_encoding() {
        // 旧编码样例(2026-10-20 记录):
        let cases: [(&Request, &str); 4] = [
            (&Request::ListSessions, "{\"type\":\"list_sessions\"}\n"),
            (
                &Request::Kill {
                    session_id: "s1".into(),
                },
                "{\"type\":\"kill\",\"session_id\":\"s1\"}\n",
            ),
            (
                &Request::HookEvent {
                    session_id: "s1".into(),
                    agent: AgentKind::Claude,
                    event: "Stop".into(),
                    ts_ms: 7,
                    data: serde_json::Value::Null,
                },
                "{\"type\":\"hook_event\",\"session_id\":\"s1\",\"agent\":\"claude\",\"event\":\"Stop\",\"ts_ms\":7,\"data\":null}\n",
            ),
            (
                &Request::Attach {
                    session_id: "s1".into(),
                    from_offset: 0,
                },
                "{\"type\":\"attach\",\"session_id\":\"s1\",\"from_offset\":0}\n",
            ),
        ];
        for (req, want) in cases {
            assert_eq!(encode_line(req), want);
        }
        assert_eq!(encode_line(&Reply::Ok), "{\"type\":\"ok\"}\n");
        assert_eq!(
            encode_line(&Reply::Error {
                message: "boom".into()
            }),
            "{\"type\":\"error\",\"message\":\"boom\"}\n"
        );
    }

    #[test]
    fn app_host_requests_and_replies_roundtrip_through_the_line_codec() {
        use bytehost_apps::id::AppId;
        use bytehost_apps::plan::{Provenance, TrustLevel};
        use bytehost_apps::proto::{
            AppReply, AppRequest, AppSource, RuntimeAvailability, RuntimeProbe,
        };

        let requests = [
            AppRequest::List,
            AppRequest::ProbeRuntimes,
            AppRequest::Start {
                id: AppId::new("excalidraw").unwrap(),
            },
            AppRequest::Plan {
                source: AppSource::LocalDir {
                    path: "/tmp/x".into(),
                },
                provenance: Provenance::Local,
                trust: TrustLevel::Trusted,
            },
        ];
        for request in requests {
            let req = Request::App { request };
            let line = encode_line(&req);
            assert!(line.contains("\"type\":\"app\""), "{line}");
            assert_eq!(decode_line::<Request>(&line).unwrap(), req);
        }
        let reply = Reply::App {
            reply: AppReply::Runtimes {
                runtimes: vec![RuntimeProbe {
                    runtime: "docker".into(),
                    availability: RuntimeAvailability::NotInstalled,
                    managed: vec!["22.11.0".into()],
                    installable: true,
                    job: None,
                }],
            },
        };
        assert_eq!(decode_line::<Reply>(&encode_line(&reply)).unwrap(), reply);
        // 未知的 op 在协议层就被拒绝
        assert!(
            decode_line::<Request>("{\"type\":\"app\",\"request\":{\"op\":\"format_disk\"}}")
                .is_err()
        );
    }

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
                id: None,
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
            task_id: None,
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
            agent: AgentKind::Unknown,
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
    fn locale_request_and_replies_roundtrip() {
        let get = Request::GetLocale;
        assert_eq!(
            decode_line::<Request>(encode_line(&get).trim()).unwrap(),
            get
        );

        let set = Request::SetLocale {
            preference: LocalePreference::Explicit {
                locale: Locale(Locale::ZH_CN.into()),
            },
            expected_revision: 3,
            system_candidates: vec!["zh-HK".into(), "en-US".into()],
        };
        assert_eq!(
            decode_line::<Request>(encode_line(&set).trim()).unwrap(),
            set
        );

        let snap = LocaleSnapshot {
            schema_version: 1,
            preference: LocalePreference::System,
            active_locale: Locale(Locale::EN.into()),
            revision: 4,
        };
        let reply = Reply::Locale {
            snapshot: snap.clone(),
        };
        assert_eq!(
            decode_line::<Reply>(encode_line(&reply).trim()).unwrap(),
            reply
        );

        let write = Reply::LocaleWrite {
            outcome: LocaleWriteOutcome::Conflict { snapshot: snap },
        };
        assert_eq!(
            decode_line::<Reply>(encode_line(&write).trim()).unwrap(),
            write
        );
    }

    #[test]
    fn set_locale_without_system_candidates_decodes_empty() {
        // `system_candidates` 为后期追加字段;老客户端不发时按空候选处理,
        // 而不是报 "missing field"。
        let old = r#"{"type":"set_locale","preference":{"kind":"system"},"expected_revision":0}"#;
        match decode_line::<Request>(old).unwrap() {
            Request::SetLocale {
                preference,
                expected_revision,
                system_candidates,
            } => {
                assert_eq!(preference, LocalePreference::System);
                assert_eq!(expected_revision, 0);
                assert!(system_candidates.is_empty());
            }
            other => panic!("unexpected: {other:?}"),
        }
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
            agent: AgentKind::Codex,
        };
        let line = encode_line(&req);
        let back: Request = decode_line(line.trim()).unwrap();
        assert_eq!(back, req);
    }

    #[test]
    fn old_create_session_request_without_agent_defaults_unknown() {
        let old = r#"{"type":"create_session","name":"n","command":"/bin/sh","args":[],"cwd":"/tmp","cols":80,"rows":24,"project_id":3}"#;
        let req: Request = decode_line(old).unwrap();
        assert!(matches!(
            req,
            Request::CreateSession {
                agent: AgentKind::Unknown,
                ..
            }
        ));
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
        assert_eq!(
            serde_json::to_string(&AgentKind::Goose).unwrap(),
            "\"goose\""
        );
        assert_eq!(
            serde_json::to_string(&AgentKind::Aider).unwrap(),
            "\"aider\""
        );
        assert_eq!(
            serde_json::from_str::<AgentKind>("\"claude\"").unwrap(),
            AgentKind::Claude
        );
        assert_eq!(
            serde_json::from_str::<AgentKind>("\"goose\"").unwrap(),
            AgentKind::Goose
        );
        assert_eq!(
            serde_json::from_str::<AgentKind>("\"aider\"").unwrap(),
            AgentKind::Aider
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
    fn v2_submit_summary_job_roundtrips_with_defaults() {
        let req = Request::SubmitSummaryJob {
            conversation_id: "c1".into(),
            source_session_id: None,
            trigger: SummaryTrigger::Manual,
            provider: Some(AgentKind::Codex),
            model: Some("gpt-5".into()),
            force: false,
        };
        let line = encode_line(&req);
        let back: Request = decode_line(&line).unwrap();
        assert_eq!(req, back);

        // 省略可选字段(旧客户端/老 JSON)应回落到默认值,不报错。
        let minimal =
            r#"{"type":"submit_summary_job","conversation_id":"c1","trigger":"backfill"}"#;
        let decoded: Request = serde_json::from_str(minimal).unwrap();
        match decoded {
            Request::SubmitSummaryJob {
                provider, force, ..
            } => {
                assert_eq!(provider, None);
                assert!(!force);
            }
            other => panic!("应解码为 SubmitSummaryJob,得到 {other:?}"),
        }
    }

    #[test]
    fn summary_trigger_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&SummaryTrigger::NaturalExit).unwrap(),
            "\"natural_exit\""
        );
        assert_eq!(
            serde_json::to_string(&SummaryTrigger::Backfill).unwrap(),
            "\"backfill\""
        );
    }

    #[test]
    fn summary_job_status_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&SummaryJobStatus::Queued).unwrap(),
            "\"queued\""
        );
        assert_eq!(
            serde_json::to_string(&SummaryJobStatus::Succeeded).unwrap(),
            "\"succeeded\""
        );
    }

    #[test]
    fn summary_provider_info_roundtrips() {
        let info = SummaryProviderInfo {
            provider: Some(AgentKind::Codex),
            model: Some("gpt-5".into()),
            source: Some("summary".into()),
            required: false,
        };
        let line = serde_json::to_string(&info).unwrap();
        let back: SummaryProviderInfo = serde_json::from_str(&line).unwrap();
        assert_eq!(info, back);
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
    fn shutdown_request_roundtrips() {
        let req = Request::Shutdown;
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
                task_id: None,
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
                task_id: None,
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
        assert_eq!(AgentKind::Goose.label(), "goose");
        assert_eq!(AgentKind::Goose.display_label(), "Goose");
        assert_eq!(AgentKind::Aider.label(), "aider");
        assert_eq!(AgentKind::Aider.display_label(), "Aider");
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
    fn save_code_health_report_serializes_with_type_tag() {
        let req = Request::SaveCodeHealthReport {
            project_id: 1,
            report_json: "{}".into(),
            total_loc: 100,
            total_functions: 10,
            critical_functions: 1,
            overall_tier: "watch".into(),
            schema_version: 2,
            git_head: Some("abc123".into()),
            git_branch: Some("main".into()),
            git_dirty: false,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"type\":\"save_code_health_report\""));
    }

    #[test]
    fn old_save_code_health_report_without_git_metadata_decodes_defaults() {
        // 旧客户端不传 schema_version/git_* 字段，必须能解出默认值。
        let old = r#"{"type":"save_code_health_report","project_id":1,"report_json":"{}","total_loc":1,"total_functions":2,"critical_functions":0,"overall_tier":"watch"}"#;
        let req: Request = decode_line(old).unwrap();
        match req {
            Request::SaveCodeHealthReport {
                schema_version,
                git_head,
                git_branch,
                git_dirty,
                ..
            } => {
                assert_eq!(schema_version, 0);
                assert_eq!(git_head, None);
                assert_eq!(git_branch, None);
                assert!(!git_dirty);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn get_code_health_report_round_trips() {
        let req = Request::GetCodeHealthReport { project_id: 7 };
        let json = serde_json::to_string(&req).unwrap();
        let back: Request = serde_json::from_str(&json).unwrap();
        assert_eq!(req, back);
    }

    #[test]
    fn list_code_health_reports_round_trips() {
        let req = Request::ListCodeHealthReports {
            project_id: 7,
            limit: 2,
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: Request = serde_json::from_str(&json).unwrap();
        assert_eq!(req, back);

        let reply = Reply::CodeHealthReports { reports: vec![] };
        let json = serde_json::to_string(&reply).unwrap();
        assert!(json.contains("\"type\":\"code_health_reports\""));
    }

    #[test]
    fn code_health_report_reply_round_trips_with_none() {
        let reply = Reply::CodeHealthReport { report: None };
        let json = serde_json::to_string(&reply).unwrap();
        let back: Reply = serde_json::from_str(&json).unwrap();
        assert_eq!(reply, back);
    }

    #[test]
    fn code_health_report_info_round_trips() {
        let info = CodeHealthReportInfo {
            total_loc: 100,
            total_functions: 10,
            critical_functions: 1,
            overall_tier: "critical".into(),
            report_json: "{\"functions\":[]}".into(),
            scanned_at_ms: 1_700_000_000_000,
            schema_version: 2,
            git_head: Some("abc".into()),
            git_branch: Some("main".into()),
            git_dirty: true,
        };
        let json = serde_json::to_string(&info).unwrap();
        let back: CodeHealthReportInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(info, back);
    }

    #[test]
    fn old_code_health_report_info_without_new_fields_decodes_defaults() {
        // 旧 daemon 发来的 CodeHealthReportInfo 无 schema_version/git 字段。
        let old = r#"{"total_loc":1,"total_functions":2,"critical_functions":0,"overall_tier":"watch","report_json":"{}","scanned_at_ms":1}"#;
        let info: CodeHealthReportInfo = serde_json::from_str(old).unwrap();
        assert_eq!(info.schema_version, 0);
        assert_eq!(info.git_head, None);
        assert!(!info.git_dirty);
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
    fn agent_kind_recognizes_qodercn() {
        // 线上/落盘字符串必须稳定:老 dozerd 读到 "qodercn" 会反序列化失败,所以名字一旦发出就不能改。
        assert_eq!(
            serde_json::to_string(&AgentKind::Qodercn).unwrap(),
            "\"qodercn\""
        );
        assert_eq!(
            serde_json::from_str::<AgentKind>("\"qodercn\"").unwrap(),
            AgentKind::Qodercn
        );
        assert_eq!(AgentKind::Qodercn.label(), "qodercn");
        assert_eq!(AgentKind::Qodercn.display_label(), "Qoder CN");
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
            ..Default::default()
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
    fn old_preview_context_json_decodes_with_new_field_defaults() {
        // Phase B 之前的 JSON 没有 revision/mode/read_only/... 字段,解码时
        // 必须取默认值,不能报错。
        let legacy = r#"{"path":"/repo/a.rs","start_line":2,"start_col":1,"end_line":2,"end_col":5,"has_selection":false,"updated_at_ms":1700000000000}"#;
        let ctx: PreviewContext = serde_json::from_str(legacy).unwrap();
        assert_eq!(ctx.path, "/repo/a.rs");
        assert_eq!(ctx.revision, 0);
        assert_eq!(ctx.mode, None);
        assert!(!ctx.read_only);
        assert_eq!(ctx.selected_text, None);
        assert_eq!(ctx.visible_start_line, None);
    }

    #[test]
    fn extended_preview_context_round_trips_via_protocol() {
        let mut ctx = PreviewContext {
            path: "/repo/a.rs".into(),
            start_line: 3,
            start_col: 1,
            end_line: 5,
            end_col: 2,
            has_selection: true,
            updated_at_ms: 42,
            revision: 7,
            mode: Some("code".into()),
            read_only: true,
            visible_start_line: Some(1),
            visible_end_line: Some(40),
            ..Default::default()
        };
        ctx.set_selected_text(Some("hello".into()));
        let req = Request::UpdatePreviewContext {
            project_id: 1,
            context: Some(ctx.clone()),
        };
        let line = encode_line(&req);
        assert_eq!(decode_line::<Request>(&line).unwrap(), req);
    }

    #[test]
    fn selected_text_is_capped_and_empty_is_none() {
        let mut ctx = PreviewContext::default();
        ctx.set_selected_text(Some(String::new()));
        assert_eq!(ctx.selected_text, None, "空选区视为无选区");

        let long = "x".repeat(PREVIEW_SELECTED_TEXT_MAX_CHARS + 500);
        ctx.set_selected_text(Some(long));
        assert_eq!(
            ctx.selected_text.as_ref().unwrap().chars().count(),
            PREVIEW_SELECTED_TEXT_MAX_CHARS
        );
    }

    #[test]
    fn tabular_context_round_trips_and_defaults_none() {
        // 旧 JSON 无 tabular 字段 → None(向后兼容)。
        let legacy = r#"{"path":"/r/a.csv","start_line":1,"start_col":1,"end_line":1,"end_col":1,"has_selection":false,"updated_at_ms":1}"#;
        let parsed: PreviewContext = serde_json::from_str(legacy).unwrap();
        assert_eq!(parsed.tabular, None);

        let ctx = PreviewContext {
            path: "/r/a.csv".into(),
            updated_at_ms: 1,
            tabular: Some(PreviewTabularContext {
                sheet: 2,
                scroll_row: 120,
                scroll_col: 3,
                ..Default::default()
            }),
            ..Default::default()
        };
        let req = Request::UpdatePreviewContext {
            project_id: 1,
            context: Some(ctx),
        };
        let line = encode_line(&req);
        assert_eq!(decode_line::<Request>(&line).unwrap(), req);
    }

    #[test]
    fn tabular_context_extended_fields_round_trip_and_default() {
        // T12:选中单元格/范围 + 可见行列;缺省字段旧 JSON 可解码。
        let legacy = r#"{"sheet":1,"scroll_row":2,"scroll_col":3}"#;
        let parsed: PreviewTabularContext = serde_json::from_str(legacy).unwrap();
        assert_eq!(parsed.selected_cell, None);
        assert_eq!(parsed.selected_range, None);
        assert_eq!(parsed.visible_rows, None);
        assert_eq!(parsed.visible_cols, None);

        let ctx = PreviewTabularContext {
            sheet: 1,
            scroll_row: 2,
            scroll_col: 3,
            visible_rows: Some((0, 40)),
            visible_cols: Some((0, 8)),
            selected_range: Some((2, 3, 5, 6)),
            ..Default::default()
        };
        let json = serde_json::to_string(&ctx).unwrap();
        assert_eq!(
            serde_json::from_str::<PreviewTabularContext>(&json).unwrap(),
            ctx
        );
        // 非单点范围 → selected_cell_from_range 为 None(单点范围才推导)。
        assert_eq!(ctx.selected_cell_from_range(), None);
    }

    #[test]
    fn preview_command_round_trips_and_outcomes_cover_categories() {
        // 只读导航命令:不带 expected_revision。
        let cmd = PreviewCommand {
            request_id: "r1".into(),
            project_id: 3,
            target: PreviewCommandTarget::Tab {
                panel: "files".into(),
                tab_id: 7,
            },
            action: PreviewCommandAction::Reveal {
                line: 12,
                column: 4,
            },
            expected_revision: None,
        };
        let req = Request::RunPreviewCommand {
            command: cmd.clone(),
        };
        let line = encode_line(&req);
        assert_eq!(decode_line::<Request>(&line).unwrap(), req);

        // replace 带 expected_revision,经 Reply 回 outcome。
        let rep = Reply::PreviewCommandResult {
            outcome: PreviewCommandOutcome::StaleRevision {
                request_id: "r2".into(),
                current_revision: 9,
            },
        };
        let line = encode_line(&rep);
        assert_eq!(decode_line::<Reply>(&line).unwrap(), rep);

        // 所有 outcome 类别都能 round-trip,且 request_id 可取出。
        for outcome in [
            PreviewCommandOutcome::Accepted {
                request_id: "x".into(),
            },
            PreviewCommandOutcome::NotFound {
                request_id: "x".into(),
                detail: "gone".into(),
            },
            PreviewCommandOutcome::StaleRevision {
                request_id: "x".into(),
                current_revision: 2,
            },
            PreviewCommandOutcome::UnsupportedBackend {
                request_id: "x".into(),
                detail: "image".into(),
            },
            PreviewCommandOutcome::LoadDenied {
                request_id: "x".into(),
                detail: "budget".into(),
            },
            PreviewCommandOutcome::Timeout {
                request_id: "x".into(),
            },
            PreviewCommandOutcome::InternalError {
                request_id: "x".into(),
                detail: "boom".into(),
            },
        ] {
            assert_eq!(outcome.request_id(), "x");
            let json = serde_json::to_string(&outcome).unwrap();
            assert_eq!(
                serde_json::from_str::<PreviewCommandOutcome>(&json).unwrap(),
                outcome
            );
        }

        // 缺省 expected_revision 的 JSON 可解码。
        let minimal: PreviewCommand = serde_json::from_str(
            r#"{"request_id":"m","project_id":1,"target":{"by":"path","path":"/p/a.rs"},"action":{"kind":"reveal","line":1,"column":1}}"#,
        )
        .unwrap();
        assert_eq!(minimal.expected_revision, None);
        assert!(matches!(
            minimal.action,
            PreviewCommandAction::Reveal { line: 1, column: 1 }
        ));
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

        let status_req = Request::SetTodoStatus {
            id: 5,
            status: TodoStoredStatus::Suspended,
        };
        let line = encode_line(&status_req);
        assert_eq!(decode_line::<Request>(&line).unwrap(), status_req);

        let todo = TodoInfo {
            id: 1,
            project_id: 1,
            text: "写完 spec".into(),
            done: false,
            paused: false,
            rank: 0,
            created_ms: 1_700_000_000_000,
            completed_at_ms: None,
            plan_date: Some("08-10".into()),
            dispatch_session_id: None,
            dispatch_at_ms: None,
            category_id: None,
            assigned_agent: None,
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
            auto_poll_enabled: false,
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
    fn memory_protocol_types_roundtrip() {
        let list_req = Request::ListMemories { project_id: 1 };
        let line = encode_line(&list_req);
        assert_eq!(decode_line::<Request>(&line).unwrap(), list_req);

        let write_req = Request::WriteMemory {
            project_id: 1,
            title: "标题A".into(),
            kind: "project".into(),
            description: "一句话摘要".into(),
            body: "正文内容".into(),
            actor: "claude".into(),
        };
        let line = encode_line(&write_req);
        assert_eq!(decode_line::<Request>(&line).unwrap(), write_req);

        let get_req = Request::GetMemory {
            project_id: 1,
            id: 5,
        };
        let line = encode_line(&get_req);
        assert_eq!(decode_line::<Request>(&line).unwrap(), get_req);

        let delete_req = Request::DeleteMemory {
            project_id: 1,
            id: 5,
            actor: "user".into(),
        };
        let line = encode_line(&delete_req);
        assert_eq!(decode_line::<Request>(&line).unwrap(), delete_req);

        let info = MemoryInfo {
            id: 1,
            project_id: 1,
            title: "标题A".into(),
            kind: "project".into(),
            description: "desc".into(),
            updated_ms: 1_700_000_000_000,
            updated_by: "claude".into(),
        };
        let list_reply = Reply::Memories {
            memories: vec![info],
        };
        let line = encode_line(&list_reply);
        assert_eq!(decode_line::<Reply>(&line).unwrap(), list_reply);

        let detail = MemoryDetail {
            id: 1,
            project_id: 1,
            title: "标题A".into(),
            kind: "project".into(),
            description: "desc".into(),
            body: "正文".into(),
            created_ms: 1_700_000_000_000,
            created_by: "claude".into(),
            updated_ms: 1_700_000_001_000,
            updated_by: "user".into(),
            history: vec![MemoryHistoryEntry {
                id: 1,
                memory_id: 1,
                changed_ms: 1_700_000_001_000,
                changed_by: "user".into(),
                change_kind: "updated".into(),
                title: "标题A".into(),
                kind: "project".into(),
                description: "desc".into(),
                body: "正文".into(),
            }],
        };
        let detail_reply = Reply::MemoryDetail { detail };
        let line = encode_line(&detail_reply);
        assert_eq!(decode_line::<Reply>(&line).unwrap(), detail_reply);
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

    #[test]
    fn assign_todo_agent_and_todo_detail_roundtrip() {
        let req = Request::AssignTodoAgent {
            id: 1,
            agent: AgentKind::Claude,
        };
        let line = encode_line(&req);
        assert_eq!(decode_line::<Request>(&line).unwrap(), req);

        let req = Request::SetCategoryAutoPoll {
            id: 10,
            enabled: true,
        };
        let line = encode_line(&req);
        assert_eq!(decode_line::<Request>(&line).unwrap(), req);

        let req = Request::ProcessTodoNow {
            id: 1,
            human_reply: Some("继续吧".into()),
        };
        let line = encode_line(&req);
        assert_eq!(decode_line::<Request>(&line).unwrap(), req);

        let req = Request::GetTodoDetail { id: 1 };
        let line = encode_line(&req);
        assert_eq!(decode_line::<Request>(&line).unwrap(), req);

        let turn = TurnRecord {
            turn_index: 0,
            role: "human".into(),
            content: "先看看这个 bug".into(),
            tool_calls: vec![],
            thinking: false,
            thinking_text: None,
            ts: Some(1),
            is_error: false,
            tool_result_call_id: None,
            tokens_in: 0,
            tokens_out: 0,
            tokens_cache_read: 0,
            tokens_cache_write: 0,
        };
        let todo = TodoInfo {
            id: 1,
            project_id: 1,
            text: "修个 bug".into(),
            done: false,
            paused: false,
            rank: 0,
            created_ms: 1,
            completed_at_ms: None,
            plan_date: None,
            dispatch_session_id: Some("mint-1".into()),
            dispatch_at_ms: Some(1),
            category_id: None,
            assigned_agent: Some(AgentKind::Claude),
        };
        let reply = Reply::TodoDetail {
            info: todo,
            turns: vec![turn],
        };
        let line = encode_line(&reply);
        assert_eq!(decode_line::<Reply>(&line).unwrap(), reply);
    }

    #[test]
    fn todo_info_assigned_agent_defaults_to_none_when_absent_from_json() {
        // 老协议帧没有 assigned_agent 字段,新增字段要能优雅缺省,不报错。
        let json = r#"{"id":1,"project_id":1,"text":"任务","done":false,"paused":false,
            "rank":0,"created_ms":0,"completed_at_ms":null,"plan_date":null,
            "dispatch_session_id":null,"dispatch_at_ms":null}"#;
        let todo: TodoInfo = serde_json::from_str(json).unwrap();
        assert_eq!(todo.assigned_agent, None);
    }

    #[test]
    fn highlight_action_round_trips() {
        let action = PreviewCommandAction::Highlight {
            start_line: 3,
            start_column: 1,
            end_line: 5,
            end_column: 10,
            duration_ms: 2000,
        };
        let json = serde_json::to_string(&action).unwrap();
        let back: PreviewCommandAction = serde_json::from_str(&json).unwrap();
        assert_eq!(action, back);
    }

    #[test]
    fn locate_in_file_request_round_trips() {
        let req = Request::LocateInFile {
            project_id: 1,
            path: "src/lib.rs".into(),
            query: "fn main".into(),
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: Request = serde_json::from_str(&json).unwrap();
        assert_eq!(req, back);
    }

    #[test]
    fn apply_precise_edit_request_round_trips() {
        let req = Request::ApplyPreciseEdit {
            project_id: 1,
            path: "src/lib.rs".into(),
            start_line: 3,
            start_col: 1,
            end_line: 3,
            end_col: 10,
            expected_text: "old".into(),
            new_text: "new".into(),
            summary: "修正拼写".into(),
            actor: "claude".into(),
            session_id: "sess-1".into(),
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: Request = serde_json::from_str(&json).unwrap();
        assert_eq!(req, back);
    }

    #[test]
    fn context_and_history_requests_round_trip() {
        let reqs = [
            Request::AddContextItem {
                project_id: 1,
                entity_kind: "file".into(),
                entity_ref: "src/lib.rs".into(),
            },
            Request::RemoveContextItem {
                project_id: 1,
                id: 7,
            },
            Request::ListContextItems { project_id: 1 },
            Request::ListFileEditHistory {
                project_id: 1,
                path_filter: Some("src".into()),
                limit: 200,
            },
            Request::ListFileEditHistory {
                project_id: 1,
                path_filter: None,
                limit: 50,
            },
        ];
        for req in reqs {
            let json = serde_json::to_string(&req).unwrap();
            let back: Request = serde_json::from_str(&json).unwrap();
            assert_eq!(req, back);
        }
    }

    #[test]
    fn context_and_history_replies_round_trip() {
        let item = ContextItemInfo {
            id: 3,
            project_id: 1,
            entity_kind: "dir".into(),
            entity_ref: "research".into(),
            created_ms: 1_700_000_000_000,
        };
        let entry = FileEditHistoryInfo {
            id: 9,
            project_id: 1,
            target_path: "a.txt".into(),
            actor: "claude".into(),
            session_id: "s1".into(),
            start_line: 2,
            start_col: 1,
            end_line: 2,
            end_col: 9,
            new_end_line: 3,
            new_end_col: 4,
            old_text: "line two".into(),
            new_text: "x\nabc".into(),
            summary: "改".into(),
            created_ms: 1_700_000_000_001,
        };
        let replies = [
            Reply::ContextItem { item: item.clone() },
            Reply::ContextItems { items: vec![item] },
            Reply::FileEditHistory {
                entries: vec![entry],
            },
        ];
        for reply in replies {
            let json = serde_json::to_string(&reply).unwrap();
            let back: Reply = serde_json::from_str(&json).unwrap();
            assert_eq!(reply, back);
        }
    }

    #[test]
    fn mutation_outcome_variants_round_trip() {
        let variants = [
            MutationOutcome::Applied {
                new_start_line: 3,
                new_start_col: 1,
                new_end_line: 3,
                new_end_col: 12,
                history_id: 42,
            },
            MutationOutcome::Conflict {
                actual_text: "实际内容".into(),
            },
            MutationOutcome::NotFound,
            MutationOutcome::PathOutOfBounds,
            MutationOutcome::Unwritable {
                reason: "非 UTF-8".into(),
            },
        ];
        for v in variants {
            let json = serde_json::to_string(&v).unwrap();
            let back: MutationOutcome = serde_json::from_str(&json).unwrap();
            assert_eq!(v, back);
        }
    }

    #[test]
    fn group_chat_protocol_types_roundtrip() {
        let req = Request::PostGroupMessage {
            group_id: 3,
            text: "@claude 看下这个方案".into(),
        };
        let back: Request = decode_line(&encode_line(&req)).unwrap();
        assert_eq!(back, req);

        let msg = GroupMessageInfo {
            id: 9,
            group_id: 3,
            seq: 4,
            rev: 12,
            author: GroupAuthor::Member { member_id: 2 },
            text: "好的".into(),
            mentions: vec![],
            status: Some(GroupMessageStatus::Failed {
                reason: "超时".into(),
            }),
            duration_ms: Some(1500),
            created_ms: 1,
            todo_id: None,
        };
        let line = encode_line(&Reply::GroupMessages {
            messages: vec![msg.clone()],
            latest_rev: 12,
        });
        assert!(line.contains("\"state\":\"failed\""), "{line}");
        assert!(line.contains("\"reason\":\"超时\""), "{line}");
        match decode_line::<Reply>(&line).unwrap() {
            Reply::GroupMessages {
                messages,
                latest_rev,
            } => {
                assert_eq!(messages, vec![msg]);
                assert_eq!(latest_rev, 12);
            }
            other => panic!("意外应答: {other:?}"),
        }

        let cancel = Request::CancelGroup {
            group_id: 3,
            scope: GroupCancelScope::Round,
        };
        let back: Request = decode_line(&encode_line(&cancel)).unwrap();
        assert_eq!(back, cancel);
    }
}
