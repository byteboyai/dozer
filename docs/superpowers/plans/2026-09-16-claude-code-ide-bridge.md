# Claude Code IDE 桥接 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 Dozer 里跑的 Claude Code 自动获得"当前预览面板打开的文件 + 选区"上下文(聊天记录里的 `In <file>` 提示条),复刻 VS Code/JetBrains 那套 IDE 自动识别协议。

**Architecture:** dozerd 新增 `ide_bridge` 模块。按 `project_id` 索引的 `IdeBridgeRegistry`,在项目的活跃会话数 0→1 时起一个绑定 `127.0.0.1:<随机端口>` 的 WebSocket JSON-RPC server + 写一份 `~/.claude/ide/<port>.lock`,1→0 时停止并删锁文件。协议侧只读复用已有的 `preview_context::PreviewContextStore`,把 `PreviewContext` 映射成 Claude Code IDE 协议要求的 `getCurrentSelection`/`getOpenEditors` 响应,`getDiagnostics` 恒返回空数组占位。

**Tech Stack:** Rust,tokio(已是 workspace 依赖),新增 `tokio-tungstenite`(WebSocket)、`futures-util`(stream/sink 组合子)、`libc`(pid 存活检测)。

**Spec:** `docs/superpowers/specs/2026-09-16-claude-code-ide-bridge-design.md`

## Global Constraints

- **在独立分支上开发**(如 `feature/claude-code-ide-bridge`),不要直接提交到 `main`;全部 6 个 Task 完成、自测通过后提请审阅,审阅通过再合并回 `main`。避免和其他并行在 `main` 上进行的计划互相干扰(过往曾因两个 agent 同时改 main 导致审阅困惑)。
- 只做"自动上下文"(`getCurrentSelection`/`getOpenEditors`),不做 `getDiagnostics` 真实实现(恒空)、不做 `openDiff`/`executeCode` 等编辑类能力。
- 默认自动开启,不加设置面板开关。
- 一个项目一份锁文件 + 一个 WebSocket server(不做"一份锁文件覆盖多个项目"的全局共享模式)。
- 锁文件目录权限 `0700`、文件权限 `0600`,token 用系统 CSPRNG(`uuid::Uuid::new_v4()`)生成。
- 鉴权 header 固定为 `x-claude-code-ide-authorization`,值必须等于锁文件里的 token。
- 不做跨 dozerd 重启的持久化——重启即清空,新会话触发时重建;dozerd 启动时清扫上次崩溃遗留的陈旧锁文件。
- 不按 `AgentKind` 过滤——对项目下所有会话一视同仁地起 bridge。
- `getCurrentSelection`/`getOpenEditors` 的精确 JSON 字段名没有官方文档,本计划给出的是基于 MCP/IDE 集成惯例的最佳草案;Task 4 的手工验证步骤要求对着真实 Claude Code CLI 实测,字段形状如与实测不符,以实测为准调整。

---

## Task 1: 锁文件读写 + 路径解析 + token 生成

**Files:**
- Create: `crates/dozerd/src/ide_bridge.rs`
- Modify: `crates/dozerd/src/lib.rs`(新增 `pub mod ide_bridge;`)
- Modify: `Cargo.toml`(workspace 根,新增 `[workspace.dependencies]` 条目)
- Modify: `crates/dozerd/Cargo.toml`(新增依赖引用)
- Test: 内联于 `crates/dozerd/src/ide_bridge.rs` 的 `#[cfg(test)]` 模块

**Interfaces:**
- Consumes:(无,本任务是基础层)
- Produces:
  - `pub(crate) fn resolve_lock_dir(override_dir: Option<String>, claude_config_dir: Option<String>, home: Option<String>) -> PathBuf`
  - `pub(crate) fn lock_dir() -> PathBuf`(薄包装,读三个环境变量后调 `resolve_lock_dir`)
  - `pub(crate) fn generate_token() -> String`
  - `pub(crate) struct LockFileContents { pid: u32, port: u16, workspace_folders: Vec<String>, ide_name: String, token: String }`(`#[serde(rename_all = "camelCase")]`)
  - `pub(crate) fn write_lock_file(dir: &Path, port: u16, workspace_root: &str, token: &str, pid: u32) -> std::io::Result<PathBuf>`
  - `pub(crate) fn remove_lock_file(path: &Path)`

- [ ] **Step 1: 加依赖**

编辑根 `Cargo.toml`,在 `[workspace.dependencies]` 里(`alacritty_terminal = "0.26"` 那一行之后)加三行:

```toml
tokio-tungstenite = "0.24"
futures-util = "0.3"
libc = "0.2"
```

编辑 `crates/dozerd/Cargo.toml`,在 `[dependencies]` 末尾(`rusqlite` 那行之后)加:

```toml
tokio-tungstenite.workspace = true
futures-util.workspace = true
libc.workspace = true
```

- [ ] **Step 2: 跑一次 `cargo build -p dozerd` 确认依赖能解析**

Run: `cargo build -p dozerd`
Expected: 编译通过(此时还没有新代码引用这些 crate,只是验证依赖能下载/解析;如果版本号解析失败,把对应版本号改成 `cargo add <crate> -p dozerd --dry-run` 报出的最新兼容版本再重试)。

- [ ] **Step 3: 写路径解析的失败测试**

创建 `crates/dozerd/src/ide_bridge.rs`,先写测试(此时 `resolve_lock_dir`/`lock_dir` 还不存在):

```rust
//! Claude Code IDE 集成桥接:让 Dozer 模拟 VS Code/JetBrains 的 IDE 握手协议,
//! 把 `preview_context::PreviewContextStore` 里的数据喂给 Claude Code CLI 的
//! 自动上下文机制(聊天记录里的 `In <file>` 提示条)。设计依据见
//! `docs/superpowers/specs/2026-09-16-claude-code-ide-bridge-design.md`。

use std::path::{Path, PathBuf};

pub(crate) fn resolve_lock_dir(
    override_dir: Option<String>,
    claude_config_dir: Option<String>,
    home: Option<String>,
) -> PathBuf {
    if let Some(dir) = override_dir {
        return PathBuf::from(dir);
    }
    if let Some(dir) = claude_config_dir {
        return PathBuf::from(dir).join("ide");
    }
    let home = home.unwrap_or_else(|| "/".to_string());
    PathBuf::from(home).join(".claude").join("ide")
}

pub(crate) fn lock_dir() -> PathBuf {
    resolve_lock_dir(
        std::env::var("DOZER_CLAUDE_IDE_LOCK_DIR").ok(),
        std::env::var("CLAUDE_CONFIG_DIR").ok(),
        std::env::var("HOME").ok(),
    )
}

#[cfg(test)]
mod path_tests {
    use super::*;

    #[test]
    fn override_dir_wins_over_everything() {
        let got = resolve_lock_dir(
            Some("/override".to_string()),
            Some("/claude-config".to_string()),
            Some("/home/u".to_string()),
        );
        assert_eq!(got, PathBuf::from("/override"));
    }

    #[test]
    fn claude_config_dir_wins_over_home() {
        let got = resolve_lock_dir(None, Some("/claude-config".to_string()), Some("/home/u".to_string()));
        assert_eq!(got, PathBuf::from("/claude-config/ide"));
    }

    #[test]
    fn falls_back_to_home_dot_claude_ide() {
        let got = resolve_lock_dir(None, None, Some("/home/u".to_string()));
        assert_eq!(got, PathBuf::from("/home/u/.claude/ide"));
    }
}
```

- [ ] **Step 2: 运行测试确认能过(纯函数,写完就该直接通过)**

Run: `cargo test -p dozerd ide_bridge::path_tests`
Expected: 3 个测试全部 PASS。

- [ ] **Step 3: 写 token 生成的测试 + 实现**

在 `ide_bridge.rs` 里,`lock_dir` 函数之后追加:

```rust
pub(crate) fn generate_token() -> String {
    // 128 位随机值,32 个十六进制字符,无连字符——复用仓库已有的 uuid 依赖,
    // 不为这一个用途单独引入 rand crate。
    uuid::Uuid::new_v4().simple().to_string()
}

#[cfg(test)]
mod token_tests {
    use super::*;

    #[test]
    fn token_is_32_lowercase_hex_chars() {
        let token = generate_token();
        assert_eq!(token.len(), 32);
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn tokens_are_not_repeated_across_calls() {
        assert_ne!(generate_token(), generate_token());
    }
}
```

- [ ] **Step 4: 运行测试确认能过**

Run: `cargo test -p dozerd ide_bridge::token_tests`
Expected: 2 个测试 PASS。

- [ ] **Step 5: 写锁文件读写的测试 + 实现**

在 `ide_bridge.rs` 里追加:

```rust
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LockFileContents {
    pub(crate) pid: u32,
    pub(crate) port: u16,
    pub(crate) workspace_folders: Vec<String>,
    pub(crate) ide_name: String,
    pub(crate) token: String,
}

pub(crate) fn write_lock_file(
    dir: &Path,
    port: u16,
    workspace_root: &str,
    token: &str,
    pid: u32,
) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let contents = LockFileContents {
        pid,
        port,
        workspace_folders: vec![workspace_root.to_string()],
        ide_name: "Dozer".to_string(),
        token: token.to_string(),
    };
    let path = dir.join(format!("{port}.lock"));
    let json = serde_json::to_string_pretty(&contents).expect("LockFileContents 序列化不应失败");
    std::fs::write(&path, json)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(path)
}

pub(crate) fn remove_lock_file(path: &Path) {
    // 幂等:文件不存在也不算错误(项目可能已经被清扫过)。
    let _ = std::fs::remove_file(path);
}

#[cfg(test)]
mod lock_file_tests {
    use super::*;

    #[test]
    fn write_then_read_round_trips_all_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_lock_file(dir.path(), 54321, "/repo/root", "abc123token", 999).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        let parsed: LockFileContents = serde_json::from_str(&raw).unwrap();
        assert_eq!(
            parsed,
            LockFileContents {
                pid: 999,
                port: 54321,
                workspace_folders: vec!["/repo/root".to_string()],
                ide_name: "Dozer".to_string(),
                token: "abc123token".to_string(),
            }
        );
    }

    #[test]
    fn lock_file_path_is_named_after_port() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_lock_file(dir.path(), 12345, "/repo", "t", 1).unwrap();
        assert_eq!(path.file_name().unwrap(), "12345.lock");
    }

    #[cfg(unix)]
    #[test]
    fn file_and_dir_permissions_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = write_lock_file(dir.path(), 1, "/repo", "t", 1).unwrap();
        let file_mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(file_mode, 0o600);
        let dir_mode = std::fs::metadata(dir.path()).unwrap().permissions().mode() & 0o777;
        assert_eq!(dir_mode, 0o700);
    }

    #[test]
    fn remove_lock_file_is_noop_when_missing() {
        // 不应 panic 或返回需要处理的错误
        remove_lock_file(Path::new("/tmp/dozer-ide-bridge-test-does-not-exist.lock"));
    }
}
```

- [ ] **Step 6: 加 `tempfile` dev-dependency(如尚未加)**

检查 `crates/dozerd/Cargo.toml` 的 `[dev-dependencies]` 段——已有 `tempfile = "3"`(现有测试已在用),无需改动。

- [ ] **Step 7: 运行全部测试确认通过**

Run: `cargo test -p dozerd ide_bridge::`
Expected: `path_tests`(3)+ `token_tests`(2)+ `lock_file_tests`(4)= 9 个测试全 PASS。

- [ ] **Step 8: 注册模块**

编辑 `crates/dozerd/src/lib.rs`,在 `pub mod headless_agent;` 之后、`pub mod preview_context;` 之前插入:

```rust
pub mod ide_bridge;
```

- [ ] **Step 9: 编译 + 提交**

Run: `cargo build -p dozerd && cargo test -p dozerd ide_bridge::`
Expected: 编译通过,测试全 PASS。

```bash
git add Cargo.toml crates/dozerd/Cargo.toml crates/dozerd/src/lib.rs crates/dozerd/src/ide_bridge.rs
git commit -m "feat(dozerd): ide_bridge 锁文件读写 + 路径解析 + token 生成"
```

---

## Task 2: PreviewContext → IDE 协议 JSON 映射

**Files:**
- Modify: `crates/dozerd/src/ide_bridge.rs`

**Interfaces:**
- Consumes: `dozer_core::protocol::PreviewContext { path, start_line, start_col, end_line, end_col, has_selection, updated_at_ms }`(1-indexed 行列,已在 Task 1 之外的既有代码中定义)
- Produces:
  - `pub(crate) fn selection_result(ctx: Option<&PreviewContext>) -> serde_json::Value`
  - `pub(crate) fn open_editors_result(ctx: Option<&PreviewContext>) -> serde_json::Value`
  - `pub(crate) fn diagnostics_result() -> serde_json::Value`

- [ ] **Step 1: 写 `selection_result` 的失败测试**

在 `ide_bridge.rs` 顶部加 `use dozer_core::protocol::PreviewContext;`,然后追加:

```rust
#[cfg(test)]
mod mapping_tests {
    use super::*;

    fn ctx() -> PreviewContext {
        PreviewContext {
            path: "/repo/src/main.rs".to_string(),
            start_line: 12,
            start_col: 3,
            end_line: 12,
            end_col: 3,
            has_selection: false,
            updated_at_ms: 1_700_000_000_000,
        }
    }

    #[test]
    fn selection_result_with_context_maps_1_indexed_to_0_indexed() {
        let got = selection_result(Some(&ctx()));
        assert_eq!(
            got,
            serde_json::json!({
                "success": true,
                "filePath": "/repo/src/main.rs",
                "selection": {
                    "start": {"line": 11, "character": 2},
                    "end": {"line": 11, "character": 2},
                    "isEmpty": true,
                },
                "text": "",
            })
        );
    }

    #[test]
    fn selection_result_has_selection_true_sets_is_empty_false() {
        let mut c = ctx();
        c.has_selection = true;
        c.end_line = 14;
        c.end_col = 1;
        let got = selection_result(Some(&c));
        assert_eq!(got["selection"]["isEmpty"], false);
        assert_eq!(got["selection"]["end"]["line"], 13);
        assert_eq!(got["selection"]["end"]["character"], 0);
    }

    #[test]
    fn selection_result_without_context_reports_failure() {
        let got = selection_result(None);
        assert_eq!(
            got,
            serde_json::json!({
                "success": false,
                "filePath": null,
                "selection": null,
                "text": "",
            })
        );
    }
}
```

- [ ] **Step 2: 运行测试确认失败(函数还不存在)**

Run: `cargo test -p dozerd ide_bridge::mapping_tests`
Expected: 编译失败,报 `cannot find function selection_result`。

- [ ] **Step 3: 实现 `selection_result`**

```rust
pub(crate) fn selection_result(ctx: Option<&PreviewContext>) -> serde_json::Value {
    match ctx {
        Some(c) => serde_json::json!({
            "success": true,
            "filePath": c.path,
            "selection": {
                "start": {
                    "line": c.start_line.saturating_sub(1),
                    "character": c.start_col.saturating_sub(1),
                },
                "end": {
                    "line": c.end_line.saturating_sub(1),
                    "character": c.end_col.saturating_sub(1),
                },
                "isEmpty": !c.has_selection,
            },
            "text": "",
        }),
        None => serde_json::json!({
            "success": false,
            "filePath": null,
            "selection": null,
            "text": "",
        }),
    }
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozerd ide_bridge::mapping_tests`
Expected: 3 个测试 PASS。

- [ ] **Step 5: 写 `open_editors_result`/`diagnostics_result` 的测试**

追加到 `mapping_tests` 模块:

```rust
    #[test]
    fn open_editors_result_with_context_lists_one_tab() {
        let got = open_editors_result(Some(&ctx()));
        assert_eq!(
            got,
            serde_json::json!({
                "tabs": [{"filePath": "/repo/src/main.rs", "isActive": true}]
            })
        );
    }

    #[test]
    fn open_editors_result_without_context_is_empty() {
        let got = open_editors_result(None);
        assert_eq!(got, serde_json::json!({"tabs": []}));
    }

    #[test]
    fn diagnostics_result_is_always_empty() {
        assert_eq!(diagnostics_result(), serde_json::json!({"diagnostics": []}));
    }
```

- [ ] **Step 6: 运行测试确认失败**

Run: `cargo test -p dozerd ide_bridge::mapping_tests`
Expected: 编译失败,报 `open_editors_result`/`diagnostics_result` 未定义。

- [ ] **Step 7: 实现两个函数**

```rust
pub(crate) fn open_editors_result(ctx: Option<&PreviewContext>) -> serde_json::Value {
    match ctx {
        Some(c) => serde_json::json!({
            "tabs": [{"filePath": c.path, "isActive": true}]
        }),
        None => serde_json::json!({"tabs": []}),
    }
}

pub(crate) fn diagnostics_result() -> serde_json::Value {
    serde_json::json!({"diagnostics": []})
}
```

- [ ] **Step 8: 运行全部映射测试确认通过**

Run: `cargo test -p dozerd ide_bridge::mapping_tests`
Expected: 6 个测试全 PASS。

- [ ] **Step 9: 提交**

```bash
git add crates/dozerd/src/ide_bridge.rs
git commit -m "feat(dozerd): ide_bridge PreviewContext -> IDE 协议 JSON 映射"
```

---

## Task 3: JSON-RPC 请求分发(纯函数,不依赖网络)

**Files:**
- Modify: `crates/dozerd/src/ide_bridge.rs`

**Interfaces:**
- Consumes: `selection_result`/`open_editors_result`/`diagnostics_result`(Task 2)
- Produces: `pub(crate) fn handle_rpc_request(request: &serde_json::Value, ctx: Option<&PreviewContext>) -> serde_json::Value`(Task 4 的连接处理循环直接调用这个函数,输入是从 WebSocket 文本帧解析出的 JSON,输出是要写回连接的 JSON)

- [ ] **Step 1: 写分发逻辑的失败测试**

追加到 `ide_bridge.rs`:

```rust
#[cfg(test)]
mod rpc_tests {
    use super::*;

    fn ctx() -> PreviewContext {
        PreviewContext {
            path: "/repo/a.rs".to_string(),
            start_line: 1,
            start_col: 1,
            end_line: 1,
            end_col: 1,
            has_selection: false,
            updated_at_ms: 0,
        }
    }

    #[test]
    fn initialize_returns_protocol_version_and_id() {
        let req = serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize"});
        let got = handle_rpc_request(&req, None);
        assert_eq!(got["id"], 1);
        assert_eq!(got["result"]["protocolVersion"], "2025-03-26");
    }

    #[test]
    fn tools_list_declares_three_tools() {
        let req = serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"});
        let got = handle_rpc_request(&req, None);
        let names: Vec<&str> = got["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, vec!["getCurrentSelection", "getOpenEditors", "getDiagnostics"]);
    }

    #[test]
    fn tools_call_get_current_selection_wraps_mapping_result_as_text_content() {
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {"name": "getCurrentSelection", "arguments": {}},
        });
        let got = handle_rpc_request(&req, Some(&ctx()));
        let text = got["result"]["content"][0]["text"].as_str().unwrap();
        let inner: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(inner, selection_result(Some(&ctx())));
    }

    #[test]
    fn tools_call_unknown_tool_returns_json_rpc_error() {
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {"name": "openDiff", "arguments": {}},
        });
        let got = handle_rpc_request(&req, None);
        assert_eq!(got["error"]["code"], -32601);
    }

    #[test]
    fn unknown_method_returns_json_rpc_error() {
        let req = serde_json::json!({"jsonrpc": "2.0", "id": 5, "method": "closeAllDiffTabs"});
        let got = handle_rpc_request(&req, None);
        assert_eq!(got["error"]["code"], -32601);
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozerd ide_bridge::rpc_tests`
Expected: 编译失败,报 `handle_rpc_request` 未定义。

- [ ] **Step 3: 实现分发逻辑**

```rust
fn tool_call_result(name: &str, ctx: Option<&PreviewContext>) -> Result<serde_json::Value, String> {
    let payload = match name {
        "getCurrentSelection" => selection_result(ctx),
        "getOpenEditors" => open_editors_result(ctx),
        "getDiagnostics" => diagnostics_result(),
        other => return Err(format!("Method not found: {other}")),
    };
    Ok(serde_json::json!({
        "content": [{"type": "text", "text": payload.to_string()}]
    }))
}

pub(crate) fn handle_rpc_request(
    request: &serde_json::Value,
    ctx: Option<&PreviewContext>,
) -> serde_json::Value {
    let id = request.get("id").cloned().unwrap_or(serde_json::Value::Null);
    let method = request.get("method").and_then(|m| m.as_str()).unwrap_or("");
    match method {
        "initialize" => serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": "2025-03-26",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "Dozer", "version": env!("CARGO_PKG_VERSION")},
            }
        }),
        "tools/list" => serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "tools": [
                    {
                        "name": "getCurrentSelection",
                        "description": "Get current editor selection",
                        "inputSchema": {"type": "object", "properties": {}},
                    },
                    {
                        "name": "getOpenEditors",
                        "description": "Get list of open editor tabs",
                        "inputSchema": {"type": "object", "properties": {}},
                    },
                    {
                        "name": "getDiagnostics",
                        "description": "Get diagnostics for a file",
                        "inputSchema": {"type": "object", "properties": {}},
                    },
                ]
            }
        }),
        "tools/call" => {
            let name = request
                .pointer("/params/name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            match tool_call_result(name, ctx) {
                Ok(result) => serde_json::json!({"jsonrpc": "2.0", "id": id, "result": result}),
                Err(message) => serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {"code": -32601, "message": message},
                }),
            }
        }
        other => serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {"code": -32601, "message": format!("Method not found: {other}")},
        }),
    }
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozerd ide_bridge::rpc_tests`
Expected: 5 个测试全 PASS。

- [ ] **Step 5: 提交**

```bash
git add crates/dozerd/src/ide_bridge.rs
git commit -m "feat(dozerd): ide_bridge JSON-RPC 请求分发"
```

---

## Task 4: WebSocket server + 鉴权 + `IdeBridgeRegistry`

**Files:**
- Modify: `crates/dozerd/src/ide_bridge.rs`

**Interfaces:**
- Consumes:
  - `lock_dir()`/`write_lock_file`/`remove_lock_file`/`generate_token`(Task 1)
  - `handle_rpc_request`(Task 3)
  - `crate::preview_context::PreviewContextStore`(既有代码,`pub fn get(&self, project_id: i64) -> Option<PreviewContext>`)
- Produces(Task 6 直接依赖这三个方法完成生命周期接线):
  - `pub(crate) struct IdeBridgeRegistry`
  - `pub(crate) fn IdeBridgeRegistry::new(lock_dir: PathBuf, preview_contexts: Arc<PreviewContextStore>) -> Arc<Self>`
  - `pub(crate) async fn IdeBridgeRegistry::session_started(self: &Arc<Self>, project_id: i64, workspace_root: &str)`
  - `pub(crate) async fn IdeBridgeRegistry::session_ended(&self, project_id: i64)`
  - `pub(crate) async fn IdeBridgeRegistry::active_projects(&self) -> Vec<i64>`(测试/自检用)

- [ ] **Step 1: 写 `IdeBridgeRegistry` 生命周期(不连真实网络)的失败测试**

追加到 `ide_bridge.rs`:

```rust
#[cfg(test)]
mod registry_lifecycle_tests {
    use super::*;
    use crate::preview_context::PreviewContextStore;
    use std::sync::Arc;

    fn new_registry() -> (Arc<IdeBridgeRegistry>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let registry = IdeBridgeRegistry::new(dir.path().to_path_buf(), Arc::new(PreviewContextStore::new()));
        (registry, dir)
    }

    #[tokio::test]
    async fn first_session_starts_bridge_second_session_same_project_is_noop() {
        let (registry, _dir) = new_registry();
        registry.session_started(1, "/repo").await;
        assert_eq!(registry.active_projects().await, vec![1]);
        registry.session_started(1, "/repo").await;
        assert_eq!(registry.active_projects().await, vec![1]);
    }

    #[tokio::test]
    async fn different_projects_get_independent_bridges() {
        let (registry, _dir) = new_registry();
        registry.session_started(1, "/repo-a").await;
        registry.session_started(2, "/repo-b").await;
        let mut projects = registry.active_projects().await;
        projects.sort();
        assert_eq!(projects, vec![1, 2]);
    }

    #[tokio::test]
    async fn last_session_ending_stops_bridge_and_removes_lock_file() {
        let (registry, dir) = new_registry();
        registry.session_started(1, "/repo").await;
        registry.session_started(1, "/repo").await; // 2 个会话
        registry.session_ended(1).await; // 还剩 1 个,不该停
        assert_eq!(registry.active_projects().await, vec![1]);
        registry.session_ended(1).await; // 归零,该停
        assert!(registry.active_projects().await.is_empty());
        // 锁文件目录应该已经清空(唯一的锁文件被删掉了)
        let remaining: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert!(remaining.is_empty());
    }

    #[tokio::test]
    async fn ending_a_project_with_no_active_bridge_is_a_harmless_noop() {
        let (registry, _dir) = new_registry();
        registry.session_ended(999).await;
        assert!(registry.active_projects().await.is_empty());
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozerd ide_bridge::registry_lifecycle_tests`
Expected: 编译失败,报 `IdeBridgeRegistry` 未定义。

- [ ] **Step 3: 实现 `IdeBridgeRegistry` + 监听循环 + 连接处理**

在 `ide_bridge.rs` 顶部加:

```rust
use crate::preview_context::PreviewContextStore;
use futures_util::{SinkExt, StreamExt};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex as AsyncMutex, oneshot};
use tokio_tungstenite::tungstenite::Message;
```

追加实现:

```rust
struct ProjectBridgeHandle {
    active_sessions: usize,
    shutdown_tx: oneshot::Sender<()>,
    lock_path: PathBuf,
}

pub(crate) struct IdeBridgeRegistry {
    lock_dir: PathBuf,
    preview_contexts: Arc<PreviewContextStore>,
    inner: AsyncMutex<HashMap<i64, ProjectBridgeHandle>>,
}

impl IdeBridgeRegistry {
    pub(crate) fn new(lock_dir: PathBuf, preview_contexts: Arc<PreviewContextStore>) -> Arc<Self> {
        Arc::new(Self {
            lock_dir,
            preview_contexts,
            inner: AsyncMutex::new(HashMap::new()),
        })
    }

    /// 项目活跃会话数 0→1 起 bridge,已存在则只加计数。绑定/写锁文件失败时
    /// 只记日志、静默放弃这个项目的自动上下文——不影响会话本身正常使用
    /// (spec"风险/未知项"一节已确认的取舍)。
    pub(crate) async fn session_started(self: &Arc<Self>, project_id: i64, workspace_root: &str) {
        let mut map = self.inner.lock().await;
        if let Some(handle) = map.get_mut(&project_id) {
            handle.active_sessions += 1;
            return;
        }
        let listener = match TcpListener::bind(("127.0.0.1", 0)).await {
            Ok(l) => l,
            Err(e) => {
                tracing::warn!(project_id, error = %e, "ide_bridge 绑定端口失败,跳过该项目的自动上下文");
                return;
            }
        };
        let port = match listener.local_addr() {
            Ok(addr) => addr.port(),
            Err(e) => {
                tracing::warn!(project_id, error = %e, "ide_bridge 读取本地端口失败,跳过");
                return;
            }
        };
        let token = generate_token();
        let lock_path = match write_lock_file(&self.lock_dir, port, workspace_root, &token, std::process::id()) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(project_id, error = %e, "ide_bridge 写锁文件失败,跳过");
                return;
            }
        };
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let preview_contexts = self.preview_contexts.clone();
        tokio::spawn(run_bridge_listener(listener, project_id, token, preview_contexts, shutdown_rx));
        map.insert(
            project_id,
            ProjectBridgeHandle { active_sessions: 1, shutdown_tx, lock_path },
        );
    }

    /// 项目活跃会话数减一,归零时停监听 + 删锁文件。项目本来就没有活跃
    /// bridge 时是无害 no-op(调用方不需要先查再调)。
    pub(crate) async fn session_ended(&self, project_id: i64) {
        let mut map = self.inner.lock().await;
        let Some(handle) = map.get_mut(&project_id) else {
            return;
        };
        handle.active_sessions = handle.active_sessions.saturating_sub(1);
        if handle.active_sessions == 0 {
            let handle = map.remove(&project_id).expect("刚判断过存在");
            let _ = handle.shutdown_tx.send(());
            remove_lock_file(&handle.lock_path);
        }
    }

    #[cfg(test)]
    pub(crate) async fn active_projects(&self) -> Vec<i64> {
        self.inner.lock().await.keys().copied().collect()
    }
}

async fn run_bridge_listener(
    listener: TcpListener,
    project_id: i64,
    token: String,
    preview_contexts: Arc<PreviewContextStore>,
    mut shutdown_rx: oneshot::Receiver<()>,
) {
    loop {
        tokio::select! {
            _ = &mut shutdown_rx => break,
            accepted = listener.accept() => {
                let Ok((stream, _)) = accepted else { continue };
                tokio::spawn(handle_bridge_connection(
                    stream,
                    project_id,
                    token.clone(),
                    preview_contexts.clone(),
                ));
            }
        }
    }
}

async fn handle_bridge_connection(
    stream: TcpStream,
    project_id: i64,
    token: String,
    preview_contexts: Arc<PreviewContextStore>,
) {
    // 握手回调里只捕获客户端带的 auth header,不在这一步拒绝——
    // `accept_hdr_async` 的错误响应构造依赖 tungstenite 具体版本的 API 形状,
    // 握手完成后立即用捕获到的值比对再关连接,逻辑等价且不依赖那部分 API。
    let presented = Arc::new(std::sync::Mutex::new(None::<String>));
    let capture = {
        let presented = presented.clone();
        move |req: &tokio_tungstenite::tungstenite::handshake::server::Request,
              resp: tokio_tungstenite::tungstenite::handshake::server::Response| {
            let header = req
                .headers()
                .get("x-claude-code-ide-authorization")
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned);
            *presented.lock().expect("auth header 锁") = header;
            Ok(resp)
        }
    };
    let Ok(ws) = tokio_tungstenite::accept_hdr_async(stream, capture).await else {
        return;
    };
    let presented = presented.lock().expect("auth header 锁").clone();
    if presented.as_deref() != Some(token.as_str()) {
        tracing::debug!(project_id, "ide_bridge 鉴权失败,关闭连接");
        return;
    }
    let (mut write, mut read) = ws.split();
    while let Some(Ok(msg)) = read.next().await {
        let Message::Text(text) = msg else { continue };
        let Ok(req) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
        let ctx = preview_contexts.get(project_id);
        let resp = handle_rpc_request(&req, ctx.as_ref());
        if write.send(Message::Text(resp.to_string())).await.is_err() {
            break;
        }
    }
}
```

- [ ] **Step 4: 运行生命周期测试确认通过**

Run: `cargo test -p dozerd ide_bridge::registry_lifecycle_tests`
Expected: 4 个测试全 PASS。

- [ ] **Step 5: 写真实 WebSocket 往返的测试(验证鉴权 + JSON-RPC 循环真的接上了)**

追加到 `ide_bridge.rs`:

```rust
#[cfg(test)]
mod live_connection_tests {
    use super::*;
    use crate::preview_context::PreviewContextStore;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_tungstenite::tungstenite::http::HeaderValue;

    async fn started_registry_with_port(project_id: i64) -> (Arc<IdeBridgeRegistry>, tempfile::TempDir, u16, String) {
        let dir = tempfile::tempdir().unwrap();
        let registry = IdeBridgeRegistry::new(dir.path().to_path_buf(), Arc::new(PreviewContextStore::new()));
        registry.session_started(project_id, "/repo").await;
        // 起监听是异步的,给事件循环一拍机会把锁文件写完
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let entry = std::fs::read_dir(dir.path())
            .unwrap()
            .next()
            .expect("bridge 应该已经写出锁文件")
            .unwrap();
        let raw = std::fs::read_to_string(entry.path()).unwrap();
        let contents: LockFileContents = serde_json::from_str(&raw).unwrap();
        (registry, dir, contents.port, contents.token)
    }

    #[tokio::test]
    async fn correct_token_can_initialize_and_call_get_current_selection() {
        let (_registry, _dir, port, token) = started_registry_with_port(42).await;
        let mut request = format!("ws://127.0.0.1:{port}/").into_client_request().unwrap();
        request
            .headers_mut()
            .insert("x-claude-code-ide-authorization", HeaderValue::from_str(&token).unwrap());
        let (ws, _) = tokio_tungstenite::connect_async(request).await.expect("握手应成功");
        let (mut write, mut read) = ws.split();

        write
            .send(Message::Text(
                serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize"}).to_string(),
            ))
            .await
            .unwrap();
        let reply = read.next().await.unwrap().unwrap();
        let Message::Text(text) = reply else { panic!("期望文本帧") };
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["result"]["protocolVersion"], "2025-03-26");
    }

    #[tokio::test]
    async fn wrong_token_gets_connection_closed_without_json_rpc_exchange() {
        let (_registry, _dir, port, _token) = started_registry_with_port(43).await;
        let mut request = format!("ws://127.0.0.1:{port}/").into_client_request().unwrap();
        request
            .headers_mut()
            .insert("x-claude-code-ide-authorization", HeaderValue::from_str("wrong-token").unwrap());
        let (ws, _) = tokio_tungstenite::connect_async(request).await.expect("WS 握手本身仍会成功");
        let (mut write, mut read) = ws.split();
        let _ = write
            .send(Message::Text(
                serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize"}).to_string(),
            ))
            .await;
        // 服务端鉴权失败后直接丢连接,不会有任何回包
        let next = tokio::time::timeout(std::time::Duration::from_millis(200), read.next()).await;
        match next {
            Ok(Some(Ok(_))) => panic!("鉴权失败不应该收到任何应答帧"),
            _ => {} // 超时或连接已关闭,都是期望结果
        }
    }
}
```

- [ ] **Step 6: 运行测试确认通过**

Run: `cargo test -p dozerd ide_bridge::live_connection_tests`
Expected: 2 个测试全 PASS。(如果 `accept_hdr_async`/`IntoClientRequest`/`connect_async` 的具体签名跟本计划写的不完全一致,以 `cargo build` 报出的编译错误为准调整调用方式,逻辑保持不变。)

- [ ] **Step 7: 跑一遍 `ide_bridge` 全量测试 + clippy**

Run: `cargo test -p dozerd ide_bridge:: && cargo clippy -p dozerd --all-targets`
Expected: 全部 PASS,无新增 clippy 警告。

- [ ] **Step 8: 提交**

```bash
git add crates/dozerd/src/ide_bridge.rs
git commit -m "feat(dozerd): ide_bridge WebSocket server + 鉴权 + 生命周期 registry"
```

---

## Task 5: 启动时清扫陈旧锁文件

**Files:**
- Modify: `crates/dozerd/src/ide_bridge.rs`

**Interfaces:**
- Consumes: `LockFileContents`(Task 1)
- Produces: `pub(crate) fn sweep_stale_locks(dir: &Path)`(Task 6 在 `serve()` 起监听之前调用一次)

- [ ] **Step 1: 写清扫逻辑的失败测试**

追加到 `ide_bridge.rs`:

```rust
#[cfg(test)]
mod sweep_tests {
    use super::*;

    #[test]
    fn sweep_removes_dead_pid_keeps_alive_pid() {
        let dir = tempfile::tempdir().unwrap();
        let alive_pid = std::process::id(); // 用当前测试进程自己的 pid 模拟"存活"
        write_lock_file(dir.path(), 1, "/repo/a", "tok-a", alive_pid).unwrap();
        write_lock_file(dir.path(), 2, "/repo/b", "tok-b", 999_999).unwrap(); // 几乎不可能存在的 pid
        sweep_stale_locks(dir.path());
        let mut remaining: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        remaining.sort();
        assert_eq!(remaining, vec!["1.lock".to_string()]);
    }

    #[test]
    fn sweep_removes_unparseable_lock_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("garbage.lock"), "not json").unwrap();
        sweep_stale_locks(dir.path());
        assert!(std::fs::read_dir(dir.path()).unwrap().next().is_none());
    }

    #[test]
    fn sweep_on_missing_dir_is_a_harmless_noop() {
        sweep_stale_locks(Path::new("/tmp/dozer-ide-bridge-sweep-test-missing-dir"));
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozerd ide_bridge::sweep_tests`
Expected: 编译失败,报 `sweep_stale_locks` 未定义。

- [ ] **Step 3: 实现清扫逻辑**

```rust
pub(crate) fn sweep_stale_locks(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("lock") {
            continue;
        }
        let parsed = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str::<LockFileContents>(&raw).ok());
        match parsed {
            Some(contents) if pid_is_alive(contents.pid) => {}
            _ => {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
}

#[cfg(unix)]
fn pid_is_alive(pid: u32) -> bool {
    // signal 0:不真的发信号,只用来探测目标 pid 是否存在/是否有权限操作它。
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

#[cfg(not(unix))]
fn pid_is_alive(_pid: u32) -> bool {
    true // 非 unix 平台(项目当前 mac 先发,未覆盖):保守起见当作存活,不清理
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozerd ide_bridge::sweep_tests`
Expected: 3 个测试全 PASS。

- [ ] **Step 5: 提交**

```bash
git add crates/dozerd/src/ide_bridge.rs
git commit -m "feat(dozerd): ide_bridge 启动时清扫陈旧锁文件"
```

---

## Task 6: 接入会话生命周期(`server.rs`)

**Files:**
- Modify: `crates/dozerd/src/server.rs`
- Test: 追加到 `crates/dozerd/src/server.rs` 现有 `#[cfg(test)]` 区域(参照 `registry.rs` 里 `create_get_list_roundtrip` 等既有测试的写法)

**Interfaces:**
- Consumes:
  - `crate::ide_bridge::{IdeBridgeRegistry, lock_dir, sweep_stale_locks}`(Task 1/4/5)
  - `crate::session::{Session, SessionEvent, SessionSpec}`(既有代码,`Session::subscribe(&self) -> broadcast::Receiver<SessionEvent>` 已存在)
  - `crate::registry::SessionRegistry::create(&self, spec: SessionSpec) -> Result<Arc<Session>>`(既有代码)
- Produces:(无下游任务——这是本功能最后一个任务,收尾接线)

- [ ] **Step 1: 在 `serve()` 里构造 registry + 清扫锁文件**

打开 `crates/dozerd/src/server.rs`,在文件顶部 `use` 区加一行:

```rust
use crate::ide_bridge::IdeBridgeRegistry;
```

找到 `pub async fn serve(` 函数体开头(现在是 `let preview_contexts = Arc::new(PreviewContextStore::new());` 那一行),改成:

```rust
    let preview_contexts = Arc::new(PreviewContextStore::new());
    let ide_lock_dir = crate::ide_bridge::lock_dir();
    crate::ide_bridge::sweep_stale_locks(&ide_lock_dir);
    let ide_bridge = IdeBridgeRegistry::new(ide_lock_dir, preview_contexts.clone());
```

- [ ] **Step 2: 把 `ide_bridge` 传进每个连接**

在 `serve()` 的 accept 循环里(`let preview_contexts = preview_contexts.clone();` 那一行之后)加一行:

```rust
        let ide_bridge = ide_bridge.clone();
```

在 `tokio::spawn(async move { if let Err(e) = handle_conn(` 的参数列表里,`preview_contexts,` 那一行之后加:

```rust
                ide_bridge,
```

- [ ] **Step 3: 扩展 `handle_conn` 签名**

`handle_conn` 函数签名里,`preview_contexts: Arc<PreviewContextStore>,` 那一行之后加:

```rust
    ide_bridge: Arc<IdeBridgeRegistry>,
```

(`handle_conn` 已经有 `#[allow(clippy::too_many_arguments)]`——这里全部是不同具体类型的 `Arc<T>`,顺序传错编译器会因类型不匹配直接报错,不属于 CLAUDE.md 里"同类型参数相邻、编译器发现不了"的那类风险,延续既有写法而不是抽参数结构体。)

- [ ] **Step 4: `CreateSession` 分支接入 bridge 生命周期**

找到:

```rust
                        Request::CreateSession { name, command, args, cwd, cols, rows, project_id } => {
                            match registry.create(SessionSpec { name, command, args, cwd, cols, rows, project_id }) {
                                Ok(s) => Reply::Created { session: s.info() },
                                Err(e) => Reply::Error { message: e.to_string() },
                            }
                        }
```

改成:

```rust
                        Request::CreateSession { name, command, args, cwd, cols, rows, project_id } => {
                            match registry.create(SessionSpec { name, command, args, cwd: cwd.clone(), cols, rows, project_id }) {
                                Ok(s) => {
                                    ide_bridge.session_started(project_id, &cwd).await;
                                    let ide_bridge_watch = ide_bridge.clone();
                                    let mut exit_rx = s.subscribe();
                                    tokio::spawn(async move {
                                        loop {
                                            match exit_rx.recv().await {
                                                Ok(SessionEvent::Exited { .. }) => {
                                                    ide_bridge_watch.session_ended(project_id).await;
                                                    break;
                                                }
                                                Ok(_) => continue,
                                                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                                                Err(broadcast::error::RecvError::Closed) => break,
                                            }
                                        }
                                    });
                                    Reply::Created { session: s.info() }
                                }
                                Err(e) => Reply::Error { message: e.to_string() },
                            }
                        }
```

(`cwd` 原本被移动进 `SessionSpec`,这里先 `cwd.clone()` 再移动,是因为后面 `ide_bridge.session_started` 还需要用它当 `workspace_root`。`broadcast::error::RecvError` 需要在文件顶部 `use` 区补一行 `use tokio::sync::broadcast;` 的 `error` 路径——检查文件顶部已有 `use tokio::sync::broadcast;`,直接用 `broadcast::error::RecvError` 全路径引用即可,不需要额外 `use`。)

- [ ] **Step 5: 编译,确认签名改动没有漏改的调用点**

Run: `cargo build -p dozerd`
Expected: 编译通过。如果报"缺少参数"之类的错误,说明还有别处调用 `handle_conn` 或 `serve` 没跟着改(目前只有 `serve()` 内部一处调用 `handle_conn`,`main.rs` 里只调用 `serve()` 本身、没有新增参数,所以不需要动 `main.rs`)。

- [ ] **Step 6: 写集成测试(真实 spawn 的会话,不 mock)**

在 `server.rs` 现有的 `#[cfg(test)]` 模块里(找到已有的测试辅助函数风格,如 `registry.rs` 里的 `spec(cmd: &str)`)追加:

```rust
#[cfg(test)]
mod ide_bridge_lifecycle_tests {
    use super::*;
    use crate::ide_bridge::IdeBridgeRegistry;
    use crate::preview_context::PreviewContextStore;
    use crate::registry::SessionRegistry;
    use crate::session::SessionSpec;
    use std::time::Duration;

    fn spec(project_id: i64, cmd: &str) -> SessionSpec {
        SessionSpec {
            name: "t".into(),
            command: "/bin/sh".into(),
            args: vec!["-c".into(), cmd.into()],
            cwd: std::env::temp_dir().to_string_lossy().into_owned(),
            cols: 80,
            rows: 24,
            project_id,
        }
    }

    #[tokio::test]
    async fn bridge_starts_on_create_and_stops_after_process_exits() {
        let dir = tempfile::tempdir().unwrap();
        let ide_bridge = IdeBridgeRegistry::new(dir.path().to_path_buf(), Arc::new(PreviewContextStore::new()));
        let registry = SessionRegistry::new();

        let s = registry.create(spec(7, "printf ready; sleep 5")).unwrap();
        ide_bridge.session_started(7, "/repo").await;
        assert_eq!(ide_bridge.active_projects().await, vec![7]);

        let mut exit_rx = s.subscribe();
        let ide_bridge_watch = ide_bridge.clone();
        let watcher = tokio::spawn(async move {
            loop {
                match exit_rx.recv().await {
                    Ok(SessionEvent::Exited { .. }) => {
                        ide_bridge_watch.session_ended(7).await;
                        break;
                    }
                    Ok(_) => continue,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });

        registry.kill(s.id()).unwrap();
        tokio::time::timeout(Duration::from_secs(2), watcher)
            .await
            .expect("watcher 任务应在超时前结束")
            .expect("watcher 任务不应 panic");

        assert!(ide_bridge.active_projects().await.is_empty());
    }
}
```

- [ ] **Step 7: 运行测试确认通过**

Run: `cargo test -p dozerd ide_bridge_lifecycle_tests`
Expected: 1 个测试 PASS。

- [ ] **Step 8: 跑一遍 dozerd 全量测试 + clippy + fmt**

Run: `cargo test -p dozerd && cargo clippy -p dozerd --all-targets && cargo fmt -- --check`
Expected: 全部通过。`cargo fmt` 如果报格式问题,直接跑 `cargo fmt` 修正后重新跑一次 `--check`。

- [ ] **Step 9: 提交**

```bash
git add crates/dozerd/src/server.rs
git commit -m "feat(dozerd): 会话生命周期接入 ide_bridge 起停"
```

- [ ] **Step 10: 人工验收(不能自动化,必须手工做)**

1. 用本地构建跑起 `dozer-app`(`cargo run -p dozer-app`),打开一个真实项目,在预览面板打开一个文件、选中一段内容。
2. 在该项目下新开一个跑 Claude Code 的会话(确认 Claude Code CLI 版本支持 IDE 集成;如果这台机器同时装了 VS Code/JetBrains 的 Claude Code 插件,先确认没有它们的锁文件残留导致连错端口)。
3. 观察 `~/.claude/ide/` 目录:该项目对应的会话建立后,应该出现一个新的 `<port>.lock` 文件,内容里的 `workspaceFolders` 指向这个项目路径。
4. 在 Claude Code 里发一条消息,检查回复前是否出现 `In <文件名>` 这行提示,文件名应该跟预览面板里打开的文件一致。
5. 如果没出现:对照 `coder/claudecode.nvim` 的 `PROTOCOL.md` 检查 Task 2/3 里 `selection_result`/`handle_rpc_request` 的字段名和结构是否跟参考实现一致,按需调整(这一步的字段形状本来就标注为"最佳草案,以实测为准")。
6. 关掉该会话(退出 Claude Code 或 kill 会话),确认 `~/.claude/ide/` 里对应的锁文件被删除。
7. 把这一步的实测结果(成功/需要调整了哪些字段)记录进 commit message 或后续 PR 描述,不能只凭代码编译通过和单测通过就认为这个功能已经生效。

---

## Self-Review Notes

- **Spec 覆盖检查**:范围(仅上下文,不做 diagnostics/openDiff)→ Task 3/4 只声明三个工具且 `getDiagnostics` 恒空;默认自动开启无开关 → 全计划没有新增设置项;每项目一份锁文件+server → `IdeBridgeRegistry` 按 `project_id` 索引;生命周期挂靠会话计数 → Task 6;权限 `0700`/`0600` → Task 1;token CSPRNG → Task 1(`uuid::Uuid::new_v4()`);鉴权 header → Task 4;不持久化跨重启 → 未新增任何落盘状态,`IdeBridgeRegistry` 纯内存;启动清扫陈旧锁 → Task 5,并在 Task 6 Step 1 接进 `serve()`。全部覆盖。
- **占位符扫描**:未发现 "TBD"/"实现细节见后文" 之类的表述;字段形状标注为"最佳草案"的地方(Task 2/3)都给了完整可编译代码,只是提示可能需要按 Task 6 Step 10 的实测结果调整字段名,不是空着不写。
- **类型一致性检查**:`selection_result`/`open_editors_result`/`diagnostics_result`(Task 2)→ `handle_rpc_request`(Task 3)→ `handle_bridge_connection`(Task 4)→ `IdeBridgeRegistry::session_started`/`session_ended`(Task 4)→ `server.rs` 的 `CreateSession` 分支(Task 6),函数名/签名在各任务间保持一致;`PreviewContext` 字段名跟 Task 1 读到的既有定义(`path`/`start_line`/`start_col`/`end_line`/`end_col`/`has_selection`/`updated_at_ms`)完全对齐。
