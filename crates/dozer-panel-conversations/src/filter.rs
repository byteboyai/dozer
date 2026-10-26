use dozer_model::{AgentKind, SessionRow};
pub const CONVERSATION_DETAIL_PAGE_SIZE: u32 = 200;
pub const CONVERSATION_PAGE_SIZE: usize = 20;

/// 按 `pages`(点过几次"更多",0 起)算出当前应该显示到第几条。抽成纯函数
/// 方便 headless 单测;视图只把返回值画出来。
pub fn conversation_visible_count(pages: usize) -> usize {
    pages
        .saturating_add(1)
        .saturating_mul(CONVERSATION_PAGE_SIZE)
}

/// 会话列表关键字 + agent 过滤:标题(`display_title`)大小写不敏感子串匹配
/// (空关键字不过滤标题这一维)叠加 agent 精确匹配(`None` = 不限)。
pub fn filter_sessions<'a>(
    rows: &'a [SessionRow],
    query: &str,
    agent: Option<AgentKind>,
) -> Vec<&'a SessionRow> {
    let needle = query.to_lowercase();
    rows.iter()
        .filter(|r| agent.map(|a| r.agent == a).unwrap_or(true))
        .filter(|r| {
            query.is_empty()
                || r.display_title.to_lowercase().contains(&needle)
                || r.summary
                    .as_deref()
                    .is_some_and(|s| s.to_lowercase().contains(&needle))
        })
        .collect()
}

/// 会话列表里出现过的 agent 种类,去重,固定展示顺序。供底部 footbar 画筛选
/// chip;返回空 = 没有会话数据,footbar 不渲染。
pub fn conversation_agents_present(rows: &[SessionRow]) -> Vec<AgentKind> {
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
        .filter(|k| rows.iter().any(|r| r.agent == *k))
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    fn session_row(title: &str, agent: AgentKind) -> SessionRow {
        SessionRow {
            conversation_id: title.to_string(),
            agent,
            last_ts: 0,
            display_title: title.to_string(),
            summary: None,
            summary_status: None,
            task_id: None,
        }
    }
    #[test]
    fn filter_sessions_matches_title_case_insensitive() {
        let row = SessionRow {
            conversation_id: "c1".into(),
            agent: AgentKind::Claude,
            last_ts: 0,
            display_title: "Fix Login Bug".into(),
            summary: None,
            summary_status: None,
            task_id: None,
        };
        let rows = vec![row];
        assert_eq!(filter_sessions(&rows, "login", None).len(), 1);
        assert_eq!(filter_sessions(&rows, "nomatch", None).len(), 0);
    }
    #[test]
    fn conversation_visible_count_starts_at_one_page() {
        assert_eq!(conversation_visible_count(0), CONVERSATION_PAGE_SIZE);
    }
    #[test]
    fn conversation_visible_count_grows_by_page_size() {
        assert_eq!(conversation_visible_count(1), 2 * CONVERSATION_PAGE_SIZE);
        assert_eq!(conversation_visible_count(2), 3 * CONVERSATION_PAGE_SIZE);
    }
    #[test]
    fn filter_sessions_empty_query_and_no_agent_returns_all() {
        let rows = vec![
            session_row("修复登录 bug", AgentKind::Claude),
            session_row("重构解析器", AgentKind::Codebuddy),
        ];
        assert_eq!(filter_sessions(&rows, "", None).len(), 2);
    }
    #[test]
    fn filter_sessions_matches_title_case_insensitive_substring() {
        let rows = vec![
            session_row("Fix Login Bug", AgentKind::Claude),
            session_row("重构解析器", AgentKind::Codebuddy),
        ];
        let filtered = filter_sessions(&rows, "login", None);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].display_title, "Fix Login Bug");
    }
    #[test]
    fn filter_sessions_matches_summary_case_insensitive_substring() {
        let mut with_summary = session_row("修复登录 bug", AgentKind::Claude);
        with_summary.summary = Some("Rewrote the OAuth callback handler".to_string());
        let rows = vec![
            with_summary,
            session_row("重构解析器", AgentKind::Codebuddy),
        ];
        let filtered = filter_sessions(&rows, "oauth", None);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].display_title, "修复登录 bug");
    }
    #[test]
    fn filter_sessions_no_summary_does_not_match_on_summary_query() {
        let rows = vec![session_row("修复登录 bug", AgentKind::Claude)];
        assert!(filter_sessions(&rows, "oauth", None).is_empty());
    }
    #[test]
    fn filter_sessions_filters_by_agent() {
        let rows = vec![
            session_row("会话 A", AgentKind::Claude),
            session_row("会话 B", AgentKind::Codebuddy),
        ];
        let filtered = filter_sessions(&rows, "", Some(AgentKind::Codebuddy));
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].display_title, "会话 B");
    }
    #[test]
    fn filter_sessions_combines_query_and_agent() {
        let rows = vec![
            session_row("修复登录 bug", AgentKind::Claude),
            session_row("修复登录 bug", AgentKind::Codebuddy),
        ];
        let filtered = filter_sessions(&rows, "登录", Some(AgentKind::Claude));
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].agent, AgentKind::Claude);
    }
    #[test]
    fn filter_sessions_no_match_yields_empty() {
        let rows = vec![session_row("会话 A", AgentKind::Claude)];
        assert!(filter_sessions(&rows, "不存在", None).is_empty());
    }
    #[test]
    fn conversation_agents_present_dedups_and_orders_stably() {
        let rows = vec![
            session_row("a", AgentKind::Opencode),
            session_row("b", AgentKind::Claude),
            session_row("c", AgentKind::Claude),
        ];
        assert_eq!(
            conversation_agents_present(&rows),
            vec![AgentKind::Claude, AgentKind::Opencode]
        );
    }
    #[test]
    fn conversation_agents_present_empty_for_no_rows() {
        assert!(conversation_agents_present(&[]).is_empty());
    }
}
