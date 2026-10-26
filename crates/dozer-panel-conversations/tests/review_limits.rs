use dozer_model::TurnRecord;
use dozer_panel_conversations::review::{ReviewEntry, review_entries_from_turns};

fn turn(role: &str, content: String) -> TurnRecord {
    serde_json::from_value(serde_json::json!({
        "turn_index": 0, "role": role, "content": content,
        "thinking": false, "ts": null
    }))
    .unwrap()
}

#[test]
fn empty_turns_serialize_as_an_empty_review() {
    let entries = review_entries_from_turns(&[]);
    assert!(entries.is_empty());
    assert_eq!(serde_json::to_string(&entries).unwrap(), "[]");
}

#[test]
fn ten_thousand_trajectory_records_fold_without_losing_tools_or_tokens() {
    let mut turns = Vec::with_capacity(10_001);
    for index in 0..10_000 {
        let mut record = turn("ai", String::new());
        record.turn_index = index;
        record.tokens_in = 1;
        record.tokens_out = 2;
        record.tool_calls.push(dozer_model::ToolCallInfo {
            summary: format!("Read {index}"),
            input_json: None,
            id: Some(format!("call-{index}")),
        });
        turns.push(record);
    }
    turns.push(turn("ai", "完成".into()));
    let entries = review_entries_from_turns(&turns);
    let [ReviewEntry::AiTurn {
        text,
        tool_calls,
        tokens_in,
        tokens_out,
        ..
    }] = entries.as_slice()
    else {
        panic!("纯轨迹与最终正文应折叠成同一个回合");
    };
    assert_eq!(text, "完成");
    assert_eq!(tool_calls.len(), 10_000);
    assert_eq!(tool_calls.first().unwrap().id.as_deref(), Some("call-0"));
    assert_eq!(tool_calls.last().unwrap().id.as_deref(), Some("call-9999"));
    assert_eq!((*tokens_in, *tokens_out), (10_000, 20_000));
}

#[test]
fn a_multi_megabyte_unicode_turn_survives_review_json_roundtrip() {
    let content = "汉🙂\u{fffd}\u{200b}\n".repeat(160_000);
    assert!(content.len() > 2 * 1024 * 1024);
    let entries = review_entries_from_turns(&[turn("human", content.clone())]);
    let [ReviewEntry::Human { text }] = entries.as_slice() else {
        panic!("单条人类发言应保留为一个条目");
    };
    assert_eq!(text, &content);
    let json = serde_json::to_string(&entries).unwrap();
    let decoded: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded[0]["Human"]["text"].as_str(), Some(content.as_str()));
}
