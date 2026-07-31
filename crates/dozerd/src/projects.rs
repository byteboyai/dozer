//! 项目存储：rusqlite（spec P1g D1）。projects 表（不再有活跃项目指针，
//! P2a 起 daemon 变成纯会话仓库）。
//! 与 AcceptanceStore 各持一个到 dozer.db 的连接；项目/验收写频度极低，
//! 多连接足够（无需连接池）。

use anyhow::{Context, Result};
use dozer_core::protocol::ProjectInfo;
use rusqlite::Connection;
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

/// 严格单调的"活跃时间戳"：至少比现有最大值大 1。毫秒时钟分辨率下同一
/// 毫秒内的多次激活也能有确定的先后（否则 `ORDER BY last_active_ms` 平局）。
fn next_active_stamp(conn: &Connection) -> u64 {
    let max_existing: u64 = conn
        .query_row(
            "SELECT COALESCE(MAX(last_active_ms), 0) FROM projects",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map(|v| v as u64)
        .unwrap_or(0);
    now_ms().max(max_existing + 1)
}

impl ProjectStore {
    pub fn new(path: &Path) -> Result<Self> {
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
             );",
        )
        .context("建表")?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// upsert（按 path）+ 刷新活跃时间，返回该项目。P2a 起不再"置为当前
    /// 项目"——daemon 没有这个概念了，"当前显示哪个"是 GUI 侧本地状态。
    pub fn open(&self, path: &str) -> Result<ProjectInfo> {
        let conn = self.conn.lock().expect("db lock");
        let ts = next_active_stamp(&conn);
        conn.execute(
            "INSERT INTO projects (path, name, last_active_ms) VALUES (?1, ?2, ?3)
             ON CONFLICT(path) DO UPDATE SET last_active_ms = ?3",
            rusqlite::params![path, basename(path), ts],
        )?;
        conn.query_row(
            "SELECT id, path, name, last_active_ms FROM projects WHERE path = ?1",
            [path],
            row_to_project,
        )
        .map_err(Into::into)
    }

    pub fn list(&self) -> Result<Vec<ProjectInfo>> {
        let conn = self.conn.lock().expect("db lock");
        let mut stmt = conn.prepare(
            "SELECT id, path, name, last_active_ms FROM projects ORDER BY last_active_ms DESC",
        )?;
        let rows = stmt.query_map([], row_to_project)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
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
    fn open_list_and_persist() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.db");
        let store = ProjectStore::new(&db).unwrap();
        assert!(store.list().unwrap().is_empty());

        let a = store.open("/repo/a").unwrap();
        assert_eq!(a.name, "a");

        // 同 path 再开:不新增,复用同 id,活跃时间刷新
        let a2 = store.open("/repo/a").unwrap();
        assert_eq!(a2.id, a.id);
        assert_eq!(store.list().unwrap().len(), 1);

        // 开第二个:两条都在,最近活跃的在前
        let b = store.open("/repo/b").unwrap();
        let list = store.list().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, b.id, "最近活跃在前");

        // 重开库:列表持久化
        drop(store);
        let store = ProjectStore::new(&db).unwrap();
        assert_eq!(store.list().unwrap().len(), 2);
    }

    #[test]
    fn name_is_basename() {
        let dir = tempfile::tempdir().unwrap();
        let store = ProjectStore::new(&dir.path().join("t.db")).unwrap();
        assert_eq!(store.open("/a/b/proj").unwrap().name, "proj");
        assert_eq!(store.open("/").unwrap().name, "/");
    }
}
