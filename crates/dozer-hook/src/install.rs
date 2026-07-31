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

/// agent 名 → 该 agent 的 hook 配置文件路径。CodeBuddy 走
/// `~/.codebuddy/settings.json`（Task 1 spike 确认的机制），环境变量
/// 覆盖用于测试，跟既有 Claude 路径同一套手法。
pub fn settings_path_for(agent: &str) -> PathBuf {
    match agent {
        "codebuddy" => {
            if let Ok(p) = std::env::var("DOZER_CODEBUDDY_SETTINGS") {
                return PathBuf::from(p);
            }
            let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
            PathBuf::from(home).join(".codebuddy").join("settings.json")
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

pub fn run_at(path: &Path, agent: &str, install: bool) -> i32 {
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
    let exe = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "dozer-hook".into());

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
