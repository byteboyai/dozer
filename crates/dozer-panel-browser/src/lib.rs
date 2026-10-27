//! 同源浏览器面板库:Dozer 与 Digger 共用的页签 / 地址栏状态机与文案。
//!
//! 依赖闭包**不含** `dozer-app` / `dozer-client` / `dozer-core` / `wry` / `winit`。库只产出纯数据
//! (如 [`adapter::BrowserWebviewSpec`]),wry 池与真正的导航执行留在宿主。

pub mod adapter;
mod addr_focus;
pub mod bookmarks;
pub mod i18n;
mod state;
mod tabs;
mod update;
mod view;

/// 面板 owner id:`locales/`(随库)。
pub const WEB_OWNER: &str = "dozer.web";

pub use adapter::{BrowserDisplay, BrowserRegion, TAB_TOOLTIP_DELAY, TabView, chrome_top_px};
pub use addr_focus::{CaptureAddrFocus, addr_field_id, take_addr_focused};
pub use bookmarks::{BookmarkStatus, bookmark_status};
pub use state::{
    NAV_BACK_KEY, NAV_FORWARD_KEY, NAV_REFRESH_KEY, NEW_TAB_KEY, STAR_HOVER_KEY, State,
};
pub use tabs::{BrowserTab, Tabs};
pub use update::{Message, NavAction, request_bookmarks_refresh, update};
pub use view::view;

#[cfg(test)]
mod testing;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod view_tests;
