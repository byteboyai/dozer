//! 四家已确认支持 MCP 的 agent 的安装器：幂等 JSON/TOML 补丁，只碰自己
//! 写的 `dozer` 条目，仿 `dozer-hook/src/install.rs` 的手法。Qoder/Kilo/
//! V8agent 不在这次范围（见设计文档"非目标"）。

use std::path::{Path, PathBuf};

fn exe_path() -> String {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "dozer-mcp".into())
}

/// Claude/Codebuddy 共用：`{"mcpServers": {"dozer": {...}}}`。
fn run_at_claude_like(path: &Path, install: bool) -> i32 {
    let text = std::fs::read_to_string(path).unwrap_or_else(|_| "{}".into());
    let mut root: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("解析 {} 失败，拒绝写入: {e}", path.display());
            return 1;
        }
    };
    let Some(obj) = root.as_object_mut() else {
        eprintln!("{} 顶层不是对象，拒绝写入", path.display());
        return 1;
    };
    let servers = obj
        .entry("mcpServers")
        .or_insert_with(|| serde_json::json!({}));
    let Some(servers) = servers.as_object_mut() else {
        eprintln!("mcpServers 段不是对象，拒绝写入");
        return 1;
    };
    if install {
        servers.insert(
            "dozer".into(),
            serde_json::json!({
                "type": "stdio",
                "command": exe_path(),
                "args": ["serve"],
                "env": {}
            }),
        );
    } else {
        servers.remove("dozer");
    }
    write_json(path, &root)
}

/// Codex：`[mcp_servers.dozer]` TOML 表。
fn run_at_codex(path: &Path, install: bool) -> i32 {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut root: toml::Value = if text.trim().is_empty() {
        toml::Value::Table(Default::default())
    } else {
        // 注意：toml 0.9.12 的 `Value: FromStr` 实现有 bug，对任何非空文档
        // （哪怕只有 `a = 1`）都会报 "unexpected content, expected nothing"；
        // `toml::from_str::<Value>` 走的是另一条正常路径，两者本应等价。
        match toml::from_str::<toml::Value>(&text) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("解析 {} 失败，拒绝写入: {e}", path.display());
                return 1;
            }
        }
    };
    let Some(root_table) = root.as_table_mut() else {
        eprintln!("{} 顶层不是 TOML table，拒绝写入", path.display());
        return 1;
    };
    let servers = root_table
        .entry("mcp_servers")
        .or_insert_with(|| toml::Value::Table(Default::default()));
    let Some(servers) = servers.as_table_mut() else {
        eprintln!("mcp_servers 段不是 table，拒绝写入");
        return 1;
    };
    if install {
        let mut entry = toml::value::Table::new();
        entry.insert("command".into(), toml::Value::String(exe_path()));
        entry.insert(
            "args".into(),
            toml::Value::Array(vec![toml::Value::String("serve".into())]),
        );
        servers.insert("dozer".into(), toml::Value::Table(entry));
    } else {
        servers.remove("dozer");
    }
    let out = match toml::to_string_pretty(&root) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("序列化 TOML 失败: {e}");
            return 1;
        }
    };
    write_text(path, &out)
}

/// OpenCode：`{"mcp": {"dozer": {"type":"local","command":[...],"enabled":true}}}`。
fn run_at_opencode(path: &Path, install: bool) -> i32 {
    let text = std::fs::read_to_string(path).unwrap_or_else(|_| "{}".into());
    let mut root: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("解析 {} 失败，拒绝写入: {e}", path.display());
            return 1;
        }
    };
    let Some(obj) = root.as_object_mut() else {
        eprintln!("{} 顶层不是对象，拒绝写入", path.display());
        return 1;
    };
    let servers = obj.entry("mcp").or_insert_with(|| serde_json::json!({}));
    let Some(servers) = servers.as_object_mut() else {
        eprintln!("mcp 段不是对象，拒绝写入");
        return 1;
    };
    if install {
        servers.insert(
            "dozer".into(),
            serde_json::json!({
                "type": "local",
                "command": [exe_path(), "serve"],
                "enabled": true
            }),
        );
    } else {
        servers.remove("dozer");
    }
    write_json(path, &root)
}

fn write_json(path: &Path, value: &serde_json::Value) -> i32 {
    write_text(
        path,
        &(serde_json::to_string_pretty(value).expect("json serializes") + "\n"),
    )
}

fn write_text(path: &Path, text: &str) -> i32 {
    if let Some(parent) = path.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        eprintln!("建目录失败: {e}");
        return 1;
    }
    if let Err(e) = std::fs::write(path, text) {
        eprintln!("写 {} 失败: {e}", path.display());
        return 1;
    }
    println!(
        "{}: {}",
        path.display(),
        if text.is_empty() {
            "已清空"
        } else {
            "已更新"
        }
    );
    0
}

/// (配置文件路径, 该 agent 对应的安装/卸载处理函数)。
type ConfigTarget = (PathBuf, fn(&Path, bool) -> i32);

fn config_path_for(agent: &str) -> Option<ConfigTarget> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
    match agent {
        "claude" => Some((
            std::env::var("DOZER_CLAUDE_MCP_CONFIG")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from(&home).join(".claude").join("mcp.json")),
            run_at_claude_like,
        )),
        "codebuddy" => Some((
            std::env::var("DOZER_CODEBUDDY_MCP_CONFIG")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from(&home).join(".codebuddy").join(".mcp.json")),
            run_at_claude_like,
        )),
        "codex" => Some((
            std::env::var("DOZER_CODEX_MCP_CONFIG")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from(&home).join(".codex").join("config.toml")),
            run_at_codex,
        )),
        "opencode" => Some((
            std::env::var("DOZER_OPENCODE_MCP_CONFIG")
                .map(PathBuf::from)
                .unwrap_or_else(|_| {
                    PathBuf::from(&home)
                        .join(".config")
                        .join("opencode")
                        .join("opencode.json")
                }),
            run_at_opencode,
        )),
        _ => None,
    }
}

pub fn run(agent: &str, install: bool) -> i32 {
    match config_path_for(agent) {
        Some((path, handler)) => handler(&path, install),
        None => {
            eprintln!("不支持的 agent: {agent}（支持 claude/codebuddy/codex/opencode）");
            2
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_claude_creates_mcp_json_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mcp.json");
        assert_eq!(run_at_claude_like(&path, true), 0);
        assert_eq!(run_at_claude_like(&path, true), 0); // 再装一次
        let root: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let servers = root["mcpServers"].as_object().unwrap();
        assert_eq!(servers.len(), 1, "重复安装不应产生第二条 dozer 记录");
        assert_eq!(servers["dozer"]["args"][0], "serve");
    }

    #[test]
    fn uninstall_claude_removes_entry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mcp.json");
        run_at_claude_like(&path, true);
        assert_eq!(run_at_claude_like(&path, false), 0);
        let root: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(root["mcpServers"].as_object().unwrap().is_empty());
    }

    #[test]
    fn install_codex_writes_toml_table_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        assert_eq!(run_at_codex(&path, true), 0);
        assert_eq!(run_at_codex(&path, true), 0);
        let text = std::fs::read_to_string(&path).unwrap();
        // `toml::Value` 的 `FromStr` 在 toml 0.9.12 里对非空文档会误报
        // "unexpected content"（见 run_at_codex 里的注释），这里改用
        // `toml::from_str` 走正常路径。
        let root: toml::Value = toml::from_str(&text).unwrap();
        let servers = root["mcp_servers"].as_table().unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers["dozer"]["args"][0].as_str().unwrap(), "serve");
    }

    #[test]
    fn install_opencode_writes_command_array_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("opencode.json");
        assert_eq!(run_at_opencode(&path, true), 0);
        assert_eq!(run_at_opencode(&path, true), 0);
        let root: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let servers = root["mcp"].as_object().unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers["dozer"]["type"], "local");
        assert_eq!(servers["dozer"]["command"][1], "serve");
    }
}
