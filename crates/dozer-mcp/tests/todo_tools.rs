use dozer_client::Client;
use dozer_mcp::server::{
    AddTodoParams, DozerMcpServer, EditTodoTextParams, NoParams, ToggleTodoParams,
};
use rmcp::handler::server::wrapper::Parameters;
use std::sync::Arc;
use std::time::Duration;

fn temp_sock() -> std::path::PathBuf {
    let id = uuid::Uuid::new_v4();
    std::path::PathBuf::from(format!("/tmp/dz-mcp-todo-{}.sock", &id.to_string()[..8]))
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
    let db = std::path::PathBuf::from(format!("/tmp/dz-mcp-todo-{}.db", uuid::Uuid::new_v4()));
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

async fn session_with_project(sock: &std::path::Path, project_id: i64) -> String {
    let client = Client::new(sock.to_path_buf());
    let session = client
        .create(
            "测试",
            "/bin/sh",
            &["-c".into(), "cat".into()],
            "/tmp",
            80,
            24,
            project_id,
        )
        .await
        .unwrap();
    session.id
}

#[tokio::test]
async fn add_list_toggle_and_edit_todo_via_mcp_tools() {
    let (sock, _guard) = start_daemon().await;
    let session_id = session_with_project(&sock, 1).await;
    let server = DozerMcpServer::new(Client::new(sock), session_id);

    let added = server
        .add_todo(Parameters(AddTodoParams {
            text: "写完 spec".into(),
        }))
        .await
        .unwrap();
    let added_value = added.structured_content.expect("结构化内容");
    assert_eq!(added_value["text"], "写完 spec");
    assert_eq!(added_value["done"], false);
    let id = added_value["id"].as_i64().unwrap();

    let listed = server.list_todos(Parameters(NoParams {})).await.unwrap();
    let listed_value = listed.structured_content.expect("结构化内容");
    assert_eq!(listed_value["todos"].as_array().unwrap().len(), 1);

    let toggled = server
        .toggle_todo(Parameters(ToggleTodoParams { id, done: true }))
        .await
        .unwrap();
    assert_eq!(
        toggled.structured_content.expect("结构化内容")["done"],
        true
    );

    let edited = server
        .edit_todo_text(Parameters(EditTodoTextParams {
            id,
            text: "改过的文字".into(),
        }))
        .await
        .unwrap();
    assert_eq!(
        edited.structured_content.expect("结构化内容")["text"],
        "改过的文字"
    );
}

#[tokio::test]
async fn list_todos_errors_for_unknown_session() {
    let (sock, _guard) = start_daemon().await;
    let server = DozerMcpServer::new(Client::new(sock), "not-a-real-session".into());
    let err = server
        .list_todos(Parameters(NoParams {}))
        .await
        .unwrap_err();
    assert!(err.message.contains("会话不存在"));
}
