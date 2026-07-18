use dozer_client::{Client, TermEvent};
use dozerd::registry::SessionRegistry;
use std::sync::Arc;
use std::time::Duration;

fn temp_sock() -> (std::path::PathBuf, uuid::Uuid) {
    let id = uuid::Uuid::new_v4();
    let short = &id.to_string()[..8];
    let path = std::path::PathBuf::from(format!("/tmp/dz-{short}.sock"));
    (path, id)
}

async fn start_daemon() -> (std::path::PathBuf, Arc<SessionRegistry>, CleanupGuard) {
    let (sock, id) = temp_sock();
    let registry = Arc::new(SessionRegistry::new());
    let s = sock.clone();
    let r = registry.clone();
    tokio::spawn(async move { dozerd::server::serve(&s, r).await });
    for _ in 0..100 {
        if sock.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    (sock, registry, CleanupGuard(id))
}

struct CleanupGuard(uuid::Uuid);
impl Drop for CleanupGuard {
    fn drop(&mut self) {
        let short = &self.0.to_string()[..8];
        let path = std::path::PathBuf::from(format!("/tmp/dz-{short}.sock"));
        let _ = std::fs::remove_file(&path);
    }
}

#[tokio::test]
async fn full_client_lifecycle() {
    let (sock, _registry, _guard) = start_daemon().await;
    let c = Client::new(sock);
    assert!(c.list().await.unwrap().is_empty());

    let info = c
        .create(
            "测试",
            "/bin/sh",
            &["-c".into(), "echo ready; cat".into()],
            "/tmp",
            80,
            24,
        )
        .await
        .unwrap();
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
            Ok(Some(TermEvent::Exited(_))) => {
                exited = true;
                break;
            }
            Ok(Some(_)) => continue,
            _ => continue,
        }
    }
    assert!(exited);
    assert!(!c.list().await.unwrap()[0].alive);
}

/// 审阅 Important：attach() 内部读任务过去只在“收到新行后 tx.send 失败”
/// 才退出。空闲会话（无输出）的 tab 被关闭、receiver 被 drop 后，读任务
/// 会永远卡在 `lines.next_line().await` 上——UnixStream 不关，daemon 侧
/// `handle_conn` 也不退出，两端各滞留一个任务 + 一个 FD。
///
/// 用 `Session::subscriber_count()`（daemon 侧 broadcast 订阅数，
/// `handle_conn` 在 attach 期间持有、连接关闭时随之 drop）直接观察修复
/// 是否把资源释放传导了回去——这是比“反复 attach 不阻塞”更强的断言，
/// 不需要改 `attach()` 的公开签名。同时保留任务描述里要求的行为断言
/// （20 轮 attach→drop 后第 21 次 attach 仍需在 1s 内即时成功）作为兜底。
#[tokio::test]
async fn reader_task_exits_when_receiver_dropped() {
    let (sock, registry, _guard) = start_daemon().await;
    let c = Client::new(sock);

    // 空闲会话：sleep 30 不产生任何输出，确保读循环唯一的退出信号
    // 只能来自 receiver 被 drop（不会被 stop 事件误触发退出）。
    let info = c
        .create(
            "idle",
            "/bin/sh",
            &["-c".into(), "sleep 30".into()],
            "/tmp",
            80,
            24,
        )
        .await
        .unwrap();
    let session = registry.get(&info.id).expect("session just created");
    assert_eq!(session.subscriber_count(), 0, "尚未 attach，不应有订阅");

    for round in 0..20 {
        let (_snap, _next, rx) = c.attach(&info.id, 0).await.unwrap();
        assert_eq!(
            session.subscriber_count(),
            1,
            "round {round}: attach 应产生一个订阅"
        );
        drop(rx); // 模拟关闭 tab：不再有人消费 TermEvent

        // 给读任务一点时间在 tokio::select! 里观察到 tx.closed() 并退出，
        // 进而 lines/writer 一并 drop → UnixStream 两端关闭 → daemon 侧
        // handle_conn 在下一次 next_line() 收到 EOF 退出 → sub 被 drop。
        let mut released = false;
        for _ in 0..100 {
            if session.subscriber_count() == 0 {
                released = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            released,
            "round {round}: receiver drop 500ms 后 daemon 侧订阅仍未释放，读任务/连接滞留"
        );
    }

    // 硬断言：即便订阅计数这条信号出于某种原因不够灵敏，也要保证
    // 20 轮滞留不会让后续 attach 阻塞——1s 超时兜底暴露卡死。
    let (_snap, _next, rx21) = tokio::time::timeout(Duration::from_secs(1), c.attach(&info.id, 0))
        .await
        .expect("第 21 次 attach 不应超时/阻塞")
        .expect("第 21 次 attach 应成功返回 Attached");
    drop(rx21);

    c.kill(&info.id).await.unwrap();
}
