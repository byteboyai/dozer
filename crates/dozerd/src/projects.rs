//! 项目存储：rusqlite（spec P1g D1）。projects 表 + meta 表（当前项目指针）。
//! 与 AcceptanceStore 各持一个到 dozer.db 的连接；项目/验收写频度极低，
//! 多连接足够（无需连接池）。

use anyhow::{Context, Result};
use dozer_core::protocol::ProjectInfo;
use rusqlite::{Connection, OptionalExtension};
use std::path::Path;
use std::sync::Mutex;

pub struct ProjectStore {
    conn: Mutex<Connection>,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn basename(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

impl ProjectStore {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("建库目录")?;
        }
        let conn = Connection::open(path).context("打开 SQLite")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS projects (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                path TEXT NOT NULL UNIQUE,
                name TEXT NOT NULL,
                last_active_ms INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
             );",
        )
        .context("建表")?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// upsert（按 path）+ 刷新活跃时间 + 置为当前项目，返回该项目。
    pub fn open_and_activate(&self, path: &str) -> Result<ProjectInfo> {
        let conn = self.conn.lock().expect("db lock");
        let ts = now_ms();
        conn.execute(
            "INSERT INTO projects (path, name, last_active_ms) VALUES (?1, ?2, ?3)
             ON CONFLICT(path) DO UPDATE SET last_active_ms = ?3",
            rusqlite::params![path, basename(path), ts],
        )?;
        let info = conn.query_row(
            "SELECT id, path, name, last_active_ms FROM projects WHERE path = ?1",
            [path],
            row_to_project,
        )?;
        conn.execute(
            "INSERT INTO meta (key, value) VALUES ('active_project_id', ?1)
             ON CONFLICT(key) DO UPDATE SET value = ?1",
            [info.id.to_string()],
        )?;
        Ok(info)
    }

    pub fn list(&self) -> Result<Vec<ProjectInfo>> {
        let conn = self.conn.lock().expect("db lock");
        let mut stmt = conn.prepare(
            "SELECT id, path, name, last_active_ms FROM projects ORDER BY last_active_ms DESC",
        )?;
        let rows = stmt.query_map([], row_to_project)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn set_active(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock().expect("db lock");
        conn.execute(
            "INSERT INTO meta (key, value) VALUES ('active_project_id', ?1)
             ON CONFLICT(key) DO UPDATE SET value = ?1",
            [id.to_string()],
        )?;
        conn.execute(
            "UPDATE projects SET last_active_ms = ?1 WHERE id = ?2",
            rusqlite::params![now_ms(), id],
        )?;
        Ok(())
    }

    pub fn active(&self) -> Result<Option<ProjectInfo>> {
        let conn = self.conn.lock().expect("db lock");
        let id: Option<i64> = conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'active_project_id'",
                [],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .and_then(|s| s.parse().ok());
        let Some(id) = id else { return Ok(None) };
        conn.query_row(
            "SELECT id, path, name, last_active_ms FROM projects WHERE id = ?1",
            [id],
            row_to_project,
        )
        .optional()
        .map_err(Into::into)
    }
}

fn row_to_project(row: &rusqlite::Row) -> rusqlite::Result<ProjectInfo> {
    Ok(ProjectInfo {
        id: row.get(0)?,
        path: row.get(1)?,
        name: row.get(2)?,
        last_active_ms: row.get::<_, i64>(3)? as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_activate_list_and_persist() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.db");
        let store = ProjectStore::open(&db).unwrap();
        assert!(store.active().unwrap().is_none());
        assert!(store.list().unwrap().is_empty());

        let a = store.open_and_activate("/repo/a").unwrap();
        assert_eq!(a.name, "a");
        assert_eq!(store.active().unwrap().unwrap().id, a.id);

        // 同 path 再开:不新增,复用同 id,活跃时间刷新
        let a2 = store.open_and_activate("/repo/a").unwrap();
        assert_eq!(a2.id, a.id);
        assert_eq!(store.list().unwrap().len(), 1);

        // 开第二个 → 成为当前;list 倒序 b 在前
        let b = store.open_and_activate("/repo/b").unwrap();
        assert_eq!(store.active().unwrap().unwrap().id, b.id);
        let list = store.list().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, b.id, "最近活跃在前");

        // 切回 a
        store.set_active(a.id).unwrap();
        assert_eq!(store.active().unwrap().unwrap().id, a.id);

        // 重开库:当前项目与列表持久化
        drop(store);
        let store = ProjectStore::open(&db).unwrap();
        assert_eq!(store.active().unwrap().unwrap().id, a.id);
        assert_eq!(store.list().unwrap().len(), 2);
    }

    #[test]
    fn name_is_basename() {
        let dir = tempfile::tempdir().unwrap();
        let store = ProjectStore::open(&dir.path().join("t.db")).unwrap();
        assert_eq!(store.open_and_activate("/a/b/proj").unwrap().name, "proj");
        assert_eq!(store.open_and_activate("/").unwrap().name, "/");
    }
}
