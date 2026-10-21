//! transcript 尾部解析(从 `dozer-app/src/transcript.rs` 搬入,S6c-2):
//! 从 JSONL 尾部提取最近一次 model / permissionMode / 人类发言摘要。

use serde_json::Value;

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
        // Codex 形状:model 不挂在消息行上,是独立的 `turn_context` 行
        // (顶层 `type:"turn_context"`),嵌套在 `payload.model`。
        let codex_model = (v.get("type").and_then(|t| t.as_str()) == Some("turn_context"))
            .then(|| v.get("payload").and_then(|p| p.get("model")))
            .flatten()
            .and_then(|s| s.as_str());
        if let Some(m) = claude_model.or(codebuddy_model).or(codex_model) {
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
        // Codex 形状:`response_item`/`payload.type:"message"`,人类发言
        // 是 `payload.role:"user"`(还有 "developer"/"assistant" 两种角色,
        // "developer" 是 CLI 自己注入的系统提示片段,不算真实用户发言,
        // 不提取)。
        Some("response_item") => {
            let payload = v.get("payload")?;
            if payload.get("type").and_then(|t| t.as_str()) != Some("message") {
                return None;
            }
            if payload.get("role").and_then(|r| r.as_str()) != Some("user") {
                return None;
            }
            let blocks = payload.get("content").and_then(|c| c.as_array())?;
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
    fn latest_model_and_mode_reads_codex_turn_context_model() {
        // Codex 形状:model 不在消息行上,而是独立的 `turn_context` 行,
        // 顶层 `type`,嵌套在 `payload.model`——跟 Claude(`message.model`)/
        // Codebuddy(顶层 `providerData.model`)都不同。字段路径核对自
        // 2026-09-21 实读本机 `~/.codex/sessions/**/rollout-*.jsonl`。
        // Codex 没有 permissionMode 等价字段,mode 应保持 None。
        let jsonl = concat!(
            "{\"type\":\"session_meta\",\"payload\":{\"model_provider\":\"openai\"}}\n",
            "{\"type\":\"turn_context\",\"payload\":{\"model\":\"gpt-6-astra\"}}\n",
        );
        let (model, mode, _activity) = latest_model_mode_and_activity(jsonl);
        assert_eq!(model.as_deref(), Some("gpt-6-astra"));
        assert_eq!(mode, None);
    }

    #[test]
    fn activity_reads_codex_shaped_last_human_message_and_skips_developer_role() {
        // Codex 的 `response_item`/`payload.type:"message"` 里,`role` 除了
        // user/assistant 还有 "developer"(CLI 自己注入的系统提示片段,
        // 等价于 Claude 的 isMeta 消息)——activity 只认 role:"user",
        // "developer" 即使排在后面也不该盖掉真实用户发言。
        let jsonl = concat!(
            "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",",
            "\"content\":[{\"type\":\"input_text\",\"text\":\"修一下光标问题\"}]}}\n",
            "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"developer\",",
            "\"content\":[{\"type\":\"input_text\",\"text\":\"系统提示词片段\"}]}}\n",
        );
        let (_model, _mode, activity) = latest_model_mode_and_activity(jsonl);
        assert_eq!(activity.as_deref(), Some("修一下光标问题"));
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

#[cfg(test)]
mod hostile_tests {
    use super::*;

    #[test]
    fn latest_model_mode_and_activity_survives_hostile_input() {
        // BOM 行本身解析失败会被跳过,但不得影响其后的正常行。
        let bom = "\u{feff}{bad}\r\n{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"hi\"}}\r\n";
        let (_, _, activity) = latest_model_mode_and_activity(bom);
        assert_eq!(activity.as_deref(), Some("hi"));

        let bad = "not json\n{\"type\":\"user\"}\n{broken\n";
        assert_eq!(latest_model_mode_and_activity(bad), (None, None, None));

        let huge = format!(
            "{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":\"{}\"}}}}\n",
            "x".repeat(10 * 1024 * 1024)
        );
        let (_, _, activity) = latest_model_mode_and_activity(&huge);
        assert_eq!(activity.map(|s| s.chars().count()), Some(61));

        // 只有 AI 回复:activity 必须是 None(只认人类发言)。
        let ai_only = "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"done\"}]}}\n";
        let (_, _, activity) = latest_model_mode_and_activity(ai_only);
        assert_eq!(activity, None);

        assert_eq!(latest_model_mode_and_activity(""), (None, None, None));
    }

    #[test]
    fn truncate_activity_counts_chars_not_bytes() {
        let sixty: String = "汉".repeat(60);
        assert_eq!(truncate_activity(&sixty), sixty);
        let sixty_one: String = "汉".repeat(61);
        let got = truncate_activity(&sixty_one);
        assert_eq!(got.chars().count(), 61);
        assert!(got.ends_with('…'));
        assert_eq!(truncate_activity("first\nsecond"), "first");
        assert_eq!(truncate_activity("   \n second"), "");
    }
}
