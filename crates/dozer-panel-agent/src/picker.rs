//! Agent 选择器内容与关页签确认框内容(S6c-2)。

use crate::i18n;
use crate::launch::PickerLaunch;
use crate::state::AgentPanelState;
use crate::style::{agent_dot_color, agent_icon};
use crate::update::Message;
use byteui::feedback::dialog::ConfirmDialog;
use byteui::interaction::icons::IconKind;
use dozer_model::AgentKind;
use iced_widget::core::Color;

/// 选择器一行。
#[derive(Debug, Clone, PartialEq)]
pub enum PickerEntry {
    Launch {
        launch: PickerLaunch,
        label: String,
        icon: IconKind,
        icon_color: Color,
    },
    Separator,
}

/// `AgentKind::known()` 派生、按展示名不分大小写排序,分隔线后固定
/// `Git Shell`、`OS Shell`。`Unknown` 不出现。
pub fn picker_entries() -> Vec<PickerEntry> {
    let mut agents: Vec<(&'static str, AgentKind)> =
        AgentKind::known().map(|k| (k.display_label(), k)).collect();
    agents.sort_by_key(|(label, _)| label.to_lowercase());
    let mut out: Vec<PickerEntry> = agents
        .into_iter()
        .map(|(label, kind)| PickerEntry::Launch {
            launch: PickerLaunch::Agent(Some(kind)),
            label: label.to_string(),
            icon: agent_icon(kind),
            icon_color: agent_dot_color(kind),
        })
        .collect();
    out.push(PickerEntry::Separator);
    for (label, launch, icon) in [
        ("Git Shell", PickerLaunch::Git, IconKind::GitBranch),
        (
            "OS Shell",
            PickerLaunch::Agent(None),
            IconKind::SquareTerminal,
        ),
    ] {
        out.push(PickerEntry::Launch {
            launch,
            label: label.to_string(),
            icon,
            icon_color: byteui::theme::color::current().body,
        });
    }
    out
}

/// 关 Agent tab 的确认框内容。`None` 表示当前没有待确认的关闭;目标会话
/// 已不存在时 agent 名回落 `agent`。
pub fn close_confirm_spec<M: Clone>(
    state: &AgentPanelState,
    locale: &str,
    wrap: impl Fn(Message) -> M,
) -> Option<ConfirmDialog<M>> {
    let id = state.pending_close_tab.as_ref()?;
    let agent_label = state
        .tabs
        .iter()
        .find(|t| &t.info.id == id)
        .map(|t| t.agent.label().to_string())
        .unwrap_or_else(|| "agent".to_string());
    Some(ConfirmDialog {
        icon: Some(IconKind::CircleAlert),
        title: i18n::text_args(
            "agent.close_tab_title",
            "Close the \"{agent}\" Agent?",
            locale,
            &[("agent", agent_label.as_str())],
        ),
        description: i18n::text_args(
            "agent.close_tab_description",
            "The \"{agent}\" Agent is currently active. Closing it ends the session and clears its context.",
            locale,
            &[("agent", agent_label.as_str())],
        ),
        cancel_label: i18n::text("close_confirm.cancel", "Cancel", locale),
        cancel_msg: wrap(Message::CloseCancel),
        confirm_label: i18n::text("close_confirm.confirm", "Close", locale),
        confirm_msg: wrap(Message::CloseConfirm),
        close_msg: Some(wrap(Message::CloseCancel)),
        confirm_color: byteui::theme::color::current().red,
        content_spacing: 8.0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tabs::{SessionTab, TabBackend};
    use dozer_model::{AgentKind, AgentState, SessionInfo};
    use dozer_panel_term::model::TerminalModel;
    use dozer_panel_term::osc::OscScanner;

    #[test]
    fn picker_entries_list_every_known_agent_once_sorted_then_a_separator_then_two_shells() {
        let entries = picker_entries();
        assert_eq!(
            entries.last_chunk::<3>().map(|c| c.to_vec()),
            Some(vec![
                PickerEntry::Separator,
                PickerEntry::Launch {
                    launch: PickerLaunch::Git,
                    label: "Git Shell".into(),
                    icon: IconKind::GitBranch,
                    icon_color: byteui::theme::color::current().body,
                },
                PickerEntry::Launch {
                    launch: PickerLaunch::Agent(None),
                    label: "OS Shell".into(),
                    icon: IconKind::SquareTerminal,
                    icon_color: byteui::theme::color::current().body,
                },
            ])
        );
        let mut labels: Vec<String> = entries
            .iter()
            .filter_map(|e| match e {
                PickerEntry::Launch { label, launch, .. } => match launch {
                    PickerLaunch::Agent(Some(kind)) if *kind != AgentKind::Unknown => {
                        Some(label.clone())
                    }
                    _ => None,
                },
                PickerEntry::Separator => None,
            })
            .collect();
        let expected_len = AgentKind::known().count();
        assert_eq!(labels.len(), expected_len, "{labels:?}");
        let mut sorted = labels.clone();
        sorted.sort_by_key(|l| l.to_lowercase());
        assert_eq!(labels, sorted);
        assert!(!labels.iter().any(|l| l.eq_ignore_ascii_case("unknown")));
        labels.dedup();
        assert_eq!(labels.len(), expected_len);
    }

    #[test]
    fn close_confirm_is_none_without_a_pending_close() {
        let state = AgentPanelState::new();
        assert!(close_confirm_spec(&state, "en", |m| m).is_none());
    }

    #[test]
    fn close_confirm_falls_back_to_a_generic_label_when_the_session_is_gone() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut state = AgentPanelState::new();
        state.pending_close_tab = Some("missing".into());
        let d = close_confirm_spec(&state, "en", |m| m).expect("dialog");
        assert!(d.title.contains("\"agent\""), "{:?}", d.title);
        assert_eq!(d.cancel_label, "Cancel");
        assert_eq!(d.confirm_label, "Close");

        // 存在的会话用真实 agent 名。
        state.tabs.push(SessionTab {
            info: SessionInfo {
                id: "s1".into(),
                name: "s".into(),
                command: "sh".into(),
                cwd: "/tmp".into(),
                alive: true,
                created_ms: 0,
                agent_state: AgentState::Running,
                transcript_path: None,
                project_id: Some(7),
                agent: AgentKind::Claude,
            },
            model: TerminalModel::new(10, 4),
            alive: true,
            agent_state: AgentState::Running,
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
            forwarder: rt.spawn(async {}),
            backend: TabBackend::Daemon,
        });
        state.pending_close_tab = Some("s1".into());
        let d = close_confirm_spec(&state, "en", |m| m).expect("dialog");
        assert!(d.title.contains("\"claude\""), "{:?}", d.title);
    }
}
