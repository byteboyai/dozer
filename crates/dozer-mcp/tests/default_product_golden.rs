//! S6e-1 黄金测试:默认产品(`ProductId::dozer()`)下的对外行为逐字节不变。
//! 夹具在本文件首次运行时用 `UPDATE_GOLDEN=1` 生成一次,之后不得重生成。

use dozer_mcp::install;
use dozer_mcp::server::DozerMcpServer;
use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn compare_or_update(name: &str, actual: &str) {
    let path = fixture(name);
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&path, actual).expect("write golden");
        return;
    }
    let expected = std::fs::read_to_string(&path).expect("golden fixture missing");
    assert_eq!(actual, expected, "{name} 与黄金不一致");
}

#[test]
fn tools_list_matches_the_golden() {
    let mut tools = DozerMcpServer::tool_router().list_all();
    tools.sort_by(|a, b| a.name.cmp(&b.name));
    let text = serde_json::to_string_pretty(&tools).expect("serialize tools") + "\n";
    compare_or_update("tools_list_golden.json", &text);
}

/// 读一行/若干行 stdout,直到看到指定 id 的应答或 EOF。
fn rpc_session(envs: &[(&str, &str)], remove: &[&str], requests: &[&str]) -> (String, String, i32) {
    let exe = env!("CARGO_BIN_EXE_dozer-mcp");
    let mut cmd = Command::new(exe);
    cmd.arg("serve")
        .env_remove("DOZER_SESSION_ID")
        .env_remove("DIGGER_SESSION_ID")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for k in remove {
        cmd.env_remove(k);
    }
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().expect("spawn dozer-mcp");
    {
        let stdin = child.stdin.as_mut().expect("stdin");
        for req in requests {
            writeln!(stdin, "{req}").expect("write request");
        }
    }
    // 关闭 stdin 让服务在 EOF 后退出。
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("wait");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or(-1),
    )
}

#[test]
fn initialize_matches_the_golden() {
    let (stdout, _stderr, _code) = rpc_session(
        &[("DOZER_SESSION_ID", "x")],
        &[],
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#,
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
        ],
    );
    let mut lines = stdout.lines().filter(|l| !l.trim().is_empty());
    let init_line = lines.next().expect("initialize response");
    let init: Value = serde_json::from_str(init_line).expect("initialize json");
    // 版本号随 rmcp crate 变化,归一化后再比;其余字段逐字比较。
    let mut init = init;
    init["result"]["serverInfo"]["version"] = Value::String("<version>".into());
    let normalized = serde_json::to_string_pretty(&init).unwrap() + "\n";
    compare_or_update("initialize_golden.json", &normalized);

    let list_line = lines.next().expect("tools/list response");
    let list: Value = serde_json::from_str(list_line).expect("tools json");
    let mut tools = list["result"]["tools"].as_array().cloned().unwrap_or_default();
    tools.sort_by(|a, b| {
        a["name"]
            .as_str()
            .unwrap_or_default()
            .cmp(b["name"].as_str().unwrap_or_default())
    });
    let golden: Vec<Value> =
        serde_json::from_str(&std::fs::read_to_string(fixture("tools_list_golden.json")).unwrap())
            .unwrap();
    assert_eq!(tools, golden, "tools/list 与工具黄金不一致");
}

fn run_install(path: &Path, agent: &str, install: bool) {
    let code = install::run_at_with_exe(path, agent, install, "/opt/dozer/dozer-mcp");
    assert_eq!(code, 0, "{agent} install={install}");
}

fn install_sequence(name: &str, preset: &str, agent: &str) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config");
    std::fs::write(&path, preset).unwrap();
    let mut text = String::from("== after install ==\n");
    run_install(&path, agent, true);
    text.push_str(&std::fs::read_to_string(&path).unwrap());
    text.push_str("== after reinstall ==\n");
    run_install(&path, agent, true);
    text.push_str(&std::fs::read_to_string(&path).unwrap());
    text.push_str("== after uninstall ==\n");
    run_install(&path, agent, false);
    text.push_str(&std::fs::read_to_string(&path).unwrap());
    compare_or_update(name, &text);
}

#[test]
fn install_outputs_match_the_golden() {
    install_sequence(
        "install_claude_golden.json",
        "{\n  \"numStartups\": 7,\n  \"mcpServers\": {\n    \"other\": {\n      \"command\": \"foo\"\n    }\n  }\n}\n",
        "claude",
    );
    install_sequence(
        "install_codex_golden.toml",
        "# keep me\nmodel = \"o3\"\n\n[mcp_servers.other]\ncommand = \"foo\"\n",
        "codex",
    );
    install_sequence(
        "install_opencode_golden.json",
        "{\n  \"theme\": \"dark\",\n  \"mcp\": {\n    \"other\": {\n      \"type\": \"local\"\n    }\n  }\n}\n",
        "opencode",
    );
}

#[test]
fn serve_without_any_session_env_prints_the_legacy_error() {
    let (_stdout, stderr, code) = rpc_session(&[], &["DOZER_SESSION_ID", "DIGGER_SESSION_ID"], &[]);
    assert_ne!(code, 0);
    compare_or_update("serve_error_golden.txt", &stderr);
}
