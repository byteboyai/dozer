mod install;

use dozer_core::protocol::{AgentKind, Request, encode_line};
use std::io::{Read, Write};
use std::time::Duration;

fn main() {
    let arg1 = std::env::args().nth(1);
    match arg1.as_deref() {
        Some("install") => std::process::exit(install::run_at(&install::settings_path(), true)),
        Some("uninstall") => std::process::exit(install::run_at(&install::settings_path(), false)),
        Some(agent_arg) => {
            let agent = parse_agent(agent_arg);
            let event_arg = std::env::args().nth(2);
            forward(agent, event_arg.as_deref());
        }
        None => forward(AgentKind::Unknown, None),
    }
}

/// CLI 里的 agent 名字（安装时写死进 hook command）→ `AgentKind`。
/// 未识别的字符串不 panic，落回 `Unknown`——保持 P1e 定下的"恒静默、
/// 绝不因为解析失败拖慢 agent"这条错误处理哲学。
fn parse_agent(arg: &str) -> AgentKind {
    match arg {
        "claude" => AgentKind::Claude,
        "codebuddy" => AgentKind::Codebuddy,
        "opencode" => AgentKind::Opencode,
        _ => AgentKind::Unknown,
    }
}

/// 把一次 hook 调用转发给 dozerd。恒静默、恒成功退出——绝不拖慢 agent
/// （spec P1e 错误处理：dozerd 不在/超时/畸形输入一律吞掉）。
fn forward(agent: AgentKind, event_arg: Option<&str>) {
    // 非 Dozer 会话（无注入的会话 id）：与 dozerd 无关，直接退出
    let Ok(session_id) = std::env::var("DOZER_SESSION_ID") else {
        return;
    };
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let data = serde_json::from_str(&input).unwrap_or(serde_json::Value::Null);
    let event = match event_arg {
        Some(e) => e.to_string(),
        None => data
            .get("hook_event_name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string(),
    };
    let ts_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let req = Request::HookEvent {
        session_id,
        agent,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_agent_recognizes_known_names() {
        assert_eq!(parse_agent("claude"), AgentKind::Claude);
        assert_eq!(parse_agent("codebuddy"), AgentKind::Codebuddy);
        assert_eq!(parse_agent("opencode"), AgentKind::Opencode);
    }

    #[test]
    fn parse_agent_falls_back_to_unknown() {
        assert_eq!(parse_agent("something-else"), AgentKind::Unknown);
        assert_eq!(parse_agent(""), AgentKind::Unknown);
    }
}
