//! agent 对话/用量摄取管线(spec 2026-08-20)。

pub mod parse;
pub mod scan;

use anyhow::{Context, Result};
use dozer_core::protocol::AgentKind;
use rusqlite::{Connection, OptionalExtension, params};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Mutex;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn agent_to_str(a: AgentKind) -> &'static str {
    match a {
        AgentKind::Unknown => "unknown",
        AgentKind::Claude => "claude",
        AgentKind::Codebuddy => "codebuddy",
        AgentKind::Opencode => "opencode",
        AgentKind::Codex => "codex",
        AgentKind::Qoder => "qoder",
        AgentKind::Kilo => "kilo",
        AgentKind::V8agent => "v8agent",
    }
}

#[allow(dead_code)] // Task 7 用到后删掉
fn agent_from_str(s: &str) -> AgentKind {
    match s {
        "claude" => AgentKind::Claude,
        "codebuddy" => AgentKind::Codebuddy,
        "opencode" => AgentKind::Opencode,
        "codex" => AgentKind::Codex,
        "qoder" => AgentKind::Qoder,
        "kilo" => AgentKind::Kilo,
        "v8agent" => AgentKind::V8agent,
        _ => AgentKind::Unknown,
    }
}

pub struct TranscriptStore {
    conn: Mutex<Connection>,
}

impl TranscriptStore {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("建库目录")?;
        }
        let conn = Connection::open(path).context("打开 SQLite")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS conversations (
                conversation_id TEXT PRIMARY KEY,
                agent_kind TEXT NOT NULL,
                dir TEXT NOT NULL,
                file_path TEXT NOT NULL,
                title TEXT,
                first_ts INTEGER NOT NULL,
                last_ts INTEGER NOT NULL,
                turn_count INTEGER NOT NULL DEFAULT 0,
                parsed_offset INTEGER NOT NULL DEFAULT 0,
                file_size_at_parse INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_conversations_dir ON conversations(dir);
            CREATE TABLE IF NOT EXISTS conversation_turns (
                conversation_id TEXT NOT NULL,
                turn_index INTEGER NOT NULL,
                message_key TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                tools_summary TEXT NOT NULL DEFAULT '[]',
                thinking INTEGER NOT NULL DEFAULT 0,
                ts INTEGER,
                tool_calls INTEGER NOT NULL DEFAULT 0,
                mutating_tool_calls INTEGER NOT NULL DEFAULT 0,
                files_touched TEXT NOT NULL DEFAULT '[]',
                tokens_in INTEGER NOT NULL DEFAULT 0,
                tokens_out INTEGER NOT NULL DEFAULT 0,
                tokens_cache_read INTEGER NOT NULL DEFAULT 0,
                tokens_cache_write INTEGER NOT NULL DEFAULT 0,
                raw_json TEXT NOT NULL,
                PRIMARY KEY (conversation_id, message_key)
            );
            CREATE INDEX IF NOT EXISTS idx_turns_order
                ON conversation_turns(conversation_id, turn_index);",
        )
        .context("建表")?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// 摄取(或增量续摄取)一份 transcript 文件。`conversation_id` 从
    /// 文件名(不含扩展名)派生——这是磁盘上天然唯一的会话标识,不依赖
    /// dozerd 自己的 PTY session id(两者是不同的 id 空间:后者只在
    /// agent 活着时存在,回填历史文件时根本没有)。
    pub fn ingest_session(&self, agent: AgentKind, file_path: &Path) -> Result<()> {
        let conversation_id = file_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        if conversation_id.is_empty() {
            anyhow::bail!("无法从文件名派生 conversation_id: {}", file_path.display());
        }
        let dir = file_path
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let file_path_str = file_path.to_string_lossy().into_owned();

        let file_size = std::fs::metadata(file_path)
            .with_context(|| format!("读取文件元信息失败: {}", file_path.display()))?
            .len();

        let mut conn = self.conn.lock().expect("db lock");
        let existing: Option<(u64, Option<u64>, Option<String>)> = conn
            .query_row(
                "SELECT parsed_offset, first_ts, title FROM conversations WHERE conversation_id = ?1",
                [&conversation_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let (mut parsed_offset, existing_first_ts, existing_title) =
            existing.unwrap_or((0, None, None));
        if file_size < parsed_offset {
            // 文件被截断/重写:只回退读取位置,不删除已入库的行(spec
            // "不做孤儿行清理")。
            parsed_offset = 0;
        }

        let mut raw = std::fs::File::open(file_path)
            .with_context(|| format!("打开文件失败: {}", file_path.display()))?;
        raw.seek(SeekFrom::Start(parsed_offset))?;
        let mut buf = String::new();
        raw.read_to_string(&mut buf)
            .with_context(|| format!("读取增量内容失败: {}", file_path.display()))?;
        let consumable = parse::last_complete_line_boundary(&buf);
        if consumable == 0 {
            // 没有新的完整行(半行未写完/没有新增内容),什么都不做。
            return Ok(());
        }
        let chunk = &buf[..consumable];

        let starting_turn_index: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(turn_index), -1) + 1 FROM conversation_turns
                 WHERE conversation_id = ?1",
                [&conversation_id],
                |row| row.get(0),
            )
            .unwrap_or(0);

        let turns = parse::parse_chunk(agent, chunk, &conversation_id, starting_turn_index);

        let tx = conn.transaction()?;
        let mut first_human_content: Option<String> = None;
        for (turn_index, t) in (starting_turn_index..).zip(&turns) {
            if first_human_content.is_none() && t.role == "human" {
                first_human_content = Some(t.content.clone());
            }
            tx.execute(
                "INSERT INTO conversation_turns
                 (conversation_id, turn_index, message_key, role, content, tools_summary,
                  thinking, ts, tool_calls, mutating_tool_calls, files_touched,
                  tokens_in, tokens_out, tokens_cache_read, tokens_cache_write, raw_json)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)
                 ON CONFLICT(conversation_id, message_key) DO UPDATE SET
                    turn_index=excluded.turn_index, role=excluded.role,
                    content=excluded.content, tools_summary=excluded.tools_summary,
                    thinking=excluded.thinking, ts=excluded.ts,
                    tool_calls=excluded.tool_calls,
                    mutating_tool_calls=excluded.mutating_tool_calls,
                    files_touched=excluded.files_touched,
                    tokens_in=excluded.tokens_in, tokens_out=excluded.tokens_out,
                    tokens_cache_read=excluded.tokens_cache_read,
                    tokens_cache_write=excluded.tokens_cache_write,
                    raw_json=excluded.raw_json",
                params![
                    conversation_id,
                    turn_index,
                    t.message_key,
                    t.role,
                    t.content,
                    serde_json::to_string(&t.tools_summary).unwrap_or_else(|_| "[]".into()),
                    t.thinking as i64,
                    t.ts,
                    t.tool_calls,
                    t.mutating_tool_calls,
                    serde_json::to_string(&t.files_touched).unwrap_or_else(|_| "[]".into()),
                    t.tokens_in,
                    t.tokens_out,
                    t.tokens_cache_read,
                    t.tokens_cache_write,
                    t.raw_json,
                ],
            )?;
        }

        let turn_count: u32 = tx.query_row(
            "SELECT COUNT(*) FROM conversation_turns WHERE conversation_id = ?1",
            [&conversation_id],
            |row| row.get(0),
        )?;
        let last_ts_from_turns: Option<u64> = turns.iter().filter_map(|t| t.ts).max();
        let now = now_ms();
        let new_parsed_offset = parsed_offset + consumable as u64;
        let title = existing_title.or(first_human_content.map(|c| c.chars().take(80).collect()));
        let first_ts = existing_first_ts.unwrap_or(now);
        tx.execute(
            "INSERT INTO conversations
             (conversation_id, agent_kind, dir, file_path, title, first_ts, last_ts,
              turn_count, parsed_offset, file_size_at_parse)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
             ON CONFLICT(conversation_id) DO UPDATE SET
                dir=excluded.dir, file_path=excluded.file_path,
                title=COALESCE(conversations.title, excluded.title),
                last_ts=excluded.last_ts, turn_count=excluded.turn_count,
                parsed_offset=excluded.parsed_offset,
                file_size_at_parse=excluded.file_size_at_parse",
            params![
                conversation_id,
                agent_to_str(agent),
                dir,
                file_path_str,
                title,
                first_ts,
                last_ts_from_turns.unwrap_or(now),
                turn_count,
                new_parsed_offset,
                file_size,
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// 临时最小实现,供本 Task 测试用;Task 8 会替换成带 keyset 分页的
    /// 完整版本(签名不变,调用方不受影响)。
    pub fn get_conversation_turns(
        &self,
        conversation_id: &str,
        _after_turn_index: i64,
        _limit: u32,
    ) -> Result<Vec<dozer_core::protocol::TurnRecord>> {
        let conn = self.conn.lock().expect("db lock");
        let mut stmt = conn.prepare(
            "SELECT turn_index, role, content, tools_summary, thinking, ts
             FROM conversation_turns WHERE conversation_id = ?1 ORDER BY turn_index ASC",
        )?;
        let rows = stmt.query_map([conversation_id], |row| {
            let tools_json: String = row.get(3)?;
            Ok(dozer_core::protocol::TurnRecord {
                turn_index: row.get(0)?,
                role: row.get(1)?,
                content: row.get(2)?,
                tools_summary: serde_json::from_str(&tools_json).unwrap_or_default(),
                thinking: row.get::<_, i64>(4)? != 0,
                ts: row.get(5)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(dir: &Path, name: &str, content: &str) -> std::path::PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, content).unwrap();
        p
    }

    #[test]
    fn ingest_incremental_then_append() {
        let tmp = tempfile::tempdir().unwrap();
        let store = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
        let file = fixture(
            tmp.path(),
            "s1.jsonl",
            "{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"role\":\"user\",\"content\":\"第一句\"}}\n",
        );

        store.ingest_session(AgentKind::Claude, &file).unwrap();
        let turns = store.get_conversation_turns("s1", -1, 100).unwrap();
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].content, "第一句");

        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&file)
            .unwrap();
        use std::io::Write;
        writeln!(
            f,
            "{{\"type\":\"assistant\",\"uuid\":\"u2\",\"message\":{{\"role\":\"assistant\",\"content\":[{{\"type\":\"text\",\"text\":\"回复\"}}]}}}}"
        )
        .unwrap();
        drop(f);

        store.ingest_session(AgentKind::Claude, &file).unwrap();
        let turns = store.get_conversation_turns("s1", -1, 100).unwrap();
        assert_eq!(turns.len(), 2, "增量摄取应该只新增第二条,不重复第一条");
        assert_eq!(turns[1].content, "回复");
    }

    #[test]
    fn half_written_line_is_not_consumed_until_completed() {
        let tmp = tempfile::tempdir().unwrap();
        let store = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
        let file = fixture(tmp.path(), "s1.jsonl", "{\"type\":\"user\",\"uuid\":\"u1\"");
        store.ingest_session(AgentKind::Claude, &file).unwrap();
        assert!(
            store
                .get_conversation_turns("s1", -1, 100)
                .unwrap()
                .is_empty()
        );

        std::fs::write(
            &file,
            "{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"role\":\"user\",\"content\":\"补完了\"}}\n",
        )
        .unwrap();
        store.ingest_session(AgentKind::Claude, &file).unwrap();
        let turns = store.get_conversation_turns("s1", -1, 100).unwrap();
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].content, "补完了");
    }

    #[test]
    fn truncated_file_resets_cursor_and_upserts_without_duplicates() {
        let tmp = tempfile::tempdir().unwrap();
        let store = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
        let file = fixture(
            tmp.path(),
            "s1.jsonl",
            concat!(
                "{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"role\":\"user\",\"content\":\"一\"}}\n",
                "{\"type\":\"user\",\"uuid\":\"u2\",\"message\":{\"role\":\"user\",\"content\":\"二\"}}\n",
            ),
        );
        store.ingest_session(AgentKind::Claude, &file).unwrap();
        assert_eq!(
            store.get_conversation_turns("s1", -1, 100).unwrap().len(),
            2
        );

        // 截断成更短的内容(模拟文件被重写)。
        std::fs::write(
            &file,
            "{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"role\":\"user\",\"content\":\"一(改过)\"}}\n",
        )
        .unwrap();
        store.ingest_session(AgentKind::Claude, &file).unwrap();
        let turns = store.get_conversation_turns("s1", -1, 100).unwrap();
        // u1 被 upsert 覆盖成新内容;u2 是"孤儿行"——按 spec"不做孤儿行
        // 清理"策略保留,不删除。
        let u1 = turns.iter().find(|t| t.content.contains("一(改过)"));
        assert!(u1.is_some(), "截断后重新解析应该覆盖 u1 的内容");
    }
}
