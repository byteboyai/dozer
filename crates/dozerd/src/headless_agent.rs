//! headless 一次性 agent 调用(spec 2026-08-28):不经过 PTY/Session/
//! registry,启动一次 agent CLI 处理单个 prompt 后退出,用固定分隔符包裹
//! 的 JSON 做输出协议——四家 CLI 的私有结构化输出格式互不相同,统一约定
//! "模型最终文本里必须包含这一段"比对齐四种进程输出协议更简单。

use dozer_core::protocol::{AgentKind, TurnRecord};
use serde::Deserialize;
use std::time::Duration;

const SUMMARY_START_MARKER: &str = "<<<DOZER_SUMMARY_JSON>>>";
const SUMMARY_END_MARKER: &str = "<<<END_DOZER_SUMMARY_JSON>>>";
const HEADLESS_TIMEOUT: Duration = Duration::from_secs(90);
const TITLE_MAX_CHARS: usize = 200;
const SUMMARY_MAX_CHARS: usize = 8000;
/// 单条回合喂给 headless agent 前的截断上限,避免一条巨型工具输出/长回复
/// 把整个 prompt 撑爆。
const MAX_TURN_CHARS: usize = 4000;
/// 拼接后的对话记录总预算(字符数)。超预算时从最早的回合开始丢弃,保留
/// 离"完成的工作"最近的尾部——用户反馈总结只看得到人类那句话、看不到
/// AI 实际做了什么(2026-08-28 用户反馈),这个预算就是为了在不炸 prompt
/// 的前提下把 AI 回合也喂进去。
const MAX_TRANSCRIPT_CHARS: usize = 16_000;

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
        "请阅读接下来这段对话记录(用户与 AI 的完整往来,包含 AI 实际做了\
         什么),生成一个简短标题和一段摘要,总结这次会话完成的工作。只输出\
         下面这一段,不要输出任何其他内容:\n\
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

/// 把该会话的人类+AI 回合(不含 `tool_result`、不含 `thinking` 内心独白,
/// 只要"用户说了什么"+"AI 最终说了什么/做了什么")拼成喂给 headless agent
/// 的对话记录文本,超预算时从最早的回合开始丢弃,保留离"完成的工作"最近
/// 的尾部。`heuristic_from_turns`(`session_summary.rs`)是独立的、故意
/// 保持"无语义压缩"的确定性兜底,不复用这个函数——那条路径本来就不追求
/// 语义质量,只在 headless 调用失败/超时/不支持时才会被打到。
fn build_transcript_text(turns: &[TurnRecord]) -> String {
    let mut lines: std::collections::VecDeque<String> = turns
        .iter()
        .filter(|t| t.role == "human" || (t.role == "ai" && !t.thinking))
        .map(|t| {
            let label = if t.role == "human" { "用户" } else { "AI" };
            format!("{label}: {}", truncate_chars(&t.content, MAX_TURN_CHARS))
        })
        .collect();
    let mut total: usize = lines.iter().map(|l| l.chars().count() + 1).sum();
    while total > MAX_TRANSCRIPT_CHARS {
        let Some(front) = lines.pop_front() else {
            break;
        };
        total -= front.chars().count() + 1;
    }
    Vec::from(lines).join("\n")
}

/// 该 `AgentKind` 对应的 headless CLI 裸命令名(供 PATH 解析用)。`None`
/// 表示没有对应的 headless 适配器。
fn bare_program_name(agent: AgentKind) -> Option<&'static str> {
    match agent {
        AgentKind::Claude => Some("claude"),
        AgentKind::Codebuddy => Some("codebuddy"),
        AgentKind::Opencode => Some("opencode"),
        AgentKind::V8agent => Some("v8agent"),
        AgentKind::Unknown | AgentKind::Codex | AgentKind::Kilo => None,
    }
}

/// 借用户登录 shell 把 `bin` 解析成绝对路径。`dozerd` 由 launchd/Finder
/// 拉起(而不是从终端 `cargo run`/`open` 继承已经 source 过 rc 文件的
/// shell)时,自身进程环境的 `PATH` 是精简系统默认值
/// (`/usr/bin:/bin:/usr/sbin:/sbin`),不含 `/usr/local/bin`、
/// `/opt/homebrew/bin`、`~/.cargo/bin`、`~/.local/bin` 等用户自装 CLI 的
/// 常见位置——`claude`/`codebuddy`/`opencode`/`v8agent` 这几个二进制几乎
/// 从不在那四条系统路径里,直接 `Command::new("claude")` 会 100% spawn
/// 失败,整批补总结因此全部静默降级到启发式兜底(2026-08-28 用户反馈
/// 实测复现:某项目补总结后全部 34 条清一色 `heuristic_fallback`)。交互式
/// PTY 会话不受这个问题影响,是因为 `spec.command` 本身是一个登录 shell
/// (`session.rs` 的 `CommandBuilder`),shell 自己 source rc 文件重建出
/// 完整 `PATH` 后用户才在里面敲 `claude`;headless 场景绕开了 shell,所以
/// 这里显式借一次登录 shell 把 `PATH` 借出来。`-ilc`(交互+登录)覆盖
/// `.zshenv`/`.zprofile`/`.zshrc`/`.zlogin` 全部来源,不假设用户把 PATH
/// 加在哪一个里。解析失败(shell 起不来、`command -v` 找不到)时返回
/// `None`,调用方回退到裸命令名,保留原有"确实没装就 spawn 失败降级"的
/// 行为,不引入新的失败模式。
async fn resolve_binary_path(bin: &str) -> Option<String> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    let output = tokio::process::Command::new(&shell)
        .arg("-ilc")
        .arg(format!("command -v {bin}"))
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    // `command -v` 在交互式 zsh 下对别名/内置命令(如用户把 `ls` alias 成
    // `eza`)会打印别名定义或裸命令名而不是文件路径——只信一段以 `/`
    // 开头、看起来真是绝对路径的输出,否则当作没解析出来,回退到裸命令名
    // (让调用方走回原有的"确实没装就 spawn 失败降级"路径,不去猜别名里
    // 藏的到底是什么)。
    if path.starts_with('/') {
        Some(path)
    } else {
        None
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

/// 按 agent 构造子进程命令 + (可选)要写进 stdin 的字节。`program` 是
/// `bare_program_name` 裸命令名或 `resolve_binary_path` 解析出的绝对路径
/// ——由调用方决定用哪个,这里只管拿它当 `Command::new` 的程序名。`None`
/// 表示这个 `AgentKind` 没有 headless 适配器。
fn build_command(
    agent: AgentKind,
    program: &str,
    turns_text: &str,
) -> Option<(tokio::process::Command, Option<Vec<u8>>)> {
    match agent {
        AgentKind::Claude => {
            let mut cmd = tokio::process::Command::new(program);
            cmd.arg("-p").arg(instruction_text());
            Some((cmd, Some(turns_text.as_bytes().to_vec())))
        }
        AgentKind::Codebuddy => {
            let mut cmd = tokio::process::Command::new(program);
            // `-y`/`--dangerously-skip-permissions`:CodeBuddy 非交互模式下
            // 执行任何需要授权的操作(哪怕这里只是让它输出文字)的必需参数,
            // 不加会卡在授权确认上,headless 场景下无人能应答(spec
            // 2026-08-28 调研结论)。
            cmd.arg("-p").arg(instruction_text()).arg("-y");
            Some((cmd, Some(turns_text.as_bytes().to_vec())))
        }
        AgentKind::Opencode => {
            let mut cmd = tokio::process::Command::new(program);
            // `run` 子命令没有独立 stdin 输入通道,拼接文本直接作为 message
            // 参数的一部分(spec 2026-08-28 调研结论)。
            cmd.arg("run")
                .arg(format!("{}\n\n{}", instruction_text(), turns_text));
            Some((cmd, None))
        }
        AgentKind::V8agent => {
            let mut cmd = tokio::process::Command::new(program);
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

/// 对外唯一入口:`turns` 是该会话已摄取的完整回合记录(人类+AI 都在内,
/// `build_transcript_text` 负责挑出有意义的部分拼成 prompt——不只喂人类
/// 说的话,不然 headless agent 无从得知"完成的工作"是什么,只能复述用户
/// 的原话,2026-08-28 用户反馈),按 `agent` 分派到对应 CLI 的一次性调用
/// 方式,拿到结果或错误(调用方失败时应降级到 `heuristic_from_turns`,不是
/// 本函数的职责)。
pub async fn summarize_headless(
    agent: AgentKind,
    turns: &[TurnRecord],
) -> Result<(String, String), HeadlessError> {
    let Some(bare) = bare_program_name(agent) else {
        return Err(HeadlessError::Unsupported);
    };
    let program = resolve_binary_path(bare)
        .await
        .unwrap_or_else(|| bare.to_string());
    let turns_text = build_transcript_text(turns);
    let Some((cmd, stdin_bytes)) = build_command(agent, &program, &turns_text) else {
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
        let (cmd, stdin) = build_command(AgentKind::Claude, "claude", "用户说了什么").unwrap();
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
        let (cmd, stdin) = build_command(AgentKind::Codebuddy, "codebuddy", "内容").unwrap();
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
        let (cmd, stdin) = build_command(AgentKind::Opencode, "opencode", "用户内容").unwrap();
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
        let (cmd, stdin) = build_command(AgentKind::V8agent, "v8agent", "用户内容").unwrap();
        let std_cmd = cmd.as_std();
        assert_eq!(std_cmd.get_program(), "v8agent");
        let envs: Vec<_> = std_cmd.get_envs().collect();
        assert!(
            envs.iter()
                .any(|(k, v)| *k == "V8AGENT_ONESHOT" && *v == Some(std::ffi::OsStr::new("1")))
        );
        // DOZER_SESSION_ID 显式清掉,避免 v8agent-cli 误挂载 dozer-mcp
        // (headless 总结走 stdout 解析,不需要 MCP,见 spec)。
        assert!(
            envs.iter()
                .any(|(k, v)| *k == "DOZER_SESSION_ID" && v.is_none())
        );
        assert!(stdin.is_some());
    }

    #[test]
    fn build_command_uses_resolved_absolute_path_as_program() {
        // `program` 参数由调用方(`resolve_binary_path` 或裸命令名兜底)
        // 决定,`build_command` 本身不应该硬编码任何一家的裸命令名。
        let (cmd, _) = build_command(AgentKind::Claude, "/usr/local/bin/claude", "x").unwrap();
        assert_eq!(cmd.as_std().get_program(), "/usr/local/bin/claude");
    }

    #[test]
    fn unsupported_kinds_return_none() {
        assert!(build_command(AgentKind::Codex, "codex", "x").is_none());
        assert!(build_command(AgentKind::Kilo, "kilo", "x").is_none());
        assert!(build_command(AgentKind::Unknown, "unknown", "x").is_none());
    }

    fn turn(role: &str, content: &str) -> TurnRecord {
        turn_with_thinking(role, content, false)
    }

    fn turn_with_thinking(role: &str, content: &str, thinking: bool) -> TurnRecord {
        TurnRecord {
            role: role.to_string(),
            content: content.to_string(),
            thinking,
            ..Default::default()
        }
    }

    #[test]
    fn build_transcript_text_includes_human_and_ai_but_not_tool_result_or_thinking() {
        let turns = vec![
            turn("human", "帮我改一下 README"),
            turn_with_thinking("ai", "让我先看看现有内容", true),
            turn("tool_result", "<file content dump>"),
            turn("ai", "已经改好了，加了安装说明"),
        ];
        let text = build_transcript_text(&turns);
        assert!(text.contains("用户: 帮我改一下 README"));
        assert!(text.contains("AI: 已经改好了，加了安装说明"));
        assert!(!text.contains("让我先看看现有内容"));
        assert!(!text.contains("file content dump"));
    }

    #[test]
    fn build_transcript_text_drops_earliest_turns_when_over_budget() {
        // 标记放在内容开头(而不是结尾),这样单条内容超过 `MAX_TURN_CHARS`
        // 被 `truncate_chars` 截断时标记不会被一起切掉。
        let filler = "x".repeat(MAX_TURN_CHARS);
        let turns: Vec<TurnRecord> = (0..(MAX_TRANSCRIPT_CHARS / MAX_TURN_CHARS + 2))
            .map(|i| turn("human", &format!("turn-{i}-{filler}")))
            .collect();
        let text = build_transcript_text(&turns);
        assert!(text.chars().count() <= MAX_TRANSCRIPT_CHARS);
        // 最早的一条(index 0)应该被挤掉,最后一条必须保留。
        assert!(!text.contains("turn-0-"));
        let last_index = turns.len() - 1;
        assert!(text.contains(&format!("turn-{last_index}-")));
    }

    #[tokio::test]
    async fn resolve_binary_path_finds_real_binary_on_path() {
        // `mkdir` 是最精简系统 PATH(`/usr/bin:/bin:/usr/sbin:/sbin`)下就有
        // 的外部二进制,不像 `ls`/`cat` 那样常被用户 alias 掉,也不像
        // `true`/`printf` 那样可能是 shell 内置命令——两者都会让 `command
        // -v` 吐出别名定义/裸名字而不是路径,不适合拿来测"确实解析出绝对
        // 路径"这件事。
        let resolved = resolve_binary_path("mkdir").await;
        assert!(resolved.is_some(), "应该能解析出 mkdir 的绝对路径");
        assert!(resolved.unwrap().ends_with("/mkdir"));
    }

    #[tokio::test]
    async fn resolve_binary_path_returns_none_for_missing_binary() {
        let resolved = resolve_binary_path("this-binary-does-not-exist-xyz").await;
        assert_eq!(resolved, None);
    }
}
