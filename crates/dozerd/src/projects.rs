//! 项目存储：rusqlite（spec P1g D1）。projects 表（不再有活跃项目指针，
//! P2a 起 daemon 变成纯会话仓库）。
//! 与 AcceptanceStore 各持一个到 dozer.db 的连接；项目/验收写频度极低，
//! 多连接足够（无需连接池）。

use anyhow::{Context, Result};
use dozer_core::protocol::ProjectInfo;
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::process::Command;
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

/// 取 `dir` 所属 git 仓库的根目录（无 git 或 git 不可用时返回 `None`）。
fn git_repo_root(dir: &Path) -> Option<PathBuf> {
    if !dir.is_dir() {
        return None;
    }
    let out = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(dir)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let line = String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()?
        .trim()
        .to_string();
    (!line.is_empty()).then(|| PathBuf::from(line))
}

/// 仓库最新 commit 的提交时间（毫秒）。无 commit / git 不可用 / 解析失败
/// 时返回 `None`。
fn git_head_commit_ms(repo: &Path) -> Option<u64> {
    let out = Command::new("git")
        .args(["log", "-1", "--format=%ct"])
        .current_dir(repo)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let secs: u64 = String::from_utf8_lossy(&out.stdout).trim().parse().ok()?;
    Some(secs * 1000)
}

/// 计算项目的 git 感知更新时间：优先取仓库最新 commit 时间，若该仓库没有
/// commit，或 commit 时间早于 `last_active_ms`，则回落为 `last_active_ms`。
fn compute_updated_ms(path: &str, last_active_ms: u64) -> u64 {
    let commit = git_repo_root(Path::new(path)).and_then(|root| git_head_commit_ms(&root));
    match commit {
        Some(c) if c >= last_active_ms => c,
        _ => last_active_ms,
    }
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
        // 迁移：老库没有 `created_ms` 列时补上。既有行的创建时间未知，
        // 回落为它们已知的 `last_active_ms`（都是首次 upsert 时写入的）。
        let has_col: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('projects') WHERE name = 'created_ms'",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);
        if has_col == 0 {
            conn.execute_batch(
                "ALTER TABLE projects ADD COLUMN created_ms INTEGER NOT NULL DEFAULT 0",
            )
            .context("加 created_ms 列")?;
            conn.execute(
                "UPDATE projects SET created_ms = last_active_ms WHERE created_ms = 0",
                [],
            )
            .context("迁移 created_ms")?;
        }
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// upsert（按 path）+ 刷新活跃时间，返回该项目。P2a 起不再"置为当前
    /// 项目"——daemon 没有这个概念了，"当前显示哪个"是 GUI 侧本地状态。
    /// `created_ms` 仅在首次插入时写入（= 本次 `open` 的活跃戳，即 Dozer
    /// 中新建该项目的时间）；之后 `open` 只刷新 `last_active_ms`。
    pub fn open(&self, path: &str) -> Result<ProjectInfo> {
        let conn = self.conn.lock().expect("db lock");
        let ts = next_active_stamp(&conn);
        conn.execute(
            "INSERT INTO projects (path, name, last_active_ms, created_ms) VALUES (?1, ?2, ?3, ?3)
             ON CONFLICT(path) DO UPDATE SET last_active_ms = ?3",
            rusqlite::params![path, basename(path), ts],
        )?;
        conn.query_row(
            "SELECT id, path, name, last_active_ms, created_ms FROM projects WHERE path = ?1",
            [path],
            row_to_project,
        )
        .map_err(Into::into)
    }

    pub fn list(&self) -> Result<Vec<ProjectInfo>> {
        let conn = self.conn.lock().expect("db lock");
        let mut stmt = conn.prepare(
            "SELECT id, path, name, last_active_ms, created_ms FROM projects ORDER BY last_active_ms DESC",
        )?;
        let rows = stmt.query_map([], row_to_project)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 按 id 改名。`id` 不存在时返回 `Err`（不做静默 no-op）。
    pub fn rename(&self, id: i64, name: &str) -> Result<ProjectInfo> {
        let conn = self.conn.lock().expect("db lock");
        let affected = conn.execute(
            "UPDATE projects SET name = ?1 WHERE id = ?2",
            rusqlite::params![name, id],
        )?;
        if affected == 0 {
            anyhow::bail!("项目 id={id} 不存在");
        }
        conn.query_row(
            "SELECT id, path, name, last_active_ms, created_ms FROM projects WHERE id = ?1",
            [id],
            row_to_project,
        )
        .map_err(Into::into)
    }

    pub fn remove(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock().expect("db lock");
        let affected = conn.execute("DELETE FROM projects WHERE id = ?1", [id])?;
        if affected == 0 {
            anyhow::bail!("项目 id={id} 不存在");
        }
        Ok(())
    }
}

fn row_to_project(row: &rusqlite::Row) -> rusqlite::Result<ProjectInfo> {
    let path: String = row.get(1)?;
    let last_active_ms = row.get::<_, i64>(3)? as u64;
    let created_ms = row.get::<_, i64>(4)? as u64;
    let updated_ms = compute_updated_ms(&path, last_active_ms);
    Ok(ProjectInfo {
        id: row.get(0)?,
        path,
        name: row.get(2)?,
        last_active_ms,
        created_ms,
        updated_ms,
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

    #[test]
    fn rename_updates_name_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let store = ProjectStore::new(&dir.path().join("t.db")).unwrap();
        let p = store.open("/repo/a").unwrap();

        let renamed = store.rename(p.id, "新名字").unwrap();
        assert_eq!(renamed.id, p.id);
        assert_eq!(renamed.name, "新名字");

        let list = store.list().unwrap();
        assert_eq!(list[0].name, "新名字");
    }

    #[test]
    fn rename_missing_id_errors() {
        let dir = tempfile::tempdir().unwrap();
        let store = ProjectStore::new(&dir.path().join("t.db")).unwrap();
        assert!(store.rename(999, "x").is_err());
    }

    #[test]
    fn remove_deletes_project_and_it_no_longer_lists() {
        let dir = tempfile::tempdir().unwrap();
        let store = ProjectStore::new(&dir.path().join("t.db")).unwrap();
        let p = store.open("/proj/a").unwrap();

        store.remove(p.id).unwrap();

        let remaining = store.list().unwrap();
        assert!(remaining.iter().all(|r| r.id != p.id));
    }

    #[test]
    fn remove_missing_id_errors() {
        let dir = tempfile::tempdir().unwrap();
        let store = ProjectStore::new(&dir.path().join("t.db")).unwrap();
        assert!(store.remove(999).is_err());
    }
}
