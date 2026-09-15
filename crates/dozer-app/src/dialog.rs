//! 统一弹窗(确认框/模态对话框)样式原语。
//!
//! 之前各面板各写一份"CARD 底 + 描边 + 遮罩"(文件树删除/移动确认、
//! 项目删除/修复进度、SSH 删主机确认、搜索弹窗、Todo 详情),描边色统一
//! 用中性 `BORDER`、圆角在 6/8 之间漂移、遮罩有的补了半透明 `SCRIM`
//! 有的干脆透明——视觉与"弹窗该有的分量感"逐处不一致。现在把外壳原语
//! 收拢成共享的两件套:
//!
//! - `card_style`: 弹窗卡片本体的容器样式(`theme::region::dialog()`——
//!   CARD 底 + 金色 `GOLD` 描边,呼应放大态浮层同款"金色描边盒",见
//!   `theme::region::maximize_overlay`)。
//! - `scrim`/`scrim_blocking`: 满窗遮罩,`SCRIM` 半透明底上叠一层
//!   [`crate::frosted::noise_layer`] 磨砂噪点贴图(同右键菜单
//!   `menu::shell_frosted` 的做法)。`scrim` 挂 `on_press` 可点击关闭;
//!   `scrim_blocking` 不挂,用于进行中不许中途打断的进度弹窗(如项目
//!   "修复"逐步骤跑完前)。弹窗都是打开/关闭才重绘一次的静态浮层,噪点层
//!   的重绘开销可忽略,同 `shell_frosted` 文档的理由。
//! - `actions`: 弹窗底部"取消/确认"这类操作按钮行的统一落位——靠右下角
//!   (之前各面板要么整行左对齐、要么干脆没套统一约定,漂移同上)。只用于
//!   纯按钮的收尾操作行;像 Todo 详情"回复框+提交"那种输入控件占满宽度
//!   的行不适用,继续各自布局。
//! - `action_button_border_color`/`action_button_style`: 弹窗内取消/确认/
//!   删除类按钮的统一描边规则(2026-09-15 起)——此前各弹窗各写一份
//!   `button::Style` 字面量,描边色跟文字色绑死(取消=中性 `BORDER`、
//!   确认/删除=各自的语义色),且都不响应 hover。现在统一成:静止态描边
//!   固定 `BORDER`(#1c3440),悬浮/按下态一律变 `GOLD`;按钮语义只通过
//!   文字色区分(灰=取消/次要、红=危险删除、金=主要确认),描边规则本身
//!   不因语义而不同。项目信息面板 footer-bar(`extensions::project::
//!   project_footer_bar`)背景走面板底色 `bg` 而非弹窗卡片 `card`,没法直接
//!   复用 `action_button_style`,但描边规则复用同一个
//!   `action_button_border_color`。

use crate::theme;
use iced_widget::core::{Border, Color, Element, Length, alignment::Horizontal};
use iced_widget::{MouseArea, Row, Stack, button, column, container};

/// 弹窗默认宽度:整个软件窗体宽度(`App::window_size.0`,逻辑像素)的
/// 1/3——2026-09-15 统一约定,取代此前各弹窗各写一个固定像素值(360/420/
/// 280 不等,窗口变宽变窄时弹窗大小不跟着变)。调用方没有更细宽度诉求时
/// 用这个默认值;字段特别多的表单(新增数据源/主机)如果 1/3 窗宽还是
/// 太挤,可以在这个基础上另外调整,不强制所有弹窗都用同一个值。
pub fn width(window_width: f32) -> Length {
    Length::Fixed(window_width / 3.0)
}

/// 弹窗卡片容器样式:CARD 底 + 金色描边 + 圆角,取代各面板各自手写的
/// `container::Style` 字面量。
pub fn card_style(_t: &iced_widget::Theme) -> container::Style {
    let region = theme::region::dialog();
    container::Style {
        background: region.background.map(Into::into),
        border: region.border.unwrap_or_default(),
        ..container::Style::default()
    }
}

/// 满窗磨砂遮罩:`SCRIM` 半透明底 + 噪点贴图,点击回传 `on_dismiss`。
pub fn scrim<'a, Msg: 'a + Clone>(
    on_dismiss: Msg,
) -> Element<'a, Msg, iced_widget::Theme, iced_renderer::Renderer> {
    MouseArea::new(scrim_layer()).on_press(on_dismiss).into()
}

/// 不可点击关闭的遮罩版本:进行中的进度弹窗(如项目"修复"跑完前)不许
/// 点遮罩打断,只挡住底层交互、不挂 `on_press`。
pub fn scrim_blocking<'a, Msg: 'a>() -> Element<'a, Msg, iced_widget::Theme, iced_renderer::Renderer>
{
    scrim_layer()
}

/// 弹窗底部操作按钮行:靠右下角对齐(取消在左、确认在右的相对顺序不变,
/// 只是整行不再贴左/居中)。
pub fn actions<'a, Msg: 'a>(
    row: Row<'a, Msg, iced_widget::Theme, iced_renderer::Renderer>,
) -> Element<'a, Msg, iced_widget::Theme, iced_renderer::Renderer> {
    container(row)
        .width(Length::Fill)
        .align_x(Horizontal::Right)
        .into()
}

/// 弹窗/footer-bar 按钮共用的描边规则:静止态固定 `BORDER`(#1c3440),
/// 悬浮/按下态一律变 `GOLD`。
pub fn action_button_border_color(status: button::Status) -> Color {
    match status {
        button::Status::Hovered | button::Status::Pressed => byteui::theme::color::current().gold,
        _ => byteui::theme::color::current().border,
    }
}

/// 弹窗操作按钮(取消/确认/删除)完整样式:CARD 底 + 统一描边规则 + 调用方
/// 指定的文字色。文字色按语义传:`dim` 给取消/次要,`red` 给危险删除,
/// `gold` 给非破坏性的主要确认(如"移动"弹窗的"确定")。
pub fn action_button_style(
    text_color: Color,
) -> impl Fn(&iced_widget::Theme, button::Status) -> button::Style {
    move |_t, s| button::Style {
        background: Some(byteui::theme::color::current().card.into()),
        text_color,
        border: Border {
            color: action_button_border_color(s),
            width: 1.0,
            radius: 4.0.into(),
        },
        ..button::Style::default()
    }
}

fn scrim_layer<'a, Msg: 'a>() -> Element<'a, Msg, iced_widget::Theme, iced_renderer::Renderer> {
    Stack::new()
        .push(
            container(column![])
                .width(Length::Fill)
                .height(Length::Fill)
                .style(|_t: &iced_widget::Theme| container::Style {
                    background: Some(byteui::theme::color::current().scrim.into()),
                    ..container::Style::default()
                }),
        )
        .push(crate::frosted::noise_layer())
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}
