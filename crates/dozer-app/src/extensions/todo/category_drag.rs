//! 左栏分类树的拖动移动:放置目标、合法性与松手收尾的纯逻辑。机制照抄文件树
//! `TreeDrag`(按下武装 `Pending` → 越过距离+时长阈值确认 `Dragging` → 松手收尾),
//! 见 `app/layout.rs::tree_drag_past_threshold` / `tree_drag_held_long_enough`。

use super::category_descendants;
use dozer_core::protocol::CategoryInfo;

/// 拖拽放置目标:顶层(「全部」伪节点)或某个真实分类节点。「未分类」伪节点
/// 不是合法目标(它不对应真实节点),所以没有对应变体。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropTarget {
    Root,
    Node(i64),
}

/// `source` 能否放到 `target`。无意义的移动(放到当前父节点、顶层节点放到顶层)
/// 一律判不合法——UI 不高亮、松手空操作,也就不发请求。服务端 `reparent`
/// 仍会再校验一次环检测,双保险。
pub fn is_valid_drop(categories: &[CategoryInfo], source: i64, target: DropTarget) -> bool {
    let Some(src) = categories.iter().find(|c| c.id == source) else {
        return false;
    };
    match target {
        DropTarget::Root => src.parent_id.is_some(),
        DropTarget::Node(t) => {
            if t == source || !categories.iter().any(|c| c.id == t) {
                return false;
            }
            if src.parent_id == Some(t) {
                return false;
            }
            !category_descendants(categories, source).contains(&t)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cat(id: i64, parent: Option<i64>) -> CategoryInfo {
        serde_json::from_value(json!({
            "id": id, "project_id": 1, "parent_id": parent, "name": format!("c{id}"),
            "rank": id, "created_ms": 0
        }))
        .expect("CategoryInfo")
    }

    /// 1 ─┬─ 2 ── 4
    ///    └─ 3
    /// 5(顶层)
    fn tree() -> Vec<CategoryInfo> {
        vec![
            cat(1, None),
            cat(2, Some(1)),
            cat(3, Some(1)),
            cat(4, Some(2)),
            cat(5, None),
        ]
    }

    // Review Focus 3
    #[test]
    fn cannot_drop_onto_itself() {
        assert!(!is_valid_drop(&tree(), 2, DropTarget::Node(2)));
    }

    #[test]
    fn cannot_drop_onto_own_descendant() {
        assert!(!is_valid_drop(&tree(), 1, DropTarget::Node(4)));
        assert!(!is_valid_drop(&tree(), 1, DropTarget::Node(2)));
        assert!(!is_valid_drop(&tree(), 2, DropTarget::Node(4)));
    }

    #[test]
    fn dropping_onto_current_parent_is_a_noop_and_invalid() {
        assert!(!is_valid_drop(&tree(), 2, DropTarget::Node(1)));
        assert!(!is_valid_drop(&tree(), 4, DropTarget::Node(2)));
    }

    #[test]
    fn can_drop_onto_unrelated_nodes_and_siblings() {
        assert!(is_valid_drop(&tree(), 2, DropTarget::Node(3))); // 兄弟:变成它的子分类
        assert!(is_valid_drop(&tree(), 4, DropTarget::Node(5))); // 换到另一棵树
        assert!(is_valid_drop(&tree(), 5, DropTarget::Node(1)));
        assert!(is_valid_drop(&tree(), 4, DropTarget::Node(1))); // 上移一层
    }

    #[test]
    fn root_target_is_valid_only_when_the_node_is_not_already_top_level() {
        assert!(is_valid_drop(&tree(), 2, DropTarget::Root));
        assert!(is_valid_drop(&tree(), 4, DropTarget::Root));
        assert!(!is_valid_drop(&tree(), 1, DropTarget::Root));
        assert!(!is_valid_drop(&tree(), 5, DropTarget::Root));
    }

    #[test]
    fn unknown_source_or_target_is_invalid() {
        assert!(!is_valid_drop(&tree(), 99, DropTarget::Root));
        assert!(!is_valid_drop(&tree(), 99, DropTarget::Node(1)));
        assert!(!is_valid_drop(&tree(), 2, DropTarget::Node(99)));
    }
}
