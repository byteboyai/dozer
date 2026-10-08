//! Usage 面板可行性探针(O12)的夹具,规格 `docs/superpowers/specs/2026-10-08-usage-panel-probe-design.md`。
//! 只在测试里存在;放在 host 一侧而不是 `extensions/`,因为它扮演的是 host。

mod fake_dozerd;

use dozer_core::protocol::{AgentKind, ConversationSummary, UsagePayload};
use std::collections::BTreeSet;

pub(crate) const NOW_MS: u64 = 1_790_000_000_000;

pub(crate) fn summary(id: &str, agent: AgentKind, last_ts_ms: u64) -> ConversationSummary {
    ConversationSummary {
        conversation_id: id.into(),
        agent,
        file_path: format!("/t/{id}.jsonl"),
        title: id.into(),
        first_ts: last_ts_ms,
        last_ts: last_ts_ms,
        turn_count: 3,
    }
}

pub(crate) fn payload(turns: u32) -> UsagePayload {
    UsagePayload {
        turns,
        tool_calls: 2,
        mutating_tool_calls: 1,
        files_touched: BTreeSet::new(),
        tokens_in: 100,
        tokens_out: 50,
        tokens_cache_read: 10,
        tokens_cache_write: 5,
    }
}
