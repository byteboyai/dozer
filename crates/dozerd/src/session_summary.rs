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
}
