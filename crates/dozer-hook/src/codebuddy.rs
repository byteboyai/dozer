//! CodeBuddy 原生 hook 事件名 → dozerd 规范事件名翻译（spec §5.2）。
//! 规范词汇表就是 dozerd::agent_state_for 认识的 7 个 Claude 事件名，
//! 这里只做翻译，不引入新词汇。

/// `None` = 这个事件不转发给 dozerd（子 agent 生命周期，不影响顶层四态机）。
/// 未知事件原样透传——翻译层不对"没见过的事件"做任何假设，交给 dozerd
/// 那边的 `agent_state_for` 自己因为认不出而不改状态。
pub fn translate_event(raw: &str) -> Option<String> {
    match raw {
        "SessionStart" => Some("SessionStart"),
        "UserPromptSubmit" => Some("UserPromptSubmit"),
        "PreToolUse" => Some("PreToolUse"),
        "PostToolUse" | "PostToolUseFailure" => Some("PostToolUse"),
        "Notification" => Some("Notification"),
        "Stop" | "StopFailure" => Some("Stop"),
        "SessionEnd" => Some("SessionEnd"),
        "SubagentStart" | "SubagentStop" => return None,
        other => Some(other),
    }
    .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passthrough_events_map_to_same_name() {
        assert_eq!(translate_event("SessionStart"), Some("SessionStart".to_string()));
        assert_eq!(translate_event("UserPromptSubmit"), Some("UserPromptSubmit".to_string()));
        assert_eq!(translate_event("PreToolUse"), Some("PreToolUse".to_string()));
        assert_eq!(translate_event("PostToolUse"), Some("PostToolUse".to_string()));
        assert_eq!(translate_event("Notification"), Some("Notification".to_string()));
        assert_eq!(translate_event("SessionEnd"), Some("SessionEnd".to_string()));
    }

    #[test]
    fn failure_variants_merge_into_success_variant() {
        assert_eq!(translate_event("PostToolUseFailure"), Some("PostToolUse".to_string()));
        assert_eq!(translate_event("StopFailure"), Some("Stop".to_string()));
        assert_eq!(translate_event("Stop"), Some("Stop".to_string()));
    }

    #[test]
    fn subagent_events_are_dropped() {
        assert_eq!(translate_event("SubagentStart"), None);
        assert_eq!(translate_event("SubagentStop"), None);
    }

    #[test]
    fn unknown_event_passes_through_unmapped() {
        // 未知事件原样透传给 dozerd 的 agent_state_for，那边自己会因为
        // 认不出而不改状态（spec §3：翻译层对未知事件不 panic、不拦截）。
        assert_eq!(translate_event("SomeFutureEvent"), Some("SomeFutureEvent".to_string()));
    }
}
