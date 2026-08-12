use dozer_client::Client;
use dozer_core::protocol::PreviewContext;
use dozer_mcp::server::DozerMcpServer;
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
    let s = sock.clone();
    tokio::spawn(
        async move { dozerd::server::serve(&s, registry, store, projects, bookmarks).await },
    );
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
async fn get_preview_context_returns_pushed_value_for_resolved_project() {
    let (sock, _guard) = start_daemon().await;
    let client = Client::new(sock.clone());
    let session = client
        .create(
            "测试",
            "/bin/sh",
            &["-c".into(), "cat".into()],
            "/tmp",
            80,
            24,
            1,
        )
        .await
        .unwrap();

    let ctx = PreviewContext {
        path: "/repo/src/main.rs".into(),
        start_line: 2,
        start_col: 1,
        end_line: 2,
        end_col: 5,
        has_selection: false,
    };
    client
        .update_preview_context(1, Some(ctx.clone()))
        .await
        .unwrap();

    let server = DozerMcpServer::new(Client::new(sock), session.id.clone());
    let got = server.fetch_context_for_test().await.unwrap();
    assert_eq!(got, Some(ctx));
}

#[tokio::test]
async fn get_preview_context_no_active_preview_returns_none() {
    let (sock, _guard) = start_daemon().await;
    let client = Client::new(sock.clone());
    let session = client
        .create(
            "测试",
            "/bin/sh",
            &["-c".into(), "cat".into()],
            "/tmp",
            80,
            24,
            1,
        )
        .await
        .unwrap();

    let server = DozerMcpServer::new(Client::new(sock), session.id.clone());
    let got = server.fetch_context_for_test().await.unwrap();
    assert_eq!(got, None);
}

#[tokio::test]
async fn get_preview_context_errors_for_unknown_session() {
    let (sock, _guard) = start_daemon().await;

    let server = DozerMcpServer::new(Client::new(sock), "not-a-real-session".into());
    let err = server.fetch_context_for_test().await.unwrap_err();
    assert!(err.to_string().contains("会话不存在"));
}
