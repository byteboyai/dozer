//! tab 的共享交互内核——选中(mousedown 即选中+备拖)与关闭按钮(仅悬停时
//! 可点)。只管交互接线，不管布局/尺寸/背景：调用方(`project_tab_item`/
//! 未来的 `panel_tab`)各自决定怎么拼 `stack!`/`row!`、怎么画背景。见
//! `docs/superpowers/specs/2026-08-12-tab-icon-button-shared-components-design.md`。

use iced_widget::core::mouse;
use iced_widget::core::{Color, Element, Length};
use iced_widget::{MouseArea, button, container, text};

/// 返回 `(select, close)` 两个独立 `Element`。`content` 是调用方已经拼好
/// 的 tab 主体(通常是状态点+标题的一个 row，包在合适尺寸的 container
/// 里)。`close_color`/`close_interactive` 由调用方预先算好传入——两个
/// 现有实现(`project_tab_item`/`panel_tab`)的 hover 混色公式不完全相同
/// (项目页签用"标题 hover 与关闭 hover 取最大值"的组合 alpha,面板 tab
/// 只用自己的 `close_hover_t`),这个差异保留在调用方，`tab_core` 不替
/// 调用方做选择。
#[allow(clippy::too_many_arguments)]
pub fn tab_core<'a, M: Clone + 'a>(
    content: Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>,
    close_sz: f32,
    close_color: Color,
    close_interactive: bool,
    on_select: M,
    on_close: M,
    on_select_hover: impl Fn(bool) -> M + 'a,
    on_close_hover: impl Fn(bool) -> M + 'a,
) -> (
    Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>,
    Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>,
) {
    let select = MouseArea::new(content)
        .on_press(on_select)
        .on_enter(on_select_hover(true))
        .on_exit(on_select_hover(false))
        .interaction(mouse::Interaction::Pointer)
        .into();

    let close_btn = button(
        container(
            text("×")
                .size(crate::theme::font::body())
                .color(close_color),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(iced_widget::core::alignment::Horizontal::Center)
        .align_y(iced_widget::core::alignment::Vertical::Center),
    )
    .width(Length::Fixed(close_sz))
    .height(Length::Fixed(close_sz))
    .padding(0)
    .style(move |_t: &iced_widget::Theme, _s| button::Style {
        background: None,
        text_color: close_color,
        ..button::Style::default()
    });
    // 仅在悬停时挂 `on_press`——悬停进度刚起步(>0.001)就立刻可点,鼠标离开
    // 后随进度归零变回不可点,既不误吞点击也不影响正常关闭。
    let close_btn = if close_interactive {
        close_btn.on_press(on_close)
    } else {
        close_btn
    };
    let close = MouseArea::new(close_btn)
        .on_enter(on_close_hover(true))
        .on_exit(on_close_hover(false))
        .into();

    (select, close)
}
