//! 产品展示适配(S6c-2 / D9):面板头、折叠按钮、页签部件、区域度量、
//! 悬停进度、终端字体与焦点由产品注入(页签部件暂不迁入 byteui)。

use crate::update::Message;
use byteui::interaction::icons::IconKind;
use dozer_panel_term::style::TermStyle;
use dozer_panel_term::view::TermEvent;
use iced_widget::Theme;
use iced_widget::core::{Color, Element, Padding};

pub type El<'a, M> = Element<'a, M, Theme, iced_renderer::Renderer>;

/// 库自有悬停槽位(产品映射到自己的 `HoverId`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentHover {
    PickerToggle,
    TabItem(usize),
    TabClose(usize),
    TabOverflow,
    OverflowRow(usize),
}

/// 区域外观(间距/内边距/背景)。
#[derive(Debug, Clone, Copy)]
pub struct PaneStyle {
    pub gap: f32,
    pub padding: Padding,
    pub background: Option<Color>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentPane {
    List,
    Terminal,
}

/// 单个页签的展示参数。
pub struct TabArgs<M> {
    pub title: String,
    pub active: bool,
    pub dot: Color,
    pub item: AgentHover,
    pub close: AgentHover,
    pub on_select: M,
    pub on_close: M,
}

/// 溢出下拉一行。
pub struct OverflowEntry {
    pub index: usize,
    pub title: String,
    pub active: bool,
    pub dot: Color,
    pub row: AgentHover,
}

/// 产品展示适配。所有方法只负责"产品自己的部件",库负责布局与文案。
pub trait PanelDisplay<'a, M: Clone + 'a> {
    fn wrap(&self, message: Message) -> M;
    fn locale(&self) -> &str;
    fn hover_progress(&self, slot: AgentHover) -> f32;
    /// 悬停状态变化对应的产品消息(避免视图闭包捕获 `&self` 而拿到非
    /// `'static` 生命周期)。
    fn hover_message(&self, slot: AgentHover, hovered: bool) -> M;
    fn hover_tooltip_ready(&self, slot: AgentHover) -> bool;
    fn pane_style(&self, pane: AgentPane) -> PaneStyle;

    fn panel_head(&self, icon: IconKind, title: &'a str, actions: Option<El<'a, M>>) -> El<'a, M>;
    fn list_collapse_button(&self, tooltip_collapse: String, tooltip_expand: String) -> El<'a, M>;
    fn tab_divider(&self) -> El<'a, M>;

    fn panel_tab(&self, args: TabArgs<M>) -> El<'a, M>;
    fn overflow_button(&self, tab_count: usize, on_press: M) -> Option<El<'a, M>>;
    fn overflow_menu(
        &self,
        entries: Vec<OverflowEntry>,
        anchor: (f32, f32),
        on_select: fn(usize) -> Message,
        on_close: fn(usize) -> Message,
        on_dismiss: Message,
    ) -> El<'a, M>;
    fn drag_surface(&self, content: El<'a, M>, index: usize) -> El<'a, M>;

    fn term_style(&self) -> &'a TermStyle;
    fn terminal_focused(&self) -> bool;
    fn ime_preedit(&self) -> Option<&'a str>;
    fn term_event(&self, event: TermEvent) -> M;
    /// 不含捕获的事件映射函数指针(终端 canvas 需要 `'static` 映射)。
    fn term_event_fn(&self) -> fn(TermEvent) -> M;
    fn tab_bar_avail_px(&self) -> f32;
}

/// 终端条里由产品塞入的两块(「发送到 Agent」上下文条与展开面板)。
pub struct TerminalSlots<'a, M> {
    pub floating: Option<Element<'a, M, Theme, iced_renderer::Renderer>>,
    pub footer: Option<Element<'a, M, Theme, iced_renderer::Renderer>>,
}
