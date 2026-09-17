//! Files(项目文件树)面板:文件树 + 右键菜单/删除确认浮层。阶段 1 扩展化
//! 重构第四个试点,设计见
//! `docs/superpowers/specs/2026-08-07-files-extension-pilot-design.md`。
use crate::delivery::FileGitStatus;

mod state;
mod tree;
mod update;
mod view;

pub(crate) use state::*;
pub(crate) use tree::*;
pub(crate) use update::*;
pub(crate) use view::*;

#[cfg(test)]
mod tests {
    use super::*;

    use crate::delivery;
    use crate::project::{FileTree, PathKind, TreeRow};
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    fn ws_with_tree(root: PathBuf) -> WorkspaceState {
        WorkspaceState::new(FileTree::new(root))
    }

    fn row(path: &str, is_dir: bool) -> TreeRow {
        TreeRow {
            path: PathBuf::from(path),
            name: String::new(),
            depth: 0,
            is_dir,
            expanded: false,
        }
    }

    /// 回归测试:main.rs 键盘路由靠 `files_search_focused()` 决定按键是否
    /// 放行给终端/agent PTY——一旦这个标志卡在 `true` 就永久堵死终端输入
    /// (用户实测反馈,2026-08-31:切到文件树面板、用过输入框之后,右侧
    /// agent 输入框再也打不进字)。根因是搜索框不可见的帧(收起 Files 面板
    /// 但 `left_view` 未变、切到别的面板等)`CaptureSearchFocus` 找不到匹配
    /// id、不会覆盖 `SEARCH_FOCUSED`,旧版 `take_search_focused` 又是非消费
    /// 读法,上一次的 `true` 就会一直卡着。改成消费式复位后,读一次没找到
    /// 真身的 `true` 就应该只生效一帧,不能无限期卡住。
    #[test]
    fn take_search_focused_consumes_stale_true_after_one_read() {
        *SEARCH_FOCUSED.lock().unwrap() = true;
        assert!(take_search_focused());
        // `CaptureSearchFocus` 这一帧没找到搜索框(未运行/未命中),复位后
        // 第二次读必须是 false,不能沿用上一帧的 true。
        assert!(!take_search_focused());
    }

    /// 同上,项目树行内编辑框(`TREE_EDIT_FOCUSED`)同款消费式复位回归测试。
    #[test]
    fn take_tree_edit_focused_consumes_stale_true_after_one_read() {
        *TREE_EDIT_FOCUSED.lock().unwrap() = true;
        assert!(take_tree_edit_focused());
        assert!(!take_tree_edit_focused());
    }

    #[test]
    fn tree_drop_target_dir_hit_highlights_and_targets_itself() {
        let rows = vec![
            row("/a", true),
            row("/a/f.rs", false),
            row("/b", true),
            row("/c", true),
        ];
        let bounds = (100.0, 100.0, 400.0, 400.0);
        let scroll = 0.0;
        let row_h = crate::theme::geometry::tree_row_h();
        let gap = crate::theme::region::project_pane().gap;
        let pitch = row_h + gap;
        let by = bounds.1;
        // 第 0 行是目录 → 命中,高亮与落点都是它自己。
        assert_eq!(
            tree_drop_target(200.0, by + row_h / 2.0, bounds, scroll, &rows),
            Some(DropHit {
                highlight: PathBuf::from("/a"),
                target: PathBuf::from("/a"),
            })
        );
        // 第 2、3 行都是目录 → 各自命中。
        assert_eq!(
            tree_drop_target(200.0, by + 2.0 * pitch + row_h / 2.0, bounds, scroll, &rows),
            Some(DropHit {
                highlight: PathBuf::from("/b"),
                target: PathBuf::from("/b"),
            })
        );
        assert_eq!(
            tree_drop_target(200.0, by + 3.0 * pitch + row_h / 2.0, bounds, scroll, &rows),
            Some(DropHit {
                highlight: PathBuf::from("/c"),
                target: PathBuf::from("/c"),
            })
        );
        // 视图上方 / 右侧外 → None。
        assert_eq!(
            tree_drop_target(50.0, by + row_h / 2.0, bounds, scroll, &rows),
            None
        );
        assert_eq!(
            tree_drop_target(200.0, by - 5.0, bounds, scroll, &rows),
            None
        );
        // 行间死区吸到下一行目录(idx1 是文件 /a/f.rs,2*pitch 的空档应在 /b
        // 之上;吸住 /b)。
        assert_eq!(
            tree_drop_target(200.0, by + 2.0 * pitch - 1.0, bounds, scroll, &rows),
            Some(DropHit {
                highlight: PathBuf::from("/b"),
                target: PathBuf::from("/b"),
            })
        );
    }

    /// 2026-09 用户实测反馈"悬浮到文件上也该有高亮":命中文件行不再是
    /// `None`——`highlight` 是文件自己(渲染高亮它),`target`(真正的落点
    /// 目录)退到其父目录,同 Finder"拖到某个文件上=拖进它所在文件夹"。
    #[test]
    fn tree_drop_target_file_hit_highlights_file_but_targets_parent() {
        let rows = vec![row("/a", true), row("/a/f.rs", false), row("/b", true)];
        let bounds = (100.0, 100.0, 400.0, 400.0);
        let scroll = 0.0;
        let row_h = crate::theme::geometry::tree_row_h();
        let gap = crate::theme::region::project_pane().gap;
        let pitch = row_h + gap;
        let by = bounds.1;
        assert_eq!(
            tree_drop_target(200.0, by + pitch + row_h / 2.0, bounds, scroll, &rows),
            Some(DropHit {
                highlight: PathBuf::from("/a/f.rs"),
                target: PathBuf::from("/a"),
            })
        );
    }

    #[test]
    fn is_valid_move_target_rejects_moving_dir_into_itself() {
        assert!(!is_valid_move_target(
            &PathBuf::from("/proj/a"),
            true,
            &PathBuf::from("/proj/a"),
        ));
    }

    #[test]
    fn is_valid_move_target_rejects_moving_dir_into_own_descendant() {
        assert!(!is_valid_move_target(
            &PathBuf::from("/proj/a"),
            true,
            &PathBuf::from("/proj/a/b"),
        ));
    }

    /// 2026-09 用户实测反馈"无法拖到父目录"后取消了这条拒绝——真无意义时
    /// `move_item` 自己会安全兜底,不需要在悬停高亮这一层提前挡。
    #[test]
    fn is_valid_move_target_allows_dropping_onto_current_parent_as_true_noop() {
        assert!(is_valid_move_target(
            &PathBuf::from("/proj/a/f.rs"),
            false,
            &PathBuf::from("/proj/a"),
        ));
    }

    #[test]
    fn is_valid_move_target_allows_moving_file_to_unrelated_dir() {
        assert!(is_valid_move_target(
            &PathBuf::from("/proj/a/f.rs"),
            false,
            &PathBuf::from("/proj/c"),
        ));
    }

    #[test]
    fn is_valid_move_target_allows_moving_dir_to_unrelated_dir() {
        assert!(is_valid_move_target(
            &PathBuf::from("/proj/a"),
            true,
            &PathBuf::from("/proj/z"),
        ));
    }

    /// 目录名前缀相同但不是真子路径(`/proj/ab` 不是 `/proj/a` 的子目录)不能
    /// 被 `starts_with` 字符串前缀误判——必须走路径分量比较。
    #[test]
    fn is_valid_move_target_does_not_confuse_sibling_with_shared_prefix() {
        assert!(is_valid_move_target(
            &PathBuf::from("/proj/a"),
            true,
            &PathBuf::from("/proj/ab"),
        ));
    }

    #[test]
    fn tree_drop_target_respects_scroll() {
        let rows = vec![row("/a", true), row("/b", true), row("/c", true)];
        let bounds = (0.0, 0.0, 500.0, 500.0);
        let row_h = crate::theme::geometry::tree_row_h();
        let gap = crate::theme::region::project_pane().gap;
        let pitch = row_h + gap;
        // 滚过一整行：视觉第 0 行其实是内容第 1 行 → 命中 /b。
        let scroll = pitch;
        assert_eq!(
            tree_drop_target(100.0, row_h / 2.0, bounds, scroll, &rows),
            Some(DropHit {
                highlight: PathBuf::from("/b"),
                target: PathBuf::from("/b"),
            })
        );
    }

    #[test]
    fn tree_state_colors() {
        assert_eq!(
            tree_state_color(delivery::TreeState::Untracked),
            byteui::theme::color::current().red
        );
        assert_eq!(
            tree_state_color(delivery::TreeState::StagedNew),
            byteui::theme::color::current().green
        );
        assert_eq!(
            tree_state_color(delivery::TreeState::Modified),
            byteui::theme::color::current().cyan
        );
        assert_eq!(
            tree_state_color(delivery::TreeState::Unchanged),
            byteui::theme::color::current().body
        );
        assert_eq!(
            tree_state_color(delivery::TreeState::Ignored),
            byteui::theme::color::current().ignored
        );
    }

    #[tokio::test]
    async fn search_input_then_submit_commits_draft_and_reset_clears() {
        let dir = tempfile::tempdir().unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        let mut app_state = AppState::default();
        let handle = tokio::runtime::Handle::current();

        // text_input::on_input 每次给全量当前字符串,不是逐字符追加。
        update(
            &mut ws_state,
            &mut app_state,
            Message::SearchInput("main".to_string()),
            1,
            &handle,
            |_| {},
        );
        assert_eq!(ws_state.tree_search, "main");
        assert!(ws_state.search_query.is_empty());

        // 提交(点右侧"搜索"按钮,或 iced text_input::on_submit 触发的
        // Enter):草稿落成为生效过滤词。
        update(
            &mut ws_state,
            &mut app_state,
            Message::SearchSubmit,
            1,
            &handle,
            |_| {},
        );
        assert_eq!(ws_state.search_query, "main");

        // 认领其它项目时清空草稿、生效词与焦点镜像,避免旧筛选残留在新
        // 项目树上。
        ws_state.reset_for_project(FileTree::new(dir.path().to_path_buf()));
        assert!(ws_state.tree_search.is_empty() && ws_state.search_query.is_empty());
        assert!(!ws_state.search_focused());
    }

    #[test]
    fn search_field_id_is_stable_across_calls() {
        // `CaptureSearchFocus` 靠 `search_field_id()` 在两处(view() 的
        // `.id()` 与每帧焦点查询)各自构造出的 `Id` 相等来认出同一个字段,
        // 这个前提必须成立。
        assert_eq!(search_field_id(), search_field_id());
    }

    #[test]
    fn set_search_focused_updates_accessor() {
        let mut ws_state = WorkspaceState::default();
        assert!(!ws_state.search_focused());
        ws_state.set_search_focused(true);
        assert!(ws_state.search_focused());
        ws_state.set_search_focused(false);
        assert!(!ws_state.search_focused());
    }

    /// 覆盖 `Message::Toggle` 的两个调用点共用的核心行为(选中 + 立即
    /// 切换展开态,不经拖拽/双击的延迟判定):目录双击、外部/内部拖拽悬停
    /// 自动展开都发的是这同一条消息(箭头点击改发 `ToggleNoSelect`,见
    /// 下一个测试)。
    #[tokio::test]
    async fn toggle_sets_selected_and_toggles_tree() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("child.txt"), b"hi").unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        let mut app_state = AppState::default();
        let handle = tokio::runtime::Handle::current();
        update(
            &mut ws_state,
            &mut app_state,
            Message::Toggle(sub.clone()),
            1,
            &handle,
            |_| {},
        );
        assert_eq!(ws_state.tree_selected, Some(sub.clone()));
        assert!(
            ws_state
                .visible_tree_rows()
                .iter()
                .any(|r| r.path == sub && r.expanded)
        );
    }

    /// 箭头点击(`ToggleNoSelect`)只切换展开态,不触碰 `tree_selected`——
    /// 2026-09 用户反馈:点箭头之前会连带把目录选中,只是想看子项却意外
    /// 改了选中态。这里先选中一个**不同**的路径,断言 `Toggle` 之后选中
    /// 原样未变。
    #[tokio::test]
    async fn toggle_no_select_toggles_tree_without_changing_selection() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("child.txt"), b"hi").unwrap();
        let other = dir.path().join("other.txt");
        std::fs::write(&other, b"x").unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        ws_state.tree_selected = Some(other.clone());
        let mut app_state = AppState::default();
        let handle = tokio::runtime::Handle::current();
        update(
            &mut ws_state,
            &mut app_state,
            Message::ToggleNoSelect(sub.clone()),
            1,
            &handle,
            |_| {},
        );
        assert_eq!(ws_state.tree_selected, Some(other));
        assert!(
            ws_state
                .visible_tree_rows()
                .iter()
                .any(|r| r.path == sub && r.expanded)
        );
    }

    #[test]
    fn arm_tree_drag_records_source_and_starts_with_no_target() {
        let mut ws_state = WorkspaceState::default();
        assert!(!ws_state.is_dragging_tree_item());
        ws_state.arm_tree_drag(
            PathBuf::from("/proj/a"),
            true,
            (10.0, 20.0),
            std::time::Instant::now(),
        );
        assert!(ws_state.is_dragging_tree_item());
    }

    /// `App::maybe_confirm_tree_drag` 的自愈路径用它清掉左键并未物理按住
    /// 时残留的 `tree_drag`(见其文档)——2026-09 用户实测反馈并截图:点击
    /// 展开箭头、松开左键后仅轻微移动鼠标(未按键)就冒出了跟随光标的幽灵
    /// 胶囊,根因是某次收尾没能触发、`Pending` 一直残留,之后纯悬停也能
    /// 凑够距离+时长阈值被误判成确认拖拽。
    #[test]
    fn cancel_tree_drag_clears_state_and_highlight() {
        let mut ws_state = WorkspaceState::default();
        ws_state.arm_tree_drag(
            PathBuf::from("/proj/a"),
            true,
            (10.0, 20.0),
            std::time::Instant::now(),
        );
        ws_state.drag_hover = std::iter::once(PathBuf::from("/proj/b")).collect();

        ws_state.cancel_tree_drag();

        assert!(!ws_state.is_dragging_tree_item());
        assert!(ws_state.drag_hover.is_empty());
    }

    #[test]
    fn tree_drag_press_pos_returns_press_point_recorded_at_arm_time() {
        let mut ws_state = WorkspaceState::default();
        assert_eq!(ws_state.tree_drag_press_pos(), None);
        ws_state.arm_tree_drag(
            PathBuf::from("/proj/a"),
            true,
            (10.0, 20.0),
            std::time::Instant::now(),
        );
        assert_eq!(ws_state.tree_drag_press_pos(), Some((10.0, 20.0)));
    }

    /// 单击(未越过拖拽确认阈值)只选中——不再自动展开/打开,见
    /// `Message::TreeDragEnd` 文档;文件/目录一视同仁,这里用文件源验证
    /// （目录源的展开分别由 `Message::ToggleNoSelect`(箭头)/`Toggle`(双击)
    /// 覆盖,见其他测试)。
    #[tokio::test]
    async fn tree_drag_end_unconfirmed_selects_file_without_opening() {
        let mut ws_state = WorkspaceState::default();
        ws_state.arm_tree_drag(
            PathBuf::from("/proj/f.rs"),
            false,
            (0.0, 0.0),
            std::time::Instant::now(),
        );
        let handle = tokio::runtime::Handle::current();
        update(
            &mut ws_state,
            &mut AppState::default(),
            Message::TreeDragEnd(false),
            1,
            &handle,
            |_| panic!("单击不该 emit 任何消息"),
        );
        assert_eq!(ws_state.tree_selected, Some(PathBuf::from("/proj/f.rs")));
    }

    /// 悬停命中一个折叠目录不立即展开——只武装计时(见 `drag_expand_
    /// pending` 文档),真正展开推迟到满 `DRAG_HOVER_EXPAND_DELAY` 由
    /// `App::advance_drag_hover_expand` 触发(2026-09 用户实测反馈:立即
    /// 展开会让拖着划过沿途目录疯狂跳动布局)。
    #[tokio::test]
    async fn tree_drag_over_arms_expand_timer_but_does_not_expand_immediately() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("child.txt"), b"hi").unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        let mut app_state = AppState::default();
        let handle = tokio::runtime::Handle::current();
        ws_state.arm_tree_drag(
            dir.path().join("other.txt"),
            false,
            (0.0, 0.0),
            std::time::Instant::now(),
        );
        ws_state.confirm_tree_drag();

        update(
            &mut ws_state,
            &mut app_state,
            Message::TreeDragOver(sub.clone()),
            1,
            &handle,
            |_| {},
        );

        assert!(
            !ws_state
                .visible_tree_rows()
                .iter()
                .any(|r| r.path == sub && r.expanded)
        );
        assert!(ws_state.drag_expand_ready().is_none());
    }

    /// `arm_drag_expand`/`drag_expand_ready` 隔离单测:未满计时返回
    /// `None`,直接回拨记录的起始时间模拟"已经悬停超过 1s"(不用真的
    /// `sleep`,保持测试快且确定性)。
    #[test]
    fn arm_drag_expand_becomes_ready_after_delay_elapses() {
        let mut ws_state = WorkspaceState::default();
        let dir = PathBuf::from("/proj/sub");
        ws_state.arm_drag_expand(dir.clone(), false);
        assert!(ws_state.drag_expand_ready().is_none());
        ws_state.drag_expand_pending = Some((
            dir.clone(),
            std::time::Instant::now() - DRAG_HOVER_EXPAND_DELAY,
        ));
        assert_eq!(ws_state.drag_expand_ready(), Some(dir));
    }

    /// 已展开(`already_expanded=true`)直接清空计时,不留残留——避免下一次
    /// 悬停别的折叠目录时被上一目录的旧计时提前触发。
    #[test]
    fn arm_drag_expand_already_expanded_clears_pending() {
        let mut ws_state = WorkspaceState::default();
        let dir = PathBuf::from("/proj/sub");
        ws_state.arm_drag_expand(dir.clone(), false);
        assert!(ws_state.next_drag_expand_wake().is_some());
        ws_state.arm_drag_expand(dir, true);
        assert!(ws_state.next_drag_expand_wake().is_none());
    }

    /// 同一目录持续悬停不重置计时——否则光标在同一行内轻微抖动,每帧都
    /// 重新起计时,永远等不满 1s。
    #[test]
    fn arm_drag_expand_on_same_dir_does_not_reset_timer() {
        let mut ws_state = WorkspaceState::default();
        let dir = PathBuf::from("/proj/sub");
        ws_state.arm_drag_expand(dir.clone(), false);
        let first_start = ws_state.drag_expand_pending.as_ref().unwrap().1;
        ws_state.arm_drag_expand(dir.clone(), false);
        assert_eq!(
            ws_state.drag_expand_pending.as_ref().unwrap().1,
            first_start
        );
    }

    #[tokio::test]
    async fn tree_drag_over_does_not_toggle_an_already_expanded_target() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("child.txt"), b"hi").unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        ws_state.file_tree.as_mut().unwrap().toggle(&sub);
        assert!(
            ws_state
                .visible_tree_rows()
                .iter()
                .any(|r| r.path == sub && r.expanded)
        );
        let mut app_state = AppState::default();
        let handle = tokio::runtime::Handle::current();
        ws_state.arm_tree_drag(
            dir.path().join("other.txt"),
            false,
            (0.0, 0.0),
            std::time::Instant::now(),
        );

        update(
            &mut ws_state,
            &mut app_state,
            Message::TreeDragOver(sub.clone()),
            1,
            &handle,
            |_| {},
        );

        // 已展开的目录不该被再 toggle 一次(那会变成收起)。
        assert!(
            ws_state
                .visible_tree_rows()
                .iter()
                .any(|r| r.path == sub && r.expanded)
        );
    }

    /// 悬停命中的是**目录行**:落点就是它自己(与 `tree_drag_over_file_
    /// hit_targets_its_parent_dir` 对照——命中文件行落点会退到父目录)。
    #[tokio::test]
    async fn tree_drag_over_valid_target_sets_target_and_highlight() {
        let dir = tempfile::tempdir().unwrap();
        let target_dir = dir.path().join("c");
        std::fs::create_dir(&target_dir).unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        let mut app_state = AppState::default();
        let handle = tokio::runtime::Handle::current();
        ws_state.arm_tree_drag(
            dir.path().join("a/f.rs"),
            false,
            (0.0, 0.0),
            std::time::Instant::now(),
        );

        ws_state.confirm_tree_drag();
        update(
            &mut ws_state,
            &mut app_state,
            Message::TreeDragOver(target_dir.clone()),
            1,
            &handle,
            |_| {},
        );

        assert_eq!(
            ws_state.tree_drag.as_ref().map(|d| &d.phase),
            Some(&TreeDragPhase::Dragging {
                target: Some(target_dir.clone())
            })
        );
        assert!(ws_state.drag_hover.contains(&target_dir));
    }

    /// 悬停命中的是**文件行**:落点退到其父目录,但高亮(`drag_hover`)记的
    /// 还是文件自己那一行——2026-09 用户实测反馈"悬浮到文件上也该有高亮",
    /// 见 `Message::TreeDragOver` 文档。
    #[tokio::test]
    async fn tree_drag_over_file_hit_targets_its_parent_dir() {
        let dir = tempfile::tempdir().unwrap();
        let target_file = dir.path().join("readme.txt");
        std::fs::write(&target_file, b"hi").unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        let mut app_state = AppState::default();
        let handle = tokio::runtime::Handle::current();
        ws_state.arm_tree_drag(
            dir.path().join("a/f.rs"),
            false,
            (0.0, 0.0),
            std::time::Instant::now(),
        );

        ws_state.confirm_tree_drag();
        update(
            &mut ws_state,
            &mut app_state,
            Message::TreeDragOver(target_file.clone()),
            1,
            &handle,
            |_| {},
        );

        assert_eq!(
            ws_state.tree_drag.as_ref().map(|d| &d.phase),
            Some(&TreeDragPhase::Dragging {
                target: Some(dir.path().to_path_buf())
            })
        );
        assert!(ws_state.drag_hover.contains(&target_file));
    }

    #[tokio::test]
    async fn tree_drag_over_invalid_target_clears_target_and_highlight() {
        let mut ws_state = WorkspaceState::default();
        let mut app_state = AppState::default();
        let handle = tokio::runtime::Handle::current();
        // 目录拖到自己的子目录:非法落点,不该被记为待定目标或高亮。
        ws_state.arm_tree_drag(
            PathBuf::from("/proj/a"),
            true,
            (0.0, 0.0),
            std::time::Instant::now(),
        );

        ws_state.confirm_tree_drag();
        update(
            &mut ws_state,
            &mut app_state,
            Message::TreeDragOver(PathBuf::from("/proj/a/b")),
            1,
            &handle,
            |_| {},
        );

        assert_eq!(
            ws_state.tree_drag.as_ref().map(|d| &d.phase),
            Some(&TreeDragPhase::Dragging { target: None })
        );
        assert!(ws_state.drag_hover.is_empty());
    }

    #[tokio::test]
    async fn tree_drag_end_unconfirmed_clears_drag_state_without_moving_anything() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f.txt");
        std::fs::write(&file, b"hi").unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        let mut app_state = AppState::default();
        let handle = tokio::runtime::Handle::current();
        ws_state.arm_tree_drag(file.clone(), false, (0.0, 0.0), std::time::Instant::now());

        update(
            &mut ws_state,
            &mut app_state,
            Message::TreeDragEnd(false),
            1,
            &handle,
            |_| panic!("no target armed, TreeDragEnd 不该 emit 任何消息"),
        );

        assert!(!ws_state.is_dragging_tree_item());
        assert!(ws_state.drag_hover.is_empty());
        assert!(file.exists());
    }

    /// 外部单文件拖入(main.rs 恒只带一个路径,见 `Message::FileDrop`
    /// 文档)命中落点后不立即移动——弹 `PendingMove` 确认框,同内部拖拽
    /// (`TreeDragEnd`)那支。
    #[tokio::test]
    async fn file_drop_single_path_arms_pending_move_without_moving_yet() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("sub");
        std::fs::create_dir(&target).unwrap();
        let external = tempfile::tempdir().unwrap();
        let source = external.path().join("outside.txt");
        std::fs::write(&source, b"hi").unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        let mut app_state = AppState::default();
        let handle = tokio::runtime::Handle::current();

        update(
            &mut ws_state,
            &mut app_state,
            Message::FileDrop {
                paths: vec![source.clone()],
                target: target.clone(),
            },
            1,
            &handle,
            |_| panic!("弹确认框不该 emit 任何消息"),
        );

        assert!(source.exists());
        assert_eq!(
            ws_state.pending_move,
            Some(PendingMove {
                source,
                source_is_dir: false,
                name_draft: "outside.txt".to_string(),
                dir_draft: target.display().to_string(),
            })
        );
    }

    /// 已经在项目内的文件不许走外部拖拽这条通路移动(见 `Message::
    /// FileDrop` 处理器的核心裁决注释):拒绝,不弹确认框、不动磁盘。
    #[tokio::test]
    async fn file_drop_rejects_path_already_inside_project() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("sub");
        std::fs::create_dir(&target).unwrap();
        let inside = dir.path().join("already-here.txt");
        std::fs::write(&inside, b"hi").unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        let mut app_state = AppState::default();
        let handle = tokio::runtime::Handle::current();

        update(
            &mut ws_state,
            &mut app_state,
            Message::FileDrop {
                paths: vec![inside.clone()],
                target,
            },
            1,
            &handle,
            |_| panic!("拒绝不该 emit 任何消息"),
        );

        assert!(ws_state.pending_move.is_none());
        assert!(ws_state.tree_error.is_some());
        assert!(inside.exists());
    }

    /// `TreeDragEnd(true)` 落到合法目标不再立即移动——弹 `PendingMove`
    /// 确认框(2026-09 用户实测反馈:拖拽移动不该悄无声息直接改路径),
    /// 真正的移动推迟到用户点"确定"(`Message::MoveConfirm`)才提交。
    #[tokio::test]
    async fn tree_drag_end_confirmed_arms_pending_move_without_moving_yet() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        let file = dir.path().join("f.txt");
        std::fs::write(&file, b"hi").unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        let mut app_state = AppState::default();
        let handle = tokio::runtime::Handle::current();
        ws_state.arm_tree_drag(file.clone(), false, (0.0, 0.0), std::time::Instant::now());
        ws_state.confirm_tree_drag();
        update(
            &mut ws_state,
            &mut app_state,
            Message::TreeDragOver(sub.clone()),
            1,
            &handle,
            |_| {},
        );

        update(
            &mut ws_state,
            &mut app_state,
            Message::TreeDragEnd(true),
            1,
            &handle,
            |_| panic!("弹确认框不该 emit 任何消息"),
        );

        assert!(!ws_state.is_dragging_tree_item());
        assert!(file.exists());
        assert_eq!(
            ws_state.pending_move,
            Some(PendingMove {
                source: file,
                source_is_dir: false,
                name_draft: "f.txt".to_string(),
                dir_draft: sub.display().to_string(),
            })
        );
    }

    /// `MoveConfirm` 真正提交移动——沿用确认框里的(未改过的)草稿,行为等同
    /// 旧版立即移动:同文件系统 `rename`,成功后 `FileDropDone` 刷新目标
    /// 目录。
    #[tokio::test]
    async fn move_confirm_with_unedited_draft_moves_file_and_refreshes_tree() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        let file = dir.path().join("f.txt");
        std::fs::write(&file, b"hi").unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        let mut app_state = AppState::default();
        let handle = tokio::runtime::Handle::current();
        ws_state.pending_move = Some(PendingMove {
            source: file.clone(),
            source_is_dir: false,
            name_draft: "f.txt".to_string(),
            dir_draft: sub.display().to_string(),
        });

        let (tx, rx) = tokio::sync::oneshot::channel();
        let tx = std::sync::Mutex::new(Some(tx));
        update(
            &mut ws_state,
            &mut app_state,
            Message::MoveConfirm,
            1,
            &handle,
            move |msg| {
                if let Some(tx) = tx.lock().unwrap().take() {
                    let _ = tx.send(msg);
                }
            },
        );
        assert!(ws_state.pending_move.is_none());
        let done_msg = rx.await.unwrap();
        update(&mut ws_state, &mut app_state, done_msg, 1, &handle, |_| {});

        assert!(!file.exists());
        assert!(sub.join("f.txt").exists());
        assert!(ws_state.tree_error.is_none());
    }

    /// `MoveConfirm` 改过草稿:新文件名 + 新目标目录都要生效——覆盖
    /// `move_item_to` 的"改名"能力,不是简单复用不改名的 `move_item`。
    #[tokio::test]
    async fn move_confirm_with_edited_draft_renames_and_retargets() {
        let dir = tempfile::tempdir().unwrap();
        let sub_b = dir.path().join("b");
        std::fs::create_dir(&sub_b).unwrap();
        let file = dir.path().join("f.txt");
        std::fs::write(&file, b"hi").unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        let mut app_state = AppState::default();
        let handle = tokio::runtime::Handle::current();
        ws_state.pending_move = Some(PendingMove {
            source: file.clone(),
            source_is_dir: false,
            name_draft: "renamed.txt".to_string(),
            dir_draft: sub_b.display().to_string(),
        });

        let (tx, rx) = tokio::sync::oneshot::channel();
        let tx = std::sync::Mutex::new(Some(tx));
        update(
            &mut ws_state,
            &mut app_state,
            Message::MoveConfirm,
            1,
            &handle,
            move |msg| {
                if let Some(tx) = tx.lock().unwrap().take() {
                    let _ = tx.send(msg);
                }
            },
        );
        let done_msg = rx.await.unwrap();
        update(&mut ws_state, &mut app_state, done_msg, 1, &handle, |_| {});

        assert!(!file.exists());
        assert!(!sub_b.join("f.txt").exists());
        assert!(sub_b.join("renamed.txt").exists());
        assert!(ws_state.tree_error.is_none());
    }

    /// 名字含路径分隔符:拒绝,`pending_move` 放回去(对话框留在屏幕上),
    /// 不提交任何磁盘改动。
    #[tokio::test]
    async fn move_confirm_rejects_name_with_path_separator() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f.txt");
        std::fs::write(&file, b"hi").unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        let mut app_state = AppState::default();
        let handle = tokio::runtime::Handle::current();
        ws_state.pending_move = Some(PendingMove {
            source: file.clone(),
            source_is_dir: false,
            name_draft: "a/b.txt".to_string(),
            dir_draft: dir.path().display().to_string(),
        });

        update(
            &mut ws_state,
            &mut app_state,
            Message::MoveConfirm,
            1,
            &handle,
            |_| panic!("校验失败不该 emit 任何消息"),
        );

        assert!(file.exists());
        assert!(ws_state.pending_move.is_some());
        assert!(ws_state.tree_error.is_some());
    }

    /// 目标目录不存在:拒绝,同上不提交任何磁盘改动。
    #[tokio::test]
    async fn move_confirm_rejects_nonexistent_target_dir() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f.txt");
        std::fs::write(&file, b"hi").unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        let mut app_state = AppState::default();
        let handle = tokio::runtime::Handle::current();
        ws_state.pending_move = Some(PendingMove {
            source: file.clone(),
            source_is_dir: false,
            name_draft: "f.txt".to_string(),
            dir_draft: dir.path().join("does-not-exist").display().to_string(),
        });

        update(
            &mut ws_state,
            &mut app_state,
            Message::MoveConfirm,
            1,
            &handle,
            |_| panic!("校验失败不该 emit 任何消息"),
        );

        assert!(file.exists());
        assert!(ws_state.pending_move.is_some());
        assert!(ws_state.tree_error.is_some());
    }

    /// 路径压根没变(草稿名字/目录都还是源本来的):静默当取消处理——不
    /// 提示错误、不留 `pending_move`、不动磁盘。若真调用 `move_item_to`
    /// 会撞上"已存在同名项"(目标就是源自己),但这对用户来说不是错误,
    /// 只是"什么都没变"(2026-09 用户实测反馈)。
    #[tokio::test]
    async fn move_confirm_silently_cancels_when_path_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f.txt");
        std::fs::write(&file, b"hi").unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        let mut app_state = AppState::default();
        let handle = tokio::runtime::Handle::current();
        ws_state.pending_move = Some(PendingMove {
            source: file.clone(),
            source_is_dir: false,
            name_draft: "f.txt".to_string(),
            dir_draft: dir.path().display().to_string(),
        });

        update(
            &mut ws_state,
            &mut app_state,
            Message::MoveConfirm,
            1,
            &handle,
            |_| panic!("路径未变不该 emit 任何消息"),
        );

        assert!(file.exists());
        assert!(ws_state.pending_move.is_none());
        assert!(ws_state.tree_error.is_none());
    }

    /// 目录被改成要移进它自己的子树:静默当取消处理,同上不提示错误——
    /// `move_item_to` 会拒绝这个操作,但对用户来说这只是"这么改没有意义",
    /// 不是需要红字提醒的错误。
    #[tokio::test]
    async fn move_confirm_silently_cancels_when_dir_targets_own_subtree() {
        let dir = tempfile::tempdir().unwrap();
        let src_dir = dir.path().join("parent");
        let nested = src_dir.join("child");
        std::fs::create_dir_all(&nested).unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        let mut app_state = AppState::default();
        let handle = tokio::runtime::Handle::current();
        ws_state.pending_move = Some(PendingMove {
            source: src_dir.clone(),
            source_is_dir: true,
            name_draft: "parent".to_string(),
            dir_draft: nested.display().to_string(),
        });

        update(
            &mut ws_state,
            &mut app_state,
            Message::MoveConfirm,
            1,
            &handle,
            |_| panic!("移进自己子树不该 emit 任何消息"),
        );

        assert!(src_dir.exists());
        assert!(nested.exists());
        assert!(ws_state.pending_move.is_none());
        assert!(ws_state.tree_error.is_none());
    }

    /// `MoveCancel` 整场作废,不做任何磁盘改动。
    #[tokio::test]
    async fn move_cancel_clears_pending_without_touching_disk() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f.txt");
        std::fs::write(&file, b"hi").unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        let mut app_state = AppState::default();
        let handle = tokio::runtime::Handle::current();
        ws_state.pending_move = Some(PendingMove {
            source: file.clone(),
            source_is_dir: false,
            name_draft: "f.txt".to_string(),
            dir_draft: dir.path().display().to_string(),
        });

        update(
            &mut ws_state,
            &mut app_state,
            Message::MoveCancel,
            1,
            &handle,
            |_| panic!("取消不该 emit 任何消息"),
        );

        assert!(ws_state.pending_move.is_none());
        assert!(file.exists());
    }

    /// `Pending` 阶段是纯粹的死区:悬停完全没有反应,不记 target、不高亮——
    /// 见 `TreeDragPhase` 文档。这是 2026-09 用户实测反馈"点一下就进入
    /// 拖拽态"的修复核心:按下(`arm_tree_drag`)只武装 `Pending`,不越过
    /// 距离+时长两道阈值(`App::maybe_confirm_tree_drag`)就永远不会对任何
    /// 悬停有反应。
    #[tokio::test]
    async fn tree_drag_over_is_a_no_op_while_still_pending() {
        let mut ws_state = WorkspaceState::default();
        let mut app_state = AppState::default();
        let handle = tokio::runtime::Handle::current();
        ws_state.arm_tree_drag(
            PathBuf::from("/proj/f.rs"),
            false,
            (0.0, 0.0),
            std::time::Instant::now(),
        );

        update(
            &mut ws_state,
            &mut app_state,
            Message::TreeDragOver(PathBuf::from("/proj/dst")),
            1,
            &handle,
            |_| {},
        );

        assert_eq!(
            ws_state.tree_drag.as_ref().map(|d| &d.phase),
            Some(&TreeDragPhase::Pending)
        );
        assert!(ws_state.drag_hover.is_empty());
    }

    /// `TreeDragEnd` 处理器自身的防线:即使已经是 `Dragging` 且记了个合法
    /// `target`,只要传进来的 `confirmed=false` 也绝不提交移动——`confirmed`
    /// 是提交与否的唯一判据,不看 `target` 是否存在。
    #[tokio::test]
    async fn tree_drag_end_unconfirmed_never_moves_even_if_a_target_was_recorded() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        let file = dir.path().join("f.txt");
        std::fs::write(&file, b"hi").unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        let mut app_state = AppState::default();
        let handle = tokio::runtime::Handle::current();
        ws_state.arm_tree_drag(file.clone(), false, (0.0, 0.0), std::time::Instant::now());
        ws_state.confirm_tree_drag();
        update(
            &mut ws_state,
            &mut app_state,
            Message::TreeDragOver(sub.clone()),
            1,
            &handle,
            |_| {},
        );
        assert_eq!(
            ws_state.tree_drag.as_ref().map(|d| &d.phase),
            Some(&TreeDragPhase::Dragging {
                target: Some(sub.clone())
            })
        );

        update(
            &mut ws_state,
            &mut app_state,
            Message::TreeDragEnd(false),
            1,
            &handle,
            |_| panic!("confirmed=false,TreeDragEnd 不该 emit 任何消息"),
        );

        assert!(!ws_state.is_dragging_tree_item());
        assert!(file.exists());
        assert!(!sub.join("f.txt").exists());
    }

    /// 单击(`confirmed=false`)目录只选中,不再展开/折叠——展开/折叠改由
    /// 箭头(`Message::ToggleNoSelect`)或双击(`TreeRowDoubleClick`)触发,
    /// 见两者文档。
    #[tokio::test]
    async fn tree_drag_end_unconfirmed_selects_directory_without_toggling() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("child.txt"), b"hi").unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        let mut app_state = AppState::default();
        let handle = tokio::runtime::Handle::current();
        ws_state.arm_tree_drag(sub.clone(), true, (0.0, 0.0), std::time::Instant::now());

        update(
            &mut ws_state,
            &mut app_state,
            Message::TreeDragEnd(false),
            1,
            &handle,
            |_| {},
        );

        assert_eq!(ws_state.tree_selected, Some(sub.clone()));
        assert!(
            !ws_state
                .visible_tree_rows()
                .iter()
                .any(|r| r.path == sub && r.expanded)
        );
    }

    #[tokio::test]
    async fn tree_drag_end_confirmed_without_target_does_not_toggle_directory_source() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("child.txt"), b"hi").unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        let mut app_state = AppState::default();
        let handle = tokio::runtime::Handle::current();
        ws_state.arm_tree_drag(sub.clone(), true, (0.0, 0.0), std::time::Instant::now());

        update(
            &mut ws_state,
            &mut app_state,
            Message::TreeDragEnd(true),
            1,
            &handle,
            |_| {},
        );

        assert!(
            !ws_state
                .visible_tree_rows()
                .iter()
                .any(|r| r.path == sub && r.expanded)
        );
    }

    #[test]
    fn reload_tree_from_disk_reflects_externally_added_and_deleted_files() {
        let dir = tempfile::tempdir().unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        // 初始只有根目录,展开它让行可见。
        ws_state.file_tree.as_mut().unwrap().toggle(dir.path());
        assert!(ws_state.visible_tree_rows().iter().all(|r| !r.is_dir));

        // 外部(agent/其他进程)新增一个文件:此刻未重载,树看不到。
        let added = dir.path().join("new.txt");
        std::fs::write(&added, "x").unwrap();
        assert!(!ws_state.visible_tree_rows().iter().any(|r| r.path == added));

        // `reload_tree_from_disk` 后即时反映新增。
        ws_state.reload_tree_from_disk();
        assert!(ws_state.visible_tree_rows().iter().any(|r| r.path == added));

        // 外部删除同一文件:重载后从树消失。
        std::fs::remove_file(&added).unwrap();
        ws_state.reload_tree_from_disk();
        assert!(!ws_state.visible_tree_rows().iter().any(|r| r.path == added));
    }

    // 仅在非 mac 平台成立:mac 上 `ContextMenuOpen` 直接同步弹原生 NSMenu、
    // 不再写 `app_state.context_menu`(原生菜单阻塞返回,`context_menu` 恒
    // `None`),这条"写状态 + 下一帧渲染"的旧行为只在非 mac 的 iced 弹层路径
    // 保留。
    #[cfg(not(target_os = "macos"))]
    #[tokio::test]
    async fn context_menu_open_uses_last_right_click_and_sets_selected() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("a.txt");
        std::fs::write(&target, "x").unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        let mut app_state = AppState::default();
        let handle = tokio::runtime::Handle::current();
        update(
            &mut ws_state,
            &mut app_state,
            Message::RightClickAt { x: 10.0, y: 20.0 },
            1,
            &handle,
            |_| {},
        );
        update(
            &mut ws_state,
            &mut app_state,
            Message::ContextMenuOpen {
                path: target.clone(),
                is_dir: false,
            },
            1,
            &handle,
            |_| {},
        );
        assert!(app_state.context_menu_is_some());
        assert_eq!(ws_state.tree_selected, Some(target));
    }

    #[tokio::test]
    async fn context_menu_close_clears_menu() {
        let mut ws_state = ws_with_tree(std::env::temp_dir());
        let mut app_state = AppState {
            context_menu: Some(ContextMenu {
                x: 0.0,
                y: 0.0,
                target: PathBuf::from("/x"),
                is_dir: false,
            }),
            ..AppState::default()
        };
        let handle = tokio::runtime::Handle::current();
        update(
            &mut ws_state,
            &mut app_state,
            Message::ContextMenuClose,
            1,
            &handle,
            |_| {},
        );
        assert!(!app_state.context_menu_is_some());
    }

    #[tokio::test]
    async fn copy_sets_clipboard() {
        let dir = tempfile::tempdir().unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        let mut app_state = AppState::default();
        let handle = tokio::runtime::Handle::current();
        update(
            &mut ws_state,
            &mut app_state,
            Message::Copy(dir.path().join("a.txt"), false),
            1,
            &handle,
            |_| {},
        );
        assert_eq!(
            ws_state.tree_clipboard,
            Some((dir.path().join("a.txt"), false))
        );
    }

    #[tokio::test]
    async fn paste_done_err_sets_tree_error() {
        let dir = tempfile::tempdir().unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        let mut app_state = AppState::default();
        let handle = tokio::runtime::Handle::current();
        update(
            &mut ws_state,
            &mut app_state,
            Message::PasteDone(1, Err("boom".to_string())),
            1,
            &handle,
            |_| {},
        );
        assert_eq!(ws_state.tree_error.as_deref(), Some("boom"));
    }

    #[tokio::test]
    async fn paste_done_ok_refreshes_parent_without_touching_stale_error() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        // 现有 `ProjectTreePasteDone` 的 `Ok` 分支不清 `tree_error`(只有
        // `ProjectTreeOpDone` 的 `Ok` 分支才清)——纯迁移原样保留这个不对称,
        // 不是这次重构该修的行为。
        ws_state.tree_error = Some("stale".to_string());
        let new_file = sub.join("new.txt");
        std::fs::write(&new_file, "x").unwrap();
        let mut app_state = AppState::default();
        let handle = tokio::runtime::Handle::current();
        update(
            &mut ws_state,
            &mut app_state,
            Message::PasteDone(1, Ok(new_file)),
            1,
            &handle,
            |_| {},
        );
        assert_eq!(ws_state.tree_error.as_deref(), Some("stale"));
    }

    #[tokio::test]
    async fn delete_request_then_cancel_clears_confirm() {
        let mut ws_state = ws_with_tree(std::env::temp_dir());
        let mut app_state = AppState::default();
        let handle = tokio::runtime::Handle::current();
        update(
            &mut ws_state,
            &mut app_state,
            Message::DeleteRequest(PathBuf::from("/x"), false),
            1,
            &handle,
            |_| {},
        );
        assert!(ws_state.tree_delete_confirm_is_some());
        update(
            &mut ws_state,
            &mut app_state,
            Message::DeleteCancel,
            1,
            &handle,
            |_| {},
        );
        assert!(!ws_state.tree_delete_confirm_is_some());
    }

    #[tokio::test]
    async fn op_done_err_sets_tree_error_ok_refreshes() {
        let dir = tempfile::tempdir().unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        let mut app_state = AppState::default();
        let handle = tokio::runtime::Handle::current();
        update(
            &mut ws_state,
            &mut app_state,
            Message::OpDone {
                project_id: 1,
                parent: Err("nope".to_string()),
                expand: false,
            },
            1,
            &handle,
            |_| {},
        );
        assert_eq!(ws_state.tree_error.as_deref(), Some("nope"));
        update(
            &mut ws_state,
            &mut app_state,
            Message::OpDone {
                project_id: 1,
                parent: Ok(dir.path().to_path_buf()),
                expand: false,
            },
            1,
            &handle,
            |_| {},
        );
        assert!(ws_state.tree_error.is_none());
    }

    #[tokio::test]
    async fn new_file_starts_tree_edit_and_closes_menu() {
        let dir = tempfile::tempdir().unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        let mut app_state = AppState {
            context_menu: Some(ContextMenu {
                x: 0.0,
                y: 0.0,
                target: dir.path().to_path_buf(),
                is_dir: true,
            }),
            ..AppState::default()
        };
        let handle = tokio::runtime::Handle::current();
        update(
            &mut ws_state,
            &mut app_state,
            Message::NewFile(dir.path().to_path_buf()),
            1,
            &handle,
            |_| {},
        );
        assert!(!app_state.context_menu_is_some());
        assert!(ws_state.tree_edit.is_some());
    }

    #[tokio::test]
    async fn rename_start_prefills_buffer_with_current_name() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("old.txt");
        std::fs::write(&target, "x").unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        let mut app_state = AppState::default();
        let handle = tokio::runtime::Handle::current();
        update(
            &mut ws_state,
            &mut app_state,
            Message::RenameStart(target.clone()),
            1,
            &handle,
            |_| {},
        );
        let edit = ws_state.tree_edit.as_ref().unwrap();
        assert_eq!(edit.buffer, "old.txt");
        assert!(matches!(edit.mode, TreeEditMode::Rename(ref p) if *p == target));
    }

    #[tokio::test]
    async fn edit_input_replaces_whole_buffer() {
        let dir = tempfile::tempdir().unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        let mut app_state = AppState::default();
        ws_state.tree_edit = Some(TreeEdit {
            parent_dir: dir.path().to_path_buf(),
            mode: TreeEditMode::NewFile,
            buffer: String::new(),
        });
        let handle = tokio::runtime::Handle::current();
        update(
            &mut ws_state,
            &mut app_state,
            Message::EditInput("ab".to_string()),
            1,
            &handle,
            |_| {},
        );
        assert_eq!(ws_state.tree_edit.as_ref().unwrap().buffer, "ab");
        // iced text_input 每次 on_input 给全量当前字符串,不是逐字符追加。
        update(
            &mut ws_state,
            &mut app_state,
            Message::EditInput("a".to_string()),
            1,
            &handle,
            |_| {},
        );
        assert_eq!(ws_state.tree_edit.as_ref().unwrap().buffer, "a");
    }

    #[test]
    fn set_tree_edit_focused_only_tracks_focus_flag() {
        let dir = tempfile::tempdir().unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        ws_state.tree_edit = Some(TreeEdit {
            parent_dir: dir.path().to_path_buf(),
            mode: TreeEditMode::NewFile,
            buffer: "ab".to_string(),
        });
        // 先置真:进入聚焦态。
        ws_state.set_tree_edit_focused(true);
        assert!(ws_state.tree_edit.is_some());
        assert!(ws_state.tree_edit_focused());
        // 焦点从真变假:这里只落焦点标志位——该不该保存/是否清空由内核
        // `App::set_tree_edit_focused` 在边缘处调 `submit_tree_edit` 决定,
        // `set_tree_edit_focused` 本身不再改动编辑态。
        ws_state.set_tree_edit_focused(false);
        assert!(ws_state.tree_edit.is_some());
        assert!(!ws_state.tree_edit_focused());
    }

    #[tokio::test]
    async fn submit_edit_creates_new_file_on_success() {
        let dir = tempfile::tempdir().unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        let mut app_state = AppState::default();
        ws_state.tree_edit = Some(TreeEdit {
            parent_dir: dir.path().to_path_buf(),
            mode: TreeEditMode::NewFile,
            buffer: "fresh.txt".to_string(),
        });
        let handle = tokio::runtime::Handle::current();
        update(
            &mut ws_state,
            &mut app_state,
            Message::EditSubmit,
            1,
            &handle,
            // 成功路径会 spawn 一个异步落盘任务,这里不能 panic。
            |_| {},
        );
        // 提交即取走编辑态(退出行内编辑框)。
        assert!(ws_state.tree_edit.is_none());
        // 让当前线程的 tokio 运行时有空档轮询刚 spawn 的落盘任务,再断言。
        // `spawn_blocking` 会把文件系统操作丢给阻塞线程池,这里 `.await`
        // 主动让出执行器,等它把结果送回。
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(dir.path().join("fresh.txt").exists());
    }

    #[tokio::test]
    async fn submit_edit_empty_name_cancels_and_closes_edit_box() {
        let dir = tempfile::tempdir().unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        let mut app_state = AppState::default();
        ws_state.tree_edit = Some(TreeEdit {
            parent_dir: dir.path().to_path_buf(),
            mode: TreeEditMode::NewFile,
            buffer: "   ".to_string(),
        });
        let handle = tokio::runtime::Handle::current();
        update(
            &mut ws_state,
            &mut app_state,
            Message::EditSubmit,
            1,
            &handle,
            |_| panic!("空名字不该发起任何异步操作"),
        );
        // 现有 `submit_tree_edit` 先 `take()` 再判断空名字,空名字直接 `return`
        // 时 `tree_edit` 已经被取走——提交空名字会关闭行内编辑框(等价于取消),
        // 不是保留编辑框让用户继续输入。
        assert!(ws_state.tree_edit.is_none());
    }

    #[tokio::test]
    async fn submit_edit_rejects_path_separator_in_name() {
        let dir = tempfile::tempdir().unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        let mut app_state = AppState::default();
        ws_state.tree_edit = Some(TreeEdit {
            parent_dir: dir.path().to_path_buf(),
            mode: TreeEditMode::NewFile,
            buffer: "a/b".to_string(),
        });
        let handle = tokio::runtime::Handle::current();
        update(
            &mut ws_state,
            &mut app_state,
            Message::EditSubmit,
            1,
            &handle,
            |_| panic!("名字非法时不该发起任何异步操作"),
        );
        assert_eq!(
            ws_state.tree_error.as_deref(),
            Some("名字不能包含路径分隔符")
        );
        assert!(ws_state.tree_edit.is_some(), "编辑框保留,让用户改名重试");
    }

    #[tokio::test]
    async fn submit_edit_rejects_existing_same_name() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("dup.txt"), "x").unwrap();
        let mut ws_state = ws_with_tree(dir.path().to_path_buf());
        let mut app_state = AppState::default();
        ws_state.tree_edit = Some(TreeEdit {
            parent_dir: dir.path().to_path_buf(),
            mode: TreeEditMode::NewFile,
            buffer: "dup.txt".to_string(),
        });
        let handle = tokio::runtime::Handle::current();
        update(
            &mut ws_state,
            &mut app_state,
            Message::EditSubmit,
            1,
            &handle,
            |_| panic!("已存在同名项时不该发起任何异步操作"),
        );
        assert!(
            ws_state
                .tree_error
                .as_deref()
                .unwrap()
                .contains("已存在同名项")
        );
    }

    #[tokio::test]
    async fn statuses_refreshed_updates_git_statuses() {
        let mut ws_state = ws_with_tree(std::env::temp_dir());
        let mut app_state = AppState::default();
        let mut statuses = HashMap::new();
        statuses.insert(
            PathBuf::from("/x"),
            crate::delivery::FileGitStatus {
                kind: crate::delivery::ChangeKind::Modified,
                staged: false,
                unstaged: true,
                ignored: false,
            },
        );
        let handle = tokio::runtime::Handle::current();
        update(
            &mut ws_state,
            &mut app_state,
            Message::StatusesRefreshed(1, statuses.clone()),
            1,
            &handle,
            |_| {},
        );
        assert_eq!(ws_state.git_statuses.len(), 1);
    }

    #[tokio::test]
    #[should_panic(expected = "由内核拦截处理")]
    async fn copy_path_reaching_update_panics() {
        let mut ws_state = ws_with_tree(std::env::temp_dir());
        let mut app_state = AppState::default();
        let handle = tokio::runtime::Handle::current();
        update(
            &mut ws_state,
            &mut app_state,
            Message::CopyPath(PathBuf::from("/x"), PathKind::Absolute),
            1,
            &handle,
            |_| {},
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn context_menu_items_hides_delete_rename_for_root() {
        let items = context_menu_items(Path::new("/proj"), true, true, false, true);
        let has_delete = items.iter().any(|i| {
            matches!(
                i,
                crate::chrome::native_menu::Item::Entry {
                    msg: Message::DeleteRequest(..),
                    ..
                }
            )
        });
        assert!(!has_delete, "项目根目录不该出现删除项");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn context_menu_items_shows_delete_rename_for_non_root() {
        let items = context_menu_items(Path::new("/proj/src"), true, false, false, true);
        let has_delete = items.iter().any(|i| {
            matches!(
                i,
                crate::chrome::native_menu::Item::Entry {
                    msg: Message::DeleteRequest(..),
                    ..
                }
            )
        });
        assert!(has_delete, "非根目录该有删除项");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn context_menu_items_paste_locked_when_clipboard_empty() {
        let items = context_menu_items(Path::new("/proj/src"), true, false, false, true);
        let paste = items.iter().find_map(|i| match i {
            crate::chrome::native_menu::Item::Entry {
                msg: Message::Paste(_),
                enabled,
                ..
            } => Some(*enabled),
            _ => None,
        });
        assert_eq!(paste, Some(false), "剪贴槽为空时粘贴该锁定");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn context_menu_items_paste_enabled_when_clipboard_has_content() {
        let items = context_menu_items(Path::new("/proj/src"), true, false, true, true);
        let paste = items.iter().find_map(|i| match i {
            crate::chrome::native_menu::Item::Entry {
                msg: Message::Paste(_),
                enabled,
                ..
            } => Some(*enabled),
            _ => None,
        });
        assert_eq!(paste, Some(true));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn context_menu_items_file_target_has_no_new_file_or_paste() {
        let items = context_menu_items(Path::new("/proj/src/main.rs"), false, false, false, true);
        let has_new_file = items.iter().any(|i| {
            matches!(
                i,
                crate::chrome::native_menu::Item::Entry {
                    msg: Message::NewFile(_),
                    ..
                }
            )
        });
        let has_paste = items.iter().any(|i| {
            matches!(
                i,
                crate::chrome::native_menu::Item::Entry {
                    msg: Message::Paste(_),
                    ..
                }
            )
        });
        assert!(!has_new_file && !has_paste, "非目录不该有新建文件/粘贴");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn context_menu_items_shows_file_history_for_git_repo_file() {
        let items = context_menu_items(Path::new("/proj/src/main.rs"), false, false, false, true);
        let has_history = items.iter().any(|i| {
            matches!(
                i,
                crate::chrome::native_menu::Item::Entry {
                    msg: Message::FileHistoryOpen(..),
                    ..
                }
            )
        });
        assert!(has_history, "git 仓库里的文件行该有「查看此文件历史」");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn context_menu_items_hides_file_history_for_non_git_repo() {
        let items = context_menu_items(Path::new("/proj/src/main.rs"), false, false, false, false);
        let has_history = items.iter().any(|i| {
            matches!(
                i,
                crate::chrome::native_menu::Item::Entry {
                    msg: Message::FileHistoryOpen(..),
                    ..
                }
            )
        });
        assert!(!has_history, "非 git 仓库不该出现「查看此文件历史」");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn context_menu_items_hides_file_history_for_directory() {
        let items = context_menu_items(Path::new("/proj/src"), true, false, false, true);
        let has_history = items.iter().any(|i| {
            matches!(
                i,
                crate::chrome::native_menu::Item::Entry {
                    msg: Message::FileHistoryOpen(..),
                    ..
                }
            )
        });
        assert!(!has_history, "目录行不该出现「查看此文件历史」");
    }
}
