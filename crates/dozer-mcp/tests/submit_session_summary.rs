use dozer_client::Client;
use dozer_core::protocol::SummaryStatus;
use dozer_mcp::server::{DozerMcpServer, SubmitSessionSummaryParams};
use rmcp::handler::server::wrapper::Parameters;
use std::sync::Arc;
use std::time::Duration;

fn temp_sock() -> std::path::PathBuf {
    let id = uuid::Uuid::new_v4();
    std::path::PathBuf::from(format!("/tmp/dz-mcp-{}.sock", &id.to_string()[..8]))
}

struct CleanupGuard(std::path::PathBuf);
impl Drop for CleanupGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

async fn start_daemon() -> (std::path::PathBuf, CleanupGuard) {
    let sock = temp_sock();
    let registry = Arc::new(dozerd::registry::SessionRegistry::new());
    let db = std::path::PathBuf::from(format!("/tmp/dz-mcp-{}.db", uuid::Uuid::new_v4()));
    let store = Arc::new(dozerd::acceptance::AcceptanceStore::open(&db).unwrap());
    let projects = Arc::new(dozerd::projects::ProjectStore::new(&db).unwrap());
    let bookmarks = Arc::new(dozerd::bookmarks::BookmarkStore::new(&db).unwrap());
    let transcripts = Arc::new(dozerd::transcripts::TranscriptStore::open(&db).unwrap());
    let session_summaries =
        Arc::new(dozerd::session_summary::SessionSummaryStore::open(&db).unwrap());
    let backfill_registry = Arc::new(dozerd::session_summary_backfill::BackfillRegistry::new());
    let todos = Arc::new(dozerd::todo::TodoStore::new(&db).unwrap());
    let categories = Arc::new(dozerd::todo_category::CategoryStore::new(&db).unwrap());
    let s = sock.clone();
    tokio::spawn(async move {
        dozerd::server::serve(
            &s,
            registry,
            store,
            projects,
            bookmarks,
            transcripts,
            session_summaries,
            backfill_registry,
            todos,
            categories,
            dozerd::task_poller::new_in_flight(),
        )
        .await
    });
    for _ in 0..100 {
        if sock.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let guard = CleanupGuard(sock.clone());
    (sock, guard)
}

#[tokio::test]
async fn submit_session_summary_records_to_dozerd() {
    let (sock, _guard) = start_daemon().await;
    let client = Client::new(sock.clone());
    let session = client
        .create(
            "测试",
            "/bin/sh",
            &["-c".into(), "sleep 5".into()],
            "/tmp",
            80,
            24,
            1,
        )
        .await
        .unwrap();

    let server = DozerMcpServer::new(Client::new(sock), session.id.clone());
    server
        .submit_session_summary(Parameters(SubmitSessionSummaryParams {
            title: "标题".into(),
            summary: "摘要".into(),
        }))
        .await
        .unwrap();

    let got = client
        .get_session_summary(&session.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got.title, "标题");
    assert_eq!(got.summary, "摘要");
    assert_eq!(got.status, SummaryStatus::AiGenerated);
}

#[tokio::test]
async fn submit_session_summary_truncates_oversized_fields() {
    let (sock, _guard) = start_daemon().await;
    let client = Client::new(sock.clone());
    let session = client
        .create(
            "测试",
            "/bin/sh",
            &["-c".into(), "sleep 5".into()],
            "/tmp",
            80,
            24,
            1,
        )
        .await
        .unwrap();

    let long_title = "标".repeat(500);
    let long_summary = "摘".repeat(20_000);
    let server = DozerMcpServer::new(Client::new(sock), session.id.clone());
    server
        .submit_session_summary(Parameters(SubmitSessionSummaryParams {
            title: long_title,
            summary: long_summary,
        }))
        .await
        .unwrap();

    let got = client
        .get_session_summary(&session.id)
        .await
        .unwrap()
        .unwrap();
    assert!(got.title.chars().count() <= 200);
    assert!(got.summary.chars().count() <= 8000);
}

#[tokio::test]
async fn submit_session_summary_errors_for_unknown_session() {
    let (sock, _guard) = start_daemon().await;
    let server = DozerMcpServer::new(Client::new(sock), "not-a-real-session".into());
    let err = server
        .submit_session_summary(Parameters(SubmitSessionSummaryParams {
            title: "t".into(),
            summary: "s".into(),
        }))
        .await
        .unwrap_err();
    assert!(err.message.contains("会话不存在") || err.message.contains("daemon 错误"));
}
