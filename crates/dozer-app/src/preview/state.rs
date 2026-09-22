//! 预览域状态结构:PreviewTab/TabKind/FindState/PreviewPane 等。

use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

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
    /// JSON/JSONL 文件的文件 tab 有值,非空即代表这个 tab 额外走 JSON 树查看器
    /// (在原生代码编辑器之外多给一个只读 Tree 视图,二者用 tab 顶部切换按钮
    /// 二选一)。**与 `editor`/`tabular` 不同,JSON 不是"独占认领"——它是双
    /// 视图**:`editor` 仍然有值(原生代码编辑器作为 RawText 半边),`json_tree`
    /// 同时有值提供 Tree 半边。因此 webview 池判据仍不能被 JSON 认领(见
    /// `desired_webviews`)。打开那一刻必为 `Some`;唯一的例外是首屏加载失败
    /// (文件内容不是合法 JSON/JSON5)时被 `PreviewPane::clear_json_tree` 清成
    /// `None`,退回纯文本编辑器(2026-09-21 用户口径)。
    pub json_tree: Option<JsonTreeState>,
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
    /// 由外部面板（目前只有代码健康度面板）请求的"打开后立即跳转到这一
    /// 行"——`Some` 只在"这个 tab 刚被新建、还在 `loading` 中"的窗口期内
    /// 有意义，`apply_native_load` 收到结果、把 `editor` 填上的那一刻立刻
    /// `take()` 消费掉。已经打开且 `editor` 已就绪的 tab 不走这个字段，
    /// 直接同步调用 `CodeView::move_cursor_to`。
    pub pending_jump_line: Option<usize>,
    /// Phase A 统一路由结果(含可展示 reason)。`Blank` 占位 tab 没有文件,
    /// 为 `None`;其余文件 tab 在**创建那一刻**就带上,是唯一的路由真相源。
    pub route: Option<PreviewRoute>,
    /// Phase A 统一 backend 描述。与 `route` 同生同灭;实际 viewer 句柄在
    /// 迁移期仍由上面的 `editor`/`tabular`/`json_tree` 字段持有(adapter)。
    pub backend: Option<PreviewBackend>,
    /// backend 生命周期状态。迁移期与旧的 `loading` 字段并存,`loading` 仍是
    /// 渲染侧的实际判据(行为不变),本字段用于状态机与后续阶段。
    pub backend_state: BackendState,
    /// `file_policy` 判定该文件应窗口化(超预算/超 128MiB/超长行)。窗口化
    /// 专用 viewer 未落地前,这类文件不吃 CodeMirror 整载。
    pub windowed: bool,
    /// 窗口化 viewer 的稀疏行索引(由后台任务建立后回填);用于按行跳转/加载
    /// 相邻窗口。非窗口化 tab 恒 `None`。
    pub window_index: Option<std::sync::Arc<crate::preview::LineIndex>>,
    /// 脏内容的 recovery snapshot 是否已落盘(允许休眠脏 tab 的前提)。
    pub recovery_written: bool,
    /// 启动恢复时从 recovery 读回的正文;editor `ready` 后经 `SetDocument` 推回,
    /// 并重新标脏。
    pub pending_restore: Option<String>,
    /// 本次物化开始时刻(用于 ready latency 观测;不记文件内容)。
    pub load_started: Option<std::time::Instant>,
    /// 启动恢复的视图状态(cursor / selection / scroll top_line),editor `ready`
    /// 后应用一次。
    pub pending_view: Option<(
        Option<crate::preview::TextPosition>,
        Option<crate::preview::TextRange>,
        Option<u32>,
    )>,
    /// CodeMirror host 的轻量镜像；正文仍由 WebView 持有，Rust 只保留 Agent、
    /// 保存和过期事件校验所需状态。
    pub web_revision: u64,
    pub web_selection: Option<crate::preview::TextRange>,
    pub web_selected_text: Option<String>,
    pub web_viewport: Option<(u32, u32)>,
    pub web_error: Option<String>,
}

impl PreviewTab {
    /// 该 tab 在 Phase A 是否需要 Flyfish wry webview。
    ///
    /// WebView 生命周期只读统一 backend；旧 viewer `Option` 不再参与正常
    /// 路由。唯一例外是原生加载进入 `Failed` 后，为保持 Phase A 用户行为不变，
    /// 状态机明确选择 Flyfish fallback。Phase D 落地正式 External/Unsupported
    /// 页面后再移除该兼容分支。
    pub fn hosts_webview(&self) -> bool {
        if self.loading
            || matches!(self.backend_state, BackendState::Suspended)
            || !matches!(self.kind, TabKind::File(_))
        {
            return false;
        }
        // 窗口化只读走 editor host(与 CodeMirror 同一 WebView 通道),不再另起
        // Flyfish webview。
        if self.uses_windowed_editor() {
            return false;
        }
        self.backend.as_ref().is_some_and(|backend| {
            backend.hosts_webview()
                || (self.editor.is_none()
                    && self.tabular.is_none()
                    && self.json_tree.is_none()
                    && (!self.uses_codemirror()
                        || matches!(self.backend_state, BackendState::Failed(_))))
        })
    }

    pub fn current_mode(&self) -> Option<PreviewMode> {
        self.backend.as_ref().map(PreviewBackend::current_mode)
    }

    /// 后端是否只读(Code backend 的 `ReadOnly` 档;其余后端如 Tree/渲染按
    /// 各自语义,这里对非 Code 返回 false)。
    pub fn backend_read_only(&self) -> bool {
        matches!(
            self.backend,
            Some(PreviewBackend::Code(CodeBackend {
                mode: CodeMode::ReadOnly,
                ..
            }))
        )
    }

    pub fn uses_codemirror(&self) -> bool {
        codemirror_enabled()
            && !self.windowed
            && matches!(self.backend, Some(PreviewBackend::Code(_)))
    }

    /// 是否走 CodeMirror 编辑 host(含窗口化只读)。
    pub fn uses_editor_host(&self) -> bool {
        codemirror_enabled() && matches!(self.backend, Some(PreviewBackend::Code(_)))
    }

    /// 是否走窗口化只读 editor host(大文件)。
    pub fn uses_windowed_editor(&self) -> bool {
        self.uses_editor_host() && self.windowed
    }

    /// debug/test 下断言 backend 描述与旧 adapter 字段一致:任何迁移漏点
    /// (新代码只读 backend 但旧字段没同步)都应立即暴露,而不是静默分叉。
    ///
    /// 只在**创建完成那一刻**调用。用户在 Markdown tab 上手动切到源码模式
    /// 后 `editor` 会被填上,那一刻 backend 仍是 Rendered(有意为之),不参与
    /// 这里的创建期一致性检查。
    pub fn debug_assert_backend_consistent(&self) {
        #[cfg(debug_assertions)]
        {
            let (Some(route), Some(backend)) = (self.route.as_ref(), self.backend.as_ref()) else {
                return;
            };
            debug_assert_eq!(
                route.kind,
                backend.kind(),
                "route.kind 与 backend 描述必须一致 (tab {:?})",
                self.title
            );
            let has_native_viewer =
                self.editor.is_some() || self.tabular.is_some() || self.json_tree.is_some();
            if has_native_viewer {
                debug_assert!(
                    !self.hosts_webview(),
                    "持有原生 viewer 的 backend 不应再 host webview (tab {:?})",
                    self.title
                );
            }
        }
    }
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
            .field("json_tree", &self.json_tree.is_some())
            .field("pending_jump_line", &self.pending_jump_line)
            .field("route", &self.route)
            .field("backend", &self.backend)
            .field("backend_state", &self.backend_state)
            .field("web_revision", &self.web_revision)
            .field("web_selection", &self.web_selection)
            .field("web_selected_text", &self.web_selected_text)
            .field("web_viewport", &self.web_viewport)
            .field("web_error", &self.web_error)
            .finish()
    }
}

/// 表格 tab 的加载态。`Loading` = 首次打开该文件、或懒加载某个 sheet 期间
/// 在后台线程跑(见 `crate::tabular::load`/`load_sheet`),完成后经
/// `Message::TabularLoaded` 落回 `Ready`；失败由统一 `BackendState::Failed`
/// 承载并提供重试/外部打开降级，不会无限停留在 Loading。
pub enum TabularState {
    Loading,
    Ready(crate::tabular::TabularView),
}

/// JSON tab 的树查看器加载态。失败时清掉树并优先退回原文编辑器，同时由统一
/// backend 状态记录最终 Ready/Failed。`Ready` 里
/// `Box<JsonTreeView>`:该结构体本身 ≥256 字节(内含 `Vec<u8>`/`PathBuf`/
/// 多个集合),不装箱会让 `Loading` 变体和它之间出现明显的枚举尺寸差
/// (clippy::large_enum_variant);每个 tab 只存一份,装箱开销可忽略。
pub enum JsonTreeState {
    Loading,
    Ready(Box<crate::json_tree::JsonTreeView>),
}

/// 预览面板空白页(`TabKind::Blank`)对应的项目根目录简介。`path` 即 `Workspace::
/// project.path`,留一份方便 view 比对——`apply_pending_blank_info` 拿到
/// `PreviewBlankInfoLoaded` 时若 `info.path != ws.project.path` 就丢弃
/// (用户中途切了项目,旧结果已失配)。`size_bytes`/`file_count` 走
/// `extensions::project::compute_blank_info`,与"用量徽章"共用
/// `DISK_USAGE_EXCLUDE`(`target/`/`.git/`/`node_modules` 等不计)。`created`/
/// `modified` 来自 `fs::metadata(root)`,平台不支持时为 `None`。
#[derive(Debug, Clone)]
pub struct BlankPaneInfo {
    pub path: PathBuf,
    pub size_bytes: u64,
    pub file_count: u64,
    pub created: Option<SystemTime>,
    pub modified: Option<SystemTime>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TabKind {
    File(PathBuf),
    /// 预览面板的"空白"占位 tab:内容区显示项目根目录简介卡(Folder icon +
    /// 名称、位置、大小、创建/修改时间,见 `workspace.rs::preview_pane_for`)——
    /// **可被关闭**(与文件 tab 一视同仁,见 `PreviewPane::close`),前提是还有
    /// 兄弟 tab 可当落点;若关完列表为空,`close` 会立即补回一个新 Blank(`next_id`
    /// 续号),所以面板永远不会停在空 list 这种不合法状态。**不可拖换位**
    /// (`reorder` 仍然把 index 0 当作锚点)——它在用户视觉上是"项目根"的代表,
    /// 不能被拖到第二位。这套形态对齐 SSH/数据库面板 tab 条最前面那个固定
    /// "空白"占位 tab(`app.rs::ssh_tab_bar`/`extensions::database`)。不进
    /// `desired_webviews()` 期望清单,没有 wry 页面,纯 iced 原生渲染。
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
/// flyfish 预览(走 wry webview 的文件 tab)的"文件内搜索"靠驱动
/// `<flyfish-file-viewer>` 元素自带的搜索 API(`searchDocument` /
/// `nextSearchResult` / `previousSearchResult` / `clearDocumentSearch` /
/// `getSearchState`),副作用只能在持有 webview 句柄的 window_events 事件环里
/// 跑——见 `platform/window_events.rs::apply_pending_preview_find`。这一枚举
/// 是"本轮要下发给 webview 的那一个动作",由 `FindState::pending_webview_exec`
/// 持有;`App` 的 update 只更新纯状态(query/case_sensitive…),真正的 JS 注入
/// 等每帧 `apply_pending_preview_find` 看到脏标记才做(同 `apply_pending_focus`
/// 的"派发完消息后轮询待处理标记"节奏)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebviewFindAction {
    /// 用当前 `query` 重新执行 `searchDocument`(⌘F 打开/键入/翻大小写开关时)。
    Search,
    /// 翻到下一个命中(`nextSearchResult`)。
    Next,
    /// 翻到上一个命中(`previousSearchResult`)。
    Prev,
}

#[derive(Debug, Clone)]
pub struct FindState {
    /// 搜索锁定到的原生 tab 的 `PreviewTab.id`。跳转光标只作用在它身上。
    pub tab_id: usize,
    /// 输入框草稿(query 原文,不做 trim)。
    pub query: String,
    /// 当前选中的匹配序号(0-based;`< count` 才有意义;nav 到末尾 wrap 回 0)。
    /// 原生 editor 档由 `CodeView::find_matches_all` 现算回填;webview(flyfish)
    /// 档由 `getSearchState().currentIndex` 回写(00 态记为 0)。
    pub current: usize,
    /// 当前 `query` 在该 tab 里总共命中数,视图只读。原生 editor 档由调用方
    /// 执行后回填;webview(flyfish)档由 `getSearchState().total` 回写。空 query
    /// / 未命中时 `count==0`。
    pub count: usize,
    /// 大小写敏感开关(false=默认的 ASCII 大小写折叠,true=逐字严格比较)。由
    /// 调用方以 `Message` 翻转后持久在这里;每次匹配 / 导航 / 编辑后现算都读它。
    pub case_sensitive: bool,
    /// “替换为”文本草稿(替换条的输入框内容,不吃 query 的大小写折叠——只是
    /// 一个要被原样插进去的字符串,不做规则匹配)。`replace_current` /
    /// `replace_all` 都拿它当替换物;空串表示“删掉那处命中”。webview(flyfish)
    /// 档 flyfish 没有替换概念,这一字段恒空、替换行永不展开。
    pub replacement: String,
    /// 替换行(第二行,含替换输入框 + 「替换当前」/「替换全部」)是否展开
    /// 显示。⌘F 打开时收起、⌘R 打开时展开;查询框前的圆盘箭头可随时手动
    /// 切换。默认收起,不占多余纵向空间——多数查找场景不需要替换。webview
    /// (flyfish)档恒为 false(flyfish 搜索不支持替换)。
    pub replace_open: bool,
    /// 查询输入框是否持有 iced 真实焦点——main.rs 每帧用
    /// `CaptureFindFocus`/`take_find_focused` 查回来写进这里(同 Files 搜索框
    /// `search_focused` 的既有手法)。边框描金不再只看"查询词非空"(`workspace.rs`
    /// `query_row` 用它 `|| !query.is_empty()` 一起决定,2026-09-11 需求:
    /// 聚焦态也该描金,不能只靠已有内容触发)。
    pub query_focused: bool,
    /// 是否锁定在走 wry webview(flyfish)的预览 tab 上。原生 editor 档为
    /// false——那种走 `CodeView::find_matches_all` 现算那一套;`true` 时搜索
    /// 引擎换成 flyfish 自带 API(`pending_webview_exec` 驱动)。
    pub is_webview: bool,
    /// webview(flyfish)档的"待下发动作":`Some` 表示 `apply_pending_preview_
    /// find` 这帧要往 webview 注入一次对应 JS(`Search`/`Next`/`Prev`),
    /// 消费后清回 `None`。原生 editor 档恒为 `None`(那套同步现算,不靠轮询
    /// 脏标记)。
    pub pending_webview_exec: Option<WebviewFindAction>,
    /// webview(flyfish)档锁定的那个 webview 在 `webviews` 池里的 key——已
    /// 把 `PROJECT_PREVIEW_ID_OFFSET` 偏移算进去(Project 面板预览用),原生
    /// editor 档为 `None`。关条/切走要清高亮时(`pending_webview_find_clear`)
    /// 用它在 `window_events` 事件环里找回句柄,因为 `FindState` 已被丢的
    /// 时刻句柄只那里拿得到(同 `pending_webview_exec` 的"派发完消息后轮询"
    /// 节奏)。
    pub webview_pool_id: Option<usize>,
}

/// 预览 Find 条在 webview(flyfish)档占的纵向高度(逻辑像素)。webview 是原生
/// 子视图、不听 iced 绘制顺序,直接叠在最上——⌘F 打开 webview 档 find 时,
/// `preview_desired` 会把 webview 矩形下推 + 压低这一高度,把这块条让给 iced
/// 渲染(见 `App::preview_find_bar_over_webview` 与 `crate::app::App::
/// preview_desired`)。原生 editor 档 find 不盖 webview,那条路不读这个常量。
pub(crate) const PREVIEW_FIND_BAR_HEIGHT: f32 = 44.0;

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
    /// 全局客户端能力快照(启动探测一次)。`Default` 取当前安装值;Workspace
    /// 装配时会用 `ShellIo` 里那份显式覆盖(见 `Workspace::from_restore`),
    /// 保证同一进程内所有预览路由读的是同一份预算。
    pub(crate) capabilities: Arc<crate::capabilities::ClientCapabilities>,
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
    /// webview(flyfish)档 find 关条/切走时,需往旧 webview 注入一次
    /// `clearDocumentSearch()` 把高亮抹掉——但 `FindState` 在 `close_find`/
    /// `cull_stale_find` 里已经被丢,句柄又只在 window_events 事件环里拿得到,
    /// 所以把"该清一次 + 清哪个 webview"这个一次性意图先落在这里,由
    /// `apply_pending_preview_find` 消费(window_events 同 `pending_focus` 的
    /// "派发完消息后轮询"节奏)。`Some(id)` 存的是该 webview 池 key(已含
    /// Project 偏移);`None` 表示无事发生。
    pub(crate) pending_webview_find_clear: Option<usize>,
    /// `push_tab` 刚创建、还没被外层 spawn 后台加载的表格 tab
    /// `(PreviewTab.id, 文件路径)` 队列。调用方在 `open_path`/`push_tab`
    /// 返回后立即 `take_pending_tabular_loads()` 取走清空,不应该攒着不取
    /// (见 `PreviewPane::take_pending_tabular_loads` 文档)。
    pub(crate) pending_tabular_loads: Vec<(usize, PathBuf)>,
    /// 同 `pending_tabular_loads`,但针对 JSON 树查看器(见 `JsonTreeState`)。
    pub(crate) pending_json_tree_loads: Vec<(usize, PathBuf)>,
    /// 待下发给 CodeMirror editor webview 的命令队列(`tab_id`, 命令)。
    /// `window_events` 每帧(同 `apply_pending_preview_find` 节奏)取走并
    /// `evaluate_script` 注入;Agent reveal/select 与外部 reload 用它。
    pub(crate) pending_editor_commands: Vec<(usize, EditorCommand)>,
    /// 空白页信息卡:激活 tab 为 `TabKind::Blank` 时,`apply_pending_blank_info`
    /// 异步跑出来的项目根目录简介。`None` 表示还没拉;view 层用 `—` 占位。
    /// `clear_all`/`PreviewTabSwitch` 路径会同步置回 `None`(项目切换后
    /// `path` 变化,旧结果失配)。
    pub(crate) blank_info: Option<BlankPaneInfo>,
    /// 防重复 spawn 的幂等位:空白页拉信息时由 `apply_pending_blank_info`
    /// 在 `handle.spawn` **之前**置 true,`PreviewBlankInfoLoaded` 到达时清回
    /// false。必须先置再 spawn,否则同帧 `user_event`/`window_event` 各扫到
    /// 一次,会起两份后台任务。
    pub(crate) blank_info_in_flight: bool,
}

impl Default for PreviewPane {
    fn default() -> Self {
        // 面板恒定携带一个第 0 项的 `TabKind::Blank` 占位 tab(见该变体文档):
        // 从没有过 tab 的初始态、以及项目切换清空后,都停在它上面。`next_id`
        // 从占位 tab 的 id 之后续,同一个 `PreviewPane` 生命周期内 id 不重复
        // ——`close` 关掉最后一个 Blank 后补回的"新 Blank"也用 `next_id` 续号。
        Self {
            tabs: vec![placeholder_tab(0)],
            active: 0,
            next_id: 1,
            capabilities: crate::capabilities::current(),
            pending_editor_focus: false,
            pending_find_focus: false,
            pending_editor_reveal_focus: false,
            find: None,
            large_file_search: None,
            pending_webview_find_clear: None,
            pending_tabular_loads: Vec::new(),
            pending_json_tree_loads: Vec::new(),
            pending_editor_commands: Vec::new(),
            blank_info: None,
            blank_info_in_flight: false,
        }
    }
}
