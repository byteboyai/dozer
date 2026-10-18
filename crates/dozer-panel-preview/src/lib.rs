//! 同源预览面板库(S5b-2 P2):Dozer 与 Digger 共用的预览面板状态、更新与视图。
//!
//! 纯逻辑(路由 / 画像 / 策略 / 加载 / 保存)在 `dozer-preview-core`;本库在其上加
//! 面板状态、`update`、iced 视图与能力开关。产品专属语义(全局单例、落盘位置、webview 池)
//! 一律经注入提供,库不依赖 `dozer-app` / `dozer-client` / `dozer-core` / wry / winit。
//!
//! 模块布局沿用迁移前 `dozer-app::preview`:搬来的文件用 `use super::*` 取同级符号,
//! 所以这里把核心的公开面和本库各模块一并重导出到 crate 根。

pub mod capabilities;
pub mod code_host;
pub mod i18n;
pub mod pane;
#[cfg(test)]
mod pane_isolation_tests;
pub mod plantuml;
pub mod state;
pub mod tabular;
pub mod webview;
pub mod webview_protocol;

/// 本库自有文案的 i18n owner(与核心的 `dozer.preview` 区分)。
pub const PANEL_OWNER: &str = "dozer.preview_panel";

pub use dozer_preview_core::PREVIEW_OWNER;
pub use dozer_preview_core::backend::*;
pub use dozer_preview_core::file_policy::*;
pub use dozer_preview_core::file_profile::*;
pub use dozer_preview_core::large_text::*;
pub use dozer_preview_core::loading::*;
pub use dozer_preview_core::native_editor::*;
pub use dozer_preview_core::recovery::*;
pub use dozer_preview_core::resources::*;
pub use dozer_preview_core::router::*;
pub use dozer_preview_core::startup::*;
pub use dozer_preview_core::text_save::*;
pub use dozer_preview_core::{file_profile, native_editor};

pub use code_host::*;
pub use pane::*;
pub use plantuml::*;
pub use state::*;
pub use webview::*;
pub use webview_protocol::*;

/// 没有产品注入能力快照时(库内测试、`PreviewPane::default()`)用的固定参考能力:
/// 一台 16 GiB / 8 核的中档机器,与探测结果无关,保证测试可复现。
pub fn default_client_capabilities() -> dozer_preview_core::capabilities::ClientCapabilities {
    use dozer_preview_core::capabilities::{HardwareCapabilities, estimate_capabilities};
    const GIB: u64 = 1024 * 1024 * 1024;
    estimate_capabilities(HardwareCapabilities {
        total_memory_bytes: 16 * GIB,
        available_memory_at_start_bytes: 8 * GIB,
        physical_cpu_count: 8,
        logical_cpu_count: 8,
    })
}
