//! 验收记录存储：rusqlite 单表（spec P1f D5）。
//! Mutex<Connection>——写入频度极低（人工验收动作），无需连接池。

use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::Path;
use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq)]
pub struct AcceptanceRecord {
    pub repo: String,
    pub goal: String,
    pub criteria_checked: Vec<String>,
    pub verdict: String,
    pub comment: String,
    pub ref_name: String,
    pub acceptor: String,
    pub ts_ms: u64,
}

pub struct AcceptanceStore {
    conn: Mutex<Connection>,
}

impl AcceptanceStore {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("建库目录")?;
        }
        let conn = Connection::open(path).context("打开 SQLite")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS acceptances (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                repo TEXT NOT NULL,
                goal TEXT NOT NULL,
                criteria_checked TEXT NOT NULL, -- JSON array
                verdict TEXT NOT NULL,
                comment TEXT NOT NULL,
                ref_name TEXT NOT NULL,
                acceptor TEXT NOT NULL,
                ts_ms INTEGER NOT NULL
            );",
        )
        .context("建表")?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn record(&self, r: &AcceptanceRecord) -> Result<()> {
        let criteria = serde_json::to_string(&r.criteria_checked)?;
        self.conn.lock().expect("db lock").execute(
            "INSERT INTO acceptances
             (repo, goal, criteria_checked, verdict, comment, ref_name, acceptor, ts_ms)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            rusqlite::params![
                r.repo, r.goal, criteria, r.verdict, r.comment, r.ref_name, r.acceptor, r.ts_ms
            ],
        )?;
        Ok(())
    }

    pub fn count(&self) -> Result<u64> {
        let n: u64 = self.conn.lock().expect("db lock").query_row(
            "SELECT COUNT(*) FROM acceptances",
            [],
            |row| row.get(0),
        )?;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_record_count_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = AcceptanceStore::open(&dir.path().join("t.db")).unwrap();
        assert_eq!(store.count().unwrap(), 0);
        store
            .record(&AcceptanceRecord {
                repo: "/r".into(),
                goal: "目标".into(),
                criteria_checked: vec!["a".into(), "b".into()],
                verdict: "accepted".into(),
                comment: "好".into(),
                ref_name: "refs/dozer/accepted/1".into(),
                acceptor: "user".into(),
                ts_ms: 42,
            })
            .unwrap();
        assert_eq!(store.count().unwrap(), 1);
        // 重开库仍在（持久化）
        drop(store);
        let store = AcceptanceStore::open(&dir.path().join("t.db")).unwrap();
        assert_eq!(store.count().unwrap(), 1);
    }
}
