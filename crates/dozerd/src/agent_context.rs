//! Agent 上下文列表(v0.1 意向文档 §10 Context Scope)的持久化,以及 GUI 修改
//! 历史弹窗的查询。表与 `file_edit_history` 同库同连接,方法挂在
//! `FileEditHistoryStore` 的第二个 `impl` 块上——不另起一个 store,是为了不给
//! `server::Stores` 加字段(那会连带改 14 处测试构造点)。
//! 设计见 `docs/superpowers/specs/2026-09-30-agent-native-file-editor-phase3-context-history-design.md`。

use anyhow::Result;
use dozer_core::protocol::{ContextItemInfo, FileEditHistoryInfo};
use rusqlite::{Connection, params};
use std::path::{Component, Path};

use crate::file_edit_history::FileEditHistoryStore;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub(crate) fn init_schema(conn: &Connection) -> rusqlite::Result<()> {
    // `permission`:v0.1 §11 权限位的占位列,本期不读不写不进 UI。
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS agent_context_items (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            project_id INTEGER NOT NULL,
            entity_kind TEXT NOT NULL,
            entity_ref TEXT NOT NULL,
            permission TEXT NOT NULL DEFAULT 'default',
            created_ms INTEGER NOT NULL,
            UNIQUE(project_id, entity_kind, entity_ref)
         );",
    )
}

/// 校验要加入上下文的相对路径落在项目根目录内。路径在磁盘上不存在是允许的
/// (agent 稍后可能创建它,列表项只是声明不是快照),所以先做纯词法检查
/// (拒绝空串/绝对路径/`..`),再对**已存在**的路径做 canonicalize 防符号链接逃逸。
pub fn validate_context_ref(root: &Path, rel: &str) -> Result<(), String> {
    if rel.is_empty() {
        return Err("路径为空".into());
    }
    let p = Path::new(rel);
    if p.is_absolute()
        || p.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::Prefix(_) | Component::RootDir
            )
        })
    {
        return Err("路径越界".into());
    }
    let joined = root.join(p);
    if let (Ok(real), Ok(real_root)) = (joined.canonicalize(), root.canonicalize())
        && !real.starts_with(&real_root)
    {
        return Err("路径越界".into());
    }
    Ok(())
}

/// 把 `text` 替换进 `(start_line, start_col)` 之后,`text` 的结束坐标(1-based,
/// 与 `file_mutation::offset_to_line_col` 同口径:结束坐标是最后一个字符之后的
/// 位置;列按字符计,不按字节)。起点在替换前后不变。
pub fn end_position_after(start_line: u32, start_col: u32, text: &str) -> (u32, u32) {
    match text.rfind('\n') {
        None => (start_line, start_col + text.chars().count() as u32),
        Some(idx) => {
            let newlines = text.matches('\n').count() as u32;
            (
                start_line + newlines,
                text[idx + 1..].chars().count() as u32 + 1,
            )
        }
    }
}

fn row_to_item(r: &rusqlite::Row<'_>) -> rusqlite::Result<ContextItemInfo> {
    Ok(ContextItemInfo {
        id: r.get(0)?,
        project_id: r.get(1)?,
        entity_kind: r.get(2)?,
        entity_ref: r.get(3)?,
        created_ms: r.get::<_, i64>(4)? as u64,
    })
}

/// SQL `LIKE ... ESCAPE '\'` 里把字面的 `\`/`%`/`_` 转义,免得目录名里的
/// 通配符字符被当成通配。
fn escape_like(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

impl FileEditHistoryStore {
    /// 加入上下文列表。`(project_id, entity_kind, entity_ref)` 唯一,重复添加
    /// 返回既有行且不改 `created_ms`。`entity_ref` 尾部的 `/` 会被裁掉。
    pub fn add_context_item(
        &self,
        project_id: i64,
        entity_kind: &str,
        entity_ref: &str,
    ) -> Result<ContextItemInfo> {
        let entity_ref = entity_ref.trim_end_matches('/');
        let conn = self.conn.lock().expect("db lock");
        conn.execute(
            "INSERT OR IGNORE INTO agent_context_items
                (project_id, entity_kind, entity_ref, created_ms)
             VALUES (?1, ?2, ?3, ?4)",
            params![project_id, entity_kind, entity_ref, now_ms() as i64],
        )?;
        let item = conn.query_row(
            "SELECT id, project_id, entity_kind, entity_ref, created_ms
             FROM agent_context_items
             WHERE project_id = ?1 AND entity_kind = ?2 AND entity_ref = ?3",
            params![project_id, entity_kind, entity_ref],
            row_to_item,
        )?;
        Ok(item)
    }

    /// 按项目移除一项;目标不存在或属于别的项目都视为成功(不报错)。
    pub fn remove_context_item(&self, project_id: i64, id: i64) -> Result<()> {
        let conn = self.conn.lock().expect("db lock");
        conn.execute(
            "DELETE FROM agent_context_items WHERE project_id = ?1 AND id = ?2",
            params![project_id, id],
        )?;
        Ok(())
    }

    pub fn list_context_items(&self, project_id: i64) -> Result<Vec<ContextItemInfo>> {
        let conn = self.conn.lock().expect("db lock");
        let mut stmt = conn.prepare(
            "SELECT id, project_id, entity_kind, entity_ref, created_ms
             FROM agent_context_items
             WHERE project_id = ?1
             ORDER BY created_ms ASC, id ASC",
        )?;
        let rows = stmt.query_map(params![project_id], row_to_item)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 项目内 agent 修改历史,时间倒序。`path_filter` 命中 `target_path == 值`
    /// 或位于该目录之下(`值/…`)。
    pub fn list_history(
        &self,
        project_id: i64,
        path_filter: Option<&str>,
        limit: u32,
    ) -> Result<Vec<FileEditHistoryInfo>> {
        let filter = path_filter.map(|f| f.trim_end_matches('/'));
        let like = filter.map(|f| format!("{}/%", escape_like(f)));
        let conn = self.conn.lock().expect("db lock");
        let mut stmt = conn.prepare(
            "SELECT id, project_id, target_path, actor, session_id,
                    start_line, start_col, end_line, end_col,
                    old_text, new_text, summary, created_ms
             FROM file_edit_history
             WHERE project_id = ?1
               AND (?2 IS NULL OR target_path = ?2 OR target_path LIKE ?3 ESCAPE '\\')
             ORDER BY created_ms DESC, id DESC
             LIMIT ?4",
        )?;
        let rows = stmt.query_map(params![project_id, filter, like, limit as i64], |r| {
            let start_line: u32 = r.get(5)?;
            let start_col: u32 = r.get(6)?;
            let new_text: String = r.get(10)?;
            let (new_end_line, new_end_col) = end_position_after(start_line, start_col, &new_text);
            Ok(FileEditHistoryInfo {
                id: r.get(0)?,
                project_id: r.get(1)?,
                target_path: r.get(2)?,
                actor: r.get(3)?,
                session_id: r.get(4)?,
                start_line,
                start_col,
                end_line: r.get(7)?,
                end_col: r.get(8)?,
                new_end_line,
                new_end_col,
                old_text: r.get(9)?,
                new_text,
                summary: r.get(11)?,
                created_ms: r.get::<_, i64>(12)? as u64,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_edit_history::NewFileEditHistoryEntry;

    fn temp_db() -> std::path::PathBuf {
        std::path::PathBuf::from(format!(
            "/tmp/dz-agent-context-test-{}.db",
            uuid::Uuid::new_v4()
        ))
    }

    fn edit(project_id: i64, path: &str, new_text: &str) -> NewFileEditHistoryEntry {
        NewFileEditHistoryEntry {
            project_id,
            target_path: path.to_string(),
            actor: "claude".into(),
            session_id: "s1".into(),
            start_line: 2,
            start_col: 1,
            end_line: 2,
            end_col: 9,
            old_text: "line two".into(),
            new_text: new_text.into(),
            summary: "改".into(),
        }
    }

    #[test]
    fn add_is_idempotent_and_trims_trailing_slash() {
        let db = temp_db();
        let store = FileEditHistoryStore::new(&db).unwrap();
        let a = store.add_context_item(1, "dir", "research").unwrap();
        let b = store.add_context_item(1, "dir", "research/").unwrap();
        assert_eq!(a.id, b.id);
        assert_eq!(b.entity_ref, "research");
        assert_eq!(a.created_ms, b.created_ms, "重复添加不能改 created_ms");
        assert_eq!(store.list_context_items(1).unwrap().len(), 1);
        std::fs::remove_file(&db).ok();
    }

    #[test]
    fn list_is_ascending_and_scoped_by_project() {
        let db = temp_db();
        let store = FileEditHistoryStore::new(&db).unwrap();
        let a = store.add_context_item(1, "file", "a.rs").unwrap();
        let b = store.add_context_item(1, "file", "b.rs").unwrap();
        store.add_context_item(2, "file", "a.rs").unwrap();
        let items = store.list_context_items(1).unwrap();
        assert_eq!(
            items.iter().map(|i| i.id).collect::<Vec<_>>(),
            vec![a.id, b.id]
        );
        std::fs::remove_file(&db).ok();
    }

    #[test]
    fn remove_is_idempotent_and_project_scoped() {
        let db = temp_db();
        let store = FileEditHistoryStore::new(&db).unwrap();
        let a = store.add_context_item(1, "file", "a.rs").unwrap();
        // 别的项目不能删掉这一项。
        store.remove_context_item(2, a.id).unwrap();
        assert_eq!(store.list_context_items(1).unwrap().len(), 1);
        store.remove_context_item(1, a.id).unwrap();
        store.remove_context_item(1, a.id).unwrap(); // 再删一次不报错
        assert!(store.list_context_items(1).unwrap().is_empty());
        std::fs::remove_file(&db).ok();
    }

    #[test]
    fn unknown_entity_kind_round_trips_verbatim() {
        let db = temp_db();
        let store = FileEditHistoryStore::new(&db).unwrap();
        store.add_context_item(1, "todo", "42").unwrap();
        assert_eq!(store.list_context_items(1).unwrap()[0].entity_kind, "todo");
        std::fs::remove_file(&db).ok();
    }

    #[test]
    fn history_is_newest_first_with_limit() {
        let db = temp_db();
        let store = FileEditHistoryStore::new(&db).unwrap();
        let id1 = store.record(edit(1, "a.txt", "x")).unwrap();
        let id2 = store.record(edit(1, "a.txt", "y")).unwrap();
        let id3 = store.record(edit(1, "b.txt", "z")).unwrap();
        let all = store.list_history(1, None, 100).unwrap();
        assert_eq!(
            all.iter().map(|e| e.id).collect::<Vec<_>>(),
            vec![id3, id2, id1]
        );
        assert_eq!(store.list_history(1, None, 2).unwrap().len(), 2);
        assert!(store.list_history(2, None, 100).unwrap().is_empty());
        std::fs::remove_file(&db).ok();
    }

    #[test]
    fn history_filter_matches_file_or_directory_subtree_only() {
        let db = temp_db();
        let store = FileEditHistoryStore::new(&db).unwrap();
        store.record(edit(1, "src/a/x.rs", "1")).unwrap();
        store.record(edit(1, "src/ab/y.rs", "2")).unwrap();
        store.record(edit(1, "src/a", "3")).unwrap(); // 恰好等于过滤值的文件
        let hits = store.list_history(1, Some("src/a"), 100).unwrap();
        let mut paths: Vec<_> = hits.iter().map(|e| e.target_path.as_str()).collect();
        paths.sort();
        assert_eq!(
            paths,
            vec!["src/a", "src/a/x.rs"],
            "src/ab 不能被 src/a 误命中"
        );
        std::fs::remove_file(&db).ok();
    }

    #[test]
    fn history_filter_treats_like_wildcards_literally() {
        let db = temp_db();
        let store = FileEditHistoryStore::new(&db).unwrap();
        store.record(edit(1, "100%_done/a.txt", "1")).unwrap();
        store.record(edit(1, "100xxdone/a.txt", "2")).unwrap();
        let hits = store.list_history(1, Some("100%_done"), 100).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].target_path, "100%_done/a.txt");
        std::fs::remove_file(&db).ok();
    }

    #[test]
    fn history_reports_post_edit_end_position() {
        let db = temp_db();
        let store = FileEditHistoryStore::new(&db).unwrap();
        store.record(edit(1, "a.txt", "x\nabc")).unwrap();
        let e = &store.list_history(1, None, 10).unwrap()[0];
        assert_eq!((e.new_end_line, e.new_end_col), (3, 4));
        assert_eq!((e.start_line, e.start_col), (2, 1));
        std::fs::remove_file(&db).ok();
    }

    #[test]
    fn end_position_after_single_line_multi_line_and_empty() {
        assert_eq!(end_position_after(2, 1, "replaced"), (2, 9));
        assert_eq!(end_position_after(2, 5, ""), (2, 5));
        assert_eq!(end_position_after(2, 1, "a\nbc"), (3, 3));
        assert_eq!(end_position_after(2, 1, "a\n"), (3, 1));
        assert_eq!(end_position_after(1, 1, "你好\n世"), (2, 2));
    }

    #[test]
    fn validate_rejects_escape_and_accepts_missing_paths() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        assert!(validate_context_ref(root, "src/lib.rs").is_ok());
        assert!(validate_context_ref(root, "尚未创建的/文件 v2.md").is_ok());
        assert!(validate_context_ref(root, "").is_err());
        assert!(validate_context_ref(root, "../x").is_err());
        assert!(validate_context_ref(root, "a/../../x").is_err());
        assert!(validate_context_ref(root, "/etc/passwd").is_err());
    }

    #[test]
    fn validate_rejects_symlink_escape_for_existing_paths() {
        let outside = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("link")).unwrap();
        assert!(validate_context_ref(root.path(), "link").is_err());
    }
}
