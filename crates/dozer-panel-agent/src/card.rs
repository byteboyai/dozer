//! 卡片纯数据与刷新(S6c-2:从 `dozer-app` 的 `workspace/{view,state}.rs`
//! 搬入)。

use crate::adapter::TaskTitleSource;
use crate::style::{agent_state_label, dot_color, format_model_label};
use crate::tabs::{SessionTab, WorkspaceGitInfo, tab_title};
use crate::update::Message;
use dozer_model::AgentKind;
use iced_widget::core::Color;
use std::path::{Path, PathBuf};

/// 卡片渲染上下文:除 tab 自身字段外,卡片文本只依赖这三项。
pub struct CardCtx<'a> {
    pub titles: &'a dyn TaskTitleSource,
    pub project_branch: Option<&'a str>,
    pub project_dirty: bool,
}

/// 卡片纯数据(可测)。
#[derive(Debug, Clone, PartialEq)]
pub struct CardModel {
    pub title: String,
    pub work_line: String,
    pub status: String,
    pub dot: Color,
    pub active: bool,
}

/// 标题:`"{title} ({model}, {mode})"`(只有一项只带那一项,都无则无括号)。
pub fn card_title(
    agent: AgentKind,
    cwd: Option<&Path>,
    fallback: &str,
    llm_model: Option<&str>,
    permission_mode: Option<&str>,
) -> String {
    let model_label = llm_model.map(format_model_label);
    let llm_mode_value = match (&model_label, permission_mode) {
        (Some(m), Some(mo)) => Some(format!("{m}, {mo}")),
        (Some(m), None) => Some(m.clone()),
        (None, Some(mo)) => Some(mo.to_string()),
        (None, None) => None,
    };
    let title = tab_title(agent, cwd, fallback);
    match llm_mode_value {
        Some(v) => format!("{title} ({v})"),
        None => title,
    }
}

/// 工作行:`"{work}  @{branch}"`;`work` 缺省只有 `@{branch}`;脏标后缀
/// `(Uncommitted)`;无分支为 `—`。
pub fn work_line(work: Option<&str>, branch: Option<&str>, dirty: bool) -> String {
    let workspace_value = match branch {
        Some(b) if dirty => format!("{b}(Uncommitted)"),
        Some(b) => b.to_string(),
        None => "—".to_string(),
    };
    match work {
        Some(w) => format!("{w}  @{workspace_value}"),
        None => format!("@{workspace_value}"),
    }
}

/// 把 tab + 上下文组装成卡片纯数据。
pub fn card_model(tab: &SessionTab, active: bool, ctx: &CardCtx<'_>, locale: &str) -> CardModel {
    let work = ctx
        .titles
        .task_title_for_session(&tab.info.id)
        .or_else(|| tab.last_activity.clone());
    let (branch, dirty) = match &tab.workspace_override {
        Some(w) => (w.branch.as_deref(), w.dirty),
        None => (ctx.project_branch, ctx.project_dirty),
    };
    CardModel {
        title: card_title(
            tab.agent,
            tab.cwd.as_deref(),
            &tab.info.name,
            tab.llm_model.as_deref(),
            tab.permission_mode.as_deref(),
        ),
        work_line: work_line(work.as_deref(), branch, dirty),
        status: agent_state_label(tab.agent_state, locale),
        dot: dot_color(tab.agent_state, tab.alive),
        active,
    }
}

/// 决定这次 hook 事件驱动的刷新要不要做 model/mode、activity、workspace。
pub fn refresh_plan(
    agent: AgentKind,
    cwd: &Path,
    project_root: Option<&Path>,
) -> (bool, bool, bool) {
    let needs_model_mode = matches!(
        agent,
        AgentKind::Claude
            | AgentKind::Unknown
            | AgentKind::Codebuddy
            | AgentKind::Opencode
            | AgentKind::Codex
    );
    let needs_activity = true;
    let needs_workspace = match project_root {
        Some(root) => !cwd.starts_with(root),
        None => true,
    };
    (needs_model_mode, needs_activity, needs_workspace)
}

/// 卡片刷新落地:`model/mode/activity` 的 `None` 保留旧值;`workspace` 的
/// `None` 是明确信号("cwd 未偏离项目根"),无条件覆盖。
pub fn apply_refresh(
    tabs: &mut [SessionTab],
    tab_id: usize,
    llm_model: Option<String>,
    mode: Option<String>,
    activity: Option<String>,
    workspace: Option<WorkspaceGitInfo>,
) {
    let Some(tab) = tabs.iter_mut().find(|t| t.tab_id == tab_id) else {
        return;
    };
    if llm_model.is_some() {
        tab.llm_model = llm_model;
    }
    if mode.is_some() {
        tab.permission_mode = mode;
    }
    if activity.is_some() {
        tab.last_activity = activity;
    }
    tab.workspace_override = workspace;
}

/// agent 会话当前 cwd 所属仓库的分支/脏标。
pub fn workspace_git_info(cwd: &Path) -> Option<WorkspaceGitInfo> {
    let repo = bytegit::Repo::discover_workdir(cwd).ok()?;
    Some(WorkspaceGitInfo {
        branch: repo.head().ok().and_then(|h| h.branch),
        dirty: repo
            .is_dirty(bytegit::StatusOptions::default())
            .unwrap_or(false),
    })
}

/// 异步刷新一个 tab 的卡片数据(原 `Workspace::spawn_agent_card_refresh`)。
pub fn spawn_refresh(
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
    tab_id: usize,
    agent: AgentKind,
    transcript_path: Option<String>,
    cwd: PathBuf,
    project_root: Option<PathBuf>,
) {
    let (needs_model_mode, needs_activity, needs_workspace) =
        refresh_plan(agent, &cwd, project_root.as_deref());
    if !needs_model_mode && !needs_activity && !needs_workspace {
        emit(Message::CardRefreshed {
            tab_id,
            llm_model: None,
            mode: None,
            activity: None,
            workspace: None,
        });
        return;
    }
    handle.spawn(async move {
        let (llm_model, mode, activity, workspace) = tokio::task::spawn_blocking(move || {
            let (llm_model, mode, activity) = if needs_model_mode || needs_activity {
                let (m, mo, a) = transcript_path
                    .and_then(|p| std::fs::read_to_string(p).ok())
                    .map(|s| crate::transcript_tail::latest_model_mode_and_activity(&s))
                    .unwrap_or((None, None, None));
                (
                    if needs_model_mode { m } else { None },
                    if needs_model_mode { mo } else { None },
                    if needs_activity { a } else { None },
                )
            } else {
                (None, None, None)
            };
            let workspace = if needs_workspace {
                workspace_git_info(&cwd)
            } else {
                None
            };
            (llm_model, mode, activity, workspace)
        })
        .await
        .unwrap_or((None, None, None, None));
        emit(Message::CardRefreshed {
            tab_id,
            llm_model,
            mode,
            activity,
            workspace,
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::TaskTitleSource;
    use crate::tabs::{TabBackend, WorkspaceGitInfo};
    use dozer_model::{AgentKind, AgentState, SessionInfo};
    use dozer_panel_term::model::TerminalModel;
    use dozer_panel_term::osc::OscScanner;

    struct Titles(Option<&'static str>);
    impl TaskTitleSource for Titles {
        fn task_title_for_session(&self, _id: &str) -> Option<String> {
            self.0.map(str::to_string)
        }
    }

    fn info(id: &str, agent: AgentKind) -> SessionInfo {
        SessionInfo {
            id: id.into(),
            name: "sess".into(),
            command: "sh".into(),
            cwd: "/tmp".into(),
            alive: true,
            created_ms: 0,
            agent_state: AgentState::Idle,
            transcript_path: None,
            project_id: Some(7),
            agent,
        }
    }

    fn tab(handle: &tokio::runtime::Handle) -> SessionTab {
        SessionTab {
            info: info("s1", AgentKind::Claude),
            model: TerminalModel::new(20, 4),
            alive: true,
            agent_state: AgentState::Idle,
            agent: AgentKind::Claude,
            transcript_path: None,
            osc: OscScanner::new(),
            cwd: None,
            last_exit: None,
            llm_model: None,
            permission_mode: None,
            last_activity: None,
            workspace_override: None,
            tab_id: 1,
            forwarder: handle.spawn(async {}),
            backend: TabBackend::Daemon,
        }
    }

    #[test]
    fn refresh_keeps_previous_values_when_the_transcript_is_unreadable() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut tabs = vec![tab(rt.handle())];
        tabs[0].llm_model = Some("keep-model".into());
        tabs[0].permission_mode = Some("keep-mode".into());
        tabs[0].last_activity = Some("keep-activity".into());
        apply_refresh(
            &mut tabs,
            1,
            None,
            None,
            None,
            Some(WorkspaceGitInfo {
                branch: Some("feat".into()),
                dirty: true,
            }),
        );
        assert_eq!(tabs[0].llm_model.as_deref(), Some("keep-model"));
        assert_eq!(tabs[0].permission_mode.as_deref(), Some("keep-mode"));
        assert_eq!(tabs[0].last_activity.as_deref(), Some("keep-activity"));
        assert!(tabs[0].workspace_override.is_some());

        // workspace 的 None 必须无条件清除覆盖。
        apply_refresh(&mut tabs, 1, None, None, None, None);
        assert!(tabs[0].workspace_override.is_none());
    }

    #[test]
    fn refresh_for_a_closed_tab_is_ignored() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut tabs = vec![tab(rt.handle())];
        apply_refresh(&mut tabs, 999, Some("x".into()), None, None, None);
        assert_eq!(tabs.len(), 1);
        assert!(tabs[0].llm_model.is_none());
    }

    #[test]
    fn card_model_covers_every_text_combination() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut t = tab(rt.handle());
        let ctx = CardCtx {
            titles: &Titles(None),
            project_branch: Some("main"),
            project_dirty: false,
        };
        t.llm_model = Some("claude-sonnet-5".into());
        t.permission_mode = Some("plan".into());
        let m = card_model(&t, true, &ctx, "en");
        assert_eq!(m.title, "claude (Sonnet 5, plan)");
        assert_eq!(m.work_line, "@main");
        assert!(m.active);

        t.permission_mode = None;
        assert_eq!(card_model(&t, false, &ctx, "en").title, "claude (Sonnet 5)");
        t.llm_model = None;
        t.permission_mode = Some("plan".into());
        assert_eq!(card_model(&t, false, &ctx, "en").title, "claude (plan)");
        t.permission_mode = None;
        assert_eq!(card_model(&t, false, &ctx, "en").title, "claude");

        assert_eq!(
            work_line(Some("task"), Some("main"), true),
            "task  @main(Uncommitted)"
        );
        assert_eq!(work_line(None, None, false), "@—");
        assert_eq!(work_line(Some("任务"), None, false), "任务  @—");
    }
}
