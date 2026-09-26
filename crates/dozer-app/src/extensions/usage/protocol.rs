//! Usage 内容侧 webview 推送协议:`content_pane`(view.rs)现有五态判定
//! 逻辑的纯函数版本,产出要序列化推给 webview 的 `UsageViewPayload`。前端
//! 只管照着渲染,不做任何聚合计算——聚合逻辑本身仍在 `aggregate.rs`,一处
//! 不动。

use super::*;
use dozer_core::protocol::AgentKind;
use serde::{Deserialize, Serialize};

/// 没有 `Loading` 变体——"统计中…"是 `byteui::feedback::math_curve` 动画
/// 组件,加载中时这个 webview 根本不挂载(见 `webview_geometry.rs::
/// usage_content_pane_bounds_for` 的 `content_desired` 参数、Task 13),
/// 原生 iced 继续画那个动画,不进这份序列化契约。
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UsageViewPayload {
    Empty,
    AgentEmpty {
        agent: AgentKind,
    },
    SingleAgent {
        agent: AgentKind,
        project: ProjectSummary,
        git_commits: u64,
        session_trend: Option<TrendChart>,
        token_trend: Option<TokenTrendSection>,
    },
    AllAgents {
        project: ProjectSummary,
        git_commits: u64,
        agent_metrics: Option<AgentMetrics>,
        daily_usage: Option<DailyUsageChart>,
        daily_behavior: Option<TrendChart>,
    },
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct ProjectSummary {
    pub conversation_count: u32,
    pub turns: u32,
    pub tool_calls: u32,
    pub files_touched: u32,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub tokens_cache_read: u64,
    pub tokens_cache_write: u64,
}

impl From<&ProjectUsageTotals> for ProjectSummary {
    fn from(t: &ProjectUsageTotals) -> Self {
        Self {
            conversation_count: t.conversation_count,
            turns: t.turns,
            tool_calls: t.tool_calls,
            files_touched: t.files_touched,
            tokens_in: t.tokens_in,
            tokens_out: t.tokens_out,
            tokens_cache_read: t.tokens_cache_read,
            tokens_cache_write: t.tokens_cache_write,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TrendChartKind {
    SessionRound,
    IoTokens,
    CacheTokens,
    Behavior,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TrendDay {
    pub label: String,
    pub values: Vec<u64>,
}

impl From<&DaySeries> for TrendDay {
    fn from(d: &DaySeries) -> Self {
        Self {
            label: d.label.clone(),
            values: d.values.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TrendChart {
    pub kind: TrendChartKind,
    pub days: Vec<TrendDay>,
}

fn trend_chart(kind: TrendChartKind, days: &[DaySeries]) -> Option<TrendChart> {
    has_any_value(days).then(|| TrendChart {
        kind,
        days: days.iter().map(TrendDay::from).collect(),
    })
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TokenTrendSection {
    pub io: Option<TrendChart>,
    pub io_total: u64,
    pub cache: Option<TrendChart>,
    pub cache_total: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AgentShare {
    pub agent: AgentKind,
    pub value: u64,
}

fn shares(v: &[(AgentKind, u64)]) -> Vec<AgentShare> {
    v.iter()
        .map(|&(agent, value)| AgentShare { agent, value })
        .collect()
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AgentMetrics {
    pub session_share: Vec<AgentShare>,
    pub turn_share: Vec<AgentShare>,
    pub io_share: Vec<AgentShare>,
    pub cache_share: Vec<AgentShare>,
    /// "Tokens" 横幅用的四项 token 总和,直接对 `filtered_rows` 求和
    /// (口径同 `view.rs:141-149` 现状的 `total_tokens`),**不是**
    /// `io_share`/`cache_share` 两个已剔除 <1% agent 份额口径的相加——
    /// 两者在"存在被 1% 阈值剔除的 agent"时会有微小差异,横幅要用更大的
    /// 那个"全项目未筛选"口径,所以单独算一份,不让前端从 share 猜。
    pub total_tokens: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DailyUsageDay {
    pub label: String,
    /// 下标与 `DailyUsageChart.agents` 一一对应。
    pub totals: Vec<u64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DailyUsageChart {
    pub agents: Vec<AgentKind>,
    pub days: Vec<DailyUsageDay>,
}

/// `days` 里是否至少一天有非零值——趋势窗口若整段都是 0(目标 agent 最近
/// 这段时间其实没活动),上层就不画这个趋势区,避免白框空难读。
fn has_any_value(days: &[DaySeries]) -> bool {
    days.iter().any(|d| d.values.iter().any(|&v| v > 0))
}

/// 某趋势窗口内所有天、所有子序列值的总和——用于把"标签 + 总量 + 天数"揉
/// 成一行摘要式图例(如 `Input/Output(23.2m/15days)`),2026-09-16 用户要求
/// 把 Token 趋势下两张子图各自的标题/图例收成这一种格式,不再分散成色点
/// 图例 + 独立的"近 N 天"标注两处。
fn trend_total(days: &[DaySeries]) -> u64 {
    days.iter().flat_map(|d| d.values.iter().copied()).sum()
}

/// `view.rs::content_pane` 剩余四态(不含"统计中…",见上面 `UsageViewPayload`
/// 文档)判定逻辑的纯函数版本,不碰 iced,直接产出要推给 webview 的
/// payload。**调用方必须先确认 `!ws_state.loading()`**(`take_usage_content_
/// script`——Task 15——只在 `content_desired` 为真时才调用这个函数);本函数
/// 自身不重复判定 loading,避免"两处都写一遍判据、以后改漏一处"。
/// `content_pane` 改造后不再自己重算这些分支,这里是唯一权威实现。
pub fn current_view_payload(ws_state: &WorkspaceState) -> UsageViewPayload {
    let rows = ws_state.rows();
    if rows.is_empty() {
        return UsageViewPayload::Empty;
    }
    let filtered_rows = filter_rows_by_agent(rows, ws_state.agent_filter);
    if filtered_rows.is_empty() {
        // `filter_rows_by_agent(rows, None)` 原样返回 `rows`(非空),所以
        // 走到这个分支时 `agent_filter` 必然是 `Some`。
        let agent = ws_state
            .agent_filter
            .expect("filtered_rows 为空且 rows 非空时 agent_filter 必为 Some");
        return UsageViewPayload::AgentEmpty { agent };
    }
    let usages: Vec<ConversationUsage> = filtered_rows.iter().map(|(_, u)| u.clone()).collect();
    let project = ProjectSummary::from(&aggregate(&usages));

    match ws_state.agent_filter {
        Some(agent) => {
            let today = today_day_index();
            let session_trend = trend_chart(
                TrendChartKind::SessionRound,
                &session_round_trend(rows, agent, today),
            );
            let token_trend = single_agent_token_trend(agent, rows, today);
            UsageViewPayload::SingleAgent {
                agent,
                project,
                git_commits: ws_state.git_commits(),
                session_trend,
                token_trend,
            }
        }
        None => {
            let session_share = agent_session_share(&filtered_rows);
            let turn_share = agent_turn_share(&filtered_rows);
            let io_share = agent_io_token_share(&filtered_rows);
            let cache_share = agent_cache_token_share(&filtered_rows);
            let agent_metrics = (!session_share.is_empty()
                || !turn_share.is_empty()
                || !io_share.is_empty()
                || !cache_share.is_empty())
            .then(|| AgentMetrics {
                session_share: shares(&session_share),
                turn_share: shares(&turn_share),
                io_share: shares(&io_share),
                cache_share: shares(&cache_share),
                total_tokens: filtered_rows
                    .iter()
                    .map(|(_, u)| {
                        u.tokens_in + u.tokens_out + u.tokens_cache_read + u.tokens_cache_write
                    })
                    .sum(),
            });

            let days = daily_totals_by_agent(&filtered_rows);
            let daily_usage = (!days.is_empty()).then(|| {
                let agents: Vec<AgentKind> = days
                    .first()
                    .map(|d| d.totals.iter().map(|(a, _)| *a).collect())
                    .unwrap_or_default();
                DailyUsageChart {
                    agents,
                    days: days
                        .iter()
                        .map(|d| DailyUsageDay {
                            label: d.label.clone(),
                            totals: d.totals.iter().map(|(_, v)| *v).collect(),
                        })
                        .collect(),
                }
            });

            let behavior = behavior_series(
                &filtered_rows,
                ws_state.git_commits_by_day(),
                today_day_index(),
            );
            let daily_behavior = trend_chart(TrendChartKind::Behavior, &behavior);

            UsageViewPayload::AllAgents {
                project,
                git_commits: ws_state.git_commits(),
                agent_metrics,
                daily_usage,
                daily_behavior,
            }
        }
    }
}

fn single_agent_token_trend(
    agent: AgentKind,
    rows: &[(crate::conversation::ConversationMeta, ConversationUsage)],
    today_index: i64,
) -> Option<TokenTrendSection> {
    let io = io_trend(rows, agent, today_index);
    let cache = cache_trend(rows, agent, today_index);
    if !has_any_value(&io) && !has_any_value(&cache) {
        return None;
    }
    Some(TokenTrendSection {
        io: trend_chart(TrendChartKind::IoTokens, &io),
        io_total: trend_total(&io),
        cache: trend_chart(TrendChartKind::CacheTokens, &cache),
        cache_total: trend_total(&cache),
    })
}

/// webview 发回的事件——只有 `Ready` 一种(JS 端 `window.__dozer.dispatch`
/// 已注册)。不复用 `preview::webview_protocol::EditorEvent`(那个枚举带
/// `DocumentLoaded`/`SelectionChanged` 等 CodeMirror 专属变体);Usage 是
/// 固定单槽、只读渲染,不需要那一整套多 tab/document 协议。
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UsageWebviewEvent {
    Ready,
}

pub fn parse_usage_event(body: &str) -> Result<UsageWebviewEvent, String> {
    serde_json::from_str(body).map_err(|e| e.to_string())
}

/// Rust → webview 推送 envelope。刻意比 `preview::webview_protocol::
/// WebviewEnvelope` 精简(没有 `tab_id`/`document_id`/`revision`/
/// `request_id`)——固定单槽面板,没有多 tab/document 身份需要携带。
/// `preview::webview_protocol::dispatch_script` 只吃"任意 JSON 字符串",
/// 与这里的 payload 类型无关,可以直接复用来包成 `evaluate_script` 脚本
/// (见 `App::take_usage_content_script`,Task 15)。
#[derive(Debug, Clone, Serialize)]
pub struct UsagePushEnvelope {
    pub protocol_version: u32,
    pub payload: UsageViewPayload,
}

pub const USAGE_PROTOCOL_VERSION: u32 = 1;

pub fn encode_usage_push(payload: UsageViewPayload) -> String {
    serde_json::to_string(&UsagePushEnvelope {
        protocol_version: USAGE_PROTOCOL_VERSION,
        payload,
    })
    .unwrap_or_else(|_| "null".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::ConversationMeta;
    use std::path::PathBuf;

    fn meta_at(agent: AgentKind, ms: u64) -> ConversationMeta {
        ConversationMeta {
            path: PathBuf::from(format!("/{ms}.jsonl")),
            title: String::new(),
            modified_ms: ms,
            agent,
        }
    }

    fn usage(turns: u32) -> ConversationUsage {
        ConversationUsage {
            turns,
            ..Default::default()
        }
    }

    /// 供 `daily_totals_by_agent` 用:那条路径吃 `agent_token_share`(按四项
    /// token 合计),零 token 的会话产不出任何天记录,所以要单独造带 token 的。
    fn token_usage(tokens: u64) -> ConversationUsage {
        ConversationUsage {
            tokens_in: tokens,
            ..Default::default()
        }
    }

    #[test]
    fn empty_rows_yields_empty_payload() {
        let ws = WorkspaceState::default();
        assert_eq!(current_view_payload(&ws), UsageViewPayload::Empty);
    }

    #[test]
    fn agent_filter_with_no_matching_rows_yields_agent_empty() {
        let mut ws = WorkspaceState::default();
        update(
            &mut ws,
            Message::Loaded(
                1,
                vec![(meta_at(AgentKind::Claude, 0), usage(1))],
                0,
                Default::default(),
            ),
        );
        update(&mut ws, Message::AgentFilterSet(Some(AgentKind::Codebuddy)));
        assert_eq!(
            current_view_payload(&ws),
            UsageViewPayload::AgentEmpty {
                agent: AgentKind::Codebuddy
            }
        );
    }

    #[test]
    fn single_agent_state_carries_project_summary_and_git_commits() {
        let mut ws = WorkspaceState::default();
        update(
            &mut ws,
            Message::Loaded(
                1,
                vec![(meta_at(AgentKind::Claude, 0), usage(3))],
                7,
                Default::default(),
            ),
        );
        update(&mut ws, Message::AgentFilterSet(Some(AgentKind::Claude)));
        match current_view_payload(&ws) {
            UsageViewPayload::SingleAgent {
                agent,
                project,
                git_commits,
                ..
            } => {
                assert_eq!(agent, AgentKind::Claude);
                assert_eq!(project.turns, 3);
                assert_eq!(git_commits, 7);
            }
            other => panic!("expected SingleAgent, got {other:?}"),
        }
    }

    #[test]
    fn all_agents_state_is_default_when_no_filter_set() {
        let mut ws = WorkspaceState::default();
        update(
            &mut ws,
            Message::Loaded(
                1,
                vec![(meta_at(AgentKind::Claude, 0), usage(3))],
                0,
                Default::default(),
            ),
        );
        assert!(matches!(
            current_view_payload(&ws),
            UsageViewPayload::AllAgents { .. }
        ));
    }

    /// `daily_totals_by_agent` 每天 `totals` 的 agent 顺序必须一致(否则不同
    /// 天的柱子颜色会错位)——`aggregate.rs` 的实现已保证这一点(`project_agents`
    /// 只算一次、逐天 zip 复用同一份顺序),这里锁定 `current_view_payload`
    /// 把 `agents` 只从第一天取出、不逐天重新推导这个简化是安全的。
    #[test]
    fn daily_usage_agents_order_matches_every_day() {
        let mut ws = WorkspaceState::default();
        update(
            &mut ws,
            Message::Loaded(
                1,
                vec![
                    (meta_at(AgentKind::Codebuddy, 0), token_usage(1)),
                    (meta_at(AgentKind::Claude, 0), token_usage(1)),
                    (meta_at(AgentKind::Claude, 86_400_000), token_usage(1)),
                ],
                0,
                Default::default(),
            ),
        );
        match current_view_payload(&ws) {
            UsageViewPayload::AllAgents {
                daily_usage: Some(chart),
                ..
            } => {
                assert_eq!(chart.days.len(), 2);
                for d in &chart.days {
                    assert_eq!(d.totals.len(), chart.agents.len());
                }
            }
            other => panic!("expected AllAgents with daily_usage, got {other:?}"),
        }
    }

    #[test]
    fn single_agent_token_trend_none_when_no_io_or_cache_activity() {
        let mut ws = WorkspaceState::default();
        update(
            &mut ws,
            Message::Loaded(
                1,
                vec![(meta_at(AgentKind::Claude, 0), usage(1))],
                0,
                Default::default(),
            ),
        );
        update(&mut ws, Message::AgentFilterSet(Some(AgentKind::Claude)));
        match current_view_payload(&ws) {
            UsageViewPayload::SingleAgent { token_trend, .. } => assert!(token_trend.is_none()),
            other => panic!("expected SingleAgent, got {other:?}"),
        }
    }

    #[test]
    fn encode_usage_push_embeds_protocol_version_and_payload() {
        let json = encode_usage_push(UsageViewPayload::Empty);
        assert!(json.contains(r#""protocol_version":1"#));
        assert!(json.contains(r#""kind":"empty""#));
    }
}
