//! 产品注入 / 产出的适配类型。

use iced_widget::Theme;
use iced_widget::core::{Color, Element, Padding};

type Renderer = iced_renderer::Renderer;

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

/// 浏览器面板区域样式:产品主题里 `browser_pane` 区域的投影(背景 / 内边距 / 行间距)。
#[derive(Debug, Clone, Copy)]
pub struct BrowserRegion {
    pub background: Option<Color>,
    pub padding: Padding,
    pub gap: f32,
}

/// 单个页签交给宿主绘制时需要的展示数据。
#[derive(Debug, Clone)]
pub struct TabView {
    pub title: String,
    pub active: bool,
    pub hover_t: f32,
    pub close_hover_t: f32,
    /// 悬停已满 [`TAB_TOOLTIP_DELAY`]:弹标题全称 tooltip。
    pub show_tooltip: bool,
}

/// 视图向宿主要的展示能力:页签控件、分隔条、区域样式都是产品语义,由宿主注入,
/// 库不重新手写一套(页签走产品的统一页签组件)。
pub trait BrowserDisplay {
    /// 面板区域样式(Dozer:`theme::region::browser_pane()` 的投影)。
    fn region(&self) -> BrowserRegion;
    /// 当前界面语言。
    fn locale(&self) -> &str;
    /// 列表/正文行的统一行高倍数(Dozer:与终端行距同源的 `terminal_font::line_height_factor()`,
    /// 用户可配,所以由宿主给,库里不写死)。
    fn line_height_factor(&self) -> f32;
    /// 页签栏下方的分割线。
    fn tab_divider<'a, M: 'a>(&self) -> Element<'a, M, Theme, Renderer>;
    /// 一个页签控件(Dozer:`chrome::tab_widget::panel_tab`)。
    fn tab<'a, M: Clone + 'a>(
        &self,
        v: TabView,
        on_select: M,
        on_close: M,
        title_hover: impl Fn(bool) -> M + 'a,
        close_hover: impl Fn(bool) -> M + 'a,
    ) -> Element<'a, M, Theme, Renderer>;
    /// 网页与收藏夹侧栏之间可拖拽的分隔条(Dozer:`app::divider_bar(Divider::BrowserBookmarksSplit, ..)`)。
    /// 只在侧栏展开时才会被调用。
    fn bookmarks_divider<'a, M: Clone + 'a>(&self, on_drag: M) -> Element<'a, M, Theme, Renderer>;
}

/// 页签栏 + 地址栏占用的高度(webview 从这里往下画)。唯一源头是
/// `byteui::theme::geometry::browser_chrome_top_px()`,宿主的 webview 几何也读它;
/// 这里只是转发,不另算一份。
pub fn chrome_top_px() -> f32 {
    byteui::theme::geometry::browser_chrome_top_px()
}
