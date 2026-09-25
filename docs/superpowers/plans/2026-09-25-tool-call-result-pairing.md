# 工具调用/工具结果按 id 精确配对 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 Claude/OpenCode/Unknown/V8agent/Goose 这五家 agent 原始数据里已有、但目前被丢弃的调用 id（`tool_use.id`/`tool_use_id`、Goose `tool_call_id`）接进 dozerd 的读时解析路径，让 review-trace 前端能精确配对工具调用与工具结果，而不是只靠数组下标近似。

**Architecture:** 只扩展"读时解析"这一条独立路径（`dozerd::transcripts::parse::extract_turn_trace_detail` 及其四个按 agent 分派的子函数），不碰摄取热路径、不加数据库列、不需要回填——`raw_json` 本身已经全量持久化，本次改动上线后所有历史会话下次查询就自动带上新字段。`dozer_core::protocol` 新增两个 `Option<String>` 字段作为线协议扩展；`dozer-app::transcript.rs` 把新字段透传进 `data.json` 契约；`review-trace` 前端按"全部调用都有 id 才用精确配对，否则整体回落已上线的下标近似"二选一。

**Tech Stack:** Rust（`dozer-core`/`dozerd`/`dozer-app`）、TypeScript/Preact（`crates/dozer-app/web/review-trace`）、`node:test` + `preact-render-to-string`（前端测试，沿用 `markdown.test.ts` 的验证手法）。

**Spec:** `docs/superpowers/specs/2026-09-25-tool-call-result-pairing-design.md`

## Global Constraints

- 不改摄取路径：`parse_claude_shaped_chunk`/`parse_codebuddy_shaped_chunk`/`parse_goose_hook_chunk`/`parse_codex_shaped_chunk`/`parse_aider_chunk` 一律不动。
- 不做数据库 schema 迁移、不做回填任务——读时解析不需要。
- 不覆盖 CodeBuddy（`function_call`/`function_call_result` 各自独立 id，互不引用，没有可用信号）、Aider（不提取结构化工具调用）、Codex（读时解析 v1 范围外）——这三家继续吃已上线的下标近似，不在本次范围内造信号。
- 新增字段一律 `Option<String>` + `#[serde(default)]`（Rust 侧）/`?: string`（TS 侧），不破坏现有序列化兼容性，不需要改 `dozer-mcp`/`dozer-client`/`dozerd::session_summary.rs`/`headless_agent.rs`/`task_processor.rs`/`dozer-app::todo/state.rs`（这些消费方都用 `..Default::default()` 或只读透传，已核实）。
- Goose `tool_call_id` 在 `PreToolUse`/`PostToolUse`/`PostToolUseFailure` 三个事件里是同一个值（官方 hooks 文档已确认，见 spec），三处都要读。
- Claude 侧一行 `raw_json` 可能合并了多个 `tool_result` block（`parse_claude_shaped_chunk` 摄取时会把同一条消息里的多个 block 拼接成一个 `ParsedTurn`）——只有恰好一个 block 时才能确定 `tool_use_id`，零个/多个都必须回落 `None`，不能瞎猜。
- 前端配对："全部 `tool_calls` 都有 `id`" 才进精确配对模式，只要有一个缺 `id` 就整体回落下标近似（不做部分 id、部分下标的混合）；一个 `id` 对应多个 result 时按遇到顺序堆在那条 call 后面，不特殊处理；result 的 `call_id` 对不上任何 call 时追加在最后，不能丢。

## Review Focus

- **多 `tool_result` block 合并成一行时不能瞎猜 id**——零个/两个 block 的输入必须都测到返回 `None`，不能只测"恰好一个"的正向路径（Task 2）。
- **`tool_use`/`tool_result` 缺 `id`/`tool_use_id` 字段时不能 panic 或返回错误 id**——老协议帧、CodeBuddy/Aider 路径必须继续吃 `None`（Task 1/2/3 的每个提取函数都要有"没有 id 字段"的用例）。
- **前端"全部有 id 才精确配对"这条二选一规则不能被绕过**——只要有一个 call 缺 id 就必须整体回落下标模式，不能出现"部分按 id、部分按下标"的混合渲染（Task 6 显式测试这条边界）。
- **一个 id 对应多个 result 时不能只渲染一个**——Task 6 要测多结果堆叠且顺序不乱。
- **老数据（没有 `id`/`tool_result_call_id` 字段的历史 JSON）反序列化不能报错**——`TurnRecord`/`ToolCallInfo`/`ToolResultEntry` 新字段必须靠 `#[serde(default)]` 兜底，Task 1 要有一个"反序列化缺字段的老 JSON"的显式测试锁住这一点。

---

## 文件结构总览

```text
crates/dozer-core/src/protocol.rs                          # 改：ToolCallInfo.id / TurnRecord.tool_result_call_id
crates/dozerd/src/transcripts/parse.rs                      # 改：TurnTraceDetail 新字段；
                                                              #     extract_claude_trace_detail / extract_goose_trace_detail 扩展
crates/dozerd/src/transcripts/mod.rs                        # 改：get_conversation_turns 透传新字段
crates/dozer-app/src/transcript.rs                          # 改：ToolResultEntry.call_id / ReviewEntry::ToolResult.call_id / 折叠逻辑
crates/dozer-app/web/review-trace/src/types.ts              # 改：ToolCall.id? / ToolResult.call_id?
crates/dozer-app/web/review-trace/src/components/TraceToggle.tsx  # 改：id 优先配对 + 下标近似兜底
crates/dozer-app/web/review-trace/src/TraceToggle.test.ts   # 新增
```

---

### Task 1: 协议字段 + Claude `tool_use.id` 提取

**Files:**
- Modify: `crates/dozer-core/src/protocol.rs`（`ToolCallInfo` 加 `id`，`TurnRecord` 加 `tool_result_call_id`）
- Modify: `crates/dozerd/src/transcripts/parse.rs`（`TurnTraceDetail` 加字段；`extract_claude_trace_detail` 的 `tool_use` 分支读 `id`）

**Interfaces:**
- Produces: `ToolCallInfo.id: Option<String>`、`TurnRecord.tool_result_call_id: Option<String>`、`TurnTraceDetail.tool_result_call_id: Option<String>`，供 Task 2-5 使用。

- [x] **Step 1: 写失败的测试**（`crates/dozerd/src/transcripts/parse.rs` 测试模块，`extract_claude_trace_detail_reads_thinking_text_and_tool_input` 测试下方）

```rust
#[test]
fn extract_claude_trace_detail_reads_tool_use_id() {
    let raw = r#"{"type":"assistant","message":{"content":[
        {"type":"tool_use","id":"toolu_01abc","name":"Edit","input":{"file_path":"README.md"}}
    ]}}"#;
    let detail = extract_turn_trace_detail(raw, AgentKind::Claude);
    assert_eq!(detail.tool_calls.len(), 1);
    assert_eq!(detail.tool_calls[0].id.as_deref(), Some("toolu_01abc"));
}

#[test]
fn extract_claude_trace_detail_tool_use_without_id_field_is_none() {
    let raw = r#"{"type":"assistant","message":{"content":[
        {"type":"tool_use","name":"Edit","input":{"file_path":"README.md"}}
    ]}}"#;
    let detail = extract_turn_trace_detail(raw, AgentKind::Claude);
    assert_eq!(detail.tool_calls[0].id, None);
}

#[test]
fn turn_record_deserializes_old_json_missing_new_fields() {
    // 老协议帧/老存量数据反序列化时没有 id/tool_result_call_id 这两个新
    // 字段，必须靠 #[serde(default)] 兜底成 None，不能报错（Review Focus）。
    let old_json = r#"{"turn_index":0,"role":"ai","content":"hi","thinking":false,"is_error":false}"#;
    let turn: dozer_core::protocol::TurnRecord = serde_json::from_str(old_json).unwrap();
    assert_eq!(turn.tool_result_call_id, None);

    let old_tool_call = r#"{"summary":"Edit README.md","input_json":null}"#;
    let call: dozer_core::protocol::ToolCallInfo = serde_json::from_str(old_tool_call).unwrap();
    assert_eq!(call.id, None);
}
```

- [x] **Step 2: 运行测试确认失败**

```bash
cargo test -p dozerd extract_claude_trace_detail_reads_tool_use_id
cargo test -p dozerd turn_record_deserializes_old_json_missing_new_fields
```

Expected: 编译失败（`ToolCallInfo`/`TurnRecord` 还没有 `id`/`tool_result_call_id` 字段）。

- [x] **Step 3: 在 `crates/dozer-core/src/protocol.rs` 给 `ToolCallInfo` 加字段**

找到：

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCallInfo {
    pub summary: String,
    pub input_json: Option<String>,
}
```

改成：

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCallInfo {
    pub summary: String,
    pub input_json: Option<String>,
    /// 这次工具调用的稳定 id（Claude tool_use.id、Goose tool_call_id 等）；
    /// 前端拿它和 tool_result 的 call_id 精确配对，agent/协议不提供 id 时
    /// 为 None(2026-09-25 起，见
    /// docs/superpowers/specs/2026-09-25-tool-call-result-pairing-design.md)。
    #[serde(default)]
    pub id: Option<String>,
}
```

- [x] **Step 4: 给 `TurnRecord` 加字段**

找到 `pub is_error: bool,` 那一行（`TurnRecord` 结构体内），在它后面加：

```rust
    #[serde(default)]
    pub is_error: bool,
    /// role == "tool_result" 时，这次结果对应的调用 id（读时解析补上，同
    /// `ToolCallInfo.id` 语义，用于前端精确配对；其余角色恒 None）。
    #[serde(default)]
    pub tool_result_call_id: Option<String>,
```

- [x] **Step 5: 给 `TurnTraceDetail` 加字段**（`crates/dozerd/src/transcripts/parse.rs`）

找到：

```rust
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TurnTraceDetail {
    pub thinking_text: Option<String>,
    pub tool_calls: Vec<ToolCallInfo>,
}
```

改成：

```rust
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TurnTraceDetail {
    pub thinking_text: Option<String>,
    pub tool_calls: Vec<ToolCallInfo>,
    /// role == "tool_result" 时对应的调用 id；见 `TurnRecord.tool_result_call_id`。
    pub tool_result_call_id: Option<String>,
}
```

- [x] **Step 6: 让 `extract_claude_trace_detail` 读 `tool_use.id`，并给所有构造 `TurnTraceDetail`/`ToolCallInfo` 的地方补上新字段**

修改 `extract_claude_trace_detail`（`Some("tool_use")` 分支）：

```rust
            Some("tool_use") => {
                let name = b.get("name").and_then(|n| n.as_str()).unwrap_or("工具");
                let input = b.get("input").cloned().unwrap_or(Value::Null);
                let id = b.get("id").and_then(|i| i.as_str()).map(str::to_string);
                tool_calls.push(ToolCallInfo {
                    summary: tool_summary(name, &input),
                    input_json: input_json_of(&input),
                    id,
                });
            }
```

函数末尾的返回值加上新字段（`tool_result_call_id` 这一步先恒 `None`，Task 2 再补真值）：

```rust
    TurnTraceDetail {
        thinking_text,
        tool_calls,
        tool_result_call_id: None,
    }
```

- [x] **Step 7: 修所有编译错误**

```bash
cargo check -p dozer-core -p dozerd -p dozer-app
```

Expected: 会报出所有构造 `ToolCallInfo { .. }`/`TurnTraceDetail { .. }` 时缺字段的位置（`extract_codebuddy_trace_detail`、`extract_goose_trace_detail` 等）；这两处按同样方式补 `id: None`（`extract_codebuddy_trace_detail`）或先留到 Task 3（`extract_goose_trace_detail`，本步先加 `id: None` 让它编译过，Task 3 再填真值）。逐个修到 `cargo check` 干净。

- [x] **Step 8: 运行测试确认通过**

```bash
cargo test -p dozerd extract_claude_trace_detail_reads_tool_use_id
cargo test -p dozerd extract_claude_trace_detail_tool_use_without_id_field_is_none
cargo test -p dozerd turn_record_deserializes_old_json_missing_new_fields
cargo test -p dozerd 2>&1 | tail -20
```

Expected: 新增 3 个测试通过；`cargo test -p dozerd` 全量跑一遍确认没有回归（已有的 `extract_claude_trace_detail_*`/`parses_human_and_ai_turn_with_usage_and_tools` 等测试应该继续通过，因为都没断言过 `id`/`tool_result_call_id` 字段）。

- [x] **Step 9: Commit**

```bash
git add crates/dozer-core/src/protocol.rs crates/dozerd/src/transcripts/parse.rs
git commit -m "feat(protocol): ToolCallInfo/TurnRecord 新增调用 id 字段，Claude tool_use.id 接入读时解析"
```

---

### Task 2: Claude `tool_result.tool_use_id` 提取（单 block 规则）

**Files:**
- Modify: `crates/dozerd/src/transcripts/parse.rs`（`extract_claude_trace_detail` 新增 `tool_result` block 扫描）

**Interfaces:**
- Consumes: Task 1 的 `TurnTraceDetail.tool_result_call_id`。
- Produces: `extract_claude_trace_detail` 在恰好一个 `tool_result` block 时填充 `tool_result_call_id`。

- [x] **Step 1: 写失败的测试**

```rust
#[test]
fn extract_claude_trace_detail_single_tool_result_block_gives_call_id() {
    let raw = r#"{"type":"user","message":{"content":[
        {"type":"tool_result","tool_use_id":"toolu_01abc","content":"ok"}
    ]}}"#;
    let detail = extract_turn_trace_detail(raw, AgentKind::Claude);
    assert_eq!(detail.tool_result_call_id.as_deref(), Some("toolu_01abc"));
}

#[test]
fn extract_claude_trace_detail_no_tool_result_block_gives_none() {
    let raw = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"hi"}]}}"#;
    let detail = extract_turn_trace_detail(raw, AgentKind::Claude);
    assert_eq!(detail.tool_result_call_id, None);
}

#[test]
fn extract_claude_trace_detail_multiple_tool_result_blocks_merged_gives_none() {
    // 一行里多个 tool_result block 合并(摄取时 parse_claude_shaped_chunk 会把
    // 它们拼成一个 ParsedTurn)，没法归属到单一 id，必须回落 None。
    let raw = r#"{"type":"user","message":{"content":[
        {"type":"tool_result","tool_use_id":"t1","content":"a"},
        {"type":"tool_result","tool_use_id":"t2","content":"b"}
    ]}}"#;
    let detail = extract_turn_trace_detail(raw, AgentKind::Claude);
    assert_eq!(detail.tool_result_call_id, None);
}

#[test]
fn extract_claude_trace_detail_tool_result_without_tool_use_id_field_gives_none() {
    let raw = r#"{"type":"user","message":{"content":[
        {"type":"tool_result","content":"ok"}
    ]}}"#;
    let detail = extract_turn_trace_detail(raw, AgentKind::Claude);
    assert_eq!(detail.tool_result_call_id, None);
}
```

- [x] **Step 2: 运行测试确认失败**

```bash
cargo test -p dozerd extract_claude_trace_detail_single_tool_result_block_gives_call_id
```

Expected: FAIL（`tool_result_call_id` 现在恒 `None`，第一个测试断言 `Some(..)` 会失败）。

- [x] **Step 3: 修改 `extract_claude_trace_detail`**

完整替换成：

```rust
fn extract_claude_trace_detail(v: &Value) -> TurnTraceDetail {
    let mut thinking_text: Option<String> = None;
    let mut tool_calls = Vec::new();
    let mut tool_result_ids: Vec<String> = Vec::new();
    let Some(blocks) = v
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_array())
    else {
        return TurnTraceDetail::default();
    };
    for b in blocks {
        match b.get("type").and_then(|t| t.as_str()) {
            Some("thinking") => {
                if let Some(t) = b.get("thinking").and_then(|t| t.as_str()) {
                    match &mut thinking_text {
                        Some(existing) => {
                            existing.push('\n');
                            existing.push_str(t);
                        }
                        None => thinking_text = Some(t.to_string()),
                    }
                }
            }
            Some("tool_use") => {
                let name = b.get("name").and_then(|n| n.as_str()).unwrap_or("工具");
                let input = b.get("input").cloned().unwrap_or(Value::Null);
                let id = b.get("id").and_then(|i| i.as_str()).map(str::to_string);
                tool_calls.push(ToolCallInfo {
                    summary: tool_summary(name, &input),
                    input_json: input_json_of(&input),
                    id,
                });
            }
            Some("tool_result") => {
                if let Some(id) = b.get("tool_use_id").and_then(|i| i.as_str()) {
                    tool_result_ids.push(id.to_string());
                }
            }
            _ => {}
        }
    }
    // 一行原始消息可能合并了多个 tool_result block(见
    // parse_claude_shaped_chunk 的 tool_result 分支，把同一条消息里的多个
    // block 拼成一个 ParsedTurn)——只有恰好一个时才能确定这行对应哪次调用，
    // 零个/多个都回落 None，不瞎猜。
    let tool_result_call_id = match tool_result_ids.as_slice() {
        [id] => Some(id.clone()),
        _ => None,
    };
    TurnTraceDetail {
        thinking_text,
        tool_calls,
        tool_result_call_id,
    }
}
```

- [x] **Step 4: 运行测试确认通过**

```bash
cargo test -p dozerd extract_claude_trace_detail_
```

Expected: Task 1 + Task 2 新增的所有 `extract_claude_trace_detail_*` 测试全部 PASS。

- [x] **Step 5: Commit**

```bash
git add crates/dozerd/src/transcripts/parse.rs
git commit -m "feat(protocol): Claude tool_result.tool_use_id 接入读时解析（单 block 才给 id）"
```

---

### Task 3: Goose `tool_call_id` 提取（调用侧 + 结果侧）

**Files:**
- Modify: `crates/dozerd/src/transcripts/parse.rs`（`extract_goose_trace_detail` 扩展支持 `PostToolUse`/`PostToolUseFailure`）

**Interfaces:**
- Consumes: Task 1 的 `TurnTraceDetail.tool_result_call_id`。

- [x] **Step 1: 写失败的测试**

```rust
#[test]
fn extract_goose_trace_detail_pre_tool_use_reads_call_id() {
    let raw = r#"{"type":"goose_hook","event":"PreToolUse","payload":{"tool_call_id":"tc-1","tool_name":"developer__shell","tool_input":{"command":"cargo test"}}}"#;
    let detail = extract_turn_trace_detail(raw, AgentKind::Goose);
    assert_eq!(detail.tool_calls.len(), 1);
    assert_eq!(detail.tool_calls[0].id.as_deref(), Some("tc-1"));
}

#[test]
fn extract_goose_trace_detail_post_tool_use_reads_call_id() {
    let raw = r#"{"type":"goose_hook","event":"PostToolUse","payload":{"tool_call_id":"tc-1","tool_name":"developer__shell"}}"#;
    let detail = extract_turn_trace_detail(raw, AgentKind::Goose);
    assert_eq!(detail.tool_calls.len(), 0);
    assert_eq!(detail.tool_result_call_id.as_deref(), Some("tc-1"));
}

#[test]
fn extract_goose_trace_detail_post_tool_use_failure_reads_call_id() {
    let raw = r#"{"type":"goose_hook","event":"PostToolUseFailure","payload":{"tool_call_id":"tc-2","tool_name":"developer__shell"}}"#;
    let detail = extract_turn_trace_detail(raw, AgentKind::Goose);
    assert_eq!(detail.tool_result_call_id.as_deref(), Some("tc-2"));
}

#[test]
fn extract_goose_trace_detail_other_events_stay_default() {
    let raw = r#"{"type":"goose_hook","event":"Stop","payload":{"last_assistant_message":"done"}}"#;
    let detail = extract_turn_trace_detail(raw, AgentKind::Goose);
    assert_eq!(detail, super::TurnTraceDetail::default());
}
```

- [x] **Step 2: 运行测试确认失败**

```bash
cargo test -p dozerd extract_goose_trace_detail_post_tool_use_reads_call_id
```

Expected: FAIL（`extract_goose_trace_detail` 目前只处理 `PreToolUse`，其余事件一律返回 `default()`）。

- [x] **Step 3: 修改 `extract_goose_trace_detail`**

完整替换成：

```rust
fn extract_goose_trace_detail(v: &Value) -> TurnTraceDetail {
    if v.get("type").and_then(|t| t.as_str()) != Some("goose_hook") {
        return TurnTraceDetail::default();
    }
    let Some(event) = v.get("event").and_then(|e| e.as_str()) else {
        return TurnTraceDetail::default();
    };
    let Some(payload) = v.get("payload") else {
        return TurnTraceDetail::default();
    };
    // tool_call_id 在 PreToolUse/PostToolUse/PostToolUseFailure 三个事件里
    // 是同一个值(goose-docs.ai hooks 文档:"Stable identifier for one tool
    // call...Correlates the events of a single call")，按事件类型分别喂给
    // tool_calls[].id 或 tool_result_call_id。
    let call_id = payload
        .get("tool_call_id")
        .and_then(|i| i.as_str())
        .map(str::to_string);
    match event {
        "PreToolUse" => {
            let name = payload
                .get("tool_name")
                .and_then(|t| t.as_str())
                .unwrap_or("工具");
            let input = payload.get("tool_input").cloned().unwrap_or(Value::Null);
            TurnTraceDetail {
                thinking_text: None,
                tool_calls: vec![ToolCallInfo {
                    summary: tool_summary(name, &input),
                    input_json: input_json_of(&input),
                    id: call_id,
                }],
                tool_result_call_id: None,
            }
        }
        "PostToolUse" | "PostToolUseFailure" => TurnTraceDetail {
            thinking_text: None,
            tool_calls: Vec::new(),
            tool_result_call_id: call_id,
        },
        _ => TurnTraceDetail::default(),
    }
}
```

- [x] **Step 4: 运行测试确认通过**

```bash
cargo test -p dozerd extract_goose_trace_detail_
cargo test -p dozerd goose_
```

Expected: 新增 4 个测试通过；已有的 `goose_pre_tool_use_trace_detail_keeps_tool_name_and_input`（如果存在同名或类似前缀的既有测试）等历史测试不受影响，继续通过。

- [x] **Step 5: Commit**

```bash
git add crates/dozerd/src/transcripts/parse.rs
git commit -m "feat(protocol): Goose tool_call_id 接入读时解析（Pre/Post/PostFailure 三个事件）"
```

---

### Task 4: `get_conversation_turns` 透传新字段 + 端到端测试

**Files:**
- Modify: `crates/dozerd/src/transcripts/mod.rs`

**Interfaces:**
- Consumes: Task 1-3 的 `TurnTraceDetail.tool_result_call_id`。
- Produces: `get_conversation_turns` 返回的 `TurnRecord` 带上 `tool_calls[].id` 和 `tool_result_call_id`。

- [x] **Step 1: 写失败的测试**（`crates/dozerd/src/transcripts/mod.rs` 测试模块，紧跟
  `get_conversation_turns_surfaces_thinking_text_and_tool_calls` 之后）

```rust
#[test]
fn get_conversation_turns_surfaces_tool_call_and_result_ids() {
    let tmp = tempfile::tempdir().unwrap();
    let store = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
    let text = concat!(
        "{\"type\":\"assistant\",\"uuid\":\"a1\",\"timestamp\":100,\"message\":{\"content\":[",
        "{\"type\":\"tool_use\",\"id\":\"toolu_01abc\",\"name\":\"Edit\",",
        "\"input\":{\"file_path\":\"README.md\"}},",
        "{\"type\":\"text\",\"text\":\"改好了\"}]}}\n",
        "{\"type\":\"user\",\"uuid\":\"u1\",\"timestamp\":200,\"message\":{\"role\":\"user\",",
        "\"content\":[{\"tool_use_id\":\"toolu_01abc\",\"type\":\"tool_result\",",
        "\"content\":\"done\"}]}}\n"
    );
    let file = fixture(tmp.path(), "s3.jsonl", text);
    store.ingest_session(AgentKind::Claude, &file).unwrap();

    let turns = store.get_conversation_turns("s3", -1, 10).unwrap();
    let ai_turn = turns.iter().find(|t| t.role == "ai").unwrap();
    assert_eq!(ai_turn.tool_calls[0].id.as_deref(), Some("toolu_01abc"));
    let result_turn = turns.iter().find(|t| t.role == "tool_result").unwrap();
    assert_eq!(result_turn.tool_result_call_id.as_deref(), Some("toolu_01abc"));
}
```

- [x] **Step 2: 运行测试确认失败**

```bash
cargo test -p dozerd get_conversation_turns_surfaces_tool_call_and_result_ids
```

Expected: FAIL（`TurnRecord` 构造还没接上 `tool_result_call_id`）。

- [x] **Step 3: 修改 `get_conversation_turns`**

找到 `Ok(dozer_core::protocol::TurnRecord { ... })` 那段构造，在 `is_error` 之后加一行：

```rust
            Ok(dozer_core::protocol::TurnRecord {
                turn_index: row.get(0)?,
                role: row.get(1)?,
                content: row.get(2)?,
                tool_calls: detail.tool_calls,
                thinking: row.get::<_, i64>(3)? != 0,
                thinking_text: detail.thinking_text,
                ts: row.get(4)?,
                is_error: row.get::<_, i64>(5)? != 0,
                tool_result_call_id: detail.tool_result_call_id,
                tokens_in: row.get(8)?,
                tokens_out: row.get(9)?,
                tokens_cache_read: row.get(10)?,
                tokens_cache_write: row.get(11)?,
            })
```

- [x] **Step 4: 运行测试确认通过**

```bash
cargo test -p dozerd get_conversation_turns_surfaces_tool_call_and_result_ids
cargo test -p dozerd 2>&1 | tail -10
```

Expected: 新测试通过；`cargo test -p dozerd` 全量跑一遍无回归。

- [x] **Step 5: Commit**

```bash
git add crates/dozerd/src/transcripts/mod.rs
git commit -m "feat(protocol): get_conversation_turns 透传调用/结果 id"
```

---

### Task 5: `dozer-app::transcript.rs` 透传进 `ReviewEntry`

**Files:**
- Modify: `crates/dozer-app/src/transcript.rs`

**Interfaces:**
- Consumes: Task 4 的 `TurnRecord.tool_result_call_id`（跨 crate，`dozer_core::protocol`）。
- Produces: `ToolResultEntry.call_id: Option<String>`、`ReviewEntry::ToolResult.call_id: Option<String>`，供 Task 6 的 `data.json` 契约使用。

- [x] **Step 1: 修改 `ToolResultEntry` 和 `ReviewEntry::ToolResult`**

```rust
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ToolResultEntry {
    pub content: String,
    pub is_error: bool,
    /// 这条结果对应的调用 id；拿不到时 None，前端回落下标近似。见
    /// docs/superpowers/specs/2026-09-25-tool-call-result-pairing-design.md。
    pub call_id: Option<String>,
}
```

`ReviewEntry::ToolResult` 变体（顶层孤儿变体，理由同 `ToolResultEntry`）：

```rust
    ToolResult { content: String, is_error: bool, call_id: Option<String> },
```

- [x] **Step 2: 修改 `review_entries_from_turns` 的折叠逻辑**

找到 `"tool_result" => { ... }` 分支，改成：

```rust
            "tool_result" => {
                let entry = ToolResultEntry {
                    content: t.content.clone(),
                    is_error: t.is_error,
                    call_id: t.tool_result_call_id.clone(),
                };
                match out.last_mut() {
                    Some(ReviewEntry::AiTurn { tool_results, .. }) => {
                        tool_results.push(entry);
                    }
                    _ => out.push(ReviewEntry::ToolResult {
                        content: entry.content,
                        is_error: entry.is_error,
                        call_id: entry.call_id,
                    }),
                }
            }
```

- [x] **Step 3: 修编译错误**

```bash
cargo check -p dozer-app 2>&1 | grep "error\[" -A5
```

Expected: 报出这个文件测试模块里所有 `ToolCallInfo { .. }`/`ToolResultEntry { .. }`/`ReviewEntry::ToolResult { .. }` 缺字段的位置（大约 8 处，含 `tool_calls: vec![ToolCallInfo { .. }]` 和 `tool_results: vec![ToolResultEntry { .. }]` 两种写法）。逐个按"`ToolCallInfo` 补 `id: None`，`ToolResultEntry`/`ReviewEntry::ToolResult` 补 `call_id: None`"补齐，直到 `cargo check -p dozer-app` 干净。这些都是机械补字段，不涉及行为判断——编译器报哪一行就补哪一行。

- [x] **Step 4: 修一处会失败的精确 JSON 断言测试**（不是编译错误，是运行时断言，编译器发现不了）

找到这段序列化测试（大约在 `review_entries_from_turns_maps_ai_turn_with_thinking_and_tool_calls` 之前）：

```rust
        let tool = ReviewEntry::ToolResult {
            content: "boom".into(),
            is_error: true,
        };
        assert_eq!(
            serde_json::to_string(&tool).unwrap(),
            r#"{"ToolResult":{"content":"boom","is_error":true}}"#
        );
```

改成：

```rust
        let tool = ReviewEntry::ToolResult {
            content: "boom".into(),
            is_error: true,
            call_id: None,
        };
        assert_eq!(
            serde_json::to_string(&tool).unwrap(),
            r#"{"ToolResult":{"content":"boom","is_error":true,"call_id":null}}"#
        );
```

（`call_id` 序列化成显式 `null` 而不是被省略——`ToolCallInfo.input_json`/`id` 这两个 `Option` 字段现有代码里也没用 `skip_serializing_if`，保持同一约定。）

- [x] **Step 5: 写一个端到端传播测试**（新增，放在 Step 4 改的那个测试模块里）

```rust
#[test]
fn review_entries_from_turns_propagates_tool_result_call_id() {
    use dozer_core::protocol::{ToolCallInfo, TurnRecord};
    let turns = vec![
        TurnRecord {
            role: "ai".into(),
            content: "done".into(),
            tool_calls: vec![ToolCallInfo {
                summary: "Edit README.md".into(),
                input_json: None,
                id: Some("toolu_1".into()),
            }],
            ..Default::default()
        },
        TurnRecord {
            role: "tool_result".into(),
            content: "ok".into(),
            tool_result_call_id: Some("toolu_1".into()),
            ..Default::default()
        },
    ];
    let entries = review_entries_from_turns(&turns);
    match &entries[0] {
        ReviewEntry::AiTurn {
            tool_calls,
            tool_results,
            ..
        } => {
            assert_eq!(tool_calls[0].id.as_deref(), Some("toolu_1"));
            assert_eq!(tool_results[0].call_id.as_deref(), Some("toolu_1"));
        }
        other => panic!("expected AiTurn, got {other:?}"),
    }
}
```

- [x] **Step 6: 运行测试确认通过**

```bash
cargo test -p dozer-app transcript:: 2>&1 | tail -30
```

Expected: 该模块全部测试（含 Step 4 改的和 Step 5 新增的）通过。

- [x] **Step 7: Commit**

```bash
git add crates/dozer-app/src/transcript.rs
git commit -m "feat(review-trace): ToolResultEntry/ReviewEntry::ToolResult 透传调用 id"
```

---

### Task 6: 前端精确配对（`types.ts` + `TraceToggle.tsx`）

**Files:**
- Modify: `crates/dozer-app/web/review-trace/src/types.ts`
- Modify: `crates/dozer-app/web/review-trace/src/components/TraceToggle.tsx`
- Create: `crates/dozer-app/web/review-trace/src/TraceToggle.test.ts`

**Interfaces:**
- Consumes: `data.json` 新字段 `ToolCall.id?`/`ToolResult.call_id?`。
- Produces: `TraceToggle` 组件行为不变的对外接口（`{ v: AiTurnData }`），内部配对逻辑升级。

- [x] **Step 1: 修改 `types.ts`**

```ts
export interface ToolCall {
  summary: string;
  input_json?: string;
  id?: string;
}

export interface ToolResult {
  content: string;
  is_error: boolean;
  call_id?: string;
}
```

- [x] **Step 2: 写失败的测试**（`crates/dozer-app/web/review-trace/src/TraceToggle.test.ts`，新文件）

```ts
import test from 'node:test';
import assert from 'node:assert/strict';
import { h } from 'preact';
import render from 'preact-render-to-string';
import { TraceToggle } from './components/TraceToggle.tsx';
import type { AiTurnData } from './types.ts';

function renderTrace(v: AiTurnData): string {
  return render(h(TraceToggle, { v }));
}

function itemClasses(html: string): string[] {
  return [...html.matchAll(/class="(trace-item[^"]*)"/g)].map((m) => m[1]);
}

test('all calls have id: pairs each call with its matching result, in call order', () => {
  const v: AiTurnData = {
    tool_calls: [
      { summary: 'call A', id: 'a' },
      { summary: 'call B', id: 'b' },
    ],
    tool_results: [
      { content: 'result B', is_error: false, call_id: 'b' },
      { content: 'result A', is_error: false, call_id: 'a' },
    ],
  };
  const html = renderTrace(v);
  const order = ['call A', 'result A', 'call B', 'result B'].map((s) => html.indexOf(s));
  assert.deepEqual(order, [...order].sort((a, b) => a - b));
});

test('one call id maps to multiple results: stacked in encounter order after that call', () => {
  const v: AiTurnData = {
    tool_calls: [{ summary: 'call A', id: 'a' }],
    tool_results: [
      { content: 'first', is_error: false, call_id: 'a' },
      { content: 'second', is_error: false, call_id: 'a' },
    ],
  };
  const html = renderTrace(v);
  assert.ok(html.indexOf('first') < html.indexOf('second'));
  assert.equal(itemClasses(html).filter((c) => c.includes('tool-result')).length, 2);
});

test('result with no matching call_id is appended at the end, not dropped', () => {
  const v: AiTurnData = {
    tool_calls: [{ summary: 'call A', id: 'a' }],
    tool_results: [
      { content: 'orphan', is_error: false, call_id: 'unknown' },
      { content: 'matched', is_error: false, call_id: 'a' },
    ],
  };
  const html = renderTrace(v);
  assert.ok(html.includes('orphan'));
  assert.ok(html.indexOf('matched') < html.indexOf('orphan'));
});

test('any call missing id falls back to index pairing for the whole turn', () => {
  const v: AiTurnData = {
    tool_calls: [{ summary: 'call A', id: 'a' }, { summary: 'call B' }],
    tool_results: [
      { content: 'result 1', is_error: false, call_id: 'a' },
      { content: 'result 2', is_error: false },
    ],
  };
  const html = renderTrace(v);
  const order = ['call A', 'result 1', 'call B', 'result 2'].map((s) => html.indexOf(s));
  assert.deepEqual(order, [...order].sort((a, b) => a - b));
});

test('no ids anywhere: behaves exactly like the existing index-based pairing', () => {
  const v: AiTurnData = {
    tool_calls: [{ summary: 'call A' }, { summary: 'call B' }],
    tool_results: [
      { content: 'result A', is_error: false },
      { content: 'result B', is_error: false },
    ],
  };
  const html = renderTrace(v);
  const order = ['call A', 'result A', 'call B', 'result B'].map((s) => html.indexOf(s));
  assert.deepEqual(order, [...order].sort((a, b) => a - b));
});
```

- [x] **Step 3: 运行测试确认失败**

```bash
cd crates/dozer-app/web/review-trace
node --test src/TraceToggle.test.ts
```

Expected: FAIL 或部分 FAIL（现在的实现是纯下标近似，前两个"id 配对"用例会因为 `call A`/`result A` 顺序不对而失败）。

- [x] **Step 4: 重写 `TraceToggle.tsx`**

```tsx
import { Fragment } from 'preact';
import type { AiTurnData, ToolCall, ToolResult } from '../types.ts';
import { buildTraceStatsLabel } from '../traceStats.ts';
import { renderMarkdown } from '../markdown.ts';
import { ToolCallRow } from './ToolCallRow.tsx';
import { ToolResultRow } from './ToolResultRow.tsx';

// 每个 call 的展示单元：这条调用本身(可能没有，见"未配对结果"那一组) +
// 配对上的结果列表(可能 0/1/多个)。
interface Pairing {
  call?: ToolCall;
  results: ToolResult[];
}

// 下标近似(现状，2026-09-25 之前唯一实现):call[i]/result[i] 相邻渲染。
function pairByIndex(toolCalls: ToolCall[], toolResults: ToolResult[]): Pairing[] {
  const pairCount = Math.max(toolCalls.length, toolResults.length);
  return Array.from({ length: pairCount }, (_, i) => ({
    call: toolCalls[i],
    results: toolResults[i] ? [toolResults[i]] : [],
  }));
}

// 精确配对：按 call.id 把 result 分组挂到对应的 call 后面，一个 id 对应
// 多个 result 时按遇到顺序堆叠;call_id 对不上任何 call(或没有 call_id)的
// result 单独追加一组，放在最后，不能丢。
function pairById(toolCalls: ToolCall[], toolResults: ToolResult[]): Pairing[] {
  const resultsByCallId = new Map<string, ToolResult[]>();
  const unmatched: ToolResult[] = [];
  for (const result of toolResults) {
    const matches = result.call_id && toolCalls.some((c) => c.id === result.call_id);
    if (matches) {
      const list = resultsByCallId.get(result.call_id as string) ?? [];
      list.push(result);
      resultsByCallId.set(result.call_id as string, list);
    } else {
      unmatched.push(result);
    }
  }
  const pairs = toolCalls.map((call) => ({
    call,
    results: resultsByCallId.get(call.id as string) ?? [],
  }));
  if (unmatched.length > 0) {
    pairs.push({ results: unmatched });
  }
  return pairs;
}

// 只有全部 tool_calls 都带 id 才进精确配对，只要缺一个就整体回落下标
// 近似——不做"部分 id、部分下标"的混合模式，行为不可预期。见
// docs/superpowers/specs/2026-09-25-tool-call-result-pairing-design.md。
function pairCallsAndResults(toolCalls: ToolCall[], toolResults: ToolResult[]): Pairing[] {
  const allCallsHaveId = toolCalls.length > 0 && toolCalls.every((c) => !!c.id);
  return allCallsHaveId ? pairById(toolCalls, toolResults) : pairByIndex(toolCalls, toolResults);
}

export function TraceToggle({ v }: { v: AiTurnData }) {
  const hasThinking = !!v.thinking_text;
  const toolCalls = v.tool_calls ?? [];
  const toolResults = v.tool_results ?? [];
  const hasToolCalls = toolCalls.length > 0;
  const hasToolResults = toolResults.length > 0;
  if (!hasThinking && !hasToolCalls && !hasToolResults) {
    return null;
  }

  const statsLabel = buildTraceStatsLabel(v);
  const pairs = pairCallsAndResults(toolCalls, toolResults);
  const firstResultPairIndex = pairs.findIndex((p) => p.results.length > 0);

  return (
    <details class="trace-toggle">
      <summary>{statsLabel ? <span class="trace-stats">{statsLabel}</span> : null}</summary>
      <div class="trace-timeline">
        {hasThinking ? (
          <div class="trace-item thinking">
            <div class="item-title">思考过程</div>
            <div class="thinking-text">{renderMarkdown(v.thinking_text as string)}</div>
          </div>
        ) : null}
        {pairs.map((pair, i) => (
          <Fragment key={i}>
            {pair.call ? (
              <div class="trace-item tool-call">
                {i === 0 ? <div class="item-title">操作过程</div> : null}
                <ToolCallRow call={pair.call} />
              </div>
            ) : null}
            {pair.results.map((result, j) => (
              <div class={`trace-item tool-result${result.is_error ? ' error' : ''}`} key={j}>
                {i === firstResultPairIndex && j === 0 ? (
                  <div class="item-title">工具结果</div>
                ) : null}
                <ToolResultRow result={result} />
              </div>
            ))}
          </Fragment>
        ))}
      </div>
    </details>
  );
}
```

- [x] **Step 5: 运行测试确认通过**

```bash
node --test src/TraceToggle.test.ts
npm run typecheck
```

Expected: 5 个新测试全部 PASS；typecheck 干净。

- [x] **Step 6: Commit**

```bash
git add crates/dozer-app/web/review-trace/src/types.ts \
  crates/dozer-app/web/review-trace/src/components/TraceToggle.tsx \
  crates/dozer-app/web/review-trace/src/TraceToggle.test.ts
git commit -m "feat(review-trace): TraceToggle 按 id 精确配对，缺 id 时回落下标近似"
```

---

### Task 7: 全量构建 + 全量测试 + 人工视觉核对

**Files:**
- 无新增/修改文件；本任务是集成验证。

**Interfaces:**
- Consumes: Task 1-6 全部产出。

- [x] **Step 1: 前端全量检查 + 重新构建 bundle**

```bash
cd crates/dozer-app/web/review-trace
npm run typecheck
npm test
npm run build
```

Expected: 三条命令全部通过；`npm run build` 打印 `built -> .../assets/review-trace`。

- [x] **Step 2: Rust 全量测试**

```bash
cd ../../../..
cargo test -p dozer-core -p dozerd -p dozer-app
```

Expected: 全部通过，无回归。

- [x] **Step 3: 人工视觉核对（真实并行工具调用场景）**

准备一份带并行工具调用 id 的 fixture（`crates/dozer-app/assets/review-trace/data.json`，验证完删除，不要
`git add`）：

```json
{
  "entries": [
    {
      "AiTurn": {
        "text": "并行读了三个文件。",
        "tool_calls": [
          { "summary": "Read a.md", "id": "t1" },
          { "summary": "Read b.md", "id": "t2" },
          { "summary": "Read c.md", "id": "t3" }
        ],
        "tool_results": [
          { "content": "b.md 的内容", "is_error": false, "call_id": "t2" },
          { "content": "a.md 的内容", "is_error": false, "call_id": "t1" },
          { "content": "c.md 的内容", "is_error": false, "call_id": "t3" }
        ]
      }
    }
  ]
}
```

这份数据故意把 `tool_results` 的顺序打乱（先 t2 再 t1 再 t3），验证的正是"下标近似会配错、id 配对不会"这个场景——旧实现（Task 6 之前）会把 "Read a.md" 后面接上 "b.md 的内容"（下标错位）；新实现应该正确地把 "Read a.md" 接 "a.md 的内容"。

用 `crates/dozer-app/assets/review-trace/` 目录起本地静态服务器核对（同
`2026-09-25-review-trace-preact-migration.md` Task 11 的手法：临时把
`review-trace.js` 里的 `dozer://review-trace/data.json` 改成相对路径
`data.json`，`python3 -m http.server` 起服务，浏览器打开确认三个调用各自
紧跟正确的结果，然后 `git checkout` 撤销 js 改动、删掉临时 `data.json`）。

- [x] **Step 4: 无需 commit**（本任务不产出提交内容，只是验证）

---

### Task 8: 收尾质量门

**Files:**
- 无新增/修改文件；本任务是最终 lint/build 校验。

**Interfaces:**
- Consumes: Task 1-7 全部产出。

- [x] **Step 1: 全 workspace 构建**

```bash
cargo build
```

Expected: 编译通过，无新增警告。

- [x] **Step 2: clippy + fmt**

```bash
cargo clippy --all-targets
cargo fmt --check
```

Expected: 无新增警告；`fmt --check` 无差异（有差异就 `cargo fmt` 后重新 `git add` 提交）。

- [x] **Step 3: 确认没有遗漏的消费方**（对应 Global Constraints 里"已核实不需要改"的那条,收尾时再核一遍防止过程中有新代码引入依赖）

```bash
grep -rn "ToolCallInfo\|TurnRecord" crates/ --include="*.rs" | grep -v "target/"
```

Expected: 结果范围仍然只在 spec 里列出的那几个文件内，没有出现新的、这次改动漏处理的构造点。

- [x] **Step 4: 最终收尾 commit（如果 fmt/clippy 产生了改动）**

```bash
git status --short
# 如果有改动:
git add -A
git commit -m "chore(tool-call-pairing): cargo fmt/clippy 收尾"
```

Expected: 工作区干净，`git log --oneline -10` 能看到本计划 Task 1-7 的全部提交。
