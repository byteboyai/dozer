//! 代码健康度报告存储：`code_health_reports`（每项目最新一份，迁移期兼容入口）
//! + `code_health_report_snapshots`（历史快照，每项目保留最近 30 份）。
//!
//! `scanned_at_ms` 由本 store 内部用 `now_ms()` 生成，调用方不传（同
//! `bookmarks.rs::add` 的既有约定：时间戳由服务端权威生成）。

use anyhow::{Context, Result};
use dozer_core::protocol::CodeHealthReportInfo;
use rusqlite::{Connection, OptionalExtension};
use std::path::Path;
use std::sync::Mutex;

/// 每项目保留的历史快照上限（spec「每项目保留最近 30 份」）。
const MAX_SNAPSHOTS: i64 = 30;

pub struct CodeHealthStore {
    conn: Mutex<Connection>,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl CodeHealthStore {
    pub fn new(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("建库目录")?;
        }
        let conn = Connection::open(path).context("打开 SQLite")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS code_health_reports (
                project_id INTEGER PRIMARY KEY,
                total_loc INTEGER NOT NULL,
                total_functions INTEGER NOT NULL,
                critical_functions INTEGER NOT NULL,
                overall_tier TEXT NOT NULL,
                report_json TEXT NOT NULL,
                scanned_at_ms INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS code_health_report_snapshots (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                project_id INTEGER NOT NULL,
                schema_version INTEGER NOT NULL,
                report_json TEXT NOT NULL,
                git_head TEXT,
                git_branch TEXT,
                git_dirty INTEGER NOT NULL,
                scanned_at_ms INTEGER NOT NULL,
                total_loc INTEGER NOT NULL,
                total_functions INTEGER NOT NULL,
                critical_functions INTEGER NOT NULL,
                overall_tier TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_code_health_snapshots_project_time
                ON code_health_report_snapshots(project_id, scanned_at_ms DESC);",
        )
        .context("建表")?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// 事务内插入快照、更新最新报告行、删除该项目第 31 份及更旧快照。
    pub fn save(&self, project_id: i64, info: &CodeHealthReportInfo) -> Result<()> {
        let mut conn = self.conn.lock().expect("db lock");
        let tx = conn.transaction().context("开事务")?;
        let ts = now_ms() as i64;

        tx.execute(
            "INSERT INTO code_health_report_snapshots
                (project_id, schema_version, report_json, git_head, git_branch,
                 git_dirty, scanned_at_ms, total_loc, total_functions,
                 critical_functions, overall_tier)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            rusqlite::params![
                project_id,
                info.schema_version as i64,
                info.report_json,
                info.git_head,
                info.git_branch,
                info.git_dirty as i64,
                ts,
                info.total_loc as i64,
                info.total_functions as i64,
                info.critical_functions as i64,
                info.overall_tier,
            ],
        )
        .context("写入快照")?;

        tx.execute(
            "INSERT INTO code_health_reports
                (project_id, total_loc, total_functions, critical_functions,
                 overall_tier, report_json, scanned_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(project_id) DO UPDATE SET
                total_loc = excluded.total_loc,
                total_functions = excluded.total_functions,
                critical_functions = excluded.critical_functions,
                overall_tier = excluded.overall_tier,
                report_json = excluded.report_json,
                scanned_at_ms = excluded.scanned_at_ms",
            rusqlite::params![
                project_id,
                info.total_loc as i64,
                info.total_functions as i64,
                info.critical_functions as i64,
                info.overall_tier,
                info.report_json,
                ts,
            ],
        )
        .context("写入最新报告")?;

        tx.execute(
            "DELETE FROM code_health_report_snapshots
             WHERE project_id = ?1 AND id NOT IN (
                SELECT id FROM code_health_report_snapshots
                WHERE project_id = ?1
                ORDER BY scanned_at_ms DESC, id DESC
                LIMIT ?2
             )",
            rusqlite::params![project_id, MAX_SNAPSHOTS],
        )
        .context("清理旧快照")?;

        tx.commit().context("提交事务")?;
        Ok(())
    }

    /// 读取最新一份报告（从快照表取，schema/git 元数据齐全）。
    pub fn get(&self, project_id: i64) -> Result<Option<CodeHealthReportInfo>> {
        let conn = self.conn.lock().expect("db lock");
        conn.query_row(
            "SELECT total_loc, total_functions, critical_functions, overall_tier,
                    report_json, scanned_at_ms, schema_version, git_head,
                    git_branch, git_dirty
             FROM code_health_report_snapshots WHERE project_id = ?1
             ORDER BY scanned_at_ms DESC, id DESC LIMIT 1",
            [project_id],
            map_info_row,
        )
        .optional()
        .context("查询代码健康度报告")
    }

    /// 读取最近 `limit` 份快照（按时间倒序）。`limit` 上限钳制到
    /// `MAX_SNAPSHOTS`。
    pub fn list(&self, project_id: i64, limit: u32) -> Result<Vec<CodeHealthReportInfo>> {
        let limit = (limit as i64).clamp(1, MAX_SNAPSHOTS);
        let conn = self.conn.lock().expect("db lock");
        let mut stmt = conn
            .prepare(
                "SELECT total_loc, total_functions, critical_functions, overall_tier,
                        report_json, scanned_at_ms, schema_version, git_head,
                        git_branch, git_dirty
                 FROM code_health_report_snapshots WHERE project_id = ?1
                 ORDER BY scanned_at_ms DESC, id DESC LIMIT ?2",
            )
            .context("准备快照查询")?;
        let rows = stmt
            .query_map(rusqlite::params![project_id, limit], map_info_row)
            .context("查询快照列表")?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.context("读取快照行")?);
        }
        Ok(out)
    }
}

/// 从快照行读取 `CodeHealthReportInfo`。快照表冗余存标量列（同
/// `code_health_reports` 的“标量列 + 大文本列并存”约定，spec DDL 只列了
/// schema/git 元数据，这里按既有哲学补齐标量列，避免每次读都反序列化整个
/// JSON）。
fn map_info_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CodeHealthReportInfo> {
    Ok(CodeHealthReportInfo {
        total_loc: row.get::<_, i64>(0)? as u64,
        total_functions: row.get::<_, i64>(1)? as u64,
        critical_functions: row.get::<_, i64>(2)? as u64,
        overall_tier: row.get(3)?,
        report_json: row.get(4)?,
        scanned_at_ms: row.get::<_, i64>(5)? as u64,
        schema_version: row.get::<_, i64>(6)? as u32,
        git_head: row.get(7)?,
        git_branch: row.get(8)?,
        git_dirty: row.get::<_, i64>(9)? != 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(loc: u64) -> CodeHealthReportInfo {
        CodeHealthReportInfo {
            total_loc: loc,
            total_functions: 10,
            critical_functions: 1,
            overall_tier: "watch".into(),
            report_json: "{}".into(),
            scanned_at_ms: 0, // 忽略，store 会覆盖成自己的 now_ms()
            schema_version: 2,
            git_head: Some("abc".into()),
            git_branch: Some("main".into()),
            git_dirty: false,
        }
    }

    #[test]
    fn get_missing_project_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let store = CodeHealthStore::new(&dir.path().join("test.db")).unwrap();
        assert_eq!(store.get(1).unwrap(), None);
    }

    #[test]
    fn save_then_get_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let store = CodeHealthStore::new(&dir.path().join("test.db")).unwrap();
        store.save(1, &info(100)).unwrap();
        let got = store.get(1).unwrap().expect("应有数据");
        assert_eq!(got.total_loc, 100);
        assert_eq!(got.total_functions, 10);
        assert_eq!(got.critical_functions, 1);
        assert_eq!(got.overall_tier, "watch");
        assert_eq!(got.schema_version, 2);
        assert_eq!(got.git_branch.as_deref(), Some("main"));
        assert!(got.scanned_at_ms > 0);
    }

    #[test]
    fn save_twice_keeps_both_snapshots_and_latest_is_last() {
        let dir = tempfile::tempdir().unwrap();
        let store = CodeHealthStore::new(&dir.path().join("test.db")).unwrap();
        store.save(1, &info(100)).unwrap();
        store.save(1, &info(200)).unwrap();
        let got = store.get(1).unwrap().expect("应有数据");
        assert_eq!(got.total_loc, 200);
        // 两份快照都在（< 30 上限）。
        assert_eq!(store.list(1, 10).unwrap().len(), 2);
    }

    #[test]
    fn different_projects_independent() {
        let dir = tempfile::tempdir().unwrap();
        let store = CodeHealthStore::new(&dir.path().join("test.db")).unwrap();
        store.save(1, &info(100)).unwrap();
        store.save(2, &info(999)).unwrap();
        assert_eq!(store.get(1).unwrap().unwrap().total_loc, 100);
        assert_eq!(store.get(2).unwrap().unwrap().total_loc, 999);
    }

    #[test]
    fn list_returns_descending_by_time() {
        let dir = tempfile::tempdir().unwrap();
        let store = CodeHealthStore::new(&dir.path().join("test.db")).unwrap();
        for loc in 1..=5 {
            store.save(1, &info(loc)).unwrap();
        }
        let reports = store.list(1, 10).unwrap();
        assert_eq!(reports.len(), 5);
        assert_eq!(reports[0].total_loc, 5, "最新一份在最前");
        assert_eq!(reports[4].total_loc, 1);
    }

    #[test]
    fn list_limit_is_clamped_to_max() {
        let dir = tempfile::tempdir().unwrap();
        let store = CodeHealthStore::new(&dir.path().join("test.db")).unwrap();
        for loc in 1..=3 {
            store.save(1, &info(loc)).unwrap();
        }
        let reports = store.list(1, 2).unwrap();
        assert_eq!(reports.len(), 2);
        assert_eq!(reports[0].total_loc, 3);
    }

    #[test]
    fn retains_only_30_snapshots_per_project() {
        let dir = tempfile::tempdir().unwrap();
        let store = CodeHealthStore::new(&dir.path().join("test.db")).unwrap();
        for loc in 1..=35 {
            store.save(1, &info(loc)).unwrap();
        }
        let reports = store.list(1, 100).unwrap();
        assert_eq!(reports.len(), 30, "只保留最近 30 份");
        assert_eq!(reports[0].total_loc, 35);
        assert_eq!(reports[29].total_loc, 6, "最旧保留到第 6 份");
    }

    #[test]
    fn old_db_without_snapshots_table_upgrades() {
        // 模拟旧数据库：只有 code_health_reports 表。new() 必须能直接跑
        // CREATE TABLE IF NOT EXISTS 建出快照表，而不是报错。
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("old.db");
        {
            let conn = Connection::open(&db).unwrap();
            conn.execute_batch(
                "CREATE TABLE code_health_reports (
                    project_id INTEGER PRIMARY KEY,
                    total_loc INTEGER NOT NULL,
                    total_functions INTEGER NOT NULL,
                    critical_functions INTEGER NOT NULL,
                    overall_tier TEXT NOT NULL,
                    report_json TEXT NOT NULL,
                    scanned_at_ms INTEGER NOT NULL
                 );",
            )
            .unwrap();
        }
        let store = CodeHealthStore::new(&db).unwrap();
        store.save(1, &info(42)).unwrap();
        assert_eq!(store.get(1).unwrap().unwrap().total_loc, 42);
    }
}
