//! Claude Code transcript(JSONL)适配器（P1i）：逐行解析成结构化审阅条目。
//! 纯函数,不碰 iced/IO。规则见 spec P1i D2。

use dozer_core::protocol::AgentKind;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub enum ReviewEntry {
    /// 人类发言（导航锚点）。
    Human { text: String },
    /// AI 一个回合：正文 + 工具一行摘要 + 是否含 thinking。
    AiTurn {
        text: String,
        tools: Vec<String>,
        thinking: bool,
    },
}

/// 一个 tool_use 块 → 一行摘要 `<name> <主参>`（spec P1i D2）。
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

fn parse_claude_shaped_jsonl(jsonl: &str) -> Vec<ReviewEntry> {
    let mut out = Vec::new();
    for line in jsonl.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        match v.get("type").and_then(|t| t.as_str()) {
            Some("user") => {
                // content 是字符串 → 人类发言;是 list(工具结果) → 跳过
                if let Some(text) = v
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_str())
                {
                    out.push(ReviewEntry::Human {
                        text: text.to_string(),
                    });
                }
            }
            Some("assistant") => {
                let Some(blocks) = v
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_array())
                else {
                    continue;
                };
                let mut text = String::new();
                let mut tools = Vec::new();
                let mut thinking = false;
                for b in blocks {
                    match b.get("type").and_then(|t| t.as_str()) {
                        Some("text") => {
                            if let Some(t) = b.get("text").and_then(|t| t.as_str()) {
                                if !text.is_empty() {
                                    text.push('\n');
                                }
                                text.push_str(t);
                            }
                        }
                        Some("tool_use") => {
                            let name = b.get("name").and_then(|n| n.as_str()).unwrap_or("工具");
                            let input = b.get("input").cloned().unwrap_or(Value::Null);
                            tools.push(tool_summary(name, &input));
                        }
                        Some("thinking") => thinking = true,
                        _ => {}
                    }
                }
                out.push(ReviewEntry::AiTurn {
                    text,
                    tools,
                    thinking,
                });
            }
            _ => {}
        }
    }
    out
}

/// 按 agent 分派 transcript 解析。`Opencode` 复用 Claude 分支——
/// dozer-hook 代写 OpenCode 的 transcript 时就是按 Claude 字段形状写的
/// （spec §5.3），不是巧合。`Unknown` 也复用 Claude 分支：老装的 hook（还没
/// 重跑 `dozer-hook install`）上报的每个事件 agent 字段都是 `Unknown`，
/// 而在 CodeBuddy/OpenCode 真正进入用户机器之前，磁盘上现存的 transcript
/// 事实上全是 Claude 形状——保守地假定 Unknown 就是 Claude 形状，比直接
/// 返回空白审阅面板是严格更好的猜测。`Codebuddy` 暂时返回空：CodeBuddy
/// 真实 transcript schema 待独立的适配计划验证后再接（spec §6），在那之前
/// "不产出数据"是唯一诚实的行为，不是占位符。
pub fn parse_transcript(agent: AgentKind, jsonl: &str) -> Vec<ReviewEntry> {
    match agent {
        AgentKind::Claude | AgentKind::Opencode | AgentKind::Unknown => {
            parse_claude_shaped_jsonl(jsonl)
        }
        AgentKind::Codebuddy => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_human_and_ai_turn_skipping_noise() {
        let jsonl = r#"
{"type":"mode","mode":"x"}
{"type":"user","message":{"role":"user","content":"改一下 README"}}
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"ok"}]}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"..."},{"type":"text","text":"好的，我来改。"},{"type":"tool_use","name":"Edit","input":{"file_path":"/r/README.md"}},{"type":"tool_use","name":"Bash","input":{"command":"cargo test --all"}}]}}
不是 json 的坏行
{"type":"attachment","attachment":{}}
"#;
        let entries = parse_transcript(AgentKind::Claude, jsonl);
        assert_eq!(
            entries.len(),
            2,
            "1 人类 + 1 AI 回合;工具结果/噪音/坏行跳过"
        );
        assert_eq!(
            entries[0],
            ReviewEntry::Human {
                text: "改一下 README".into()
            }
        );
        match &entries[1] {
            ReviewEntry::AiTurn {
                text,
                tools,
                thinking,
            } => {
                assert_eq!(text, "好的，我来改。");
                assert!(*thinking);
                assert_eq!(
                    tools,
                    &vec![
                        "Edit README.md".to_string(),
                        "Bash cargo test --all".to_string()
                    ]
                );
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn empty_and_all_noise_yield_nothing() {
        assert!(parse_transcript(AgentKind::Claude, "").is_empty());
        assert!(parse_transcript(AgentKind::Claude, "{\"type\":\"mode\"}\nbad\n").is_empty());
    }

    #[test]
    fn opencode_reuses_claude_shaped_parser() {
        let jsonl =
            r#"{"type":"user","message":{"role":"user","content":"opencode 里也这么解析"}}"#;
        let entries = parse_transcript(AgentKind::Opencode, jsonl);
        assert_eq!(
            entries,
            vec![ReviewEntry::Human {
                text: "opencode 里也这么解析".into()
            }]
        );
    }

    #[test]
    fn codebuddy_yields_empty_until_schema_confirmed() {
        let jsonl = r#"{"type":"user","message":{"role":"user","content":"应该被忽略"}}"#;
        assert!(parse_transcript(AgentKind::Codebuddy, jsonl).is_empty());
    }

    #[test]
    fn unknown_falls_back_to_claude_shaped_parser() {
        let jsonl = r#"
{"type":"user","message":{"role":"user","content":"改一下 README"}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"好的，我来改。"},{"type":"tool_use","name":"Edit","input":{"file_path":"/r/README.md"}}]}}
"#;
        let unknown_entries = parse_transcript(AgentKind::Unknown, jsonl);
        let claude_entries = parse_transcript(AgentKind::Claude, jsonl);
        assert_eq!(
            unknown_entries, claude_entries,
            "老装 hook 上报的 Unknown agent 应该和 Claude 解析出一样的条目，\
             而不是空白审阅面板"
        );
        assert_eq!(unknown_entries.len(), 2);
    }
}
