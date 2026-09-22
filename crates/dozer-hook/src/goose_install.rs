//! Goose Open Plugins hook 安装器（spec `2026-09-21-goose-agent-integration`）。
//!
//! 在用户级插件目录 `~/.agents/plugins/dozer/` 下生成 `plugin.json` 和
//! `hooks/hooks.json` 两个文件。`hooks.json` 注册八类事件，每类一条 command
//! hook，命令指向 `dozer-hook` 二进制并带上事件名（`'<exe>' goose <Event>`，
//! 见 spec D2）。不设 `on_failure: block`，超时 1 秒——Dozer 不在线、socket
//! 不存在或 JSON 异常都不能改变 Goose 的工具决策或阻塞会话。
//!
//! Dozer 只维护 `~/.agents/plugins/dozer` 这个自有目录，不碰用户其他插件、
//! Goose 的配置或 `sessions.db`。写入用"临时文件 + rename"原子替换，卸载只
//! 删除这两个由 Dozer 生成的文件；目录内出现未知文件时保留目录并 warning，
//! 不递归删除，避免误伤用户自己放进去的东西。

use std::path::{Path, PathBuf};

/// 注册的八类 Goose 事件（官方 hooks 文档 + spec D2）。
const GOOSE_EVENTS: [&str; 8] = [
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PostToolUseFailure",
    "AfterFileEdit",
    "Stop",
    "SessionEnd",
];

/// 用户级插件发现根目录。`DOZER_GOOSE_PLUGINS_DIR` 覆盖用于测试（同
/// `install.rs` 的 `DOZER_CLAUDE_SETTINGS` 手法）；默认 `~/.agents/plugins`。
pub fn plugins_dir() -> PathBuf {
    if let Ok(p) = std::env::var("DOZER_GOOSE_PLUGINS_DIR") {
        return PathBuf::from(p);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
    PathBuf::from(home).join(".agents").join("plugins")
}

/// Dozer 独占的插件子目录（`plugin.json`/`hooks/hooks.json` 所在处）。
fn dozer_plugin_dir(base: &Path) -> PathBuf {
    base.join("dozer")
}

/// 把一段（可能是绝对路径、可能带空格）安全地包进 `sh -c` 的单引号里。
fn sh_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn build_plugin_json() -> String {
    serde_json::json!({
        "name": "dozer",
        "version": "0.1.0",
        "description": "Dozer governance & acceptance hooks"
    })
    .to_string()
}

/// 八类事件 → 一条 command hook（命令带事件名，超时 1 秒，不写
/// `on_failure`）。`exe` 是 `dozer-hook` 的绝对路径。
fn build_hooks_json(exe: &str) -> String {
    let quoted = sh_single_quote(exe);
    let mut hooks = serde_json::Map::new();
    for event in GOOSE_EVENTS {
        let action = serde_json::json!({
            "type": "command",
            "command": format!("{quoted} goose {event}"),
            "timeout": 1,
        });
        let rule = serde_json::json!({ "hooks": [action] });
        hooks.insert(event.to_string(), serde_json::json!([rule]));
    }
    serde_json::json!({ "hooks": hooks }).to_string()
}

/// 原子写：先写同目录临时文件再 rename 覆盖，失败不留下半份配置（spec §5）。
fn atomic_write(path: &Path, content: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file_name = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "tmp".to_string());
    let tmp = path.with_file_name(format!(".{file_name}.tmp-{}", std::process::id()));
    std::fs::write(&tmp, content)?;
    std::fs::rename(&tmp, path)
}

/// CLI 场景（`dozer-hook install goose`）用：调用方就是 `dozer-hook` 自身，
/// `current_exe()` 天然指向正确的二进制。GUI 场景（`dozer-app` 在 agent
/// 启动时静默自动注册）不能走这条路——见 `run_at_with_exe`。
pub fn run_at(dir: &Path, install: bool) -> i32 {
    let exe = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "dozer-hook".into());
    run_at_with_exe(dir, install, &exe)
}

/// `run_at` 的可测试内核：exe 路径由调用方显式传入（`dozer-app` 在自己进程
/// 内调用这层，沿用 `current_exe()` 会把 hook command 写成 `dozer-app` 自己的
/// 可执行文件路径，跟 `install::run_at_with_exe` 同一根因）。
pub fn run_at_with_exe(dir: &Path, install: bool, exe: &str) -> i32 {
    let plugin_dir = dozer_plugin_dir(dir);
    if install {
        if let Err(e) = atomic_write(&plugin_dir.join("plugin.json"), &build_plugin_json()) {
            eprintln!("写 plugin.json 失败: {e}");
            return 1;
        }
        if let Err(e) = atomic_write(
            &plugin_dir.join("hooks").join("hooks.json"),
            &build_hooks_json(exe),
        ) {
            eprintln!("写 hooks.json 失败: {e}");
            return 1;
        }
        println!("已安装: {}", plugin_dir.display());
    } else {
        uninstall(&plugin_dir);
    }
    0
}

/// 卸载：只删 `plugin.json` 与 `hooks/hooks.json` 两个自有文件，然后尽力清
/// 空目录。目录里出现未知文件（不是这两个）时保留目录并 warning，绝不递归
/// 删除。
fn uninstall(plugin_dir: &Path) {
    let hooks_json = plugin_dir.join("hooks").join("hooks.json");
    if hooks_json.exists()
        && let Err(e) = std::fs::remove_file(&hooks_json)
    {
        eprintln!("删 {} 失败: {e}", hooks_json.display());
    }
    // hooks/ 子目录若已空则顺手删掉（删不掉不算失败）。
    let _ = std::fs::remove_dir(plugin_dir.join("hooks"));

    let plugin_json = plugin_dir.join("plugin.json");
    if plugin_json.exists()
        && let Err(e) = std::fs::remove_file(&plugin_json)
    {
        eprintln!("删 {} 失败: {e}", plugin_json.display());
    }

    // 只剩未知文件时保留目录并 warning，不递归删。
    match std::fs::remove_dir(plugin_dir) {
        Ok(()) => println!("已卸载: {}", plugin_dir.display()),
        Err(_) if plugin_dir.exists() => {
            eprintln!(
                "保留 {}（目录内还有 Dozer 不认识的其它文件，不递归删除）",
                plugin_dir.display()
            );
        }
        Err(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    fn install(exe: &str, dir: &Path) -> i32 {
        run_at_with_exe(dir, true, exe)
    }

    #[test]
    fn install_writes_plugin_json_and_hooks_json() {
        let d = base_dir();
        assert_eq!(install("/opt/dozer-hook", d.path()), 0);
        let plugin_json = d.path().join("dozer/plugin.json");
        let hooks_json = d.path().join("dozer/hooks/hooks.json");
        assert!(plugin_json.exists());
        assert!(hooks_json.exists());
        let root: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&plugin_json).unwrap()).unwrap();
        assert_eq!(root["name"], "dozer");
    }

    #[test]
    fn hooks_json_registers_all_eight_events_with_quoted_exe_and_event_arg() {
        let d = base_dir();
        assert_eq!(
            install(
                "/Applications/Dozer AI Coder.app/Contents/MacOS/dozer-hook",
                d.path()
            ),
            0
        );
        let hooks_json = d.path().join("dozer/hooks/hooks.json");
        let raw = std::fs::read_to_string(&hooks_json).unwrap();
        let root: serde_json::Value = serde_json::from_str(&raw).unwrap();
        for event in GOOSE_EVENTS {
            let cmd = root["hooks"][event][0]["hooks"][0]["command"]
                .as_str()
                .unwrap()
                .to_string();
            assert!(
                cmd.starts_with('\'') && cmd.ends_with(&format!("goose {event}")),
                "{event}: {cmd}"
            );
            assert_eq!(root["hooks"][event][0]["hooks"][0]["timeout"], 1);
        }
        // 不写 on_failure:block——Doozer 故障不得阻塞 Goose 工具决策。
        assert!(
            !raw.contains("on_failure"),
            "hooks.json 不该出现 on_failure"
        );
    }

    #[test]
    fn install_is_idempotent() {
        let d = base_dir();
        assert_eq!(install("/opt/dozer-hook", d.path()), 0);
        assert_eq!(install("/opt/dozer-hook", d.path()), 0);
        assert!(d.path().join("dozer/plugin.json").exists());
    }

    #[test]
    fn uninstall_removes_both_files_and_empty_dirs() {
        let d = base_dir();
        assert_eq!(install("/opt/dozer-hook", d.path()), 0);
        assert_eq!(run_at_with_exe(d.path(), false, "/opt/dozer-hook"), 0);
        assert!(!d.path().join("dozer/plugin.json").exists());
        assert!(!d.path().join("dozer/hooks/hooks.json").exists());
        assert!(!d.path().join("dozer").exists());
    }

    #[test]
    fn uninstall_keeps_dir_with_unknown_files() {
        let d = base_dir();
        assert_eq!(install("/opt/dozer-hook", d.path()), 0);
        // 用户自己往 dozer/ 里放了个未知文件。
        std::fs::write(d.path().join("dozer/user-notes.txt"), "hi").unwrap();
        assert_eq!(run_at_with_exe(d.path(), false, "/opt/dozer-hook"), 0);
        // 我们的两个文件被删，但目录因含未知文件而保留。
        assert!(!d.path().join("dozer/plugin.json").exists());
        assert!(d.path().join("dozer/user-notes.txt").exists());
        assert!(d.path().join("dozer").exists());
    }

    #[test]
    fn uninstall_on_missing_dir_does_not_error() {
        let d = base_dir();
        assert_eq!(
            run_at_with_exe(&d.path().join("nope"), false, "/x/dozer-hook"),
            0
        );
    }

    #[test]
    fn run_at_with_exe_uses_passed_exe_not_current_exe() {
        let d = base_dir();
        assert_eq!(install("/opt/my-dozer-hook", d.path()), 0);
        let hooks_json = std::fs::read_to_string(d.path().join("dozer/hooks/hooks.json")).unwrap();
        assert!(hooks_json.contains("/opt/my-dozer-hook"), "{hooks_json}");
    }

    #[test]
    fn plugins_dir_honors_env_override() {
        unsafe { std::env::set_var("DOZER_GOOSE_PLUGINS_DIR", "/tmp/probe-goose-plugins") };
        assert_eq!(plugins_dir(), PathBuf::from("/tmp/probe-goose-plugins"));
        unsafe { std::env::remove_var("DOZER_GOOSE_PLUGINS_DIR") };
    }
}
