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

/// CodeBuddy transcript(JSONL)独立 schema：顶层 `type:"message"` + `role` +
/// `content[].type: "input_text"/"output_text"`,与 Claude 的
/// `type:"user"/"assistant"` 不兼容,不能复用 `parse_claude_shaped_jsonl`
/// （spec `2026-07-31-codebuddy-spike-findings.md` 实测确认）。混杂的
/// `type:"file-history-snapshot"` 行是快照噪音,原样跳过。样本里没有出现
/// 工具调用/thinking 的等价字段,`tools`/`thinking` 一律留空/false——
/// 等真的观测到再补,不臆测。
fn parse_codebuddy_shaped_jsonl(jsonl: &str) -> Vec<ReviewEntry> {
    let mut out = Vec::new();
    for line in jsonl.lines() {
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
        match v.get("role").and_then(|r| r.as_str()) {
            Some("user") => {
                let text = join_codebuddy_text_blocks(blocks, "input_text");
                if !text.is_empty() {
                    out.push(ReviewEntry::Human { text });
                }
            }
            Some("assistant") => out.push(ReviewEntry::AiTurn {
                text: join_codebuddy_text_blocks(blocks, "output_text"),
                tools: Vec::new(),
                thinking: false,
            }),
            _ => {}
        }
    }
    out
}

/// `content` 数组里 `type == kind` 的块按顺序拼 `text` 字段,多块用换行分隔
/// （与 `parse_claude_shaped_jsonl` 里 assistant 文本块的拼接规则一致）。
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

/// 按 agent 分派 transcript 解析。`Opencode`/`Kilo` 复用 Claude 分支——
/// dozer-hook 代写它们的 transcript 时就是按 Claude 字段形状写的（spec
/// §5.3/§7），不是巧合，且不依赖各自适配层是否已实现。`Unknown` 也复用
/// Claude 分支：老装的 hook 上报的事件 agent 字段恒 `Unknown`，磁盘上
/// 现存的 transcript 事实上全是 Claude 形状。`Codebuddy` 走独立 schema
/// 的解析器（见 `parse_codebuddy_shaped_jsonl`)。`Codex`/`Qoder` 暂时返回
/// 空：真实 transcript schema 待各自 spike 产出 fixture 后另开计划接（见
/// 计划 Global Constraints），在那之前"不产出数据"是唯一诚实的行为，
/// 不是占位符。
pub fn parse_transcript(agent: AgentKind, jsonl: &str) -> Vec<ReviewEntry> {
    match agent {
        AgentKind::Claude | AgentKind::Opencode | AgentKind::Kilo | AgentKind::Unknown => {
            parse_claude_shaped_jsonl(jsonl)
        }
        AgentKind::Codebuddy => parse_codebuddy_shaped_jsonl(jsonl),
        AgentKind::Codex | AgentKind::Qoder | AgentKind::V8agent => Vec::new(),
    }
}

/// 从 transcript 尾部提取最后一次出现的 model id / permissionMode(后
/// 出现的覆盖先出现的,只关心最新值)。model 兼认两种互斥的行形状:
/// Claude(`message.model`)和 Codebuddy(顶层 `providerData.model`,字段名
/// 核对自 `crates/dozer-hook/fixtures/codebuddy-transcript-sample.jsonl`
/// 真实样本)——同一份 transcript 只会是其中一种形状,两条路径共存不冲突,
/// 不需要按 `agent` 分派。`permissionMode` 只有 Claude 形状(`user`/
/// `assistant`/`permission-mode` 三种行的顶层)才有,Codebuddy 没有等价
/// 字段,不提取,`mode` 对 Codebuddy 恒 `None`(Agent 卡片视图据此不渲染
/// Mode 行,不是 bug)。解析失败的行跳过,不中断整体扫描(同
/// `parse_claude_shaped_jsonl` 的既有容错口径)。不像 `parse_transcript`
/// 那样建 `ReviewEntry` 列表,只回三个标量,给 Agent 卡片的实时刷新用
/// (每次 hook 事件都会重跑一次,故意做得比 `parse_transcript` 轻)。
/// 第三个标量 `activity` 是同一次扫描里顺带提取的"最后一句活动摘要"
/// (Agent 卡片"当前工作内容"的兜底数据源,见 `workspace.rs::agent_card`
/// ——优先用 Todo 派发记录的任务标题,拿不到才落到这里),不新开一次
/// 读取:两者读的是同一份 transcript,分两次扫描纯属浪费 IO。`activity`
/// 识别 Claude 形状(`type:"user"`,`message.content` 是字符串;
/// `type:"assistant"`,`message.content` 数组里 `type:"text"` 块拼接)和
/// CodeBuddy 形状(`type:"message"`+`role`,`content[].type:
/// "input_text"/"output_text"`)的人类发言/AI 回复文本,取最后一条、截到
/// 60 字符——卡片一行放不下长句,截断比换行/溢出更可控,不需要精确到字。
pub fn latest_model_mode_and_activity(
    jsonl: &str,
) -> (Option<String>, Option<String>, Option<String>) {
    let mut model = None;
    let mut mode = None;
    let mut activity = None;
    for line in jsonl.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let claude_model = v
            .get("message")
            .and_then(|m| m.get("model"))
            .and_then(|s| s.as_str());
        let codebuddy_model = v
            .get("providerData")
            .and_then(|p| p.get("model"))
            .and_then(|s| s.as_str());
        if let Some(m) = claude_model.or(codebuddy_model) {
            model = Some(m.to_string());
        }
        if let Some(pm) = v.get("permissionMode").and_then(|s| s.as_str()) {
            mode = Some(pm.to_string());
        }
        if let Some(text) = extract_line_activity(&v) {
            activity = Some(truncate_activity(&text));
        }
    }
    (model, mode, activity)
}

/// 单行 → 这行代表的人类发言/AI 回复文本(工具结果/快照噪音/识别不出
/// 的行形状一律 `None`,不当错误)。
fn extract_line_activity(v: &Value) -> Option<String> {
    match v.get("type").and_then(|t| t.as_str()) {
        Some("user") => v
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .map(str::to_string),
        Some("assistant") => {
            let blocks = v
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(|c| c.as_array())?;
            join_text_blocks(blocks, "text")
        }
        Some("message") => {
            let role = v.get("role").and_then(|r| r.as_str())?;
            let kind = match role {
                "user" => "input_text",
                "assistant" => "output_text",
                _ => return None,
            };
            let blocks = v.get("content").and_then(|c| c.as_array())?;
            join_text_blocks(blocks, kind)
        }
        _ => None,
    }
}

fn join_text_blocks(blocks: &[Value], kind: &str) -> Option<String> {
    let mut text = String::new();
    for b in blocks {
        if b.get("type").and_then(|t| t.as_str()) == Some(kind)
            && let Some(t) = b.get("text").and_then(|t| t.as_str())
        {
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(t);
        }
    }
    if text.is_empty() { None } else { Some(text) }
}

/// 截到首行、最多 60 字符,超长补 `…`。
fn truncate_activity(text: &str) -> String {
    let first_line = text.lines().next().unwrap_or(text).trim();
    let truncated: String = first_line.chars().take(60).collect();
    if first_line.chars().count() > 60 {
        format!("{truncated}…")
    } else {
        truncated
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
    fn kilo_reuses_claude_shaped_parser() {
        // Kilo 的 transcript 由 dozer-hook 代写成 Claude 形状（跟 OpenCode
        // 同理，见 spec §7），不依赖 Kilo 插件本身是否已实现。
        let jsonl = r#"{"type":"user","message":{"role":"user","content":"kilo 里也这么解析"}}"#;
        let entries = parse_transcript(AgentKind::Kilo, jsonl);
        assert_eq!(
            entries,
            vec![ReviewEntry::Human {
                text: "kilo 里也这么解析".into()
            }]
        );
    }

    #[test]
    fn codex_and_qoder_yield_empty_until_schema_confirmed() {
        let jsonl = r#"{"type":"user","message":{"role":"user","content":"应该被忽略"}}"#;
        assert!(parse_transcript(AgentKind::Codex, jsonl).is_empty());
        assert!(parse_transcript(AgentKind::Qoder, jsonl).is_empty());
        assert!(parse_transcript(AgentKind::V8agent, jsonl).is_empty());
    }

    #[test]
    fn codebuddy_parses_real_fixture_sample() {
        // 真实脱敏样本，见 docs/superpowers/specs/2026-07-31-codebuddy-spike-findings.md。
        // include_str! 直接读已入库文件，避免测试数据和真实 fixture 走漂。
        let jsonl = include_str!("../../dozer-hook/fixtures/codebuddy-transcript-sample.jsonl");
        let entries = parse_transcript(AgentKind::Codebuddy, jsonl);
        assert_eq!(
            entries.len(),
            2,
            "1 用户消息 + 1 assistant 消息;中间的 file-history-snapshot 行应被跳过"
        );
        assert_eq!(
            entries[0],
            ReviewEntry::Human {
                text: "reply with exactly one word: hello".into()
            }
        );
        assert_eq!(
            entries[1],
            ReviewEntry::AiTurn {
                text: "hello".into(),
                tools: Vec::new(),
                thinking: false,
            }
        );
    }

    #[test]
    fn codebuddy_joins_multiple_text_blocks_and_skips_other_kinds() {
        let jsonl = concat!(
            "{\"type\":\"message\",\"role\":\"user\",\"content\":",
            "[{\"type\":\"input_text\",\"text\":\"第一段\"},",
            "{\"type\":\"input_text\",\"text\":\"第二段\"}]}\n",
            "{\"type\":\"message\",\"role\":\"assistant\",\"content\":",
            "[{\"type\":\"output_text\",\"text\":\"回复一\"}],\"status\":\"completed\"}\n",
            "不是 json 的坏行\n",
            "{\"type\":\"message\",\"role\":\"tool\",\"content\":[]}\n"
        );
        let entries = parse_transcript(AgentKind::Codebuddy, jsonl);
        assert_eq!(
            entries,
            vec![
                ReviewEntry::Human {
                    text: "第一段\n第二段".into()
                },
                ReviewEntry::AiTurn {
                    text: "回复一".into(),
                    tools: Vec::new(),
                    thinking: false,
                },
            ],
            "多个 input_text/output_text 块按顺序拼接;坏行与未知 role 跳过"
        );
    }

    #[test]
    fn codebuddy_user_row_without_input_text_block_yields_no_human_entry() {
        // role:"user" 行的 content 里没有 input_text 类型的块（结构上可能，
        // 只是当前 fixture 里没观测到）——join 出来是空字符串，不该推入一个
        // 空 Human 气泡（见 final review finding：要跟 conversation.rs 的
        // codebuddy_shaped_title 弃权行为对齐）。
        let jsonl = concat!(
            "{\"type\":\"message\",\"role\":\"user\",\"content\":",
            "[{\"type\":\"something_else\",\"text\":\"x\"}]}\n",
            "{\"type\":\"message\",\"role\":\"assistant\",\"content\":",
            "[{\"type\":\"output_text\",\"text\":\"回复\"}]}\n"
        );
        let entries = parse_transcript(AgentKind::Codebuddy, jsonl);
        assert_eq!(
            entries,
            vec![ReviewEntry::AiTurn {
                text: "回复".into(),
                tools: Vec::new(),
                thinking: false,
            }],
            "user 行没有 input_text 块时不产出空 Human 气泡，直接跳过该行"
        );
    }

    #[test]
    fn codebuddy_empty_and_all_noise_yield_nothing() {
        assert!(parse_transcript(AgentKind::Codebuddy, "").is_empty());
        assert!(
            parse_transcript(
                AgentKind::Codebuddy,
                "{\"type\":\"file-history-snapshot\"}\nbad\n"
            )
            .is_empty()
        );
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

    #[test]
    fn latest_model_and_mode_picks_last_occurrence() {
        let jsonl = r#"
{"type":"user","message":{"role":"user","content":"改一下 README"},"permissionMode":"plan"}
{"type":"assistant","message":{"role":"assistant","content":[],"model":"claude-sonnet-5"}}
{"type":"permission-mode","permissionMode":"auto"}
{"type":"assistant","message":{"role":"assistant","content":[],"model":"claude-opus-5"}}
"#;
        let (model, mode, _activity) = latest_model_mode_and_activity(jsonl);
        assert_eq!(model.as_deref(), Some("claude-opus-5"));
        assert_eq!(mode.as_deref(), Some("auto"));
    }

    #[test]
    fn latest_model_and_mode_skips_noise_and_bad_lines() {
        let jsonl = "{\"type\":\"mode\",\"mode\":\"normal\"}\n不是 json 的坏行\n{\"type\":\"attachment\",\"attachment\":{}}\n";
        let (model, mode, _activity) = latest_model_mode_and_activity(jsonl);
        assert_eq!((model, mode), (None, None));
    }

    #[test]
    fn latest_model_and_mode_empty_input_yields_none() {
        let (model, mode, activity) = latest_model_mode_and_activity("");
        assert_eq!((model, mode, activity), (None, None, None));
    }

    #[test]
    fn latest_model_and_mode_reads_codebuddy_provider_data_model() {
        // Codebuddy 形状(顶层 providerData.model,不是 message.model)——真实
        // 字段名/路径核对自 crates/dozer-hook/fixtures/
        // codebuddy-transcript-sample.jsonl。Codebuddy 没有 permissionMode
        // 等价字段,mode 应保持 None。
        let jsonl = concat!(
            "{\"type\":\"message\",\"role\":\"user\",\"content\":[]}\n",
            "{\"type\":\"message\",\"role\":\"assistant\",\"content\":[],",
            "\"providerData\":{\"model\":\"glm-5.2\",\"requestModelId\":\"glm-5.2\"}}\n",
        );
        let (model, mode, _activity) = latest_model_mode_and_activity(jsonl);
        assert_eq!(model.as_deref(), Some("glm-5.2"));
        assert_eq!(mode, None);
    }

    #[test]
    fn latest_model_and_mode_last_occurrence_across_mixed_shapes() {
        // message.model(Claude 形状)和 providerData.model(Codebuddy 形状)
        // 互斥,不会同一行出现——但函数本身不按 agent 分派,这里验证两种
        // 形状各自都命中时仍按"最后出现覆盖"处理(同一份 transcript 实际
        // 只会是其中一种形状,这个用例只是确认两条提取路径互不干扰)。
        let jsonl = concat!(
            "{\"type\":\"assistant\",\"message\":{\"model\":\"claude-sonnet-5\"}}\n",
            "{\"type\":\"message\",\"role\":\"assistant\",",
            "\"providerData\":{\"model\":\"glm-5.2\"}}\n",
        );
        let (model, _mode, _activity) = latest_model_mode_and_activity(jsonl);
        assert_eq!(model.as_deref(), Some("glm-5.2"));
    }

    #[test]
    fn activity_picks_last_human_or_ai_text_claude_shaped() {
        let jsonl = concat!(
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"改一下 README\"}}\n",
            "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"好的，我来改\"}]}}\n",
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"再加一段安装说明\"}}\n",
        );
        let (_model, _mode, activity) = latest_model_mode_and_activity(jsonl);
        assert_eq!(activity.as_deref(), Some("再加一段安装说明"));
    }

    #[test]
    fn activity_ignores_tool_result_content_arrays() {
        // content 是数组(工具结果)而不是字符串的 user 行不算"人类发言",
        // 不该被当成 activity 摘要。
        let jsonl = concat!(
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"写个测试\"}}\n",
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"tool_result\",\"content\":\"ok\"}]}}\n",
        );
        let (_model, _mode, activity) = latest_model_mode_and_activity(jsonl);
        assert_eq!(activity.as_deref(), Some("写个测试"));
    }

    #[test]
    fn activity_reads_codebuddy_shaped_last_message() {
        let jsonl = concat!(
            "{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"修一下光标问题\"}]}\n",
            "{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"已定位到问题\"}]}\n",
        );
        let (_model, _mode, activity) = latest_model_mode_and_activity(jsonl);
        assert_eq!(activity.as_deref(), Some("已定位到问题"));
    }

    #[test]
    fn activity_truncates_long_first_line_to_60_chars() {
        let long = "a".repeat(80);
        let jsonl = format!(
            "{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":\"{long}\"}}}}\n"
        );
        let (_model, _mode, activity) = latest_model_mode_and_activity(&jsonl);
        let got = activity.expect("should extract activity");
        assert_eq!(got.chars().count(), 61, "60 字符 + 省略号");
        assert!(got.ends_with('…'));
    }

    #[test]
    fn activity_none_when_transcript_has_no_recognizable_message() {
        let jsonl = "{\"type\":\"attachment\",\"attachment\":{}}\n";
        let (_model, _mode, activity) = latest_model_mode_and_activity(jsonl);
        assert_eq!(activity, None);
    }
}
