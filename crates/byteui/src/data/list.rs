//! amis `list`(列表项行):<https://baidu.github.io/amis/zh-CN/components/list>

use iced_widget::core::{Element, Length};
use iced_widget::{Row, container, text};

pub fn item<'a, Message: 'a>(
    leading: Option<Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>>,
    label: &'a str,
    trailing: Option<Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let colors = crate::theme::color::current();
    let mut r = Row::new()
        .spacing(8)
        .align_y(iced_widget::core::alignment::Vertical::Center);
    if let Some(l) = leading {
        r = r.push(l);
    }
    r = r.push(
        text(label)
            .size(crate::theme::font::body())
            .color(colors.cream)
            .width(Length::Fill),
    );
    if let Some(t) = trailing {
        r = r.push(t);
    }
    container(r).padding([6, 10]).into()
}
