//! 统一弹出菜单(popup menu)样式原语。
//!
//! 实现已搬进共享库 `byteui::interaction::menu`(2026-10-28,X1;与 Digger 共用),这里只保留原路径的
//! 重导出,调用点不用改;外框度量见 `byteui::theme::region::context_menu`(背景不透明)。原语的完整说明
//! (外壳、单项、锁定、分组、浮层)在 byteui 的模块文档里。
//!
//! 本文件只留下依赖 Dozer 本地 `crate::frosted` 的 [`shell_frosted`]。

pub use byteui::interaction::menu::{
    icon_leading, item, item_locked, item_row, item_row_fill, separator, shell,
};
use iced_widget::Stack;
use iced_widget::core::{Element, Length};

/// `shell()` 的磨砂版:在面板上叠一层 [`crate::frosted::noise_layer`]
/// 噪点贴图模拟磨砂玻璃颗粒。
/// **只用于内容不会逐帧重绘的静态弹层**——多出的 `Stack` 非 base 层每次
/// 都要重新构建/重绘一遍,静态弹层只在打开/关闭或离散 hover 状态切换时
/// 重绘一次,这份开销可忽略;但像 `tab_widget::tab_overflow_menu` 那样
/// 内容里有 `hover_t` 缓动动画的弹层,`HOVER_ANIM_INTERVAL`(`main.rs`,
/// ~60fps)会在过渡期间持续触发整棵子树重绘,噪点层的额外开销被按帧数
/// 放大——2026-09-13 用户反馈那里 hover 明显卡顿,release 构建下依然卡,
/// 排除是 debug 构建噪音,定位到就是这份放大。因此那个调用点保留不带
/// 噪点的 `shell()`,其余静态弹层改用这个。
pub fn shell_frosted<'a, Msg: 'a>(
    items: Vec<Element<'a, Msg, iced_widget::Theme, iced_renderer::Renderer>>,
    width: Length,
) -> Element<'a, Msg, iced_widget::Theme, iced_renderer::Renderer> {
    Stack::new()
        .push(shell(items, width))
        .push(crate::frosted::noise_layer())
        .into()
}
