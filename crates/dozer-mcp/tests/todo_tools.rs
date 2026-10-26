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
    let code_health = Arc::new(dozerd::code_health::CodeHealthStore::new(&db).unwrap());
    let transcripts = Arc::new(dozerd::transcripts::TranscriptStore::open(&db).unwrap());
    let session_summaries =
        Arc::new(dozerd::session_summary::SessionSummaryStore::open(&db).unwrap());
    let summary_jobs = Arc::new(dozerd::summary_jobs::SummaryJobStore::open(&db).unwrap());
    let backfill_registry = Arc::new(dozerd::session_summary_backfill::BackfillRegistry::new());
    let todos = Arc::new(dozerd::todo::TodoStore::new(&db).unwrap());
    let categories = Arc::new(dozerd::todo_category::CategoryStore::new(&db).unwrap());
    let memories = Arc::new(dozerd::memory::MemoryStore::new(&db).unwrap());
    let file_edit_history =
        Arc::new(dozerd::file_edit_history::FileEditHistoryStore::new(&db).unwrap());
    let groups = dozerd::group_service::GroupService::for_tests();
    let ide_lock_dir = tempfile::tempdir().expect("ide_lock_dir tempdir");
    let s = sock.clone();
    tokio::spawn(async move {
        dozerd::server::serve(
            &s,
            ide_lock_dir.path().to_path_buf(),
            dozerd::server::Stores {
                registry,
                projects,
                bookmarks,
                code_health,
                transcripts,
                session_summaries,
                summary_jobs,
                backfill_registry,
                todos,
                categories,
                memories,
                file_edit_history,
                groups,
                apps: bytehost_apps::service::AppService::unavailable("test"),
                locale: std::sync::Arc::new(
                    dozerd::locale_settings::LocaleSettingsStore::open(
                        &std::env::temp_dir()
                            .join(format!("dz-locale-{}.json", uuid::Uuid::new_v4())),
                    )
                    .unwrap(),
                ),
            },
            dozerd::task_poller::new_in_flight(),
            true,
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

async fn listed(server: &DozerMcpServer) -> Vec<serde_json::Value> {
    server
        .list_todos(Parameters(NoParams {}))
        .await
        .unwrap()
        .structured_content
        .expect("结构化内容")["todos"]
        .as_array()
        .unwrap()
        .clone()
}

#[tokio::test]
async fn blank_text_is_rejected_when_adding_and_creates_no_task() {
    let (sock, _guard) = start_daemon().await;
    let session_id = session_with_project(&sock, 1).await;
    let server = DozerMcpServer::new(Client::new(sock), session_id);
    for text in ["", "   ", " \n\t "] {
        let err = server
            .add_todo(Parameters(AddTodoParams { text: text.into() }))
            .await
            .unwrap_err();
        assert!(
            err.message.contains("不能为空"),
            "{text:?}: {}",
            err.message
        );
    }
    assert!(listed(&server).await.is_empty(), "空白文字不得产生任务");
}

#[tokio::test]
async fn blank_text_is_rejected_when_editing_and_keeps_the_old_text() {
    let (sock, _guard) = start_daemon().await;
    let session_id = session_with_project(&sock, 1).await;
    let server = DozerMcpServer::new(Client::new(sock), session_id);
    let added = server
        .add_todo(Parameters(AddTodoParams {
            text: "原文".into(),
        }))
        .await
        .unwrap();
    let id = added.structured_content.unwrap()["id"].as_i64().unwrap();
    for text in ["", "  \n "] {
        let err = server
            .edit_todo_text(Parameters(EditTodoTextParams {
                id,
                text: text.into(),
            }))
            .await
            .unwrap_err();
        assert!(
            err.message.contains("不能为空"),
            "{text:?}: {}",
            err.message
        );
    }
    assert_eq!(listed(&server).await[0]["text"], "原文");
}

#[tokio::test]
async fn text_with_surrounding_whitespace_is_still_accepted_as_given() {
    // 只拒绝"全是空白";有内容的文字原样保存,不替 agent 改写。
    let (sock, _guard) = start_daemon().await;
    let session_id = session_with_project(&sock, 1).await;
    let server = DozerMcpServer::new(Client::new(sock), session_id);
    let added = server
        .add_todo(Parameters(AddTodoParams {
            text: "  两侧有空格  ".into(),
        }))
        .await
        .unwrap();
    assert_eq!(added.structured_content.unwrap()["text"], "  两侧有空格  ");
}

#[tokio::test]
async fn a_session_cannot_toggle_or_edit_another_projects_todo() {
    let (sock, _guard) = start_daemon().await;
    let session_a = session_with_project(&sock, 1).await;
    let a = DozerMcpServer::new(Client::new(sock.clone()), session_a);
    let other = Client::new(sock.clone())
        .add_todo(2, "别的项目的任务")
        .await
        .unwrap();

    let err = a
        .toggle_todo(Parameters(ToggleTodoParams {
            id: other.id,
            done: true,
        }))
        .await
        .unwrap_err();
    assert!(err.message.contains("不属于当前项目"), "{}", err.message);
    let err = a
        .edit_todo_text(Parameters(EditTodoTextParams {
            id: other.id,
            text: "被改了".into(),
        }))
        .await
        .unwrap_err();
    assert!(err.message.contains("不属于当前项目"), "{}", err.message);

    // 别的项目的任务原样未动。
    let still = Client::new(sock).list_todos(2).await.unwrap();
    assert_eq!(still.len(), 1);
    assert_eq!(still[0].text, "别的项目的任务");
    assert!(!still[0].done);
}

#[tokio::test]
async fn a_todo_id_that_does_not_exist_is_a_readable_error() {
    let (sock, _guard) = start_daemon().await;
    let session_id = session_with_project(&sock, 1).await;
    let server = DozerMcpServer::new(Client::new(sock), session_id);
    let err = server
        .toggle_todo(Parameters(ToggleTodoParams {
            id: 9_999_999,
            done: true,
        }))
        .await
        .unwrap_err();
    assert!(err.message.contains("不属于当前项目"), "{}", err.message);
}
