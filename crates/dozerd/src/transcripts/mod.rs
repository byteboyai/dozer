//! agent 对话/用量摄取管线(spec 2026-08-20)。

pub mod parse;
pub mod scan;

use anyhow::{Context, Result};
use dozer_core::protocol::AgentKind;
use rusqlite::{Connection, OptionalExtension, params};
use std::io::{BufRead, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Mutex;

dozer_core::scope!(LOG, module, "transcripts");

/// 从 transcript 文件路径派生 `conversation_id`(即文件名去掉扩展名)。
/// `ingest_session` 与 `dozerd::server` 的 `RecordSessionSummary`/
/// `CloseWithSummary` 处理器共用同一份派生逻辑,避免各自写一份、两处
/// 拼写分叉(2026-08-27 修正)。
pub fn conversation_id_for_path(file_path: &Path) -> Option<String> {
    file_path
        .file_stem()
        .and_then(|s| s.to_str())
        .map(String::from)
}

/// 读 Codex rollout 文件头部的 `session_meta` 行,取出这个会话的项目 cwd。
/// Codex 不按项目建目录(见 `dozer_core::agent_paths::codex_sessions_dir_in`
/// 注释),项目归属只有这一个来源,所以"摄取时落库"(`ingest_session`)和
/// "按项目发现文件"(`scan::discover_project_transcript_files_in`)两条路都
/// 得靠它。
///
/// 只读文件头几行(`session_meta` 实测恒为第一行,留点余量),不为了拿 cwd 把
/// 十几 MB 的 rollout 整份读一遍;也刻意**不**走 `parsed_offset` 增量窗口——
/// 续摄取时 `session_meta` 早就不在窗口里了。
pub(crate) fn codex_session_cwd(file_path: &Path) -> Option<String> {
    const MAX_HEAD_LINES: usize = 8;
    let file = std::fs::File::open(file_path).ok()?;
    for line in std::io::BufReader::new(file).lines().take(MAX_HEAD_LINES) {
        let Ok(line) = line else {
            return None;
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
            continue;
        };
        if v.get("type").and_then(|t| t.as_str()) == Some("session_meta") {
            return v
                .get("payload")
                .and_then(|p| p.get("cwd"))
                .and_then(|c| c.as_str())
                .map(str::to_string);
        }
    }
    None
}

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
        AgentKind::Goose => "goose",
        AgentKind::Aider => "aider",
        AgentKind::V8agent => "v8agent",
    }
}

fn agent_from_str(s: &str) -> AgentKind {
    match s {
        "claude" => AgentKind::Claude,
        "codebuddy" => AgentKind::Codebuddy,
        "opencode" => AgentKind::Opencode,
        "codex" => AgentKind::Codex,
        "goose" => AgentKind::Goose,
        "aider" => AgentKind::Aider,
        "v8agent" => AgentKind::V8agent,
        _ => AgentKind::Unknown,
    }
}

/// `list_conversations_in` 两个查询分支(按 `dir` 的四家 / 按 `cwd` 的 Codex)
/// 共用的 SELECT 列表与行映射——列顺序两边必须一致,合成一份常量避免漂移。
const CONVERSATION_SUMMARY_COLUMNS: &str =
    "conversation_id, agent_kind, file_path, title, first_ts, last_ts, turn_count";

fn conversation_summary_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<dozer_core::protocol::ConversationSummary> {
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
}

pub struct TranscriptStore {
    conn: Mutex<Connection>,
}

impl TranscriptStore {
    /// Import conversations from Codex's paginated SQLite history (Codex 0.16x+).
    ///
    /// Recent Codex builds keep `~/.codex/sessions` empty and store thread metadata
    /// and rendered history in `state_N.sqlite` / `thread_history_N.sqlite`.
    /// Keeping this reader here (rather than teaching the UI about Codex-private
    /// storage) preserves the existing conversations/usage query contract.
    pub fn ingest_codex_sqlite(&self, cwd: Option<&str>) -> Result<u32> {
        self.ingest_codex_sqlite_in(&dozer_core::agent_paths::home_dir(), cwd)
    }

    fn ingest_codex_sqlite_in(&self, home: &Path, cwd: Option<&str>) -> Result<u32> {
        use rusqlite::OpenFlags;

        fn newest_numbered_db(root: &Path, prefix: &str) -> Option<std::path::PathBuf> {
            std::fs::read_dir(root)
                .ok()?
                .filter_map(|entry| entry.ok().map(|e| e.path()))
                .filter_map(|path| {
                    let name = path.file_name()?.to_str()?;
                    let version = name
                        .strip_prefix(prefix)?
                        .strip_suffix(".sqlite")?
                        .parse::<u64>()
                        .ok()?;
                    Some((version, path))
                })
                .max_by_key(|(version, _)| *version)
                .map(|(_, path)| path)
        }

        let codex = home.join(".codex");
        let Some(state_path) = newest_numbered_db(&codex, "state_") else {
            return Ok(0);
        };
        let Some(history_path) = newest_numbered_db(&codex, "thread_history_") else {
            return Ok(0);
        };
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let state = Connection::open_with_flags(&state_path, flags)
            .with_context(|| format!("打开 Codex 状态库失败: {}", state_path.display()))?;
        let history = Connection::open_with_flags(&history_path, flags)
            .with_context(|| format!("打开 Codex 历史库失败: {}", history_path.display()))?;

        let sql = if cwd.is_some() {
            "SELECT id, cwd, title, created_at, updated_at, tokens_used, rollout_path
             FROM threads WHERE cwd = ?1 AND archived = 0 ORDER BY updated_at"
        } else {
            "SELECT id, cwd, title, created_at, updated_at, tokens_used, rollout_path
             FROM threads WHERE archived = 0 ORDER BY updated_at"
        };
        let mut stmt = state.prepare(sql)?;
        let map_thread = |row: &rusqlite::Row<'_>| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, u64>(3)?,
                row.get::<_, u64>(4)?,
                row.get::<_, u64>(5)?,
                row.get::<_, String>(6)?,
            ))
        };
        let threads: Vec<_> = if let Some(cwd) = cwd {
            stmt.query_map([cwd], map_thread)?
                .collect::<rusqlite::Result<_>>()?
        } else {
            stmt.query_map([], map_thread)?
                .collect::<rusqlite::Result<_>>()?
        };

        let mut imported = 0u32;
        for (id, thread_cwd, title, created, updated, tokens_used, rollout_path) in threads {
            let mut item_stmt = history.prepare(
                "SELECT item_id, created_at_ms, item_type, item_json
                 FROM thread_items WHERE thread_id = ?1 ORDER BY rollout_ordinal",
            )?;
            let items = item_stmt.query_map([&id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, u64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?;
            let mut turns = Vec::new();
            for item in items {
                let (item_id, ts, item_type, raw_json) = item?;
                let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw_json) else {
                    continue;
                };
                let (role, content, tool_calls, mutating, files) = match item_type.as_str() {
                    "userMessage" => {
                        let text = value
                            .get("content")
                            .and_then(|v| v.as_array())
                            .into_iter()
                            .flatten()
                            .filter_map(|b| b.get("text").and_then(|v| v.as_str()))
                            .collect::<Vec<_>>()
                            .join("\n");
                        if text.trim().is_empty() {
                            continue;
                        }
                        ("human", text, 0, 0, Vec::new())
                    }
                    "agentMessage" => {
                        let text = value.get("text").and_then(|v| v.as_str()).unwrap_or("");
                        if text.trim().is_empty() {
                            continue;
                        }
                        ("ai", text.to_string(), 0, 0, Vec::new())
                    }
                    "commandExecution" => {
                        let command = value.get("command").and_then(|v| v.as_str()).unwrap_or("");
                        ("ai", String::new(), 1, 0, Vec::from([command.to_string()]))
                    }
                    "fileChange" => {
                        let files = value
                            .get("changes")
                            .and_then(|v| v.as_array())
                            .into_iter()
                            .flatten()
                            .filter_map(|change| change.get("path").and_then(|v| v.as_str()))
                            .map(str::to_string)
                            .collect::<Vec<_>>();
                        ("ai", String::new(), 1, 1, files)
                    }
                    _ => continue,
                };
                turns.push(parse::ParsedTurn {
                    message_key: item_id,
                    role: role.into(),
                    content,
                    tool_calls,
                    mutating_tool_calls: mutating,
                    files_touched: files,
                    ts: Some(ts),
                    raw_json,
                    ..Default::default()
                });
            }
            // The paginated history DB does not expose the old per-response token
            // breakdown. Preserve Codex's authoritative thread total as one usage
            // row so totals continue to work without inventing an input/output split.
            turns.push(parse::ParsedTurn {
                message_key: format!("codex-sqlite-usage:{id}"),
                role: "token_usage".into(),
                tokens_in: tokens_used,
                ts: Some(updated.saturating_mul(1000)),
                raw_json: "{\"type\":\"codex_sqlite_usage\"}".into(),
                ..Default::default()
            });

            let mut conn = self.conn.lock().expect("db lock");
            let tx = conn.transaction()?;
            tx.execute(
                "DELETE FROM conversation_turns WHERE conversation_id = ?1",
                [&id],
            )?;
            for (index, turn) in turns.iter().enumerate() {
                tx.execute(
                    "INSERT INTO conversation_turns
                     (conversation_id, turn_index, message_key, role, content, tools_summary,
                      thinking, ts, tool_calls, mutating_tool_calls, files_touched,
                      tokens_in, tokens_out, tokens_cache_read, tokens_cache_write, raw_json, is_error)
                     VALUES (?1,?2,?3,?4,?5,'[]',0,?6,?7,?8,?9,?10,0,0,0,?11,0)",
                    params![id, index as i64, turn.message_key, turn.role, turn.content,
                        turn.ts, turn.tool_calls, turn.mutating_tool_calls,
                        serde_json::to_string(&turn.files_touched).unwrap_or_else(|_| "[]".into()),
                        turn.tokens_in, turn.raw_json],
                )?;
            }
            tx.execute(
                "INSERT INTO conversations
                 (conversation_id,agent_kind,dir,file_path,title,first_ts,last_ts,turn_count,
                  parsed_offset,file_size_at_parse,cwd)
                 VALUES (?1,'codex','',?2,?3,?4,?5,?6,0,0,?7)
                 ON CONFLICT(conversation_id) DO UPDATE SET agent_kind='codex', file_path=excluded.file_path,
                    title=excluded.title, first_ts=excluded.first_ts, last_ts=excluded.last_ts,
                    turn_count=excluded.turn_count, cwd=excluded.cwd",
                params![id, rollout_path, title, created.saturating_mul(1000),
                    updated.saturating_mul(1000), turns.len() as u32, thread_cwd],
            )?;
            tx.commit()?;
            imported += 1;
        }
        Ok(imported)
    }

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
                file_size_at_parse INTEGER NOT NULL DEFAULT 0,
                cwd TEXT
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
        // Codex 的项目归属列(老库同样要单独补)。其余四家的 `dir` 就是它们
        // 各自的项目存储目录,天然能按项目查;Codex 的目录按日期建
        // (`~/.codex/sessions/YYYY/MM/DD/`,见
        // `dozer_core::agent_paths::codex_sessions_dir_in` 注释),`dir` 里没有
        // 项目信息,只能把 `session_meta.payload.cwd` 单独存一列,查询侧按它
        // 过滤(`list_conversations_in`)。
        let has_cwd: bool = conn
            .prepare("SELECT 1 FROM pragma_table_info('conversations') WHERE name = 'cwd'")?
            .exists([])?;
        if !has_cwd {
            conn.execute("ALTER TABLE conversations ADD COLUMN cwd TEXT", [])
                .context("迁移 cwd 列")?;
        }
        // 索引建在 ALTER 之后:写进上面那段批量 CREATE 里的话,老库(还没补出
        // cwd 列)会直接报 "no such column: cwd"。
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_conversations_cwd ON conversations(cwd)",
        )
        .context("建 cwd 索引")?;
        // 一次性数据修复(user_version 0 → 1):Codex 解析器是 2026-09-21 才
        // 落地的,在那之前 `parse_chunk` 对 Codex 恒返回空 Vec,但 dozerd 已经
        // 按常规流程把线上 Codex 会话整份"消费"过一遍——`parsed_offset` 推到
        // 了接近 EOF,而增量摄取只读 offset 之后的字节,这些会话于是永久停在
        // 0 回合(实测线上库 3 条:11 MB 的文件 unread 只剩 2 KB)。这里把
        // offset 归零、连带删掉旧回合重解析一次。
        //
        // 必须连 `conversation_turns` 一起删,不能只归零 offset:Codex 的
        // message 大多没有 `payload.id`(实测 43 份 rollout:234 条 user 消息
        // 里 213 条没有),`message_key` 走 `fallback_key`=`{会话id}:{序号}`,
        // 而序号从"库里已有回合的最大序号+1"起算——留着旧回合再从头解析,
        // 同一批消息会拿到新序号、UPSERT 撞不上主键,直接翻倍。
        //
        // `file_path <> ''` 是为了放过 `record_task_turns` 建的 headless 任务
        // 占位行(那种行没有真实 transcript 文件,`file_path` 恒为空串,删了
        // 就再也补不回来)。
        let user_version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap_or(0);
        if user_version < 1 {
            conn.execute_batch(
                "DELETE FROM conversation_turns WHERE conversation_id IN (
                     SELECT conversation_id FROM conversations
                     WHERE agent_kind = 'codex' AND file_path <> ''
                 );
                 UPDATE conversations
                    SET parsed_offset = 0, turn_count = 0, title = NULL
                  WHERE agent_kind = 'codex' AND file_path <> '';",
            )
            .context("重置 Codex 摄取进度")?;
            conn.execute_batch("PRAGMA user_version = 1")
                .context("写 user_version")?;
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
        let conversation_id = conversation_id_for_path(file_path).ok_or_else(|| {
            anyhow::anyhow!("无法从文件名派生 conversation_id: {}", file_path.display())
        })?;
        let dir = file_path
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let file_path_str = file_path.to_string_lossy().into_owned();
        // Codex 的项目归属只写在文件头 `session_meta.payload.cwd` 里,单独存一
        // 列供按项目查询;其余四家的 `dir` 本身就是项目存储目录,留 NULL。
        // 放在取 DB 锁之前做——这是一次文件 IO,不该占着锁。
        let cwd = match agent {
            AgentKind::Codex => codex_session_cwd(file_path),
            _ => None,
        };

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
                  tokens_in, tokens_out, tokens_cache_read, tokens_cache_write, raw_json,
                  is_error)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)
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
                    raw_json=excluded.raw_json, is_error=excluded.is_error",
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
                    t.is_error as i64,
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
              turn_count, parsed_offset, file_size_at_parse, cwd)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
             ON CONFLICT(conversation_id) DO UPDATE SET
                dir=excluded.dir, file_path=excluded.file_path,
                title=COALESCE(conversations.title, excluded.title),
                last_ts=excluded.last_ts, turn_count=excluded.turn_count,
                parsed_offset=excluded.parsed_offset,
                file_size_at_parse=excluded.file_size_at_parse,
                cwd=COALESCE(excluded.cwd, conversations.cwd)",
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
                cwd,
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// headless 任务处理专用:确保 `conversation_id` 对应的 `conversations`
    /// 占位行存在(`file_path` 用空字符串——这个会话没有真实 transcript
    /// 文件,和"扫描磁盘文件"的摄取路径完全独立),再插入一条人类回合 +
    /// 一条 AI 回合。`turn_index` 从该 `conversation_id` 现有最大值 + 1 起
    /// 连续分配,`message_key` 用 `"{conversation_id}:{turn_index}"` 保证
    /// 主键不冲突。
    pub fn record_task_turns(
        &self,
        conversation_id: &str,
        agent: AgentKind,
        project_dir: &str,
        task_title: &str,
        human_content: &str,
        ai_content: &str,
    ) -> Result<()> {
        let mut conn = self.conn.lock().expect("db lock");
        let tx = conn.transaction()?;
        let now = now_ms() as i64;
        tx.execute(
            "INSERT INTO conversations
             (conversation_id, agent_kind, dir, file_path, title, first_ts, last_ts,
              turn_count, parsed_offset, file_size_at_parse, cwd)
             VALUES (?1,?2,?3,'',?4,?5,?5,0,0,0,?3)
             ON CONFLICT(conversation_id) DO NOTHING",
            params![
                conversation_id,
                agent_to_str(agent),
                project_dir,
                task_title,
                now
            ],
        )?;
        let starting_turn_index: i64 = tx.query_row(
            "SELECT COALESCE(MAX(turn_index), -1) + 1 FROM conversation_turns
             WHERE conversation_id = ?1",
            [conversation_id],
            |row| row.get(0),
        )?;
        for (offset, (role, content)) in [("human", human_content), ("ai", ai_content)]
            .into_iter()
            .enumerate()
        {
            let turn_index = starting_turn_index + offset as i64;
            let message_key = format!("{conversation_id}:{turn_index}");
            tx.execute(
                "INSERT INTO conversation_turns
                 (conversation_id, turn_index, message_key, role, content, tools_summary,
                  thinking, ts, tool_calls, mutating_tool_calls, files_touched,
                  tokens_in, tokens_out, tokens_cache_read, tokens_cache_write, raw_json,
                  is_error)
                 VALUES (?1,?2,?3,?4,?5,'[]',0,?6,0,0,'[]',0,0,0,0,'{}',0)",
                params![conversation_id, turn_index, message_key, role, content, now],
            )?;
        }
        let turn_count: u32 = tx.query_row(
            "SELECT COUNT(*) FROM conversation_turns WHERE conversation_id = ?1",
            [conversation_id],
            |row| row.get(0),
        )?;
        tx.execute(
            "UPDATE conversations SET last_ts = ?1, turn_count = ?2 WHERE conversation_id = ?3",
            params![now, turn_count, conversation_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// keyset 分页:返回 `turn_index > after_turn_index` 的前 `limit` 条。
    /// `after_turn_index` 传 `-1` 表示从第一条开始。JOIN `conversations`
    /// 拿 `agent_kind` 只为了给 `parse::extract_turn_trace_detail` 挑对
    /// 解析形状——不新增参数(client/protocol 签名都不用改),`raw_json`
    /// 每行都读一次、当场解析,不落新列(见 spec"读时解析"一节)。token 四列
    /// (`tokens_in`/`tokens_out`/`tokens_cache_read`/`tokens_cache_write`)
    /// 直接落库读出,不用读时解析——摄取阶段(`parse_claude_shaped_chunk`)
    /// 已经从 `message.usage` 抽好存了(2026-08-23 起补进 `TurnRecord`,
    /// 供审阅面板"轨迹"统计用)。
    pub fn get_conversation_turns(
        &self,
        conversation_id: &str,
        after_turn_index: i64,
        limit: u32,
    ) -> Result<Vec<dozer_core::protocol::TurnRecord>> {
        let conn = self.conn.lock().expect("db lock");
        let mut stmt = conn.prepare(
            "SELECT t.turn_index, t.role, t.content, t.thinking, t.ts, t.is_error,
                    t.raw_json, c.agent_kind, t.tokens_in, t.tokens_out,
                    t.tokens_cache_read, t.tokens_cache_write
             FROM conversation_turns t
             JOIN conversations c ON c.conversation_id = t.conversation_id
             WHERE t.conversation_id = ?1 AND t.turn_index > ?2
             ORDER BY t.turn_index ASC LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![conversation_id, after_turn_index, limit], |row| {
            let raw_json: String = row.get(6)?;
            let agent_kind: String = row.get(7)?;
            let detail = parse::extract_turn_trace_detail(&raw_json, agent_from_str(&agent_kind));
            Ok(dozer_core::protocol::TurnRecord {
                turn_index: row.get(0)?,
                role: row.get(1)?,
                content: row.get(2)?,
                tool_calls: detail.tool_calls,
                thinking: row.get::<_, i64>(3)? != 0,
                thinking_text: detail.thinking_text,
                ts: row.get(4)?,
                is_error: row.get::<_, i64>(5)? != 0,
                tool_result_call_id: detail.tool_result_call_id,
                tokens_in: row.get(8)?,
                tokens_out: row.get(9)?,
                tokens_cache_read: row.get(10)?,
                tokens_cache_write: row.get(11)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// 分页循环读取一个 conversation 的完整 transcript 快照(spec 2026-09-26
    /// 第 6 节"固定读取快照"):内部按 `get_conversation_turns` 的 keyset 分页
    /// 逐页拉取,直到取完或超过 `max_turns` 上限。上限不是截断语义,而是
    /// "防御性熔断"——正常会话远达不到,一旦达到说明数据异常或调用方没把
    /// 预算切成更小的块,报错而不是静默返回半截。
    pub fn refresh_conversation(&self, conversation_id: &str) -> Result<()> {
        let source: Option<(String, String)> = {
            let conn = self.conn.lock().expect("db lock");
            conn.query_row(
                "SELECT agent_kind,file_path FROM conversations WHERE conversation_id=?1",
                [conversation_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
        };
        if let Some((agent, path)) = source
            && Path::new(&path).is_file()
        {
            self.ingest_session(agent_from_str(&agent), Path::new(&path))?;
        }
        Ok(())
    }

    pub fn get_conversation_turns_all(
        &self,
        conversation_id: &str,
        max_turns: u32,
    ) -> Result<Vec<dozer_core::protocol::TurnRecord>> {
        const PAGE: u32 = 512;
        let mut all: Vec<dozer_core::protocol::TurnRecord> = Vec::new();
        let mut after: i64 = -1;
        loop {
            let page = self.get_conversation_turns(conversation_id, after, PAGE)?;
            if page.is_empty() {
                return Ok(all);
            }
            after = page.last().map(|t| t.turn_index).unwrap_or(after);
            all.extend(page);
            if all.len() as u32 > max_turns {
                anyhow::bail!(
                    "conversation {conversation_id} 回合数超过上限 {max_turns},拒绝一次性整读"
                );
            }
        }
    }

    /// 按需回填单个项目的 agent transcript 历史(区别于 `crate::backfill::
    /// backfill_all` 的"daemon 启动全量回填")——`OpenProject` 之外的独立
    /// 请求(`Request::BackfillProjectTranscripts`,见 `server.rs`),供"新建
    /// 项目"/"修复项目"按需触发。返回本次实际摄取的文件数,0 不代表出错
    /// (可能这个项目此前已经全部摄取过)。
    pub fn backfill_project(&self, cwd: &str) -> u32 {
        let files =
            crate::transcripts::scan::discover_project_transcript_files(std::path::Path::new(cwd));
        let legacy = crate::backfill::ingest_files(self, files);
        match self.ingest_codex_sqlite(Some(cwd)) {
            Ok(count) => legacy.saturating_add(count),
            Err(e) => {
                dozer_core::log_warn!(LOG, error = %e, "Codex SQLite 项目回填失败");
                legacy
            }
        }
    }

    /// 生产入口,内部用 `dozer_core::agent_paths::home_dir()`。按项目根目录
    /// `cwd` 算出的三家 agent 存储目录，删掉这些目录下已摄取的
    /// conversations 与对应 conversation_turns;Codex 不按项目建目录,单独按
    /// `cwd` 列删。返回被删的 conversations 行数(各家加总)。
    pub fn delete_project_transcripts(&self, cwd: &str) -> Result<u32> {
        self.delete_project_transcripts_in(&dozer_core::agent_paths::home_dir(), cwd)
    }

    /// `home` 显式传入版本，测试用。
    pub fn delete_project_transcripts_in(&self, home: &Path, cwd: &str) -> Result<u32> {
        use dozer_core::agent_paths::{
            aider_project_dir_in, claude_project_dir_in, codebuddy_project_dir_in,
            goose_project_dir_in, opencode_project_dir_in,
        };
        let cwd_path = Path::new(cwd);
        let dirs = [
            claude_project_dir_in(home, cwd_path),
            codebuddy_project_dir_in(home, cwd_path),
            opencode_project_dir_in(home, cwd_path),
            goose_project_dir_in(home, cwd_path),
            aider_project_dir_in(home, cwd_path),
        ];

        let mut conn = self.conn.lock().expect("db lock");
        let tx = conn.transaction()?;
        let mut deleted = 0u32;
        for dir in &dirs {
            let d = dir.to_string_lossy().into_owned();
            tx.execute(
                "DELETE FROM conversation_turns WHERE conversation_id IN
                 (SELECT conversation_id FROM conversations WHERE dir = ?1)",
                [&d],
            )?;
            let affected = tx.execute("DELETE FROM conversations WHERE dir = ?1", [&d])?;
            deleted = deleted.saturating_add(affected as u32);
        }
        // Codex 按 cwd 删,不按 dir:它的 `dir` 是日期目录
        // (`~/.codex/sessions/YYYY/MM/DD/`),同一目录下混着所有项目的会话,
        // 按 dir 删会把别的项目一起清掉。
        tx.execute(
            "DELETE FROM conversation_turns WHERE conversation_id IN
             (SELECT conversation_id FROM conversations
              WHERE agent_kind = 'codex' AND cwd = ?1)",
            [cwd],
        )?;
        let affected = tx.execute(
            "DELETE FROM conversations WHERE agent_kind = 'codex' AND cwd = ?1",
            [cwd],
        )?;
        deleted = deleted.saturating_add(affected as u32);
        tx.commit()?;
        Ok(deleted)
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
            aider_project_dir_in, claude_project_dir_in, codebuddy_project_dir_in,
            goose_project_dir_in, opencode_project_dir_in, v8agent_project_dir_in,
        };
        let cwd_path = Path::new(cwd);
        let conn = self.conn.lock().expect("db lock");
        let mut out = Vec::new();

        // 按 `dir` 查的六家:它们的 `dir` 本身就是各自的项目存储目录,天然
        // 能按项目过滤。Codex 不在这里——它的 `dir` 是日期目录
        // (`~/.codex/sessions/YYYY/MM/DD/`),项目归属存在 `cwd` 列,走下面
        // 独立的按 cwd 分支(见 `agent_paths::codex_sessions_dir_in` 注释)。
        let dir_agents = [
            AgentKind::Claude,
            AgentKind::Codebuddy,
            AgentKind::Opencode,
            AgentKind::Goose,
            AgentKind::Aider,
            AgentKind::V8agent,
        ];
        let dir_candidates: Vec<(AgentKind, String)> = dir_agents
            .iter()
            .filter(|a| agent.is_none() || agent == Some(**a))
            .map(|a| {
                let dir = match a {
                    AgentKind::Claude => claude_project_dir_in(home, cwd_path),
                    AgentKind::Codebuddy => codebuddy_project_dir_in(home, cwd_path),
                    AgentKind::Opencode => opencode_project_dir_in(home, cwd_path),
                    AgentKind::Goose => goose_project_dir_in(home, cwd_path),
                    AgentKind::Aider => aider_project_dir_in(home, cwd_path),
                    AgentKind::V8agent => v8agent_project_dir_in(home, cwd_path),
                    _ => unreachable!("dir_agents 只有六家"),
                };
                (*a, dir.to_string_lossy().into_owned())
            })
            .collect();
        for (_, dir) in &dir_candidates {
            let mut stmt = conn.prepare(&format!(
                "SELECT {CONVERSATION_SUMMARY_COLUMNS}
                 FROM conversations WHERE dir = ?1 ORDER BY last_ts DESC"
            ))?;
            let rows = stmt.query_map([dir], conversation_summary_from_row)?;
            for r in rows {
                out.push(r?);
            }
        }

        // Codex 按 `cwd` 查:`agent_kind = 'codex'` 固定,再限定项目 cwd。上面
        // 的 `dir_candidates` 已把 agent==Some(其余四家)时该不该查 Codex 的
        // 情况排除干净,这里只需判断"要不要查 Codex":None(全部)或显式 Codex。
        let want_codex = agent.is_none() || agent == Some(AgentKind::Codex);
        if want_codex {
            let mut stmt = conn.prepare(&format!(
                "SELECT {CONVERSATION_SUMMARY_COLUMNS}
                 FROM conversations
                 WHERE agent_kind = 'codex' AND cwd = ?1 ORDER BY last_ts DESC"
            ))?;
            let rows = stmt.query_map([cwd], conversation_summary_from_row)?;
            for r in rows {
                out.push(r?);
            }
        }

        // Headless Todo conversations have no transcript file and use their
        // project cwd directly, regardless of which agent processed the task.
        let mut task_sql = format!(
            "SELECT {CONVERSATION_SUMMARY_COLUMNS} FROM conversations
             WHERE file_path = '' AND cwd = ?1"
        );
        if agent.is_some() {
            task_sql.push_str(" AND agent_kind = ?2");
        }
        task_sql.push_str(" ORDER BY last_ts DESC");
        let mut stmt = conn.prepare(&task_sql)?;
        if let Some(agent) = agent {
            let rows = stmt.query_map(
                params![cwd, agent_to_str(agent)],
                conversation_summary_from_row,
            )?;
            for row in rows {
                let row = row?;
                if !out.iter().any(|c| c.conversation_id == row.conversation_id) {
                    out.push(row);
                }
            }
        } else {
            let rows = stmt.query_map([cwd], conversation_summary_from_row)?;
            for row in rows {
                let row = row?;
                if !out.iter().any(|c| c.conversation_id == row.conversation_id) {
                    out.push(row);
                }
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
    ///
    /// 曾经的实现在 `for c in conversations` 循环里每条会话各 `prepare`
    /// 一次带 `owners` CTE 的查询——`owners` 本身是对**全库**
    /// `conversation_turns`(不限 cwd,机器上所有项目、所有 agent 的全部
    /// 回合)做一次 `GROUP BY message_key`,一个项目有 N 条会话就等于把这
    /// 个全库扫描重跑 N 遍,是"打开用量面板很慢"的根因(2026-08-21 验收
    /// 反馈定位)。改成只 `prepare`/扫描一次:`owners` 的 `WHERE
    /// t.conversation_id IN (...)` 限定在这次查询实际涉及的会话集合内,
    /// 一条 SQL 把所有会话的回合一次取回,按 `conversation_id` 分桶聚合。
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
        let mut conversations = self.list_conversations_in(home, cwd, None, u32::MAX, 0)?;
        if let Some(since) = since_ts {
            conversations.retain(|c| c.last_ts >= since);
        }
        if conversations.is_empty() {
            return Ok(Vec::new());
        }
        let ids: Vec<String> = conversations
            .iter()
            .map(|c| c.conversation_id.clone())
            .collect();
        let placeholders = std::iter::repeat_n("?", ids.len())
            .collect::<Vec<_>>()
            .join(",");
        let conn = self.conn.lock().expect("db lock");
        let sql = format!(
            "WITH owners AS (
                SELECT t.message_key,
                       MIN(printf('%020lld|', c2.first_ts) || c2.conversation_id) AS owner_key
                FROM conversation_turns t
                JOIN conversations c2 ON c2.conversation_id = t.conversation_id
                WHERE t.conversation_id IN ({placeholders})
                GROUP BY t.message_key
             )
             SELECT t.conversation_id, t.role, t.tool_calls, t.mutating_tool_calls,
                    t.files_touched, t.tokens_in, t.tokens_out, t.tokens_cache_read,
                    t.tokens_cache_write
             FROM conversation_turns t
             JOIN conversations c1 ON c1.conversation_id = t.conversation_id
             JOIN owners o ON o.message_key = t.message_key
             WHERE t.conversation_id IN ({placeholders})
               AND (printf('%020lld|', c1.first_ts) || c1.conversation_id) = o.owner_key"
        );
        let mut stmt = conn.prepare(&sql)?;
        // 两处 IN (...) 各用一份完整的 ids 列表——占位符在 SQL 里出现两次,
        // 绑定参数也要给两份,`chain` 直接拼接,不需要两次 prepare。
        let rows = stmt.query_map(
            rusqlite::params_from_iter(ids.iter().chain(ids.iter())),
            |row| {
                let files_json: String = row.get(4)?;
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, u32>(2)?,
                    row.get::<_, u32>(3)?,
                    files_json,
                    row.get::<_, u64>(5)?,
                    row.get::<_, u64>(6)?,
                    row.get::<_, u64>(7)?,
                    row.get::<_, u64>(8)?,
                ))
            },
        )?;
        let mut by_conversation: std::collections::HashMap<
            String,
            dozer_core::protocol::UsagePayload,
        > = std::collections::HashMap::new();
        for r in rows {
            let (conversation_id, role, tool_calls, mutating, files_json, tin, tout, tcr, tcw) = r?;
            let payload = by_conversation.entry(conversation_id).or_default();
            // "回合"只统计 human 发言数:P1j 时代的呈现口径(每一条 user 消息
            // 算一个回合),AI 回合/工具调用不计入这一项(2026-08-27 调整)。
            if role == "human" {
                payload.turns += 1;
            }
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
        let out = conversations
            .into_iter()
            .map(|c| {
                let payload = by_conversation
                    .remove(&c.conversation_id)
                    .unwrap_or_default();
                (c, payload)
            })
            .collect();
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
    fn ingest_codex_sqlite_imports_paginated_history_and_usage() {
        let tmp = tempfile::tempdir().unwrap();
        let codex = tmp.path().join(".codex");
        std::fs::create_dir_all(&codex).unwrap();
        let state = rusqlite::Connection::open(codex.join("state_5.sqlite")).unwrap();
        state
            .execute_batch(
                "CREATE TABLE threads (
                id TEXT PRIMARY KEY, cwd TEXT NOT NULL, title TEXT NOT NULL,
                created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
                tokens_used INTEGER NOT NULL, rollout_path TEXT NOT NULL,
                archived INTEGER NOT NULL DEFAULT 0
             );
             INSERT INTO threads VALUES
                ('c1','/work/project','最近的 Codex 会话',100,200,321,'/gone/rollout.jsonl',0),
                ('other','/work/other','别的项目',100,300,999,'/gone/other.jsonl',0);",
            )
            .unwrap();
        let history = rusqlite::Connection::open(codex.join("thread_history_1.sqlite")).unwrap();
        history.execute_batch(
            "CREATE TABLE thread_items (
                thread_id TEXT NOT NULL, item_id TEXT NOT NULL,
                rollout_ordinal INTEGER NOT NULL, created_at_ms INTEGER NOT NULL,
                item_type TEXT NOT NULL, item_json TEXT NOT NULL
             );
             INSERT INTO thread_items VALUES
                ('c1','u1',1,101000,'userMessage',
                 '{\"type\":\"userMessage\",\"content\":[{\"type\":\"text\",\"text\":\"帮我修复统计\"}]}'),
                ('c1','a1',2,102000,'agentMessage',
                 '{\"type\":\"agentMessage\",\"text\":\"已经修复\"}');",
        )
        .unwrap();
        drop(state);
        drop(history);

        let store = TranscriptStore::open(&tmp.path().join("dozer.db")).unwrap();
        assert_eq!(
            store
                .ingest_codex_sqlite_in(tmp.path(), Some("/work/project"))
                .unwrap(),
            1
        );
        let rows = store
            .list_conversations_in(tmp.path(), "/work/project", Some(AgentKind::Codex), 10, 0)
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].conversation_id, "c1");
        assert_eq!(rows[0].title, "最近的 Codex 会话");
        let turns = store.get_conversation_turns("c1", -1, 10).unwrap();
        assert_eq!(turns.iter().filter(|t| t.role == "human").count(), 1);
        let usage = store
            .get_usage_summary_in(tmp.path(), "/work/project", None)
            .unwrap();
        assert_eq!(usage.len(), 1);
        assert_eq!(usage[0].1.turns, 1);
        assert_eq!(usage[0].1.tokens_in, 321);
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
    fn ingested_tool_result_is_error_flag_persists_and_is_queryable() {
        let tmp = tempfile::tempdir().unwrap();
        let store = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
        let text = concat!(
            "{\"type\":\"user\",\"uuid\":\"u1\",\"timestamp\":100,\"message\":{\"role\":\"user\",",
            "\"content\":\"帮我查一下\"}}\n",
            "{\"type\":\"user\",\"uuid\":\"u2\",\"timestamp\":200,\"message\":{\"role\":\"user\",",
            "\"content\":[{\"type\":\"tool_result\",\"is_error\":true,",
            "\"content\":\"boom\"}]}}\n"
        );
        let path = fixture(tmp.path(), "conv1.jsonl", text);
        store.ingest_session(AgentKind::Claude, &path).unwrap();
        let conversation_id = path.file_stem().unwrap().to_string_lossy().into_owned();
        let turns = store
            .get_conversation_turns(&conversation_id, -1, 100)
            .unwrap();
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[1].role, "tool_result");
        assert!(turns[1].is_error);
        assert!(!turns[0].is_error);
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

    /// 回归测试：`list_conversations_in` 曾经完全没有 V8agent 的候选目录
    /// 分支（`Some(V8agent)` 落进 `_ => return Ok(Vec::new())`，`None`
    /// 分支的候选列表里压根没有它），导致哪怕 `conversations` 表里已经有
    /// 真实的 v8agent 数据，按项目查询也永远查不到——数据在库里，UI 却
    /// 看不到。
    #[test]
    fn list_conversations_includes_v8agent() {
        let tmp = tempfile::tempdir().unwrap();
        let store = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
        let home = tempfile::tempdir().unwrap();
        let cwd = std::path::Path::new("/proj");
        let v8agent_dir = dozer_core::agent_paths::v8agent_project_dir_in(home.path(), cwd);
        std::fs::create_dir_all(&v8agent_dir).unwrap();
        let f = fixture(
            &v8agent_dir,
            "a.jsonl",
            "{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"role\":\"user\",\"content\":\"v8agent 对话\"}}\n",
        );
        store.ingest_session(AgentKind::V8agent, &f).unwrap();

        let all = store
            .list_conversations_in(home.path(), "/proj", None, 10, 0)
            .unwrap();
        assert!(all.iter().any(|c| c.agent == AgentKind::V8agent));

        let v8agent_only = store
            .list_conversations_in(home.path(), "/proj", Some(AgentKind::V8agent), 10, 0)
            .unwrap();
        assert_eq!(v8agent_only.len(), 1);
        assert_eq!(v8agent_only[0].agent, AgentKind::V8agent);
    }

    /// Goose 的 transcript 是按项目建目录的 journal
    /// (`~/.dozer/agents/goose/projects/<cwd-key>/<session>.jsonl`),应像
    /// 其余四家按 `dir` 查询一样被 `list_conversations_in` 命中。
    #[test]
    fn list_conversations_includes_goose_by_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let store = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
        let home = tempfile::tempdir().unwrap();
        let cwd = std::path::Path::new("/proj");
        let goose_dir = dozer_core::agent_paths::goose_project_dir_in(home.path(), cwd);
        std::fs::create_dir_all(&goose_dir).unwrap();
        let f = fixture(
            &goose_dir,
            "ds.jsonl",
            "{\"schema_version\":1,\"type\":\"goose_hook\",\"event\":\"UserPromptSubmit\",\"ts_ms\":1,\"dozer_session_id\":\"ds\",\"payload\":{\"message\":\"goose 对话\"}}\n",
        );
        store.ingest_session(AgentKind::Goose, &f).unwrap();

        let all = store
            .list_conversations_in(home.path(), "/proj", None, 10, 0)
            .unwrap();
        assert!(all.iter().any(|c| c.agent == AgentKind::Goose));

        let goose_only = store
            .list_conversations_in(home.path(), "/proj", Some(AgentKind::Goose), 10, 0)
            .unwrap();
        assert_eq!(goose_only.len(), 1);
        assert_eq!(goose_only[0].agent, AgentKind::Goose);
    }

    /// Aider 的 transcript 也是按项目建目录的 canonical JSONL
    /// (`~/.dozer/agents/aider/projects/<cwd-key>/<session>.jsonl`),应被
    /// `list_conversations_in` 命中。
    #[test]
    fn list_conversations_includes_aider_by_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let store = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
        let home = tempfile::tempdir().unwrap();
        let cwd = std::path::Path::new("/proj");
        let aider_dir = dozer_core::agent_paths::aider_project_dir_in(home.path(), cwd);
        std::fs::create_dir_all(&aider_dir).unwrap();
        let f = fixture(
            &aider_dir,
            "ds.jsonl",
            "{\"schema_version\":1,\"type\":\"aider_message\",\"message_id\":\"m1\",\"role\":\"human\",\"content\":\"aider 对话\",\"ts_ms\":1}\n",
        );
        store.ingest_session(AgentKind::Aider, &f).unwrap();

        let all = store
            .list_conversations_in(home.path(), "/proj", None, 10, 0)
            .unwrap();
        assert!(all.iter().any(|c| c.agent == AgentKind::Aider));

        let aider_only = store
            .list_conversations_in(home.path(), "/proj", Some(AgentKind::Aider), 10, 0)
            .unwrap();
        assert_eq!(aider_only.len(), 1);
        assert_eq!(aider_only[0].agent, AgentKind::Aider);
    }

    /// 回归测试：`list_conversations_in` 曾经没有 Codex 的按 cwd 查询分支
    /// （`Some(Codex)` 落进 `_ => return Ok(Vec::new())`，`None` 分支只有四家
    /// 的 dir 候选，压根不碰 `cwd` 列），导致哪怕 `conversations` 表里已有
    /// 带 cwd 的真实 codex 数据，按项目查询也永远查不到。
    #[test]
    fn list_conversations_includes_codex_by_cwd() {
        let tmp = tempfile::tempdir().unwrap();
        let store = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
        let home = tempfile::tempdir().unwrap();
        // Codex 不按项目建目录,直接放临时目录模拟——`ingest_session` 读文件头
        // `session_meta.payload.cwd` 落库,查询侧按 cwd 过滤,跟文件放哪无关。
        let f = fixture(
            tmp.path(),
            "rollout-1.jsonl",
            concat!(
                "{\"type\":\"session_meta\",\"payload\":{\"cwd\":\"/proj/a\"}}\n",
                "{\"timestamp\":\"2026-09-21T02:50:44.030Z\",\"type\":\"response_item\",",
                "\"payload\":{\"type\":\"message\",\"id\":\"m1\",\"role\":\"user\",",
                "\"content\":[{\"type\":\"input_text\",\"text\":\"codex 对话\"}]}}\n",
            ),
        );
        store.ingest_session(AgentKind::Codex, &f).unwrap();

        let a = store
            .list_conversations_in(home.path(), "/proj/a", None, 10, 0)
            .unwrap();
        assert!(a.iter().any(|c| c.agent == AgentKind::Codex));

        let b = store
            .list_conversations_in(home.path(), "/proj/b", None, 10, 0)
            .unwrap();
        assert!(!b.iter().any(|c| c.agent == AgentKind::Codex));

        let codex_only = store
            .list_conversations_in(home.path(), "/proj/a", Some(AgentKind::Codex), 10, 0)
            .unwrap();
        assert_eq!(codex_only.len(), 1);
        assert_eq!(codex_only[0].agent, AgentKind::Codex);
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
    fn get_conversation_turns_surfaces_thinking_text_and_tool_calls() {
        let tmp = tempfile::tempdir().unwrap();
        let store = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
        let text = concat!(
            "{\"type\":\"user\",\"uuid\":\"u1\",\"timestamp\":100,\"message\":{\"role\":\"user\",",
            "\"content\":\"改一下 README\"}}\n",
            "{\"type\":\"assistant\",\"uuid\":\"a1\",\"timestamp\":110,\"message\":{\"content\":[",
            "{\"type\":\"thinking\",\"thinking\":\"先看看现有内容\"},",
            "{\"type\":\"tool_use\",\"name\":\"Edit\",\"input\":{\"file_path\":\"README.md\"}},",
            "{\"type\":\"text\",\"text\":\"改好了\"}],",
            "\"usage\":{\"input_tokens\":100,\"output_tokens\":50,",
            "\"cache_read_input_tokens\":5,\"cache_creation_input_tokens\":2}}}\n"
        );
        let file = fixture(tmp.path(), "s2.jsonl", text);
        store.ingest_session(AgentKind::Claude, &file).unwrap();

        let turns = store.get_conversation_turns("s2", -1, 10).unwrap();
        let ai_turn = turns.iter().find(|t| t.role == "ai").unwrap();
        assert_eq!(ai_turn.thinking_text.as_deref(), Some("先看看现有内容"));
        assert_eq!(ai_turn.tool_calls.len(), 1);
        assert_eq!(ai_turn.tool_calls[0].summary, "Edit README.md");
        assert!(
            ai_turn.tool_calls[0]
                .input_json
                .as_deref()
                .unwrap()
                .contains("README.md")
        );
        // token 四列摄取阶段就落库了(`parse_claude_shaped_chunk` 抽
        // `message.usage`),`get_conversation_turns` 直接读列,不用像
        // thinking_text/tool_calls 那样读时重新解析 raw_json。
        assert_eq!(ai_turn.tokens_in, 100);
        assert_eq!(ai_turn.tokens_out, 50);
        assert_eq!(ai_turn.tokens_cache_read, 5);
        assert_eq!(ai_turn.tokens_cache_write, 2);
    }

    #[test]
    fn get_conversation_turns_surfaces_tool_call_and_result_ids() {
        let tmp = tempfile::tempdir().unwrap();
        let store = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
        let text = concat!(
            "{\"type\":\"assistant\",\"uuid\":\"a1\",\"timestamp\":100,\"message\":{\"content\":[",
            "{\"type\":\"tool_use\",\"id\":\"toolu_01abc\",\"name\":\"Edit\",",
            "\"input\":{\"file_path\":\"README.md\"}},",
            "{\"type\":\"text\",\"text\":\"改好了\"}]}}\n",
            "{\"type\":\"user\",\"uuid\":\"u1\",\"timestamp\":200,\"message\":{\"role\":\"user\",",
            "\"content\":[{\"tool_use_id\":\"toolu_01abc\",\"type\":\"tool_result\",",
            "\"content\":\"done\"}]}}\n"
        );
        let file = fixture(tmp.path(), "s3.jsonl", text);
        store.ingest_session(AgentKind::Claude, &file).unwrap();

        let turns = store.get_conversation_turns("s3", -1, 10).unwrap();
        let ai_turn = turns.iter().find(|t| t.role == "ai").unwrap();
        assert_eq!(ai_turn.tool_calls[0].id.as_deref(), Some("toolu_01abc"));
        let result_turn = turns.iter().find(|t| t.role == "tool_result").unwrap();
        assert_eq!(
            result_turn.tool_result_call_id.as_deref(),
            Some("toolu_01abc")
        );
    }

    #[test]
    fn backfill_project_ingests_only_that_projects_transcripts() {
        let home = tempfile::tempdir().unwrap();
        let db_dir = tempfile::tempdir().unwrap();
        let store = TranscriptStore::open(&db_dir.path().join("t.db")).unwrap();
        let cwd = "/proj/a";
        let claude_dir =
            dozer_core::agent_paths::claude_project_dir_in(home.path(), std::path::Path::new(cwd));
        std::fs::create_dir_all(&claude_dir).unwrap();
        std::fs::write(
            claude_dir.join("s1.jsonl"),
            "{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"role\":\"user\",\"content\":\"你好\"}}\n",
        )
        .unwrap();

        // `backfill_project` 内部用真实 `home_dir()`,这个测试没法直接控制
        // `HOME` 环境变量、也不该在单测里改全局状态——直接调用
        // `scan::discover_project_transcript_files_in` + `backfill::ingest_files`
        // 这条底层组合验证"两个函数拼起来行为对不对",`backfill_project`
        // 本身只是这两行的固定拼接,不单独测(实现阶段如果签名变化,这个测试
        // 需要跟着挪)。
        let files = super::scan::discover_project_transcript_files_in(
            home.path(),
            std::path::Path::new(cwd),
        );
        let n = crate::backfill::ingest_files(&store, files);
        assert_eq!(n, 1);
        assert_eq!(store.get_conversation_turns("s1", -1, 10).unwrap().len(), 1);
    }

    #[test]
    fn delete_project_transcripts_in_removes_only_that_project() {
        let home = tempfile::tempdir().unwrap();
        let store = TranscriptStore::open(&home.path().join("t.db")).unwrap();

        let cwd_project = "/work/proj-a";
        let cwd_other = "/work/proj-b";

        let claude_a = dozer_core::agent_paths::claude_project_dir_in(
            home.path(),
            std::path::Path::new(cwd_project),
        );
        let claude_b = dozer_core::agent_paths::claude_project_dir_in(
            home.path(),
            std::path::Path::new(cwd_other),
        );
        std::fs::create_dir_all(&claude_a).unwrap();
        std::fs::create_dir_all(&claude_b).unwrap();
        std::fs::write(
            claude_a.join("s1.jsonl"),
            "{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"role\":\"user\",\"content\":\"你好\"}}\n",
        )
        .unwrap();
        std::fs::write(
            claude_b.join("s2.jsonl"),
            "{\"type\":\"user\",\"uuid\":\"u2\",\"message\":{\"role\":\"user\",\"content\":\"别的项目\"}}\n",
        )
        .unwrap();

        let files_a = super::scan::discover_project_transcript_files_in(
            home.path(),
            std::path::Path::new(cwd_project),
        );
        let files_b = super::scan::discover_project_transcript_files_in(
            home.path(),
            std::path::Path::new(cwd_other),
        );
        crate::backfill::ingest_files(&store, files_a);
        crate::backfill::ingest_files(&store, files_b);
        assert_eq!(
            store
                .list_conversations_in(home.path(), cwd_project, None, 100, 0)
                .unwrap()
                .len(),
            1
        );

        // 删 proj-a：它的对话(含 turns)被删，别的项目不受影响。
        let deleted = store
            .delete_project_transcripts_in(home.path(), cwd_project)
            .unwrap();
        assert_eq!(deleted, 1);
        assert_eq!(
            store
                .list_conversations_in(home.path(), cwd_project, None, 100, 0)
                .unwrap()
                .len(),
            0
        );
        assert_eq!(store.get_conversation_turns("s1", -1, 10).unwrap().len(), 0);
        assert_eq!(
            store
                .list_conversations_in(home.path(), cwd_other, None, 100, 0)
                .unwrap()
                .len(),
            1
        );
    }

    /// Codex 删除按 cwd 不按 dir(它的 dir 是日期目录,同目录混着所有项目),
    /// 删一个项目不能把同日期目录下别的项目的 Codex 会话一起清掉。
    #[test]
    fn delete_project_transcripts_in_removes_codex_by_cwd_only() {
        let tmp = tempfile::tempdir().unwrap();
        let store = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
        let home = tempfile::tempdir().unwrap();
        let codex_a = fixture(
            tmp.path(),
            "rollout-a.jsonl",
            "{\"type\":\"session_meta\",\"payload\":{\"cwd\":\"/proj/a\"}}\n",
        );
        let codex_b = fixture(
            tmp.path(),
            "rollout-b.jsonl",
            "{\"type\":\"session_meta\",\"payload\":{\"cwd\":\"/proj/b\"}}\n",
        );
        store.ingest_session(AgentKind::Codex, &codex_a).unwrap();
        store.ingest_session(AgentKind::Codex, &codex_b).unwrap();

        let deleted = store
            .delete_project_transcripts_in(home.path(), "/proj/a")
            .unwrap();
        assert_eq!(deleted, 1);
        assert_eq!(
            store
                .list_conversations_in(home.path(), "/proj/a", None, 100, 0)
                .unwrap()
                .len(),
            0
        );
        assert_eq!(
            store
                .list_conversations_in(home.path(), "/proj/b", None, 100, 0)
                .unwrap()
                .len(),
            1
        );
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

    #[test]
    fn get_usage_summary_batches_multiple_conversations_without_cross_contamination() {
        // 单条 SQL 一次取回所有会话的回合、按 conversation_id 分桶聚合
        // (2026-08-21 重写，此前是每条会话各查一次、每次都重新全库扫描
        // owners CTE)——这个测试锁死"分桶不串台"这条正确性要求。
        let tmp = tempfile::tempdir().unwrap();
        let store = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
        let home = tempfile::tempdir().unwrap();
        let cwd = std::path::Path::new("/proj");
        let dir = dozer_core::agent_paths::claude_project_dir_in(home.path(), cwd);
        std::fs::create_dir_all(&dir).unwrap();

        let a = fixture(
            &dir,
            "a.jsonl",
            "{\"type\":\"assistant\",\"uuid\":\"a-1\",\"message\":{\"usage\":{\"input_tokens\":10,\"output_tokens\":1},\"content\":[]}}\n",
        );
        let b = fixture(
            &dir,
            "b.jsonl",
            "{\"type\":\"assistant\",\"uuid\":\"b-1\",\"message\":{\"usage\":{\"input_tokens\":20,\"output_tokens\":2},\"content\":[]}}\n",
        );
        // c 会话只有人类发言,没有 assistant usage——应该仍然出现在结果
        // 里,payload 是默认零值,不能因为没有匹配行就从结果集里消失。
        let c = fixture(
            &dir,
            "c.jsonl",
            "{\"type\":\"user\",\"uuid\":\"c-1\",\"message\":{\"role\":\"user\",\"content\":\"你好\"}}\n",
        );
        store.ingest_session(AgentKind::Claude, &a).unwrap();
        store.ingest_session(AgentKind::Claude, &b).unwrap();
        store.ingest_session(AgentKind::Claude, &c).unwrap();

        let rows = store
            .get_usage_summary_in(home.path(), "/proj", None)
            .unwrap();
        assert_eq!(rows.len(), 3);
        let usage_of = |id: &str| -> u64 {
            rows.iter()
                .find(|(c, _)| c.conversation_id == id)
                .unwrap()
                .1
                .tokens_in
        };
        assert_eq!(usage_of("a"), 10);
        assert_eq!(usage_of("b"), 20);
        assert_eq!(
            usage_of("c"),
            0,
            "没有 assistant usage 的会话仍应出现,只是用量为零"
        );
    }

    #[test]
    fn usage_turns_counts_human_messages_only() {
        // "回合"口径 2026-08-27 调整为只统计 human 发言数:AI 回合(assistant,
        // 即便带 usage)和工具调用的 tool_result 都不计入 `turns`,只有
        // `role == "human"` 的 user 消息才算。
        let tmp = tempfile::tempdir().unwrap();
        let store = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
        let home = tempfile::tempdir().unwrap();
        let cwd = std::path::Path::new("/proj");
        let dir = dozer_core::agent_paths::claude_project_dir_in(home.path(), cwd);
        std::fs::create_dir_all(&dir).unwrap();

        // 2 条 human + 1 条 ai(带 usage) + 1 条 tool_result。
        let lines = concat!(
            "{\"type\":\"user\",\"uuid\":\"h1\",\"timestamp\":100,\"message\":{\"role\":\"user\",\"content\":\"第一问\"}}\n",
            "{\"type\":\"user\",\"uuid\":\"h2\",\"timestamp\":200,\"message\":{\"role\":\"user\",\"content\":\"第二问\"}}\n",
            "{\"type\":\"assistant\",\"uuid\":\"a1\",\"timestamp\":300,\"message\":{\"usage\":{\"input_tokens\":10,\"output_tokens\":5},\"content\":[{\"type\":\"text\",\"text\":\"答复\"}]}}\n",
            "{\"type\":\"user\",\"uuid\":\"tr1\",\"timestamp\":400,\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"tool_result\",\"content\":\"结果\"}]}}\n"
        );
        fixture(&dir, "s.jsonl", lines);
        let path = dir.join("s.jsonl");
        store.ingest_session(AgentKind::Claude, &path).unwrap();

        // 全部回合数是 4,但 "回合"统计只算 human 的 2 条。
        let all = store.get_conversation_turns("s", -1, 100).unwrap();
        assert_eq!(all.len(), 4, "入库的原始回合是 4 条");

        let rows = store
            .get_usage_summary_in(home.path(), "/proj", None)
            .unwrap();
        assert_eq!(rows.len(), 1);
        let (_, usage) = &rows[0];
        assert_eq!(usage.turns, 2, "回合只统计 human 发言数");
        assert_eq!(
            usage.tokens_in, 10,
            "token 用量不受口径调整影响,仍按全量回合统计"
        );
        assert_eq!(usage.tool_calls, 0, "本夹具没有工具调用,tool_calls 仍为 0");
    }

    #[test]
    fn get_conversation_turns_returns_empty_without_matching_conversations_row() {
        let tmp = tempfile::tempdir().unwrap();
        let store = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
        // 直接绕过 `ingest_session`,只手动插 conversation_turns,不插 conversations——
        // 复现"headless 任务处理如果忘记同时插 conversations 占位行"这个坑。
        store
            .conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO conversation_turns
                 (conversation_id, turn_index, message_key, role, content, tools_summary,
                  thinking, ts, tool_calls, mutating_tool_calls, files_touched,
                  tokens_in, tokens_out, tokens_cache_read, tokens_cache_write, raw_json, is_error)
                 VALUES ('orphan',0,'orphan:0','human','hi','[]',0,1,0,0,'[]',0,0,0,0,'{}',0)",
                [],
            )
            .unwrap();
        let turns = store.get_conversation_turns("orphan", -1, 10).unwrap();
        assert!(
            turns.is_empty(),
            "没有 conversations 行时,JOIN 应该拿不到任何数据"
        );
    }

    #[test]
    fn record_task_turns_creates_conversations_row_and_two_turns() {
        let tmp = tempfile::tempdir().unwrap();
        let store = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
        store
            .record_task_turns(
                "task-session-1",
                AgentKind::Claude,
                "/tmp/proj",
                "修个 bug",
                "先看看这个 bug",
                "已经修好了,提交在 abc123",
            )
            .unwrap();
        let turns = store
            .get_conversation_turns("task-session-1", -1, 10)
            .unwrap();
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].role, "human");
        assert_eq!(turns[1].role, "ai");
        assert_eq!(turns[1].content, "已经修好了,提交在 abc123");
    }

    #[test]
    fn record_task_turns_appends_on_second_call_without_duplicating_conversations_row() {
        let tmp = tempfile::tempdir().unwrap();
        let store = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
        store
            .record_task_turns(
                "task-session-1",
                AgentKind::Claude,
                "/tmp/proj",
                "修个 bug",
                "第一句",
                "第一次回复",
            )
            .unwrap();
        store
            .record_task_turns(
                "task-session-1",
                AgentKind::Claude,
                "/tmp/proj",
                "修个 bug",
                "第二句",
                "第二次回复",
            )
            .unwrap();
        let turns = store
            .get_conversation_turns("task-session-1", -1, 10)
            .unwrap();
        assert_eq!(turns.len(), 4);
        assert_eq!(turns[2].turn_index, 2);
        assert_eq!(turns[3].content, "第二次回复");
    }
}
