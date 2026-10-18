//! 视图向产品要的展示能力(形状同 Project / Todo / Usage 三个面板库的 `PanelDisplay`)。
//!
//! 库只画预览面板自己的结构(页签条、工具栏、查找条、降级页……),凡是依赖产品 chrome 的东西——
//! 页签部件、悬停动画表、折叠按钮、配色区域、顶层消息类型——都经这个 trait 注入。

use std::path::PathBuf;

use dozer_model::LocalizedText;
use iced_widget::core::{Color, Element as CoreElement, Padding};

use crate::{Message, ViewerPanel};

pub type Element<'a, M> = CoreElement<'a, M, iced_widget::Theme, iced_renderer::Renderer>;

/// 预览面板里有悬停动画的部件。产品把它们一一映射到自己的悬停表键。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PreviewHover {
    TabItem(usize),
    TabClose(usize),
    TabOverflow,
    TabOverflowRow(usize),
    RenderMode,
    TabularMode,
    JsonMode,
    FindNext,
    FindPrev,
    FindReplaceToggle,
    FindReplaceCurrent,
    FindReplaceAll,
}

/// 面板底板的样式(Dozer:`theme::region::preview_pane()`)。
#[derive(Debug, Clone, Copy)]
pub struct PaneStyle {
    pub padding: Padding,
    pub gap: f32,
    pub background: Option<Color>,
}

/// 一个页签部件的入参(产品用自己的页签部件画)。
pub struct TabArgs<'a, M> {
    pub title: String,
    pub active: bool,
    pub hover_t: f32,
    pub close_hover_t: f32,
    pub on_select: M,
    pub on_close: M,
    pub show_tooltip: bool,
    pub title_hover: Box<dyn Fn(bool) -> M + 'a>,
    pub close_hover: Box<dyn Fn(bool) -> M + 'a>,
}

/// 溢出下拉的一行。
pub struct OverflowEntry {
    pub index: usize,
    pub title: String,
    pub active: bool,
    /// `Blank` 占位固定存在、不可关闭,这一行不画 ×。
    pub closable: bool,
    pub hover_t: f32,
}

pub struct OverflowMenuArgs<'a, M> {
    pub entries: Vec<OverflowEntry>,
    pub anchor: (f32, f32),
    pub on_select: Box<dyn Fn(usize) -> M + 'a>,
    pub on_close: Box<dyn Fn(usize) -> M + 'a>,
    pub on_dismiss: M,
    pub on_row_hover: Box<dyn Fn(usize, bool) -> M + 'a>,
}

pub type ActionButtonStyle =
    Box<dyn Fn(&iced_widget::Theme, iced_widget::button::Status) -> iced_widget::button::Style>;

pub trait PanelDisplay<M: Clone + 'static> {
    /// 库消息 → 产品顶层消息(Files 与 Project 各自的路由)。
    fn wrap(&self, panel: ViewerPanel, message: Message) -> M;
    fn locale(&self) -> &str;
    /// 产品名(降级页文案里的 `{product}`)。
    fn product_name(&self) -> &str;
    /// 渲染面板级错误文案(可能带库以外的 owner,由产品的解析器渲染)。
    fn render_text(&self, text: &LocalizedText) -> String;
    fn pane_style(&self) -> PaneStyle;
    fn hover_progress(&self, panel: ViewerPanel, hover: PreviewHover) -> f32;
    fn hover_tooltip_ready(&self, panel: ViewerPanel, hover: PreviewHover) -> bool;
    fn hover_message(&self, panel: ViewerPanel, hover: PreviewHover, hovered: bool) -> M;
    /// 页签条的真实可用宽度(翻页窗口数学用)。
    fn tab_bar_avail_px(&self, panel: ViewerPanel) -> f32;
    /// 页签标题的估算展示宽度。
    fn tab_display_width(&self, title: &str) -> f32;
    fn tab_divider<'a>(&self) -> Element<'a, M>;
    fn panel_tab<'a>(&self, args: TabArgs<'a, M>) -> Element<'a, M>;
    /// 给一个页签套上产品的拖拽换位 / 右键菜单手势。`path` 仅文件页签有。
    fn tab_gestures<'a>(
        &self,
        panel: ViewerPanel,
        index: usize,
        tab: Element<'a, M>,
        path: Option<PathBuf>,
    ) -> Element<'a, M>;
    fn overflow_button<'a>(
        &self,
        tab_count: usize,
        hover_t: f32,
        on_press: M,
        on_hover: Box<dyn Fn(bool) -> M + 'a>,
    ) -> Option<Element<'a, M>>;
    fn render_mode_button<'a>(
        &self,
        in_code_mode: bool,
        hover_t: f32,
        on_press: M,
        on_hover: Box<dyn Fn(bool) -> M + 'a>,
    ) -> Element<'a, M>;
    fn tabular_mode_button<'a>(
        &self,
        in_text: bool,
        hover_t: f32,
        on_press: M,
        on_hover: Box<dyn Fn(bool) -> M + 'a>,
    ) -> Element<'a, M>;
    fn json_mode_button<'a>(
        &self,
        in_text: bool,
        hover_t: f32,
        on_press: M,
        on_hover: Box<dyn Fn(bool) -> M + 'a>,
    ) -> Element<'a, M>;
    /// 右上角「收起 / 展开列表列」按钮(Files:文件树;Project:信息列)。
    fn collapse_button<'a>(&self, panel: ViewerPanel) -> Element<'a, M>;
    fn overflow_menu<'a>(&self, args: OverflowMenuArgs<'a, M>) -> Element<'a, M>;
    /// 降级页 / 失败条的操作按钮样式。
    fn action_button_style(&self, text_color: Color) -> ActionButtonStyle;
    /// 查找条输入文字的字号(Dozer:终端字号 × 图标缩放)。
    fn find_font_size(&self) -> f32;
    /// 统一行高(Dozer:终端行高倍数)。
    fn line_height_factor(&self) -> f32;
}
