//! O7 特征化:43 个数据类型 + 两个 Request/Reply 样本的线上 JSON 形状。
//! 本文件在类型搬家前后**不得修改**——它走 `dozer_core::protocol::*` 路径,
//! 搬家后靠兼容出口继续通过,即证明 wire 未变。
use dozer_core::protocol::*;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::fmt::Debug;

fn golden<T: Serialize + DeserializeOwned + PartialEq + Debug>(v: T, want: Value) {
    assert_eq!(serde_json::to_value(&v).unwrap(), want, "serialize");
    let back: T = serde_json::from_value(want).unwrap();
    assert_eq!(back, v, "deserialize");
}

fn decodes<T: DeserializeOwned + PartialEq + Debug>(json: Value, want: T) {
    assert_eq!(serde_json::from_value::<T>(json).unwrap(), want);
}

#[test]
fn agent_group() {
    golden(AgentState::AwaitingInput, json!("awaiting_input"));
    golden(AgentState::TurnEnded, json!("turn_ended"));
    golden(AgentKind::V8agent, json!("v8agent"));
    golden(AgentKind::Codebuddy, json!("codebuddy"));
    let s = SessionInfo {
        id: "s1".into(),
        name: "主线".into(),
        command: "claude".into(),
        cwd: "/tmp".into(),
        alive: true,
        created_ms: 9,
        agent_state: AgentState::Running,
        transcript_path: Some("/t.jsonl".into()),
        project_id: Some(3),
        agent: AgentKind::Claude,
    };
    golden(
        s,
        json!({"id":"s1","name":"主线","command":"claude","cwd":"/tmp","alive":true,"created_ms":9,
               "agent_state":"running","transcript_path":"/t.jsonl","project_id":3,"agent":"claude"}),
    );
    // 老帧缺 agent_state/transcript_path/project_id/agent 时回落默认
    decodes(
        json!({"id":"s","name":"n","command":"c","cwd":"/","alive":false,"created_ms":1}),
        SessionInfo {
            id: "s".into(),
            name: "n".into(),
            command: "c".into(),
            cwd: "/".into(),
            alive: false,
            created_ms: 1,
            agent_state: AgentState::Idle,
            transcript_path: None,
            project_id: None,
            agent: AgentKind::Unknown,
        },
    );
}

#[test]
fn agent_kind_labels_are_stable() {
    assert_eq!(AgentKind::Unknown.label(), "shell");
    assert_eq!(AgentKind::Claude.label(), "claude");
    assert_eq!(AgentKind::Claude.display_label(), "Claude");
    assert_eq!(AgentKind::Codebuddy.display_label(), "CodeBuddy");
    assert_eq!(AgentKind::V8agent.label(), "v8agent");
}

#[test]
fn conversation_group() {
    golden(
        ConversationSummary {
            conversation_id: "c1".into(),
            agent: AgentKind::Codex,
            file_path: "/h/c1.jsonl".into(),
            title: "标题".into(),
            first_ts: 1,
            last_ts: 2,
            turn_count: 3,
        },
        json!({"conversation_id":"c1","agent":"codex","file_path":"/h/c1.jsonl","title":"标题",
               "first_ts":1,"last_ts":2,"turn_count":3}),
    );
    golden(
        ToolCallInfo {
            summary: "Read x".into(),
            input_json: Some("{}".into()),
            id: Some("t1".into()),
        },
        json!({"summary":"Read x","input_json":"{}","id":"t1"}),
    );
    decodes(
        json!({"summary":"s","input_json":null}),
        ToolCallInfo {
            summary: "s".into(),
            input_json: None,
            id: None,
        },
    );
    golden(
        TurnRecord {
            turn_index: 1,
            role: "assistant".into(),
            content: "hi".into(),
            tool_calls: vec![],
            thinking: false,
            thinking_text: None,
            ts: Some(5),
            is_error: true,
            tool_result_call_id: None,
            tokens_in: 1,
            tokens_out: 2,
            tokens_cache_read: 3,
            tokens_cache_write: 4,
        },
        json!({"turn_index":1,"role":"assistant","content":"hi","tool_calls":[],"thinking":false,
               "thinking_text":null,"ts":5,"is_error":true,"tool_result_call_id":null,
               "tokens_in":1,"tokens_out":2,"tokens_cache_read":3,"tokens_cache_write":4}),
    );
    // 老帧缺 tool_calls/is_error/tokens_* 时回落
    decodes(
        json!({"turn_index":0,"role":"user","content":"x","thinking":false,"ts":null}),
        TurnRecord {
            turn_index: 0,
            role: "user".into(),
            content: "x".into(),
            ts: None,
            ..Default::default()
        },
    );
}

#[test]
fn summary_group() {
    golden(SummaryStatus::AiGenerated, json!("ai_generated"));
    golden(
        SummaryStatus::HeuristicFallback,
        json!("heuristic_fallback"),
    );
    golden(
        SummaryErrorKind::ConfigurationRequired,
        json!("configuration_required"),
    );
    golden(SummaryErrorKind::RateLimit, json!("rate_limit"));
    golden(SummaryErrorKind::NonzeroExit, json!("nonzero_exit"));
    golden(SummaryJobStatus::Succeeded, json!("succeeded"));
    golden(SummaryTrigger::NaturalExit, json!("natural_exit"));
    golden(
        SessionSummaryPayload {
            session_id: "s1".into(),
            agent_kind: AgentKind::Claude,
            conversation_id: Some("c1".into()),
            title: "t".into(),
            summary: "x".into(),
            status: SummaryStatus::AiGenerated,
            created_ts_ms: 5,
            task_id: Some(7),
        },
        json!({"session_id":"s1","agent_kind":"claude","conversation_id":"c1","title":"t","summary":"x",
               "status":"ai_generated","created_ts_ms":5,"task_id":7}),
    );
    decodes(
        json!({"session_id":"s","agent_kind":"unknown","conversation_id":null,"title":"t","summary":"x",
               "status":"heuristic_fallback","created_ts_ms":1}),
        SessionSummaryPayload {
            session_id: "s".into(),
            agent_kind: AgentKind::Unknown,
            conversation_id: None,
            title: "t".into(),
            summary: "x".into(),
            status: SummaryStatus::HeuristicFallback,
            created_ts_ms: 1,
            task_id: None,
        },
    );
    golden(
        ConversationSummaryResult {
            conversation_id: "c".into(),
            title: "t".into(),
            summary: "s".into(),
            facts_json: Some("{}".into()),
            source_revision: "r1".into(),
            pipeline_version: "p1".into(),
            provider: AgentKind::Claude,
            requested_model: None,
            reported_model: Some("m".into()),
            coverage: None,
            generated_at: 9,
            source_job_id: 4,
        },
        json!({"conversation_id":"c","title":"t","summary":"s","facts_json":"{}","source_revision":"r1",
               "pipeline_version":"p1","provider":"claude","requested_model":null,"reported_model":"m",
               "coverage":null,"generated_at":9,"source_job_id":4}),
    );
    golden(
        SummaryJobInfo {
            job_id: 1,
            conversation_id: "c".into(),
            source_session_id: None,
            trigger: SummaryTrigger::Manual,
            provider: AgentKind::Codex,
            requested_model: None,
            source_revision: "r".into(),
            pipeline_version: "p".into(),
            generation: 2,
            status: SummaryJobStatus::Failed,
            attempt: 3,
            error_kind: Some(SummaryErrorKind::Timeout),
            error_detail: Some("d".into()),
            created_ts_ms: 10,
            updated_ts_ms: 11,
            phase: Some("run".into()),
        },
        json!({"job_id":1,"conversation_id":"c","source_session_id":null,"trigger":"manual",
               "provider":"codex","requested_model":null,"source_revision":"r","pipeline_version":"p",
               "generation":2,"status":"failed","attempt":3,"error_kind":"timeout","error_detail":"d",
               "created_ts_ms":10,"updated_ts_ms":11,"phase":"run"}),
    );
    golden(
        SummaryBatchInfo {
            batch_id: 1,
            cwd: "/p".into(),
            strategy: "all".into(),
            total: 6,
            queued: 1,
            running: 1,
            succeeded: 1,
            failed: 1,
            cancelled: 1,
            skipped: 1,
            created_ts_ms: 2,
            updated_ts_ms: 3,
        },
        json!({"batch_id":1,"cwd":"/p","strategy":"all","total":6,"queued":1,"running":1,"succeeded":1,
               "failed":1,"cancelled":1,"skipped":1,"created_ts_ms":2,"updated_ts_ms":3}),
    );
    golden(
        SummaryProviderInfo::default(),
        json!({"provider":null,"model":null,"source":null,"required":false}),
    );
}

#[test]
fn usage_group() {
    golden(
        UsagePayload {
            turns: 1,
            tool_calls: 2,
            mutating_tool_calls: 3,
            files_touched: BTreeSet::from(["a.rs".to_string(), "b.rs".to_string()]),
            tokens_in: 4,
            tokens_out: 5,
            tokens_cache_read: 6,
            tokens_cache_write: 7,
        },
        json!({"turns":1,"tool_calls":2,"mutating_tool_calls":3,"files_touched":["a.rs","b.rs"],
               "tokens_in":4,"tokens_out":5,"tokens_cache_read":6,"tokens_cache_write":7}),
    );
}

#[test]
fn todo_group() {
    golden(TodoStoredStatus::Suspended, json!("suspended"));
    golden(CategoryMoveDirection::Down, json!("down"));
    golden(
        TodoInfo {
            id: 1,
            project_id: 2,
            text: "写".into(),
            done: false,
            paused: true,
            rank: 3,
            created_ms: 4,
            completed_at_ms: None,
            plan_date: Some("2026-10-09".into()),
            dispatch_session_id: None,
            dispatch_at_ms: None,
            category_id: Some(8),
            assigned_agent: Some(AgentKind::Claude),
        },
        json!({"id":1,"project_id":2,"text":"写","done":false,"paused":true,"rank":3,"created_ms":4,
               "completed_at_ms":null,"plan_date":"2026-10-09","dispatch_session_id":null,
               "dispatch_at_ms":null,"category_id":8,"assigned_agent":"claude"}),
    );
    decodes(
        json!({"id":1,"project_id":2,"text":"t","done":true,"rank":0,"created_ms":0,"completed_at_ms":null,
               "plan_date":null,"dispatch_session_id":null,"dispatch_at_ms":null}),
        TodoInfo {
            id: 1,
            project_id: 2,
            text: "t".into(),
            done: true,
            paused: false,
            rank: 0,
            created_ms: 0,
            completed_at_ms: None,
            plan_date: None,
            dispatch_session_id: None,
            dispatch_at_ms: None,
            category_id: None,
            assigned_agent: None,
        },
    );
    golden(
        CategoryInfo {
            id: 1,
            project_id: 2,
            parent_id: None,
            name: "n".into(),
            rank: 3,
            created_ms: 4,
            auto_poll_enabled: true,
        },
        json!({"id":1,"project_id":2,"parent_id":null,"name":"n","rank":3,"created_ms":4,"auto_poll_enabled":true}),
    );
}

#[test]
fn project_group() {
    golden(
        ProjectInfo {
            id: 1,
            path: "/p".into(),
            name: "n".into(),
            last_active_ms: 2,
            created_ms: 3,
            updated_ms: 4,
        },
        json!({"id":1,"path":"/p","name":"n","last_active_ms":2,"created_ms":3,"updated_ms":4}),
    );
    golden(
        MemoryInfo {
            id: 1,
            project_id: 2,
            title: "t".into(),
            kind: "note".into(),
            description: "d".into(),
            updated_ms: 3,
            updated_by: "claude".into(),
        },
        json!({"id":1,"project_id":2,"title":"t","kind":"note","description":"d","updated_ms":3,"updated_by":"claude"}),
    );
    let h = MemoryHistoryEntry {
        id: 9,
        memory_id: 1,
        changed_ms: 5,
        changed_by: "me".into(),
        change_kind: "edit".into(),
        title: "t".into(),
        kind: "note".into(),
        description: "d".into(),
        body: "b".into(),
    };
    golden(
        h.clone(),
        json!({"id":9,"memory_id":1,"changed_ms":5,"changed_by":"me","change_kind":"edit","title":"t",
               "kind":"note","description":"d","body":"b"}),
    );
    golden(
        MemoryDetail {
            id: 1,
            project_id: 2,
            title: "t".into(),
            kind: "note".into(),
            description: "d".into(),
            body: "b".into(),
            created_ms: 3,
            created_by: "a".into(),
            updated_ms: 4,
            updated_by: "b".into(),
            history: vec![h],
        },
        json!({"id":1,"project_id":2,"title":"t","kind":"note","description":"d","body":"b",
               "created_ms":3,"created_by":"a","updated_ms":4,"updated_by":"b",
               "history":[{"id":9,"memory_id":1,"changed_ms":5,"changed_by":"me","change_kind":"edit",
                           "title":"t","kind":"note","description":"d","body":"b"}]}),
    );
}

#[test]
fn preview_group() {
    assert_eq!(PREVIEW_SELECTED_TEXT_MAX_CHARS, 4096);
    assert_eq!(PREVIEW_COMMAND_TIMEOUT_MS, 2000);
    golden(
        PreviewContext {
            path: "/a.rs".into(),
            start_line: 1,
            start_col: 2,
            end_line: 3,
            end_col: 4,
            has_selection: true,
            updated_at_ms: 5,
            revision: 6,
            mode: Some("code".into()),
            read_only: true,
            selected_text: Some("x".into()),
            visible_start_line: Some(1),
            visible_end_line: Some(9),
            tabular: Some(PreviewTabularContext {
                sheet: 1,
                scroll_row: 2,
                scroll_col: 3,
                visible_rows: Some((0, 5)),
                visible_cols: None,
                selected_cell: Some((1, 1)),
                selected_range: Some((1, 1, 1, 1)),
            }),
        },
        json!({"path":"/a.rs","start_line":1,"start_col":2,"end_line":3,"end_col":4,"has_selection":true,
               "updated_at_ms":5,"revision":6,"mode":"code","read_only":true,"selected_text":"x",
               "visible_start_line":1,"visible_end_line":9,
               "tabular":{"sheet":1,"scroll_row":2,"scroll_col":3,"visible_rows":[0,5],
                          "selected_cell":[1,1],"selected_range":[1,1,1,1]}}),
    );
    // 老 JSON 缺新增可选字段时回落;可选字段为 None 时不输出
    golden(
        PreviewContext {
            path: "/b".into(),
            ..Default::default()
        },
        json!({"path":"/b","start_line":0,"start_col":0,"end_line":0,"end_col":0,"has_selection":false,
               "updated_at_ms":0,"revision":0,"read_only":false}),
    );
    let t = PreviewTabularContext {
        selected_range: Some((2, 3, 2, 3)),
        ..Default::default()
    };
    assert_eq!(t.selected_cell_from_range(), Some((2, 3)));
    let mut c = PreviewContext::default();
    c.set_selected_text(Some("a".repeat(5000)));
    assert_eq!(c.selected_text.as_ref().unwrap().chars().count(), 4096);
    c.set_selected_text(Some(String::new()));
    assert_eq!(c.selected_text, None);

    golden(
        PreviewCommandTarget::Tab {
            panel: "files".into(),
            tab_id: 2,
        },
        json!({"by":"tab","panel":"files","tab_id":2}),
    );
    golden(
        PreviewCommandTarget::Path { path: "/x".into() },
        json!({"by":"path","path":"/x"}),
    );
    golden(
        PreviewCommandAction::RevealCell {
            sheet: 0,
            row: 1,
            col: 2,
        },
        json!({"kind":"reveal_cell","sheet":0,"row":1,"col":2}),
    );
    golden(
        PreviewCommandAction::Highlight {
            start_line: 1,
            start_column: 2,
            end_line: 3,
            end_column: 4,
            duration_ms: 500,
        },
        json!({"kind":"highlight","start_line":1,"start_column":2,"end_line":3,"end_column":4,"duration_ms":500}),
    );
    golden(
        PreviewCommand {
            request_id: "r1".into(),
            project_id: 1,
            target: PreviewCommandTarget::Path { path: "/x".into() },
            action: PreviewCommandAction::Reveal { line: 1, column: 1 },
            expected_revision: None,
        },
        json!({"request_id":"r1","project_id":1,"target":{"by":"path","path":"/x"},
               "action":{"kind":"reveal","line":1,"column":1}}),
    );
    let o = PreviewCommandOutcome::StaleRevision {
        request_id: "r".into(),
        current_revision: 4,
    };
    assert_eq!(o.request_id(), "r");
    golden(
        o,
        json!({"status":"stale_revision","request_id":"r","current_revision":4}),
    );
    golden(
        LocateMatch {
            start_line: 1,
            start_col: 2,
            end_line: 3,
            end_col: 4,
            context: "c".into(),
        },
        json!({"start_line":1,"start_col":2,"end_line":3,"end_col":4,"context":"c"}),
    );
    golden(
        MutationOutcome::Applied {
            new_start_line: 1,
            new_start_col: 2,
            new_end_line: 3,
            new_end_col: 4,
            history_id: 9,
        },
        json!({"kind":"applied","new_start_line":1,"new_start_col":2,"new_end_line":3,"new_end_col":4,"history_id":9}),
    );
    golden(MutationOutcome::NotFound, json!({"kind":"not_found"}));
    golden(
        MutationOutcome::PathOutOfBounds,
        json!({"kind":"path_out_of_bounds"}),
    );
    golden(
        MutationOutcome::Conflict {
            actual_text: "t".into(),
        },
        json!({"kind":"conflict","actual_text":"t"}),
    );
    golden(
        MutationOutcome::Unwritable { reason: "r".into() },
        json!({"kind":"unwritable","reason":"r"}),
    );
}

#[test]
fn group_chat_group() {
    golden(GroupCancelScope::Round, json!("round"));
    golden(GroupAuthor::Human, json!({"kind":"human"}));
    golden(
        GroupAuthor::Member { member_id: 3 },
        json!({"kind":"member","member_id":3}),
    );
    golden(GroupMessageStatus::Queued, json!({"state":"queued"}));
    golden(
        GroupMessageStatus::Failed { reason: "x".into() },
        json!({"state":"failed","reason":"x"}),
    );
    let m = GroupMemberInfo {
        id: 1,
        group_id: 2,
        agent: AgentKind::Claude,
        handle: "cc".into(),
        role_prompt: "p".into(),
    };
    golden(
        m.clone(),
        json!({"id":1,"group_id":2,"agent":"claude","handle":"cc","role_prompt":"p"}),
    );
    golden(
        GroupInfo {
            id: 2,
            project_id: 1,
            topic: "t".into(),
            created_ms: 5,
            members: vec![m],
        },
        json!({"id":2,"project_id":1,"topic":"t","created_ms":5,
               "members":[{"id":1,"group_id":2,"agent":"claude","handle":"cc","role_prompt":"p"}]}),
    );
    golden(
        GroupMessageInfo {
            id: 1,
            group_id: 2,
            seq: 3,
            rev: 4,
            author: GroupAuthor::System,
            text: "hi".into(),
            mentions: vec![1, 2],
            status: Some(GroupMessageStatus::Done),
            duration_ms: Some(10),
            created_ms: 6,
            todo_id: None,
        },
        json!({"id":1,"group_id":2,"seq":3,"rev":4,"author":{"kind":"system"},"text":"hi","mentions":[1,2],
               "status":{"state":"done"},"duration_ms":10,"created_ms":6,"todo_id":null}),
    );
}

#[test]
fn misc_group() {
    golden(BookmarkScope::Project, json!("project"));
    golden(
        BookmarkInfo {
            id: 1,
            scope: BookmarkScope::Global,
            project_id: None,
            url: "https://x".into(),
            title: "t".into(),
            created_ms: 2,
        },
        json!({"id":1,"scope":"global","project_id":null,"url":"https://x","title":"t","created_ms":2}),
    );
    golden(
        CodeHealthReportInfo {
            total_loc: 1,
            total_functions: 2,
            critical_functions: 3,
            overall_tier: "B".into(),
            report_json: "{}".into(),
            scanned_at_ms: 4,
            schema_version: 2,
            git_head: Some("abc".into()),
            git_branch: None,
            git_dirty: true,
        },
        json!({"total_loc":1,"total_functions":2,"critical_functions":3,"overall_tier":"B","report_json":"{}",
               "scanned_at_ms":4,"schema_version":2,"git_head":"abc","git_branch":null,"git_dirty":true}),
    );
    decodes(
        json!({"total_loc":1,"total_functions":2,"critical_functions":3,"overall_tier":"B","report_json":"{}",
               "scanned_at_ms":4}),
        CodeHealthReportInfo {
            total_loc: 1,
            total_functions: 2,
            critical_functions: 3,
            overall_tier: "B".into(),
            report_json: "{}".into(),
            scanned_at_ms: 4,
            schema_version: 0,
            git_head: None,
            git_branch: None,
            git_dirty: false,
        },
    );
    golden(
        ContextItemInfo {
            id: 1,
            project_id: 2,
            entity_kind: "file".into(),
            entity_ref: "/a".into(),
            created_ms: 3,
        },
        json!({"id":1,"project_id":2,"entity_kind":"file","entity_ref":"/a","created_ms":3}),
    );
    golden(
        FileEditHistoryInfo {
            id: 1,
            project_id: 2,
            target_path: "/a".into(),
            actor: "claude".into(),
            session_id: "s".into(),
            start_line: 1,
            start_col: 2,
            end_line: 3,
            end_col: 4,
            new_end_line: 5,
            new_end_col: 6,
            old_text: "o".into(),
            new_text: "n".into(),
            summary: "sum".into(),
            created_ms: 7,
        },
        json!({"id":1,"project_id":2,"target_path":"/a","actor":"claude","session_id":"s","start_line":1,
               "start_col":2,"end_line":3,"end_col":4,"new_end_line":5,"new_end_col":6,"old_text":"o",
               "new_text":"n","summary":"sum","created_ms":7}),
    );
}

/// wire 抽样:携带被搬类型的 Request/Reply 的整行编码在搬家前后逐字节相同。
#[test]
fn request_and_reply_lines_are_byte_stable() {
    let req = Request::CreateSession {
        name: "主线".into(),
        command: "/bin/sh".into(),
        args: vec!["-c".into(), "cat".into()],
        cwd: "/tmp".into(),
        cols: 80,
        rows: 24,
        project_id: 1,
        agent: AgentKind::Codex,
    };
    let line = encode_line(&req);
    assert!(line.ends_with('\n'));
    assert_eq!(
        serde_json::from_str::<Value>(line.trim()).unwrap(),
        json!({"type":"create_session","name":"主线","command":"/bin/sh","args":["-c","cat"],"cwd":"/tmp",
               "cols":80,"rows":24,"project_id":1,"agent":"codex"}),
    );
    let req = Request::SetTodoStatus {
        id: 5,
        status: TodoStoredStatus::Suspended,
    };
    assert_eq!(
        serde_json::from_str::<Value>(encode_line(&req).trim()).unwrap(),
        json!({"type":"set_todo_status","id":5,"status":"suspended"}),
    );
    let reply = Reply::Sessions {
        sessions: vec![SessionInfo {
            id: "s".into(),
            name: "n".into(),
            command: "c".into(),
            cwd: "/".into(),
            alive: true,
            created_ms: 1,
            agent_state: AgentState::Idle,
            transcript_path: None,
            project_id: Some(1),
            agent: AgentKind::Claude,
        }],
    };
    assert_eq!(
        serde_json::from_str::<Value>(encode_line(&reply).trim()).unwrap(),
        json!({"type":"sessions","sessions":[{"id":"s","name":"n","command":"c","cwd":"/","alive":true,
               "created_ms":1,"agent_state":"idle","transcript_path":null,"project_id":1,"agent":"claude"}]}),
    );
}
