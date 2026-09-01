//! Todo 存储:rusqlite,与 `ProjectStore`/`AcceptanceStore`/`BookmarkStore`
//! 共享同一个 `dozer.db`(见 `main.rs` 里 `new` 时传入同一路径)。任务
//! 正文+全部元数据(派发记录/计划日期/完成时间)合并成一张表、一行一条
//! 任务,`id` 是稳定身份,取代 v1(`.dozer/todo.md` + `todo_meta.json`)
//! 靠"文本哈希"关联元数据的做法(2026-09-01 SQLite 迁移设计
//! `docs/superpowers/specs/2026-09-01-dozer-todo-sqlite-migration-design.md`)。

use anyhow::{Context, Result};
use dozer_core::protocol::TodoInfo;
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;
use std::sync::Mutex;

pub struct TodoStore {
    conn: Mutex<Connection>,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn id_not_found(id: i64) -> anyhow::Error {
    anyhow::anyhow!("任务不存在: id={id}")
}

/// `RETURNING`/`SELECT` 都用这份列顺序,`query_row`/`query_map` 的行映射
/// 闭包与之一一对应。
const TODO_COLUMNS: &str = "id, project_id, text, done, rank, created_ms, \
    completed_at_ms, plan_date, dispatch_session_id, dispatch_at_ms";

fn row_to_todo(row: &rusqlite::Row) -> rusqlite::Result<TodoInfo> {
    Ok(TodoInfo {
        id: row.get(0)?,
        project_id: row.get(1)?,
        text: row.get(2)?,
        done: row.get(3)?,
        rank: row.get(4)?,
        created_ms: row.get::<_, i64>(5)? as u64,
        completed_at_ms: row.get::<_, Option<i64>>(6)?.map(|v| v as u64),
        plan_date: row.get(7)?,
        dispatch_session_id: row.get(8)?,
        dispatch_at_ms: row.get::<_, Option<i64>>(9)?.map(|v| v as u64),
    })
}

impl TodoStore {
    pub fn new(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("建库目录")?;
        }
        let conn = Connection::open(path).context("打开 SQLite")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS todos (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                project_id INTEGER NOT NULL,
                text TEXT NOT NULL,
                done INTEGER NOT NULL DEFAULT 0,
                rank INTEGER NOT NULL,
                created_ms INTEGER NOT NULL,
                completed_at_ms INTEGER,
                plan_date TEXT,
                dispatch_session_id TEXT,
                dispatch_at_ms INTEGER
             );
             CREATE INDEX IF NOT EXISTS idx_todos_project_order
                ON todos(project_id, done, rank);
             CREATE TABLE IF NOT EXISTS todo_legacy_imported (
                project_id INTEGER PRIMARY KEY,
                imported_ms INTEGER NOT NULL
             );",
        )
        .context("建表")?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// 某项目全部任务,待办按 rank 升序在前、已完成按 rank 升序沉底。
    pub fn list(&self, project_id: i64) -> Result<Vec<TodoInfo>> {
        let conn = self.conn.lock().expect("db lock");
        let sql = format!(
            "SELECT {TODO_COLUMNS} FROM todos WHERE project_id = ?1 ORDER BY done ASC, rank ASC"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([project_id], row_to_todo)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 新增即置顶:新 rank = 当前待办最小 rank - 1(表内该项目还没有待办
    /// 时从 0 开始)。
    pub fn add(&self, project_id: i64, text: &str) -> Result<TodoInfo> {
        let conn = self.conn.lock().expect("db lock");
        let min_rank: Option<i64> = conn.query_row(
            "SELECT MIN(rank) FROM todos WHERE project_id = ?1 AND done = 0",
            [project_id],
            |r| r.get(0),
        )?;
        let rank = min_rank.map(|r| r - 1).unwrap_or(0);
        let now = now_ms() as i64;
        let sql = format!(
            "INSERT INTO todos (project_id, text, done, rank, created_ms)
             VALUES (?1, ?2, 0, ?3, ?4) RETURNING {TODO_COLUMNS}"
        );
        conn.query_row(&sql, params![project_id, text, rank, now], row_to_todo)
            .map_err(Into::into)
    }

    /// `done` 从假变真时写 `completed_at_ms = now`,真变假时清空——与现状
    /// `completed_at_for_toggle` 纯函数行为等价,合并进这一次 UPDATE。
    pub fn toggle(&self, id: i64, done: bool) -> Result<TodoInfo> {
        let conn = self.conn.lock().expect("db lock");
        let completed_at_ms: Option<i64> = done.then(|| now_ms() as i64);
        let sql = format!(
            "UPDATE todos SET done = ?1, completed_at_ms = ?2 WHERE id = ?3 RETURNING {TODO_COLUMNS}"
        );
        conn.query_row(&sql, params![done, completed_at_ms, id], row_to_todo)
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => id_not_found(id),
                e => e.into(),
            })
    }

    pub fn edit_text(&self, id: i64, text: &str) -> Result<TodoInfo> {
        let conn = self.conn.lock().expect("db lock");
        let sql = format!("UPDATE todos SET text = ?1 WHERE id = ?2 RETURNING {TODO_COLUMNS}");
        conn.query_row(&sql, params![text, id], row_to_todo)
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => id_not_found(id),
                e => e.into(),
            })
    }

    pub fn set_plan_date(&self, id: i64, plan_date: Option<&str>) -> Result<TodoInfo> {
        let conn = self.conn.lock().expect("db lock");
        let sql = format!("UPDATE todos SET plan_date = ?1 WHERE id = ?2 RETURNING {TODO_COLUMNS}");
        conn.query_row(&sql, params![plan_date, id], row_to_todo)
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => id_not_found(id),
                e => e.into(),
            })
    }

    pub fn record_dispatch(&self, id: i64, session_id: &str) -> Result<TodoInfo> {
        let conn = self.conn.lock().expect("db lock");
        let now = now_ms() as i64;
        let sql = format!(
            "UPDATE todos SET dispatch_session_id = ?1, dispatch_at_ms = ?2
             WHERE id = ?3 RETURNING {TODO_COLUMNS}"
        );
        conn.query_row(&sql, params![session_id, now, id], row_to_todo)
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => id_not_found(id),
                e => e.into(),
            })
    }

    /// 把 `id` 挪到 `after_id` 之后(`None` = 待办块最前);只在待办子集内
    /// 生效。做法:取该 project 全部待办 id(按当前 rank 升序),摘出 `id`
    /// 插到目标位置,对整个待办子集重新编号(0..N-1)——待办条数通常几十
    /// 条以内,全量重编号比"两个相邻 rank 间插值"更简单且不存在整数间隙
    /// 耗尽的问题,一次事务内完成,防止中途崩溃留下部分重排的脏状态。
    /// `after_id` 不在当前待办里(比如已经是已完成,或调用方传了脏 id)
    /// 时退化为"挪到待办块末尾",不报错。
    pub fn reorder(&self, id: i64, after_id: Option<i64>) -> Result<TodoInfo> {
        let mut conn = self.conn.lock().expect("db lock");
        let project_id: i64 = conn
            .query_row(
                "SELECT project_id FROM todos WHERE id = ?1 AND done = 0",
                [id],
                |r| r.get(0),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => id_not_found(id),
                e => e.into(),
            })?;
        let mut pending_ids: Vec<i64> = {
            let mut stmt = conn.prepare(
                "SELECT id FROM todos WHERE project_id = ?1 AND done = 0 ORDER BY rank ASC",
            )?;
            stmt.query_map([project_id], |r| r.get(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        let Some(pos) = pending_ids.iter().position(|&x| x == id) else {
            return Err(id_not_found(id));
        };
        pending_ids.remove(pos);
        let insert_at = match after_id {
            None => 0,
            Some(aid) => pending_ids
                .iter()
                .position(|&x| x == aid)
                .map(|p| p + 1)
                .unwrap_or(pending_ids.len()),
        };
        pending_ids.insert(insert_at, id);
        let tx = conn.transaction()?;
        for (rank, task_id) in pending_ids.iter().enumerate() {
            tx.execute(
                "UPDATE todos SET rank = ?1 WHERE id = ?2",
                params![rank as i64, task_id],
            )?;
        }
        tx.commit()?;
        let sql = format!("SELECT {TODO_COLUMNS} FROM todos WHERE id = ?1");
        conn.query_row(&sql, [id], row_to_todo).map_err(Into::into)
    }

    /// 对外入口:算出真实磁盘路径后调 `import_legacy_if_needed_at`。
    /// `cwd` 是项目根目录绝对路径(与 `Request::OpenProject.path` 同一种
    /// 形状)。
    pub fn import_legacy_if_needed(&self, project_id: i64, cwd: &Path) -> Result<()> {
        let todo_md_path = cwd.join(".dozer").join("todo.md");
        let meta_json_path = dozer_core::paths::config_dir().join("todo_meta.json");
        self.import_legacy_if_needed_at(project_id, &todo_md_path, &meta_json_path)
    }

    /// 参数化的实现,便于单测用临时目录路径调用,不依赖真实的
    /// `dozer_core::paths::config_dir()`(那是机器级全局路径,测试不该碰)。
    /// 一次性:`todo_legacy_imported` 里已有该 `project_id` 记录就直接
    /// 返回,不重复导入(即使 `todo_md_path` 之后又变了)。读取/解析失败
    /// 按"这个项目没有历史任务"处理,仍然写迁移标记,避免每次 `OpenProject`
    /// 都重新尝试同一个读不了的文件。
    fn import_legacy_if_needed_at(
        &self,
        project_id: i64,
        todo_md_path: &Path,
        meta_json_path: &Path,
    ) -> Result<()> {
        let mut conn = self.conn.lock().expect("db lock");
        let already = conn
            .query_row(
                "SELECT 1 FROM todo_legacy_imported WHERE project_id = ?1",
                [project_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if already {
            return Ok(());
        }
        let md = std::fs::read_to_string(todo_md_path).unwrap_or_default();
        let items = parse_legacy_markdown(&md);
        let meta = read_legacy_meta_json(meta_json_path, project_id);
        let now = now_ms() as i64;
        let tx = conn.transaction()?;
        for (rank, (text, done)) in items.iter().enumerate() {
            let key = legacy_todo_line_key(text);
            let m = meta.get(&key);
            tx.execute(
                "INSERT INTO todos (project_id, text, done, rank, created_ms,
                    completed_at_ms, plan_date, dispatch_session_id, dispatch_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    project_id,
                    text,
                    *done,
                    rank as i64,
                    now,
                    m.and_then(|m| m.completed_at_ms),
                    m.and_then(|m| m.plan_date.clone()),
                    m.and_then(|m| m.dispatch_session_id.clone()),
                    m.and_then(|m| m.dispatch_at_ms),
                ],
            )?;
        }
        tx.execute(
            "INSERT INTO todo_legacy_imported (project_id, imported_ms) VALUES (?1, ?2)",
            params![project_id, now],
        )?;
        tx.commit()?;
        Ok(())
    }
}

/// 镜像 `dozer-app::extensions::todo::parse_todo`:只认顶格(列 0)的
/// `- [ ]`/`- [x]` 一级列表项,按文件出现顺序返回 `(text, done)`。
fn parse_legacy_markdown(md: &str) -> Vec<(String, bool)> {
    let mut items = Vec::new();
    for line in md.lines() {
        if let Some(rest) = line.strip_prefix("- [ ]") {
            items.push((rest.trim().to_string(), false));
        } else if let Some(rest) = line.strip_prefix("- [x]") {
            items.push((rest.trim().to_string(), true));
        }
    }
    items
}

/// 镜像 `dozer-app::extensions::todo::todo_line_key`:trim 后文本的
/// `DefaultHasher` 哈希,用来跟旧 `todo_meta.json` 里的记录关联。
fn legacy_todo_line_key(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.trim().hash(&mut hasher);
    hasher.finish()
}

struct LegacyMeta {
    dispatch_session_id: Option<String>,
    dispatch_at_ms: Option<i64>,
    plan_date: Option<String>,
    completed_at_ms: Option<i64>,
}

/// `SystemTime` 经 serde 序列化成 `{"secs_since_epoch": u64,
/// "nanos_since_epoch": u32}`(serde 标准库支持,`dozer-app` 那边
/// `TodoTaskMeta`/`DispatchRecord` 的 `SystemTime` 字段就是这么落盘的),
/// 这里手动转换成毫秒,不需要在 `dozerd` 里重新定义匹配的 Rust 结构体。
fn system_time_json_to_ms(v: &serde_json::Value) -> Option<i64> {
    let secs = v.get("secs_since_epoch")?.as_i64()?;
    let nanos = v
        .get("nanos_since_epoch")
        .and_then(|n| n.as_i64())
        .unwrap_or(0);
    Some(secs * 1000 + nanos / 1_000_000)
}

/// 读旧 `todo_meta.json`(`TodoMetaState = HashMap<i64 project_id,
/// HashMap<u64 hash, TodoTaskMeta>>`,整数 key 经 serde_json 序列化成字符串
/// key),按 `project_id` 取子表。文件不存在/JSON 损坏/这个 project_id
/// 没有记录,一律回落空表,不报错(与 `dozer-app` 侧 `meta_load` 同样的
/// 宽松读取哲学)。
fn read_legacy_meta_json(
    path: &Path,
    project_id: i64,
) -> std::collections::HashMap<u64, LegacyMeta> {
    let mut out = std::collections::HashMap::new();
    let Ok(raw) = std::fs::read_to_string(path) else {
        return out;
    };
    let Ok(root) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return out;
    };
    let Some(project_map) = root
        .get(project_id.to_string().as_str())
        .and_then(|v| v.as_object())
    else {
        return out;
    };
    for (key, entry) in project_map {
        let Ok(hash_key) = key.parse::<u64>() else {
            continue;
        };
        let dispatch = entry.get("dispatch").filter(|d| !d.is_null());
        out.insert(
            hash_key,
            LegacyMeta {
                dispatch_session_id: dispatch
                    .and_then(|d| d.get("session_id"))
                    .and_then(|s| s.as_str())
                    .map(String::from),
                dispatch_at_ms: dispatch
                    .and_then(|d| d.get("dispatched_at"))
                    .and_then(system_time_json_to_ms),
                plan_date: entry
                    .get("plan_date")
                    .filter(|v| !v.is_null())
                    .and_then(|v| v.as_str())
                    .map(String::from),
                completed_at_ms: entry
                    .get("completed_at")
                    .filter(|v| !v.is_null())
                    .and_then(system_time_json_to_ms),
            },
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, TodoStore) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.db");
        let store = TodoStore::new(&db).unwrap();
        (dir, store)
    }

    #[test]
    fn add_list_and_toggle() {
        let (_dir, store) = store();
        let t = store.add(1, "写 spec").unwrap();
        assert_eq!(t.text, "写 spec");
        assert!(!t.done);
        assert_eq!(t.completed_at_ms, None);

        let listed = store.list(1).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, t.id);

        let toggled = store.toggle(t.id, true).unwrap();
        assert!(toggled.done);
        assert!(toggled.completed_at_ms.is_some());

        let untoggled = store.toggle(t.id, false).unwrap();
        assert!(!untoggled.done);
        assert_eq!(untoggled.completed_at_ms, None);
    }

    #[test]
    fn toggle_unknown_id_errors() {
        let (_dir, store) = store();
        assert!(store.toggle(999, true).is_err());
    }

    #[test]
    fn add_prepends_new_task_before_existing_pending() {
        let (_dir, store) = store();
        let first = store.add(1, "第一条").unwrap();
        let second = store.add(1, "第二条").unwrap();
        let listed = store.list(1).unwrap();
        assert_eq!(listed[0].id, second.id, "后加的排最前");
        assert_eq!(listed[1].id, first.id);
    }

    #[test]
    fn done_tasks_sink_below_pending_regardless_of_rank() {
        let (_dir, store) = store();
        let a = store.add(1, "A").unwrap();
        let b = store.add(1, "B").unwrap();
        store.toggle(a.id, true).unwrap();
        let listed = store.list(1).unwrap();
        assert_eq!(listed[0].id, b.id, "待办在前");
        assert_eq!(listed[1].id, a.id, "已完成沉底");
    }

    #[test]
    fn edit_text_updates_and_unknown_id_errors() {
        let (_dir, store) = store();
        let t = store.add(1, "旧文字").unwrap();
        let edited = store.edit_text(t.id, "新文字").unwrap();
        assert_eq!(edited.text, "新文字");
        assert!(store.edit_text(999, "x").is_err());
    }

    #[test]
    fn set_plan_date_sets_and_clears() {
        let (_dir, store) = store();
        let t = store.add(1, "任务").unwrap();
        let with_date = store.set_plan_date(t.id, Some("08-10")).unwrap();
        assert_eq!(with_date.plan_date, Some("08-10".to_string()));
        let cleared = store.set_plan_date(t.id, None).unwrap();
        assert_eq!(cleared.plan_date, None);
    }

    #[test]
    fn record_dispatch_sets_session_and_timestamp() {
        let (_dir, store) = store();
        let t = store.add(1, "任务").unwrap();
        let dispatched = store.record_dispatch(t.id, "sess-1").unwrap();
        assert_eq!(dispatched.dispatch_session_id, Some("sess-1".to_string()));
        assert!(dispatched.dispatch_at_ms.is_some());
    }

    #[test]
    fn reorder_moves_task_after_target() {
        let (_dir, store) = store();
        let a = store.add(1, "A").unwrap();
        let b = store.add(1, "B").unwrap();
        let c = store.add(1, "C").unwrap();
        // 当前顺序(新加置顶): C, B, A。把 A 挪到 C 之后 → C, A, B。
        let moved = store.reorder(a.id, Some(c.id)).unwrap();
        assert_eq!(moved.id, a.id);
        let listed = store.list(1).unwrap();
        assert_eq!(
            listed.iter().map(|t| t.id).collect::<Vec<_>>(),
            vec![c.id, a.id, b.id]
        );
    }

    #[test]
    fn reorder_to_front_with_none() {
        let (_dir, store) = store();
        let a = store.add(1, "A").unwrap();
        let b = store.add(1, "B").unwrap();
        let moved = store.reorder(a.id, None).unwrap();
        assert_eq!(moved.rank, 0);
        let listed = store.list(1).unwrap();
        assert_eq!(listed[0].id, a.id);
        assert_eq!(listed[1].id, b.id);
    }

    #[test]
    fn reorder_unknown_id_errors() {
        let (_dir, store) = store();
        assert!(store.reorder(999, None).is_err());
    }

    #[test]
    fn tasks_are_isolated_per_project() {
        let (_dir, store) = store();
        store.add(1, "项目1的任务").unwrap();
        store.add(2, "项目2的任务").unwrap();
        assert_eq!(store.list(1).unwrap().len(), 1);
        assert_eq!(store.list(2).unwrap().len(), 1);
    }

    #[test]
    fn import_skips_when_no_md_file() {
        let (dir, store) = store();
        let md_path = dir.path().join("nope").join("todo.md");
        let meta_path = dir.path().join("nope").join("todo_meta.json");
        store
            .import_legacy_if_needed_at(1, &md_path, &meta_path)
            .unwrap();
        assert!(store.list(1).unwrap().is_empty());
    }

    #[test]
    fn import_parses_markdown_without_meta_file() {
        let (dir, store) = store();
        let md_path = dir.path().join("todo.md");
        std::fs::write(&md_path, "- [ ] 任务一\n- [x] 任务二\n").unwrap();
        let meta_path = dir.path().join("todo_meta.json");
        store
            .import_legacy_if_needed_at(1, &md_path, &meta_path)
            .unwrap();
        let listed = store.list(1).unwrap();
        assert_eq!(listed.len(), 2);
        assert!(listed.iter().any(|t| t.text == "任务一" && !t.done));
        assert!(listed.iter().any(|t| t.text == "任务二" && t.done));
    }

    #[test]
    fn import_correlates_meta_by_legacy_hash() {
        let (dir, store) = store();
        let md_path = dir.path().join("todo.md");
        std::fs::write(&md_path, "- [ ] 派发过的任务\n").unwrap();
        let key = legacy_todo_line_key("派发过的任务");
        let meta_json = serde_json::json!({
            "1": {
                key.to_string(): {
                    "dispatch": {
                        "session_id": "sess-abc",
                        "dispatched_at": {"secs_since_epoch": 1_700_000_000u64, "nanos_since_epoch": 0},
                    },
                    "plan_date": "08-10",
                    "completed_at": null,
                }
            }
        });
        let meta_path = dir.path().join("todo_meta.json");
        std::fs::write(&meta_path, meta_json.to_string()).unwrap();
        store
            .import_legacy_if_needed_at(1, &md_path, &meta_path)
            .unwrap();
        let listed = store.list(1).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].dispatch_session_id, Some("sess-abc".to_string()));
        assert_eq!(listed[0].dispatch_at_ms, Some(1_700_000_000_000));
        assert_eq!(listed[0].plan_date, Some("08-10".to_string()));
    }

    #[test]
    fn import_is_not_repeated_on_second_call() {
        let (dir, store) = store();
        let md_path = dir.path().join("todo.md");
        std::fs::write(&md_path, "- [ ] 任务\n").unwrap();
        let meta_path = dir.path().join("todo_meta.json");
        store
            .import_legacy_if_needed_at(1, &md_path, &meta_path)
            .unwrap();
        // 追加内容后再导入一次,不应该产生新行(已标记导入过)。
        std::fs::write(&md_path, "- [ ] 任务\n- [ ] 后来加的\n").unwrap();
        store
            .import_legacy_if_needed_at(1, &md_path, &meta_path)
            .unwrap();
        assert_eq!(store.list(1).unwrap().len(), 1);
    }

    #[test]
    fn import_is_isolated_per_project() {
        let (dir, store) = store();
        let md_path = dir.path().join("todo.md");
        std::fs::write(&md_path, "- [ ] 任务\n").unwrap();
        let meta_path = dir.path().join("todo_meta.json");
        store
            .import_legacy_if_needed_at(1, &md_path, &meta_path)
            .unwrap();
        // project 2 没导入过,即使复用同一份 md 路径也应该正常导入。
        store
            .import_legacy_if_needed_at(2, &md_path, &meta_path)
            .unwrap();
        assert_eq!(store.list(1).unwrap().len(), 1);
        assert_eq!(store.list(2).unwrap().len(), 1);
    }
}
