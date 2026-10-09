# dozer-model Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `dozer-core::protocol` 里 43 个纯数据类型 + `dozer-app/src/conversation.rs` 搬进只依赖 serde 的新 crate `dozer-model`,让面板代码不再依赖 `dozer_core::protocol` / `crate::conversation`,线上格式零变化。

**Architecture:** 先在原位置用黄金 JSON 测试(集成测试,走 `dozer_core::protocol::*` 路径)钉死格式;再用一个一次性脚本把类型按模块机械搬进 `crates/dozer-model`,`protocol.rs` 用显式列表 `pub use dozer_model::{..}` 兼容出口,黄金测试一字不改仍须通过;再下沉 `conversation.rs`;再把 `extensions/**` 的导入切到 `dozer_model::`;最后加 `R-PROTOCOL` 棘轮门禁。

**Tech Stack:** Rust 2024 workspace、serde/serde_json、Python 3 审计脚本(`scripts/audit`)。

**Spec:** `docs/superpowers/specs/2026-10-09-dozer-model-crate-design.md`

## Global Constraints

- 在独立 worktree/分支上实施(分支名 `feat/dozer-model`,基于含本计划与规格的 `spec/dozer-model`),**不在 main 上直接做**。
- 绝不提交用户 WIP:`Cargo.lock`(例外见下)、`crates/dozer-app/Cargo.toml`(例外见下)、`crates/dozer-app/packaging/macos/Info.plist`。worktree 里构建会因 `../byteui` path patch 改 `Cargo.lock` 的 source 行——那类变动 `git checkout -p`/手工剔除,不入提交。**例外:** 新增 workspace 成员 `dozer-model` 与新增依赖边必然改 `Cargo.lock` 与相关 `Cargo.toml`;提交时只带这些变化(`git diff Cargo.lock` 逐块核对)。
- `dozer-model` 的依赖只有 `serde`(derive);不得依赖 `dozer-core`、`bytehost-*`、`dozer-client`、`serde_json`(仅 dev-dependency 可用)。
- 线上格式零变化:`#[derive]`/`#[serde(...)]` 原样搬,不改任何类型的字段、顺序、属性。
- main 上已有、**不修**:`files::delete_confirm_spec_reflects_pending_target` 测试失败;`R-HOST-EXECUTORS 0 -> 1` 门禁失败;`usage/aggregate.rs:311` clippy 警告。
- 日志/字体等 CLAUDE.md 规则与本任务无关;`dozer-hook` 的依赖闭包只允许多出 `dozer-model`(仅 serde)。
- 每个提交消息以 `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>` 结尾。
- 测试命令用 `cargo test -p <crate>`;全量用 `cargo test --workspace`(输出重定向到文件,读尾部)。

## Review Focus

- `pub use` 显式列表漏项或与 `protocol` 内同名项冲突 → 编译期暴露,但 `dozer-hook`/`dozer-mcp` 不在 `dozer-app` 里,务必 `cargo build --workspace` 全量编译而不只编 `dozer-app`。
- 脚本搬运丢失类型前的文档注释/属性(`#[derive]`、`#[serde]`)→ 黄金测试 + 抽查 diff(`git diff --stat` 搬运前后类型总行数守恒)。
- `dozer-model` 意外多出依赖(如 `serde_json` 进了 `[dependencies]`)→ Task 2 的 `cargo tree` 验收。
- 门禁规则把注释/字符串里的 `dozer_core::protocol` 也计入 → Task 5 的单测覆盖。
- 重新测量的基线与规格里的数字(43/23/14)不一致 → Task 1 把实测值写入 ledger,以实测为准。

---

### Task 1: M0 特征化——黄金 JSON 测试与基线测量

**Files:**
- Create: `crates/dozer-core/tests/model_golden.rs`

**Interfaces:**
- Consumes: `dozer_core::protocol` 里全部 43 个数据类型、`Request`、`Reply`、`encode_line`(现状)。
- Produces: `model_golden.rs`(之后各任务**不得修改**它,Task 2 起它是"格式未变"的证据);ledger 里的基线数字。

- [ ] **Step 1: 记录基线(只测量,不改代码)**

```bash
cd <worktree>
grep -c '^pub \(struct\|enum\)' <(sed -n 1,860p crates/dozer-core/src/protocol.rs)   # 期望 43
grep -rl 'dozer_core::protocol' crates/dozer-app/src/extensions | wc -l               # 期望 23(实测为准)
grep -rn 'crate::conversation' crates/dozer-app/src | grep -v '^crates/dozer-app/src/conversation.rs' | wc -l
cargo test --workspace > /tmp/baseline-tests.txt 2>&1; grep -E '^test result' /tmp/baseline-tests.txt
cargo clippy --all-targets 2>&1 | grep -c '^warning' 
```

把 5 个数字写进 ledger(`progress.md`)。`cargo test --workspace` 允许有已知的 `files::delete_confirm_spec_reflects_pending_target` 失败;加 `--no-fail-fast`。

- [ ] **Step 2: 写黄金测试文件**

`crates/dozer-core/tests/model_golden.rs`:

```rust
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
        ToolCallInfo { summary: "s".into(), input_json: None, id: None },
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
    golden(SummaryStatus::HeuristicFallback, json!("heuristic_fallback"));
    golden(SummaryErrorKind::ConfigurationRequired, json!("configuration_required"));
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
        PreviewContext { path: "/b".into(), ..Default::default() },
        json!({"path":"/b","start_line":0,"start_col":0,"end_line":0,"end_col":0,"has_selection":false,
               "updated_at_ms":0,"revision":0,"read_only":false}),
    );
    let t = PreviewTabularContext { selected_range: Some((2, 3, 2, 3)), ..Default::default() };
    assert_eq!(t.selected_cell_from_range(), Some((2, 3)));
    let mut c = PreviewContext::default();
    c.set_selected_text(Some("a".repeat(5000)));
    assert_eq!(c.selected_text.as_ref().unwrap().chars().count(), 4096);
    c.set_selected_text(Some(String::new()));
    assert_eq!(c.selected_text, None);

    golden(
        PreviewCommandTarget::Tab { panel: "files".into(), tab_id: 2 },
        json!({"by":"tab","panel":"files","tab_id":2}),
    );
    golden(PreviewCommandTarget::Path { path: "/x".into() }, json!({"by":"path","path":"/x"}));
    golden(
        PreviewCommandAction::RevealCell { sheet: 0, row: 1, col: 2 },
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
    let o = PreviewCommandOutcome::StaleRevision { request_id: "r".into(), current_revision: 4 };
    assert_eq!(o.request_id(), "r");
    golden(o, json!({"status":"stale_revision","request_id":"r","current_revision":4}));
    golden(
        LocateMatch { start_line: 1, start_col: 2, end_line: 3, end_col: 4, context: "c".into() },
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
    golden(MutationOutcome::PathOutOfBounds, json!({"kind":"path_out_of_bounds"}));
    golden(
        MutationOutcome::Conflict { actual_text: "t".into() },
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
    golden(GroupAuthor::Member { member_id: 3 }, json!({"kind":"member","member_id":3}));
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
        GroupInfo { id: 2, project_id: 1, topic: "t".into(), created_ms: 5, members: vec![m] },
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
    let req = Request::SetTodoStatus { id: 5, status: TodoStoredStatus::Suspended };
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
```

- [ ] **Step 3: 跑测试,校正夹具(不是改代码)**

Run: `cargo test -p dozer-core --test model_golden 2>&1 | tail -40`
Expected: 首次可能有失败——只可能因为**我手写的期望 JSON 与现状 serde 输出不符**(字段名拼错、`Request::CreateSession` 的 tag 名、`Reply::Sessions` 是否结构体变体等)。逐个失败读 diff:若是夹具笔误,改夹具使其等于**现状**输出;**严禁**改 `protocol.rs`。目标:全绿。同时在 ledger 记 `Task 1: Ruling:` 列出被校正的夹具项。`serde_json::json!` 里 tuple 序列化为数组、`files_touched` 的 BTreeSet 有序,这两点是预期行为。

- [ ] **Step 4: 确认测试能抓到格式变化(RED 证明)**

临时把 `protocol.rs` 里 `TodoStoredStatus` 的 `#[serde(rename_all = "snake_case")]` 删掉,跑 `cargo test -p dozer-core --test model_golden`,Expected: `todo_group` 与 `request_and_reply_lines_are_byte_stable` FAIL;然后 `git checkout crates/dozer-core/src/protocol.rs` 还原,再跑一次 Expected: PASS。

- [ ] **Step 5: 全量确认并提交**

Run: `cargo test -p dozer-core 2>&1 | tail -5` → PASS。

```bash
git add crates/dozer-core/tests/model_golden.rs
git commit -m "test(core): O7 M0 黄金 JSON 特征化(43 个数据类型 + Request/Reply 抽样)

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 2: M1——建 `dozer-model` crate 并搬 43 个类型

**Files:**
- Create: `crates/dozer-model/Cargo.toml`, `crates/dozer-model/src/lib.rs`, `crates/dozer-model/src/{agent,conversation,summary,usage,todo,project,preview,group_chat,misc}.rs`
- Modify: `crates/dozer-core/Cargo.toml`, `crates/dozer-core/src/protocol.rs`, `Cargo.lock`(只含新 crate 与依赖边)
- Scratch(不提交): `<scratchpad>/split_protocol.py`

**Interfaces:**
- Consumes: Task 1 的 `model_golden.rs`(只读)。
- Produces: crate `dozer_model`,各类型在 crate 根可见(`dozer_model::AgentKind` 等,由 `lib.rs` 的 `pub use <mod>::*` 提供);`dozer_core::protocol` 仍导出同名项(显式 `pub use dozer_model::{...}`)。`conversation` 模块此时**仅含** `ConversationSummary`/`ToolCallInfo`/`TurnRecord`,Task 3 追加 `ConversationMeta`/`SessionRow`。

- [ ] **Step 1: 写 crate 骨架**

`crates/dozer-model/Cargo.toml`:
```toml
[package]
name = "dozer-model"
version = "0.1.0"
edition.workspace = true

# 纯数据类型:只许依赖 serde。不要加 dozer-core / bytehost-* / 运行时依赖(spec 2026-10-09)。
[dependencies]
serde.workspace = true

[dev-dependencies]
serde_json.workspace = true
```
`crates/dozer-model/src/lib.rs` 先写空壳 `//! Dozer 领域数据类型(线上 JSON 形状的唯一定义处)。`。`crates/dozer-core/Cargo.toml` 的 `[dependencies]` 加 `dozer-model = { path = "../dozer-model" }`。

- [ ] **Step 2: 写搬运脚本(scratchpad,不提交)**

`split_protocol.py`:读 `crates/dozer-core/src/protocol.rs`,把第 3 行起到 `pub enum Request` 之前(含其上方的 `#[derive]`/文档注释)按**顶层项**切分(项 = 前导注释/属性 + `pub struct|enum|const|impl` 到匹配的收尾);按下表分配模块;`impl X` 跟随 `X`;遇到不在表里的项就报错退出:

```python
MODS = {
 "agent": ["AgentState","AgentKind","SessionInfo"],
 "conversation": ["ConversationSummary","ToolCallInfo","TurnRecord"],
 "summary": ["SummaryStatus","SessionSummaryPayload","SummaryErrorKind","SummaryJobStatus","SummaryTrigger",
             "ConversationSummaryResult","SummaryJobInfo","SummaryBatchInfo","SummaryProviderInfo"],
 "usage": ["UsagePayload"],
 "todo": ["TodoStoredStatus","TodoInfo","CategoryInfo","CategoryMoveDirection"],
 "project": ["ProjectInfo","MemoryInfo","MemoryDetail","MemoryHistoryEntry"],
 "preview": ["PREVIEW_SELECTED_TEXT_MAX_CHARS","PreviewContext","PreviewTabularContext","PREVIEW_COMMAND_TIMEOUT_MS",
             "PreviewCommandTarget","PreviewCommandAction","PreviewCommand","PreviewCommandOutcome",
             "LocateMatch","MutationOutcome"],
 "group_chat": ["GroupMemberInfo","GroupInfo","GroupAuthor","GroupMessageStatus","GroupMessageInfo","GroupCancelScope"],
 "misc": ["BookmarkScope","BookmarkInfo","CodeHealthReportInfo","ContextItemInfo","FileEditHistoryInfo"],
}
```
切分规则:从 `protocol.rs` 第 3 行到 `Request` 枚举前的那个 `#[derive(...)]` 之前;项的起点是一段连续的 `///`/`//`/`#[...]` 行加后面的 `pub`/`impl` 行,终点是行首 `}`(单行 const 以 `;` 结束)。脚本职责:
1. 为每个模块写文件头:`use serde::{Deserialize, Serialize};`(模块文本含 `Serialize` 才写)、`use std::collections::BTreeSet;`(含 `BTreeSet` 才写)、`use crate::{<该模块引用到的其他模块的类型名>};`(用词边界正则检测,没有则不写);
2. 写 `lib.rs`:`mod <m>;`(私有)+ `pub use <m>::*;`(逐模块);
3. 重写 `protocol.rs`:删掉被搬走的项与 `use std::collections::BTreeSet;`,在文件头加 `pub use dozer_model::{<所有 43 个类型名 + 2 个常量,逐个列出,按字母序>};`(显式列表,不用通配),保留 `use serde::{Deserialize, Serialize, de::DeserializeOwned};`;`Request`/`Reply`/`encode_line`/`decode_line`/`mod tests` 原样不动;
4. 断言:搬走的顶层 `pub struct|enum` 数 == 43、`pub const` 数 == 2,否则退出非 0。

- [ ] **Step 3: 运行脚本并编译**

Run:
```bash
python3 -I <scratchpad>/split_protocol.py crates/dozer-core/src/protocol.rs crates/dozer-model/src
cargo fmt -p dozer-model -p dozer-core
cargo build --workspace 2>&1 | tail -30
```
Expected: 编译通过。可能的问题与处理——`unused import`(删掉多余 import);某个 impl 引用了未导入的类型(补 `use crate::…`);`dozer-hook`/`dozer-mcp`/`dozerd`/`dozer-app` 里有对被搬类型的**外部 trait 实现**(孤儿规则报错 E0117/E0116)——先 `git grep -n 'impl .* for \(AgentKind\|TodoInfo\|…\)'` 定位,在 ledger 记 Ruling 并把该实现挪进 `dozer-model` 或改为适配函数。

- [ ] **Step 4: 黄金测试与全量测试(未修改)**

Run:
```bash
git diff --stat -- crates/dozer-core/tests/model_golden.rs   # 期望:无输出(文件未改)
cargo test -p dozer-core 2>&1 | tail -8
cargo test --workspace --no-fail-fast > /tmp/m1-tests.txt 2>&1; grep -E '^test result|FAILED|failed' /tmp/m1-tests.txt | head
```
Expected: `model_golden` 全绿;`protocol.rs` 内原有 `mod tests` 全绿(靠 `use super::*` 与 `pub use`);workspace 测试数 = 基线 + Task 1 新增的 11 个,仅已知的 `files::delete_confirm_spec_reflects_pending_target` 失败。

- [ ] **Step 5: 守恒与依赖验收**

```bash
cargo tree -p dozer-model --edges normal | head -20        # 期望只有 serde 系
grep -c '^pub \(struct\|enum\)' crates/dozer-model/src/*.rs | paste -sd' '   # 合计 43
grep -n 'Request\|Reply' crates/dozer-model/src/*.rs        # 期望无输出
cargo tree -p dozer-hook --edges normal | grep -c dozer-model   # 期望 1
```
`cargo tree -p dozer-model` 若出现 `serde_json`,说明写进了 `[dependencies]`,改回 dev。

- [ ] **Step 6: 提交**

只暂存新 crate、两个被改文件、与 `Cargo.lock` 中新增 `dozer-model` 的块:
```bash
git add crates/dozer-model crates/dozer-core/Cargo.toml crates/dozer-core/src/protocol.rs
git add -p Cargo.lock    # 只选 dozer-model 条目与 dozer-core 依赖列表那一行
git status --short       # 确认没有 dozer-app/Cargo.toml、Info.plist
git commit -m "refactor(model): O7 M1 新建 dozer-model,搬出 43 个纯数据类型

protocol.rs 以显式 pub use 兼容出口,线上格式不变(黄金测试未改动)。

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 3: M2——下沉 `conversation.rs`

**Files:**
- Modify: `crates/dozer-model/src/conversation.rs`, `crates/dozer-model/src/lib.rs`, `crates/dozer-app/Cargo.toml`, `crates/dozer-app/src/main.rs:6`, `crates/dozer-app/src/usage_probe/scenarios.rs:7`, `crates/dozer-app/src/app/update.rs`(64, 6237, 6596, 6630 附近), `crates/dozer-app/src/extensions/conversations.rs:14`, `crates/dozer-app/src/extensions/usage/{protocol.rs,mod.rs,view.rs,aggregate.rs}`, `crates/dozer-app/src/chrome/homespace.rs:10`(以 Task 1 实测的调用点清单为准)
- Delete: `crates/dozer-app/src/conversation.rs`

**Interfaces:**
- Consumes: Task 2 的 `dozer_model::{AgentKind, ConversationSummary, SessionSummaryPayload, SummaryStatus}`。
- Produces: `dozer_model::{ConversationMeta, SessionRow}`(字段与方法签名与原文件逐字相同:`ConversationMeta { path: PathBuf, title: String, modified_ms: u64, agent: AgentKind }` + `from_summary(&ConversationSummary) -> Self`;`SessionRow { conversation_id, agent, last_ts, display_title, summary, summary_status, task_id }` + `from_row(&ConversationSummary, Option<&SessionSummaryPayload>) -> Self`)。注意:这两个类型**不**加 serde derive(原来就没有),也**不**加入 `dozer_core::protocol` 出口。

- [ ] **Step 1: 先把测试搬过去(RED)**

把 `crates/dozer-app/src/conversation.rs` 的 `mod tests`(`from_summary_maps_fields`、`session_row_from_row_uses_summary_title_and_text_when_present`、`session_row_from_row_falls_back_to_conversation_title_without_summary`)原样追加到 `crates/dozer-model/src/conversation.rs` 末尾(`use super::*;` 保持),此时类型尚未搬,`cargo test -p dozer-model` Expected: 编译失败 `cannot find type ConversationMeta`(RED)。

- [ ] **Step 2: 搬类型与 impl(GREEN)**

在 `crates/dozer-model/src/conversation.rs` 顶部把原文件中 `ConversationMeta`、`SessionRow` 及其 `impl`(含文档注释)原样粘入,头部加 `use std::path::PathBuf;` 和 `use crate::{AgentKind, SessionSummaryPayload, SummaryStatus};`(`ConversationSummary` 同模块无需导入;`AgentKind` 等来自根)。Run: `cargo test -p dozer-model` Expected: 3 passed。

- [ ] **Step 3: 切换 dozer-app 的调用点**

`crates/dozer-app/Cargo.toml` 的 `[dependencies]` 加 `dozer-model = { path = "../dozer-model" }`(**提交时只带这一行**;该文件有用户未提交改动,用 `git add -p`)。删除 `crates/dozer-app/src/conversation.rs` 与 `main.rs` 的 `mod conversation;`。逐处替换:
- `use crate::conversation::{ConversationMeta|SessionRow};` → `use dozer_model::{…};`
- `crate::conversation::ConversationMeta` / `crate::conversation::SessionRow`(含 `update.rs` 的 4 处和 `usage/mod.rs:190`、`usage/protocol.rs:261,316`)→ `dozer_model::…`
- `extensions/conversations.rs:3` 注释里提到 `crate::conversation` 的文字改为 `dozer_model::conversation`。

Run: `grep -rn 'crate::conversation' crates/dozer-app/src | grep -v 'extensions/conversations'` Expected: 无输出。

- [ ] **Step 4: 构建与测试**

Run:
```bash
cargo build --workspace 2>&1 | tail -5
cargo test --workspace --no-fail-fast > /tmp/m2-tests.txt 2>&1; grep -E '^test result|FAILED' /tmp/m2-tests.txt | head
cargo fmt --check -p dozer-model -p dozer-app
```
Expected: 构建通过;测试总数不变(三条测试从 dozer-app 挪到 dozer-model:dozer-app 少 3、dozer-model 多 3);仅已知失败。

- [ ] **Step 5: 提交**

```bash
git add crates/dozer-model crates/dozer-app/src
git add -p crates/dozer-app/Cargo.toml Cargo.lock    # 只选 dozer-model 依赖行与对应 lock 变化
git status --short
git commit -m "refactor(model): O7 M2 ConversationMeta/SessionRow 下沉到 dozer-model

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 4: M3——面板导入切到 `dozer_model::`

**Files:**
- Modify: `crates/dozer-app/src/extensions/**/*.rs` 中所有含 `dozer_core::protocol` 的文件(Task 1 实测为 23 个)

**Interfaces:**
- Consumes: Task 2/3 的 `dozer_model::*`。
- Produces: `extensions/**` 零 `dozer_core::protocol` 引用(Task 5 的门禁以此为基线)。

- [ ] **Step 1: 确认没有文件用到 `Request`/`Reply`/编解码**

Run: `grep -rn 'dozer_core::protocol' crates/dozer-app/src/extensions | grep -E 'Request|Reply|encode_line|decode_line'`
Expected: 无输出。若有输出(含多行 `use dozer_core::protocol::{ … }` 里的这些名字,需读该文件的 import 块确认),这些文件保留 `dozer_core::protocol` 引用,**只**迁移其中的数据类型,并在 ledger 记 Ruling;Task 5 的 R-PROTOCOL 基线相应不为 0(以实测为准,并更新规格验收 4 的口径)。

- [ ] **Step 2: 机械替换**

```bash
grep -rl 'dozer_core::protocol' crates/dozer-app/src/extensions | xargs perl -pi -e 's/dozer_core::protocol::/dozer_model::/g'
grep -rn 'dozer_core::protocol' crates/dozer-app/src/extensions   # 期望无输出(含注释)
```
(使用 perl 而非 BSD `sed -i`。)

- [ ] **Step 3: 构建、格式、测试**

Run:
```bash
cargo build -p dozer-app 2>&1 | tail -5
cargo fmt -p dozer-app            # 导入顺序变化可能触发 rustfmt 重排
cargo test --workspace --no-fail-fast > /tmp/m3-tests.txt 2>&1; grep -E '^test result|FAILED' /tmp/m3-tests.txt | head
cargo clippy --all-targets 2>&1 | grep -c '^warning'
```
Expected: 构建通过;测试数与 Task 3 相同;clippy 警告数 ≤ Task 1 基线。

- [ ] **Step 4: 重判探针 G6**

Run: `python3 scripts/audit/unit_edges.py` 对 Usage 面板的调用方式见该脚本 `--help`(参数为单元路径 `extensions/usage`)。Expected: 退出码 0;输出里 C 类边不再含 `conversation::` 或 `dozer_core::protocol`,只含 `dozer_model`。把输出尾部贴进 ledger。若脚本把 `dozer_model` 判为未分类(退出 1),在 `scripts/audit/unit_edges.py` 的分类表里把 `dozer_model` 归入 C 类(与此前 `dozer_core::protocol` 同类),并补 `test_unit_edges.py` 一条用例(先写、先看 FAIL、再改)。

- [ ] **Step 5: 提交**

```bash
git add crates/dozer-app/src/extensions scripts/audit
git commit -m "refactor(panels): O7 M3 extensions 的数据类型导入改为 dozer_model

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

---

### Task 5: M4——`R-PROTOCOL` 棘轮门禁与终验

**Files:**
- Modify: `scripts/audit/check_panel_boundary.py`, `scripts/audit/test_check_panel_boundary.py`, `scripts/audit/panel-boundary.baseline.json`(仅当 `--update` 后确有变化)
- Modify: `docs/dozer-v2/bytehost-H0/07-usage-probe-gaps.md`(G6 状态回填), `docs/dozer-v2/bytehost-H8-evaluation.md`(§7 追一句"O7 已解"), `CLAUDE.md`(Workspace 布局表加 `dozer-model` 一行)

**Interfaces:**
- Consumes: Task 4 的零引用状态。
- Produces: 规则 `R-PROTOCOL`:`extensions/` 下各文件对 `dozer_core::protocol` 的引用次数(注释与字符串不计),基线 0,只降不升。

- [ ] **Step 1: 先写失败的测试**

在 `scripts/audit/test_check_panel_boundary.py` 的 `Compare` 类之前加:

```python
class ProtocolCoupling(unittest.TestCase):
    """R-PROTOCOL:面板代码不得再依赖 dozer_core::protocol(数据类型走 dozer_model,O7)。"""
    def scan1(self, rel, text):
        return g.scan({rel: text}).get(rel, {})

    def test_use_and_qualified_path_in_extensions_are_counted(self):
        text = (
            "use dozer_core::protocol::AgentKind;\n"
            "fn f(_: dozer_core::protocol::TodoInfo) {}\n"
        )
        self.assertEqual(self.scan1("extensions/todo.rs", text), {"R-PROTOCOL": 2})

    def test_comments_and_strings_are_not_counted(self):
        text = '// dozer_core::protocol::X\nconst S: &str = "dozer_core::protocol::Y";\n'
        self.assertEqual(self.scan1("extensions/todo.rs", text), {})

    def test_other_directories_are_not_counted(self):
        text = "use dozer_core::protocol::AgentKind;\n"
        self.assertEqual(self.scan1("app/update.rs", text), {})

    def test_dozer_model_paths_are_not_counted(self):
        self.assertEqual(self.scan1("extensions/todo.rs", "use dozer_model::AgentKind;\n"), {})
```
Run: `python3 -I -m unittest scripts/audit/test_check_panel_boundary.py -v 2>&1 | tail -15`(若该仓库惯用 `cd scripts/audit && python3 -m unittest test_check_panel_boundary`,照惯用法)。Expected: 前两个新测试 FAIL(规则未实现)。

- [ ] **Step 2: 实现规则**

在 `check_panel_boundary.py` 的 `HOST_EXECUTOR_RE` 定义后加:
```python
# R-PROTOCOL(O7):面板代码里对 `dozer_core::protocol` 的引用次数。数据类型已搬进 dozer-model,
# 面板应写 `dozer_model::…`;只许降不许增(基线 0)。注释与字符串字面量不计。
PROTOCOL_RE = re.compile(r"\bdozer_core::protocol\b")
```
在 `scan()` 里 `if rel.startswith("extensions/"):` 块内、`for rule, (mod, name)` 循环之后加:
```python
            k = len(PROTOCOL_RE.findall(STRING_RE.sub('""', edges.strip_comments(text))))
            if k:
                hit["R-PROTOCOL"] = k
```
并在文件头文档字符串的规则列表里加一行 `规则 R-PROTOCOL: extensions 下对 dozer_core::protocol 的引用次数(O7,基线 0)`。

- [ ] **Step 3: 验证 GREEN 与真实基线**

Run:
```bash
python3 -I -m unittest scripts/audit/test_check_panel_boundary.py 2>&1 | tail -5     # 全绿
python3 scripts/audit/check_panel_boundary.py; echo "exit=$?"
```
Expected: 单测全绿;门禁输出里**只允许**出现已知的 `R-HOST-EXECUTORS 0 -> 1`(main 上既有,不修),没有 `R-PROTOCOL`。不要对基线跑 `--update`(那会把既有的 R-HOST-EXECUTORS 回退固化),`R-PROTOCOL` 基线本来就是"无条目即 0"。

- [ ] **Step 4: 文档回填**

- `07-usage-probe-gaps.md`:G6 状态改为"已解(O7,dozer-model,2026-10-09)",附 Task 4 Step 4 的边分类结果。
- `bytehost-H8-evaluation.md` §7:末尾加一句"O7 已解:数据类型已在 `dozer-model`,面板对 `dozer_core::protocol` 引用为 0(`R-PROTOCOL`)。"
- `CLAUDE.md` Workspace 布局表加行:`| crates/dozer-model | 领域数据类型(线上 JSON 形状的唯一定义处,仅依赖 serde);面板代码从这里取数据类型,不再引用 dozer_core::protocol(R-PROTOCOL 门禁) |`。

- [ ] **Step 5: 终验清单(逐条跑,读输出)**

```bash
cargo tree -p dozer-model --edges normal                       # 1: 只有 serde 系
cargo test -p dozer-core --test model_golden                   # 2: 全绿,git diff 该文件为空
git diff <分支起点>..HEAD --stat -- crates/dozer-core/tests/model_golden.rs   # 仅 Task 1 新增
cargo test --workspace --no-fail-fast > /tmp/final-tests.txt 2>&1; grep -E '^test result|FAILED' /tmp/final-tests.txt   # 3
python3 scripts/audit/check_panel_boundary.py                  # 4: 无 R-PROTOCOL
python3 scripts/audit/unit_edges.py                            # 5: 见 Task 4 Step 4
cargo clippy --all-targets 2>&1 | grep -c '^warning'           # 6: ≤ 基线
cargo fmt --check                                              # 6
git status --short                                             # 无 Info.plist、无无关 Cargo.toml/lock 变动
```
任一不满足即不收尾,回到对应任务修。

- [ ] **Step 6: 提交**

```bash
git add scripts/audit docs/dozer-v2 CLAUDE.md
git commit -m "chore(audit): O7 M4 R-PROTOCOL 棘轮门禁 + 文档回填

Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>"
```

之后按 executing-plans 的 Final Review 流程(整分支 fresh reviewer),再用 finishing-a-development-branch;合并进 main 前先对一次 main 上用户 WIP(`Cargo.lock`、`crates/dozer-app/Cargo.toml`、`Info.plist`)与本分支改动的重叠,**合并属于需先询问用户的外部副作用**。
