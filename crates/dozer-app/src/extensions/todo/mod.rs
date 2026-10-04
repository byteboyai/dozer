//! Todo 面板(设计 2026-08-06;SQLite 迁移计划 2026-09-01;已并入
//! `extensions::todo`):任务列表的唯一权威源在 dozerd 侧 `dozer.db` 的
//! SQLite `todos` 表(TodoStore 归档)。本模块只持有每个 `Workspace` 上的
//! 渲染态(`WorkspaceState`)与视图/更新逻辑,List/Add/Toggle 经 `Client`
//! 走 UDS 与 dozerd 同步;不再直接读写 `.dozer/todo.md`/`todo_meta.json`
//! (MARKDOWN 整文件视图已随迁移移除)。

mod category_drag;
mod filter;
mod protocol;
mod state;
mod update;
mod view;

pub(crate) use category_drag::*;
pub(crate) use filter::*;
pub(crate) use protocol::*;
pub(crate) use state::*;
pub(crate) use update::*;
pub(crate) use view::*;

#[cfg(test)]
mod tests {
    use super::*;

    use dozer_core::protocol::{CategoryInfo, TodoInfo};

    fn todo_info(id: i64, text: &str, done: bool) -> TodoInfo {
        TodoInfo {
            id,
            project_id: 1,
            text: text.to_string(),
            done,
            paused: false,
            rank: 0,
            created_ms: 0,
            completed_at_ms: None,
            plan_date: None,
            dispatch_session_id: None,
            dispatch_at_ms: None,
            category_id: None,
            assigned_agent: None,
        }
    }

    #[test]
    fn display_state_done_wins_over_dispatch() {
        let mut item = todo_info(1, "任务A", true);
        item.dispatch_session_id = Some("s1".into());
        assert_eq!(todo_display_state(&item, true), TodoState::Done);
        item.done = false;
        assert_eq!(todo_display_state(&item, true), TodoState::InProgress);
        assert_eq!(todo_display_state(&item, false), TodoState::Pending);
        item.dispatch_session_id = None;
        assert_eq!(todo_display_state(&item, true), TodoState::Pending);
    }

    #[test]
    fn completed_at_for_toggle_reflects_done() {
        let now = std::time::SystemTime::now();
        assert_eq!(completed_at_for_toggle(true, now), Some(now));
        assert_eq!(completed_at_for_toggle(false, now), None);
    }

    #[test]
    fn calendar_parse_month_day_accepts_mm_dd_and_rejects_bad_input() {
        assert_eq!(parse_month_day("08-10"), Some((8, 10)));
        assert_eq!(parse_month_day("8-3"), Some((8, 3)));
        assert_eq!(parse_month_day("13-01"), None);
        assert_eq!(parse_month_day("08"), None);
        assert_eq!(parse_month_day("abc"), None);
    }

    #[test]
    fn format_month_day_from_ms() {
        // 1970-01-01 的毫秒。
        assert_eq!(format_todo_month_day(0), "01-01");
    }

    #[test]
    fn task_title_for_session_finds_dispatched_task() {
        let mut ws_state = WorkspaceState {
            items: vec![todo_info(1, "任务A", false), todo_info(2, "任务B", false)],
            ..Default::default()
        };
        ws_state.items[1].dispatch_session_id = Some("sess-1".to_string());
        assert_eq!(ws_state.task_title_for_session("sess-1"), Some("任务B"));
    }

    #[test]
    fn task_title_for_session_none_when_no_dispatch_matches() {
        let ws_state = WorkspaceState {
            items: vec![todo_info(1, "任务A", false)],
            ..Default::default()
        };
        assert_eq!(ws_state.task_title_for_session("sess-1"), None);
    }

    fn cat(id: i64, parent_id: Option<i64>, name: &str) -> CategoryInfo {
        CategoryInfo {
            id,
            project_id: 1,
            parent_id,
            name: name.into(),
            rank: 0,
            created_ms: 0,
            auto_poll_enabled: false,
        }
    }

    #[test]
    fn visible_category_rows_flattens_by_expanded_state() {
        // 前端(展开) -> UI, 性能(未展开无子节点)
        //   UI(未展开) -> 组件库(不可见,父未展开)
        // 后端(未展开) -> 数据库(不可见)
        let categories = vec![
            cat(1, None, "前端"),
            cat(2, Some(1), "UI"),
            cat(3, Some(1), "性能"),
            cat(4, Some(2), "组件库"),
            cat(5, None, "后端"),
            cat(6, Some(5), "数据库"),
        ];
        let mut expanded = std::collections::HashSet::new();
        expanded.insert(1);
        let rows = visible_category_rows(&categories, &expanded);
        let visible_ids: Vec<i64> = rows.iter().map(|r| r.id).collect();
        assert_eq!(
            visible_ids,
            vec![1, 2, 3, 5],
            "只展开了前端,UI/后端的子节点都不可见"
        );
        let front = rows.iter().find(|r| r.id == 1).unwrap();
        assert_eq!(front.depth, 0);
        assert!(front.has_children);
        assert!(front.expanded);
        let ui = rows.iter().find(|r| r.id == 2).unwrap();
        assert_eq!(ui.depth, 1);
        assert!(
            ui.has_children,
            "UI 有子节点组件库,即使未展开也要标记有子节点"
        );
        assert!(!ui.expanded);
        let perf = rows.iter().find(|r| r.id == 3).unwrap();
        assert!(!perf.has_children);
    }

    #[test]
    fn category_descendants_covers_multi_level_and_excludes_root() {
        let categories = vec![
            cat(1, None, "前端"),
            cat(2, Some(1), "UI"),
            cat(3, Some(2), "组件库"),
            cat(4, None, "后端"),
        ];
        let descendants = category_descendants(&categories, 1);
        assert_eq!(
            descendants,
            std::collections::HashSet::from([2, 3]),
            "根节点自己不算子孙,后端这条不相关分支不应该出现"
        );
        assert!(
            category_descendants(&categories, 3).is_empty(),
            "叶子节点没有子孙"
        );
    }

    #[test]
    fn filter_todos_by_category_all_returns_everything() {
        let todos = vec![todo_with_category(1, Some(10)), todo_with_category(2, None)];
        let categories = vec![cat(10, None, "分类")];
        let ids = filter_todos_by_category(&todos, &categories, CategoryFilter::All);
        assert_eq!(ids, std::collections::HashSet::from([1, 2]));
    }

    #[test]
    fn filter_todos_by_category_uncategorized_only() {
        let todos = vec![todo_with_category(1, Some(10)), todo_with_category(2, None)];
        let ids = filter_todos_by_category(&todos, &[], CategoryFilter::Uncategorized);
        assert_eq!(ids, std::collections::HashSet::from([2]));
    }

    #[test]
    fn filter_todos_by_category_node_includes_descendant_tasks() {
        // 前端(id 10) -> UI(id 11);任务 1 挂前端,任务 2 挂 UI,任务 3 未分类。
        // 选中"前端"应该看到任务 1 和任务 2(子孙汇总)。
        let todos = vec![
            todo_with_category(1, Some(10)),
            todo_with_category(2, Some(11)),
            todo_with_category(3, None),
        ];
        let categories = vec![cat(10, None, "前端"), cat(11, Some(10), "UI")];
        let ids = filter_todos_by_category(&todos, &categories, CategoryFilter::Node(10));
        assert_eq!(ids, std::collections::HashSet::from([1, 2]));
        // 选中叶子节点"UI"只看到任务 2。
        let ids = filter_todos_by_category(&todos, &categories, CategoryFilter::Node(11));
        assert_eq!(ids, std::collections::HashSet::from([2]));
    }

    fn todo_with_category(id: i64, category_id: Option<i64>) -> TodoInfo {
        TodoInfo {
            id,
            project_id: 1,
            text: format!("任务{id}"),
            done: false,
            paused: false,
            rank: 0,
            created_ms: 0,
            completed_at_ms: None,
            plan_date: None,
            dispatch_session_id: None,
            dispatch_at_ms: None,
            category_id,
            assigned_agent: None,
        }
    }

    #[test]
    fn start_flash_bumps_scroll_nonce_and_sets_selected_id() {
        let mut ws_state = WorkspaceState {
            items: vec![todo_info(5, "新", false), todo_info(6, "旧", false)],
            ..WorkspaceState::default()
        };
        assert_eq!(ws_state.scroll_nonce, 0);
        assert_eq!(ws_state.selected_id(), None);
        ws_state.start_flash(0);
        assert_eq!(ws_state.scroll_nonce, 1);
        assert_eq!(ws_state.selected_id(), Some(5));
        ws_state.start_flash(1);
        assert_eq!(ws_state.scroll_nonce, 2);
        assert_eq!(ws_state.selected_id(), Some(6));
    }

    // 审阅 Important 4:一次往返内连续新增两条,临时项的 id 不能撞(前端按 id 做 key 与路由)。
    #[test]
    fn optimistic_ids_are_unique_and_never_look_like_real_ids() {
        let mut ws_state = WorkspaceState::default();
        let a = ws_state.alloc_optimistic_id();
        let b = ws_state.alloc_optimistic_id();
        assert!(a != b);
        assert!(is_optimistic_id(a) && is_optimistic_id(b));
        assert!(!is_optimistic_id(1), "dozerd 的真实 id 从 1 起");
    }

    #[test]
    fn two_quick_optimistic_adds_get_distinct_ids_and_newest_is_on_top() {
        let mut ws_state = WorkspaceState {
            items: vec![todo_info(10, "已有", false)],
            ..WorkspaceState::default()
        };
        let first = insert_optimistic_todo(&mut ws_state, 1, "第一条".into());
        let second = insert_optimistic_todo(&mut ws_state, 1, "第二条".into());
        assert!(first != second);
        let ids: Vec<i64> = ws_state.items.iter().map(|i| i.id).collect();
        assert_eq!(ids, vec![second, first, 10]);
        assert_eq!(ws_state.selected_id(), Some(second), "闪光落在最新一条上");
    }

    #[test]
    fn set_add_input_height_clamps_to_bounds() {
        let mut ws_state = WorkspaceState::default();
        ws_state.set_add_input_height(10_000.0);
        assert!(ws_state.add_input_height() <= ADD_INPUT_MAX_HEIGHT);
        ws_state.set_add_input_height(1.0);
        assert!(ws_state.add_input_height() >= ADD_INPUT_MIN_HEIGHT);
    }

    #[test]
    fn category_drag_lifecycle_arm_confirm_over_take() {
        let mut ws_state = WorkspaceState::default();
        assert!(ws_state.category_drag().is_none());
        ws_state.arm_category_drag(7, (10.0, 10.0), std::time::Instant::now());
        assert!(!ws_state.category_drag_confirmed());
        // Pending 期间设置悬停目标无效(不允许有任何反应)
        ws_state.set_category_drag_over(Some(DropTarget::Root));
        assert_eq!(ws_state.category_drag().unwrap().over, None);
        ws_state.confirm_category_drag();
        assert!(ws_state.category_drag_confirmed());
        ws_state.set_category_drag_over(Some(DropTarget::Node(3)));
        assert_eq!(
            ws_state.category_drag().unwrap().over,
            None,
            "不存在的目标 id 应被拒绝"
        );
        let taken = ws_state.take_category_drag().unwrap();
        assert_eq!(taken.source, 7);
        assert!(ws_state.category_drag().is_none());
    }

    // 审阅 Important 8:拖进一个折叠的分类后,新父节点要展开,否则被移动的节点(可能正是
    // 选中项)会从树里消失。
    #[test]
    fn reparent_expands_the_new_parent_so_the_node_stays_visible() {
        let mut ws_state = WorkspaceState::default();
        assert!(!ws_state.category_expanded().contains(&5));
        ws_state.expand_for_reparent(Some(5));
        assert!(ws_state.category_expanded().contains(&5));
        // 已展开的不会被收起(不是 toggle)
        ws_state.expand_for_reparent(Some(5));
        assert!(ws_state.category_expanded().contains(&5));
    }

    #[test]
    fn reparent_to_top_level_changes_no_expansion() {
        let mut ws_state = WorkspaceState::default();
        ws_state.expand_for_reparent(None);
        assert!(ws_state.category_expanded().is_empty());
    }

    #[test]
    fn cancel_category_drag_clears_any_phase() {
        let mut ws_state = WorkspaceState::default();
        ws_state.arm_category_drag(7, (0.0, 0.0), std::time::Instant::now());
        ws_state.cancel_category_drag();
        assert!(ws_state.category_drag().is_none());
        ws_state.cancel_category_drag(); // 幂等
    }

    // ---- H6:切入钩子 ----

    use crate::panel_host::testing::{TIMEOUT, offline_ctx, runtime};

    #[test]
    fn on_activate_requests_both_the_todo_list_and_the_category_tree() {
        let rt = runtime();
        let (ctx, rx) = offline_ctx::<Message>(&rt, 7, None);
        on_activate(&ctx);
        let first = rx.recv_timeout(TIMEOUT).unwrap();
        let second = rx.recv_timeout(TIMEOUT).unwrap();
        let loaded = |m: &Message| matches!(m, Message::Loaded(_));
        let categories = |m: &Message| matches!(m, Message::CategoriesLoaded(_));
        assert!(
            (loaded(&first) && categories(&second)) || (categories(&first) && loaded(&second)),
            "两个请求各回一条消息"
        );
    }
}
