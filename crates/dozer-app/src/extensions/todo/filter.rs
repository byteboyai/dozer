//! Todo 纯过滤/分类函数:关键字过滤、分类树行、后代分类、按分类过滤、
//! 完成时间戳。

use dozer_core::protocol::{CategoryInfo, TodoInfo};

use super::*;

/// 纯前端关键字过滤：对 `TodoInfo.text` 做大小写不敏感子串匹配（空
/// `query` 不过滤）。作用在"已经解析+推导好状态"的内存列表上，不碰
/// 数据库（design 第 7 节）。左栏"按状态分类"筛选维度移除后，可视
/// 范围只由「关键字 × 自定义分类」两口径取交集决定。
pub fn filter_todos(items: &[TodoInfo], query: &str) -> Vec<usize> {
    let query_lower = query.trim().to_lowercase();
    items
        .iter()
        .enumerate()
        .filter(|(_, item)| {
            query_lower.is_empty() || item.text.to_lowercase().contains(&query_lower)
        })
        .map(|(i, _)| i)
        .collect()
}

/// 按 `parent_id` 把 `categories` 拼成树,按 `expanded` 展开态深度优先
/// 拍平成可见行(未展开节点的子孙不出现在结果里,但节点自身若有子节点
/// 仍会标 `has_children = true`,供左侧渲染箭头)。同级顺序按
/// `CategoryInfo.rank` 升序。
pub fn visible_category_rows(
    categories: &[CategoryInfo],
    expanded: &std::collections::HashSet<i64>,
) -> Vec<CategoryTreeRow> {
    let mut children_of: std::collections::HashMap<Option<i64>, Vec<&CategoryInfo>> =
        std::collections::HashMap::new();
    for c in categories {
        children_of.entry(c.parent_id).or_default().push(c);
    }
    for siblings in children_of.values_mut() {
        siblings.sort_by_key(|c| c.rank);
    }
    let mut rows = Vec::new();
    fn walk(
        parent: Option<i64>,
        depth: usize,
        children_of: &std::collections::HashMap<Option<i64>, Vec<&CategoryInfo>>,
        expanded: &std::collections::HashSet<i64>,
        rows: &mut Vec<CategoryTreeRow>,
    ) {
        let Some(siblings) = children_of.get(&parent) else {
            return;
        };
        for c in siblings {
            let has_children = children_of
                .get(&Some(c.id))
                .map(|v| !v.is_empty())
                .unwrap_or(false);
            let is_expanded = expanded.contains(&c.id);
            rows.push(CategoryTreeRow {
                id: c.id,
                name: c.name.clone(),
                depth,
                has_children,
                expanded: is_expanded,
            });
            if is_expanded {
                walk(Some(c.id), depth + 1, children_of, expanded, rows);
            }
        }
    }
    walk(None, 0, &children_of, expanded, &mut rows);
    rows
}

/// `root` 的全部子孙节点 id(不含 `root` 自己)。给"选中父节点汇总子孙
/// 任务"和"reparent 目标合法性校验"(GUI 侧提前拦截,dozerd 侧
/// `CategoryStore::reparent` 仍会再校验一次,双保险)复用。
pub fn category_descendants(
    categories: &[CategoryInfo],
    root: i64,
) -> std::collections::HashSet<i64> {
    let mut children_of: std::collections::HashMap<i64, Vec<i64>> =
        std::collections::HashMap::new();
    for c in categories {
        if let Some(p) = c.parent_id {
            children_of.entry(p).or_default().push(c.id);
        }
    }
    let mut out = std::collections::HashSet::new();
    let mut frontier = vec![root];
    while let Some(node) = frontier.pop() {
        if let Some(children) = children_of.get(&node) {
            for &child in children {
                if out.insert(child) {
                    frontier.push(child);
                }
            }
        }
    }
    out
}

/// 按当前分类过滤选中态,返回符合条件的任务 id 集合(不是下标——调用方
/// 若需要按下标跟既有 `filter_todos` 的结果取交集,自己按 `todos[i].id`
/// 是否在这个集合里判断)。`Node(id)` 汇总该节点及其全部子孙下的任务。
pub fn filter_todos_by_category(
    todos: &[TodoInfo],
    categories: &[CategoryInfo],
    filter: CategoryFilter,
) -> std::collections::HashSet<i64> {
    match filter {
        CategoryFilter::All => todos.iter().map(|t| t.id).collect(),
        CategoryFilter::Uncategorized => todos
            .iter()
            .filter(|t| t.category_id.is_none())
            .map(|t| t.id)
            .collect(),
        CategoryFilter::Node(root) => {
            let mut allowed = category_descendants(categories, root);
            allowed.insert(root);
            todos
                .iter()
                .filter(|t| t.category_id.is_some_and(|cid| allowed.contains(&cid)))
                .map(|t| t.id)
                .collect()
        }
    }
}

/// `done` 翻转成 `completed_at` 该有的值：完成 → `Some(now)`，取消
/// 完成 → `None`。抽成纯函数是为了能不起 GUI/不碰文件单测这条转换
/// 规则本身。
pub fn completed_at_for_toggle(
    done: bool,
    now: std::time::SystemTime,
) -> Option<std::time::SystemTime> {
    done.then_some(now)
}
