//! 应用日志查看器的共享状态机(A6f Task 4)——状态机本体已搬到 `bytehost-panel::logs`
//! (与 Digger 共用),这里只保留**iced 专属**的滚动区 `Id`。
//!
//! [`LogsState`]/[`LogsView`]/[`LogsEffect`]/[`REFRESH_INTERVAL`]/[`FETCH_LINES`] 直接复用共享实现;
//! 两个宿主不各写一遍。

#[allow(unused_imports)] // `LogsEffect` 是共享状态机的对外接口,dozer 侧经 `LogsState` 间接使用
pub use bytehost_panel::logs::{FETCH_LINES, LogsEffect, LogsState, LogsView, REFRESH_INTERVAL};

/// 日志查看器滚动区的稳定 `Id`(按应用 id 区分,两个宿主共用同一个前缀)。
pub fn scroll_id(app_id: &str) -> iced_widget::core::widget::Id {
    iced_widget::core::widget::Id::from(format!("app-log-view:{app_id}"))
}
