//! amis `flex`(css flex 排列封装):<https://baidu.github.io/amis/zh-CN/components/flex>

use iced_widget::core::Element;
use iced_widget::{Column, Row};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Row,
    Column,
}

pub fn view<'a, Message: 'a>(
    direction: Direction,
    gap: f32,
    children: Vec<Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    match direction {
        Direction::Row => Row::with_children(children).spacing(gap).into(),
        Direction::Column => Column::with_children(children).spacing(gap).into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    enum Msg {}

    #[test]
    fn view_row_and_column_construct_without_panic() {
        let child = || -> Element<'static, Msg, iced_widget::Theme, iced_renderer::Renderer> {
            iced_widget::text("x").into()
        };
        let _ = view(Direction::Row, 8.0, vec![child(), child()]);
        let _ = view(Direction::Column, 8.0, vec![child(), child()]);
    }
}
