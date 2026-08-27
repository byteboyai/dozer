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
fn run_at_claude_like(path: &Path, install: bool, exe: &str) -> i32 {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => "{}".into(),
        Err(e) => {
            eprintln!("读 {} 失败: {e}", path.display());
            return 1;
        }
    };
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
                "command": exe,
                "args": ["serve"],
                "env": {}
            }),
        );
    } else {
        servers.remove("dozer");
    }
    write_json(path, &root)
}

/// `[mcp_servers.dozer]` 的内容（`command` + `args`）。父表是行内表时得
/// 造行内表值，否则造独立表头，不然编码出来的 TOML 不合法。
fn codex_dozer_entry(inline: bool, exe: &str) -> toml_edit::Item {
    let mut args = toml_edit::Array::new();
    args.push("serve");
    if inline {
        let mut t = toml_edit::InlineTable::new();
        t.insert("command", exe.into());
        t.insert("args", toml_edit::Value::Array(args));
        toml_edit::Item::Value(toml_edit::Value::InlineTable(t))
    } else {
        let mut t = toml_edit::Table::new();
        t.insert("command", toml_edit::value(exe.to_string()));
        t.insert("args", toml_edit::value(args));
        toml_edit::Item::Table(t)
    }
}

/// Codex：`[mcp_servers.dozer]` TOML 表。
///
/// 用 `toml_edit`（格式保留式编辑）而不是 `toml`：`~/.codex/config.toml` 是
/// 用户手写的文件，走 `toml::Value` 往返序列化会把注释、空行、键序全部抹
/// 掉——对一个不归我们所有的文件来说那是实打实的数据损坏。`toml_edit` 只
/// 改我们碰过的那几个键，其余原样保留。
fn run_at_codex(path: &Path, install: bool, exe: &str) -> i32 {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => {
            eprintln!("读 {} 失败: {e}", path.display());
            return 1;
        }
    };
    let mut doc: toml_edit::DocumentMut = match text.parse() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("解析 {} 失败，拒绝写入: {e}", path.display());
            return 1;
        }
    };
    let root = doc.as_table_mut();
    if !root.contains_key("mcp_servers") {
        // 新建时置 implicit，输出成 `[mcp_servers.dozer]` 一个表头而不是
        // 多一行空的 `[mcp_servers]`。已存在的表不动它的 implicit 标志，
        // 免得把用户自己写的 `[mcp_servers]` 表头抹掉。
        let mut t = toml_edit::Table::new();
        t.set_implicit(true);
        root.insert("mcp_servers", toml_edit::Item::Table(t));
    }
    let servers_item = root
        .get_mut("mcp_servers")
        .expect("mcp_servers 刚确认存在/已建");
    let inline = servers_item.is_inline_table();
    let Some(servers) = servers_item.as_table_like_mut() else {
        eprintln!("mcp_servers 段不是 table，拒绝写入");
        return 1;
    };
    if install {
        servers.insert("dozer", codex_dozer_entry(inline, exe));
    } else {
        servers.remove("dozer");
    }
    write_text(path, &doc.to_string())
}

/// OpenCode：`{"mcp": {"dozer": {"type":"local","command":[...],"enabled":true}}}`。
fn run_at_opencode(path: &Path, install: bool, exe: &str) -> i32 {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => "{}".into(),
        Err(e) => {
            eprintln!("读 {} 失败: {e}", path.display());
            return 1;
        }
    };
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
                "command": [exe.to_string(), "serve"],
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
type ConfigTarget = (PathBuf, fn(&Path, bool, &str) -> i32);

pub fn config_path_for(agent: &str) -> Option<ConfigTarget> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
    match agent {
        "claude" => Some((
            std::env::var("DOZER_CLAUDE_MCP_CONFIG")
                .map(PathBuf::from)
                // Claude Code 的用户级 MCP 注册表是 `~/.claude.json` 里的
                // `mcpServers` 键——那是个混装了别的 app 状态的单文件,不是
                // 专用 MCP 配置文件;`~/.claude/mcp.json` 压根不存在也不被读。
                .unwrap_or_else(|_| PathBuf::from(&home).join(".claude.json")),
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
    run_at_with_exe_impl(agent, install, &exe_path())
}

/// GUI 场景(`dozer-app` 在 agent 启动时静默自动注册)不能走 `run()`——它
/// 内部调 `exe_path()`,在 `dozer-app` 进程内直接函数调用时会拿到
/// `dozer-app` 自己的可执行文件路径,写出一条指向错误二进制的 MCP server
/// 注册项。必须显式传入 `dozer-mcp` 的 sibling 二进制路径。
pub fn run_at_with_exe(path: &Path, agent: &str, install: bool, exe: &str) -> i32 {
    match config_path_for(agent) {
        Some((_, handler)) => handler(path, install, exe),
        None => {
            eprintln!("不支持的 agent: {agent}（支持 claude/codebuddy/codex/opencode）");
            2
        }
    }
}

fn run_at_with_exe_impl(agent: &str, install: bool, exe: &str) -> i32 {
    match config_path_for(agent) {
        Some((path, handler)) => handler(&path, install, exe),
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
        assert_eq!(run_at_claude_like(&path, true, "dozer-mcp"), 0);
        assert_eq!(run_at_claude_like(&path, true, "dozer-mcp"), 0); // 再装一次
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
        run_at_claude_like(&path, true, "dozer-mcp");
        assert_eq!(run_at_claude_like(&path, false, "dozer-mcp"), 0);
        let root: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(root["mcpServers"].as_object().unwrap().is_empty());
    }

    #[test]
    fn install_codex_writes_toml_table_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        assert_eq!(run_at_codex(&path, true, "dozer-mcp"), 0);
        assert_eq!(run_at_codex(&path, true, "dozer-mcp"), 0);
        let text = std::fs::read_to_string(&path).unwrap();
        // `.parse()`（`Value: FromStr`）解析的是裸值字面量而非完整文档，
        // 见 run_at_codex 里的注释；这里同样改用 `toml::from_str`。
        let root: toml::Value = toml::from_str(&text).unwrap();
        let servers = root["mcp_servers"].as_table().unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers["dozer"]["args"][0].as_str().unwrap(), "serve");
    }

    #[test]
    fn install_opencode_writes_command_array_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("opencode.json");
        assert_eq!(run_at_opencode(&path, true, "dozer-mcp"), 0);
        assert_eq!(run_at_opencode(&path, true, "dozer-mcp"), 0);
        let root: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let servers = root["mcp"].as_object().unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers["dozer"]["type"], "local");
        assert_eq!(servers["dozer"]["command"][1], "serve");
    }

    /// 读一个存在但不是文件（这里用目录代替）的路径，在所有平台上都会产生
    /// 非 `NotFound` 的 I/O 错误，用来验证"读失败（非文件不存在）应拒绝
    /// 写入"而不是把它当空配置误覆盖。
    #[test]
    fn install_claude_refuses_to_write_when_read_fails_non_notfound() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mcp.json");
        std::fs::create_dir(&path).unwrap();
        assert_eq!(run_at_claude_like(&path, true, "dozer-mcp"), 1);
        assert!(path.is_dir(), "读失败时不应把目录路径覆盖成文件");
    }

    #[test]
    fn install_codex_refuses_to_write_when_read_fails_non_notfound() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::create_dir(&path).unwrap();
        assert_eq!(run_at_codex(&path, true, "dozer-mcp"), 1);
        assert!(path.is_dir(), "读失败时不应把目录路径覆盖成文件");
    }

    #[test]
    fn install_opencode_refuses_to_write_when_read_fails_non_notfound() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("opencode.json");
        std::fs::create_dir(&path).unwrap();
        assert_eq!(run_at_opencode(&path, true, "dozer-mcp"), 1);
        assert!(path.is_dir(), "读失败时不应把目录路径覆盖成文件");
    }

    /// Claude 的用户级注册表是 `~/.claude.json`——一个混装别的 app 状态的
    /// 单文件，不是专用 MCP 配置文件。走 env 覆盖读一遍默认路径，确保没人
    /// 又把它改回不存在的 `~/.claude/mcp.json`。
    #[test]
    fn claude_default_config_path_is_dot_claude_json() {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
        let (path, _) = config_path_for("claude").unwrap();
        // 有 env 覆盖时跳过（CI/本机可能设了 DOZER_CLAUDE_MCP_CONFIG）。
        if std::env::var("DOZER_CLAUDE_MCP_CONFIG").is_err() {
            assert_eq!(path, PathBuf::from(&home).join(".claude.json"));
        }
    }

    /// `~/.codex/config.toml` 是用户手写文件，安装器绝不能吃掉注释——这是
    /// 换用 `toml_edit` 要防的那一类退化。
    #[test]
    fn install_codex_preserves_comments_and_unrelated_tables() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let original =
            "# 这行注释必须活下来\nmodel = \"o3\"\n\n[mcp_servers.other]\ncommand = \"foo\"\n";
        std::fs::write(&path, original).unwrap();

        assert_eq!(run_at_codex(&path, true, "dozer-mcp"), 0);

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("# 这行注释必须活下来"),
            "注释被 TOML 往返序列化吃掉了:\n{text}"
        );
        let root: toml::Value = toml::from_str(&text).unwrap();
        assert_eq!(root["model"].as_str().unwrap(), "o3");
        let servers = root["mcp_servers"].as_table().unwrap();
        assert_eq!(servers["other"]["command"].as_str().unwrap(), "foo");
        assert_eq!(servers["dozer"]["args"][0].as_str().unwrap(), "serve");
    }

    /// 安装器只碰自己的 `dozer` 条目：别家 MCP server 的注册必须原样还在。
    /// Claude/Codebuddy 共用 `run_at_claude_like`，一条覆盖两家。
    #[test]
    fn run_at_with_exe_uses_explicit_exe_not_current_exe() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mcp.json");
        assert_eq!(
            run_at_with_exe(&path, "claude", true, "/opt/dozer/dozer-mcp"),
            0
        );
        let root: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            root["mcpServers"]["dozer"]["command"],
            "/opt/dozer/dozer-mcp"
        );
    }

    #[test]
    fn run_at_with_exe_unsupported_agent_returns_error_code() {
        let dir = tempfile::tempdir().unwrap();
        // 未支持的 agent 名不该落到任何默认路径去改文件,直接报错码。
        assert_eq!(
            run_at_with_exe(&dir.path().join("x"), "kilo", true, "/x/dozer-mcp"),
            2
        );
    }

    #[test]
    fn install_claude_like_keeps_unrelated_entries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".claude.json");
        std::fs::write(
            &path,
            r#"{"numStartups": 7, "mcpServers": {"other-server": {"command": "foo"}}}"#,
        )
        .unwrap();

        assert_eq!(run_at_claude_like(&path, true, "dozer-mcp"), 0);

        let root: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        // 同文件里别的 app 状态也不能丢。
        assert_eq!(root["numStartups"], 7);
        let servers = root["mcpServers"].as_object().unwrap();
        assert_eq!(servers.len(), 2);
        assert_eq!(servers["other-server"]["command"], "foo");
        assert_eq!(servers["dozer"]["args"][0], "serve");
    }

    #[test]
    fn install_codex_keeps_unrelated_entries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[mcp_servers.other]\ncommand = \"foo\"\n").unwrap();

        assert_eq!(run_at_codex(&path, true, "dozer-mcp"), 0);

        let root: toml::Value = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let servers = root["mcp_servers"].as_table().unwrap();
        assert_eq!(servers.len(), 2);
        assert_eq!(servers["other"]["command"].as_str().unwrap(), "foo");
        assert_eq!(servers["dozer"]["args"][0].as_str().unwrap(), "serve");
    }

    #[test]
    fn install_opencode_keeps_unrelated_entries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("opencode.json");
        std::fs::write(
            &path,
            r#"{"theme": "dark", "mcp": {"other-server": {"type": "local"}}}"#,
        )
        .unwrap();

        assert_eq!(run_at_opencode(&path, true, "dozer-mcp"), 0);

        let root: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(root["theme"], "dark");
        let servers = root["mcp"].as_object().unwrap();
        assert_eq!(servers.len(), 2);
        assert_eq!(servers["other-server"]["type"], "local");
        assert_eq!(servers["dozer"]["command"][1], "serve");
    }
}
