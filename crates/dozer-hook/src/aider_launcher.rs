//! Aider launcher（见 spec `2026-09-22-aider-agent-integration` D1/D3）。
//!
//! `dozer-hook launch aider` 拉起真实 `aider`，stdio/TTY 原样继承（用户看到的
//! 仍是原生 Aider TUI）。launcher 同时：上报 SessionStart、后台轮询 input
//! history 增长上报 UserPromptSubmit、等 aider 退出后做最后一次 transcript 同步
//! 并上报 SessionEnd。notification command 是独立的 `dozer-hook aider Stop`
//! 进程（见 `handle_stop`），先 bridge 同步再发 Stop。

use dozer_core::agent_paths::{AiderSessionPaths, aider_session_paths, home_dir};
use dozer_core::protocol::AgentKind;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// POSIX 单引号转义（notification command 里的 dozer-hook 绝对路径可能含空格）。
pub fn sh_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// 构造 aider 子进程命令（不 spawn，便于单测）。只注入 Dozer 管理的 history /
/// notification 参数；model/provider/git/确认策略一律不碰（用户 `.aider.conf.yml`
/// 生效）。stdio 保持 inherit。`extra_args` 原样 `Command::arg` 透传。
pub fn build_aider_command(
    paths: &AiderSessionPaths,
    hook_exe: &str,
    extra_args: &[String],
) -> Command {
    let mut cmd = Command::new("aider");
    cmd.arg("--chat-history-file").arg(&paths.chat_history);
    cmd.arg("--input-history-file").arg(&paths.input_history);
    cmd.arg("--notifications");
    cmd.arg("--notifications-command")
        .arg(format!("{} aider Stop", sh_single_quote(hook_exe)));
    cmd.args(extra_args);
    cmd
}

/// `dozer-hook launch aider` 入口：拉起 aider、上报生命周期、后台 watcher、
/// 最后同步 transcript。返回 aider 退出码（launcher 自身不吞子进程退出码）。
pub fn launch(extra_args: &[String]) -> i32 {
    let Ok(session_id) = std::env::var("DOZER_SESSION_ID") else {
        eprintln!("dozer-hook launch aider: 缺少 DOZER_SESSION_ID，拒绝进入集成模式");
        return 1;
    };
    let Ok(cwd) = std::env::current_dir() else {
        eprintln!("dozer-hook launch aider: 无法确定当前工作目录");
        return 1;
    };
    let Some(paths) = aider_session_paths(&home_dir(), &cwd, &session_id) else {
        eprintln!("dozer-hook launch aider: session id 非法");
        return 1;
    };
    let hook_exe = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "dozer-hook".into());

    crate::forward::send_hook_event(
        AgentKind::Aider,
        &session_id,
        "SessionStart",
        now_ms(),
        &serde_json::json!({}),
    );

    let mut cmd = build_aider_command(&paths, &hook_exe, extra_args);
    cmd.env("DOZER_AIDER_CWD", &cwd);
    cmd.env("DOZER_AIDER_CHAT_HISTORY", &paths.chat_history);
    cmd.env("DOZER_AIDER_INPUT_HISTORY", &paths.input_history);
    cmd.env("DOZER_AIDER_CANONICAL_TRANSCRIPT", &paths.canonical);
    cmd.env("DOZER_AIDER_BRIDGE_STATE", &paths.bridge_state);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("找不到或启动失败 aider: {e}");
            crate::forward::send_hook_event(
                AgentKind::Aider,
                &session_id,
                "SessionEnd",
                now_ms(),
                &serde_json::json!({ "transcript_path": paths.canonical }),
            );
            return 1;
        }
    };

    let shutdown = Arc::new(AtomicBool::new(false));
    let watcher = {
        let input = paths.input_history.clone();
        let sid = session_id.clone();
        let sd = Arc::clone(&shutdown);
        std::thread::spawn(move || watch_input(&input, &sid, &sd))
    };

    let status = child.wait();
    shutdown.store(true, Ordering::SeqCst);
    let _ = watcher.join();

    let _ = crate::aider_bridge::sync(
        &paths.chat_history,
        &paths.input_history,
        &paths.canonical,
        &paths.bridge_state,
    );
    crate::forward::send_hook_event(
        AgentKind::Aider,
        &session_id,
        "SessionEnd",
        now_ms(),
        &serde_json::json!({ "transcript_path": paths.canonical }),
    );

    match status {
        Ok(s) => s.code().unwrap_or(0),
        Err(_) => 1,
    }
}

/// `dozer-hook aider Stop` 入口（notification command）：先 bridge 同步
/// Markdown → canonical，再发 Stop。缺任何受控环境变量时静默退出 0（spec §6：
/// notification 子命令恒退出 0，不阻塞 Aider）。
pub fn handle_stop() {
    let Ok(session_id) = std::env::var("DOZER_SESSION_ID") else {
        return;
    };
    let (Some(chat), Some(input), Some(canonical), Some(bridge)) = (
        std::env::var("DOZER_AIDER_CHAT_HISTORY").ok(),
        std::env::var("DOZER_AIDER_INPUT_HISTORY").ok(),
        std::env::var("DOZER_AIDER_CANONICAL_TRANSCRIPT").ok(),
        std::env::var("DOZER_AIDER_BRIDGE_STATE").ok(),
    ) else {
        return;
    };
    let _ = crate::aider_bridge::sync(
        Path::new(&chat),
        Path::new(&input),
        Path::new(&canonical),
        Path::new(&bridge),
    );
    crate::forward::send_hook_event(
        AgentKind::Aider,
        &session_id,
        "Stop",
        now_ms(),
        &serde_json::json!({ "transcript_path": canonical }),
    );
}

#[derive(Debug, Default, Clone, Copy)]
pub struct InputWatcherState {
    seen_entries: usize,
    file_len: usize,
}

/// 从 prompt-toolkit `FileHistory` 文本解析出条目（`+` 前缀 = 新条目，`#` =
/// 时间戳元数据，空行分隔，多行条目是续行）。
/// slash command（`/...`）默认不触发 Running（spec D3：多数是本地控制命令，
/// 不代表一次 LLM turn）。
fn is_reportable(entry: &str) -> bool {
    !entry.trim_start().starts_with('/')
}

/// 读取当前文件内容，返回**新增**的、需要上报为 UserPromptSubmit 的条目文本。
/// 文件被截断/重建（长度倒退）时重置 `seen_entries`，避免漏报或重复。
pub fn detect_new_inputs(content: &str, state: &mut InputWatcherState) -> Vec<String> {
    if content.len() < state.file_len {
        state.seen_entries = 0;
    }
    state.file_len = content.len();
    let entries = crate::aider_bridge::parse_input_history(content);
    let new_entries: Vec<String> = entries
        .iter()
        .skip(state.seen_entries)
        .filter(|e| is_reportable(e))
        .cloned()
        .collect();
    state.seen_entries = entries.len();
    new_entries
}

fn watch_input(input_history: &Path, session_id: &str, shutdown: &AtomicBool) {
    let mut state = InputWatcherState::default();
    while !shutdown.load(Ordering::SeqCst) {
        if let Ok(content) = fs::read_to_string(input_history) {
            for _entry in detect_new_inputs(&content, &mut state) {
                crate::forward::send_hook_event(
                    AgentKind::Aider,
                    session_id,
                    "UserPromptSubmit",
                    now_ms(),
                    &serde_json::json!({}),
                );
            }
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(tmp: &Path) -> AiderSessionPaths {
        AiderSessionPaths {
            chat_history: tmp.join("s.chat.md"),
            input_history: tmp.join("s.input.history"),
            canonical: tmp.join("s.jsonl"),
            bridge_state: tmp.join("s.bridge.json"),
        }
    }

    #[test]
    fn build_aider_command_injects_history_and_notification_only() {
        let dir = tempfile::tempdir().unwrap();
        let p = paths(dir.path());
        let hook = "/Applications/Dozer AI Coder.app/Contents/MacOS/dozer-hook";
        let cmd = build_aider_command(&p, hook, &[]);
        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(args.contains(&"--chat-history-file".to_string()));
        assert!(args.contains(&"--notifications".to_string()));
        let idx = args
            .iter()
            .position(|a| a == "--notifications-command")
            .unwrap();
        assert_eq!(
            args[idx + 1],
            format!("'{hook}' aider Stop"),
            "notification command 必须单引号包裹 hook 绝对路径"
        );
        // 不改写 model/provider/git/确认策略。
        assert!(!args.contains(&"--model".to_string()));
        assert!(!args.contains(&"--no-auto-commits".to_string()));
        assert!(!args.contains(&"--yes-always".to_string()));
    }

    #[test]
    fn build_aider_command_passes_extra_args_verbatim() {
        let dir = tempfile::tempdir().unwrap();
        let p = paths(dir.path());
        let cmd = build_aider_command(&p, "/x/dozer-hook", &["--dark-mode".to_string()]);
        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(args.contains(&"--dark-mode".to_string()));
    }

    #[test]
    fn detect_new_inputs_reports_single_line_and_skips_slash() {
        let content = "# 2026-09-22 09:37:05.602984\n+Reply hello\n";
        let mut state = InputWatcherState::default();
        assert_eq!(detect_new_inputs(content, &mut state), vec!["Reply hello"]);

        let slash =
            "# 2026-09-22 09:37:06.0\n+Reply hello\n\n# 2026-09-22 09:37:07.0\n+/model foo\n";
        assert_eq!(
            detect_new_inputs(slash, &mut state),
            Vec::<String>::new(),
            "slash command 不触发"
        );
    }

    #[test]
    fn detect_new_inputs_handles_multiline_and_duplicate_metadata() {
        let content = "# 2026-09-22 09:37:05.0\n+line one\n+line two\n";
        let mut state = InputWatcherState::default();
        assert_eq!(
            detect_new_inputs(content, &mut state),
            vec!["line one\nline two"]
        );
        // 重复 metadata(时间戳行)不影响;同内容不再上报。
        assert_eq!(detect_new_inputs(content, &mut state), Vec::<String>::new());
    }

    #[test]
    fn detect_new_inputs_skips_entire_multiline_slash_command() {
        let content = "# 2026-09-22 09:37:05.0\n+/ask explain\n+this code\n";
        let mut state = InputWatcherState::default();
        assert_eq!(detect_new_inputs(content, &mut state), Vec::<String>::new());
    }

    #[test]
    fn detect_new_inputs_resets_on_truncation() {
        let mut state = InputWatcherState::default();
        assert_eq!(
            detect_new_inputs("# t\n+first\n", &mut state),
            vec!["first"]
        );
        // 文件被重建/截断:长度倒退 → seen_entries 重置,重新上报。
        assert_eq!(
            detect_new_inputs("# t\n+first\n\n# t2\n+second\n", &mut state),
            vec!["second"]
        );
    }
}
