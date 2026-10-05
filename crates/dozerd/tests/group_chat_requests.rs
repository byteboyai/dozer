use dozer_client::Client;
use dozer_core::protocol::{AgentKind, GroupCancelScope, GroupMessageStatus};
use dozerd::group_adapter::{BoxFuture, GroupAgentRunner, TurnError, TurnRequest};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;

struct ScriptedRunner;
impl GroupAgentRunner for ScriptedRunner {
    fn run<'a>(
        &'a self,
        req: TurnRequest,
        mut cancel: watch::Receiver<bool>,
    ) -> BoxFuture<'a, Result<String, TurnError>> {
        Box::pin(async move {
            if req.prompt.contains("请卡住") {
                loop {
                    if *cancel.borrow() {
                        return Err(TurnError::Cancelled);
                    }
                    if cancel.changed().await.is_err() {
                        std::future::pending::<()>().await;
                    }
                }
            }
            Ok(format!("收到,工作目录={}", req.project_dir.display()))
        })
    }
}

fn temp_sock() -> std::path::PathBuf {
    std::path::PathBuf::from(format!("/tmp/dz-group-{}.sock", uuid::Uuid::new_v4()))
}

struct CleanupGuard(std::path::PathBuf);
impl Drop for CleanupGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

struct TestDaemon {
    socket: std::path::PathBuf,
    _cleanup: CleanupGuard,
    projects: Arc<dozerd::projects::ProjectStore>,
    _task: tokio::task::JoinHandle<anyhow::Result<()>>,
}

async fn start_daemon() -> TestDaemon {
    let sock = temp_sock();
    let db = std::path::PathBuf::from(format!("/tmp/dz-group-{}.db", uuid::Uuid::new_v4()));
    let projects = Arc::new(dozerd::projects::ProjectStore::new(&db).unwrap());
    let dir_lookup = projects.clone();
    let groups = dozerd::group_service::GroupService::new(
        Arc::new(dozerd::group_store::GroupStore::new(&db).unwrap()),
        Arc::new(ScriptedRunner),
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
    let task = tokio::spawn(async move {
        dozerd::server::serve(
            &s,
            ide_lock_dir.path().to_path_buf(),
            stores,
            dozerd::task_poller::new_in_flight(),
        )
        .await
    });
    for _ in 0..100 {
        if tokio::net::UnixStream::connect(&sock).await.is_ok() {
            return TestDaemon {
                socket: sock.clone(),
                _cleanup: CleanupGuard(sock),
                projects,
                _task: task,
            };
        }
        if task.is_finished() {
            let result = task.await.expect("测试 dozerd task panic");
            panic!("测试 dozerd 在监听前退出: {result:?}");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("等待测试 dozerd 监听超时");
}

async fn poll_until_settled(
    client: &Client,
    gid: i64,
) -> Vec<dozer_core::protocol::GroupMessageInfo> {
    for _ in 0..100 {
        let (msgs, _) = client.list_group_messages(gid, 0, 500).await.unwrap();
        let busy = msgs.iter().any(|m| {
            matches!(
                m.status,
                Some(GroupMessageStatus::Queued) | Some(GroupMessageStatus::Running)
            )
        });
        if !busy {
            return msgs;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    panic!("等待发言完成超时");
}

#[tokio::test]
async fn full_flow_create_post_poll_incrementally() {
    let daemon = start_daemon().await;
    let dir = tempfile::tempdir().unwrap();
    let project = daemon.projects.open(dir.path().to_str().unwrap()).unwrap();
    let client = Client::new(daemon.socket.clone());

    let group = client
        .create_group(project.id, "评审登录方案")
        .await
        .unwrap();
    client
        .add_group_member(group.id, AgentKind::Claude, "claude", "")
        .await
        .unwrap();
    let group = client
        .add_group_member(group.id, AgentKind::Codex, "codex", "")
        .await
        .unwrap();
    assert_eq!(group.members.len(), 2);
    assert!(client.list_groups(project.id).await.unwrap().len() == 1);

    let (human, placeholders, unknown) = client
        .post_group_message(group.id, "@claude @codex @nobody 你们怎么看")
        .await
        .unwrap();
    assert_eq!(placeholders.len(), 2);
    assert_eq!(unknown, vec!["nobody".to_string()]);

    let msgs = poll_until_settled(&client, group.id).await;
    assert_eq!(msgs.len(), 3);
    assert_eq!(msgs[0].id, human.id);
    assert!(
        msgs[1].text.contains("工作目录="),
        "走了项目目录: {}",
        msgs[1].text
    );
    assert_eq!(msgs[2].status, Some(GroupMessageStatus::Done));

    // 增量:拿到 latest_rev 之后没有新变更
    let (_, latest) = client.list_group_messages(group.id, 0, 500).await.unwrap();
    let (delta, same) = client
        .list_group_messages(group.id, latest, 500)
        .await
        .unwrap();
    assert!(delta.is_empty());
    assert_eq!(same, latest);
}

#[tokio::test]
async fn cancel_round_over_the_wire_then_retry() {
    let daemon = start_daemon().await;
    let dir = tempfile::tempdir().unwrap();
    let project = daemon.projects.open(dir.path().to_str().unwrap()).unwrap();
    let client = Client::new(daemon.socket.clone());
    let group = client.create_group(project.id, "t").await.unwrap();
    client
        .add_group_member(group.id, AgentKind::Claude, "claude", "")
        .await
        .unwrap();
    client
        .add_group_member(group.id, AgentKind::Codex, "codex", "")
        .await
        .unwrap();

    client
        .post_group_message(group.id, "@claude @codex 请卡住")
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    client
        .cancel_group(group.id, GroupCancelScope::Round)
        .await
        .unwrap();
    let msgs = poll_until_settled(&client, group.id).await;
    assert_eq!(msgs[1].status, Some(GroupMessageStatus::Cancelled));
    assert_eq!(msgs[2].status, Some(GroupMessageStatus::Cancelled));

    // 重试 claude:提示词里仍含"请卡住",所以会再次卡住,取消即可;这里只验证 retry 把它带回排队/运行
    let m = client.retry_group_message(msgs[1].id).await.unwrap();
    assert_eq!(m.status, Some(GroupMessageStatus::Queued));
    client
        .cancel_group(group.id, GroupCancelScope::Round)
        .await
        .unwrap();
    poll_until_settled(&client, group.id).await;
}

#[tokio::test]
async fn errors_come_back_as_errors() {
    let daemon = start_daemon().await;
    let client = Client::new(daemon.socket.clone());
    assert!(client.post_group_message(424242, "hi").await.is_err());
    assert!(client.create_group(1, "   ").await.is_err());
    assert!(client.delete_group(424242).await.is_err());
}

#[tokio::test]
async fn push_message_to_todo_creates_todo_links_message_and_only_once() {
    let daemon = start_daemon().await;
    let dir = tempfile::tempdir().unwrap();
    let project = daemon.projects.open(dir.path().to_str().unwrap()).unwrap();
    let client = Client::new(daemon.socket.clone());
    let group = client.create_group(project.id, "t").await.unwrap();
    client
        .add_group_member(group.id, AgentKind::Claude, "claude", "")
        .await
        .unwrap();
    client
        .post_group_message(group.id, "@claude 说说")
        .await
        .unwrap();
    let msgs = poll_until_settled(&client, group.id).await;
    let reply = &msgs[1];

    let todo = client
        .push_group_message_to_todo(reply.id, "把登录页加上验证码")
        .await
        .unwrap();
    assert_eq!(todo.text, "把登录页加上验证码");
    assert_eq!(todo.project_id, project.id, "待办落在群所属项目");
    assert_eq!(todo.assigned_agent, None, "群聊不分配任务");

    let (msgs, _) = client.list_group_messages(group.id, 0, 500).await.unwrap();
    assert_eq!(msgs[1].todo_id, Some(todo.id));

    // 同一条消息不能重复转
    assert!(
        client
            .push_group_message_to_todo(reply.id, "再来一遍")
            .await
            .is_err()
    );
    assert_eq!(
        client.list_todos(project.id).await.unwrap().len(),
        1,
        "没有产生第二条待办"
    );
    // 空文本拒绝,且不消耗"已转"名额
    assert!(
        client
            .push_group_message_to_todo(msgs[0].id, "  ")
            .await
            .is_err()
    );
    assert!(
        client
            .push_group_message_to_todo(msgs[0].id, "人类这条也能转")
            .await
            .is_ok()
    );
}
