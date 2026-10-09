use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// 单个会话的用量统计(token/工具调用/改动文件),已按 `message_key` 做过
/// fork/resume 去重(spec"用量去重"一节)。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UsagePayload {
    pub turns: u32,
    pub tool_calls: u32,
    pub mutating_tool_calls: u32,
    pub files_touched: BTreeSet<String>,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub tokens_cache_read: u64,
    pub tokens_cache_write: u64,
}
