use crate::AgentKind;
use crate::{SessionSummaryPayload, SummaryStatus};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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

#[derive(Debug, Clone, PartialEq)]
pub struct ConversationMeta {
    pub path: PathBuf,
    pub title: String,
    pub modified_ms: u64,
    pub agent: AgentKind,
}

impl ConversationMeta {
    pub fn from_summary(s: &ConversationSummary) -> Self {
        Self {
            path: PathBuf::from(&s.file_path),
            title: s.title.clone(),
            modified_ms: s.last_ts,
            agent: s.agent,
        }
    }
}

/// 会话列表一行(2026-08-27,取代按回合分组的 `TurnGroupRow`)。有总结用
/// 总结的标题/全文,没有降级用 `ConversationSummary.title`(旧数据/纯
/// shell/SSH/Codex)。`summary` 存全文不截断——列表渲染时截断成
/// 预览,详情页直接整段展示,不为详情页单独发一次查询。
#[derive(Debug, Clone, PartialEq)]
pub struct SessionRow {
    pub conversation_id: String,
    pub agent: AgentKind,
    pub last_ts: u64,
    pub display_title: String,
    pub summary: Option<String>,
    pub summary_status: Option<SummaryStatus>,
    /// 该会话关联的 Todo 任务 id(有则来自 `SessionSummaryPayload.task_id`),
    /// 会话面板据此展示"关联任务"标签。`None` = 与任务无关的普通交互式
    /// 会话。
    pub task_id: Option<i64>,
}

impl SessionRow {
    pub fn from_row(c: &ConversationSummary, s: Option<&SessionSummaryPayload>) -> Self {
        Self {
            conversation_id: c.conversation_id.clone(),
            agent: c.agent,
            last_ts: c.last_ts,
            display_title: s
                .map(|s| s.title.clone())
                .unwrap_or_else(|| c.title.clone()),
            summary: s.map(|s| s.summary.clone()),
            summary_status: s.map(|s| s.status),
            task_id: s.and_then(|s| s.task_id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_summary_maps_fields() {
        let s = ConversationSummary {
            conversation_id: "abc".into(),
            agent: AgentKind::Claude,
            file_path: "/h/.claude/projects/x/abc.jsonl".into(),
            title: "标题".into(),
            first_ts: 1,
            last_ts: 2,
            turn_count: 3,
        };
        let meta = ConversationMeta::from_summary(&s);
        assert_eq!(
            meta.path,
            std::path::PathBuf::from("/h/.claude/projects/x/abc.jsonl")
        );
        assert_eq!(meta.title, "标题");
        assert_eq!(meta.modified_ms, 2);
        assert_eq!(meta.agent, AgentKind::Claude);
    }

    #[test]
    fn session_row_from_row_uses_summary_title_and_text_when_present() {
        let c = ConversationSummary {
            conversation_id: "c1".into(),
            agent: AgentKind::Claude,
            file_path: "/h/.claude/projects/x/c1.jsonl".into(),
            title: "原始标题".into(),
            first_ts: 1,
            last_ts: 100,
            turn_count: 3,
        };
        let s = SessionSummaryPayload {
            session_id: "s1".into(),
            agent_kind: AgentKind::Claude,
            conversation_id: Some("c1".into()),
            title: "总结标题".into(),
            summary: "总结全文".into(),
            status: SummaryStatus::AiGenerated,
            created_ts_ms: 1,
            task_id: None,
        };
        let row = SessionRow::from_row(&c, Some(&s));
        assert_eq!(row.conversation_id, "c1");
        assert_eq!(row.agent, AgentKind::Claude);
        assert_eq!(row.last_ts, 100);
        assert_eq!(row.display_title, "总结标题");
        assert_eq!(row.summary.as_deref(), Some("总结全文"));
        assert_eq!(row.summary_status, Some(SummaryStatus::AiGenerated));
    }

    #[test]
    fn session_row_from_row_falls_back_to_conversation_title_without_summary() {
        let c = ConversationSummary {
            conversation_id: "c2".into(),
            agent: AgentKind::Codex,
            file_path: "/h/.codex/x/c2.jsonl".into(),
            title: "原始标题".into(),
            first_ts: 1,
            last_ts: 100,
            turn_count: 1,
        };
        let row = SessionRow::from_row(&c, None);
        assert_eq!(row.display_title, "原始标题");
        assert_eq!(row.summary, None);
        assert_eq!(row.summary_status, None);
    }
}
