//! 卡片/页签/分组共用的样式助手(从 `dozer-app` 的 `workspace/{hook,view}.rs`
//! 搬入,S6c-2)。

use crate::i18n;
use crate::tabs::SessionTab;
use byteui::interaction::icons::IconKind;
use dozer_model::{AgentKind, AgentState};
use iced_widget::core::Color;

/// agent 四态文案(按 locale 渲染)。
pub fn agent_state_label(state: AgentState, locale: &str) -> String {
    let (key, fallback) = match state {
        AgentState::Running => ("agent.state_running", "Running"),
        AgentState::AwaitingInput => ("agent.state_awaiting_input", "Awaiting input"),
        AgentState::TurnEnded => ("agent.state_turn_ended", "Turn ended"),
        AgentState::Idle => ("agent.state_idle", "Idle"),
    };
    i18n::text(key, fallback, locale)
}

/// tab 前状态点配色:死会话灰;存活按 agent 状态——绿=空闲、青=运行、
/// 红=待输入、金=回合结束(金是甲方动作专属色:该出手了)。
pub fn dot_color(state: AgentState, alive: bool) -> Color {
    if !alive {
        return byteui::theme::color::current().dim;
    }
    match state {
        AgentState::Idle => byteui::theme::color::current().cyan,
        AgentState::Running => byteui::theme::color::current().green,
        AgentState::AwaitingInput => byteui::theme::color::current().red,
        AgentState::TurnEnded => byteui::theme::color::current().gold,
    }
}

/// `claude-sonnet-5` → `Sonnet 5`:去掉 `claude-` 前缀,按 `-` 分词、每词
/// 首字母大写、空格拼接;不以 `claude-` 开头的原样返回。
pub fn format_model_label(raw: &str) -> String {
    match raw.strip_prefix("claude-") {
        Some(rest) => rest
            .split('-')
            .map(|w| {
                let mut chars = w.chars();
                match chars.next() {
                    Some(f) => f.to_uppercase().collect::<String>() + chars.as_str(),
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
        None => raw.to_string(),
    }
}

/// agent → 对话列表圆点颜色(避开金,甲方动作专属色)。
pub fn agent_dot_color(agent: AgentKind) -> Color {
    let c = byteui::theme::color::current();
    match agent {
        AgentKind::Claude => c.cyan,
        AgentKind::Codebuddy => c.purple,
        AgentKind::Opencode => c.green,
        AgentKind::Codex => c.blue,
        AgentKind::Goose => c.teal,
        AgentKind::Aider => c.magenta,
        AgentKind::V8agent => c.lime,
        AgentKind::Qodercn => c.red,
        AgentKind::Unknown => c.dim,
    }
}

/// agent → 品牌图标。Goose/Qoder 按当前明暗主题选固定色剪影。
pub fn agent_icon(agent: AgentKind) -> IconKind {
    match agent {
        AgentKind::Claude => IconKind::Claude,
        AgentKind::Codebuddy => IconKind::Codebuddy,
        AgentKind::Opencode => IconKind::Opencode,
        AgentKind::Aider => IconKind::Aider,
        AgentKind::Codex => IconKind::Codex,
        AgentKind::Goose => match byteui::theme::color::current_scheme() {
            byteui::theme::color::ColorScheme::Light => IconKind::GooseLight,
            byteui::theme::color::ColorScheme::Dark => IconKind::GooseDark,
        },
        AgentKind::Qodercn => match byteui::theme::color::current_scheme() {
            byteui::theme::color::ColorScheme::Light => IconKind::QoderLight,
            byteui::theme::color::ColorScheme::Dark => IconKind::QoderDark,
        },
        AgentKind::V8agent | AgentKind::Unknown => IconKind::Bot,
    }
}

/// 按 `AgentKind` 把会话 tab 分组,固定顺序 Claude → … → Unknown,只返回
/// 非空分组;组内保持 `tabs` 原有顺序。返回下标而非引用,便于渲染层既发
/// `SelectTab(idx)` 又回查 `tabs[idx]`。
pub fn group_tabs_by_agent(tabs: &[SessionTab]) -> Vec<(AgentKind, Vec<usize>)> {
    const ORDER: [AgentKind; 9] = [
        AgentKind::Claude,
        AgentKind::Codebuddy,
        AgentKind::Opencode,
        AgentKind::Codex,
        AgentKind::Goose,
        AgentKind::Aider,
        AgentKind::V8agent,
        AgentKind::Qodercn,
        AgentKind::Unknown,
    ];
    ORDER
        .into_iter()
        .filter_map(|kind| {
            let idxs: Vec<usize> = tabs
                .iter()
                .enumerate()
                .filter(|(_, t)| t.agent == kind)
                .map(|(i, _)| i)
                .collect();
            (!idxs.is_empty()).then_some((kind, idxs))
        })
        .collect()
}
