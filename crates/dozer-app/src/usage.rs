// crates/dozer-app/src/usage.rs
//! Agent 用量统计面板的数据层与视图层（spec
//! docs/superpowers/specs/2026-08-07-agent-usage-panel-design.md）。数据层是
//! 纯函数,不碰 iced/IO,镜像 `transcript.rs` 的按 agent 分派解析方式;
//! `dozerd`/`dozer-core::protocol` 完全不参与——所有数据直接读磁盘上的
//! agent transcript JSONL。

// Task 5 接线后（`workspace.rs` 调用 `usage::view`/`parse_usage`）这些死代码
// 警告会自然消失；在此之前的中间态暂时放行，避免每轮 cargo check 刷噪音。
#![allow(dead_code)]

use crate::conversation::ConversationMeta;
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
}
