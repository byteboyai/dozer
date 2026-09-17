//! 用量统计的纯聚合计算:ConversationUsage/ProjectUsageTotals/DailySeries
//! 及 agent 份额、趋势序列等纯函数(不碰 iced/IO)。

use crate::conversation::ConversationMeta;
use dozer_core::protocol::AgentKind;
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

impl From<&dozer_core::protocol::UsagePayload> for ConversationUsage {
    fn from(p: &dozer_core::protocol::UsagePayload) -> Self {
        Self {
            turns: p.turns,
            tool_calls: p.tool_calls,
            mutating_tool_calls: p.mutating_tool_calls,
            files_touched: p.files_touched.clone(),
            tokens_in: p.tokens_in,
            tokens_out: p.tokens_out,
            tokens_cache_read: p.tokens_cache_read,
            tokens_cache_write: p.tokens_cache_write,
        }
    }
}

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

/// `daily_totals_by_agent` 只保留最近这么多天的数据(2026-08-28 起
/// 15→7,产品改回看近一周的紧凑窗口)。
pub(crate) const DAILY_CHART_WINDOW_DAYS: usize = 7;

/// epoch 毫秒 → 该毫秒所在的 UTC 日索引(自 1970-01-01 起的第几天)。用于
/// 按天分桶;**不做本地时区换算**——纯 std 没有时区能力,引入 `chrono`/`time`
/// 属于新增依赖(spec 明确不新增)，UTC 分桶对"看近 7 天趋势形状"这个用途
/// 足够，不追求跟用户本地墙上时钟严格对齐。
pub(crate) fn day_index_from_ms(ms: u64) -> i64 {
    (ms / 86_400_000) as i64
}

/// 当前时刻对应的 UTC 日索引，供趋势图当时间轴的右端点（“最近 N 天”）。同上
/// 不做时区换算，与 `day_index_from_ms`/`daily_totals_by_agent` 的分桶口径一致。
pub(crate) fn today_day_index() -> i64 {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    day_index_from_ms(ms)
}

/// UTC 日索引 → (year, month, day)。Howard Hinnant 的公开 civil_from_days
/// 算法(纯数学换算，不依赖任何日期库)。
pub(crate) fn civil_from_days(z: i64) -> (i32, u32, u32) {
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

/// 按天、按 agent 聚合的 token 合计（四项 token 加总，不细分 in/out/
/// cache——见 spec"关键语义确认"）。只保留最近 `DAILY_CHART_WINDOW_DAYS`
/// 天，不足这个天数不补占位空天，按 `day_index` 升序（最旧在前，最新在后，
/// 图表从左到右自然是时间顺序）。`totals` 只含这个项目实际用过的 agent
/// (与 `agent_token_share` 同一套判定+顺序),不再是写死的 3/4 家——某个
/// agent 这个项目压根没用过,就不该在柱状图里占一个永远是 0 的位置
/// (2026-08-27 修正,原先固定 claude/codebuddy/opencode 三个字段,V8agent
/// 完全进不了图,别的项目哪怕只用一家 agent 也照样画三根柱子)。同一个
/// agent 在所有日期分组里 `totals` 的顺序保持一致,方便跨日对比同一根
/// 柱子的颜色/位置。
#[derive(Debug, Clone, PartialEq)]
pub struct DayAgentTotals {
    pub day_index: i64,
    pub label: String,
    pub totals: Vec<(AgentKind, u64)>,
}

pub fn daily_totals_by_agent(
    rows: &[(ConversationMeta, ConversationUsage)],
) -> Vec<DayAgentTotals> {
    use std::collections::BTreeMap;
    let project_agents: Vec<AgentKind> = agent_token_share(rows)
        .into_iter()
        .map(|(k, _)| k)
        .collect();
    if project_agents.is_empty() {
        return Vec::new();
    }
    // 内层按 `project_agents` 里的下标(而不是 `AgentKind` 本身)分桶——
    // `AgentKind` 没实现 `Ord`/`Hash`,当不了 `BTreeMap`/`HashMap` 的键;
    // agent 数量本来就是个位数,线性查下标够用,不值得为此给协议层的
    // `AgentKind` 加派生。
    let mut by_day: BTreeMap<i64, Vec<u64>> = BTreeMap::new();
    for (meta, usage) in rows {
        let Some(idx) = project_agents.iter().position(|&a| a == meta.agent) else {
            continue;
        };
        let day = day_index_from_ms(meta.modified_ms);
        let total =
            usage.tokens_in + usage.tokens_out + usage.tokens_cache_read + usage.tokens_cache_write;
        let bucket = by_day
            .entry(day)
            .or_insert_with(|| vec![0; project_agents.len()]);
        bucket[idx] += total;
    }
    let mut days: Vec<DayAgentTotals> = by_day
        .into_iter()
        .map(|(day_index, bucket)| {
            // 年份在"近 7 天"这种短窗口的标签里用不上，解构时直接忽略。
            let (_, m, d) = civil_from_days(day_index);
            let totals = project_agents.iter().copied().zip(bucket).collect();
            DayAgentTotals {
                day_index,
                label: format!("{m:02}/{d:02}"),
                totals,
            }
        })
        .collect();
    let start = days.len().saturating_sub(DAILY_CHART_WINDOW_DAYS);
    days.split_off(start)
}

/// 选中具体 agent 后的存量趋势聚合（2026-09-05,+n）数据的单天结构：某天的
/// 若干并列可度量量（`values[i]` 的含义由调用方给出序列标签）。与上面
/// `DayAgentTotals` 专为“每天按 agent 分几根柱”不同，这里“每天并列几根
/// 量”由 `values` 按序列下标承载——实际服务 Session 趋势（会话/回合）与
/// Token 趋势（Input/Output、Cache read/write）。
#[derive(Debug, Clone, PartialEq)]
pub struct DaySeries {
    pub day_index: i64,
    pub label: String,
    pub values: Vec<u64>,
}

/// 三个趋势各自的最近窗口天数（2026-09-05 用户要求：Session 与 Input/Output
/// 看近 15 天整的一条轴；Cache read/write 原本只看近 5 天更紧的一跳，
/// 2026-09-14 用户要求统一改成 15 天，跟其余两条趋势轴同口径）。
pub(crate) const SESSION_TREND_WINDOW: i64 = 15;
pub(crate) const IO_TREND_WINDOW: i64 = 15;
pub(crate) const CACHE_TREND_WINDOW: i64 = 15;

/// 把某 agent 的会话按天分桶、摊成最近 `window` 天的一根连续时间轴（最旧在
/// 左、今天在右，`values[bar_idx]` 交给 `per_row` 逐条会话各取一个标量求和；
/// `conversation_count` 的“每会话记 1”由 `session` 桶单独算）。空的天不会
/// 被丢掉——趋势类图表如果只画实际有数据的日期，稀疏使用看起来就像是连续的
/// 活跃曲线（计划待办里测过、产品方向也认可连续轴）。时间轴右端落在
/// `last_day_index`（调方传“今天”，单测传固定值保证可复现），不从最新的
/// 数据那天开始反向截——那样一旦最近几天没数据窗口就会漂移。
///
/// 返回的 `DaySeries.values.len() == per_series_count`（统一每列都有那么多根
/// 柱，未填充的是 0），保证渲染时某天某序列缺数据也能占一根、不塌列。
pub(crate) fn trend_series(
    rows: &[(ConversationMeta, ConversationUsage)],
    agent: AgentKind,
    window: i64,
    last_day_index: i64,
    per_row: impl Fn(&ConversationUsage) -> Vec<u64>,
) -> Vec<DaySeries> {
    let first_day = last_day_index - window + 1;
    let count = per_row(&ConversationUsage::default()).len();
    let mut buckets: std::collections::BTreeMap<i64, Vec<u64>> = std::collections::BTreeMap::new();
    for (meta, usage) in rows {
        if meta.agent != agent {
            continue;
        }
        let day = day_index_from_ms(meta.modified_ms);
        if day < first_day || day > last_day_index {
            continue;
        }
        let row = per_row(usage);
        let slot = buckets.entry(day).or_insert_with(|| vec![0; count]);
        for (i, v) in row.into_iter().enumerate() {
            slot[i] = slot[i].saturating_add(v);
        }
    }
    (first_day..=last_day_index)
        .map(|day_index| {
            let (_, m, d) = civil_from_days(day_index);
            let values = buckets.remove(&day_index).unwrap_or_else(|| vec![0; count]);
            DaySeries {
                day_index,
                label: format!("{m:02}/{d:02}"),
                values,
            }
        })
        .collect()
}

/// Session 趋势：某 agent 最近 `SESSION_TREND_WINDOW` 天逐日 `[会话数, 回合数]`
/// 两序列。会话数=当日的 transcript（会话文件）条数，与 `agent_session_share`
/// 口径一致（含零回合空会话，各记 1）；回合数=`turns` 求和。
pub(crate) fn session_round_trend(
    rows: &[(ConversationMeta, ConversationUsage)],
    agent: AgentKind,
    today_index: i64,
) -> Vec<DaySeries> {
    trend_series(rows, agent, SESSION_TREND_WINDOW, today_index, |u| {
        vec![1, u.turns as u64]
    })
}

pub(crate) fn io_trend(
    rows: &[(ConversationMeta, ConversationUsage)],
    agent: AgentKind,
    today_index: i64,
) -> Vec<DaySeries> {
    trend_series(rows, agent, IO_TREND_WINDOW, today_index, |u| {
        vec![u.tokens_in, u.tokens_out]
    })
}

pub(crate) fn cache_trend(
    rows: &[(ConversationMeta, ConversationUsage)],
    agent: AgentKind,
    today_index: i64,
) -> Vec<DaySeries> {
    trend_series(rows, agent, CACHE_TREND_WINDOW, today_index, |u| {
        vec![u.tokens_cache_read, u.tokens_cache_write]
    })
}

/// 展示固定顺序（含 V8agent，CLAUDE.md：新功能默认覆盖它，不能像 Codex/
/// Kilo 那样被漏掉）。2026-09-05 起把原先三份拷贝收拢成这一份常量，供下面
/// 各 `agent_*_share` 与 `agents_present` 共用，避免动一处漏一处的旧坑。
pub(crate) const AGENT_ORDER: [AgentKind; 4] = [
    AgentKind::Claude,
    AgentKind::Codebuddy,
    AgentKind::Opencode,
    AgentKind::V8agent,
];

/// 整个项目范围按 agent 归总某会话级标量（`per_row` 从每条会话取一个数）。
/// 只返回总和 > 0 的 agent，不产生全零占位记录、不改变 `AGENT_ORDER` 顺序
/// ——四种统计饼图共用同一套口径与起止，图例顺序始终对齐。
pub(crate) fn agent_metric_share(
    rows: &[(ConversationMeta, ConversationUsage)],
    per_row: impl Fn(&ConversationUsage) -> u64,
) -> Vec<(AgentKind, u64)> {
    AGENT_ORDER
        .into_iter()
        .filter_map(|kind| {
            let total: u64 = rows
                .iter()
                .filter(|(meta, _)| meta.agent == kind)
                .map(|(_, u)| per_row(u))
                .sum();
            (total > 0).then_some((kind, total))
        })
        .collect()
}

/// 整个项目范围（不限"近 7 天"）按 agent 的 token 总量（四项合计）。四张
/// 统计饼图从这条总口径切出细分口径（见 `agent_io_token_share`、`agent_cache_
/// token_share`）前，`daily_totals_by_agent` 仍用它的 agent 集合当"本项目出现
/// 过的 agent"排序基准。
pub fn agent_token_share(rows: &[(ConversationMeta, ConversationUsage)]) -> Vec<(AgentKind, u64)> {
    agent_metric_share(rows, |u| {
        u.tokens_in + u.tokens_out + u.tokens_cache_read + u.tokens_cache_write
    })
}

/// 整个项目范围按 agent 的"回合"数合计——口径沿用用量面板的"回合"即
/// human 发言数（2026-08-27 调整，见 dozerd `get_usage_summary_in`）。
pub fn agent_turn_share(rows: &[(ConversationMeta, ConversationUsage)]) -> Vec<(AgentKind, u64)> {
    agent_metric_share(rows, |u| u.turns as u64)
}

/// 每个 agent 的"会话数"＝它在项目里留下的 transcript 会话条数。每一条
/// 会话记 1（含零回合的空会话），让"这个 agent 跑过几个会话"独立成立。
pub fn agent_session_share(
    rows: &[(ConversationMeta, ConversationUsage)],
) -> Vec<(AgentKind, u64)> {
    agent_metric_share(rows, |_| 1)
}

/// Input/Output token：每会话上下文进出量（`tokens_in + tokens_out`）。
pub fn agent_io_token_share(
    rows: &[(ConversationMeta, ConversationUsage)],
) -> Vec<(AgentKind, u64)> {
    agent_metric_share(rows, |u| u.tokens_in + u.tokens_out)
}

/// Cache Read/Write token：每会话提示缓存读写/写入量
/// （`tokens_cache_read + tokens_cache_write`），与 IO 量分开看图。
pub fn agent_cache_token_share(
    rows: &[(ConversationMeta, ConversationUsage)],
) -> Vec<(AgentKind, u64)> {
    agent_metric_share(rows, |u| u.tokens_cache_read + u.tokens_cache_write)
}

/// 项目里实际出现过的 agent,顺序固定(共用上面的 `AGENT_ORDER`,含 V8agent
/// ——新功能默认覆盖它,不能像 Codex/Kilo 那样被漏掉)。供右侧筛选栏用:传入
/// 未经筛选的全量 `rows`,这样切换到某个 agent 之后,列表本身不会跟着收缩到
/// 只剩它自己。
pub fn agents_present(rows: &[(ConversationMeta, ConversationUsage)]) -> Vec<AgentKind> {
    AGENT_ORDER
        .into_iter()
        .filter(|kind| rows.iter().any(|(meta, _)| meta.agent == *kind))
        .collect()
}

/// 按右侧筛选栏当前选中的 agent 过滤统计用的行,`None` = 不过滤("全部
/// agent")。返回拥有所有权的克隆而不是借用——下游 `aggregate`/
/// `agent_token_share`/`daily_totals_by_agent` 都吃
/// `&[(ConversationMeta, ConversationUsage)]`,筛选后条数通常不大,直接拥
/// 有一份比额外弄一层借用包装简单。
pub(crate) fn filter_rows_by_agent(
    rows: &[(ConversationMeta, ConversationUsage)],
    filter: Option<AgentKind>,
) -> Vec<(ConversationMeta, ConversationUsage)> {
    match filter {
        None => rows.to_vec(),
        Some(agent) => rows
            .iter()
            .filter(|(meta, _)| meta.agent == agent)
            .cloned()
            .collect(),
    }
}
