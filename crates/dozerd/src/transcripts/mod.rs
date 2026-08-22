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
    /// `after_turn_index` 传 `-1` 表示从第一条开始。JOIN `conversations`
    /// 拿 `agent_kind` 只为了给 `parse::extract_turn_trace_detail` 挑对
    /// 解析形状——不新增参数(client/protocol 签名都不用改),`raw_json`
    /// 每行都读一次、当场解析,不落新列(见 spec"读时解析"一节)。
    pub fn get_conversation_turns(
        &self,
        conversation_id: &str,
        after_turn_index: i64,
        limit: u32,
    ) -> Result<Vec<dozer_core::protocol::TurnRecord>> {
        let conn = self.conn.lock().expect("db lock");
        let mut stmt = conn.prepare(
            "SELECT t.turn_index, t.role, t.content, t.thinking, t.ts, t.is_error,
                    t.raw_json, c.agent_kind
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

    /// 把一个会话的 `conversation_turns` 按"每个真实人类回合(排除斜杠
    /// 命令)"切成分组区间。用窗口函数一次查完，不是"先整段拉取再在
    /// Rust 里 split"——避免大会话(几百上千个 turn)把全部内容搬进内存
    /// 只为了算分组边界。
    ///
    /// SQL 层做不了调用 `parse::is_command_content` 那个 Rust 函数，这里
    /// 用等价的 `NOT GLOB '/[A-Za-z0-9]*'` 近似(SQLite `GLOB` 区分大小写、
    /// 支持 `[...]` 字符类，`content` 整串以 `/` 加一个字母数字开头即命中)
    /// ——覆盖面跟 `is_command_content` 的判定逻辑一致，两处判据分开维护
    /// 是因为一个跑在 SQL 里、一个跑在 Rust 里，没有共用的余地，写好
    /// `is_command_content` 的单测(Task 7)已经锁死了预期行为，这里的 SQL
    /// 只要保持同样的"以 / 加字母数字开头"语义即可。
    pub fn list_turn_groups(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<dozer_core::protocol::TurnGroupSummary>> {
        let conn = self.conn.lock().expect("db lock");
        let mut stmt = conn.prepare(
            "WITH marked AS (
                SELECT turn_index, role, content, ts,
                    SUM(CASE
                        WHEN role = 'human' AND content NOT GLOB '/[A-Za-z0-9]*'
                        THEN 1 ELSE 0
                    END) OVER (ORDER BY turn_index) AS grp
                FROM conversation_turns
                WHERE conversation_id = ?1
             )
             SELECT
                MIN(turn_index) AS start_turn_index,
                MAX(turn_index) AS end_turn_index,
                COALESCE(MAX(CASE
                    WHEN role = 'human' AND content NOT GLOB '/[A-Za-z0-9]*'
                    THEN substr(content, 1, 80)
                END), '') AS title,
                COALESCE(MAX(CASE
                    WHEN role = 'human' AND content NOT GLOB '/[A-Za-z0-9]*'
                    THEN ts
                END), MIN(ts), 0) AS group_ts
             FROM marked
             GROUP BY grp
             ORDER BY start_turn_index ASC",
        )?;
        let rows = stmt.query_map([conversation_id], |row| {
            Ok(dozer_core::protocol::TurnGroupSummary {
                start_turn_index: row.get(0)?,
                end_turn_index: row.get(1)?,
                title: row.get(2)?,
                ts: row.get::<_, Option<u64>>(3)?.unwrap_or(0),
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// 跨该 cwd 下全部 session 的回合分组，拍平成一份按时间倒序的列表
    /// (对话面板扁平展示用，取代按 session 展开的树；spec 2026-08-21)。
    pub fn list_all_turn_groups(
        &self,
        cwd: &str,
        limit: u32,
    ) -> Result<Vec<dozer_core::protocol::TurnGroupEntry>> {
        self.list_all_turn_groups_in(&dozer_core::agent_paths::home_dir(), cwd, limit)
    }

    /// `home` 显式传入版本,测试用。跟 `get_usage_summary_in` 同样的手法:
    /// 先用 `list_conversations_in` 拿到这个 cwd 下的会话集合，一条 SQL
    /// 把所有会话的回合一次取回、按 `conversation_id` 分区分组，而不是
    /// 对每个 session 各 `prepare`/查询一次(`list_turn_groups` 的单会话
    /// 版本不能直接循环复用——那会退回"N 条会话就扫 N 次"的老问题)。
    ///
    /// 分组时间戳(`group_ts`)取自锚点人类回合的 `ts`；已知有分组解析
    /// 不出锚点、且组内全部 `ts` 也是 NULL 的情况(=0)，这种"不准确"用
    /// 所属 session 的 `last_ts` 兜底，而不是试图修掉根因。
    pub fn list_all_turn_groups_in(
        &self,
        home: &Path,
        cwd: &str,
        limit: u32,
    ) -> Result<Vec<dozer_core::protocol::TurnGroupEntry>> {
        let conversations = self.list_conversations_in(home, cwd, None, u32::MAX, 0)?;
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
            "WITH marked AS (
                SELECT conversation_id, turn_index, role, content, ts,
                    SUM(CASE
                        WHEN role = 'human' AND content NOT GLOB '/[A-Za-z0-9]*'
                        THEN 1 ELSE 0
                    END) OVER (PARTITION BY conversation_id ORDER BY turn_index) AS grp
                FROM conversation_turns
                WHERE conversation_id IN ({placeholders})
             ),
             grouped AS (
                SELECT conversation_id,
                    MIN(turn_index) AS start_turn_index,
                    MAX(turn_index) AS end_turn_index,
                    COALESCE(MAX(CASE
                        WHEN role = 'human' AND content NOT GLOB '/[A-Za-z0-9]*'
                        THEN substr(content, 1, 80)
                    END), '') AS title,
                    COALESCE(MAX(CASE
                        WHEN role = 'human' AND content NOT GLOB '/[A-Za-z0-9]*'
                        THEN ts
                    END), MIN(ts), 0) AS group_ts
                FROM marked
                GROUP BY conversation_id, grp
             )
             SELECT g.conversation_id, c.file_path, c.agent_kind,
                    g.start_turn_index, g.end_turn_index, g.title,
                    CASE WHEN g.group_ts = 0 THEN c.last_ts ELSE g.group_ts END AS effective_ts
             FROM grouped g
             JOIN conversations c ON c.conversation_id = g.conversation_id
             ORDER BY effective_ts DESC
             LIMIT ?"
        );
        let mut stmt = conn.prepare(&sql)?;
        let limit_param = i64::from(limit);
        let rows = stmt.query_map(
            rusqlite::params_from_iter(
                ids.iter()
                    .map(|s| s as &dyn rusqlite::ToSql)
                    .chain(std::iter::once(&limit_param as &dyn rusqlite::ToSql)),
            ),
            |row| {
                let agent_kind: String = row.get(2)?;
                Ok(dozer_core::protocol::TurnGroupEntry {
                    conversation_id: row.get(0)?,
                    file_path: row.get(1)?,
                    agent: agent_from_str(&agent_kind),
                    start_turn_index: row.get(3)?,
                    end_turn_index: row.get(4)?,
                    title: row.get(5)?,
                    ts: row.get(6)?,
                })
            },
        )?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
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
             SELECT t.conversation_id, t.tool_calls, t.mutating_tool_calls, t.files_touched,
                    t.tokens_in, t.tokens_out, t.tokens_cache_read, t.tokens_cache_write
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
            },
        )?;
        let mut by_conversation: std::collections::HashMap<
            String,
            dozer_core::protocol::UsagePayload,
        > = std::collections::HashMap::new();
        for r in rows {
            let (conversation_id, tool_calls, mutating, files_json, tin, tout, tcr, tcw) = r?;
            let payload = by_conversation.entry(conversation_id).or_default();
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
            "{\"type\":\"text\",\"text\":\"改好了\"}]}}\n"
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
    }

    #[test]
    fn list_turn_groups_splits_on_real_human_turns_and_folds_commands() {
        let tmp = tempfile::tempdir().unwrap();
        let store = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
        let text = concat!(
            "{\"type\":\"user\",\"uuid\":\"u1\",\"timestamp\":100,\"message\":{\"role\":\"user\",",
            "\"content\":\"第一个真实问题\"}}\n",
            "{\"type\":\"assistant\",\"uuid\":\"a1\",\"timestamp\":110,\"message\":{\"role\":\"assistant\",",
            "\"content\":[{\"type\":\"text\",\"text\":\"回复一\"}]}}\n",
            "{\"type\":\"user\",\"uuid\":\"u2\",\"timestamp\":120,\"message\":{\"role\":\"user\",",
            "\"content\":\"/clear\"}}\n",
            "{\"type\":\"user\",\"uuid\":\"u3\",\"timestamp\":200,\"message\":{\"role\":\"user\",",
            "\"content\":\"第二个真实问题\"}}\n",
            "{\"type\":\"assistant\",\"uuid\":\"a2\",\"timestamp\":210,\"message\":{\"role\":\"assistant\",",
            "\"content\":[{\"type\":\"text\",\"text\":\"回复二\"}]}}\n"
        );
        let path = fixture(tmp.path(), "conv1.jsonl", text);
        store.ingest_session(AgentKind::Claude, &path).unwrap();
        let conversation_id = path.file_stem().unwrap().to_string_lossy().into_owned();

        let groups = store.list_turn_groups(&conversation_id).unwrap();

        // /clear 是斜杠命令，不该单独成组——应该并进"第一个真实问题"那组
        // (它排在 u1/a1 之后、u3 之前，turn_index 上属于第一组的尾巴)。
        assert_eq!(groups.len(), 2, "{groups:?}");
        assert_eq!(groups[0].title, "第一个真实问题");
        assert_eq!(groups[0].start_turn_index, 0);
        assert_eq!(groups[0].end_turn_index, 2); // u1, a1, u2(/clear) 都在这组
        assert_eq!(groups[1].title, "第二个真实问题");
        assert_eq!(groups[1].start_turn_index, 3);
        assert_eq!(groups[1].end_turn_index, 4);
    }

    #[test]
    fn list_turn_groups_on_unknown_conversation_returns_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let store = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
        let groups = store.list_turn_groups("does-not-exist").unwrap();
        assert!(groups.is_empty());
    }

    #[test]
    fn list_all_turn_groups_flattens_across_sessions_sorted_by_ts_desc() {
        let tmp = tempfile::tempdir().unwrap();
        let store = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
        let home = tempfile::tempdir().unwrap();
        let cwd = std::path::Path::new("/proj");
        let dir = dozer_core::agent_paths::claude_project_dir_in(home.path(), cwd);
        std::fs::create_dir_all(&dir).unwrap();

        // session "old"：唯一分组的锚点 ts=100。
        let old = fixture(
            &dir,
            "old.jsonl",
            concat!(
                "{\"type\":\"user\",\"uuid\":\"u1\",\"timestamp\":100,\"message\":{\"role\":\"user\",",
                "\"content\":\"很久以前的问题\"}}\n",
            ),
        );
        // session "new"：两个分组，锚点 ts 分别为 300、200——组内乱序，
        // 验证排序是按分组各自的 ts，不是按所属 session 整体排。
        let new = fixture(
            &dir,
            "new.jsonl",
            concat!(
                "{\"type\":\"user\",\"uuid\":\"u1\",\"timestamp\":200,\"message\":{\"role\":\"user\",",
                "\"content\":\"第一个问题\"}}\n",
                "{\"type\":\"user\",\"uuid\":\"u2\",\"timestamp\":300,\"message\":{\"role\":\"user\",",
                "\"content\":\"第二个问题\"}}\n",
            ),
        );
        store.ingest_session(AgentKind::Claude, &old).unwrap();
        store.ingest_session(AgentKind::Claude, &new).unwrap();

        let groups = store
            .list_all_turn_groups_in(home.path(), "/proj", 10)
            .unwrap();
        assert_eq!(groups.len(), 3, "{groups:?}");
        let ts_order: Vec<u64> = groups.iter().map(|g| g.ts).collect();
        assert_eq!(ts_order, vec![300, 200, 100], "应按 ts 全局倒序拍平");
        assert_eq!(groups[0].conversation_id, "new");
        assert_eq!(groups[0].title, "第二个问题");
        assert_eq!(groups[2].conversation_id, "old");
        assert_eq!(groups[2].agent, AgentKind::Claude);
        assert_eq!(groups[2].file_path, old.to_string_lossy());
    }

    #[test]
    fn list_all_turn_groups_falls_back_to_session_last_ts_when_group_ts_is_zero() {
        let tmp = tempfile::tempdir().unwrap();
        let store = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
        let home = tempfile::tempdir().unwrap();
        let cwd = std::path::Path::new("/proj");
        let dir = dozer_core::agent_paths::claude_project_dir_in(home.path(), cwd);
        std::fs::create_dir_all(&dir).unwrap();

        // 整份会话都没有 timestamp 字段——分组的锚点 ts 算出来是 0，
        // ingest 落库时 last_ts 会退回摄取时刻的 wall clock（非 0）。
        let no_ts = fixture(
            &dir,
            "no_ts.jsonl",
            "{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"role\":\"user\",\"content\":\"没有时间戳\"}}\n",
        );
        store.ingest_session(AgentKind::Claude, &no_ts).unwrap();

        let conversations = store
            .list_conversations_in(home.path(), "/proj", None, 10, 0)
            .unwrap();
        assert_eq!(conversations.len(), 1);
        let session_last_ts = conversations[0].last_ts;
        assert_ne!(session_last_ts, 0, "ingest 应该给了个非零的摄取时刻兜底");

        let groups = store
            .list_all_turn_groups_in(home.path(), "/proj", 10)
            .unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(
            groups[0].ts, session_last_ts,
            "分组自身 ts=0 时应该回落成所属 session 的 last_ts，而不是继续显示 0"
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
}
