//! 单个任务的 headless 处理编排:铸造/复用会话、拼历史、调
//! `headless_agent::process_task_headless`、落回合。轮询器
//! (`task_poller.rs`)和"处理"按钮的 `ProcessTodoNow` handler 共用这个
//! 模块,避免两条触发路径各写一份逻辑(spec 2026-09-02)。

use crate::projects::ProjectStore;
use crate::session_summary::SessionSummaryStore;
use crate::task_execution::{
    Admission, AdmissionError, AttemptContext, RunningAttempt, TaskExecutionStore,
};
use crate::todo::TodoStore;
use crate::transcripts::TranscriptStore;
use dozer_core::protocol::{TaskExecutionInfo, TaskExecutionStatus, TodoInfo, TurnRecord};
use std::sync::Arc;

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

/// 只凭最新一条 role 的轻量判定(轮询器不拉完整轨迹)。
pub fn needs_processing_role(latest_role: Option<&str>) -> bool {
    match latest_role {
        None => true,
        Some(role) => role != "ai",
    }
}

/// 自动轮询是否允许重跑。Failed/Interrupted 只有在**确实出现新的人类输入**
/// (最新 human 回合索引大于 attempt 记录的输入代次)时才允许;Running/Queued
/// 永不自动重跑。旧历史没有 attempt 时沿用 `needs_processing` 的旧规则。
pub fn auto_process_allowed(latest: Option<&TaskExecutionInfo>, latest_human: i64) -> bool {
    match latest {
        Some(attempt) => match attempt.status {
            TaskExecutionStatus::Queued | TaskExecutionStatus::Running => false,
            TaskExecutionStatus::Failed | TaskExecutionStatus::Interrupted => {
                latest_human > attempt.input_generation
            }
            TaskExecutionStatus::Succeeded => true,
        },
        None => true,
    }
}

fn input_fingerprint(todo_text: &str, human_reply: Option<&str>) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in todo_text
        .as_bytes()
        .iter()
        .chain(std::iter::once(&0u8))
        .chain(human_reply.unwrap_or("").as_bytes())
    {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// 解析项目/agent/会话并准入一次执行。异步启动路径先调用它,拿到 attempt
/// 后立即返回,再由后台任务调用 `run_attempt`。
#[allow(clippy::too_many_arguments)]
pub fn start_task(
    executions: &Arc<TaskExecutionStore>,
    todos: &TodoStore,
    projects: &ProjectStore,
    todo: &TodoInfo,
    human_reply: Option<&str>,
    idempotency_key: Option<&str>,
) -> Result<Admission, AdmissionError> {
    let agent = todo
        .assigned_agent
        .ok_or_else(|| AdmissionError::Other(anyhow::anyhow!("任务未指派 agent")))?;
    let project_dir = projects
        .path_of(todo.project_id)
        .map_err(|e| AdmissionError::Other(anyhow::anyhow!(e.to_string())))?
        .ok_or_else(|| AdmissionError::Other(anyhow::anyhow!("找不到该任务所属的项目")))?;
    let session_id = match &todo.dispatch_session_id {
        Some(id) => id.clone(),
        None => {
            let minted = format!("task-{}-{}", todo.id, now_ms());
            todos
                .set_dispatch_session(todo.id, &minted)
                .map_err(|e| AdmissionError::Other(anyhow::anyhow!(e.to_string())))?;
            minted
        }
    };
    let context = AttemptContext {
        conversation_id: session_id,
        agent,
        project_dir,
        task_title: todo.text.clone(),
    };
    executions.admit(
        todo.id,
        &input_fingerprint(&todo.text, human_reply),
        idempotency_key,
        context,
    )
}

/// 执行一个已准入的 attempt:先持久化人类输入,再调 headless;成功/失败都
/// 通过 attempt 的原子事务落库。失败不会被包装成普通成功 AI 回合。
pub async fn run_attempt(
    transcripts: &TranscriptStore,
    todo: &TodoInfo,
    human_reply: Option<&str>,
    attempt: RunningAttempt,
    summary_service: Option<&crate::summary_service::SummaryService>,
) -> Result<TaskExecutionInfo, String> {
    let context = attempt.context().clone();
    let human_instruction = human_reply.unwrap_or(&todo.text);

    // 先读历史(不含本次人类输入),再把人类输入持久化;崩溃时至少留下输入。
    const PROMPT_BYTES: usize = 64 * 1024;
    let required = todo
        .text
        .len()
        .saturating_add(human_instruction.len())
        .saturating_add(4096);
    if required > PROMPT_BYTES {
        return attempt
            .finish_failure("input_limit", "任务或最新输入超过 prompt 字节预算")
            .map_err(|e| e.to_string());
    }
    let prior_turns_text = transcripts
        .task_context(&context.conversation_id, PROMPT_BYTES - required)
        .map_err(|e| e.to_string())?;

    transcripts
        .record_task_human_turn(
            &context.conversation_id,
            context.agent,
            &context.project_dir,
            &context.task_title,
            human_instruction,
        )
        .map_err(|e| e.to_string())?;
    attempt
        .mark_input_persisted(transcripts)
        .map_err(|e| e.to_string())?;

    let result = crate::headless_agent::process_task_headless_cancellable(
        context.agent,
        std::path::Path::new(&context.project_dir),
        &context.conversation_id,
        &todo.text,
        &prior_turns_text,
        human_instruction,
        Some(attempt.cancellation()),
    )
    .await;

    if *attempt.cancellation().borrow() {
        return Err("daemon draining; attempt interrupted".into());
    }
    match result {
        Ok(stdout) => {
            let info = attempt
                .finish_success(&stdout)
                .map_err(|e| format!("AI 回合落库失败: {e}"))?;
            if let Some(service) = summary_service
                && let crate::summary_config::SummaryProviderResolution::Configured(config, _) =
                    crate::summary_config::resolve_provider(None, None)
                && let Err(e) = service.submit_single(&crate::summary_service::SubmitSpec {
                    conversation_id: context.conversation_id,
                    source_session_id: None,
                    trigger: dozer_core::protocol::SummaryTrigger::NaturalExit,
                    provider: config.provider,
                    requested_model: config.model,
                    force: false,
                })
            {
                dozer_core::log_warn!(LOG, error = %e, todo_id = todo.id, "Todo 完成后提交总结任务失败");
            }
            Ok(info)
        }
        Err(error) => {
            let message = format!("{error:?}");
            attempt
                .finish_failure("headless", &message)
                .map_err(|e| format!("失败状态落库失败: {e}; 原始错误: {message}"))
        }
    }
}

/// 同步处理入口(旧 `ProcessTodoNow`):准入后立刻执行,返回最新 attempt。
#[allow(clippy::too_many_arguments)]
pub async fn process_task(
    executions: &Arc<TaskExecutionStore>,
    todos: &TodoStore,
    _session_summaries: &SessionSummaryStore,
    transcripts: &TranscriptStore,
    projects: &ProjectStore,
    todo: &TodoInfo,
    human_reply: Option<&str>,
    idempotency_key: Option<&str>,
    summary_service: Option<&crate::summary_service::SummaryService>,
) -> Result<TaskExecutionInfo, String> {
    match start_task(
        executions,
        todos,
        projects,
        todo,
        human_reply,
        idempotency_key,
    )
    .map_err(|e| e.to_string())?
    {
        Admission::Existing(info) => Ok(info),
        Admission::Started(attempt) => {
            run_attempt(transcripts, todo, human_reply, attempt, summary_service).await
        }
    }
}

/// 把失败/错误文本裁到可落库的短回合预算,避免把完整 stderr/prompt 写进
/// 会话历史。
pub fn truncate_for_turn(text: &str) -> String {
    const MAX: usize = 2000;
    if text.chars().count() <= MAX {
        return text.to_string();
    }
    let head: String = text.chars().take(MAX).collect();
    format!("{head}…")
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

    #[test]
    fn role_only_detection_matches_full_turn_detection() {
        assert!(needs_processing_role(None));
        assert!(needs_processing_role(Some("human")));
        assert!(!needs_processing_role(Some("ai")));
    }

    #[tokio::test]
    async fn process_task_errors_without_assigned_agent() {
        let tmp = tempfile::tempdir().unwrap();
        let todos = TodoStore::new(&tmp.path().join("t.db")).unwrap();
        let executions =
            std::sync::Arc::new(TaskExecutionStore::open(&tmp.path().join("t.db")).unwrap());
        let session_summaries = SessionSummaryStore::open(&tmp.path().join("t.db")).unwrap();
        let transcripts = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
        let projects = ProjectStore::new(&tmp.path().join("t.db")).unwrap();
        let todo = todos.add(1, "任务").unwrap();
        let err = process_task(
            &executions,
            &todos,
            &session_summaries,
            &transcripts,
            &projects,
            &todo,
            None,
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
        let executions =
            std::sync::Arc::new(TaskExecutionStore::open(&tmp.path().join("t.db")).unwrap());
        let session_summaries = SessionSummaryStore::open(&tmp.path().join("t.db")).unwrap();
        let transcripts = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
        let projects = ProjectStore::new(&tmp.path().join("t.db")).unwrap();
        let todo = todos.add(999, "任务").unwrap();
        let assigned = todos
            .assign_agent(todo.id, dozer_core::protocol::AgentKind::Claude)
            .unwrap();
        let err = process_task(
            &executions,
            &todos,
            &session_summaries,
            &transcripts,
            &projects,
            &assigned,
            None,
            None,
            None,
        )
        .await
        .unwrap_err();
        assert!(err.contains("项目"));
    }

    #[test]
    fn auto_process_skips_failed_attempt_until_new_human_input() {
        let attempt = TaskExecutionInfo {
            attempt_id: "a".into(),
            todo_id: 1,
            input_generation: 3,
            idempotency_key: None,
            status: TaskExecutionStatus::Failed,
            error_kind: Some("headless".into()),
            error_message: None,
            started_ms: Some(1),
            finished_ms: Some(2),
            conversation_id: Some("c".into()),
            created_ms: 1,
        };
        assert!(!auto_process_allowed(Some(&attempt), 3));
        assert!(auto_process_allowed(Some(&attempt), 4));
    }

    #[test]
    fn auto_process_never_repeats_running_attempt() {
        let attempt = TaskExecutionInfo {
            attempt_id: "a".into(),
            todo_id: 1,
            input_generation: 0,
            idempotency_key: None,
            status: TaskExecutionStatus::Running,
            error_kind: None,
            error_message: None,
            started_ms: Some(1),
            finished_ms: None,
            conversation_id: Some("c".into()),
            created_ms: 1,
        };
        assert!(!auto_process_allowed(Some(&attempt), 99));
    }
}
