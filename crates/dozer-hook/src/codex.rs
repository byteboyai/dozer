//! Codex 原生 hook 事件名 → dozerd 规范事件名翻译（spec §5）。规范词汇表
//! 就是 dozerd::agent_state_for 认识的 7 个 Claude 事件名，这里只做翻译，
//! 不引入新词汇。事件名词汇经官方文档核实（`developers.openai.com/codex/
//! hooks`），与 Claude Code 高度对齐；Task 4 spike 若发现实测偏差，回来
//! 改这张表即可，不影响其余逻辑。

/// `None` = 这个事件不转发给 dozerd（压缩/子 agent 生命周期，不影响顶层
/// 四态机）。未知事件原样透传——翻译层不对"没见过的事件"做任何假设，
/// 交给 dozerd 那边的 `agent_state_for` 自己因为认不出而不改状态。
pub fn translate_event(raw: &str) -> Option<String> {
    match raw {
        "SessionStart" => Some("SessionStart"),
        "UserPromptSubmit" => Some("UserPromptSubmit"),
        "PreToolUse" => Some("PreToolUse"),
        "PostToolUse" => Some("PostToolUse"),
        "PermissionRequest" => Some("Notification"),
        "Stop" => Some("Stop"),
        "SessionEnd" => Some("SessionEnd"),
        "PreCompact" | "PostCompact" | "SubagentStart" | "SubagentStop" => return None,
        other => Some(other),
    }
    .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passthrough_events_map_to_same_name() {
        assert_eq!(
            translate_event("SessionStart"),
            Some("SessionStart".to_string())
        );
        assert_eq!(
            translate_event("UserPromptSubmit"),
            Some("UserPromptSubmit".to_string())
        );
        assert_eq!(
            translate_event("PreToolUse"),
            Some("PreToolUse".to_string())
        );
        assert_eq!(
            translate_event("PostToolUse"),
            Some("PostToolUse".to_string())
        );
        assert_eq!(translate_event("Stop"), Some("Stop".to_string()));
        assert_eq!(
            translate_event("SessionEnd"),
            Some("SessionEnd".to_string())
        );
    }

    #[test]
    fn permission_request_merges_into_notification() {
        assert_eq!(
            translate_event("PermissionRequest"),
            Some("Notification".to_string())
        );
    }

    #[test]
    fn compaction_and_subagent_events_are_dropped() {
        assert_eq!(translate_event("PreCompact"), None);
        assert_eq!(translate_event("PostCompact"), None);
        assert_eq!(translate_event("SubagentStart"), None);
        assert_eq!(translate_event("SubagentStop"), None);
    }

    #[test]
    fn unknown_event_passes_through_unmapped() {
        assert_eq!(
            translate_event("SomeFutureEvent"),
            Some("SomeFutureEvent".to_string())
        );
    }
}
