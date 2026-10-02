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

use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CategoryDragPhase {
    /// 已按下武装,但还没越过阈值:此时**不得有任何视觉反应**("点一下"零副作用)。
    Pending,
    /// 已确认是一次真实拖拽:行才挂 `on_move`、换抓取光标、画落点高亮。
    Dragging,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CategoryDrag {
    pub source: i64,
    pub phase: CategoryDragPhase,
    /// 按下瞬间的光标位置(逻辑像素),用来过距离阈值。
    pub press_pos: (f32, f32),
    pub armed_at: Instant,
    /// 当前悬停的**合法**放置目标(非法目标不记录)。
    pub over: Option<DropTarget>,
}

/// `Pending → Dragging` 的确认条件:距离与按住时长两道阈值都越过(同文件树)。
pub fn should_confirm(drag: &CategoryDrag, cursor: (f32, f32), now: Instant) -> bool {
    drag.phase == CategoryDragPhase::Pending
        && crate::app::tree_drag_past_threshold(drag.press_pos, cursor)
        && crate::app::tree_drag_held_long_enough(now.saturating_duration_since(drag.armed_at))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseAction {
    /// 没确认就松手 = 点击:选中该分类。
    Select(i64),
    Reparent {
        id: i64,
        new_parent: Option<i64>,
    },
    Nothing,
}

/// 松手收尾:未确认 → 点击(源行仍存在才选中);已确认 → 目标合法才移动,否则空操作。
/// 源或目标在拖拽期间被删掉(别的会话 / agent)一律落到 `Nothing`。
pub fn release_action(drag: &CategoryDrag, categories: &[CategoryInfo]) -> ReleaseAction {
    if !categories.iter().any(|c| c.id == drag.source) {
        return ReleaseAction::Nothing;
    }
    if drag.phase == CategoryDragPhase::Pending {
        return ReleaseAction::Select(drag.source);
    }
    match drag.over {
        Some(target) if is_valid_drop(categories, drag.source, target) => ReleaseAction::Reparent {
            id: drag.source,
            new_parent: match target {
                DropTarget::Root => None,
                DropTarget::Node(id) => Some(id),
            },
        },
        _ => ReleaseAction::Nothing,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tick {
    Nothing,
    /// 左键没按下却还有残留拖拽态:自愈清空。
    Clear,
    /// 该推进到 `Dragging`。
    Confirm,
}

/// 每次 `CursorMoved` 的决策(纯函数,便于测试):没有物理按住左键就绝不确认,
/// 并清掉任何残留(正常路径下松手收尾早该清过;还留着只可能是那次收尾丢了)。
pub fn tick(
    drag: Option<&CategoryDrag>,
    left_mouse_down: bool,
    cursor: (f32, f32),
    now: Instant,
) -> Tick {
    let Some(d) = drag else {
        return Tick::Nothing;
    };
    if !left_mouse_down {
        return Tick::Clear;
    }
    if should_confirm(d, cursor, now) {
        Tick::Confirm
    } else {
        Tick::Nothing
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

    use std::time::{Duration, Instant};

    fn drag(source: i64, phase: CategoryDragPhase, over: Option<DropTarget>) -> CategoryDrag {
        CategoryDrag {
            source,
            phase,
            press_pos: (100.0, 100.0),
            armed_at: Instant::now(),
            over,
        }
    }

    // Review Focus 1
    #[test]
    fn should_confirm_needs_distance_and_hold_time() {
        let d = CategoryDrag {
            armed_at: Instant::now() - Duration::from_millis(500),
            ..drag(2, CategoryDragPhase::Pending, None)
        };
        assert!(should_confirm(&d, (100.0, 130.0), Instant::now()));
        // 距离不够(抖动)
        assert!(!should_confirm(&d, (103.0, 102.0), Instant::now()));
        // 按住时间不够
        let fresh = drag(2, CategoryDragPhase::Pending, None);
        assert!(!should_confirm(
            &fresh,
            (100.0, 130.0),
            fresh.armed_at + Duration::from_millis(50)
        ));
    }

    #[test]
    fn already_dragging_is_not_reconfirmed() {
        let d = CategoryDrag {
            armed_at: Instant::now() - Duration::from_millis(500),
            ..drag(2, CategoryDragPhase::Dragging, None)
        };
        assert!(!should_confirm(&d, (100.0, 300.0), Instant::now()));
    }

    // Review Focus 1:没确认就松手 = 点击,选中该分类。
    #[test]
    fn release_action_unconfirmed_is_a_click() {
        let d = drag(2, CategoryDragPhase::Pending, None);
        assert_eq!(release_action(&d, &tree()), ReleaseAction::Select(2));
        // 即使 over 里有值,Pending 也只是点击
        let d = drag(2, CategoryDragPhase::Pending, Some(DropTarget::Node(5)));
        assert_eq!(release_action(&d, &tree()), ReleaseAction::Select(2));
    }

    #[test]
    fn release_action_confirmed_with_valid_target_reparents() {
        let d = drag(4, CategoryDragPhase::Dragging, Some(DropTarget::Node(5)));
        assert_eq!(
            release_action(&d, &tree()),
            ReleaseAction::Reparent {
                id: 4,
                new_parent: Some(5)
            }
        );
        let d = drag(4, CategoryDragPhase::Dragging, Some(DropTarget::Root));
        assert_eq!(
            release_action(&d, &tree()),
            ReleaseAction::Reparent {
                id: 4,
                new_parent: None
            }
        );
    }

    #[test]
    fn release_action_confirmed_without_or_with_invalid_target_does_nothing() {
        let none = drag(2, CategoryDragPhase::Dragging, None);
        assert_eq!(release_action(&none, &tree()), ReleaseAction::Nothing);
        let onto_self = drag(2, CategoryDragPhase::Dragging, Some(DropTarget::Node(2)));
        assert_eq!(release_action(&onto_self, &tree()), ReleaseAction::Nothing);
        let onto_child = drag(1, CategoryDragPhase::Dragging, Some(DropTarget::Node(4)));
        assert_eq!(release_action(&onto_child, &tree()), ReleaseAction::Nothing);
    }

    // Review Focus 4:松手时目标已被删掉。
    #[test]
    fn release_action_with_vanished_target_is_none() {
        let d = drag(4, CategoryDragPhase::Dragging, Some(DropTarget::Node(77)));
        assert_eq!(release_action(&d, &tree()), ReleaseAction::Nothing);
    }

    #[test]
    fn release_action_with_vanished_source_is_nothing() {
        let d = drag(99, CategoryDragPhase::Dragging, Some(DropTarget::Root));
        assert_eq!(release_action(&d, &tree()), ReleaseAction::Nothing);
        // 点击一个已不存在的行不选中任何东西
        let click = drag(99, CategoryDragPhase::Pending, None);
        assert_eq!(release_action(&click, &tree()), ReleaseAction::Nothing);
    }

    // Review Focus 5:左键并未按下却残留了拖拽态 → 必须清空,不能确认。
    #[test]
    fn tick_clears_a_stale_drag_when_the_mouse_is_not_down() {
        let d = CategoryDrag {
            armed_at: Instant::now() - Duration::from_millis(500),
            ..drag(2, CategoryDragPhase::Pending, None)
        };
        assert_eq!(
            tick(Some(&d), false, (100.0, 300.0), Instant::now()),
            Tick::Clear
        );
    }

    #[test]
    fn tick_confirms_only_with_the_mouse_down_and_both_thresholds() {
        let d = CategoryDrag {
            armed_at: Instant::now() - Duration::from_millis(500),
            ..drag(2, CategoryDragPhase::Pending, None)
        };
        assert_eq!(
            tick(Some(&d), true, (100.0, 300.0), Instant::now()),
            Tick::Confirm
        );
        assert_eq!(
            tick(Some(&d), true, (101.0, 101.0), Instant::now()),
            Tick::Nothing
        );
        assert_eq!(tick(None, true, (0.0, 0.0), Instant::now()), Tick::Nothing);
        assert_eq!(tick(None, false, (0.0, 0.0), Instant::now()), Tick::Nothing);
    }
}
