// crates/dozer-app/src/workspace.rs
//! `Workspace` 是 iced 程序状态，承担 spike B 里 `controls.rs` 的角色：
//! 持有 UI 状态、暴露 `view()`/`update()`。它渲染 ByteBoy2077 的四栏
//! 骨架布局（项目 / 预览 / 终端 / AI），当前均为占位内容——
//! 真正的终端渲染是本计划（P1c）后续任务的主战场。
use crate::theme;
use iced_widget::core::{Border, Color, Element, Length};
use iced_widget::{column, container, row, text};

#[derive(Debug, Clone)]
pub enum Message {
    #[allow(dead_code)] // Task 5/6 才会真正构造（键盘/resize 消息）
    Noop,
}

pub struct Workspace;

impl Workspace {
    pub fn new() -> Self {
        Self
    }

    pub fn update(&mut self, _message: Message) {}

    pub fn view(
        &self,
    ) -> iced_widget::core::Element<'_, Message, iced_widget::Theme, iced_widget::Renderer>
    {
        let col1 = pane("项目 · P1e", 240.0, theme::PANEL);
        let col2 = pane("预览 · P1d", 0.0, theme::PANEL); // 0=FILL
        let col3 = pane("终端 · 本计划", 0.0, theme::TERM_BG);
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
