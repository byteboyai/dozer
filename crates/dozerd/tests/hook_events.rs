//! P1e hook 事件通路端到端：HookEvent 入站 → attach 流收 AgentEvent →
//! ListSessions 可见最新态；未知会话丢弃不报错。

use dozer_core::protocol::{AgentState, Reply, Request, decode_line, encode_line};
use dozerd::registry::SessionRegistry;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

async fn send_req(sock: &std::path::Path, req: &Request) -> Reply {
    let stream = UnixStream::connect(sock).await.expect("connect");
    let (r, mut w) = stream.into_split();
    w.write_all(encode_line(req).as_bytes())
        .await
        .expect("send");
    let mut lines = BufReader::new(r).lines();
    let line = tokio::time::timeout(Duration::from_secs(5), lines.next_line())
        .await
        .expect("reply within 5s")
        .expect("io ok")
        .expect("stream open");
    decode_line(&line).expect("valid reply")
}

#[tokio::test]
async fn hook_event_reaches_attached_client_and_list() {
    let sock = std::env::temp_dir().join(format!("dozerd-hook-{}.sock", uuid::Uuid::new_v4()));
    let registry = Arc::new(SessionRegistry::new());
    tokio::spawn({
        let sock = sock.clone();
        let registry = registry.clone();
        async move { dozerd::server::serve(&sock, registry, test_store()).await }
    });
    for _ in 0..100 {
        if sock.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let created = match send_req(
        &sock,
        &Request::CreateSession {
            name: "t".into(),
            command: "/bin/sh".into(),
            args: vec!["-c".into(), "sleep 5".into()],
            cwd: std::env::temp_dir().to_string_lossy().into_owned(),
            cols: 80,
            rows: 24,
        },
    )
    .await
    {
        Reply::Created { session } => session,
        other => panic!("{other:?}"),
    };

    // 连接 A：attach 并持流
    let stream = UnixStream::connect(&sock).await.expect("connect");
    let (r, mut w) = stream.into_split();
    w.write_all(
        encode_line(&Request::Attach {
            session_id: created.id.clone(),
            from_offset: 0,
        })
        .as_bytes(),
    )
    .await
    .expect("send attach");
    let mut lines = BufReader::new(r).lines();
    let _attached = lines.next_line().await.expect("io").expect("open");

    // 连接 B：hook 事件
    match send_req(
        &sock,
        &Request::HookEvent {
            session_id: created.id.clone(),
            event: "Stop".into(),
            ts_ms: 7,
            data: serde_json::Value::Null,
        },
    )
    .await
    {
        Reply::Ok => {}
        other => panic!("{other:?}"),
    }

    // A 在 PTY 噪音里等到 AgentEvent
    loop {
        let line = tokio::time::timeout(Duration::from_secs(5), lines.next_line())
            .await
            .expect("AgentEvent within 5s")
            .expect("io ok")
            .expect("stream open");
        if let Ok(Reply::AgentEvent { state, event, .. }) = decode_line::<Reply>(&line) {
            assert_eq!(state, AgentState::TurnEnded);
            assert_eq!(event, "Stop");
            break;
        }
    }

    match send_req(&sock, &Request::ListSessions).await {
        Reply::Sessions { sessions } => {
            assert_eq!(sessions[0].agent_state, AgentState::TurnEnded)
        }
        other => panic!("{other:?}"),
    }

    // 未知会话：Ok 不报错
    match send_req(
        &sock,
        &Request::HookEvent {
            session_id: "ghost".into(),
            event: "Stop".into(),
            ts_ms: 8,
            data: serde_json::Value::Null,
        },
    )
    .await
    {
        Reply::Ok => {}
        other => panic!("{other:?}"),
    }
}

/// 每次调用建一个独立临时库的验收存储（测试用；P1f serve 需要）。
fn test_store() -> std::sync::Arc<dozerd::acceptance::AcceptanceStore> {
    let db = std::env::temp_dir().join(format!("dozerd-test-{}.db", uuid::Uuid::new_v4()));
    std::sync::Arc::new(dozerd::acceptance::AcceptanceStore::open(&db).unwrap())
}

#[tokio::test]
async fn record_acceptance_persists() {
    let sock = std::env::temp_dir().join(format!("dozerd-acc-{}.sock", uuid::Uuid::new_v4()));
    let db = std::env::temp_dir().join(format!("dozerd-acc-{}.db", uuid::Uuid::new_v4()));
    let registry = Arc::new(SessionRegistry::new());
    let store = Arc::new(dozerd::acceptance::AcceptanceStore::open(&db).unwrap());
    tokio::spawn({
        let (sock, registry, store) = (sock.clone(), registry.clone(), store.clone());
        async move { dozerd::server::serve(&sock, registry, store).await }
    });
    for _ in 0..100 {
        if sock.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    match send_req(
        &sock,
        &Request::RecordAcceptance {
            repo: "/r".into(),
            goal: "g".into(),
            criteria_checked: vec![],
            verdict: "accepted".into(),
            comment: "".into(),
            ref_name: "refs/dozer/accepted/1".into(),
            ts_ms: 1,
        },
    )
    .await
    {
        Reply::Ok => {}
        other => panic!("{other:?}"),
    }
    assert_eq!(store.count().unwrap(), 1);
}
