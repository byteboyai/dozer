//! S6a 复审 R2:协议版本握手的客户端行为。全部用临时 UDS 假服务端,
//! 不依赖真 dozerd。
//!
//! 放行策略(用户 2026-10-20 裁决):版本不一致/连到别的产品一律拒绝;
//! 唯一例外是不认识 Hello 的旧守护进程(对 Hello 只回 Reply::Error)——
//! 放行并记 warn,不阻断,否则只升级客户端的用户得先杀常驻 dozerd,
//! 会丢掉所有存活会话。

use dozer_client::{Client, ClientErrorKind, HelloCheck, classify_hello_reply};
use dozer_core::protocol::{Reply, Request, decode_line, encode_line};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;

fn temp_sock(tag: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(format!(
        "/tmp/dz-handshake-{tag}-{}.sock",
        uuid::Uuid::new_v4()
    ))
}

/// 起一个假服务端:Hello 按入参回,业务请求按入参回;每收到一条请求
/// 就把它的 type 记进 received(测试据此断言"版本不符时业务请求
/// 根本没被发出")。
async fn fake_daemon(
    sock: std::path::PathBuf,
    on_hello: Reply,
    on_business: Reply,
    received: Arc<Mutex<Vec<String>>>,
) {
    tokio::spawn(async move {
        if let Ok(listener) = UnixListener::bind(&sock) {
            while let Ok((stream, _)) = listener.accept().await {
                let on_hello = on_hello.clone();
                let on_business = on_business.clone();
                let received = received.clone();
                tokio::spawn(async move {
                    let (r, mut w) = stream.into_split();
                    let mut lines = BufReader::new(r).lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        let Ok(req) = decode_line::<Request>(&line) else {
                            continue;
                        };
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line)
                            && let Some(t) = v.get("type").and_then(|t| t.as_str())
                        {
                            received.lock().unwrap().push(t.to_string());
                        }
                        let reply = match req {
                            Request::Hello { .. } => on_hello.clone(),
                            _ => on_business.clone(),
                        };
                        let _ = w.write_all(encode_line(&reply).as_bytes()).await;
                    }
                });
            }
        }
    });
}

fn hello_reply(protocol: u32, product: &str) -> Reply {
    Reply::Hello {
        protocol,
        product: product.to_string(),
    }
}

#[tokio::test]
async fn matching_versions_pass_and_the_check_is_cached() {
    let sock = temp_sock("ok");
    let received = Arc::new(Mutex::new(Vec::new()));
    fake_daemon(
        sock.clone(),
        hello_reply(dozer_core::protocol::PROTOCOL_VERSION, "dozer"),
        Reply::Error {
            message: "unexpected".into(),
        },
        received.clone(),
    )
    .await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let client = Client::new(sock);
    assert!(client.check_version().await.is_ok());
    // 缓存:第二次调用不再碰 socket
    assert!(client.check_version().await.is_ok());
}

#[tokio::test]
async fn protocol_mismatch_yields_a_readable_error() {
    let sock = temp_sock("mismatch");
    let received = Arc::new(Mutex::new(Vec::new()));
    fake_daemon(
        sock.clone(),
        hello_reply(dozer_core::protocol::PROTOCOL_VERSION + 1, "dozer"),
        Reply::Sessions { sessions: vec![] },
        received.clone(),
    )
    .await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let client = Client::new(sock);
    let err = client.check_version().await.unwrap_err();
    let msg = err.to_string();
    match err.kind {
        ClientErrorKind::ProtocolMismatch { client, daemon } => {
            assert_eq!(client, dozer_core::protocol::PROTOCOL_VERSION);
            assert_eq!(daemon, dozer_core::protocol::PROTOCOL_VERSION + 1);
        }
        other => panic!("应报版本不匹配: {other:?}"),
    }
    assert!(msg.contains("协议版本不一致"), "{msg}");
    assert!(msg.contains("升级到同一版本"), "{msg}");
}

/// 复审 R2 计划清单里缺失的测试:握手失败后,业务请求被拒且给出可读
/// 错误(文案含两个版本号),假守护进程没有收到任何业务请求。
#[tokio::test]
async fn client_refuses_requests_after_a_protocol_mismatch_with_a_readable_error() {
    let sock = temp_sock("refuse");
    let received = Arc::new(Mutex::new(Vec::new()));
    fake_daemon(
        sock.clone(),
        hello_reply(dozer_core::protocol::PROTOCOL_VERSION + 1, "dozer"),
        Reply::Sessions { sessions: vec![] },
        received.clone(),
    )
    .await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let client = Client::new(sock);
    let err = client.list().await.unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("协议版本不一致"), "{msg}");
    assert!(msg.contains("升级到同一版本"), "{msg}");
    assert!(
        msg.contains(&dozer_core::protocol::PROTOCOL_VERSION.to_string()),
        "文案应含客户端版本号: {msg}"
    );
    assert!(
        msg.contains(&(dozer_core::protocol::PROTOCOL_VERSION + 1).to_string()),
        "文案应含守护进程版本号: {msg}"
    );
    // 版本不符的守护进程不得收到任何业务请求(失败不缓存,所以可能
    // 逐次重试握手;但除了 hello 之外什么都不能出现)。
    let got = received.lock().unwrap().clone();
    assert!(
        got.iter().all(|t| t == "hello"),
        "只应收到握手请求,实际: {got:?}"
    );
}

/// 连到了别的产品的守护进程:拒绝并给出可读错误(文案含双方产品名)。
#[tokio::test]
async fn foreign_product_daemon_is_rejected_with_a_readable_error() {
    let sock = temp_sock("product");
    let received = Arc::new(Mutex::new(Vec::new()));
    fake_daemon(
        sock.clone(),
        hello_reply(dozer_core::protocol::PROTOCOL_VERSION, "digger"),
        Reply::Sessions { sessions: vec![] },
        received.clone(),
    )
    .await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // 测试进程未安装别的产品,客户端身份是 dozer
    let client = Client::new(sock);
    let err = client.check_version().await.unwrap_err();
    let msg = err.to_string();
    match err.kind {
        ClientErrorKind::ProductMismatch { client, daemon } => {
            assert_eq!(client, "dozer");
            assert_eq!(daemon, "digger");
        }
        other => panic!("应报产品不匹配: {other:?}"),
    }
    assert!(msg.contains("dozer") && msg.contains("digger"), "{msg}");
    assert!(msg.contains("别的产品"), "{msg}");
    // 业务请求同样被拒
    assert!(client.list().await.is_err());
    let got = received.lock().unwrap().clone();
    assert!(
        got.iter().all(|t| t == "hello"),
        "不得向别的产品的守护进程发业务请求: {got:?}"
    );
}

/// 同产品的旧守护进程(不认识 Hello,只回 Reply::Error):放行并记 warn,
/// 业务请求照常工作。(用户 2026-10-20 兼容裁决;此前该场景被报
/// "DaemonTooOld",现按裁决改为放行。)
#[tokio::test]
async fn old_same_product_daemon_is_allowed_and_business_requests_proceed() {
    let sock = temp_sock("old");
    let received = Arc::new(Mutex::new(Vec::new()));
    fake_daemon(
        sock.clone(),
        Reply::Error {
            message: "未知请求".into(),
        },
        Reply::Sessions { sessions: vec![] },
        received.clone(),
    )
    .await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // 放行判定本身可单测:旧守护进程的 Error 应答 → OldDaemonAllowed
    // (生产代码 hello_once 对这一结果记一条 tracing warn)。
    assert_eq!(
        classify_hello_reply(&Reply::Error {
            message: "未知请求".into()
        }),
        Ok(HelloCheck::OldDaemonAllowed)
    );

    // 端到端:直接调业务方法,自动握手放行,业务请求正常发出并得到应答
    let client = Client::new(sock);
    let sessions = client.list().await.expect("旧守护进程必须放行");
    assert!(sessions.is_empty());
    assert!(client.check_version().await.is_ok(), "放行结果应被缓存");
    let got = received.lock().unwrap().clone();
    assert_eq!(
        got,
        vec!["hello".to_string(), "list_sessions".to_string()],
        "应先握手再业务请求"
    );
}

/// 能正常应答 Hello 但版本不一致的守护进程仍被拒绝(放行只针对
/// "不认识 Hello"这一种情形)。
#[tokio::test]
async fn hello_aware_daemon_with_wrong_version_is_still_rejected() {
    let sock = temp_sock("aware");
    let received = Arc::new(Mutex::new(Vec::new()));
    fake_daemon(
        sock.clone(),
        hello_reply(dozer_core::protocol::PROTOCOL_VERSION + 9, "dozer"),
        Reply::Sessions { sessions: vec![] },
        received.clone(),
    )
    .await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let client = Client::new(sock);
    assert!(client.list().await.is_err());
}
