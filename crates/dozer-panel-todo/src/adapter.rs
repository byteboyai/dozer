//! 产品注入的适配类型。

use bytehost_desktop::HoverSlot;
use iced_widget::core::{Element, Padding};

/// 文本框的右键菜单目标:库只发出这个不透明目标,由产品转成自己的菜单请求。
/// 形状与 Dozer `app::TextInputTarget` 相同。
#[derive(Debug, Clone)]
pub struct TextInputTarget {
    pub id: iced_widget::core::widget::Id,
    pub secure: bool,
}

/// agent 图标的数据(原来 `protocol.rs` 直接调 `workspace::agent_icon` 取字节与"保留原色"标记)。
#[derive(Debug, Clone)]
pub struct AgentIconData {
    pub bytes: Vec<u8>,
    pub preserves_color: bool,
}

/// 产品给 Todo 视图的展示能力(同 `dozer-panel-usage` 的同名 trait,形状沿用,只加 todo 确实需要的)。
/// 视图写 `app: &impl PanelDisplay`,不 import 宿主的 `App`。
pub trait PanelDisplay {
    /// 某个按钮 / 卡片的悬停动画进度 0.0..=1.0。
    fn hover_progress(&self, slot: HoverSlot) -> f32;
    /// 列表列当前是否收起。
    fn list_collapsed(&self) -> bool;
    /// 列表列的折叠 / 展开按钮;`on_select` 与 `on_hover` 由库给出。
    fn list_collapse_button<'a, M: Clone + 'a>(
        &self,
        collapsed: bool,
        tooltip_collapse: String,
        tooltip_expand: String,
        on_select: M,
        on_hover: impl Fn(bool) -> M + 'a,
    ) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>;
    /// 面板头(图标 + 标题 + 分割线)。
    fn panel_head<'a, M: 'a>(
        &self,
        icon: byteui::interaction::icons::IconKind,
        title: String,
    ) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>;
    /// 面板头与分割线外侧的内边距(Dozer 取自 `workspace.json` 的 `project_pane` 区域)。
    fn header_padding(&self) -> Padding;
    /// 两个 pane 之间的水平分割线。
    fn tab_divider<'a, M: 'a>(&self) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>;
    /// 当前生效语言(BCP-47)。
    fn locale(&self) -> &str {
        "en"
    }
}

/// 右区顶栏行的内高:`tab_button_size + 8`。**唯一来源**:视图和产品的 webview 让位几何都读它。
pub fn top_row_inner_h_px() -> f32 {
    byteui::theme::geometry::tab_button_size() + 8.0
}
