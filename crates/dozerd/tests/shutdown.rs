use dozer_core::protocol::{Reply, Request, decode_line, encode_line};
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

struct Client {
    lines: tokio::io::Lines<BufReader<tokio::net::unix::OwnedReadHalf>>,
    w: tokio::net::unix::OwnedWriteHalf,
}

impl Client {
    async fn connect(sock: &PathBuf) -> Client {
        let stream = UnixStream::connect(sock).await.expect("connect dozerd");
        let (r, w) = stream.into_split();
        Client {
            lines: BufReader::new(r).lines(),
            w,
        }
    }

    async fn send(&mut self, req: &Request) {
        self.w
            .write_all(encode_line(req).as_bytes())
            .await
            .expect("send");
    }

    async fn recv(&mut self) -> Reply {
        let line = tokio::time::timeout(Duration::from_secs(5), self.lines.next_line())
            .await
            .expect("reply within 5s")
            .expect("io ok")
            .expect("stream open");
        decode_line(&line).expect("valid reply")
    }
}

async fn start_test_daemon() -> PathBuf {
    // UDS 路径必须短于 SUN_LEN(~104 字节),macOS 的 `std::env::temp_dir()`
    // 是 `/var/folders/...` 长路径,所以 socket 固定放 `/tmp`,与
    // `against_real_daemon.rs` 的约定一致。
    let short = &uuid::Uuid::new_v4().to_string()[..8];
    let sock = PathBuf::from(format!("/tmp/dz-shutdown-{short}.sock"));
    let ide_lock_dir = tempfile::tempdir().expect("ide_lock_dir tempdir");
    let s = sock.clone();
    tokio::spawn(async move {
        if let Err(e) = dozerd::server::serve(
            &s,
            ide_lock_dir.path().to_path_buf(),
            dozerd::server::Stores {
                registry: test_registry(),
                projects: test_projects(),
                bookmarks: test_bookmarks(),
                code_health: test_code_health(),
                transcripts: test_transcripts(),
                session_summaries: test_session_summaries(),
                summary_jobs: test_summary_jobs(),
                backfill_registry: test_backfill_registry(),
                todos: test_todos(),
                categories: test_categories(),
                memories: test_memories(),
                file_edit_history: test_file_edit_history(),
                groups: test_group_service(),
            },
            dozerd::task_poller::new_in_flight(),
        )
        .await
        {
            eprintln!("serve 启动错误: {e}");
        }
    });
    for _ in 0..100 {
        if sock.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    sock
}

fn create_long_running_session_req() -> Request {
    Request::CreateSession {
        name: "跑着的会话".into(),
        command: "/bin/sh".into(),
        args: vec!["-c".into(), "sleep 30".into()],
        cwd: std::env::temp_dir().to_string_lossy().into_owned(),
        cols: 80,
        rows: 24,
        project_id: 1,
        agent: dozer_core::protocol::AgentKind::Unknown,
    }
}

#[tokio::test]
async fn shutdown_with_no_sessions_exits_daemon_promptly() {
    let sock = start_test_daemon().await;
    let mut c = Client::connect(&sock).await;

    let started = std::time::Instant::now();
    c.send(&Request::Shutdown).await;
    assert_eq!(c.recv().await, Reply::Ok);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "0 会话时应该几乎立刻收尾完成，不应该跑满 60s 常量"
    );

    let mut removed = false;
    for _ in 0..100 {
        if !sock.exists() {
            removed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(removed, "daemon 应在 Shutdown 收尾后删除 socket 文件并退出");
}

#[tokio::test]
async fn shutdown_with_running_session_exits_promptly_and_kills_it() {
    let sock = start_test_daemon().await;
    let mut c1 = Client::connect(&sock).await;
    c1.send(&create_long_running_session_req()).await;
    let Reply::Created { session } = c1.recv().await else {
        panic!("expect Created")
    };

    let started = std::time::Instant::now();
    c1.send(&Request::Shutdown).await;
    assert_eq!(c1.recv().await, Reply::Ok);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "Shutdown 不应等待模型,应立即收尾退出(spec 第 3 节)"
    );

    let mut removed = false;
    for _ in 0..100 {
        if !sock.exists() {
            removed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(removed, "daemon 应在 Shutdown 收尾后删除 socket 并退出");
    let _ = session;
}

#[tokio::test]
async fn duplicate_shutdown_rejected_while_draining() {
    let sock = start_test_daemon().await;
    let mut c1 = Client::connect(&sock).await;
    let mut c2 = Client::connect(&sock).await;
    c1.send(&Request::Shutdown).await;
    assert_eq!(c1.recv().await, Reply::Ok);
    c2.send(&Request::Shutdown).await;
    let Reply::Error { message } = c2.recv().await else {
        panic!("draining 期间重复 Shutdown 应该被拒绝")
    };
    assert!(
        message.contains("停止"),
        "错误文案应说明正在停止: {message}"
    );
}

#[tokio::test]
async fn create_session_rejected_while_draining() {
    let sock = start_test_daemon().await;
    let mut c1 = Client::connect(&sock).await;
    let mut c2 = Client::connect(&sock).await;
    c1.send(&Request::Shutdown).await;
    assert_eq!(c1.recv().await, Reply::Ok);
    c2.send(&create_long_running_session_req()).await;
    let Reply::Error { message } = c2.recv().await else {
        panic!("draining 期间 CreateSession 应该被拒绝")
    };
    assert!(
        message.contains("停止"),
        "错误文案应说明正在停止: {message}"
    );
}

fn test_registry() -> std::sync::Arc<dozerd::registry::SessionRegistry> {
    std::sync::Arc::new(dozerd::registry::SessionRegistry::new())
}

fn test_projects() -> std::sync::Arc<dozerd::projects::ProjectStore> {
    let db = std::env::temp_dir().join(format!("dozerd-test-{}.db", uuid::Uuid::new_v4()));
    std::sync::Arc::new(dozerd::projects::ProjectStore::new(&db).unwrap())
}

fn test_bookmarks() -> std::sync::Arc<dozerd::bookmarks::BookmarkStore> {
    let db = std::env::temp_dir().join(format!("dozerd-test-{}.db", uuid::Uuid::new_v4()));
    std::sync::Arc::new(dozerd::bookmarks::BookmarkStore::new(&db).unwrap())
}

/// 每次调用建独立临时库的代码健康度存储（测试用；serve 需要）。
fn test_code_health() -> std::sync::Arc<dozerd::code_health::CodeHealthStore> {
    let db = std::env::temp_dir().join(format!("dozerd-test-{}.db", uuid::Uuid::new_v4()));
    std::sync::Arc::new(dozerd::code_health::CodeHealthStore::new(&db).unwrap())
}

fn test_transcripts() -> std::sync::Arc<dozerd::transcripts::TranscriptStore> {
    let db = std::env::temp_dir().join(format!("dozerd-test-{}.db", uuid::Uuid::new_v4()));
    std::sync::Arc::new(dozerd::transcripts::TranscriptStore::open(&db).unwrap())
}

fn test_session_summaries() -> std::sync::Arc<dozerd::session_summary::SessionSummaryStore> {
    let db = std::env::temp_dir().join(format!("dozerd-test-{}.db", uuid::Uuid::new_v4()));
    std::sync::Arc::new(dozerd::session_summary::SessionSummaryStore::open(&db).unwrap())
}

fn test_backfill_registry() -> std::sync::Arc<dozerd::session_summary_backfill::BackfillRegistry> {
    std::sync::Arc::new(dozerd::session_summary_backfill::BackfillRegistry::new())
}

fn test_summary_jobs() -> std::sync::Arc<dozerd::summary_jobs::SummaryJobStore> {
    let db = std::env::temp_dir().join(format!("dozerd-sumjob-{}.db", uuid::Uuid::new_v4()));
    std::sync::Arc::new(dozerd::summary_jobs::SummaryJobStore::open(&db).unwrap())
}

fn test_todos() -> std::sync::Arc<dozerd::todo::TodoStore> {
    let db = std::env::temp_dir().join(format!("dozerd-test-{}.db", uuid::Uuid::new_v4()));
    std::sync::Arc::new(dozerd::todo::TodoStore::new(&db).unwrap())
}

fn test_categories() -> std::sync::Arc<dozerd::todo_category::CategoryStore> {
    let db = std::env::temp_dir().join(format!("dozerd-cat-{}.db", uuid::Uuid::new_v4()));
    std::sync::Arc::new(dozerd::todo_category::CategoryStore::new(&db).unwrap())
}

fn test_memories() -> std::sync::Arc<dozerd::memory::MemoryStore> {
    let db = std::env::temp_dir().join(format!("dozerd-mem-{}.db", uuid::Uuid::new_v4()));
    std::sync::Arc::new(dozerd::memory::MemoryStore::new(&db).unwrap())
}

fn test_file_edit_history() -> std::sync::Arc<dozerd::file_edit_history::FileEditHistoryStore> {
    let db = std::env::temp_dir().join(format!("dozerd-feh-{}.db", uuid::Uuid::new_v4()));
    std::sync::Arc::new(dozerd::file_edit_history::FileEditHistoryStore::new(&db).unwrap())
}

fn test_group_service() -> std::sync::Arc<dozerd::group_service::GroupService> {
    dozerd::group_service::GroupService::for_tests()
}
