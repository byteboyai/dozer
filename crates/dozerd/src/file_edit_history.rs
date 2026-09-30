//! Agent 精确修改的历史记录:project 级 SQLite,纯追加事件日志(不像
//! `MemoryStore` 那样按标题 upsert——一次修改一行,同一路径的多次修改按时间
//! 顺序排列)。`task_ref`/`source_ref` 对应 v0.1 意向文档 Provenance 模型里的
//! `Task`/`Source`,Phase 1 恒为 `NULL`,先建列占位,详见
//! `docs/superpowers/specs/2026-09-28-agent-native-file-editor-design.md`。

use anyhow::{Context, Result};
use rusqlite::{Connection, params};
use std::path::Path;
use std::sync::Mutex;

pub struct FileEditHistoryStore {
    pub(crate) conn: Mutex<Connection>,
}

/// 写入一条历史记录所需的字段;不含 `id`/`created_ms`(由存储层生成)。
#[derive(Debug, Clone)]
pub struct NewFileEditHistoryEntry {
    pub project_id: i64,
    pub target_path: String,
    pub actor: String,
    pub session_id: String,
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
    pub old_text: String,
    pub new_text: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FileEditHistoryEntry {
    pub id: i64,
    pub project_id: i64,
    pub target_path: String,
    pub actor: String,
    pub session_id: String,
    pub task_ref: Option<String>,
    pub source_ref: Option<String>,
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
    pub old_text: String,
    pub new_text: String,
    pub summary: String,
    pub created_ms: u64,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl FileEditHistoryStore {
    pub fn new(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("建库目录")?;
        }
        let conn = Connection::open(path).context("打开 SQLite")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS file_edit_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                project_id INTEGER NOT NULL,
                target_path TEXT NOT NULL,
                actor TEXT NOT NULL,
                session_id TEXT NOT NULL,
                task_ref TEXT,
                source_ref TEXT,
                start_line INTEGER NOT NULL,
                start_col INTEGER NOT NULL,
                end_line INTEGER NOT NULL,
                end_col INTEGER NOT NULL,
                old_text TEXT NOT NULL,
                new_text TEXT NOT NULL,
                summary TEXT NOT NULL,
                created_ms INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS file_edit_history_project_path
                ON file_edit_history(project_id, target_path, created_ms);",
        )
        .context("建表")?;
        crate::agent_context::init_schema(&conn).context("建上下文表")?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// 写一条历史记录,返回新行 `id`(供 `MutationOutcome::Applied::history_id`
    /// 使用)。
    pub fn record(&self, entry: NewFileEditHistoryEntry) -> Result<i64> {
        let conn = self.conn.lock().expect("db lock");
        conn.execute(
            "INSERT INTO file_edit_history
                (project_id, target_path, actor, session_id, task_ref, source_ref,
                 start_line, start_col, end_line, end_col, old_text, new_text,
                 summary, created_ms)
             VALUES (?1, ?2, ?3, ?4, NULL, NULL, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                entry.project_id,
                entry.target_path,
                entry.actor,
                entry.session_id,
                entry.start_line,
                entry.start_col,
                entry.end_line,
                entry.end_col,
                entry.old_text,
                entry.new_text,
                entry.summary,
                now_ms() as i64,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// 列出某项目某文件的历史,按时间正序(最早的在前)。
    pub fn list_for_path(
        &self,
        project_id: i64,
        target_path: &str,
    ) -> Result<Vec<FileEditHistoryEntry>> {
        let conn = self.conn.lock().expect("db lock");
        let mut stmt = conn.prepare(
            "SELECT id, project_id, target_path, actor, session_id, task_ref,
                    source_ref, start_line, start_col, end_line, end_col,
                    old_text, new_text, summary, created_ms
             FROM file_edit_history
             WHERE project_id = ?1 AND target_path = ?2
             ORDER BY created_ms ASC",
        )?;
        let rows = stmt.query_map(params![project_id, target_path], |r| {
            Ok(FileEditHistoryEntry {
                id: r.get(0)?,
                project_id: r.get(1)?,
                target_path: r.get(2)?,
                actor: r.get(3)?,
                session_id: r.get(4)?,
                task_ref: r.get(5)?,
                source_ref: r.get(6)?,
                start_line: r.get(7)?,
                start_col: r.get(8)?,
                end_line: r.get(9)?,
                end_col: r.get(10)?,
                old_text: r.get(11)?,
                new_text: r.get(12)?,
                summary: r.get(13)?,
                created_ms: r.get::<_, i64>(14)? as u64,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db() -> std::path::PathBuf {
        std::path::PathBuf::from(format!(
            "/tmp/dz-file-edit-history-test-{}.db",
            uuid::Uuid::new_v4()
        ))
    }

    fn sample_entry(project_id: i64, path: &str) -> NewFileEditHistoryEntry {
        NewFileEditHistoryEntry {
            project_id,
            target_path: path.to_string(),
            actor: "claude".to_string(),
            session_id: "sess-1".to_string(),
            start_line: 3,
            start_col: 1,
            end_line: 3,
            end_col: 10,
            old_text: "old".to_string(),
            new_text: "new".to_string(),
            summary: "修正拼写".to_string(),
        }
    }

    #[test]
    fn record_then_list_for_path_returns_in_order() {
        let db = temp_db();
        let store = FileEditHistoryStore::new(&db).unwrap();
        let id1 = store.record(sample_entry(1, "src/lib.rs")).unwrap();
        let id2 = store.record(sample_entry(1, "src/lib.rs")).unwrap();
        assert_ne!(id1, id2);

        let rows = store.list_for_path(1, "src/lib.rs").unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, id1);
        assert_eq!(rows[1].id, id2);
        assert_eq!(rows[0].actor, "claude");
        assert!(rows[0].task_ref.is_none());
        assert!(rows[0].source_ref.is_none());

        std::fs::remove_file(&db).ok();
    }

    #[test]
    fn list_for_path_scopes_by_project_and_path() {
        let db = temp_db();
        let store = FileEditHistoryStore::new(&db).unwrap();
        store.record(sample_entry(1, "src/lib.rs")).unwrap();
        store.record(sample_entry(1, "src/other.rs")).unwrap();
        store.record(sample_entry(2, "src/lib.rs")).unwrap();

        let rows = store.list_for_path(1, "src/lib.rs").unwrap();
        assert_eq!(rows.len(), 1);

        std::fs::remove_file(&db).ok();
    }
}
