//! Claude Code transcript(JSONL)适配器（P1i）：逐行解析成结构化审阅条目。
//! 纯函数,不碰 iced/IO。规则见 spec P1i D2。
// 过渡期:T5 审阅 tab 接线前无调用方（沿用先例）。
#![allow(dead_code)]

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

pub fn parse_transcript(jsonl: &str) -> Vec<ReviewEntry> {
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
        let entries = parse_transcript(jsonl);
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
        assert!(parse_transcript("").is_empty());
        assert!(parse_transcript("{\"type\":\"mode\"}\nbad\n").is_empty());
    }
}
