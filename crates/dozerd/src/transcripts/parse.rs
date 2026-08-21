//! agent transcript(JSONL)增量解析:按 `AgentKind` 分派,产出既带展示
//! 字段又带用量字段的 `ParsedTurn`(合并原 dozer-app `transcript.rs` +
//! `usage.rs` 两套平行解析器,避免长期重复维护——spec"参考调研"一节)。

use dozer_core::protocol::AgentKind;
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
            Some("user") => {
                let Some(text) = v
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_str())
                else {
                    continue;
                };
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
        if v.get("type").and_then(|t| t.as_str()) != Some("message") {
            continue;
        }
        let Some(blocks) = v.get("content").and_then(|c| c.as_array()) else {
            continue;
        };
        let ts = v.get("timestamp").and_then(|t| t.as_u64());
        let message_key = v
            .get("id")
            .and_then(|i| i.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| fallback_key(conversation_id, turn_index));
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
        let text = concat!(
            "{\"type\":\"user\",\"uuid\":\"u1\",\"isMeta\":true,\"message\":{\"role\":\"user\",",
            "\"content\":\"<local-command-caveat>Caveat: ...</local-command-caveat>\"}}\n",
            "{\"type\":\"user\",\"uuid\":\"u2\",\"message\":{\"role\":\"user\",",
            "\"content\":\"<command-name>/clear</command-name>\"}}\n",
            "{\"type\":\"user\",\"uuid\":\"u3\",\"message\":{\"role\":\"user\",",
            "\"content\":\"<local-command-stdout>Set model to Sonnet 5</local-command-stdout>\"}}\n",
            "{\"type\":\"user\",\"uuid\":\"u4\",\"message\":{\"role\":\"user\",",
            "\"content\":\"真正的问题在这里\"}}\n",
        );
        let turns = parse_chunk(AgentKind::Claude, text, "conv1", 0);
        assert_eq!(turns.len(), 1, "只有真实消息应该摄取成回合");
        assert_eq!(turns[0].content, "真正的问题在这里");
        assert_eq!(turns[0].message_key, "u4");
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
