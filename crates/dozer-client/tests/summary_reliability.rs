use dozer_core::protocol::{AgentKind, SummaryJobStatus, SummaryTrigger};
use dozerd::summary_pipeline::{PipelineError, Summarizer};
use std::sync::Arc;

struct FakeModel;
impl Summarizer for FakeModel {
    fn call(
        &mut self,
        instruction: &str,
        _: &str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, PipelineError>> + Send>>
    {
        let result = if instruction.contains("合并") {
            r#"{"title":"修复完成","summary":"实现了修改；测试未执行。","goals":[],"actions":["修改"],"decisions":[],"results":[],"incomplete":["测试未执行"]}"#
        } else {
            r#"{"goals":["修复"],"actions":["修改"],"decisions":[],"results":[],"incomplete":["测试未执行"]}"#
        }.to_string();
        Box::pin(async move { Ok(result) })
    }
}

#[tokio::test]
async fn generated_summary_is_visible_through_panel_query_and_shared_batches_finish() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("test.db");
    let transcripts = Arc::new(dozerd::transcripts::TranscriptStore::open(&db).unwrap());
    let legacy = Arc::new(dozerd::session_summary::SessionSummaryStore::open(&db).unwrap());
    let jobs = Arc::new(dozerd::summary_jobs::SummaryJobStore::open(&db).unwrap());
    transcripts
        .record_task_turns(
            "c1",
            AgentKind::Codex,
            "/summary-test",
            "原始标题",
            "请修复",
            "已修改，未测试",
        )
        .unwrap();
    legacy
        .record(&dozer_core::protocol::SessionSummaryPayload {
            session_id: "s1".into(),
            conversation_id: Some("c1".into()),
            agent_kind: AgentKind::Claude,
            title: "旧拼接".into(),
            summary: "请修复".into(),
            status: dozer_core::protocol::SummaryStatus::HeuristicFallback,
            created_ts_ms: 1,
            task_id: Some(42),
        })
        .unwrap();
    let service = dozerd::summary_service::SummaryService {
        jobs: jobs.clone(),
        transcripts: transcripts.clone(),
        session_summaries: legacy.clone(),
        scratch_root: dir.path().join("scratch"),
    };
    let spec = dozerd::summary_service::SubmitSpec {
        conversation_id: "c1".into(),
        source_session_id: None,
        trigger: SummaryTrigger::Manual,
        provider: AgentKind::Claude,
        requested_model: Some("test-model".into()),
        force: true,
    };
    let id = service.submit_single(&spec).unwrap();
    assert_eq!(
        service.submit_single(&spec).unwrap(),
        id,
        "重复点击不能额外调用模型"
    );
    let a = jobs.create_batch("/summary-test", "repair").unwrap();
    let b = jobs.create_batch("/summary-test", "repair").unwrap();
    jobs.add_batch_job(a, id).unwrap();
    jobs.add_batch_job(b, id).unwrap();
    jobs.claim_next_queued().unwrap();
    let config = dozerd::summary_config::SummaryConfig {
        provider: AgentKind::Claude,
        model: Some("test-model".into()),
        call_timeout_secs: 2,
        max_retries: 0,
        input_budget_chars: 1000,
    };
    service
        .process_job(id, &config, &mut FakeModel)
        .await
        .unwrap();
    assert_eq!(
        jobs.get_job(id).unwrap().unwrap().status,
        SummaryJobStatus::Succeeded
    );
    assert_eq!(jobs.get_batch(a).unwrap().unwrap().succeeded, 1);
    assert_eq!(jobs.get_batch(b).unwrap().unwrap().succeeded, 1);

    let sock = std::path::PathBuf::from(format!("/tmp/dz-summary-{}.sock", uuid::Uuid::new_v4()));
    let stores = dozerd::server::Stores {
        registry: Arc::new(dozerd::registry::SessionRegistry::new()),
        projects: Arc::new(dozerd::projects::ProjectStore::new(&db).unwrap()),
        bookmarks: Arc::new(dozerd::bookmarks::BookmarkStore::new(&db).unwrap()),
        code_health: Arc::new(dozerd::code_health::CodeHealthStore::new(&db).unwrap()),
        transcripts,
        session_summaries: legacy,
        summary_jobs: jobs,
        backfill_registry: Arc::new(dozerd::session_summary_backfill::BackfillRegistry::new()),
        todos: Arc::new(dozerd::todo::TodoStore::new(&db).unwrap()),
        categories: Arc::new(dozerd::todo_category::CategoryStore::new(&db).unwrap()),
        memories: Arc::new(dozerd::memory::MemoryStore::new(&db).unwrap()),
        file_edit_history: Arc::new(
            dozerd::file_edit_history::FileEditHistoryStore::new(&db).unwrap(),
        ),
        groups: dozerd::group_service::GroupService::for_tests(),
        apps: bytehost_apps::service::AppService::unavailable("test"),
    };
    let server = tokio::spawn({
        let sock = sock.clone();
        let locks = dir.path().join("locks");
        async move {
            dozerd::server::serve(&sock, locks, stores, dozerd::task_poller::new_in_flight()).await
        }
    });
    for _ in 0..100 {
        if sock.exists() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let client = dozer_client::Client::new(sock.clone());
    let rows = client
        .list_conversations_with_summaries("/summary-test", None, 50, 0)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    let result = rows[0].1.as_ref().unwrap();
    assert_eq!(result.title, "修复完成");
    assert_eq!(result.summary, "实现了修改；测试未执行。");
    assert_eq!(result.task_id, Some(42));
    server.abort();
    let _ = server.await;
    let _ = std::fs::remove_file(&sock);
}
