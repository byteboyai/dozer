use dozer_client::Client;
use dozer_core::protocol::PreviewContext;
use dozer_mcp::server::{DozerMcpServer, NoParams};
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
    let projects = Arc::new(dozerd::projects::ProjectStore::new(&db).unwrap());
    let bookmarks = Arc::new(dozerd::bookmarks::BookmarkStore::new(&db).unwrap());
    let transcripts = Arc::new(dozerd::transcripts::TranscriptStore::open(&db).unwrap());
    let session_summaries =
        Arc::new(dozerd::session_summary::SessionSummaryStore::open(&db).unwrap());
    let backfill_registry = Arc::new(dozerd::session_summary_backfill::BackfillRegistry::new());
    let todos = Arc::new(dozerd::todo::TodoStore::new(&db).unwrap());
    let categories = Arc::new(dozerd::todo_category::CategoryStore::new(&db).unwrap());
    let ide_lock_dir = tempfile::tempdir().expect("ide_lock_dir tempdir");
    let s = sock.clone();
    tokio::spawn(async move {
        dozerd::server::serve(
            &s,
            ide_lock_dir.path().to_path_buf(),
            registry,
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
        updated_at_ms: 1_700_000_000_000,
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

/// 直接调真正的 `#[tool]` 方法（不是绕开 JSON 的 `fetch_context_for_test`），
/// 断言无预览时那份 payload 的形状：全 `null` + `reason`。
#[tokio::test]
async fn tool_json_payload_reports_no_active_preview() {
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
    let result = server
        .get_preview_context(Parameters(NoParams {}))
        .await
        .unwrap();

    let value = result.structured_content.expect("tool 返回结构化内容");
    assert_eq!(value["reason"], "no_active_preview");
    for key in [
        "path",
        "start_line",
        "start_col",
        "end_line",
        "end_col",
        "has_selection",
        "updated_at_ms",
    ] {
        assert!(value[key].is_null(), "{key} 在无预览时应为 null: {value}");
    }
}

/// 有预览时的 payload 形状：字段逐个透传，含新鲜度戳 `updated_at_ms`。
#[tokio::test]
async fn tool_json_payload_carries_context_fields_and_timestamp() {
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
    client
        .update_preview_context(
            1,
            Some(PreviewContext {
                path: "/repo/src/lib.rs".into(),
                start_line: 3,
                start_col: 1,
                end_line: 9,
                end_col: 4,
                has_selection: true,
                updated_at_ms: 1_700_000_000_000,
            }),
        )
        .await
        .unwrap();

    let server = DozerMcpServer::new(Client::new(sock), session.id.clone());
    let result = server
        .get_preview_context(Parameters(NoParams {}))
        .await
        .unwrap();

    let value = result.structured_content.expect("tool 返回结构化内容");
    assert_eq!(value["path"], "/repo/src/lib.rs");
    assert_eq!(value["start_line"], 3);
    assert_eq!(value["start_col"], 1);
    assert_eq!(value["end_line"], 9);
    assert_eq!(value["end_col"], 4);
    assert_eq!(value["has_selection"], true);
    assert_eq!(value["updated_at_ms"], 1_700_000_000_000u64);
    assert!(value["reason"].is_null());
}

#[tokio::test]
async fn get_preview_context_errors_for_unknown_session() {
    let (sock, _guard) = start_daemon().await;

    let server = DozerMcpServer::new(Client::new(sock), "not-a-real-session".into());
    let err = server.fetch_context_for_test().await.unwrap_err();
    assert!(err.to_string().contains("会话不存在"));
}
