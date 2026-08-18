//! amis `wrapper`(无装饰的间距包裹):<https://baidu.github.io/amis/zh-CN/components/wrapper>

use iced_widget::container;
use iced_widget::core::Element;

pub fn view<'a, Message: 'a>(
    content: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>,
    padding: f32,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    container(content).padding(padding).into()
}
