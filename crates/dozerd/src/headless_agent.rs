//! headless 一次性 agent 调用(spec 2026-08-28):不经过 PTY/Session/
//! registry,启动一次 agent CLI 处理单个 prompt 后退出,用固定分隔符包裹
//! 的 JSON 做输出协议——四家 CLI 的私有结构化输出格式互不相同,统一约定
//! "模型最终文本里必须包含这一段"比对齐四种进程输出协议更简单。

use dozer_core::protocol::AgentKind;
use serde::Deserialize;
use std::time::Duration;

const SUMMARY_START_MARKER: &str = "<<<DOZER_SUMMARY_JSON>>>";
const SUMMARY_END_MARKER: &str = "<<<END_DOZER_SUMMARY_JSON>>>";
const HEADLESS_TIMEOUT: Duration = Duration::from_secs(90);
const TITLE_MAX_CHARS: usize = 200;
const SUMMARY_MAX_CHARS: usize = 8000;

#[derive(Debug, Clone, PartialEq)]
pub enum HeadlessError {
    /// 子进程启动失败(二进制不在 PATH 上等)。
    Spawn(String),
    /// 超过 `HEADLESS_TIMEOUT` 仍未退出。
    Timeout,
    /// stdout 里找不到完整的一对分隔符。
    NoMarkers,
    /// 分隔符之间的内容不是合法 JSON,或缺 `title`/`summary` 字段。
    InvalidJson(String),
    /// 该 `AgentKind` 没有对应的 headless 适配器(理论上调用方只会传四家
    /// 已覆盖的 kind,这个分支是防御性的)。
    Unsupported,
}

fn instruction_text() -> String {
    format!(
        "请阅读接下来这段对话记录里用户说过的话,生成一个简短标题和一段摘要,\
         总结这次会话完成的工作。只输出下面这一段,不要输出任何其他内容:\n\
         {SUMMARY_START_MARKER}{{\"title\":\"...\",\"summary\":\"...\"}}{SUMMARY_END_MARKER}"
    )
}

#[derive(Deserialize)]
struct SummaryJson {
    title: String,
    summary: String,
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let head: String = s.chars().take(max).collect();
        format!("{head}…")
    }
}

/// 从完整 stdout 里抠出分隔符之间的 JSON 并解析。公开给单测直接调用,不
/// 需要真的起子进程。
pub fn extract_summary(stdout: &str) -> Result<(String, String), HeadlessError> {
    let start = stdout
        .find(SUMMARY_START_MARKER)
        .ok_or(HeadlessError::NoMarkers)?;
    let after_start = start + SUMMARY_START_MARKER.len();
    let end = stdout[after_start..]
        .find(SUMMARY_END_MARKER)
        .ok_or(HeadlessError::NoMarkers)?;
    let json_str = stdout[after_start..after_start + end].trim();
    let parsed: SummaryJson =
        serde_json::from_str(json_str).map_err(|e| HeadlessError::InvalidJson(e.to_string()))?;
    Ok((
        truncate_chars(&parsed.title, TITLE_MAX_CHARS),
        truncate_chars(&parsed.summary, SUMMARY_MAX_CHARS),
    ))
}

/// 按 agent 构造子进程命令 + (可选)要写进 stdin 的字节。`None` 表示这个
/// `AgentKind` 没有 headless 适配器。
fn build_command(
    agent: AgentKind,
    turns_text: &str,
) -> Option<(tokio::process::Command, Option<Vec<u8>>)> {
    match agent {
        AgentKind::Claude => {
            let mut cmd = tokio::process::Command::new("claude");
            cmd.arg("-p").arg(instruction_text());
            Some((cmd, Some(turns_text.as_bytes().to_vec())))
        }
        AgentKind::Codebuddy => {
            let mut cmd = tokio::process::Command::new("codebuddy");
            // `-y`/`--dangerously-skip-permissions`:CodeBuddy 非交互模式下
            // 执行任何需要授权的操作(哪怕这里只是让它输出文字)的必需参数,
            // 不加会卡在授权确认上,headless 场景下无人能应答(spec
            // 2026-08-28 调研结论)。
            cmd.arg("-p").arg(instruction_text()).arg("-y");
            Some((cmd, Some(turns_text.as_bytes().to_vec())))
        }
        AgentKind::Opencode => {
            let mut cmd = tokio::process::Command::new("opencode");
            // `run` 子命令没有独立 stdin 输入通道,拼接文本直接作为 message
            // 参数的一部分(spec 2026-08-28 调研结论)。
            cmd.arg("run")
                .arg(format!("{}\n\n{}", instruction_text(), turns_text));
            Some((cmd, None))
        }
        AgentKind::V8agent => {
            let mut cmd = tokio::process::Command::new("v8agent");
            cmd.env("V8AGENT_ONESHOT", "1");
            cmd.env_remove("DOZER_SESSION_ID");
            let stdin_text = format!("{}\n\n{}", instruction_text(), turns_text);
            Some((cmd, Some(stdin_text.into_bytes())))
        }
        AgentKind::Unknown | AgentKind::Codex | AgentKind::Kilo => None,
    }
}

/// 跑一个已经构造好的子进程,拿完整 stdout 后解析。跟 `build_command` 分开
/// 是为了这一段能用真实存在的 `sh`/不存在的二进制名做确定性单测,不需要
/// 装 claude/codebuddy/opencode/v8agent 才能测超时/spawn 失败这些分支。
async fn run_and_extract(
    mut cmd: tokio::process::Command,
    stdin_bytes: Option<Vec<u8>>,
    timeout: Duration,
) -> Result<(String, String), HeadlessError> {
    use std::process::Stdio;
    cmd.stdin(if stdin_bytes.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::null());
    let mut child = cmd
        .spawn()
        .map_err(|e| HeadlessError::Spawn(e.to_string()))?;
    if let Some(bytes) = stdin_bytes {
        use tokio::io::AsyncWriteExt;
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(&bytes).await;
        }
    }
    let output = tokio::time::timeout(timeout, child.wait_with_output())
        .await
        .map_err(|_| HeadlessError::Timeout)?
        .map_err(|e| HeadlessError::Spawn(e.to_string()))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    extract_summary(&stdout)
}

/// 对外唯一入口:`human_turns` 是该会话已摄取的人类回合内容(与
/// `session_summary::heuristic_from_turns` 同一数据源),按 `agent` 分派到
/// 对应 CLI 的一次性调用方式,拿到结果或错误(调用方失败时应降级到
/// `heuristic_from_turns`,不是本函数的职责)。
pub async fn summarize_headless(
    agent: AgentKind,
    human_turns: &[String],
) -> Result<(String, String), HeadlessError> {
    let turns_text = human_turns.join("\n");
    let Some((cmd, stdin_bytes)) = build_command(agent, &turns_text) else {
        return Err(HeadlessError::Unsupported);
    };
    run_and_extract(cmd, stdin_bytes, HEADLESS_TIMEOUT).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_summary_parses_delimited_json() {
        let stdout = format!(
            "some preamble\n{SUMMARY_START_MARKER}{{\"title\":\"标题\",\"summary\":\"摘要\"}}{SUMMARY_END_MARKER}\ntrailing"
        );
        let (title, summary) = extract_summary(&stdout).unwrap();
        assert_eq!(title, "标题");
        assert_eq!(summary, "摘要");
    }

    #[test]
    fn extract_summary_missing_markers_errors() {
        assert_eq!(
            extract_summary("no markers here"),
            Err(HeadlessError::NoMarkers)
        );
    }

    #[test]
    fn extract_summary_invalid_json_errors() {
        let stdout = format!("{SUMMARY_START_MARKER}not json{SUMMARY_END_MARKER}");
        assert!(matches!(
            extract_summary(&stdout),
            Err(HeadlessError::InvalidJson(_))
        ));
    }

    #[test]
    fn extract_summary_truncates_overlong_title() {
        let long_title = "a".repeat(300);
        let json = format!("{{\"title\":\"{long_title}\",\"summary\":\"s\"}}");
        let stdout = format!("{SUMMARY_START_MARKER}{json}{SUMMARY_END_MARKER}");
        let (title, _) = extract_summary(&stdout).unwrap();
        assert_eq!(title.chars().count(), TITLE_MAX_CHARS + 1); // +1 是省略号
        assert!(title.ends_with('…'));
    }

    #[test]
    fn claude_command_uses_dash_p_and_pipes_turns_via_stdin() {
        let (cmd, stdin) = build_command(AgentKind::Claude, "用户说了什么").unwrap();
        let std_cmd = cmd.as_std();
        assert_eq!(std_cmd.get_program(), "claude");
        let args: Vec<String> = std_cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args[0], "-p");
        assert!(args[1].contains(SUMMARY_START_MARKER));
        assert_eq!(stdin, Some("用户说了什么".as_bytes().to_vec()));
    }

    #[tokio::test]
    async fn run_and_extract_reads_stdout_after_process_exits() {
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c").arg("cat");
        let stdin = Some(
            format!(
                "{SUMMARY_START_MARKER}{{\"title\":\"t\",\"summary\":\"s\"}}{SUMMARY_END_MARKER}"
            )
            .into_bytes(),
        );
        let (title, summary) = run_and_extract(cmd, stdin, Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(title, "t");
        assert_eq!(summary, "s");
    }

    #[tokio::test]
    async fn run_and_extract_times_out_on_slow_process() {
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c").arg("sleep 5");
        let result = run_and_extract(cmd, None, Duration::from_millis(100)).await;
        assert_eq!(result, Err(HeadlessError::Timeout));
    }

    #[tokio::test]
    async fn run_and_extract_reports_spawn_failure_for_missing_binary() {
        let cmd = tokio::process::Command::new("this-binary-does-not-exist-xyz");
        let result = run_and_extract(cmd, None, Duration::from_secs(5)).await;
        assert!(matches!(result, Err(HeadlessError::Spawn(_))));
    }

    #[tokio::test]
    async fn summarize_headless_returns_unsupported_for_kind_without_adapter() {
        // Codex 目前没有 headless 适配器(Task 5 只补 CodeBuddy/OpenCode/
        // V8agent,Codex 本来就不在覆盖范围内,见 spec 非目标)。
        let result = summarize_headless(AgentKind::Codex, &[]).await;
        assert_eq!(result, Err(HeadlessError::Unsupported));
    }

    #[test]
    fn codebuddy_command_includes_dash_y_for_non_interactive_permission() {
        let (cmd, stdin) = build_command(AgentKind::Codebuddy, "内容").unwrap();
        let std_cmd = cmd.as_std();
        assert_eq!(std_cmd.get_program(), "codebuddy");
        let args: Vec<String> = std_cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args[0], "-p");
        assert!(args.contains(&"-y".to_string()));
        assert_eq!(stdin, Some("内容".as_bytes().to_vec()));
    }

    #[test]
    fn opencode_command_uses_run_subcommand_with_inline_message_no_stdin() {
        let (cmd, stdin) = build_command(AgentKind::Opencode, "用户内容").unwrap();
        let std_cmd = cmd.as_std();
        assert_eq!(std_cmd.get_program(), "opencode");
        let args: Vec<String> = std_cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args[0], "run");
        assert!(args[1].contains(SUMMARY_START_MARKER));
        assert!(args[1].contains("用户内容"));
        assert_eq!(stdin, None);
    }

    #[test]
    fn v8agent_command_sets_oneshot_env_and_clears_session_id() {
        let (cmd, stdin) = build_command(AgentKind::V8agent, "用户内容").unwrap();
        let std_cmd = cmd.as_std();
        assert_eq!(std_cmd.get_program(), "v8agent");
        let envs: Vec<_> = std_cmd.get_envs().collect();
        assert!(envs
            .iter()
            .any(|(k, v)| *k == "V8AGENT_ONESHOT" && *v == Some(std::ffi::OsStr::new("1"))));
        // DOZER_SESSION_ID 显式清掉,避免 v8agent-cli 误挂载 dozer-mcp
        // (headless 总结走 stdout 解析,不需要 MCP,见 spec)。
        assert!(envs.iter().any(|(k, v)| *k == "DOZER_SESSION_ID" && v.is_none()));
        assert!(stdin.is_some());
    }

    #[test]
    fn unsupported_kinds_return_none() {
        assert!(build_command(AgentKind::Codex, "x").is_none());
        assert!(build_command(AgentKind::Kilo, "x").is_none());
        assert!(build_command(AgentKind::Unknown, "x").is_none());
    }
}
