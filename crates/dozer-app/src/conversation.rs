//! 历史对话展示用的中间表示(P1j 起步;P2b 扩展到 CodeBuddy/OpenCode;
//! spec 2026-08-20 起数据来源改为查询 dozerd,本文件不再直接碰磁盘)。

use dozer_core::protocol::{AgentKind, ConversationSummary, TurnGroupEntry};
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

/// 一行回合分组在 GUI 侧的展示用镜像(`TurnGroupEntry` 的 1:1 拷贝)。
/// 对话面板扁平列表用(2026-08-21，取代按 session 展开的树状展示)——
/// 自带 `path`/`agent`，点击一行就能直接打开审阅，不用像树状版那样
/// 反查所属 session。
#[derive(Debug, Clone, PartialEq)]
pub struct TurnGroupRow {
    pub path: PathBuf,
    pub agent: AgentKind,
    pub start_turn_index: i64,
    pub end_turn_index: i64,
    pub title: String,
    pub ts: u64,
}

impl TurnGroupRow {
    pub fn from_entry(e: &TurnGroupEntry) -> Self {
        Self {
            path: PathBuf::from(&e.file_path),
            agent: e.agent,
            start_turn_index: e.start_turn_index,
            end_turn_index: e.end_turn_index,
            title: e.title.clone(),
            ts: e.ts,
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
    fn turn_group_row_from_entry_maps_fields() {
        let e = TurnGroupEntry {
            conversation_id: "abc".into(),
            file_path: "/h/.claude/projects/x/abc.jsonl".into(),
            agent: AgentKind::Claude,
            start_turn_index: 2,
            end_turn_index: 7,
            title: "标题".into(),
            ts: 100,
        };
        let row = TurnGroupRow::from_entry(&e);
        assert_eq!(
            row.path,
            std::path::PathBuf::from("/h/.claude/projects/x/abc.jsonl")
        );
        assert_eq!(row.agent, AgentKind::Claude);
        assert_eq!(row.start_turn_index, 2);
        assert_eq!(row.end_turn_index, 7);
        assert_eq!(row.title, "标题");
        assert_eq!(row.ts, 100);
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
