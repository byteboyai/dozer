//! 选择器选择项与启动命令的纯逻辑(从 `dozer-app` 的 `workspace/hook.rs`、
//! `workspace/state.rs`、`workspace/view.rs` 搬入,S6c-1)。

use dozer_model::AgentKind;

/// Agent 面板"＋"弹出菜单选择项的语义。`Agent` 复用原 `Option<AgentKind>`
/// 语义(`None` = 纯 Shell);`Git` 在项目根开一个 shell 并自动跑 `git status`
/// 看仓库状态。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PickerLaunch {
    Agent(Option<AgentKind>),
    Git,
}

/// shell 单引号转义(`'` → `'\''`)。
pub fn sh_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// agent 选择菜单选中的 agent → 要自动键入 PTY 的 CLI 命令。`Unknown`
/// 不该从选择菜单产生,但函数保持穷尽 match,防止未来枚举新增变体时静默
/// 漏写。**不能**一律取 `AgentKind::label()`:
/// - Goose 的交互命令需要子命令(`goose session`,见 spec D1);
/// - Aider 走 `dozer-hook launch aider`(launcher bridge,见 spec D1),命令里
///   必须带 sibling `dozer-hook` 的绝对路径。
pub fn agent_launch_command(agent: AgentKind, hook_exe: &str) -> Option<String> {
    match agent {
        AgentKind::Unknown => None,
        AgentKind::Claude => Some("claude".into()),
        AgentKind::Codebuddy => Some("codebuddy".into()),
        AgentKind::Opencode => Some("opencode".into()),
        AgentKind::Codex => Some("codex".into()),
        AgentKind::Goose => Some("goose session".into()),
        AgentKind::Aider => Some(format!("{} launch aider", sh_single_quote(hook_exe))),
        AgentKind::V8agent => Some("v8agent".into()),
        AgentKind::Qodercn => Some("qodercn".into()),
    }
}

/// picker 选择项 → attach 成功后自动键入 PTY 的初始命令。`Agent(Some(a))`
/// 复用 [`agent_launch_command`](Aider 需要 `hook_exe` 绝对路径);`Agent(None)`
/// 不键入(纯 Shell);`Git` 键入 `git status`。
pub fn picker_launch_command(launch: PickerLaunch, hook_exe: &str) -> Option<String> {
    match launch {
        PickerLaunch::Agent(Some(agent)) => agent_launch_command(agent, hook_exe),
        PickerLaunch::Agent(None) => None,
        PickerLaunch::Git => Some("git status".to_string()),
    }
}

/// attach 时决定要不要回应 OSC 10/11 终端探测查询——`info.agent` 在新建
/// 会话这一刻几乎总还是 daemon 侧的默认值,真实 agent 要靠 hook 事后上报,
/// 而那必然晚于 agent CLI 启动瞬间就发出的查询;picker 选中的 `picked_agent`
/// 才是这一刻唯一"确定即将变成谁"的信号,`info.agent` 仅在 picker 未给出
/// 提示(如 SSH attach、或重连时 hook 已先一步上报过)时兜底。
pub fn should_answer_dynamic_color(picked_agent: Option<AgentKind>, info_agent: AgentKind) -> bool {
    picked_agent == Some(AgentKind::Opencode) || info_agent == AgentKind::Opencode
}

/// attach 时的初始 agent。daemon 里已有的识别结果优先;若首个 hook 尚未
/// 到达、`SessionInfo.agent` 仍为 `Unknown`,则使用 picker 已经明确选中的
/// 目标 agent。
pub fn attached_agent(picked_agent: Option<AgentKind>, info_agent: AgentKind) -> AgentKind {
    if info_agent != AgentKind::Unknown {
        info_agent
    } else {
        picked_agent.unwrap_or(AgentKind::Unknown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attached_agent_uses_picker_until_daemon_has_identified_session() {
        assert_eq!(
            attached_agent(Some(AgentKind::Codex), AgentKind::Unknown),
            AgentKind::Codex
        );
        assert_eq!(
            attached_agent(Some(AgentKind::Codex), AgentKind::Claude),
            AgentKind::Claude
        );
        assert_eq!(attached_agent(None, AgentKind::Unknown), AgentKind::Unknown);
    }

    #[test]
    fn should_answer_dynamic_color_true_when_picker_targets_opencode_even_if_info_agent_lags() {
        assert!(should_answer_dynamic_color(
            Some(AgentKind::Opencode),
            AgentKind::Unknown,
        ));
    }

    #[test]
    fn should_answer_dynamic_color_true_when_info_agent_already_known_opencode() {
        assert!(should_answer_dynamic_color(None, AgentKind::Opencode));
    }

    #[test]
    fn should_answer_dynamic_color_false_for_other_agents() {
        assert!(!should_answer_dynamic_color(
            Some(AgentKind::Claude),
            AgentKind::Unknown
        ));
        assert!(!should_answer_dynamic_color(None, AgentKind::Unknown));
        assert!(!should_answer_dynamic_color(None, AgentKind::Claude));
    }

    #[test]
    fn agent_launch_command_maps_known_agents_and_none_for_unknown() {
        let hook_exe = "/Applications/Dozer AI Coder.app/Contents/MacOS/dozer-hook";
        assert_eq!(
            agent_launch_command(AgentKind::Claude, hook_exe),
            Some("claude".into())
        );
        assert_eq!(
            agent_launch_command(AgentKind::Codebuddy, hook_exe),
            Some("codebuddy".into())
        );
        assert_eq!(
            agent_launch_command(AgentKind::Opencode, hook_exe),
            Some("opencode".into())
        );
        assert_eq!(
            agent_launch_command(AgentKind::Codex, hook_exe),
            Some("codex".into())
        );
        assert_eq!(
            agent_launch_command(AgentKind::Goose, hook_exe),
            Some("goose session".into())
        );
        assert_eq!(
            agent_launch_command(AgentKind::Aider, hook_exe),
            Some(
                "'/Applications/Dozer AI Coder.app/Contents/MacOS/dozer-hook' launch aider".into()
            )
        );
        assert_eq!(
            agent_launch_command(AgentKind::V8agent, hook_exe),
            Some("v8agent".into())
        );
        assert_eq!(
            agent_launch_command(AgentKind::Qodercn, hook_exe),
            Some("qodercn".into())
        );
        assert_eq!(agent_launch_command(AgentKind::Unknown, hook_exe), None);
    }

    #[test]
    fn agent_launch_command_aider_escapes_single_quotes_in_path() {
        let hook_exe = "/opt/weird'path/dozer-hook";
        let cmd = agent_launch_command(AgentKind::Aider, hook_exe).unwrap();
        assert_eq!(cmd, "'/opt/weird'\\''path/dozer-hook' launch aider");
    }

    #[test]
    fn picker_launch_command_maps_selection_to_initial_command() {
        assert_eq!(
            picker_launch_command(
                PickerLaunch::Agent(Some(AgentKind::Claude)),
                "/x/dozer-hook"
            ),
            Some("claude".to_string())
        );
        assert_eq!(
            picker_launch_command(
                PickerLaunch::Agent(Some(AgentKind::Codebuddy)),
                "/x/dozer-hook"
            ),
            Some("codebuddy".to_string())
        );
        assert_eq!(
            picker_launch_command(
                PickerLaunch::Agent(Some(AgentKind::Opencode)),
                "/x/dozer-hook"
            ),
            Some("opencode".to_string())
        );
        assert_eq!(
            picker_launch_command(PickerLaunch::Agent(Some(AgentKind::Codex)), "/x/dozer-hook"),
            Some("codex".to_string())
        );
        assert_eq!(
            picker_launch_command(PickerLaunch::Agent(Some(AgentKind::Goose)), "/x/dozer-hook"),
            Some("goose session".to_string())
        );
        assert_eq!(
            picker_launch_command(PickerLaunch::Agent(Some(AgentKind::Aider)), "/x/dozer-hook"),
            Some("'/x/dozer-hook' launch aider".to_string())
        );
        assert_eq!(
            picker_launch_command(
                PickerLaunch::Agent(Some(AgentKind::V8agent)),
                "/x/dozer-hook"
            ),
            Some("v8agent".to_string())
        );
        assert_eq!(
            picker_launch_command(PickerLaunch::Agent(None), "/x/dozer-hook"),
            None
        );
        assert_eq!(
            picker_launch_command(PickerLaunch::Git, "/x/dozer-hook"),
            Some("git status".to_string())
        );
    }

    /// S6c-1 新增:每个 AgentKind 都走一遍,不 panic;Unknown 无命令。
    #[test]
    fn picker_launch_command_covers_every_agent_kind_without_panicking() {
        for agent in AgentKind::ALL {
            let got = picker_launch_command(PickerLaunch::Agent(Some(agent)), "/x/dozer-hook");
            if agent == AgentKind::Unknown {
                assert_eq!(got, None);
            } else {
                assert!(got.is_some(), "{agent:?} 应有启动命令");
            }
        }
    }
}
