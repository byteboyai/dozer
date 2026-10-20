//! S6a Task 4:hook 转发在协议版本不一致时必须静默放弃——不写 stderr、
//! 退出码 0、只写日志。用 socketpair 假扮守护进程钉住这条行为。

use dozer_core::protocol::{PROTOCOL_VERSION, Reply, Request, decode_line, encode_line};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;

#[test]
fn hook_forward_is_silent_on_mismatch() {
    let (mut hook_side, mut daemon_side) = UnixStream::pair().unwrap();
    // 守护进程扮演者:读 Hello,回一个不一致的版本,然后守着是否还有后续字节。
    let server = std::thread::spawn(move || {
        let mut raw = [0u8; 4096];
        let n = daemon_side.read(&mut raw).unwrap();
        let text = String::from_utf8_lossy(&raw[..n]).to_string();
        let req: Request = decode_line(text.trim_end()).unwrap();
        assert!(matches!(req, Request::Hello { .. }), "{req:?}");
        let reply = Reply::Hello {
            protocol: PROTOCOL_VERSION + 1,
            product: "dozer".into(),
        };
        daemon_side
            .write_all(encode_line(&reply).as_bytes())
            .unwrap();
        // 版本不一致后,不得再收到任何 HookEvent 字节。
        let mut raw2 = [0u8; 64];
        let n2 = daemon_side.read(&mut raw2).unwrap_or(0);
        assert_eq!(n2, 0, "版本不一致后不得再发 HookEvent");
    });

    // 直接往这对 socket 发送事件:内部先握手,版本不一致就静默放弃。
    dozer_hook::forward::send_hook_event_via(
        &mut hook_side,
        dozer_core::protocol::AgentKind::Claude,
        "sess-1",
        "Stop",
        7,
        &serde_json::Value::Null,
    );
    // hook 进程退出即关连接;显式关闭让守候端 read 返回 0(EOF)。
    drop(hook_side);
    server.join().unwrap();
}

#[test]
fn hook_forward_proceeds_when_versions_match() {
    let (mut hook_side, mut daemon_side) = UnixStream::pair().unwrap();
    let server = std::thread::spawn(move || {
        // 读 Hello,回相同版本
        let mut raw = [0u8; 4096];
        let n = daemon_side.read(&mut raw).unwrap();
        let text = String::from_utf8_lossy(&raw[..n]).to_string();
        let req: Request = decode_line(text.trim_end()).unwrap();
        assert!(matches!(req, Request::Hello { .. }), "{req:?}");
        let reply = Reply::Hello {
            protocol: PROTOCOL_VERSION,
            product: "dozer".into(),
        };
        daemon_side
            .write_all(encode_line(&reply).as_bytes())
            .unwrap();
        // 读 HookEvent
        let mut raw2 = [0u8; 4096];
        let n2 = daemon_side.read(&mut raw2).unwrap_or(0);
        assert!(n2 > 0, "握手通过后 HookEvent 必须被发出");
        let text2 = String::from_utf8_lossy(&raw2[..n2]).to_string();
        let second: Request = decode_line(text2.trim_end()).unwrap();
        assert!(matches!(second, Request::HookEvent { .. }), "{second:?}");
    });

    dozer_hook::forward::send_hook_event_via(
        &mut hook_side,
        dozer_core::protocol::AgentKind::Claude,
        "sess-1",
        "Stop",
        7,
        &serde_json::Value::Null,
    );
    server.join().unwrap();
}

#[test]
fn old_daemon_that_replies_error_gets_no_hook_event() {
    let (mut hook_side, mut daemon_side) = UnixStream::pair().unwrap();
    let server = std::thread::spawn(move || {
        let mut raw = [0u8; 4096];
        let n = daemon_side.read(&mut raw).unwrap();
        let text = String::from_utf8_lossy(&raw[..n]).to_string();
        let req: Request = decode_line(text.trim_end()).unwrap();
        assert!(matches!(req, Request::Hello { .. }));
        // 旧守护进程:不认识 Hello,回 Error
        let reply = Reply::Error {
            message: "未知请求".into(),
        };
        daemon_side
            .write_all(encode_line(&reply).as_bytes())
            .unwrap();
        let mut raw2 = [0u8; 64];
        let n2 = daemon_side.read(&mut raw2).unwrap_or(0);
        assert_eq!(n2, 0, "过旧守护进程之后不得再发 HookEvent");
    });

    dozer_hook::forward::send_hook_event_via(
        &mut hook_side,
        dozer_core::protocol::AgentKind::Claude,
        "sess-1",
        "Stop",
        7,
        &serde_json::Value::Null,
    );
    // hook 进程退出即关连接;显式关闭让守候端 read 返回 0(EOF)。
    drop(hook_side);
    server.join().unwrap();
}
