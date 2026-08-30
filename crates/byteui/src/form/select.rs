//! amis `form/select`(下拉选择):<https://baidu.github.io/amis/zh-CN/components/form/select>

use iced_widget::core::{Border, Element};
use iced_widget::pick_list::{self, Status};
use std::borrow::Borrow;

pub fn view<'a, T, L, V, Message>(
    options: L,
    selected: Option<V>,
    on_select: impl Fn(T) -> Message + 'a,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>
where
    T: ToString + PartialEq + Clone + 'a,
    L: Borrow<[T]> + 'a,
    V: Borrow<T> + 'a,
    Message: Clone + 'a,
{
    iced_widget::pick_list(options, selected, on_select)
        .padding(8)
        .style(|_theme: &iced_widget::Theme, _status: Status| {
            let colors = crate::theme::color::current();
            pick_list::Style {
                text_color: colors.cream,
                placeholder_color: colors.dim,
                handle_color: colors.gold,
                background: colors.card.into(),
                border: Border {
                    color: colors.border,
                    width: 1.0,
                    radius: 6.0.into(),
                },
            }
        })
        .into()
}
