//! Rust 代码结构分析：函数/文件/项目三层复杂度指标，供「代码健康度」
//! 面板使用（spec docs/superpowers/specs/2026-09-20-code-health-panel-design.md）。
//! 纯函数库，不依赖 iced/UDS 协议。

mod function_metric;
pub use function_metric::{functions_in_source, severity_for, FunctionMetric, Severity};
