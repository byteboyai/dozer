//! Todo 存储:rusqlite,与 `ProjectStore`/`BookmarkStore`
//! 共享同一个 `dozer.db`(见 `main.rs` 里 `new` 时传入同一路径)。任务
//! 正文+全部元数据(派发记录/计划日期/完成时间)合并成一张表、一行一条
//! 任务,`id` 是稳定身份,取代 v1(`.dozer/todo.md` + `todo_meta.json`)
//! 靠"文本哈希"关联元数据的做法(2026-09-01 SQLite 迁移设计
//! `docs/superpowers/specs/2026-09-01-dozer-todo-sqlite-migration-design.md`)。

use anyhow::{Context, Result};
use dozer_core::protocol::{TodoInfo, TodoStoredStatus};
use rusqlite::{Connection, params};
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
const TODO_COLUMNS: &str = "id, project_id, text, done, paused, rank, created_ms, \
    completed_at_ms, plan_date, dispatch_session_id, dispatch_at_ms, category_id, \
    assigned_agent";

/// 只覆盖有 headless 适配器的四种(与 `dozerd::headless_agent::bare_program_name`
/// 覆盖面一致)——`assigned_agent` 列不会存其余三种。未识别字符串(理论上
/// 不会出现,防御性)落回 `None`,不是恐慌。
fn agent_from_label(s: &str) -> Option<dozer_core::protocol::AgentKind> {
    use dozer_core::protocol::AgentKind;
    match s {
        "claude" => Some(AgentKind::Claude),
        "codebuddy" => Some(AgentKind::Codebuddy),
        "opencode" => Some(AgentKind::Opencode),
        "v8agent" => Some(AgentKind::V8agent),
        _ => None,
    }
}

fn row_to_todo(row: &rusqlite::Row) -> rusqlite::Result<TodoInfo> {
    Ok(TodoInfo {
        id: row.get(0)?,
        project_id: row.get(1)?,
        text: row.get(2)?,
        done: row.get(3)?,
        paused: row.get(4)?,
        rank: row.get(5)?,
        created_ms: row.get::<_, i64>(6)? as u64,
        completed_at_ms: row.get::<_, Option<i64>>(7)?.map(|v| v as u64),
        plan_date: row.get(8)?,
        dispatch_session_id: row.get(9)?,
        dispatch_at_ms: row.get::<_, Option<i64>>(10)?.map(|v| v as u64),
        category_id: row.get(11)?,
        assigned_agent: row
            .get::<_, Option<String>>(12)?
            .and_then(|s| agent_from_label(&s)),
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
                paused INTEGER NOT NULL DEFAULT 0,
                rank INTEGER NOT NULL,
                created_ms INTEGER NOT NULL,
                completed_at_ms INTEGER,
                plan_date TEXT,
                dispatch_session_id TEXT,
                dispatch_at_ms INTEGER,
                category_id INTEGER,
                assigned_agent TEXT
             );
             CREATE INDEX IF NOT EXISTS idx_todos_project_order
                ON todos(project_id, done, paused, rank);
             CREATE INDEX IF NOT EXISTS idx_todos_dispatch_on_project
                ON todos(project_id, dispatch_at_ms);",
        )
        .context("建表")?;
        // 老库(建表时还没有 category_id/paused 列)迁移:CREATE TABLE IF NOT
        // EXISTS 对已存在的表不生效,新列需要单独补。SQLite 的 ALTER TABLE
        // ADD COLUMN 没有 IF NOT EXISTS 语法,靠 PRAGMA table_info 先查有没有
        // 再决定要不要补(同 transcripts.rs 的 is_error 列前例)。
        let has_category_id: bool = conn
            .prepare("SELECT 1 FROM pragma_table_info('todos') WHERE name = 'category_id'")?
            .exists([])?;
        if !has_category_id {
            conn.execute("ALTER TABLE todos ADD COLUMN category_id INTEGER", [])
                .context("迁移 category_id 列")?;
        }
        let has_paused: bool = conn
            .prepare("SELECT 1 FROM pragma_table_info('todos') WHERE name = 'paused'")?
            .exists([])?;
        if !has_paused {
            conn.execute(
                "ALTER TABLE todos ADD COLUMN paused INTEGER NOT NULL DEFAULT 0",
                [],
            )
            .context("迁移 paused 列")?;
        }
        let has_assigned_agent: bool = conn
            .prepare("SELECT 1 FROM pragma_table_info('todos') WHERE name = 'assigned_agent'")?
            .exists([])?;
        if !has_assigned_agent {
            conn.execute("ALTER TABLE todos ADD COLUMN assigned_agent TEXT", [])
                .context("迁移 assigned_agent 列")?;
        }
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// 某项目全部任务:进行中/待办(`done=0,paused=0`)按 rank 升序在最前,
    /// 搁置(`paused=1`)按 rank 升序其次,已完成(`done=1`)按 rank 升序沉底。
    /// (服务端不区分"进行中/待办",那是派生状态;同在此块内按 rank 排。)
    pub fn list(&self, project_id: i64) -> Result<Vec<TodoInfo>> {
        let conn = self.conn.lock().expect("db lock");
        let sql = format!(
            "SELECT {TODO_COLUMNS} FROM todos WHERE project_id = ?1
             ORDER BY done ASC, paused ASC, rank ASC"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([project_id], row_to_todo)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 查单条任务(`SetTodoCategory` 校验分类归属用)。`id` 不存在返回
    /// `Err`。
    pub fn get(&self, id: i64) -> Result<TodoInfo> {
        let conn = self.conn.lock().expect("db lock");
        let sql = format!("SELECT {TODO_COLUMNS} FROM todos WHERE id = ?1");
        conn.query_row(&sql, [id], row_to_todo)
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => id_not_found(id),
                e => e.into(),
            })
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
    /// `completed_at_for_toggle` 纯函数行为等价,合并进这一次 UPDATE。真
    /// (已完成)时同时清 `paused`(把一个已搁置任务勾成完成,不应同时残留
    /// "搁置"),假时不动 `paused`(取消勾选只回到待办/进行中,停不停搁置由
    /// 用户用状态下拉决定)。`id` 不存在 → `Err`。
    pub fn toggle(&self, id: i64, done: bool) -> Result<TodoInfo> {
        let conn = self.conn.lock().expect("db lock");
        let completed_at_ms: Option<i64> = done.then(|| now_ms() as i64);
        let sql = format!(
            "UPDATE todos SET done = ?1,
                completed_at_ms = ?2,
                paused = CASE WHEN ?1 = 1 THEN 0 ELSE paused END
             WHERE id = ?3 RETURNING {TODO_COLUMNS}"
        );
        conn.query_row(&sql, params![done, completed_at_ms, id], row_to_todo)
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => id_not_found(id),
                e => e.into(),
            })
    }

    /// 把任务设为某个已存储逻辑状态(`TodoStoredStatus`),一次 UPDATE 原子落定
    /// (覆盖 `done`/`paused`/派发记录三者的组合,避免"先改 done 再改 paused"
    /// 两步各自落盘被打断的脏状态)。语义:
    /// - `Todo`:完成撤销、搁置撤销、派发摘掉(拿回即脱离会话);若由已完成降级,
    ///   顺手清 `completed_at_ms`。
    /// - `Suspended`:完成撤销、置搁置、摘派发(同「拿回」,搁置即不推进);若由
    ///   已完成降级,顺手清 `completed_at_ms`。
    /// - `Done`:置完成、搁置撤销、写 `completed_at_ms`(等价勾选)。
    ///   只下已存储字段;`进行中`(存活派发)派生状态不在本函数管辖。
    pub fn set_status(&self, id: i64, status: TodoStoredStatus) -> Result<TodoInfo> {
        let conn = self.conn.lock().expect("db lock");
        let now = now_ms() as i64;
        let (done, paused, unset_dispatch, completed): (i64, i64, bool, Option<i64>) = match status
        {
            TodoStoredStatus::Done => (1, 0, false, Some(now)),
            TodoStoredStatus::Suspended => (0, 1, true, None),
            TodoStoredStatus::Todo => (0, 0, true, None),
        };
        let sql = format!(
            "UPDATE todos SET done = ?1, paused = ?2,
                dispatch_session_id = CASE WHEN ?3 THEN NULL ELSE dispatch_session_id END,
                dispatch_at_ms = CASE WHEN ?3 THEN NULL ELSE dispatch_at_ms END,
                completed_at_ms = ?4
             WHERE id = ?5 RETURNING {TODO_COLUMNS}"
        );
        conn.query_row(
            &sql,
            params![done, paused, unset_dispatch, completed, id],
            row_to_todo,
        )
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

    /// 指派随身 agent 去 headless 干这条任务(干完回填 `process_agent` / 两条
    /// `__chat` turn,派生回 `session_id`)。指派不申请也不会清掉已有
    /// `dispatch_at_ms` 存活派发。
    pub fn assign_agent(
        &self,
        id: i64,
        agent: dozer_core::protocol::AgentKind,
    ) -> Result<TodoInfo> {
        let conn = self.conn.lock().expect("db lock");
        let label = agent.label().to_string();
        let sql =
            format!("UPDATE todos SET assigned_agent = ?1 WHERE id = ?2 RETURNING {TODO_COLUMNS}");
        conn.query_row(&sql, params![label, id], row_to_todo)
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => id_not_found(id),
                e => e.into(),
            })
    }

    /// 首次真正触发 headless 处理时,由 `task_processor` 铸造好
    /// `session_id` 后写回;之后每次处理复用同一个,不再变化。**不**碰
    /// `assigned_agent`——那是独立的"长期指派给谁"状态,`dispatch_session_id`
    /// 只是"当前处理会话"标识,两者互不清空(此前误加过
    /// `assigned_agent = NULL`,会导致任务处理一轮后从轮询扫描里永久消失、
    /// 也会让"处理"按钮后续调用因 `assigned_agent` 为空而直接报错,已修复)。
    pub fn set_dispatch_session(&self, id: i64, session_id: &str) -> Result<TodoInfo> {
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

    /// 某分类下已指派但未完成的任务(供 `task_poller` 扫描用)。
    pub fn list_assigned_incomplete_in_category(&self, category_id: i64) -> Result<Vec<TodoInfo>> {
        let conn = self.conn.lock().expect("db lock");
        let sql = format!(
            "SELECT {TODO_COLUMNS} FROM todos
             WHERE category_id = ?1 AND done = 0 AND assigned_agent IS NOT NULL
             ORDER BY rank ASC"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([category_id], row_to_todo)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 分页拉某项目"需要推进"的活动派发任务:进行中(`done=0,paused=0`)里
    /// 有存活派发(`dispatch_at_ms NOT NULL`)的那些,从小到大取 `limit` 条
    /// (给 dispatch_at 加索引以保证稳定升序)。
    pub fn dispatchable_pending(&self, project_id: i64, limit: i64) -> Result<Vec<TodoInfo>> {
        let conn = self.conn.lock().expect("db lock");
        let sql = format!(
            "SELECT {TODO_COLUMNS} FROM todos
             WHERE project_id = ?1 AND done = 0 AND paused = 0
               AND dispatch_at_ms IS NOT NULL
             ORDER BY dispatch_at_ms ASC
             LIMIT ?2"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![project_id, limit], row_to_todo)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// `[dispatch_session_id] = ?` 为空的都行的稳定备用 id 源不用加索引。
    /// 全表查归自己 project 下 `[dispatch_at_ms] IS NOT NULL`：选中的调度用
    /// 例行被 processnow 打断,完成后清空,避免再被秒级轮询拾起来反复开进程。
    /// 补充索引(见 `new()`)。返回带 `done`/`paused`(前端进度判定用)。
    pub fn occupying_sessions(&self, project_id: i64) -> Result<Vec<TodoInfo>> {
        let conn = self.conn.lock().expect("db lock");
        let sql = format!(
            "SELECT {TODO_COLUMNS} FROM todos
             WHERE project_id = ?1 AND dispatch_at_ms IS NOT NULL
             ORDER BY rank ASC"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([project_id], row_to_todo)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 挂/摘任务的分类。`category_id: None` 摘掉分类(变回未分类)。
    /// **不校验** `category_id` 指向的分类是否存在——那是跨 store 的
    /// 校验,`TodoStore` 不知道 `CategoryStore` 的存在,交给
    /// `server.rs` 的请求处理器在调用前做(见 Task 4)。`id` 不存在
    /// 返回 `Err`。
    pub fn set_category(&self, id: i64, category_id: Option<i64>) -> Result<TodoInfo> {
        let conn = self.conn.lock().expect("db lock");
        let sql =
            format!("UPDATE todos SET category_id = ?1 WHERE id = ?2 RETURNING {TODO_COLUMNS}");
        conn.query_row(&sql, params![category_id, id], row_to_todo)
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
        let dispatched = store.set_dispatch_session(t.id, "sess-1").unwrap();
        assert_eq!(dispatched.dispatch_session_id, Some("sess-1".to_string()));
        assert!(dispatched.dispatch_at_ms.is_some());
    }

    #[test]
    fn assign_agent_sets_label() {
        use dozer_core::protocol::AgentKind;
        let (_dir, store) = store();
        let t = store.add(1, "任务").unwrap();
        let a = store.assign_agent(t.id, AgentKind::Claude).unwrap();
        assert_eq!(a.assigned_agent, Some(AgentKind::Claude));
        assert!(store.assign_agent(999, AgentKind::Claude).is_err());
    }

    #[test]
    fn set_dispatch_session_preserves_assigned_agent() {
        use dozer_core::protocol::AgentKind;
        let (_dir, store) = store();
        let t = store.add(1, "任务").unwrap();
        store.assign_agent(t.id, AgentKind::Claude).unwrap();
        let dispatched = store.set_dispatch_session(t.id, "sess-1").unwrap();
        assert_eq!(
            dispatched.assigned_agent,
            Some(AgentKind::Claude),
            "铸造/复用处理会话不应清掉长期指派——否则轮询扫描和后续\"处理\"按钮都会失效"
        );
        assert_eq!(dispatched.dispatch_session_id, Some("sess-1".to_string()));
    }

    #[test]
    fn dispatchable_pending_and_occupying_sessions_filter() {
        let (_dir, store) = store();
        let a = store.add(1, "派发1").unwrap();
        let b = store.add(1, "派发2").unwrap();
        let c = store.add(1, "已完成派发").unwrap();
        store.set_dispatch_session(a.id, "s1").unwrap();
        store.set_dispatch_session(b.id, "s2").unwrap();
        store.set_dispatch_session(c.id, "s3").unwrap();
        store.toggle(c.id, true).unwrap();

        // c 已 done:dispatchable 只该有两张;dispatching 汇总也该含已 done 的 c。
        let dp = store.dispatchable_pending(1, 10).unwrap();
        assert_eq!(dp.len(), 2);
        let occ = store.occupying_sessions(1).unwrap();
        assert_eq!(occ.len(), 3);
    }

    #[test]
    fn list_assigned_incomplete_in_category_filters_correctly() {
        use dozer_core::protocol::AgentKind;
        let (_dir, store) = store();
        let t1 = store.add(1, "已指派未完成").unwrap();
        store.assign_agent(t1.id, AgentKind::Claude).unwrap();
        store.set_category(t1.id, Some(10)).unwrap();
        let t2 = store.add(1, "未指派").unwrap();
        store.set_category(t2.id, Some(10)).unwrap();
        let t3 = store.add(1, "已指派已完成").unwrap();
        store.assign_agent(t3.id, AgentKind::Claude).unwrap();
        store.set_category(t3.id, Some(10)).unwrap();
        store.toggle(t3.id, true).unwrap();

        let result = store.list_assigned_incomplete_in_category(10).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, t1.id);
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
    fn get_returns_task_and_unknown_id_errors() {
        let (_dir, store) = store();
        let t = store.add(1, "任务").unwrap();
        let fetched = store.get(t.id).unwrap();
        assert_eq!(fetched.id, t.id);
        assert!(store.get(999).is_err());
    }

    #[test]
    fn set_category_assigns_and_clears() {
        let (_dir, store) = store();
        let t = store.add(1, "任务").unwrap();
        assert_eq!(t.category_id, None);

        let categorized = store.set_category(t.id, Some(42)).unwrap();
        assert_eq!(categorized.category_id, Some(42));

        let cleared = store.set_category(t.id, None).unwrap();
        assert_eq!(cleared.category_id, None);
    }

    #[test]
    fn set_category_unknown_id_errors() {
        let (_dir, store) = store();
        assert!(store.set_category(999, Some(1)).is_err());
    }

    #[test]
    fn added_tasks_default_not_paused_and_list_orders_paused_middle() {
        let (_dir, store) = store();
        let t = store.add(1, "任务").unwrap();
        assert!(!t.done);
        assert!(!t.paused);

        // 建三条,分别落成 待办 / 搁置 / 已完成,期望三段次序。
        let paused = store.add(1, "搁置任务").unwrap();
        let done = store.add(1, "完成后沉底").unwrap();
        store.toggle(done.id, true).unwrap();
        store
            .set_status(paused.id, TodoStoredStatus::Suspended)
            .unwrap();
        store.add(1, "进行中或待办").unwrap(); // 新加置顶属活动段

        let listed = store.list(1).unwrap();
        let texts: Vec<&str> = listed.iter().map(|i| i.text.as_str()).collect();
        let a = texts.iter().position(|&x| x == "进行中或待办").unwrap();
        let b = texts.iter().position(|&x| x == "搁置任务").unwrap();
        let c = texts.iter().position(|&x| x == "完成后沉底").unwrap();
        assert!(a < b, "活动待办在搁置前");
        assert!(b < c, "搁置在已完成前");
        assert!(listed.get(b).unwrap().paused);
    }

    #[test]
    fn set_status_maps_three_stored_states() {
        let (_dir, store) = store();
        let t = store.add(1, "任务").unwrap();

        // → 搁置:置 paused、清派发,未完成
        store.set_dispatch_session(t.id, "sess-1").unwrap();
        let s = store.set_status(t.id, TodoStoredStatus::Suspended).unwrap();
        assert!(s.paused);
        assert!(!s.done);
        assert_eq!(s.dispatch_session_id, None);

        // → 已完成:完成 + 撤销搁置
        let done = store.set_status(t.id, TodoStoredStatus::Done).unwrap();
        assert!(done.done);
        assert!(!done.paused);
        assert!(done.completed_at_ms.is_some());

        // → 待办:完成/搁置都撤销、摘派发
        store.set_dispatch_session(t.id, "sess-2").unwrap();
        let todo = store.set_status(t.id, TodoStoredStatus::Todo).unwrap();
        assert!(!todo.done);
        assert!(!todo.paused);
        assert_eq!(todo.dispatch_session_id, None);
        assert_eq!(todo.completed_at_ms, None);
    }

    #[test]
    fn set_status_unknown_id_errors() {
        let (_dir, store) = store();
        assert!(store.set_status(999, TodoStoredStatus::Done).is_err());
    }

    #[test]
    fn toggle_done_clears_paused_and_back_keeps_it_as_last_set() {
        let (_dir, store) = store();
        let t = store.add(1, "任务").unwrap();
        store.set_status(t.id, TodoStoredStatus::Suspended).unwrap();
        let done = store.toggle(t.id, true).unwrap();
        assert!(done.done);
        assert!(!done.paused, "已完成时不应残留搁置");
        store.toggle(t.id, false).unwrap();
        let listed = store.list(1).unwrap();
        assert!(!listed[0].paused, "取消勾选回到待办(不残留搁置)");
    }
}
