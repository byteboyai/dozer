//! 应用面板的宿主逻辑(bytehost A4b2)——状态机本体已搬到 `bytehost-panel::host`(与 Digger 共用),
//! 这里只保留 **dozer 专属的绑定**:
//!
//! - 以进程内槽号 [`AppSlot`] 作为状态机的 [`AppKey`];
//! - `State`/`Message`/`Effect` 三个别名把槽号填进泛型状态机;
//! - [`Effect::Notice`] 的等级([`NoticeLevel`])映射到 dozer 的 [`toast::Level`]。
//!
//! 约定(规格 A4 与 A4b 评审留下的,原样保留在共享 crate 里):
//! - **轮询,不做推送**:应用面板可见时每 `POLL_INTERVAL` 拉一次 `List`;没有应用面板可见时只在启动后拉一次;
//! - **启动地址含令牌(秘密)**:每次面板切入都清掉旧地址、重新取;
//! - **瞬时失败走 Notice,持久状态留在面板里**;列表请求的传输层失败不清空已知应用、不弹 Toast。

use crate::app::AppSlot;

#[allow(unused_imports)] // 轮询档位常量是 crate 的对外接口,dozer 侧暂无直接引用
pub use bytehost_panel::host::{
    Act, Failure, LogsView, POLL_INTERVAL, PanelState, RESUBSCRIBE_BACKOFF, SAFETY_POLL_INTERVAL,
    issue_texts,
};
pub use bytehost_panel::host::{PanelEffect, PanelMessage, PanelView};
pub use bytehost_panel::key::AppKey;
pub use bytehost_panel::notice::NoticeLevel;

/// 以进程内槽号标识应用:dozer 的 [`AppSlot`] 只增不减,同一个 id 永远得到同一个槽。
impl AppKey for AppSlot {
    fn from_app_id(id: &bytehost_apps::id::AppId) -> Option<Self> {
        AppSlot::intern(id.as_str())
    }

    fn app_id(&self) -> &str {
        self.id()
    }
}

/// 应用面板状态机(键 = 进程内槽号)。
pub type State = PanelState<AppSlot>;
/// 应用面板状态机的消息。
pub type Message = PanelMessage<AppSlot>;
/// 应用面板状态机要 `App` 做的事。
pub type Effect = PanelEffect<AppSlot>;

/// 把共享状态机的瞬时消息等级映射到 dozer 的 Toast 等级。
pub fn notice_level(level: NoticeLevel) -> crate::extensions::toast::Level {
    use crate::extensions::toast::Level;
    match level {
        NoticeLevel::Info => Level::Info,
        NoticeLevel::Success => Level::Success,
        NoticeLevel::Warning => Level::Warning,
        NoticeLevel::Error => Level::Error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytehost_apps::id::AppId;

    /// `AppSlot` 作为 [`AppKey`] 的往返与非法 id 过滤。
    #[test]
    fn app_slot_implements_appkey() {
        let a = AppSlot::intern("appkey-a").unwrap();
        let id = AppId::new("appkey-a").unwrap();
        assert_eq!(AppSlot::from_app_id(&id), Some(a));
        assert_eq!(AppKey::app_id(&a), "appkey-a");

        // 形状非法的 id(大写)在 `AppSlot::intern` 与 `AppId::new` 都被拒;
        // 这里构造一个 `AppSlot` 认不了但 `AppId` 认得的形状不合,所以只钉往返口径。
        let b = AppSlot::intern("appkey-b").unwrap();
        assert_ne!(a, b);
        assert_eq!(
            AppSlot::from_app_id(&AppId::new("appkey-b").unwrap()),
            Some(b)
        );
    }
}
