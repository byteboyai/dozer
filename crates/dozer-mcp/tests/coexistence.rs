//! S6e-1:两个产品的 MCP 条目在同一配置文件里并存,互不改写。

use dozer_core::paths::{ProductDirs, ProductId};
use dozer_mcp::install::run_at_with_exe_for;
use std::path::{Path, PathBuf};

fn dozer() -> ProductId {
    ProductId::dozer()
}

fn digger() -> ProductId {
    ProductId::custom(
        "digger",
        "DIGGER_SESSION_ID",
        "digger-hook",
        "diggerd.sock",
        "digger.db",
        ProductDirs::Fixed {
            config: PathBuf::from("/tmp/digger/config"),
            state: PathBuf::from("/tmp/digger/state"),
        },
    )
}

fn install(path: &Path, product: &ProductId, agent: &str, install: bool, exe: &str) {
    assert_eq!(
        run_at_with_exe_for(product, path, agent, install, exe),
        0,
        "{agent} {install}"
    );
}

fn entry_snapshot(path: &Path, agent: &str, key: &str) -> String {
    if agent == "codex" {
        let root: toml::Value = toml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        root["mcp_servers"][key].to_string()
    } else {
        let root: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let map = if agent == "opencode" {
            &root["mcp"]
        } else {
            &root["mcpServers"]
        };
        serde_json::to_string(&map[key]).unwrap()
    }
}

fn sequence(agent: &str, suffix: &str, preset: &str) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(suffix);
    std::fs::write(&path, preset).unwrap();

    install(&path, &dozer(), agent, true, "/x/dozer-mcp");
    let dozer_entry = entry_snapshot(&path, agent, "dozer");
    install(&path, &digger(), agent, true, "/x/digger-mcp");
    assert_eq!(
        entry_snapshot(&path, agent, "dozer"),
        dozer_entry,
        "{agent}: 装 digger 后 dozer 条目被改动"
    );
    let digger_entry = entry_snapshot(&path, agent, "digger");

    install(&path, &digger(), agent, true, "/x/digger-mcp");
    assert_eq!(
        entry_snapshot(&path, agent, "digger"),
        digger_entry,
        "{agent}: 重装不幂等"
    );
    install(&path, &dozer(), agent, true, "/x/dozer-mcp");
    assert_eq!(
        entry_snapshot(&path, agent, "digger"),
        digger_entry,
        "{agent}: 重装 dozer 改动了 digger"
    );

    install(&path, &digger(), agent, false, "/x/digger-mcp");
    assert_eq!(
        entry_snapshot(&path, agent, "dozer"),
        dozer_entry,
        "{agent}: 卸载 digger 后 dozer 条目被改动"
    );

    install(&path, &dozer(), agent, false, "/x/dozer-mcp");
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        !text.contains("dozer-mcp"),
        "{agent}: dozer 条目未卸载: {text}"
    );
}

#[test]
fn claude_entries_coexist_across_every_sequence() {
    sequence(
        "claude",
        "c.json",
        r#"{"numStartups":7,"mcpServers":{"other":{"command":"foo"}}}"#,
    );
}

#[test]
fn opencode_entries_coexist_across_every_sequence() {
    sequence(
        "opencode",
        "o.json",
        r#"{"theme":"dark","mcp":{"other":{"type":"local"}}}"#,
    );
}

#[test]
fn codex_comments_blank_lines_key_order_and_unrelated_tables_survive_every_sequence() {
    sequence(
        "codex",
        "c.toml",
        "# keep me\nmodel = \"o3\"\n\n[mcp_servers.other]\ncommand = \"foo\"\n",
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("c.toml");
    std::fs::write(
        &path,
        "# keep me\nmodel = \"o3\"\n\n[mcp_servers.other]\ncommand = \"foo\"\n",
    )
    .unwrap();
    install(&path, &dozer(), "codex", true, "/x/dozer-mcp");
    install(&path, &digger(), "codex", true, "/x/digger-mcp");
    install(&path, &dozer(), "codex", false, "/x/dozer-mcp");
    install(&path, &digger(), "codex", false, "/x/digger-mcp");
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("# keep me"), "{text}");
    assert!(text.contains("model = \"o3\""), "{text}");
    assert!(text.contains("[mcp_servers.other]"), "{text}");
    assert!(text.contains("command = \"foo\""), "{text}");
}

#[test]
fn inline_mcp_servers_parent_is_preserved_for_codex() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("c.toml");
    std::fs::write(&path, "mcp_servers = { other = { command = \"foo\" } }\n").unwrap();
    install(&path, &dozer(), "codex", true, "/x/dozer-mcp");
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        text.lines().next().unwrap().starts_with("mcp_servers = {"),
        "父级行内表被改写:\n{text}"
    );
    assert!(text.contains("dozer"), "{text}");
}
