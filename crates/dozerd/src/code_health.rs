//! 代码健康度报告存储：rusqlite 单表，主键 project_id（一个项目只保留
//! 最新一次扫描结果，历史不保留——同 spec「架构与数据流」的落盘持久化
//! 需求，只要"上次结果"，不要历史趋势）。
//! `scanned_at_ms` 由本 store 内部用 `now_ms()` 生成，调用方不传（同
//! `bookmarks.rs::add` 的既有约定：时间戳由服务端权威生成）。

use anyhow::{Context, Result};
use dozer_core::protocol::CodeHealthReportInfo;
use rusqlite::{Connection, OptionalExtension};
use std::path::Path;
use std::sync::Mutex;

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
             );",
        )
        .context("建表")?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn save(&self, project_id: i64, info: &CodeHealthReportInfo) -> Result<()> {
        let conn = self.conn.lock().expect("db lock");
        let ts = now_ms() as i64;
        conn.execute(
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
        .context("写入代码健康度报告")?;
        Ok(())
    }

    pub fn get(&self, project_id: i64) -> Result<Option<CodeHealthReportInfo>> {
        let conn = self.conn.lock().expect("db lock");
        conn.query_row(
            "SELECT total_loc, total_functions, critical_functions, overall_tier,
                    report_json, scanned_at_ms
             FROM code_health_reports WHERE project_id = ?1",
            [project_id],
            |row| {
                Ok(CodeHealthReportInfo {
                    total_loc: row.get::<_, i64>(0)? as u64,
                    total_functions: row.get::<_, i64>(1)? as u64,
                    critical_functions: row.get::<_, i64>(2)? as u64,
                    overall_tier: row.get(3)?,
                    report_json: row.get(4)?,
                    scanned_at_ms: row.get::<_, i64>(5)? as u64,
                })
            },
        )
        .optional()
        .context("查询代码健康度报告")
    }
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
        assert!(got.scanned_at_ms > 0);
    }

    #[test]
    fn save_twice_upserts_not_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        let store = CodeHealthStore::new(&dir.path().join("test.db")).unwrap();
        store.save(1, &info(100)).unwrap();
        store.save(1, &info(200)).unwrap();
        let got = store.get(1).unwrap().expect("应有数据");
        assert_eq!(got.total_loc, 200);
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
}
