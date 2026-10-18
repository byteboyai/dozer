//! 产品专属语义的注入点。库本身不依赖 `dozer-app` / `dozer-client` / `dozer-core`;
//! 产品(Dozer / Digger)在这里把"面板外观"与"文本框菜单目标"接进来。

use iced_widget::core::{Element, Padding};

pub use dozer_panel_host::{Notice, NoticeLevel, NoticeOutbox};

/// 产品给 Project 视图的展示能力(形状沿用 `dozer-panel-todo` / `dozer-panel-usage`
/// 的同名 trait,只保留 Project 单栏实际用到的几项)。
///
/// 泛型 `M` 是产品的面板 `Message` 类型。视图写 `display: &impl PanelDisplay<M>`,
/// 不 import 宿主的 `App`。
pub trait PanelDisplay<M> {
    /// 把库内的面板 [`crate::state::Message`] 包成产品的顶层 `Message`(通常是
    /// 产品 `Message` 里 wraps project `Message` 的那个变体)。
    fn wrap(&self, message: crate::state::Message) -> M;
    /// 面板头(图标 + 标题 + 分割线)。`title` 是已按当前 locale 解析好的展示文本。
    fn panel_head<'a>(
        &self,
        icon: byteui::interaction::icons::IconKind,
        title: String,
    ) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>;
    /// 面板头与分割线外侧的内边距(Dozer 取自 `workspace.json` 的 `project_pane` 区域)。
    fn header_padding(&self) -> Padding;
    /// 两个 pane 之间的水平分割线。
    fn tab_divider<'a>(&self) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>;
    /// 当前生效语言(BCP-47)。
    fn locale(&self) -> &str {
        "en"
    }
}

/// 文本框的右键菜单目标:库只发出这个不透明目标,由产品转成自己的菜单请求。
/// 形状与 Dozer `app::TextInputTarget` 相同。
#[derive(Debug, Clone)]
pub struct TextInputTarget {
    pub id: iced_widget::core::widget::Id,
    pub secure: bool,
}
