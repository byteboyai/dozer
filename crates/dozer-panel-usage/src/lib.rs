//! 同源 Usage 面板库(Task 19):Dozer 与 Digger 共用的 Usage 面板数据层、
//! 视图层与 web 资源。
//!
//! 依赖闭包**不含** `dozer-app` / `dozer-client` / `dozer-core`(计划 §Task 19
//! 第 5 条):允许领域 `dozer-model`/`dozer-services` 与公共 UI(`byteui`、
//! `bytehost-core`/`desktop`、iced)。产品专属语义经注入的适配器提供,见
//! [`adapter`]:
//! - Git 统计 → [`adapter::GitStatsProvider`]
//! - 一次性提示 → [`adapter::UsageOutbox`] / [`adapter::UsageNotice`]
//! - webview 推送脚本构造 → [`adapter::ScriptDispatch`]
//! - 展示语义(agent 图标/面板头/折叠按钮)→ [`adapter::PanelDisplay`]
//!
//! 产品(Dozer)只保留"重新导出/注册包装",在 `PanelSelect(PanelKind::Usage)`
//! 分支构造 [`activation::ActivationCtx`] 调 [`state::on_activate`],并把本库的
//! `Message` 纳入顶层 `Message`。

pub mod activation;
pub mod adapter;
pub mod aggregate;
pub mod protocol;
pub mod state;
pub mod surface;
pub mod view;

pub use activation::ActivationCtx;
pub use adapter::{
    GitStatsProvider, NoGitStats, NoticeLevel, PanelDisplay, ScriptDispatch, UsageNotice,
    UsageOutbox,
};
pub use aggregate::{
    ConversationUsage, DayAgentTotals, DaySeries, ProjectUsageTotals, agent_cache_token_share,
    agent_io_token_share, agent_metric_share, agent_session_share, agent_token_share,
    agent_turn_share, agents_present, aggregate, behavior_series, civil_from_days,
    daily_totals_by_agent, filter_rows_by_agent, session_round_trend, today_day_index,
    trend_series,
};
pub use protocol::{
    AgentMetrics, AgentShare, DailyUsageChart, DailyUsageDay, ProjectSummary, TokenTrendSection,
    TrendChart, TrendChartKind, TrendDay, USAGE_PROTOCOL_VERSION, UsagePushEnvelope,
    UsageViewPayload, UsageWebviewEvent, current_view_payload, encode_usage_push,
    parse_usage_event,
};
pub use state::{Message, WorkspaceState, on_activate, spawn_refresh, update};
pub use surface::UsageSurface;
pub use view::{content_pane, list_pane};

/// 本 crate 自带的 usage-content web 资源根(`assets/usage-content/`,
/// 由 `web/usage-content/build.mjs` 构建)。**唯一来源**(计划 §Task 19 第 4
/// 条):消费方(Dozer / Digger)从这条路径取 `host.html` / `usage-content.js`
/// / `usage-content.css`,不各自留副本。
///
/// 返回 `PathBuf` 而非 `&'static Path`,便于消费方在打包态改为指向
/// `Contents/Resources/usage-content`。
pub fn web_assets_root() -> std::path::PathBuf {
    std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/usage-content"))
}

#[cfg(test)]
mod lib_tests;
