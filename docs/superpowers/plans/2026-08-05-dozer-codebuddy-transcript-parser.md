# CodeBuddy Transcript 真实解析 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 `AgentKind::Codebuddy` 的 transcript 从"诚实返回空"升级为真实解析——会话审阅面板（`transcript.rs`）能显示 CodeBuddy 对话的人类发言/AI 回合，对话历史侧栏（`conversation.rs`）能显示 CodeBuddy 对话的真实标题（而不是文件名 UUID）。

**Architecture:** CodeBuddy transcript 是独立 schema（`type:"message"` + `role` + `content[].type: input_text/output_text`，混有 `type:"file-history-snapshot"` 噪音行），与 Claude 的 `type:"user"/"assistant"` schema 不兼容，不能复用 Claude 分支。按已有的"按 agent 分派"模式（`parse_transcript(agent, jsonl)` 已经是这个形状），新增一个 `parse_codebuddy_shaped_jsonl` 私有解析函数并接进 match 分支；`conversation_title` 目前是不分 agent 的单一函数，需要先泛化成按 agent 分派，再补 CodeBuddy 分支。

**Tech Stack:** Rust, serde_json（已是依赖），现有测试用 `#[cfg(test)]` 内联模块 + `tempfile`（已是依赖）。

## Global Constraints

- Schema 依据：`crates/dozer-hook/fixtures/codebuddy-transcript-sample.jsonl`（已入库的真实脱敏样本，来自 `docs/superpowers/specs/2026-07-31-codebuddy-spike-findings.md` 的 spike 验证）与该 spike 文档的字段对照表——不得引入未经这两处证实的字段假设（例如 CodeBuddy 是否有 `tool_use` 等价物、是否有 `thinking` 字段，样本里都没出现，本计划不臆测，`tools`/`thinking` 一律留空/false）。
- `type:"file-history-snapshot"` 及任何非 `type:"message"` 的行必须被跳过，不能当噪音以外的东西处理。
- 不改动 `AgentKind::Claude`/`AgentKind::Opencode`/`AgentKind::Unknown` 分支的既有行为——两个函数改动前后，用 Claude 输入跑一遍现有测试必须逐字节不变。
- 遵循仓库现有代码风格：纯函数、不碰 iced/IO（`transcript.rs`）；`conversation.rs` 里唯一允许的 IO 是已有的文件读取，不新增。

---

### Task 1: `transcript.rs` —— CodeBuddy 分支真实解析

**Files:**
- Modify: `crates/dozer-app/src/transcript.rs:114-130`（`parse_transcript` 的文档注释与 match 分支）、文件末尾新增私有函数
- Modify: `crates/dozer-app/src/transcript.rs:197-201`（删除/替换 `codebuddy_yields_empty_until_schema_confirmed` 测试）
- Test: `crates/dozer-app/src/transcript.rs`（同文件内 `#[cfg(test)] mod tests`）

**Interfaces:**
- Consumes: `dozer_core::protocol::AgentKind`（已 import）、`serde_json::Value`（已 import）、`ReviewEntry::{Human, AiTurn}`（本文件已定义，字段 `Human { text: String }`、`AiTurn { text: String, tools: Vec<String>, thinking: bool }`）。
- Produces: `parse_transcript(AgentKind::Codebuddy, jsonl: &str) -> Vec<ReviewEntry>` 从恒返回 `Vec::new()` 变为真实解析结果，供 P1i 会话审阅面板消费（`workspace.rs` 里 `parse_transcript` 的调用点不变，签名不变，无需改调用方）。

- [ ] **Step 1: 写失败测试——用真实 fixture 验证解析结果**

在 `crates/dozer-app/src/transcript.rs` 的 `mod tests` 里，把现有的 `codebuddy_yields_empty_until_schema_confirmed` 测试整个替换成：

```rust
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
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozer-app transcript:: -- --nocapture`
Expected: `codebuddy_parses_real_fixture_sample`、`codebuddy_joins_multiple_text_blocks_and_skips_other_kinds` 两个新测试 FAIL（当前 `parse_transcript(AgentKind::Codebuddy, ...)` 恒返回空 `Vec`，`entries.len()` 断言失败）；`codebuddy_empty_and_all_noise_yield_nothing` 碰巧仍能 PASS（空输入本来就是空输出），这是预期的,不是问题。

- [ ] **Step 3: 实现 `parse_codebuddy_shaped_jsonl`**

在 `crates/dozer-app/src/transcript.rs` 里，紧跟在 `parse_claude_shaped_jsonl` 函数（第 49-112 行）后面新增：

```rust
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
            Some("user") => out.push(ReviewEntry::Human {
                text: join_codebuddy_text_blocks(blocks, "input_text"),
            }),
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
```

然后把 `parse_transcript` 的文档注释与 match 分支（第 114-130 行）替换成：

```rust
/// 按 agent 分派 transcript 解析。`Opencode` 复用 Claude 分支——
/// dozer-hook 代写 OpenCode 的 transcript 时就是按 Claude 字段形状写的
/// （spec §5.3），不是巧合。`Unknown` 也复用 Claude 分支：老装的 hook（还没
/// 重跑 `dozer-hook install`）上报的每个事件 agent 字段都是 `Unknown`，
/// 而在 CodeBuddy/OpenCode 真正进入用户机器之前，磁盘上现存的 transcript
/// 事实上全是 Claude 形状——保守地假定 Unknown 就是 Claude 形状，比直接
/// 返回空白审阅面板是严格更好的猜测。`Codebuddy` 走独立 schema 的解析器
/// （见 `parse_codebuddy_shaped_jsonl`)。
pub fn parse_transcript(agent: AgentKind, jsonl: &str) -> Vec<ReviewEntry> {
    match agent {
        AgentKind::Claude | AgentKind::Opencode | AgentKind::Unknown => {
            parse_claude_shaped_jsonl(jsonl)
        }
        AgentKind::Codebuddy => parse_codebuddy_shaped_jsonl(jsonl),
    }
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozer-app transcript:: -- --nocapture`
Expected: PASS，全部 `transcript` 模块测试（含新增的 3 个 + 原有的 4 个）绿。

- [ ] **Step 5: 提交**

```bash
git add crates/dozer-app/src/transcript.rs
git commit -m "feat(dozer-app): parse CodeBuddy transcript JSONL for review panel"
```

---

### Task 2: `conversation.rs` —— 对话标题按 agent 分派

**Files:**
- Modify: `crates/dozer-app/src/conversation.rs:54-68`（`conversation_title` 函数签名与实现）
- Modify: `crates/dozer-app/src/conversation.rs:100`（`list_conversations` 里的调用点）
- Modify: `crates/dozer-app/src/conversation.rs:150-158`（`title_from_first_string_user_turn` 测试）
- Test: `crates/dozer-app/src/conversation.rs`（同文件内 `#[cfg(test)] mod tests`）

**Interfaces:**
- Consumes: Task 1 未产出新符号供本任务使用（两个任务相互独立，都消费 `dozer_core::protocol::AgentKind`）。
- Produces: `conversation_title(agent: AgentKind, jsonl_head: &str) -> Option<String>`（签名从 `(jsonl_head: &str)` 改为多一个 `agent` 参数在前——跟 `parse_transcript(agent, jsonl)` 的参数顺序约定一致）。`list_conversations` 内部调用点同步更新，函数导出签名 `list_conversations(agent: AgentKind, dir: &Path) -> Vec<ConversationMeta>` 不变。

- [ ] **Step 1: 写失败测试**

把 `crates/dozer-app/src/conversation.rs` 里现有的 `title_from_first_string_user_turn` 测试（第 150-158 行）替换成：

```rust
    #[test]
    fn title_from_first_string_user_turn() {
        let head = concat!(
            "{\"type\":\"mode\",\"mode\":\"x\"}\n",
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"tool_result\"}]}}\n",
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"改一下 README\"}}\n"
        );
        assert_eq!(
            conversation_title(AgentKind::Claude, head).as_deref(),
            Some("改一下 README")
        );
        assert_eq!(
            conversation_title(AgentKind::Claude, "{\"type\":\"mode\"}\nbad\n"),
            None
        );
    }

    #[test]
    fn codebuddy_title_from_real_fixture_sample() {
        let jsonl = include_str!("../../dozer-hook/fixtures/codebuddy-transcript-sample.jsonl");
        assert_eq!(
            conversation_title(AgentKind::Codebuddy, jsonl).as_deref(),
            Some("reply with exactly one word: hello")
        );
    }

    #[test]
    fn codebuddy_title_skips_snapshot_and_non_user_messages() {
        let head = concat!(
            "{\"type\":\"file-history-snapshot\"}\n",
            "{\"type\":\"message\",\"role\":\"assistant\",\"content\":",
            "[{\"type\":\"output_text\",\"text\":\"不该被当标题\"}]}\n",
            "{\"type\":\"message\",\"role\":\"user\",\"content\":",
            "[{\"type\":\"input_text\",\"text\":\"真正的第一句\"}]}\n"
        );
        assert_eq!(
            conversation_title(AgentKind::Codebuddy, head).as_deref(),
            Some("真正的第一句")
        );
        assert_eq!(conversation_title(AgentKind::Codebuddy, "bad\n"), None);
    }
```

同时把第 100 行的调用点从 `conversation_title(&head)` 改成 `conversation_title(agent, &head)`（`list_conversations` 函数体内，`agent` 已是该函数的入参，直接可用）。

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p dozer-app conversation:: -- --nocapture`
Expected: 编译失败——`conversation_title` 函数定义仍是单参数，新测试调用双参数版本，签名不匹配。

- [ ] **Step 3: 实现**

把 `crates/dozer-app/src/conversation.rs` 第 53-68 行的 `conversation_title` 整个替换成：

```rust
/// transcript 首段 → 首句人类发言。按 agent 分派——Claude/Opencode/Unknown
/// 共用一套 schema（`type:"user"` + `message.content` 是字符串)，CodeBuddy
/// 是独立 schema（`type:"message"` + `role:"user"` + `content[].type:
/// "input_text"`），跟 `transcript.rs::parse_transcript` 的分派方式对齐。
/// 纯函数。
pub fn conversation_title(agent: AgentKind, jsonl_head: &str) -> Option<String> {
    match agent {
        AgentKind::Claude | AgentKind::Opencode | AgentKind::Unknown => {
            claude_shaped_title(jsonl_head)
        }
        AgentKind::Codebuddy => codebuddy_shaped_title(jsonl_head),
    }
}

fn claude_shaped_title(jsonl_head: &str) -> Option<String> {
    for line in jsonl_head.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line.trim()) else {
            continue;
        };
        if v.get("type").and_then(|t| t.as_str()) == Some("user")
            && let Some(text) = v
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(|c| c.as_str())
        {
            return Some(text.to_string());
        }
    }
    None
}

fn codebuddy_shaped_title(jsonl_head: &str) -> Option<String> {
    for line in jsonl_head.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line.trim()) else {
            continue;
        };
        if v.get("type").and_then(|t| t.as_str()) != Some("message")
            || v.get("role").and_then(|r| r.as_str()) != Some("user")
        {
            continue;
        }
        let Some(blocks) = v.get("content").and_then(|c| c.as_array()) else {
            continue;
        };
        for b in blocks {
            if b.get("type").and_then(|t| t.as_str()) == Some("input_text")
                && let Some(t) = b.get("text").and_then(|t| t.as_str())
            {
                return Some(t.to_string());
            }
        }
    }
    None
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p dozer-app conversation:: -- --nocapture`
Expected: PASS，全部 `conversation` 模块测试绿（含新增 3 个 + 既有的其余测试，包括未直接触碰但同文件的 `list_sorts_by_mtime_desc_and_titles` 等）。

- [ ] **Step 5: 提交**

```bash
git add crates/dozer-app/src/conversation.rs
git commit -m "feat(dozer-app): dispatch conversation title extraction by agent, add CodeBuddy shape"
```

---

### Task 3: 全 workspace 验证

**Files:**
- 无新文件；本任务只运行验证命令。

**Interfaces:**
- Consumes: Task 1 + Task 2 的全部改动。
- Produces: 无新符号，验证收尾。

- [ ] **Step 1: 全量测试**

Run: `cargo test --workspace`
Expected: 所有 crate 测试全绿，无回归（尤其 `dozer-app` 里 `transcript::` 和 `conversation::` 两个模块）。

- [ ] **Step 2: lint + 格式检查**

Run: `cargo clippy --all-targets && cargo fmt --check`
Expected: clippy 无新增警告；`fmt --check` 无差异（若有差异，跑 `cargo fmt` 后重新 `git add` 涉及文件，不新开 commit，合并进 Task 1/2 的 commit 前完成——若已提交则单独开一个 `style:` commit）。

- [ ] **Step 3: 全量构建**

Run: `cargo build --workspace`
Expected: 编译通过，无警告。

---

## 完成检查

- [ ] `parse_transcript(AgentKind::Codebuddy, jsonl)` 对真实 fixture（`crates/dozer-hook/fixtures/codebuddy-transcript-sample.jsonl`）产出 1 条 `Human` + 1 条 `AiTurn`，`file-history-snapshot` 行被跳过。
- [ ] `conversation_title(AgentKind::Codebuddy, jsonl_head)` 对同一份 fixture 产出 `Some("reply with exactly one word: hello")`，不再退化成文件名 UUID。
- [ ] Claude/Opencode/Unknown 三个既有分支行为逐字节不变（现有测试全部原样通过，未做行为性改动）。
- [ ] `cargo test --workspace`、`cargo clippy --all-targets`、`cargo fmt --check` 全绿。
- [ ] 本计划范围明确排除：CodeBuddy 是否有 `tool_use`/`thinking` 等价字段——样本未出现，不臆测；出现新样本后再开后续小任务补。
