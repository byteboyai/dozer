use dozer_client::Client;
use dozer_mcp::server::{ApplyPreciseEditParams, DozerMcpServer, LocateInFileParams};
use rmcp::handler::server::wrapper::Parameters;
use std::sync::Arc;
use std::time::Duration;

fn temp_sock() -> std::path::PathBuf {
    std::path::PathBuf::from(format!(
        "/tmp/dz-mcp-fm-{}.sock",
        &uuid::Uuid::new_v4().to_string()[..8]
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
    Arc<dozerd::projects::ProjectStore>,
) {
    let sock = temp_sock();
    let db = std::path::PathBuf::from(format!("/tmp/dz-mcp-fm-{}.db", uuid::Uuid::new_v4()));
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
    tokio::time::sleep(Duration::from_millis(50)).await;
    (sock.clone(), CleanupGuard(sock), projects)
}

async fn session_for_project(sock: &std::path::Path, project_id: i64) -> String {
    let client = Client::new(sock.to_path_buf());
    let session = client
        .create(
            "fm-test",
            "/bin/sh",
            &["-c".into(), "cat".into()],
            "/tmp",
            80,
            24,
            project_id,
        )
        .await
        .expect("建会话");
    session.id
}

#[tokio::test]
async fn locate_then_apply_roundtrip_via_mcp_tools() {
    let (sock, _guard, projects) = start_daemon().await;
    let project_dir = tempfile::tempdir().unwrap();
    std::fs::write(
        project_dir.path().join("a.txt"),
        "line one\nline two\nline three\n",
    )
    .unwrap();
    let project = projects.open(project_dir.path().to_str().unwrap()).unwrap();
    let session_id = session_for_project(&sock, project.id).await;
    let server = DozerMcpServer::new(Client::new(sock), session_id);

    let locate_result = server
        .locate_in_file(Parameters(LocateInFileParams {
            path: "a.txt".into(),
            query: "line two".into(),
        }))
        .await
        .expect("locate_in_file 应该成功");
    let value = locate_result
        .structured_content
        .expect("locate_in_file 应返回结构化内容");
    let matches = value["matches"].as_array().expect("matches 应是数组");
    assert_eq!(matches.len(), 1);
    let start_line = matches[0]["start_line"].as_u64().unwrap() as u32;
    let start_col = matches[0]["start_col"].as_u64().unwrap() as u32;
    let end_line = matches[0]["end_line"].as_u64().unwrap() as u32;
    let end_col = matches[0]["end_col"].as_u64().unwrap() as u32;

    let apply_result = server
        .apply_precise_edit(Parameters(ApplyPreciseEditParams {
            path: "a.txt".into(),
            start_line,
            start_col,
            end_line,
            end_col,
            expected_text: "line two".into(),
            new_text: "replaced".into(),
            summary: "测试替换".into(),
        }))
        .await
        .expect("apply_precise_edit 应该成功");
    let outcome = apply_result
        .structured_content
        .expect("apply_precise_edit 应返回结构化内容");
    assert_eq!(outcome["kind"], "applied");

    let on_disk = std::fs::read_to_string(project_dir.path().join("a.txt")).unwrap();
    assert_eq!(on_disk, "line one\nreplaced\nline three\n");
}

#[tokio::test]
async fn apply_precise_edit_reports_conflict_with_actual_text() {
    let (sock, _guard, projects) = start_daemon().await;
    let project_dir = tempfile::tempdir().unwrap();
    std::fs::write(project_dir.path().join("a.txt"), "actual\n").unwrap();
    let project = projects.open(project_dir.path().to_str().unwrap()).unwrap();
    let session_id = session_for_project(&sock, project.id).await;
    let server = DozerMcpServer::new(Client::new(sock), session_id);

    let apply_result = server
        .apply_precise_edit(Parameters(ApplyPreciseEditParams {
            path: "a.txt".into(),
            start_line: 1,
            start_col: 1,
            end_line: 1,
            end_col: 7,
            expected_text: "stale!".into(),
            new_text: "x".into(),
            summary: "应该冲突".into(),
        }))
        .await
        .expect("调用本身应该成功(冲突是结果,不是调用错误)");
    let outcome = apply_result.structured_content.expect("应有结构化内容");
    assert_eq!(outcome["kind"], "conflict");
    assert_eq!(outcome["actual_text"], "actual");
}
