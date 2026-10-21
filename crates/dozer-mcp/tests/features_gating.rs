use dozer_client::Client;
use dozer_mcp::features::McpFeatures;
use dozer_mcp::server::DozerMcpServer;
use rmcp::handler::server::ServerHandler;
use serde_json::Value;
use std::path::PathBuf;

fn golden_names() -> Vec<String> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/tools_list_golden.json");
    let tools: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let mut names: Vec<String> = tools
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    names.sort();
    names
}

#[test]
fn all_features_match_the_tools_list_golden() {
    let mut names = McpFeatures::all_tool_names();
    names.sort();
    assert_eq!(names, golden_names());
}

#[test]
fn tool_names_cover_exactly_the_twelve_known_tools_once() {
    let mut names = McpFeatures::all_tool_names();
    names.sort();
    let mut dedup = names.clone();
    dedup.dedup();
    assert_eq!(names, dedup, "不应有重复工具名");
    assert_eq!(names.len(), 12);
    assert_eq!(names, golden_names());
}

#[test]
fn each_feature_group_removes_exactly_its_tools() {
    let all = golden_names();
    let groups: [(&str, &[&str]); 5] = [
        ("memory", &["write_memory", "list_memories", "get_memory"]),
        ("summary", &["submit_session_summary"]),
        ("todos", &["list_todos", "add_todo", "toggle_todo", "edit_todo_text"]),
        ("preview", &["get_preview_context", "preview_navigate", "locate_in_file"]),
        ("file_edit", &["apply_precise_edit"]),
    ];
    for (group, removed) in groups {
        let mut f = McpFeatures::all();
        match group {
            "memory" => f.memory = false,
            "summary" => f.summary = false,
            "todos" => f.todos = false,
            "preview" => f.preview = false,
            "file_edit" => f.file_edit = false,
            _ => unreachable!(),
        }
        let mut names = f.tool_names();
        names.sort();
        let mut expected: Vec<String> = all
            .iter()
            .filter(|n| !removed.contains(&n.as_str()))
            .cloned()
            .collect();
        expected.sort();
        assert_eq!(names, expected, "{group}");
        for name in removed {
            assert!(!DozerMcpServer::has_tool(f, name), "{group} 仍注册 {name}");
        }
    }
}

#[test]
fn features_none_lists_no_tools() {
    assert!(McpFeatures::none().tool_names().is_empty());
    for name in McpFeatures::all_tool_names() {
        assert!(!DozerMcpServer::has_tool(McpFeatures::none(), name));
    }
}

#[test]
fn calling_a_disabled_tool_by_name_is_unknown_tool_not_execution() {
    let f = McpFeatures {
        memory: false,
        ..McpFeatures::all()
    };
    assert!(!DozerMcpServer::has_tool(f, "write_memory"));
    assert!(DozerMcpServer::has_tool(f, "list_todos"));
}

#[test]
fn get_info_uses_the_given_server_name() {
    let client = Client::new(PathBuf::from("/nonexistent/dozerd.sock"));
    let default = DozerMcpServer::new(Client::new(PathBuf::from("/nonexistent/dozerd.sock")), "x".into());
    assert_eq!(default.get_info().server_info.name, "rmcp");

    let custom = DozerMcpServer::with_options(
        client,
        None,
        McpFeatures::none(),
        "digger-mcp".into(),
    );
    let info = custom.get_info();
    assert_eq!(info.server_info.name, "digger-mcp");
    assert!(info.capabilities.tools.is_some());
}
