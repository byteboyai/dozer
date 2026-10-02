//! 群聊存储:project 级 SQLite,`chat_groups`/`chat_group_members`/
//! `chat_group_messages` 三张表,与 `TodoStore` 共享同一个 `dozer.db`。
//! 表名带 `chat_` 前缀是因为 `groups` 是 SQLite 关键字。
//! 设计见 `docs/superpowers/specs/2026-10-02-group-chat-panel-design.md`。

use crate::group_mentions::{handle_key, validate_handle};
use anyhow::{Context, Result, bail};
use dozer_core::protocol::{
    AgentKind, GroupAuthor, GroupInfo, GroupMemberInfo, GroupMessageInfo, GroupMessageStatus,
};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;
use std::sync::Mutex;

/// `history_before` 最多取回的消息条数;更早的只计数(提示词里写"已省略 N 条")。
pub const HISTORY_FETCH_LIMIT: i64 = 300;

const INTERRUPTED_REASON: &str = "dozerd 重启，本次发言被中断";

pub struct GroupStore {
    conn: Mutex<Connection>,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 第一版只有这两家能入群。
fn agent_to_str(agent: AgentKind) -> Result<&'static str> {
    match agent {
        AgentKind::Claude => Ok("claude"),
        AgentKind::Codex => Ok("codex"),
        other => bail!("{} 暂不支持加入群聊", other.display_label()),
    }
}

fn agent_from_str(s: &str) -> AgentKind {
    match s {
        "claude" => AgentKind::Claude,
        "codex" => AgentKind::Codex,
        _ => AgentKind::Unknown,
    }
}

fn status_parts(status: &GroupMessageStatus) -> (&'static str, Option<&str>) {
    match status {
        GroupMessageStatus::Queued => ("queued", None),
        GroupMessageStatus::Running => ("running", None),
        GroupMessageStatus::Done => ("done", None),
        GroupMessageStatus::Failed { reason } => ("failed", Some(reason.as_str())),
        GroupMessageStatus::Cancelled => ("cancelled", None),
    }
}

const MESSAGE_COLUMNS: &str = "id, group_id, seq, rev, author_kind, author_member_id, text, \
    mentions, status, fail_reason, duration_ms, created_ms, todo_id";

fn row_to_message(row: &rusqlite::Row) -> rusqlite::Result<GroupMessageInfo> {
    let author_kind: String = row.get(4)?;
    let author_member_id: Option<i64> = row.get(5)?;
    let author = match (author_kind.as_str(), author_member_id) {
        ("human", _) => GroupAuthor::Human,
        ("member", Some(member_id)) => GroupAuthor::Member { member_id },
        _ => GroupAuthor::System,
    };
    let mentions_json: String = row.get(7)?;
    let status: Option<String> = row.get(8)?;
    let fail_reason: Option<String> = row.get(9)?;
    let status = status.map(|s| match s.as_str() {
        "queued" => GroupMessageStatus::Queued,
        "running" => GroupMessageStatus::Running,
        "done" => GroupMessageStatus::Done,
        "failed" => GroupMessageStatus::Failed {
            reason: fail_reason.unwrap_or_default(),
        },
        _ => GroupMessageStatus::Cancelled,
    });
    Ok(GroupMessageInfo {
        id: row.get(0)?,
        group_id: row.get(1)?,
        seq: row.get(2)?,
        rev: row.get(3)?,
        author,
        text: row.get(6)?,
        mentions: serde_json::from_str(&mentions_json).unwrap_or_default(),
        status,
        duration_ms: row.get::<_, Option<i64>>(10)?.map(|v| v as u64),
        created_ms: row.get::<_, i64>(11)? as u64,
        todo_id: row.get(12)?,
    })
}

fn row_to_member(row: &rusqlite::Row) -> rusqlite::Result<GroupMemberInfo> {
    Ok(GroupMemberInfo {
        id: row.get(0)?,
        group_id: row.get(1)?,
        agent: agent_from_str(&row.get::<_, String>(2)?),
        handle: row.get(3)?,
        role_prompt: row.get(4)?,
    })
}

fn load_members(conn: &Connection, group_id: i64) -> Result<Vec<GroupMemberInfo>> {
    let mut stmt = conn.prepare(
        "SELECT id, group_id, agent, handle, role_prompt FROM chat_group_members
         WHERE group_id = ?1 ORDER BY id",
    )?;
    let rows = stmt.query_map([group_id], row_to_member)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn load_group(conn: &Connection, group_id: i64) -> Result<GroupInfo> {
    let (id, project_id, topic, created_ms): (i64, i64, String, i64) = conn
        .query_row(
            "SELECT id, project_id, topic, created_ms FROM chat_groups WHERE id = ?1",
            [group_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?
        .ok_or_else(|| anyhow::anyhow!("群聊不存在: id={group_id}"))?;
    Ok(GroupInfo {
        id,
        project_id,
        topic,
        created_ms: created_ms as u64,
        members: load_members(conn, id)?,
    })
}

fn next_seq(conn: &Connection, group_id: i64) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COALESCE(MAX(seq), 0) + 1 FROM chat_group_messages WHERE group_id = ?1",
        [group_id],
        |r| r.get(0),
    )?)
}

fn next_rev(conn: &Connection, group_id: i64) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COALESCE(MAX(rev), 0) + 1 FROM chat_group_messages WHERE group_id = ?1",
        [group_id],
        |r| r.get(0),
    )?)
}

fn message_by_id(conn: &Connection, id: i64) -> Result<GroupMessageInfo> {
    let sql = format!("SELECT {MESSAGE_COLUMNS} FROM chat_group_messages WHERE id = ?1");
    conn.query_row(&sql, [id], row_to_message)
        .optional()?
        .ok_or_else(|| anyhow::anyhow!("消息不存在: id={id}"))
}

impl GroupStore {
    pub fn new(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("建库目录")?;
        }
        let conn = Connection::open(path).context("打开 SQLite")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS chat_groups (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                project_id INTEGER NOT NULL,
                topic TEXT NOT NULL,
                created_ms INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS chat_groups_project ON chat_groups(project_id);
             CREATE TABLE IF NOT EXISTS chat_group_members (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                group_id INTEGER NOT NULL,
                agent TEXT NOT NULL,
                handle TEXT NOT NULL,
                handle_key TEXT NOT NULL,
                role_prompt TEXT NOT NULL DEFAULT ''
             );
             CREATE UNIQUE INDEX IF NOT EXISTS chat_group_members_handle
                ON chat_group_members(group_id, handle_key);
             CREATE TABLE IF NOT EXISTS chat_group_messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                group_id INTEGER NOT NULL,
                seq INTEGER NOT NULL,
                rev INTEGER NOT NULL,
                author_kind TEXT NOT NULL,
                author_member_id INTEGER,
                text TEXT NOT NULL DEFAULT '',
                mentions TEXT NOT NULL DEFAULT '[]',
                status TEXT,
                fail_reason TEXT,
                duration_ms INTEGER,
                created_ms INTEGER NOT NULL,
                todo_id INTEGER
             );
             CREATE UNIQUE INDEX IF NOT EXISTS chat_group_messages_seq
                ON chat_group_messages(group_id, seq);
             CREATE INDEX IF NOT EXISTS chat_group_messages_rev
                ON chat_group_messages(group_id, rev);",
        )
        .context("建表")?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn create_group(&self, project_id: i64, topic: &str) -> Result<GroupInfo> {
        let topic = topic.trim();
        if topic.is_empty() {
            bail!("群主题不能为空");
        }
        let conn = self.conn.lock().expect("db lock");
        conn.execute(
            "INSERT INTO chat_groups (project_id, topic, created_ms) VALUES (?1, ?2, ?3)",
            params![project_id, topic, now_ms() as i64],
        )?;
        load_group(&conn, conn.last_insert_rowid())
    }

    pub fn list_groups(&self, project_id: i64) -> Result<Vec<GroupInfo>> {
        let conn = self.conn.lock().expect("db lock");
        let ids: Vec<i64> = {
            let mut stmt =
                conn.prepare("SELECT id FROM chat_groups WHERE project_id = ?1 ORDER BY id DESC")?;
            let rows = stmt.query_map([project_id], |r| r.get(0))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        ids.into_iter().map(|id| load_group(&conn, id)).collect()
    }

    pub fn get_group(&self, group_id: i64) -> Result<GroupInfo> {
        let conn = self.conn.lock().expect("db lock");
        load_group(&conn, group_id)
    }

    pub fn delete_group(&self, group_id: i64) -> Result<()> {
        let mut conn = self.conn.lock().expect("db lock");
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM chat_group_messages WHERE group_id = ?1",
            [group_id],
        )?;
        tx.execute(
            "DELETE FROM chat_group_members WHERE group_id = ?1",
            [group_id],
        )?;
        let affected = tx.execute("DELETE FROM chat_groups WHERE id = ?1", [group_id])?;
        if affected == 0 {
            bail!("群聊不存在: id={group_id}");
        }
        tx.commit()?;
        Ok(())
    }

    pub fn add_member(
        &self,
        group_id: i64,
        agent: AgentKind,
        handle: &str,
        role_prompt: &str,
    ) -> Result<GroupInfo> {
        let agent_str = agent_to_str(agent)?;
        validate_handle(handle).map_err(anyhow::Error::msg)?;
        let conn = self.conn.lock().expect("db lock");
        load_group(&conn, group_id)?; // 群必须存在
        conn.execute(
            "INSERT INTO chat_group_members (group_id, agent, handle, handle_key, role_prompt)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![group_id, agent_str, handle, handle_key(handle), role_prompt],
        )
        .map_err(|e| match e {
            rusqlite::Error::SqliteFailure(f, _)
                if f.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                anyhow::anyhow!("群里已有名为 {handle} 的成员")
            }
            other => other.into(),
        })?;
        load_group(&conn, group_id)
    }

    pub fn update_member(
        &self,
        member_id: i64,
        handle: &str,
        role_prompt: &str,
    ) -> Result<GroupInfo> {
        validate_handle(handle).map_err(anyhow::Error::msg)?;
        let conn = self.conn.lock().expect("db lock");
        let group_id: i64 = conn
            .query_row(
                "SELECT group_id FROM chat_group_members WHERE id = ?1",
                [member_id],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(|| anyhow::anyhow!("成员不存在: id={member_id}"))?;
        conn.execute(
            "UPDATE chat_group_members SET handle = ?1, handle_key = ?2, role_prompt = ?3
             WHERE id = ?4",
            params![handle, handle_key(handle), role_prompt, member_id],
        )
        .map_err(|e| match e {
            rusqlite::Error::SqliteFailure(f, _)
                if f.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                anyhow::anyhow!("群里已有名为 {handle} 的成员")
            }
            other => other.into(),
        })?;
        load_group(&conn, group_id)
    }

    pub fn remove_member(&self, member_id: i64) -> Result<GroupInfo> {
        let conn = self.conn.lock().expect("db lock");
        let group_id: i64 = conn
            .query_row(
                "SELECT group_id FROM chat_group_members WHERE id = ?1",
                [member_id],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(|| anyhow::anyhow!("成员不存在: id={member_id}"))?;
        conn.execute("DELETE FROM chat_group_members WHERE id = ?1", [member_id])?;
        load_group(&conn, group_id)
    }

    pub fn get_member(&self, member_id: i64) -> Result<Option<GroupMemberInfo>> {
        let conn = self.conn.lock().expect("db lock");
        Ok(conn
            .query_row(
                "SELECT id, group_id, agent, handle, role_prompt FROM chat_group_members
                 WHERE id = ?1",
                [member_id],
                row_to_member,
            )
            .optional()?)
    }

    /// human 发消息 + 为每个被点名成员一次性创建 `Queued` 占位(seq 连续,
    /// 见 plan Ruling 2)。`mentions` 必须都是本群成员。
    pub fn post_human_message(
        &self,
        group_id: i64,
        text: &str,
        mentions: &[i64],
    ) -> Result<(GroupMessageInfo, Vec<GroupMessageInfo>)> {
        let mut conn = self.conn.lock().expect("db lock");
        let members = load_members(&conn, group_id)?;
        if let Some(bad) = mentions
            .iter()
            .find(|m| !members.iter().any(|mem| mem.id == **m))
        {
            bail!("成员 id={bad} 不在群 id={group_id} 里");
        }
        load_group(&conn, group_id)?;
        let tx = conn.transaction()?;
        let now = now_ms() as i64;
        let mentions_json = serde_json::to_string(mentions)?;

        let seq = next_seq(&tx, group_id)?;
        let rev = next_rev(&tx, group_id)?;
        tx.execute(
            "INSERT INTO chat_group_messages
               (group_id, seq, rev, author_kind, text, mentions, created_ms)
             VALUES (?1, ?2, ?3, 'human', ?4, ?5, ?6)",
            params![group_id, seq, rev, text, mentions_json, now],
        )?;
        let human_id = tx.last_insert_rowid();

        let mut placeholder_ids = Vec::with_capacity(mentions.len());
        for member_id in mentions {
            let seq = next_seq(&tx, group_id)?;
            let rev = next_rev(&tx, group_id)?;
            tx.execute(
                "INSERT INTO chat_group_messages
                   (group_id, seq, rev, author_kind, author_member_id, status, created_ms)
                 VALUES (?1, ?2, ?3, 'member', ?4, 'queued', ?5)",
                params![group_id, seq, rev, member_id, now],
            )?;
            placeholder_ids.push(tx.last_insert_rowid());
        }
        tx.commit()?;

        let human = message_by_id(&conn, human_id)?;
        let placeholders = placeholder_ids
            .into_iter()
            .map(|id| message_by_id(&conn, id))
            .collect::<Result<Vec<_>>>()?;
        Ok((human, placeholders))
    }

    pub fn get_message(&self, message_id: i64) -> Result<GroupMessageInfo> {
        let conn = self.conn.lock().expect("db lock");
        message_by_id(&conn, message_id)
    }

    /// 返回 `rev > after_rev` 的消息(按 rev 升序,至多 `limit` 条)和
    /// 本次返回里的最大 rev(没有新变更时原样返回 `after_rev`,不回退)。
    pub fn list_messages_after_rev(
        &self,
        group_id: i64,
        after_rev: i64,
        limit: u32,
    ) -> Result<(Vec<GroupMessageInfo>, i64)> {
        let conn = self.conn.lock().expect("db lock");
        let sql = format!(
            "SELECT {MESSAGE_COLUMNS} FROM chat_group_messages
             WHERE group_id = ?1 AND rev > ?2 ORDER BY rev ASC LIMIT ?3"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![group_id, after_rev, limit as i64], row_to_message)?;
        let messages = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        let latest = messages.iter().map(|m| m.rev).max().unwrap_or(after_rev);
        Ok((messages, latest))
    }

    /// 提示词用的历史:`seq < before_seq`、只含 human 与 `Done` 的 agent 消息,
    /// 按 seq 升序,最多 `HISTORY_FETCH_LIMIT` 条;第二个返回值是更早、未取回
    /// 的条数。
    pub fn history_before(
        &self,
        group_id: i64,
        before_seq: i64,
    ) -> Result<(Vec<GroupMessageInfo>, usize)> {
        let conn = self.conn.lock().expect("db lock");
        let eligible = "group_id = ?1 AND seq < ?2 AND (author_kind = 'human' OR status = 'done')";
        let total: i64 = conn.query_row(
            &format!("SELECT COUNT(*) FROM chat_group_messages WHERE {eligible}"),
            params![group_id, before_seq],
            |r| r.get(0),
        )?;
        let sql = format!(
            "SELECT {MESSAGE_COLUMNS} FROM chat_group_messages WHERE {eligible}
             ORDER BY seq DESC LIMIT ?3"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(
            params![group_id, before_seq, HISTORY_FETCH_LIMIT],
            row_to_message,
        )?;
        let mut messages = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        messages.reverse();
        let omitted = (total as usize).saturating_sub(messages.len());
        Ok((messages, omitted))
    }

    pub fn last_human_before(
        &self,
        group_id: i64,
        before_seq: i64,
    ) -> Result<Option<GroupMessageInfo>> {
        let conn = self.conn.lock().expect("db lock");
        let sql = format!(
            "SELECT {MESSAGE_COLUMNS} FROM chat_group_messages
             WHERE group_id = ?1 AND seq < ?2 AND author_kind = 'human'
             ORDER BY seq DESC LIMIT 1"
        );
        Ok(conn
            .query_row(&sql, params![group_id, before_seq], row_to_message)
            .optional()?)
    }

    /// 条件更新:仅当当前状态在 `from` 里才改。返回是否真的改了一行。
    fn transition(
        &self,
        message_id: i64,
        from: &[&str],
        to: &GroupMessageStatus,
        text: Option<&str>,
        duration_ms: Option<u64>,
    ) -> Result<bool> {
        let conn = self.conn.lock().expect("db lock");
        let group_id: i64 = match conn
            .query_row(
                "SELECT group_id FROM chat_group_messages WHERE id = ?1",
                [message_id],
                |r| r.get(0),
            )
            .optional()?
        {
            Some(g) => g,
            None => return Ok(false),
        };
        let (to_status, reason) = status_parts(to);
        let rev = next_rev(&conn, group_id)?;
        let placeholders = from.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "UPDATE chat_group_messages
             SET status = ?1, fail_reason = ?2, rev = ?3,
                 text = COALESCE(?4, text), duration_ms = ?5
             WHERE id = ?6 AND status IN ({placeholders})"
        );
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = vec![
            Box::new(to_status.to_string()),
            Box::new(reason.map(str::to_string)),
            Box::new(rev),
            Box::new(text.map(str::to_string)),
            Box::new(duration_ms.map(|v| v as i64)),
            Box::new(message_id),
        ];
        for f in from {
            args.push(Box::new(f.to_string()));
        }
        let refs: Vec<&dyn rusqlite::ToSql> = args.iter().map(|b| b.as_ref()).collect();
        Ok(conn.execute(&sql, refs.as_slice())? > 0)
    }

    /// `Queued → Running`;不是 `Queued`(已被取消等)返回 `None`。
    pub fn try_start(&self, message_id: i64) -> Result<Option<GroupMessageInfo>> {
        if !self.transition(
            message_id,
            &["queued"],
            &GroupMessageStatus::Running,
            None,
            None,
        )? {
            return Ok(None);
        }
        self.get_message(message_id).map(Some)
    }

    pub fn finish(&self, message_id: i64, text: &str, duration_ms: u64) -> Result<bool> {
        self.transition(
            message_id,
            &["running"],
            &GroupMessageStatus::Done,
            Some(text),
            Some(duration_ms),
        )
    }

    /// `Queued`/`Running → Failed`。`Done`/`Cancelled` 不会被覆盖。
    pub fn fail(&self, message_id: i64, reason: &str, duration_ms: Option<u64>) -> Result<bool> {
        self.transition(
            message_id,
            &["queued", "running"],
            &GroupMessageStatus::Failed {
                reason: reason.to_string(),
            },
            None,
            duration_ms,
        )
    }

    pub fn cancel_running(&self, message_id: i64) -> Result<bool> {
        self.transition(
            message_id,
            &["running"],
            &GroupMessageStatus::Cancelled,
            None,
            None,
        )
    }

    /// 把本群全部 `Queued` 置 `Cancelled`,返回条数。
    pub fn cancel_queued(&self, group_id: i64) -> Result<usize> {
        let ids: Vec<i64> = {
            let conn = self.conn.lock().expect("db lock");
            let mut stmt = conn.prepare(
                "SELECT id FROM chat_group_messages WHERE group_id = ?1 AND status = 'queued'",
            )?;
            let rows = stmt.query_map([group_id], |r| r.get(0))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let mut n = 0;
        for id in ids {
            if self.transition(id, &["queued"], &GroupMessageStatus::Cancelled, None, None)? {
                n += 1;
            }
        }
        Ok(n)
    }

    /// `Failed`/`Cancelled → Queued`,清空旧正文/耗时/失败原因。
    pub fn requeue(&self, message_id: i64) -> Result<GroupMessageInfo> {
        {
            let conn = self.conn.lock().expect("db lock");
            let group_id: i64 = conn
                .query_row(
                    "SELECT group_id FROM chat_group_messages WHERE id = ?1",
                    [message_id],
                    |r| r.get(0),
                )
                .optional()?
                .ok_or_else(|| anyhow::anyhow!("消息不存在: id={message_id}"))?;
            let rev = next_rev(&conn, group_id)?;
            let affected = conn.execute(
                "UPDATE chat_group_messages
                 SET status = 'queued', fail_reason = NULL, text = '', duration_ms = NULL, rev = ?1
                 WHERE id = ?2 AND status IN ('failed', 'cancelled') AND author_kind = 'member'",
                params![rev, message_id],
            )?;
            if affected == 0 {
                bail!("只有失败或已取消的 agent 发言可以重试");
            }
        }
        self.get_message(message_id)
    }

    /// 启动恢复:库里遗留的 `Queued`/`Running` 全部置 `Failed`(进程已经没了)。
    pub fn recover_interrupted(&self) -> Result<usize> {
        let ids: Vec<i64> = {
            let conn = self.conn.lock().expect("db lock");
            let mut stmt = conn.prepare(
                "SELECT id FROM chat_group_messages WHERE status IN ('queued', 'running')",
            )?;
            let rows = stmt.query_map([], |r| r.get(0))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let mut n = 0;
        for id in ids {
            if self.fail(id, INTERRUPTED_REASON, None)? {
                n += 1;
            }
        }
        Ok(n)
    }

    /// 记录"这条消息已转为待办"。已有关联则报错(一条消息只转一次)。
    pub fn set_todo_link(&self, message_id: i64, todo_id: i64) -> Result<()> {
        let conn = self.conn.lock().expect("db lock");
        let affected = conn.execute(
            "UPDATE chat_group_messages SET todo_id = ?1 WHERE id = ?2 AND todo_id IS NULL",
            params![todo_id, message_id],
        )?;
        if affected == 0 {
            bail!("该消息不存在或已转为待办");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, GroupStore) {
        let dir = tempfile::tempdir().unwrap();
        let s = GroupStore::new(&dir.path().join("g.db")).unwrap();
        (dir, s)
    }

    fn group_with_two(s: &GroupStore) -> (i64, i64, i64) {
        let g = s.create_group(1, "评审登录方案").unwrap();
        let g = s.add_member(g.id, AgentKind::Claude, "claude", "").unwrap();
        let g = s
            .add_member(g.id, AgentKind::Codex, "codex", "挑刺的审阅者")
            .unwrap();
        (g.id, g.members[0].id, g.members[1].id)
    }

    #[test]
    fn create_group_and_members_roundtrip() {
        let (_d, s) = store();
        let (gid, m1, _m2) = group_with_two(&s);
        let g = s.get_group(gid).unwrap();
        assert_eq!(g.topic, "评审登录方案");
        assert_eq!(g.members.len(), 2);
        assert_eq!(g.members[1].role_prompt, "挑刺的审阅者");
        assert_eq!(s.list_groups(1).unwrap().len(), 1);
        assert!(s.list_groups(2).unwrap().is_empty(), "项目隔离");
        assert_eq!(s.get_member(m1).unwrap().unwrap().handle, "claude");
    }

    #[test]
    fn empty_topic_rejected() {
        let (_d, s) = store();
        assert!(s.create_group(1, "   ").is_err());
    }

    #[test]
    fn handle_unique_ignoring_case_and_validated() {
        let (_d, s) = store();
        let (gid, m1, _) = group_with_two(&s);
        assert!(s.add_member(gid, AgentKind::Claude, "CLAUDE", "").is_err());
        assert!(s.add_member(gid, AgentKind::Claude, "a b", "").is_err());
        assert!(
            s.update_member(m1, "codex", "").is_err(),
            "改名撞已有 handle"
        );
        // 同一个成员原名更新(只改角色)不算冲突
        assert!(s.update_member(m1, "claude", "新角色").is_ok());
    }

    #[test]
    fn only_claude_and_codex_may_join() {
        let (_d, s) = store();
        let g = s.create_group(1, "t").unwrap();
        assert!(s.add_member(g.id, AgentKind::Goose, "goose", "").is_err());
        assert!(s.add_member(g.id, AgentKind::Unknown, "x", "").is_err());
    }

    #[test]
    fn post_human_message_creates_contiguous_queued_placeholders() {
        let (_d, s) = store();
        let (gid, m1, m2) = group_with_two(&s);
        let (human, ph) = s
            .post_human_message(gid, "@codex @claude 看下", &[m2, m1])
            .unwrap();
        assert_eq!(human.seq, 1);
        assert_eq!(human.author, GroupAuthor::Human);
        assert_eq!(human.status, None);
        assert_eq!(human.mentions, vec![m2, m1]);
        assert_eq!(ph.len(), 2);
        assert_eq!(ph[0].seq, 2);
        assert_eq!(ph[1].seq, 3);
        assert_eq!(ph[0].author, GroupAuthor::Member { member_id: m2 });
        assert_eq!(ph[0].status, Some(GroupMessageStatus::Queued));

        // 排队期间 human 又发一条:seq 排在占位之后,不会插到前面
        let (human2, _) = s.post_human_message(gid, "补充", &[]).unwrap();
        assert_eq!(human2.seq, 4);
    }

    #[test]
    fn post_rejects_member_from_other_group() {
        let (_d, s) = store();
        let (gid, _, _) = group_with_two(&s);
        let other = s.create_group(1, "别的群").unwrap();
        let other = s.add_member(other.id, AgentKind::Claude, "x", "").unwrap();
        assert!(
            s.post_human_message(gid, "hi", &[other.members[0].id])
                .is_err()
        );
    }

    #[test]
    fn status_machine_start_finish_and_guards() {
        let (_d, s) = store();
        let (gid, m1, _) = group_with_two(&s);
        let (_, ph) = s.post_human_message(gid, "@claude hi", &[m1]).unwrap();
        let id = ph[0].id;

        assert!(!s.finish(id, "x", 1).unwrap(), "Queued 不能直接 finish");
        let started = s.try_start(id).unwrap().unwrap();
        assert_eq!(started.status, Some(GroupMessageStatus::Running));
        assert!(
            s.try_start(id).unwrap().is_none(),
            "已 Running 不能再 start"
        );

        assert!(s.finish(id, "回复正文", 120).unwrap());
        let m = s.get_message(id).unwrap();
        assert_eq!(m.status, Some(GroupMessageStatus::Done));
        assert_eq!(m.text, "回复正文");
        assert_eq!(m.duration_ms, Some(120));
        assert!(!s.fail(id, "晚到的失败", None).unwrap(), "Done 不会被覆盖");
    }

    #[test]
    fn cancel_queued_and_requeue() {
        let (_d, s) = store();
        let (gid, m1, m2) = group_with_two(&s);
        let (_, ph) = s
            .post_human_message(gid, "@claude @codex", &[m1, m2])
            .unwrap();
        assert_eq!(s.cancel_queued(gid).unwrap(), 2);
        let m = s.get_message(ph[0].id).unwrap();
        assert_eq!(m.status, Some(GroupMessageStatus::Cancelled));

        let again = s.requeue(ph[0].id).unwrap();
        assert_eq!(again.status, Some(GroupMessageStatus::Queued));
        assert!(again.rev > m.rev, "requeue 要推进 rev");
        assert!(s.requeue(ph[0].id).is_err(), "Queued 不能再 requeue");
    }

    #[test]
    fn requeue_clears_previous_failure() {
        let (_d, s) = store();
        let (gid, m1, _) = group_with_two(&s);
        let (_, ph) = s.post_human_message(gid, "@claude", &[m1]).unwrap();
        s.try_start(ph[0].id).unwrap();
        s.fail(ph[0].id, "超时", Some(5)).unwrap();
        let q = s.requeue(ph[0].id).unwrap();
        assert_eq!(q.status, Some(GroupMessageStatus::Queued));
        assert_eq!(q.text, "");
        assert_eq!(q.duration_ms, None);
    }

    #[test]
    fn list_after_rev_includes_updated_old_messages() {
        let (_d, s) = store();
        let (gid, m1, _) = group_with_two(&s);
        let (human, ph) = s.post_human_message(gid, "@claude", &[m1]).unwrap();
        let (all, latest) = s.list_messages_after_rev(gid, 0, 100).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(latest, all.iter().map(|m| m.rev).max().unwrap());

        s.try_start(ph[0].id).unwrap();
        s.finish(ph[0].id, "好", 10).unwrap();
        let (delta, latest2) = s.list_messages_after_rev(gid, latest, 100).unwrap();
        assert_eq!(delta.len(), 1, "只有变更过的那条");
        assert_eq!(delta[0].id, ph[0].id);
        assert!(latest2 > latest);
        assert_ne!(delta[0].id, human.id);

        let (none, same) = s.list_messages_after_rev(gid, latest2, 100).unwrap();
        assert!(none.is_empty());
        assert_eq!(same, latest2, "没有新变更时 latest_rev 不回退");
    }

    #[test]
    fn history_before_keeps_human_and_done_only_in_seq_order() {
        let (_d, s) = store();
        let (gid, m1, m2) = group_with_two(&s);
        let (_h, ph) = s
            .post_human_message(gid, "@claude @codex 议题", &[m1, m2])
            .unwrap();
        s.try_start(ph[0].id).unwrap();
        s.finish(ph[0].id, "claude 观点", 1).unwrap();
        s.try_start(ph[1].id).unwrap();
        s.fail(ph[1].id, "超时", None).unwrap();

        let (hist, omitted) = s.history_before(gid, ph[1].seq + 1).unwrap();
        let texts: Vec<_> = hist.iter().map(|m| m.text.as_str()).collect();
        assert_eq!(
            texts,
            vec!["@claude @codex 议题", "claude 观点"],
            "Failed 不进历史"
        );
        assert_eq!(omitted, 0);

        let (hist, _) = s.history_before(gid, ph[0].seq).unwrap();
        assert_eq!(hist.len(), 1, "只含 seq 更小的");
    }

    #[test]
    fn last_human_before_finds_the_trigger() {
        let (_d, s) = store();
        let (gid, m1, _) = group_with_two(&s);
        let (h1, ph) = s.post_human_message(gid, "第一问 @claude", &[m1]).unwrap();
        let (_h2, _) = s.post_human_message(gid, "第二问", &[]).unwrap();
        let t = s.last_human_before(gid, ph[0].seq).unwrap().unwrap();
        assert_eq!(t.id, h1.id);
    }

    #[test]
    fn recover_interrupted_fails_queued_and_running() {
        let (_d, s) = store();
        let (gid, m1, m2) = group_with_two(&s);
        let (_, ph) = s
            .post_human_message(gid, "@claude @codex", &[m1, m2])
            .unwrap();
        s.try_start(ph[0].id).unwrap();
        assert_eq!(s.recover_interrupted().unwrap(), 2);
        for p in &ph {
            match s.get_message(p.id).unwrap().status {
                Some(GroupMessageStatus::Failed { reason }) => assert!(reason.contains("重启")),
                other => panic!("应为 Failed,实际 {other:?}"),
            }
        }
        assert_eq!(s.recover_interrupted().unwrap(), 0, "幂等");
    }

    #[test]
    fn todo_link_set_once() {
        let (_d, s) = store();
        let (gid, _, _) = group_with_two(&s);
        let (h, _) = s.post_human_message(gid, "做这个", &[]).unwrap();
        s.set_todo_link(h.id, 7).unwrap();
        assert_eq!(s.get_message(h.id).unwrap().todo_id, Some(7));
        assert!(s.set_todo_link(h.id, 8).is_err(), "已转待办不能再转");
    }

    #[test]
    fn delete_group_removes_members_and_messages() {
        let (_d, s) = store();
        let (gid, m1, _) = group_with_two(&s);
        let (_, ph) = s.post_human_message(gid, "@claude", &[m1]).unwrap();
        s.delete_group(gid).unwrap();
        assert!(s.get_group(gid).is_err());
        assert!(s.get_message(ph[0].id).is_err());
        assert!(s.delete_group(gid).is_err(), "不存在要报错,不静默");
    }

    #[test]
    fn remove_member_keeps_their_old_messages() {
        let (_d, s) = store();
        let (gid, m1, _) = group_with_two(&s);
        let (_, ph) = s.post_human_message(gid, "@claude", &[m1]).unwrap();
        let g = s.remove_member(m1).unwrap();
        assert_eq!(g.members.len(), 1);
        assert!(s.get_message(ph[0].id).is_ok());
        assert!(s.get_member(m1).unwrap().is_none());
    }
}
