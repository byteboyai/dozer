//! 共享记忆存储:project 级 SQLite,`memories`(现状)+ `memory_history`
//! (审计轨迹)两张表,与 `TodoStore` 共享同一个 `dozer.db`。
//! 详见设计 `docs/superpowers/specs/2026-09-23-agent-memory-sharing-design.md`。

use anyhow::{Context, Result};
use dozer_core::protocol::{MemoryDetail, MemoryHistoryEntry, MemoryInfo};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;
use std::sync::Mutex;

pub struct MemoryStore {
    conn: Mutex<Connection>,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn id_not_found(id: i64) -> anyhow::Error {
    anyhow::anyhow!("记忆不存在: id={id}")
}

/// `get` 的行形状:(title, kind, description, body, created_ms, created_by,
/// updated_ms, updated_by)。抽成别名避免 clippy `type_complexity`。
type MemoryRow = (String, String, String, String, i64, String, i64, String);

impl MemoryStore {
    pub fn new(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("建库目录")?;
        }
        let conn = Connection::open(path).context("打开 SQLite")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS memories (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                project_id INTEGER NOT NULL,
                title TEXT NOT NULL,
                kind TEXT NOT NULL,
                description TEXT NOT NULL DEFAULT '',
                body TEXT NOT NULL,
                created_ms INTEGER NOT NULL,
                created_by TEXT NOT NULL,
                updated_ms INTEGER NOT NULL,
                updated_by TEXT NOT NULL
             );
             CREATE UNIQUE INDEX IF NOT EXISTS memories_project_title
                ON memories(project_id, title);
             CREATE TABLE IF NOT EXISTS memory_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                memory_id INTEGER NOT NULL,
                project_id INTEGER NOT NULL,
                changed_ms INTEGER NOT NULL,
                changed_by TEXT NOT NULL,
                change_kind TEXT NOT NULL,
                title_snapshot TEXT NOT NULL,
                kind_snapshot TEXT NOT NULL,
                description_snapshot TEXT NOT NULL,
                body_snapshot TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS memory_history_memory
                ON memory_history(memory_id, changed_ms);",
        )
        .context("建表")?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn list(&self, project_id: i64) -> Result<Vec<MemoryInfo>> {
        let conn = self.conn.lock().expect("db lock");
        let mut stmt = conn.prepare(
            "SELECT id, project_id, title, kind, description, updated_ms, updated_by
             FROM memories WHERE project_id = ?1 ORDER BY updated_ms DESC",
        )?;
        let rows = stmt.query_map([project_id], |r| {
            Ok(MemoryInfo {
                id: r.get(0)?,
                project_id: r.get(1)?,
                title: r.get(2)?,
                kind: r.get(3)?,
                description: r.get(4)?,
                updated_ms: r.get::<_, i64>(5)? as u64,
                updated_by: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn get(&self, project_id: i64, id: i64) -> Result<MemoryDetail> {
        let conn = self.conn.lock().expect("db lock");
        let row: Option<MemoryRow> = conn
            .query_row(
                "SELECT title, kind, description, body, created_ms, created_by, updated_ms, updated_by
                 FROM memories WHERE id = ?1 AND project_id = ?2",
                params![id, project_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?)),
            )
            .optional()?;
        let Some((title, kind, description, body, created_ms, created_by, updated_ms, updated_by)) =
            row
        else {
            return Err(id_not_found(id));
        };
        let history = history_query(&conn, project_id, id, 20)?;
        Ok(MemoryDetail {
            id,
            project_id,
            title,
            kind,
            description,
            body,
            created_ms: created_ms as u64,
            created_by,
            updated_ms: updated_ms as u64,
            updated_by,
            history,
        })
    }

    pub fn write(
        &self,
        project_id: i64,
        title: &str,
        kind: &str,
        description: &str,
        body: &str,
        actor: &str,
    ) -> Result<MemoryDetail> {
        let conn = self.conn.lock().expect("db lock");
        let now = now_ms() as i64;
        let existing_id: Option<i64> = conn
            .query_row(
                "SELECT id FROM memories WHERE project_id = ?1 AND title = ?2",
                params![project_id, title],
                |r| r.get(0),
            )
            .optional()?;
        let change_kind = if existing_id.is_some() {
            "updated"
        } else {
            "created"
        };
        let sql = "INSERT INTO memories
                (project_id, title, kind, description, body, created_ms, created_by, updated_ms, updated_by)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?6, ?7)
             ON CONFLICT(project_id, title) DO UPDATE SET
                kind = excluded.kind,
                description = excluded.description,
                body = excluded.body,
                updated_ms = excluded.updated_ms,
                updated_by = excluded.updated_by
             RETURNING id, created_ms, created_by";
        let (id, created_ms, created_by): (i64, i64, String) = conn.query_row(
            sql,
            params![project_id, title, kind, description, body, now, actor],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        conn.execute(
            "INSERT INTO memory_history
                (memory_id, project_id, changed_ms, changed_by, change_kind,
                 title_snapshot, kind_snapshot, description_snapshot, body_snapshot)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                id,
                project_id,
                now,
                actor,
                change_kind,
                title,
                kind,
                description,
                body
            ],
        )?;
        Ok(MemoryDetail {
            id,
            project_id,
            title: title.to_string(),
            kind: kind.to_string(),
            description: description.to_string(),
            body: body.to_string(),
            created_ms: created_ms as u64,
            created_by,
            updated_ms: now as u64,
            updated_by: actor.to_string(),
            history: Vec::new(),
        })
    }

    pub fn delete(&self, project_id: i64, id: i64, actor: &str) -> Result<()> {
        let conn = self.conn.lock().expect("db lock");
        let now = now_ms() as i64;
        let row: Option<(String, String, String, String)> = conn
            .query_row(
                "SELECT title, kind, description, body FROM memories WHERE id = ?1 AND project_id = ?2",
                params![id, project_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        let Some((title, kind, description, body)) = row else {
            return Err(id_not_found(id));
        };
        conn.execute(
            "INSERT INTO memory_history
                (memory_id, project_id, changed_ms, changed_by, change_kind,
                 title_snapshot, kind_snapshot, description_snapshot, body_snapshot)
             VALUES (?1, ?2, ?3, ?4, 'deleted', ?5, ?6, ?7, ?8)",
            params![id, project_id, now, actor, title, kind, description, body],
        )?;
        conn.execute("DELETE FROM memories WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn history(
        &self,
        project_id: i64,
        memory_id: i64,
        limit: i64,
    ) -> Result<Vec<MemoryHistoryEntry>> {
        let conn = self.conn.lock().expect("db lock");
        history_query(&conn, project_id, memory_id, limit)
    }
}

/// `get`(已持锁)和公开的 `history`(自己上锁)共用的查询体,避免
/// `Mutex` 二次加锁死锁。
fn history_query(
    conn: &Connection,
    project_id: i64,
    memory_id: i64,
    limit: i64,
) -> Result<Vec<MemoryHistoryEntry>> {
    let mut stmt = conn.prepare(
        "SELECT id, memory_id, changed_ms, changed_by, change_kind,
                title_snapshot, kind_snapshot, description_snapshot, body_snapshot
         FROM memory_history
         WHERE project_id = ?1 AND memory_id = ?2
         ORDER BY changed_ms DESC LIMIT ?3",
    )?;
    let rows = stmt.query_map(params![project_id, memory_id, limit], |r| {
        Ok(MemoryHistoryEntry {
            id: r.get(0)?,
            memory_id: r.get(1)?,
            changed_ms: r.get::<_, i64>(2)? as u64,
            changed_by: r.get(3)?,
            change_kind: r.get(4)?,
            title: r.get(5)?,
            kind: r.get(6)?,
            description: r.get(7)?,
            body: r.get(8)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, MemoryStore) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("m.db");
        let store = MemoryStore::new(&db).unwrap();
        (dir, store)
    }

    #[test]
    fn write_creates_then_updates_same_title() {
        let (_dir, store) = store();
        let created = store
            .write(1, "标题A", "project", "desc", "正文一", "claude")
            .unwrap();
        assert_eq!(created.body, "正文一");
        assert_eq!(created.created_by, "claude");

        let updated = store
            .write(1, "标题A", "project", "desc2", "正文二", "user")
            .unwrap();
        assert_eq!(updated.id, created.id, "同标题应该更新同一条,不新建");
        assert_eq!(updated.body, "正文二");
        assert_eq!(updated.updated_by, "user");

        let listed = store.list(1).unwrap();
        assert_eq!(listed.len(), 1, "upsert 不应产生第二行");
    }

    #[test]
    fn write_records_history_for_create_and_update() {
        let (_dir, store) = store();
        store
            .write(1, "标题A", "project", "d", "v1", "claude")
            .unwrap();
        store
            .write(1, "标题A", "project", "d", "v2", "codebuddy")
            .unwrap();
        let detail = store.get(1, 1).unwrap();
        let history = store.history(1, detail.id, 10).unwrap();
        assert_eq!(history.len(), 2, "两次写入各留一条历史");
        // 倒序:最新的在前。
        assert_eq!(history[0].change_kind, "updated");
        assert_eq!(history[0].body, "v2");
        assert_eq!(history[0].changed_by, "codebuddy");
        assert_eq!(history[1].change_kind, "created");
        assert_eq!(history[1].body, "v1");
    }

    #[test]
    fn empty_title_is_a_valid_independent_key() {
        let (_dir, store) = store();
        let a = store
            .write(1, "", "project", "", "空标题的内容", "claude")
            .unwrap();
        let b = store
            .write(1, "标题B", "project", "", "另一条", "claude")
            .unwrap();
        assert_ne!(a.id, b.id);
        assert_eq!(store.list(1).unwrap().len(), 2);
        // 空标题第二次写入应该更新同一条(和非空标题的 upsert 语义一致),
        // 不会因为标题是空串就退化成每次都新建。
        let a2 = store
            .write(1, "", "project", "", "空标题改了", "claude")
            .unwrap();
        assert_eq!(a2.id, a.id);
        assert_eq!(
            store.list(1).unwrap().len(),
            2,
            "空标题的 upsert 不应产生第三行"
        );
    }

    #[test]
    fn delete_removes_row_but_history_survives() {
        let (_dir, store) = store();
        let created = store
            .write(1, "标题A", "project", "d", "v1", "claude")
            .unwrap();
        store.delete(1, created.id, "user").unwrap();

        assert!(store.list(1).unwrap().is_empty());
        assert!(store.get(1, created.id).is_err(), "删除后 get 应该报不存在");

        let history = store.history(1, created.id, 10).unwrap();
        assert_eq!(history.len(), 2, "创建 + 删除各一条历史");
        assert_eq!(history[0].change_kind, "deleted");
        assert_eq!(history[0].changed_by, "user");
        assert_eq!(history[0].body, "v1", "删除快照存的是删除前的最后内容");
    }

    #[test]
    fn projects_are_isolated_for_title_get_and_delete() {
        let (_dir, store) = store();
        let a = store
            .write(1, "同名", "project", "", "项目1的内容", "claude")
            .unwrap();
        let b = store
            .write(2, "同名", "project", "", "项目2的内容", "claude")
            .unwrap();
        assert_ne!(a.id, b.id, "不同 project 下同标题不冲突");

        assert!(store.get(2, a.id).is_err(), "不能跨项目读到别的项目的记忆");
        assert!(
            store.delete(2, a.id, "user").is_err(),
            "不能跨项目删掉别的项目的记忆"
        );

        // 确认没有被误删。
        assert_eq!(store.get(1, a.id).unwrap().body, "项目1的内容");
    }

    #[test]
    fn get_and_delete_unknown_id_error() {
        let (_dir, store) = store();
        assert!(store.get(1, 999).is_err());
        assert!(store.delete(1, 999, "user").is_err());
    }

    #[test]
    fn list_orders_by_updated_ms_desc() {
        let (_dir, store) = store();
        let a = store
            .write(1, "先写的", "project", "", "v", "claude")
            .unwrap();
        let b = store
            .write(1, "后写的", "project", "", "v", "claude")
            .unwrap();
        let listed = store.list(1).unwrap();
        assert_eq!(listed[0].id, b.id, "最近更新的排最前");
        assert_eq!(listed[1].id, a.id);
    }
}
