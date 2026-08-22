//! Claude Code transcript(JSONL)适配器（P1i）。spec 2026-08-20 起回合明细
//! 改为直接消费 dozerd 查询回来的 `TurnRecord`（`review_entries_from_turns`），
//! 不再在 app 侧碰磁盘 JSONL。本文件保留的 `latest_model_mode_and_activity`
//! 系列仍直接读 transcript 原始文本（Agent 卡片实时指示器用），不在本次
//! 迁移范围。纯函数，不碰 iced/IO。

use dozer_core::protocol::{ToolCallInfo, TurnRecord};
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum ReviewEntry {
    /// 人类发言（导航锚点）。
    Human { text: String },
    /// AI 一个回合：正文 + 真实思考文本 + 结构化工具调用 + 紧跟它的工具
    /// 结果(2026-08-22 起按位置邻接折叠进来,不再是顶层独立条目——同一
    /// `AiTurn` 之后、下一个 `Human`/`AiTurn` 之前的连续 `ToolResult` 都
    /// 算它的,见 `review_entries_from_turns`)。
    AiTurn {
        text: String,
        thinking_text: Option<String>,
        tool_calls: Vec<ToolCallInfo>,
        tool_results: Vec<ToolResultEntry>,
    },
    /// 孤儿兜底:前面没有 `AiTurn` 的 `ToolResult`(理论边界情况,如导出
    /// 片段从工具结果行开始)。正常情况下 `ToolResult` 都会被折叠进
    /// 上面 `AiTurn::tool_results`,这个顶层变体只在没有归属对象时才用。
    ToolResult { content: String, is_error: bool },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ToolResultEntry {
    pub content: String,
    pub is_error: bool,
}

/// dozerd 查询回来的回合明细 → 面板展示用的 `ReviewEntry`。顺序 fold 而
/// 不是逐条 map:`tool_result` 角色的行按位置邻接归到紧邻它前面那个
/// `AiTurn` 的 `tool_results`(`out.last_mut()` 还是同一个 `AiTurn` 就
/// 一直往里塞),前面没有 `AiTurn`(比如会话/导出片段从工具结果开始)才
/// 落回顶层 `ReviewEntry::ToolResult`。
pub fn review_entries_from_turns(turns: &[TurnRecord]) -> Vec<ReviewEntry> {
    let mut out: Vec<ReviewEntry> = Vec::new();
    for t in turns {
        match t.role.as_str() {
            "human" => out.push(ReviewEntry::Human {
                text: t.content.clone(),
            }),
            "tool_result" => {
                let entry = ToolResultEntry {
                    content: t.content.clone(),
                    is_error: t.is_error,
                };
                match out.last_mut() {
                    Some(ReviewEntry::AiTurn { tool_results, .. }) => {
                        tool_results.push(entry);
                    }
                    _ => out.push(ReviewEntry::ToolResult {
                        content: entry.content,
                        is_error: entry.is_error,
                    }),
                }
            }
            _ => out.push(ReviewEntry::AiTurn {
                text: t.content.clone(),
                thinking_text: t.thinking_text.clone(),
                tool_calls: t.tool_calls.clone(),
                tool_results: Vec::new(),
            }),
        }
    }
    out
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
/// 第三个标量 `activity` 是同一次扫描里顺带提取的"最后一句人类发言"
/// (Agent 卡片"当前工作内容"的兜底数据源,见 `workspace.rs::agent_card`
/// ——优先用 Todo 派发记录的任务标题,拿不到才落到这里),不新开一次
/// 读取:两者读的是同一份 transcript,分两次扫描纯属浪费 IO。`activity`
/// **只认人类发言,不认 AI 回复**(用户实测反馈:两者都认时,回合结束
/// 后"最后一行"常是 AI 的收尾总结,看不出这一轮到底在做什么——人类的
/// 原始请求比 AI 的总结更能说明"当前在干什么")。识别 Claude 形状
/// (`type:"user"`,`message.content` 是字符串)和 CodeBuddy 形状
/// (`type:"message"`+`role:"user"`,`content[].type:"input_text"`)的
/// 人类发言文本,取最后一条、截到 60 字符——卡片一行放不下长句,截断
/// 比换行/溢出更可控,不需要精确到字。
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
/// 只认人类发言,不认 AI 回复——"当前工作内容"要展示的是人类发起的
/// 那句话(用户实测反馈:之前两种行都认,导致回合结束后"最后一行"常常
/// 是 AI 的收尾总结,看不出这一轮到底在做什么;人类的原始请求比 AI 的
/// 总结更能说明"当前在干什么")。
fn extract_line_activity(v: &Value) -> Option<String> {
    match v.get("type").and_then(|t| t.as_str()) {
        Some("user") => v
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .map(str::to_string),
        Some("message") => {
            let role = v.get("role").and_then(|r| r.as_str())?;
            if role != "user" {
                return None;
            }
            let blocks = v.get("content").and_then(|c| c.as_array())?;
            join_text_blocks(blocks, "input_text")
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
    fn review_entry_serializes_with_external_tag_shape() {
        let human = ReviewEntry::Human {
            text: "你好".into(),
        };
        assert_eq!(
            serde_json::to_string(&human).unwrap(),
            r#"{"Human":{"text":"你好"}}"#
        );

        let ai = ReviewEntry::AiTurn {
            text: "回复".into(),
            thinking_text: Some("先想想".into()),
            tool_calls: vec![ToolCallInfo {
                summary: "Edit README.md".into(),
                input_json: Some("{\"file_path\":\"README.md\"}".into()),
            }],
            tool_results: Vec::new(),
        };
        let json = serde_json::to_string(&ai).unwrap();
        assert!(json.starts_with(r#"{"AiTurn":"#));
        assert!(json.contains(r#""thinking_text":"#));
        assert!(json.contains(r#""Edit README.md""#));

        let tool = ReviewEntry::ToolResult {
            content: "boom".into(),
            is_error: true,
        };
        assert_eq!(
            serde_json::to_string(&tool).unwrap(),
            r#"{"ToolResult":{"content":"boom","is_error":true}}"#
        );
    }

    #[test]
    fn review_entries_from_turns_maps_ai_turn_with_thinking_and_tool_calls() {
        use dozer_core::protocol::{ToolCallInfo, TurnRecord};
        let turns = vec![
            TurnRecord {
                turn_index: 0,
                role: "human".into(),
                content: "你好".into(),
                tool_calls: vec![],
                thinking: false,
                thinking_text: None,
                ts: None,
                is_error: false,
            },
            TurnRecord {
                turn_index: 1,
                role: "ai".into(),
                content: "回复".into(),
                tool_calls: vec![ToolCallInfo {
                    summary: "Edit README.md".into(),
                    input_json: Some("{\"file_path\":\"README.md\"}".into()),
                }],
                thinking: true,
                thinking_text: Some("先看看现有实现".into()),
                ts: None,
                is_error: false,
            },
        ];
        let entries = review_entries_from_turns(&turns);
        assert_eq!(entries.len(), 2);
        assert_eq!(
            entries[0],
            ReviewEntry::Human {
                text: "你好".into()
            }
        );
        match &entries[1] {
            ReviewEntry::AiTurn {
                text,
                thinking_text,
                tool_calls,
                tool_results,
            } => {
                assert_eq!(text, "回复");
                assert_eq!(thinking_text.as_deref(), Some("先看看现有实现"));
                assert_eq!(tool_calls.len(), 1);
                assert_eq!(tool_calls[0].summary, "Edit README.md");
                assert!(tool_results.is_empty());
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn review_entries_from_turns_nests_consecutive_tool_results_under_preceding_ai_turn() {
        use dozer_core::protocol::TurnRecord;
        fn ai(content: &str) -> TurnRecord {
            TurnRecord {
                turn_index: 0,
                role: "ai".into(),
                content: content.into(),
                tool_calls: vec![],
                thinking: false,
                thinking_text: None,
                ts: None,
                is_error: false,
            }
        }
        fn tool_result(content: &str, is_error: bool) -> TurnRecord {
            TurnRecord {
                turn_index: 0,
                role: "tool_result".into(),
                content: content.into(),
                tool_calls: vec![],
                thinking: false,
                thinking_text: None,
                ts: None,
                is_error,
            }
        }
        let turns = vec![
            ai("第一轮"),
            tool_result("ok1", false),
            tool_result("boom", true),
            ai("第二轮"),
            tool_result("ok2", false),
        ];
        let entries = review_entries_from_turns(&turns);
        assert_eq!(entries.len(), 2);
        match &entries[0] {
            ReviewEntry::AiTurn {
                text, tool_results, ..
            } => {
                assert_eq!(text, "第一轮");
                assert_eq!(
                    tool_results,
                    &vec![
                        ToolResultEntry {
                            content: "ok1".into(),
                            is_error: false
                        },
                        ToolResultEntry {
                            content: "boom".into(),
                            is_error: true
                        },
                    ]
                );
            }
            other => panic!("{other:?}"),
        }
        match &entries[1] {
            ReviewEntry::AiTurn {
                text, tool_results, ..
            } => {
                assert_eq!(text, "第二轮");
                assert_eq!(
                    tool_results,
                    &vec![ToolResultEntry {
                        content: "ok2".into(),
                        is_error: false
                    }]
                );
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn review_entries_from_turns_keeps_orphan_tool_result_at_top_level() {
        use dozer_core::protocol::TurnRecord;
        let turns = vec![TurnRecord {
            turn_index: 0,
            role: "tool_result".into(),
            content: "ok output".into(),
            tool_calls: vec![],
            thinking: false,
            thinking_text: None,
            ts: None,
            is_error: false,
        }];
        let entries = review_entries_from_turns(&turns);
        assert_eq!(
            entries,
            vec![ReviewEntry::ToolResult {
                content: "ok output".into(),
                is_error: false,
            }]
        );
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
    fn activity_picks_last_human_message_not_ai_reply_claude_shaped() {
        // 人类发一句 → AI 回一句总结 → 人类再发一句:activity 应该是最后
        // 那句人类发言,即使 AI 的总结在 transcript 里排在它前面。
        let jsonl = concat!(
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"改一下 README\"}}\n",
            "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"好的，我来改\"}]}}\n",
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"再加一段安装说明\"}}\n",
            "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"已加完，任务结束\"}]}}\n",
        );
        let (_model, _mode, activity) = latest_model_mode_and_activity(jsonl);
        assert_eq!(
            activity.as_deref(),
            Some("再加一段安装说明"),
            "AI 的收尾总结不该盖掉人类的原始请求"
        );
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
    fn activity_reads_codebuddy_shaped_last_human_message() {
        let jsonl = concat!(
            "{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"修一下光标问题\"}]}\n",
            "{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"已定位到问题\"}]}\n",
        );
        let (_model, _mode, activity) = latest_model_mode_and_activity(jsonl);
        assert_eq!(
            activity.as_deref(),
            Some("修一下光标问题"),
            "CodeBuddy 形状同样只认人类发言,不认 AI 回复"
        );
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
