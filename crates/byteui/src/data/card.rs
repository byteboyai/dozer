//! amis `card`(卡片):<https://baidu.github.io/amis/zh-CN/components/card>
//! 复用 `interaction::cards::container_card` 的三态样式(一般/hover/选中),
//! 这里只加标题+副标题的内容排版约定。

use iced_widget::core::Element;
use iced_widget::{Column, container, text};

pub fn view<'a, Message: 'a>(
    title: &'a str,
    subtitle: Option<&'a str>,
    selected: bool,
    hovered: bool,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let colors = crate::theme::color::current();
    let mut col = Column::new().spacing(4).push(
        text(title)
            .size(crate::theme::font::body())
            .color(colors.cream),
    );
    if let Some(sub) = subtitle {
        col = col.push(
            text(sub)
                .size(crate::theme::font::caption())
                .color(colors.dim),
        );
    }
    container(col)
        .padding(12)
        .style(move |_theme: &iced_widget::Theme| {
            crate::interaction::cards::container_card(selected, hovered, colors.card)
        })
        .into()
}
