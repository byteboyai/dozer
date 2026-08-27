//! 会话总结存储:rusqlite 单表,主键 session_id(spec 2026-08-27)。
//! Mutex<Connection>——同 AcceptanceStore,写入频度低,无需连接池。

use anyhow::{Context, Result};
use dozer_core::protocol::{AgentKind, SessionSummaryPayload, SummaryStatus};
use rusqlite::Connection;
use std::path::Path;
use std::sync::Mutex;

fn agent_to_str(a: AgentKind) -> &'static str {
    a.label()
}

fn agent_from_str(s: &str) -> AgentKind {
    match s {
        "claude" => AgentKind::Claude,
        "codebuddy" => AgentKind::Codebuddy,
        "opencode" => AgentKind::Opencode,
        "codex" => AgentKind::Codex,
        "kilo" => AgentKind::Kilo,
        "v8agent" => AgentKind::V8agent,
        _ => AgentKind::Unknown,
    }
}

fn status_to_str(s: SummaryStatus) -> &'static str {
    match s {
        SummaryStatus::AiGenerated => "ai_generated",
        SummaryStatus::HeuristicFallback => "heuristic_fallback",
    }
}

fn status_from_str(s: &str) -> SummaryStatus {
    match s {
        "ai_generated" => SummaryStatus::AiGenerated,
        _ => SummaryStatus::HeuristicFallback,
    }
}

const HEURISTIC_PLACEHOLDER: &str = "(无对话记录)";
const HEURISTIC_TITLE_MAX_CHARS: usize = 60;

/// 没等到 agent 交回总结时的兜底算法:用已摄取的对话数据(仅取人类回合,
/// 与 `truncate_activity` 的 60 字符惯例保持一致)拼一份"没有语义压缩"的
/// 降级总结。空结果返回固定占位文案而不是空字符串,保证消费方不用处理
/// "有的 session 干脆没有总结"这种特例(spec 2026-08-27 D5)。
pub fn heuristic_from_turns(turns: &[dozer_core::protocol::TurnRecord]) -> (String, String) {
    let human: Vec<&str> = turns
        .iter()
        .filter(|t| t.role == "human")
        .map(|t| t.content.as_str())
        .collect();
    if human.is_empty() {
        return (HEURISTIC_PLACEHOLDER.into(), HEURISTIC_PLACEHOLDER.into());
    }
    (
        truncate_chars(human[0], HEURISTIC_TITLE_MAX_CHARS),
        human.join("\n"),
    )
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let head: String = s.chars().take(max).collect();
        format!("{head}…")
    }
}

pub struct SessionSummaryStore {
    conn: Mutex<Connection>,
}

impl SessionSummaryStore {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("建库目录")?;
        }
        let conn = Connection::open(path).context("打开 SQLite")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS session_summaries (
                session_id TEXT PRIMARY KEY,
                agent_kind TEXT NOT NULL,
                title TEXT NOT NULL,
                summary TEXT NOT NULL,
                status TEXT NOT NULL,
                created_ts_ms INTEGER NOT NULL
            );",
        )
        .context("建表")?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn record(&self, payload: &SessionSummaryPayload) -> Result<()> {
        self.conn.lock().expect("db lock").execute(
            "INSERT INTO session_summaries
             (session_id, agent_kind, title, summary, status, created_ts_ms)
             VALUES (?1,?2,?3,?4,?5,?6)
             ON CONFLICT(session_id) DO UPDATE SET
                agent_kind = excluded.agent_kind,
                title = excluded.title,
                summary = excluded.summary,
                status = excluded.status,
                created_ts_ms = excluded.created_ts_ms",
            rusqlite::params![
                payload.session_id,
                agent_to_str(payload.agent_kind),
                payload.title,
                payload.summary,
                status_to_str(payload.status),
                payload.created_ts_ms,
            ],
        )?;
        Ok(())
    }

    pub fn get(&self, session_id: &str) -> Result<Option<SessionSummaryPayload>> {
        let conn = self.conn.lock().expect("db lock");
        let mut stmt = conn.prepare(
            "SELECT session_id, agent_kind, title, summary, status, created_ts_ms
             FROM session_summaries WHERE session_id = ?1",
        )?;
        let mut rows = stmt.query(rusqlite::params![session_id])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        let agent_kind: String = row.get(1)?;
        let status: String = row.get(4)?;
        let created_ts_ms: i64 = row.get(5)?;
        Ok(Some(SessionSummaryPayload {
            session_id: row.get(0)?,
            agent_kind: agent_from_str(&agent_kind),
            title: row.get(2)?,
            summary: row.get(3)?,
            status: status_from_str(&status),
            created_ts_ms: created_ts_ms as u64,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(session_id: &str) -> SessionSummaryPayload {
        SessionSummaryPayload {
            session_id: session_id.into(),
            agent_kind: AgentKind::Claude,
            title: "改了个函数".into(),
            summary: "详细过程".into(),
            status: SummaryStatus::AiGenerated,
            created_ts_ms: 1_700_000_000_000,
        }
    }

    #[test]
    fn open_record_get_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionSummaryStore::open(&dir.path().join("t.db")).unwrap();
        assert!(store.get("s1").unwrap().is_none());
        store.record(&payload("s1")).unwrap();
        let got = store.get("s1").unwrap().unwrap();
        assert_eq!(got, payload("s1"));
    }

    #[test]
    fn record_twice_overwrites_not_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionSummaryStore::open(&dir.path().join("t.db")).unwrap();
        store.record(&payload("s1")).unwrap();
        let mut second = payload("s1");
        second.title = "第二次总结".into();
        second.status = SummaryStatus::HeuristicFallback;
        store.record(&second).unwrap();
        let got = store.get("s1").unwrap().unwrap();
        assert_eq!(got.title, "第二次总结");
        assert_eq!(got.status, SummaryStatus::HeuristicFallback);
    }

    #[test]
    fn persists_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        {
            let store = SessionSummaryStore::open(&path).unwrap();
            store.record(&payload("s1")).unwrap();
        }
        let store = SessionSummaryStore::open(&path).unwrap();
        assert!(store.get("s1").unwrap().is_some());
    }

    fn turn(role: &str, content: &str) -> dozer_core::protocol::TurnRecord {
        dozer_core::protocol::TurnRecord {
            role: role.into(),
            content: content.into(),
            ..Default::default()
        }
    }

    #[test]
    fn heuristic_uses_first_human_turn_as_title() {
        let turns = vec![
            turn("human", "帮我改一下 README"),
            turn("ai", "好的,我来改"),
            turn("human", "再加一段安装说明"),
        ];
        let (title, summary) = heuristic_from_turns(&turns);
        assert_eq!(title, "帮我改一下 README");
        assert_eq!(summary, "帮我改一下 README\n再加一段安装说明");
    }

    #[test]
    fn heuristic_ignores_non_human_turns() {
        let turns = vec![turn("ai", "纯 AI 输出"), turn("tool", "工具结果")];
        let (title, summary) = heuristic_from_turns(&turns);
        assert_eq!(title, "(无对话记录)");
        assert_eq!(summary, "(无对话记录)");
    }

    #[test]
    fn heuristic_truncates_long_title_at_60_chars() {
        let long = "a".repeat(100);
        let turns = vec![turn("human", &long)];
        let (title, _) = heuristic_from_turns(&turns);
        assert_eq!(title.chars().count(), 61); // 60 + "…"
        assert!(title.ends_with('…'));
    }

    #[test]
    fn heuristic_empty_turns_returns_placeholder() {
        let (title, summary) = heuristic_from_turns(&[]);
        assert_eq!(title, "(无对话记录)");
        assert_eq!(summary, "(无对话记录)");
    }
}
