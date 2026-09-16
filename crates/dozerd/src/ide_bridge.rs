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

/// `pub`(不是 `pub(crate)`):`dozerd` 的 `main.rs` 是独立的 bin crate,
/// 只能看到 lib crate 里真正公开的符号,拿生产环境默认锁目录得靠它。
pub fn lock_dir() -> PathBuf {
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
    let id = request
        .get("id")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
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

/// 每个连接最多等待这么久完成 WS 握手,超时直接丢弃连接——本地
/// (`127.0.0.1` 恒可达)握手正常应该是毫秒级,给个宽松上限只是防止一个
/// 连不上/不说话的客户端占着任务不放。
const HANDSHAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
/// 单个项目 bridge 同时允许的最大连接数——本地场景下 Claude Code 正常只会
/// 建一条连接,恒为 0/1;这个上限只是防止异常/失控的本地进程无限攒连接,
/// 不是真实容量规划。
const MAX_CONCURRENT_CONNECTIONS: usize = 4;

pub(crate) struct IdeBridgeRegistry {
    lock_dir: PathBuf,
    preview_contexts: Arc<PreviewContextStore>,
    // 外层只锁"project_id -> 该项目专属锁"这张索引,查一次就放手;真正的
    // 起停记账 + 绑端口/写锁文件这些慢操作,由每个项目自己独立的
    // `AsyncMutex` 保护,不同项目之间完全不互相阻塞——这正是要解决的问题:
    // 避免一个项目的磁盘 IO/端口绑定卡住同一时刻其它项目的会话起停记账。
    // 索引本身只增不减(条目数等于本次 daemon 运行期间出现过的不同项目
    // 数,量级是"用户开过的项目数",这点常驻内存可以接受,换来的是完全
    // 不需要处理"移除索引条目时是否有并发调用者正持有它"这类竞态)。
    projects: std::sync::Mutex<HashMap<i64, Arc<AsyncMutex<Option<ProjectBridgeHandle>>>>>,
}

impl IdeBridgeRegistry {
    pub(crate) fn new(lock_dir: PathBuf, preview_contexts: Arc<PreviewContextStore>) -> Arc<Self> {
        Arc::new(Self {
            lock_dir,
            preview_contexts,
            projects: std::sync::Mutex::new(HashMap::new()),
        })
    }

    /// 拿到(必要时创建)某个项目专属的锁——这一步本身只锁索引表,不持锁
    /// 跨 `.await`,拿到手的 `Arc<AsyncMutex<..>>` 之后再各自独立加锁。
    fn project_lock(&self, project_id: i64) -> Arc<AsyncMutex<Option<ProjectBridgeHandle>>> {
        self.projects
            .lock()
            .expect("ide_bridge projects 索引锁")
            .entry(project_id)
            .or_insert_with(|| Arc::new(AsyncMutex::new(None)))
            .clone()
    }

    /// 项目活跃会话数 0→1 起 bridge,已存在则只加计数,返回 `true`。
    ///
    /// 返回 `false` 表示这次调用没能让项目挂上任何 bridge(绑端口/写锁
    /// 文件失败)——调用方(`server.rs`)必须只在返回 `true` 时才去 spawn
    /// 对应的退出监听器,否则这个会话将来退出时会对一个自己从未真正占过
    /// 名额的项目调用 `session_ended`,可能把同项目下另一个真正活着的
    /// 会话的 bridge 提前拆掉。
    ///
    /// 绑端口 + 写锁文件这两步慢操作,是在持有"这一个项目专属"的锁期间做
    /// 的——会阻塞同一项目里并发到达的第二个 `session_started`/
    /// `session_ended` 调用(它们排队等这把锁,等到手时要么看见已经起好的
    /// bridge 直接计数,要么看见上一次失败留下的 `None` 自己重新尝试起,
    /// 两种情况都不需要额外的状态机),但完全不影响其它项目——那些项目有
    /// 各自独立的锁。
    pub(crate) async fn session_started(&self, project_id: i64, workspace_root: &str) -> bool {
        let project_lock = self.project_lock(project_id);
        let mut slot = project_lock.lock().await;
        if let Some(handle) = slot.as_mut() {
            handle.active_sessions += 1;
            return true;
        }

        let listener = match TcpListener::bind(("127.0.0.1", 0)).await {
            Ok(l) => l,
            Err(e) => {
                tracing::warn!(project_id, error = %e, "ide_bridge 绑定端口失败,跳过该项目的自动上下文");
                return false;
            }
        };
        let port = match listener.local_addr() {
            Ok(addr) => addr.port(),
            Err(e) => {
                tracing::warn!(project_id, error = %e, "ide_bridge 读取本地端口失败,跳过");
                return false;
            }
        };
        let token = generate_token();
        // 写锁文件是同步阻塞 IO(建目录/设权限/写文件),挪到阻塞线程池
        // 执行,不占用当前 tokio 工作线程。
        let lock_dir = self.lock_dir.clone();
        let workspace_root_owned = workspace_root.to_string();
        let write_token = token.clone();
        let write_result = tokio::task::spawn_blocking(move || {
            write_lock_file(
                &lock_dir,
                port,
                &workspace_root_owned,
                &write_token,
                std::process::id(),
            )
        })
        .await;
        let lock_path = match write_result {
            Ok(Ok(path)) => path,
            Ok(Err(e)) => {
                tracing::warn!(project_id, error = %e, "ide_bridge 写锁文件失败,跳过");
                return false;
            }
            Err(e) => {
                tracing::warn!(project_id, error = %e, "ide_bridge 写锁文件任务崩溃,跳过");
                return false;
            }
        };

        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        tokio::spawn(run_bridge_listener(
            listener,
            project_id,
            token,
            self.preview_contexts.clone(),
            shutdown_rx,
        ));
        *slot = Some(ProjectBridgeHandle {
            active_sessions: 1,
            shutdown_tx,
            lock_path,
        });
        true
    }

    /// 项目活跃会话数减一,归零时停监听 + 删锁文件。项目本来就没有活跃
    /// bridge 时是无害 no-op(调用方不需要先查再调)。
    pub(crate) async fn session_ended(&self, project_id: i64) {
        let project_lock = {
            let map = self.projects.lock().expect("ide_bridge projects 索引锁");
            match map.get(&project_id) {
                Some(lock) => lock.clone(),
                None => return,
            }
        };
        let mut slot = project_lock.lock().await;
        let remaining = match slot.as_mut() {
            Some(handle) => {
                handle.active_sessions = handle.active_sessions.saturating_sub(1);
                handle.active_sessions
            }
            None => return,
        };
        if remaining == 0
            && let Some(handle) = slot.take()
        {
            let _ = handle.shutdown_tx.send(());
            remove_lock_file(&handle.lock_path);
        }
    }

    #[cfg(test)]
    pub(crate) async fn active_projects(&self) -> Vec<i64> {
        let entries: Vec<(i64, Arc<AsyncMutex<Option<ProjectBridgeHandle>>>)> = {
            let map = self.projects.lock().expect("ide_bridge projects 索引锁");
            map.iter().map(|(id, lock)| (*id, lock.clone())).collect()
        };
        let mut active = Vec::new();
        for (id, lock) in entries {
            if lock.lock().await.is_some() {
                active.push(id);
            }
        }
        active.sort_unstable();
        active
    }
}

async fn run_bridge_listener(
    listener: TcpListener,
    project_id: i64,
    token: String,
    preview_contexts: Arc<PreviewContextStore>,
    mut shutdown_rx: oneshot::Receiver<()>,
) {
    let connection_slots = Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_CONNECTIONS));
    loop {
        tokio::select! {
            _ = &mut shutdown_rx => break,
            accepted = listener.accept() => {
                let Ok((stream, _)) = accepted else { continue };
                let Ok(permit) = connection_slots.clone().try_acquire_owned() else {
                    tracing::debug!(project_id, "ide_bridge 并发连接数已达上限,丢弃这次连接");
                    continue;
                };
                let token = token.clone();
                let preview_contexts = preview_contexts.clone();
                tokio::spawn(async move {
                    handle_bridge_connection(stream, project_id, token, preview_contexts).await;
                    drop(permit);
                });
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
    //
    // 回调在握手期间被同步调用且只调用一次,读到的值在紧接着的 `.await`
    // 之后立刻被读取,整个过程只有这一个任务在用,不存在跨线程共享,一个
    // 被闭包借用的栈上局部变量就够了——不需要 `Arc<Mutex<..>>` 这类
    // 引用计数/可能中毒的内部可变性。
    let mut presented: Option<String> = None;
    // `accept_hdr_async` 的握手回调签名由 tungstenite 规定,返回的
    // `Result` 错误分支是它自己的巨型 `Response` 类型,我们无法改成
    // `Box<...>`(会与 trait 约定不符),只能就地放行这一条 lint。
    #[allow(clippy::result_large_err)]
    let capture =
        |req: &tokio_tungstenite::tungstenite::handshake::server::Request,
         resp: tokio_tungstenite::tungstenite::handshake::server::Response| {
            presented = req
                .headers()
                .get("x-claude-code-ide-authorization")
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned);
            Ok(resp)
        };
    let handshake = tokio::time::timeout(
        HANDSHAKE_TIMEOUT,
        tokio_tungstenite::accept_hdr_async(stream, capture),
    )
    .await;
    let Ok(Ok(ws)) = handshake else {
        return;
    };
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
        assert!(registry.session_started(1, "/repo").await);
        assert_eq!(registry.active_projects().await, vec![1]);
        assert!(registry.session_started(1, "/repo").await);
        assert_eq!(registry.active_projects().await, vec![1]);
    }

    #[tokio::test]
    async fn different_projects_get_independent_bridges() {
        let (registry, _dir) = new_registry();
        assert!(registry.session_started(1, "/repo-a").await);
        assert!(registry.session_started(2, "/repo-b").await);
        assert_eq!(registry.active_projects().await, vec![1, 2]);
    }

    #[tokio::test]
    async fn session_started_returns_false_and_records_nothing_when_lock_dir_is_unusable() {
        let dir = tempfile::tempdir().unwrap();
        // `lock_dir` 指向一个已经存在的普通文件而不是目录,
        // `write_lock_file` 内部的 `create_dir_all` 必然失败。
        let blocked_lock_dir = dir.path().join("blocked");
        std::fs::write(&blocked_lock_dir, b"not a directory").unwrap();
        let registry =
            IdeBridgeRegistry::new(blocked_lock_dir, Arc::new(PreviewContextStore::new()));

        assert!(!registry.session_started(1, "/repo").await);
        assert!(registry.active_projects().await.is_empty());
    }

    #[tokio::test]
    async fn session_started_can_retry_after_a_previous_failure_for_the_same_project() {
        let dir = tempfile::tempdir().unwrap();
        let lock_dir = dir.path().join("ide-locks");
        std::fs::write(&lock_dir, b"not a directory yet").unwrap();
        let registry =
            IdeBridgeRegistry::new(lock_dir.clone(), Arc::new(PreviewContextStore::new()));

        assert!(!registry.session_started(1, "/repo").await);
        assert!(registry.active_projects().await.is_empty());

        // 环境问题后来自己好了(比如磁盘空间恢复/权限修好):同一个项目
        // 后续的 `session_started` 应该能重新尝试并成功,不会被上一次的
        // 失败卡死。
        std::fs::remove_file(&lock_dir).unwrap();
        assert!(registry.session_started(1, "/repo").await);
        assert_eq!(registry.active_projects().await, vec![1]);
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

    #[tokio::test]
    async fn connections_beyond_the_cap_are_dropped_before_handshake_completes() {
        let (_registry, _dir, port, token) = started_registry_with_port(45).await;
        let connect = || {
            let token = token.clone();
            async move {
                let mut request = format!("ws://127.0.0.1:{port}/")
                    .into_client_request()
                    .unwrap();
                request.headers_mut().insert(
                    "x-claude-code-ide-authorization",
                    HeaderValue::from_str(&token).unwrap(),
                );
                tokio_tungstenite::connect_async(request).await
            }
        };

        // 占满并发连接上限,全部保持打开(不 drop,占着许可不放)。
        let mut held = Vec::new();
        for _ in 0..MAX_CONCURRENT_CONNECTIONS {
            held.push(connect().await.expect("前 N 条应该正常握手成功"));
        }

        // 第 N+1 条:服务端并发连接数已到上限,accept 之后直接丢弃底层
        // TCP 连接、不进行 WS 握手,客户端这次连接应该失败或超时,不会
        // 拿到一个正常的 WS 会话。
        let extra = tokio::time::timeout(std::time::Duration::from_millis(500), connect()).await;
        if let Ok(Ok(_)) = extra {
            panic!("超过并发上限的连接不应该握手成功");
        }

        drop(held); // 显式释放:必须在上面的断言跑完之前一直保持连接存活
    }
}

pub(crate) fn sweep_stale_locks(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
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
