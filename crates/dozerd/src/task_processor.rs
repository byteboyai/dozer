//! 单个任务的 headless 处理编排:铸造/复用会话、拼历史、调
//! `headless_agent::process_task_headless`、落回合。轮询器
//! (`task_poller.rs`)和"处理"按钮的 `ProcessTodoNow` handler 共用这个
//! 模块,避免两条触发路径各写一份逻辑(spec 2026-09-02)。

use crate::projects::ProjectStore;
use crate::session_summary::SessionSummaryStore;
use crate::todo::TodoStore;
use crate::transcripts::TranscriptStore;
use dozer_core::protocol::{TodoInfo, TurnRecord};

dozer_core::scope!(LOG, module, "task");

/// 待处理判定:不新增标记字段,直接看该任务关联会话最新一条回合的
/// `role`。没有任何回合(刚指派/从未处理过)、或最新一条是 `"human"`
/// (人类刚回复,或上一次处理失败没能写入 ai 回合)→ 待处理;最新一条是
/// `"ai"` → 不需要处理。
pub fn needs_processing(turns: &[TurnRecord]) -> bool {
    match turns.last() {
        None => true,
        Some(t) => t.role != "ai",
    }
}

/// 处理一个任务:解析项目目录 → 铸造/复用会话 id → 拼历史 → 调
/// `process_task_headless` → 落回合。成功/失败(spawn 失败/超时/不支持的
/// agent 种类)都会写一条 `ai` 回合(失败时内容是可读错误描述),不静默
/// 丢弃、不在本函数内重试——重试节奏由调用方(轮询器的下一轮 tick,或
/// 人类再次点"处理")决定。
pub async fn process_task(
    todos: &TodoStore,
    _session_summaries: &SessionSummaryStore,
    transcripts: &TranscriptStore,
    projects: &ProjectStore,
    todo: &TodoInfo,
    human_reply: Option<&str>,
    summary_service: Option<&crate::summary_service::SummaryService>,
) -> Result<(), String> {
    let Some(agent) = todo.assigned_agent else {
        return Err("任务未指派 agent".into());
    };
    let project = projects
        .list()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|p| p.id == todo.project_id)
        .ok_or_else(|| "找不到该任务所属的项目".to_string())?;

    let session_id = match &todo.dispatch_session_id {
        Some(id) => id.clone(),
        None => {
            let minted = format!("task-{}-{}", todo.id, now_ms());
            todos
                .set_dispatch_session(todo.id, &minted)
                .map_err(|e| e.to_string())?;
            // 不再创建"空摘要 = AiGenerated"的占位行(spec 2026-09-26 第 7
            // 节:Todo 创建时移除该语义)。task 关联由 `task_id` 字段独立保留,
            // 真正的总结在实际回合完成后走统一队列。
            minted
        }
    };

    let prior_turns = transcripts
        .get_conversation_turns(&session_id, -1, u32::MAX)
        .map_err(|e| e.to_string())?;
    let prior_turns_text = prior_turns
        .iter()
        .map(|t| {
            let label = if t.role == "human" { "人类" } else { "AI" };
            format!("{label}: {}", t.content)
        })
        .collect::<Vec<_>>()
        .join("\n");
    let human_instruction = human_reply.unwrap_or(&todo.text);

    let result = crate::headless_agent::process_task_headless(
        agent,
        std::path::Path::new(&project.path),
        &session_id,
        &todo.text,
        &prior_turns_text,
        human_instruction,
    )
    .await;

    let ai_content = match result {
        Ok(stdout) => stdout,
        Err(e) => format!("(headless 处理失败: {e:?})"),
    };
    transcripts
        .record_task_turns(
            &session_id,
            agent,
            &project.path,
            &todo.text,
            human_instruction,
            &ai_content,
        )
        .map_err(|e| e.to_string())?;
    if let Some(service) = summary_service
        && let crate::summary_config::SummaryProviderResolution::Configured(config, _) =
            crate::summary_config::resolve_provider(None, None)
        && let Err(e) = service.submit_single(&crate::summary_service::SubmitSpec {
            conversation_id: session_id,
            source_session_id: None,
            trigger: dozer_core::protocol::SummaryTrigger::NaturalExit,
            provider: config.provider,
            requested_model: config.model,
            force: false,
        })
    {
        dozer_core::log_warn!(LOG, error = %e, todo_id = todo.id, "Todo 完成后提交总结任务失败");
    }
    Ok(())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(role: &str) -> TurnRecord {
        TurnRecord {
            turn_index: 0,
            role: role.into(),
            content: "x".into(),
            tool_calls: vec![],
            thinking: false,
            thinking_text: None,
            ts: Some(1),
            is_error: false,
            tool_result_call_id: None,
            tokens_in: 0,
            tokens_out: 0,
            tokens_cache_read: 0,
            tokens_cache_write: 0,
        }
    }

    #[test]
    fn empty_turns_need_processing() {
        assert!(needs_processing(&[]));
    }

    #[test]
    fn latest_human_turn_needs_processing() {
        assert!(needs_processing(&[turn("ai"), turn("human")]));
    }

    #[test]
    fn latest_ai_turn_does_not_need_processing() {
        assert!(!needs_processing(&[turn("human"), turn("ai")]));
    }

    #[tokio::test]
    async fn process_task_errors_without_assigned_agent() {
        let tmp = tempfile::tempdir().unwrap();
        let todos = TodoStore::new(&tmp.path().join("t.db")).unwrap();
        let session_summaries = SessionSummaryStore::open(&tmp.path().join("t.db")).unwrap();
        let transcripts = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
        let projects = ProjectStore::new(&tmp.path().join("t.db")).unwrap();
        let todo = todos.add(1, "任务").unwrap();
        let err = process_task(
            &todos,
            &session_summaries,
            &transcripts,
            &projects,
            &todo,
            None,
            None,
        )
        .await
        .unwrap_err();
        assert!(err.contains("未指派"));
    }

    #[tokio::test]
    async fn process_task_errors_when_project_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let todos = TodoStore::new(&tmp.path().join("t.db")).unwrap();
        let session_summaries = SessionSummaryStore::open(&tmp.path().join("t.db")).unwrap();
        let transcripts = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
        let projects = ProjectStore::new(&tmp.path().join("t.db")).unwrap();
        let todo = todos.add(999, "任务").unwrap();
        let assigned = todos
            .assign_agent(todo.id, dozer_core::protocol::AgentKind::Claude)
            .unwrap();
        let err = process_task(
            &todos,
            &session_summaries,
            &transcripts,
            &projects,
            &assigned,
            None,
            None,
        )
        .await
        .unwrap_err();
        assert!(err.contains("项目"));
    }
}
