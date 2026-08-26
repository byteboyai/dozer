//! 文本输入/编辑控件的统一右键菜单触发封装。
//!
//! iced 原生 `text_input` / `text_editor` 已内置 ⌘/Ctrl+C/X/V/A 键盘快捷键
//! (由窗口聚焦态放行闸门交给控件自身的 `on_event`/clipboard 处理),但**没有**
//! 原生右键菜单。这里提供纯粹的"右键触发"封装:把输入外层包一个
//! `MouseArea::on_right_press`,右键时下发调用方指定的"打开菜单"消息。
//!
//! 真正的菜单外壳/项渲染(`crate::menu::shell`/`item`)与剪贴板动作
//! (把动作消息合成回 ⌘+C/X/V/A 的键盘事件喂给聚焦输入框)都在 dozer-app 侧,
//! 因为 byteui 是泛型 UI 库、不持有 dozer 的 `Message`/全局布局状态,而弹出层
//! 定位必须用窗口绝对坐标在顶层 view 的 `stack` 里渲染——这些都在 dozer-app
//! 的 `App`/`main.rs` 完成。byteui 这里只负责"同一套右键接线"这一个统一接口。
//!
//! `on_right_click` 为 `None` 时原样返回元素(不需要菜单的输入框不受影响)。

use iced_widget::MouseArea;
use iced_widget::core::Element;

/// 给任意输入/编辑元素接上"右键弹菜单"的触发:`Some(msg)` 时外包一层
/// `MouseArea::on_right_press`,右键即下发 `msg`;`None` 时原样返回。
/// `MouseArea` 只吃掉右键(lower-events),左键聚焦/拖选光标等交互仍透传给
/// 下层输入框——同文件树行右键(`files.rs`)同一接线手法。
pub fn wrap<'a, Message>(
    element: Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>,
    on_right_click: Option<Message>,
) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>
where
    Message: Clone + 'a,
{
    match on_right_click {
        Some(msg) => MouseArea::new(element).on_right_press(msg).into(),
        None => element,
    }
}

/// 右键菜单里要展示的四个标准编辑动作。dozer-app 用它给菜单项选图标/标签,
/// 并决定把动作映射成哪个键盘事件(见 `main.rs` 的剪贴板合成)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditAction {
    Cut,
    Copy,
    Paste,
    SelectAll,
}
