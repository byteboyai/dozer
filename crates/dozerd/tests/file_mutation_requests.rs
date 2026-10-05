use dozer_client::Client;
use dozer_core::protocol::MutationOutcome;
use std::sync::Arc;

fn temp_sock() -> std::path::PathBuf {
    std::path::PathBuf::from(format!(
        "/tmp/dz-file-mutation-{}.sock",
        uuid::Uuid::new_v4()
    ))
}

struct CleanupGuard(std::path::PathBuf);
impl Drop for CleanupGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

async fn start_daemon() -> (
    std::path::PathBuf,
    CleanupGuard,
    std::sync::Arc<dozerd::projects::ProjectStore>,
) {
    let sock = temp_sock();
    let db = std::path::PathBuf::from(format!("/tmp/dz-file-mutation-{}.db", uuid::Uuid::new_v4()));
    let projects = Arc::new(dozerd::projects::ProjectStore::new(&db).unwrap());
    let stores = dozerd::server::Stores {
        registry: Arc::new(dozerd::registry::SessionRegistry::new()),
        projects: projects.clone(),
        bookmarks: Arc::new(dozerd::bookmarks::BookmarkStore::new(&db).unwrap()),
        code_health: Arc::new(dozerd::code_health::CodeHealthStore::new(&db).unwrap()),
        transcripts: Arc::new(dozerd::transcripts::TranscriptStore::open(&db).unwrap()),
        session_summaries: Arc::new(
            dozerd::session_summary::SessionSummaryStore::open(&db).unwrap(),
        ),
        summary_jobs: Arc::new(dozerd::summary_jobs::SummaryJobStore::open(&db).unwrap()),
        backfill_registry: Arc::new(dozerd::session_summary_backfill::BackfillRegistry::new()),
        todos: Arc::new(dozerd::todo::TodoStore::new(&db).unwrap()),
        categories: Arc::new(dozerd::todo_category::CategoryStore::new(&db).unwrap()),
        memories: Arc::new(dozerd::memory::MemoryStore::new(&db).unwrap()),
        file_edit_history: Arc::new(
            dozerd::file_edit_history::FileEditHistoryStore::new(&db).unwrap(),
        ),
        groups: dozerd::group_service::GroupService::for_tests(),
        apps: dozerd::app_service::AppService::unavailable("test"),
    };
    let ide_lock_dir = tempfile::tempdir().expect("ide_lock_dir tempdir");
    let s = sock.clone();
    tokio::spawn(async move {
        dozerd::server::serve(
            &s,
            ide_lock_dir.path().to_path_buf(),
            stores,
            dozerd::task_poller::new_in_flight(),
        )
        .await
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    (sock.clone(), CleanupGuard(sock), projects)
}

#[tokio::test]
async fn locate_then_apply_then_conflict_on_reapply() {
    let (sock, _guard, projects) = start_daemon().await;
    let project_dir = tempfile::tempdir().unwrap();
    std::fs::write(
        project_dir.path().join("a.txt"),
        "line one\nline two\nline three\n",
    )
    .unwrap();
    let project = projects.open(project_dir.path().to_str().unwrap()).unwrap();

    let client = Client::new(sock);
    let matches = client
        .locate_in_file(project.id, "a.txt", "line two")
        .await
        .unwrap();
    assert_eq!(matches.len(), 1);
    let m = &matches[0];

    let outcome = client
        .apply_precise_edit(
            project.id,
            "a.txt",
            m.start_line,
            m.start_col,
            m.end_line,
            m.end_col,
            "line two",
            "replaced line",
            "把第二行替换掉",
            "claude",
            "sess-1",
        )
        .await
        .unwrap();
    assert!(matches!(outcome, MutationOutcome::Applied { .. }));

    // 用刚才(已经过期的)坐标+旧内容再打一次,应该产生 Conflict,并且不会
    // 真的再改一次盘。
    let stale_outcome = client
        .apply_precise_edit(
            project.id,
            "a.txt",
            m.start_line,
            m.start_col,
            m.end_line,
            m.end_col,
            "line two",
            "should not apply",
            "重复调用",
            "claude",
            "sess-1",
        )
        .await
        .unwrap();
    assert!(matches!(stale_outcome, MutationOutcome::Conflict { .. }));

    let on_disk = std::fs::read_to_string(project_dir.path().join("a.txt")).unwrap();
    assert_eq!(on_disk, "line one\nreplaced line\nline three\n");
}
