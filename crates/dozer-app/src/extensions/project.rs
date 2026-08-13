//! 项目信息(Project)面板:项目名、git 分支/脏标、验收次数。阶段 1 扩展化
//! 重构项目,设计见 `docs/superpowers/specs/2026-08-13-project-info-pane-v2-design.md`。
use crate::delivery::WorktreeInfo;
use crate::{icons, theme};
use dozer_core::protocol::ProjectInfo;
use iced_widget::core::{Border, Element, Length};
use iced_widget::{column, container, row, text};

/// 挂在每个 Workspace 上的项目信息面板状态。
#[derive(Default)]
pub struct WorkspaceState {
    branch: Option<String>,
    dirty: bool,
    worktrees: Vec<WorktreeInfo>,
    project_acceptance_count: Option<u64>,
    error: Option<String>,
}

impl WorkspaceState {
    /// 供内核 `worktree_strip`(Git Log 视图外层装饰,不属于
    /// `extensions::git_log`)读取——`worktrees` 数据来自组合 git 刷新,但
    /// 消费方是 Git Log 视图,归属判断见设计文档"关键语义确认"。
    pub fn worktrees(&self) -> &[WorktreeInfo] {
        &self.worktrees
    }
}

/// 组合 git 刷新结果里跟 Project 有关的部分(`branch`/`dirty`/`worktrees`)、
/// 验收次数。`GitRefreshed`/`AcceptanceCountLoaded` 由内核分发,带
/// `project_id`,走 `with_project`。
#[derive(Debug, Clone)]
pub enum Message {
    GitRefreshed(i64, Option<String>, bool, Vec<WorktreeInfo>),
    AcceptanceCountLoaded(i64, Option<u64>),
}

/// 处理全部消息——本模块不触碰终端会话域,没有需要内核拦截、`update` 里
/// `unreachable!` 的消息(不像 Files/Acceptance),全部同步完成,不需要
/// `handle`/`emit`。
pub fn update(ws_state: &mut WorkspaceState, msg: Message, _project_id: i64) {
    match msg {
        Message::GitRefreshed(_, branch, dirty, worktrees) => {
            ws_state.branch = branch;
            ws_state.dirty = dirty;
            ws_state.worktrees = worktrees;
        }
        Message::AcceptanceCountLoaded(_, n) => {
            ws_state.project_acceptance_count = n;
        }
    }
}

/// 项目卡分支标签:`分支` / `分支*`(脏)/ `—`(非 git)。
fn project_branch_label(branch: Option<&str>, dirty: bool) -> String {
    match branch {
        Some(b) if dirty => format!("{b}*"),
        Some(b) => b.to_string(),
        None => "—".to_string(),
    }
}

/// 面板主入口(单栏,不与任何其它面板配对——同 GitLog/Usage)。`project` 为
/// `None` 时内核不会真正走到这里(`left_panel_area` 对 `LeftView::Project`
/// 无条件调用本函数,但 `App::view()` 顶层只在有聚焦项目时才会渲染到这个
/// 分支),这里仍保留一次防御性判断,风格对齐 Files 试点。
pub fn view<'a>(
    ws_state: &'a WorkspaceState,
    project: Option<&'a ProjectInfo>,
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let Some(p) = project else {
        return container(iced_widget::Space::new())
            .width(width)
            .height(Length::Fill)
            .into();
    };

    let mut content = column![].spacing(12).padding(14);

    content = content.push(crate::homespace::home_panel_head(
        icons::IconKind::Briefcase,
        "项目",
    ));

    content = content.push(
        text(p.name.clone())
            .size(theme::font::title())
            .color(theme::color::CREAM),
    );

    let label = project_branch_label(ws_state.branch.as_deref(), ws_state.dirty);
    let bcolor = if ws_state.dirty {
        theme::color::GOLD
    } else {
        theme::color::BODY
    };
    content = content.push(
        row![
            icons::view(
                icons::IconKind::GitBranch,
                crate::theme::icon_size::row(),
                bcolor
            ),
            text(label).size(theme::font::label()).color(bcolor),
        ]
        .spacing(6)
        .align_y(iced_widget::core::Alignment::Center),
    );

    if let Some(n) = ws_state.project_acceptance_count.filter(|n| *n > 0) {
        content = content.push(
            text(format!("{n} 次验收"))
                .size(theme::font::caption())
                .color(theme::color::GOLD),
        );
    }

    container(content)
        .width(width)
        .height(Length::Fill)
        .style(
            move |_t: &iced_widget::Theme| iced_widget::container::Style {
                background: Some(theme::color::PANEL.into()),
                border: outer,
                ..iced_widget::container::Style::default()
            },
        )
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_ws() -> WorkspaceState {
        WorkspaceState::default()
    }

    #[test]
    fn git_refreshed_updates_three_fields() {
        let mut ws = new_ws();
        update(
            &mut ws,
            Message::GitRefreshed(1, Some("main".to_string()), true, vec![]),
            1,
        );
        assert_eq!(ws.branch.as_deref(), Some("main"));
        assert!(ws.dirty);
        assert_eq!(ws.worktrees().len(), 0);
    }

    #[test]
    fn acceptance_count_loaded_sets_field() {
        let mut ws = new_ws();
        update(&mut ws, Message::AcceptanceCountLoaded(1, Some(3)), 1);
        assert_eq!(ws.project_acceptance_count, Some(3));
    }

    #[test]
    fn project_card_branch_label() {
        assert_eq!(project_branch_label(Some("main"), false), "main");
        assert_eq!(project_branch_label(Some("main"), true), "main*");
        assert_eq!(project_branch_label(None, false), "—");
    }
}
