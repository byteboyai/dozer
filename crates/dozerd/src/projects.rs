//! 项目存储：rusqlite（spec P1g D1）。projects 表（不再有活跃项目指针，
//! P2a 起 daemon 变成纯会话仓库）。
//! 各 store 各持一个到 dozer.db 的连接；项目写频度极低，
//! 多连接足够（无需连接池）。

use anyhow::{Context, Result};
use bytegit::Repo;
use dozer_core::protocol::ProjectInfo;
use rusqlite::Connection;
use rusqlite::OptionalExtension;
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

/// 项目所属 git 仓库 HEAD 提交的提交时间(毫秒)。不是 git 工作区(含裸仓库)、
/// 仓库没有提交或读取失败时返回 `None`。`dir` 在仓库子目录里时向上查找;
/// linked worktree 取该 worktree 自己的 HEAD。
fn git_head_commit_ms(dir: &Path) -> Option<u64> {
    if !dir.is_dir() {
        return None;
    }
    let repo = Repo::discover(dir).ok()?;
    // 裸仓库没有工作区:迁移前 `git rev-parse --show-toplevel` 在那里失败,保持不计。
    if repo.is_bare() {
        return None;
    }
    let time = repo.head_commit_time().ok()??;
    let secs = time.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    Some(secs * 1000)
}

/// 计算项目的 git 感知更新时间：优先取仓库最新 commit 时间，若该仓库没有
/// commit，或 commit 时间早于 `last_active_ms`，则回落为 `last_active_ms`。
fn compute_updated_ms(path: &str, last_active_ms: u64) -> u64 {
    let commit = git_head_commit_ms(Path::new(path));
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

    /// 轻量按 id 查项目路径(不算 git 派生的 `updated_ms`,群聊每次发言都要查)。
    pub fn path_of(&self, id: i64) -> Result<Option<String>> {
        let conn = self.conn.lock().expect("db lock");
        Ok(conn
            .query_row("SELECT path FROM projects WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .optional()?)
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
    use std::process::Command;

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

    #[test]
    fn path_of_returns_path_or_none() {
        let dir = tempfile::tempdir().unwrap();
        let store = ProjectStore::new(&dir.path().join("p.db")).unwrap();
        let p = store.open("/tmp/some-proj").unwrap();
        assert_eq!(
            store.path_of(p.id).unwrap().as_deref(),
            Some("/tmp/some-proj")
        );
        assert_eq!(store.path_of(9999).unwrap(), None);
    }

    // ---- bytegit P3:compute_updated_ms 迁移前后必须一致的口径 ----

    const T: i64 = 20_000 * 86_400;

    fn git_at(dir: &Path, args: &[&str], when: Option<i64>) {
        let mut cmd = Command::new("git");
        cmd.args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            // 作者日期固定为很早以前:更新时间取的是**提交者**日期。
            .env("GIT_AUTHOR_DATE", "946684800 +0000");
        if let Some(w) = when {
            cmd.env("GIT_COMMITTER_DATE", format!("{w} +0000"));
        }
        let out = cmd.output().unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
    }

    fn commit_at(dir: &Path, rel: &str, content: &str, when: i64) {
        let full = dir.join(rel);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(full, content).unwrap();
        git_at(dir, &["add", "."], None);
        git_at(dir, &["commit", "-qm", "c"], Some(when));
    }

    fn updated(dir: &Path, last_active_ms: u64) -> u64 {
        compute_updated_ms(dir.to_str().unwrap(), last_active_ms)
    }

    #[test]
    fn updated_ms_is_last_active_outside_a_repo() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(updated(dir.path(), 5_000), 5_000);
    }

    #[test]
    fn updated_ms_is_last_active_for_a_repo_without_commits() {
        let dir = tempfile::tempdir().unwrap();
        git_at(dir.path(), &["init", "-q"], None);
        assert_eq!(updated(dir.path(), 5_000), 5_000);
    }

    #[test]
    fn updated_ms_is_the_head_commit_time_when_it_is_not_older_than_last_active() {
        let dir = tempfile::tempdir().unwrap();
        git_at(dir.path(), &["init", "-q"], None);
        commit_at(dir.path(), "a.txt", "1", T + 123);
        assert_eq!(updated(dir.path(), 1_000), (T as u64 + 123) * 1000);
        // 恰好相等也取提交时间(`>=`)。
        assert_eq!(
            updated(dir.path(), (T as u64 + 123) * 1000),
            (T as u64 + 123) * 1000
        );
    }

    #[test]
    fn updated_ms_falls_back_to_last_active_when_the_commit_is_older() {
        let dir = tempfile::tempdir().unwrap();
        git_at(dir.path(), &["init", "-q"], None);
        commit_at(dir.path(), "a.txt", "1", T);
        let later = (T as u64 + 10) * 1000;
        assert_eq!(updated(dir.path(), later), later);
    }

    #[test]
    fn updated_ms_uses_the_head_commit_even_when_an_ancestor_has_a_newer_timestamp() {
        let dir = tempfile::tempdir().unwrap();
        git_at(dir.path(), &["init", "-q"], None);
        commit_at(dir.path(), "a.txt", "1", T + 5_000);
        commit_at(dir.path(), "a.txt", "2", T);
        assert_eq!(updated(dir.path(), 1_000), (T as u64) * 1000);
    }

    #[test]
    fn updated_ms_looks_upward_from_a_repo_subdirectory() {
        let dir = tempfile::tempdir().unwrap();
        git_at(dir.path(), &["init", "-q"], None);
        commit_at(dir.path(), "sub/a.txt", "1", T + 7);
        assert_eq!(
            updated(&dir.path().join("sub"), 1_000),
            (T as u64 + 7) * 1000
        );
    }

    #[test]
    fn updated_ms_works_on_a_detached_head() {
        let dir = tempfile::tempdir().unwrap();
        git_at(dir.path(), &["init", "-q"], None);
        commit_at(dir.path(), "a.txt", "1", T + 9);
        git_at(dir.path(), &["checkout", "-q", "--detach"], None);
        assert_eq!(updated(dir.path(), 1_000), (T as u64 + 9) * 1000);
    }

    #[test]
    fn updated_ms_ignores_a_bare_repo_because_it_has_no_work_tree() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir(&src).unwrap();
        git_at(&src, &["init", "-q"], None);
        commit_at(&src, "a.txt", "1", T + 11);
        let bare = dir.path().join("bare.git");
        git_at(
            dir.path(),
            &[
                "clone",
                "-q",
                "--bare",
                src.to_str().unwrap(),
                bare.to_str().unwrap(),
            ],
            None,
        );
        assert_eq!(updated(&bare, 1_000), 1_000);
    }

    #[test]
    fn updated_ms_of_a_linked_worktree_follows_that_worktrees_head() {
        let dir = tempfile::tempdir().unwrap();
        let main = dir.path().join("main");
        std::fs::create_dir(&main).unwrap();
        git_at(&main, &["init", "-q"], None);
        commit_at(&main, "a.txt", "1", T + 100);
        let wt = dir.path().join("wt");
        git_at(
            &main,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "feature",
                wt.to_str().unwrap(),
            ],
            None,
        );
        commit_at(&wt, "b.txt", "2", T + 200);
        assert_eq!(updated(&main, 1_000), (T as u64 + 100) * 1000);
        assert_eq!(updated(&wt, 1_000), (T as u64 + 200) * 1000);
    }
}
