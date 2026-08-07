//! 收藏夹存储:rusqlite,与 `ProjectStore`/`AcceptanceStore` 共享同一个
//! `dozer.db`(见 `main.rs` 里三者各自 `new`/`open` 时传入同一路径)。
//! `bookmarks` 表用两条局部唯一索引分别去重全局/项目收藏——SQLite 的
//! 表级 `UNIQUE` 把多个 NULL 视为互不相同,`project_id` 为 NULL 时不能靠
//! 它给全局收藏去重,必须用 `WHERE scope = 'global'` 的局部索引。

use anyhow::{Context, Result};
use dozer_core::protocol::{BookmarkInfo, BookmarkScope};
use rusqlite::Connection;
use std::path::Path;
use std::sync::Mutex;

pub struct BookmarkStore {
    conn: Mutex<Connection>,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn scope_str(scope: BookmarkScope) -> &'static str {
    match scope {
        BookmarkScope::Global => "global",
        BookmarkScope::Project => "project",
    }
}

fn scope_from_str(s: &str) -> BookmarkScope {
    match s {
        "project" => BookmarkScope::Project,
        _ => BookmarkScope::Global,
    }
}

fn row_to_bookmark(row: &rusqlite::Row) -> rusqlite::Result<BookmarkInfo> {
    let scope: String = row.get(1)?;
    Ok(BookmarkInfo {
        id: row.get(0)?,
        scope: scope_from_str(&scope),
        project_id: row.get(2)?,
        url: row.get(3)?,
        title: row.get(4)?,
        created_ms: row.get::<_, i64>(5)? as u64,
    })
}

impl BookmarkStore {
    pub fn new(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("建库目录")?;
        }
        let conn = Connection::open(path).context("打开 SQLite")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS bookmarks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                scope TEXT NOT NULL CHECK(scope IN ('global', 'project')),
                project_id INTEGER,
                url TEXT NOT NULL,
                title TEXT NOT NULL,
                created_ms INTEGER NOT NULL
             );
             CREATE UNIQUE INDEX IF NOT EXISTS ux_bookmarks_global
                ON bookmarks(url) WHERE scope = 'global';
             CREATE UNIQUE INDEX IF NOT EXISTS ux_bookmarks_project
                ON bookmarks(project_id, url) WHERE scope = 'project';",
        )
        .context("建表")?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// 幂等添加:`INSERT OR IGNORE` 命中已存在行时不报错、不改标题,
    /// 用 `scope+project_id+url`(`IS` 做 NULL 安全比较)查回那条已有记录。
    pub fn add(
        &self,
        scope: BookmarkScope,
        project_id: Option<i64>,
        url: &str,
        title: &str,
    ) -> Result<BookmarkInfo> {
        let conn = self.conn.lock().expect("db lock");
        let ts = now_ms() as i64;
        conn.execute(
            "INSERT OR IGNORE INTO bookmarks (scope, project_id, url, title, created_ms)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![scope_str(scope), project_id, url, title, ts],
        )?;
        conn.query_row(
            "SELECT id, scope, project_id, url, title, created_ms FROM bookmarks
             WHERE scope = ?1 AND project_id IS ?2 AND url = ?3",
            rusqlite::params![scope_str(scope), project_id, url],
            row_to_bookmark,
        )
        .map_err(Into::into)
    }

    pub fn remove(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock().expect("db lock");
        conn.execute("DELETE FROM bookmarks WHERE id = ?1", [id])?;
        Ok(())
    }

    /// 全局收藏 ∪(`project_id` 给定时)该项目的收藏,按加入时间升序。
    /// `project_id = None` 时 `project_id IS ?1` 恒假,天然只剩全局收藏,
    /// 不需要额外分支。
    pub fn list(&self, project_id: Option<i64>) -> Result<Vec<BookmarkInfo>> {
        let conn = self.conn.lock().expect("db lock");
        let mut stmt = conn.prepare(
            "SELECT id, scope, project_id, url, title, created_ms FROM bookmarks
             WHERE scope = 'global' OR (scope = 'project' AND project_id IS ?1)
             ORDER BY created_ms ASC",
        )?;
        let rows = stmt.query_map([project_id], row_to_bookmark)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, BookmarkStore) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.db");
        let store = BookmarkStore::new(&db).unwrap();
        (dir, store)
    }

    #[test]
    fn add_list_and_remove() {
        let (_dir, store) = store();
        let b = store
            .add(BookmarkScope::Global, None, "https://a.com", "A")
            .unwrap();
        assert_eq!(b.url, "https://a.com");
        assert_eq!(b.scope, BookmarkScope::Global);
        assert_eq!(b.project_id, None);

        let listed = store.list(None).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, b.id);

        store.remove(b.id).unwrap();
        assert!(store.list(None).unwrap().is_empty());
    }

    #[test]
    fn project_scope_isolated_per_project() {
        let (_dir, store) = store();
        store
            .add(BookmarkScope::Project, Some(1), "https://a.com", "A")
            .unwrap();
        store
            .add(BookmarkScope::Project, Some(2), "https://b.com", "B")
            .unwrap();
        store
            .add(BookmarkScope::Global, None, "https://g.com", "G")
            .unwrap();

        let for_1 = store.list(Some(1)).unwrap();
        assert_eq!(for_1.len(), 2, "全局 + 项目1自己的");
        assert!(for_1.iter().any(|b| b.url == "https://a.com"));
        assert!(for_1.iter().any(|b| b.url == "https://g.com"));
        assert!(
            !for_1.iter().any(|b| b.url == "https://b.com"),
            "不该看到项目2的收藏"
        );

        let none = store.list(None).unwrap();
        assert_eq!(none.len(), 1, "未开项目只看到全局");
        assert_eq!(none[0].url, "https://g.com");
    }

    #[test]
    fn add_is_idempotent_and_does_not_update_title() {
        let (_dir, store) = store();
        let first = store
            .add(BookmarkScope::Global, None, "https://a.com", "A")
            .unwrap();
        let second = store
            .add(
                BookmarkScope::Global,
                None,
                "https://a.com",
                "改了标题也不生效",
            )
            .unwrap();
        assert_eq!(first.id, second.id);
        assert_eq!(second.title, "A");
        assert_eq!(store.list(None).unwrap().len(), 1);
    }

    #[test]
    fn same_url_can_exist_in_both_global_and_project_scope() {
        let (_dir, store) = store();
        store
            .add(BookmarkScope::Global, None, "https://a.com", "A")
            .unwrap();
        store
            .add(BookmarkScope::Project, Some(1), "https://a.com", "A")
            .unwrap();
        assert_eq!(
            store.list(Some(1)).unwrap().len(),
            2,
            "全局和项目各一条,互不去重"
        );
    }

    #[test]
    fn remove_unknown_id_is_noop() {
        let (_dir, store) = store();
        store.remove(9999).unwrap();
    }

    #[test]
    fn list_orders_by_created_ms_ascending() {
        let (_dir, store) = store();
        store
            .add(BookmarkScope::Global, None, "https://first.com", "First")
            .unwrap();
        store
            .add(BookmarkScope::Global, None, "https://second.com", "Second")
            .unwrap();
        let listed = store.list(None).unwrap();
        assert_eq!(listed[0].url, "https://first.com");
        assert_eq!(listed[1].url, "https://second.com");
    }
}
