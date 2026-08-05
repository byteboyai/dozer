mod codebuddy;
mod install;
mod opencode;
mod opencode_install;

use dozer_core::protocol::{AgentKind, Request, encode_line};
use std::io::{Read, Write};
use std::time::Duration;

fn main() {
    let arg1 = std::env::args().nth(1);
    match arg1.as_deref() {
        Some("install") => {
            let agent = std::env::args().nth(2).unwrap_or_else(|| "claude".into());
            if agent == "opencode" {
                std::process::exit(opencode_install::run_at(
                    &opencode_install::plugins_dir(),
                    true,
                ));
            }
            std::process::exit(install::run_at(
                &install::settings_path_for(&agent),
                &agent,
                true,
            ))
        }
        Some("uninstall") => {
            let agent = std::env::args().nth(2).unwrap_or_else(|| "claude".into());
            if agent == "opencode" {
                std::process::exit(opencode_install::run_at(
                    &opencode_install::plugins_dir(),
                    false,
                ));
            }
            std::process::exit(install::run_at(
                &install::settings_path_for(&agent),
                &agent,
                false,
            ))
        }
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

/// 决定最终转发给 dozerd 的事件名：先取原始事件名（CLI 参数缺失时退回
/// hook JSON 里的 `hook_event_name` 字段），再按 agent 过一遍翻译表。
/// `None` 表示这次调用不该转发（spec §5.2：子 agent 生命周期事件丢弃）。
fn resolve_event(agent: AgentKind, event_arg: Option<&str>) -> Option<String> {
    let raw = event_arg.map(str::to_string);
    let raw = raw.unwrap_or_else(|| "unknown".to_string());
    match agent {
        AgentKind::Codebuddy => codebuddy::translate_event(&raw),
        _ => Some(raw),
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
    let event_arg = event_arg.or_else(|| data.get("hook_event_name").and_then(|v| v.as_str()));
    let Some(event) = resolve_event(agent, event_arg) else {
        return; // 该事件按翻译表规则被丢弃（如 CodeBuddy 的 Subagent* 事件）
    };
    let ts_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    if agent == AgentKind::Opencode
        && let Some(line) = data.get("transcript_line").filter(|v| !v.is_null())
    {
        let cwd = data.get("cwd").and_then(|v| v.as_str()).unwrap_or(".");
        if let Err(e) = opencode::append_transcript_line(cwd, &session_id, line) {
            eprintln!("opencode transcript 落盘失败（已忽略，不影响转发）: {e}");
        }
    }
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

    #[test]
    fn resolve_event_translates_codebuddy_failure_variants() {
        assert_eq!(
            resolve_event(AgentKind::Codebuddy, Some("PostToolUseFailure")),
            Some("PostToolUse".to_string())
        );
        assert_eq!(
            resolve_event(AgentKind::Codebuddy, Some("StopFailure")),
            Some("Stop".to_string())
        );
    }

    #[test]
    fn resolve_event_drops_codebuddy_subagent_events() {
        assert_eq!(
            resolve_event(AgentKind::Codebuddy, Some("SubagentStart")),
            None
        );
    }

    #[test]
    fn resolve_event_passes_claude_events_through_untranslated() {
        assert_eq!(
            resolve_event(AgentKind::Claude, Some("PostToolUseFailure")),
            Some("PostToolUseFailure".to_string()),
            "Claude 分支不套用 CodeBuddy 的翻译表"
        );
    }
}
