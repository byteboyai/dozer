//! amis `divider`(分隔线):<https://baidu.github.io/amis/zh-CN/components/divider>

use iced_widget::core::Element;
use iced_widget::rule::{self, FillMode};

pub fn horizontal<'a, Message: 'a>()
-> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    rule::horizontal(1.0)
        .style(|_theme: &iced_widget::Theme| rule::Style {
            color: crate::theme::color::current().border,
            radius: 0.0.into(),
            fill_mode: FillMode::Full,
            snap: true,
        })
        .into()
}

pub fn vertical<'a, Message: 'a>()
-> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    rule::vertical(1.0)
        .style(|_theme: &iced_widget::Theme| rule::Style {
            color: crate::theme::color::current().border,
            radius: 0.0.into(),
            fill_mode: FillMode::Full,
            snap: true,
        })
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    enum Msg {}

    #[test]
    fn horizontal_and_vertical_construct_without_panic() {
        let _: Element<Msg, iced_widget::Theme, iced_renderer::Renderer> = horizontal();
        let _: Element<Msg, iced_widget::Theme, iced_renderer::Renderer> = vertical();
    }
}
