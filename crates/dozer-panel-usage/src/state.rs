//! Usage 面板的数据层:WorkspaceState(每项目一份)、Message、update、
//! spawn_refresh、on_activate。镜像迁移前 `extensions/usage/mod.rs` 的行为。
//!
//! 与产品的两处解耦(计划 §Task 19):
//! - 一次性提示:不再用产品的 `toast::Outbox`(带 `dozer_core::log::Scope`),
//!   改成本库的 [`UsageOutbox`](见 `adapter.rs`);产品排空后按自己的方式呈现。
//! - Git 统计:不再在库内直连 `bytegit::Repo`,改为 [`GitStatsProvider`] 注入。

use crate::activation::ActivationCtx;
use crate::adapter::{GitStatsProvider, NoticeLevel, UsageOutbox};
use crate::aggregate::ConversationUsage;
use dozer_model::AgentKind;
use dozer_model::ConversationMeta;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

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
    /// 已发出的最新一次刷新的代数(每次 `begin_refresh` 加一)。`Loaded` 带着发起时的代数回来,
    /// 代数比这个小的就是被更新的刷新取代了的过期结果,`update` 直接丢弃——同一项目快速切入两次会
    /// 并发两次刷新,较慢的旧结果后到时不能覆盖新结果(host 的 `with_project` 只按项目路由,不去重)。
    refresh_generation: u64,
    /// 取数失败等一次性提示;产品排空后转成自己的 Toast。
    outbox: UsageOutbox,
}

impl WorkspaceState {
    /// 排空待发提示(产品排空后转成自己的 Toast;失败/成功一次性事件)。
    pub fn take_outbox(&mut self) -> Vec<crate::adapter::UsageNotice> {
        self.outbox.take()
    }

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

    /// 当前选中的 agent 筛选(`None` = "全部agent")。
    pub fn agent_filter(&self) -> Option<AgentKind> {
        self.agent_filter
    }

    /// 供内核 `PanelSelect(PanelKind::Usage)` 分支调用——切到面板时
    /// 立即标记"统计中",不等 `spawn_refresh` 的异步结果落地才置真。
    pub fn set_loading(&mut self, loading: bool) {
        self.loading = loading;
    }

    /// 发起一次刷新:置"统计中"并返回这次刷新的代数,调用方把它带进 `spawn_refresh`。
    pub fn begin_refresh(&mut self) -> u64 {
        self.loading = true;
        self.refresh_generation += 1;
        self.refresh_generation
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
/// `usage::on_activate` 自动刷新,不再需要面板内按钮。
#[derive(Debug, Clone)]
pub enum Message {
    /// 第二字段是发起这次刷新时的代数(见 `WorkspaceState::refresh_generation`)。
    /// 第四字段是项目 git 提交总数(供"Git提交"格),第五字段是每日提交计数
    /// (供"每日行为统计"折线图)——两者都与 transcript 用量无关,顺手在同一个
    /// 刷新任务里算掉,避免再加一条异步链路。
    Loaded(
        i64,
        u64,
        Vec<(ConversationMeta, ConversationUsage)>,
        u64,
        BTreeMap<i64, u64>,
    ),
    /// 取数失败(G4):与 `Loaded` 区分开,不再把失败当成空数据。第三字段是失败文本;
    /// 第四、五字段仍带 git 数据——它是本地仓库遍历,不依赖 dozerd。
    LoadFailed(i64, u64, String, u64, BTreeMap<i64, u64>),
    /// 右侧 agent 筛选栏点击(2026-08-28):`None` 选"全部agent"。
    AgentFilterSet(Option<AgentKind>),
    /// 内容侧"收起/展开列表列"按钮:产品内核拦截,不进 `update`——转发成顶层
    /// `Message::TogglePanelListCollapse(PanelKind::Usage)`(见 app.rs)。
    ToggleListCollapse,
    /// 任意顶部 `HoverId` 的悬停进入/离开(收起按钮等)。产品内核拦截转发给
    /// 顶层 `App::set_hover`,本面板 `update` 保 no-op 分支维持 match 穷尽。
    /// 库内不含产品的 `HoverId` 类型,载荷用 `bool` 表示"哪一类悬停"由产品
    /// 侧闭包绑定——这里只承载"进入/离开"这一个布尔,避免引入宿主类型。
    CollapseHover(bool),
}

pub fn update(ws_state: &mut WorkspaceState, msg: Message) {
    match msg {
        Message::Loaded(_, generation, rows, git_commits, git_commits_by_day) => {
            if generation < ws_state.refresh_generation {
                // 被更新的刷新取代了:丢弃,且不动 `loading`(新的还在路上)。
                return;
            }
            ws_state.rows = rows;
            ws_state.git_commits = git_commits;
            ws_state.git_commits_by_day = git_commits_by_day;
            ws_state.loading = false;
        }
        Message::LoadFailed(_, generation, message, git_commits, git_commits_by_day) => {
            if generation < ws_state.refresh_generation {
                return;
            }
            ws_state.git_commits = git_commits;
            ws_state.git_commits_by_day = git_commits_by_day;
            ws_state.loading = false;
            ws_state.outbox.push(
                NoticeLevel::Error,
                format!("用量统计失败: {message}"),
                "usage-load",
            );
        }
        Message::AgentFilterSet(agent) => {
            ws_state.agent_filter = agent;
        }
        Message::ToggleListCollapse => {
            unreachable!("由内核拦截处理,见 usage::Message::ToggleListCollapse 文档")
        }
        Message::CollapseHover(_) => {
            unreachable!("由内核拦截处理,见 usage::Message::CollapseHover 文档")
        }
    }
}

/// 异步扫描项目全部 agent transcript 并逐个解析用量。内核在
/// `PanelSelect(PanelKind::Usage)` 分支(切到面板时自动刷新)调用,
/// 经 `usage::on_activate` 转发。逻辑不变(读失败的会话
/// 整条跳过、不计入汇总)。
pub fn spawn_refresh(
    project_id: i64,
    generation: u64,
    project_path: PathBuf,
    service: Arc<dyn dozer_services::UsageService>,
    git: Arc<dyn GitStatsProvider>,
    handle: &tokio::runtime::Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    handle.spawn(async move {
        let cwd = project_path.to_string_lossy().into_owned();
        let result = service.usage_summary(cwd, None).await;
        // git 提交数(总数 + 每日分桶)是纯本地仓库遍历,与 dozerd 成败无关,两条路径都要算;
        // 丢进 spawn_blocking 不占异步 worker。
        let git_path = project_path.clone();
        let (git_commits, git_commits_by_day) = tokio::task::spawn_blocking(move || {
            let total = git.commit_count(&git_path);
            let by_day = git.commits_by_day(&git_path);
            (total, by_day)
        })
        .await
        .unwrap_or((0, BTreeMap::new()));
        match result {
            Ok(rows) => {
                let rows = rows
                    .iter()
                    .map(|(summary, payload)| {
                        (
                            dozer_model::ConversationMeta::from_summary(summary),
                            ConversationUsage::from(payload),
                        )
                    })
                    .collect::<Vec<_>>();
                emit(Message::Loaded(
                    project_id,
                    generation,
                    rows,
                    git_commits,
                    git_commits_by_day,
                ));
            }
            Err(e) => emit(Message::LoadFailed(
                project_id,
                generation,
                e.message,
                git_commits,
                git_commits_by_day,
            )),
        }
    });
}

/// 面板切入:先置"统计中",有项目就发起一次用量统计(没有项目时只置 loading——迁移前的行为)。
pub fn on_activate(state: &mut WorkspaceState, ctx: Option<&ActivationCtx<Message>>) {
    state.set_loading(true);
    let Some(ctx) = ctx else {
        return;
    };
    let Some(path) = ctx.project_path.clone() else {
        return;
    };
    let generation = state.begin_refresh();
    spawn_refresh(
        ctx.project_id,
        generation,
        path,
        Arc::clone(&ctx.usage),
        Arc::clone(&ctx.git),
        &ctx.handle,
        ctx.emitter(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(agent: AgentKind, title: &str) -> ConversationMeta {
        ConversationMeta {
            path: std::path::PathBuf::from(format!("/{title}.jsonl")),
            title: title.to_string(),
            modified_ms: 0,
            agent,
        }
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
            Message::Loaded(1, 0, rows.clone(), 42, Default::default()),
        );
        assert!(!ws_state.loading());
        assert_eq!(ws_state.rows(), rows.as_slice());
        assert_eq!(ws_state.git_commits(), 42);
        assert!(ws_state.git_commits_by_day().is_empty());
    }

    fn rows_of(n: usize) -> Vec<(ConversationMeta, ConversationUsage)> {
        (0..n)
            .map(|i| {
                (
                    meta(AgentKind::Claude, &format!("r{i}")),
                    ConversationUsage::default(),
                )
            })
            .collect()
    }

    /// G3b:同一项目快速切入两次会发起两次刷新;较早发起的那次后到时,不能覆盖较新的结果。
    #[test]
    fn a_stale_refresh_arriving_late_does_not_overwrite_the_newer_one() {
        let mut state = WorkspaceState::default();
        let first = state.begin_refresh();
        let second = state.begin_refresh();
        update(
            &mut state,
            Message::Loaded(1, second, rows_of(2), 5, Default::default()),
        );
        update(
            &mut state,
            Message::Loaded(1, first, rows_of(1), 9, Default::default()),
        );
        assert_eq!(state.rows().len(), 2, "较早发起的刷新后到,必须被丢弃");
        assert_eq!(state.git_commits(), 5);
        assert!(!state.loading());
    }

    /// 较旧的结果先到、较新的还在路上:不能把"统计中"提前清掉,也不能展示旧数据。
    #[test]
    fn a_stale_refresh_arriving_first_keeps_loading_until_the_newest_lands() {
        let mut state = WorkspaceState::default();
        let first = state.begin_refresh();
        let second = state.begin_refresh();
        update(
            &mut state,
            Message::Loaded(1, first, rows_of(1), 9, Default::default()),
        );
        assert!(state.loading());
        assert!(state.rows().is_empty());
        update(
            &mut state,
            Message::Loaded(1, second, rows_of(2), 5, Default::default()),
        );
        assert!(!state.loading());
        assert_eq!(state.rows().len(), 2);
    }

    /// 同一代数的结果照常应用(单次刷新的正常路径)。
    #[test]
    fn the_latest_refresh_is_applied() {
        let mut state = WorkspaceState::default();
        let only = state.begin_refresh();
        update(
            &mut state,
            Message::Loaded(1, only, rows_of(3), 0, Default::default()),
        );
        assert_eq!(state.rows().len(), 3);
    }

    #[test]
    fn load_failed_keeps_rows_updates_git_and_queues_one_error_toast() {
        let mut ws = WorkspaceState::default();
        let g1 = ws.begin_refresh();
        update(
            &mut ws,
            Message::Loaded(1, g1, rows_of(2), 5, Default::default()),
        );
        let g2 = ws.begin_refresh();
        update(
            &mut ws,
            Message::LoadFailed(1, g2, "daemon 断开".into(), 9, Default::default()),
        );
        assert_eq!(ws.rows().len(), 2, "失败不得抹掉上次成功的数据");
        assert_eq!(ws.git_commits(), 9, "git 提交数是独立数据源,照常更新");
        assert!(!ws.loading(), "失败后要退出'统计中'");
        let notices = ws.take_outbox();
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].level, NoticeLevel::Error);
        assert!(notices[0].text.contains("daemon 断开"));
        assert_eq!(notices[0].key.as_deref(), Some("usage-load"));
        assert!(ws.take_outbox().is_empty(), "排空后不重复");
    }

    #[test]
    fn stale_load_failed_is_dropped_without_touching_loading() {
        let mut ws = WorkspaceState::default();
        let first = ws.begin_refresh();
        let second = ws.begin_refresh();
        update(
            &mut ws,
            Message::LoadFailed(1, first, "旧的失败".into(), 9, Default::default()),
        );
        assert!(ws.loading(), "新的刷新还在路上");
        assert_eq!(ws.git_commits(), 0);
        assert!(ws.take_outbox().is_empty());
        update(
            &mut ws,
            Message::LoadFailed(1, second, "新的失败".into(), 1, Default::default()),
        );
        assert!(!ws.loading());
        assert_eq!(ws.take_outbox().len(), 1);
    }
}
