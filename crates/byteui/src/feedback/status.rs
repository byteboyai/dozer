//! amis `status`(成功/失败/进行中状态展示):<https://baidu.github.io/amis/zh-CN/components/status>
//!
//! 只提供状态点本身,不绑定固定的"点+标签"排版——调用方各自的状态标签
//! 内容/位置/相邻元素差异较大(有的紧跟文字、有的作为 tab 前缀单独用、
//! 有的还要接更多内容),统一交给调用方自己拼 `row!`。颜色也不内置枚举:
//! 调用方的状态分类(如 agent 运行态)往往不是通用的"成功/失败/进行中/
//! 空闲"能覆盖的,颜色映射逻辑留在调用方。

use iced_widget::core::{Color, Element};
use iced_widget::text;

/// 状态点(实心圆点字形),固定用 `dot_sm` 字号——app 内所有状态点统一
/// 大小,不由各调用方各自选字号。颜色由调用方按自己的状态语义算好传入。
pub fn dot<'a, Message: 'a>(
    color: Color,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
    text("●")
        .size(crate::theme::font::dot_sm())
        .color(color)
        .into()
}
