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
        AgentKind::Kilo => "kilo",
        AgentKind::V8agent => "v8agent",
    }
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
                is_error INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (conversation_id, message_key)
            );
            CREATE INDEX IF NOT EXISTS idx_turns_order
                ON conversation_turns(conversation_id, turn_index);",
        )
        .context("建表")?;
        // 老库(建表时还没有 is_error 列)迁移：CREATE TABLE IF NOT EXISTS
        // 对已存在的表不生效，新列需要单独补。SQLite 的
        // ALTER TABLE ADD COLUMN 没有 IF NOT EXISTS 语法(老版本不支持)，
        // 靠 PRAGMA table_info 先查有没有再决定要不要补。
        let has_is_error: bool = conn
            .prepare(
                "SELECT 1 FROM pragma_table_info('conversation_turns') WHERE name = 'is_error'",
            )?
            .exists([])?;
        if !has_is_error {
            conn.execute(
                "ALTER TABLE conversation_turns ADD COLUMN is_error INTEGER NOT NULL DEFAULT 0",
                [],
            )
            .context("迁移 is_error 列")?;
        }
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

    /// keyset 分页:返回 `turn_index > after_turn_index` 的前 `limit` 条。
    /// `after_turn_index` 传 `-1` 表示从第一条开始。
    pub fn get_conversation_turns(
        &self,
        conversation_id: &str,
        after_turn_index: i64,
        limit: u32,
    ) -> Result<Vec<dozer_core::protocol::TurnRecord>> {
        let conn = self.conn.lock().expect("db lock");
        let mut stmt = conn.prepare(
            "SELECT turn_index, role, content, tools_summary, thinking, ts
             FROM conversation_turns
             WHERE conversation_id = ?1 AND turn_index > ?2
             ORDER BY turn_index ASC LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![conversation_id, after_turn_index, limit], |row| {
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

    /// 生产入口,内部用 `dozer_core::agent_paths::home_dir()`。
    pub fn list_conversations(
        &self,
        cwd: &str,
        agent: Option<AgentKind>,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<dozer_core::protocol::ConversationSummary>> {
        self.list_conversations_in(
            &dozer_core::agent_paths::home_dir(),
            cwd,
            agent,
            limit,
            offset,
        )
    }

    /// `home` 显式传入版本,测试用。
    pub fn list_conversations_in(
        &self,
        home: &Path,
        cwd: &str,
        agent: Option<AgentKind>,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<dozer_core::protocol::ConversationSummary>> {
        use dozer_core::agent_paths::{
            claude_project_dir_in, codebuddy_project_dir_in, opencode_project_dir_in,
        };
        let cwd_path = Path::new(cwd);
        let candidate_dirs: Vec<(AgentKind, String)> = match agent {
            Some(a) => {
                let dir = match a {
                    AgentKind::Claude => claude_project_dir_in(home, cwd_path),
                    AgentKind::Codebuddy => codebuddy_project_dir_in(home, cwd_path),
                    AgentKind::Opencode => opencode_project_dir_in(home, cwd_path),
                    _ => return Ok(Vec::new()),
                };
                vec![(a, dir.to_string_lossy().into_owned())]
            }
            None => vec![
                (
                    AgentKind::Claude,
                    claude_project_dir_in(home, cwd_path)
                        .to_string_lossy()
                        .into_owned(),
                ),
                (
                    AgentKind::Codebuddy,
                    codebuddy_project_dir_in(home, cwd_path)
                        .to_string_lossy()
                        .into_owned(),
                ),
                (
                    AgentKind::Opencode,
                    opencode_project_dir_in(home, cwd_path)
                        .to_string_lossy()
                        .into_owned(),
                ),
            ],
        };

        let conn = self.conn.lock().expect("db lock");
        let mut out = Vec::new();
        for (_, dir) in &candidate_dirs {
            let mut stmt = conn.prepare(
                "SELECT conversation_id, agent_kind, file_path, title, first_ts, last_ts, turn_count
                 FROM conversations WHERE dir = ?1 ORDER BY last_ts DESC",
            )?;
            let rows = stmt.query_map([dir], |row| {
                let agent_kind: String = row.get(1)?;
                Ok(dozer_core::protocol::ConversationSummary {
                    conversation_id: row.get(0)?,
                    agent: agent_from_str(&agent_kind),
                    file_path: row.get(2)?,
                    title: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                    first_ts: row.get(4)?,
                    last_ts: row.get(5)?,
                    turn_count: row.get(6)?,
                })
            })?;
            for r in rows {
                out.push(r?);
            }
        }
        out.sort_by_key(|c| std::cmp::Reverse(c.last_ts));
        let start = (offset as usize).min(out.len());
        let end = (start + limit as usize).min(out.len());
        Ok(out[start..end].to_vec())
    }

    /// 用量聚合,`cwd` → 该项目在各 agent 下的存储目录;`since_ts` 非空时
    /// 只统计 `last_ts >= since_ts` 的会话。
    pub fn get_usage_summary(
        &self,
        cwd: &str,
        since_ts: Option<u64>,
    ) -> Result<
        Vec<(
            dozer_core::protocol::ConversationSummary,
            dozer_core::protocol::UsagePayload,
        )>,
    > {
        self.get_usage_summary_in(&dozer_core::agent_paths::home_dir(), cwd, since_ts)
    }

    /// `home` 显式传入版本,测试用。
    pub fn get_usage_summary_in(
        &self,
        home: &Path,
        cwd: &str,
        since_ts: Option<u64>,
    ) -> Result<
        Vec<(
            dozer_core::protocol::ConversationSummary,
            dozer_core::protocol::UsagePayload,
        )>,
    > {
        let conversations = self.list_conversations_in(home, cwd, None, u32::MAX, 0)?;
        let conn = self.conn.lock().expect("db lock");
        let mut out = Vec::new();
        for c in conversations {
            if since_ts.is_some_and(|since| c.last_ts < since) {
                continue;
            }
            let mut stmt = conn.prepare(
                "WITH owners AS (
                    SELECT t.message_key,
                           MIN(printf('%020lld|', c2.first_ts) || c2.conversation_id) AS owner_key
                    FROM conversation_turns t
                    JOIN conversations c2 ON c2.conversation_id = t.conversation_id
                    GROUP BY t.message_key
                 )
                 SELECT t.role, t.tool_calls, t.mutating_tool_calls, t.files_touched,
                        t.tokens_in, t.tokens_out, t.tokens_cache_read, t.tokens_cache_write
                 FROM conversation_turns t
                 JOIN conversations c1 ON c1.conversation_id = t.conversation_id
                 JOIN owners o ON o.message_key = t.message_key
                 WHERE t.conversation_id = ?1
                   AND (printf('%020lld|', c1.first_ts) || c1.conversation_id) = o.owner_key",
            )?;
            let rows = stmt.query_map([&c.conversation_id], |row| {
                let files_json: String = row.get(3)?;
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, u32>(1)?,
                    row.get::<_, u32>(2)?,
                    files_json,
                    row.get::<_, u64>(4)?,
                    row.get::<_, u64>(5)?,
                    row.get::<_, u64>(6)?,
                    row.get::<_, u64>(7)?,
                ))
            })?;
            let mut payload = dozer_core::protocol::UsagePayload::default();
            for r in rows {
                let (_role, tool_calls, mutating, files_json, tin, tout, tcr, tcw) = r?;
                payload.turns += 1;
                payload.tool_calls += tool_calls;
                payload.mutating_tool_calls += mutating;
                payload.tokens_in += tin;
                payload.tokens_out += tout;
                payload.tokens_cache_read += tcr;
                payload.tokens_cache_write += tcw;
                if let Ok(files) = serde_json::from_str::<Vec<String>>(&files_json) {
                    payload.files_touched.extend(files);
                }
            }
            out.push((c, payload));
        }
        Ok(out)
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
    fn open_on_pre_existing_db_without_is_error_column_adds_it() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("old.db");
        // 模拟老库：手写不带 is_error 列的旧表结构。
        {
            let conn = rusqlite::Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE conversations (
                    conversation_id TEXT PRIMARY KEY, agent_kind TEXT NOT NULL,
                    dir TEXT NOT NULL, file_path TEXT NOT NULL, title TEXT,
                    first_ts INTEGER NOT NULL, last_ts INTEGER NOT NULL,
                    turn_count INTEGER NOT NULL DEFAULT 0,
                    parsed_offset INTEGER NOT NULL DEFAULT 0,
                    file_size_at_parse INTEGER NOT NULL DEFAULT 0
                );
                CREATE TABLE conversation_turns (
                    conversation_id TEXT NOT NULL, turn_index INTEGER NOT NULL,
                    message_key TEXT NOT NULL, role TEXT NOT NULL, content TEXT NOT NULL,
                    tools_summary TEXT NOT NULL DEFAULT '[]', thinking INTEGER NOT NULL DEFAULT 0,
                    ts INTEGER, tool_calls INTEGER NOT NULL DEFAULT 0,
                    mutating_tool_calls INTEGER NOT NULL DEFAULT 0,
                    files_touched TEXT NOT NULL DEFAULT '[]',
                    tokens_in INTEGER NOT NULL DEFAULT 0, tokens_out INTEGER NOT NULL DEFAULT 0,
                    tokens_cache_read INTEGER NOT NULL DEFAULT 0,
                    tokens_cache_write INTEGER NOT NULL DEFAULT 0, raw_json TEXT NOT NULL,
                    PRIMARY KEY (conversation_id, message_key)
                );",
            )
            .unwrap();
        }
        // open() 应该在老库上补出 is_error 列，不报错。
        let store = TranscriptStore::open(&db_path).unwrap();
        let conn = store.conn.lock().unwrap();
        let has_col: bool = conn
            .prepare("SELECT is_error FROM conversation_turns LIMIT 0")
            .is_ok();
        assert!(has_col, "老库 open() 后应该已经补上 is_error 列");
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

    #[test]
    fn list_conversations_merges_three_agents_sorted_by_last_ts() {
        let tmp = tempfile::tempdir().unwrap();
        let store = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
        let home = tempfile::tempdir().unwrap();
        let cwd = std::path::Path::new("/proj");
        let claude_dir = dozer_core::agent_paths::claude_project_dir_in(home.path(), cwd);
        let codebuddy_dir = dozer_core::agent_paths::codebuddy_project_dir_in(home.path(), cwd);
        std::fs::create_dir_all(&claude_dir).unwrap();
        std::fs::create_dir_all(&codebuddy_dir).unwrap();
        let f1 = fixture(
            &claude_dir,
            "a.jsonl",
            "{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"role\":\"user\",\"content\":\"claude 对话\"}}\n",
        );
        let f2 = fixture(
            &codebuddy_dir,
            "b.jsonl",
            "{\"id\":\"m1\",\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"codebuddy 对话\"}]}\n",
        );
        store.ingest_session(AgentKind::Claude, &f1).unwrap();
        store.ingest_session(AgentKind::Codebuddy, &f2).unwrap();

        let list = store
            .list_conversations_in(home.path(), "/proj", None, 10, 0)
            .unwrap();
        assert_eq!(list.len(), 2);
        assert!(list.iter().any(|c| c.agent == AgentKind::Claude));
        assert!(list.iter().any(|c| c.agent == AgentKind::Codebuddy));

        let claude_only = store
            .list_conversations_in(home.path(), "/proj", Some(AgentKind::Claude), 10, 0)
            .unwrap();
        assert_eq!(claude_only.len(), 1);
        assert_eq!(claude_only[0].agent, AgentKind::Claude);
    }

    #[test]
    fn list_conversations_respects_limit_and_offset() {
        let tmp = tempfile::tempdir().unwrap();
        let store = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
        let home = tempfile::tempdir().unwrap();
        let cwd = std::path::Path::new("/proj");
        let claude_dir = dozer_core::agent_paths::claude_project_dir_in(home.path(), cwd);
        std::fs::create_dir_all(&claude_dir).unwrap();
        for i in 0..3 {
            let f = fixture(
                &claude_dir,
                &format!("s{i}.jsonl"),
                &format!(
                    "{{\"type\":\"user\",\"uuid\":\"u{i}\",\"message\":{{\"role\":\"user\",\"content\":\"第{i}条\"}}}}\n"
                ),
            );
            store.ingest_session(AgentKind::Claude, &f).unwrap();
        }
        let page = store
            .list_conversations_in(home.path(), "/proj", None, 2, 0)
            .unwrap();
        assert_eq!(page.len(), 2);
    }

    #[test]
    fn get_conversation_turns_paginates_by_keyset() {
        let tmp = tempfile::tempdir().unwrap();
        let store = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
        let mut jsonl = String::new();
        for i in 0..5 {
            jsonl.push_str(&format!(
                "{{\"type\":\"user\",\"uuid\":\"u{i}\",\"message\":{{\"role\":\"user\",\"content\":\"第{i}条\"}}}}\n"
            ));
        }
        let file = fixture(tmp.path(), "s1.jsonl", &jsonl);
        store.ingest_session(AgentKind::Claude, &file).unwrap();

        let first_page = store.get_conversation_turns("s1", -1, 2).unwrap();
        assert_eq!(first_page.len(), 2);
        assert_eq!(first_page[0].turn_index, 0);
        assert_eq!(first_page[1].turn_index, 1);

        let last_seen = first_page[1].turn_index;
        let second_page = store.get_conversation_turns("s1", last_seen, 2).unwrap();
        assert_eq!(second_page.len(), 2);
        assert_eq!(second_page[0].turn_index, 2);
        assert_eq!(second_page[1].turn_index, 3);
    }

    #[test]
    fn get_usage_summary_dedupes_forked_message_key_by_earliest_owner() {
        let tmp = tempfile::tempdir().unwrap();
        let store = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
        let home = tempfile::tempdir().unwrap();
        let cwd = std::path::Path::new("/proj");
        let dir = dozer_core::agent_paths::claude_project_dir_in(home.path(), cwd);
        std::fs::create_dir_all(&dir).unwrap();

        let original = fixture(
            &dir,
            "original.jsonl",
            "{\"type\":\"assistant\",\"uuid\":\"shared-1\",\"message\":{\"usage\":{\"input_tokens\":10,\"output_tokens\":5},\"content\":[]}}\n",
        );
        store.ingest_session(AgentKind::Claude, &original).unwrap();

        std::thread::sleep(std::time::Duration::from_millis(5));
        let forked = fixture(
            &dir,
            "forked.jsonl",
            concat!(
                "{\"type\":\"assistant\",\"uuid\":\"shared-1\",\"message\":{\"usage\":{\"input_tokens\":10,\"output_tokens\":5},\"content\":[]}}\n",
                "{\"type\":\"assistant\",\"uuid\":\"new-1\",\"message\":{\"usage\":{\"input_tokens\":3,\"output_tokens\":1},\"content\":[]}}\n",
            ),
        );
        store.ingest_session(AgentKind::Claude, &forked).unwrap();

        let rows = store
            .get_usage_summary_in(home.path(), "/proj", None)
            .unwrap();
        assert_eq!(rows.len(), 2);
        let original_row = rows
            .iter()
            .find(|(c, _)| c.conversation_id == "original")
            .unwrap();
        let forked_row = rows
            .iter()
            .find(|(c, _)| c.conversation_id == "forked")
            .unwrap();
        assert_eq!(original_row.1.tokens_in, 10, "原始会话拥有 shared-1 的用量");
        assert_eq!(
            forked_row.1.tokens_in, 3,
            "forked 会话里复制来的 shared-1 不重复计入,只有自己新增的 new-1 计入"
        );
    }

    #[test]
    fn get_usage_summary_breaks_first_ts_ties_deterministically() {
        let tmp = tempfile::tempdir().unwrap();
        let store = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
        let home = tempfile::tempdir().unwrap();
        let cwd = std::path::Path::new("/proj");
        let dir = dozer_core::agent_paths::claude_project_dir_in(home.path(), cwd);
        std::fs::create_dir_all(&dir).unwrap();

        let a = fixture(
            &dir,
            "a.jsonl",
            "{\"type\":\"assistant\",\"uuid\":\"shared-tie\",\"message\":{\"usage\":{\"input_tokens\":10,\"output_tokens\":0},\"content\":[]}}\n",
        );
        let b = fixture(
            &dir,
            "b.jsonl",
            "{\"type\":\"assistant\",\"uuid\":\"shared-tie\",\"message\":{\"usage\":{\"input_tokens\":10,\"output_tokens\":0},\"content\":[]}}\n",
        );
        store.ingest_session(AgentKind::Claude, &a).unwrap();
        store.ingest_session(AgentKind::Claude, &b).unwrap();

        let rows = store
            .get_usage_summary_in(home.path(), "/proj", None)
            .unwrap();
        let total_tokens_in: u64 = rows.iter().map(|(_, u)| u.tokens_in).sum();
        assert_eq!(
            total_tokens_in, 10,
            "无论 first_ts 是否平局,shared-tie 只能被恰好一个会话计入一次"
        );
    }
}
