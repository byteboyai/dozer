//! 会话总结存储:rusqlite 单表,主键 session_id(spec 2026-08-27)。
//! Mutex<Connection>——同 AcceptanceStore,写入频度低,无需连接池。

use anyhow::{Context, Result};
use dozer_core::protocol::{AgentKind, SessionSummaryPayload, SummaryStatus};
use rusqlite::Connection;
use std::collections::HashMap;
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
                created_ts_ms INTEGER NOT NULL,
                task_id INTEGER
            );",
        )
        .context("建表")?;
        // 老库(建表时还没有 conversation_id 列)迁移:CREATE TABLE IF NOT
        // EXISTS 对已存在的表不生效,新列需要单独补(同 `conversation_turns`
        // 补 `is_error` 列的既有写法,2026-08-27 修正)。
        let has_conversation_id: bool = conn
            .prepare(
                "SELECT 1 FROM pragma_table_info('session_summaries') WHERE name = 'conversation_id'",
            )?
            .exists([])?;
        if !has_conversation_id {
            conn.execute(
                "ALTER TABLE session_summaries ADD COLUMN conversation_id TEXT",
                [],
            )
            .context("迁移 conversation_id 列")?;
        }
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_session_summaries_conversation_id
                ON session_summaries(conversation_id);",
        )
        .context("建索引")?;
        let has_task_id: bool = conn
            .prepare("SELECT 1 FROM pragma_table_info('session_summaries') WHERE name = 'task_id'")?
            .exists([])?;
        if !has_task_id {
            conn.execute(
                "ALTER TABLE session_summaries ADD COLUMN task_id INTEGER",
                [],
            )
            .context("迁移 task_id 列")?;
        }
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn record(&self, payload: &SessionSummaryPayload) -> Result<()> {
        self.conn.lock().expect("db lock").execute(
            "INSERT INTO session_summaries
             (session_id, agent_kind, conversation_id, title, summary, status, created_ts_ms, task_id)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)
             ON CONFLICT(session_id) DO UPDATE SET
                agent_kind = excluded.agent_kind,
                conversation_id = excluded.conversation_id,
                title = excluded.title,
                summary = excluded.summary,
                status = excluded.status,
                created_ts_ms = excluded.created_ts_ms,
                task_id = excluded.task_id",
            rusqlite::params![
                payload.session_id,
                agent_to_str(payload.agent_kind),
                payload.conversation_id,
                payload.title,
                payload.summary,
                status_to_str(payload.status),
                payload.created_ts_ms,
                payload.task_id,
            ],
        )?;
        Ok(())
    }

    pub fn get(&self, session_id: &str) -> Result<Option<SessionSummaryPayload>> {
        let conn = self.conn.lock().expect("db lock");
        let mut stmt = conn.prepare(
            "SELECT session_id, agent_kind, conversation_id, title, summary, status, created_ts_ms, task_id
             FROM session_summaries WHERE session_id = ?1",
        )?;
        let mut rows = stmt.query(rusqlite::params![session_id])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        let agent_kind: String = row.get(1)?;
        let status: String = row.get(5)?;
        let created_ts_ms: i64 = row.get(6)?;
        Ok(Some(SessionSummaryPayload {
            session_id: row.get(0)?,
            agent_kind: agent_from_str(&agent_kind),
            conversation_id: row.get(2)?,
            title: row.get(3)?,
            summary: row.get(4)?,
            status: status_from_str(&status),
            created_ts_ms: created_ts_ms as u64,
            task_id: row.get(7)?,
        }))
    }

    /// 按 `conversation_id` 批量查总结,返回以 `conversation_id` 为键的 map。
    /// 空输入返回空 map;某个 id 查不到就不出现在 map 里(调用方据此把该项
    /// 判为"没有总结")。只 `prepare` 一次,避免"N 条会话 prepare N 次"
    /// (同 `TranscriptStore::get_usage_summary_in` 的既有口径)。SQL 注入走
    /// rusqlite 参数绑定,id 本身永远是占位符不是拼接。
    pub fn get_many(
        &self,
        conversation_ids: &[String],
    ) -> Result<HashMap<String, SessionSummaryPayload>> {
        if conversation_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let placeholders = conversation_ids
            .iter()
            .map(|_| "?")
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT session_id, agent_kind, conversation_id, title, summary, status, created_ts_ms, task_id
             FROM session_summaries WHERE conversation_id IN ({placeholders})"
        );
        let conn = self.conn.lock().expect("db lock");
        let mut stmt = conn.prepare(&sql)?;
        let mut rows = stmt.query(rusqlite::params_from_iter(conversation_ids))?;
        let mut out = HashMap::new();
        while let Some(row) = rows.next()? {
            let agent_kind: String = row.get(1)?;
            let status: String = row.get(5)?;
            let created_ts_ms: i64 = row.get(6)?;
            let payload = SessionSummaryPayload {
                session_id: row.get(0)?,
                agent_kind: agent_from_str(&agent_kind),
                conversation_id: row.get(2)?,
                title: row.get(3)?,
                summary: row.get(4)?,
                status: status_from_str(&status),
                created_ts_ms: created_ts_ms as u64,
                task_id: row.get(7)?,
            };
            if let Some(id) = &payload.conversation_id {
                out.insert(id.clone(), payload);
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(session_id: &str) -> SessionSummaryPayload {
        SessionSummaryPayload {
            session_id: session_id.into(),
            agent_kind: AgentKind::Claude,
            conversation_id: Some(format!("conv-{session_id}")),
            title: "改了个函数".into(),
            summary: "详细过程".into(),
            status: SummaryStatus::AiGenerated,
            created_ts_ms: 1_700_000_000_000,
            task_id: None,
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

    #[test]
    fn record_and_get_roundtrip_with_no_conversation_id() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionSummaryStore::open(&dir.path().join("t.db")).unwrap();
        let mut p = payload("s1");
        p.conversation_id = None;
        store.record(&p).unwrap();
        let got = store.get("s1").unwrap().unwrap();
        assert_eq!(got.conversation_id, None);
    }

    /// 老库(建表时还没有 conversation_id 列)迁移:手写不带该列的旧表结构,
    /// 重新 `open` 应该幂等补上这一列,而不是报错或丢数据(2026-08-27 修正)。
    #[test]
    fn open_on_pre_existing_db_without_conversation_id_column_adds_it() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("old.db");
        {
            let conn = rusqlite::Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE session_summaries (
                    session_id TEXT PRIMARY KEY,
                    agent_kind TEXT NOT NULL,
                    title TEXT NOT NULL,
                    summary TEXT NOT NULL,
                    status TEXT NOT NULL,
                    created_ts_ms INTEGER NOT NULL
                );
                INSERT INTO session_summaries VALUES
                    ('s1', 'claude', '旧标题', '旧摘要', 'ai_generated', 1);",
            )
            .unwrap();
        }
        let store = SessionSummaryStore::open(&db_path).unwrap();
        let got = store.get("s1").unwrap().unwrap();
        assert_eq!(got.title, "旧标题");
        assert_eq!(got.conversation_id, None, "老行迁移后该列应为 NULL");

        // 迁移后的库仍能正常写入带 conversation_id 的新数据。
        store.record(&payload("s2")).unwrap();
        assert_eq!(
            store.get("s2").unwrap().unwrap().conversation_id,
            Some("conv-s2".into())
        );
    }

    #[test]
    fn record_and_get_roundtrips_task_id() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionSummaryStore::open(&dir.path().join("t.db")).unwrap();
        let mut p = payload("s1");
        p.task_id = Some(42);
        store.record(&p).unwrap();
        let got = store.get("s1").unwrap().unwrap();
        assert_eq!(got.task_id, Some(42));
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

    #[test]
    fn get_many_empty_input_returns_empty_map() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionSummaryStore::open(&dir.path().join("t.db")).unwrap();
        assert!(store.get_many(&[]).unwrap().is_empty());
    }

    #[test]
    fn get_many_batches_multiple_ids_partial_hit() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionSummaryStore::open(&dir.path().join("t.db")).unwrap();
        let mut p1 = payload("s1");
        p1.conversation_id = Some("c1".into());
        let mut p2 = payload("s2");
        p2.conversation_id = Some("c2".into());
        store.record(&p1).unwrap();
        store.record(&p2).unwrap();

        let got = store
            .get_many(&["c1".to_string(), "c2".to_string(), "c-missing".to_string()])
            .unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got["c1"].session_id, "s1");
        assert_eq!(got["c2"].session_id, "s2");
        assert!(!got.contains_key("c-missing"));
    }

    #[test]
    fn get_many_ignores_rows_with_null_conversation_id() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionSummaryStore::open(&dir.path().join("t.db")).unwrap();
        let mut p = payload("s1");
        p.conversation_id = None;
        store.record(&p).unwrap();
        // 查一个跟这行完全无关的 id 列表——不该因为库里存在 conversation_id
        // 为 NULL 的行就出错或误命中。
        assert!(store.get_many(&["c1".to_string()]).unwrap().is_empty());
    }

    #[test]
    fn get_many_ids_with_special_characters_are_parameter_bound_not_concatenated() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionSummaryStore::open(&dir.path().join("t.db")).unwrap();
        let mut p = payload("s1");
        // 含单引号的 id——若实现拼字符串而不是走参数绑定,这里会破坏 SQL。
        p.conversation_id = Some("weird'id".into());
        store.record(&p).unwrap();
        let got = store.get_many(&["weird'id".to_string()]).unwrap();
        assert_eq!(got.len(), 1);
    }
}
