//! 统一搜索框:文本输入框 + 内嵌搜索按钮共享同一圈边框,参照首页项目列表
//! 搜索框(dozer-app `homespace::home_project_list_view`)的原始设计——
//! 输入框本身透明无边框,由外层容器统一画一圈边框/底色;边框高亮由
//! `highlight` 参数决定(调用方通常传"真实聚焦 || 已生效的搜索词非空"),
//! 不看 iced 内部 `text_input::Status`。dozer-app 里 Todo/文件树/Git Log/
//! 会话列表等面板的搜索框统一收敛到这份实现,避免各面板各画一套、视觉
//! 逐渐漂移。

use crate::interaction::icons;
use iced_widget::core::widget;
use iced_widget::core::{Border, Color, Element, Length};

#[allow(clippy::too_many_arguments)]
pub fn view<'a, Message: Clone + 'a>(
    placeholder: &str,
    value: &'a str,
    id: Option<widget::Id>,
    highlight: bool,
    on_input: impl Fn(String) -> Message + 'a,
    on_submit: Message,
    submit_hover_t: f32,
    on_submit_hover: impl Fn(bool) -> Message + 'a,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    view_inner(
        placeholder,
        value,
        id,
        highlight,
        on_input,
        on_submit,
        submit_hover_t,
        on_submit_hover,
        None,
    )
}

/// `view` 的变体:允许在输入区左前方(同一圈边框内)内嵌一个引导位
/// (`prefix`,通常是分类筛选等带下拉的 segment)。`None` 时与 `view`
/// 完全等同——现有无引导位的调用方无需改任何代码。带 `prefix` 时常配
/// 合屏幕点选的下拉浮层使用(浮层由调用方另叠)。
#[allow(clippy::too_many_arguments)]
pub fn view_with_prefix<'a, Message: Clone + 'a>(
    placeholder: &str,
    value: &'a str,
    id: Option<widget::Id>,
    highlight: bool,
    on_input: impl Fn(String) -> Message + 'a,
    on_submit: Message,
    submit_hover_t: f32,
    on_submit_hover: impl Fn(bool) -> Message + 'a,
    prefix: Option<Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    view_inner(
        placeholder,
        value,
        id,
        highlight,
        on_input,
        on_submit,
        submit_hover_t,
        on_submit_hover,
        prefix,
    )
}

#[allow(clippy::too_many_arguments)]
fn view_inner<'a, Message: Clone + 'a>(
    placeholder: &str,
    value: &'a str,
    id: Option<widget::Id>,
    highlight: bool,
    on_input: impl Fn(String) -> Message + 'a,
    on_submit: Message,
    submit_hover_t: f32,
    on_submit_hover: impl Fn(bool) -> Message + 'a,
    prefix: Option<Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    let colors = crate::theme::color::current();
    let mut input = iced_widget::text_input(placeholder, value)
        .on_input(on_input)
        .on_submit(on_submit.clone())
        .size(crate::theme::font::body())
        .padding(0)
        .style(move |_t: &iced_widget::Theme, _status| {
            // 边框/底色由外层容器统一画(见下),输入框自己恒透明——
            // 不看 `status.focused`,高亮完全交给调用方传入的 `highlight`。
            iced_widget::text_input::Style {
                background: Color::TRANSPARENT.into(),
                border: Border {
                    color: Color::TRANSPARENT,
                    width: 0.0,
                    radius: 0.0.into(),
                },
                icon: colors.dim,
                placeholder: colors.dim,
                value: colors.cream,
                selection: crate::theme::color::mix(colors.gold, colors.card, 0.6),
            }
        });
    if let Some(id) = id {
        input = input.id(id);
    }
    let field: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> = input.into();

    let submit_button = icons::icon_button_entry(
        icons::IconKind::Search,
        crate::theme::icon_size::row(),
        false,
        false,
        submit_hover_t,
        true,
        crate::theme::icon_size::row() + 12.0,
        true,
        on_submit,
        on_submit_hover,
        "搜索",
    );

    let content_h = crate::theme::icon_size::row() + 12.0;
    // `spacing` 只在带前缀时拉开:无前缀时保持"输入区紧贴右侧提交按钮"的
    // 既有观感(默认 `view` 行为不变),避免旁支搜索框整体内距被改动。
    let with_prefix = prefix.is_some();
    let prefix_cell: Option<Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>> =
        prefix.map(|p| {
            // 引导位独占一行内容高,与输入区共用外圈边框;右侧留一条缝与文字
            // 区分(分割线由调用方自带,这里克制不加第二道竖线)。
            iced_widget::container(p)
                .height(Length::Fill)
                .align_y(iced_widget::core::alignment::Vertical::Center)
                .into()
        });
    iced_widget::container(
        iced_widget::row![
            prefix_cell,
            iced_widget::container(field)
                .width(Length::Fill)
                .height(Length::Fill)
                .align_y(iced_widget::core::alignment::Vertical::Center)
                .align_x(iced_widget::core::alignment::Horizontal::Left),
            iced_widget::container(submit_button)
                .align_y(iced_widget::core::alignment::Vertical::Center),
        ]
        .spacing(if with_prefix { 8.0 } else { 0.0 })
        .width(Length::Fill)
        .height(Length::Fixed(content_h))
        .align_y(iced_widget::core::Alignment::Center),
    )
    .width(Length::Fill)
    .height(Length::Fixed(content_h + 12.0))
    .padding([6, 8])
    .style(
        move |_t: &iced_widget::Theme| iced_widget::container::Style {
            background: Some(colors.bg.into()),
            border: Border {
                color: if highlight {
                    colors.gold
                } else {
                    colors.border
                },
                width: 1.0,
                radius: 4.0.into(),
            },
            ..iced_widget::container::Style::default()
        },
    )
    .into()
}
