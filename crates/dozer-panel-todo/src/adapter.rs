//! 产品注入的适配类型。

/// 文本框的右键菜单目标:库只发出这个不透明目标,由产品转成自己的菜单请求。
/// 形状与 Dozer `app::TextInputTarget` 相同。
#[derive(Debug, Clone)]
pub struct TextInputTarget {
    pub id: iced_widget::core::widget::Id,
    pub secure: bool,
}

/// agent 图标的数据(原来 `protocol.rs` 直接调 `workspace::agent_icon` 取字节与"保留原色"标记)。
#[derive(Debug, Clone)]
pub struct AgentIconData {
    pub bytes: Vec<u8>,
    pub preserves_color: bool,
}
