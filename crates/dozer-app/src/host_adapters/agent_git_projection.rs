//! Agent 卡片的异步 Git 投影:工作区快照顺序与 tab 身份必须一直保留到应用结果时。

use dozer_panel_agent::tabs::{SessionTab, WorkspaceGitInfo};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ProjectionVersion(u64);

#[derive(Debug, Default)]
pub(crate) struct ProjectionState {
    latest: u64,
}

/// 身份解析针对的是这一会话、这一 cwd,不能只保留可被重用的 tab_id。
#[derive(Debug, Clone)]
pub(crate) struct TabIdentity {
    tab_id: usize,
    session_id: String,
    pub(crate) cwd: PathBuf,
}

impl TabIdentity {
    pub(crate) fn capture(tab: &SessionTab) -> Self {
        Self {
            tab_id: tab.tab_id,
            session_id: tab.info.id.clone(),
            cwd: tab.effective_cwd(),
        }
    }

    fn matches(&self, tab: &SessionTab) -> bool {
        self.tab_id == tab.tab_id
            && self.session_id == tab.info.id
            && self.cwd == tab.effective_cwd()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Projection {
    pub(crate) project_id: i64,
    pub(crate) token: dozer_git_runtime::SubscriptionToken,
    pub(crate) version: ProjectionVersion,
    pub(crate) tabs: Vec<TabIdentity>,
    pub(crate) info: WorkspaceGitInfo,
}

impl ProjectionState {
    /// 每次接受工作区快照都推进版本,即使本次没有候选 tab,也撤销上批结果。
    pub(crate) fn begin(&mut self) -> ProjectionVersion {
        self.latest = self
            .latest
            .checked_add(1)
            .expect("projection version exhausted");
        ProjectionVersion(self.latest)
    }

    pub(crate) fn apply(
        &self,
        version: ProjectionVersion,
        identities: Vec<TabIdentity>,
        info: WorkspaceGitInfo,
        tabs: &mut [SessionTab],
    ) -> usize {
        if version != ProjectionVersion(self.latest) {
            return 0;
        }
        let mut applied = 0;
        for identity in identities {
            if let Some(tab) = tabs.iter_mut().find(|tab| identity.matches(tab)) {
                tab.workspace_override = Some(info.clone());
                applied += 1;
            }
        }
        applied
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dozer_model::{AgentKind, AgentState, SessionInfo};
    use dozer_panel_agent::tabs::TabBackend;

    fn tab(handle: &tokio::runtime::Handle, id: usize, session: &str) -> SessionTab {
        SessionTab {
            info: SessionInfo {
                id: session.into(),
                name: "shell".into(),
                command: "sh".into(),
                cwd: "/repo".into(),
                alive: true,
                created_ms: 0,
                agent_state: AgentState::Idle,
                transcript_path: None,
                project_id: Some(1),
                agent: AgentKind::Claude,
            },
            model: crate::term::term_model::TerminalModel::new(20, 4),
            alive: true,
            agent_state: AgentState::Idle,
            agent: AgentKind::Claude,
            transcript_path: None,
            osc: crate::osc::OscScanner::new(),
            cwd: None,
            last_exit: None,
            llm_model: None,
            permission_mode: None,
            last_activity: None,
            workspace_override: None,
            tab_id: id,
            forwarder: handle.spawn(async {}),
            backend: TabBackend::Daemon,
        }
    }

    fn info(branch: &str, dirty: bool) -> WorkspaceGitInfo {
        WorkspaceGitInfo {
            branch: Some(branch.into()),
            dirty,
        }
    }

    #[test]
    fn late_old_snapshot_cannot_overwrite_new_projection() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut tabs = vec![tab(rt.handle(), 1, "s1")];
        let identity = TabIdentity::capture(&tabs[0]);
        let mut state = ProjectionState::default();
        let old = state.begin();
        let new = state.begin();
        assert_eq!(
            state.apply(new, vec![identity.clone()], info("new", false), &mut tabs),
            1
        );
        assert_eq!(
            state.apply(old, vec![identity], info("old", true), &mut tabs),
            0
        );
        assert_eq!(tabs[0].workspace_override, Some(info("new", false)));
    }

    #[test]
    fn changed_cwd_rejects_a_previously_matching_workspace() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut tabs = vec![tab(rt.handle(), 1, "s1")];
        let identity = TabIdentity::capture(&tabs[0]);
        let mut state = ProjectionState::default();
        let version = state.begin();
        tabs[0].cwd = Some("/repo/.worktrees/other".into());
        tabs[0].workspace_override = Some(info("other", false));
        assert_eq!(
            state.apply(version, vec![identity], info("parent", true), &mut tabs),
            0
        );
        assert_eq!(tabs[0].workspace_override, Some(info("other", false)));
    }

    #[test]
    fn reopened_session_cannot_reuse_an_old_tabs_projection() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let old = tab(rt.handle(), 1, "old-session");
        let identity = TabIdentity::capture(&old);
        drop(old);
        let mut tabs = vec![tab(rt.handle(), 1, "new-session")];
        let mut state = ProjectionState::default();
        let version = state.begin();
        assert_eq!(
            state.apply(version, vec![identity], info("old", true), &mut tabs),
            0
        );
        assert!(tabs[0].workspace_override.is_none());
    }

    #[test]
    fn reordering_tabs_preserves_identity_and_closed_tabs_are_ignored() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut tabs = vec![tab(rt.handle(), 1, "s1"), tab(rt.handle(), 2, "s2")];
        let identity = TabIdentity::capture(&tabs[0]);
        let closed = TabIdentity::capture(&tabs[1]);
        tabs.swap(0, 1);
        tabs.remove(0);
        let mut state = ProjectionState::default();
        let version = state.begin();
        assert_eq!(
            state.apply(
                version,
                vec![identity, closed],
                info("main", false),
                &mut tabs
            ),
            1
        );
        assert_eq!(tabs[0].info.id, "s1");
        assert_eq!(tabs[0].workspace_override, Some(info("main", false)));
    }

    #[test]
    fn snapshot_without_candidates_still_invalidates_older_work() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut tabs = vec![tab(rt.handle(), 1, "s1")];
        let identity = TabIdentity::capture(&tabs[0]);
        let mut state = ProjectionState::default();
        let old = state.begin();
        let _no_candidates = state.begin();
        assert_eq!(
            state.apply(old, vec![identity], info("old", true), &mut tabs),
            0
        );
        assert!(tabs[0].workspace_override.is_none());
    }
}
