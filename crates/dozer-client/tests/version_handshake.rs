//! 协议版本握手(S6a Task 4):客户端对版本不一致/过旧的守护进程必须给出
//! 可读错误并拒绝使用;对假服务端(只回 Error 的旧守护进程形状)的行为
//! 也要钉死。全部用临时 UDS 假服务端,不依赖真 dozerd。

use dozer_client::{Client, ClientErrorKind};
use dozer_core::protocol::{Reply, Request, decode_line, encode_line};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;

fn temp_sock(tag: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(format!(
        "/tmp/dz-handshake-{tag}-{}.sock",
        uuid::Uuid::new_v4()
    ))
}

/// 起一个假服务端:对每条请求行,按 respond 闭包生成应答行写回。
async fn fake_daemon(sock: std::path::PathBuf, respond: fn(Request) -> Reply) {
    tokio::spawn(async move {
        if let Ok(listener) = UnixListener::bind(&sock) {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let (r, mut w) = stream.into_split();
                    let mut lines = BufReader::new(r).lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        if let Ok(req) = decode_line::<Request>(&line) {
                            let reply = respond(req);
                            let _ = w.write_all(encode_line(&reply).as_bytes()).await;
                        }
                    }
                });
            }
        }
    });
}

#[tokio::test]
async fn matching_versions_pass_and_the_check_is_cached() {
    let sock = temp_sock("ok");
    fake_daemon(sock.clone(), |req| match req {
        Request::Hello { protocol } => Reply::Hello {
            protocol,
            product: "dozer".into(),
        },
        _ => Reply::Error {
            message: "unexpected".into(),
        },
    })
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
    fake_daemon(sock.clone(), |req| match req {
        Request::Hello { .. } => Reply::Hello {
            protocol: dozer_core::protocol::PROTOCOL_VERSION + 1,
            product: "dozer".into(),
        },
        _ => Reply::Error {
            message: "unexpected".into(),
        },
    })
    .await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let client = Client::new(sock);
    let err = client.check_version().await.unwrap_err();
    match err.kind {
        ClientErrorKind::ProtocolMismatch { client, daemon } => {
            assert_eq!(client, dozer_core::protocol::PROTOCOL_VERSION);
            assert_eq!(daemon, dozer_core::protocol::PROTOCOL_VERSION + 1);
        }
        other => panic!("应报版本不匹配: {other:?}"),
    }
    let msg = err.to_string();
    assert!(msg.contains("协议版本不一致"), "{msg}");
    assert!(msg.contains("升级到同一版本"), "{msg}");
}

#[tokio::test]
async fn old_daemon_that_only_replies_error_is_reported_as_too_old() {
    let sock = temp_sock("old");
    // 旧守护进程的形状:不认识 Hello,一律回 Error
    fake_daemon(sock.clone(), |_| Reply::Error {
        message: "未知请求".into(),
    })
    .await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let client = Client::new(sock);
    let err = client.check_version().await.unwrap_err();
    assert!(
        matches!(err.kind, ClientErrorKind::DaemonTooOld),
        "应报守护进程过旧: {err:?}"
    );
    assert!(err.to_string().contains("过旧"), "{}", err);
    // 失败不缓存:修好守护进程后还能重试
    assert!(client.check_version().await.is_err());
}
