//! 通用 HookEvent 单向发送:把一次 hook 事件编码成 `Request::HookEvent` 并经
//! UDS 发给 dozerd。供 `main.rs` 的 `forward()` 与 `aider_launcher` 复用,
//! 避免 launcher 与普通 forward 各写一套 UDS 代码(见 spec 2026-09-22)。

use dozer_core::protocol::{AgentKind, Request, encode_line};
use serde_json::Value;
use std::io::Write;
use std::time::Duration;

/// 把一次 HookEvent 单向发送给 dozerd。恒静默、恒成功——dozerd 不在、socket
/// 不存在、写入失败/超时一律吞掉,绝不因为上报失败拖慢或阻塞 agent(与
/// `main.rs::forward` 的既有哲学一致,见 spec P1e)。
pub fn send_hook_event(agent: AgentKind, session_id: &str, event: &str, ts_ms: u64, data: &Value) {
    let req = Request::HookEvent {
        session_id: session_id.to_string(),
        agent,
        event: event.to_string(),
        ts_ms,
        data: data.clone(),
    };
    let Ok(mut stream) = std::os::unix::net::UnixStream::connect(dozer_core::paths::socket_path())
    else {
        return;
    };
    let _ = stream.set_write_timeout(Some(Duration::from_millis(200)));
    let _ = stream.write_all(encode_line(&req).as_bytes());
    // 单向语义：不读应答，发完即走
}
