//! 历史对话展示用的中间表示(P1j 起步;P2b 扩展到 CodeBuddy/OpenCode;
//! spec 2026-08-20 起数据来源改为查询 dozerd,本文件不再直接碰磁盘)。

use dozer_core::protocol::{AgentKind, ConversationSummary, SessionSummaryPayload, SummaryStatus};
use std::path::PathBuf;

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
/// shell/SSH/Codex/Kilo)。`summary` 存全文不截断——列表渲染时截断成
/// 预览,详情页直接整段展示,不为详情页单独发一次查询。
#[derive(Debug, Clone, PartialEq)]
pub struct SessionRow {
    pub conversation_id: String,
    pub agent: AgentKind,
    pub last_ts: u64,
    pub display_title: String,
    pub summary: Option<String>,
    pub summary_status: Option<SummaryStatus>,
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
        }
    }
}

/// 该会话是否是当前活会话(其 `conversation_id` 等于打开着的某个 transcript
/// 文件名的 `file_stem()`)。纯函数,2026-08-27 取代按完整文件路径匹配的
/// `is_current_conversation`。
pub fn is_current_conversation_id(conversation_id: &str, open_transcripts: &[String]) -> bool {
    open_transcripts.iter().any(|p| {
        std::path::Path::new(p).file_stem().and_then(|s| s.to_str()) == Some(conversation_id)
    })
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

    #[test]
    fn is_current_matches_open_transcript_by_file_stem() {
        let opens = vec!["/t/a.jsonl".to_string(), "/t/b.jsonl".to_string()];
        assert!(is_current_conversation_id("a", &opens));
        assert!(is_current_conversation_id("b", &opens));
        assert!(!is_current_conversation_id("c", &opens));
        assert!(!is_current_conversation_id("", &opens));
    }
}
