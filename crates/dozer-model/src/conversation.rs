use crate::AgentKind;
use serde::{Deserialize, Serialize};

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
    /// 这次工具调用的稳定 id（Claude tool_use.id、Goose tool_call_id 等）；
    /// 前端拿它和 tool_result 的 call_id 精确配对，agent/协议不提供 id 时
    /// 为 None(2026-09-25 起，见
    /// docs/superpowers/specs/2026-09-25-tool-call-result-pairing-design.md)。
    #[serde(default)]
    pub id: Option<String>,
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
    /// role == "tool_result" 时，这次结果对应的调用 id（读时解析补上，同
    /// `ToolCallInfo.id` 语义，用于前端精确配对；其余角色恒 None）。
    #[serde(default)]
    pub tool_result_call_id: Option<String>,
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
