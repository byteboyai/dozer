//! dozerd 第一个周期性后台任务(spec 2026-09-02):按固定间隔扫描开启了
//! `auto_poll_enabled` 的分类,把待处理的任务逐个交给
//! `task_processor::process_task`。全仓此前没有任何 `tokio::time::interval`
//! 用例(搜索确认过,唯一的周期性需求),范围限定在这一个用例,不做成
//! 通用 scheduler。

use crate::projects::ProjectStore;
use crate::session_summary::SessionSummaryStore;
use crate::task_execution::TaskExecutionStore;
use crate::todo::TodoStore;
use crate::todo_category::CategoryStore;
use crate::transcripts::TranscriptStore;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_secs(30);

/// 正在处理中的任务 id 集合,`task_poller` 和 `ProcessTodoNow` handler
/// (Task 9)共用同一份,防止同一任务被两条触发路径并发跑两次。
pub type InFlight = Arc<Mutex<HashSet<i64>>>;

pub fn new_in_flight() -> InFlight {
    Arc::new(Mutex::new(HashSet::new()))
}

/// RAII 占用标记:取消/panic/提前 return 都会在 Drop 时移除,避免任务永久
/// 卡在内存 in_flight。持久化状态仍以 attempts 表为准。
pub struct InFlightGuard {
    in_flight: InFlight,
    id: i64,
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        self.in_flight
            .lock()
            .expect("in_flight lock")
            .remove(&self.id);
    }
}

pub fn try_enter(in_flight: &InFlight, id: i64) -> Option<InFlightGuard> {
    let mut guard = in_flight.lock().expect("in_flight lock");
    if !guard.insert(id) {
        return None;
    }
    drop(guard);
    Some(InFlightGuard {
        in_flight: in_flight.clone(),
        id,
    })
}

/// 启动常驻轮询任务。返回的 `JoinHandle` 调用方通常不需要 `.await`
/// (dozerd 进程存活期间一直跑,随进程退出而结束),但保留返回值供测试
/// 场景需要时可以 `.abort()`。
pub fn spawn(
    todos: Arc<TodoStore>,
    categories: Arc<CategoryStore>,
    session_summaries: Arc<SessionSummaryStore>,
    transcripts: Arc<TranscriptStore>,
    projects: Arc<ProjectStore>,
    executions: Arc<TaskExecutionStore>,
    in_flight: InFlight,
    summary_service: Arc<crate::summary_service::SummaryService>,
) -> tokio::task::JoinHandle<()> {
    spawn_after_ready(
        todos,
        categories,
        session_summaries,
        transcripts,
        projects,
        executions,
        in_flight,
        summary_service,
        None,
    )
}

pub fn spawn_after_ready(
    todos: Arc<TodoStore>,
    categories: Arc<CategoryStore>,
    session_summaries: Arc<SessionSummaryStore>,
    transcripts: Arc<TranscriptStore>,
    projects: Arc<ProjectStore>,
    executions: Arc<TaskExecutionStore>,
    in_flight: InFlight,
    summary_service: Arc<crate::summary_service::SummaryService>,
    startup: Option<tokio::sync::watch::Receiver<bool>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        if let Some(mut ready) = startup {
            if !crate::daemon::wait_for_startup(&mut ready).await {
                return;
            }
        }
        let mut stop = executions.cancellation();
        let mut ticker = tokio::time::interval(POLL_INTERVAL);
        loop {
            if executions.is_stopping() {
                break;
            }
            tokio::select! {_=stop.changed()=>break,_=ticker.tick()=>{}}
            let task_owner = executions.clone();
            let todo_store = todos.clone();
            let category_store = categories.clone();
            let summaries = session_summaries.clone();
            let transcript_store = transcripts.clone();
            let project_store = projects.clone();
            let execution_store = executions.clone();
            let flight = in_flight.clone();
            let service = summary_service.clone();
            let runtime = tokio::runtime::Handle::current();
            let _ = task_owner
                .run_blocking(move || {
                    runtime.block_on(tick_once(
                        &todo_store,
                        &category_store,
                        &summaries,
                        &transcript_store,
                        &project_store,
                        &execution_store,
                        &flight,
                        Some(&service),
                    ))
                })
                .await;
        }
    })
}

/// 单次扫描,拆成独立函数供测试直接调用(不需要真的等 30 秒)。
pub async fn tick_once(
    todos: &TodoStore,
    categories: &CategoryStore,
    session_summaries: &SessionSummaryStore,
    transcripts: &TranscriptStore,
    projects: &ProjectStore,
    executions: &Arc<TaskExecutionStore>,
    in_flight: &InFlight,
    summary_service: Option<&crate::summary_service::SummaryService>,
) {
    let Ok(enabled_categories) = categories.list_auto_poll_enabled_all() else {
        return;
    };
    for cat in enabled_categories {
        let Ok(candidates) = todos.list_assigned_incomplete_in_category(cat.id) else {
            continue;
        };
        for todo in candidates {
            let conversation_id = todo.dispatch_session_id.as_deref().unwrap_or("");
            let latest_role = transcripts
                .latest_turn_role(conversation_id)
                .unwrap_or(None);
            if !crate::task_processor::needs_processing_role(latest_role.as_deref()) {
                continue;
            }
            let latest_attempt = executions.latest(todo.id).unwrap_or(None);
            let latest_human = transcripts
                .latest_human_turn_index(conversation_id)
                .unwrap_or(-1);
            if !crate::task_processor::auto_process_allowed(latest_attempt.as_ref(), latest_human) {
                continue;
            }
            {
                let Some(_guard) = try_enter(in_flight, todo.id) else {
                    continue; // 已经在处理中,跳过这次
                };
                let _ = crate::task_processor::process_task(
                    executions,
                    todos,
                    session_summaries,
                    transcripts,
                    projects,
                    &todo,
                    None,
                    None,
                    summary_service,
                )
                .await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn tick_once_skips_task_already_in_flight() {
        let tmp = tempfile::tempdir().unwrap();
        let todos = TodoStore::new(&tmp.path().join("t.db")).unwrap();
        let categories = CategoryStore::new(&tmp.path().join("t.db")).unwrap();
        let session_summaries = SessionSummaryStore::open(&tmp.path().join("t.db")).unwrap();
        let transcripts = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
        let projects = ProjectStore::new(&tmp.path().join("t.db")).unwrap();
        let executions = Arc::new(TaskExecutionStore::open(&tmp.path().join("t.db")).unwrap());

        let cat = categories.add(1, None, "自动分类").unwrap();
        categories.set_auto_poll(cat.id, true).unwrap();
        let todo = todos.add(1, "任务").unwrap();
        todos
            .assign_agent(todo.id, dozer_core::protocol::AgentKind::Claude)
            .unwrap();
        todos.set_category(todo.id, Some(cat.id)).unwrap();

        let in_flight = new_in_flight();
        in_flight.lock().unwrap().insert(todo.id); // 模拟"已经在处理中"

        tick_once(
            &todos,
            &categories,
            &session_summaries,
            &transcripts,
            &projects,
            &executions,
            &in_flight,
            None,
        )
        .await;
        assert!(in_flight.lock().unwrap().contains(&todo.id));
    }

    #[tokio::test]
    async fn tick_once_ignores_categories_without_auto_poll() {
        let tmp = tempfile::tempdir().unwrap();
        let todos = TodoStore::new(&tmp.path().join("t.db")).unwrap();
        let categories = CategoryStore::new(&tmp.path().join("t.db")).unwrap();
        let session_summaries = SessionSummaryStore::open(&tmp.path().join("t.db")).unwrap();
        let transcripts = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
        let projects = ProjectStore::new(&tmp.path().join("t.db")).unwrap();
        let executions = Arc::new(TaskExecutionStore::open(&tmp.path().join("t.db")).unwrap());

        let cat = categories.add(1, None, "未开启轮询").unwrap();
        let todo = todos.add(1, "任务").unwrap();
        todos
            .assign_agent(todo.id, dozer_core::protocol::AgentKind::Claude)
            .unwrap();
        todos.set_category(todo.id, Some(cat.id)).unwrap();

        let in_flight = new_in_flight();
        tick_once(
            &todos,
            &categories,
            &session_summaries,
            &transcripts,
            &projects,
            &executions,
            &in_flight,
            None,
        )
        .await;
        assert!(
            in_flight.lock().unwrap().is_empty(),
            "未开启轮询的分类不该被扫到"
        );
    }
}
