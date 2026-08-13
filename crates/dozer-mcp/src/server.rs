//! `get_preview_context` MCP tool 的实现。会话→项目的解析每次 tool call
//! 现连 `dozerd`(daemon 可能重启,本进程是长驻的,不维护长连接;见设计
//! 文档"session → project 解析"一节)。

use dozer_client::Client;
use dozer_core::protocol::PreviewContext;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::{ErrorData as McpError, ServiceExt, tool, tool_router, transport::stdio};
use serde_json::json;

#[derive(Clone)]
pub struct DozerMcpServer {
    client: Client,
    session_id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct NoParams {}

#[tool_router(server_handler)]
impl DozerMcpServer {
    pub fn new(client: Client, session_id: String) -> Self {
        Self { client, session_id }
    }

    #[tool(
        description = "返回用户当前在 Dozer 预览面板里看的文件路径和光标/选中范围;想知道用户正在看哪段代码时调用。"
    )]
    /// `pub` 是为了让 `tests/*.rs`(外部 crate)能直接调真正的 tool 方法、
    /// 断言它拼出来的 JSON 形状,而不是只测绕开 JSON 的
    /// `fetch_context_for_test`。
    pub async fn get_preview_context(
        &self,
        Parameters(NoParams {}): Parameters<NoParams>,
    ) -> Result<CallToolResult, McpError> {
        match self.fetch_context().await {
            Ok(context) => {
                let value = match context {
                    Some(ctx) => json!({
                        "path": ctx.path,
                        "start_line": ctx.start_line,
                        "start_col": ctx.start_col,
                        "end_line": ctx.end_line,
                        "end_col": ctx.end_col,
                        "has_selection": ctx.has_selection,
                        // 新鲜度:这份上下文是什么时候推上来的(Unix 毫秒)。
                        // dozerd 可能比 GUI 活得久,调用方得能自己判断陈旧。
                        "updated_at_ms": ctx.updated_at_ms,
                        "reason": null,
                    }),
                    None => json!({
                        "path": null,
                        "start_line": null,
                        "start_col": null,
                        "end_line": null,
                        "end_col": null,
                        "has_selection": null,
                        "updated_at_ms": null,
                        "reason": "no_active_preview",
                    }),
                };
                Ok(CallToolResult::structured(value))
            }
            Err(e) => Err(McpError::internal_error(format!("{e}"), None)),
        }
    }
}

impl DozerMcpServer {
    /// 会话/项目解析 + 查询预览上下文的裸逻辑,不依赖任何 MCP 类型,供
    /// `#[tool]` 方法体和测试共用,避免同一段逻辑写两份。
    async fn fetch_context(&self) -> anyhow::Result<Option<PreviewContext>> {
        let sessions = self
            .client
            .list()
            .await
            .map_err(|e| anyhow::anyhow!("连接 dozerd 失败: {e}"))?;
        let session = sessions
            .iter()
            .find(|s| s.id == self.session_id)
            .ok_or_else(|| anyhow::anyhow!("会话不存在于 dozerd"))?;
        let project_id = session
            .project_id
            .ok_or_else(|| anyhow::anyhow!("会话尚未归属任何项目"))?;
        self.client
            .get_preview_context(project_id)
            .await
            .map_err(|e| anyhow::anyhow!("查询预览上下文失败: {e}"))
    }

    /// 测试专用入口：绕开 MCP `Parameters`/`CallToolResult` 包装，直接跑
    /// 会话解析 + 查询逻辑。`#[cfg(test)]` 在这里不适用——`tests/*.rs`
    /// 作为外部 crate 链接 lib target 时不会带 `cfg(test)`，加了反而会
    /// 让集成测试看不到这个方法，所以保持普通 `pub`。
    pub async fn fetch_context_for_test(&self) -> anyhow::Result<Option<PreviewContext>> {
        self.fetch_context().await
    }
}

pub async fn run() -> anyhow::Result<()> {
    let session_id = std::env::var("DOZER_SESSION_ID").map_err(|_| {
        anyhow::anyhow!("缺少 DOZER_SESSION_ID：dozer-mcp 只能在 dozer 拉起的会话里跑")
    })?;
    let client = Client::new(dozer_core::paths::socket_path());
    let server = DozerMcpServer::new(client, session_id);
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
