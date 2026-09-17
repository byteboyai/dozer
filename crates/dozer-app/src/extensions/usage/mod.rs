//! Agent 用量统计面板的数据层与视图层（spec
//! docs/superpowers/specs/2026-08-07-agent-usage-panel-design.md）。数据层是
//! 纯函数,不碰 iced/IO,镜像 `transcript.rs` 的按 agent 分派解析方式;
//! `dozerd`/`dozer-core::protocol` 完全不参与——所有数据直接读磁盘上的
//! agent transcript JSONL。

use crate::conversation::ConversationMeta;
use dozer_core::protocol::AgentKind;
use std::path::PathBuf;

mod aggregate;
mod chart;
mod view;

pub(crate) use aggregate::*;
pub(crate) use chart::*;
pub(crate) use view::*;

/// 挂在每个 Workspace 上的 Usage 面板状态,对应现有 `Workspace` 上
/// `usage`/`usage_loading` 两个字段。
#[derive(Default)]
pub struct WorkspaceState {
    rows: Vec<(ConversationMeta, ConversationUsage)>,
    loading: bool,
    /// 右侧 agent 筛选栏当前选中项:`None` = "全部agent"(默认,不过滤)。
    agent_filter: Option<AgentKind>,
}

impl WorkspaceState {
    pub fn rows(&self) -> &[(ConversationMeta, ConversationUsage)] {
        &self.rows
    }

    pub fn loading(&self) -> bool {
        self.loading
    }

    /// 供内核 `PanelSelect(PanelKind::Usage)` 分支调用——切到面板时
    /// 立即标记"统计中",不等 `spawn_refresh` 的异步结果落地才置真。
    pub fn set_loading(&mut self, loading: bool) {
        self.loading = loading;
    }

    /// 有没有数据可展示 agent 筛选栏(`list_pane`)——加载中/还没数据时
    /// 不拼两栏,内容侧独占全宽,同改造前 `sidebar: Option<..>` 为 `None`
    /// 时的行为(见 app.rs `PanelKind::Usage` 分支)。
    pub fn has_agent_filter(&self) -> bool {
        !self.loading && !self.rows.is_empty()
    }
}

/// 对应现在顶层 `Message` 里的 `UsageLoaded` 变体,去前缀原样搬来。
/// `Refresh`/`Hover` 随手动刷新按钮一起移除——进入面板时由
/// `Workspace::spawn_usage_refresh` 自动刷新,不再需要面板内按钮。
#[derive(Debug, Clone)]
pub enum Message {
    Loaded(i64, Vec<(ConversationMeta, ConversationUsage)>),
    /// 右侧 agent 筛选栏点击(2026-08-28):`None` 选"全部agent"。
    AgentFilterSet(Option<AgentKind>),
    /// 内容侧"收起/展开列表列"按钮:内核拦截,不进 `update`——转发成顶层
    /// `Message::TogglePanelListCollapse(PanelKind::Usage)`(见 app.rs)。
    ToggleListCollapse,
    /// 任意顶部 `HoverId` 的悬停进入/离开(收起按钮等)。内核拦截转发给
    /// 顶层 `App::set_hover`,本面板 `update` 保 no-op 分支维持 match 穷尽。
    Hover(crate::app::HoverId, bool),
}

pub fn update(ws_state: &mut WorkspaceState, msg: Message) {
    match msg {
        Message::Loaded(_, rows) => {
            ws_state.rows = rows;
            ws_state.loading = false;
        }
        Message::AgentFilterSet(agent) => {
            ws_state.agent_filter = agent;
        }
        Message::ToggleListCollapse => {
            unreachable!("由内核拦截处理,见 usage::Message::ToggleListCollapse 文档")
        }
        Message::Hover(_, _) => {
            unreachable!("由内核拦截处理,见 usage::Message::Hover 文档")
        }
    }
}

/// 异步扫描项目全部 agent transcript 并逐个解析用量。内核在
/// `PanelSelect(PanelKind::Usage)` 分支(切到面板时自动刷新)调用,
/// 经 `Workspace::spawn_usage_refresh` 转发。现有
/// `Workspace::spawn_usage_refresh` 的搬家版本,逻辑不变(读失败的会话
/// 整条跳过、不计入汇总)。
pub fn spawn_refresh(
    project_id: i64,
    project_path: PathBuf,
    client: &dozer_client::Client,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    let client = client.clone();
    handle.spawn(async move {
        let cwd = project_path.to_string_lossy().into_owned();
        let rows = client
            .get_usage_summary(&cwd, None)
            .await
            .unwrap_or_default()
            .iter()
            .map(|(summary, payload)| {
                (
                    crate::conversation::ConversationMeta::from_summary(summary),
                    ConversationUsage::from(payload),
                )
            })
            .collect::<Vec<_>>();
        emit(Message::Loaded(project_id, rows));
    });
}

/// 面板内容侧:统计图表 + 顶部"用量"标题。原先跟 `list_pane`(agent 筛选栏)
/// 挤在同一个 `view` 函数里手写 `row![content, sidebar]`,现在拆成独立的
/// 列表/内容两个面板函数,接入跟 Database/Agent 面板一样的可拖拽 split +
/// 收起机制(见 app.rs `PanelKind::Usage` 分支、`Divider::UsageSplit`)。
/// `rows` 为空且 `loading` 为假时是"还没数据"的空态；`loading` 为真时是
/// 刷新中占位态；两者互斥由 `update` 保证(`WorkspaceState::set_loading`
/// 调用后、`Loaded` 落地时清掉)。
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nice_tick_step_rounds_to_1_2_5_family() {
        // 7 / 4 = 1.75 → 落在 (1,2] 档,取 2。
        assert_eq!(nice_tick_step(7, 4), 2);
        // 42 / 4 = 10.5 → 数量级 10,余数 1.05 → (1,2] 档,取 2*10=20。
        assert_eq!(nice_tick_step(42, 4), 20);
        // 1 / 4 = 0.25 → 数量级 0.1,余数 2.5 → (2,5] 档,取 5*0.1 四舍五入为 1。
        assert_eq!(nice_tick_step(1, 4), 1);
    }

    #[test]
    fn grid_ticks_stops_at_max_and_never_empty() {
        assert_eq!(grid_ticks(0), Vec::<u64>::new());
        // step=2(见上一条用例),数到 <=7 为止:2,4,6。
        assert_eq!(grid_ticks(7), vec![2, 4, 6]);
        // 数据量很小时(max_total < step)也至少给一条线兜底。
        assert_eq!(grid_ticks(1), vec![1]);
    }

    #[test]
    fn format_count_three_tier_k_m_rounding() {
        // 千以下原样。
        assert_eq!(format_count(0_u64), "0");
        assert_eq!(format_count(999_u64), "999");
        // ≥1000 且 <1,000,000 → k,1 位小数。`1000` 直接进 `1.0k`,
        // 不出现 `1000.0k` 的中间档(见函数注释)。
        assert_eq!(format_count(1000_u64), "1.0k");
        assert_eq!(format_count(1234_u64), "1.2k");
        assert_eq!(format_count(123_456_u64), "123.5k");
        assert_eq!(format_count(999_999_u64), "1000.0k");
        // ≥1,000,000 → m,1 位小数。`1,000,000` 直接进 `1.0m`。
        assert_eq!(format_count(1_000_000_u64), "1.0m");
        assert_eq!(format_count(1_234_567_u64), "1.2m");
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
    fn conversation_usage_from_payload_maps_all_fields() {
        let payload = dozer_core::protocol::UsagePayload {
            turns: 2,
            tool_calls: 1,
            mutating_tool_calls: 1,
            files_touched: std::collections::BTreeSet::from(["README.md".to_string()]),
            tokens_in: 10,
            tokens_out: 20,
            tokens_cache_read: 1,
            tokens_cache_write: 2,
        };
        let usage = ConversationUsage::from(&payload);
        assert_eq!(usage.turns, 2);
        assert_eq!(usage.tool_calls, 1);
        assert_eq!(usage.mutating_tool_calls, 1);
        assert_eq!(
            usage.files_touched,
            std::collections::BTreeSet::from(["README.md".to_string()])
        );
        assert_eq!(usage.tokens_in, 10);
        assert_eq!(usage.tokens_out, 20);
        assert_eq!(usage.tokens_cache_read, 1);
        assert_eq!(usage.tokens_cache_write, 2);
    }

    #[test]
    fn aggregate_sums_fields_and_dedups_files_across_conversations() {
        let rows = [
            sample_usage(&["/a.rs", "/b.rs"]),
            sample_usage(&["/a.rs", "/c.rs"]),
        ];
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
            agent,
        }
    }

    #[test]
    fn civil_from_days_known_epoch_dates() {
        // 1970-01-01 是 epoch day 0。
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        // 2026-08-07 手工核对(用 `date -u -j -f "%Y-%m-%d" 2026-08-07 +%s`
        // 算出 epoch 秒 1786071805 再除以 86400 取整得到 day_index=20672)。
        assert_eq!(civil_from_days(20672), (2026, 8, 7));
    }

    fn meta_at(agent: AgentKind, ms: u64) -> ConversationMeta {
        ConversationMeta {
            path: std::path::PathBuf::from(format!("/{ms}.jsonl")),
            title: String::new(),
            modified_ms: ms,
            agent,
        }
    }

    fn usage_with_tokens(input: u64) -> ConversationUsage {
        ConversationUsage {
            tokens_in: input,
            ..Default::default()
        }
    }

    fn total_for(day: &DayAgentTotals, agent: AgentKind) -> u64 {
        day.totals
            .iter()
            .find(|(a, _)| *a == agent)
            .map(|(_, v)| *v)
            .unwrap_or(0)
    }

    #[test]
    fn daily_totals_by_agent_buckets_by_day_and_sums_per_agent() {
        // day 20672 = 2026-08-07 00:00:00 UTC 起的毫秒;+3600_000 还在同一天。
        let day0_ms = 20_672u64 * 86_400_000;
        let rows = vec![
            (meta_at(AgentKind::Claude, day0_ms), usage_with_tokens(10)),
            (
                meta_at(AgentKind::Claude, day0_ms + 3_600_000),
                usage_with_tokens(5),
            ),
            (
                meta_at(AgentKind::Codebuddy, day0_ms + 1000),
                usage_with_tokens(2),
            ),
            (
                meta_at(AgentKind::Opencode, day0_ms + 86_400_000),
                usage_with_tokens(7),
            ), // 次日
        ];
        let days = daily_totals_by_agent(&rows);
        assert_eq!(days.len(), 2, "只返回实际有数据的两天,不补空天占位");
        assert_eq!(days[0].day_index, 20_672);
        assert_eq!(
            total_for(&days[0], AgentKind::Claude),
            15,
            "同一天两条 Claude 会话的 token 要累加"
        );
        assert_eq!(total_for(&days[0], AgentKind::Codebuddy), 2);
        assert_eq!(
            total_for(&days[0], AgentKind::Opencode),
            0,
            "Opencode 这个项目里用过(次日有),当天没用则该 agent 那天是 0,不是不出现"
        );
        assert_eq!(days[1].day_index, 20_673);
        assert_eq!(total_for(&days[1], AgentKind::Opencode), 7);
        assert_eq!(days[0].label, "08/07");
    }

    #[test]
    fn daily_totals_by_agent_only_includes_agents_actually_used_in_project() {
        // 这个项目只用过 Claude,不该在图里给从没出现过的 Codebuddy/Opencode
        // 各占一根常年 0 的柱子(2026-08-27 修正的真实 bug)。
        let rows = vec![(meta_at(AgentKind::Claude, 0), usage_with_tokens(10))];
        let days = daily_totals_by_agent(&rows);
        assert_eq!(days.len(), 1);
        assert_eq!(
            days[0].totals,
            vec![(AgentKind::Claude, 10)],
            "只应该出现 Claude 一项,不该混进 Codebuddy/Opencode 的 0 值占位"
        );
    }

    #[test]
    fn daily_totals_by_agent_includes_v8agent() {
        let rows = vec![(meta_at(AgentKind::V8agent, 0), usage_with_tokens(3))];
        let days = daily_totals_by_agent(&rows);
        assert_eq!(days.len(), 1);
        assert_eq!(total_for(&days[0], AgentKind::V8agent), 3);
    }

    #[test]
    fn daily_totals_ignores_agents_without_dedicated_bucket() {
        // Codex/Kilo 目前不产出可统计的用量数据（见计划 Global
        // Constraints），跟 Unknown 一样被忽略，不能 panic,也不产生
        // 任何一天的记录(没有任何可展示的 agent,图表应该整体不渲染)。
        let rows = vec![(meta_at(AgentKind::Codex, 0), usage_with_tokens(99))];
        let days = daily_totals_by_agent(&rows);
        assert!(days.is_empty(), "没有任何已知 agent 有数据时不该产出天记录");
    }

    #[test]
    fn daily_totals_by_agent_keeps_only_most_recent_7_days() {
        let rows: Vec<_> = (0..20)
            .map(|i| {
                (
                    meta_at(AgentKind::Claude, (20_668 + i) as u64 * 86_400_000),
                    usage_with_tokens(1),
                )
            })
            .collect();
        let days = daily_totals_by_agent(&rows);
        assert_eq!(days.len(), 7, "超过 7 天的历史只保留最近 7 天");
        assert_eq!(
            days.last().unwrap().day_index,
            20_687,
            "最后一天是最新的那天"
        );
        assert_eq!(days.first().unwrap().day_index, 20_681);
    }

    #[test]
    fn agent_token_share_sums_four_token_fields_per_agent_and_skips_absent_agents() {
        let rows = vec![
            (meta(AgentKind::Claude, "a"), usage_with_tokens(10)),
            (meta(AgentKind::Claude, "b"), usage_with_tokens(5)),
            (meta(AgentKind::Codebuddy, "c"), usage_with_tokens(3)),
        ];
        let share = agent_token_share(&rows);
        assert_eq!(
            share,
            vec![(AgentKind::Claude, 15), (AgentKind::Codebuddy, 3)]
        );
    }

    /// V8agent(用户自研 agent)默认要被用量统计覆盖,不能像 Codex/Kilo 那样
    /// 被排除——2026-08-27 之前 `ORDER` 常量漏了它,饼图/图例里完全不出现
    /// 这家的用量,是真实 bug 不是刻意范围收窄。
    #[test]
    fn agent_token_share_includes_v8agent() {
        let rows = vec![(meta(AgentKind::V8agent, "a"), usage_with_tokens(7))];
        let share = agent_token_share(&rows);
        assert_eq!(share, vec![(AgentKind::V8agent, 7)]);
    }

    #[test]
    fn agent_session_share_counts_one_per_conversation_including_zero_turn() {
        let rows = vec![
            // 同一 agent 两条会话。
            (meta(AgentKind::Claude, "a"), usage_with_turns(3)),
            (meta(AgentKind::Claude, "b"), usage_with_turns(0)),
            // 不同 agent 一条会话(也给回合)。
            (meta(AgentKind::Codebuddy, "c"), usage_with_turns(1)),
        ];
        // 回合口径把"零回合会话"的 Claude 剔除 → 只有 (Claude,3),(Codebuddy,1);
        assert_eq!(
            agent_turn_share(&rows),
            vec![(AgentKind::Claude, 3), (AgentKind::Codebuddy, 1)]
        );
        // 会话数口径每会话记 1 → 空回合会话也要占一条。
        assert_eq!(
            agent_session_share(&rows),
            vec![(AgentKind::Claude, 2), (AgentKind::Codebuddy, 1)]
        );
    }

    #[test]
    fn agent_io_cache_shares_keep_io_and_cache_separate() {
        let usage = |io: u64, cache: u64| ConversationUsage {
            tokens_in: io,             // IO 读
            tokens_out: io,            // IO 写(同量便于断言合值)
            tokens_cache_read: cache,  // 缓存读
            tokens_cache_write: cache, // 缓存写
            ..Default::default()
        };
        let rows = vec![
            (meta(AgentKind::Claude, "a"), usage(10, 4)),
            (meta(AgentKind::Claude, "b"), usage(1, 1)),
            (meta(AgentKind::V8agent, "c"), usage(0, 3)),
        ];
        // IO = tokens_in+tokens_out 之和;缓存 = cache_read+cache_write 之和,
        // 两类分开计、不混在一个图里。
        assert_eq!(agent_io_token_share(&rows), vec![(AgentKind::Claude, 22)]);
        assert_eq!(
            agent_cache_token_share(&rows),
            vec![(AgentKind::Claude, 10), (AgentKind::V8agent, 6)]
        );
    }

    #[test]
    fn agents_present_returns_used_agents_in_fixed_order() {
        let rows = vec![
            (meta(AgentKind::Codebuddy, "a"), usage_with_tokens(1)),
            (meta(AgentKind::Claude, "b"), usage_with_tokens(1)),
            (meta(AgentKind::V8agent, "c"), usage_with_tokens(1)),
        ];
        assert_eq!(
            agents_present(&rows),
            vec![AgentKind::Claude, AgentKind::Codebuddy, AgentKind::V8agent]
        );
    }

    #[test]
    fn agents_present_ignores_agents_without_dedicated_bucket() {
        let rows = vec![(meta(AgentKind::Codex, "a"), usage_with_tokens(1))];
        assert_eq!(agents_present(&rows), Vec::new());
    }

    #[test]
    fn filter_rows_by_agent_none_keeps_everything() {
        let rows = vec![
            (meta(AgentKind::Claude, "a"), usage_with_tokens(1)),
            (meta(AgentKind::Codebuddy, "b"), usage_with_tokens(1)),
        ];
        assert_eq!(filter_rows_by_agent(&rows, None), rows);
    }

    #[test]
    fn filter_rows_by_agent_some_keeps_only_that_agent() {
        let rows = vec![
            (meta(AgentKind::Claude, "a"), usage_with_tokens(1)),
            (meta(AgentKind::Codebuddy, "b"), usage_with_tokens(2)),
            (meta(AgentKind::Claude, "c"), usage_with_tokens(3)),
        ];
        let filtered = filter_rows_by_agent(&rows, Some(AgentKind::Claude));
        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().all(|(m, _)| m.agent == AgentKind::Claude));
    }

    fn usage_with_turns(turns: u32) -> ConversationUsage {
        ConversationUsage {
            turns,
            ..Default::default()
        }
    }

    #[test]
    fn agent_turn_share_sums_human_turns_per_agent_and_skips_empty() {
        let rows = vec![
            (meta(AgentKind::Claude, "a"), usage_with_turns(3)),
            (meta(AgentKind::Claude, "b"), usage_with_turns(2)),
            (meta(AgentKind::Codebuddy, "c"), usage_with_turns(5)),
            // Opencode 有会话但零回合——不产生占位,也不进图例。
            (meta(AgentKind::Opencode, "d"), usage_with_turns(0)),
        ];
        let share = agent_turn_share(&rows);
        assert_eq!(
            share,
            vec![(AgentKind::Claude, 5), (AgentKind::Codebuddy, 5)]
        );
    }

    #[test]
    fn agent_turn_share_includes_v8agent() {
        let rows = vec![(meta(AgentKind::V8agent, "a"), usage_with_turns(4))];
        let share = agent_turn_share(&rows);
        assert_eq!(share, vec![(AgentKind::V8agent, 4)]);
    }

    #[test]
    fn loaded_clears_loading_and_stores_rows() {
        let mut ws_state = WorkspaceState {
            loading: true,
            ..WorkspaceState::default()
        };
        let rows = vec![(
            meta(AgentKind::Claude, "a"),
            ConversationUsage {
                turns: 3,
                ..Default::default()
            },
        )];
        update(&mut ws_state, Message::Loaded(1, rows.clone()));
        assert!(!ws_state.loading());
        assert_eq!(ws_state.rows(), rows.as_slice());
    }

    #[test]
    fn session_round_trend_zero_fills_full_window_only_target_agent() {
        // day 20672 = 2026-08-07;13 天前 = 20659,仍在同一 15 天窗口内。
        let today = 20_672i64;
        let ms = |day: i64| (day as u64) * 86_400_000;
        let rows = vec![
            (
                meta_at(AgentKind::Claude, ms(today)),
                ConversationUsage {
                    turns: 2,
                    ..Default::default()
                },
            ),
            (
                meta_at(AgentKind::Claude, ms(today - 13)),
                ConversationUsage {
                    turns: 4,
                    ..Default::default()
                },
            ),
            // 其他 agent 即便同在今天有会话也不该被某单 agent 趋势吃进去。
            (
                meta_at(AgentKind::Codebuddy, ms(today)),
                ConversationUsage {
                    turns: 9,
                    ..Default::default()
                },
            ),
        ];
        let trend = session_round_trend(&rows, AgentKind::Claude, today);
        // 窗口从 today 往回整 15 个日历天,哪怕没有数据也补占位,不允许空窗漂。
        assert_eq!(trend.len(), 15);
        assert_eq!(trend[0].day_index, today - 14, "最左应正好是窗口首日(0 值)");
        assert_eq!(trend[0].values, vec![0, 0]);
        // 13 天前那条:today-13 落在窗口内,序列 [1,4](会话一次、回合 4)。
        let earlier = &trend[(today - 13 - (today - 14)) as usize];
        assert_eq!(earlier.day_index, today - 13);
        assert_eq!(earlier.values, vec![1, 4]);
        // 今天(窗口右端):只算 Claude,CodeBuddy 不进。
        let last = trend.last().unwrap();
        assert_eq!(last.day_index, today);
        assert_eq!(last.values, vec![1, 2]);
    }

    #[test]
    fn io_and_cache_trend_share_window_but_split_fields() {
        let today = 20_672i64;
        let ms = |day: i64| (day as u64) * 86_400_000;
        // sample_usage:in=10,out=2,read=1,write=1。
        let rows = vec![(meta_at(AgentKind::Claude, ms(today)), sample_usage(&[]))];
        let io = io_trend(&rows, AgentKind::Claude, today);
        let cache = cache_trend(&rows, AgentKind::Claude, today);
        assert_eq!(io.len(), 15, "Input/Output 看近 15 天");
        assert_eq!(
            cache.len(),
            15,
            "Cache read/write 统一改成近 15 天,跟 Input/Output 同口径"
        );
        assert_eq!(io.last().unwrap().values, vec![10, 2]);
        assert_eq!(cache.last().unwrap().values, vec![1, 1]);
    }

    #[test]
    fn window_axis_is_self_contained_when_recent_days_idle() {
        // 数据停在 14 天前,今天(窗口右端)本身没有更新——窗口仍从 today 起铺
        // 整 15 天,最新若干格是 0,不只收在有数据那几天。
        let today = 20_672i64;
        let ms = |day: i64| (day as u64) * 86_400_000;
        let rows = vec![(
            meta_at(AgentKind::Claude, ms(today - 14)),
            ConversationUsage {
                turns: 1,
                ..Default::default()
            },
        )];
        let trend = session_round_trend(&rows, AgentKind::Claude, today);
        assert_eq!(trend.len(), 15);
        assert_eq!(trend[0].values, vec![1, 1], "最旧天数据落在窗口最左");
        assert_eq!(
            trend.last().unwrap().values,
            vec![0, 0],
            "今天无活动但保留占位"
        );
    }
}
