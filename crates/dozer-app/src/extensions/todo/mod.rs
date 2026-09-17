//! Todo 面板(设计 2026-08-06;SQLite 迁移计划 2026-09-01;已并入
//! `extensions::todo`):任务列表的唯一权威源在 dozerd 侧 `dozer.db` 的
//! SQLite `todos` 表(TodoStore 归档)。本模块只持有每个 `Workspace` 上的
//! 渲染态(`WorkspaceState`)与视图/更新逻辑,List/Add/Toggle 经 `Client`
//! 走 UDS 与 dozerd 同步;不再直接读写 `.dozer/todo.md`/`todo_meta.json`
//! (MARKDOWN 整文件视图已随迁移移除)。

mod filter;
mod state;
mod update;
mod view;

pub(crate) use filter::*;
pub(crate) use state::*;
pub(crate) use update::*;
pub(crate) use view::*;

#[cfg(test)]
mod tests {
    use super::*;

    use dozer_core::protocol::{AgentKind, CategoryInfo, TodoInfo};

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

    fn sample_states() -> Vec<TodoInfo> {
        vec![
            todo_info(1, "修复登录页闪烁", false),
            todo_info(2, "补 README 安装说明", true),
            todo_info(3, "给 claude 指派生成报告", false),
        ]
    }

    #[test]
    fn filter_all_with_empty_query_keeps_everything() {
        let items = sample_states();
        let hits = filter_todos(&items, "");
        assert_eq!(hits, vec![0, 1, 2]);
    }

    #[test]
    fn filter_by_keyword_case_insensitive_substring() {
        let items = sample_states();
        assert_eq!(filter_todos(&items, "CLAUDE"), vec![2]);
    }

    #[test]
    fn filter_keyword_is_substring_match_on_text() {
        let items = sample_states();
        assert_eq!(filter_todos(&items, "登录"), vec![0]);
        assert_eq!(filter_todos(&items, "不存在的关键词"), Vec::<usize>::new());
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
    fn calendar_days_in_month_handles_leap_years() {
        assert_eq!(days_in_month(2024, 2), 29); // 闰
        assert_eq!(days_in_month(2023, 2), 28);
        assert_eq!(days_in_month(2024, 4), 30);
        assert_eq!(days_in_month(2024, 1), 31);
        assert_eq!(days_in_month(2024, 13), 0);
    }

    #[test]
    fn calendar_first_weekday_of_1970_jan_is_thursday() {
        assert_eq!(first_weekday_of_month(1970, 1), 4); // 周四
    }

    #[test]
    fn calendar_days_from_civil_round_trips() {
        use super::civil_from_days;
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(days_from_civil(2024, 2, 29)), (2024, 2, 29));
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

    fn editing_state(text: &str) -> WorkspaceState {
        WorkspaceState {
            items: vec![todo_info(7, "旧文字", false)],
            editing_content: Some((0, iced_widget::text_editor::Content::with_text(text))),
            ..Default::default()
        }
    }

    #[test]
    fn commit_content_edit_returns_id_and_text_when_changed() {
        let mut ws_state = editing_state("新文字");
        let result = ws_state.commit_content_edit();
        assert_eq!(result, Some((7, "新文字".to_string())));
        assert!(ws_state.editing_content.is_none());
    }

    #[test]
    fn commit_content_edit_none_when_unchanged() {
        let mut ws_state = WorkspaceState {
            items: vec![todo_info(7, "同样的文字", false)],
            editing_content: Some((
                0,
                iced_widget::text_editor::Content::with_text("同样的文字"),
            )),
            ..Default::default()
        };
        assert_eq!(ws_state.commit_content_edit(), None);
    }

    #[test]
    fn commit_content_edit_none_when_draft_empty() {
        let mut ws_state = WorkspaceState {
            items: vec![todo_info(7, "旧文字", false)],
            editing_content: Some((0, iced_widget::text_editor::Content::new())),
            ..Default::default()
        };
        assert_eq!(ws_state.commit_content_edit(), None);
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

    #[cfg(target_os = "macos")]
    #[test]
    fn dispatch_items_covers_the_four_headless_agents() {
        let items = dispatch_items(2);
        assert_eq!(items.len(), 4);
        let agents: Vec<AgentKind> = items
            .iter()
            .map(|i| match i {
                crate::chrome::native_menu::Item::Entry {
                    msg: Message::AssignAgent(idx, agent),
                    ..
                } => {
                    assert_eq!(*idx, 2);
                    *agent
                }
                _ => panic!("expected AssignAgent entry"),
            })
            .collect();
        assert_eq!(
            agents,
            vec![
                AgentKind::Claude,
                AgentKind::Codebuddy,
                AgentKind::Opencode,
                AgentKind::V8agent,
            ]
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn status_items_covers_all_four_states_for_given_index() {
        let items = status_items(9);
        assert_eq!(items.len(), 4);
        let states: Vec<TodoState> = items
            .iter()
            .map(|i| match i {
                crate::chrome::native_menu::Item::Entry {
                    msg: Message::StatusPick(idx, st),
                    ..
                } => {
                    assert_eq!(*idx, 9);
                    *st
                }
                _ => panic!("expected StatusPick entry"),
            })
            .collect();
        assert_eq!(
            states,
            vec![
                TodoState::Pending,
                TodoState::InProgress,
                TodoState::Suspended,
                TodoState::Done,
            ]
        );
    }
}
