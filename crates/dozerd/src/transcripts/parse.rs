//! agent transcript(JSONL)增量解析:按 `AgentKind` 分派,产出既带展示
//! 字段又带用量字段的 `ParsedTurn`(合并原 dozer-app `transcript.rs` +
//! `usage.rs` 两套平行解析器,避免长期重复维护——spec"参考调研"一节)。

use dozer_core::protocol::{AgentKind, ToolCallInfo};
use serde_json::Value;

pub const MUTATING_TOOLS: [&str; 4] = ["Edit", "Write", "MultiEdit", "NotebookEdit"];
const V8AGENT_MUTATING_TOOLS: [&str; 3] = ["write_file", "edit_file", "git_commit"];

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
    mutating_tools: &[&str],
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
                            if mutating_tools.contains(&name) {
                                mutating_tool_calls += 1;
                                // 跟 tool_summary() 一样的 file_path→path 回退链——
                                // Claude 自己的工具用 file_path,V8agent 的
                                // write_file/edit_file 用 path(见
                                // v8agent-core/src/tools/fs.rs 的
                                // WriteFileArgs/EditFileArgs),不回退会让
                                // V8agent 的 files_touched 恒为空。
                                let path = input
                                    .get("file_path")
                                    .or_else(|| input.get("path"))
                                    .and_then(|p| p.as_str());
                                if let Some(path) = path {
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

fn join_text_blocks_by_kind(blocks: &[Value], kind: &str) -> String {
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

/// Codex CLI 注入到 `role:"user"` 消息里的合成 content 块:环境上下文、用户
/// 指令、推荐插件、回合中断通知、`# AGENTS.md instructions` 项目指令、图片
/// 占位标记——都不是用户真正打的字。
///
/// 跟 `is_synthetic_wrapper_content` 的区别是**粒度**:那个判"整条消息",
/// Codex 这些是按块跟真实用户文本混在同一条消息里的。实测本机 43 份真实
/// rollout(234 条 user 消息):`<environment_context>` 有 11 次落在第二个块、
/// 第一个块是 `# AGENTS.md instructions`;`<image ...>`/`</image>` 与用户真实
/// 文字同处一条消息。所以必须**逐块**剔除、保留剩余真实文本,只有全部块都
/// 是合成时整条消息才丢弃(同一批数据:58/234 条整条丢弃,其余 176 条留下
/// 的都是用户真实输入)。按整条消息判前缀会同时漏掉那 11 条、并误杀带图片
/// 的真实消息。
///
/// `dozer-app/src/transcript.rs::extract_line_activity` 的 Codex 分支维护同
/// 一份前缀表(agent card 的 activity 也读这些文件;两侧解析器按既有做法
/// 不跨 crate 复用,同 `days_from_civil`/`civil_from_days`),改这里要同步改
/// 那边。
pub(crate) fn is_synthetic_codex_block(text: &str) -> bool {
    let trimmed = text.trim_start();
    trimmed.starts_with("<environment_context")
        || trimmed.starts_with("<user_instructions")
        || trimmed.starts_with("<recommended_plugins")
        || trimmed.starts_with("<turn_aborted")
        || trimmed.starts_with("<image ")
        || trimmed.starts_with("<image>")
        || trimmed.starts_with("</image")
        || trimmed.starts_with("# AGENTS.md instructions")
}

/// `join_text_blocks_by_kind` 的 Codex 版:先按块剔掉 CLI 注入的合成块,再
/// 拼接(为什么必须按块见 `is_synthetic_codex_block`)。
fn join_real_codex_text_blocks(blocks: &[Value], kind: &str) -> String {
    let mut text = String::new();
    for b in blocks {
        if b.get("type").and_then(|t| t.as_str()) == Some(kind)
            && let Some(t) = b.get("text").and_then(|t| t.as_str())
            && !is_synthetic_codex_block(t)
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
                        let content = join_text_blocks_by_kind(blocks, "input_text");
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
                            content: join_text_blocks_by_kind(blocks, "output_text"),
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

/// Howard Hinnant 的公开 days_from_civil 算法——`dozer-app` 的
/// `usage/aggregate.rs::civil_from_days` 是它的逆运算(那边是"天数→年月日"
/// 给图表标签用,这边是"年月日→天数"给时间戳解析用),两边各自私有实现,
/// `dozerd`/`dozer-app` 之间没有共享的日期工具模块,不做跨 crate 复用。
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let mp = ((m as i64 + 9) % 12) as u64;
    let doy = (153 * mp + 2) / 5 + d as u64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe as i64 - 719_468
}

/// Codex 的 timestamp 字段(如 `"2026-09-21T02:50:44.008Z"`)→ epoch 毫秒。
/// 定长解析,不引入 chrono/time(见 Global Constraints)。格式跟实测样本
/// (`~/.codex/sessions/**/rollout-*.jsonl`)完全一致:固定 UTC、毫秒精度、
/// `Z` 结尾;只要有一处不匹配就整体返回 `None`,不做宽松容错——时间戳解析
/// 失败只影响这一行的排序展示,不值得为极端形状维护正则级别的解析器。
fn parse_codex_timestamp_ms(s: &str) -> Option<u64> {
    let b = s.as_bytes();
    if b.len() != 24
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
        || b[19] != b'.'
        || b[23] != b'Z'
    {
        return None;
    }
    let year: i64 = s.get(0..4)?.parse().ok()?;
    let month: u32 = s.get(5..7)?.parse().ok()?;
    let day: u32 = s.get(8..10)?.parse().ok()?;
    let hour: u64 = s.get(11..13)?.parse().ok()?;
    let minute: u64 = s.get(14..16)?.parse().ok()?;
    let second: u64 = s.get(17..19)?.parse().ok()?;
    let millis: u64 = s.get(20..23)?.parse().ok()?;
    let days = days_from_civil(year, month, day);
    let day_ms = u64::try_from(days).ok()?.checked_mul(86_400_000)?;
    let time_ms = hour * 3_600_000 + minute * 60_000 + second * 1_000 + millis;
    Some(day_ms + time_ms)
}

/// Codex rollout transcript(`session_meta`/`event_msg`/`response_item`/
/// `turn_context`/`token_usage_record`,顶层 `type` + 嵌套 `payload`,跟
/// Claude/Codebuddy 的扁平结构完全不同,见 spike 记录
/// `docs/superpowers/specs/2026-08-07-codex-spike-findings.md`)→
/// `ParsedTurn`。v1 范围只摘"人类/AI 文本"(`response_item`/
/// `payload.type:"message"`,role user/assistant,developer 角色是 CLI 注入
/// 的系统提示片段,跳过)和"token 用量"(`token_usage_record`,产出一条独立
/// 的 `role:"token_usage"` 行,不含正文)。`reasoning`/`custom_tool_call`/
/// `custom_tool_call_output`/`session_meta`/`event_msg`/`world_state`/
/// `turn_context` 等行本计划不解析,原样跳过(同 Claude 侧对未知行类型的
/// 既有策略)。
fn parse_codex_shaped_chunk(
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
        let top_type = v.get("type").and_then(|t| t.as_str());
        let ts = v
            .get("timestamp")
            .and_then(|t| t.as_str())
            .and_then(parse_codex_timestamp_ms);
        let Some(payload) = v.get("payload") else {
            continue;
        };
        match top_type {
            Some("response_item")
                if payload.get("type").and_then(|t| t.as_str()) == Some("message") =>
            {
                let role = payload.get("role").and_then(|r| r.as_str());
                let Some(blocks) = payload.get("content").and_then(|c| c.as_array()) else {
                    continue;
                };
                let message_key = payload
                    .get("id")
                    .and_then(|i| i.as_str())
                    .map(str::to_string)
                    .unwrap_or_else(|| fallback_key(conversation_id, turn_index));
                match role {
                    Some("user") => {
                        // 逐块剔除 CLI 注入的合成块(环境上下文/AGENTS.md 指令/
                        // 图片占位等);剩下的全是合成块时整条丢弃,否则会污染
                        // "首条人类消息当标题"的推导和用量面板的回合计数。
                        let content = join_real_codex_text_blocks(blocks, "input_text");
                        if content.is_empty() {
                            continue;
                        }
                        out.push(ParsedTurn {
                            message_key,
                            role: "human".into(),
                            content,
                            ts,
                            raw_json: line.to_string(),
                            ..Default::default()
                        });
                        turn_index += 1;
                    }
                    Some("assistant") => {
                        out.push(ParsedTurn {
                            message_key,
                            role: "ai".into(),
                            content: join_text_blocks_by_kind(blocks, "output_text"),
                            ts,
                            raw_json: line.to_string(),
                            ..Default::default()
                        });
                        turn_index += 1;
                    }
                    // "developer" 是 Codex CLI 自己注入的系统提示片段(等价
                    // 于 Claude 的 isMeta 消息),不是真实用户发言,不摄取。
                    _ => {}
                }
            }
            Some("token_usage_record") => {
                let Some(usage) = payload.get("usage") else {
                    continue;
                };
                let field = |k: &str| usage.get(k).and_then(|n| n.as_u64()).unwrap_or(0);
                let input_tokens = field("input_tokens");
                let cached_input_tokens = field("cached_input_tokens");
                let message_key = payload
                    .get("response_id")
                    .and_then(|i| i.as_str())
                    .map(str::to_string)
                    .unwrap_or_else(|| fallback_key(conversation_id, turn_index));
                out.push(ParsedTurn {
                    message_key,
                    role: "token_usage".into(),
                    // input_tokens 已经把 cached_input_tokens 算在内(见
                    // Global Constraints),减掉才符合 tokens_in/
                    // tokens_cache_read 互不重叠、直接相加的既有展示公式。
                    tokens_in: input_tokens.saturating_sub(cached_input_tokens),
                    tokens_out: field("output_tokens"),
                    tokens_cache_read: cached_input_tokens,
                    tokens_cache_write: field("cache_write_input_tokens"),
                    ts,
                    raw_json: line.to_string(),
                    ..Default::default()
                });
                turn_index += 1;
            }
            _ => {}
        }
    }
    out
}

/// Goose hook journal(`dozer-hook/src/goose.rs` 落盘的 schema v1 行:
/// `{"schema_version":1,"type":"goose_hook","event":..,"ts_ms":..,
/// "dozer_session_id":..,"goose_session_id":..,"payload":{...}}`)→
/// `ParsedTurn`。只认 `type:"goose_hook"` 的行,`payload` 是 Goose 原生
/// hook 事件 JSON(字段见官方 hooks 文档)。事件映射见 spec D5:
///
/// - `UserPromptSubmit` → `human`(取 `payload.message`)
/// - `PreToolUse` → `ai` 工具调用(记 `tool_name`/`tool_input`/`tool_call_id`)
/// - `PostToolUse` → `tool_result` 成功(`is_error=false`)
/// - `PostToolUseFailure` → `tool_result` 失败(`is_error=true`)
/// - `AfterFileEdit` → `ai` 文件修改(路径取 `matcher_context`,恒
///   `mutating_tool_calls=1`)
/// - `Stop` → `ai`(取 `last_assistant_message`,为空则不产出 turn)
///
/// 生命周期(`SessionStart`/`SessionEnd`)与未知事件不产出 turn。未知
/// `schema_version` 跳过(记录 warning),不 panic——未来 schema 演进时
/// 旧 parser 只降级为"读不到",不崩溃。
fn parse_goose_hook_chunk(
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
        if v.get("type").and_then(|t| t.as_str()) != Some("goose_hook") {
            continue;
        }
        if v.get("schema_version").and_then(|n| n.as_u64()) != Some(1) {
            tracing::warn!("goose hook journal 未知 schema_version,跳过该行");
            continue;
        }
        let Some(event) = v.get("event").and_then(|e| e.as_str()) else {
            continue;
        };
        let ts = v.get("ts_ms").and_then(|n| n.as_u64());
        let Some(payload) = v.get("payload") else {
            continue;
        };
        let fallback = || fallback_key(conversation_id, turn_index);
        match event {
            "UserPromptSubmit" => {
                let Some(content) = payload.get("message").and_then(|m| m.as_str()) else {
                    continue;
                };
                let content = content.trim().to_string();
                if content.is_empty() {
                    continue;
                }
                out.push(ParsedTurn {
                    message_key: fallback(),
                    role: "human".into(),
                    content,
                    ts,
                    raw_json: line.to_string(),
                    ..Default::default()
                });
                turn_index += 1;
            }
            "PreToolUse" => {
                let tool_name = payload
                    .get("tool_name")
                    .and_then(|t| t.as_str())
                    .unwrap_or("工具");
                let tool_input = payload.get("tool_input").cloned().unwrap_or(Value::Null);
                let message_key = payload
                    .get("tool_call_id")
                    .and_then(|i| i.as_str())
                    .map(str::to_string)
                    .unwrap_or_else(fallback);
                out.push(ParsedTurn {
                    message_key,
                    role: "ai".into(),
                    content: String::new(),
                    tools_summary: vec![tool_summary(tool_name, &tool_input)],
                    tool_calls: 1,
                    ts,
                    raw_json: line.to_string(),
                    ..Default::default()
                });
                turn_index += 1;
            }
            "PostToolUse" | "PostToolUseFailure" => {
                let is_error = event == "PostToolUseFailure";
                let tool_name = payload
                    .get("tool_name")
                    .and_then(|t| t.as_str())
                    .unwrap_or("工具");
                let content = if is_error {
                    format!("工具执行失败: {tool_name}")
                } else {
                    format!("工具执行成功: {tool_name}")
                };
                out.push(ParsedTurn {
                    message_key: fallback(),
                    role: "tool_result".into(),
                    content,
                    is_error,
                    ts,
                    raw_json: line.to_string(),
                    ..Default::default()
                });
                turn_index += 1;
            }
            "AfterFileEdit" => {
                let tool_name = payload
                    .get("tool_name")
                    .and_then(|t| t.as_str())
                    .unwrap_or("file_edit");
                // matcher_context 是编辑的文件路径(官方 hooks 文档明确)。
                let path = payload
                    .get("matcher_context")
                    .and_then(|p| p.as_str())
                    .map(str::to_string);
                let mut files_touched = Vec::new();
                if let Some(p) = &path {
                    files_touched.push(p.clone());
                }
                let summary = match &path {
                    Some(p) => {
                        let base = std::path::Path::new(p)
                            .file_name()
                            .map(|s| s.to_string_lossy().into_owned())
                            .unwrap_or_else(|| p.clone());
                        format!("{tool_name} {base}")
                    }
                    None => tool_name.to_string(),
                };
                out.push(ParsedTurn {
                    message_key: fallback(),
                    role: "ai".into(),
                    content: String::new(),
                    tools_summary: vec![summary],
                    tool_calls: 1,
                    mutating_tool_calls: 1,
                    files_touched,
                    ts,
                    raw_json: line.to_string(),
                    ..Default::default()
                });
                turn_index += 1;
            }
            "Stop" => {
                let Some(content) = payload
                    .get("last_assistant_message")
                    .and_then(|m| m.as_str())
                else {
                    continue;
                };
                let content = content.trim().to_string();
                if content.is_empty() {
                    continue;
                }
                out.push(ParsedTurn {
                    message_key: fallback(),
                    role: "ai".into(),
                    content,
                    ts,
                    raw_json: line.to_string(),
                    ..Default::default()
                });
                turn_index += 1;
            }
            _ => {}
        }
    }
    out
}

/// Aider canonical transcript(dozer-hook bridge 落盘的 schema v1 行:
/// `{"schema_version":1,"type":"aider_message","message_id":..,"role":"human"|"ai",
/// "content":..,"ts_ms":..}`)→`ParsedTurn`。只认 `type:"aider_message"` 的行;
/// `role:"human"` → human、`role:"ai"` → ai,`message_key` 取 `message_id`。
/// 工具/mutation/files/thinking/token 字段全零——Aider chat history 没有这些
/// 稳定结构化字段,不伪造(见 spec D6)。未知 schema/role/畸形行跳过,不 panic。
fn parse_aider_chunk(
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
        if v.get("type").and_then(|t| t.as_str()) != Some("aider_message") {
            continue;
        }
        if v.get("schema_version").and_then(|n| n.as_u64()) != Some(1) {
            tracing::warn!("aider canonical 未知 schema_version,跳过该行");
            continue;
        }
        let Some(role) = v.get("role").and_then(|r| r.as_str()) else {
            continue;
        };
        let role = match role {
            "human" => "human",
            "ai" => "ai",
            _ => continue,
        };
        let content = v
            .get("content")
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if content.is_empty() {
            continue;
        }
        let message_key = v
            .get("message_id")
            .and_then(|m| m.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| fallback_key(conversation_id, turn_index));
        let ts = v.get("ts_ms").and_then(|n| n.as_u64());
        out.push(ParsedTurn {
            message_key,
            role: role.into(),
            content,
            ts,
            raw_json: line.to_string(),
            ..Default::default()
        });
        turn_index += 1;
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
        AgentKind::Claude | AgentKind::Opencode | AgentKind::Unknown => {
            parse_claude_shaped_chunk(text, conversation_id, starting_turn_index, &MUTATING_TOOLS)
        }
        AgentKind::V8agent => parse_claude_shaped_chunk(
            text,
            conversation_id,
            starting_turn_index,
            &V8AGENT_MUTATING_TOOLS,
        ),
        AgentKind::Codebuddy => {
            parse_codebuddy_shaped_chunk(text, conversation_id, starting_turn_index)
        }
        AgentKind::Codex => parse_codex_shaped_chunk(text, conversation_id, starting_turn_index),
        AgentKind::Goose => parse_goose_hook_chunk(text, conversation_id, starting_turn_index),
        AgentKind::Aider => parse_aider_chunk(text, conversation_id, starting_turn_index),
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
    /// role == "tool_result" 时对应的调用 id；见 `TurnRecord.tool_result_call_id`。
    pub tool_result_call_id: Option<String>,
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
        // Claude/Opencode/Unknown/V8agent 摄取时都走
        // parse_claude_shaped_chunk(parse_chunk 的分派,parse.rs 上方),
        // 读时解析沿用同一分派。
        AgentKind::Claude | AgentKind::Opencode | AgentKind::Unknown | AgentKind::V8agent => {
            extract_claude_trace_detail(&v)
        }
        // Codex 现在摄取人类/AI 文本 + 用量(见 parse_codex_shaped_chunk),
        // 但结构化工具调用/思考文本不在 v1 范围内,读时补全维持全空
        // ——跟"有 raw_json 但选择不解析"是两回事,不是没有数据可读。
        AgentKind::Codex => TurnTraceDetail::default(),
        // Goose 的 PreToolUse 行携带 tool_name/tool_input,读时补出结构化
        // 工具调用明细(其余 Goose 行无结构化内容,回全空)。
        AgentKind::Goose => extract_goose_trace_detail(&v),
        // Aider canonical 行只有 human/ai 文本,没有结构化工具/思考字段,
        // 读时补全维持全空(见 parse_aider_chunk)。
        AgentKind::Aider => TurnTraceDetail::default(),
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
    let mut tool_result_ids: Vec<String> = Vec::new();
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
                let id = b.get("id").and_then(|i| i.as_str()).map(str::to_string);
                tool_calls.push(ToolCallInfo {
                    summary: tool_summary(name, &input),
                    input_json: input_json_of(&input),
                    id,
                });
            }
            Some("tool_result") => {
                if let Some(id) = b.get("tool_use_id").and_then(|i| i.as_str()) {
                    tool_result_ids.push(id.to_string());
                }
            }
            _ => {}
        }
    }
    // 一行原始消息可能合并了多个 tool_result block(见
    // parse_claude_shaped_chunk 的 tool_result 分支，把同一条消息里的多个
    // block 拼成一个 ParsedTurn)——只有恰好一个时才能确定这行对应哪次调用，
    // 零个/多个都回落 None，不瞎猜。
    let tool_result_call_id = match tool_result_ids.as_slice() {
        [id] => Some(id.clone()),
        _ => None,
    };
    TurnTraceDetail {
        thinking_text,
        tool_calls,
        tool_result_call_id,
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
            let text = join_text_blocks_by_kind(&blocks, "reasoning_text");
            TurnTraceDetail {
                thinking_text: if text.is_empty() { None } else { Some(text) },
                tool_calls: Vec::new(),
                tool_result_call_id: None,
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
                    id: None,
                }],
                tool_result_call_id: None,
            }
        }
        _ => TurnTraceDetail::default(),
    }
}

/// Goose journal 行的读时结构化工具调用明细:`PreToolUse` 行产出一个
/// `ToolCallInfo`(summary 复用通用 `tool_summary`、`input_json` 是
/// `tool_input` 的 pretty JSON、`id` 取 payload 的 `tool_call_id`);
/// `PostToolUse`/`PostToolUseFailure` 行产出 `tool_result_call_id`。
/// `tool_call_id` 在三个事件里是同一次调用的稳定标识(goose-docs.ai hooks
/// 文档:"Stable identifier for one tool call...Correlates the events of a
/// single call"),按事件类型分别喂给调用侧或结果侧。其余 Goose 行没有
/// 结构化内容,回全空。
fn extract_goose_trace_detail(v: &Value) -> TurnTraceDetail {
    if v.get("type").and_then(|t| t.as_str()) != Some("goose_hook") {
        return TurnTraceDetail::default();
    }
    let Some(event) = v.get("event").and_then(|e| e.as_str()) else {
        return TurnTraceDetail::default();
    };
    let Some(payload) = v.get("payload") else {
        return TurnTraceDetail::default();
    };
    let call_id = payload
        .get("tool_call_id")
        .and_then(|i| i.as_str())
        .map(str::to_string);
    match event {
        "PreToolUse" => {
            let name = payload
                .get("tool_name")
                .and_then(|t| t.as_str())
                .unwrap_or("工具");
            let input = payload.get("tool_input").cloned().unwrap_or(Value::Null);
            TurnTraceDetail {
                thinking_text: None,
                tool_calls: vec![ToolCallInfo {
                    summary: tool_summary(name, &input),
                    input_json: input_json_of(&input),
                    id: call_id,
                }],
                tool_result_call_id: None,
            }
        }
        "PostToolUse" | "PostToolUseFailure" => TurnTraceDetail {
            thinking_text: None,
            tool_calls: Vec::new(),
            tool_result_call_id: call_id,
        },
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
    fn codex_yields_empty() {
        let text = "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"忽略\"}}\n";
        assert!(parse_chunk(AgentKind::Codex, text, "c", 0).is_empty());
    }

    #[test]
    fn v8agent_uses_its_own_mutating_tool_list() {
        // v8agent-core 的 WriteFileArgs/EditFileArgs 用 "path" 做参数键,
        // 不是 Claude 工具的 "file_path"——这里必须用真实的 v8agent 参数
        // 形状,否则测不出 file_path→path 回退链缺失的 bug。
        let text = concat!(
            "{\"type\":\"assistant\",\"message\":{\"content\":[",
            "{\"type\":\"tool_use\",\"name\":\"write_file\",\"input\":{\"path\":\"foo.rs\"}}",
            "]}}\n"
        );
        let turns = parse_chunk(AgentKind::V8agent, text, "conv1", 0);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].mutating_tool_calls, 1);
        assert_eq!(
            turns[0].files_touched,
            vec!["foo.rs".to_string()],
            "files_touched must fall back to the \"path\" key for V8agent's own tool args"
        );
    }

    #[test]
    fn v8agent_does_not_recognize_claudes_mutating_tool_names() {
        let text = concat!(
            "{\"type\":\"assistant\",\"message\":{\"content\":[",
            "{\"type\":\"tool_use\",\"name\":\"Edit\",\"input\":{\"file_path\":\"foo.rs\"}}",
            "]}}\n"
        );
        let turns = parse_chunk(AgentKind::V8agent, text, "conv1", 0);
        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].mutating_tool_calls, 0,
            "V8agent's own tool is write_file/edit_file, not Claude's Edit — must not cross-recognize"
        );
    }

    #[test]
    fn claude_does_not_recognize_v8agents_mutating_tool_names() {
        let text = concat!(
            "{\"type\":\"assistant\",\"message\":{\"content\":[",
            "{\"type\":\"tool_use\",\"name\":\"write_file\",\"input\":{\"file_path\":\"foo.rs\"}}",
            "]}}\n"
        );
        let turns = parse_chunk(AgentKind::Claude, text, "conv1", 0);
        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].mutating_tool_calls, 0,
            "parameterizing must not leak V8agent's tool names into Claude's list"
        );
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
    fn codex_parses_real_shaped_fixture_sample_with_usage() {
        let text = include_str!("../../../dozer-hook/fixtures/codex-transcript-sample.jsonl");
        let turns = parse_chunk(AgentKind::Codex, text, "conv1", 0);
        assert_eq!(
            turns.len(),
            3,
            "1 用户消息 + 1 assistant 回复 + 1 用量行;developer/session_meta/\
             turn_context/event_msg 等不摄取"
        );
        assert_eq!(turns[0].role, "human");
        assert_eq!(turns[0].content, "reply with exactly one word: hello");
        assert_eq!(turns[1].role, "ai");
        assert_eq!(turns[1].content, "hello");
        assert_eq!(turns[2].role, "token_usage");
        assert_eq!(turns[2].content, "");
        // input_tokens(14768)已经把 cached_input_tokens(12160)算在内
        // (OpenAI Responses API 语义,跟 Claude 相反),摄取时要减掉缓存
        // 命中部分,不然会跟 tokens_cache_read 重复计入。
        assert_eq!(turns[2].tokens_in, 14768 - 12160);
        assert_eq!(turns[2].tokens_out, 5);
        assert_eq!(turns[2].tokens_cache_read, 12160);
        assert_eq!(turns[2].tokens_cache_write, 0);
        assert!(turns[0].ts.is_some(), "timestamp 字符串应该被解析成毫秒");
        assert_ne!(
            turns[0].message_key, "conv1:0",
            "应该取 payload.id,不退化成 fallback_key"
        );
    }

    #[test]
    fn codex_sums_usage_across_multiple_api_calls_within_one_turn() {
        // 一次逻辑回合内可能有好几次模型 API 调用(工具调用往返),每次都
        // 各自产出一条 token_usage_record;`usage` 字段是每次调用自己的
        // 增量花费,不是累计值——两条用量行应该各自摘出一条 ParsedTurn,
        // 求和交给 dozerd 的 SUM 查询,这里只验证"没有被错误合并/覆盖"。
        let text = concat!(
            "{\"timestamp\":\"2026-09-21T00:00:00.000Z\",\"type\":\"response_item\",",
            "\"payload\":{\"type\":\"message\",\"id\":\"m1\",\"role\":\"assistant\",",
            "\"content\":[{\"type\":\"output_text\",\"text\":\"第一步\"}]}}\n",
            "{\"timestamp\":\"2026-09-21T00:00:01.000Z\",\"type\":\"token_usage_record\",",
            "\"payload\":{\"response_id\":\"r1\",\"usage\":{\"input_tokens\":100,",
            "\"cached_input_tokens\":0,\"cache_write_input_tokens\":0,\"output_tokens\":10,",
            "\"reasoning_output_tokens\":0,\"total_tokens\":110}}}\n",
            "{\"timestamp\":\"2026-09-21T00:00:02.000Z\",\"type\":\"response_item\",",
            "\"payload\":{\"type\":\"message\",\"id\":\"m2\",\"role\":\"assistant\",",
            "\"content\":[{\"type\":\"output_text\",\"text\":\"第二步\"}]}}\n",
            "{\"timestamp\":\"2026-09-21T00:00:03.000Z\",\"type\":\"token_usage_record\",",
            "\"payload\":{\"response_id\":\"r2\",\"usage\":{\"input_tokens\":50,",
            "\"cached_input_tokens\":0,\"cache_write_input_tokens\":0,\"output_tokens\":5,",
            "\"reasoning_output_tokens\":0,\"total_tokens\":55}}}\n",
        );
        let turns = parse_chunk(AgentKind::Codex, text, "conv1", 0);
        assert_eq!(turns.len(), 4);
        let usage_turns: Vec<_> = turns.iter().filter(|t| t.role == "token_usage").collect();
        assert_eq!(usage_turns.len(), 2);
        assert_eq!(usage_turns[0].tokens_in, 100);
        assert_eq!(usage_turns[1].tokens_in, 50);
        assert_ne!(
            usage_turns[0].message_key, usage_turns[1].message_key,
            "两次调用各有独立 response_id,不能共用 message_key 互相覆盖"
        );
    }

    #[test]
    fn codex_timestamp_parses_iso8601_to_epoch_millis() {
        let text = concat!(
            "{\"timestamp\":\"1970-01-01T00:00:00.000Z\",\"type\":\"response_item\",",
            "\"payload\":{\"type\":\"message\",\"id\":\"m1\",\"role\":\"user\",",
            "\"content\":[{\"type\":\"input_text\",\"text\":\"hi\"}]}}\n",
        );
        let turns = parse_chunk(AgentKind::Codex, text, "conv1", 0);
        assert_eq!(turns[0].ts, Some(0));
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
    fn extract_claude_trace_detail_reads_tool_use_id() {
        let raw = r#"{"type":"assistant","message":{"content":[
            {"type":"tool_use","id":"toolu_01abc","name":"Edit","input":{"file_path":"README.md"}}
        ]}}"#;
        let detail = extract_turn_trace_detail(raw, AgentKind::Claude);
        assert_eq!(detail.tool_calls.len(), 1);
        assert_eq!(detail.tool_calls[0].id.as_deref(), Some("toolu_01abc"));
    }

    #[test]
    fn extract_claude_trace_detail_tool_use_without_id_field_is_none() {
        let raw = r#"{"type":"assistant","message":{"content":[
            {"type":"tool_use","name":"Edit","input":{"file_path":"README.md"}}
        ]}}"#;
        let detail = extract_turn_trace_detail(raw, AgentKind::Claude);
        assert_eq!(detail.tool_calls[0].id, None);
    }

    #[test]
    fn turn_record_deserializes_old_json_missing_new_fields() {
        // 老协议帧/老存量数据反序列化时没有 id/tool_result_call_id 这两个新
        // 字段，必须靠 #[serde(default)] 兜底成 None，不能报错（Review Focus）。
        let old_json =
            r#"{"turn_index":0,"role":"ai","content":"hi","thinking":false,"is_error":false}"#;
        let turn: dozer_core::protocol::TurnRecord = serde_json::from_str(old_json).unwrap();
        assert_eq!(turn.tool_result_call_id, None);

        let old_tool_call = r#"{"summary":"Edit README.md","input_json":null}"#;
        let call: dozer_core::protocol::ToolCallInfo = serde_json::from_str(old_tool_call).unwrap();
        assert_eq!(call.id, None);
    }

    #[test]
    fn extract_claude_trace_detail_single_tool_result_block_gives_call_id() {
        let raw = r#"{"type":"user","message":{"content":[
            {"type":"tool_result","tool_use_id":"toolu_01abc","content":"ok"}
        ]}}"#;
        let detail = extract_turn_trace_detail(raw, AgentKind::Claude);
        assert_eq!(detail.tool_result_call_id.as_deref(), Some("toolu_01abc"));
    }

    #[test]
    fn extract_claude_trace_detail_no_tool_result_block_gives_none() {
        let raw = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"hi"}]}}"#;
        let detail = extract_turn_trace_detail(raw, AgentKind::Claude);
        assert_eq!(detail.tool_result_call_id, None);
    }

    #[test]
    fn extract_claude_trace_detail_multiple_tool_result_blocks_merged_gives_none() {
        // 一行里多个 tool_result block 合并(摄取时 parse_claude_shaped_chunk 会把
        // 它们拼成一个 ParsedTurn)，没法归属到单一 id，必须回落 None。
        let raw = r#"{"type":"user","message":{"content":[
            {"type":"tool_result","tool_use_id":"t1","content":"a"},
            {"type":"tool_result","tool_use_id":"t2","content":"b"}
        ]}}"#;
        let detail = extract_turn_trace_detail(raw, AgentKind::Claude);
        assert_eq!(detail.tool_result_call_id, None);
    }

    #[test]
    fn extract_claude_trace_detail_tool_result_without_tool_use_id_field_gives_none() {
        let raw = r#"{"type":"user","message":{"content":[
            {"type":"tool_result","content":"ok"}
        ]}}"#;
        let detail = extract_turn_trace_detail(raw, AgentKind::Claude);
        assert_eq!(detail.tool_result_call_id, None);
    }

    #[test]
    fn extract_turn_trace_detail_works_for_v8agent_via_claude_shape() {
        let raw = r#"{"type":"assistant","message":{"content":[
            {"type":"tool_use","name":"edit_file","input":{"file_path":"foo.rs"}}
        ]}}"#;
        let detail = extract_turn_trace_detail(raw, AgentKind::V8agent);
        assert_eq!(detail.tool_calls.len(), 1);
        assert_eq!(detail.tool_calls[0].summary, "edit_file foo.rs");
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

    // —— Goose journal parser ——

    #[test]
    fn goose_parses_human_tool_and_stop_turns() {
        let text = concat!(
            "{\"schema_version\":1,\"type\":\"goose_hook\",\"event\":\"SessionStart\",\"ts_ms\":1,\"dozer_session_id\":\"ds\",\"payload\":{\"session_id\":\"g-1\"}}\n",
            "{\"schema_version\":1,\"type\":\"goose_hook\",\"event\":\"UserPromptSubmit\",\"ts_ms\":2,\"dozer_session_id\":\"ds\",\"payload\":{\"message\":\"帮我修一下\"}}\n",
            "{\"schema_version\":1,\"type\":\"goose_hook\",\"event\":\"PreToolUse\",\"ts_ms\":3,\"dozer_session_id\":\"ds\",\"payload\":{\"tool_name\":\"developer__shell\",\"tool_call_id\":\"tc-1\",\"tool_input\":{\"command\":\"cargo test\"}}}\n",
            "{\"schema_version\":1,\"type\":\"goose_hook\",\"event\":\"PostToolUse\",\"ts_ms\":4,\"dozer_session_id\":\"ds\",\"payload\":{\"tool_name\":\"developer__shell\"}}\n",
            "{\"schema_version\":1,\"type\":\"goose_hook\",\"event\":\"Stop\",\"ts_ms\":5,\"dozer_session_id\":\"ds\",\"payload\":{\"last_assistant_message\":\"修好了\"}}\n",
        );
        let turns = parse_chunk(AgentKind::Goose, text, "conv1", 0);
        // SessionStart 不产出 turn。
        assert_eq!(turns.len(), 4);
        assert_eq!(turns[0].role, "human");
        assert_eq!(turns[0].content, "帮我修一下");
        assert_eq!(turns[1].role, "ai");
        assert_eq!(turns[1].tool_calls, 1);
        assert_eq!(turns[1].tools_summary, vec!["developer__shell cargo test"]);
        assert_eq!(turns[2].role, "tool_result");
        assert!(!turns[2].is_error);
        assert_eq!(turns[3].role, "ai");
        assert_eq!(turns[3].content, "修好了");
    }

    #[test]
    fn goose_post_tool_use_failure_sets_is_error() {
        let text = "{\"schema_version\":1,\"type\":\"goose_hook\",\"event\":\"PostToolUseFailure\",\"ts_ms\":4,\"dozer_session_id\":\"ds\",\"payload\":{\"tool_name\":\"developer__shell\"}}\n";
        let turns = parse_chunk(AgentKind::Goose, text, "conv1", 0);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].role, "tool_result");
        assert!(turns[0].is_error);
        assert!(turns[0].content.contains("失败"));
    }

    #[test]
    fn goose_after_file_edit_sets_mutating_and_files_touched() {
        let text = "{\"schema_version\":1,\"type\":\"goose_hook\",\"event\":\"AfterFileEdit\",\"ts_ms\":5,\"dozer_session_id\":\"ds\",\"payload\":{\"tool_name\":\"developer__edit\",\"matcher_context\":\"/proj/src/main.rs\"}}\n";
        let turns = parse_chunk(AgentKind::Goose, text, "conv1", 0);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].role, "ai");
        assert_eq!(turns[0].mutating_tool_calls, 1);
        assert_eq!(turns[0].files_touched, vec!["/proj/src/main.rs"]);
    }

    #[test]
    fn goose_stop_with_empty_message_produces_no_turn() {
        let text = "{\"schema_version\":1,\"type\":\"goose_hook\",\"event\":\"Stop\",\"ts_ms\":5,\"dozer_session_id\":\"ds\",\"payload\":{}}\n";
        let turns = parse_chunk(AgentKind::Goose, text, "conv1", 0);
        assert!(turns.is_empty());
    }

    #[test]
    fn goose_skips_unknown_schema_version_and_unknown_events() {
        let text = concat!(
            "{\"schema_version\":2,\"type\":\"goose_hook\",\"event\":\"UserPromptSubmit\",\"ts_ms\":1,\"dozer_session_id\":\"ds\",\"payload\":{\"message\":\"未来版本\"}}\n",
            "{\"schema_version\":1,\"type\":\"goose_hook\",\"event\":\"SessionEnd\",\"ts_ms\":2,\"dozer_session_id\":\"ds\",\"payload\":{}}\n",
            "{\"type\":\"some_other_shape\"}\n",
            "not-json\n",
        );
        let turns = parse_chunk(AgentKind::Goose, text, "conv1", 0);
        assert!(
            turns.is_empty(),
            "未知 schema 版本/生命周期事件/畸形行都不产出 turn"
        );
    }

    /// 用真机捕获的一整段 goose 1.51.0 会话(28 条真实事件,`spike/
    /// goose-adapter` 2026-09-26 补的验证捕获,原始事件见
    /// `crates/dozer-hook/fixtures/goose/real-session-hook-events.jsonl`;
    /// 这里手工包一层 journal envelope 而不是依赖 `dozer-hook` 的
    /// `build_journal_line`——`dozerd` 不依赖 `dozer-hook`,`dozer-hook` 按
    /// 设计要保持零依赖,这层重复是刻意的,不是遗漏)确认真实数据整段走
    /// `parse_chunk` 不 panic,且各类事件都产出了预期角色的 turn——尤其是
    /// 缺 `tool_call_id` 的 `AfterFileEdit`/`PostToolUseFailure`,以及裸
    /// `tool_name`(真实是 `shell`/`tree`/`write`,不是文档假设的
    /// `developer__` 前缀,验证 parser 确实不依赖这个前缀)。
    #[test]
    fn goose_parses_a_full_real_captured_session_without_panicking() {
        const RAW: &str =
            include_str!("../../../dozer-hook/fixtures/goose/real-session-hook-events.jsonl");
        let mut journal = String::new();
        for (i, line) in RAW.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let payload: Value = serde_json::from_str(line).expect("fixture 每行都应是合法 JSON");
            let event = payload["event"].as_str().expect("fixture 每行都应有 event");
            journal.push_str(&format!(
                r#"{{"schema_version":1,"type":"goose_hook","event":"{event}","ts_ms":{ts},"dozer_session_id":"ds","payload":{payload}}}"#,
                ts = 1_000 + i as u64,
            ));
            journal.push('\n');
        }
        let turns = parse_chunk(AgentKind::Goose, &journal, "conv1", 0);
        // 28 条真实事件里 SessionStart 不产出 turn,其余 27 条(1 human +
        // 12 PreToolUse + 12 PostToolUse/PostToolUseFailure + 1
        // AfterFileEdit + 1 Stop)都应该产出。
        assert_eq!(turns.len(), 27, "真实事件应逐条产出 turn,不该有静默丢失");
        assert_eq!(turns[0].role, "human");
        assert!(
            turns.iter().any(|t| t.role == "tool_result" && t.is_error),
            "真实捕获里含一次 PostToolUseFailure,必须映射成 is_error 的 tool_result"
        );
        assert!(
            turns.iter().any(|t| t.mutating_tool_calls == 1),
            "真实捕获里含一次 AfterFileEdit,必须计入 mutating_tool_calls"
        );
        assert_eq!(
            turns.last().unwrap().role,
            "ai",
            "最后一条是 Stop,应映射成 ai 角色的 last_assistant_message"
        );
    }

    #[test]
    fn goose_pre_tool_use_trace_detail_keeps_tool_name_and_input() {
        let raw = "{\"schema_version\":1,\"type\":\"goose_hook\",\"event\":\"PreToolUse\",\"ts_ms\":3,\"dozer_session_id\":\"ds\",\"payload\":{\"tool_name\":\"developer__shell\",\"tool_call_id\":\"tc-1\",\"tool_input\":{\"command\":\"cargo test\"}}}";
        let detail = extract_turn_trace_detail(raw, AgentKind::Goose);
        assert_eq!(detail.thinking_text, None);
        assert_eq!(detail.tool_calls.len(), 1);
        assert_eq!(detail.tool_calls[0].summary, "developer__shell cargo test");
        assert!(
            detail.tool_calls[0]
                .input_json
                .as_deref()
                .unwrap()
                .contains("cargo test")
        );
    }

    #[test]
    fn extract_goose_trace_detail_pre_tool_use_reads_call_id() {
        let raw = r#"{"type":"goose_hook","event":"PreToolUse","payload":{"tool_call_id":"tc-1","tool_name":"developer__shell","tool_input":{"command":"cargo test"}}}"#;
        let detail = extract_turn_trace_detail(raw, AgentKind::Goose);
        assert_eq!(detail.tool_calls.len(), 1);
        assert_eq!(detail.tool_calls[0].id.as_deref(), Some("tc-1"));
    }

    #[test]
    fn extract_goose_trace_detail_post_tool_use_reads_call_id() {
        let raw = r#"{"type":"goose_hook","event":"PostToolUse","payload":{"tool_call_id":"tc-1","tool_name":"developer__shell"}}"#;
        let detail = extract_turn_trace_detail(raw, AgentKind::Goose);
        assert_eq!(detail.tool_calls.len(), 0);
        assert_eq!(detail.tool_result_call_id.as_deref(), Some("tc-1"));
    }

    #[test]
    fn extract_goose_trace_detail_post_tool_use_failure_reads_call_id() {
        let raw = r#"{"type":"goose_hook","event":"PostToolUseFailure","payload":{"tool_call_id":"tc-2","tool_name":"developer__shell"}}"#;
        let detail = extract_turn_trace_detail(raw, AgentKind::Goose);
        assert_eq!(detail.tool_result_call_id.as_deref(), Some("tc-2"));
    }

    #[test]
    fn extract_goose_trace_detail_other_events_stay_default() {
        let raw =
            r#"{"type":"goose_hook","event":"Stop","payload":{"last_assistant_message":"done"}}"#;
        let detail = extract_turn_trace_detail(raw, AgentKind::Goose);
        assert_eq!(detail, super::TurnTraceDetail::default());
    }

    #[test]
    fn goose_trace_detail_non_pretooluse_returns_empty() {
        let raw = "{\"schema_version\":1,\"type\":\"goose_hook\",\"event\":\"UserPromptSubmit\",\"ts_ms\":2,\"dozer_session_id\":\"ds\",\"payload\":{\"message\":\"hi\"}}";
        assert_eq!(
            extract_turn_trace_detail(raw, AgentKind::Goose),
            TurnTraceDetail::default()
        );
    }

    // —— Aider canonical parser ——

    #[test]
    fn aider_parses_human_and_ai_with_stable_message_key() {
        let text = concat!(
            "{\"schema_version\":1,\"type\":\"aider_message\",\"message_id\":\"abc\",\"role\":\"human\",\"content\":\"请修复测试\",\"ts_ms\":1}\n",
            "{\"schema_version\":1,\"type\":\"aider_message\",\"message_id\":\"def\",\"role\":\"ai\",\"content\":\"已修复\",\"ts_ms\":2}\n",
        );
        let turns = parse_chunk(AgentKind::Aider, text, "conv1", 0);
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].role, "human");
        assert_eq!(turns[0].content, "请修复测试");
        assert_eq!(turns[0].message_key, "abc", "message_key 取 message_id");
        assert_eq!(turns[0].ts, Some(1));
        assert_eq!(turns[1].role, "ai");
        assert_eq!(turns[1].message_key, "def");
        // 工具/mutation/files/thinking/token 全零。
        assert_eq!(turns[1].tool_calls, 0);
        assert_eq!(turns[1].mutating_tool_calls, 0);
        assert!(turns[1].files_touched.is_empty());
        assert_eq!(turns[1].tokens_in, 0);
    }

    #[test]
    fn aider_skips_unknown_schema_role_and_malformed_lines() {
        let text = concat!(
            "{\"schema_version\":2,\"type\":\"aider_message\",\"message_id\":\"x\",\"role\":\"human\",\"content\":\"未来版本\"}\n",
            "{\"schema_version\":1,\"type\":\"aider_message\",\"message_id\":\"y\",\"role\":\"tool\",\"content\":\"未知角色\"}\n",
            "{\"schema_version\":1,\"type\":\"aider_message\",\"message_id\":\"z\",\"role\":\"human\",\"content\":\"   \"}\n",
            "not-json\n",
            "{\"schema_version\":1,\"type\":\"aider_message\",\"message_id\":\"ok\",\"role\":\"human\",\"content\":\"真实\"}\n",
        );
        let turns = parse_chunk(AgentKind::Aider, text, "conv1", 0);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].message_key, "ok");
    }

    #[test]
    fn aider_trace_detail_returns_empty() {
        let raw = "{\"schema_version\":1,\"type\":\"aider_message\",\"message_id\":\"x\",\"role\":\"ai\",\"content\":\"hi\"}";
        assert_eq!(
            extract_turn_trace_detail(raw, AgentKind::Aider),
            TurnTraceDetail::default()
        );
    }
}
