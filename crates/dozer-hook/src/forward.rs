//! 通用 HookEvent 单向发送:把一次 hook 事件编码成 `Request::HookEvent` 并经
//! UDS 发给 dozerd。供 `main.rs` 的 `forward()` 与 `aider_launcher` 复用,
//! 避免 launcher 与普通 forward 各写一套 UDS 代码(见 spec 2026-09-22)。

use dozer_core::protocol::{AgentKind, Reply, Request, PROTOCOL_VERSION, decode_line, encode_line};
use serde_json::Value;
use std::io::{Read, Write};
use std::time::Duration;

dozer_core::scope!(LOG, module, "hook_forward");

/// 把一次 HookEvent 单向发送给 dozerd。恒静默、恒成功——dozerd 不在、socket
/// 不存在、写入失败/超时、协议版本不一致一律吞掉(版本不符时写一条日志,
/// 不写 stderr、退出码 0),绝不因为上报失败拖慢或阻塞 agent(与
/// `main.rs::forward` 的既有哲学一致,见 spec P1e)。
pub fn send_hook_event(agent: AgentKind, session_id: &str, event: &str, ts_ms: u64, data: &Value) {
    let Ok(mut stream) = std::os::unix::net::UnixStream::connect(dozer_core::paths::socket_path())
    else {
        return;
    };
    let _ = stream.set_write_timeout(Some(Duration::from_millis(200)));
    let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
    send_hook_event_via(&mut stream, agent, session_id, event, ts_ms, data);
}

/// send_hook_event 的可测试内核:连接由调用方传入(socketpair 即可扮演
/// 守护进程)。握手失败静默放弃,成功则发出 HookEvent(单向,不读应答)。
pub fn send_hook_event_via(
    stream: &mut std::os::unix::net::UnixStream,
    agent: AgentKind,
    session_id: &str,
    event: &str,
    ts_ms: u64,
    data: &Value,
) {
    if !hello_handshake_ok(stream) {
        return; // 版本不一致或守护进程过旧:静默放弃本次上报
    }
    let req = Request::HookEvent {
        session_id: session_id.to_string(),
        agent,
        event: event.to_string(),
        ts_ms,
        data: data.clone(),
    };
    let _ = stream.write_all(encode_line(&req).as_bytes());
    // HookEvent 之后仍是单向语义：不读应答，发完即走
}

/// 发 Hello 并核对回帧。守护进程版本一致返回 true;回帧不是 Hello
/// (过旧)、版本不一致、读失败都返回 false——hook 对协议分歧的全部
/// 反应只是静默放弃 + 写日志(P1e:绝不往 stderr 塞东西)。
fn hello_handshake_ok(stream: &mut std::os::unix::net::UnixStream) -> bool {
    let hello = Request::Hello {
        protocol: PROTOCOL_VERSION,
    };
    if stream.write_all(encode_line(&hello).as_bytes()).is_err() {
        return false;
    }
    // 只读到换行为止(守护进程保持连接开着,read_to_string 会等到 EOF 卡死)。
    let mut line = String::new();
    let mut byte = [0u8; 1];
    loop {
        match stream.read(&mut byte) {
            Ok(0) => break, // EOF:对面把连接关了
            Ok(_) => {
                line.push(byte[0] as char);
                if byte[0] == b'\n' {
                    break;
                }
            }
            Err(_) => {
                dozer_core::plain_warn!(
                    dozer_core::log::Component::Hook,
                    LOG,
                    "守护进程对协议握手无应答,放弃本次 hook 上报"
                );
                return false;
            }
        }
    }
    if line.is_empty() {
        dozer_core::plain_warn!(
            dozer_core::log::Component::Hook,
            LOG,
            "守护进程对协议握手无应答,放弃本次 hook 上报"
        );
        return false;
    }
    match decode_line::<Reply>(&line) {
        Ok(Reply::Hello { protocol, .. }) if protocol == PROTOCOL_VERSION => true,
        Ok(Reply::Hello { protocol, .. }) => {
            dozer_core::plain_warn!(
                dozer_core::log::Component::Hook,
                LOG,
                "守护进程协议版本不一致(hook={PROTOCOL_VERSION}, daemon={protocol}),放弃本次 hook 上报;请把产品与守护进程升级到同一版本"
            );
            false
        }
        _ => {
            dozer_core::plain_warn!(
                dozer_core::log::Component::Hook,
                LOG,
                "守护进程不认识协议握手(版本过旧),放弃本次 hook 上报"
            );
            false
        }
    }
}
