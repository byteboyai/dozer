//! 产品注入 / 产出的适配类型。

/// 浏览器页签期望的 webview(纯数据):由库产出,宿主转成自己的 webview 池规格。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserWebviewSpec {
    pub id: usize,
    pub url: String,
    pub visible: bool,
}

/// 文本框的右键菜单目标:库只发出这个不透明目标,由产品转成自己的菜单请求。
/// 形状与 Dozer `app::TextInputTarget` 相同。
#[derive(Debug, Clone)]
pub struct TextInputTarget {
    pub id: iced_widget::core::widget::Id,
    pub secure: bool,
}

/// 页签标题悬停多久后弹全称 tooltip(与 Dozer 顶栏同一取值)。
pub const TAB_TOOLTIP_DELAY: std::time::Duration = std::time::Duration::from_secs(2);
