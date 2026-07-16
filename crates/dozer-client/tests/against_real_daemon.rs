use dozer_client::{Client, TermEvent};
use dozerd::registry::SessionRegistry;
use std::sync::Arc;
use std::time::Duration;

/// macOS SUN_LEN 上限 104 字节，用 /tmp 短路径
fn short_temp_sock() -> std::path::PathBuf {
    let nonce: String = (0..6).map(|_| (b'a' + rand() % 26) as char).collect();
    std::path::PathBuf::from(format!("/tmp/dz-{nonce}.sock"))
}
fn rand() -> u8 {
    use std::time::{SystemTime, UNIX_EPOCH};
    (SystemTime::now().duration_since(UNIX_EPOCH).unwrap().subsec_nanos() & 0xFF) as u8
}

async fn start_daemon() -> std::path::PathBuf {
    let sock = short_temp_sock();
    let registry = Arc::new(SessionRegistry::new());
    let s = sock.clone();
    tokio::spawn(async move { dozerd::server::serve(&s, registry).await });
    for _ in 0..100 {
        if sock.exists() { break; }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    sock
}

#[tokio::test]
async fn full_client_lifecycle() {
    let sock = start_daemon().await;
    let c = Client::new(sock);
    assert!(c.list().await.unwrap().is_empty());

    let info = c.create("测试", "/bin/sh", &["-c".into(), "echo ready; cat".into()],
                        "/tmp", 80, 24).await.unwrap();
    assert!(info.alive);

    let (snap, _next, mut rx) = c.attach(&info.id, 0).await.unwrap();
    let mut seen = snap;
    while !seen.windows(5).any(|w| w == b"ready") {
        match tokio::time::timeout(Duration::from_secs(3), rx.recv()).await {
            Ok(Some(TermEvent::Output(d))) => seen.extend(d),
            other => panic!("expect output, got {other:?}"),
        }
    }

    c.write(&info.id, b"pong\n").await.unwrap();
    let mut echoed = Vec::new();
    while !echoed.windows(4).any(|w| w == b"pong") {
        match tokio::time::timeout(Duration::from_secs(3), rx.recv()).await {
            Ok(Some(TermEvent::Output(d))) => echoed.extend(d),
            other => panic!("expect echo, got {other:?}"),
        }
    }

    c.resize(&info.id, 100, 30).await.unwrap();
    c.kill(&info.id).await.unwrap();
    // kill 后 attach 流应收到 Exited
    let mut exited = false;
    for _ in 0..50 {
        match tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
            Ok(Some(TermEvent::Exited(_))) => { exited = true; break; }
            Ok(Some(_)) => continue,
            _ => continue,
        }
    }
    assert!(exited);
    assert!(!c.list().await.unwrap()[0].alive);
}
