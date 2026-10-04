//! Agent 用量统计面板的数据层与视图层（spec
//! docs/superpowers/specs/2026-08-07-agent-usage-panel-design.md）。数据层是
//! 纯函数,不碰 iced/IO,镜像 `transcript.rs` 的按 agent 分派解析方式;
//! `dozerd`/`dozer-core::protocol` 完全不参与——所有数据直接读磁盘上的
//! agent transcript JSONL。

use crate::conversation::ConversationMeta;
use dozer_core::protocol::AgentKind;
use std::collections::BTreeMap;
use std::path::PathBuf;

mod aggregate;
mod protocol;
mod view;

pub(crate) use aggregate::*;
pub(crate) use protocol::*;
pub(crate) use view::*;

/// 挂在每个 Workspace 上的 Usage 面板状态,对应现有 `Workspace` 上
/// `usage`/`usage_loading` 两个字段。
#[derive(Default)]
pub struct WorkspaceState {
    rows: Vec<(ConversationMeta, ConversationUsage)>,
    loading: bool,
    /// 右侧 agent 筛选栏当前选中项:`None` = "全部agent"(默认,不过滤)。
    agent_filter: Option<AgentKind>,
    /// 项目仓库 HEAD 可达的 git 提交总数(2026-09-20 新增的"Git提交"格)。
    /// 与 agent 筛选无关——它是项目级统计,不随 `agent_filter` 收缩;非
    /// git 仓库记 0。
    git_commits: u64,
    /// HEAD 可达提交按 UTC 提交日的逐日计数,供"每日行为统计"折线图用。
    /// 键是 `day_index_from_ms` 同口径的 UTC 日索引;窗口外/非 git 仓库为空。
    git_commits_by_day: BTreeMap<i64, u64>,
}

impl WorkspaceState {
    pub fn rows(&self) -> &[(ConversationMeta, ConversationUsage)] {
        &self.rows
    }

    pub fn loading(&self) -> bool {
        self.loading
    }

    /// 项目 git 提交总数,见 [`WorkspaceState::git_commits`]。
    pub fn git_commits(&self) -> u64 {
        self.git_commits
    }

    /// 每日 git 提交计数(UTC 日索引 → 次数),见 [`WorkspaceState::git_commits_by_day`]。
    pub fn git_commits_by_day(&self) -> &BTreeMap<i64, u64> {
        &self.git_commits_by_day
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

/// App 级(不按项目分,同 `git_log::State` 先例)内容侧 webview 推送状态:
/// `ready`(JS `window.__dozer.dispatch` 已注册)+ `last_sent`(最近一次
/// 真正 `evaluate_script` 成功的 payload)。纯状态,可单测,不碰 webview 池
/// ——调用方(`App::take_usage_content_script`,Task 15)拿 `pending_push`
/// 的结果去组 envelope,只有真正 `evaluate_script` 成功才调 `mark_sent`。
#[derive(Default)]
pub struct WebviewPushState {
    ready: bool,
    last_sent: Option<UsageViewPayload>,
}

impl WebviewPushState {
    pub fn set_ready(&mut self, ready: bool) {
        self.ready = ready;
        if ready {
            // 新实例(或重新确认 ready)一律强制重发一次当前内容,同
            // `git_log::State::set_diff_webview_ready` 配
            // `clear_diff_sent_for` 的先例。
            self.last_sent = None;
        }
    }

    pub fn pending_push(&self, desired: &UsageViewPayload) -> Option<UsageViewPayload> {
        if !self.ready {
            return None;
        }
        if self.last_sent.as_ref() == Some(desired) {
            return None;
        }
        Some(desired.clone())
    }

    pub fn mark_sent(&mut self, payload: UsageViewPayload) {
        self.last_sent = Some(payload);
    }
}

/// 对应现在顶层 `Message` 里的 `UsageLoaded` 变体,去前缀原样搬来。
/// `Refresh`/`Hover` 随手动刷新按钮一起移除——进入面板时由
/// `Workspace::spawn_usage_refresh` 自动刷新,不再需要面板内按钮。
#[derive(Debug, Clone)]
pub enum Message {
    /// 第三字段是项目 git 提交总数(供"Git提交"格),第四字段是每日提交计数
    /// (供"每日行为统计"折线图)——两者都与 transcript 用量无关,顺手在同一个
    /// 刷新任务里算掉,避免再加一条异步链路。
    Loaded(
        i64,
        Vec<(ConversationMeta, ConversationUsage)>,
        u64,
        BTreeMap<i64, u64>,
    ),
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
        Message::Loaded(_, rows, git_commits, git_commits_by_day) => {
            ws_state.rows = rows;
            ws_state.git_commits = git_commits;
            ws_state.git_commits_by_day = git_commits_by_day;
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
        // git 提交数(总数 + 每日分桶)是纯本地仓库遍历,丢进 spawn_blocking
        // 不占异步 worker。
        let (git_commits, git_commits_by_day) = tokio::task::spawn_blocking(move || {
            let total = count_git_commits(&project_path);
            let by_day = count_git_commits_by_day(&project_path);
            (total, by_day)
        })
        .await
        .unwrap_or((0, BTreeMap::new()));
        emit(Message::Loaded(
            project_id,
            rows,
            git_commits,
            git_commits_by_day,
        ));
    });
}

/// 面板切入:先置"统计中",有项目就发起一次用量统计(没有项目时只置 loading——迁移前的行为)。
pub fn on_activate(
    state: &mut WorkspaceState,
    ctx: Option<&crate::panel_host::ActivationCtx<Message>>,
) {
    state.set_loading(true);
    let Some(ctx) = ctx else {
        return;
    };
    let Some(path) = ctx.project_path.clone() else {
        return;
    };
    spawn_refresh(
        ctx.project_id,
        path,
        ctx.io.client(),
        ctx.io.handle(),
        ctx.io.emitter(),
    );
}

/// 统计项目仓库 HEAD 可达的提交总数(当前分支口径,同 git_log 面板的
/// revwalk 起点)。项目不是 git 仓库、空仓库或任何 bytegit 报错都一律记 0
/// ——这格统计不值得让整个面板失败。
fn count_git_commits(path: &std::path::Path) -> u64 {
    // `discover` 兼容项目路径是仓库子目录的情况;`open` 只认仓库根。
    let Ok(repo) = bytegit::Repo::discover(path) else {
        return 0;
    };
    repo.commit_count().unwrap_or(0)
}

/// 统计 HEAD 可达提交按 UTC 提交日的逐日计数,供"每日行为统计"折线图用。
/// 与 `day_index_from_ms` 同口径:UTC 日索引 = `提交时间秒 / 86_400`(注意这里
/// 整段除以 86400,跟毫秒口径 `ms / 86_400_000` 等价)。项目不是 git 仓库/
/// 任何 bytegit 报错都返回空 map;不会因提交多而爆炸——返回的只是"有提交的那
/// 些天"的计数,不是每一条提交。
fn count_git_commits_by_day(path: &std::path::Path) -> BTreeMap<i64, u64> {
    let Ok(repo) = bytegit::Repo::discover(path) else {
        return BTreeMap::new();
    };
    repo.commit_count_by_day().unwrap_or_default()
}

/// 面板内容侧:顶部"用量"标题 + 加载态动画占位。四态图表内容已迁到
/// `dozer://usage-content` Preact webview(见 `protocol.rs`)。`list_pane`
/// (agent 筛选栏)仍原生 iced,接入跟 Database/Agent 面板一样的可拖拽
/// split + 收起机制(见 app.rs `PanelKind::Usage` 分支、`Divider::UsageSplit`)。
#[cfg(test)]
mod tests {
    use super::*;

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
        // Unknown 不产出可统计的用量数据,跟任何没被 AGENT_ORDER 收录的
        // agent 一样被忽略,不能 panic,也不产生任何一天的记录(没有任何
        // 可展示的 agent,图表应该整体不渲染)。
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

    /// 占比不足 1% 的 agent 不参与统计(2026-09-24 用户要求):饼图/图例里
    /// 一圈几乎看不见的色块、一行 `0%` 没有意义,直接从份额口径里剔除。
    /// 恰好等于 1%(`total*100 >= grand_total`)的仍保留,边界取闭。
    #[test]
    fn agent_metric_share_drops_agents_under_one_percent() {
        let rows = vec![
            (meta(AgentKind::Claude, "a"), usage_with_tokens(1000)),
            // 1/1000 = 0.1%,剔除。
            (meta(AgentKind::Codebuddy, "b"), usage_with_tokens(1)),
        ];
        assert_eq!(agent_token_share(&rows), vec![(AgentKind::Claude, 1000)]);

        // 恰好 1%（10/1000）保留。
        let boundary = vec![
            (meta(AgentKind::Claude, "a"), usage_with_tokens(990)),
            (meta(AgentKind::Codebuddy, "b"), usage_with_tokens(10)),
        ];
        assert_eq!(
            agent_token_share(&boundary),
            vec![(AgentKind::Claude, 990), (AgentKind::Codebuddy, 10)]
        );
    }

    /// V8agent(用户自研 agent)默认要被用量统计覆盖,不能像 Unknown 那样
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
        assert_eq!(
            last.values,
            vec![2, 5],
            "today 触达 a.rs/c.rs 去重后 2 个文件,Git 5 次"
        );
        let two_ago = &series[series.len() - 3];
        assert_eq!(two_ago.day_index, day);
        assert_eq!(
            two_ago.values,
            vec![2, 3],
            "day 触达 a.rs/b.rs 去重后 2 个文件,Git 3 次"
        );
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
        update(
            &mut ws_state,
            Message::Loaded(1, rows.clone(), 42, Default::default()),
        );
        assert!(!ws_state.loading());
        assert_eq!(ws_state.rows(), rows.as_slice());
        assert_eq!(ws_state.git_commits(), 42);
        assert!(ws_state.git_commits_by_day().is_empty());
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

    fn payload_a() -> UsageViewPayload {
        UsageViewPayload::AgentEmpty {
            agent: AgentKind::Claude,
        }
    }

    fn payload_b() -> UsageViewPayload {
        UsageViewPayload::AgentEmpty {
            agent: AgentKind::Codebuddy,
        }
    }

    #[test]
    fn pending_push_none_when_not_ready() {
        let state = WebviewPushState::default();
        assert!(state.pending_push(&payload_a()).is_none());
    }

    #[test]
    fn pending_push_some_when_ready_and_never_sent() {
        let mut state = WebviewPushState::default();
        state.set_ready(true);
        assert_eq!(state.pending_push(&payload_a()), Some(payload_a()));
    }

    #[test]
    fn pending_push_none_once_marked_sent_and_desired_unchanged() {
        let mut state = WebviewPushState::default();
        state.set_ready(true);
        state.mark_sent(payload_a());
        assert!(state.pending_push(&payload_a()).is_none());
    }

    /// 对应本计划 Review Focus"多 agent 筛选切换的最新覆盖旧语义":desired
    /// 在 A→B→A 之间反复横跳,每次跟上一次真正送达的不一样都要判定为待推,
    /// 不能因为"A 以前发过一次"就误判成不用重发。
    #[test]
    fn pending_push_resends_when_desired_flips_back_to_a_previously_sent_value() {
        let mut state = WebviewPushState::default();
        state.set_ready(true);
        state.mark_sent(payload_a());
        assert_eq!(state.pending_push(&payload_b()), Some(payload_b()));
        state.mark_sent(payload_b());
        assert_eq!(
            state.pending_push(&payload_a()),
            Some(payload_a()),
            "desired 变回 A(即便 A 是更早发过的值)也必须判定为待推"
        );
    }

    /// 对应本计划 Review Focus"切换项目后 webview 显示旧项目数据":这里用
    /// "desired 突然换成另一个 workspace 算出来的、从未出现过的 payload"模拟
    /// 项目切换——`last_sent` 是 App 级单槽,不按项目分,天然不会因为
    /// "这个项目以前没发过"而漏推。
    #[test]
    fn pending_push_treats_a_different_workspaces_payload_as_new() {
        let mut state = WebviewPushState::default();
        state.set_ready(true);
        state.mark_sent(payload_a()); // 上一个显示这个 webview 的项目留下的内容
        let other_project_payload = UsageViewPayload::Empty; // 切到的新项目,当前态
        assert_eq!(
            state.pending_push(&other_project_payload),
            Some(UsageViewPayload::Empty)
        );
    }

    #[test]
    fn set_ready_true_clears_last_sent_forcing_a_resend() {
        let mut state = WebviewPushState::default();
        state.set_ready(true);
        state.mark_sent(payload_a());
        assert!(state.pending_push(&payload_a()).is_none());
        // webview 被销毁重建(全新实例),重新 ready——即便 desired 没变,也要
        // 强制重发一次,因为新实例的 JS 端状态是空的(同 git_log 的先例)。
        state.set_ready(false);
        state.set_ready(true);
        assert_eq!(state.pending_push(&payload_a()), Some(payload_a()));
    }

    // ---- bytegit P3:git 提交计数迁移前后必须一致的口径 ----

    const DAY_20000: i64 = 20_000 * 86_400;

    /// 在 `repo` 里跑一条 git 命令。作者日期固定为 2000-01-01(证明统计用的是**提交者**时间,
    /// 不是作者时间);`when` 是提交者日期(unix 秒),`None` 用当前时间。
    fn git_at(repo: &std::path::Path, args: &[&str], when: Option<i64>) {
        let mut cmd = std::process::Command::new("git");
        cmd.args(args)
            .current_dir(repo)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .env("GIT_AUTHOR_DATE", "946684800 +0000");
        if let Some(w) = when {
            cmd.env("GIT_COMMITTER_DATE", format!("{w} +0000"));
        }
        let out = cmd.output().unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
    }

    fn commit_at(repo: &std::path::Path, rel: &str, content: &str, when: Option<i64>) {
        let full = repo.join(rel);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(full, content).unwrap();
        git_at(repo, &["add", "."], None);
        git_at(repo, &["commit", "-qm", "c"], when);
    }

    fn init_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        git_at(dir.path(), &["init", "-q"], None);
        dir
    }

    #[test]
    fn git_commit_counts_are_zero_and_empty_outside_a_repo() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(count_git_commits(dir.path()), 0);
        assert!(count_git_commits_by_day(dir.path()).is_empty());
    }

    #[test]
    fn git_commit_counts_are_zero_and_empty_for_a_repo_without_commits() {
        let dir = init_repo();
        assert_eq!(count_git_commits(dir.path()), 0);
        assert!(count_git_commits_by_day(dir.path()).is_empty());
    }

    #[test]
    fn count_git_commits_counts_commits_reachable_from_head() {
        let dir = init_repo();
        for i in 0..3 {
            commit_at(dir.path(), "a.txt", &i.to_string(), None);
        }
        assert_eq!(count_git_commits(dir.path()), 3);
    }

    #[test]
    fn git_commit_counts_look_upward_from_a_repo_subdirectory() {
        let dir = init_repo();
        commit_at(dir.path(), "sub/a.txt", "1", Some(DAY_20000));
        commit_at(dir.path(), "sub/a.txt", "2", Some(DAY_20000 + 1));
        let sub = dir.path().join("sub");
        assert_eq!(count_git_commits(&sub), 2);
        assert_eq!(
            count_git_commits_by_day(&sub),
            BTreeMap::from([(20_000, 2)])
        );
    }

    #[test]
    fn count_git_commits_counts_a_merged_commit_once() {
        let dir = init_repo();
        let r = dir.path();
        commit_at(r, "a.txt", "a", None);
        git_at(r, &["checkout", "-q", "-b", "other"], None);
        commit_at(r, "b.txt", "b", None);
        git_at(r, &["checkout", "-q", "-"], None);
        commit_at(r, "c.txt", "c", None);
        git_at(r, &["merge", "-q", "--no-ff", "other", "-m", "merge"], None);
        // base + other + main + merge
        assert_eq!(count_git_commits(r), 4);
    }

    #[test]
    fn git_commit_counts_work_on_a_detached_head() {
        let dir = init_repo();
        commit_at(dir.path(), "a.txt", "1", None);
        commit_at(dir.path(), "a.txt", "2", None);
        git_at(dir.path(), &["checkout", "-q", "--detach"], None);
        assert_eq!(count_git_commits(dir.path()), 2);
        assert_eq!(
            count_git_commits_by_day(dir.path()).values().sum::<u64>(),
            2
        );
    }

    #[test]
    fn git_commits_by_day_buckets_by_utc_committer_day_not_author_day() {
        let dir = init_repo();
        commit_at(dir.path(), "a.txt", "1", Some(DAY_20000));
        commit_at(dir.path(), "a.txt", "2", Some(DAY_20000 + 86_399));
        commit_at(dir.path(), "a.txt", "3", Some(DAY_20000 + 86_400));
        // 作者日期都是 2000-01-01,桶却按提交者日期落在 20000/20001 天。
        assert_eq!(
            count_git_commits_by_day(dir.path()),
            BTreeMap::from([(20_000, 2), (20_001, 1)])
        );
    }

    // ---- H6:切入钩子 ----

    use crate::panel_host::testing::{TIMEOUT, offline_ctx, runtime};

    #[test]
    fn on_activate_marks_loading_and_requests_a_refresh_for_that_project() {
        let rt = runtime();
        let (ctx, rx) = offline_ctx::<Message>(&rt, 7, Some(std::env::temp_dir()));
        let mut state = WorkspaceState::default();
        on_activate(&mut state, Some(&ctx));
        assert!(state.loading());
        assert!(matches!(
            rx.recv_timeout(TIMEOUT).unwrap(),
            Message::Loaded(7, ..)
        ));
    }

    #[test]
    fn on_activate_without_a_project_still_marks_loading_but_requests_nothing() {
        let mut state = WorkspaceState::default();
        on_activate(&mut state, None);
        assert!(state.loading(), "迁移前的行为:先置 loading,再看有没有项目");
    }
}
