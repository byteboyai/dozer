//! 从 `dozer-app` 原 `extensions/usage/mod.rs` 迁移过来的行为测试。除"经
//! 注入接缝"外,断言口径与迁移前完全一致(计划 §Task 19 验证要求:Usage 旧测试
//! 全部迁入库并通过)。

use crate::adapter::{GitStatsProvider, NoGitStats};
use crate::aggregate::*;
use crate::state::{Message, WorkspaceState, on_activate};
use dozer_model::{AgentKind, ConversationMeta};
use std::collections::{BTreeMap, BTreeSet};

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
    let payload = dozer_model::UsagePayload {
        turns: 2,
        tool_calls: 1,
        mutating_tool_calls: 1,
        files_touched: BTreeSet::from(["README.md".to_string()]),
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
        BTreeSet::from(["README.md".to_string()])
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
    assert_eq!(civil_from_days(0), (1970, 1, 1));
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

fn usage_with_turns(turns: u32) -> ConversationUsage {
    ConversationUsage {
        turns,
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
        ),
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
    let rows = vec![(meta_at(AgentKind::Unknown, 0), usage_with_tokens(99))];
    let days = daily_totals_by_agent(&rows);
    assert!(days.is_empty(), "没有任何已知 agent 有数据时不该产出天记录");
}

#[test]
fn daily_totals_by_agent_includes_codex() {
    let rows = vec![(meta_at(AgentKind::Codex, 0), usage_with_tokens(3))];
    let days = daily_totals_by_agent(&rows);
    assert_eq!(days.len(), 1);
    assert_eq!(total_for(&days[0], AgentKind::Codex), 3);
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

#[test]
fn agent_metric_share_drops_agents_under_one_percent() {
    let rows = vec![
        (meta(AgentKind::Claude, "a"), usage_with_tokens(1000)),
        (meta(AgentKind::Codebuddy, "b"), usage_with_tokens(1)),
    ];
    assert_eq!(agent_token_share(&rows), vec![(AgentKind::Claude, 1000)]);

    let boundary = vec![
        (meta(AgentKind::Claude, "a"), usage_with_tokens(990)),
        (meta(AgentKind::Codebuddy, "b"), usage_with_tokens(10)),
    ];
    assert_eq!(
        agent_token_share(&boundary),
        vec![(AgentKind::Claude, 990), (AgentKind::Codebuddy, 10)]
    );
}

#[test]
fn agent_token_share_includes_v8agent() {
    let rows = vec![(meta(AgentKind::V8agent, "a"), usage_with_tokens(7))];
    let share = agent_token_share(&rows);
    assert_eq!(share, vec![(AgentKind::V8agent, 7)]);
}

#[test]
fn agent_session_share_counts_one_per_conversation_including_zero_turn() {
    let rows = vec![
        (meta(AgentKind::Claude, "a"), usage_with_turns(3)),
        (meta(AgentKind::Claude, "b"), usage_with_turns(0)),
        (meta(AgentKind::Codebuddy, "c"), usage_with_turns(1)),
    ];
    assert_eq!(
        agent_turn_share(&rows),
        vec![(AgentKind::Claude, 3), (AgentKind::Codebuddy, 1)]
    );
    assert_eq!(
        agent_session_share(&rows),
        vec![(AgentKind::Claude, 2), (AgentKind::Codebuddy, 1)]
    );
}

#[test]
fn agent_io_cache_shares_keep_io_and_cache_separate() {
    let usage = |io: u64, cache: u64| ConversationUsage {
        tokens_in: io,
        tokens_out: io,
        tokens_cache_read: cache,
        tokens_cache_write: cache,
        ..Default::default()
    };
    let rows = vec![
        (meta(AgentKind::Claude, "a"), usage(10, 4)),
        (meta(AgentKind::Claude, "b"), usage(1, 1)),
        (meta(AgentKind::V8agent, "c"), usage(0, 3)),
    ];
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
    let rows = vec![(meta(AgentKind::Unknown, "a"), usage_with_tokens(1))];
    assert_eq!(agents_present(&rows), Vec::new());
}

#[test]
fn agents_present_includes_codex() {
    let rows = vec![(meta(AgentKind::Codex, "a"), usage_with_tokens(1))];
    assert_eq!(agents_present(&rows), vec![AgentKind::Codex]);
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

#[test]
fn agent_turn_share_sums_human_turns_per_agent_and_skips_empty() {
    let rows = vec![
        (meta(AgentKind::Claude, "a"), usage_with_turns(3)),
        (meta(AgentKind::Claude, "b"), usage_with_turns(2)),
        (meta(AgentKind::Codebuddy, "c"), usage_with_turns(5)),
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
fn behavior_series_counts_files_touched_and_git_commits_per_day() {
    let today = 20_672i64;
    let day = today - 2;
    let rows = vec![
        (
            meta_at(AgentKind::Claude, day as u64 * 86_400_000),
            ConversationUsage {
                files_touched: ["a.rs", "b.rs"].iter().map(|s| s.to_string()).collect(),
                ..Default::default()
            },
        ),
        (
            meta_at(AgentKind::Claude, today as u64 * 86_400_000),
            ConversationUsage {
                files_touched: ["a.rs", "c.rs"].iter().map(|s| s.to_string()).collect(),
                ..Default::default()
            },
        ),
    ];
    let mut git = BTreeMap::new();
    git.insert(day, 3);
    git.insert(today, 5);
    let series = behavior_series(&rows, &git, today);
    assert_eq!(series.len(), 7, "连续 7 天窗口");
    let last = series.last().unwrap();
    assert_eq!(last.day_index, today);
    assert_eq!(last.values, vec![2, 5]);
    let two_ago = &series[series.len() - 3];
    assert_eq!(two_ago.day_index, day);
    assert_eq!(two_ago.values, vec![2, 3]);
}

#[test]
fn session_round_trend_zero_fills_full_window_only_target_agent() {
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
        (
            meta_at(AgentKind::Codebuddy, ms(today)),
            ConversationUsage {
                turns: 9,
                ..Default::default()
            },
        ),
    ];
    let trend = session_round_trend(&rows, AgentKind::Claude, today);
    assert_eq!(trend.len(), 15);
    assert_eq!(trend[0].day_index, today - 14, "最左应正好是窗口首日(0 值)");
    assert_eq!(trend[0].values, vec![0, 0]);
    let earlier = &trend[(today - 13 - (today - 14)) as usize];
    assert_eq!(earlier.day_index, today - 13);
    assert_eq!(earlier.values, vec![1, 4]);
    let last = trend.last().unwrap();
    assert_eq!(last.day_index, today);
    assert_eq!(last.values, vec![1, 2]);
}

#[test]
fn io_and_cache_trend_share_window_but_split_fields() {
    let today = 20_672i64;
    let ms = |day: i64| (day as u64) * 86_400_000;
    let rows = vec![(meta_at(AgentKind::Claude, ms(today)), sample_usage(&[]))];
    let io = io_trend(&rows, AgentKind::Claude, today);
    let cache = cache_trend(&rows, AgentKind::Claude, today);
    assert_eq!(io.len(), 15, "Input/Output 看近 15 天");
    assert_eq!(cache.len(), 15, "Cache read/write 统一改成近 15 天");
    assert_eq!(io.last().unwrap().values, vec![10, 2]);
    assert_eq!(cache.last().unwrap().values, vec![1, 1]);
}

#[test]
fn window_axis_is_self_contained_when_recent_days_idle() {
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
    assert_eq!(trend.last().unwrap().values, vec![0, 0]);
}

// ---- Surface 去重/重建语义(迁移自原 mod.rs) ----

fn stamp() -> dozer_model::LocaleStamp {
    dozer_model::LocaleStamp {
        ui_locale: dozer_model::Locale::default_locale(),
        locale_revision: 0,
    }
}

fn payload_a() -> dozer_model::Stamped<crate::protocol::UsageViewPayload> {
    dozer_model::Stamped::new(
        stamp(),
        crate::protocol::UsageViewPayload::AgentEmpty {
            agent: AgentKind::Claude,
        },
    )
}

fn payload_b() -> dozer_model::Stamped<crate::protocol::UsageViewPayload> {
    dozer_model::Stamped::new(
        stamp(),
        crate::protocol::UsageViewPayload::AgentEmpty {
            agent: AgentKind::Codebuddy,
        },
    )
}

fn new_surface() -> crate::surface::UsageSurface {
    crate::surface::UsageSurface::new(7, |s| format!("dispatch({s})"))
}

#[test]
fn pending_push_none_when_not_ready() {
    let mut surface = new_surface();
    surface.observe_instance(true, false);
    surface.set_desired(payload_a());
    assert!(surface.take_pending_script().is_none());
}

#[test]
fn pending_push_some_when_ready_and_never_sent() {
    let mut surface = new_surface();
    surface.observe_instance(true, false);
    surface.apply_ready();
    surface.set_desired(payload_a());
    let (slot, script) = surface.take_pending_script().expect("ready 首推");
    assert_eq!(slot, 7);
    assert!(script.contains("agent_empty"));
}

#[test]
fn pending_push_none_once_marked_sent_and_desired_unchanged() {
    let mut surface = new_surface();
    surface.observe_instance(true, false);
    surface.apply_ready();
    surface.set_desired(payload_a());
    assert!(surface.take_pending_script().is_some());
    surface.set_desired(payload_a());
    assert!(surface.take_pending_script().is_none());
}

#[test]
fn pending_push_resends_when_desired_flips_back_to_a_previously_sent_value() {
    let mut surface = new_surface();
    surface.observe_instance(true, false);
    surface.apply_ready();
    surface.set_desired(payload_a());
    assert!(surface.take_pending_script().is_some());
    surface.set_desired(payload_b());
    assert!(surface.take_pending_script().is_some());
    surface.set_desired(payload_a());
    assert!(surface.take_pending_script().is_some());
}

#[test]
fn pending_push_treats_a_different_workspaces_payload_as_new() {
    let mut surface = new_surface();
    surface.observe_instance(true, false);
    surface.apply_ready();
    surface.set_desired(payload_a());
    assert!(surface.take_pending_script().is_some());
    surface.set_desired(dozer_model::Stamped::new(
        stamp(),
        crate::protocol::UsageViewPayload::Empty,
    ));
    assert!(surface.take_pending_script().is_some());
}

/// spec §8:业务 payload 不变、仅语言上下文变化时也必须推送。
#[test]
fn pending_push_resends_when_only_locale_changes() {
    let mut surface = new_surface();
    surface.observe_instance(true, false);
    surface.apply_ready();
    let payload = payload_a();
    surface.set_desired(payload.clone());
    assert!(surface.take_pending_script().is_some());
    surface.set_desired(payload.clone());
    assert!(surface.take_pending_script().is_none(), "完全同值不重推");
    let switched = dozer_model::Stamped::new(
        dozer_model::LocaleStamp {
            ui_locale: dozer_model::Locale(dozer_model::Locale::ZH_CN.to_string()),
            locale_revision: 1,
        },
        payload.payload,
    );
    surface.set_desired(switched);
    assert!(surface.take_pending_script().is_some(), "仅语言变也要重推");
}

#[test]
fn rebuild_forces_a_resend_even_when_desired_unchanged() {
    let mut surface = new_surface();
    surface.observe_instance(true, false);
    surface.apply_ready();
    surface.set_desired(payload_a());
    assert!(surface.take_pending_script().is_some());
    surface.observe_instance(false, false);
    surface.observe_instance(true, false);
    surface.apply_ready();
    surface.set_desired(payload_a());
    assert!(
        surface.take_pending_script().is_some(),
        "重建后新实例必须重发"
    );
}

#[test]
fn stale_generation_ipc_is_rejected_after_rebuild() {
    let mut surface = new_surface();
    surface.observe_instance(true, false);
    surface.apply_ready();
    let old = surface.id();
    surface.observe_instance(false, false);
    surface.observe_instance(true, false);
    assert!(!surface.route_of(old).is_accepted(), "旧代数归属被拒");
    assert!(surface.route_of(surface.id()).is_accepted(), "当前代数接受");
}

#[test]
fn same_frame_destroy_and_recreate_opens_a_new_generation() {
    let mut surface = new_surface();
    surface.observe_instance(true, false);
    surface.apply_ready();
    surface.set_desired(payload_a());
    assert!(surface.take_pending_script().is_some(), "旧实例已推快照");
    let old = surface.id();
    surface.observe_instance(true, true);
    assert!(
        !surface.route_of(old).is_accepted(),
        "旧实例的 ready/IPC 必须被拒"
    );
    assert_ne!(surface.id(), old, "原生实例替换必须换代数");
    assert!(
        surface.take_pending_script().is_none(),
        "新实例尚未握手,不得推"
    );
    surface.apply_ready();
    surface.set_desired(payload_a());
    assert!(surface.take_pending_script().is_some());
}

// ---- H6:切入钩子(注入接缝版) ----

use crate::activation::testing::{FakeUsage, TIMEOUT, offline_ctx, runtime, with_usage};

#[test]
fn on_activate_marks_loading_and_requests_a_refresh_for_that_project() {
    let rt = runtime();
    let (ctx, rx) = offline_ctx::<Message>(&rt, 7, Some(std::env::temp_dir()));
    // 激活测试关心"发出了刷新",不关心 dozerd:换成进程内假实现。
    let ctx = with_usage(ctx, std::sync::Arc::new(FakeUsage::ok(vec![])));
    let mut state = WorkspaceState::default();
    on_activate(&mut state, Some(&ctx));
    assert!(state.loading());
    assert!(matches!(
        rx.recv_timeout(TIMEOUT).unwrap(),
        Message::Loaded(7, ..)
    ));
}

#[test]
fn each_activation_issues_a_newer_generation() {
    let rt = runtime();
    let (ctx, rx) = offline_ctx::<Message>(&rt, 7, Some(std::env::temp_dir()));
    let ctx = with_usage(ctx, std::sync::Arc::new(FakeUsage::ok(vec![])));
    let mut state = WorkspaceState::default();
    on_activate(&mut state, Some(&ctx));
    on_activate(&mut state, Some(&ctx));
    let mut gens = Vec::new();
    for _ in 0..2 {
        match rx.recv_timeout(TIMEOUT).unwrap() {
            Message::Loaded(7, generation, ..) => gens.push(generation),
            other => panic!("意外消息: {other:?}"),
        }
    }
    gens.sort_unstable();
    assert_eq!(gens, vec![1, 2]);
}

#[test]
fn on_activate_without_a_project_still_marks_loading_but_requests_nothing() {
    let mut state = WorkspaceState::default();
    on_activate(&mut state, None);
    assert!(state.loading(), "迁移前的行为:先置 loading,再看有没有项目");
}

// ---- Git 统计窄接口(用假 provider,证明库不再直连 bytegit) ----

struct FixedGit {
    total: u64,
    by_day: BTreeMap<i64, u64>,
}

impl crate::adapter::GitStatsProvider for FixedGit {
    fn commit_count(&self, _p: &std::path::Path) -> u64 {
        self.total
    }
    fn commits_by_day(&self, _p: &std::path::Path) -> BTreeMap<i64, u64> {
        self.by_day.clone()
    }
}

#[test]
fn spawn_refresh_surfaces_git_stats_from_the_injected_provider() {
    use crate::state::spawn_refresh;
    let rt = runtime();
    let (tx, rx) = std::sync::mpsc::channel();
    let git = FixedGit {
        total: 42,
        by_day: BTreeMap::from([(20_000, 3)]),
    };
    spawn_refresh(
        1,
        0,
        std::env::temp_dir(),
        std::sync::Arc::new(FakeUsage::ok(vec![])),
        std::sync::Arc::new(git),
        rt.handle(),
        move |m| {
            let _ = tx.send(m);
        },
    );
    match rx.recv_timeout(TIMEOUT).unwrap() {
        Message::Loaded(1, 0, rows, commits, by_day) => {
            assert!(rows.is_empty());
            assert_eq!(commits, 42);
            assert_eq!(by_day, BTreeMap::from([(20_000, 3)]));
        }
        other => panic!("意外消息: {other:?}"),
    }
}

#[test]
fn no_git_stats_is_the_neutral_default() {
    let g = NoGitStats;
    assert_eq!(g.commit_count(std::path::Path::new("/does/not/exist")), 0);
    assert!(
        g.commits_by_day(std::path::Path::new("/does/not/exist"))
            .is_empty()
    );
}
