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
