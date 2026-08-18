//! amis `status`(成功/失败/进行中状态展示):<https://baidu.github.io/amis/zh-CN/components/status>

use iced_widget::core::{Color, Element};
use iced_widget::{container, row, text};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Success,
    Failed,
    Running,
    Idle,
}

impl Kind {
    fn color(self, colors: &crate::theme::color::ColorTokens) -> Color {
        match self {
            Kind::Success => colors.green,
            Kind::Failed => colors.red,
            Kind::Running => colors.cyan,
            Kind::Idle => colors.dim,
        }
    }
}

pub fn view<'a, Message: 'a>(
    kind: Kind,
    label: &'a str,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let colors = crate::theme::color::current();
    let dot_color = kind.color(&colors);
    row![
        container(text("●").size(8).color(dot_color)),
        text(label).size(crate::theme::font::body()).color(colors.cream),
    ]
    .spacing(6)
    .align_y(iced_widget::core::alignment::Vertical::Center)
    .into()
}
