mod install;

use dozer_core::protocol::{Request, encode_line};
use std::io::{Read, Write};
use std::time::Duration;

fn main() {
    let arg = std::env::args().nth(1);
    match arg.as_deref() {
        Some("install") => std::process::exit(install::run_at(&install::settings_path(), true)),
        Some("uninstall") => std::process::exit(install::run_at(&install::settings_path(), false)),
        other => forward(other.unwrap_or("unknown")),
    }
}

/// 把一次 hook 调用转发给 dozerd。恒静默、恒成功退出——绝不拖慢 agent
/// （spec P1e 错误处理：dozerd 不在/超时/畸形输入一律吞掉）。
fn forward(event_arg: &str) {
    // 非 Dozer 会话（无注入的会话 id）：与 dozerd 无关，直接退出
    let Ok(session_id) = std::env::var("DOZER_SESSION_ID") else {
        return;
    };
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let data = serde_json::from_str(&input).unwrap_or(serde_json::Value::Null);
    let event = if event_arg == "unknown" {
        data.get("hook_event_name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string()
    } else {
        event_arg.to_string()
    };
    let ts_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let req = Request::HookEvent {
        session_id,
        event,
        ts_ms,
        data,
    };

    let Ok(mut stream) = std::os::unix::net::UnixStream::connect(dozer_core::paths::socket_path())
    else {
        return;
    };
    let _ = stream.set_write_timeout(Some(Duration::from_millis(200)));
    let _ = stream.write_all(encode_line(&req).as_bytes());
    // 单向语义：不读应答，发完即走
}
