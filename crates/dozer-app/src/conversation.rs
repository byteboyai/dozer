//! 历史对话展示用的中间表示(P1j 起步;P2b 扩展到 CodeBuddy/OpenCode;
//! spec 2026-08-20 起数据来源改为查询 dozerd,本文件不再直接碰磁盘)。

use dozer_core::protocol::{AgentKind, ConversationSummary};
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

/// 该对话是否是当前活会话(其路径在打开着的 transcript 集合中)。纯函数。
pub fn is_current_conversation(meta_path: &std::path::Path, open_transcripts: &[String]) -> bool {
    let p = meta_path.to_string_lossy();
    open_transcripts.iter().any(|o| o.as_str() == p)
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
    fn is_current_matches_open_transcript() {
        let opens = vec!["/t/a.jsonl".to_string(), "/t/b.jsonl".to_string()];
        assert!(is_current_conversation(
            std::path::Path::new("/t/a.jsonl"),
            &opens
        ));
        assert!(!is_current_conversation(
            std::path::Path::new("/t/c.jsonl"),
            &opens
        ));
    }
}
