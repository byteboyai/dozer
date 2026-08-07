# Agent 用量统计面板 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 Dozer 右图标栏新增一个"用量统计"面板：展示当前项目的 agent token 用量、复杂度指标（轮次/工具调用/触达文件/会话数），外加一张近 7 天堆叠条形图和一张按 agent 的 token 占比饼图。

**Architecture:** 新增 `crates/dozer-app/src/usage.rs`（纯函数数据解析/聚合 + 该面板的 iced 视图渲染，镜像 `git_log.rs` 的"数据+视图同文件、workspace.rs 只做接线"分工），`dozer-core::protocol`/`dozerd` 完全不动（客户端纯计算，读磁盘上的 agent transcript JSONL）。`workspace.rs` 增加 `RightView::Usage` 第三个右图标栏视图、`Workspace` 上两个新字段（`usage`/`usage_loading`）、一对 `Message` 变体、一个 `spawn_usage_refresh` 方法。

**Tech Stack:** Rust, iced 0.14（`iced_widget::canvas` 画饼图扇形，堆叠条形图用普通 `column!`/`container` 色块），不新增 crate 依赖。

## Global Constraints

- **只显示 token 原始数，不换算金额**（spec 非目标）。
- **不改 `dozer-core::protocol` / `dozerd`**——所有数据来自客户端本地读 transcript 文件。
- **不做实时更新/文件监听**——只在面板打开（首次切到 `RightView::Usage`）或点手动刷新按钮时触发一次异步扫描。
- **CodeBuddy 的 `tool_calls`/`mutating_tool_calls`/`files_touched` 恒为 0/空**——现有 fixture 里没见过 CodeBuddy 的 `tool_use` 形状，不臆测。
- **agent 配色复用 `workspace.rs::agent_dot_color`**（Claude=CYAN、CodeBuddy=PURPLE、OpenCode=GREEN），不新造配色。
- **不新增依赖**（含日期处理——用纯 std 手写的 UTC 日换算，不引入 `chrono`/`time`，虽然二者已在 `Cargo.lock` 里以传递依赖存在，但从未被任何 crate 直接依赖过，直接引入属于新增依赖边）。
- **v1 不做**：点击行跳转到对话原文、5h/weekly limit 拝扯、跨项目全局视图、"当前会话金框高亮"这个视觉细节（spec 里明确标注"非硬性需求"，本计划不排它的任务）。
- 参考规格：`docs/superpowers/specs/2026-08-07-agent-usage-panel-design.md`；参考设计稿：Figma "Dozer Phase 1 UI" node-id=158-30。

---

## File Structure

- **Create** `crates/dozer-app/src/usage.rs`：
  - 数据层：`ConversationUsage`、`parse_usage`、`ProjectUsageTotals`、`aggregate`、`group_usage_by_agent`、`DayAgentTotals`、`daily_totals_by_agent`、`agent_token_share`（含一个内部 `civil_from_days` 纯换算函数）。
  - 视图层：`pub fn view(...)`（面板主入口，对应 `RightView::Usage`）+ 私有的汇总条/图表卡/分组列表子渲染函数 + 饼图 `canvas::Program` 实现。
- **Create** `crates/dozer-app/assets/icons/bar-chart-3.svg`：Lucide `bar-chart-3`，`stroke="currentColor"`（同 `refresh-cw.svg` 的写法，不能像 Figma 稿里那样写死颜色）。
- **Modify** `crates/dozer-app/src/icons.rs`：加 `IconKind::BarChart3` 变体 + `bytes()` 里的一条 `include_bytes!` 分支。
- **Modify** `crates/dozer-app/src/main.rs`：加 `mod usage;`（按现有字母序插入 `todo_meta`/`transcript` 之间）。
- **Modify** `crates/dozer-app/src/workspace.rs`：
  - `RightView` 枚举加 `Usage` 变体；`RailButton` 枚举加 `RightUsage` 变体。
  - `Workspace` 结构体加 `usage: Vec<(ConversationMeta, ConversationUsage)>`、`usage_loading: bool` 两个私有字段（`Workspace::new()` 和 `adopt_project` 里各初始化/重置一次，同 `conversations` 字段的既有写法）。
  - `Message` 枚举加 `UsageRefresh`、`UsageLoaded(ProjectId, Vec<(ConversationMeta, ConversationUsage)>)` 两个变体。
  - `Workspace::spawn_usage_refresh(&self, io: &ShellIo)` 方法，镜像 `spawn_conversations_refresh`。
  - `update()` 里 `RightIconSelect` 分支追加"切到 Usage 视图时触发一次刷新"；新增 `Message::UsageRefresh`/`Message::UsageLoaded` 两个 match 分支。
  - `right_icon_rail()` 加第三个图标按钮；`right_panel_area()` 加 `RightView::Usage` 分支（单栏，不是配对分栏），并把原来的二元 `(lc, rc)` 圆角元组扩成三元 `(lc, rc, ac)`（同 `left_panel_area` 现有写法）。
  - `agent_dot_color` 的可见性从私有改成 `pub(crate)`（`usage.rs` 要跨模块调它复用配色）。

## Task 1: `usage.rs` 骨架 + transcript 逐条解析（`ConversationUsage`/`parse_usage`）

**Files:**
- Create: `crates/dozer-app/src/usage.rs`
- Modify: `crates/dozer-app/src/main.rs`（加 `mod usage;`）
- Test: 同文件内 `#[cfg(test)] mod tests`（同 `transcript.rs`/`conversation.rs` 惯例，不单开测试文件）

**Interfaces:**
- Produces: `pub struct ConversationUsage { turns: u32, tool_calls: u32, mutating_tool_calls: u32, files_touched: BTreeSet<String>, tokens_in: u64, tokens_out: u64, tokens_cache_read: u64, tokens_cache_write: u64 }`（派生 `Debug, Clone, Default, PartialEq`），`pub fn parse_usage(agent: dozer_core::protocol::AgentKind, jsonl: &str) -> ConversationUsage`。

- [x] **Step 1: 建文件 + `mod` 声明，写 Claude 分支的失败测试**

`crates/dozer-app/src/main.rs` 里 `mod todo_meta;` 和 `mod transcript;` 之间插入：

```rust
mod usage;
```

新建 `crates/dozer-app/src/usage.rs`：

```rust
// crates/dozer-app/src/usage.rs
//! Agent 用量统计面板的数据层与视图层（spec
//! docs/superpowers/specs/2026-08-07-agent-usage-panel-design.md）。数据层是
//! 纯函数,不碰 iced/IO,镜像 `transcript.rs` 的按 agent 分派解析方式;
//! `dozerd`/`dozer-core::protocol` 完全不参与——所有数据直接读磁盘上的
//! agent transcript JSONL。

use dozer_core::protocol::AgentKind;
use serde_json::Value;
use std::collections::BTreeSet;

/// 单个会话（= 一份 transcript 文件）的用量统计。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConversationUsage {
    pub turns: u32,
    pub tool_calls: u32,
    pub mutating_tool_calls: u32,
    pub files_touched: BTreeSet<String>,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub tokens_cache_read: u64,
    pub tokens_cache_write: u64,
}

/// 改动类工具——命中这些名字才计入 `mutating_tool_calls`/`files_touched`。
const MUTATING_TOOLS: [&str; 4] = ["Edit", "Write", "MultiEdit", "NotebookEdit"];

/// 按 agent 分派解析,Claude/OpenCode 共用一套 schema、CodeBuddy 独立一套,
/// 与 `transcript.rs::parse_transcript` 同一分派方式。单行解析失败/字段
/// 缺失一律跳过该行/记 0,不 panic、不中断整份文件的解析。
pub fn parse_usage(agent: AgentKind, jsonl: &str) -> ConversationUsage {
    match agent {
        AgentKind::Claude | AgentKind::Opencode | AgentKind::Unknown => {
            parse_claude_shaped_usage(jsonl)
        }
        AgentKind::Codebuddy => parse_codebuddy_shaped_usage(jsonl),
    }
}

fn parse_claude_shaped_usage(jsonl: &str) -> ConversationUsage {
    let mut u = ConversationUsage::default();
    for line in jsonl.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        match v.get("type").and_then(|t| t.as_str()) {
            Some("user") => u.turns += 1,
            Some("assistant") => {
                u.turns += 1;
                if let Some(usage) = v.get("message").and_then(|m| m.get("usage")) {
                    u.tokens_in += usage
                        .get("input_tokens")
                        .and_then(|n| n.as_u64())
                        .unwrap_or(0);
                    u.tokens_out += usage
                        .get("output_tokens")
                        .and_then(|n| n.as_u64())
                        .unwrap_or(0);
                    u.tokens_cache_read += usage
                        .get("cache_read_input_tokens")
                        .and_then(|n| n.as_u64())
                        .unwrap_or(0);
                    u.tokens_cache_write += usage
                        .get("cache_creation_input_tokens")
                        .and_then(|n| n.as_u64())
                        .unwrap_or(0);
                }
                let Some(blocks) = v
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_array())
                else {
                    continue;
                };
                for b in blocks {
                    if b.get("type").and_then(|t| t.as_str()) != Some("tool_use") {
                        continue;
                    }
                    u.tool_calls += 1;
                    let name = b.get("name").and_then(|n| n.as_str()).unwrap_or("");
                    if MUTATING_TOOLS.contains(&name) {
                        u.mutating_tool_calls += 1;
                        if let Some(path) = b
                            .get("input")
                            .and_then(|i| i.get("file_path"))
                            .and_then(|p| p.as_str())
                        {
                            u.files_touched.insert(path.to_string());
                        }
                    }
                }
            }
            _ => {}
        }
    }
    u
}

fn parse_codebuddy_shaped_usage(jsonl: &str) -> ConversationUsage {
    let mut u = ConversationUsage::default();
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
        if v.get("role").and_then(|r| r.as_str()).is_some() {
            u.turns += 1;
        }
        if let Some(usage) = v.get("providerData").and_then(|p| p.get("usage")) {
            u.tokens_in += usage
                .get("inputTokens")
                .and_then(|n| n.as_u64())
                .unwrap_or(0);
            u.tokens_out += usage
                .get("outputTokens")
                .and_then(|n| n.as_u64())
                .unwrap_or(0);
        }
        // tool_calls/mutating_tool_calls/files_touched 恒为 0/空——CodeBuddy
        // 的 fixture 样本里没见过 tool_use 形状的消息,不臆测其结构
        // （spec"非目标"一节）。
    }
    u
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_claude_shaped_counts_turns_and_tokens() {
        let jsonl = concat!(
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"改一下\"}}\n",
            "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"好\"}],",
            "\"usage\":{\"input_tokens\":100,\"output_tokens\":20,",
            "\"cache_read_input_tokens\":5,\"cache_creation_input_tokens\":3}}}\n",
        );
        let u = parse_usage(AgentKind::Claude, jsonl);
        assert_eq!(u.turns, 2);
        assert_eq!(u.tokens_in, 100);
        assert_eq!(u.tokens_out, 20);
        assert_eq!(u.tokens_cache_read, 5);
        assert_eq!(u.tokens_cache_write, 3);
        assert_eq!(u.tool_calls, 0);
    }
}
```

- [x] **Step 2: 跑测试确认能通过（先确认新代码没写错，再补覆盖率）**

Run: `cargo test -p dozer-app usage::tests::parse_claude_shaped_counts_turns_and_tokens`
Expected: PASS（这一步是新模块的冒烟测试，不是"先失败后实现"的 TDD 起点——`parse_usage` 是新写的纯函数，没有历史实现可回归，直接验证正确性即可）。

- [x] **Step 3: 补 `tool_use`/改动类工具/`files_touched` 的测试**

在 `mod tests` 里追加：

```rust
    #[test]
    fn parse_claude_shaped_counts_tool_calls_and_mutating_files() {
        let jsonl = concat!(
            "{\"type\":\"assistant\",\"message\":{\"content\":[",
            "{\"type\":\"tool_use\",\"name\":\"Read\",\"input\":{\"file_path\":\"/a.rs\"}},",
            "{\"type\":\"tool_use\",\"name\":\"Edit\",\"input\":{\"file_path\":\"/a.rs\"}},",
            "{\"type\":\"tool_use\",\"name\":\"Write\",\"input\":{\"file_path\":\"/b.rs\"}}",
            "],\"usage\":{}}}\n",
        );
        let u = parse_usage(AgentKind::Claude, jsonl);
        assert_eq!(u.tool_calls, 3, "Read/Edit/Write 全部计入 tool_calls");
        assert_eq!(u.mutating_tool_calls, 2, "只有 Edit/Write 是改动类");
        assert_eq!(
            u.files_touched,
            BTreeSet::from(["/a.rs".to_string(), "/b.rs".to_string()])
        );
    }

    #[test]
    fn parse_usage_skips_malformed_lines_without_panicking() {
        let jsonl = "not json\n{\"type\":\"user\",\"message\":{\"content\":\"hi\"}}\n";
        let u = parse_usage(AgentKind::Claude, jsonl);
        assert_eq!(u.turns, 1, "坏行跳过,好行照常计入");
    }

    #[test]
    fn parse_usage_empty_file_is_all_zero() {
        assert_eq!(parse_usage(AgentKind::Claude, ""), ConversationUsage::default());
    }

    #[test]
    fn parse_usage_opencode_reuses_claude_shape() {
        let jsonl = "{\"type\":\"user\",\"message\":{\"content\":\"hi\"}}\n";
        assert_eq!(parse_usage(AgentKind::Opencode, jsonl).turns, 1);
    }
```

- [x] **Step 4: 跑全部新测试**

Run: `cargo test -p dozer-app usage::tests`
Expected: 4 个测试全 PASS。

- [x] **Step 5: CodeBuddy 分支——先写失败测试**

复用现有 fixture（`crates/dozer-hook/fixtures/codebuddy-transcript-sample.jsonl`，内容见该文件：一条 user 消息 + 一条 assistant 消息，assistant 消息的 `providerData.usage` 里有 `inputTokens:22563, outputTokens:3`）。追加测试：

```rust
    #[test]
    fn parse_codebuddy_shaped_reads_provider_usage_and_zero_tool_calls() {
        let jsonl =
            include_str!("../../dozer-hook/fixtures/codebuddy-transcript-sample.jsonl");
        let u = parse_usage(AgentKind::Codebuddy, jsonl);
        assert_eq!(u.tokens_in, 22563);
        assert_eq!(u.tokens_out, 3);
        assert_eq!(u.turns, 2, "一条 user + 一条 assistant");
        assert_eq!(u.tool_calls, 0, "CodeBuddy 工具调用形状未观测到,恒为 0");
        assert!(u.files_touched.is_empty());
    }
```

Run: `cargo test -p dozer-app usage::tests::parse_codebuddy_shaped_reads_provider_usage_and_zero_tool_calls -- --nocapture`
Expected: 应该直接 PASS（`parse_codebuddy_shaped_usage` 在 Step 1 已经写好），如果 FAIL 说明对 fixture 真实字段形状的假设有误——打印 `u` 核对 `providerData.usage.inputTokens` 路径是否与 fixture 一致再改。

- [x] **Step 6: 跑 `usage.rs` 全部测试 + `cargo check`**

Run: `cargo test -p dozer-app usage:: && cargo check -p dozer-app`
Expected: 全 PASS，`cargo check` 无 warning（新文件暂时没有任何调用方，`#![allow(dead_code)]` 还不需要——Task 5 接线后死代码警告会自然消失；如果这一步 `cargo check` 报 `parse_usage`/`ConversationUsage` 未使用，在文件顶部临时加 `#![allow(dead_code)]` 并在注释写明"Task 5 接线后移除"，同 `icons.rs` 现有先例）。

- [x] **Step 7: Commit**

```bash
git add crates/dozer-app/src/usage.rs crates/dozer-app/src/main.rs
git commit -m "feat(dozer-app): usage.rs 骨架 + 逐条 transcript 用量解析(parse_usage)"
```

## Task 2: 聚合层——`aggregate` / `group_usage_by_agent`

**Files:**
- Modify: `crates/dozer-app/src/usage.rs`

**Interfaces:**
- Consumes: `ConversationUsage`（Task 1）、`dozer_core::protocol::AgentKind`、`crate::conversation::ConversationMeta`。
- Produces: `pub struct ProjectUsageTotals { conversation_count: u32, turns: u32, tool_calls: u32, mutating_tool_calls: u32, files_touched: u32, tokens_in: u64, tokens_out: u64, tokens_cache_read: u64, tokens_cache_write: u64 }`（`Debug, Clone, Default, PartialEq`），`pub fn aggregate(rows: &[ConversationUsage]) -> ProjectUsageTotals`，`pub fn group_usage_by_agent(rows: &[(ConversationMeta, ConversationUsage)]) -> Vec<(AgentKind, Vec<usize>)>`（下标语义同 `workspace.rs::group_tabs_by_agent`：返回下标而不是引用，调用方按下标回查 `rows`）。

- [x] **Step 1: 写 `aggregate` 的测试（先写测试，此时 `aggregate` 还不存在，编译会失败——这是本任务真正的"红"）**

在 `usage.rs` 顶部 `use` 区加 `use crate::conversation::ConversationMeta;`，`mod tests` 里追加：

```rust
    fn sample_usage(files: &[&str]) -> ConversationUsage {
        ConversationUsage {
            turns: 2,
            tool_calls: 3,
            mutating_tool_calls: 1,
            files_touched: files.iter().map(|s| s.to_string()).collect(),
            tokens_in: 10,
            tokens_out: 2,
            tokens_cache_read: 1,
            tokens_cache_write: 1,
        }
    }

    #[test]
    fn aggregate_sums_fields_and_dedups_files_across_conversations() {
        let rows = [sample_usage(&["/a.rs", "/b.rs"]), sample_usage(&["/a.rs", "/c.rs"])];
        let totals = aggregate(&rows);
        assert_eq!(totals.conversation_count, 2);
        assert_eq!(totals.turns, 4);
        assert_eq!(totals.tool_calls, 6);
        assert_eq!(totals.mutating_tool_calls, 2);
        assert_eq!(totals.files_touched, 3, "/a.rs 在两个会话里都出现,只算一次");
        assert_eq!(totals.tokens_in, 20);
        assert_eq!(totals.tokens_out, 4);
        assert_eq!(totals.tokens_cache_read, 2);
        assert_eq!(totals.tokens_cache_write, 2);
    }

    #[test]
    fn aggregate_empty_slice_is_all_zero() {
        assert_eq!(aggregate(&[]), ProjectUsageTotals::default());
    }
```

- [x] **Step 2: 跑测试确认失败（`aggregate`/`ProjectUsageTotals` 未定义）**

Run: `cargo test -p dozer-app usage::tests::aggregate_sums_fields_and_dedups_files_across_conversations`
Expected: FAIL，编译错误 `cannot find function 'aggregate'`。

- [x] **Step 3: 实现 `ProjectUsageTotals` + `aggregate`**

```rust
/// 多个会话的 `ConversationUsage` 加总成项目级汇总。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProjectUsageTotals {
    pub conversation_count: u32,
    pub turns: u32,
    pub tool_calls: u32,
    pub mutating_tool_calls: u32,
    pub files_touched: u32,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub tokens_cache_read: u64,
    pub tokens_cache_write: u64,
}

pub fn aggregate(rows: &[ConversationUsage]) -> ProjectUsageTotals {
    let mut files = BTreeSet::new();
    let mut totals = ProjectUsageTotals {
        conversation_count: rows.len() as u32,
        ..Default::default()
    };
    for r in rows {
        totals.turns += r.turns;
        totals.tool_calls += r.tool_calls;
        totals.mutating_tool_calls += r.mutating_tool_calls;
        totals.tokens_in += r.tokens_in;
        totals.tokens_out += r.tokens_out;
        totals.tokens_cache_read += r.tokens_cache_read;
        totals.tokens_cache_write += r.tokens_cache_write;
        files.extend(r.files_touched.iter().cloned());
    }
    totals.files_touched = files.len() as u32;
    totals
}
```

- [x] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app usage::tests::aggregate_sums_fields_and_dedups_files_across_conversations usage::tests::aggregate_empty_slice_is_all_zero`
Expected: 2 个测试 PASS。

- [x] **Step 5: `group_usage_by_agent` 的测试 + 实现（同一步做完,函数本身是 `group_tabs_by_agent` 的直接改写,风险低,不必单独拆一轮红绿）**

测试：

```rust
    fn meta(agent: AgentKind, title: &str) -> ConversationMeta {
        ConversationMeta {
            path: std::path::PathBuf::from(format!("/{title}.jsonl")),
            title: title.to_string(),
            modified_ms: 0,
            size_bytes: 0,
            agent,
        }
    }

    #[test]
    fn group_usage_by_agent_orders_claude_codebuddy_opencode_and_skips_empty_groups() {
        let rows = vec![
            (meta(AgentKind::Codebuddy, "b"), ConversationUsage::default()),
            (meta(AgentKind::Claude, "a"), ConversationUsage::default()),
        ];
        let groups = group_usage_by_agent(&rows);
        assert_eq!(groups.len(), 2, "没有 OpenCode 数据,不留空分组");
        assert_eq!(groups[0].0, AgentKind::Claude, "固定顺序:Claude 先于 CodeBuddy");
        assert_eq!(groups[0].1, vec![1]);
        assert_eq!(groups[1].0, AgentKind::Codebuddy);
        assert_eq!(groups[1].1, vec![0]);
    }
```

实现（紧接在 `aggregate` 之后）：

```rust
/// 按 `AgentKind` 把会话分组，固定顺序 Claude → Codebuddy → Opencode →
/// Unknown，只返回非空分组；组内保持传入顺序。返回下标而非引用，语义同
/// `workspace.rs::group_tabs_by_agent`——渲染时既要下标回查
/// `rows[idx]` 取展示字段，直接存下标比存 `&(ConversationMeta, ConversationUsage)`
/// 省一次生命周期纠缠。
pub fn group_usage_by_agent(
    rows: &[(ConversationMeta, ConversationUsage)],
) -> Vec<(AgentKind, Vec<usize>)> {
    const ORDER: [AgentKind; 4] = [
        AgentKind::Claude,
        AgentKind::Codebuddy,
        AgentKind::Opencode,
        AgentKind::Unknown,
    ];
    ORDER
        .into_iter()
        .filter_map(|kind| {
            let idxs: Vec<usize> = rows
                .iter()
                .enumerate()
                .filter(|(_, (meta, _))| meta.agent == kind)
                .map(|(i, _)| i)
                .collect();
            (!idxs.is_empty()).then_some((kind, idxs))
        })
        .collect()
}
```

- [x] **Step 6: 跑全部测试**

Run: `cargo test -p dozer-app usage::`
Expected: 全 PASS（6 个测试）。

- [x] **Step 7: Commit**

```bash
git add crates/dozer-app/src/usage.rs
git commit -m "feat(dozer-app): usage.rs 聚合层(aggregate/group_usage_by_agent)"
```

## Task 3: 图表数据层——`daily_totals_by_agent` / `agent_token_share`

**Files:**
- Modify: `crates/dozer-app/src/usage.rs`

**Interfaces:**
- Consumes: `ConversationUsage`/`ConversationMeta`（Task 1/2）。
- Produces: `pub struct DayAgentTotals { pub day_index: i64, pub label: String, pub claude: u64, pub codebuddy: u64, pub opencode: u64 }`（`Debug, Clone, PartialEq`），`pub fn daily_totals_by_agent(rows: &[(ConversationMeta, ConversationUsage)]) -> Vec<DayAgentTotals>`，`pub fn agent_token_share(rows: &[(ConversationMeta, ConversationUsage)]) -> Vec<(AgentKind, u64)>`。

- [x] **Step 1: 写 UTC 日期换算的失败测试（`civil_from_days` 尚不存在）**

```rust
    #[test]
    fn civil_from_days_known_epoch_dates() {
        // 1970-01-01 是 epoch day 0。
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        // 2026-08-07 手工核对(用 `date -u -j -f "%Y-%m-%d" 2026-08-07 +%s`
        // 算出 epoch 秒再除以 86400 得到 day_index=20668)。
        assert_eq!(civil_from_days(20668), (2026, 8, 7));
    }
```

- [x] **Step 2: 跑测试确认失败**

Run: `cargo test -p dozer-app usage::tests::civil_from_days_known_epoch_dates`
Expected: FAIL，`cannot find function 'civil_from_days'`。

- [x] **Step 3: 实现 `civil_from_days` + `day_index_from_ms`**

```rust
/// epoch 毫秒 → 该毫秒所在的 UTC 日索引(自 1970-01-01 起的第几天)。用于
/// 按天分桶;**不做本地时区换算**——纯 std 没有时区能力,引入 `chrono`/`time`
/// 属于新增依赖(spec 明确不新增)，UTC 分桶对"看近 7 天趋势形状"这个用途
/// 足够，不追求跟用户本地墙上时钟严格对齐。
fn day_index_from_ms(ms: u64) -> i64 {
    (ms / 86_400_000) as i64
}

/// UTC 日索引 → (year, month, day)。Howard Hinnant 的公开 civil_from_days
/// 算法(纯数学换算，不依赖任何日期库)。
fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m, d)
}
```

- [x] **Step 4: 跑测试确认通过**

Run: `cargo test -p dozer-app usage::tests::civil_from_days_known_epoch_dates`
Expected: PASS。

- [x] **Step 5: `daily_totals_by_agent` 的失败测试**

```rust
    fn meta_at(agent: AgentKind, ms: u64) -> ConversationMeta {
        ConversationMeta {
            path: std::path::PathBuf::from(format!("/{ms}.jsonl")),
            title: String::new(),
            modified_ms: ms,
            size_bytes: 0,
            agent,
        }
    }

    fn usage_with_tokens(input: u64) -> ConversationUsage {
        ConversationUsage {
            tokens_in: input,
            ..Default::default()
        }
    }

    #[test]
    fn daily_totals_by_agent_buckets_by_day_and_sums_per_agent() {
        // day 20668 = 2026-08-07 00:00:00 UTC 起的毫秒;+3600_000 还在同一天。
        let day0_ms = 20_668u64 * 86_400_000;
        let rows = vec![
            (meta_at(AgentKind::Claude, day0_ms), usage_with_tokens(10)),
            (meta_at(AgentKind::Claude, day0_ms + 3_600_000), usage_with_tokens(5)),
            (meta_at(AgentKind::Codebuddy, day0_ms + 1000), usage_with_tokens(2)),
            (meta_at(AgentKind::Opencode, day0_ms + 86_400_000), usage_with_tokens(7)), // 次日
        ];
        let days = daily_totals_by_agent(&rows);
        assert_eq!(days.len(), 2, "只返回实际有数据的两天,不补空天占位");
        assert_eq!(days[0].day_index, 20_668);
        assert_eq!(days[0].claude, 15, "同一天两条 Claude 会话的 token 要累加");
        assert_eq!(days[0].codebuddy, 2);
        assert_eq!(days[0].opencode, 0);
        assert_eq!(days[1].day_index, 20_669);
        assert_eq!(days[1].opencode, 7);
        assert_eq!(days[0].label, "08/07");
    }

    #[test]
    fn daily_totals_by_agent_keeps_only_most_recent_7_days() {
        let rows: Vec<_> = (0..10)
            .map(|i| {
                (
                    meta_at(AgentKind::Claude, (20_668 + i) as u64 * 86_400_000),
                    usage_with_tokens(1),
                )
            })
            .collect();
        let days = daily_totals_by_agent(&rows);
        assert_eq!(days.len(), 7, "超过 7 天的历史只保留最近 7 天");
        assert_eq!(days.last().unwrap().day_index, 20_677, "最后一天是最新的那天");
        assert_eq!(days.first().unwrap().day_index, 20_671);
    }
```

- [x] **Step 6: 跑测试确认失败**

Run: `cargo test -p dozer-app usage::tests::daily_totals_by_agent`
Expected: FAIL，`cannot find function 'daily_totals_by_agent'`。

- [x] **Step 7: 实现 `DayAgentTotals` + `daily_totals_by_agent`**

```rust
/// 按天、按 agent 聚合的 token 合计（四项 token 加总，不细分 in/out/
/// cache——见 spec"关键语义确认"）。只保留最近 7 天，不足 7 天不补占位
/// 空天，按 `day_index` 升序（最旧在前，最新在后，图表从左到右自然是时间
/// 顺序）。
#[derive(Debug, Clone, PartialEq)]
pub struct DayAgentTotals {
    pub day_index: i64,
    pub label: String,
    pub claude: u64,
    pub codebuddy: u64,
    pub opencode: u64,
}

pub fn daily_totals_by_agent(rows: &[(ConversationMeta, ConversationUsage)]) -> Vec<DayAgentTotals> {
    use std::collections::BTreeMap;
    let mut by_day: BTreeMap<i64, (u64, u64, u64)> = BTreeMap::new();
    for (meta, usage) in rows {
        let day = day_index_from_ms(meta.modified_ms);
        let total = usage.tokens_in + usage.tokens_out + usage.tokens_cache_read + usage.tokens_cache_write;
        let entry = by_day.entry(day).or_insert((0, 0, 0));
        match meta.agent {
            AgentKind::Claude => entry.0 += total,
            AgentKind::Codebuddy => entry.1 += total,
            AgentKind::Opencode => entry.2 += total,
            AgentKind::Unknown => {}
        }
    }
    let mut days: Vec<DayAgentTotals> = by_day
        .into_iter()
        .map(|(day_index, (claude, codebuddy, opencode))| {
            // 年份在"近 7 天"这种短窗口的标签里用不上，解构时直接忽略。
            let (_, m, d) = civil_from_days(day_index);
            DayAgentTotals {
                day_index,
                label: format!("{m:02}/{d:02}"),
                claude,
                codebuddy,
                opencode,
            }
        })
        .collect();
    let start = days.len().saturating_sub(7);
    days.split_off(start)
}
```

- [x] **Step 8: 跑测试确认通过**

Run: `cargo test -p dozer-app usage::tests::daily_totals_by_agent`
Expected: 2 个测试 PASS。

- [x] **Step 9: `agent_token_share` 的测试 + 实现**

```rust
    #[test]
    fn agent_token_share_sums_four_token_fields_per_agent_and_skips_absent_agents() {
        let rows = vec![
            (meta(AgentKind::Claude, "a"), usage_with_tokens(10)),
            (meta(AgentKind::Claude, "b"), usage_with_tokens(5)),
            (meta(AgentKind::Codebuddy, "c"), usage_with_tokens(3)),
        ];
        let share = agent_token_share(&rows);
        assert_eq!(share, vec![(AgentKind::Claude, 15), (AgentKind::Codebuddy, 3)]);
    }
```

实现：

```rust
/// 整个项目范围（不限"近 7 天"）按 agent 的 token 总量（四项合计），供
/// 饼图用；只返回项目里实际出现过的 agent，不产生全零占位记录。
pub fn agent_token_share(rows: &[(ConversationMeta, ConversationUsage)]) -> Vec<(AgentKind, u64)> {
    const ORDER: [AgentKind; 3] = [AgentKind::Claude, AgentKind::Codebuddy, AgentKind::Opencode];
    ORDER
        .into_iter()
        .filter_map(|kind| {
            let total: u64 = rows
                .iter()
                .filter(|(meta, _)| meta.agent == kind)
                .map(|(_, u)| u.tokens_in + u.tokens_out + u.tokens_cache_read + u.tokens_cache_write)
                .sum();
            (total > 0).then_some((kind, total))
        })
        .collect()
}
```

- [x] **Step 10: 跑全部测试**

Run: `cargo test -p dozer-app usage::`
Expected: 全 PASS（10 个测试）。

- [x] **Step 11: Commit**

```bash
git add crates/dozer-app/src/usage.rs
git commit -m "feat(dozer-app): usage.rs 图表数据层(daily_totals_by_agent/agent_token_share)"
```

## Task 4: 新增 `BarChart3` 图标

**Files:**
- Create: `crates/dozer-app/assets/icons/bar-chart-3.svg`
- Modify: `crates/dozer-app/src/icons.rs`

**Interfaces:**
- Produces: `icons::IconKind::BarChart3`，可传给已有的 `icons::view(kind, size, color)`。

- [x] **Step 1: 加 svg 资源**

新建 `crates/dozer-app/assets/icons/bar-chart-3.svg`（Lucide `bar-chart-3`，颜色走 `currentColor`——同目录下 `refresh-cw.svg` 的写法，不能像 Figma 稿里那样写死十六进制颜色，否则 `icons::view()` 的 `.style(...)` 颜色覆盖不生效）：

```svg
<svg
  xmlns="http://www.w3.org/2000/svg"
  width="24"
  height="24"
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="2"
  stroke-linecap="round"
  stroke-linejoin="round"
>
  <path d="M3 3v18h18" />
  <path d="M18 17V9" />
  <path d="M13 17V5" />
  <path d="M8 17v-3" />
</svg>
```

- [x] **Step 2: 加枚举变体 + `bytes()` 分支**

`crates/dozer-app/src/icons.rs`，`IconKind` 枚举里 `ListChecks,` 之后加：

```rust
    /// Agent 用量统计面板的图标(Lucide bar-chart-3)。
    BarChart3,
```

`bytes()` 的 `match` 里 `IconKind::ListChecks => ...` 那一行之后加：

```rust
            IconKind::BarChart3 => include_bytes!("../assets/icons/bar-chart-3.svg"),
```

- [x] **Step 3: 编译确认没有漏改的 match 分支**

Run: `cargo check -p dozer-app`
Expected: 通过（`IconKind` 的 `match` 是穷举的，漏加 `bytes()` 分支会编译失败；这一步就是这个穷举检查本身，没有额外测试要写）。

- [x] **Step 4: Commit**

```bash
git add crates/dozer-app/assets/icons/bar-chart-3.svg crates/dozer-app/src/icons.rs
git commit -m "feat(dozer-app): 新增 BarChart3 图标(用量统计面板入口)"
```

## Task 5: workspace.rs 接线——`RightView::Usage` + 异步刷新 + 图标栏按钮

**Files:**
- Modify: `crates/dozer-app/src/workspace.rs`

**Interfaces:**
- Consumes: `usage::{ConversationUsage, parse_usage}`（Task 1）、`icons::IconKind::BarChart3`（Task 4）、`crate::conversation::{self, ConversationMeta}`（已有）。
- Produces: `Message::UsageRefresh`、`Message::UsageLoaded(ProjectId, Vec<(ConversationMeta, ConversationUsage)>)`、`RightView::Usage`、`pub(crate) fn agent_dot_color(agent: AgentKind) -> Color`（可见性从私有改宽，供 Task 6-8 的 `usage.rs` 调用）。本任务先把 `right_panel_area` 的 `RightView::Usage` 分支接到 `usage::view(...)` 的一个**最小实现**（只有头部 + 加载态/空态，见 Task 6 起再补汇总条/图表/列表）——不是占位符，是这个状态下（还没数据）本来就该长这样的真实渲染。

- [x] **Step 1: `usage.rs` 里先加一个最小 `view()`（本步只让它编译通过，不追求完整视觉）**

`crates/dozer-app/src/usage.rs` 顶部 `use` 区补齐视图层需要的类型：

```rust
use crate::icons;
use crate::theme;
use crate::workspace::Message;
use crate::workspace_font;
use iced_widget::core::{Border, Color, Element, Length};
use iced_widget::{button, column, container, text};
```

文件末尾（`#[cfg(test)]` 之前）加：

```rust
/// 头部：标题 + 项目名 + 右侧手动刷新按钮（spec"面板渲染"#1）。
fn panel_header(project_name: &str) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    iced_widget::row![
        column![
            text("用量统计")
                .size(workspace_font::subtitle())
                .color(theme::CREAM),
            text(project_name)
                .size(workspace_font::label())
                .color(theme::DIM),
        ]
        .spacing(2),
        iced_widget::Space::new().width(Length::Fill),
        button(icons::view::<Message>(icons::IconKind::RefreshCw, 14.0, theme::DIM))
            .on_press(Message::UsageRefresh)
            .style(|_t, _s| button::Style::default()),
    ]
    .align_y(iced_widget::core::Alignment::Center)
    .into()
}

/// 面板主入口，对应右图标栏的"用量统计"视图（单栏，不像 Conversations
/// 那样是"列表:内容"配对分栏——见 spec）。`rows` 为空且 `loading` 为假时
/// 是"还没数据"的空态；`loading` 为真时是刷新中占位态；两者互斥由调用方
/// 保证（`Workspace::usage_loading` 一旦收到 `UsageLoaded` 就会清掉）。
pub fn view<'a>(
    rows: &'a [(ConversationMeta, ConversationUsage)],
    loading: bool,
    project_name: &'a str,
    width: Length,
    outer: Border,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    let mut content = column![panel_header(project_name)].spacing(12).padding(14);

    if loading {
        content = content.push(
            text("统计中…")
                .size(workspace_font::body())
                .color(theme::DIM),
        );
    } else if rows.is_empty() {
        content = content.push(
            text("这个项目还没有 agent 对话记录")
                .size(workspace_font::body())
                .color(theme::DIM),
        );
    }

    container(content)
        .width(width)
        .height(Length::Fill)
        .style(move |_t: &iced_widget::Theme| iced_widget::container::Style {
            background: Some(theme::PANEL.into()),
            border: outer,
            ..iced_widget::container::Style::default()
        })
        .into()
}
```

- [x] **Step 2: `RightView`/`RailButton` 枚举加变体**

`workspace.rs` 里 `pub enum RightView { Agent, Conversations, }`（约第 93-96 行）改成：

```rust
pub enum RightView {
    Agent,
    Conversations,
    Usage,
}
```

`pub enum RailButton { ... RightConversations, }`（约第 102-109 行）加一行：

```rust
    RightUsage,
```

- [x] **Step 3: `Workspace` 结构体加字段 + 初始化**

`pub struct Workspace { ... conversations: Vec<ConversationMeta>, ... }`（约第 1478 行）后面加：

```rust
    /// 当前项目的 agent 用量统计（会话粒度；扫描+解析全量 transcript，比
    /// `conversations` 贵得多，所以不像它那样跟着 `DeliveryChecked` 自动
    /// 刷新——只在切到 `RightView::Usage` 或点手动刷新按钮时才重新扫
    /// （spec 非目标"不做实时更新"）。
    usage: Vec<(ConversationMeta, usage::ConversationUsage)>,
    /// `spawn_usage_refresh` 发起到 `UsageLoaded` 落地之间为真；面板据此
    /// 显示"统计中…"，避免展示陈旧数据被误读成最新值。
    usage_loading: bool,
```

`Workspace::new()`（约第 1804 行 `conversations: Vec::new(),` 附近）加：

```rust
            usage: Vec::new(),
            usage_loading: false,
```

`Workspace::adopt_project`（约第 2225 行 `self.conversations = Vec::new();` 附近）加：

```rust
        self.usage = Vec::new();
        self.usage_loading = false;
```

（这里**不**调用 `spawn_usage_refresh`——新开/切换项目不自动刷新用量,跟 `conversations` 的自动刷新行为刻意不同，见 spec 非目标。）

顶部 `use` 区加：

```rust
use crate::usage;
```

- [x] **Step 4: `Message` 枚举加两个变体**

`pub enum Message { ... ConversationsRefreshed(ProjectId, Vec<ConversationMeta>), ... }`（约第 944 行）附近加：

```rust
    UsageRefresh,
    UsageLoaded(ProjectId, Vec<(ConversationMeta, usage::ConversationUsage)>),
```

- [x] **Step 5: `spawn_usage_refresh` 方法**

紧跟在 `spawn_conversations_refresh`（约第 2076-2092 行）之后加：

```rust
    /// 异步扫当前项目的全部 transcript 并逐个解析用量 → `UsageLoaded`。
    /// 比 `spawn_conversations_refresh` 贵得多(要读整份文件内容，不只是
    /// 文件头)，所以不接入它那条"回合结束自动刷新"的调用链——只在
    /// `RightIconSelect(RightView::Usage)` 或手动刷新按钮时触发。
    fn spawn_usage_refresh(&self, io: &ShellIo) {
        let Some(p) = &self.project else {
            return;
        };
        let project_id = p.id;
        let cwd = PathBuf::from(&p.path);
        let proxy = io.proxy.clone();
        io.handle.spawn(async move {
            let rows = tokio::task::spawn_blocking(move || {
                conversation::list_all_conversations(&cwd)
                    .into_iter()
                    .filter_map(|meta| {
                        // 读失败(权限/IO error)的会话整条跳过、不计入汇总——
                        // 不能退化成"记一条全零 usage"，那样会把这次失败悄悄
                        // 算进 `ProjectUsageTotals::conversation_count`（spec
                        // 错误处理:"该会话跳过、不计入汇总"）。
                        let jsonl = std::fs::read_to_string(&meta.path).ok()?;
                        let u = usage::parse_usage(meta.agent, &jsonl);
                        Some((meta, u))
                    })
                    .collect::<Vec<_>>()
            })
            .await
            .unwrap_or_default();
            let _ = proxy.send_event(Message::UsageLoaded(project_id, rows));
        });
    }
```

- [x] **Step 6: `update()` 里接消息**

`Message::RightIconSelect(v) => { ... }`（约第 4050-4063 行）里，`self.right_view = v; self.right_collapsed = false;` 之后加：

```rust
                    if v == RightView::Usage {
                        self.with_focused_project(|ws, io| {
                            ws.usage_loading = true;
                            ws.spawn_usage_refresh(io);
                        });
                    }
```

`Message::ConversationsRefreshed(project_id, list) => { ... }`（约第 3788 行）附近加两个新分支：

```rust
            Message::UsageRefresh => {
                self.with_focused_project(|ws, io| {
                    ws.usage_loading = true;
                    ws.spawn_usage_refresh(io);
                });
            }
            Message::UsageLoaded(project_id, rows) => {
                self.with_project(project_id, move |ws, _io| {
                    ws.usage = rows;
                    ws.usage_loading = false;
                });
            }
```

- [x] **Step 7: `agent_dot_color` 可见性放宽**

`fn agent_dot_color(agent: AgentKind) -> Color {`（约第 9162 行）改成：

```rust
pub(crate) fn agent_dot_color(agent: AgentKind) -> Color {
```

- [x] **Step 8: `right_icon_rail` 加第三个按钮**

`fn right_icon_rail` 里第二个 `MouseArea::new(rail_icon_button(...))`（`RightConversations`，约第 6608-6621 行）之后加：

```rust
        MouseArea::new(rail_icon_button(
            icons::IconKind::BarChart3,
            app.right_view == RightView::Usage && right_open,
            app.hover_progress(HoverId::Rail(RailButton::RightUsage)),
            Message::RightIconSelect(RightView::Usage),
        ))
        .on_enter(Message::Hover(HoverId::Rail(RailButton::RightUsage), true))
        .on_exit(Message::Hover(HoverId::Rail(RailButton::RightUsage), false)),
```

- [x] **Step 9: `right_panel_area` 加 `RightView::Usage` 分支（单栏，不分割）**

`fn right_panel_area` 里 `let (lc, rc) = if maximized { ... } else { ... };`（约第 6911-6915 行）改成三元，同 `left_panel_area` 现有写法：

```rust
    let (lc, rc, ac) = if maximized {
        (PaneCorner::None, PaneCorner::None, PaneCorner::None)
    } else {
        (PaneCorner::Left, PaneCorner::Right, PaneCorner::All)
    };
```

`match app.right_view { RightView::Agent => {...} RightView::Conversations => {...} }`（约第 6917-6972 行）加第三分支：

```rust
            RightView::Usage => usage::view(
                &ws.usage,
                ws.usage_loading,
                ws.project
                    .as_ref()
                    .map(|p| p.name.as_str())
                    .unwrap_or("未打开项目"),
                Length::Fill,
                zone_pane_border(zone, ac),
            ),
```

- [x] **Step 10: 编译**

Run: `cargo check -p dozer-app`
Expected: 通过。若报 `ws.project`/`ws.usage` 字段不可见——检查是否漏了这段代码本来就在 `workspace.rs` 模块内（`right_panel_area` 在同一个文件里，私有字段直接可读，不需要额外可见性调整）。若报 `usage` 模块里 `use iced_widget::container` 与已导入的 `container` 冲突——按 `iced_widget::container::Style` 全路径写法（Step 1 已经这样写了），不要额外 `use iced_widget::container;`。

- [x] **Step 11: 人工冒烟（这一步没有自动化测试可写——`RightIconSelect`/`with_project` 路由本身已经被 `async_results_route_by_project_id_not_by_focus` 等既有回归测试覆盖，本任务只是新增了一个复用同一条路由机制的调用点，不重复造轮子）**

Run: `cargo run -p dozer-app`，打开任意一个项目，点右图标栏新出现的柱状图图标：应该看到"用量统计 / <项目名>"标题，紧接着"统计中…"一闪而过，然后变成"这个项目还没有 agent 对话记录"（除非这台机器上这个项目路径下真有 `~/.claude/projects/...` 之类的历史 transcript，那样会短暂显示"统计中…"后仍是空态——因为 Task 6 还没实现列表渲染）。

- [x] **Step 12: Commit**

```bash
git add crates/dozer-app/src/workspace.rs crates/dozer-app/src/usage.rs
git commit -m "feat(dozer-app): 接线 RightView::Usage(图标栏按钮/异步刷新/最小面板)"
```

## Task 6: 项目汇总条 + 按 agent 分组的明细列表

**Files:**
- Modify: `crates/dozer-app/src/usage.rs`

**Interfaces:**
- Consumes: `aggregate`、`group_usage_by_agent`（Task 2）、`workspace::agent_dot_color`（Task 5，现在 `pub(crate)`）。
- Produces: `view()` 渲染出汇总条 + 分组列表（本任务不涉及测试——纯 iced 视图代码，同代码库"view 函数不测，人工验收"的既有惯例，测试都在 Task 1-3 的数据层完成了）。

- [x] **Step 1: 写汇总条渲染函数**

`usage.rs` 里加：

```rust
fn summary_card(totals: &ProjectUsageTotals) -> Element<'static, Message, iced_widget::Theme, iced_widget::Renderer> {
    fn stat(label: &'static str, value: String, color: Color) -> Element<'static, Message, iced_widget::Theme, iced_widget::Renderer> {
        column![
            text(label).size(workspace_font::caption()).color(theme::DIM),
            text(value).size(15.0).color(color).font(iced_widget::core::Font::MONOSPACE),
        ]
        .spacing(2)
        .into()
    }

    let row = iced_widget::row![
        stat("轮次", totals.turns.to_string(), theme::CREAM),
        stat(
            "工具调用(改动)",
            format!("{} ({})", totals.tool_calls, totals.mutating_tool_calls),
            theme::CREAM
        ),
        stat("触达文件", totals.files_touched.to_string(), theme::CREAM),
        stat("input", totals.tokens_in.to_string(), theme::CYAN),
        stat("output", totals.tokens_out.to_string(), theme::CYAN),
        stat("cache 读", totals.tokens_cache_read.to_string(), theme::CYAN),
        stat("cache 写", totals.tokens_cache_write.to_string(), theme::CYAN),
    ]
    .spacing(24);

    container(
        column![
            text(format!("项目汇总 · {} 会话", totals.conversation_count))
                .size(workspace_font::caption())
                .color(theme::DIM),
            row,
        ]
        .spacing(10),
    )
    .padding(12)
    .style(|_t: &iced_widget::Theme| iced_widget::container::Style {
        background: Some(theme::CARD.into()),
        border: Border {
            radius: 10.0.into(),
            ..Border::default()
        },
        ..iced_widget::container::Style::default()
    })
    .into()
}
```

- [x] **Step 2: 写单条明细行渲染函数**

```rust
fn usage_row<'a>(
    meta: &'a ConversationMeta,
    u: &'a ConversationUsage,
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    let activity = format!(
        "{} 轮 · {} 次工具({} 改动) · {} 文件",
        u.turns,
        u.tool_calls,
        u.mutating_tool_calls,
        u.files_touched.len()
    );
    let tokens = format!(
        "in {} · out {} · cache读 {} · cache写 {}",
        u.tokens_in, u.tokens_out, u.tokens_cache_read, u.tokens_cache_write
    );
    container(
        column![
            text(meta.title.clone())
                .size(workspace_font::body())
                .color(theme::CREAM),
            text(activity)
                .size(workspace_font::caption_sm())
                .color(theme::DIM)
                .font(iced_widget::core::Font::MONOSPACE),
            text(tokens)
                .size(workspace_font::caption_sm())
                .color(theme::CYAN)
                .font(iced_widget::core::Font::MONOSPACE),
        ]
        .spacing(4),
    )
    .width(Length::Fill)
    .padding(10)
    .style(|_t: &iced_widget::Theme| iced_widget::container::Style {
        background: Some(theme::CARD.into()),
        border: Border {
            radius: 10.0.into(),
            ..Border::default()
        },
        ..iced_widget::container::Style::default()
    })
    .into()
}
```

- [x] **Step 3: 写分组渲染 + 接进 `view()`**

```rust
fn grouped_list<'a>(
    rows: &'a [(ConversationMeta, ConversationUsage)],
) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    let groups = group_usage_by_agent(rows);
    let mut col = column![].spacing(12);
    for (agent, idxs) in groups {
        let group_tokens: u64 = idxs
            .iter()
            .map(|&i| {
                let u = &rows[i].1;
                u.tokens_in + u.tokens_out + u.tokens_cache_read + u.tokens_cache_write
            })
            .sum();
        col = col.push(
            iced_widget::row![
                text(agent.label())
                    .size(workspace_font::caption())
                    .color(crate::workspace::agent_dot_color(agent)),
                text(format!("{} 会话 · {} tokens", idxs.len(), group_tokens))
                    .size(workspace_font::caption())
                    .color(theme::DIM),
            ]
            .spacing(8),
        );
        for &i in &idxs {
            let (meta, u) = &rows[i];
            col = col.push(usage_row(meta, u));
        }
    }
    col.into()
}
```

`view()` 里，把 Task 5 写的"空态/加载态"分支扩成：数据非空且非加载中时追加汇总条+分组列表。改成：

```rust
    if loading {
        content = content.push(
            text("统计中…")
                .size(workspace_font::body())
                .color(theme::DIM),
        );
    } else if rows.is_empty() {
        content = content.push(
            text("这个项目还没有 agent 对话记录")
                .size(workspace_font::body())
                .color(theme::DIM),
        );
    } else {
        let usages: Vec<ConversationUsage> = rows.iter().map(|(_, u)| u.clone()).collect();
        content = content.push(summary_card(&aggregate(&usages)));
        content = content.push(grouped_list(rows));
    }
```

- [x] **Step 4: 编译 + 人工验收**

Run: `cargo check -p dozer-app && cargo run -p dozer-app`

打开一个真的有 Claude/CodeBuddy 历史对话的项目（本机 `~/.claude/projects/` 下随便挑一个已经跑过 Dozer 或原生 CLI 的项目路径），点用量统计图标：应该看到汇总条（7 个统计位）+ 按 agent 分组的卡片列表，数值跟随便手工核对的几条 transcript 大致对得上（不要求逐位精确核对，能看出"数字不是零、不是乱码"就算过关，精确性已经在 Task 1-3 的单测里锁死了）。

- [x] **Step 5: Commit**

```bash
git add crates/dozer-app/src/usage.rs
git commit -m "feat(dozer-app): 用量面板汇总条 + 按 agent 分组明细列表"
```

## Task 7: 近 7 天堆叠条形图

**Files:**
- Modify: `crates/dozer-app/src/usage.rs`

**Interfaces:**
- Consumes: `daily_totals_by_agent`（Task 3）、`workspace::agent_dot_color`。
- Produces: `fn bar_chart(days: &[DayAgentTotals]) -> Element<...>`，接进 `view()`。

- [x] **Step 1: 写堆叠条形图渲染函数**

```rust
const BAR_MAX_HEIGHT: f32 = 72.0;
const BAR_WIDTH: f32 = 20.0;

fn bar_segment(height: f32, color: Color, round_top: bool) -> Element<'static, Message, iced_widget::Theme, iced_widget::Renderer> {
    let radius = if round_top {
        iced_widget::core::border::Radius {
            top_left: 4.0,
            top_right: 4.0,
            ..iced_widget::core::border::Radius::from(0.0)
        }
    } else {
        iced_widget::core::border::Radius::from(0.0)
    };
    container(iced_widget::Space::new())
        .width(Length::Fixed(BAR_WIDTH))
        .height(Length::Fixed(height.max(1.0)))
        .style(move |_t: &iced_widget::Theme| iced_widget::container::Style {
            background: Some(color.into()),
            border: Border {
                radius,
                ..Border::default()
            },
            ..iced_widget::container::Style::default()
        })
        .into()
}

fn bar_chart<'a>(days: &'a [DayAgentTotals]) -> Element<'a, Message, iced_widget::Theme, iced_widget::Renderer> {
    let max_total = days
        .iter()
        .map(|d| d.claude + d.codebuddy + d.opencode)
        .max()
        .unwrap_or(1)
        .max(1);

    let mut bars = iced_widget::row![].spacing(10);
    for d in days {
        let total = d.claude + d.codebuddy + d.opencode;
        let scale = BAR_MAX_HEIGHT / max_total as f32;
        // 自底向上固定顺序:Claude 贴基线(直角)→ CodeBuddy → OpenCode 顶部(圆角)。
        let stack = column![
            bar_segment(d.opencode as f32 * scale, theme::GREEN, true),
            bar_segment(d.codebuddy as f32 * scale, theme::PURPLE, false),
            bar_segment(d.claude as f32 * scale, theme::CYAN, false),
        ]
        .spacing(2);

        let col = column![
            container(
                column![
                    text(format_token_short(total))
                        .size(8.0)
                        .color(theme::DIM)
                        .font(iced_widget::core::Font::MONOSPACE),
                    stack,
                ]
                .spacing(2)
                .align_x(iced_widget::core::alignment::Horizontal::Center),
            )
            .height(Length::Fixed(BAR_MAX_HEIGHT + 14.0))
            .align_y(iced_widget::core::alignment::Vertical::Bottom),
            text(d.label.clone())
                .size(8.0)
                .color(theme::DIM)
                .font(iced_widget::core::Font::MONOSPACE),
        ]
        .spacing(4)
        .align_x(iced_widget::core::alignment::Horizontal::Center);

        bars = bars.push(col);
    }
    bars.into()
}

/// 紧凑数字标签(1234 → "1.2k"，小于 1000 原样显示)，只用于条形图顶部的
/// 总量标注，跟汇总条/明细行的完整数字(不做单位换算)是两回事——图表标签
/// 空间小，明细数字要精确,两者刻意不共用格式化函数。
fn format_token_short(n: u64) -> String {
    if n >= 1000 {
        format!("{:.1}k", n as f32 / 1000.0)
    } else {
        n.to_string()
    }
}
```

- [x] **Step 2: 接进 `view()`**

`view()` 里 `content.push(summary_card(&aggregate(&usages)));` 之后加：

```rust
        let days = daily_totals_by_agent(rows);
        if !days.is_empty() {
            content = content.push(bar_chart(&days));
        }
```

- [x] **Step 3: 编译 + 人工验收**

Run: `cargo check -p dozer-app && cargo run -p dozer-app`

打开有历史用量的项目，用量面板汇总条下方应该出现最多 7 根竖向堆叠柱：每根柱子顶部一个总量数字、柱子分三色段（青/紫/绿，自底向上），底部一个日期标签；柱高应随数值大小有明显差异（不能所有柱子一样高——那说明 `scale` 算错了）。

- [x] **Step 4: Commit**

```bash
git add crates/dozer-app/src/usage.rs
git commit -m "feat(dozer-app): 用量面板近 7 天堆叠条形图"
```

## Task 8: 饼图 + 共享图例

**Files:**
- Modify: `crates/dozer-app/src/usage.rs`

**Interfaces:**
- Consumes: `agent_token_share`（Task 3）、`workspace::agent_dot_color`。
- Produces: `fn pie_chart(share: &[(AgentKind, u64)]) -> Element<...>`（内部一个 `canvas::Program` 实现），`fn chart_legend(share: &[(AgentKind, u64)]) -> Element<...>`，接进 `view()`。

- [x] **Step 1: 写 `canvas::Program` 扇形绘制**

`usage.rs` 顶部补 `use`：

```rust
use iced_widget::canvas::{self, Canvas};
use iced_widget::core::{Pixels, Point, Radians, Rectangle};
```

加：

```rust
const PIE_RADIUS: f32 = 52.0;
const PIE_GAP_RAD: f32 = 0.035;

struct PieChart {
    share: Vec<(AgentKind, u64)>,
}

impl canvas::Program<Message, iced_widget::Theme, iced_widget::Renderer> for PieChart {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced_widget::Renderer,
        _theme: &iced_widget::Theme,
        bounds: Rectangle,
        _cursor: iced_widget::core::mouse::Cursor,
    ) -> Vec<canvas::Geometry<iced_widget::Renderer>> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let center = frame.center();
        let total: u64 = self.share.iter().map(|(_, v)| v).sum();
        if total == 0 {
            return vec![frame.into_geometry()];
        }
        // 12 点钟方向起(-90°),顺时针累加每片的角度(iced 的 Radians 约定
        // "从正 x 轴顺时针"——见 iced_graphics::geometry::path::arc::Arc 文档)。
        let mut angle = Radians(-std::f32::consts::FRAC_PI_2);
        for (agent, value) in &self.share {
            let sweep = Radians(2.0 * std::f32::consts::PI * (*value as f32 / total as f32));
            let start = Radians(angle.0 + PIE_GAP_RAD / 2.0);
            let end = Radians(angle.0 + sweep.0 - PIE_GAP_RAD / 2.0);
            let path = canvas::Path::new(|b| {
                b.arc(canvas::path::Arc {
                    center,
                    radius: PIE_RADIUS,
                    start_angle: start,
                    end_angle: end,
                });
                b.line_to(center);
                b.close();
            });
            frame.fill(&path, crate::workspace::agent_dot_color(*agent));
            angle = Radians(angle.0 + sweep.0);
        }
        vec![frame.into_geometry()]
    }
}

fn pie_chart(share: &[(AgentKind, u64)]) -> Element<'static, Message, iced_widget::Theme, iced_widget::Renderer> {
    Canvas::new(PieChart {
        share: share.to_vec(),
    })
    .width(Length::Fixed(PIE_RADIUS * 2.0 + 8.0))
    .height(Length::Fixed(PIE_RADIUS * 2.0 + 8.0))
    .into()
}
```

- [x] **Step 2: 写共享图例**

```rust
fn chart_legend(share: &[(AgentKind, u64)]) -> Element<'static, Message, iced_widget::Theme, iced_widget::Renderer> {
    let total: u64 = share.iter().map(|(_, v)| v).sum();
    let mut row = iced_widget::row![].spacing(18);
    for (agent, value) in share {
        let pct = if total == 0 { 0 } else { value * 100 / total };
        let dot = container(iced_widget::Space::new())
            .width(Length::Fixed(8.0))
            .height(Length::Fixed(8.0))
            .style({
                let color = crate::workspace::agent_dot_color(*agent);
                move |_t: &iced_widget::Theme| iced_widget::container::Style {
                    background: Some(color.into()),
                    border: Border {
                        radius: 4.0.into(),
                        ..Border::default()
                    },
                    ..iced_widget::container::Style::default()
                }
            });
        row = row.push(
            iced_widget::row![
                dot,
                text(format!("{} {}% · {}", agent.label(), pct, format_token_short(*value)))
                    .size(workspace_font::caption_sm())
                    .color(theme::DIM)
                    .font(iced_widget::core::Font::MONOSPACE),
            ]
            .spacing(6)
            .align_y(iced_widget::core::Alignment::Center),
        );
    }
    row.into()
}
```

- [x] **Step 3: 接进 `view()`**

`view()` 里紧跟 Task 7 加的条形图之后：

```rust
        let share = agent_token_share(rows);
        if !share.is_empty() {
            content = content.push(
                column![
                    chart_legend(&share),
                    pie_chart(&share),
                ]
                .spacing(10),
            );
        }
```

- [x] **Step 4: 编译 + 人工验收**

Run: `cargo check -p dozer-app && cargo run -p dozer-app`

用量面板里应该看到条形图下方一行图例（色点+agent名+百分比+token 数）+ 一个圆形饼图，扇形颜色跟图例一致，扇形之间有细微缝隙（不是完整实心圆被硬切成扇形那种描边感）。跟 Figma 设计稿（node-id=158-30）比对整体观感是否一致。

- [x] **Step 5: Commit**

```bash
git add crates/dozer-app/src/usage.rs
git commit -m "feat(dozer-app): 用量面板 agent 占比饼图 + 共享图例"
```

## Task 9: 收尾验证

**Files:** 无新改动（本任务是全量验证，不产出代码变更）。

- [x] **Step 1: 全量测试**

Run: `cargo test -p dozer-app`
Expected: 全 PASS，无 `usage::` 相关失败。

- [x] **Step 2: clippy + fmt**

Run: `cargo clippy -p dozer-app --all-targets -- -D warnings && cargo fmt --check -p dozer-app`
Expected: 无警告/无格式差异。若 `fmt --check` 有差异，跑 `cargo fmt -p dozer-app` 后重新 `git add`/commit 格式修正。

- [x] **Step 3: 全 workspace 编译**

Run: `cargo build`
Expected: 通过（确认没有破坏 `dozer-core`/`dozerd`/`dozer-hook`/`legacy-boy` 等其它 crate 的编译——本计划理论上完全不碰它们，这一步是兜底确认）。

- [x] **Step 4: 完整人工验收**

Run: `cargo run -p dozer-app`，对照 spec 的"面板渲染"一节逐条过一遍：

1. 右图标栏第三个图标（柱状图）能点开/收起，选中态是金框。
2. 打开一个有真实 Claude/CodeBuddy 历史的项目：加载中短暂显示"统计中…"，随后显示汇总条 + 条形图 + 饼图 + 分组列表。
3. 打开一个全新、从没跑过 agent 的项目：显示"这个项目还没有 agent 对话记录"，不是空白也不是报错。
4. 点头部的刷新图标按钮（Task 5 `panel_header` 已经接了 `Message::UsageRefresh`）：能重新触发一次"统计中…"。
5. 切到另一个项目页签再切回来：数据仍在（不需要重新点刷新），因为 `usage`/`usage_loading` 是每个 `Workspace` 各自持有的。

- [x] **Step 5: 最终确认**

Run: `git log --oneline -10`
Expected: 能看到本计划 Task 1-9 的全部 commit，逐条对应一个可编译可测试的增量。
