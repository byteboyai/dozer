//! amis `panel`(带描边圆角的信息容器):<https://baidu.github.io/amis/zh-CN/components/panel>

use iced_widget::container;
use iced_widget::core::{Border, Element};

pub fn view<'a, Message: 'a>(
    content: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    container(content)
        .padding(12)
        .style(|_theme: &iced_widget::Theme| {
            let colors = crate::theme::color::current();
            container::Style {
                background: Some(colors.card.into()),
                border: Border {
                    color: colors.border,
                    width: 1.0,
                    radius: 8.0.into(),
                },
                ..container::Style::default()
            }
        })
        .into()
}
