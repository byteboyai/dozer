//! Usage 面板的**产品侧适配层**(Task 19 阶段 3)。
//!
//! 面板本体(数据/视图/web 资源)已抽到独立库 `dozer-panel-usage`,Dozer 与 Digger
//! 共用唯一实现。库通过接缝(`dozer_panel_usage::adapter`)反过来要产品提供:
//!
//! - **展示语义**:[`adapter::PanelDisplay`] 的 `App` 实现(agent 图标/颜色、面板头、
//!   列表折叠按钮、折叠/悬停事件映射)——这是产品展示适配,天然属于 host 侧,故放本层
//!   而非 `extensions/usage.rs`(面板边界门禁 `R-APP` 要求 `extensions/` 下不出现
//!   `crate::app::App`;见 `scripts/audit/check_panel_boundary.py`)。
//! - **Git 统计**:[`adapter::GitStatsProvider`],DoGitStats 包 `bytegit::Repo`,语义与
//!   抽取前 `extensions/usage/mod.rs` 的 `count_git_commits`/`count_git_commits_by_day`
//!   逐字一致(非 git 仓库/任何错误一律 0 / 空)。
//! - **Toast 转换** + **面板切入**:库排空的 [`adapter::UsageNotice`] → 产品
//!   `toast::Pending`;host 的 [`crate::panel_host::ActivationCtx`] → 库的
//!   `activation::ActivationCtx`。
//!
//! `extensions/usage.rs` 只做重新导出(计划第 5 条:"Dozer 仅保留重新导出/注册包装")。

use crate::app::{HoverId, Message as AppMessage, PanelKind};
use byteui::interaction::icons;
use dozer_model::AgentKind;
use dozer_panel_usage::adapter::{
    GitStatsError, GitStatsProvider, GitSummary, NoticeLevel, PanelDisplay, PanelHeadKind,
    UsageNotice,
};
use dozer_panel_usage::{WorkspaceState, activation, on_activate};
use iced_widget::core::{Color, Element};

dozer_core::scope!(LOG, module, "usage_panel");

impl PanelDisplay<AppMessage> for crate::app::App {
    fn wrap(&self, message: dozer_panel_usage::state::Message) -> AppMessage {
        AppMessage::Usage(message)
    }

    fn agent_icon_color(&self, agent: AgentKind) -> Color {
        crate::workspace::agent_dot_color(agent)
    }

    fn agent_icon(
        &self,
        agent: AgentKind,
        color: Color,
    ) -> Element<'static, AppMessage, iced_widget::Theme, iced_renderer::Renderer> {
        icons::view(
            crate::workspace::agent_icon(agent),
            byteui::theme::icon_size::row(),
            color,
        )
    }

    fn panel_head<'a>(
        &self,
        kind: PanelHeadKind,
        title: &'a str,
        actions: Option<Element<'a, AppMessage, iced_widget::Theme, iced_renderer::Renderer>>,
    ) -> Element<'a, AppMessage, iced_widget::Theme, iced_renderer::Renderer> {
        // 图标按稳定身份选(不解析标题——i18n 后标题会变):内容侧"用量"用
        // `BarChart3`,筛选栏"Agent"用 `Bot`——与抽取前 `view.rs`/`mod.rs` 的
        // 两处头部逐字一致。
        let icon = match kind {
            PanelHeadKind::AgentFilter => icons::IconKind::Bot,
            PanelHeadKind::Content => icons::IconKind::BarChart3,
        };
        crate::chrome::homespace::home_panel_head_with_actions(icon, title, actions)
    }

    fn list_collapse_button<'a>(
        &self,
        collapsed: bool,
        tooltip_collapse: impl Into<std::borrow::Cow<'a, str>>,
        tooltip_expand: impl Into<std::borrow::Cow<'a, str>>,
        on_select: AppMessage,
        on_hover: impl Fn(bool) -> AppMessage + 'a,
    ) -> Element<'a, AppMessage, iced_widget::Theme, iced_renderer::Renderer> {
        crate::app::App::list_collapse_button(
            self,
            PanelKind::Usage,
            collapsed,
            HoverId::list_collapse(PanelKind::Usage),
            tooltip_collapse,
            tooltip_expand,
            on_select,
            on_hover,
        )
    }

    fn collapse_hover(&self, hovered: bool) -> AppMessage {
        AppMessage::Hover(HoverId::list_collapse(PanelKind::Usage), hovered)
    }

    fn toggle_list_collapse(&self) -> AppMessage {
        AppMessage::TogglePanelListCollapse(PanelKind::Usage)
    }
}

/// 产品的 [`GitStatsProvider`](adapter::GitStatsProvider):包 `bytegit::Repo`,语义与
/// 抽取前 `extensions/usage/mod.rs` 的 `count_git_commits`/`count_git_commits_by_day`
/// 逐字一致(非 git 仓库/任何错误一律 0 / 空)。
pub struct DoGitStats {
    cache: dozer_git_runtime::StatsCache,
}

impl DoGitStats {
    pub fn new() -> Self {
        Self {
            cache: dozer_git_runtime::StatsCache::default(),
        }
    }
}

/// 进程级共享的 Git 统计 provider:缓存必须跨 Usage 激活存活,不能在每次
/// `activate` 里新建。
static SHARED_GIT_STATS: std::sync::OnceLock<std::sync::Arc<DoGitStats>> =
    std::sync::OnceLock::new();

pub fn shared_git_stats() -> std::sync::Arc<DoGitStats> {
    SHARED_GIT_STATS
        .get_or_init(|| std::sync::Arc::new(DoGitStats::new()))
        .clone()
}

impl Default for DoGitStats {
    fn default() -> Self {
        Self::new()
    }
}

/// 影响统计结果的小型版本指纹:历史边界(replace/grafts/alternates)与配置。
fn stats_fingerprint(identity: &bytegit::RepoIdentity) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let paths = [
        identity.common_dir().join("shallow"),
        identity.common_dir().join("info/grafts"),
        identity.common_dir().join("objects/info/alternates"),
        identity.common_dir().join("refs/replace"),
        identity.common_dir().join("config"),
        identity.git_dir().join("config.worktree"),
    ];
    for path in paths {
        path.hash(&mut hasher);
        if let Ok(meta) = std::fs::metadata(&path) {
            meta.len().hash(&mut hasher);
            if let Ok(modified) = meta.modified()
                && let Ok(since) = modified.duration_since(std::time::UNIX_EPOCH)
            {
                since.as_nanos().hash(&mut hasher);
            }
        }
    }
    hasher.finish()
}

impl GitStatsProvider for DoGitStats {
    fn commit_count(&self, project_path: &std::path::Path) -> u64 {
        // `discover` 兼容项目路径是仓库子目录的情况;`open` 只认仓库根。
        let Ok(repo) = bytegit::Repo::discover(project_path) else {
            return 0;
        };
        repo.commit_count().unwrap_or(0)
    }

    fn commits_by_day(
        &self,
        project_path: &std::path::Path,
    ) -> std::collections::BTreeMap<i64, u64> {
        let Ok(repo) = bytegit::Repo::discover(project_path) else {
            return std::collections::BTreeMap::new();
        };
        repo.commit_count_by_day().unwrap_or_default()
    }

    /// 一次日桶遍历后求和;非 git/空仓库/后端错误仍按旧语义回落 0。
    fn summary(&self, project_path: &std::path::Path) -> Result<GitSummary, GitStatsError> {
        let Ok(repo) = bytegit::Repo::discover(project_path) else {
            return Ok(GitSummary::default());
        };
        let Ok(identity) = repo.identity() else {
            return Ok(GitSummary::default());
        };
        let Ok(workspace_key) = identity.workspace_key() else {
            return Ok(GitSummary::default());
        };
        let head = repo.head().ok().and_then(|head| head.commit);
        let key = dozer_git_runtime::StatsKey {
            workspace_key,
            head,
            fingerprint: stats_fingerprint(&identity),
        };
        if let Some(cached) = self.cache.get(&key) {
            return Ok(GitSummary {
                total: cached.total,
                by_day: cached.by_day,
            });
        }
        let by_day = repo.commit_count_by_day().unwrap_or_default();
        let total = by_day.values().sum();
        let stats = dozer_git_runtime::CommitStats {
            total,
            by_day: by_day.clone(),
        };
        self.cache.insert(key, stats);
        Ok(GitSummary { total, by_day })
    }
}

/// 把库排空的 [`UsageNotice`] 转成产品的 `toast::Pending`(日志来源用本适配层
/// `LOG`)。语义与抽取前 `WorkspaceState::take_outbox` 直接返回 `toast::Pending`
/// 一致:级别同形,去重键照搬。
pub fn notices_to_pending(notices: Vec<UsageNotice>) -> Vec<crate::extensions::toast::Pending> {
    notices
        .into_iter()
        .map(|notice| {
            let level = match notice.level {
                NoticeLevel::Info => crate::extensions::toast::Level::Info,
                NoticeLevel::Success => crate::extensions::toast::Level::Success,
                NoticeLevel::Warning => crate::extensions::toast::Level::Warning,
                NoticeLevel::Error => crate::extensions::toast::Level::Error,
            };
            crate::extensions::toast::Pending {
                scope: LOG,
                level,
                text: notice.text,
                key: notice.key,
            }
        })
        .collect()
}

/// 面板切入:host 的 [`crate::panel_host::ActivationCtx`] → 库的
/// [`activation::ActivationCtx`],再调库的 `on_activate`。产品注入 Git 统计提供者
/// (库内不再直连 `bytegit`)。`ctx` 为 `None`(无焦点项目)时只置"统计中",同迁移前行为。
///
/// 注意 `ctx` 的泛型 `M` 是**库的消息类型**:`workspace_activation_ctx(ws, io,
/// Message::Usage)` 用 `Message::Usage` 当 `wrap`,于是 `io` 的发射器已经带上了
/// "库消息 → 顶层 `Message::Usage`"这层包装,直接交给库即可,不必再包一次。
pub fn activate(
    state: &mut WorkspaceState,
    ctx: Option<crate::panel_host::ActivationCtx<dozer_panel_usage::Message>>,
) {
    let lib_ctx = ctx.map(|ctx| {
        activation::ActivationCtx::new(
            ctx.project_id,
            ctx.project_path,
            ctx.io.services().usage.clone(),
            shared_git_stats(),
            ctx.io.handle().clone(),
            ctx.io.emitter(),
        )
    });
    on_activate(state, lib_ctx.as_ref());
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytegit::testutil::TempRepo;

    #[test]
    fn shared_git_stats_survives_activation_and_keeps_one_cache() {
        let a = shared_git_stats();
        let b = shared_git_stats();
        assert!(
            std::sync::Arc::ptr_eq(&a, &b),
            "Usage 激活必须共用一个 provider/cache"
        );
    }

    #[test]
    fn do_git_stats_summary_sums_the_single_day_bucket_pass() {
        let t = TempRepo::new();
        t.commit_file("a.txt", "1\n", "one");
        t.commit_file("a.txt", "2\n", "two");
        let git = DoGitStats::new();
        let summary = git.summary(t.path()).unwrap();
        assert_eq!(summary.total, summary.by_day.values().sum::<u64>());
        assert_eq!(summary.total, 2);
        assert_eq!(git.cache.len(), 1, "首次结果应进入缓存");

        // 相同 HEAD/边界命中缓存,不再新增条目。
        let again = git.summary(t.path()).unwrap();
        assert_eq!(again, summary);
        assert_eq!(git.cache.len(), 1);

        // 新提交 → HEAD 变化 → 新键、新结果。
        t.commit_file("a.txt", "3\n", "three");
        let newer = git.summary(t.path()).unwrap();
        assert_eq!(newer.total, 3);
        assert_eq!(git.cache.len(), 2, "HEAD 变化不能复用旧结果");
    }
}
