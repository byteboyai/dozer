//! amis `property`(key-value 属性网格):<https://baidu.github.io/amis/zh-CN/components/property>

use iced_widget::core::{Element, Length};
use iced_widget::{Column, Row, text};

pub fn row<'a, Message: 'a>(
    key: &'a str,
    value: &'a str,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let colors = crate::theme::color::current();
    Row::new()
        .spacing(8)
        .push(
            text(key)
                .size(crate::theme::font::caption())
                .color(colors.dim)
                .width(Length::FillPortion(2)),
        )
        .push(
            text(value)
                .size(crate::theme::font::body())
                .color(colors.cream)
                .width(Length::FillPortion(3)),
        )
        .into()
}

pub fn view<'a, Message: 'a>(
    items: Vec<(&'a str, &'a str)>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let rows: Vec<_> = items.into_iter().map(|(k, v)| row(k, v)).collect();
    Column::with_children(rows).spacing(6).into()
}
