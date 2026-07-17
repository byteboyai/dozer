// crates/dozer-app/src/workspace.rs
//! `Workspace` 是 iced 程序状态，承担 spike B 里 `controls.rs` 的角色：
//! 持有 UI 状态、暴露 `view()`/`update()`。它渲染 ByteBoy2077 的四栏
//! 骨架布局（项目 / 预览 / 终端 / AI）。终端栏（本计划 T5 主战场）接了
//! `TerminalModel` + `term_view::view`；项目/预览/AI 三栏仍是 T4 之前
//! 遗留的占位内容，留给 P1d/P1e。
use crate::term_model::TerminalModel;
use crate::term_view;
use crate::theme;
use iced_widget::core::{Border, Color, Element, Length};
use iced_widget::{column, container, row, text};

/// 终端默认网格尺寸（列 x 行）。按 pane 实际像素动态换算
/// （`term_view::grid_size`）留给窗口 resize 接线（T6+）；本任务先用固定
/// 尺寸把"键盘 → keymap → feed → 渲染"这条管线跑通。
const DEFAULT_COLS: u16 = 80;
const DEFAULT_ROWS: u16 = 24;

#[derive(Debug, Clone)]
pub enum Message {
    #[allow(dead_code)] // 暂无其它 UI 消息来源，占位以保持枚举可扩展
    Noop,
    /// 终端聚焦时的键盘/IME 输入字节（已经过 `keymap` 翻译）。本任务先
    /// 直接回灌 `TerminalModel::feed` 做本地 echo，验证渲染管线；daemon
    /// 接线在 T6。
    TermInput(Vec<u8>),
}

pub struct Workspace {
    term: TerminalModel,
    /// 终端是否聚焦（决定光标反色画法）。当前是单窗口应用且没有其它可
    /// 聚焦的输入控件，因此终端默认常驻聚焦。
    term_focused: bool,
}

impl Workspace {
    pub fn new() -> Self {
        Self {
            term: TerminalModel::new(DEFAULT_COLS, DEFAULT_ROWS),
            term_focused: true,
        }
    }

    pub fn update(&mut self, message: Message) {
        match message {
            Message::Noop => {}
            Message::TermInput(bytes) => self.term.feed(&bytes),
        }
    }

    pub fn view(
        &self,
    ) -> iced_widget::core::Element<'_, Message, iced_widget::Theme, iced_widget::Renderer>
    {
        let col1 = pane("项目 · P1e", 240.0, theme::PANEL);
        let col2 = pane("预览 · P1d", 0.0, theme::PANEL); // 0=FILL
        let col3 = terminal_pane(&self.term, self.term_focused);
        let col4 = pane("AI · P1e", 280.0, theme::PANEL);
        row![col1, col2, col3, col4].into()
    }
}

impl Default for Workspace {
    fn default() -> Self {
        Self::new()
    }
}

/// 四栏骨架里的单栏。`width <= 0.0` 表示 Fill，否则是固定逻辑像素宽度。
/// 顶部一行 CREAM 13px 标签文字；用 1px BORDER 描边充当栏间分隔线。
fn pane(
    label: &str,
    width: f32,
    background: Color,
) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let header = text(label).size(13).color(theme::CREAM);

    container(column![header].spacing(4).padding(8))
        .width(if width > 0.0 {
            Length::Fixed(width)
        } else {
            Length::Fill
        })
        .height(Length::Fill)
        .style(move |_theme: &iced_widget::Theme| container::Style {
            background: Some(background.into()),
            border: Border {
                color: theme::BORDER,
                width: 1.0,
                radius: 0.0.into(),
            },
            ..container::Style::default()
        })
        .into()
}

/// 终端栏：复用 `pane` 的标签+边框外观，正文换成 `term_view::view` 渲染
/// 的终端网格，而不是占位文字。
fn terminal_pane(
    term: &TerminalModel,
    focused: bool,
) -> Element<'_, Message, iced_widget::Theme, iced_widget::Renderer> {
    let header = text("终端 · 本计划").size(13).color(theme::CREAM);

    container(
        column![header, term_view::view(term, focused)]
            .spacing(4)
            .padding(8),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .style(move |_theme: &iced_widget::Theme| container::Style {
        background: Some(theme::TERM_BG.into()),
        border: Border {
            color: theme::BORDER,
            width: 1.0,
            radius: 0.0.into(),
        },
        ..container::Style::default()
    })
    .into()
}
