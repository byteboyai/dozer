//! 浏览器面板扩展(阶段 1 扩展化重构第二个试点)。自己的 `Message`/
//! `State`/`update`/`view`,内核只认一个包装变体 `Message::Browser(..)`
//! 做转发,不知道自己被包在哪个外层类型里。收藏夹(全局/本项目两级书签)
//! 是浏览器域专属功能,状态/消息/UI 整体并入这个模块。
//!
//! `Tabs` 是浏览器专用的精简 tab/地址栏状态机,只认 URL——不复用
//! `crate::preview::PreviewPane`(文件预览专用,那边还有 `TabKind::File`/
//! `Acceptance`、`open_path`/`is_editable_extension`/`flyfish_url` 等浏览器
//! 用不到的逻辑),两者各自维护、互不知情。
//!
//! 状态机、`update`、视图都在共享库 `dozer-panel-browser`;本文件只做重新导出与
//! `WebviewSpec` 转换。视图要的页签控件 / 分隔条 / 区域样式由
//! `host_adapters::browser_panel` 里 `BrowserDisplay for App` 注入(放那里而不是这里,
//! 是因为 `extensions/` 下不得新增对 `crate::app::App` 的使用,见面板边界门禁 R-APP)。

use crate::preview::WebviewSpec;

/// 库产出的纯数据 webview 清单项 → 宿主的 `WebviewSpec`。其余字段的取值与搬迁前
/// `Tabs::desired_webviews` 里逐项相同:浏览器 webview 不参与预览资源 reserve、不走
/// Surface、没有隐藏预创建所以不需要离屏停放。
impl From<dozer_panel_browser::adapter::BrowserWebviewSpec> for WebviewSpec {
    fn from(s: dozer_panel_browser::adapter::BrowserWebviewSpec) -> Self {
        WebviewSpec {
            id: s.id,
            url: s.url,
            visible: s.visible,
            editor_binding: None,
            reserve: None,
            loading_generation: None,
            surface: None,
            park_offscreen: false,
            display_i18n: None,
        }
    }
}

/// 地址栏焦点机制(进程级 `ADDR_FOCUSED` 唯一一份,在共享库里)。
pub use dozer_panel_browser::{
    CaptureAddrFocus, FindRequest, addr_field_id, find_field_id, take_addr_focused,
};

/// 状态机 / 消息 / `update` / 收藏夹逻辑已搬进共享库,这里重新导出,`app/` 里的调用点不动。
pub use dozer_panel_browser::{Message, NavAction, State, request_bookmarks_refresh, update};

/// 视图(页签栏 / 地址栏 / 收藏夹侧栏)在共享库里,经 `BrowserDisplay` 取产品展示。
pub use dozer_panel_browser::view;
