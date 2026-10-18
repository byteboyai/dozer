//! 产品专属语义的注入点。库本身不依赖 `dozer-app` / `dozer-client` / `dozer-core`;
//! 产品(Dozer / Digger)在这里把"面板外观"与"文本框菜单目标"接进来。

pub use dozer_panel_host::{Notice, NoticeLevel, NoticeOutbox};

/// 面板外观参数(取色、面板头、分隔线);单个 workspace 的 hover 动画进度另行传入。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PanelDisplay {
    /// 面板头的高度与内边距。
    pub header_padding: [u16; 4],
    /// 面板头与列表之间的分隔线是否显示。
    pub tab_divider: bool,
    /// 当前 locale(`en` / `zh-CN`)。
    pub locale: &'static str,
}

impl PanelDisplay {
    /// 默认外观(与 Dozer 的 Project 面板一致)。
    pub const DEFAULT: Self = Self {
        header_padding: [12, 16, 8, 16],
        tab_divider: false,
        locale: "en",
    };
}

/// 文本框的右键菜单目标:库只发出这个不透明目标,由产品转成自己的菜单请求。
/// 形状与 Dozer `app::TextInputTarget` 相同。
#[derive(Debug, Clone)]
pub struct TextInputTarget {
    pub id: iced_widget::core::widget::Id,
    pub secure: bool,
}
