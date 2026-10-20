//! S6a Task 5:产品隔离(两个独立 OS 进程,install 是进程级)。
//! 起两个最小功能的守护进程(一个 Fixed 目录下的 "dozer 替身"、一个
//! "digger"),根目录各自独立:socket/数据库/日志互不重叠、事件不串线、
//! 同根第二个实例必须非零退出且不碰数据、app host 关闭时对 AppRequest
//! 回 Unavailable。

use dozer_client::Client;
use dozer_core::protocol::{Reply, Request, decode_line, encode_line};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

fn example_bin() -> PathBuf {
    // cargo 会给集成测试注入 CARGO_BIN_EXE_<target>(bin 目标);示例目标
    // 没有这个注入,按 cargo 的既定布局自己找。
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_product_daemon") {
        return PathBuf::from(p);
    }
    // cargo 会把示例二进制放进 target/<triple>/debug/examples(本仓库的
    // 默认布局);triple 从 cargo metadata 拿不到时逐个常见位置兜底。
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let target_root = manifest
        .parent()
        .and_then(|c| c.parent())
        .expect("crates/dozerd 的上两级是仓库根")
        .join("target");
    let candidates = [
        target_root.join("debug/examples/product_daemon"),
        target_root.join("aarch64-apple-darwin/debug/examples/product_daemon"),
    ];
    for c in &candidates {
        if c.exists() {
            return c.clone();
        }
    }
    panic!(
        "找不到 product_daemon 示例二进制,先跑 cargo build -p dozerd --example product_daemon: {:?}",
        candidates
    );
}

fn spawn_daemon(name: &str, root: &std::path::Path) -> Child {
    Command::new(example_bin())
        .arg("--name")
        .arg(name)
        .arg("--root")
        .arg(root)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn product_daemon")
}

fn wait_for_socket(sock: &std::path::Path, child: &mut Child) {
    for _ in 0..100 {
        if sock.exists() {
            return;
        }
        if let Some(status) = child.try_wait().expect("wait") {
            panic!("守护进程提前退出: {status}(socket 应在 {sock:?})");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("socket 未在 10s 内出现: {sock:?}");
}

/// 对 socket 发一个请求并读一行应答。
async fn rpc(sock: &std::path::Path, req: Request) -> anyhow::Result<Reply> {
    let mut stream = UnixStream::connect(sock).await?;
    stream.write_all(encode_line(&req).as_bytes()).await?;
    let mut lines = BufReader::new(stream).lines();
    let line = lines
        .next_line()
        .await?
        .ok_or_else(|| anyhow::anyhow!("daemon 断开"))?;
    Ok(decode_line(&line)?)
}

/// 订阅流:attach 之后读若干行(带超时),收集是否收到任何输出。
async fn read_some(sock: &std::path::Path, req: Request, ms: u64) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(mut stream) = UnixStream::connect(sock).await else {
        return out;
    };
    if stream
        .write_all(encode_line(&req).as_bytes())
        .await
        .is_err()
    {
        return out;
    }
    let (r, _) = stream.into_split();
    let mut lines = BufReader::new(r).lines();
    let deadline = tokio::time::Instant::now() + Duration::from_millis(ms);
    loop {
        let Ok(Some(line)) = tokio::time::timeout_at(deadline, lines.next_line())
            .await
            .unwrap_or(Ok(None))
        else {
            break;
        };
        out.push(line);
        if out.len() >= 8 {
            break;
        }
    }
    out
}

#[tokio::test]
async fn isolated_roots_sockets_and_dbs_do_not_overlap() {
    let root_a = tempfile::tempdir().unwrap();
    let root_b = tempfile::tempdir().unwrap();
    let mut a = spawn_daemon("digger", root_a.path());
    let mut b = spawn_daemon("dozer", root_b.path());
    let sock_a = root_a.path().join("state").join("testd.sock");
    let sock_b = root_b.path().join("state").join("testd.sock");
    wait_for_socket(&sock_a, &mut a);
    wait_for_socket(&sock_b, &mut b);

    let db_a = root_a.path().join("state").join("test.db");
    let db_b = root_b.path().join("state").join("test.db");
    assert!(db_a.exists());
    assert!(db_b.exists());
    // 两个根目录互不包含,socket/数据库路径也互不相交
    assert!(!root_a.path().starts_with(root_b.path()));
    assert!(!root_b.path().starts_with(root_a.path()));
    assert_ne!(sock_a, sock_b);
    assert_ne!(db_a, db_b);

    let _ = a.kill();
    let _ = b.kill();
    let _ = a.wait();
    let _ = b.wait();
}

#[tokio::test]
async fn events_do_not_cross_between_two_daemons() {
    let root_a = tempfile::tempdir().unwrap();
    let root_b = tempfile::tempdir().unwrap();
    let mut a = spawn_daemon("digger", root_a.path());
    let mut b = spawn_daemon("dozer", root_b.path());
    let sock_a = root_a.path().join("state").join("testd.sock");
    let sock_b = root_b.path().join("state").join("testd.sock");
    wait_for_socket(&sock_a, &mut a);
    wait_for_socket(&sock_b, &mut b);

    // 先在 A 里建一个会话(事件源),再对 B 的 socket 发 A 会话的事件。
    let created = rpc(
        &sock_a,
        Request::CreateSession {
            name: "n".into(),
            command: "/bin/sh".into(),
            args: vec![],
            cwd: "/".into(),
            cols: 80,
            rows: 24,
            project_id: 0,
            agent: dozer_core::protocol::AgentKind::Unknown,
        },
    )
    .await
    .unwrap();
    let Reply::Created { session } = created else {
        panic!("建会话失败: {created:?}");
    };

    // A 的订阅者应能收到事件(正向对照)
    let got_a = read_some(
        &sock_a,
        Request::Attach {
            session_id: session.id.clone(),
            from_offset: 0,
        },
        400,
    )
    .await;
    assert!(!got_a.is_empty(), "A 自己的订阅者必须能收到事件");

    // 把 A 会话的事件发给 B:attach 不存在的会话,B 必须拒绝/回错误,
    // 绝不能把事件投递出去。
    let cross = rpc(
        &sock_b,
        Request::Attach {
            session_id: session.id.clone(),
            from_offset: 0,
        },
    )
    .await;
    assert!(
        matches!(cross, Ok(Reply::Error { .. })),
        "B 不认识 A 的会话,应回错误: {cross:?}"
    );

    // HookEvent 同理:B 收到 A 会话的事件,只影响 B 自己(会话不存在,
    // 不能误写进任何 A 的数据文件;A 的数据库字节在事件前后不变)。
    let db_a_before = std::fs::read(root_a.path().join("state").join("test.db")).unwrap();
    let _ = rpc(
        &sock_b,
        Request::HookEvent {
            session_id: session.id.clone(),
            agent: dozer_core::protocol::AgentKind::Claude,
            event: "Stop".into(),
            ts_ms: 1,
            data: serde_json::Value::Null,
        },
    )
    .await;
    let db_a_after = std::fs::read(root_a.path().join("state").join("test.db")).unwrap();
    assert_eq!(db_a_before, db_a_after, "发给 B 的事件不得改动 A 的数据库");

    let _ = a.kill();
    let _ = b.kill();
    let _ = a.wait();
    let _ = b.wait();
}

#[tokio::test]
async fn second_daemon_on_the_same_root_exits_without_touching_data() {
    let root = tempfile::tempdir().unwrap();
    let mut first = spawn_daemon("digger", root.path());
    let sock = root.path().join("state").join("testd.sock");
    wait_for_socket(&sock, &mut first);
    let db_before = std::fs::read(root.path().join("state").join("test.db")).unwrap();

    // 同一 root 起第二个:单实例检查必须让它非零退出。
    let mut second = spawn_daemon("digger", root.path());
    let status = loop {
        match second.try_wait().unwrap() {
            Some(s) => break s,
            None => std::thread::sleep(Duration::from_millis(100)),
        }
    };
    assert!(!status.success(), "第二个守护进程必须非零退出: {status}");
    let db_after = std::fs::read(root.path().join("state").join("test.db")).unwrap();
    assert_eq!(db_before, db_after, "第二个实例不得改动数据库字节");

    let _ = first.kill();
    let _ = first.wait();
    let _ = second.wait();
}

#[tokio::test]
async fn app_host_is_unavailable_when_the_feature_is_off() {
    let root = tempfile::tempdir().unwrap();
    let mut d = spawn_daemon("digger", root.path());
    let sock = root.path().join("state").join("testd.sock");
    wait_for_socket(&sock, &mut d);

    let reply = rpc(
        &sock,
        Request::App {
            request: bytehost_apps::proto::AppRequest::List,
        },
    )
    .await
    .unwrap();
    match reply {
        Reply::App { reply } => {
            // 不可用语义:AppReply::Failed 且 kind=Unavailable
            let failed = match reply {
                bytehost_apps::proto::AppReply::Failed { failure } => failure,
                other => panic!("minimal 守护进程对 AppRequest::List 应回 Failed,实得 {other:?}"),
            };
            assert_eq!(
                failed.kind,
                bytehost_apps::proto::AppErrorKind::Unavailable,
                "{failed:?}"
            );
        }
        other => panic!("应回 App 应答: {other:?}"),
    }

    let _ = d.kill();
    let _ = d.wait();
}
