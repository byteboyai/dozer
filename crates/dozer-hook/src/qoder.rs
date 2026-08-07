//! Qoder 原生 hook 事件名 → dozerd 规范事件名翻译（spec §6）。规范词汇表
//! 就是 dozerd::agent_state_for 认识的 7 个 Claude 事件名。Qoder 的事件
//! 名词汇（20 个，跨会话/工具/agent flow/压缩/通知/文件六大类）经官方
//! 文档核实（`docs.qoder.com/en/cli/hooks`），字段名与 Claude 逐字对齐。
//! Task 7 spike 若发现实测偏差，回来改这张表即可，不影响其余逻辑。

/// `None` = 这个事件不转发给 dozerd（压缩/子 agent/文件变更/worktree/MCP
/// elicitation 等不影响顶层四态机的事件）。未知事件原样透传。
pub fn translate_event(raw: &str) -> Option<String> {
    match raw {
        "SessionStart" => Some("SessionStart"),
        "SessionEnd" => Some("SessionEnd"),
        "UserPromptSubmit" => Some("UserPromptSubmit"),
        "PreToolUse" => Some("PreToolUse"),
        "PostToolUse" | "PostToolUseFailure" => Some("PostToolUse"),
        "Stop" | "StopFailure" => Some("Stop"),
        "Notification" | "PermissionRequest" | "PermissionDenied" => Some("Notification"),
        "SubagentStart" | "SubagentStop" | "PreCompact" | "PostCompact" | "InstructionsLoaded"
        | "ConfigChange" | "CwdChanged" | "FileChanged" | "WorktreeCreate" | "WorktreeRemove"
        | "Elicitation" | "ElicitationResult" => return None,
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
            translate_event("SessionEnd"),
            Some("SessionEnd".to_string())
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
            translate_event("Notification"),
            Some("Notification".to_string())
        );
    }

    #[test]
    fn failure_variants_merge_into_success_variant() {
        assert_eq!(
            translate_event("PostToolUseFailure"),
            Some("PostToolUse".to_string())
        );
        assert_eq!(translate_event("StopFailure"), Some("Stop".to_string()));
    }

    #[test]
    fn permission_events_merge_into_notification() {
        assert_eq!(
            translate_event("PermissionRequest"),
            Some("Notification".to_string())
        );
        assert_eq!(
            translate_event("PermissionDenied"),
            Some("Notification".to_string())
        );
    }

    #[test]
    fn non_state_machine_events_are_dropped() {
        for ev in [
            "SubagentStart",
            "SubagentStop",
            "PreCompact",
            "PostCompact",
            "InstructionsLoaded",
            "ConfigChange",
            "CwdChanged",
            "FileChanged",
            "WorktreeCreate",
            "WorktreeRemove",
            "Elicitation",
            "ElicitationResult",
        ] {
            assert_eq!(translate_event(ev), None, "{ev}");
        }
    }

    #[test]
    fn unknown_event_passes_through_unmapped() {
        assert_eq!(
            translate_event("SomeFutureEvent"),
            Some("SomeFutureEvent".to_string())
        );
    }
}
