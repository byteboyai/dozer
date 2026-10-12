//! 用量面板:产品侧**重新导出壳**(计划 `docs/superpowers/plans/2026-10-11-digger-bytehost-full-integration.md`
//! §Task 19)。面板本体(数据层/视图层/web 资源)已抽到独立库 `dozer-panel-usage`,Dozer 与
//! Digger 共用唯一实现。本文件只做第 5 条要求的"重新导出/注册包装",让既有调用点
//! (`usage::content_pane` / `usage::Message` / `usage::WorkspaceState` / `usage::UsageSurface` /
//! `usage::update` 等)不改名。
//!
//! 产品专属适配(`PanelDisplay<AppMessage> for App`、`DoGitStats`、`notices_to_pending`、
//! `activate`)在 host 侧的 [`crate::host_adapters::usage_panel`]——那里会出现 `crate::app::App`,
//! 而面板边界门禁(`scripts/audit/check_panel_boundary.py`)要求 `extensions/` 下不出现该引用。
//!
//! 库依赖闭包不含 `dozer-app`/`dozer-client`/`dozer-core`。

pub use dozer_panel_usage::*;

pub use crate::host_adapters::usage_panel::{activate, notices_to_pending};
