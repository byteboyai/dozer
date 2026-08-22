# 会话审阅面板：话题上下文 + 三段式 trace 折叠面板 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 会话审阅面板的 AI 回合从"单一思考徽标 + 一行工具摘要 + 独立的工具结果条目"，改成"思考过程/操作过程/工具结果"三段独立折叠的 trace 面板，并在展开的话题上下加同一 session 内前一话题/下一话题的静态预览。

**Architecture:** 真实思考文本、工具调用完整 `input` JSON，已经作为 `raw_json` 全量持久化在 `dozerd` 的 `conversation_turns` 表里，只是解析时被丢弃。新增一个纯函数 `extract_turn_trace_detail`，只在用户打开审阅面板时（`get_conversation_turns` 查询期间）对 `raw_json` 做一次读时解析，不碰摄取热路径、不加 DB 迁移。`ReviewEntry::AiTurn` 从"文本+工具摘要+思考布尔位"升级为"文本+思考文本+结构化工具调用+紧跟它的工具结果"，工具结果的归属用位置邻接（同一 `AiTurn` 之后、下一个 `Human`/`AiTurn` 之前的连续 `ToolResult` 都算它的）。前一话题/下一话题从已经加载好的 `conversation_turn_groups` 列表里按同一 session（同 `path`）、按 `start_turn_index` 排序取邻居，纯静态文字预览，不新增请求、不可点击跳转。

**Tech Stack:** Rust（`dozer-core`/`dozerd`/`dozer-app`），rusqlite，serde_json，wry webview（`review_trace.html`，原生 HTML/CSS/JS，无框架）。

**Spec:** `docs/superpowers/specs/2026-08-22-review-trace-detail-revamp-design.md`

## Global Constraints

- 不改摄取路径：`parse_claude_shaped_chunk`/`parse_codebuddy_shaped_chunk`（`crates/dozerd/src/transcripts/parse.rs`）原样不动。
- 不做 DB 迁移/历史数据回填：`raw_json` 列已存在，读时解析即可覆盖全部历史数据。
- 工具调用 ↔ 结果的归属用位置邻接，不引入 `tool_use_id` 精确匹配。
- 前一/下一话题只在同一 session（同 `TurnGroupRow.path`）内，不跨 session；纯静态预览，不可点击跳转；预览文字用已加载的 `TurnGroupRow.title`，不额外发请求抓正文。
- `crates/dozer-app/src/review_trace.html` 没有自动化测试基础设施，人工 GUI 验收，符合现状（同 Files/Project/browser 三个面板）。

---

## Task 1: `dozer-core::protocol` — 新增 `ToolCallInfo`，`TurnRecord` 换字段

**Files:**
- Modify: `crates/dozer-core/src/protocol.rs:99-108`（`TurnRecord` 定义）
- Modify: `crates/dozer-core/src/protocol.rs:430-524`（`conversation_protocol_types_roundtrip`、`turn_record_is_error_field_roundtrips` 两个既有测试，字段跟着改）

**Interfaces:**
- Produces: `pub struct ToolCallInfo { pub summary: String, pub input_json: Option<String> }`（`Debug, Clone, PartialEq, Serialize, Deserialize`）；`TurnRecord` 的 `tools_summary: Vec<String>` 替换为 `tool_calls: Vec<ToolCallInfo>`，新增 `thinking_text: Option<String>`（`#[serde(default)]`，跟现有 `is_error` 同款老协议帧兼容写法）。

- [ ] **Step 1: 改 `TurnRecord` 定义，新增 `ToolCallInfo`**

在 `crates/dozer-core/src/protocol.rs:99-108`，把：

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TurnRecord {
    pub turn_index: i64,
    pub role: String,
    pub content: String,
    pub tools_summary: Vec<String>,
    pub thinking: bool,
    pub ts: Option<u64>,
    #[serde(default)]
    pub is_error: bool,
}
```

改成：

```rust
/// 一次工具调用的结构化明细：`summary` 是既有的一行摘要（`tool_summary()`
/// 产出，如 "Edit README.md"），`input_json` 是完整 `input`/`arguments`
/// pretty-print 后的 JSON 字符串，`None` 表示没有参数或解析失败。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCallInfo {
    pub summary: String,
    pub input_json: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TurnRecord {
    pub turn_index: i64,
    pub role: String,
    pub content: String,
    pub tool_calls: Vec<ToolCallInfo>,
    pub thinking: bool,
    /// 真实思考文本（2026-08-22 起读时解析补上，见
    /// `dozerd::transcripts::parse::extract_turn_trace_detail`）；老协议帧/
    /// 无思考内容时为 `None`。
    #[serde(default)]
    pub thinking_text: Option<String>,
    pub ts: Option<u64>,
    #[serde(default)]
    pub is_error: bool,
}
```

- [ ] **Step 2: 改现有测试的字段字面量**

`crates/dozer-core/src/protocol.rs:467-475`（`conversation_protocol_types_roundtrip` 里的 `turn` 变量）：

```rust
let turn = TurnRecord {
    turn_index: 0,
    role: "human".into(),
    content: "你好".into(),
    tool_calls: vec![ToolCallInfo {
        summary: "Edit README.md".into(),
        input_json: Some("{\"file_path\":\"README.md\"}".into()),
    }],
    thinking: true,
    thinking_text: Some("先看看现有实现".into()),
    ts: Some(42),
    is_error: false,
};
```

`crates/dozer-core/src/protocol.rs:511-519`（`turn_record_is_error_field_roundtrips` 里的 `turn` 变量）：

```rust
let turn = TurnRecord {
    turn_index: 0,
    role: "tool_result".into(),
    content: "boom".into(),
    tool_calls: vec![],
    thinking: false,
    thinking_text: None,
    ts: Some(1),
    is_error: true,
};
```

- [ ] **Step 3: 跑测试确认通过**

Run: `cargo test -p dozer-core`
Expected: `conversation_protocol_types_roundtrip`、`turn_record_is_error_field_roundtrips` 均 PASS。

- [ ] **Step 4: fmt + clippy**

Run: `cargo fmt -p dozer-core && cargo clippy -p dozer-core --all-targets`
Expected: 无警告。

- [ ] **Step 5: Commit**

```bash
git add crates/dozer-core/src/protocol.rs
git commit -m "feat(protocol): TurnRecord 加 thinking_text，tools_summary 换成结构化 ToolCallInfo"
```

---

## Task 2: `dozerd::transcripts::parse` — 读时提取思考文本 + 工具参数

**Files:**
- Modify: `crates/dozerd/src/transcripts/parse.rs`（文件顶部 `use` 块、文件末尾新增函数与测试模块）

**Interfaces:**
- Consumes: `dozer_core::protocol::{AgentKind, ToolCallInfo}`（Task 1 产出）；`tool_summary(name: &str, input: &Value) -> String`（已有私有函数，`parse.rs:84`）；`join_codebuddy_text_blocks(blocks: &[Value], kind: &str) -> String`（已有私有函数，`parse.rs:284`）。
- Produces: `pub struct TurnTraceDetail { pub thinking_text: Option<String>, pub tool_calls: Vec<ToolCallInfo> }`（`Debug, Clone, PartialEq, Default`）；`pub fn extract_turn_trace_detail(raw_json: &str, agent: AgentKind) -> TurnTraceDetail`。Task 3 直接调用这个函数。

- [ ] **Step 1: 顶部 `use` 加 `ToolCallInfo`**

`crates/dozerd/src/transcripts/parse.rs:5`，把：

```rust
use dozer_core::protocol::AgentKind;
```

改成：

```rust
use dozer_core::protocol::{AgentKind, ToolCallInfo};
```

- [ ] **Step 2: 写失败的单测（Claude 形状：thinking + 单个 tool_use）**

在文件末尾新增 `#[cfg(test)] mod trace_detail_tests`（跟已有 `mod tests`平级，独立模块避免跟已有 fixture 常量混淆）：

```rust
#[cfg(test)]
mod trace_detail_tests {
    use super::*;

    #[test]
    fn extract_claude_trace_detail_reads_thinking_text_and_tool_input() {
        let raw = r#"{"type":"assistant","message":{"content":[
            {"type":"thinking","thinking":"先看看现有实现"},
            {"type":"tool_use","name":"Edit","input":{"file_path":"README.md","old_string":"a","new_string":"b"}},
            {"type":"text","text":"改好了"}
        ]}}"#;
        let detail = extract_turn_trace_detail(raw, AgentKind::Claude);
        assert_eq!(detail.thinking_text.as_deref(), Some("先看看现有实现"));
        assert_eq!(detail.tool_calls.len(), 1);
        assert_eq!(detail.tool_calls[0].summary, "Edit README.md");
        let input_json = detail.tool_calls[0].input_json.as_deref().unwrap();
        assert!(input_json.contains("README.md"));
        assert!(input_json.contains("old_string"));
    }

    #[test]
    fn extract_claude_trace_detail_multiple_tool_use_and_no_thinking() {
        let raw = r#"{"type":"assistant","message":{"content":[
            {"type":"tool_use","name":"Read","input":{"file_path":"a.txt"}},
            {"type":"tool_use","name":"Bash","input":{"command":"ls -la"}}
        ]}}"#;
        let detail = extract_turn_trace_detail(raw, AgentKind::Claude);
        assert_eq!(detail.thinking_text, None);
        assert_eq!(detail.tool_calls.len(), 2);
        assert_eq!(detail.tool_calls[0].summary, "Read a.txt");
        assert_eq!(detail.tool_calls[1].summary, "Bash ls -la");
    }

    #[test]
    fn extract_claude_trace_detail_no_blocks_returns_empty() {
        let raw = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"hi"}]}}"#;
        let detail = extract_turn_trace_detail(raw, AgentKind::Claude);
        assert_eq!(detail, TurnTraceDetail::default());
    }

    #[test]
    fn extract_trace_detail_malformed_json_returns_empty() {
        let detail = extract_turn_trace_detail("not json", AgentKind::Claude);
        assert_eq!(detail, TurnTraceDetail::default());
    }

    #[test]
    fn extract_codebuddy_trace_detail_function_call_has_input() {
        let raw = r#"{"type":"function_call","name":"Edit","arguments":"{\"file_path\":\"a.rs\"}"}"#;
        let detail = extract_turn_trace_detail(raw, AgentKind::Codebuddy);
        assert_eq!(detail.thinking_text, None);
        assert_eq!(detail.tool_calls.len(), 1);
        assert_eq!(detail.tool_calls[0].summary, "Edit a.rs");
        assert!(detail.tool_calls[0].input_json.as_deref().unwrap().contains("a.rs"));
    }

    #[test]
    fn extract_codebuddy_trace_detail_reasoning_with_text_blocks() {
        // CodeBuddy 的 reasoning 事件真实字段名未在现有 fixture 里观测到，
        // 按该文件里 assistant 消息用 "output_text"/"input_text" 类型化
        // block 数组的既有约定类推为 "reasoning_text"；如果拿到真实样本发现
        // 字段名不同，回来改这一个函数即可，不影响其它任何东西。
        let raw = r#"{"type":"reasoning","content":[{"type":"reasoning_text","text":"先想想"}]}"#;
        let detail = extract_turn_trace_detail(raw, AgentKind::Codebuddy);
        assert_eq!(detail.thinking_text.as_deref(), Some("先想想"));
        assert_eq!(detail.tool_calls, Vec::new());
    }

    #[test]
    fn extract_codebuddy_trace_detail_reasoning_without_matching_blocks_is_none() {
        let raw = r#"{"type":"reasoning","content":[{"type":"summary_text","text":"other shape"}]}"#;
        let detail = extract_turn_trace_detail(raw, AgentKind::Codebuddy);
        assert_eq!(detail.thinking_text, None);
    }

    #[test]
    fn extract_trace_detail_unhandled_agent_returns_empty() {
        let detail = extract_turn_trace_detail(r#"{"type":"whatever"}"#, AgentKind::Codex);
        assert_eq!(detail, TurnTraceDetail::default());
    }
}
```

- [ ] **Step 3: 跑测试确认全部失败（函数还不存在）**

Run: `cargo test -p dozerd trace_detail_tests`
Expected: FAIL，报 `extract_turn_trace_detail`/`TurnTraceDetail` 未定义。

- [ ] **Step 4: 实现 `TurnTraceDetail`/`extract_turn_trace_detail`**

在 `crates/dozerd/src/transcripts/parse.rs` 文件末尾（`trace_detail_tests` 模块之前）新增：

```rust
/// 一个回合读时解析出的 trace 明细（思考文本 + 结构化工具调用），只在
/// `dozerd::transcripts::mod::get_conversation_turns` 查询期间对 `raw_json`
/// 现算，不在摄取时落库、不加新列——`raw_json` 本身已经全量持久化，读时
/// 解析一次的代价可忽略（用户打开审阅面板才触发，单个 session 几十行）。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TurnTraceDetail {
    pub thinking_text: Option<String>,
    pub tool_calls: Vec<ToolCallInfo>,
}

/// 从一行原始 JSONL(`raw_json`)按 `agent` 对应的形状提取真实思考文本和
/// 结构化工具调用参数——`parse_claude_shaped_chunk`/`parse_codebuddy_shaped_chunk`
/// 摄取时只留了布尔位/一行摘要，这里是独立的读时补全，**不复用/不修改**
/// 那两个摄取函数(摄取路径是热路径、已有测试覆盖，不承担这次改动的风险；
/// 两边对 `type` 字段的判断口径保持一致即可)。解析失败/形状不认识时返回
/// 全空的 `TurnTraceDetail`，不 panic。
pub fn extract_turn_trace_detail(raw_json: &str, agent: AgentKind) -> TurnTraceDetail {
    let Ok(v) = serde_json::from_str::<Value>(raw_json) else {
        return TurnTraceDetail::default();
    };
    match agent {
        AgentKind::Codebuddy => extract_codebuddy_trace_detail(&v),
        // Claude/Opencode/Kilo/Unknown 摄取时都走 parse_claude_shaped_chunk
        // (parse_chunk 的分派,parse.rs:456-457),读时解析沿用同一分派。
        AgentKind::Claude | AgentKind::Opencode | AgentKind::Kilo | AgentKind::Unknown => {
            extract_claude_trace_detail(&v)
        }
        // Codex/V8agent 目前完全不摄取(parse_chunk 分派到空 Vec,
        // parse.rs:462),没有 raw_json 可读。
        AgentKind::Codex | AgentKind::V8agent => TurnTraceDetail::default(),
    }
}

fn input_json_of(input: &Value) -> Option<String> {
    if input.is_null() {
        None
    } else {
        serde_json::to_string_pretty(input).ok()
    }
}

fn extract_claude_trace_detail(v: &Value) -> TurnTraceDetail {
    let mut thinking_text: Option<String> = None;
    let mut tool_calls = Vec::new();
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
                tool_calls.push(ToolCallInfo {
                    summary: tool_summary(name, &input),
                    input_json: input_json_of(&input),
                });
            }
            _ => {}
        }
    }
    TurnTraceDetail {
        thinking_text,
        tool_calls,
    }
}

fn extract_codebuddy_trace_detail(v: &Value) -> TurnTraceDetail {
    match v.get("type").and_then(|t| t.as_str()) {
        Some("reasoning") => {
            let blocks = v
                .get("content")
                .and_then(|c| c.as_array())
                .cloned()
                .unwrap_or_default();
            let text = join_codebuddy_text_blocks(&blocks, "reasoning_text");
            TurnTraceDetail {
                thinking_text: if text.is_empty() { None } else { Some(text) },
                tool_calls: Vec::new(),
            }
        }
        Some("function_call") => {
            let name = v.get("name").and_then(|n| n.as_str()).unwrap_or("工具");
            let input: Value = v
                .get("arguments")
                .and_then(|a| a.as_str())
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or(Value::Null);
            TurnTraceDetail {
                thinking_text: None,
                tool_calls: vec![ToolCallInfo {
                    summary: tool_summary(name, &input),
                    input_json: input_json_of(&input),
                }],
            }
        }
        _ => TurnTraceDetail::default(),
    }
}
```

- [ ] **Step 5: 跑测试确认全部通过**

Run: `cargo test -p dozerd trace_detail_tests`
Expected: 全部 PASS（8 个测试）。

- [ ] **Step 6: fmt + clippy**

Run: `cargo fmt -p dozerd && cargo clippy -p dozerd --all-targets`
Expected: 无警告。

- [ ] **Step 7: Commit**

```bash
git add crates/dozerd/src/transcripts/parse.rs
git commit -m "feat(dozerd): 新增读时解析 raw_json 提取真实思考文本/工具参数"
```

---

## Task 3: `dozerd::transcripts::mod` — `get_conversation_turns` 接入读时解析

**Files:**
- Modify: `crates/dozerd/src/transcripts/mod.rs:261-286`（`get_conversation_turns` 函数体）
- Modify: `crates/dozerd/src/transcripts/mod.rs:869-892`（既有 `get_conversation_turns_paginates_by_keyset` 测试，字段访问跟着改）

**Interfaces:**
- Consumes: `parse::extract_turn_trace_detail(raw_json: &str, agent: AgentKind) -> TurnTraceDetail`（Task 2 产出）；`agent_from_str(s: &str) -> AgentKind`（已有私有函数，`mod.rs:32`）；`dozer_core::protocol::TurnRecord`（Task 1 新字段形状）。
- Produces: `get_conversation_turns` 签名不变（`&self, conversation_id: &str, after_turn_index: i64, limit: u32) -> Result<Vec<TurnRecord>>`）——调用方（`dozer-client`/`dozer-app`）零改动。

- [ ] **Step 1: 改 SQL 与行映射**

`crates/dozerd/src/transcripts/mod.rs:261-286`，把：

```rust
    pub fn get_conversation_turns(
        &self,
        conversation_id: &str,
        after_turn_index: i64,
        limit: u32,
    ) -> Result<Vec<dozer_core::protocol::TurnRecord>> {
        let conn = self.conn.lock().expect("db lock");
        let mut stmt = conn.prepare(
            "SELECT turn_index, role, content, tools_summary, thinking, ts, is_error
             FROM conversation_turns
             WHERE conversation_id = ?1 AND turn_index > ?2
             ORDER BY turn_index ASC LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![conversation_id, after_turn_index, limit], |row| {
            let tools_json: String = row.get(3)?;
            Ok(dozer_core::protocol::TurnRecord {
                turn_index: row.get(0)?,
                role: row.get(1)?,
                content: row.get(2)?,
                tools_summary: serde_json::from_str(&tools_json).unwrap_or_default(),
                thinking: row.get::<_, i64>(4)? != 0,
                ts: row.get(5)?,
                is_error: row.get::<_, i64>(6)? != 0,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }
```

改成：

```rust
    /// keyset 分页:返回 `turn_index > after_turn_index` 的前 `limit` 条。
    /// `after_turn_index` 传 `-1` 表示从第一条开始。JOIN `conversations`
    /// 拿 `agent_kind` 只为了给 `parse::extract_turn_trace_detail` 挑对
    /// 解析形状——不新增参数(client/protocol 签名都不用改),`raw_json`
    /// 每行都读一次、当场解析,不落新列(见 spec"读时解析"一节)。
    pub fn get_conversation_turns(
        &self,
        conversation_id: &str,
        after_turn_index: i64,
        limit: u32,
    ) -> Result<Vec<dozer_core::protocol::TurnRecord>> {
        let conn = self.conn.lock().expect("db lock");
        let mut stmt = conn.prepare(
            "SELECT t.turn_index, t.role, t.content, t.thinking, t.ts, t.is_error,
                    t.raw_json, c.agent_kind
             FROM conversation_turns t
             JOIN conversations c ON c.conversation_id = t.conversation_id
             WHERE t.conversation_id = ?1 AND t.turn_index > ?2
             ORDER BY t.turn_index ASC LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![conversation_id, after_turn_index, limit], |row| {
            let raw_json: String = row.get(6)?;
            let agent_kind: String = row.get(7)?;
            let detail = parse::extract_turn_trace_detail(&raw_json, agent_from_str(&agent_kind));
            Ok(dozer_core::protocol::TurnRecord {
                turn_index: row.get(0)?,
                role: row.get(1)?,
                content: row.get(2)?,
                tool_calls: detail.tool_calls,
                thinking: row.get::<_, i64>(3)? != 0,
                thinking_text: detail.thinking_text,
                ts: row.get(4)?,
                is_error: row.get::<_, i64>(5)? != 0,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }
```

- [ ] **Step 2: 改既有分页测试的字段访问**

`crates/dozerd/src/transcripts/mod.rs:869-892`（`get_conversation_turns_paginates_by_keyset`）不需要改字段访问（测试只读 `turn_index`），但要确认 fixture 走的是 `ingest_session`（已经会写 `conversations` 表，JOIN 不会返回空结果）——这一步不改代码，只需确认：

Run: `cargo test -p dozerd get_conversation_turns_paginates_by_keyset -- --nocapture`
Expected: PASS（不需要改动就该过，因为 `ingest_session` 已经建好 `conversations` 行）。

- [ ] **Step 3: 写新测试验证 trace 明细透传**

在 `crates/dozerd/src/transcripts/mod.rs` 测试模块里，紧跟 `get_conversation_turns_paginates_by_keyset` 之后新增：

```rust
#[test]
fn get_conversation_turns_surfaces_thinking_text_and_tool_calls() {
    let tmp = tempfile::tempdir().unwrap();
    let store = TranscriptStore::open(&tmp.path().join("t.db")).unwrap();
    let text = concat!(
        "{\"type\":\"user\",\"uuid\":\"u1\",\"timestamp\":100,\"message\":{\"role\":\"user\",",
        "\"content\":\"改一下 README\"}}\n",
        "{\"type\":\"assistant\",\"uuid\":\"a1\",\"timestamp\":110,\"message\":{\"content\":[",
        "{\"type\":\"thinking\",\"thinking\":\"先看看现有内容\"},",
        "{\"type\":\"tool_use\",\"name\":\"Edit\",\"input\":{\"file_path\":\"README.md\"}},",
        "{\"type\":\"text\",\"text\":\"改好了\"}]}}\n"
    );
    let file = fixture(tmp.path(), "s2.jsonl", text);
    store.ingest_session(AgentKind::Claude, &file).unwrap();

    let turns = store.get_conversation_turns("s2", -1, 10).unwrap();
    let ai_turn = turns.iter().find(|t| t.role == "ai").unwrap();
    assert_eq!(ai_turn.thinking_text.as_deref(), Some("先看看现有内容"));
    assert_eq!(ai_turn.tool_calls.len(), 1);
    assert_eq!(ai_turn.tool_calls[0].summary, "Edit README.md");
    assert!(ai_turn.tool_calls[0]
        .input_json
        .as_deref()
        .unwrap()
        .contains("README.md"));
}
```

（复用同文件里已有的 `fixture(dir, name, content)` 测试辅助函数——`get_conversation_turns_paginates_by_keyset` 测试紧上方就在用它，签名照抄即可。）

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozerd get_conversation_turns`
Expected: 两个测试（`get_conversation_turns_paginates_by_keyset`、`get_conversation_turns_surfaces_thinking_text_and_tool_calls`）均 PASS。

- [ ] **Step 5: 跑全量 dozerd 测试（确认没有其它调用点因为字段改名而炸）**

Run: `cargo test -p dozerd`
Expected: 全部 PASS。如果有其它测试因为 `TurnRecord { tools_summary: ... }` 字面量而编译失败，按 Task 1 同样的方式把字段名换成 `tool_calls`/补 `thinking_text` 后再跑。

- [ ] **Step 6: fmt + clippy**

Run: `cargo fmt -p dozerd && cargo clippy -p dozerd --all-targets`
Expected: 无警告。

- [ ] **Step 7: Commit**

```bash
git add crates/dozerd/src/transcripts/mod.rs
git commit -m "feat(dozerd): get_conversation_turns JOIN conversations 读时解析 trace 明细"
```

---

## Task 4: `dozer-app::transcript` — `ReviewEntry` 三段式 + 位置邻接折叠

**Files:**
- Modify: `crates/dozer-app/src/transcript.rs:1-45`（`ReviewEntry` 定义、`review_entries_from_turns`）
- Modify: `crates/dozer-app/src/transcript.rs`（测试模块里 `review_entries_from_turns_maps_role_and_tools`、`review_entries_from_turns_maps_tool_result_role` 两个既有测试）

**Interfaces:**
- Consumes: `dozer_core::protocol::{TurnRecord, ToolCallInfo}`（Task 1/3 产出）。
- Produces: `ReviewEntry::AiTurn { text, thinking_text, tool_calls: Vec<ToolCallInfo>, tool_results: Vec<ToolResultEntry> }`；`pub struct ToolResultEntry { pub content: String, pub is_error: bool }`（`Debug, Clone, PartialEq, Serialize`）。Task 5/6/7 消费这个新形状。

- [ ] **Step 1: 写失败的单测（覆盖折叠逻辑三种场景）**

先看当前 `crates/dozer-app/src/transcript.rs:189-232`（`review_entries_from_turns_maps_role_and_tools`）和 `:234-260+`（`review_entries_from_turns_maps_tool_result_role`），把这两个测试**替换**成：

```rust
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
```

- [ ] **Step 2: 跑测试确认失败（结构体形状还没改）**

Run: `cargo test -p dozer-app review_entries_from_turns`
Expected: FAIL（编译错误：`ReviewEntry::AiTurn` 没有 `thinking_text`/`tool_results` 字段，`ToolResultEntry` 未定义）。

- [ ] **Step 3: 改 `ReviewEntry` 定义 + 折叠逻辑**

`crates/dozer-app/src/transcript.rs:1-45`，把整块：

```rust
use dozer_core::protocol::TurnRecord;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum ReviewEntry {
    /// 人类发言（导航锚点）。
    Human { text: String },
    /// AI 一个回合：正文 + 工具一行摘要 + 是否含 thinking。
    AiTurn {
        text: String,
        tools: Vec<String>,
        thinking: bool,
    },
    /// 工具调用的返回结果(2026-08-21 补摄取——此前这类数据在
    /// dozerd 解析层被整体丢弃，见 parse.rs 的 tool_result 处理)。
    ToolResult { content: String, is_error: bool },
}

/// dozerd 查询回来的回合明细 → 面板展示用的 `ReviewEntry`。
pub fn review_entries_from_turns(turns: &[TurnRecord]) -> Vec<ReviewEntry> {
    turns
        .iter()
        .map(|t| match t.role.as_str() {
            "human" => ReviewEntry::Human {
                text: t.content.clone(),
            },
            "tool_result" => ReviewEntry::ToolResult {
                content: t.content.clone(),
                is_error: t.is_error,
            },
            _ => ReviewEntry::AiTurn {
                text: t.content.clone(),
                tools: t.tools_summary.clone(),
                thinking: t.thinking,
            },
        })
        .collect()
}
```

改成：

```rust
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
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app review_entries_from_turns`
Expected: 全部 PASS（`review_entries_from_turns_maps_ai_turn_with_thinking_and_tool_calls`、`review_entries_from_turns_nests_consecutive_tool_results_under_preceding_ai_turn`、`review_entries_from_turns_keeps_orphan_tool_result_at_top_level`）。

- [ ] **Step 5: 跑全量 dozer-app 测试确认编译通过**

Run: `cargo test -p dozer-app 2>&1 | tail -80`
Expected: 全部 PASS。这一步会暴露 `workspace.rs`/`app.rs` 里因为 `ReviewEntry`/`ReviewView` 形状改变而炸的编译错误——先记下报错位置，留给 Task 5/6 修（不要在这个 Task 里顺手改，保持任务边界清晰）。

- [ ] **Step 6: fmt + clippy（只跑本任务改的文件，忽略 Task 5/6 还没修的编译错误）**

Run: `cargo fmt -p dozer-app -- crates/dozer-app/src/transcript.rs`
Expected: 该文件格式化完成，无需等全 crate 能编译。

- [ ] **Step 7: Commit**

```bash
git add crates/dozer-app/src/transcript.rs
git commit -m "feat(dozer-app): ReviewEntry::AiTurn 拆思考文本/工具调用/工具结果三段"
```

（此时 `cargo build -p dozer-app` 预期还会因为 Task 5/6 未完成而失败——这是预期状态,不是本任务的回归,Task 6 结束后才应该全绿。）

---

## Task 5: `dozer-app::workspace` — `TopicPreview` + 邻居计算 + `ReviewView` 新字段

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs:173-183`（`ReviewView` 定义，紧邻新增 `TopicPreview`）
- Modify: `crates/dozer-app/src/workspace.rs:3928-3959`（三处 `ReviewView { .. }` 测试字面量）
- Modify: `crates/dozer-app/src/workspace.rs`（测试模块里新增 `adjacent_topic_previews` 单测）

**Interfaces:**
- Consumes: `TurnGroupRow { path: PathBuf, agent: AgentKind, start_turn_index: i64, end_turn_index: i64, title: String, ts: u64 }`（已有，`crates/dozer-app/src/conversation.rs:31-38`）。
- Produces: `pub struct TopicPreview { pub label: String }`（`Debug, Clone, PartialEq, Serialize`）；`pub(crate) fn adjacent_topic_previews(groups: &[TurnGroupRow], path: &Path, start_turn_index: i64, end_turn_index: i64) -> (Option<TopicPreview>, Option<TopicPreview>)`；`ReviewView` 新增 `pub prev_topic: Option<TopicPreview>` / `pub next_topic: Option<TopicPreview>`。Task 6 消费这两个产出。

- [ ] **Step 1: 写失败的单测**

在 `crates/dozer-app/src/workspace.rs` 测试模块（`mod tests`，紧邻已有 `review_webview_spec_*` 测试）新增：

```rust
#[test]
fn adjacent_topic_previews_finds_same_session_neighbors_sorted_by_turn_index() {
    let path = PathBuf::from("/tmp/s1.jsonl");
    let other_path = PathBuf::from("/tmp/s2.jsonl");
    let groups = vec![
        TurnGroupRow {
            path: path.clone(),
            agent: AgentKind::Claude,
            start_turn_index: 0,
            end_turn_index: 1,
            title: "第一话题".into(),
            ts: 100,
        },
        TurnGroupRow {
            path: path.clone(),
            agent: AgentKind::Claude,
            start_turn_index: 2,
            end_turn_index: 3,
            title: "第二话题".into(),
            ts: 200,
        },
        TurnGroupRow {
            path: path.clone(),
            agent: AgentKind::Claude,
            start_turn_index: 4,
            end_turn_index: 5,
            title: "第三话题".into(),
            ts: 300,
        },
        // 同名文件名、不同 session(不同 path)不能被当邻居。
        TurnGroupRow {
            path: other_path,
            agent: AgentKind::Claude,
            start_turn_index: 6,
            end_turn_index: 7,
            title: "别的会话".into(),
            ts: 400,
        },
    ];
    let (prev, next) = adjacent_topic_previews(&groups, &path, 2, 3);
    assert_eq!(
        prev,
        Some(TopicPreview {
            label: "第一话题".into()
        })
    );
    assert_eq!(
        next,
        Some(TopicPreview {
            label: "第三话题".into()
        })
    );
}

#[test]
fn adjacent_topic_previews_none_at_session_boundaries() {
    let path = PathBuf::from("/tmp/s1.jsonl");
    let groups = vec![TurnGroupRow {
        path: path.clone(),
        agent: AgentKind::Claude,
        start_turn_index: 0,
        end_turn_index: 1,
        title: "唯一话题".into(),
        ts: 100,
    }];
    let (prev, next) = adjacent_topic_previews(&groups, &path, 0, 1);
    assert_eq!(prev, None);
    assert_eq!(next, None);
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app adjacent_topic_previews`
Expected: FAIL（`adjacent_topic_previews`/`TopicPreview` 未定义）。

- [ ] **Step 3: 加 `TopicPreview` + `adjacent_topic_previews` + `ReviewView` 新字段**

`crates/dozer-app/src/workspace.rs:173-183`，把：

```rust
/// 会话审阅 tab 的内容（P1i）。
pub struct ReviewView {
    pub source: ReviewSource,
    pub entries: Vec<ReviewEntry>,
    pub error: Option<String>,
    /// 审阅 webview 的重新加载水位:每次 `ReviewLoaded` 成功都从
    /// `Workspace.review_nonce` 拷一份新值,写进 `dozer://review-trace/
    /// host.html?_r=<nonce>` 的查询参数,逼 wry 在内容变化时重新导航
    /// 拉取(同 `preview.rs::PreviewTab.reload_nonce` 的手法)。
    pub nonce: u64,
}
```

改成：

```rust
/// 前一话题/下一话题的静态预览(2026-08-22):只是一行标签文字,不可点击
/// 跳转,文字直接用已加载的 `TurnGroupRow.title`,不额外发请求抓正文
/// (见 spec"已知取舍")。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TopicPreview {
    pub label: String,
}

/// 会话审阅 tab 的内容（P1i）。
pub struct ReviewView {
    pub source: ReviewSource,
    pub entries: Vec<ReviewEntry>,
    pub error: Option<String>,
    /// 审阅 webview 的重新加载水位:每次 `ReviewLoaded` 成功都从
    /// `Workspace.review_nonce` 拷一份新值,写进 `dozer://review-trace/
    /// host.html?_r=<nonce>` 的查询参数,逼 wry 在内容变化时重新导航
    /// 拉取(同 `preview.rs::PreviewTab.reload_nonce` 的手法)。
    pub nonce: u64,
    /// 打开这个话题时算好、跟着 `ReviewView` 一起存的邻居预览——跟
    /// `entries`(异步加载)不同,这两个在 `app.rs::conversation_turn_group_open`
    /// 里同步算好,不随 `ReviewLoaded` 变化。
    pub prev_topic: Option<TopicPreview>,
    pub next_topic: Option<TopicPreview>,
}

/// 从已加载的全量 turn-group 列表里,找出跟 `path` 同一 session、按
/// `start_turn_index` 排序后紧邻当前话题([`start_turn_index`,
/// `end_turn_index`])的前一个/后一个。首/末话题,或该邻居因为
/// `conversation_turn_groups` 的 500 条上限没被加载进来,都返回 `None`——
/// 不额外发请求去补(见 spec"已知取舍")。
pub(crate) fn adjacent_topic_previews(
    groups: &[TurnGroupRow],
    path: &Path,
    start_turn_index: i64,
    end_turn_index: i64,
) -> (Option<TopicPreview>, Option<TopicPreview>) {
    let mut same_session: Vec<&TurnGroupRow> = groups.iter().filter(|g| g.path == path).collect();
    same_session.sort_by_key(|g| g.start_turn_index);
    let prev = same_session
        .iter()
        .rev()
        .find(|g| g.end_turn_index < start_turn_index)
        .map(|g| TopicPreview {
            label: g.title.clone(),
        });
    let next = same_session
        .iter()
        .find(|g| g.start_turn_index > end_turn_index)
        .map(|g| TopicPreview {
            label: g.title.clone(),
        });
    (prev, next)
}
```

- [ ] **Step 4: 改三处既有 `ReviewView { .. }` 测试字面量**

`crates/dozer-app/src/workspace.rs:3928-3959`，三个字面量各加两行 `prev_topic: None, next_topic: None,`：

```rust
let with_error = ReviewView {
    source: ReviewSource::FileRange(PathBuf::from("/tmp/a.jsonl"), 0, 1),
    entries: vec![ReviewEntry::Human { text: "hi".into() }],
    error: Some("boom".into()),
    nonce: 3,
    prev_topic: None,
    next_topic: None,
};
```

```rust
let empty_entries = ReviewView {
    source: ReviewSource::FileRange(PathBuf::from("/tmp/a.jsonl"), 0, 1),
    entries: Vec::new(),
    error: None,
    nonce: 3,
    prev_topic: None,
    next_topic: None,
};
```

```rust
let rv = ReviewView {
    source: ReviewSource::FileRange(PathBuf::from("/tmp/a.jsonl"), 0, 1),
    entries: vec![ReviewEntry::Human { text: "hi".into() }],
    error: None,
    nonce: 7,
    prev_topic: None,
    next_topic: None,
};
```

- [ ] **Step 5: 跑测试确认通过**

Run: `cargo test -p dozer-app adjacent_topic_previews`
Expected: 两个新测试 PASS。

Run: `cargo test -p dozer-app review_webview_spec`
Expected: 三个既有测试仍然 PASS。

- [ ] **Step 6: fmt（只对本文件；全 crate 编译要等 Task 6 结束）**

Run: `cargo fmt -p dozer-app -- crates/dozer-app/src/workspace.rs`

- [ ] **Step 7: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "feat(dozer-app): 加 TopicPreview + 同 session 前后话题邻居计算"
```

---

## Task 6: `dozer-app::app` — 打开话题时算邻居，快照 payload 带上前后话题

**Files:**
- Modify: `crates/dozer-app/src/app.rs:6022-6042`（`conversation_turn_group_open`）
- Modify: `crates/dozer-app/src/app.rs:3790-3817`（`Message::ReviewLoaded` 处理，快照序列化）

**Interfaces:**
- Consumes: `workspace::adjacent_topic_previews`/`TopicPreview`（Task 5 产出）；`ws.conversation_turn_groups: Option<Vec<TurnGroupRow>>`（已有字段，`workspace.rs:396`）。
- Produces: `dozer://review-trace/data.json` 响应体从裸数组 `[...]` 变成 `{"entries": [...], "prev_topic": {...}|null, "next_topic": {...}|null}`——Task 7 的 `review_trace.html` 消费这个新形状。

- [ ] **Step 1: `conversation_turn_group_open` 算邻居并存进 `ReviewView`**

`crates/dozer-app/src/app.rs:6022-6042`，把：

```rust
    fn conversation_turn_group_open(
        &mut self,
        path: PathBuf,
        agent: AgentKind,
        start: i64,
        end: i64,
    ) {
        self.with_focused_project(move |ws, io| {
            let path_s = path.to_string_lossy().into_owned();
            let source = ReviewSource::FileRange(path.clone(), start, end);
            ws.review = Some(ReviewView {
                source: source.clone(),
                entries: Vec::new(),
                error: None,
                nonce: 0,
            });
            let after = start - 1;
            let limit = (end - start + 1).max(0) as u32;
            ws.spawn_review_load(io, source, path_s, agent, after, limit);
        });
    }
```

改成：

```rust
    fn conversation_turn_group_open(
        &mut self,
        path: PathBuf,
        agent: AgentKind,
        start: i64,
        end: i64,
    ) {
        self.with_focused_project(move |ws, io| {
            let path_s = path.to_string_lossy().into_owned();
            let source = ReviewSource::FileRange(path.clone(), start, end);
            // 前后话题邻居用已经加载好的 `conversation_turn_groups` 同步
            // 算,不发新请求——数据没加载过(还没打开过会话面板)时兜底
            // 成"没有邻居",不阻塞打开当前话题。
            let (prev_topic, next_topic) = ws
                .conversation_turn_groups
                .as_deref()
                .map(|groups| {
                    crate::workspace::adjacent_topic_previews(groups, &path, start, end)
                })
                .unwrap_or((None, None));
            ws.review = Some(ReviewView {
                source: source.clone(),
                entries: Vec::new(),
                error: None,
                nonce: 0,
                prev_topic,
                next_topic,
            });
            let after = start - 1;
            let limit = (end - start + 1).max(0) as u32;
            ws.spawn_review_load(io, source, path_s, agent, after, limit);
        });
    }
```

- [ ] **Step 2: `Message::ReviewLoaded` 快照 payload 包一层 `prev_topic`/`next_topic`**

`crates/dozer-app/src/app.rs:3790-3817`，把：

```rust
            Message::ReviewLoaded(project_id, source, result) => {
                self.with_project(project_id, |ws, _io| {
                    if result.is_ok() {
                        ws.review_nonce = ws.review_nonce.wrapping_add(1);
                    }
                    let nonce = ws.review_nonce;
                    if let Some(rv) = &mut ws.review
                        && rv.source == source
                    {
                        match result {
                            Ok(entries) => {
                                // 快照写入必须放在 `rv.source == source` 判断
                                // 通过之后——这是它跟旧实现(main.rs 里的裸
                                // `static`,过期/乱序结果也会无条件覆盖)的
                                // 关键区别,过期加载结果到这里已经被
                                // 上面的守卫挡在外面,不会再污染快照。
                                let json = serde_json::to_string(&entries).unwrap_or_default();
                                *ws.review_snapshot.lock().expect("review snapshot 锁") =
                                    Some(json);
                                rv.nonce = nonce;
                                rv.entries = entries;
                                rv.error = None;
                            }
                            Err(e) => rv.error = Some(e),
                        }
                    }
                });
            }
```

改成：

```rust
            Message::ReviewLoaded(project_id, source, result) => {
                self.with_project(project_id, |ws, _io| {
                    if result.is_ok() {
                        ws.review_nonce = ws.review_nonce.wrapping_add(1);
                    }
                    let nonce = ws.review_nonce;
                    if let Some(rv) = &mut ws.review
                        && rv.source == source
                    {
                        match result {
                            Ok(entries) => {
                                // 快照写入必须放在 `rv.source == source` 判断
                                // 通过之后——这是它跟旧实现(main.rs 里的裸
                                // `static`,过期/乱序结果也会无条件覆盖)的
                                // 关键区别,过期加载结果到这里已经被
                                // 上面的守卫挡在外面,不会再污染快照。
                                // 前后话题预览(`prev_topic`/`next_topic`)
                                // 是打开话题那一刻同步算好、存在 `rv` 上的,
                                // 这里原样带进同一份 data.json,不重算。
                                let snapshot = ReviewSnapshot {
                                    entries: &entries,
                                    prev_topic: rv.prev_topic.clone(),
                                    next_topic: rv.next_topic.clone(),
                                };
                                let json = serde_json::to_string(&snapshot).unwrap_or_default();
                                *ws.review_snapshot.lock().expect("review snapshot 锁") =
                                    Some(json);
                                rv.nonce = nonce;
                                rv.entries = entries;
                                rv.error = None;
                            }
                            Err(e) => rv.error = Some(e),
                        }
                    }
                });
            }
```

紧邻这个 `Message` match 分支所在的 `impl` 块之外(文件里任意合适的私有类型聚集处,比如紧邻 `conversation_turn_group_open` 函数上方)新增：

```rust
/// `dozer://review-trace/data.json` 的响应体形状——`review_trace.html` 按
/// 这个结构消费(`entries`/`prev_topic`/`next_topic` 三个顶层字段)。
#[derive(serde::Serialize)]
struct ReviewSnapshot<'a> {
    entries: &'a [ReviewEntry],
    prev_topic: Option<workspace::TopicPreview>,
    next_topic: Option<workspace::TopicPreview>,
}
```

（`workspace::TopicPreview`/`ReviewEntry` 需要在 `app.rs` 已有的 `use` 范围内可见——`app.rs` 已经 `use crate::workspace::{..., ReviewView, ...}` 一类的批量导入，检查这块 `use` 列表，把 `TopicPreview` 加进去；`ReviewEntry` 大概率已经在导入列表里(被 `ws.review.entries: Vec<ReviewEntry>` 用到)。）

- [ ] **Step 3: 全量编译确认 Task 4/5/6 三步合起来能过**

Run: `cargo build -p dozer-app 2>&1 | tail -100`
Expected: 编译成功。如果还有残留的字段访问报错(比如别处也读了 `ReviewEntry::AiTurn { tools, thinking, .. }` 的旧字段名)，照报错位置改成新字段名。

- [ ] **Step 4: 跑全量 dozer-app 测试**

Run: `cargo test -p dozer-app 2>&1 | tail -100`
Expected: 全部 PASS。

- [ ] **Step 5: fmt + clippy**

Run: `cargo fmt -p dozer-app && cargo clippy -p dozer-app --all-targets 2>&1 | tail -80`
Expected: 无警告。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/app.rs
git commit -m "feat(dozer-app): 打开话题时算前后邻居，data.json payload 带上话题预览"
```

---

## Task 7: `review_trace.html` — 三段式 trace 面板 + 前后话题预览条

**Files:**
- Modify: `crates/dozer-app/src/review_trace.html`（整份文件：CSS + JS 渲染逻辑）

**Interfaces:**
- Consumes: `fetch("dozer://review-trace/data.json")` 返回 `{ entries: ReviewEntry[], prev_topic: {label}|null, next_topic: {label}|null }`（Task 6 产出的新 payload 形状）；`ReviewEntry::AiTurn` 的 JSON 形状是 `{"AiTurn": {"text", "thinking_text", "tool_calls": [{"summary","input_json"}], "tool_results": [{"content","is_error"}]}}`（Task 4 产出）。
- Produces: 无(叶子文件，没有下游任务消费它)。

这个文件没有自动化测试基础设施(现状如此，同 Files/Project/browser 三个面板的 webview 部分)，这个 Task 的验证方式是人工起 app、打开一个有工具调用/thinking 的话题走查。

- [ ] **Step 1: 整份替换 `crates/dozer-app/src/review_trace.html`**

把整个文件内容替换成：

```html
<!doctype html>
<html lang="zh">
<head>
<meta charset="utf-8" />
<title>trace</title>
<style>
  :root {
    --bg: #0a0e16;
    --gold: #F2D94E;
    --cream: #FFE5B4;
    --cyan: #47DEF0;
    --green: #1AD585;
    --red: #ff5c5c;
    --dim: #6b7280;
    --line: #232b3a;
  }
  * { box-sizing: border-box; }
  html, body {
    margin: 0; height: 100%;
    background: var(--bg); color: var(--cream);
    font: 13px/1.5 -apple-system, "PingFang SC", sans-serif;
  }
  #timeline { height: 100%; overflow-y: auto; padding: 16px 20px; }
  .topic-preview { padding: 4px 0 14px; }
  .topic-preview .label { font-size: 11px; color: var(--dim); margin-bottom: 4px; }
  .topic-preview .text { font-size: 12px; color: var(--dim); }
  .topic-divider {
    height: 1px; background: var(--line); border: none; margin: 14px 0;
  }
  .entry { position: relative; padding-left: 22px; margin-bottom: 14px; }
  .entry::before {
    content: ""; position: absolute; left: 6px; top: 4px; bottom: -14px;
    width: 1px; background: var(--line);
  }
  .entry:last-child::before { display: none; }
  .entry::after {
    content: ""; position: absolute; left: 2px; top: 4px;
    width: 9px; height: 9px; border-radius: 50%;
  }
  .entry.Human::after { background: var(--gold); }
  .entry.AiTurn::after { background: var(--cyan); }
  .entry.ToolResult::after { background: var(--green); }
  .entry.ToolResult.error::after { background: var(--red); }
  .entry .role {
    font-size: 11px; text-transform: uppercase; letter-spacing: .04em;
    margin-bottom: 3px;
  }
  .entry.Human .role { color: var(--gold); }
  .entry.AiTurn .role { color: var(--cyan); }
  .entry.ToolResult .role { color: var(--green); }
  .entry.ToolResult.error .role { color: var(--red); }
  .entry .text { white-space: pre-wrap; }
  .entry.ToolResult .text {
    color: var(--dim); font-family: ui-monospace, monospace; font-size: 12px;
  }
  .entry.ToolResult.error .text { color: var(--red); }
  details.trace-toggle { margin-top: 6px; }
  details.trace-toggle > summary {
    cursor: pointer; color: var(--dim); font-size: 11px; list-style: none;
  }
  details.trace-toggle > summary::-webkit-details-marker { display: none; }
  details.trace-toggle > summary::before { content: "轨迹 ▸ "; }
  details.trace-toggle[open] > summary::before { content: "轨迹 ▾ "; }
  .trace-section { margin: 8px 0 0 10px; padding-left: 10px; border-left: 1px solid var(--line); }
  .trace-section .section-title {
    font-size: 11px; color: var(--dim); margin-bottom: 4px;
  }
  .trace-section .thinking-text {
    white-space: pre-wrap; font-size: 12px; color: var(--dim);
  }
  .tool-call-row { margin-bottom: 6px; }
  .tool-call-row summary {
    cursor: pointer; color: var(--cyan); font-size: 11px;
    background: #10151f; border: 1px solid var(--line); border-radius: 4px;
    padding: 2px 8px; display: inline-block;
  }
  .tool-call-row .input-json {
    white-space: pre-wrap; font-family: ui-monospace, monospace; font-size: 11px;
    color: var(--dim); margin-top: 4px; padding-left: 4px;
  }
  .tool-result-row { margin-bottom: 6px; }
  .tool-result-row summary {
    cursor: pointer; color: var(--dim); font-size: 11px;
  }
  .tool-result-row.error summary { color: var(--red); }
  .tool-result-row .body {
    white-space: pre-wrap; font-family: ui-monospace, monospace; font-size: 12px;
    color: var(--dim); margin-top: 2px;
  }
  .tool-result-row.error .body { color: var(--red); }
</style>
</head>
<body>
<div id="timeline"></div>
<script>
const ROLE_LABEL = { Human: "甲方", AiTurn: "AI", ToolResult: "工具结果" };

function renderTopicPreview(label, topic) {
  const wrap = document.createElement("div");
  wrap.className = "topic-preview";
  const l = document.createElement("div");
  l.className = "label";
  l.textContent = label + ":";
  const t = document.createElement("div");
  t.className = "text";
  t.textContent = topic.label;
  wrap.appendChild(l);
  wrap.appendChild(t);
  return wrap;
}

function renderToolCallRow(call) {
  const details = document.createElement("details");
  details.className = "tool-call-row";
  const summary = document.createElement("summary");
  summary.textContent = call.summary;
  details.appendChild(summary);
  if (call.input_json) {
    const body = document.createElement("div");
    body.className = "input-json";
    body.textContent = call.input_json;
    details.appendChild(body);
  }
  return details;
}

function renderToolResultRow(result) {
  const details = document.createElement("details");
  details.className = "tool-result-row" + (result.is_error ? " error" : "");
  details.open = result.content.length < 200;
  const summary = document.createElement("summary");
  summary.textContent = (result.is_error ? "⚠ 失败 · " : "→ ") + result.content.split("\n")[0].slice(0, 80);
  const body = document.createElement("div");
  body.className = "body";
  body.textContent = result.content;
  details.appendChild(summary);
  details.appendChild(body);
  return details;
}

function renderTraceToggle(v) {
  const hasThinking = !!v.thinking_text;
  const hasToolCalls = v.tool_calls && v.tool_calls.length > 0;
  const hasToolResults = v.tool_results && v.tool_results.length > 0;
  if (!hasThinking && !hasToolCalls && !hasToolResults) {
    return null;
  }
  const details = document.createElement("details");
  details.className = "trace-toggle";
  const summary = document.createElement("summary");
  details.appendChild(summary);

  if (hasThinking) {
    const section = document.createElement("div");
    section.className = "trace-section";
    const title = document.createElement("div");
    title.className = "section-title";
    title.textContent = "思考过程";
    const text = document.createElement("div");
    text.className = "thinking-text";
    text.textContent = v.thinking_text;
    section.appendChild(title);
    section.appendChild(text);
    details.appendChild(section);
  }
  if (hasToolCalls) {
    const section = document.createElement("div");
    section.className = "trace-section";
    const title = document.createElement("div");
    title.className = "section-title";
    title.textContent = "操作过程";
    section.appendChild(title);
    v.tool_calls.forEach(function (call) {
      section.appendChild(renderToolCallRow(call));
    });
    details.appendChild(section);
  }
  if (hasToolResults) {
    const section = document.createElement("div");
    section.className = "trace-section";
    const title = document.createElement("div");
    title.className = "section-title";
    title.textContent = "工具结果";
    section.appendChild(title);
    v.tool_results.forEach(function (result) {
      section.appendChild(renderToolResultRow(result));
    });
    details.appendChild(section);
  }
  return details;
}

function renderEntry(entry) {
  const kind = Object.keys(entry)[0];
  const v = entry[kind];
  const el = document.createElement("div");
  el.className = "entry " + kind + (kind === "ToolResult" && v.is_error ? " error" : "");

  const role = document.createElement("div");
  role.className = "role";
  role.textContent = ROLE_LABEL[kind] || kind;
  el.appendChild(role);

  if (kind === "ToolResult") {
    el.appendChild(renderToolResultRow(v));
    return el;
  }

  if (kind === "Human") {
    const text = document.createElement("div");
    text.className = "text";
    text.textContent = v.text;
    el.appendChild(text);
    return el;
  }

  // AiTurn
  if (v.text) {
    const text = document.createElement("div");
    text.className = "text";
    text.textContent = v.text;
    el.appendChild(text);
  }
  const trace = renderTraceToggle(v);
  if (trace) {
    el.appendChild(trace);
  }
  return el;
}

fetch("dozer://review-trace/data.json")
  .then(function (r) { return r.json(); })
  .then(function (data) {
    const el = document.getElementById("timeline");
    el.innerHTML = "";
    if (data.prev_topic) {
      el.appendChild(renderTopicPreview("前一话题", data.prev_topic));
      const hr = document.createElement("hr");
      hr.className = "topic-divider";
      el.appendChild(hr);
    }
    data.entries.forEach(function (entry) { el.appendChild(renderEntry(entry)); });
    if (data.next_topic) {
      const hr = document.createElement("hr");
      hr.className = "topic-divider";
      el.appendChild(hr);
      el.appendChild(renderTopicPreview("下一话题", data.next_topic));
    }
  })
  .catch(function (err) {
    document.getElementById("timeline").textContent = "加载失败: " + err;
  });
</script>
</body>
</html>
```

- [ ] **Step 2: 全工作区编译确认没有残留问题**

Run: `cargo build 2>&1 | tail -60`
Expected: 全 workspace 编译成功（`review_trace.html` 是 `include_str!` 内嵌资源，改动本身不参与 Rust 编译检查，这一步是确认前 6 个 Task 的改动整体没有遗留问题）。

- [ ] **Step 3: 全工作区测试**

Run: `cargo test 2>&1 | tail -100`
Expected: 全部 PASS。

- [ ] **Step 4: fmt + clippy 全工作区**

Run: `cargo fmt && cargo clippy --all-targets 2>&1 | tail -100`
Expected: 无警告。

- [ ] **Step 5: 人工 GUI 走查**

Run: `cargo run -p dozer-app`

打开一个有真实 Claude/CodeBuddy 对话记录的项目 → 左侧"会话"面板点开一个有工具调用的话题 → 确认：
- 有前一/后一话题的场景下，展开话题上下出现"前一话题:"/"下一话题:"预览文字，首/末话题场景下对应位置不出现空块。
- AI 回合的"轨迹"折叠展开后，能看到独立的"思考过程"/"操作过程"/"工具结果"三个子区（某个 turn 没有对应内容时，那个子区不出现）。
- "操作过程"里每条工具调用点开能看到 `input_json`。
- 没有思考也没有工具活动的 AI 回合，不出现"轨迹"折叠入口（跟改动前行为一致）。

这一步没有自动化断言，走查通过后记录在 commit message 里，走查不通过就回头改 Step 1 的 JS/CSS。

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/review_trace.html
git commit -m "feat(dozer-app): review-trace 页面三段式 trace 面板 + 前后话题预览条

人工 GUI 走查通过:轨迹展开显示思考过程/操作过程/工具结果三个独立
折叠子区,展开话题上下正确展示同 session 前一/下一话题预览。"
```

---

## Self-Review（写完后已完成的检查）

**Spec 覆盖**：背景/范围边界 → Global Constraints 与各任务描述；"读时解析"→ Task 2；"结构调整"→ Task 1/4；"前后话题"→ Task 5/6；"webview 渲染"→ Task 7；"错误处理"三条（`raw_json` 解析失败/邻居缺失/无工具结果）分别在 Task 2 Step 2 测试、Task 5 `adjacent_topic_previews` 的 `find` 返回 `None`、Task 4 折叠逻辑的 `_ =>` 分支里体现；"测试"一节列的四类测试分别对应 Task 2/4/5/7。

**占位符扫描**：无 TBD/"类似 Task N"/无代码的步骤描述；Task 7 Step 5 的人工走查是该文件现状决定的既定模式，不是偷懒占位。

**类型一致性**：`ToolCallInfo`（Task 1 定义）在 Task 2/3/4 里字段名/类型一致（`summary: String`, `input_json: Option<String>`）；`TopicPreview`（Task 5 定义）在 Task 6 的 `ReviewSnapshot` 里字段名/类型一致（`label: String`）；`ReviewEntry::AiTurn` 的四个字段名（`text`/`thinking_text`/`tool_calls`/`tool_results`）在 Task 4 定义、Task 6 注释、Task 7 JS 里保持一致。
