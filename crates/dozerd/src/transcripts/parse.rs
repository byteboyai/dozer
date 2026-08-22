//! agent transcript(JSONL)增量解析:按 `AgentKind` 分派,产出既带展示
//! 字段又带用量字段的 `ParsedTurn`(合并原 dozer-app `transcript.rs` +
//! `usage.rs` 两套平行解析器,避免长期重复维护——spec"参考调研"一节)。

use dozer_core::protocol::{AgentKind, ToolCallInfo};
use serde_json::Value;

pub const MUTATING_TOOLS: [&str; 4] = ["Edit", "Write", "MultiEdit", "NotebookEdit"];

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ParsedTurn {
    pub message_key: String,
    pub role: String,
    pub content: String,
    pub tools_summary: Vec<String>,
    pub thinking: bool,
    pub tool_calls: u32,
    pub mutating_tool_calls: u32,
    pub files_touched: Vec<String>,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub tokens_cache_read: u64,
    pub tokens_cache_write: u64,
    pub ts: Option<u64>,
    pub raw_json: String,
    /// 只对 `role == "tool_result"` 有意义:这次工具调用是否失败。
    pub is_error: bool,
}

/// `text` 中"只含完整行"的前缀长度——最后一个 `'\n'` 之后的偏移;没有
/// 换行符(整段都是未写完的半行)时返回 0,即"这次什么都不消费"。
pub fn last_complete_line_boundary(text: &str) -> usize {
    text.rfind('\n').map(|i| i + 1).unwrap_or(0)
}

fn fallback_key(conversation_id: &str, turn_index: i64) -> String {
    format!("{conversation_id}:{turn_index}")
}

/// CLI 自己往 transcript 里注入的合成"人类消息"(斜杠命令回显、本地
/// 命令的标准输出回显、local-command-caveat/system-reminder 包裹块)——
/// 不是用户真正打的字,不该被当成一个对话回合摄取:既污染逐回合审阅
/// 列表,也污染"取第一条人类消息前 80 字当标题"这条推导(验收反馈
/// 截图:会话列表标题全是 `<local-command-caveat>Caveat: ...`/
/// `<system-reminder ...>`/`<local-command-stdout>Switch model to ...`)。
/// Claude 侧这类消息通常带 `isMeta: true`(调用方另行判断),但
/// CodeBuddy 侧同款包裹块直接以纯文本出现在 `content` 里、没有对应
/// 的结构化标记,只能认前缀——两边共用同一份前缀表,不重复维护。
fn is_synthetic_wrapper_content(content: &str) -> bool {
    let trimmed = content.trim_start();
    trimmed.starts_with("<local-command-caveat")
        || trimmed.starts_with("<local-command-stdout")
        || trimmed.starts_with("<command-name")
        || trimmed.starts_with("<system-reminder")
        || trimmed.starts_with("<task-notification")
}

/// 真实用户敲的斜杠命令(`/clear`/`/model xxx`/`/compact` 等，Claude
/// Code/CodeBuddy/OpenCode 三家 CLI 通用的命令语法)——是真实用户行为，
/// 内容要保留在数据库里，只是不该单独成为一个"回合分组"的锚点(用户
/// 验收反馈:命令类回合不该在会话列表里单独占一行)。跟
/// `is_synthetic_wrapper_content` 是两回事:那个是 CLI 自己注入的合成
/// 消息，从 ingestion 阶段就整体丢弃；这个是真实用户输入，只影响分组
/// 查询(Task 9)怎么切边界，不影响是否落库。
///
/// 判定:trim 后以 `/` 开头，且紧跟着至少一个字母/数字(排除"光一个
/// `/`"和"以 `/` 开头的文件路径讨论"这种误判——后者虽然堵不住所有
/// case，但"整条消息以 /word 开头"这个模式已经覆盖了三家 CLI 实测
/// 见过的全部命令形态)。
///
/// 目前在生产代码里没有直接调用方——回合分组查询(Task 9)用的是 SQL
/// `NOT GLOB '/[A-Za-z0-9]*'` 等价近似,而 SQL 跑不了 Rust 函数。这个
/// 函数作为"什么叫斜杠命令"的单一权威实现保留,由单测锁定行为,SQL
/// 侧按同样的"以 / + 字母数字开头"语义分开维护。
#[allow(dead_code)]
pub(crate) fn is_command_content(content: &str) -> bool {
    let trimmed = content.trim();
    let Some(rest) = trimmed.strip_prefix('/') else {
        return false;
    };
    rest.chars().next().is_some_and(|c| c.is_alphanumeric())
}

fn tool_summary(name: &str, input: &Value) -> String {
    let arg = input
        .get("file_path")
        .and_then(|v| v.as_str())
        .map(|p| {
            std::path::Path::new(p)
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.to_string())
        })
        .or_else(|| {
            input
                .get("command")
                .and_then(|v| v.as_str())
                .map(|c| c.chars().take(40).collect())
        })
        .or_else(|| {
            input
                .get("pattern")
                .or_else(|| input.get("path"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        });
    match arg {
        Some(a) => format!("{name} {a}"),
        None => name.to_string(),
    }
}

/// 一个 `tool_result` 内容块(`b.get("content")`)的实际文本——可能是
/// 纯字符串，也可能是 `[{"type":"text","text":...}]` 数组(2026-08-21
/// 实测两种真实 Claude transcript 样本都存在)。
fn tool_result_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn parse_claude_shaped_chunk(
    text: &str,
    conversation_id: &str,
    starting_turn_index: i64,
) -> Vec<ParsedTurn> {
    let mut out = Vec::new();
    let mut turn_index = starting_turn_index;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let ts = v.get("timestamp").and_then(|t| t.as_u64());
        let message_key = v
            .get("uuid")
            .and_then(|u| u.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| fallback_key(conversation_id, turn_index));
        match v.get("type").and_then(|t| t.as_str()) {
            Some("user") => match v.get("message").and_then(|m| m.get("content")) {
                Some(Value::String(text)) => {
                    let is_meta = v.get("isMeta").and_then(|m| m.as_bool()).unwrap_or(false);
                    if is_meta || is_synthetic_wrapper_content(text) {
                        continue;
                    }
                    out.push(ParsedTurn {
                        message_key,
                        role: "human".into(),
                        content: text.to_string(),
                        ts,
                        raw_json: line.to_string(),
                        ..Default::default()
                    });
                    turn_index += 1;
                }
                Some(Value::Array(blocks)) => {
                    let mut content = String::new();
                    let mut is_error = false;
                    for b in blocks {
                        if b.get("type").and_then(|t| t.as_str()) != Some("tool_result") {
                            continue;
                        }
                        if b.get("is_error").and_then(|e| e.as_bool()).unwrap_or(false) {
                            is_error = true;
                        }
                        let text = tool_result_text(b.get("content"));
                        if text.is_empty() {
                            continue;
                        }
                        if !content.is_empty() {
                            content.push('\n');
                        }
                        content.push_str(&text);
                    }
                    if content.is_empty() {
                        continue;
                    }
                    out.push(ParsedTurn {
                        message_key,
                        role: "tool_result".into(),
                        content,
                        is_error,
                        ts,
                        raw_json: line.to_string(),
                        ..Default::default()
                    });
                    turn_index += 1;
                }
                _ => continue,
            },
            Some("assistant") => {
                let Some(blocks) = v
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_array())
                else {
                    continue;
                };
                let mut content = String::new();
                let mut tools_summary = Vec::new();
                let mut thinking = false;
                let mut tool_calls = 0u32;
                let mut mutating_tool_calls = 0u32;
                let mut files_touched = Vec::new();
                for b in blocks {
                    match b.get("type").and_then(|t| t.as_str()) {
                        Some("text") => {
                            if let Some(t) = b.get("text").and_then(|t| t.as_str()) {
                                if !content.is_empty() {
                                    content.push('\n');
                                }
                                content.push_str(t);
                            }
                        }
                        Some("tool_use") => {
                            let name = b.get("name").and_then(|n| n.as_str()).unwrap_or("工具");
                            let input = b.get("input").cloned().unwrap_or(Value::Null);
                            tools_summary.push(tool_summary(name, &input));
                            tool_calls += 1;
                            if MUTATING_TOOLS.contains(&name) {
                                mutating_tool_calls += 1;
                                if let Some(path) = input.get("file_path").and_then(|p| p.as_str())
                                {
                                    files_touched.push(path.to_string());
                                }
                            }
                        }
                        Some("thinking") => thinking = true,
                        _ => {}
                    }
                }
                let usage = v.get("message").and_then(|m| m.get("usage"));
                let tokens_in = usage
                    .and_then(|u| u.get("input_tokens"))
                    .and_then(|n| n.as_u64())
                    .unwrap_or(0);
                let tokens_out = usage
                    .and_then(|u| u.get("output_tokens"))
                    .and_then(|n| n.as_u64())
                    .unwrap_or(0);
                let tokens_cache_read = usage
                    .and_then(|u| u.get("cache_read_input_tokens"))
                    .and_then(|n| n.as_u64())
                    .unwrap_or(0);
                let tokens_cache_write = usage
                    .and_then(|u| u.get("cache_creation_input_tokens"))
                    .and_then(|n| n.as_u64())
                    .unwrap_or(0);
                out.push(ParsedTurn {
                    message_key,
                    role: "ai".into(),
                    content,
                    tools_summary,
                    thinking,
                    tool_calls,
                    mutating_tool_calls,
                    files_touched,
                    tokens_in,
                    tokens_out,
                    tokens_cache_read,
                    tokens_cache_write,
                    ts,
                    raw_json: line.to_string(),
                    is_error: false,
                });
                turn_index += 1;
            }
            _ => {}
        }
    }
    out
}

fn join_codebuddy_text_blocks(blocks: &[Value], kind: &str) -> String {
    let mut text = String::new();
    for b in blocks {
        if b.get("type").and_then(|t| t.as_str()) == Some(kind)
            && let Some(t) = b.get("text").and_then(|t| t.as_str())
        {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(t);
        }
    }
    text
}

fn parse_codebuddy_shaped_chunk(
    text: &str,
    conversation_id: &str,
    starting_turn_index: i64,
) -> Vec<ParsedTurn> {
    let mut out = Vec::new();
    let mut turn_index = starting_turn_index;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let ts = v.get("timestamp").and_then(|t| t.as_u64());
        let message_key = v
            .get("id")
            .and_then(|i| i.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| fallback_key(conversation_id, turn_index));
        match v.get("type").and_then(|t| t.as_str()) {
            Some("message") => {
                let Some(blocks) = v.get("content").and_then(|c| c.as_array()) else {
                    continue;
                };
                let usage = v.get("providerData").and_then(|p| p.get("usage"));
                let tokens_in = usage
                    .and_then(|u| u.get("inputTokens"))
                    .and_then(|n| n.as_u64())
                    .unwrap_or(0);
                let tokens_out = usage
                    .and_then(|u| u.get("outputTokens"))
                    .and_then(|n| n.as_u64())
                    .unwrap_or(0);
                match v.get("role").and_then(|r| r.as_str()) {
                    Some("user") => {
                        let content = join_codebuddy_text_blocks(blocks, "input_text");
                        if content.is_empty() || is_synthetic_wrapper_content(&content) {
                            continue;
                        }
                        out.push(ParsedTurn {
                            message_key,
                            role: "human".into(),
                            content,
                            ts,
                            tokens_in,
                            tokens_out,
                            raw_json: line.to_string(),
                            ..Default::default()
                        });
                        turn_index += 1;
                    }
                    Some("assistant") => {
                        out.push(ParsedTurn {
                            message_key,
                            role: "ai".into(),
                            content: join_codebuddy_text_blocks(blocks, "output_text"),
                            ts,
                            tokens_in,
                            tokens_out,
                            raw_json: line.to_string(),
                            ..Default::default()
                        });
                        turn_index += 1;
                    }
                    _ => {}
                }
            }
            // 工具调用本身——跟 "message" 平级的顶层类型，不是嵌在某条
            // message 的 content 数组里(2026-08-21 实测确认，此前完全
            // 没被摄取)。`arguments` 是 JSON 编码的字符串，需要先解析
            // 一层才能喂给通用的 `tool_summary()`。
            Some("function_call") => {
                let name = v.get("name").and_then(|n| n.as_str()).unwrap_or("工具");
                let input: Value = v
                    .get("arguments")
                    .and_then(|a| a.as_str())
                    .and_then(|s| serde_json::from_str(s).ok())
                    .unwrap_or(Value::Null);
                let mutating = MUTATING_TOOLS.contains(&name);
                let mut files_touched = Vec::new();
                if mutating && let Some(path) = input.get("file_path").and_then(|p| p.as_str()) {
                    files_touched.push(path.to_string());
                }
                out.push(ParsedTurn {
                    message_key,
                    role: "ai".into(),
                    tools_summary: vec![tool_summary(name, &input)],
                    tool_calls: 1,
                    mutating_tool_calls: if mutating { 1 } else { 0 },
                    files_touched,
                    ts,
                    raw_json: line.to_string(),
                    ..Default::default()
                });
                turn_index += 1;
            }
            // 工具调用的返回结果。没有观测到明确的错误信号字段(真实样本
            // status 只见过 "completed"/"incomplete"，output.type 恒为
            // "text")，`status == "incomplete"` 是启发式近似，不是精确
            // 信号——以后如果找到更可靠的错误字段，回来改这一行。
            Some("function_call_result") => {
                let content = v
                    .get("output")
                    .and_then(|o| o.get("text"))
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string();
                if content.is_empty() {
                    continue;
                }
                let is_error = v.get("status").and_then(|s| s.as_str()) == Some("incomplete");
                out.push(ParsedTurn {
                    message_key,
                    role: "tool_result".into(),
                    content,
                    is_error,
                    ts,
                    raw_json: line.to_string(),
                    ..Default::default()
                });
                turn_index += 1;
            }
            // CodeBuddy 的 thinking 等价物。跟 Claude 的 thinking block 同一
            // 口径:只留一个"有没有思考"的布尔标记，不落全文(既有既定行为，
            // 见 parse_claude_shaped_chunk 的 Some("thinking") => thinking =
            // true 分支，这里保持一致，不新开先例)。
            Some("reasoning") => {
                out.push(ParsedTurn {
                    message_key,
                    role: "ai".into(),
                    thinking: true,
                    ts,
                    raw_json: line.to_string(),
                    ..Default::default()
                });
                turn_index += 1;
            }
            // file-history-snapshot / ai-title / summary / 其它未知类型：
            // 跟既有行为一致，不摄取。
            _ => {}
        }
    }
    out
}

/// 按 agent 分派解析一段(必为完整行)transcript 文本。`starting_turn_index`
/// 是这段文本第一条产出的 `ParsedTurn` 应该编到的 `turn_index`(调用方从
/// `conversations`/`conversation_turns` 已有数据算出,续接编号,不重置)。
pub fn parse_chunk(
    agent: AgentKind,
    text: &str,
    conversation_id: &str,
    starting_turn_index: i64,
) -> Vec<ParsedTurn> {
    match agent {
        AgentKind::Claude | AgentKind::Opencode | AgentKind::Kilo | AgentKind::Unknown => {
            parse_claude_shaped_chunk(text, conversation_id, starting_turn_index)
        }
        AgentKind::Codebuddy => {
            parse_codebuddy_shaped_chunk(text, conversation_id, starting_turn_index)
        }
        AgentKind::Codex | AgentKind::V8agent => Vec::new(),
    }
}

/// 一个回合读时解析出的 trace 明细（思考文本 + 结构化工具调用），只在
/// `dozerd::transcripts::mod::get_conversation_turns` 查询期间对 `raw_json`
/// 现算，不在摄取时落库、不加新列——`raw_json` 本身已经全量持久化，读时
/// 解析一次的代价可忽略（用户打开审阅面板才触发，单个 session 几十行）。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TurnTraceDetail {
    pub thinking_text: Option<String>,
    pub tool_calls: Vec<ToolCallInfo>,
}

/// 从一行原始 JSONL(`raw_json`)按 `agent` 对应的形状提取真实思考文本和
/// 结构化工具调用参数——`parse_claude_shaped_chunk`/`parse_codebuddy_shaped_chunk`
/// 摄取时只留了布尔位/一行摘要，这里是独立的读时补全，**不复用/不修改**
/// 那两个摄取函数(摄取路径是热路径、已有测试覆盖，不承担这次改动的风险；
/// 两边对 `type` 字段的判断口径保持一致即可)。解析失败/形状不认识时返回
/// 全空的 `TurnTraceDetail`，不 panic。
pub fn extract_turn_trace_detail(raw_json: &str, agent: AgentKind) -> TurnTraceDetail {
    let Ok(v) = serde_json::from_str::<Value>(raw_json) else {
        return TurnTraceDetail::default();
    };
    match agent {
        AgentKind::Codebuddy => extract_codebuddy_trace_detail(&v),
        // Claude/Opencode/Kilo/Unknown 摄取时都走 parse_claude_shaped_chunk
        // (parse_chunk 的分派,parse.rs:456-457),读时解析沿用同一分派。
        AgentKind::Claude | AgentKind::Opencode | AgentKind::Kilo | AgentKind::Unknown => {
            extract_claude_trace_detail(&v)
        }
        // Codex/V8agent 目前完全不摄取(parse_chunk 分派到空 Vec,
        // parse.rs:462),没有 raw_json 可读。
        AgentKind::Codex | AgentKind::V8agent => TurnTraceDetail::default(),
    }
}

fn input_json_of(input: &Value) -> Option<String> {
    if input.is_null() {
        None
    } else {
        serde_json::to_string_pretty(input).ok()
    }
}

fn extract_claude_trace_detail(v: &Value) -> TurnTraceDetail {
    let mut thinking_text: Option<String> = None;
    let mut tool_calls = Vec::new();
    let Some(blocks) = v
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_array())
    else {
        return TurnTraceDetail::default();
    };
    for b in blocks {
        match b.get("type").and_then(|t| t.as_str()) {
            Some("thinking") => {
                if let Some(t) = b.get("thinking").and_then(|t| t.as_str()) {
                    match &mut thinking_text {
                        Some(existing) => {
                            existing.push('\n');
                            existing.push_str(t);
                        }
                        None => thinking_text = Some(t.to_string()),
                    }
                }
            }
            Some("tool_use") => {
                let name = b.get("name").and_then(|n| n.as_str()).unwrap_or("工具");
                let input = b.get("input").cloned().unwrap_or(Value::Null);
                tool_calls.push(ToolCallInfo {
                    summary: tool_summary(name, &input),
                    input_json: input_json_of(&input),
                });
            }
            _ => {}
        }
    }
    TurnTraceDetail {
        thinking_text,
        tool_calls,
    }
}

fn extract_codebuddy_trace_detail(v: &Value) -> TurnTraceDetail {
    match v.get("type").and_then(|t| t.as_str()) {
        Some("reasoning") => {
            let blocks = v
                .get("content")
                .and_then(|c| c.as_array())
                .cloned()
                .unwrap_or_default();
            let text = join_codebuddy_text_blocks(&blocks, "reasoning_text");
            TurnTraceDetail {
                thinking_text: if text.is_empty() { None } else { Some(text) },
                tool_calls: Vec::new(),
            }
        }
        Some("function_call") => {
            let name = v.get("name").and_then(|n| n.as_str()).unwrap_or("工具");
            let input: Value = v
                .get("arguments")
                .and_then(|a| a.as_str())
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or(Value::Null);
            TurnTraceDetail {
                thinking_text: None,
                tool_calls: vec![ToolCallInfo {
                    summary: tool_summary(name, &input),
                    input_json: input_json_of(&input),
                }],
            }
        }
        _ => TurnTraceDetail::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_human_and_ai_turn_with_usage_and_tools() {
        let text = concat!(
            "{\"type\":\"user\",\"uuid\":\"u1\",\"timestamp\":100,",
            "\"message\":{\"role\":\"user\",\"content\":\"改一下 README\"}}\n",
            "{\"type\":\"assistant\",\"uuid\":\"u2\",\"timestamp\":200,\"message\":{",
            "\"role\":\"assistant\",\"usage\":{\"input_tokens\":10,\"output_tokens\":20},",
            "\"content\":[{\"type\":\"thinking\",\"thinking\":\"...\"},",
            "{\"type\":\"text\",\"text\":\"好的，我来改。\"},",
            "{\"type\":\"tool_use\",\"name\":\"Edit\",\"input\":{\"file_path\":\"/r/README.md\"}}]}}\n"
        );
        let turns = parse_chunk(AgentKind::Claude, text, "conv1", 0);
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].role, "human");
        assert_eq!(turns[0].content, "改一下 README");
        assert_eq!(turns[0].message_key, "u1");
        assert_eq!(turns[0].ts, Some(100));
        assert_eq!(turns[1].role, "ai");
        assert_eq!(turns[1].content, "好的，我来改。");
        assert!(turns[1].thinking);
        assert_eq!(turns[1].tool_calls, 1);
        assert_eq!(turns[1].mutating_tool_calls, 1);
        assert_eq!(turns[1].files_touched, vec!["/r/README.md".to_string()]);
        assert_eq!(turns[1].tokens_in, 10);
        assert_eq!(turns[1].tokens_out, 20);
        assert_eq!(turns[1].message_key, "u2");
    }

    #[test]
    fn missing_uuid_falls_back_to_conversation_and_turn_index_key() {
        let text = "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"无 uuid\"}}\n";
        let turns = parse_chunk(AgentKind::Claude, text, "conv1", 5);
        assert_eq!(turns[0].message_key, "conv1:5");
    }

    #[test]
    fn starting_turn_index_offsets_fallback_keys() {
        let text = concat!(
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"第一\"}}\n",
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"第二\"}}\n",
        );
        let turns = parse_chunk(AgentKind::Claude, text, "conv1", 10);
        assert_eq!(turns[0].message_key, "conv1:10");
        assert_eq!(turns[1].message_key, "conv1:11");
    }

    #[test]
    fn claude_skips_synthetic_caveat_and_command_echo_messages() {
        // 验收反馈截图:会话列表标题全是 `<local-command-caveat>`/
        // `<command-name>` 这类 CLI 自己注入的合成消息,不是用户真正
        // 打的字——不该摄取成一个对话回合(否则"取第一条人类消息当
        // 标题"这条推导必然踩中它们)。前者带 `isMeta: true`,后者
        // (纯斜杠命令回显,如 `/clear`)不带,两条判据都要覆盖。
        // `<task-notification>` 是同类问题:后台子 agent 完成时 CLI 注入
        // 的通知块,不带 `isMeta`,同样会污染逐回合审阅列表(2026-08-21
        // 实测本项目自己会话的原始 jsonl 文件确认)。
        let text = concat!(
            "{\"type\":\"user\",\"uuid\":\"u1\",\"isMeta\":true,\"message\":{\"role\":\"user\",",
            "\"content\":\"<local-command-caveat>Caveat: ...</local-command-caveat>\"}}\n",
            "{\"type\":\"user\",\"uuid\":\"u2\",\"message\":{\"role\":\"user\",",
            "\"content\":\"<command-name>/clear</command-name>\"}}\n",
            "{\"type\":\"user\",\"uuid\":\"u3\",\"message\":{\"role\":\"user\",",
            "\"content\":\"<local-command-stdout>Set model to Sonnet 5</local-command-stdout>\"}}\n",
            "{\"type\":\"user\",\"uuid\":\"u4\",\"message\":{\"role\":\"user\",",
            "\"content\":\"<task-notification>\\n<task-id>abc</task-id>\\n<status>completed</status>\\n</task-notification>\"}}\n",
            "{\"type\":\"user\",\"uuid\":\"u5\",\"message\":{\"role\":\"user\",",
            "\"content\":\"真正的问题在这里\"}}\n",
        );
        let turns = parse_chunk(AgentKind::Claude, text, "conv1", 0);
        assert_eq!(turns.len(), 1, "只有真实消息应该摄取成回合");
        assert_eq!(turns[0].content, "真正的问题在这里");
        assert_eq!(turns[0].message_key, "u5");
    }

    #[test]
    fn claude_captures_tool_result_as_its_own_turn() {
        // 真实 Claude transcript 里 tool_result 的 content 可能是纯字符串，
        // 也可能是 [{"type":"text","text":"..."}] 数组——两种都要处理
        // (2026-08-21 实测本项目自己会话的原始 jsonl 文件确认)。
        let text = concat!(
            "{\"type\":\"user\",\"uuid\":\"u1\",\"timestamp\":100,\"message\":{\"role\":\"user\",",
            "\"content\":[{\"tool_use_id\":\"t1\",\"type\":\"tool_result\",",
            "\"content\":\"plain string result\"}]}}\n",
            "{\"type\":\"user\",\"uuid\":\"u2\",\"timestamp\":200,\"message\":{\"role\":\"user\",",
            "\"content\":[{\"tool_use_id\":\"t2\",\"type\":\"tool_result\",\"is_error\":true,",
            "\"content\":[{\"type\":\"text\",\"text\":\"boom\"}]}]}}\n"
        );
        let turns = parse_chunk(AgentKind::Claude, text, "conv1", 0);
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].role, "tool_result");
        assert_eq!(turns[0].content, "plain string result");
        assert!(!turns[0].is_error);
        assert_eq!(turns[0].message_key, "u1");
        assert_eq!(turns[1].role, "tool_result");
        assert_eq!(turns[1].content, "boom");
        assert!(turns[1].is_error);
    }

    #[test]
    fn claude_tool_result_with_multiple_text_blocks_joins_with_newline() {
        let text = concat!(
            "{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"role\":\"user\",",
            "\"content\":[{\"type\":\"tool_result\",",
            "\"content\":[{\"type\":\"text\",\"text\":\"第一段\"},",
            "{\"type\":\"text\",\"text\":\"第二段\"}]}]}}\n"
        );
        let turns = parse_chunk(AgentKind::Claude, text, "conv1", 0);
        assert_eq!(turns[0].content, "第一段\n第二段");
    }

    #[test]
    fn is_command_content_recognizes_slash_commands() {
        assert!(is_command_content("/clear"));
        assert!(is_command_content("/model glm-5.2"));
        assert!(is_command_content("/compact"));
        assert!(is_command_content("  /help  ")); // 前后空白不影响判断
    }

    #[test]
    fn is_command_content_rejects_real_messages() {
        assert!(!is_command_content("主机面板，一个主机只能打开一个SSH tab"));
        assert!(!is_command_content(""));
        assert!(!is_command_content("/")); // 光一个斜杠，后面没有命令名
        // 注：`/path/...` 这种"整条以 / 开头的文件路径"是已知的误判盲区
        // (见 is_command_content 文档注释:"堵不住所有 case")，这里不把它
        // 当断言——实现按计划有意为之，`/ + 字母数字` 即判为命令。
    }

    #[test]
    fn codebuddy_captures_function_call_and_result() {
        // 2026-08-21 实测真实 CodeBuddy transcript：function_call/
        // function_call_result 是跟 "message" 平级的顶层 type，不是嵌在
        // message.content 数组里的 block——此前整个类型分支都没被认，
        // 一律在最上面的 `type != "message"` 直接 continue 掉。
        let text = concat!(
            "{\"id\":\"fc1\",\"type\":\"function_call\",\"timestamp\":100,",
            "\"name\":\"Grep\",\"arguments\":\"{\\\"pattern\\\":\\\"foo\\\",\\\"path\\\":\\\"src\\\"}\"}\n",
            "{\"id\":\"fcr1\",\"type\":\"function_call_result\",\"timestamp\":200,",
            "\"name\":\"Grep\",\"status\":\"completed\",",
            "\"output\":{\"type\":\"text\",\"text\":\"src/a.rs\\nsrc/b.rs\"}}\n",
            "{\"id\":\"fcr2\",\"type\":\"function_call_result\",\"timestamp\":300,",
            "\"name\":\"Bash\",\"status\":\"incomplete\",",
            "\"output\":{\"type\":\"text\",\"text\":\"command timed out\"}}\n"
        );
        let turns = parse_chunk(AgentKind::Codebuddy, text, "conv1", 0);
        assert_eq!(turns.len(), 3);
        assert_eq!(turns[0].role, "ai");
        assert_eq!(turns[0].tool_calls, 1);
        // `tool_summary()` 优先取 `pattern` 而非 `path`——与 Claude 侧既有
        // 逻辑一致(工具调用本身通用，不在此处另开特例)。
        assert_eq!(turns[0].tools_summary, vec!["Grep foo".to_string()]);
        assert_eq!(turns[1].role, "tool_result");
        assert_eq!(turns[1].content, "src/a.rs\nsrc/b.rs");
        assert!(!turns[1].is_error);
        assert_eq!(turns[2].role, "tool_result");
        assert!(turns[2].is_error, "status=incomplete 应判定为失败");
    }

    #[test]
    fn codebuddy_captures_reasoning_as_thinking_marker() {
        let text = "{\"id\":\"r1\",\"type\":\"reasoning\",\"timestamp\":100}\n";
        let turns = parse_chunk(AgentKind::Codebuddy, text, "conv1", 0);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].role, "ai");
        assert!(turns[0].thinking);
        assert_eq!(turns[0].content, "");
    }

    #[test]
    fn codebuddy_ignores_snapshot_and_summary_lines() {
        let text = concat!(
            "{\"id\":\"s1\",\"type\":\"file-history-snapshot\"}\n",
            "{\"id\":\"s2\",\"type\":\"ai-title\",\"title\":\"foo\"}\n",
            "{\"id\":\"s3\",\"type\":\"summary\"}\n"
        );
        let turns = parse_chunk(AgentKind::Codebuddy, text, "conv1", 0);
        assert!(turns.is_empty());
    }

    #[test]
    fn unsupported_agents_yield_empty() {
        let text = "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"忽略\"}}\n";
        assert!(parse_chunk(AgentKind::Codex, text, "c", 0).is_empty());
        assert!(parse_chunk(AgentKind::V8agent, text, "c", 0).is_empty());
    }

    #[test]
    fn last_complete_line_boundary_excludes_trailing_half_line() {
        assert_eq!(last_complete_line_boundary("a\nb\n"), 4);
        assert_eq!(last_complete_line_boundary("a\nb"), 2);
        assert_eq!(last_complete_line_boundary("no newline yet"), 0);
        assert_eq!(last_complete_line_boundary(""), 0);
    }

    #[test]
    fn codebuddy_parses_real_fixture_sample_with_usage() {
        let text = include_str!("../../../dozer-hook/fixtures/codebuddy-transcript-sample.jsonl");
        let turns = parse_chunk(AgentKind::Codebuddy, text, "conv1", 0);
        assert_eq!(turns.len(), 2, "1 用户消息 + 1 assistant 消息;快照行跳过");
        assert_eq!(turns[0].role, "human");
        assert_eq!(turns[0].content, "reply with exactly one word: hello");
        assert_eq!(turns[1].role, "ai");
        assert_eq!(turns[1].content, "hello");
        // fixture 里两条消息 id 不同,取到即视为通过(不用本机真实 fixture
        // 猜数值);关键是不再退化成 fallback_key。
        assert_ne!(turns[0].message_key, "conv1:0");
        assert_ne!(turns[1].message_key, "conv1:1");
        assert!(turns[0].ts.is_some());
    }

    #[test]
    fn codebuddy_joins_multiple_text_blocks_and_skips_snapshot() {
        let text = concat!(
            "{\"id\":\"m1\",\"type\":\"message\",\"role\":\"user\",\"content\":",
            "[{\"type\":\"input_text\",\"text\":\"第一段\"},",
            "{\"type\":\"input_text\",\"text\":\"第二段\"}]}\n",
            "{\"id\":\"m2\",\"type\":\"message\",\"role\":\"assistant\",\"content\":",
            "[{\"type\":\"output_text\",\"text\":\"回复一\"}]}\n",
            "{\"id\":\"m3\",\"type\":\"file-history-snapshot\"}\n",
        );
        let turns = parse_chunk(AgentKind::Codebuddy, text, "conv1", 0);
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].content, "第一段\n第二段");
        assert_eq!(turns[0].message_key, "m1");
        assert_eq!(turns[1].content, "回复一");
        assert_eq!(turns[1].message_key, "m2");
    }

    #[test]
    fn codebuddy_skips_synthetic_system_reminder_wrapper() {
        // CodeBuddy 侧同款包裹块(`<system-reminder data-role="...">`)没有
        // `isMeta` 这类结构化标记,直接以纯文本出现在 content 里,只能靠
        // 前缀识别——验收反馈截图里就有一条 codebuddy 会话标题是这个。
        let text = concat!(
            "{\"id\":\"m1\",\"type\":\"message\",\"role\":\"user\",\"content\":",
            "[{\"type\":\"input_text\",\"text\":",
            "\"<system-reminder data-role=\\\"command-caveat\\\">Caveat: ...\"}]}\n",
            "{\"id\":\"m2\",\"type\":\"message\",\"role\":\"user\",\"content\":",
            "[{\"type\":\"input_text\",\"text\":\"真正的问题\"}]}\n",
        );
        let turns = parse_chunk(AgentKind::Codebuddy, text, "conv1", 0);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].content, "真正的问题");
        assert_eq!(turns[0].message_key, "m2");
    }

    #[test]
    fn codebuddy_usage_and_tool_fields_stay_zero() {
        // CodeBuddy fixture 里没见过 tool_use 形状消息,不臆测其结构——
        // tool_calls/mutating_tool_calls/files_touched 恒零/空(同原
        // dozer-app usage.rs::parse_codebuddy_shaped_usage 的既有口径)。
        let text = concat!(
            "{\"id\":\"m1\",\"type\":\"message\",\"role\":\"assistant\",",
            "\"content\":[{\"type\":\"output_text\",\"text\":\"x\"}],",
            "\"providerData\":{\"usage\":{\"inputTokens\":5,\"outputTokens\":7}}}\n",
        );
        let turns = parse_chunk(AgentKind::Codebuddy, text, "conv1", 0);
        assert_eq!(turns[0].tokens_in, 5);
        assert_eq!(turns[0].tokens_out, 7);
        assert_eq!(turns[0].tool_calls, 0);
        assert!(turns[0].files_touched.is_empty());
    }
}

#[cfg(test)]
mod trace_detail_tests {
    use super::*;

    #[test]
    fn extract_claude_trace_detail_reads_thinking_text_and_tool_input() {
        let raw = r#"{"type":"assistant","message":{"content":[
            {"type":"thinking","thinking":"先看看现有实现"},
            {"type":"tool_use","name":"Edit","input":{"file_path":"README.md","old_string":"a","new_string":"b"}},
            {"type":"text","text":"改好了"}
        ]}}"#;
        let detail = extract_turn_trace_detail(raw, AgentKind::Claude);
        assert_eq!(detail.thinking_text.as_deref(), Some("先看看现有实现"));
        assert_eq!(detail.tool_calls.len(), 1);
        assert_eq!(detail.tool_calls[0].summary, "Edit README.md");
        let input_json = detail.tool_calls[0].input_json.as_deref().unwrap();
        assert!(input_json.contains("README.md"));
        assert!(input_json.contains("old_string"));
    }

    #[test]
    fn extract_claude_trace_detail_multiple_tool_use_and_no_thinking() {
        let raw = r#"{"type":"assistant","message":{"content":[
            {"type":"tool_use","name":"Read","input":{"file_path":"a.txt"}},
            {"type":"tool_use","name":"Bash","input":{"command":"ls -la"}}
        ]}}"#;
        let detail = extract_turn_trace_detail(raw, AgentKind::Claude);
        assert_eq!(detail.thinking_text, None);
        assert_eq!(detail.tool_calls.len(), 2);
        assert_eq!(detail.tool_calls[0].summary, "Read a.txt");
        assert_eq!(detail.tool_calls[1].summary, "Bash ls -la");
    }

    #[test]
    fn extract_claude_trace_detail_no_blocks_returns_empty() {
        let raw = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"hi"}]}}"#;
        let detail = extract_turn_trace_detail(raw, AgentKind::Claude);
        assert_eq!(detail, TurnTraceDetail::default());
    }

    #[test]
    fn extract_trace_detail_malformed_json_returns_empty() {
        let detail = extract_turn_trace_detail("not json", AgentKind::Claude);
        assert_eq!(detail, TurnTraceDetail::default());
    }

    #[test]
    fn extract_codebuddy_trace_detail_function_call_has_input() {
        let raw =
            r#"{"type":"function_call","name":"Edit","arguments":"{\"file_path\":\"a.rs\"}"}"#;
        let detail = extract_turn_trace_detail(raw, AgentKind::Codebuddy);
        assert_eq!(detail.thinking_text, None);
        assert_eq!(detail.tool_calls.len(), 1);
        assert_eq!(detail.tool_calls[0].summary, "Edit a.rs");
        assert!(
            detail.tool_calls[0]
                .input_json
                .as_deref()
                .unwrap()
                .contains("a.rs")
        );
    }

    #[test]
    fn extract_codebuddy_trace_detail_reasoning_with_text_blocks() {
        // CodeBuddy 的 reasoning 事件真实字段名未在现有 fixture 里观测到，
        // 按该文件里 assistant 消息用 "output_text"/"input_text" 类型化
        // block 数组的既有约定类推为 "reasoning_text"；如果拿到真实样本发现
        // 字段名不同，回来改这一个函数即可，不影响其它任何东西。
        let raw = r#"{"type":"reasoning","content":[{"type":"reasoning_text","text":"先想想"}]}"#;
        let detail = extract_turn_trace_detail(raw, AgentKind::Codebuddy);
        assert_eq!(detail.thinking_text.as_deref(), Some("先想想"));
        assert_eq!(detail.tool_calls, Vec::new());
    }

    #[test]
    fn extract_codebuddy_trace_detail_reasoning_without_matching_blocks_is_none() {
        let raw =
            r#"{"type":"reasoning","content":[{"type":"summary_text","text":"other shape"}]}"#;
        let detail = extract_turn_trace_detail(raw, AgentKind::Codebuddy);
        assert_eq!(detail.thinking_text, None);
    }

    #[test]
    fn extract_trace_detail_unhandled_agent_returns_empty() {
        let detail = extract_turn_trace_detail(r#"{"type":"whatever"}"#, AgentKind::Codex);
        assert_eq!(detail, TurnTraceDetail::default());
    }
}
