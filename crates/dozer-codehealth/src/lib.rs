//! Rust 代码结构分析：函数/文件/项目三层复杂度指标，供「代码健康度」
//! 面板使用（spec docs/superpowers/specs/2026-09-20-code-health-panel-design.md）。
//! 纯函数库，不依赖 iced/UDS 协议。

pub mod architecture;
mod diff;
mod discovery;
mod finding;
mod function_metric;
mod report;
mod scan_metadata;
mod ui_metrics;

pub use architecture::{
    ArchitectureEdge, ArchitectureEdgeKind, ArchitectureEvidence, ArchitectureNode,
    ArchitectureNodeKind, ArchitectureReport, ArchitectureStatus, DependencyCycle,
};
pub use diff::{FindingChange, ReportDiff, diff_reports};
pub use discovery::{Discovery, ProjectConfig, discover, load_project_config};
pub use finding::{
    Applicability, Finding, FindingCategory, FindingEvidence, FindingSeverity,
    normalize_path_for_id, rule_ids, stable_finding_id,
};
pub use function_metric::{FunctionMetric, Severity, functions_in_source, severity_for};
pub use report::{
    FileMetric, HealthTier, ProjectReport, density_tier, file_metric, scale_tier, scan_project,
};
pub use scan_metadata::{
    GitSnapshot, LEGACY_SCHEMA_VERSION, LanguageSummary, SCHEMA_VERSION, ScanMetadata, ScanStatus,
    SkipReason, SkippedFile, schema_version_or_one,
};
pub use ui_metrics::{
    DuplicateCluster, RawLiteralFinding, cluster_tier, color_tier, event_handler_tier, font_tier,
    nesting_depth_tier, spacing_tier,
};
