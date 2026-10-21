//! S6e-1:安静模式与缺变量报错。

use serde_json::Value;
use std::io::Write;
use std::process::{Command, Stdio};

const INIT: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#;
const INITED: &str = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
const LIST: &str = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#;

fn run(envs: &[(&str, &str)], home: &std::path::Path) -> (Vec<Value>, String, i32) {
    let exe = env!("CARGO_BIN_EXE_dozer-mcp");
    let mut cmd = Command::new(exe);
    cmd.arg("serve")
        .env_remove("DOZER_SESSION_ID")
        .env_remove("DIGGER_SESSION_ID")
        .env("HOME", home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().expect("spawn");
    {
        let stdin = child.stdin.as_mut().expect("stdin");
        for line in [INIT, INITED, LIST] {
            writeln!(stdin, "{line}").expect("write");
        }
    }
    drop(child.stdin.take());
    let out = child.wait_with_output().expect("wait");
    let responses: Vec<Value> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("json response"))
        .collect();
    (
        responses,
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

fn tool_count(responses: &[Value]) -> usize {
    responses
        .iter()
        .find(|v| v["id"] == 2)
        .and_then(|v| v["result"]["tools"].as_array())
        .map(|a| a.len())
        .unwrap_or(usize::MAX)
}

#[test]
fn quiet_mode_initializes_and_lists_no_tools() {
    let home = tempfile::tempdir().unwrap();
    let (responses, stderr, code) = run(&[("DIGGER_SESSION_ID", "abc")], home.path());
    assert_eq!(code, 0, "stderr={stderr}");
    assert!(stderr.is_empty(), "安静模式不得写 stderr: {stderr}");
    assert!(responses.iter().any(|v| v["id"] == 1 && v["result"].is_object()));
    assert_eq!(tool_count(&responses), 0);
    let socket = home.path().join("Library/Application Support/ai.byteboy.dozer/dozerd.sock");
    assert!(!socket.exists(), "安静模式不应创建/连接 socket");
}

#[test]
fn quiet_mode_is_not_entered_when_no_known_session_env_exists() {
    let home = tempfile::tempdir().unwrap();
    let (_responses, stderr, code) = run(&[], home.path());
    assert_ne!(code, 0);
    let golden = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/serve_error_golden.txt"),
    )
    .unwrap();
    assert_eq!(stderr, golden);
}

#[test]
fn quiet_mode_is_not_entered_for_an_empty_other_variable() {
    let home = tempfile::tempdir().unwrap();
    let (_responses, _stderr, code) = run(&[("DIGGER_SESSION_ID", "")], home.path());
    assert_ne!(code, 0);
}

#[test]
fn own_variable_wins_over_the_other_product_variable() {
    let home = tempfile::tempdir().unwrap();
    let (responses, stderr, code) = run(
        &[("DOZER_SESSION_ID", "x"), ("DIGGER_SESSION_ID", "y")],
        home.path(),
    );
    assert_eq!(code, 0, "stderr={stderr}");
    assert_eq!(tool_count(&responses), 12, "自己的变量在时应正常列工具");
}

#[test]
fn serve_error_message_is_parameterized_by_the_product() {
    let dozer = dozer_core::paths::ProductId::dozer();
    assert_eq!(
        dozer_mcp::server::missing_session_message(&dozer),
        "缺少 DOZER_SESSION_ID：dozer-mcp 只能在 dozer 拉起的会话里跑"
    );
    let digger = dozer_core::paths::ProductId::custom(
        "digger",
        "DIGGER_SESSION_ID",
        "digger-hook",
        "diggerd.sock",
        "digger.db",
        dozer_core::paths::ProductDirs::Fixed {
            config: std::path::PathBuf::from("/tmp/digger/config"),
            state: std::path::PathBuf::from("/tmp/digger/state"),
        },
    );
    assert_eq!(
        dozer_mcp::server::missing_session_message(&digger),
        "缺少 DIGGER_SESSION_ID：digger-mcp 只能在 digger 拉起的会话里跑"
    );
}
