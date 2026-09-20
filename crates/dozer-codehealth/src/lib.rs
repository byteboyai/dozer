//! Rust 代码结构分析：函数/文件/项目三层复杂度指标，供「代码健康度」
//! 面板使用（spec docs/superpowers/specs/2026-09-20-code-health-panel-design.md）。
//! 纯函数库，不依赖 iced/UDS 协议。

mod function_metric;
mod report;
mod ui_metrics;

pub use function_metric::{FunctionMetric, Severity, functions_in_source, severity_for};
pub use report::{
    FileMetric, HealthTier, ProjectReport, density_tier, file_metric, scale_tier, scan_project,
};
pub use ui_metrics::{
    DuplicateCluster, RawLiteralFinding, color_tier, cluster_tier, event_handler_tier, font_tier,
    nesting_depth_tier, spacing_tier,
};
