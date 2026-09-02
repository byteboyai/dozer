//! dozerd 第一个周期性后台任务(spec 2026-09-02):按固定间隔扫描开启了
//! `auto_poll_enabled` 的分类,把待处理的任务逐个交给
//! `task_processor::process_task`。全仓此前没有任何 `tokio::time::interval`
//! 用例(搜索确认过,唯一的周期性需求),范围限定在这一个用例,不做成
//! 通用 scheduler。

use crate::projects::ProjectStore;
use crate::session_summary::SessionSummaryStore;
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

/// 启动常驻轮询任务。返回的 `JoinHandle` 调用方通常不需要 `.await`
/// (dozerd 进程存活期间一直跑,随进程退出而结束),但保留返回值供测试
/// 场景需要时可以 `.abort()`。
pub fn spawn(
    todos: Arc<TodoStore>,
    categories: Arc<CategoryStore>,
    session_summaries: Arc<SessionSummaryStore>,
    transcripts: Arc<TranscriptStore>,
    projects: Arc<ProjectStore>,
    in_flight: InFlight,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(POLL_INTERVAL);
        loop {
            ticker.tick().await;
            tick_once(
                &todos,
                &categories,
                &session_summaries,
                &transcripts,
                &projects,
                &in_flight,
            )
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
    in_flight: &InFlight,
) {
    let Ok(enabled_categories) = categories.list_auto_poll_enabled_all() else {
        return;
    };
    for cat in enabled_categories {
        let Ok(candidates) = todos.list_assigned_incomplete_in_category(cat.id) else {
            continue;
        };
        for todo in candidates {
            let turns = transcripts
                .get_conversation_turns(
                    todo.dispatch_session_id.as_deref().unwrap_or(""),
                    -1,
                    u32::MAX,
                )
                .unwrap_or_default();
            if !crate::task_processor::needs_processing(&turns) {
                continue;
            }
            {
                let mut guard = in_flight.lock().expect("in_flight lock");
                if !guard.insert(todo.id) {
                    continue; // 已经在处理中,跳过这次
                }
            }
            let _ = crate::task_processor::process_task(
                todos,
                session_summaries,
                transcripts,
                projects,
                &todo,
                None,
            )
            .await;
            in_flight.lock().expect("in_flight lock").remove(&todo.id);
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
            &in_flight,
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
            &in_flight,
        )
        .await;
        assert!(
            in_flight.lock().unwrap().is_empty(),
            "未开启轮询的分类不该被扫到"
        );
    }
}
