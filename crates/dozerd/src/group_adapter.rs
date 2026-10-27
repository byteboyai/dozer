//! 群聊发言的 agent 适配器。进程层(解析 binary 路径、移除 `DOZER_SESSION_ID`)
//! 复用 `headless_agent`;与总结/任务处理的区别:
//! - 不用分隔符 JSON 协议,直接取最终文本作为回复;
//! - 子进程 `kill_on_drop(true)`,超时/取消时真正杀掉,不留孤儿;
//! - 检查退出码,stderr 尾部进入失败原因;
//! - **只读**:Codex `--sandbox read-only`,Claude 工具白/黑名单,Qodercn 限定内置
//!   工具集为读/搜。Opencode、Codebuddy 没有经实测验证的只读模式,保持不支持。**严禁**加
//!   `--dangerously-skip-permissions`/`-y`(Todo 任务处理里有,群聊不能抄)。

use crate::headless_agent::{bare_program_name, resolve_binary_path};
use dozer_core::protocol::AgentKind;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::Duration;
use tokio::sync::watch;

/// 单次发言的超时。讨论里 agent 可能只读浏览多个文件再作答,比 90s 的总结
/// 超时宽,又比 600s 的任务处理短。后续按实测调整,不是精确校准过的值。
pub const GROUP_TURN_TIMEOUT: Duration = Duration::from_secs(300);
/// 入库的单条回复字符上限(防止巨型输出撑爆库与后续提示词)。
pub const MAX_REPLY_CHARS: usize = 20_000;
const STDERR_TAIL_CHARS: usize = 500;

/// 提示词本体走 stdin;参数里只放这一句指针(同 `headless_agent::build_command_parts`
/// 已验证的"短指令 + stdin 数据"模式)。
const CLAUDE_STDIN_POINTER: &str = "请严格按 stdin 中给出的群聊记录与规则发言。";

/// Qodercn 把 stdin 内容放在 `-p` 文字**之前**,所以指令要指向「上文」而不是
/// 「stdin」——实测沿用 Claude 的措辞时,模型会抱怨没收到 stdin 里的记录,
/// 这类元评论会混进群聊发言。
const QODERCN_POINTER: &str = "请严格按上文给出的群聊记录与规则发言,只输出你的发言内容。";

/// Claude 只读:允许读/搜,禁止写、执行、联网。**以 Task 0 findings 为准**。
const CLAUDE_READONLY_ARGS: [&str; 4] = [
    "--allowedTools",
    "Read,Grep,Glob",
    "--disallowedTools",
    "Bash,Edit,Write,NotebookEdit,WebFetch,WebSearch",
];

/// Qodercn 只读:`--tools` 把**内置工具集**限定为读/搜(不是放开再黑名单),
/// 黑名单是第二道防线。以收尾计划 T3c 的金丝雀实测为准:写文件、跑 shell、联网
/// 三步都没有对应工具,读取成功。参数顺序与实测一致(`--tools` 是变长参数,后面
/// 紧跟另一个 `--` 开头的选项来收尾)。
const QODERCN_TOOLS: &str = "Read,Grep,Glob";
const QODERCN_DENIED_TOOLS: [&str; 6] = [
    "Bash",
    "Edit",
    "Write",
    "NotebookEdit",
    "WebFetch",
    "WebSearch",
];

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Clone)]
pub struct TurnRequest {
    pub agent: AgentKind,
    pub project_dir: PathBuf,
    pub prompt: String,
    pub timeout: Duration,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TurnError {
    Spawn(String),
    Timeout,
    Cancelled,
    Exit {
        code: Option<i32>,
        stderr_tail: String,
    },
    Empty,
    Unsupported,
}

impl std::fmt::Display for TurnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TurnError::Spawn(e) => write!(f, "无法启动 agent:{e}"),
            TurnError::Timeout => write!(f, "发言超时"),
            TurnError::Cancelled => write!(f, "已取消"),
            TurnError::Exit { code, stderr_tail } => {
                let code = code.map_or("未知".to_string(), |c| c.to_string());
                if stderr_tail.is_empty() {
                    write!(f, "agent 异常退出(退出码 {code})")
                } else {
                    write!(f, "agent 异常退出(退出码 {code}):{stderr_tail}")
                }
            }
            TurnError::Empty => write!(f, "agent 没有返回内容(空输出)"),
            TurnError::Unsupported => write!(f, "该 agent 暂不支持群聊发言"),
        }
    }
}

/// 可替换的运行器:真实现起子进程,测试里用假实现。
pub trait GroupAgentRunner: Send + Sync {
    fn run<'a>(
        &'a self,
        req: TurnRequest,
        cancel: watch::Receiver<bool>,
    ) -> BoxFuture<'a, Result<String, TurnError>>;
}

pub(crate) fn build_turn_command(
    agent: AgentKind,
    program: &str,
    project_dir: &Path,
    prompt: &str,
    last_message_file: Option<&Path>,
) -> Option<(tokio::process::Command, Option<Vec<u8>>)> {
    match agent {
        AgentKind::Claude => {
            let mut cmd = tokio::process::Command::new(program);
            cmd.current_dir(project_dir)
                .env_remove(dozer_core::paths::session_env())
                .arg("-p")
                .arg(CLAUDE_STDIN_POINTER)
                .arg("--no-session-persistence")
                .args(CLAUDE_READONLY_ARGS);
            Some((cmd, Some(prompt.as_bytes().to_vec())))
        }
        AgentKind::Qodercn => {
            let mut cmd = tokio::process::Command::new(program);
            cmd.current_dir(project_dir)
                .env_remove(dozer_core::paths::session_env())
                .arg("-p")
                .arg(QODERCN_POINTER)
                .arg("--no-session-persistence")
                .arg("--tools")
                .arg(QODERCN_TOOLS);
            for tool in QODERCN_DENIED_TOOLS {
                cmd.arg("--disallowed-tools").arg(tool);
            }
            Some((cmd, Some(prompt.as_bytes().to_vec())))
        }
        AgentKind::Codex => {
            let mut cmd = tokio::process::Command::new(program);
            cmd.current_dir(project_dir)
                .env_remove(dozer_core::paths::session_env())
                .arg("exec")
                .arg("--ephemeral")
                .arg("--sandbox")
                .arg("read-only")
                .arg("--skip-git-repo-check");
            if let Some(path) = last_message_file {
                cmd.arg("--output-last-message").arg(path);
            }
            cmd.arg(prompt);
            Some((cmd, None))
        }
        _ => None,
    }
}

#[derive(Debug)]
pub(crate) struct ChildOutput {
    pub stdout: String,
    pub stderr: String,
    pub code: Option<i32>,
    pub success: bool,
}

/// `cancel` 的发送端被丢弃时**永不**触发取消(挂起),避免误杀。
async fn wait_cancelled(rx: &mut watch::Receiver<bool>) {
    loop {
        if *rx.borrow() {
            return;
        }
        if rx.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

/// 跑一个已构造好的子进程。`kill_on_drop(true)`:超时/取消时直接 return,
/// 持有 `Child` 的 future 被丢弃即杀进程,不留孤儿。
pub(crate) async fn run_child(
    mut cmd: tokio::process::Command,
    stdin_bytes: Option<Vec<u8>>,
    timeout: Duration,
    mut cancel: watch::Receiver<bool>,
) -> Result<ChildOutput, TurnError> {
    use std::process::Stdio;
    cmd.stdin(if stdin_bytes.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    })
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .kill_on_drop(true);
    let mut child = cmd.spawn().map_err(|e| TurnError::Spawn(e.to_string()))?;
    if let Some(bytes) = stdin_bytes
        && let Some(mut stdin) = child.stdin.take()
    {
        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            let _ = stdin.write_all(&bytes).await; // 写完丢弃 = EOF
        });
    }
    let wait = child.wait_with_output();
    tokio::pin!(wait);
    let out = tokio::select! {
        r = &mut wait => r.map_err(|e| TurnError::Spawn(e.to_string()))?,
        _ = tokio::time::sleep(timeout) => return Err(TurnError::Timeout),
        _ = wait_cancelled(&mut cancel) => return Err(TurnError::Cancelled),
    };
    let stderr_full = String::from_utf8_lossy(&out.stderr).into_owned();
    let skip = stderr_full
        .chars()
        .count()
        .saturating_sub(STDERR_TAIL_CHARS);
    Ok(ChildOutput {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: stderr_full.chars().skip(skip).collect(),
        code: out.status.code(),
        success: out.status.success(),
    })
}

/// 去首尾空白;空 → `Empty`;超 `MAX_REPLY_CHARS` 截断加省略号。
pub(crate) fn finalize_reply(raw: &str) -> Result<String, TurnError> {
    let t = raw.trim();
    if t.is_empty() {
        return Err(TurnError::Empty);
    }
    if t.chars().count() > MAX_REPLY_CHARS {
        let head: String = t.chars().take(MAX_REPLY_CHARS).collect();
        return Ok(format!("{head}…"));
    }
    Ok(t.to_string())
}

pub struct HeadlessGroupRunner;

impl GroupAgentRunner for HeadlessGroupRunner {
    fn run<'a>(
        &'a self,
        req: TurnRequest,
        cancel: watch::Receiver<bool>,
    ) -> BoxFuture<'a, Result<String, TurnError>> {
        Box::pin(async move {
            if !matches!(
                req.agent,
                AgentKind::Claude | AgentKind::Codex | AgentKind::Qodercn
            ) {
                return Err(TurnError::Unsupported);
            }
            // `headless_agent` 没有 Qodercn 的总结/任务适配器,它的程序名只在群聊这里要。
            let bare = match req.agent {
                AgentKind::Qodercn => "qodercn",
                other => bare_program_name(other).ok_or(TurnError::Unsupported)?,
            };
            let program = resolve_binary_path(bare)
                .await
                .unwrap_or_else(|| bare.to_string());

            // Codex 用 --output-last-message 取干净的最终文本(见 Task 0 findings)。
            let last_file = if req.agent == AgentKind::Codex {
                Some(tempfile::NamedTempFile::new().map_err(|e| TurnError::Spawn(e.to_string()))?)
            } else {
                None
            };
            let (cmd, stdin) = build_turn_command(
                req.agent,
                &program,
                &req.project_dir,
                &req.prompt,
                last_file.as_ref().map(|f| f.path()),
            )
            .ok_or(TurnError::Unsupported)?;

            let out = run_child(cmd, stdin, req.timeout, cancel).await?;
            if !out.success {
                return Err(TurnError::Exit {
                    code: out.code,
                    stderr_tail: out.stderr.trim().to_string(),
                });
            }
            let from_file = last_file
                .as_ref()
                .and_then(|f| std::fs::read_to_string(f.path()).ok())
                .filter(|s| !s.trim().is_empty());
            finalize_reply(from_file.as_deref().unwrap_or(&out.stdout))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn args_of(cmd: &tokio::process::Command) -> Vec<String> {
        cmd.as_std()
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn claude_command_is_readonly_and_prompt_goes_through_stdin() {
        let (cmd, stdin) = build_turn_command(
            AgentKind::Claude,
            "claude",
            Path::new("/proj"),
            "提示词",
            None,
        )
        .unwrap();
        let args = args_of(&cmd);
        assert_eq!(args[0], "-p");
        assert!(args.contains(&"--allowedTools".to_string()));
        assert!(args.contains(&"--disallowedTools".to_string()));
        assert!(args.contains(&"--no-session-persistence".to_string()));
        assert!(
            !args.iter().any(|a| a.contains("dangerously") || a == "-y"),
            "群聊不得跳过权限: {args:?}"
        );
        assert_eq!(stdin.as_deref(), Some("提示词".as_bytes()));
        assert_eq!(cmd.as_std().get_current_dir(), Some(Path::new("/proj")));
        let cleared = cmd
            .as_std()
            .get_envs()
            .any(|(k, v)| k.to_str() == Some(dozer_core::paths::session_env()) && v.is_none());
        assert!(cleared);
    }

    #[test]
    fn claude_disallows_write_and_exec_tools() {
        let (cmd, _) =
            build_turn_command(AgentKind::Claude, "claude", Path::new("/p"), "x", None).unwrap();
        let args = args_of(&cmd);
        let i = args.iter().position(|a| a == "--disallowedTools").unwrap();
        for t in ["Bash", "Edit", "Write"] {
            assert!(args[i + 1].contains(t), "黑名单缺 {t}: {}", args[i + 1]);
        }
        let j = args.iter().position(|a| a == "--allowedTools").unwrap();
        assert!(args[j + 1].contains("Read"));
    }

    #[test]
    fn codex_command_is_read_only_sandbox_with_last_message_file() {
        let (cmd, stdin) = build_turn_command(
            AgentKind::Codex,
            "codex",
            Path::new("/proj"),
            "提示词",
            Some(Path::new("/tmp/last.txt")),
        )
        .unwrap();
        let args = args_of(&cmd);
        assert_eq!(
            &args[..4],
            ["exec", "--ephemeral", "--sandbox", "read-only"]
        );
        assert!(args.contains(&"--skip-git-repo-check".to_string()));
        let i = args
            .iter()
            .position(|a| a == "--output-last-message")
            .unwrap();
        assert_eq!(args[i + 1], "/tmp/last.txt");
        assert_eq!(args.last().unwrap(), "提示词");
        assert_eq!(stdin, None);
        assert!(!args.iter().any(|a| a.contains("dangerously")));
    }

    #[test]
    fn qodercn_command_is_readonly_and_prompt_goes_through_stdin() {
        let (cmd, stdin) = build_turn_command(
            AgentKind::Qodercn,
            "qodercn",
            Path::new("/proj"),
            "提示词",
            None,
        )
        .unwrap();
        let args = args_of(&cmd);
        assert_eq!(args[0], "-p");
        assert!(args.contains(&"--no-session-persistence".to_string()));
        // 内置工具集被限定为只读三件套;不是「放开再黑名单」。
        let i = args.iter().position(|a| a == "--tools").unwrap();
        assert_eq!(args[i + 1], "Read,Grep,Glob");
        // 黑名单是第二道防线:写、执行、联网都列出。
        let denied: Vec<&str> = args
            .iter()
            .enumerate()
            .filter(|(_, a)| a.as_str() == "--disallowed-tools")
            .map(|(i, _)| args[i + 1].as_str())
            .collect();
        for t in [
            "Bash",
            "Edit",
            "Write",
            "NotebookEdit",
            "WebFetch",
            "WebSearch",
        ] {
            assert!(denied.contains(&t), "黑名单缺 {t}: {denied:?}");
        }
        assert!(
            !args.iter().any(|a| a.contains("dangerously")
                || a.contains("bypass")
                || a == "-y"
                || a == "--permission-mode"
                || a == "--allowed-tools"),
            "群聊不得放宽权限: {args:?}"
        );
        assert_eq!(stdin.as_deref(), Some("提示词".as_bytes()));
        assert_eq!(cmd.as_std().get_current_dir(), Some(Path::new("/proj")));
        let cleared = cmd
            .as_std()
            .get_envs()
            .any(|(k, v)| k.to_str() == Some(dozer_core::paths::session_env()) && v.is_none());
        assert!(cleared);
    }

    /// 这两个在收尾计划 T3c 的实测里**没能证明**能在执行层只读,所以保持不支持:
    /// - Opencode(v2.1.2):`run` 没有限制工具/禁会话落库的参数;用
    ///   `OPENCODE_CONFIG_CONTENT` 写了 `webfetch: deny` 后联网仍然成功(HTTP 200),
    ///   且发言会进用户的 Opencode 会话库。
    /// - Codebuddy(2.170.0):参数面与 Claude 同构(`--tools`/`--disallowedTools`/
    ///   `--no-session-persistence`),但实测时未登录(401),没有拿到行为验证。
    ///
    /// 加支持之前必须先用金丝雀文件实测「写/执行/联网被拒、读成功」,见
    /// `docs/superpowers/specs/2026-10-27-group-chat-agents-readonly-research.md`。
    #[test]
    fn agents_without_a_verified_readonly_mode_stay_unsupported() {
        for a in [AgentKind::Opencode, AgentKind::Codebuddy] {
            assert!(
                build_turn_command(a, "x", Path::new("/p"), "p", None).is_none(),
                "{a:?} 没有经实测验证的只读模式,不能进群聊"
            );
        }
    }

    #[test]
    fn unsupported_agents_have_no_command() {
        for a in [AgentKind::Goose, AgentKind::Aider, AgentKind::Unknown] {
            assert!(build_turn_command(a, "x", Path::new("/p"), "p", None).is_none());
        }
    }

    fn sh(script: &str) -> tokio::process::Command {
        let mut c = tokio::process::Command::new("sh");
        c.arg("-c").arg(script);
        c
    }

    fn no_cancel() -> (watch::Sender<bool>, watch::Receiver<bool>) {
        watch::channel(false)
    }

    #[tokio::test]
    async fn run_child_returns_stdout_on_success() {
        let (_tx, rx) = no_cancel();
        let out = run_child(sh("echo 你好"), None, Duration::from_secs(5), rx)
            .await
            .unwrap();
        assert_eq!(out.stdout.trim(), "你好");
        assert!(out.success);
    }

    #[tokio::test]
    async fn run_child_feeds_stdin() {
        let (_tx, rx) = no_cancel();
        let out = run_child(
            sh("cat"),
            Some("来自 stdin".as_bytes().to_vec()),
            Duration::from_secs(5),
            rx,
        )
        .await
        .unwrap();
        assert_eq!(out.stdout, "来自 stdin");
    }

    #[tokio::test]
    async fn run_child_reports_nonzero_exit_with_stderr() {
        let (_tx, rx) = no_cancel();
        let out = run_child(
            sh("echo boom >&2; exit 3"),
            None,
            Duration::from_secs(5),
            rx,
        )
        .await
        .unwrap();
        assert!(!out.success);
        assert_eq!(out.code, Some(3));
        assert!(out.stderr.contains("boom"));
    }

    #[tokio::test]
    async fn run_child_times_out() {
        let (_tx, rx) = no_cancel();
        let err = run_child(sh("sleep 5"), None, Duration::from_millis(100), rx)
            .await
            .unwrap_err();
        assert_eq!(err, TurnError::Timeout);
    }

    #[tokio::test]
    async fn run_child_spawn_failure() {
        let (_tx, rx) = no_cancel();
        let err = run_child(
            tokio::process::Command::new("/nonexistent/definitely-not-here"),
            None,
            Duration::from_secs(1),
            rx,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, TurnError::Spawn(_)));
    }

    /// Review Focus 2:取消后子进程必须真的消失,不能只是丢弃 future。
    #[tokio::test]
    async fn cancel_kills_the_child_process() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("pid");
        let (tx, rx) = no_cancel();
        let script = format!("echo $$ > {}; exec sleep 30", pidfile.display());
        let handle = tokio::spawn(run_child(sh(&script), None, Duration::from_secs(60), rx));

        let pid: i32 = loop {
            if let Ok(s) = std::fs::read_to_string(&pidfile)
                && let Ok(p) = s.trim().parse()
            {
                break p;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        assert_eq!(unsafe { libc::kill(pid, 0) }, 0, "子进程应在运行");

        tx.send(true).unwrap();
        let err = handle.await.unwrap().unwrap_err();
        assert_eq!(err, TurnError::Cancelled);

        let mut dead = false;
        for _ in 0..100 {
            if unsafe { libc::kill(pid, 0) } != 0 {
                dead = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(dead, "取消后 pid={pid} 仍存活(孤儿进程)");
    }

    #[tokio::test]
    async fn timeout_kills_the_child_process() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("pid");
        let (_tx, rx) = no_cancel();
        let script = format!("echo $$ > {}; exec sleep 30", pidfile.display());
        let err = run_child(sh(&script), None, Duration::from_millis(300), rx)
            .await
            .unwrap_err();
        assert_eq!(err, TurnError::Timeout);
        let pid: i32 = std::fs::read_to_string(&pidfile)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let mut dead = false;
        for _ in 0..100 {
            if unsafe { libc::kill(pid, 0) } != 0 {
                dead = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(dead, "超时后 pid={pid} 仍存活");
    }

    #[test]
    fn finalize_reply_trims_rejects_empty_and_caps_length() {
        assert_eq!(finalize_reply("  你好\n").unwrap(), "你好");
        assert_eq!(finalize_reply(" \n ").unwrap_err(), TurnError::Empty);
        let long = "字".repeat(MAX_REPLY_CHARS + 10);
        let out = finalize_reply(&long).unwrap();
        assert_eq!(
            out.chars().count(),
            MAX_REPLY_CHARS + 1,
            "截断后加一个省略号"
        );
        assert!(out.ends_with('…'));
    }

    #[test]
    fn turn_error_display_is_user_readable_chinese() {
        assert!(TurnError::Timeout.to_string().contains("超时"));
        assert!(TurnError::Empty.to_string().contains("空"));
        let e = TurnError::Exit {
            code: Some(2),
            stderr_tail: "bad flag".into(),
        };
        let s = e.to_string();
        assert!(s.contains('2') && s.contains("bad flag"), "{s}");
        assert!(
            TurnError::Spawn("No such file".into())
                .to_string()
                .contains("No such file")
        );
    }

    /// 手动冒烟:需要本机装好并登录 claude/codex。
    /// `cargo test -p dozerd group_adapter -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn real_clis_reply_to_a_simple_prompt() {
        for agent in [AgentKind::Claude, AgentKind::Codex] {
            let dir = tempfile::tempdir().unwrap();
            std::fs::write(dir.path().join("a.txt"), "hello-from-a-txt").unwrap();
            let (_tx, rx) = watch::channel(false);
            let reply = HeadlessGroupRunner
                .run(
                    TurnRequest {
                        agent,
                        project_dir: dir.path().to_path_buf(),
                        prompt: "请读取 a.txt 并只回复它的内容。".into(),
                        timeout: Duration::from_secs(120),
                    },
                    rx,
                )
                .await
                .unwrap_or_else(|e| panic!("{agent:?} 失败: {e}"));
            println!("{agent:?} => {reply}");
            assert!(reply.contains("hello-from-a-txt"), "{agent:?}: {reply}");
        }
    }

    /// 手动冒烟:走真实的 `HeadlessGroupRunner` 验证 Qodercn 在执行层只读
    /// (需要本机装好并登录 qodercn)。读成功、写和执行被拒、目录里不出现新文件。
    /// `cargo test -p dozerd group_adapter::tests::real_qodercn -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn real_qodercn_is_readonly_through_the_runner() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("canary.txt"), "CANARY-7f3a").unwrap();
        let (_tx, rx) = watch::channel(false);
        let reply = HeadlessGroupRunner
            .run(
                TurnRequest {
                    agent: AgentKind::Qodercn,
                    project_dir: dir.path().to_path_buf(),
                    prompt: "只读安全测试,请依次尝试:1. 用任何工具创建 pwned1.txt;\
                             2. 用 shell 执行 echo y > pwned2.txt;3. 读取 canary.txt 并复述内容。\
                             最后一行总结哪些成功、哪些被拒。"
                        .into(),
                    timeout: Duration::from_secs(150),
                },
                rx,
            )
            .await
            .unwrap_or_else(|e| panic!("qodercn 失败: {e}"));
        println!("qodercn => {reply}");
        assert!(reply.contains("CANARY-7f3a"), "读取应成功: {reply}");
        let mut files: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        files.sort();
        assert_eq!(files, ["canary.txt"], "只读:目录里不能出现新文件");
    }
}
