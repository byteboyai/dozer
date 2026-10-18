//! 同源预览面板库(S5b-2 P2):Dozer 与 Digger 共用的预览面板状态、更新与视图。
//!
//! 纯逻辑(路由 / 画像 / 策略 / 加载 / 保存)在 `dozer-preview-core`;本库在其上加
//! 面板状态、`update`、iced 视图与能力开关。产品专属语义(全局单例、落盘位置、webview 池)
//! 一律经注入提供,库不依赖 `dozer-app` / `dozer-client` / `dozer-core` / wry / winit。

pub mod capabilities;
pub mod i18n;
pub mod tabular;

/// 本库自有文案的 i18n owner(与核心的 `dozer.preview` 区分)。
pub const PANEL_OWNER: &str = "dozer.preview_panel";
