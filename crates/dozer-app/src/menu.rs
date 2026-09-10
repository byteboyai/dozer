//! 统一弹出菜单(popup menu)样式原语。
//!
//! 以文件树面板右键菜单(context menu)为**唯一基准**——本模块之前它写在
//! `extensions/files.rs::menu_item`/`menu_separator`/`context_menu_popup`,
//! 各面板要么"就地照抄"(ssh/sftp、git_log)、要么另起一套(CARD 底 + 圆角
//! 6/8 + 自定内边距的 agent 选择器、浏览器星标、数据库驱动、todo 派发等),
//! 视觉与基准逐处漂移。现在把这些原语抽出来共享:
//!
//! - 外壳 `shell`: `theme::region::context_menu()`(底色/描边/圆角/内边距/
//!   项间距 + `menu_item_width` 的恒定菜单总宽),已含全局 scale。
//! - 单项 `item`: 恒定宽 `menu_item_width` 的"图标+CREAM 文字";`item_row`
//!   接受任意前置元素(图标/指示点/复选框…)+ 自定义文字色 + 可选点击/锁定;
//!   `item_row_fill` 为整行撑满(`Fill`)的版本(窄面板内选择器用)。三者统一
//!   `menu_pad_v/h` 内外边距 + `menu_gap` 图标↔文字间距;hover/pressed →
//!   `TAB_HOVER` 底色 + 圆角 4。
//! - 锁定 `item_locked`/`msg: None`: 置灰且不可点、hover 不高亮。
//! - 分组 `separator`: 一条 1px `BORDER` 分隔线。
//!
//! 所有原语对消息类型 `Msg` 泛型化,任何面板的 `Message` 都能直接复用,
//! 不再为每个面板各写一份 `menu_item`。

use crate::theme;
use byteui::interaction::icons;
use iced_widget::core::{Border, Color, Element, Length, Shadow, Vector};
use iced_widget::{button, column, container, row, text};

/// macOS 原生右键菜单的高亮/命中区是较大的圆角矩形(而非直角),`item`/
/// `item_row` 的 hover/pressed 态、`item_row_fill` 一并复用这个半径。
const MENU_HOVER_RADIUS: f32 = 6.0;

/// 单个菜单项:可选图标 + 文字。常宽固定(菜单不随 label 长短伸缩),hover/
/// pressed 切到 `TAB_HOVER` 底。文字与图标取默认 `CREAM`(图标无色的只用
/// 文字行)。`msg` 为点击下发消息。
pub fn item<'a, Msg: 'a + Clone>(
    icon: Option<icons::IconKind>,
    label: impl Into<String>,
    msg: Msg,
) -> Element<'a, Msg, iced_widget::Theme, iced_renderer::Renderer> {
    item_row(
        icon_leading(icon, byteui::theme::color::current().cream),
        label,
        byteui::theme::color::current().cream,
        Some(msg),
    )
}

/// 置灰且不可点的菜单项(如剪贴板为空时的"粘贴"、dirty 锁定的非当前分支):
/// 无 `on_press`,hover 也不高亮,始终透出容器底。文字/图标取 `color`
/// (调用方置 `DIM`)。
pub fn item_locked<'a, Msg: 'a + Clone>(
    icon: Option<icons::IconKind>,
    label: impl Into<String>,
    color: Color,
) -> Element<'a, Msg, iced_widget::Theme, iced_renderer::Renderer> {
    item_row(icon_leading(icon, color), label, color, None)
}

/// 最通用的菜单项:接受任意**前置元素**(图标、状态指示点、复选框…)+
/// 文字。行为同其它项(常宽、hover 高亮、可锁定)。`leading` 为 `None` 时
/// 只渲染文字。`msg` 为 `Some` 挂点击并 hover 高亮,`None` 表示锁定/置灰。
pub fn item_row<'a, Msg: 'a + Clone>(
    leading: Option<Element<'a, Msg, iced_widget::Theme, iced_renderer::Renderer>>,
    label: impl Into<String>,
    color: Color,
    msg: Option<Msg>,
) -> Element<'a, Msg, iced_widget::Theme, iced_renderer::Renderer> {
    let content = match leading {
        Some(leading) => row![
            leading,
            text(label.into())
                .size(byteui::theme::font::body())
                .color(color)
        ],
        None => row![
            text(label.into())
                .size(byteui::theme::font::body())
                .color(color)
        ],
    };
    menu_button(
        content,
        color,
        msg,
        Length::Fixed(byteui::theme::geometry::menu_item_width()),
    )
}

/// 满宽菜单项:同上 `item_row`,但按键/整行撑满可用宽度(`Fill`),供需要整行
/// 可点区域的窄面板内选择器(如 Git 面板的分支下拉)使用;外观、hover、锁定
/// 语义与固定宽菜单项完全一致。
pub fn item_row_fill<'a, Msg: 'a + Clone>(
    leading: Option<Element<'a, Msg, iced_widget::Theme, iced_renderer::Renderer>>,
    label: impl Into<String>,
    color: Color,
    msg: Option<Msg>,
) -> Element<'a, Msg, iced_widget::Theme, iced_renderer::Renderer> {
    let content = match leading {
        Some(leading) => row![
            leading,
            text(label.into())
                .size(byteui::theme::font::body())
                .color(color)
        ],
        None => row![
            text(label.into())
                .size(byteui::theme::font::body())
                .color(color)
        ],
    };
    menu_button(content, color, msg, Length::Fill)
}

/// 把可选的图标转成前置元素(`None` = 无图标,只渲染文字)。
fn icon_leading<'a, Msg: 'a>(
    icon: Option<icons::IconKind>,
    color: Color,
) -> Option<Element<'a, Msg, iced_widget::Theme, iced_renderer::Renderer>> {
    icon.map(|icon| icons::view(icon, byteui::theme::icon_size::row(), color))
}

/// 菜单项之间的细分隔线:1px `BORDER` 高度,宽同菜单常宽(与 macOS 系统菜单
/// 分组线同款)。
pub fn separator<'a, Msg: 'a>() -> Element<'a, Msg, iced_widget::Theme, iced_renderer::Renderer> {
    container(byteui::layout::divider::horizontal())
        .width(Length::Fixed(byteui::theme::geometry::menu_item_width()))
        .into()
}

/// 菜单外壳:把一列菜单项(或分隔线)包进 `context_menu` 区域样式
/// (底色/1px 描边/圆角/内边距 + 项间距)。返回可再套一层定位容器
/// (左上线锚 `Padding{top,left}` 或底线锚 `align_y(Bottom)`+下 padding)
/// 来摆放。`width` 通常传 `Length::Fixed(menu_item_width())` 得到常宽菜单;
/// 需要内容自适应宽的条目(复选框列表、星标、todo 派发等)可传 `Shrink`。
pub fn shell<'a, Msg: 'a>(
    items: Vec<Element<'a, Msg, iced_widget::Theme, iced_renderer::Renderer>>,
    width: Length,
) -> Element<'a, Msg, iced_widget::Theme, iced_renderer::Renderer> {
    let region = theme::region::context_menu();
    container(column(items).spacing(region.gap))
        .width(width)
        .padding(region.padding)
        .style(move |_t: &iced_widget::Theme| container::Style {
            background: region.background.map(Into::into),
            border: region.border.unwrap_or_default(),
            // macOS 原生右键菜单靠系统合成器的大模糊投影把浮层和背景内容
            // 拉开层次;iced 没有实时高斯模糊可用,这里退而求其次用一圈软阴影
            // 模拟同样的"浮起"观感。
            shadow: Shadow {
                color: Color {
                    a: 0.45,
                    ..Color::BLACK
                },
                offset: Vector::new(0.0, 10.0),
                blur_radius: 28.0,
            },
            ..container::Style::default()
        })
        .into()
}

/// 菜单单项按钮的通用样式接线:透明底(透出容器底色)、文字 `base_color`、
/// hover/pressed 切 `TAB_HOVER` 底 + 圆角 4。`Some(msg)` 挂 `on_press`,
/// `None` 表示锁定/置灰项(不可点、不高亮)。
fn menu_button<'a, Msg: 'a + Clone>(
    content: iced_widget::Row<'a, Msg, iced_widget::Theme, iced_renderer::Renderer>,
    base_color: Color,
    msg: Option<Msg>,
    width: Length,
) -> Element<'a, Msg, iced_widget::Theme, iced_renderer::Renderer> {
    let content = content
        .spacing(byteui::theme::geometry::menu_gap())
        .align_y(iced_widget::core::Alignment::Center);
    let enabled = msg.is_some();
    let btn = button(content)
        .width(width)
        .padding([
            byteui::theme::geometry::menu_pad_v(),
            byteui::theme::geometry::menu_pad_h(),
        ])
        .style(move |_t: &iced_widget::Theme, s: button::Status| {
            let base = button::Style {
                background: None,
                text_color: base_color,
                ..button::Style::default()
            };
            let hovered =
                matches!(s, button::Status::Hovered) || matches!(s, button::Status::Pressed);
            if enabled && hovered {
                button::Style {
                    background: Some(byteui::theme::color::current().tab_hover.into()),
                    text_color: base_color,
                    border: Border {
                        color: Color::TRANSPARENT,
                        width: 0.0,
                        radius: MENU_HOVER_RADIUS.into(),
                    },
                    ..base
                }
            } else {
                base
            }
        });
    match msg {
        Some(msg) => btn.on_press(msg.clone()).into(),
        None => btn.into(),
    }
}
