//! Claude Code hooks 注册：幂等增量合并 ~/.claude/settings.json，
//! 只增删自己的条目，绝不动用户其他配置（spec P1e D5）。

use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// 注册的 hook 事件集（与 dozerd server::agent_state_for 的映射面一致）。
pub const EVENTS: [&str; 7] = [
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "Notification",
    "Stop",
    "SessionEnd",
];

/// agent 名 → 该 agent 的 hook 配置文件路径。CodeBuddy/Codex/Qoder 走各自
/// 的全局配置文件（与 Claude 同构的 JSON 补丁机制，结构已通过官方文档
/// 核实、spike 现场验证），环境变量覆盖用于测试，跟既有 Claude 路径同一套手法。
pub fn settings_path_for(agent: &str) -> PathBuf {
    match agent {
        "codebuddy" => {
            if let Ok(p) = std::env::var("DOZER_CODEBUDDY_SETTINGS") {
                return PathBuf::from(p);
            }
            let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
            PathBuf::from(home).join(".codebuddy").join("settings.json")
        }
        "codex" => {
            if let Ok(p) = std::env::var("DOZER_CODEX_SETTINGS") {
                return PathBuf::from(p);
            }
            let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
            PathBuf::from(home).join(".codex").join("hooks.json")
        }
        "qoder" => {
            if let Ok(p) = std::env::var("DOZER_QODER_SETTINGS") {
                return PathBuf::from(p);
            }
            let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
            PathBuf::from(home).join(".qoder").join("settings.json")
        }
        _ => {
            if let Ok(p) = std::env::var("DOZER_CLAUDE_SETTINGS") {
                return PathBuf::from(p);
            }
            let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
            PathBuf::from(home).join(".claude").join("settings.json")
        }
    }
}

fn entry_is_dozer(entry: &Value) -> bool {
    // 测试/开发环境下二进制名可能是 dozer_hook-<hash>（cargo test 的
    // deps 命名），发行名是 dozer-hook——两种拼写都认。
    entry["hooks"]
        .as_array()
        .map(|hs| {
            hs.iter().any(|h| {
                h["command"]
                    .as_str()
                    .is_some_and(|c| c.contains("dozer-hook") || c.contains("dozer_hook"))
            })
        })
        .unwrap_or(false)
}

/// CLI 场景（`dozer-hook install <agent>`）用：调用方就是 `dozer-hook`
/// 自身，`current_exe()` 天然指向正确的二进制。GUI 场景（`dozer-app` 在
/// agent 启动时静默自动注册）不能走这条路——见 `run_at_with_exe`。
pub fn run_at(path: &Path, agent: &str, install: bool) -> i32 {
    let exe = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "dozer-hook".into());
    run_at_with_exe(path, agent, install, &exe)
}

/// 单引号包住 exe 路径，防止路径里的空格被 hook 命令的执行方（Claude
/// Code/CodeBuddy 等按空白切分/走 shell 解释这个 command 字符串）当成参数
/// 分隔符截断——`Dozer AI Coder.app` 这个 bundle 名本身就带空格，是这次
/// 要修的线上事故的根因（写进去的路径在空格处断开，被截成
/// `/Applications/Dozer`，报 `no such file or directory`）。单引号内部若
/// 本身含单引号，用 `'\''` 转义（跟 `keymap.rs` 现成的同款手法一致）。
fn shell_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// `run_at` 的可测试内核：exe 路径由调用方显式传入，而不是隐式读
/// `current_exe()`——`dozer-app` 在自己进程内直接调用这层（省掉 spawn
/// 一个子进程的开销），如果沿用 `current_exe()` 会拿到 `dozer-app` 自己
/// 的可执行文件路径而不是 `dozer-hook` 的，写出一条指向错误二进制、且
/// 因为不含 "dozer-hook" 子串而被 `entry_is_dozer` 认不出、每次都重复
/// 追加的 hook 命令（2026-08 线上事故：`~/.claude/settings.json` 里同一个
/// 事件堆出 3-4 条重复且指向 `dozer` GUI 二进制的坏 hook）。
pub fn run_at_with_exe(path: &Path, agent: &str, install: bool, exe: &str) -> i32 {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => "{}".into(),
        Err(e) => {
            eprintln!("读 {} 失败: {e}", path.display());
            return 1;
        }
    };
    let mut root: Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("解析 {} 失败，拒绝写入: {e}", path.display());
            return 1;
        }
    };
    let exe = shell_single_quote(exe);

    let Some(obj) = root.as_object_mut() else {
        eprintln!("{} 顶层不是对象，拒绝写入", path.display());
        return 1;
    };
    let hooks = obj.entry("hooks").or_insert_with(|| json!({}));
    let Some(hooks) = hooks.as_object_mut() else {
        eprintln!("hooks 段不是对象，拒绝写入");
        return 1;
    };

    for ev in EVENTS {
        let arr = hooks.entry(ev).or_insert_with(|| json!([]));
        let Some(arr) = arr.as_array_mut() else {
            continue;
        };
        arr.retain(|e| !entry_is_dozer(e));
        if install {
            arr.push(json!({
                "hooks": [{ "type": "command", "command": format!("{exe} {agent} {ev}") }]
            }));
        }
    }
    // 卸载后为空的事件键移除，不留空数组
    let empties: Vec<String> = hooks
        .iter()
        .filter(|(_, v)| v.as_array().is_some_and(|a| a.is_empty()))
        .map(|(k, _)| k.clone())
        .collect();
    for k in empties {
        hooks.remove(&k);
    }

    if let Some(parent) = path.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        eprintln!("建目录失败: {e}");
        return 1;
    }
    let out = serde_json::to_string_pretty(&root).expect("json serializes") + "\n";
    if let Err(e) = std::fs::write(path, out) {
        eprintln!("写 {} 失败: {e}", path.display());
        return 1;
    }
    println!(
        "{}: {}",
        if install { "已注册" } else { "已移除" },
        path.display()
    );
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(path: &std::path::Path) -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn install_creates_settings_and_registers_all_events() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        assert_eq!(run_at(&path, "claude", true), 0);
        let root = read(&path);
        for ev in EVENTS {
            let arr = root["hooks"][ev].as_array().expect(ev);
            assert_eq!(arr.len(), 1, "{ev}");
            let cmd = arr[0]["hooks"][0]["command"].as_str().unwrap();
            assert!(
                cmd.contains("dozer-hook") || cmd.contains("dozer_hook"),
                "{cmd}"
            );
            assert!(cmd.contains(" claude "), "{cmd}: 应携带 agent 标识");
            assert!(cmd.ends_with(ev), "{cmd}");
        }
    }

    #[test]
    fn run_at_with_exe_quotes_paths_containing_spaces() {
        // 回归线上事故：`/Applications/Dozer AI Coder.app/.../dozer-hook`
        // 这类带空格的安装路径，如果不加引号拼进 command 字符串，会被 hook
        // 的执行方在空格处截断成 `/Applications/Dozer`，报
        // "no such file or directory"。
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let exe = "/Applications/Dozer AI Coder.app/Contents/MacOS/dozer-hook";
        assert_eq!(run_at_with_exe(&path, "claude", true, exe), 0);
        let root = read(&path);
        let cmd = root["hooks"]["Stop"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        assert_eq!(
            cmd,
            "'/Applications/Dozer AI Coder.app/Contents/MacOS/dozer-hook' claude Stop"
        );
    }

    #[test]
    fn run_at_with_exe_uses_the_passed_exe_not_current_exe() {
        // ensure_hook_installed（dozer-app）调这层是为了绕开 current_exe()
        // 在跨进程场景下拿错二进制的问题——这里直接断言传参优先。
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        assert_eq!(run_at_with_exe(&path, "claude", true, "/opt/dozer-hook"), 0);
        let root = read(&path);
        let cmd = root["hooks"]["Stop"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        assert_eq!(cmd, "'/opt/dozer-hook' claude Stop");
    }

    #[test]
    fn install_writes_agent_specific_command_for_codebuddy() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        assert_eq!(run_at(&path, "codebuddy", true), 0);
        let root = read(&path);
        let cmd = root["hooks"]["Stop"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        assert!(cmd.contains(" codebuddy "), "{cmd}");
    }

    #[test]
    fn settings_path_for_codebuddy_points_at_codebuddy_dir() {
        // DOZER_CODEBUDDY_SETTINGS 覆盖，跟既有 DOZER_CLAUDE_SETTINGS 同一套测试手法。
        unsafe { std::env::set_var("DOZER_CODEBUDDY_SETTINGS", "/tmp/probe-codebuddy.json") };
        assert_eq!(
            settings_path_for("codebuddy"),
            std::path::PathBuf::from("/tmp/probe-codebuddy.json")
        );
        unsafe { std::env::remove_var("DOZER_CODEBUDDY_SETTINGS") };
    }

    #[test]
    fn install_writes_agent_specific_command_for_codex() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hooks.json");
        assert_eq!(run_at(&path, "codex", true), 0);
        let root = read(&path);
        let cmd = root["hooks"]["Stop"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        assert!(cmd.contains(" codex "), "{cmd}");
    }

    #[test]
    fn settings_path_for_codex_points_at_codex_hooks_json() {
        unsafe { std::env::set_var("DOZER_CODEX_SETTINGS", "/tmp/probe-codex.json") };
        assert_eq!(
            settings_path_for("codex"),
            std::path::PathBuf::from("/tmp/probe-codex.json")
        );
        unsafe { std::env::remove_var("DOZER_CODEX_SETTINGS") };
    }

    #[test]
    fn install_writes_agent_specific_command_for_qoder() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        assert_eq!(run_at(&path, "qoder", true), 0);
        let root = read(&path);
        let cmd = root["hooks"]["Stop"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        assert!(cmd.contains(" qoder "), "{cmd}");
    }

    #[test]
    fn settings_path_for_qoder_points_at_qoder_dir() {
        unsafe { std::env::set_var("DOZER_QODER_SETTINGS", "/tmp/probe-qoder.json") };
        assert_eq!(
            settings_path_for("qoder"),
            std::path::PathBuf::from("/tmp/probe-qoder.json")
        );
        unsafe { std::env::remove_var("DOZER_QODER_SETTINGS") };
    }

    #[test]
    fn install_is_idempotent_and_preserves_foreign_hooks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{"model":"opus","hooks":{"Stop":[{"hooks":[{"type":"command","command":"other-tool"}]}]}}"#,
        )
        .unwrap();
        assert_eq!(run_at(&path, "claude", true), 0);
        assert_eq!(run_at(&path, "claude", true), 0);
        let root = read(&path);
        assert_eq!(root["model"], "opus", "无关配置保留");
        let stop = root["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 2, "他人 hook 保留 + 自己恰一条");
        assert!(
            stop.iter()
                .any(|e| e["hooks"][0]["command"] == "other-tool")
        );
    }

    #[test]
    fn uninstall_removes_only_ours() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"other-tool"}]}]}}"#,
        )
        .unwrap();
        assert_eq!(run_at(&path, "claude", true), 0);
        assert_eq!(run_at(&path, "claude", false), 0);
        let root = read(&path);
        let stop = root["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 1);
        assert_eq!(stop[0]["hooks"][0]["command"], "other-tool");
        assert!(root["hooks"]["UserPromptSubmit"].is_null(), "空键移除");
    }

    #[test]
    fn malformed_settings_refused_without_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "{broken").unwrap();
        assert_eq!(run_at(&path, "claude", true), 1);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{broken",
            "拒写不破坏"
        );
    }
}
