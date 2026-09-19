//! 预览域状态结构:PreviewTab/TabKind/FindState/PreviewPane 等。

use std::path::PathBuf;

use super::*;

/// 一个预览 tab。
pub struct PreviewTab {
    pub id: usize,
    pub kind: TabKind,
    pub title: String,
    /// 保存编辑后 `+1`,驱动 `desired_webviews()` 换 URL 逼 `sync_webview_pool`
    /// 重新 `load_url`(同 URL 不会重载,flyfish 的 WKWebView 会一直显示
    /// 保存前的旧内容)。
    pub reload_nonce: u64,
    /// 仅白名单扩展名(`is_editable_extension`)的文件 tab 有值。非空即代表这个
    /// tab 走原生渲染路径,`desired_webviews()` 据此把它从 wry 期望清单里排除。
    /// `CodeView` 没有实现 `Clone`/`PartialEq`,这也是 `PreviewTab` 摘掉这两个
    /// derive 的原因(见下方手写的 `Debug`)。
    pub editor: Option<crate::code_editor::CodeView>,
    /// 表格类文件(`tabular::is_tabular_extension`)的文件 tab 有值,非空即代表
    /// 这个 tab 走 Tabular Viewer 原生渲染(网格)。与 `editor` 互斥:同一 tab
    /// 要么代码编辑器、要么表格、要么 webview,三者取其一。表格加载是异步的
    /// (见 `TabularState` 文档),`Some` 在"是不是表格 tab"这个问题上从打开
    /// 那一刻起就恒定,不随加载有没有完成而改变——所有原有 `tabular.is_some()
    /// /is_none()` 判断("这个 tab 是不是已被表格/编辑器认领")因此不用改。
    pub tabular: Option<TabularState>,
    /// 原生可编辑 tab 的"buffer 与磁盘不一致"标记:用户就地改过、还没 ⌘S 保存
    /// (或右键"刷新"/项目切换丢弃归零)为 `true`。`Blank`/`webview` tab 恒
    /// `false`。2026-09-06 原生预览不再只读,有了就地编辑就必须能显式挂脏并兜底,
    /// 否则用户会无声丢改动(见 workspace.rs 关闭/切换前的确认)。
    pub dirty: bool,
    /// 只读大文件档(`FullLoadReadOnly`/`ChunkedReadOnly`)已载入的字节数;
    /// 编辑档/非文件 tab 恒为 0(无意义,不展示)。
    pub loaded_bytes: u64,
    /// 打开时 `fs::metadata` 测到的文件总字节数;语义同上,非只读大文件 tab
    /// 恒为 0。
    pub total_bytes: u64,
    /// 是否被截断(`ChunkedReadOnly` 档为真;其余恒假)。UI 据此渲染"仅加载
    /// 前 X MB"横幅 + "加载更多"按钮。
    pub truncated: bool,
    /// 原生编辑器正在异步读盘中(`PreviewPane::insert_loading_tab` 置真,
    /// `apply_native_load` 收到结果后置假)。为真时 `editor`/`tabular` 均
    /// `None`,但这个 tab **不**应该被当成"该文件没有原生编辑器"误判进
    /// webview 池——`desired_webviews()`/`active_webview_id()`/`select()` 等
    /// 判据要额外排除 `loading` 为真的 tab。
    pub loading: bool,
}

impl std::fmt::Debug for PreviewTab {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreviewTab")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("title", &self.title)
            .field("reload_nonce", &self.reload_nonce)
            .field("dirty", &self.dirty)
            .field("loaded_bytes", &self.loaded_bytes)
            .field("total_bytes", &self.total_bytes)
            .field("truncated", &self.truncated)
            .field("loading", &self.loading)
            .field("editor", &self.editor.is_some())
            .field("tabular", &self.tabular.is_some())
            .finish()
    }
}

/// 表格 tab 的加载态。`Loading` = 首次打开该文件、或懒加载某个 sheet 期间
/// 在后台线程跑(见 `crate::tabular::load`/`load_sheet`),完成后经
/// `Message::TabularLoaded` 落回 `Ready`。加载失败时(极少见:文件在打开
/// 那一刻被删/损坏)保留 `Loading`,不额外建一个 `Failed` 变体——那种情况下
/// 用户能做的唯一有意义动作是关掉这个 tab 重开,持续显示 loading 转圈比
/// 静默切回空白/报内部错误码更不容易让人误以为"文件是空的"。
pub enum TabularState {
    Loading,
    Ready(crate::tabular::TabularView),
}

#[derive(Debug, Clone, PartialEq)]
pub enum TabKind {
    File(PathBuf),
    /// 预览面板固定的"空白"占位 tab(内容区显示 Dozer 品牌标,见
    /// `workspace.rs::preview_pane_for`)——恒为 tab 列表的**第 0 项**、不可
    /// 关闭、不可拖换位,`active_idx()==0` 即代表"当前没有可预览文件、停在
    /// 空白页"这个落点。不进 `desired_webviews()` 期望清单,没有 wry 页面,
    /// 纯 iced 原生渲染。这套形态对齐 SSH/数据库面板 tab 条最前面那个固定
    /// "空白"占位 tab(`app.rs::ssh_tab_bar`/`extensions::database`)。
    Blank,
}

/// 原生预览编辑器"文件内搜索"(⌘F)会话状态。挂在 `PreviewPane` 上——Files 与
/// Project 各持一份,`⌘F` 只作用于当前聚焦 pane 的那个 pane 的激活原生 tab
/// (隔离天然成立)。`tab_id` 固定这次会话锁定的 tab:用户切走别的 tab / 关闭 /
/// 整个 pane 清空后,`cull_stale_find` 会把整条会话丢掉(Find 是"此刻对着这个
/// 文件"的一次性 UI,不该在切到另一份文件后还挂着一个失配的输入框)。
///
/// 本结构**不缓存匹配清点表**:`count` 只是"最近一次执行/输入后"的展示数字,
/// 由调用方(Workspace 层)每次触发时用 [`CodeView::find_matches_all`] 现算回填;
/// 空 query / 未命中时 `count==0`。编辑正文时若输入框开着,旧 count 会短暂陈旧,
/// 但下一次 typing(受 `PreviewFindText` 驱动)或导航会基于**当前 buffer** 重算,
/// 因此陈旧值只在期间展示,不产生错误落点。
#[derive(Debug, Clone)]
pub struct FindState {
    /// 搜索锁定到的原生 tab 的 `PreviewTab.id`。跳转光标只作用在它身上。
    pub tab_id: usize,
    /// 输入框草稿(query 原文,不做 trim)。
    pub query: String,
    /// 当前选中的匹配序号(0-based;`< count` 才有意义;nav 到末尾 wrap 回 0)。
    pub current: usize,
    /// 当前 `query` 在该 tab buffer 里总共命中数,调用方执行后回填,视图只读。
    pub count: usize,
    /// 大小写敏感开关(false=默认的 ASCII 大小写折叠,true=逐字严格比较)。由
    /// 调用方以 `Message` 翻转后持久在这里;每次匹配 / 导航 / 编辑后现算都读它。
    pub case_sensitive: bool,
    /// “替换为”文本草稿(替换条的输入框内容,不吃 query 的大小写折叠——只是
    /// 一个要被原样插进去的字符串,不做规则匹配)。`replace_current` /
    /// `replace_all` 都拿它当替换物;空串表示“删掉那处命中”。
    pub replacement: String,
    /// 替换行(第二行,含替换输入框 + 「替换当前」/「替换全部」)是否展开
    /// 显示。⌘F 打开时收起、⌘R 打开时展开;查询框前的圆盘箭头可随时手动
    /// 切换。默认收起,不占多余纵向空间——多数查找场景不需要替换。
    pub replace_open: bool,
    /// 查询输入框是否持有 iced 真实焦点——main.rs 每帧用
    /// `CaptureFindFocus`/`take_find_focused` 查回来写进这里(同 Files 搜索框
    /// `search_focused` 的既有手法)。边框描金不再只看"查询词非空"(`workspace.rs`
    /// `query_row` 用它 `|| !query.is_empty()` 一起决定,2026-09-11 需求:
    /// 聚焦态也该描金,不能只靠已有内容触发)。
    pub query_focused: bool,
}

/// 只读大文件档的搜索会话——⌘F 在这类 tab 上不打开 `FindState`(内存线性
/// 扫描,大文件上代价不可接受),而是打开这个,复用
/// `extensions::search::search_scope` 的磁盘流式扫描。`line_no` 是 1-based
/// (grep_searcher 惯例,见 `SearchHit` 文档)。
#[derive(Debug, Clone, Default)]
pub struct LargeFileSearch {
    pub tab_id: usize,
    pub query: String,
    pub hits: Vec<crate::extensions::search::SearchHit>,
    pub current: usize,
    pub running: bool,
}

pub struct PreviewPane {
    pub(crate) tabs: Vec<PreviewTab>,
    pub(crate) active: usize,
    pub(crate) next_id: usize,
    /// 新建原生编辑器 tab 时置位的一次性程序化聚焦标记——`CodeView` 的焦点
    /// 是真实 iced 焦点树的一部分,构造时不能直接拿到,要等下一帧
    /// `UserInterface::build` 之后由 main.rs 用 `operation::focusable::focus`
    /// 强制聚焦(同项目树行内编辑/Todo 内容编辑的既有手法)。
    pub(crate) pending_editor_focus: bool,
    /// Find 条输入框同样要一次性程序化聚焦(⌘F 打开输入框那帧无法直接拿到
    /// 真正的 text_input 焦点,靠 main.rs 下一帧 operation 拨)。`*_focus_for_find`
    /// 在 open/close 时置位,消费式读走。
    pub(crate) pending_find_focus: bool,
    /// 一次性标记:`select_range`/`select_range_backward` 刚给编辑器选中一段
    /// 查找命中时置位——原生 `iced_widget::text_editor` 的选区高亮只在**真持有
    /// iced 焦点**时才画(`draw()` 里 `if let Some(focus) = state.focus.as_ref()`
    /// 才会渲染 `Selection::Range`,vendored 源码已核实),而 Find 输入框才是
    /// 这一刻的真焦点(`pending_find_focus` 打开时已把编辑器 unfocus 掉,见
    /// `operation::focusable::focus` 文档:命中 target 之外的 focusable 全部
    /// `unfocus()`)——不补这一步,选中的命中永远是"选了但看不见"。main.rs 消费
    /// 后用**不会 unfocus 别的 widget**的自定义 Operation(`FocusAlso`,只
    /// `.focus()` 目标、不碰其它 focusable)把编辑器也标记为聚焦,让它画出高亮,
    /// 同时不动 Find 输入框已有的真焦点(2026-09 用户实测反馈:查找跳到第 n 个
    /// 命中时代码里应该真的选中那段文字,不能只是计数器数字变了)。
    pub(crate) pending_editor_reveal_focus: bool,
    /// 文件内搜索(⌘F)会话,`Some` 表示条已显示;Files / Project 各一份,独立。
    pub(crate) find: Option<FindState>,
    /// 只读大文件档的搜索会话,`Some` 表示条已显示。与 `find`(小文件 ⌘F)
    /// 互斥:同一时刻一个 tab 只可能命中其中一种(`open_large_file_search`/
    /// `preview_find_open` 由调用方按 `editor.is_read_only()` 二选一触发)。
    pub(crate) large_file_search: Option<LargeFileSearch>,
    /// `push_tab` 刚创建、还没被外层 spawn 后台加载的表格 tab
    /// `(PreviewTab.id, 文件路径)` 队列。调用方在 `open_path`/`push_tab`
    /// 返回后立即 `take_pending_tabular_loads()` 取走清空,不应该攒着不取
    /// (见 `PreviewPane::take_pending_tabular_loads` 文档)。
    pub(crate) pending_tabular_loads: Vec<(usize, PathBuf)>,
}

impl Default for PreviewPane {
    fn default() -> Self {
        // 面板恒定携带一个第 0 项的 `TabKind::Blank` 占位 tab(见该变体文档):
        // 从没有过 tab 的初始态、以及项目切换清空后,都停在它上面。`next_id`
        // 从占位 tab 的 id 之后续,同一个 `PreviewPane` 生命周期内 id 不重复。
        Self {
            tabs: vec![placeholder_tab(0)],
            active: 0,
            next_id: 1,
            pending_editor_focus: false,
            pending_find_focus: false,
            pending_editor_reveal_focus: false,
            find: None,
            large_file_search: None,
            pending_tabular_loads: Vec::new(),
        }
    }
}
