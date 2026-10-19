//! 产品侧能力的注入点:哪些功能在这个产品里存在(`FilesCapabilities`),
//! 以及面板要做而自己做不了的事(`FilesRequest`)——库不调用任何原生菜单、
//! 预览或 Agent 通道,只把"想做的事"排进队列交给产品。

use crate::Message;
use crate::menu::MenuItem;

/// 这个产品给文件树开哪些能力。关掉的能力不是"置灰",而是整条菜单项
/// 不出现、对应消息也不会被生成——库内所有产生这些消息的分支都要先看
/// 这里。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FilesCapabilities {
    /// 文件的 git 历史(回滚 / 历史 / 逐行 blame)。Dozer 有,Digger 暂无。
    pub file_history: bool,
    /// 右键"添加到 Agent 上下文"与预览选区"发送给 Agent"。
    pub send_to_agent: bool,
    /// 目录 / 文件搜索(右键"搜索")。
    pub search: bool,
    /// 预览标签页(多标签右键菜单、按来源刷新)。Dozer 有,Digger 用单文档预览。
    pub preview_tabs: bool,
}

impl FilesCapabilities {
    /// 全部打开 —— Dozer 用(`adapter` 之下这些能力都由 `App` 实现)。
    pub fn all() -> Self {
        Self {
            file_history: true,
            send_to_agent: true,
            search: true,
            preview_tabs: true,
        }
    }
}

/// 面板请求产品做的事。库的 `update` 只往 `WorkspaceState::requests` 里
/// 推,产品每帧 `take_requests()` 排空并转成自己的渲染/命令。
#[derive(Debug, Clone)]
pub enum FilesRequest {
    /// 在给定屏幕坐标弹出右键菜单。`items` 已规整(见 `menu::context_menu_items`),
    /// 选中后把对应 `msg` 送回 `update`;空 `Vec` 表示不弹。
    ShowContextMenu {
        items: Vec<MenuItem<Message>>,
        at: (f32, f32),
    },
    /// 用产品自己的菜单机制弹同一个条目表。Dozer 走 macOS 原生 NSMenu
    /// (选中后经回调把 `msg` 送回 `update`),非 macOS 退化成 `ShowContextMenu`;
    /// 不认识原生菜单的产品(如 Digger)同样按 `ShowContextMenu` 处理。
    ///
    /// 与 `ShowContextMenu` 分开是因为两者语义不同:前者一定会被渲染成产品
    /// 自己的浮层,而原生菜单在 `ShowNativeMenu` 弹出后**要等用户选择**才
    /// 产生消息,库不能在弹出点就把菜单状态写死。
    ShowNativeMenu {
        items: Vec<MenuItem<Message>>,
        at: (f32, f32),
    },
    /// 打开分支选择器。库不认识产品的弹层外观(宽度、磨砂外壳、当前分支
    /// 高亮色都是主题细节),所以只报"该弹了"和内容,渲染交给产品:选中后
    /// 把 `Message::BranchSwitch(name)` 送回 `update`;`Message::BranchPickerClose`
    /// 仍由产品自己发。
    ShowBranchPicker {
        branches: Vec<String>,
        current: Option<String>,
        /// 工作区有未提交改动时,除当前分支外全部禁用。
        lock_others: bool,
    },
}

/// 文本输入框右键菜单的目标。`id` 是产生右键的那个输入框(主窗口搜索框 /
/// 新建与重命名输入框),`secure` 决定是否出现"复制"类条目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextInputTarget {
    pub id: iced_widget::core::widget::Id,
    pub secure: bool,
}

/// 文本选区(1-based 行列)——库不认识预览引擎的类型,产品在调用点转换。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextRange {
    pub start: TextPosition,
    pub end: TextPosition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextPosition {
    pub line: u32,
    pub column: u32,
}

/// 预览标签的来源面板 —— `TabContextMenu*` 消息用它决定"刷新这个标签"去
/// 哪拿内容。只在 `FilesCapabilities::preview_tabs` 为真时产生。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewKind {
    Files,
    Project,
}

/// 面板展示尺寸/字体的注入点。S5a 里同名 trait 还带列表折叠等一整套
/// 面板骨架方法,文件树只用得上这几个,所以单独收窄——产品实现时不必
/// 关心文件树用不到的面板语义。
pub trait PanelDisplay {
    /// 面板头(图标 + 已本地化的标题 + 分割线)。文件树头部纯展示、不产生
    /// 消息,但标题字号/分割线颜色来自产品主题,所以仍由产品渲染,库只
    /// 负责把它放进自己的布局。
    fn panel_head<'a>(
        &self,
        icon: byteui::interaction::icons::IconKind,
        title: String,
    ) -> iced_widget::core::Element<'a, crate::Message, iced_widget::Theme, iced_renderer::Renderer>;

    /// 头部内边距。
    fn header_padding(&self) -> iced_widget::core::Padding;
    /// 树行字号(随产品密度变化)。
    fn tree_row_font_size(&self) -> f32;
    /// 树行行高倍数(相对字号)。Dozer 与终端行距对齐,取终端字体配置。
    fn line_height_factor(&self) -> f32;
    /// Files 面板所在区域的样式(背景/边框/内边距/间隔)。
    fn region(&self) -> byteui::theme::region::RegionStyle;
    /// 当前 locale(`"en"` / `"zh-CN"`)。
    fn locale(&self) -> &str;
    /// 窗口逻辑尺寸 `(宽, 高)`——拖拽幽灵胶囊据此把自身钳在窗口内。
    fn window_size(&self) -> (f32, f32);
    /// 最近一次光标位置(窗口逻辑坐标)——拖拽幽灵胶囊跟着它走。
    fn last_cursor(&self) -> (f32, f32);
}
