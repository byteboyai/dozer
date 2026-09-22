//! `JsonTreeView` 的视图组装:只画树主体。
//!
//! 「树 / 原始文本」切换按钮**不在这里** —— 2026-09-22 起画在预览 tab 栏上
//! (见 `workspace::preview_pane_for` 的 `json_tree_mode_button` 与
//! `chrome::tab_widget::tab_json_tree_mode_button`),与 `.md`/`.html` 的
//! 「预览/代码」切换同处、同一视觉。原先那行「查看原始文本 / 查看 Tree」
//! 头部已随之移除。
//!
//! RawText 半边也不在这里渲染 —— 外层 `workspace/view.rs` 直接渲染该 tab
//! 已有的原生 `editor`(CodeView),切换复用同一份已加载状态,不重新解析文件。

use iced_widget::core::Element;

use super::{Action, JsonTreeView};

impl JsonTreeView {
    /// 树主体(Tree 模式下由调用方用 `Length::Fill` 容器承接)。
    pub fn view(&self) -> Element<'_, Action, iced_widget::Theme, iced_renderer::Renderer> {
        super::tree::view(self)
    }
}
