use crate::AgentKind;
use serde::{Deserialize, Serialize};

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
    /// 该会话是否由 Todo 任务处理产生,是则记该任务 `TodoInfo.id`。会话
    /// 面板据此展示"关联任务"标签。与交互式 PTY 会话无关时为 `None`。
    #[serde(default)]
    pub task_id: Option<i64>,
}

/// 总结任务的错误分类(spec 2026-09-26 第 5 节)。持久化到 `summary_jobs`,
/// UI 据此给出可操作的修复提示。区分"瞬态可重试"(rate_limit/timeout/
/// nonzero_exit)与"配置/能力问题"(configuration_required/unsupported_/
/// authentication),前者自动重试、后者熔断整批。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SummaryErrorKind {
    ConfigurationRequired,
    UnsupportedCapability,
    Spawn,
    Authentication,
    RateLimit,
    Timeout,
    NonzeroExit,
    InvalidOutput,
    InputUnavailable,
    StorageError,
    BudgetExceeded,
    Cancelled,
}

/// 总结任务状态机:`queued → running → succeeded | failed`;取消为
/// `cancelled`(spec 2026-09-26 第 4 节)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SummaryJobStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

/// 总结任务的触发来源(spec 2026-09-26 第 3 节)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SummaryTrigger {
    Close,
    Shutdown,
    Backfill,
    Manual,
    NaturalExit,
}

/// 一份规范会话总结结果(`conversation_summary_results` 表一行,
/// `conversation_id` 唯一)。与旧 `SessionSummaryPayload` 的差异:这里带
/// `source_revision`/`pipeline_version`/`provider`/`coverage` 等覆盖证据,
/// 是"新版完整总结"的真相源,旧表只作为历史展示数据保留。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConversationSummaryResult {
    pub conversation_id: String,
    pub title: String,
    pub summary: String,
    /// 结构化 facts(JSON 字符串),空为 None。
    #[serde(default)]
    pub facts_json: Option<String>,
    pub source_revision: String,
    pub pipeline_version: String,
    pub provider: AgentKind,
    pub requested_model: Option<String>,
    pub reported_model: Option<String>,
    /// 覆盖范围描述(如 `turns 0..42`,或 `partial`),供 UI 展示"有没有漏"。
    #[serde(default)]
    pub coverage: Option<String>,
    pub generated_at: u64,
    pub source_job_id: i64,
}

/// 一条总结任务(`summary_jobs` 表一行)的对外视图。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SummaryJobInfo {
    pub job_id: i64,
    pub conversation_id: String,
    pub source_session_id: Option<String>,
    pub trigger: SummaryTrigger,
    pub provider: AgentKind,
    pub requested_model: Option<String>,
    pub source_revision: String,
    pub pipeline_version: String,
    pub generation: i64,
    pub status: SummaryJobStatus,
    pub attempt: u32,
    pub error_kind: Option<SummaryErrorKind>,
    /// 脱敏后的错误描述(不含 transcript 全文、认证值)。
    #[serde(default)]
    pub error_detail: Option<String>,
    pub created_ts_ms: u64,
    pub updated_ts_ms: u64,
    /// 当前分块阶段(如 `extract:3/7`),`None` 表示无阶段信息。
    #[serde(default)]
    pub phase: Option<String>,
}

/// 一条总结批次(`summary_batches` 表一行)的对外视图。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SummaryBatchInfo {
    pub batch_id: i64,
    pub cwd: String,
    /// 选择策略(如 `missing_or_stale`、`force`),纯展示/诊断用。
    pub strategy: String,
    pub total: u32,
    pub queued: u32,
    pub running: u32,
    pub succeeded: u32,
    pub failed: u32,
    pub cancelled: u32,
    pub skipped: u32,
    pub created_ts_ms: u64,
    pub updated_ts_ms: u64,
}

/// summary provider 解析结果(spec 2026-09-26 第 5 节)。`required` 为真表示
/// 未配置(UI 应提示选择已安装总结器);`source` 取值 `ui`/`summary`/
/// `default_agent`,供 UI 明示来源。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SummaryProviderInfo {
    pub provider: Option<AgentKind>,
    pub model: Option<String>,
    pub source: Option<String>,
    pub required: bool,
}
