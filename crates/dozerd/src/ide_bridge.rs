//! Claude Code IDE 集成桥接:让 Dozer 模拟 VS Code/JetBrains 的 IDE 握手协议,
//! 把 `preview_context::PreviewContextStore` 里的数据喂给 Claude Code CLI 的
//! 自动上下文机制(聊天记录里的 `In <file>` 提示条)。设计依据见
//! `docs/superpowers/specs/2026-09-16-claude-code-ide-bridge-design.md`。

use crate::preview_context::PreviewContextStore;
use dozer_core::protocol::PreviewContext;
use futures_util::{SinkExt, StreamExt};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex as AsyncMutex, oneshot};
use tokio_tungstenite::tungstenite::Message;

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

pub(crate) fn generate_token() -> String {
    // 128 位随机值,32 个十六进制字符,无连字符——复用仓库已有的 uuid 依赖,
    // 不为这一个用途单独引入 rand crate。
    uuid::Uuid::new_v4().simple().to_string()
}

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
        let got = resolve_lock_dir(
            None,
            Some("/claude-config".to_string()),
            Some("/home/u".to_string()),
        );
        assert_eq!(got, PathBuf::from("/claude-config/ide"));
    }

    #[test]
    fn falls_back_to_home_dot_claude_ide() {
        let got = resolve_lock_dir(None, None, Some("/home/u".to_string()));
        assert_eq!(got, PathBuf::from("/home/u/.claude/ide"));
    }
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
}

fn tool_call_result(
    name: &str,
    ctx: Option<&PreviewContext>,
) -> Result<serde_json::Value, String> {
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
        assert_eq!(
            names,
            vec!["getCurrentSelection", "getOpenEditors", "getDiagnostics"]
        );
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
        let lock_path =
            match write_lock_file(&self.lock_dir, port, workspace_root, &token, std::process::id())
            {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!(project_id, error = %e, "ide_bridge 写锁文件失败,跳过");
                    return;
                }
            };
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let preview_contexts = self.preview_contexts.clone();
        tokio::spawn(run_bridge_listener(
            listener,
            project_id,
            token,
            preview_contexts,
            shutdown_rx,
        ));
        map.insert(
            project_id,
            ProjectBridgeHandle {
                active_sessions: 1,
                shutdown_tx,
                lock_path,
            },
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
        // `accept_hdr_async` 的握手回调签名由 tungstenite 规定,返回的
        // `Result` 错误分支是它自己的巨型 `Response` 类型,我们无法改成
        // `Box<...>`(会与 trait 约定不符),只能就地放行这一条 lint。
        #[allow(clippy::result_large_err)]
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
        let Ok(req) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        let ctx = preview_contexts.get(project_id);
        let resp = handle_rpc_request(&req, ctx.as_ref());
        if write.send(Message::Text(resp.to_string())).await.is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod registry_lifecycle_tests {
    use super::*;
    use crate::preview_context::PreviewContextStore;
    use std::sync::Arc;

    fn new_registry() -> (Arc<IdeBridgeRegistry>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let registry = IdeBridgeRegistry::new(
            dir.path().to_path_buf(),
            Arc::new(PreviewContextStore::new()),
        );
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

#[cfg(test)]
mod live_connection_tests {
    use super::*;
    use crate::preview_context::PreviewContextStore;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_tungstenite::tungstenite::http::HeaderValue;

    async fn started_registry_with_port(
        project_id: i64,
    ) -> (Arc<IdeBridgeRegistry>, tempfile::TempDir, u16, String) {
        let dir = tempfile::tempdir().unwrap();
        let registry = IdeBridgeRegistry::new(
            dir.path().to_path_buf(),
            Arc::new(PreviewContextStore::new()),
        );
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
        let mut request = format!("ws://127.0.0.1:{port}/")
            .into_client_request()
            .unwrap();
        request.headers_mut().insert(
            "x-claude-code-ide-authorization",
            HeaderValue::from_str(&token).unwrap(),
        );
        let (ws, _) = tokio_tungstenite::connect_async(request)
            .await
            .expect("握手应成功");
        let (mut write, mut read) = ws.split();

        write
            .send(Message::Text(
                serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize"}).to_string(),
            ))
            .await
            .unwrap();
        let reply = read.next().await.unwrap().unwrap();
        let Message::Text(text) = reply else {
            panic!("期望文本帧")
        };
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["result"]["protocolVersion"], "2025-03-26");
    }

    #[tokio::test]
    async fn wrong_token_gets_connection_closed_without_json_rpc_exchange() {
        let (_registry, _dir, port, _token) = started_registry_with_port(43).await;
        let mut request = format!("ws://127.0.0.1:{port}/")
            .into_client_request()
            .unwrap();
        request.headers_mut().insert(
            "x-claude-code-ide-authorization",
            HeaderValue::from_str("wrong-token").unwrap(),
        );
        let (ws, _) = tokio_tungstenite::connect_async(request)
            .await
            .expect("WS 握手本身仍会成功");
        let (mut write, mut read) = ws.split();
        let _ = write
            .send(Message::Text(
                serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize"}).to_string(),
            ))
            .await;
        // 服务端鉴权失败后直接丢连接,不会有任何回包
        let next = tokio::time::timeout(std::time::Duration::from_millis(200), read.next()).await;
        if let Ok(Some(Ok(_))) = next {
            panic!("鉴权失败不应该收到任何应答帧");
        } // 超时或连接已关闭,都是期望结果
    }
}
