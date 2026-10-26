//! `get_preview_context` MCP tool 的实现。会话→项目的解析每次 tool call
//! 现连 `dozerd`(daemon 可能重启,本进程是长驻的,不维护长连接;见设计
//! 文档"session → project 解析"一节)。

use crate::features::McpFeatures;
use dozer_client::Client;
use dozer_core::protocol::{
    PreviewCommand, PreviewCommandAction, PreviewCommandTarget, PreviewContext,
};
use rmcp::handler::server::ServerHandler;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::{ErrorData as McpError, ServiceExt, tool, tool_handler, tool_router, transport::stdio};
use serde_json::json;

#[derive(Clone)]
pub struct DozerMcpServer {
    client: Client,
    session_id: Option<String>,
    #[allow(dead_code)]
    features: McpFeatures,
    tool_router: ToolRouter<Self>,
    server_name: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct NoParams {}

const SUMMARY_TITLE_MAX_CHARS: usize = 200;
const SUMMARY_TEXT_MAX_CHARS: usize = 200;

fn truncate_chars(s: String, max: usize) -> String {
    if s.chars().count() <= max {
        s
    } else {
        s.chars().take(max).collect()
    }
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SubmitSessionSummaryParams {
    pub title: String,
    pub summary: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AddTodoParams {
    pub text: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ToggleTodoParams {
    pub id: i64,
    pub done: bool,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct EditTodoTextParams {
    pub id: i64,
    pub text: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct WriteMemoryParams {
    pub title: String,
    pub body: String,
    pub kind: String,
    pub description: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GetMemoryParams {
    pub title_or_id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct LocateInFileParams {
    pub path: String,
    pub query: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ApplyPreciseEditParams {
    pub path: String,
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
    pub expected_text: String,
    pub new_text: String,
    pub summary: String,
}

/// T13:`preview_navigate` 参数。给 `path` + 起止(只给 line/column 是 reveal,
/// 再给 end_line/end_column 就是 select,均 1-based)。
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct PreviewNavigateParams {
    pub path: String,
    pub line: u32,
    pub column: u32,
    #[serde(default)]
    pub end_line: Option<u32>,
    #[serde(default)]
    pub end_column: Option<u32>,
}

#[tool_router(vis = "pub")]
impl DozerMcpServer {
    /// 兼容旧调用点:全部工具、默认服务器名(`rmcp`,与抽库前 initialize
    /// 应答逐字一致)。
    pub fn new(client: Client, session_id: String) -> Self {
        Self::with_options(client, Some(session_id), McpFeatures::all(), "rmcp".into())
    }

    /// 按特性裁剪路由;`server_name` 只影响 `initialize` 的 serverInfo.name。
    pub fn with_options(
        client: Client,
        session_id: Option<String>,
        features: McpFeatures,
        server_name: String,
    ) -> Self {
        let mut tool_router = Self::tool_router();
        let enabled: std::collections::HashSet<&str> = features.tool_names().into_iter().collect();
        for tool in Self::tool_router().list_all() {
            if !enabled.contains(tool.name.as_ref()) {
                tool_router.remove_route(tool.name.as_ref());
            }
        }
        Self {
            client,
            session_id,
            features,
            tool_router,
            server_name,
        }
    }

    /// 测试/进程内查询:给定特性下是否注册了某个工具。
    pub fn has_tool(features: McpFeatures, name: &str) -> bool {
        let server = Self::with_options(
            Client::new(std::path::PathBuf::from("/nonexistent/dozerd.sock")),
            None,
            features,
            "rmcp".into(),
        );
        server.tool_router.get(name).is_some()
    }

    fn session_id(&self) -> Result<&str, McpError> {
        self.session_id.as_deref().ok_or_else(|| {
            McpError::internal_error("当前会话没有 session id(安静模式不应调用工具)", None)
        })
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
                        // Phase B 扩展:revision/mode/只读/选区文本(有上限)与
                        // 可见行范围;后端不提供时为 null。
                        "revision": ctx.revision,
                        "mode": ctx.mode,
                        "read_only": ctx.read_only,
                        "selected_text": ctx.selected_text,
                        "visible_start_line": ctx.visible_start_line,
                        "visible_end_line": ctx.visible_end_line,
                        "tabular": ctx.tabular,
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
                        "revision": null,
                        "mode": null,
                        "read_only": null,
                        "selected_text": null,
                        "visible_start_line": null,
                        "visible_end_line": null,
                        "tabular": null,
                        "reason": "no_active_preview",
                    }),
                };
                Ok(CallToolResult::structured(value))
            }
            Err(e) => Err(McpError::internal_error(format!("{e}"), None)),
        }
    }

    #[tool(
        description = "让 Dozer 预览滚动/选中到指定文件的某段代码(只读导航,不写入)。line/column 为 1-based;再给 end_line/end_column 则选中范围。"
    )]
    pub async fn preview_navigate(
        &self,
        Parameters(p): Parameters<PreviewNavigateParams>,
    ) -> Result<CallToolResult, McpError> {
        let (project_id, _agent) = self
            .resolve_project_and_agent()
            .await
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        let action = match (p.end_line, p.end_column) {
            (Some(end_line), Some(end_column)) => PreviewCommandAction::Select {
                start_line: p.line,
                start_column: p.column,
                end_line,
                end_column,
            },
            _ => PreviewCommandAction::Reveal {
                line: p.line,
                column: p.column,
            },
        };
        let request_id = format!(
            "nav-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0)
        );
        let command = PreviewCommand {
            request_id,
            project_id,
            target: PreviewCommandTarget::Path { path: p.path },
            action,
            expected_revision: None,
        };
        let outcome = self
            .client
            .run_preview_command(command)
            .await
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        let value = serde_json::to_value(&outcome)
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        Ok(CallToolResult::structured(value))
    }

    #[tool(
        description = "提交本次会话的总结:一个简短标题和不超过 200 字的摘要；涉及多个事件时使用编号分项列出，语言保持简练明确。仅在被要求总结当前会话时调用一次,不要在其他场景主动调用。"
    )]
    pub async fn submit_session_summary(
        &self,
        Parameters(SubmitSessionSummaryParams { title, summary }): Parameters<
            SubmitSessionSummaryParams,
        >,
    ) -> Result<CallToolResult, McpError> {
        let title = truncate_chars(title, SUMMARY_TITLE_MAX_CHARS);
        let summary = truncate_chars(summary, SUMMARY_TEXT_MAX_CHARS);
        let session_id = self.session_id()?;
        self.client
            .record_session_summary(session_id, &title, &summary)
            .await
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        Ok(CallToolResult::structured(json!({ "recorded": true })))
    }

    #[tool(
        description = "列出当前项目的任务列表(id/文字/是否完成)。想知道当前有哪些待办任务时调用。"
    )]
    pub async fn list_todos(
        &self,
        Parameters(NoParams {}): Parameters<NoParams>,
    ) -> Result<CallToolResult, McpError> {
        let (project_id, _agent) = self
            .resolve_project_and_agent()
            .await
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        let todos = self
            .client
            .list_todos(project_id)
            .await
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        let value = json!(
            todos
                .into_iter()
                .map(|t| json!({ "id": t.id, "text": t.text, "done": t.done }))
                .collect::<Vec<_>>()
        );
        Ok(CallToolResult::structured(json!({ "todos": value })))
    }

    #[tool(description = "新增一条任务,置顶到列表最前。")]
    pub async fn add_todo(
        &self,
        Parameters(AddTodoParams { text }): Parameters<AddTodoParams>,
    ) -> Result<CallToolResult, McpError> {
        require_text(&text)?;
        let (project_id, _agent) = self
            .resolve_project_and_agent()
            .await
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        let todo = self
            .client
            .add_todo(project_id, &text)
            .await
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        Ok(CallToolResult::structured(
            json!({ "id": todo.id, "text": todo.text, "done": todo.done }),
        ))
    }

    #[tool(description = "勾选或取消勾选一条任务的完成状态。")]
    pub async fn toggle_todo(
        &self,
        Parameters(ToggleTodoParams { id, done }): Parameters<ToggleTodoParams>,
    ) -> Result<CallToolResult, McpError> {
        self.require_own_todo(id).await?;
        let todo = self
            .client
            .toggle_todo(id, done)
            .await
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        Ok(CallToolResult::structured(
            json!({ "id": todo.id, "text": todo.text, "done": todo.done }),
        ))
    }

    #[tool(description = "修改一条任务的文字内容。")]
    pub async fn edit_todo_text(
        &self,
        Parameters(EditTodoTextParams { id, text }): Parameters<EditTodoTextParams>,
    ) -> Result<CallToolResult, McpError> {
        require_text(&text)?;
        self.require_own_todo(id).await?;
        let todo = self
            .client
            .edit_todo_text(id, &text)
            .await
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        Ok(CallToolResult::structured(
            json!({ "id": todo.id, "text": todo.text, "done": todo.done }),
        ))
    }

    #[tool(
        description = "记忆优先读写这里,不要用你自己本地的记忆机制。按标题在项目内 upsert:标题已存在就更新,不存在就新建。写入前建议先调 list_memories 看看有没有同名条目可以更新,避免重复记忆。kind 建议用 user/feedback/project/reference 之一,但不强制。"
    )]
    pub async fn write_memory(
        &self,
        Parameters(WriteMemoryParams {
            title,
            body,
            kind,
            description,
        }): Parameters<WriteMemoryParams>,
    ) -> Result<CallToolResult, McpError> {
        let (project_id, agent) = self
            .resolve_project_and_agent()
            .await
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        let detail = self
            .client
            .write_memory(
                project_id,
                &title,
                &kind,
                &description,
                &body,
                agent.label(),
            )
            .await
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        Ok(CallToolResult::structured(json!({
            "id": detail.id,
            "title": detail.title,
            "kind": detail.kind,
            "description": detail.description,
            "updated_by": detail.updated_by,
        })))
    }

    #[tool(
        description = "列出当前项目的共享记忆(标题/分类/摘要/最后更新方,不含正文)。读记忆前先调这个,别猜有没有同名条目。"
    )]
    pub async fn list_memories(
        &self,
        Parameters(NoParams {}): Parameters<NoParams>,
    ) -> Result<CallToolResult, McpError> {
        let (project_id, _agent) = self
            .resolve_project_and_agent()
            .await
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        let memories = self
            .client
            .list_memories(project_id)
            .await
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        let value = json!(
            memories
                .into_iter()
                .map(|m| json!({
                    "id": m.id,
                    "title": m.title,
                    "kind": m.kind,
                    "description": m.description,
                    "updated_by": m.updated_by,
                }))
                .collect::<Vec<_>>()
        );
        Ok(CallToolResult::structured(json!({ "memories": value })))
    }

    #[tool(
        description = "查一条共享记忆的完整正文。title_or_id 可以传标题(和 list_memories 里看到的一致)或数字 id;传标题时会先内部查一遍 list_memories 做匹配。"
    )]
    pub async fn get_memory(
        &self,
        Parameters(GetMemoryParams { title_or_id }): Parameters<GetMemoryParams>,
    ) -> Result<CallToolResult, McpError> {
        let (project_id, _agent) = self
            .resolve_project_and_agent()
            .await
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        let id = if let Ok(id) = title_or_id.parse::<i64>() {
            id
        } else {
            let memories = self
                .client
                .list_memories(project_id)
                .await
                .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
            memories
                .into_iter()
                .find(|m| m.title == title_or_id)
                .ok_or_else(|| {
                    McpError::internal_error(format!("未找到标题为 {title_or_id} 的记忆"), None)
                })?
                .id
        };
        let detail = self
            .client
            .get_memory(project_id, id)
            .await
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        Ok(CallToolResult::structured(json!({
            "id": detail.id,
            "title": detail.title,
            "kind": detail.kind,
            "description": detail.description,
            "body": detail.body,
            "updated_by": detail.updated_by,
            "history": detail.history.iter().map(|h| json!({
                "changed_ms": h.changed_ms,
                "changed_by": h.changed_by,
                "change_kind": h.change_kind,
            })).collect::<Vec<_>>(),
        })))
    }

    #[tool(
        description = "在项目内某个文本文件里搜索一段文字,返回精确坐标(1-based 行列)。唯一匹配才算定位成功;多处匹配会把候选全部列出,重新传更长/更具体的 query 缩小范围。调用 apply_precise_edit 前应该先用这个工具拿到准确坐标,不要自己数行号。"
    )]
    pub async fn locate_in_file(
        &self,
        Parameters(LocateInFileParams { path, query }): Parameters<LocateInFileParams>,
    ) -> Result<CallToolResult, McpError> {
        let (project_id, _agent) = self
            .resolve_project_and_agent()
            .await
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        let matches = self
            .client
            .locate_in_file(project_id, &path, &query)
            .await
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        let value = json!(
            matches
                .into_iter()
                .map(|m| json!({
                    "start_line": m.start_line,
                    "start_col": m.start_col,
                    "end_line": m.end_line,
                    "end_col": m.end_col,
                    "context": m.context,
                }))
                .collect::<Vec<_>>()
        );
        Ok(CallToolResult::structured(json!({ "matches": value })))
    }

    #[tool(
        description = "精确替换项目内某文本文件 [start_line,start_col]~[end_line,end_col] 区间(1-based,含端点)的内容。expected_text 必须是这段区间当前的原样内容(用 locate_in_file 拿到坐标后紧跟着读到的那段文字),不一致会返回 conflict 并附带磁盘上的真实内容,不会写入;整篇重写就把区间设成整个文件。summary 必填,一句话说明这次改了什么。"
    )]
    pub async fn apply_precise_edit(
        &self,
        Parameters(ApplyPreciseEditParams {
            path,
            start_line,
            start_col,
            end_line,
            end_col,
            expected_text,
            new_text,
            summary,
        }): Parameters<ApplyPreciseEditParams>,
    ) -> Result<CallToolResult, McpError> {
        let (project_id, agent) = self
            .resolve_project_and_agent()
            .await
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        let outcome = self
            .client
            .apply_precise_edit(
                project_id,
                &path,
                start_line,
                start_col,
                end_line,
                end_col,
                &expected_text,
                &new_text,
                &summary,
                agent.label(),
                self.session_id()?,
            )
            .await
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        let value = match outcome {
            dozer_core::protocol::MutationOutcome::Applied {
                new_start_line,
                new_start_col,
                new_end_line,
                new_end_col,
                history_id,
            } => json!({
                "kind": "applied",
                "new_start_line": new_start_line,
                "new_start_col": new_start_col,
                "new_end_line": new_end_line,
                "new_end_col": new_end_col,
                "history_id": history_id,
            }),
            dozer_core::protocol::MutationOutcome::Conflict { actual_text } => json!({
                "kind": "conflict",
                "actual_text": actual_text,
            }),
            dozer_core::protocol::MutationOutcome::NotFound => json!({ "kind": "not_found" }),
            dozer_core::protocol::MutationOutcome::PathOutOfBounds => {
                json!({ "kind": "path_out_of_bounds" })
            }
            dozer_core::protocol::MutationOutcome::Unwritable { reason } => json!({
                "kind": "unwritable",
                "reason": reason,
            }),
        };
        Ok(CallToolResult::structured(value))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for DozerMcpServer {
    fn get_info(&self) -> rmcp::model::ServerConfig {
        let mut info = rmcp::model::ServerConfig::new(
            rmcp::model::ServerCapabilities::builder()
                .enable_tools()
                .build(),
        );
        info.server_info.name = self.server_name.clone();
        info
    }
}

impl DozerMcpServer {
    /// 会话/项目解析 + 查询预览上下文的裸逻辑,不依赖任何 MCP 类型,供
    /// `#[tool]` 方法体和测试共用,避免同一段逻辑写两份。
    async fn fetch_context(&self) -> anyhow::Result<Option<PreviewContext>> {
        let session_id = self
            .session_id
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("当前会话没有 session id"))?;
        let sessions = self
            .client
            .list()
            .await
            .map_err(|e| anyhow::anyhow!("连接 dozerd 失败: {e}"))?;
        let session = sessions
            .iter()
            .find(|s| s.id == session_id)
            .ok_or_else(|| anyhow::anyhow!("会话不存在于 dozerd"))?;
        let project_id = session
            .project_id
            .ok_or_else(|| anyhow::anyhow!("会话尚未归属任何项目"))?;
        self.client
            .get_preview_context(project_id)
            .await
            .map_err(|e| anyhow::anyhow!("查询预览上下文失败: {e}"))
    }

    /// 任务必须属于本会话所在的项目:agent 只能改自己项目的待办,
    /// 用别的项目(或不存在)的 id 一律拒绝,什么都不写。
    async fn require_own_todo(&self, id: i64) -> Result<(), McpError> {
        let (project_id, _agent) = self
            .resolve_project_and_agent()
            .await
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        let own = self
            .client
            .list_todos(project_id)
            .await
            .map_err(|e| McpError::internal_error(format!("{e}"), None))?;
        if own.iter().any(|t| t.id == id) {
            Ok(())
        } else {
            Err(McpError::invalid_params(
                format!("任务 {id} 不属于当前项目(或不存在);先用 list_todos 查看本项目的任务 id"),
                None,
            ))
        }
    }

    /// session_id → (project_id, agent) 的解析逻辑。`agent` 用于
    /// `write_memory` 的 `actor` 归属;`list_todos`/`add_todo` 等既有
    /// 调用方不需要 `agent`,解构时用 `_` 丢弃即可。
    async fn resolve_project_and_agent(
        &self,
    ) -> anyhow::Result<(i64, dozer_core::protocol::AgentKind)> {
        let session_id = self
            .session_id
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("当前会话没有 session id"))?;
        let sessions = self
            .client
            .list()
            .await
            .map_err(|e| anyhow::anyhow!("连接 dozerd 失败: {e}"))?;
        let session = sessions
            .iter()
            .find(|s| s.id == session_id)
            .ok_or_else(|| anyhow::anyhow!("会话不存在于 dozerd"))?;
        let project_id = session
            .project_id
            .ok_or_else(|| anyhow::anyhow!("会话尚未归属任何项目"))?;
        Ok((project_id, session.agent))
    }

    /// 测试专用入口：绕开 MCP `Parameters`/`CallToolResult` 包装，直接跑
    /// 会话解析 + 查询逻辑。`#[cfg(test)]` 在这里不适用——`tests/*.rs`
    /// 作为外部 crate 链接 lib target 时不会带 `cfg(test)`，加了反而会
    /// 让集成测试看不到这个方法，所以保持普通 `pub`。
    pub async fn fetch_context_for_test(&self) -> anyhow::Result<Option<PreviewContext>> {
        self.fetch_context().await
    }
}

/// 任务文字不能为空,也不能只有空白;有内容的文字原样保存,不替 agent 改写。
fn require_text(text: &str) -> Result<(), McpError> {
    if text.trim().is_empty() {
        Err(McpError::invalid_params("任务文字不能为空", None))
    } else {
        Ok(())
    }
}

/// 按产品运行 `serve`。`session_id` 缺失且存在别的已知产品的会话变量时
/// 进入安静模式:零工具、不写 stderr、不连接守护进程。
pub async fn run_for(
    product: &dozer_core::paths::ProductId,
    features: McpFeatures,
) -> anyhow::Result<()> {
    let own = std::env::var(product.session_env)
        .ok()
        .filter(|v| !v.is_empty());
    let (session_id, features) = match own {
        Some(id) => (Some(id), features),
        None => {
            let other =
                dozer_core::paths::other_known_session_env_present(product.session_env, &|name| {
                    std::env::var(name).ok()
                });
            if other {
                (None, McpFeatures::none())
            } else {
                return Err(anyhow::anyhow!(missing_session_message(product)));
            }
        }
    };
    let server_name = if product.name == "dozer" {
        // 与抽库前的 `initialize` 应答逐字一致。
        "rmcp".to_string()
    } else {
        product.mcp_bin()
    };
    let client = Client::new(product.socket_path());
    let server = DozerMcpServer::with_options(client, session_id, features, server_name);
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}

/// 缺会话变量时的报错文案(默认产品逐字保留旧文案)。
pub fn missing_session_message(product: &dozer_core::paths::ProductId) -> String {
    format!(
        "缺少 {}：{} 只能在 {} 拉起的会话里跑",
        product.session_env,
        product.mcp_bin(),
        product.name
    )
}

/// 产品入口:子命令与产品/特性注入(替代写死的 `main`)。
pub fn run_main(product: dozer_core::paths::ProductId, features: McpFeatures) -> ! {
    if let Err(e) = dozer_core::paths::install(product.clone()) {
        eprintln!("{e}");
        std::process::exit(1);
    }
    let arg1 = std::env::args().nth(1);
    match arg1.as_deref() {
        Some("serve") => {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("build tokio runtime");
            if let Err(e) = rt.block_on(run_for(&product, features)) {
                eprintln!("{e}");
                std::process::exit(1);
            }
            std::process::exit(0);
        }
        Some("install") => {
            let agent = std::env::args().nth(2).unwrap_or_else(|| "claude".into());
            std::process::exit(crate::install::run_for(&product, &agent, true));
        }
        Some("uninstall") => {
            let agent = std::env::args().nth(2).unwrap_or_else(|| "claude".into());
            std::process::exit(crate::install::run_for(&product, &agent, false));
        }
        _ => {
            eprintln!(
                "用法: {} <serve|install|uninstall> [agent]",
                product.mcp_bin()
            );
            std::process::exit(2);
        }
    }
}
