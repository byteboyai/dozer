# V8agent Transcript Dispatch Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Route `AgentKind::V8agent` into dozerd's existing Claude-shaped transcript parser (instead of the "always empty" branch), with its own mutating-tool-name list, and stop the agent-card activity gate in `dozer-app` from skipping V8agent — so once the sibling `v8agent` repo starts writing a Claude-shaped transcript file and reporting its path (separate plan, `../v8agent/docs/superpowers/plans/2026-08-24-v8agent-dozer-transcript-integration.md`), conversation content, mutating-tool stats, and the usage panel all light up for V8agent with no further dozer-side work.

**Architecture:** `parse_claude_shaped_chunk` currently hardcodes Claude's mutating-tool-name list (`Edit`/`Write`/`MultiEdit`/`NotebookEdit`) inline. It gains a `mutating_tools: &[&str]` parameter so the same parsing logic can be reused for V8agent's own snake_case tool names without misclassifying which tool calls count as file-mutating. `parse_chunk` and `extract_turn_trace_detail` — the two functions that dispatch by `AgentKind` — move `V8agent` out of the "always returns nothing" arm into the Claude-shaped arm. Separately, `dozer-app/workspace.rs`'s `agent_card_refresh_plan` stops treating V8agent like Codex (whose transcript genuinely never parses to anything) for the "is it worth reading activity from this transcript" gate.

**Tech Stack:** Rust.

**Spec:** `docs/superpowers/specs/2026-08-24-v8agent-integration-design.md` (改动三 — this plan implements that section; 改动一/二 live in the `v8agent` repo's own plan).

## Global Constraints

- `MUTATING_TOOLS` (Claude/Opencode/Kilo/Unknown's list) is unchanged; V8agent gets its own separate constant, not a merged superset.
- `Codex` stays in the "always empty" branch in both `parse_chunk` and `extract_turn_trace_detail` — this plan does not touch Codex's transcript-ingestion status.
- `usage.rs` is out of scope for this plan (verified during spec review: the main usage table needs no changes; the daily-chart's 3-column `DayAgentTotals` struct is a separate, deliberately out-of-scope UI change — see spec's 非目标 section).
- `hook_install_target`/`ensure_hook_installed` returning `None` for `AgentKind::V8agent` is correct behavior and must not change — only the stale comments claiming V8agent is "纯 GUI 占位,没有真实 hook 支持" get corrected.

---

### Task 1: `parse.rs` — parameterize mutating tools, dispatch V8agent to the Claude-shaped parser

**Files:**
- Modify: `crates/dozerd/src/transcripts/parse.rs`

**Interfaces:**
- Produces: `parse_claude_shaped_chunk(text: &str, conversation_id: &str, starting_turn_index: i64, mutating_tools: &[&str]) -> Vec<ParsedTurn>` (signature change — the function is only called from `parse_chunk`, no other callers in the codebase to update).

- [ ] **Step 1: Write the failing tests**

In `crates/dozerd/src/transcripts/parse.rs`, replace the existing test:

```rust
    #[test]
    fn unsupported_agents_yield_empty() {
        let text = "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"忽略\"}}\n";
        assert!(parse_chunk(AgentKind::Codex, text, "c", 0).is_empty());
        assert!(parse_chunk(AgentKind::V8agent, text, "c", 0).is_empty());
    }
```

with:

```rust
    #[test]
    fn codex_yields_empty() {
        let text = "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"忽略\"}}\n";
        assert!(parse_chunk(AgentKind::Codex, text, "c", 0).is_empty());
    }

    #[test]
    fn v8agent_uses_its_own_mutating_tool_list() {
        let text = concat!(
            "{\"type\":\"assistant\",\"message\":{\"content\":[",
            "{\"type\":\"tool_use\",\"name\":\"write_file\",\"input\":{\"file_path\":\"foo.rs\"}}",
            "]}}\n"
        );
        let turns = parse_chunk(AgentKind::V8agent, text, "conv1", 0);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].mutating_tool_calls, 1);
        assert_eq!(turns[0].files_touched, vec!["foo.rs".to_string()]);
    }

    #[test]
    fn v8agent_does_not_recognize_claudes_mutating_tool_names() {
        let text = concat!(
            "{\"type\":\"assistant\",\"message\":{\"content\":[",
            "{\"type\":\"tool_use\",\"name\":\"Edit\",\"input\":{\"file_path\":\"foo.rs\"}}",
            "]}}\n"
        );
        let turns = parse_chunk(AgentKind::V8agent, text, "conv1", 0);
        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].mutating_tool_calls, 0,
            "V8agent's own tool is write_file/edit_file, not Claude's Edit — must not cross-recognize"
        );
    }

    #[test]
    fn claude_does_not_recognize_v8agents_mutating_tool_names() {
        let text = concat!(
            "{\"type\":\"assistant\",\"message\":{\"content\":[",
            "{\"type\":\"tool_use\",\"name\":\"write_file\",\"input\":{\"file_path\":\"foo.rs\"}}",
            "]}}\n"
        );
        let turns = parse_chunk(AgentKind::Claude, text, "conv1", 0);
        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].mutating_tool_calls, 0,
            "parameterizing must not leak V8agent's tool names into Claude's list"
        );
    }
```

Also add a direct extraction test near the existing `extract_claude_trace_detail_reads_thinking_text_and_tool_input` test:

```rust
    #[test]
    fn extract_turn_trace_detail_works_for_v8agent_via_claude_shape() {
        let raw = r#"{"type":"assistant","message":{"content":[
            {"type":"tool_use","name":"edit_file","input":{"file_path":"foo.rs"}}
        ]}}"#;
        let detail = extract_turn_trace_detail(raw, AgentKind::V8agent);
        assert_eq!(detail.tool_calls.len(), 1);
        assert_eq!(detail.tool_calls[0].summary, "edit_file foo.rs");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p dozerd --lib transcripts::parse::tests::v8agent`
Expected: FAIL — `parse_chunk(AgentKind::V8agent, ...)` still returns an empty `Vec` (current dispatch sends it to the always-empty branch), and the new `codex_yields_empty`/renamed test won't be found under the old name (compile error from the removed `unsupported_agents_yield_empty` if anything still referenced it — nothing does).

- [ ] **Step 3: Parameterize `parse_claude_shaped_chunk` and fix dispatch**

At the top of `crates/dozerd/src/transcripts/parse.rs`, add a new constant alongside the existing one:

```rust
pub const MUTATING_TOOLS: [&str; 4] = ["Edit", "Write", "MultiEdit", "NotebookEdit"];
const V8AGENT_MUTATING_TOOLS: [&str; 3] = ["write_file", "edit_file", "git_commit"];
```

Change the function signature:

```rust
fn parse_claude_shaped_chunk(
    text: &str,
    conversation_id: &str,
    starting_turn_index: i64,
    mutating_tools: &[&str],
) -> Vec<ParsedTurn> {
```

Inside it, change the mutating-tool check (in the `Some("assistant")` branch, inside the `Some("tool_use")` match arm):

```rust
                        Some("tool_use") => {
                            let name = b.get("name").and_then(|n| n.as_str()).unwrap_or("工具");
                            let input = b.get("input").cloned().unwrap_or(Value::Null);
                            tools_summary.push(tool_summary(name, &input));
                            tool_calls += 1;
                            if mutating_tools.contains(&name) {
                                mutating_tool_calls += 1;
                                if let Some(path) = input.get("file_path").and_then(|p| p.as_str())
                                {
                                    files_touched.push(path.to_string());
                                }
                            }
                        }
```

Update `parse_chunk`'s dispatch:

```rust
pub fn parse_chunk(
    agent: AgentKind,
    text: &str,
    conversation_id: &str,
    starting_turn_index: i64,
) -> Vec<ParsedTurn> {
    match agent {
        AgentKind::Claude | AgentKind::Opencode | AgentKind::Kilo | AgentKind::Unknown => {
            parse_claude_shaped_chunk(text, conversation_id, starting_turn_index, &MUTATING_TOOLS)
        }
        AgentKind::V8agent => parse_claude_shaped_chunk(
            text,
            conversation_id,
            starting_turn_index,
            &V8AGENT_MUTATING_TOOLS,
        ),
        AgentKind::Codebuddy => {
            parse_codebuddy_shaped_chunk(text, conversation_id, starting_turn_index)
        }
        AgentKind::Codex => Vec::new(),
    }
}
```

Update `extract_turn_trace_detail`'s dispatch (mutating-tool classification doesn't apply here — `extract_claude_trace_detail` reads thinking text and tool calls generically, with no tool-name filtering, so V8agent just joins the existing Claude-shaped arm):

```rust
pub fn extract_turn_trace_detail(raw_json: &str, agent: AgentKind) -> TurnTraceDetail {
    let Ok(v) = serde_json::from_str::<Value>(raw_json) else {
        return TurnTraceDetail::default();
    };
    match agent {
        AgentKind::Codebuddy => extract_codebuddy_trace_detail(&v),
        // Claude/Opencode/Kilo/Unknown/V8agent 摄取时都走
        // parse_claude_shaped_chunk(parse_chunk 的分派,parse.rs 上方),
        // 读时解析沿用同一分派。
        AgentKind::Claude
        | AgentKind::Opencode
        | AgentKind::Kilo
        | AgentKind::Unknown
        | AgentKind::V8agent => extract_claude_trace_detail(&v),
        // Codex 目前完全不摄取(parse_chunk 分派到空 Vec),没有 raw_json
        // 可读。
        AgentKind::Codex => TurnTraceDetail::default(),
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p dozerd --lib transcripts::parse::`
Expected: PASS — every test in the module green, including the 5 new/renamed ones.

- [ ] **Step 5: Commit**

```bash
git add crates/dozerd/src/transcripts/parse.rs
git commit -m "feat(dozerd): parse V8agent transcripts via the Claude-shaped parser"
```

---

### Task 2: `workspace.rs` — stop excluding V8agent from the activity gate, fix stale hook comments

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`

**Interfaces:**
- No signature changes — `agent_card_refresh_plan`'s return type `(bool, bool, bool)` is unchanged, only the value it returns for `AgentKind::V8agent` changes.

- [ ] **Step 1: Write the failing test**

In `crates/dozer-app/src/workspace.rs`, inside the `agent_card_refresh_plan_decides_by_agent_and_cwd` test, add a new assertion (placed after the existing `AgentKind::Codex` case):

```rust
        assert_eq!(
            agent_card_refresh_plan(dozer_core::protocol::AgentKind::V8agent, &root, Some(&root)),
            (false, true, false),
            "V8agent 现在走 Claude 形状解析(parse.rs 的 dispatch 改动),\
             transcript 能真正解出内容,activity 门禁应该打开;\
             model/mode 门禁不动(V8agent 的 transcript 里没有 model 字段)"
        );
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p dozer-app --lib workspace::tests::agent_card_refresh_plan_decides_by_agent_and_cwd`
Expected: FAIL — current code returns `(false, false, false)` for `AgentKind::V8agent` (still in the `needs_activity` exclusion list), not `(false, true, false)`.

- [ ] **Step 3: Flip the exclusion and update the comment above it**

In `crates/dozer-app/src/workspace.rs`, inside `agent_card_refresh_plan`, change:

```rust
    // "当前工作内容"兜底摘要的门禁比 model/mode 宽——只要 transcript
    // schema 能被 `parse_transcript` 解出人类/AI 文本就值得读(Opencode/
    // Kilo 的合成 transcript 是 Claude 形状,真有内容,只是没写 model/mode
    // 字段而已);Codex/V8agent 目前 `parse_transcript` 恒回空,读了也提取
    // 不出东西,不值得为它们打开这道门。
    let needs_activity = !matches!(agent, AgentKind::Codex | AgentKind::V8agent);
```

to:

```rust
    // "当前工作内容"兜底摘要的门禁比 model/mode 宽——只要 transcript
    // schema 能被 `parse_transcript` 解出人类/AI 文本就值得读(Opencode/
    // Kilo 的合成 transcript 是 Claude 形状,真有内容,只是没写 model/mode
    // 字段而已;V8agent 的 transcript 现在也走同一条 Claude 形状解析路径,
    // 见 dozerd `transcripts/parse.rs` 的 `parse_chunk` 分派)。Codex 目前
    // `parse_transcript` 恒回空,读了也提取不出东西,不值得为它打开这道门。
    let needs_activity = !matches!(agent, AgentKind::Codex);
```

- [ ] **Step 4: Fix the two stale "V8agent 是占位" comments**

These are comment-only edits with no behavior change (`hook_install_target` still returns `None` for `AgentKind::V8agent` — that's correct, not a bug, per the spec).

Change the doc comment above `HookInstallTarget`:

```rust
/// 已接入 `dozer-hook` 安装器的 agent 集合。刻意穷尽 match 而不是拿
/// `agent.label()` 当 catch-all 参数：`install::settings_path_for` 对未识别
/// 的 agent 名一律落回 Claude 的 `settings.json`路径，如果不显式排除
/// Kilo/V8agent(纯 GUI 占位，没有真实 hook 支持)，误调用会把
/// "kilo"/"v8agent" 的 hook 命令写进 Claude 的 settings.json，顶掉真正的
/// claude hook 条目。
```

to:

```rust
/// 已接入 `dozer-hook` 安装器的 agent 集合。刻意穷尽 match 而不是拿
/// `agent.label()` 当 catch-all 参数：`install::settings_path_for` 对未识别
/// 的 agent 名一律落回 Claude 的 `settings.json`路径，如果不显式排除
/// Kilo/V8agent，误调用会把 "kilo"/"v8agent" 的 hook 命令写进 Claude 的
/// settings.json，顶掉真正的 claude hook 条目。两者排除的原因不同：Kilo
/// 是真实缺口(没有任何 hook 上报机制)；V8agent 走的是完全不同的路子——
/// `v8agent-cli` 在 `DOZER_SESSION_ID` 存在时直接通过 UDS socket 上报
/// `Request::HookEvent`(见 `dozer-core::protocol`),不依赖这套"往
/// agent 自己的配置文件里写 hook 命令"的安装机制，所以这里返回 `None`
/// 对 V8agent 而言是正确行为，不是待办事项(spec
/// `docs/superpowers/specs/2026-08-24-v8agent-integration-design.md`)。
```

Change the comment inside the `hook_install_target_covers_only_agents_wired_up_in_dozer_hook` test:

```rust
        // Kilo/V8agent 在 `dozer-hook::install::settings_path_for` 里没有专属
        // 分支，会落回 Claude 的 settings.json 路径——绝不能对它们调用安装
        // 逻辑，否则会把 "kilo"/"v8agent" 的 hook 命令误写进 Claude 的配置，
        // 顶掉真正的 claude hook 条目。Unknown 同理，从不该触发安装。
```

to:

```rust
        // Kilo/V8agent 在 `dozer-hook::install::settings_path_for` 里没有专属
        // 分支，会落回 Claude 的 settings.json 路径——绝不能对它们调用安装
        // 逻辑，否则会把 "kilo"/"v8agent" 的 hook 命令误写进 Claude 的配置，
        // 顶掉真正的 claude hook 条目。Unknown 同理，从不该触发安装。V8agent
        // 不属于这里(它走 socket 直连上报，见 hook_install_target 上方文档
        // 注释)，只是恰好也该返回 None——跟 Kilo 是两个不同的理由。
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p dozer-app --lib workspace::`
Expected: PASS — the whole `workspace.rs` test module green, including the updated `agent_card_refresh_plan_decides_by_agent_and_cwd`.

- [ ] **Step 6: Commit**

```bash
git add crates/dozer-app/src/workspace.rs
git commit -m "fix(dozer-app): stop excluding V8agent from the agent-card activity gate"
```
