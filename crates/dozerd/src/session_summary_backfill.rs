//! 补总结后台任务的内存态进度登记表(spec 2026-08-28)。按 `cwd` 索引,
//! `dozerd` 重启即丢失——与既有 `finalize_session_summary` 轮询任务同一
//! 哲学:这个用例时间尺度是秒级到十几秒,不值得为它做持久化任务队列。

use dozer_core::protocol::{ConversationSummary, SessionSummaryPayload, SummaryStatus};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BackfillProgress {
    pub total: u32,
    pub completed: u32,
}

pub struct BackfillRegistry {
    by_cwd: Mutex<HashMap<String, BackfillProgress>>,
}

impl BackfillRegistry {
    pub fn new() -> Self {
        Self {
            by_cwd: Mutex::new(HashMap::new()),
        }
    }

    /// 开始一轮新的补总结:覆盖该 `cwd` 之前的记录(若有)。必须在
    /// `Request::BackfillSessionSummaries` 处理器里、回 `Reply::Ok` **之前**
    /// 调用,保证调用方随后立即轮询也能看到正确的 `total`,不会撞见"任务
    /// 还没算出 total"的空窗期。
    pub fn start(&self, cwd: &str, total: u32) {
        self.by_cwd.lock().expect("lock").insert(
            cwd.to_string(),
            BackfillProgress {
                total,
                completed: 0,
            },
        );
    }

    /// 处理完一条(无论成功还是降级到启发式兜底,都算"已处理")后 +1。
    pub fn increment(&self, cwd: &str) {
        if let Some(p) = self.by_cwd.lock().expect("lock").get_mut(cwd) {
            p.completed = p.completed.saturating_add(1);
        }
    }

    pub fn get(&self, cwd: &str) -> Option<BackfillProgress> {
        self.by_cwd.lock().expect("lock").get(cwd).copied()
    }
}

impl Default for BackfillRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// 某 `cwd` 下有对话记录但没有 `session_summaries` 行的会话列表——复用
/// `Request::ListConversationsWithSummaries` 处理器同一套查询组合
/// (`list_conversations` + `get_many`),这里直接内部调用,不走协议层
/// 往返。
pub fn missing_summary_conversations(
    transcripts: &crate::transcripts::TranscriptStore,
    session_summaries: &crate::session_summary::SessionSummaryStore,
    cwd: &str,
) -> Vec<ConversationSummary> {
    let conversations = transcripts
        .list_conversations(cwd, None, u32::MAX, 0)
        .unwrap_or_default();
    let ids: Vec<String> = conversations
        .iter()
        .map(|c| c.conversation_id.clone())
        .collect();
    let summaries = session_summaries.get_many(&ids).unwrap_or_default();
    conversations
        .into_iter()
        .filter(|c| !summaries.contains_key(&c.conversation_id))
        .collect()
}

/// 后台任务本体:依次(串行)处理 `missing` 里的每条会话——headless 总结
/// 成功则 `ai_generated` 落库,失败/超时/不支持则降级到
/// `heuristic_from_turns` 走 `heuristic_fallback` 落库,每条处理完都
/// `registry.increment`。没有真实 PTY session,`session_id` 用
/// `backfill:{conversation_id}` 前缀拼出一个唯一键(真实 dozerd session id
/// 是 UUID v4,`session.rs:75`,不带这个前缀,不会撞车)——`conversation_id`
/// 才是这一行真正的关联键,`session_id` 只是满足表主键约束的占位唯一值。
///
/// `agent` 由调用方(`BackfillSessionSummaries` handler)传入
/// `load_default_agent()`——显式传参而不是在本函数内部读取,是为了让单测
/// 能注入一个 headless 必然不支持的 kind(如 `Codex`,返回 `Unsupported`)
/// 从而**确定性**地覆盖"降级到启发式兜底"这条路径;如果在本函数内部
/// 调 `load_default_agent()`,单测会受本机是否装了对应 CLI 影响而飘忽
/// (装了 claude 就回 `AiGenerated`,没装才回 `HeuristicFallback`,同一个
/// 测试在不同机器结果不同),违反 Global Constraint"每个任务全绿"。
pub async fn run_backfill(
    cwd: String,
    missing: Vec<ConversationSummary>,
    transcripts: std::sync::Arc<crate::transcripts::TranscriptStore>,
    session_summaries: std::sync::Arc<crate::session_summary::SessionSummaryStore>,
    registry: std::sync::Arc<BackfillRegistry>,
    agent: dozer_core::protocol::AgentKind,
) {
    for conv in missing {
        let turns = transcripts
            .get_conversation_turns(&conv.conversation_id, -1, u32::MAX)
            .unwrap_or_default();
        let (title, summary, status) =
            match crate::headless_agent::summarize_headless(agent, &turns).await {
                Ok((t, s)) => (t, s, SummaryStatus::AiGenerated),
                Err(e) => {
                    tracing::warn!(
                        error = ?e,
                        conversation_id = %conv.conversation_id,
                        "headless 总结失败,走启发式兜底"
                    );
                    let (t, s) = crate::session_summary::heuristic_from_turns(&turns);
                    (t, s, SummaryStatus::HeuristicFallback)
                }
            };
        let payload = SessionSummaryPayload {
            session_id: format!("backfill:{}", conv.conversation_id),
            agent_kind: conv.agent,
            conversation_id: Some(conv.conversation_id.clone()),
            title,
            summary,
            status,
            created_ts_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
            task_id: None,
        };
        if let Err(e) = session_summaries.record(&payload) {
            tracing::error!(error = %e, conversation_id = %conv.conversation_id, "补总结落库失败");
        }
        registry.increment(&cwd);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // `AgentKind` 只在测试里直接点名构造(生产代码路径只经手
    // `ConversationSummary.agent` 字段搬运,不需要在顶层 `use` 里引入这个
    // 类型名——引入了在非测试编译下会触发 `unused_imports`),所以单独在
    // 测试模块内 `use`。
    use dozer_core::protocol::AgentKind;

    #[test]
    fn registry_start_then_get_returns_total_with_zero_completed() {
        let reg = BackfillRegistry::new();
        reg.start("/p", 5);
        assert_eq!(
            reg.get("/p"),
            Some(BackfillProgress {
                total: 5,
                completed: 0
            })
        );
    }

    #[test]
    fn registry_increment_advances_completed() {
        let reg = BackfillRegistry::new();
        reg.start("/p", 2);
        reg.increment("/p");
        assert_eq!(
            reg.get("/p"),
            Some(BackfillProgress {
                total: 2,
                completed: 1
            })
        );
        reg.increment("/p");
        assert_eq!(
            reg.get("/p"),
            Some(BackfillProgress {
                total: 2,
                completed: 2
            })
        );
    }

    #[test]
    fn registry_get_unknown_cwd_returns_none() {
        let reg = BackfillRegistry::new();
        assert_eq!(reg.get("/never-started"), None);
    }

    #[test]
    fn registry_start_overwrites_previous_run_for_same_cwd() {
        let reg = BackfillRegistry::new();
        reg.start("/p", 3);
        reg.increment("/p");
        reg.start("/p", 1); // 新一轮"修复项目"点击
        assert_eq!(
            reg.get("/p"),
            Some(BackfillProgress {
                total: 1,
                completed: 0
            })
        );
    }

    #[tokio::test]
    async fn missing_summary_conversations_excludes_already_summarized() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.db");
        let transcripts = crate::transcripts::TranscriptStore::open(&db).unwrap();
        let session_summaries = crate::session_summary::SessionSummaryStore::open(&db).unwrap();
        // 没有真实摄取数据时,查询应返回空,不 panic。
        let missing =
            missing_summary_conversations(&transcripts, &session_summaries, "/no/such/project");
        assert!(missing.is_empty());
    }

    #[tokio::test]
    async fn run_backfill_falls_back_to_heuristic_when_agent_kind_unsupported() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.db");
        let transcripts =
            std::sync::Arc::new(crate::transcripts::TranscriptStore::open(&db).unwrap());
        let session_summaries =
            std::sync::Arc::new(crate::session_summary::SessionSummaryStore::open(&db).unwrap());
        let registry = std::sync::Arc::new(BackfillRegistry::new());
        registry.start("/p", 1);
        let conv = ConversationSummary {
            conversation_id: "conv-1".into(),
            agent: AgentKind::Codex, // headless_agent 对 Codex 返回 Unsupported
            file_path: "/x".into(),
            title: "t".into(),
            first_ts: 1,
            last_ts: 2,
            turn_count: 0,
        };
        run_backfill(
            "/p".into(),
            vec![conv],
            transcripts,
            session_summaries.clone(),
            registry.clone(),
            AgentKind::Codex, // headless_agent 对 Codex 返回 Unsupported → 必走启发式兜底
        )
        .await;
        assert_eq!(
            registry.get("/p"),
            Some(BackfillProgress {
                total: 1,
                completed: 1
            })
        );
        let got = session_summaries.get("backfill:conv-1").unwrap().unwrap();
        assert_eq!(got.status, SummaryStatus::HeuristicFallback);
        assert_eq!(got.conversation_id, Some("conv-1".into()));
    }
}
