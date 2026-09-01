//! 分类树存储:与 `TodoStore` 平级、共享同一个 `dozer.db`。一行一个
//! 分类节点,`parent_id` 表达树形结构;删除/reparent 的级联逻辑全部在
//! Rust 层手动实现(不用 SQLite 外键 `ON DELETE` 系列——级联规则本身
//! 是业务语义,见 2026-09-01 分类树设计)。

use anyhow::{Context, Result};
use dozer_core::protocol::{CategoryInfo, CategoryMoveDirection};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;
use std::sync::Mutex;

pub struct CategoryStore {
    conn: Mutex<Connection>,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn id_not_found(id: i64) -> anyhow::Error {
    anyhow::anyhow!("分类不存在: id={id}")
}

const CATEGORY_COLUMNS: &str = "id, project_id, parent_id, name, rank, created_ms";

fn row_to_category(row: &rusqlite::Row) -> rusqlite::Result<CategoryInfo> {
    Ok(CategoryInfo {
        id: row.get(0)?,
        project_id: row.get(1)?,
        parent_id: row.get(2)?,
        name: row.get(3)?,
        rank: row.get(4)?,
        created_ms: row.get::<_, i64>(5)? as u64,
    })
}

impl CategoryStore {
    pub fn new(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("建库目录")?;
        }
        let conn = Connection::open(path).context("打开 SQLite")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS todo_categories (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                project_id INTEGER NOT NULL,
                parent_id INTEGER,
                name TEXT NOT NULL,
                rank INTEGER NOT NULL,
                created_ms INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_todo_categories_project_parent
                ON todo_categories(project_id, parent_id, rank);",
        )
        .context("建表")?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// 某项目全部分类节点,扁平返回(树形结构由调用方按 `parent_id` 自己
    /// 拼)。数据量小,不做懒加载/分页。
    pub fn list(&self, project_id: i64) -> Result<Vec<CategoryInfo>> {
        let conn = self.conn.lock().expect("db lock");
        let sql = format!(
            "SELECT {CATEGORY_COLUMNS} FROM todo_categories WHERE project_id = ?1 ORDER BY parent_id ASC, rank ASC"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([project_id], row_to_category)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 同一 `parent_id` 下下一个 `rank`(当前最大 + 1,该 `parent_id` 下
    /// 还没有节点则从 0 开始)。`add`/`reparent` 共用。
    fn next_sibling_rank(
        conn: &Connection,
        project_id: i64,
        parent_id: Option<i64>,
    ) -> rusqlite::Result<i64> {
        let max_rank: Option<i64> = conn.query_row(
            "SELECT MAX(rank) FROM todo_categories WHERE project_id = ?1
             AND parent_id IS ?2",
            params![project_id, parent_id],
            |r| r.get(0),
        )?;
        Ok(max_rank.map(|r| r + 1).unwrap_or(0))
    }

    /// 新增分类,追加到 `parent_id` 下兄弟节点末尾。
    pub fn add(&self, project_id: i64, parent_id: Option<i64>, name: &str) -> Result<CategoryInfo> {
        let conn = self.conn.lock().expect("db lock");
        let rank = Self::next_sibling_rank(&conn, project_id, parent_id)?;
        let now = now_ms() as i64;
        let sql = format!(
            "INSERT INTO todo_categories (project_id, parent_id, name, rank, created_ms)
             VALUES (?1, ?2, ?3, ?4, ?5) RETURNING {CATEGORY_COLUMNS}"
        );
        conn.query_row(
            &sql,
            params![project_id, parent_id, name, rank, now],
            row_to_category,
        )
        .map_err(Into::into)
    }

    pub fn rename(&self, id: i64, name: &str) -> Result<CategoryInfo> {
        let conn = self.conn.lock().expect("db lock");
        let sql =
            format!("UPDATE todo_categories SET name = ?1 WHERE id = ?2 RETURNING {CATEGORY_COLUMNS}");
        conn.query_row(&sql, params![name, id], row_to_category)
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => id_not_found(id),
                e => e.into(),
            })
    }

    /// 递归收集 `root` 自身及其全部子孙分类 id(树深度不大,BFS 就够,不
    /// 需要 SQL 递归 CTE)。
    fn collect_subtree_ids(conn: &Connection, root: i64) -> rusqlite::Result<Vec<i64>> {
        let mut ids = vec![root];
        let mut frontier = vec![root];
        while !frontier.is_empty() {
            let mut next = Vec::new();
            for parent in frontier {
                let mut stmt =
                    conn.prepare("SELECT id FROM todo_categories WHERE parent_id = ?1")?;
                let children: Vec<i64> =
                    stmt.query_map([parent], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
                next.extend(children);
            }
            ids.extend(&next);
            frontier = next;
        }
        Ok(ids)
    }

    /// 删除该节点及其全部子孙节点;这棵子树下原本挂靠的任务全部降级为
    /// 未分类(`category_id = NULL`),任务本身不删除。`id` 不存在返回
    /// `Err`(不是静默 no-op,对齐 `ProjectStore::rename`/`remove` 风格)。
    pub fn delete(&self, id: i64) -> Result<()> {
        let mut conn = self.conn.lock().expect("db lock");
        let exists: bool = conn
            .query_row(
                "SELECT 1 FROM todo_categories WHERE id = ?1",
                [id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !exists {
            return Err(id_not_found(id));
        }
        let subtree_ids = Self::collect_subtree_ids(&conn, id)?;
        let tx = conn.transaction()?;
        {
            let placeholders = subtree_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let update_sql =
                format!("UPDATE todos SET category_id = NULL WHERE category_id IN ({placeholders})");
            let params: Vec<&dyn rusqlite::ToSql> =
                subtree_ids.iter().map(|id| id as &dyn rusqlite::ToSql).collect();
            tx.execute(&update_sql, params.as_slice())?;
            let delete_sql =
                format!("DELETE FROM todo_categories WHERE id IN ({placeholders})");
            tx.execute(&delete_sql, params.as_slice())?;
        }
        tx.commit()?;
        Ok(())
    }

    /// 沿 `parent_id` 链从 `start` 往根走,判断路径上是否经过 `target`——
    /// 用来判断"把 `id` 挪到 `candidate` 下面会不会成环":如果从
    /// `candidate` 往上走能走到 `id`,说明 `candidate` 是 `id` 的子孙,
    /// 不能把 `id` 挂到自己的子孙下面。
    fn is_ancestor_or_self(conn: &Connection, start: i64, target: i64) -> rusqlite::Result<bool> {
        let mut cur = Some(start);
        while let Some(node) = cur {
            if node == target {
                return Ok(true);
            }
            cur = conn
                .query_row(
                    "SELECT parent_id FROM todo_categories WHERE id = ?1",
                    [node],
                    |r| r.get::<_, Option<i64>>(0),
                )
                .optional()?
                .flatten();
        }
        Ok(false)
    }

    /// 重新挂到 `new_parent_id` 下(`None` = 顶层),追加到新父节点子级
    /// 末尾。目标是自己或自己的子孙时拒绝(防止成环)。`id` 不存在返回
    /// `Err`。
    pub fn reparent(&self, id: i64, new_parent_id: Option<i64>) -> Result<CategoryInfo> {
        let conn = self.conn.lock().expect("db lock");
        let project_id: i64 = conn
            .query_row(
                "SELECT project_id FROM todo_categories WHERE id = ?1",
                [id],
                |r| r.get(0),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => id_not_found(id),
                e => e.into(),
            })?;
        if let Some(target) = new_parent_id {
            if Self::is_ancestor_or_self(&conn, target, id)? {
                anyhow::bail!("不能把分类移动到自己或自己的子分类下面");
            }
        }
        let rank = Self::next_sibling_rank(&conn, project_id, new_parent_id)?;
        let sql = format!(
            "UPDATE todo_categories SET parent_id = ?1, rank = ?2 WHERE id = ?3
             RETURNING {CATEGORY_COLUMNS}"
        );
        conn.query_row(&sql, params![new_parent_id, rank, id], row_to_category)
            .map_err(Into::into)
    }

    /// 与同一 `parent_id` 下相邻的前一个/后一个兄弟节点交换 `rank`。
    /// 已经在最前/最后时对应方向是 no-op(返回当前状态,不报错)。`id`
    /// 不存在返回 `Err`。
    pub fn move_sibling(&self, id: i64, direction: CategoryMoveDirection) -> Result<CategoryInfo> {
        let mut conn = self.conn.lock().expect("db lock");
        let (project_id, parent_id, rank): (i64, Option<i64>, i64) = conn
            .query_row(
                "SELECT project_id, parent_id, rank FROM todo_categories WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => id_not_found(id),
                e => e.into(),
            })?;
        let neighbor: Option<(i64, i64)> = match direction {
            CategoryMoveDirection::Up => conn
                .query_row(
                    "SELECT id, rank FROM todo_categories WHERE project_id = ?1
                     AND parent_id IS ?2 AND rank < ?3 ORDER BY rank DESC LIMIT 1",
                    params![project_id, parent_id, rank],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?,
            CategoryMoveDirection::Down => conn
                .query_row(
                    "SELECT id, rank FROM todo_categories WHERE project_id = ?1
                     AND parent_id IS ?2 AND rank > ?3 ORDER BY rank ASC LIMIT 1",
                    params![project_id, parent_id, rank],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?,
        };
        if let Some((neighbor_id, neighbor_rank)) = neighbor {
            let tx = conn.transaction()?;
            tx.execute(
                "UPDATE todo_categories SET rank = ?1 WHERE id = ?2",
                params![neighbor_rank, id],
            )?;
            tx.execute(
                "UPDATE todo_categories SET rank = ?1 WHERE id = ?2",
                params![rank, neighbor_id],
            )?;
            tx.commit()?;
        }
        let sql = format!("SELECT {CATEGORY_COLUMNS} FROM todo_categories WHERE id = ?1");
        conn.query_row(&sql, [id], row_to_category)
            .map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, CategoryStore) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.db");
        let store = CategoryStore::new(&db).unwrap();
        (dir, store)
    }

    #[test]
    fn add_and_list_top_level() {
        let (_dir, store) = store();
        let a = store.add(1, None, "前端").unwrap();
        let b = store.add(1, None, "后端").unwrap();
        assert_eq!(a.rank, 0);
        assert_eq!(b.rank, 1, "追加到末尾,不是置顶");
        let listed = store.list(1).unwrap();
        assert_eq!(listed.len(), 2);
    }

    #[test]
    fn add_child_ranks_independent_of_siblings_at_other_levels() {
        let (_dir, store) = store();
        let parent = store.add(1, None, "前端").unwrap();
        let child1 = store.add(1, Some(parent.id), "UI").unwrap();
        let child2 = store.add(1, Some(parent.id), "性能").unwrap();
        assert_eq!(child1.rank, 0);
        assert_eq!(child2.rank, 1);
        // 顶层再加一个,rank 从顶层自己的序列算,不受子级影响。
        let sibling = store.add(1, None, "后端").unwrap();
        assert_eq!(sibling.rank, 1);
    }

    #[test]
    fn categories_are_isolated_per_project() {
        let (_dir, store) = store();
        store.add(1, None, "项目1分类").unwrap();
        store.add(2, None, "项目2分类").unwrap();
        assert_eq!(store.list(1).unwrap().len(), 1);
        assert_eq!(store.list(2).unwrap().len(), 1);
    }

    #[test]
    fn rename_updates_name_and_unknown_id_errors() {
        let (_dir, store) = store();
        let a = store.add(1, None, "旧名字").unwrap();
        let renamed = store.rename(a.id, "新名字").unwrap();
        assert_eq!(renamed.name, "新名字");
        assert!(store.rename(999, "x").is_err());
    }

    #[test]
    fn delete_removes_node_and_descendants() {
        let (_dir, store) = store();
        let parent = store.add(1, None, "前端").unwrap();
        let child = store.add(1, Some(parent.id), "UI").unwrap();
        let grandchild = store.add(1, Some(child.id), "组件库").unwrap();
        store.delete(parent.id).unwrap();
        let listed = store.list(1).unwrap();
        assert!(listed.is_empty(), "父子孙三级都应该被删掉: {listed:?}");
        let _ = grandchild; // 仅用于构造场景
    }

    #[test]
    fn delete_unknown_id_errors() {
        let (_dir, store) = store();
        assert!(store.delete(999).is_err());
    }

    #[test]
    fn reparent_moves_to_new_parent_appended_to_end() {
        let (_dir, store) = store();
        let a = store.add(1, None, "A").unwrap();
        let b = store.add(1, None, "B").unwrap();
        let c = store.add(1, None, "C").unwrap(); // 挪去当 B 的子节点
        let moved = store.reparent(c.id, Some(b.id)).unwrap();
        assert_eq!(moved.parent_id, Some(b.id));
        assert_eq!(moved.rank, 0, "B 下还没有子节点,从 0 开始");
        let _ = a;
    }

    #[test]
    fn reparent_to_self_is_rejected() {
        let (_dir, store) = store();
        let a = store.add(1, None, "A").unwrap();
        assert!(store.reparent(a.id, Some(a.id)).is_err());
    }

    #[test]
    fn reparent_to_own_descendant_is_rejected() {
        let (_dir, store) = store();
        let parent = store.add(1, None, "前端").unwrap();
        let child = store.add(1, Some(parent.id), "UI").unwrap();
        let grandchild = store.add(1, Some(child.id), "组件库").unwrap();
        // 想把「前端」挪到自己的孙节点「组件库」下面——必须拒绝,否则成环。
        assert!(store.reparent(parent.id, Some(grandchild.id)).is_err());
        // 直接子节点同理。
        assert!(store.reparent(parent.id, Some(child.id)).is_err());
    }

    #[test]
    fn reparent_to_top_level_with_none() {
        let (_dir, store) = store();
        let parent = store.add(1, None, "前端").unwrap();
        let child = store.add(1, Some(parent.id), "UI").unwrap();
        let moved = store.reparent(child.id, None).unwrap();
        assert_eq!(moved.parent_id, None);
    }

    #[test]
    fn reparent_unknown_id_errors() {
        let (_dir, store) = store();
        assert!(store.reparent(999, None).is_err());
    }

    #[test]
    fn move_sibling_swaps_rank_with_neighbor() {
        let (_dir, store) = store();
        let a = store.add(1, None, "A").unwrap(); // rank 0
        let b = store.add(1, None, "B").unwrap(); // rank 1
        let c = store.add(1, None, "C").unwrap(); // rank 2
        // 把 B 上移:B、A 交换 rank → 顺序变 B, A, C。
        store
            .move_sibling(b.id, CategoryMoveDirection::Up)
            .unwrap();
        let listed = store.list(1).unwrap();
        assert_eq!(
            listed.iter().map(|c| c.id).collect::<Vec<_>>(),
            vec![b.id, a.id, c.id]
        );
    }

    #[test]
    fn move_sibling_up_at_front_is_noop() {
        let (_dir, store) = store();
        let a = store.add(1, None, "A").unwrap();
        let b = store.add(1, None, "B").unwrap();
        let result = store
            .move_sibling(a.id, CategoryMoveDirection::Up)
            .unwrap();
        assert_eq!(result.rank, 0, "已经在最前,不报错也不改变");
        let listed = store.list(1).unwrap();
        assert_eq!(listed[0].id, a.id);
        assert_eq!(listed[1].id, b.id);
    }

    #[test]
    fn move_sibling_down_at_back_is_noop() {
        let (_dir, store) = store();
        let a = store.add(1, None, "A").unwrap();
        let b = store.add(1, None, "B").unwrap();
        store
            .move_sibling(b.id, CategoryMoveDirection::Down)
            .unwrap();
        let listed = store.list(1).unwrap();
        assert_eq!(listed[0].id, a.id);
        assert_eq!(listed[1].id, b.id, "已经在最后,顺序不变");
    }

    #[test]
    fn move_sibling_only_affects_same_parent() {
        let (_dir, store) = store();
        let parent = store.add(1, None, "前端").unwrap();
        let child = store.add(1, Some(parent.id), "UI").unwrap();
        let top_level = store.add(1, None, "后端").unwrap();
        // child 只有一个同级(它自己),上移应该是 no-op,不会跟顶层的
        // top_level 混到一起交换。
        let result = store
            .move_sibling(child.id, CategoryMoveDirection::Up)
            .unwrap();
        assert_eq!(result.parent_id, Some(parent.id));
        let _ = top_level;
    }
}
