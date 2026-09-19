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
        dozerd::server::serve(
            &s,
            ide_lock_dir.path().to_path_buf(),
            test_registry(),
            test_projects(),
            test_bookmarks(),
            test_transcripts(),
            test_session_summaries(),
            test_backfill_registry(),
            test_todos(),
            test_categories(),
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
async fn create_session_rejected_while_draining() {
    let sock = start_test_daemon().await;

    let mut c1 = Client::connect(&sock).await;
    c1.send(&create_long_running_session_req()).await;
    let Reply::Created { session } = c1.recv().await else {
        panic!("expect Created")
    };
    let sid = session.id;

    let mut c_shutdown = Client::connect(&sock).await;
    c_shutdown.send(&Request::Shutdown).await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut c_probe = Client::connect(&sock).await;
    c_probe.send(&create_long_running_session_req()).await;
    let Reply::Error { message } = c_probe.recv().await else {
        panic!("draining 期间 CreateSession 应该被拒绝")
    };
    assert!(
        message.contains("停止"),
        "错误文案应说明正在停止: {message}"
    );

    let mut c_finish = Client::connect(&sock).await;
    c_finish
        .send(&Request::RecordSessionSummary {
            session_id: sid,
            title: "标题".into(),
            summary: "摘要".into(),
        })
        .await;
    assert_eq!(c_finish.recv().await, Reply::Ok);

    assert_eq!(c_shutdown.recv().await, Reply::Ok);
}

#[tokio::test]
async fn duplicate_shutdown_rejected_while_draining() {
    let sock = start_test_daemon().await;

    let mut c1 = Client::connect(&sock).await;
    c1.send(&create_long_running_session_req()).await;
    let Reply::Created { session } = c1.recv().await else {
        panic!("expect Created")
    };
    let sid = session.id;

    let mut c_shutdown = Client::connect(&sock).await;
    c_shutdown.send(&Request::Shutdown).await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut c_dup = Client::connect(&sock).await;
    c_dup.send(&Request::Shutdown).await;
    let Reply::Error { message } = c_dup.recv().await else {
        panic!("draining 期间重复 Shutdown 应该被拒绝")
    };
    assert!(
        message.contains("停止"),
        "错误文案应说明正在停止: {message}"
    );

    let mut c_finish = Client::connect(&sock).await;
    c_finish
        .send(&Request::RecordSessionSummary {
            session_id: sid,
            title: "标题".into(),
            summary: "摘要".into(),
        })
        .await;
    assert_eq!(c_finish.recv().await, Reply::Ok);

    assert_eq!(c_shutdown.recv().await, Reply::Ok);
}

#[tokio::test]
async fn recorded_summary_is_not_overwritten_by_heuristic_before_shutdown_completes() {
    let sock = start_test_daemon().await;

    let mut c1 = Client::connect(&sock).await;
    c1.send(&create_long_running_session_req()).await;
    let Reply::Created { session } = c1.recv().await else {
        panic!("expect Created")
    };
    let sid = session.id;

    let mut c_shutdown = Client::connect(&sock).await;
    c_shutdown.send(&Request::Shutdown).await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut c_finish = Client::connect(&sock).await;
    c_finish
        .send(&Request::RecordSessionSummary {
            session_id: sid.clone(),
            title: "真实标题".into(),
            summary: "真实摘要".into(),
        })
        .await;
    assert_eq!(c_finish.recv().await, Reply::Ok);

    // 在 daemon 真正退出、socket 消失之前查一次——此时 drain 正在等
    // 2s 轮询间隔醒来，socket 还活着。
    let mut c_check = Client::connect(&sock).await;
    c_check
        .send(&Request::GetSessionSummary {
            session_id: sid.clone(),
        })
        .await;
    let Reply::SessionSummary { summary } = c_check.recv().await else {
        panic!("expect SessionSummary reply")
    };
    let summary = summary.expect("summary should exist");
    assert_eq!(
        summary.status,
        dozer_core::protocol::SummaryStatus::AiGenerated,
        "已经真实提交的总结不应该被超时兜底覆盖"
    );
    assert_eq!(summary.title, "真实标题");

    assert_eq!(c_shutdown.recv().await, Reply::Ok);
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

fn test_todos() -> std::sync::Arc<dozerd::todo::TodoStore> {
    let db = std::env::temp_dir().join(format!("dozerd-test-{}.db", uuid::Uuid::new_v4()));
    std::sync::Arc::new(dozerd::todo::TodoStore::new(&db).unwrap())
}

fn test_categories() -> std::sync::Arc<dozerd::todo_category::CategoryStore> {
    let db = std::env::temp_dir().join(format!("dozerd-cat-{}.db", uuid::Uuid::new_v4()));
    std::sync::Arc::new(dozerd::todo_category::CategoryStore::new(&db).unwrap())
}
