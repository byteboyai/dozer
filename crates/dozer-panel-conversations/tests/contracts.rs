use dozer_model::{AgentKind, SessionRow, TurnRecord};
use dozer_panel_conversations::{assets::*, filter::*, review::*, snapshot::*};
use sha2::{Digest, Sha256};

fn review() -> ReviewView {
    ReviewView {
        source: ReviewSource::Conversation("c1".into()),
        entries: vec![ReviewEntry::Human {
            text: "hello".into(),
        }],
        error: None,
        agent: AgentKind::Claude,
        nonce: 42,
        summary_title: None,
        summary_text: None,
        summary_time: None,
        loaded_nonce: None,
    }
}
#[test]
fn review_snapshot_through_the_library_matches_the_golden() {
    let turns: Vec<TurnRecord> = serde_json::from_str(include_str!(
        "../../dozer-app/tests/fixtures/review_turns_input.json"
    ))
    .unwrap();
    let entries = review_entries_from_turns(&turns);
    let snapshot = ReviewSnapshot {
        entries: &entries,
        agent_label: AgentKind::Claude.label(),
        summary_title: Some("审阅 👋".into()),
        summary_text: Some("第一行\n第二行 �\u{200b}".into()),
        summary_time: Some("3 分钟前".into()),
    };
    assert_eq!(
        serde_json::to_string(&snapshot).unwrap(),
        include_str!("../../dozer-app/tests/fixtures/review_snapshot_golden.json")
    );
}
#[test]
fn serve_review_trace_through_the_library_matches_the_golden() {
    let snapshot = include_str!("../../dozer-app/tests/fixtures/review_snapshot_golden.json");
    let mut actual = String::new();
    for name in [
        "host.html",
        "review-trace.js",
        "review-trace.css",
        "data.json",
    ] {
        let (mime, body) = serve_review_trace(name, Some(snapshot)).unwrap();
        actual.push_str(&format!(
            "{name}\t{mime}\t{}\t{:x}\n",
            body.len(),
            Sha256::digest(&body)
        ));
    }
    assert!(serve_review_trace("data.json", None).is_none());
    actual.push_str("data.json without snapshot\t404\n");
    actual.push_str(snapshot);
    assert_eq!(
        actual,
        include_str!("../../dozer-app/tests/fixtures/review_trace_serve_golden.txt")
    );
}
#[test]
fn review_webview_spec_url_uses_the_given_scheme_and_keeps_the_dozer_bytes() {
    let rv = review();
    assert_eq!(
        review_webview_spec(Some(&rv), "en", "dozer").unwrap().url,
        "dozer://review-trace/host.html?_r=42&locale=en"
    );
    assert_eq!(
        review_webview_spec(Some(&rv), "en", "digger").unwrap().url,
        "digger://review-trace/host.html?_r=42&locale=en"
    );
}
#[test]
fn webview_spec_is_none_for_error_or_empty_entries_and_parked_until_loaded() {
    assert!(review_webview_spec(None, "en", "dozer").is_none());
    let mut rv = review();
    let spec = review_webview_spec(Some(&rv), "en", "dozer").unwrap();
    assert!(spec.park_offscreen);
    assert!(!spec.visible);
    rv.loaded_nonce = Some(42);
    let spec = review_webview_spec(Some(&rv), "en", "dozer").unwrap();
    assert!(!spec.park_offscreen);
    assert!(spec.visible);
    rv.nonce = 43;
    assert!(
        review_webview_spec(Some(&rv), "en", "dozer")
            .unwrap()
            .park_offscreen
    );
    rv.error = Some("boom".into());
    assert!(review_webview_spec(Some(&rv), "en", "dozer").is_none());
    rv.error = None;
    rv.entries.clear();
    assert!(review_webview_spec(Some(&rv), "en", "dozer").is_none());
}
#[test]
fn serve_review_trace_rejects_path_traversal_and_unknown_paths() {
    for path in [
        "../host.html",
        "%2e%2e/host.html",
        "/host.html",
        "",
        "unknown",
        "a/../host.html",
    ] {
        assert!(serve_review_trace(path, Some("{}")).is_none(), "{path}");
    }
}
#[test]
fn serve_review_trace_data_json_is_404_without_a_snapshot_exactly_as_before() {
    assert_eq!(serve_review_trace("data.json", None), None);
    assert_eq!(
        serve_review_trace("data.json", Some("{}")),
        Some(("application/json", b"{}".to_vec()))
    );
}
#[test]
fn filter_handles_unicode_case_and_empty_inputs() {
    assert!(filter_sessions(&[], "", None).is_empty());
    let rows = vec![SessionRow {
        conversation_id: "c1".into(),
        display_title: "ÄPFEL 中文 🦀".into(),
        agent: AgentKind::Claude,
        last_ts: 0,
        summary: None,
        summary_status: None,
        task_id: None,
    }];
    for query in ["", "äpfel", "中文", "🦀"] {
        assert_eq!(filter_sessions(&rows, query, None).len(), 1);
    }
}
#[test]
fn visible_count_never_overflows_for_large_page_numbers() {
    assert_eq!(conversation_visible_count(usize::MAX), usize::MAX);
    assert_eq!(conversation_visible_count(usize::MAX / 20), usize::MAX);
}
#[test]
fn only_one_row_is_marked_current() {
    let rv = review();
    let current = current_review_conversation_id(Some(&rv));
    assert_eq!(
        ["c1", "c2", "c3"]
            .iter()
            .filter(|id| Some(**id) == current)
            .count(),
        1
    );
    assert_eq!(current_review_conversation_id(None), None);
}
