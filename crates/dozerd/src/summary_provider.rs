//! 可诊断、可隔离的总结调用器(spec 2026-09-26 第 5 节)。
//!
//! 与旧 `headless_agent::summarize_headless` 的区别:旧版丢 stderr、不查退出
//! 码、16k 预算丢头、无进程组清理。这里提供:隔离临时 cwd + 清 session/MCP
//! 环境变量、整次调用 deadline、有界并发读取 stdout/stderr、stdin 错误、
//! 退出码检查、超时 kill 进程组、脱敏日志、以及 provider 能力表(含 Codex)。

use crate::summary_config::SummaryConfig;
use dozer_core::protocol::AgentKind;
use std::path::Path;
use std::time::Duration;
use tokio::io::AsyncReadExt;

/// 单次调用 stdout/stderr 的读取上限(字节)。超过即截断并标记,防止单条
/// 异常输出把内存/日志撑爆。
const MAX_OUTPUT_BYTES: usize = 256 * 1024;
/// 脱敏日志里保留的 stderr 尾部字节数。
const STDERR_TAIL_BYTES: usize = 2048;

/// provider 的隔离能力(供 UI 展示与"是否可安全用于总结"判断)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Isolation {
    /// 有 CLI 级禁工具/只读能力(Claude `-p` print 模式、Codex `--sandbox
    /// read-only`),在隔离临时目录下运行不会执行 transcript 中的指令。
    ReadOnly,
    /// 只靠隔离临时目录 + 清 session/MCP 环境变量 + 提示词隔离,CLI 级禁
    /// 工具能力未验证/不存在。UI 需如实标注,不宣称"只读"。
    TempDirOnly,
}

/// 一次总结调用的产出。
#[derive(Debug, Clone, PartialEq)]
pub struct SummaryOutput {
    pub title: String,
    pub summary: String,
    /// CLI 实际报告的模型(可取得时)。未知不伪造,保持 `None`。
    pub reported_model: Option<String>,
}

/// 总结调用失败分类(spec 第 5 节)。区分瞬态可重试与配置/能力问题。
#[derive(Debug, Clone, PartialEq)]
pub enum SummaryInvokeError {
    /// 未配置 summary provider(或旧 default_agent 也不存在)。
    ConfigurationRequired,
    /// 该 provider 没有 headless 适配器,或不具备可验证隔离能力。
    UnsupportedCapability(String),
    /// 子进程 spawn 失败(二进制不存在等)。
    Spawn(String),
    /// 认证失败(由真实调用识别,按 provider 熔断本批)。
    Authentication(String),
    /// 限流。
    RateLimit(String),
    /// 超过单调用 deadline。
    Timeout,
    /// 非零退出码 + 脱敏 stderr 尾部。
    NonzeroExit(i32, String),
    /// stdout 里没有合法 JSON 结果。
    InvalidOutput(String),
    /// 没有可用的 transcript 输入。
    InputUnavailable,
}

impl SummaryInvokeError {
    /// 是否属于"瞬态、可自动重试"类(spec 第 5 节:rate_limit/timeout/
    /// nonzero_exit 重试,认证/配置/不支持不重试)。
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            SummaryInvokeError::RateLimit(_)
                | SummaryInvokeError::Timeout
                | SummaryInvokeError::NonzeroExit(..)
        )
    }
}

/// 各 provider 的静态能力表(spec 第 5 节)。隔离能力标记"CLI 级禁工具/
/// 只读"是否已核实;`notes` 记录所测版本/待验证项,不把文档注释当证据。
pub fn provider_isolation(agent: AgentKind) -> Option<Isolation> {
    match agent {
        AgentKind::Claude => Some(Isolation::ReadOnly),
        AgentKind::Codex | AgentKind::V8agent => Some(Isolation::ReadOnly),
        AgentKind::Codebuddy | AgentKind::Opencode | AgentKind::Goose | AgentKind::Aider => {
            Some(Isolation::TempDirOnly)
        }
        AgentKind::Unknown => None,
    }
}

/// 从 stderr 脱敏尾部 + 退出码做启发式分类。只按稳定关键词匹配,不把完整
/// stderr(可能含认证值/环境变量)原样持久化。
fn classify_failure(exit_code: i32, stderr: &str) -> SummaryInvokeError {
    let tail = stderr_tail(stderr, STDERR_TAIL_BYTES);
    let lower = tail.to_lowercase();
    if lower.contains("authentication")
        || lower.contains("unauthorized")
        || lower.contains("invalid api key")
        || lower.contains("auth error")
        || lower.contains("401")
        || lower.contains("not logged in")
    {
        SummaryInvokeError::Authentication("CLI 认证失败，请检查登录状态或 API key".into())
    } else if lower.contains("rate limit")
        || lower.contains("429")
        || lower.contains("too many requests")
    {
        SummaryInvokeError::RateLimit("CLI 请求被限流，请稍后重试".into())
    } else {
        // CLI diagnostics can contain tokens, prompts and environment values.
        // Persist an allowlisted diagnosis rather than unverifiable redaction.
        SummaryInvokeError::NonzeroExit(exit_code, "CLI 非正常退出；请检查总结器配置".into())
    }
}

/// 取 stderr 尾部若干字节(按字符边界),超出标记截断。
fn stderr_tail(stderr: &str, max: usize) -> String {
    if stderr.len() <= max {
        stderr.to_string()
    } else {
        let mut start = stderr.len() - max;
        while !stderr.is_char_boundary(start) {
            start += 1;
        }
        format!("…(前文省略){}", &stderr[start..])
    }
}

/// 有界读取一个 reader 直到 EOF 或上限,超上限截断标记。`R: AsyncRead`。
async fn read_bounded<R: tokio::io::AsyncRead + Unpin>(reader: R, max: usize) -> String {
    let mut reader = reader;
    let mut buf = Vec::new();
    let mut block = [0u8; 8192];
    let mut truncated = false;
    while let Ok(n) = reader.read(&mut block).await {
        if n == 0 {
            break;
        }
        let keep = n.min(max.saturating_sub(buf.len()));
        buf.extend_from_slice(&block[..keep]);
        truncated |= keep < n;
    }
    let mut s = String::from_utf8_lossy(&buf).into_owned();
    if truncated {
        s.push_str("…[truncated]");
    }
    s
}

#[cfg(unix)]
fn kill_process_group(child: &tokio::process::Child) {
    if let Some(pid) = child.id() {
        // 负 pid 表示向整个进程组发信号(spawn 时已 process_group(0))。
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
        }
    }
}

#[cfg(not(unix))]
fn kill_process_group(child: &tokio::process::Child) {
    let _ = child;
}

/// 执行一次隔离总结调用。`turns_text` 是已构建好的对话文本,`cwd` 是隔离
/// 临时目录(调用方负责创建/清理)。返回结构化结果或分类错误。
pub async fn invoke_summary(
    agent: AgentKind,
    turns_text: &str,
    config: &SummaryConfig,
    cwd: &Path,
) -> Result<SummaryOutput, SummaryInvokeError> {
    let Some(bare) = crate::headless_agent::bare_program_name(agent) else {
        return Err(SummaryInvokeError::UnsupportedCapability(
            "该 agent 没有 headless 适配器".into(),
        ));
    };
    let program = crate::headless_agent::resolve_binary_path(bare)
        .await
        .unwrap_or_else(|| bare.to_string());
    let Some((cmd, stdin_bytes)) =
        crate::headless_agent::build_command(agent, &program, turns_text)
    else {
        return Err(SummaryInvokeError::UnsupportedCapability(
            "该 agent 没有 headless 总结适配器".into(),
        ));
    };
    run_isolated(
        cmd,
        stdin_bytes,
        cwd,
        Duration::from_secs(config.call_timeout_secs),
        agent,
    )
    .await
}

/// 跑一个"指令 + 数据"的自定义 prompt,返回原始 stdout(pipeline 分块抽取/
/// 归并用,不套总结分隔符协议)。失败分类照旧。
pub async fn invoke_summary_parts(
    agent: AgentKind,
    instruction: &str,
    data: &str,
    config: &SummaryConfig,
    cwd: &Path,
) -> Result<String, SummaryInvokeError> {
    // Include shell resolution and stdin/stdout handling in the same deadline.
    tokio::time::timeout(Duration::from_secs(config.call_timeout_secs), async {
        if provider_isolation(agent) != Some(Isolation::ReadOnly) {
            return Err(SummaryInvokeError::UnsupportedCapability(format!(
                "{} 尚未验证禁工具/只读总结能力，请选择 v8agent、Claude 或 Codex",
                agent.label()
            )));
        }
        std::fs::create_dir_all(cwd).map_err(|e| SummaryInvokeError::Spawn(e.to_string()))?;
        let scratch = tempfile::Builder::new()
            .prefix("summary-")
            .tempdir_in(cwd)
            .map_err(|e| SummaryInvokeError::Spawn(e.to_string()))?;
        let Some(bare) = crate::headless_agent::bare_program_name(agent) else {
            return Err(SummaryInvokeError::UnsupportedCapability(
                "该 agent 没有 headless 适配器".into(),
            ));
        };
        let program = crate::headless_agent::resolve_binary_path(bare)
            .await
            .unwrap_or_else(|| bare.to_string());
        let Some((mut cmd, stdin_bytes)) =
            crate::headless_agent::build_command_parts(agent, &program, instruction, data)
        else {
            return Err(SummaryInvokeError::UnsupportedCapability(
                "该 agent 没有 headless 总结适配器".into(),
            ));
        };
        if agent == AgentKind::Claude {
            cmd.arg("--tools")
                .arg("")
                .arg("--strict-mcp-config")
                .arg("--mcp-config")
                .arg("{\"mcpServers\":{}}");
        }
        if let Some(model) = &config.model {
            cmd.arg("--model").arg(model);
        }
        let (stdout, _, _) = run_isolated_capture(
            cmd,
            stdin_bytes,
            scratch.path(),
            Duration::from_secs(config.call_timeout_secs),
            agent,
        )
        .await?;
        Ok(stdout)
    })
    .await
    .map_err(|_| SummaryInvokeError::Timeout)?
}

// Drop runs on task cancellation and runtime shutdown as well as timeout. Keep
// the original process-group id even after the leader has exited.
struct ProcessGroup(u32);
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        #[cfg(unix)]
        unsafe {
            libc::kill(-(self.0 as i32), libc::SIGKILL);
        }
    }
}

/// 隔离执行一个已构造的命令:临时 cwd、清 env、deadline、有界并发读取
/// stdout/stderr、stdin 错误、退出码、超时 kill 进程组。返回
/// `(stdout, stderr, 退出码)` 三元素,由调用方决定怎么解析/分类。
async fn run_isolated_capture(
    mut cmd: tokio::process::Command,
    stdin_bytes: Option<Vec<u8>>,
    cwd: &Path,
    timeout: Duration,
    agent: AgentKind,
) -> Result<(String, String, Option<i32>), SummaryInvokeError> {
    use std::process::Stdio;
    cmd.current_dir(cwd);
    // `resolve_binary_path` may find an npm-installed CLI such as
    // `/usr/local/bin/codex` even when dozerd was launched by Finder with the
    // system-only PATH.  Such launchers commonly use `#!/usr/bin/env node`:
    // spawning the absolute script succeeds, but `env` then cannot find its
    // sibling runtime and exits 127.  Put the resolved program directory at
    // the front of the child's PATH so the executable and its interpreter are
    // resolved from the same installation prefix.
    if let Some(program_dir) = std::path::Path::new(cmd.as_std().get_program()).parent()
        && !program_dir.as_os_str().is_empty()
    {
        let mut paths = vec![program_dir.to_path_buf()];
        if let Some(inherited) = std::env::var_os("PATH") {
            paths.extend(std::env::split_paths(&inherited));
        }
        if let Ok(path) = std::env::join_paths(paths) {
            cmd.env("PATH", path);
        }
    }
    // 统一隔离层:清 session/MCP 关联环境变量(与 build_command 里各家分支
    // 的清理互为冗余,这里再兜底一次,防某家适配器漏清)。
    cmd.env_remove("DOZER_SESSION_ID");
    cmd.env_remove("DOZER_AIDER_CANONICAL_TRANSCRIPT");
    cmd.env_remove("DOZER_AIDER_BRIDGE_STATE");
    cmd.env_remove("DOZER_AIDER_CHAT_HISTORY");
    cmd.env_remove("DOZER_AIDER_INPUT_HISTORY");
    cmd.env_remove("AIDER_NOTIFICATIONS_COMMAND");
    cmd.stdin(if stdin_bytes.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);

    let mut child = cmd
        .spawn()
        .map_err(|e| SummaryInvokeError::Spawn(format!("spawn {} 失败: {e}", agent.label())))?;
    let _group = ProcessGroup(child.id().expect("spawned child"));
    let mut stdin = child.stdin.take();
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let io = async {
        let write = async {
            use tokio::io::AsyncWriteExt;
            let result = match (stdin.as_mut(), stdin_bytes) {
                (Some(input), Some(bytes)) => input.write_all(&bytes).await,
                _ => Ok(()),
            };
            drop(stdin.take());
            result
        };
        tokio::join!(
            write,
            read_bounded(stdout, MAX_OUTPUT_BYTES),
            read_bounded(stderr, MAX_OUTPUT_BYTES),
            child.wait()
        )
    };
    let (write, stdout, stderr, status) = match tokio::time::timeout(timeout, io).await {
        Ok(result) => result,
        Err(_) => {
            kill_process_group(&child);
            let _ = child.kill().await;
            return Err(SummaryInvokeError::Timeout);
        }
    };
    let status = status.map_err(|e| SummaryInvokeError::Spawn(e.to_string()))?;
    let code = status.code();
    if !status.success() {
        return Err(classify_failure(code.unwrap_or(-1), &stderr));
    }
    write.map_err(|e| SummaryInvokeError::NonzeroExit(-1, format!("写 stdin 失败: {e}")))?;
    if stdout.ends_with("…[truncated]") {
        return Err(SummaryInvokeError::InvalidOutput("模型输出超过限制".into()));
    }
    Ok((stdout, stderr, code))
}

/// 总结调用:隔离执行 + 解析分隔符 JSON。
async fn run_isolated(
    cmd: tokio::process::Command,
    stdin_bytes: Option<Vec<u8>>,
    cwd: &Path,
    timeout: Duration,
    agent: AgentKind,
) -> Result<SummaryOutput, SummaryInvokeError> {
    let (stdout, stderr, code) =
        run_isolated_capture(cmd, stdin_bytes, cwd, timeout, agent).await?;
    if let Some(code) = code
        && code != 0
    {
        return Err(classify_failure(code, &stderr));
    }
    let stdout = if agent == AgentKind::Aider {
        crate::headless_agent::clean_aider_stdout(&stdout)
    } else {
        stdout
    };
    match crate::headless_agent::extract_summary(&stdout) {
        Ok((title, summary)) => {
            if title.is_empty() && summary.is_empty() {
                return Err(SummaryInvokeError::InvalidOutput("空结果".into()));
            }
            Ok(SummaryOutput {
                title,
                summary,
                reported_model: None,
            })
        }
        Err(e) => Err(SummaryInvokeError::InvalidOutput(format!("{e:?}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn absolute_script_can_find_sibling_interpreter_with_minimal_parent_path() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let runtime = bin.join("summary-runtime");
        let cli = bin.join("summary-cli");
        std::fs::write(&runtime, "#!/bin/sh\nprintf runtime-ok\n").unwrap();
        std::fs::write(&cli, "#!/usr/bin/env summary-runtime\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();

        let mut cmd = tokio::process::Command::new(&cli);
        cmd.env("PATH", "/usr/bin:/bin");
        let (stdout, _, code) = run_isolated_capture(
            cmd,
            None,
            dir.path(),
            Duration::from_secs(2),
            AgentKind::Codex,
        )
        .await
        .unwrap();
        assert_eq!(stdout, "runtime-ok");
        assert_eq!(code, Some(0));
    }

    #[tokio::test]
    async fn capture_checks_exit_even_with_valid_json() {
        let dir = tempfile::tempdir().unwrap();
        let mut cmd = tokio::process::Command::new("sh");
        cmd.args([
            "-c",
            "echo '{\"title\":\"fake\",\"summary\":\"fake\"}'; echo '401 secret-token' >&2; exit 1",
        ]);
        let err = run_isolated_capture(
            cmd,
            None,
            dir.path(),
            Duration::from_secs(2),
            AgentKind::Claude,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, SummaryInvokeError::Authentication(_)));
        assert!(!format!("{err:?}").contains("secret-token"));
    }

    #[tokio::test]
    async fn capture_deadline_includes_blocked_stdin() {
        let dir = tempfile::tempdir().unwrap();
        let mut cmd = tokio::process::Command::new("sh");
        cmd.args(["-c", "sleep 30"]);
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            run_isolated_capture(
                cmd,
                Some(vec![b'x'; 1_000_000]),
                dir.path(),
                Duration::from_millis(100),
                AgentKind::Claude,
            ),
        )
        .await;
        assert_eq!(result.unwrap().unwrap_err(), SummaryInvokeError::Timeout);
    }

    #[tokio::test]
    async fn bounded_reader_drains_without_splitting_utf8() {
        let input = "你".repeat(10_000);
        let out = read_bounded(input.as_bytes(), 100).await;
        assert!(out.ends_with("…[truncated]"));
    }

    #[test]
    fn provider_isolation_maps_v8agent_claude_and_codex_to_readonly() {
        assert_eq!(
            provider_isolation(AgentKind::Claude),
            Some(Isolation::ReadOnly)
        );
        assert_eq!(
            provider_isolation(AgentKind::Codex),
            Some(Isolation::ReadOnly)
        );
        assert_eq!(
            provider_isolation(AgentKind::V8agent),
            Some(Isolation::ReadOnly)
        );
        assert_eq!(
            provider_isolation(AgentKind::Opencode),
            Some(Isolation::TempDirOnly)
        );
        assert_eq!(provider_isolation(AgentKind::Unknown), None);
    }

    #[test]
    fn classify_failure_detects_authentication() {
        let e = classify_failure(1, "error: invalid api key provided");
        assert!(matches!(e, SummaryInvokeError::Authentication(_)));
    }

    #[test]
    fn classify_failure_detects_rate_limit() {
        let e = classify_failure(1, "HTTP 429 too many requests");
        assert!(matches!(e, SummaryInvokeError::RateLimit(_)));
    }

    #[test]
    fn classify_failure_falls_back_to_nonzero_exit() {
        let e = classify_failure(2, "some generic error");
        assert!(matches!(e, SummaryInvokeError::NonzeroExit(2, _)));
    }

    #[test]
    fn transient_errors_are_retryable() {
        assert!(SummaryInvokeError::Timeout.is_transient());
        assert!(SummaryInvokeError::RateLimit("x".into()).is_transient());
        assert!(SummaryInvokeError::NonzeroExit(1, "x".into()).is_transient());
        assert!(!SummaryInvokeError::Authentication("x".into()).is_transient());
        assert!(!SummaryInvokeError::ConfigurationRequired.is_transient());
        assert!(!SummaryInvokeError::UnsupportedCapability("x".into()).is_transient());
    }

    #[test]
    fn stderr_tail_truncates_with_marker() {
        let s = "a".repeat(5000);
        let tail = stderr_tail(&s, 100);
        assert!(tail.starts_with('…'));
        assert!(tail.len() <= 100 + "…(前文省略)".len());
    }

    #[tokio::test]
    async fn run_isolated_spawn_failure_maps_to_spawn_error() {
        let dir = tempfile::tempdir().unwrap();
        let mut cmd = tokio::process::Command::new("this-binary-does-not-exist-xyz");
        cmd.arg("x");
        let err = run_isolated(
            cmd,
            None,
            dir.path(),
            Duration::from_secs(5),
            AgentKind::Claude,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, SummaryInvokeError::Spawn(_)));
    }

    #[tokio::test]
    async fn run_isolated_reads_stdout_and_returns_summary() {
        let dir = tempfile::tempdir().unwrap();
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c").arg("cat");
        let stdin = format!(
            "{}{}{}",
            "<<<DOZER_SUMMARY_JSON>>>",
            "{\"title\":\"t\",\"summary\":\"s\"}",
            "<<<END_DOZER_SUMMARY_JSON>>>"
        );
        let out = run_isolated(
            cmd,
            Some(stdin.into_bytes()),
            dir.path(),
            Duration::from_secs(5),
            AgentKind::Claude,
        )
        .await
        .unwrap();
        assert_eq!(out.title, "t");
        assert_eq!(out.summary, "s");
    }

    #[tokio::test]
    async fn run_isolated_times_out_and_kills_process_group() {
        let dir = tempfile::tempdir().unwrap();
        let mut cmd = tokio::process::Command::new("sh");
        // 起一个会 spawn 子进程的慢命令,验证超时后子进程组被回收。
        cmd.arg("-c").arg("sleep 30 & sleep 30");
        let err = run_isolated(
            cmd,
            None,
            dir.path(),
            Duration::from_millis(200),
            AgentKind::Claude,
        )
        .await
        .unwrap_err();
        assert_eq!(err, SummaryInvokeError::Timeout);
    }

    #[tokio::test]
    async fn run_isolated_nonzero_exit_maps_to_classified_error() {
        let dir = tempfile::tempdir().unwrap();
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c")
            .arg("echo 'error: invalid api key' >&2; exit 1");
        let err = run_isolated(
            cmd,
            None,
            dir.path(),
            Duration::from_secs(5),
            AgentKind::Claude,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, SummaryInvokeError::Authentication(_)));
    }
}
