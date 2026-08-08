//! 项目信息(Project)面板:项目名、git 分支/脏标、验收次数、目标(标题+
//! 标准列表,支持应用内编辑)。阶段 1 扩展化重构第七个试点,设计见
//! `docs/superpowers/specs/2026-08-08-project-info-pane-design.md`。
use crate::delivery::WorktreeInfo;
use crate::goal::{self, Goal};
use crate::workspace::AddrEvent;
use std::path::Path;

/// 挂在每个 Workspace 上的项目信息面板状态。
#[derive(Default)]
pub struct WorkspaceState {
    branch: Option<String>,
    dirty: bool,
    worktrees: Vec<WorktreeInfo>,
    project_acceptance_count: Option<u64>,
    goal: Option<Goal>,
    /// 目标标题的行内编辑态(None=未在编辑;创建新目标时也复用这个字段,
    /// `goal` 为 `None` 且 `title_editing` 有值就是"正在设置首个目标")。
    title_editing: Option<String>,
    /// 新增标准的输入草稿。
    add_criterion_draft: String,
    error: Option<String>,
}

impl WorkspaceState {
    /// 打开一个新项目时构造(`goal` 由调用方通过 `load_goal` 同步读一次
    /// `.dozer/goal.md` 拿到,传进来)。
    pub fn new(goal: Option<Goal>) -> Self {
        Self {
            goal,
            ..Self::default()
        }
    }

    /// 供内核 `worktree_strip`(Git Log 视图外层装饰,不属于
    /// `extensions::git_log`)读取——`worktrees` 数据来自组合 git 刷新,但
    /// 消费方是 Git Log 视图,归属判断见设计文档"关键语义确认"。
    pub fn worktrees(&self) -> &[WorktreeInfo] {
        &self.worktrees
    }

    /// 供内核 main.rs 键盘路由判断"标题是否在自绘编辑态"。
    pub fn title_editing_is_some(&self) -> bool {
        self.title_editing.is_some()
    }

    /// 供内核 `Workspace::blur_inputs` 调用——点击输入框外时取消标题编辑
    /// (不保存半输入,同 Files 试点项目树编辑取消的处理口径)。
    pub fn cancel_title_edit(&mut self) {
        self.title_editing = None;
    }
}

/// 同步读一次 `.dozer/goal.md`(文件极小,可容忍同步读)。现有
/// `workspace.rs::load_project_goal` 的搬家版本,仅参数类型从 `&str` 改成
/// `&Path`。供内核在项目打开(`from_restore`/`loading_for_project`)/
/// `adopt_project` 时调用,结果传给 `WorkspaceState::new`。
pub fn load_goal(repo_path: &Path) -> Option<Goal> {
    let md = std::fs::read_to_string(goal::goal_path(repo_path)).ok()?;
    goal::parse_goal(&md)
}

/// 组合 git 刷新结果里跟 Project 有关的部分(`branch`/`dirty`/`worktrees`)、
/// 验收次数、标题/标准列表的应用内编辑。`GitRefreshed`/`AcceptanceCountLoaded`
/// 由内核分发,带 `project_id`,走 `with_project`;其余是用户交互消息,走
/// `with_focused_project`。
#[derive(Debug, Clone)]
pub enum Message {
    GitRefreshed(i64, Option<String>, bool, Vec<WorktreeInfo>),
    AcceptanceCountLoaded(i64, Option<u64>),
    TitleEditStart,
    TitleEditEvent(AddrEvent),
    CriterionAddInputChanged(String),
    CriterionAddSubmit,
    CriterionRemove(usize),
}

/// 处理全部消息——本模块不触碰终端会话域,没有需要内核拦截、`update` 里
/// `unreachable!` 的消息(不像 Files/Acceptance),全部同步完成,不需要
/// `handle`/`emit`。
pub fn update(ws_state: &mut WorkspaceState, msg: Message, _project_id: i64, repo_path: &Path) {
    match msg {
        Message::GitRefreshed(_, branch, dirty, worktrees) => {
            ws_state.branch = branch;
            ws_state.dirty = dirty;
            ws_state.worktrees = worktrees;
        }
        Message::AcceptanceCountLoaded(_, n) => {
            ws_state.project_acceptance_count = n;
        }
        Message::TitleEditStart => {
            ws_state.title_editing = Some(
                ws_state
                    .goal
                    .as_ref()
                    .map(|g| g.title.clone())
                    .unwrap_or_default(),
            );
        }
        Message::TitleEditEvent(ev) => match ev {
            AddrEvent::Text(s) => {
                if let Some(buf) = &mut ws_state.title_editing {
                    buf.push_str(&s);
                }
            }
            AddrEvent::Backspace => {
                if let Some(buf) = &mut ws_state.title_editing {
                    buf.pop();
                }
            }
            AddrEvent::Cancel => ws_state.title_editing = None,
            AddrEvent::Submit => {
                let Some(raw) = ws_state.title_editing.take() else {
                    return;
                };
                let title = raw.trim().to_string();
                if title.is_empty() {
                    // `take()` 已经关闭编辑框——空标题视为取消,同 Files
                    // 试点"提交空名字关闭编辑框而非保留"的既有处理口径。
                    return;
                }
                let goal = match &ws_state.goal {
                    Some(g) => Goal {
                        title,
                        criteria: g.criteria.clone(),
                    },
                    None => Goal {
                        title,
                        criteria: Vec::new(),
                    },
                };
                match goal::write_goal(repo_path, &goal) {
                    Ok(()) => {
                        ws_state.error = None;
                        ws_state.goal = Some(goal);
                    }
                    Err(e) => {
                        ws_state.error = Some(format!("保存失败: {e}"));
                        // 写盘失败,保留原始输入(未 trim)允许重试。
                        ws_state.title_editing = Some(raw);
                    }
                }
            }
        },
        Message::CriterionAddInputChanged(s) => ws_state.add_criterion_draft = s,
        Message::CriterionAddSubmit => {
            let text = ws_state.add_criterion_draft.trim().to_string();
            if text.is_empty() {
                ws_state.add_criterion_draft = String::new();
                return;
            }
            let Some(goal) = &mut ws_state.goal else {
                return;
            };
            goal.criteria.push(text);
            match goal::write_goal(repo_path, goal) {
                Ok(()) => {
                    ws_state.error = None;
                    ws_state.add_criterion_draft = String::new();
                }
                Err(e) => {
                    goal.criteria.pop();
                    ws_state.error = Some(format!("保存失败: {e}"));
                    // `add_criterion_draft` 不清空,允许重试。
                }
            }
        }
        Message::CriterionRemove(i) => {
            let Some(goal) = &mut ws_state.goal else {
                return;
            };
            if i >= goal.criteria.len() {
                return;
            }
            let removed = goal.criteria.remove(i);
            if let Err(e) = goal::write_goal(repo_path, goal) {
                goal.criteria.insert(i, removed);
                ws_state.error = Some(format!("保存失败: {e}"));
            } else {
                ws_state.error = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws_with_goal(goal: Option<Goal>) -> WorkspaceState {
        WorkspaceState::new(goal)
    }

    #[test]
    fn title_edit_start_prefills_existing_title() {
        let mut ws = ws_with_goal(Some(Goal {
            title: "旧标题".into(),
            criteria: vec![],
        }));
        let repo = tempfile::tempdir().unwrap();
        update(&mut ws, Message::TitleEditStart, 1, repo.path());
        assert_eq!(ws.title_editing.as_deref(), Some("旧标题"));
    }

    #[test]
    fn title_edit_start_empty_when_no_goal() {
        let mut ws = ws_with_goal(None);
        let repo = tempfile::tempdir().unwrap();
        update(&mut ws, Message::TitleEditStart, 1, repo.path());
        assert_eq!(ws.title_editing.as_deref(), Some(""));
    }

    #[test]
    fn title_edit_submit_creates_new_goal_and_writes_disk() {
        let mut ws = ws_with_goal(None);
        let repo = tempfile::tempdir().unwrap();
        update(&mut ws, Message::TitleEditStart, 1, repo.path());
        update(
            &mut ws,
            Message::TitleEditEvent(AddrEvent::Text("新目标".into())),
            1,
            repo.path(),
        );
        update(
            &mut ws,
            Message::TitleEditEvent(AddrEvent::Submit),
            1,
            repo.path(),
        );
        assert!(ws.title_editing.is_none());
        assert_eq!(ws.goal.as_ref().unwrap().title, "新目标");
        let content = std::fs::read_to_string(goal::goal_path(repo.path())).unwrap();
        assert!(content.starts_with("# 新目标"));
    }

    #[test]
    fn title_edit_submit_updates_title_keeps_criteria() {
        let mut ws = ws_with_goal(Some(Goal {
            title: "旧".into(),
            criteria: vec!["标准".into()],
        }));
        let repo = tempfile::tempdir().unwrap();
        update(&mut ws, Message::TitleEditStart, 1, repo.path());
        update(
            &mut ws,
            Message::TitleEditEvent(AddrEvent::Backspace),
            1,
            repo.path(),
        );
        update(
            &mut ws,
            Message::TitleEditEvent(AddrEvent::Text("新".into())),
            1,
            repo.path(),
        );
        update(
            &mut ws,
            Message::TitleEditEvent(AddrEvent::Submit),
            1,
            repo.path(),
        );
        let g = ws.goal.as_ref().unwrap();
        assert_eq!(g.title, "新");
        assert_eq!(g.criteria, vec!["标准".to_string()]);
    }

    #[test]
    fn title_edit_submit_empty_closes_without_writing() {
        let mut ws = ws_with_goal(None);
        let repo = tempfile::tempdir().unwrap();
        update(&mut ws, Message::TitleEditStart, 1, repo.path());
        update(
            &mut ws,
            Message::TitleEditEvent(AddrEvent::Submit),
            1,
            repo.path(),
        );
        assert!(ws.title_editing.is_none());
        assert!(ws.goal.is_none());
        assert!(!goal::goal_path(repo.path()).exists());
    }

    #[test]
    fn criterion_add_submit_appends_and_writes_disk() {
        let mut ws = ws_with_goal(Some(Goal {
            title: "标题".into(),
            criteria: vec![],
        }));
        let repo = tempfile::tempdir().unwrap();
        update(
            &mut ws,
            Message::CriterionAddInputChanged("新标准".into()),
            1,
            repo.path(),
        );
        update(&mut ws, Message::CriterionAddSubmit, 1, repo.path());
        assert_eq!(
            ws.goal.as_ref().unwrap().criteria,
            vec!["新标准".to_string()]
        );
        assert_eq!(ws.add_criterion_draft, "");
        let content = std::fs::read_to_string(goal::goal_path(repo.path())).unwrap();
        assert!(content.contains("- [ ] 新标准"));
    }

    #[test]
    fn criterion_add_submit_empty_draft_is_noop() {
        let mut ws = ws_with_goal(Some(Goal {
            title: "标题".into(),
            criteria: vec![],
        }));
        let repo = tempfile::tempdir().unwrap();
        update(&mut ws, Message::CriterionAddSubmit, 1, repo.path());
        assert!(ws.goal.as_ref().unwrap().criteria.is_empty());
    }

    #[test]
    fn criterion_remove_deletes_and_writes_disk() {
        let g = Goal {
            title: "标题".into(),
            criteria: vec!["a".into(), "b".into()],
        };
        let repo = tempfile::tempdir().unwrap();
        goal::write_goal(repo.path(), &g).unwrap();
        let mut ws = ws_with_goal(Some(g));
        update(&mut ws, Message::CriterionRemove(0), 1, repo.path());
        assert_eq!(ws.goal.as_ref().unwrap().criteria, vec!["b".to_string()]);
        let content = std::fs::read_to_string(goal::goal_path(repo.path())).unwrap();
        assert!(!content.contains("- [ ] a"));
        assert!(content.contains("- [ ] b"));
    }

    #[test]
    fn git_refreshed_updates_three_fields() {
        let mut ws = ws_with_goal(None);
        let repo = tempfile::tempdir().unwrap();
        update(
            &mut ws,
            Message::GitRefreshed(1, Some("main".to_string()), true, vec![]),
            1,
            repo.path(),
        );
        assert_eq!(ws.branch.as_deref(), Some("main"));
        assert!(ws.dirty);
        assert_eq!(ws.worktrees().len(), 0);
    }

    #[test]
    fn acceptance_count_loaded_sets_field() {
        let mut ws = ws_with_goal(None);
        let repo = tempfile::tempdir().unwrap();
        update(
            &mut ws,
            Message::AcceptanceCountLoaded(1, Some(3)),
            1,
            repo.path(),
        );
        assert_eq!(ws.project_acceptance_count, Some(3));
    }

    #[test]
    fn load_goal_reads_existing_file() {
        let repo = tempfile::tempdir().unwrap();
        let g = Goal {
            title: "读回".into(),
            criteria: vec!["一".into()],
        };
        goal::write_goal(repo.path(), &g).unwrap();
        assert_eq!(load_goal(repo.path()), Some(g));
    }

    #[test]
    fn load_goal_missing_file_is_none() {
        let repo = tempfile::tempdir().unwrap();
        assert_eq!(load_goal(repo.path()), None);
    }
}
