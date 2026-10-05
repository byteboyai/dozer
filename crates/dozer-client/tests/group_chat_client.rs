use dozer_client::Client;
use dozer_core::protocol::{AgentKind, GroupCancelScope, GroupMessageStatus};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;

struct EchoRunner;
impl dozerd::group_adapter::GroupAgentRunner for EchoRunner {
    fn run<'a>(
        &'a self,
        _req: dozerd::group_adapter::TurnRequest,
        _cancel: watch::Receiver<bool>,
    ) -> dozerd::group_adapter::BoxFuture<'a, Result<String, dozerd::group_adapter::TurnError>>
    {
        Box::pin(async { Ok("echo".to_string()) })
    }
}

fn temp_sock() -> std::path::PathBuf {
    std::path::PathBuf::from(format!(
        "/tmp/dz-group-client-{}.sock",
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
    Arc<dozerd::projects::ProjectStore>,
) {
    let sock = temp_sock();
    let db = std::path::PathBuf::from(format!("/tmp/dz-group-client-{}.db", uuid::Uuid::new_v4()));
    let projects = Arc::new(dozerd::projects::ProjectStore::new(&db).unwrap());
    let dir_lookup = projects.clone();
    let groups = dozerd::group_service::GroupService::new(
        Arc::new(dozerd::group_store::GroupStore::new(&db).unwrap()),
        Arc::new(EchoRunner),
        Arc::new(move |id| {
            dir_lookup
                .path_of(id)
                .ok()
                .flatten()
                .map(std::path::PathBuf::from)
        }),
    );
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
        groups,
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
    tokio::time::sleep(Duration::from_millis(50)).await;
    (sock.clone(), CleanupGuard(sock), projects)
}

#[tokio::test]
async fn gui_contract_create_post_poll_cancel_push_todo() {
    let (sock, _guard, projects) = start_daemon().await;
    let dir = tempfile::tempdir().unwrap();
    let project = projects.open(dir.path().to_str().unwrap()).unwrap();
    let client = Client::new(sock);

    let group = client.create_group(project.id, "契约").await.unwrap();
    client
        .add_group_member(group.id, AgentKind::Claude, "claude", "")
        .await
        .unwrap();
    let (human, ph, unknown) = client
        .post_group_message(group.id, "@claude 你好 @ghost")
        .await
        .unwrap();
    assert_eq!(ph.len(), 1);
    assert_eq!(unknown, vec!["ghost".to_string()]);

    // GUI 的轮询语义:after_rev=0 取全部,之后只取变更。
    let mut latest = 0;
    let mut all = Vec::new();
    for _ in 0..100 {
        let (msgs, l) = client
            .list_group_messages(group.id, latest, 200)
            .await
            .unwrap();
        latest = l;
        all.extend(msgs);
        if all
            .iter()
            .any(|m| m.status == Some(GroupMessageStatus::Done))
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    assert!(all.iter().any(|m| m.id == human.id));
    let done = all
        .iter()
        .find(|m| m.status == Some(GroupMessageStatus::Done))
        .expect("agent 回复完成");
    let todo = client
        .push_group_message_to_todo(done.id, "跟进")
        .await
        .unwrap();
    assert_eq!(todo.text, "跟进");
    client
        .cancel_group(group.id, GroupCancelScope::Round)
        .await
        .unwrap();
}
