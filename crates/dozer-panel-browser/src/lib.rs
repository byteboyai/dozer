//! 同源浏览器面板库:Dozer 与 Digger 共用的页签 / 地址栏状态机与文案。
//!
//! 依赖闭包**不含** `dozer-app` / `dozer-client` / `dozer-core` / `wry` / `winit`。库只产出纯数据
//! (如 [`adapter::BrowserWebviewSpec`]),wry 池与真正的导航执行留在宿主。

pub mod adapter;
pub mod i18n;
mod tabs;

/// 面板 owner id:`locales/`(随库)。
pub const WEB_OWNER: &str = "dozer.web";

pub use tabs::{BrowserTab, Tabs};
