use dozer_client::Client;
use dozer_core::protocol::MutationOutcome;
use std::sync::Arc;

struct CleanupGuard(std::path::PathBuf);
impl Drop for CleanupGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

async fn start_daemon() -> (
    std::path::PathBuf,
    CleanupGuard,
    Arc<dozerd::projects::ProjectStore>,
) {
    let sock = std::path::PathBuf::from(format!("/tmp/dz-agent-ctx-{}.sock", uuid::Uuid::new_v4()));
    let db = std::path::PathBuf::from(format!("/tmp/dz-agent-ctx-{}.db", uuid::Uuid::new_v4()));
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
        apps: bytehost_apps::service::AppService::unavailable("test"),
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
async fn add_list_remove_and_idempotent() {
    let (sock, _guard, projects) = start_daemon().await;
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("research")).unwrap();
    let project = projects.open(dir.path().to_str().unwrap()).unwrap();
    let client = Client::new(sock);

    let a = client
        .add_context_item(project.id, "dir", "research")
        .await
        .unwrap();
    let again = client
        .add_context_item(project.id, "dir", "research/")
        .await
        .unwrap();
    assert_eq!(a.id, again.id);
    // 磁盘上不存在的路径也允许添加。
    client
        .add_context_item(project.id, "file", "later.md")
        .await
        .unwrap();

    let items = client.list_context_items(project.id).await.unwrap();
    assert_eq!(items.len(), 2);

    client.remove_context_item(project.id, a.id).await.unwrap();
    client.remove_context_item(project.id, a.id).await.unwrap();
    assert_eq!(
        client.list_context_items(project.id).await.unwrap().len(),
        1
    );
}

#[tokio::test]
async fn add_rejects_out_of_bounds_and_unknown_project() {
    let (sock, _guard, projects) = start_daemon().await;
    let dir = tempfile::tempdir().unwrap();
    let project = projects.open(dir.path().to_str().unwrap()).unwrap();
    let client = Client::new(sock);

    assert!(
        client
            .add_context_item(project.id, "file", "../secret")
            .await
            .is_err()
    );
    assert!(
        client
            .add_context_item(project.id, "file", "/etc/passwd")
            .await
            .is_err()
    );
    assert!(
        client
            .add_context_item(9999, "file", "a.txt")
            .await
            .is_err()
    );
    assert!(
        client
            .list_context_items(project.id)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn history_lists_newest_first_filters_and_reports_new_end() {
    let (sock, _guard, projects) = start_daemon().await;
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    std::fs::write(dir.path().join("a.txt"), "one\ntwo\n").unwrap();
    std::fs::write(dir.path().join("sub/b.txt"), "alpha\n").unwrap();
    let project = projects.open(dir.path().to_str().unwrap()).unwrap();
    let client = Client::new(sock);

    let o1 = client
        .apply_precise_edit(
            project.id, "a.txt", 2, 1, 2, 4, "two", "2\n2b", "改 two", "claude", "s1",
        )
        .await
        .unwrap();
    assert!(matches!(o1, MutationOutcome::Applied { .. }));
    let o2 = client
        .apply_precise_edit(
            project.id,
            "sub/b.txt",
            1,
            1,
            1,
            6,
            "alpha",
            "ALPHA",
            "大写",
            "codex",
            "s2",
        )
        .await
        .unwrap();
    assert!(matches!(o2, MutationOutcome::Applied { .. }));

    let all = client
        .list_file_edit_history(project.id, None, 50)
        .await
        .unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].target_path, "sub/b.txt", "最新的在前");
    assert_eq!(all[1].actor, "claude");
    assert_eq!(
        (all[1].new_end_line, all[1].new_end_col),
        (3, 3),
        "\"2\\n2b\" 结束于第 3 行第 3 列"
    );

    let only_sub = client
        .list_file_edit_history(project.id, Some("sub"), 50)
        .await
        .unwrap();
    assert_eq!(only_sub.len(), 1);
    assert_eq!(only_sub[0].target_path, "sub/b.txt");
}
