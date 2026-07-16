use dozer_core::protocol::{Reply, Request, decode_line, encode_line};
use dozerd::registry::SessionRegistry;
use std::path::PathBuf;
use std::sync::Arc;
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
        Client { lines: BufReader::new(r).lines(), w }
    }

    async fn send(&mut self, req: &Request) {
        self.w.write_all(encode_line(req).as_bytes()).await.expect("send");
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

fn b64(s: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(s)
}

fn from_b64(s: &str) -> Vec<u8> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(s).expect("valid b64")
}

#[tokio::test]
async fn session_survives_client_disconnect() {
    let sock = std::env::temp_dir().join(format!("dozerd-test-{}.sock", uuid::Uuid::new_v4()));
    let registry = Arc::new(SessionRegistry::new());
    let server = tokio::spawn({
        let sock = sock.clone();
        let registry = registry.clone();
        async move { dozerd::server::serve(&sock, registry).await }
    });
    // 等 socket 就绪
    for _ in 0..100 {
        if sock.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // 客户端 1：建会话（echo hello 后 cat 保持存活），attach 应看到 hello
    let mut c1 = Client::connect(&sock).await;
    c1.send(&Request::CreateSession {
        name: "主线".into(),
        command: "/bin/sh".into(),
        args: vec!["-c".into(), "echo hello; cat".into()],
        cwd: std::env::temp_dir().to_string_lossy().into_owned(),
        cols: 80,
        rows: 24,
    })
    .await;
    let Reply::Created { session } = c1.recv().await else { panic!("expect Created") };
    let sid = session.id;

    c1.send(&Request::Attach { session_id: sid.clone(), from_offset: 0 }).await;
    let mut seen = Vec::new();
    // Attached 的 snapshot 可能尚未含 hello（竞态），继续吃 Output 直到看到
    loop {
        match c1.recv().await {
            Reply::Attached { snapshot_b64, .. } => seen.extend(from_b64(&snapshot_b64)),
            Reply::Output { data_b64, .. } => seen.extend(from_b64(&data_b64)),
            other => panic!("unexpected: {other:?}"),
        }
        if seen.windows(5).any(|w| w == b"hello") {
            break;
        }
    }

    // 客户端 1 断连（模拟关窗/崩溃）
    drop(c1);
    tokio::time::sleep(Duration::from_millis(300)).await;

    // 客户端 2：重连——会话必须还在、滚屏必须还有 hello（会话存活语义）
    let mut c2 = Client::connect(&sock).await;
    c2.send(&Request::ListSessions).await;
    let Reply::Sessions { sessions } = c2.recv().await else { panic!("expect Sessions") };
    assert_eq!(sessions.len(), 1);
    assert!(sessions[0].alive, "session must survive client disconnect");

    c2.send(&Request::Attach { session_id: sid.clone(), from_offset: 0 }).await;
    let Reply::Attached { snapshot_b64, .. } = c2.recv().await else { panic!("expect Attached") };
    let snap = from_b64(&snapshot_b64);
    assert!(snap.windows(5).any(|w| w == b"hello"), "scrollback survives disconnect");

    // attach 状态下写入：cat 回显 roundtrip
    c2.send(&Request::Write { session_id: sid.clone(), data_b64: b64("roundtrip\n") }).await;
    let mut echoed = Vec::new();
    loop {
        match c2.recv().await {
            Reply::Ok => continue, // Write 的应答
            Reply::Output { data_b64, .. } => {
                echoed.extend(from_b64(&data_b64));
                if echoed.windows(9).any(|w| w == b"roundtrip") {
                    break;
                }
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    // kill 后收到 Exited；ListSessions 显示 alive=false
    c2.send(&Request::Kill { session_id: sid.clone() }).await;
    let mut exited = false;
    for _ in 0..50 {
        match tokio::time::timeout(Duration::from_millis(200), c2.recv()).await {
            Ok(Reply::Exited { .. }) => {
                exited = true;
                break;
            }
            Ok(_) => continue,
            Err(_) => continue,
        }
    }
    assert!(exited, "should receive Exited after kill");

    server.abort();
    let _ = std::fs::remove_file(&sock);
}

#[tokio::test]
async fn unknown_session_returns_error_reply() {
    let sock = std::env::temp_dir().join(format!("dozerd-test-{}.sock", uuid::Uuid::new_v4()));
    let registry = Arc::new(SessionRegistry::new());
    let server = tokio::spawn({
        let sock = sock.clone();
        async move { dozerd::server::serve(&sock, registry).await }
    });
    for _ in 0..100 {
        if sock.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let mut c = Client::connect(&sock).await;
    c.send(&Request::Attach { session_id: "ghost".into(), from_offset: 0 }).await;
    let Reply::Error { message } = c.recv().await else { panic!("expect Error") };
    assert!(message.contains("ghost"));
    server.abort();
    let _ = std::fs::remove_file(&sock);
}

/// 验证 Attach 的"先 subscribe 后取快照"顺序不会导致同一段输出被投递两次：
/// snapshot 与随后的 broadcast Output 事件之间必须以水位（sent_until）去重。
#[tokio::test]
async fn attach_delivers_marker_exactly_once() {
    let sock = std::env::temp_dir().join(format!("dozerd-test-{}.sock", uuid::Uuid::new_v4()));
    let registry = Arc::new(SessionRegistry::new());
    let server = tokio::spawn({
        let sock = sock.clone();
        async move { dozerd::server::serve(&sock, registry).await }
    });
    for _ in 0..100 {
        if sock.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let mut c = Client::connect(&sock).await;
    c.send(&Request::CreateSession {
        name: "标记".into(),
        command: "/bin/sh".into(),
        args: vec!["-c".into(), "printf once_marker; cat".into()],
        cwd: std::env::temp_dir().to_string_lossy().into_owned(),
        cols: 80,
        rows: 24,
    })
    .await;
    let Reply::Created { session } = c.recv().await else { panic!("expect Created") };
    let sid = session.id;

    // 等输出落缓冲（PTY 输出是异步的），确保 marker 在 attach 前已完整写入
    tokio::time::sleep(Duration::from_millis(400)).await;

    c.send(&Request::Attach { session_id: sid.clone(), from_offset: 0 }).await;

    // 收集 snapshot + 后续 Output 事件的字节，直到连续 500ms 无新事件（视为静默）
    let mut collected = Vec::new();
    loop {
        match tokio::time::timeout(Duration::from_millis(500), c.recv()).await {
            Ok(Reply::Attached { snapshot_b64, .. }) => collected.extend(from_b64(&snapshot_b64)),
            Ok(Reply::Output { data_b64, .. }) => collected.extend(from_b64(&data_b64)),
            Ok(other) => panic!("unexpected: {other:?}"),
            Err(_) => break, // 500ms 静默
        }
    }

    let marker = b"once_marker";
    let occurrences = collected.windows(marker.len()).filter(|w| *w == marker).count();
    assert_eq!(
        occurrences, 1,
        "marker 应恰好出现一次，实际出现 {occurrences} 次；collected={collected:?}"
    );

    server.abort();
    let _ = std::fs::remove_file(&sock);
}

/// 验证从中间 offset 重新 attach 时，只返回窗口内未投递的尾部字节（单锁 read_from_with_next）。
#[tokio::test]
async fn attach_from_offset_resumes_within_window() {
    let sock = std::env::temp_dir().join(format!("dozerd-test-{}.sock", uuid::Uuid::new_v4()));
    let registry = Arc::new(SessionRegistry::new());
    let server = tokio::spawn({
        let sock = sock.clone();
        async move { dozerd::server::serve(&sock, registry).await }
    });
    for _ in 0..100 {
        if sock.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let mut c1 = Client::connect(&sock).await;
    c1.send(&Request::CreateSession {
        name: "断点续传".into(),
        command: "/bin/sh".into(),
        args: vec!["-c".into(), "printf 0123456789; cat".into()],
        cwd: std::env::temp_dir().to_string_lossy().into_owned(),
        cols: 80,
        rows: 24,
    })
    .await;
    let Reply::Created { session } = c1.recv().await else { panic!("expect Created") };
    let sid = session.id;

    // 等输出落缓冲
    tokio::time::sleep(Duration::from_millis(400)).await;

    // 第一次 attach：全量快照 + 记录 next_offset
    c1.send(&Request::Attach { session_id: sid.clone(), from_offset: 0 }).await;
    let Reply::Attached { snapshot_b64: first_snapshot_b64, next_offset, .. } = c1.recv().await
    else {
        panic!("expect Attached")
    };
    let first_snapshot = from_b64(&first_snapshot_b64);
    assert!(first_snapshot.len() as u64 >= next_offset.min(4), "首次快照应至少覆盖尾部窗口");

    // 客户端 1 断连
    drop(c1);
    tokio::time::sleep(Duration::from_millis(100)).await;

    // 第二次 attach：从 next_offset - 4 处续传
    let from_offset = next_offset - 4;
    let mut c2 = Client::connect(&sock).await;
    c2.send(&Request::Attach { session_id: sid.clone(), from_offset }).await;
    let Reply::Attached { snapshot_b64: second_snapshot_b64, next_offset: next_offset2, .. } =
        c2.recv().await
    else {
        panic!("expect Attached")
    };
    let second_snapshot = from_b64(&second_snapshot_b64);

    // 续传窗口必须与第一次全量快照的尾部严格一致（避免 PTY 回显等噪声干扰断言）
    let want_len = (next_offset - from_offset) as usize;
    let expected_tail = &first_snapshot[first_snapshot.len() - want_len..];
    assert_eq!(second_snapshot, expected_tail, "续传字节应等于首次快照的尾部窗口");
    assert_eq!(next_offset2, next_offset, "续传时 next_offset 不应变化（无新输出）");

    server.abort();
    let _ = std::fs::remove_file(&sock);
}
