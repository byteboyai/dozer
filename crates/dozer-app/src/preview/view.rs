//! 预览域 view:`impl PreviewPane`(渲染/期望清单/Find 焦点捕获)+ 测试。

use iced_widget::core::Rectangle;
use iced_widget::core::widget::operation::Focusable;
use iced_widget::core::widget::{Id, Operation};
use std::path::PathBuf;

use super::*;

/// 窗口化 viewer 每次推给 CodeMirror 的窗口大小(以目标行为中心的前后行数)。
pub const WINDOW_BEFORE: u32 = 1000;
pub const WINDOW_AFTER: u32 = 2000;

/// 构造一个 `TabKind::Blank` 占位 tab。**可被关掉**(`close(0)` 在有兄弟
/// tab 时真关;没兄弟时关完自动补一个新的,见 [`PreviewPane::close`]),所以
/// "id 0" 不再是不变量。`Default` 与 `clear_all`(项目切换)都靠它把面板
/// 复位成"只剩空白页"这个恒定形态。
pub(crate) fn placeholder_tab(id: usize) -> PreviewTab {
    PreviewTab {
        id,
        kind: TabKind::Blank,
        title: "空白".into(),
        reload_nonce: 0,
        editor: None,
        tabular: None,
        json_tree: None,
        dirty: false,
        loaded_bytes: 0,
        total_bytes: 0,
        truncated: false,
        loading: false,
        pending_jump_line: None,
        route: None,
        backend: None,
        backend_state: BackendState::Ready,
        windowed: false,
        window_index: None,
        recovery_written: false,
        pending_restore: None,
        load_started: None,
        pending_view: None,
        web_revision: 0,
        web_selection: None,
        web_selected_text: None,
        web_viewport: None,
        web_error: None,
    }
}

/// 为文件 tab 计算 Phase A 的唯一 route/backend 描述。画像是**有界采样**
/// (头/尾各至多 64KiB);文件读不到(测试里的假路径、权限问题)时退回空画像,
/// 不影响按扩展名路由。
pub(crate) fn route_and_backend(
    path: &std::path::Path,
    capabilities: &crate::capabilities::ClientCapabilities,
) -> (PreviewRoute, PreviewBackend, BackendState) {
    let profile = profile_file(path).unwrap_or_else(|_| analyze(&[], None, 0, None));
    let route = classify_preview(path, &profile, capabilities, None);
    // 只读档:非 UTF-8/二进制(CodeMirror fetch().text() 会丢字节)或
    // `file_policy` 判定非 EditableCode(超 30MiB / 超预算)一律只读。
    let non_text = matches!(profile.utf8, Utf8Status::Invalid)
        || matches!(profile.content_kind, ContentKind::Binary);
    let policy = decide_text_policy(&profile, &capabilities.budgets);
    let read_only = non_text || policy.policy != TextFilePolicy::EditableCode;
    let backend = PreviewBackend::from_route(&route, path, read_only);
    // 同步路径(表格/webview/渲染)创建即 Ready;原生编辑器异步路径的调用方
    // 会用 `insert_loading_tab` 把状态置为 Loading。
    (route, backend, BackendState::Ready)
}

/// 该文件是否应按 `file_policy` 走**窗口化**(超预算 / 超 128MiB / 超长行)。
/// 窗口化专用 viewer 尚未落地,Phase C 期间在 feature 打开时仍走老的 iced
/// 分块只读,避免把超大文件整载进 CodeMirror WebView。
pub(crate) fn path_is_windowed(
    path: &std::path::Path,
    capabilities: &crate::capabilities::ClientCapabilities,
) -> bool {
    let profile = profile_file(path).unwrap_or_else(|_| analyze(&[], None, 0, None));
    decide_text_policy(&profile, &capabilities.budgets).is_windowed()
}

/// `path` 是否会被打开为"原生编辑器候选"(即 `is_editable_extension &&
/// !prefers_rendered_preview`,与 `push_tab` 里挑 `read_and_build_native_editor`
/// 分支的判据完全一致)。`App::preview_open_path`/`project_preview_open_path`
/// 用它决定走 `insert_loading_tab`(异步读盘+构造)还是原有 `open_path`
/// (表格/webview 类,同步、本来就不慢)。
pub(crate) fn is_native_editor_candidate(path: &std::path::Path) -> bool {
    let (route, _, _) = route_and_backend(path, &crate::capabilities::current());
    match route.kind {
        // feature 打开后 Code/JSON/Streamed 的文本视图统一走 editor host:
        // 普通文件由 host 自取正文,窗口化由 Rust 推窗口,JSON Tree 仍原生。
        PreviewKind::Code | PreviewKind::Json | PreviewKind::Streamed => !codemirror_enabled(),
        _ => false,
    }
}

/// Find 输入框的稳定 `widget::Id`。Files / Project 两个预览面板各渲染一根
/// Find 条,两个 text_input 同帧都存在于 iced 焦点树,id 必须按面板区分
/// (iced 里同 id 的 focusable 会撞车),不能像全局单个搜索框那样给固定 id。
/// workspace.rs 渲染条、main.rs 用 `operation::focusable::focus` 拨焦点都用
/// 同一个 `kind` 解析出同一个 id。
pub(crate) fn find_field_id(kind: crate::app::PanelKind) -> iced_widget::core::widget::Id {
    // 两个面板的输入框以 distinct 静态 id 区分(iced 焦点树里同 id 会撞车)。
    match kind {
        crate::app::PanelKind::Project => {
            iced_widget::core::widget::Id::new("preview-find-project")
        }
        _ => iced_widget::core::widget::Id::new("preview-find-files"),
    }
}

static FIND_FOCUSED_FILES: std::sync::LazyLock<std::sync::Mutex<bool>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(false));
static FIND_FOCUSED_PROJECT: std::sync::LazyLock<std::sync::Mutex<bool>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(false));

/// 读走并复位(消费式)Find 查询输入框上一帧是否持有 iced 内部真实焦点,
/// 按 `kind` 分 Files/Project 两份——同 `extensions::files::take_search_
/// focused` 的既有桥接手法:消费式复位避免条不可见的帧里卡住上一次的
/// `true`(条关掉后 `CaptureFindFocus` 找不到匹配 id,不会自己覆盖成
/// `false`)。
pub(crate) fn take_find_focused(kind: crate::app::PanelKind) -> bool {
    let cell = match kind {
        crate::app::PanelKind::Project => &FIND_FOCUSED_PROJECT,
        _ => &FIND_FOCUSED_FILES,
    };
    std::mem::replace(&mut *cell.lock().unwrap(), false)
}

/// 每帧 `interface.operate()` 跑一遍,把 Files/Project 两个 Find 查询输入框
/// (`find_field_id`)当前是否持有 iced 焦点分别写进对应 static——两条 Find
/// 条可能同帧都存在(Files/Project 各挂一侧),用 id 区分写入哪一份。
/// `traverse` 必须调用传入的 `operate` 闭包才能继续递归子节点,道理同
/// `extensions::files::CaptureSearchFocus` 的既有文档。
pub(crate) struct CaptureFindFocus;
impl Operation<()> for CaptureFindFocus {
    fn focusable(&mut self, id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Focusable) {
        if id == Some(&find_field_id(crate::app::PanelKind::Files)) {
            *FIND_FOCUSED_FILES.lock().unwrap() = state.is_focused();
        } else if id == Some(&find_field_id(crate::app::PanelKind::Project)) {
            *FIND_FOCUSED_PROJECT.lock().unwrap() = state.is_focused();
        }
    }

    fn traverse(&mut self, operate: &mut dyn for<'a> FnMut(&'a mut (dyn Operation<()> + 'a))) {
        operate(self);
    }
}

impl PreviewPane {
    /// 给窗口化 viewer 推一个以 `center_line`(全局 1-based)为中心的窗口:
    /// 用 tab 上的稀疏索引定位,读**有界**窗口,排队 `SetWindow` 由
    /// `window_events` 注入。返回是否真的推了(不是窗口化/索引未就绪则 false)。
    pub fn queue_windowed_view(&mut self, tab_id: usize, center_line: u32) -> bool {
        let command = {
            let Some(tab) = self.tabs.iter().find(|t| t.id == tab_id) else {
                return false;
            };
            let Some(index) = tab.window_index.as_ref() else {
                return false;
            };
            let TabKind::File(path) = &tab.kind else {
                return false;
            };
            let window = match crate::preview::read_window(
                path,
                index,
                center_line,
                WINDOW_BEFORE,
                WINDOW_AFTER,
            ) {
                Ok(w) => w,
                Err(_) => return false,
            };
            crate::preview::EditorCommand::SetWindow {
                text: window.text,
                start_line: window.start_line,
                total_lines: index.total_lines(),
                revision: tab.web_revision,
            }
        };
        self.queue_editor_command(tab_id, command);
        true
    }

    pub fn tabs(&self) -> &[PreviewTab] {
        &self.tabs
    }

    pub fn tabs_mut(&mut self) -> &mut [PreviewTab] {
        &mut self.tabs
    }

    pub fn active_idx(&self) -> usize {
        self.active
    }

    /// 读走(消费式)一次性程序化聚焦标记。
    pub fn take_pending_editor_focus(&mut self) -> bool {
        std::mem::take(&mut self.pending_editor_focus)
    }

    /// 请求把焦点拨给 Find 输入框(⌘F 打开的那帧置位,次帧 main.rs 拨;与
    /// `pending_editor_focus` 互斥——见字段注释)。
    pub fn request_find_focus(&mut self) {
        self.pending_find_focus = true;
        self.pending_editor_focus = false;
    }

    /// 读走(消费式)Find 输入框的一次性程序化聚焦标记。
    pub fn take_pending_find_focus(&mut self) -> bool {
        std::mem::take(&mut self.pending_find_focus)
    }

    /// 读走(消费式)"编辑器需要补聚焦以显出查找命中高亮"标记,见
    /// `pending_editor_reveal_focus` 字段文档。
    pub fn take_pending_editor_reveal_focus(&mut self) -> bool {
        std::mem::take(&mut self.pending_editor_reveal_focus)
    }

    /// 当前锁定 Find 会话的那个 tab 的原生编辑器 `focus_id`——
    /// `take_pending_editor_reveal_focus` 为真时 main.rs 用它跑
    /// `FocusAlso` 操作。没有 Find 会话/该 tab 没有原生编辑器都返回 `None`。
    pub fn find_editor_focus_id(&self) -> Option<iced_widget::core::widget::Id> {
        let tab_id = self.find.as_ref()?.tab_id;
        self.tabs
            .iter()
            .find(|t| t.id == tab_id)
            .and_then(|t| t.editor.as_ref())
            .map(|e| e.focus_id())
    }

    /// Find 条关闭(⌘F 第二下 / × / Esc)后焦点归回其下的代码编辑器——复用
    /// `pending_editor_focus` 机制,次帧 main.rs 用 `active_editor_focus_id`
    /// 把真实焦点拨回编辑器。
    pub fn request_editor_focus(&mut self) {
        self.pending_editor_focus = true;
        self.pending_find_focus = false;
    }

    /// 当前激活 tab 若走原生渲染,返回其编辑器的 `widget::Id`(供
    /// `operation::focusable::focus` 定位)。
    pub fn active_editor_focus_id(&self) -> Option<iced_widget::core::widget::Id> {
        self.tabs
            .get(self.active)
            .and_then(|t| t.editor.as_ref())
            .map(|e| e.focus_id())
    }

    /// 当前激活 tab 是否为原生可编辑的 `CodeView`(有真实 iced `text_editor`,
    /// 点其内容区那帧会 self-focus 出光标)。main.rs 据此判断这次左键按下该
    /// 不该把预览编辑器一起 `blur`(否则点到编辑器本身就会把刚自聚焦出的光标
    /// 同一帧抬掉——"点代码预览无法获得光标")。
    pub fn active_tab_is_native(&self) -> bool {
        self.tabs
            .get(self.active)
            .is_some_and(|t| t.editor.is_some() || t.tabular.is_some())
    }

    /// 同一文件已开的 tab 下标(供 `open_path` 与 `App::preview_open_path`
    /// 的异步路径共用同一份"已打开则复用"判重逻辑)。
    pub fn find_existing_file_tab(&self, path: &std::path::Path) -> Option<usize> {
        self.tabs
            .iter()
            .position(|t| t.kind == TabKind::File(path.to_path_buf()))
    }

    /// 取某个预览 tab 内部原生 `CodeView` 的 iced 焦点 id——编辑器右键菜单
    /// 的"复制/剪切/粘贴"要合成 ⌘C/⌘X/⌘V 键盘事件、且事件必须作用于这个
    /// 编辑器,靠的就是先把焦点拨到这个 id(见 `App::preview_editor_focus_id`
    /// 与 `pending_native_menu_edit_key` 的用法)。没有原生编辑器(表格/webview
    /// 档 / 占位 tab)返回 `None`,调用方据此取消菜单的剪贴板动作。
    pub fn editor_focus_id(&self, tab_id: usize) -> Option<iced_widget::core::widget::Id> {
        self.tabs
            .iter()
            .find(|t| t.id == tab_id)
            .and_then(|t| t.editor.as_ref())
            .map(|e| e.focus_id())
    }

    pub fn open_path(&mut self, path: PathBuf) -> usize {
        // 同一文件已开则切过去,不重复开 tab（验收反馈）。
        if let Some(idx) = self.find_existing_file_tab(&path) {
            let id = self.tabs[idx].id;
            self.active = idx;
            // 直接切激活(不经过 `push_tab`)—若换到的文件不是正在搜索的,丢条。
            self.cull_stale_find();
            return id;
        }
        let title = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        self.push_tab(TabKind::File(path), title)
    }

    /// 追加一个真实 tab(占位 `Blank` 恒在 index 0,这里只用来加 `File`)。
    /// 文件 tab 一律追加在末尾,占位 tab 永远留在最前面,形态对齐 SSH/数据库
    /// 面板 tab 条最前面那个固定"空白"占位。**这是同步路径**——阻塞调用方
    /// 线程读盘+构造,供测试 fixture 与内部小文件场景(README 自动预览等)
    /// 用;用户从文件树打开任意大小文件走 `App::preview_open_path` 的异步路径
    /// (`insert_loading_tab` + `apply_native_load`,见下方),不经过这里。
    fn push_tab(&mut self, kind: TabKind, title: String) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        let (route, backend, backend_state) = match &kind {
            TabKind::File(path) => {
                let (route, backend, state) = route_and_backend(path, &self.capabilities);
                (Some(route), Some(backend), state)
            }
            TabKind::Blank => (None, None, BackendState::Ready),
        };
        let native_load = match &kind {
            TabKind::File(path)
                if route.as_ref().is_some_and(|route| {
                    !codemirror_enabled()
                        && matches!(
                            route.kind,
                            PreviewKind::Code | PreviewKind::Json | PreviewKind::Streamed
                        )
                }) =>
            {
                read_and_build_native_editor(path).ok()
            }
            _ => None,
        };
        let (editor, loaded_bytes, total_bytes, truncated) = match native_load {
            Some(load) => (
                Some(load.view),
                load.loaded_bytes,
                load.total_bytes,
                load.truncated,
            ),
            None => (None, 0, 0, false),
        };
        // 表格类文件委托 Tabular Viewer(与 editor 互斥)。实际解析是异步的
        // (calamine/csv 对大文件可能要跑一阵,不能卡在这个同步方法里,见
        // `crate::tabular` 模块文档的"够数即停"性能策略)——这里只登记
        // "这是个表格 tab、正在加载",把 `(id, path)` 记进
        // `pending_tabular_loads` 队列,交给调用方(手上有 handle/proxy 那层,
        // 即 `app/update.rs` 的 `preview_open_path`/`project_preview_open_path`)
        // 在 `push_tab` 返回之后立即 `take_pending_tabular_loads()` 取走并
        // spawn 后台加载。`open_path` 复用已存在 tab 的分支不经过 `push_tab`,
        // 不会重复入队,不会对一个正在加载的 tab 触发第二次加载。
        let tabular = match &kind {
            TabKind::File(path)
                if route
                    .as_ref()
                    .is_some_and(|route| route.kind == PreviewKind::Tabular) =>
            {
                self.pending_tabular_loads.push((id, path.clone()));
                Some(TabularState::Loading)
            }
            _ => None,
        };
        // JSON/JSONL:在原生代码编辑器之外**额外**挂一个树查看器(双视图,
        // 不排斥 editor)。同 tabular,只登记"正在加载",`(id, path)` 交给
        // 调用方 `take_pending_json_tree_loads()` 取走 spawn 后台加载。
        let json_tree = match &kind {
            TabKind::File(path)
                if route.as_ref().is_some_and(|route| {
                    matches!(route.kind, PreviewKind::Json | PreviewKind::Streamed)
                }) =>
            {
                self.pending_json_tree_loads.push((id, path.clone()));
                Some(JsonTreeState::Loading)
            }
            _ => None,
        };
        // 新建原生编辑器 tab:键盘事件无需先点击一次即可直达编辑器(见
        // `pending_editor_focus` 文档)。
        if editor.is_some() {
            self.pending_editor_focus = true;
        }
        let backend_state = if tabular.is_some() || json_tree.is_some() {
            BackendState::Loading
        } else {
            backend_state
        };
        let windowed = match &kind {
            TabKind::File(path) => path_is_windowed(path, &self.capabilities),
            TabKind::Blank => false,
        };
        let tab = PreviewTab {
            id,
            kind,
            title,
            reload_nonce: 0,
            editor,
            tabular,
            json_tree,
            dirty: false,
            loaded_bytes,
            total_bytes,
            truncated,
            loading: false,
            pending_jump_line: None,
            route,
            backend,
            backend_state,
            windowed,
            window_index: None,
            recovery_written: false,
            pending_restore: None,
            load_started: None,
            pending_view: None,
            web_revision: 0,
            web_selection: None,
            web_selected_text: None,
            web_viewport: None,
            web_error: None,
        };
        tab.debug_assert_backend_consistent();
        self.tabs.push(tab);
        self.active = self.tabs.len() - 1;
        // 新 tab 成为激活者(可能顶掉旧 find tab)——清掉不再匹配的 Find
        // (譬如把搜索着的文件替换掉了,或有 Blank 顶到激活位)。对"同一文件复用
        // 已存在 tab"的 `open_path` 路径,`push_tab` 不跑,见其自行 cull。
        self.cull_stale_find();
        id
    }

    /// 追加一个"原生编辑器候选、但内容尚未读到"的占位 tab,立即返回 `id`——
    /// 供调用方(`App::preview_open_path`)紧接着 spawn 异步读盘+构造任务,
    /// 完成后用 `apply_native_load` 回填。`title`/`pending_editor_focus`/
    /// `cull_stale_find` 等副作用与 `push_tab` 对齐(新 tab 成为激活者、清
    /// stale find)。
    pub fn insert_loading_tab(&mut self, kind: TabKind, title: String) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        // JSON 走的就是这条异步路径(`is_native_editor_candidate(json)==true`),
        // 所以 json_tree 的双视图登记必须在这里也做一遍——只在 `push_tab`
        // 登记会让用户从文件树打开 JSON(走异步路径)时拿不到树视图。
        let (route, backend) = match &kind {
            TabKind::File(path) => {
                let (route, backend, _) = route_and_backend(path, &self.capabilities);
                (Some(route), Some(backend))
            }
            TabKind::Blank => (None, None),
        };
        let windowed = match &kind {
            TabKind::File(path) => path_is_windowed(path, &self.capabilities),
            TabKind::Blank => false,
        };
        let json_tree = match &kind {
            TabKind::File(path)
                if route.as_ref().is_some_and(|route| {
                    matches!(route.kind, PreviewKind::Json | PreviewKind::Streamed)
                }) =>
            {
                self.pending_json_tree_loads.push((id, path.clone()));
                Some(JsonTreeState::Loading)
            }
            _ => None,
        };
        let tab = PreviewTab {
            id,
            kind,
            title,
            reload_nonce: 0,
            editor: None,
            tabular: None,
            json_tree,
            dirty: false,
            loaded_bytes: 0,
            total_bytes: 0,
            truncated: false,
            loading: true,
            pending_jump_line: None,
            route,
            backend,
            backend_state: BackendState::Loading,
            windowed,
            window_index: None,
            recovery_written: false,
            pending_restore: None,
            load_started: None,
            pending_view: None,
            web_revision: 0,
            web_selection: None,
            web_selected_text: None,
            web_viewport: None,
            web_error: None,
        };
        tab.debug_assert_backend_consistent();
        self.tabs.push(tab);
        self.active = self.tabs.len() - 1;
        self.cull_stale_find();
        id
    }

    /// 追加一个"只恢复 tab 壳、内容未加载"的 `Suspended` 文件 tab(启动恢复
    /// 用,Phase C Task 5)。route/backend 在创建时就定好,但**不建任何 viewer、
    /// 不读盘**;真正切到/打开它时再由 `Workspace::load_preview_tab` 物化。
    /// 返回 id,且**不改 active**(恢复顺序由调用方决定)。
    pub fn push_shell_tab(&mut self, path: PathBuf, mode: Option<PreviewMode>) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        let title = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        let (route, mut backend, _) = route_and_backend(&path, &self.capabilities);
        // 用户持久化的 mode 只在该 route 支持时覆盖默认值。
        let route = match mode {
            Some(m) if route.supports(m) => PreviewRoute {
                default_mode: m,
                reason: RouteReason::PersistedMode,
                ..route
            },
            _ => route,
        };
        // Json backend 的 Tree/Text 模式跟着 route 默认值走(from_route 时是
        // 按未覆盖的默认值构造的,这里补一次)。
        if let PreviewBackend::Json(json) = &mut backend {
            json.mode = match route.default_mode {
                PreviewMode::Text => JsonMode::Text,
                _ => JsonMode::Tree,
            };
        }
        // Rendered(Markdown/HTML)的 Source 持久模式同样补一次。
        if let PreviewBackend::Rendered(rendered) = &mut backend {
            rendered.mode = match route.default_mode {
                PreviewMode::Source => RenderedMode::Source,
                _ => RenderedMode::Rendered,
            };
        }
        let windowed = path_is_windowed(&path, &self.capabilities);
        let tab = PreviewTab {
            id,
            kind: TabKind::File(path),
            title,
            reload_nonce: 0,
            editor: None,
            tabular: None,
            json_tree: None,
            dirty: false,
            loaded_bytes: 0,
            total_bytes: 0,
            truncated: false,
            loading: false,
            pending_jump_line: None,
            route: Some(route),
            backend: Some(backend),
            backend_state: BackendState::Suspended,
            windowed,
            window_index: None,
            recovery_written: false,
            pending_restore: None,
            load_started: None,
            pending_view: None,
            web_revision: 0,
            web_selection: None,
            web_selected_text: None,
            web_viewport: None,
            web_error: None,
        };
        self.tabs.push(tab);
        id
    }

    /// 某个 tab 是否还停在 `Suspended`(只恢复了壳,未物化)。
    pub fn is_suspended(&self, tab_id: usize) -> bool {
        self.tabs
            .iter()
            .find(|t| t.id == tab_id)
            .is_some_and(|t| matches!(t.backend_state, BackendState::Suspended))
    }

    /// 物化一个 Suspended 壳:标记为 `Loading`(Suspended→Loading 合法)。
    /// 返回 false 表示该 tab 不存在或已不是 Suspended(幂等保护)。
    pub fn begin_shell_load(&mut self, tab_id: usize) -> bool {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) else {
            return false;
        };
        if !matches!(tab.backend_state, BackendState::Suspended) {
            return false;
        }
        let _ = tab.backend_state.try_transition(BackendState::Loading);
        true
    }

    /// 物化完成(Loading→Ready),或同步可立即就绪的 shell 直接置 Ready。
    pub fn finish_shell_load(&mut self, tab_id: usize) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
            let _ = tab.backend_state.try_transition(BackendState::Ready);
        }
    }

    /// 物化时把 tab 标为 iced 异步读盘中(`loading=true`,供渲染/期望清单判据)。
    pub fn set_tab_loading(&mut self, tab_id: usize, loading: bool) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
            tab.loading = loading;
        }
    }

    /// 物化 JSON/Streamed shell:登记 json_tree 后台加载并置 Loading 态。
    pub fn set_json_tree_loading(&mut self, tab_id: usize, path: PathBuf) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
            tab.json_tree = Some(JsonTreeState::Loading);
        }
        self.pending_json_tree_loads.push((tab_id, path));
    }

    /// 物化 Tabular shell:登记表格后台加载并置 Loading 态。
    pub fn set_tabular_loading(&mut self, tab_id: usize, path: PathBuf) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
            tab.tabular = Some(TabularState::Loading);
        }
        self.pending_tabular_loads.push((tab_id, path));
    }

    /// 记录启动恢复的视图状态,editor `ready` 后应用一次。
    pub fn set_pending_view(
        &mut self,
        tab_id: usize,
        cursor: Option<crate::preview::TextPosition>,
        selection: Option<crate::preview::TextRange>,
        top_line: Option<u32>,
    ) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
            tab.pending_view = Some((cursor, selection, top_line));
        }
    }

    /// 异步读盘+构造结果回灌:按 `tab_id` 定位(用户可能在结果回来前关掉/
    /// 切走这个 tab,找不到就静默丢弃)。成功则把已经在后台线程构造好的
    /// `CodeView` 取出装进 tab(`NativeEditorLoadHandle::take` 只应在这里
    /// 调用一次)并置一次性聚焦位(同步路径 `push_tab` 原有行为);失败则
    /// 保持 `editor: None`(该 tab 落回 webview/flyfish 兜底,`loading` 已
    /// 置假,`desired_webviews()` 会在下一帧自然把它纳入期望清单)。
    pub fn apply_native_load(
        &mut self,
        tab_id: usize,
        result: Result<native_editor::NativeEditorLoadHandle, String>,
    ) {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) else {
            return;
        };
        tab.loading = false;
        let mut loaded = false;
        if let Ok(handle) = result
            && let Some(load) = handle.take()
        {
            tab.editor = Some(load.view);
            tab.loaded_bytes = load.loaded_bytes;
            tab.total_bytes = load.total_bytes;
            tab.truncated = load.truncated;
            self.pending_editor_focus = true;
            loaded = true;
            // 外部面板（代码健康度）请求的"打开后跳转定位"：编辑器刚填上，
            // 消费掉 pending 行号并把光标落过去（1-based → 0-based）。
            if let Some(line) = tab.pending_jump_line.take()
                && let Some(editor) = tab.editor.as_mut()
            {
                editor.move_cursor_to((line.saturating_sub(1), 0));
            }
        }
        // backend 状态机与真实的成功/失败对齐:成功 -> Ready;失败保持
        // editor: None 并标记 Failed(该 tab 会按旧行为落回 webview 兜底,
        // 见 `hosts_webview` 的兜底子句)。
        if loaded {
            if !matches!(tab.json_tree, Some(JsonTreeState::Loading)) {
                tab.backend_state.try_transition(BackendState::Ready);
            }
        } else {
            tab.backend_state
                .try_transition(BackendState::Failed(PreviewError::new(
                    "原生编辑器加载失败",
                    true,
                )));
        }
    }

    /// "加载更多"异步续读结果回灌:tab 已不存在/没有 `editor`(结果回来前
    /// 用户关掉了 tab,或该 tab 根本不是原生编辑器)则 no-op。
    pub fn apply_more_loaded(
        &mut self,
        tab_id: usize,
        result: Result<(String, u64, bool), String>,
    ) {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) else {
            return;
        };
        let Ok((more_text, new_loaded, truncated)) = result else {
            return;
        };
        if let Some(editor) = &mut tab.editor {
            editor.append_text(&more_text);
        }
        tab.loaded_bytes = new_loaded;
        tab.truncated = truncated;
    }

    /// 打开只读大文件档搜索条,锁定 `tab_id`。已开着且锁的是同一个 tab 则
    /// no-op(保留已输入的 query,同 `FindState` 既有语义);换 tab 重开会
    /// 丢旧会话(`query`/`hits` 清空)。
    pub fn open_large_file_search(&mut self, tab_id: usize) {
        if self
            .large_file_search
            .as_ref()
            .is_some_and(|s| s.tab_id == tab_id)
        {
            return;
        }
        self.large_file_search = Some(LargeFileSearch {
            tab_id,
            ..Default::default()
        });
    }

    pub fn close_large_file_search(&mut self) {
        self.large_file_search = None;
    }

    /// 只读访问器,供渲染层判断要不要画大文件搜索条(同 `find_state` 的既有
    /// 写法)。
    pub fn large_file_search_state(&self) -> Option<&LargeFileSearch> {
        self.large_file_search.as_ref()
    }

    /// 异步搜索结果回灌:会话已被关闭,或已换锁到别的 tab(用户在结果回来
    /// 前又做了别的操作)时静默丢弃。
    pub fn set_large_file_search_results(
        &mut self,
        tab_id: usize,
        result: Result<Vec<crate::extensions::search::SearchHit>, String>,
    ) {
        let Some(session) = self.large_file_search.as_mut() else {
            return;
        };
        if session.tab_id != tab_id {
            return;
        }
        session.running = false;
        if let Ok(hits) = result {
            session.hits = hits;
            session.current = 0;
        }
    }

    /// 当前激活 tab 若是**文件**且走 wry 路径则返回其 id(=webview 池的 key)。
    /// 原生渲染 tab(有 `editor`)与 `Blank` 占位都返回 `None`——它们不进
    /// webview 池(`desired_webviews()` 同样跳过这两类),返回一个池里并不
    /// 存在的 id 会把"当前激活的是不是真 webview"这个问题答错。
    pub fn active_webview_id(&self) -> Option<usize> {
        self.tabs
            .get(self.active)
            .filter(|t| t.hosts_webview())
            .map(|t| t.id)
    }
    /// 手动点 tab / 打开时切到已存在 tab。切到**另一个**文件 tab 时,若目标
    /// 走 wry 路径(没有原生 editor)则顺手推进 `reload_nonce`,让 webview
    /// 切回来时重新 `load_url` 读到磁盘最新内容(同右键"刷新"的机制)。原生
    /// editor tab 不动——它重载会重建实例、丢滚动/只读态,按品鉴保留(用户在
    /// 别处手动确认过:原生 tab 切回不自动重载)。点当前已激活 tab 是 no-op。
    pub fn select(&mut self, idx: usize) {
        if idx >= self.tabs.len() || idx == self.active {
            return;
        }
        let is_webview_file = self.tabs[idx].hosts_webview();
        self.active = idx;
        if is_webview_file {
            let id = self.tabs[idx].id;
            self.bump_reload(id);
        }
        self.cull_stale_find();
    }

    /// 项目切换清理专用:只留下一个 `Blank` 占位 tab(§对 `Default` 的同一
    /// 不变式)。调用方(`Workspace::close_all_tabs_for_switch`)拿到的这份
    /// pane 要换主人,旧项目的文件 tab 全丢弃,但"空白占位恒在第 0 项"这条
    /// 不变式不因换项目而破——落回空白页,而不是一个没有占位 tab 的空列表。
    pub fn clear_all(&mut self) {
        self.tabs.clear();
        self.tabs.push(placeholder_tab(self.next_id));
        self.next_id += 1;
        self.active = 0;
        // 整个 pane 换主人/清空:Find 必然失配,直接丢。
        self.find = None;
        // 空白页信息卡也丢——项目根路径变了,旧结果失配;`apply_pending_blank_info`
        // 会在新项目上重新 spawn 一次。
        self.blank_info = None;
        self.blank_info_in_flight = false;
    }

    /// 关掉一个 tab。越界是 no-op。`TabKind::Blank`(index 0)与文件 tab 一视同仁
    /// ——**可以关**,前提是列表里还有别的 tab(用户关闭 Blank 是为了腾出位);当
    /// 关掉后列表为空,**自动补回一个全新的 Blank**(`next_id` 续号,id 不复用),
    /// 模拟浏览器"关到只剩新标签页"的体验,而不是让面板停在空 list 这种不合法
    /// 状态。`active` 同步收敛:关后空 → `0`(新 Blank);否则沿用旧逻辑——
    /// 越界压到末位、`idx < active` 左移一格。`close` 触发的"激活换到不同文件"
    /// 顺路清掉失配的 Find。
    pub fn close(&mut self, idx: usize) {
        if idx >= self.tabs.len() {
            return;
        }
        self.tabs.remove(idx);
        if self.tabs.is_empty() {
            // 关到只剩空气 → 自动补一个 Blank 占位,id 用 `next_id` 续号
            // (新 Blank 与被删的 Blank id 不同,不影响旧 Find 会话比对)。
            self.tabs.push(placeholder_tab(self.next_id));
            self.next_id += 1;
            self.active = 0;
        } else if self.active >= self.tabs.len() {
            self.active = self.tabs.len().saturating_sub(1);
        } else if idx < self.active {
            self.active -= 1;
        }
        // 关掉正在搜的 tab / 关闭导致激活换到别的文件:清掉失配的 Find。
        self.cull_stale_find();
    }

    /// 拖拽换位:把 `from` 处的 tab 移到 `to` 处,并同步 `active` 下标。`from`
    /// 与 `to` 相等或越界时是 no-op。index 0 的 `Blank` 占位 tab 固定在首位,
    /// 任何牵扯到它的换位(把它拖走、或把别的 tab 拖到它前面)都是 no-op——
    /// 与 SSH/数据库面板"空白占位恒在最前"的形态一致。
    pub fn reorder(&mut self, from: usize, to: usize) {
        if from == 0 || to == 0 || from == to || from >= self.tabs.len() || to >= self.tabs.len() {
            return;
        }
        let tab = self.tabs.remove(from);
        self.tabs.insert(to, tab);
        if self.active == from {
            self.active = to;
        } else if from < self.active && to >= self.active {
            // 源在激活项左侧,且目标落到了激活项右侧/身上——激活项左移一位。
            self.active -= 1;
        } else if from > self.active && to <= self.active {
            // 源在激活项右侧,且目标落到了激活项左侧/身上——激活项右移一位。
            self.active += 1;
        }
        // 拖拽换位可能把激活项换到别的文件——激活指的已是不同 tab 时丢 Find。
        self.cull_stale_find();
    }

    /// webview 期望清单:每文件 tab 一个,仅激活者可见(设计 D2)。是否走
    /// webview 由统一 backend 判定(见 `PreviewTab::hosts_webview`)。
    pub fn desired_webviews(&self) -> Vec<WebviewSpec> {
        self.tabs
            .iter()
            .enumerate()
            .filter(|(_, tab)| tab.hosts_webview())
            .filter_map(|(idx, tab)| {
                // `Blank` 没有 wry 页面(内容区是纯 iced 渲染的 Dozer 品牌标),
                // 不进期望清单——`sync_webview_pool` 据此不会为它创建 webview。
                let TabKind::File(path) = &tab.kind else {
                    return None;
                };
                let mut u = preview_url(path);
                if tab.reload_nonce > 0 {
                    // flyfish URL 已经带 `?p=...` 查询串,html 的 file:// URL
                    // 还没有——按 URL 是否已有查询串决定用 `&` 还是 `?` 起头。
                    let sep = if u.contains('?') { '&' } else { '?' };
                    u.push_str(&format!("{sep}_r={}", tab.reload_nonce));
                }
                Some(WebviewSpec {
                    id: tab.id,
                    url: u,
                    visible: idx == self.active,
                    editor_binding: None,
                })
            })
            .collect()
    }

    pub fn desired_editor_webviews(
        &self,
        project_id: i64,
        panel: crate::app::PanelKind,
    ) -> Vec<WebviewSpec> {
        if !codemirror_enabled() {
            return Vec::new();
        }
        self.tabs
            .iter()
            .enumerate()
            .filter_map(|(idx, tab)| {
                let TabKind::File(path) = &tab.kind else {
                    return None;
                };
                if tab.loading || !tab.backend_state.is_ready() || tab.editor.is_some() {
                    return None;
                }
                // 语言与只读先从 backend 推出:Code(可含窗口化只读)或
                // Rendered 的 Source 模式(Markdown/HTML 源码)。
                // 只读从 backend 推出:Code(可含窗口化只读)只读;Rendered 的
                // Source 模式(Markdown/HTML 源码)可编辑。语言由 `url()` 按扩展名给。
                let read_only = match tab.backend.as_ref()? {
                    PreviewBackend::Code(code) => {
                        tab.windowed || matches!(code.mode, CodeMode::ReadOnly)
                    }
                    PreviewBackend::Rendered(rendered) if rendered.mode == RenderedMode::Source => {
                        false
                    }
                    // JSON/Streamed 的 Text 模式(feature 下由 editor host 承载;
                    // Tree/Streamed 视图仍走原生 json_tree)。
                    PreviewBackend::Json(json) if json.mode == JsonMode::Text => tab.windowed,
                    PreviewBackend::Streamed(streamed) if streamed.mode == PreviewMode::Text => {
                        tab.windowed
                    }
                    // CSV/TSV 原文模式:可编辑纯文本。
                    PreviewBackend::Tabular(tabular) if tabular.mode == TabularMode::Text => false,
                    _ => return None,
                };
                let binding = EditorHostBinding::new(project_id, panel, tab.id, path.clone());
                let mut url = binding.url(scheme_query_value(), read_only);
                if tab.windowed {
                    // 窗口化只读:正文由 Rust 经 set_window 推送,host 不自行拉取。
                    url.push_str("&windowed=1");
                }
                // 外部变更/右键刷新的重载:换 URL 逼 WebView 重新导航拉取最新
                // 内容(与 flyfish 的 `_r=` 同一手法)。窗口化 tab 的重载会重推窗口。
                if tab.reload_nonce > 0 {
                    url.push_str(&format!("&_r={}", tab.reload_nonce));
                }
                Some(WebviewSpec {
                    id: tab.id,
                    url,
                    visible: idx == self.active,
                    editor_binding: Some(binding),
                })
            })
            .collect()
    }

    /// 按 `PreviewTab.id` 把某个原生 tab 标脏(当且仅当其编辑器收到过"改正文"
    /// 的 Action 时由 Workspace 转发层调用;见 `preview_tab_editor_event`)。
    /// tab 不存在/非原生时 no-op。
    pub fn mark_dirty_by_id(&mut self, tab_id: usize) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id)
            && tab.editor.is_some()
        {
            tab.dirty = true;
        }
    }

    /// 按 `PreviewTab.id` 清除脏标记(⌘S 成功落盘后调用)。
    pub fn clear_dirty_by_id(&mut self, tab_id: usize) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
            tab.dirty = false;
        }
    }

    /// Find 条目当前是否显示(至少打开过一次且没被生命周期/手动关掉)。
    pub fn find_bar_open(&self) -> bool {
        self.find.is_some()
    }

    /// 针对**当前激活** tab 打开(或刷新)Find。原生 editor tab 走
    /// `CodeView::find_matches_all` 同步现算那一套;webview(flyfish) tab 走
    /// flyfish 自带搜索 API——此时 `is_webview=true` 且锁定 `webview_pool_id`
    /// (webview 池 key,已含 Project 偏移),真正的 JS 注入延后到
    /// `window_events::apply_pending_preview_find`(句柄只那里拿得到)。
    ///
    /// 已对该 tab 开着时是 no-op(⌘F 连按只把 focus 还给输入框,不清输入内容);
    /// 切到别的文件后再开,丢弃旧会话重建空 query。激活 tab 是 Blank 占位时
    /// no-op——Find 只对"有内容可搜"的 tab 有意义。
    ///
    /// `replace_open` 定住这次打开动作要的替换行展开态——⌘F 传 `false`(收起)、
    /// ⌘R 传 `true`(展开),每次调用都显式生效(哪怕会话已开着),不是只在
    /// 新建时起作用:用户按下的是哪个快捷键,条就该立刻呈现对应形态。webview
    /// 档 flyfish 没有替换概念,`replace_open` 在这里被强制收起。
    ///
    /// `webview_pool_id` 是调用方(`Workspace::preview_find_open` 已用
    /// `active_preview_webview_id(kind)` 算好的、含 Project 偏移的池 key)传
    /// 进来的——只有激活 tab 正好是 webview 时才会被用到,原生档忽略它。
    pub fn open_find_on_active(&mut self, replace_open: bool, webview_pool_id: Option<usize>) {
        // 优先:激活 tab 是原生 editor → 原生 Find 会话。
        if let Some(active_id) = self
            .tabs
            .get(self.active)
            .filter(|t| t.editor.is_some())
            .map(|t| t.id)
        {
            if self.find.as_ref().is_some_and(|f| f.tab_id == active_id) {
                if let Some(f) = self.find.as_mut() {
                    f.replace_open = replace_open;
                }
            } else {
                self.find = Some(FindState {
                    tab_id: active_id,
                    query: String::new(),
                    current: 0,
                    count: 0,
                    case_sensitive: false,
                    replacement: String::new(),
                    replace_open,
                    query_focused: false,
                    is_webview: false,
                    pending_webview_exec: None,
                    webview_pool_id: None,
                });
            }
            return;
        }
        // 其次:激活 tab 是 flyfish webview(无原生 editor、是文件、未在读盘
        // 中)→ webview Find 会话。锁 webview 池 key,强制收起替换行(无替换)。
        let Some(active_id) = self.active_webview_id() else {
            return;
        };
        if self.find.as_ref().is_some_and(|f| f.tab_id == active_id) {
            if let Some(f) = self.find.as_mut() {
                f.replace_open = false;
                // 连按 ⌘F 重触发一次搜索,把已输入 query 重新高亮出来。
                f.pending_webview_exec = Some(WebviewFindAction::Search);
            }
        } else {
            self.find = Some(FindState {
                tab_id: active_id,
                query: String::new(),
                current: 0,
                count: 0,
                case_sensitive: false,
                replacement: String::new(),
                replace_open: false,
                query_focused: false,
                is_webview: true,
                pending_webview_exec: Some(WebviewFindAction::Search),
                webview_pool_id,
            });
        }
    }

    /// 查询框前的圆盘箭头点击:手动翻转替换行展开态,不受 ⌘F/⌘R 的固定值
    /// 约束。条未开是 no-op。
    pub fn toggle_find_replace(&mut self) {
        if let Some(f) = self.find.as_mut() {
            f.replace_open = !f.replace_open;
        }
    }

    /// 关掉 Find 条目(⌘F 里输入框 ×、Esc、切走文件后的自动清扫都走这里)。
    /// webview(flyfish)档丢会话前先把"该清哪块 webview 高亮"记下来,等
    /// `apply_pending_preview_find` 在事件环里注入 `clearDocumentSearch()`。
    pub fn close_find(&mut self) {
        if let Some(id) = self.find.as_ref().and_then(|f| f.webview_pool_id) {
            self.pending_webview_find_clear = Some(id);
        }
        self.find = None;
    }

    /// 取走(消费式)webview(flyfish)档 Find 待下发的搜索动作 + 当前查询词 +
    /// 大小写敏感开关,供 `platform/window_events::apply_pending_preview_find`
    /// 注入 flyfish JS。不是 webview 会话或没有待下发动作时返回 `None`;取走即
    /// 把 `pending_webview_exec` 清回 `None`(同 `pending_focus` 的"派发完消息后
    /// 轮询"节奏,避免重复注入)。
    pub fn take_pending_webview_find(&mut self) -> Option<(WebviewFindAction, String, bool)> {
        if self.find.as_ref()?.is_webview {
            let action = self.find.as_mut()?.pending_webview_exec.take()?;
            let query = self.find.as_ref()?.query.clone();
            let cs = self.find.as_ref()?.case_sensitive;
            Some((action, query, cs))
        } else {
            None
        }
    }

    /// 取走(消费式)webview(flyfish)档待清理的 webview 池 key,供
    /// `apply_pending_preview_find` 注入 `clearDocumentSearch()` 抹掉高亮。无则
    /// `None`(取走即清)。
    pub fn take_pending_webview_find_clear(&mut self) -> Option<usize> {
        self.pending_webview_find_clear.take()
    }

    /// Find 会话只读引用(视图展示 n/m 与判灰用);未打开时 `None`。
    pub fn find_state(&self) -> Option<&FindState> {
        self.find.as_ref()
    }

    /// 查询输入框是否持有真实焦点;条未开恒 `false`。
    pub fn find_query_focused(&self) -> bool {
        self.find.as_ref().is_some_and(|f| f.query_focused)
    }

    /// 每帧渲染循环用 `take_find_focused` 查回来的真实焦点态写进这里;
    /// 条未开是 no-op(没有 `FindState` 可写)。
    pub fn set_find_query_focused(&mut self, focused: bool) {
        if let Some(f) = self.find.as_mut() {
            f.query_focused = focused;
        }
    }

    /// 用户在输入框里编辑 query。每次落键就同步进 `state.query`,并**当场**按新
    /// query 在锁定 buffer 上重算:命中>0 就把第 1 个**整段选中**([`select_range`],
    /// 选区在代码里高亮、光标停在命中末缘,方向和「下一个」一致),否则切到
    /// "无命中"展示(count 0、输入框下灰提示),不移动光标。query 为空同样只清展示
    /// 不动光标。
    ///
    /// 为什么不只 `move_cursor_to` 落个点:find_type 一有命中就应从 Find 输入框
    /// **自动在正文里框出关键词**(用户输入时眼睛跟着查询词的位置),只把光标放
    /// 起点是无选区的裸光标,代码里看不到任何东西被选中。
    pub fn find_type(&mut self, query: String) {
        // webview(flyfish)档:只把 query 落进状态、挂一次待下发的搜索动作,
        // 真正的命中清点交给 flyfish API(`apply_pending_preview_find` 注入
        // `searchDocument`),不在这里碰 `CodeView`(webview tab 没有原生 editor)。
        if self.find.as_ref().is_some_and(|f| f.is_webview) {
            if let Some(s) = self.find.as_mut() {
                s.query = query;
                s.pending_webview_exec = Some(WebviewFindAction::Search);
            }
            return;
        }
        let Some(tab_id) = self.find.as_ref().map(|f| f.tab_id) else {
            return;
        };
        // 先放 query 进状态(下一段要读它重算)。
        if let Some(state) = self.find.as_mut() {
            state.query = query;
        }
        // 现算命中:清点并选第 1 个整段。
        let matches = {
            let Some(editor) = self
                .tabs
                .iter()
                .find(|t| t.id == tab_id)
                .and_then(|t| t.editor.as_ref())
            else {
                return;
            };
            editor.find_matches_all(
                &self.find.as_ref().unwrap().query,
                self.find.as_ref().unwrap().case_sensitive,
            )
        };
        let count = matches.len();
        let first = matches.first().copied();
        // 写回 count/current。
        if let Some(state) = self.find.as_mut() {
            state.count = count;
            state.current = 0;
        }
        // 有命中就把第 1 个整段选中([s,e)、光标落末缘),没命中/空 query 不留选区。
        if let Some((start, end)) = first
            && let Some(editor) = self.editor_mut(tab_id)
        {
            editor.select_range(start, end);
            // 编辑器此刻没有真 iced 焦点(Find 输入框才有),选区不会被原生
            // `text_editor` 画出来——武装补聚焦标记,见 `pending_editor_reveal_
            // focus` 文档。
            self.pending_editor_reveal_focus = true;
        }
    }

    /// 导航到下一个/上一个命中,落点**以当下光标为锚**而非内部序号:先按 `query`
    /// 在锁定 buffer 上现算命中列表(编辑正文会改变命中集,永远以当下 buffer 为
    /// 准),读回缓冲区内真实光标坐标,解出"光标站在/贴着哪个命中"(站在命中内部、
    /// 或停在某命中边界——normal 相邻时以**前缘含、末缘不含**判归属),再朝方向
    /// 步进一格并循环(光标在当前命中起点再按「上一个」会 wrap 到最后一个等)。
    ///
    /// 为什么以光标为准:用户可能先把光标点到文件中段再看那一带,再按「下一个/
    /// 上一个」就应以可见区域为起点就近接续,而不是从条的计数 0 一路算。落点选中
    /// 整段命中;**光标落边与方向一致**——「下一个」把光标停到命中**末缘**、
    /// 「上一个」停到命中**前缘**([`CodeView::select_range_backward`]),这样同向
    /// 连按能单调续走得动,不会因锚点卡在原命中原处。query 空或 0 命中不动作。
    pub fn find_go(&mut self, next: bool) {
        // webview(flyfish)档:翻命中只挂一次待下发的 `Next`/`Prev` 动作,真正
        // 的跳转高亮交给 `apply_pending_preview_find` 注入
        // `nextSearchResult`/`previousSearchResult`;不在这里用 `CodeView` 现算。
        if self.find.as_ref().is_some_and(|f| f.is_webview) {
            if let Some(s) = self.find.as_mut() {
                s.pending_webview_exec = Some(if next {
                    WebviewFindAction::Next
                } else {
                    WebviewFindAction::Prev
                });
            }
            return;
        }
        let Some(state) = self.find.as_ref() else {
            return;
        };
        let empty_query = state.query.is_empty();
        let tab_id = state.tab_id;
        let (hits, caret) = {
            let Some(editor) = self
                .tabs
                .iter()
                .find(|t| t.id == tab_id)
                .and_then(|t| t.editor.as_ref())
            else {
                return;
            };
            if empty_query {
                return;
            }
            let q = self.find.as_ref().unwrap().query.clone();
            let cs = self.find.as_ref().unwrap().case_sensitive;
            (editor.find_matches_all(&q, cs), editor.cursor_position())
        };
        let n = hits.len();
        if n == 0 {
            if let Some(s) = self.find.as_mut() {
                s.count = 0;
            }
            return;
        }
        // 纵坐标比较:命中与光标都在同一份字节布局里,字典序(line,col)即文件序。
        let le = |a: (usize, usize), b: (usize, usize)| a.0 < b.0 || (a.0 == b.0 && a.1 <= b.1);
        let lt = |a: (usize, usize), b: (usize, usize)| a.0 < b.0 || (a.0 == b.0 && a.1 < b.1);
        // 「当前命中」:光标落在这段 [s,e) 里(s 含 e 不含)。不命中任何段时,取
        // 「最后一段起点不晚于光标」者——即光标右边还有个更近的段不算;光标压过
        // 所有段末尾时是最后一个。全段起点都在光标之后=> `None`,视"在一切之前"。
        let anchor = hits
            .iter()
            .position(|(s, e)| le(*s, caret) && lt(caret, *e))
            .or_else(|| hits.iter().rposition(|(s, _)| le(*s, caret)));
        let idx = match anchor {
            Some(a) => {
                if next {
                    (a + 1) % n
                } else {
                    (a + n - 1) % n
                }
            }
            None => {
                if next {
                    0
                } else {
                    n - 1
                }
            }
        };
        let (start, end) = hits[idx];
        if let Some(s) = self.find.as_mut() {
            s.count = n;
            s.current = idx;
        }
        if let Some(editor) = self.editor_mut(tab_id) {
            if next {
                editor.select_range(start, end);
            } else {
                editor.select_range_backward(start, end);
            }
            // 同 `find_type`:补聚焦标记,让原生 `text_editor` 画出这次跳转
            // 选中的命中(见 `pending_editor_reveal_focus` 文档)。
            self.pending_editor_reveal_focus = true;
        }
    }

    /// 编辑事件后刷新展示量:当锁定 tab 的 buffer 被就地改过(用户在条开着时回到
    /// 编辑器敲字),把 `state.count` 按当下 buffer 重算,`current` 钳到有效范围。
    /// 不移动光标(改动发生在用户聚焦编辑器处,不该被 yank)。供 Workspace 编辑
    /// 事件转发层每收到一个 Edit Action 调用;非锁定 tab/未开条是 no-op。
    pub fn find_refresh_after_edit(&mut self, edited_tab_id: usize) {
        if !self
            .find
            .as_ref()
            .is_some_and(|f| f.tab_id == edited_tab_id)
        {
            return;
        }
        let tab_id = edited_tab_id;
        let count = self
            .tabs
            .iter()
            .find(|t| t.id == tab_id)
            .and_then(|t| t.editor.as_ref())
            .map(|e| {
                e.find_matches_all(
                    &self.find.as_ref().unwrap().query,
                    self.find.as_ref().unwrap().case_sensitive,
                )
                .len()
            })
            .unwrap_or(0);
        if let Some(s) = self.find.as_mut() {
            s.count = count;
            s.current = usize::min(s.current, count.saturating_sub(1));
        }
    }

    /// 翻转大小写敏感开关(`true`=逐字严格,`false`=ASCII 大小写折叠)。只改
    /// 状态里持久下一轮的标识,**不移动光标/选区**(等价一次"编辑后刷新":把
    /// count/current 按新敏感度重算)。改完后用户再敲下一轮 query 或点下一个/
    /// 上一个即以新敏感度重搜;翻回相同值 no-op。
    pub fn set_find_case(&mut self, case_sensitive: bool) {
        // webview(flyfish)档:只翻状态里的敏感开关、挂一次待下发的搜索动作,
        // 真正的重搜交给 flyfish(`apply_pending_preview_find` 注入
        // `searchDocument` 时带上新 `caseSensitive`)。不调 `find_refresh_
        // after_edit`(那是原生 editor 的计数刷新路径)。
        if self.find.as_ref().is_some_and(|f| f.is_webview) {
            if self
                .find
                .as_ref()
                .is_some_and(|f| f.case_sensitive == case_sensitive)
            {
                return;
            }
            if let Some(s) = self.find.as_mut() {
                s.case_sensitive = case_sensitive;
                s.pending_webview_exec = Some(WebviewFindAction::Search);
            }
            return;
        }
        let Some(tab_id) = self.find.as_ref().map(|f| f.tab_id) else {
            return;
        };
        if self
            .find
            .as_ref()
            .is_some_and(|f| f.case_sensitive == case_sensitive)
        {
            return;
        }
        if let Some(s) = self.find.as_mut() {
            s.case_sensitive = case_sensitive;
        }
        self.find_refresh_after_edit(tab_id);
    }

    /// 更新「替换为」草稿(替换条的输入框每键触发)。只写 state,不做任何计算 —
    /// 真实替换发生(点「替换」/「替换全部」)时才会带着它一起扫。
    pub fn set_find_replacement(&mut self, replacement: String) {
        if let Some(s) = self.find.as_mut() {
            s.replacement = replacement;
        }
    }

    /// 「替换全部」:把当前 `query`(用 `case_sensitive` / 折叠语义 + `replacement`)
    /// 在锁定 buffer 里的一次性替换做完。与普通打字一致只改**未保存 buffer**并标
    /// 脏(`mark_dirty_by_id`),真正写盘仍交 ⌘S;这是 Find 条没有 [CLAUDE.md 裁决
    /// 的“预览尽量不改产物”]冲突的落点——改动先驻留在预览缓冲区、用户决定保存与
    /// 否。替换完按新 buffer 刷新 `count/current`。空 query / 没条 / 0 命中 no-op,
    /// 返回 `false`。
    pub fn replace_all(&mut self) -> bool {
        let Some((tab_id, query, cs, repl)) = self.find.as_ref().map(|f| {
            (
                f.tab_id,
                f.query.clone(),
                f.case_sensitive,
                f.replacement.clone(),
            )
        }) else {
            return false;
        };
        if query.is_empty() {
            return false;
        }
        let replaced = self
            .editor_mut(tab_id)
            .map(|editor| editor.replace_all(&query, cs, &repl))
            .unwrap_or(0);
        if replaced == 0 {
            return false;
        }
        self.mark_dirty_by_id(tab_id);
        // 重算之后可能有残留命中(尤其 replacement 又重现 query),count 忠实反映。
        self.find_refresh_after_edit(tab_id);
        true
    }

    /// 「替换当前命中」:把 `find_state().current` 指着的那一处替换掉(第 `nth`
    /// 个命中,窗口序与 `find_matches_all` 一致)。动作与 [`PreviewPane::replace_all`]
    /// 相同——只动未保存 buffer、标脏等 ⌘S。替换成功后把光标拨回被删匹配的起点,
    /// 再由 [`PreviewPane::find_go`] 按当下 buffer 往**下一个**命中走(替换者通常要
    /// 一路往下逐个处理;简单同字符替换时光标就卡在被换处以便继续替换)。空 query /
    /// 无命中 / 目标已是文件尾(替换后不再有该 query)都会安全 no-op 返回 `false`。
    pub fn replace_current(&mut self) -> bool {
        let Some((tab_id, query, cs, repl, n)) = self.find.as_ref().map(|f| {
            (
                f.tab_id,
                f.query.clone(),
                f.case_sensitive,
                f.replacement.clone(),
                f.current,
            )
        }) else {
            return false;
        };
        if query.is_empty() {
            return false;
        }
        // 被替换命中的起点坐标(换完 buffer 重建会丢光标,靠它把焦点落回原位再
        // 让 find_go 续next)。前缀在此之前的字节原样保留,坐标在简单替换中仍成立。
        let lost_start = {
            let Some(editor) = self
                .tabs
                .iter()
                .find(|t| t.id == tab_id)
                .and_then(|t| t.editor.as_ref())
            else {
                return false;
            };
            let all = editor.find_matches_all(&query, cs);
            let idx = n.min(all.len().saturating_sub(1));
            all.get(idx).map(|&(start, _)| start)
        };
        let did = self
            .editor_mut(tab_id)
            .map(|editor| editor.replace_nth(n, &query, cs, &repl))
            .unwrap_or(false);
        if !did {
            return false;
        }
        self.mark_dirty_by_id(tab_id);
        // 光标复位到被换处附近,再走一次「下一个」续递。
        if let Some(start) = lost_start
            && let Some(editor) = self.editor_mut(tab_id)
        {
            editor.move_cursor_to(start);
        }
        self.find_refresh_after_edit(tab_id);
        self.find_go(true);
        true
    }

    /// 内部:激活 tab / 关闭/清空导致激活的原生 tab 变了时,清掉不再匹配的 Find。
    /// tab 交换(reorder)也隐式适用。用户从"正在搜索的文件 A"切到 B 或关掉 A,
    /// 挂着上一文件的失配搜索条毫无意义,直接丢弃。webview(flyfish)档的会话
    /// 同样只在"激活 tab 仍是同一块 webview"时保留,切走/关掉就丢——丢之前把
    /// 该 webview 的搜索高亮清掉(记进 `pending_webview_find_clear`,等
    /// `apply_pending_preview_find` 注入 `clearDocumentSearch()`)。
    fn cull_stale_find(&mut self) {
        let keep = self.find.as_ref().is_some_and(|f| {
            let Some(t) = self.tabs.get(self.active) else {
                return false;
            };
            if f.is_webview {
                // webview 档:激活 tab 必须还是同一块 webview(无原生 editor)。
                t.id == f.tab_id
                    && t.editor.is_none()
                    && t.tabular.is_none()
                    && !t.loading
                    && matches!(t.kind, TabKind::File(_))
            } else {
                // 原生 editor 档:激活 tab 必须是有 editor 且 id 匹配。
                matches!(t, _ if t.editor.is_some() && t.id == f.tab_id)
            }
        });
        if !keep {
            if let Some(id) = self.find.as_ref().and_then(|f| f.webview_pool_id) {
                self.pending_webview_find_clear = Some(id);
            }
            self.find = None;
        }
    }

    /// 按 tab id 取该 tab 的原生 editor 可变引用。tab 不存在或该 tab 走 wry
    /// 路径(没有 editor)都返回 `None`。main.rs 把 `Message::PreviewEditorEvent`
    /// 转发的 `Action` 用它路由给正确的 tab。
    pub fn editor_mut(&mut self, tab_id: usize) -> Option<&mut crate::code_editor::CodeView> {
        self.tabs
            .iter_mut()
            .find(|t| t.id == tab_id)
            .and_then(|t| t.editor.as_mut())
    }

    /// 按 tab id 取该 tab 的 Tabular Viewer 可变引用。tab 不存在、该 tab 不是
    /// 表格、或表格还在后台加载中(`TabularState::Loading`)都返回 `None`
    /// (滚动/切 sheet 这类交互动作在数据到位前没有意义,直接 no-op)。
    /// workspace 把 `Message::TabularAction` 的 `Action` 用它路由给正确的
    /// tab 后 `apply`。
    pub fn tabular_mut(&mut self, tab_id: usize) -> Option<&mut crate::tabular::TabularView> {
        self.tabs
            .iter_mut()
            .find(|t| t.id == tab_id)
            .and_then(|t| t.tabular.as_mut())
            .and_then(|t| match t {
                TabularState::Ready(view) => Some(view),
                TabularState::Loading => None,
            })
    }

    /// 按 tab id 取出该 tab 的 `TabularState` 可变引用,供加载完成/懒加载
    /// sheet 完成的回填使用(`Message::TabularLoaded`/`TabularSheetLoaded`
    /// 的处理函数)。与 `tabular_mut` 不同,这个不区分 `Loading`/`Ready`——
    /// 回填就是要把 `Loading` 变成 `Ready`。
    pub fn tabular_state_mut(&mut self, tab_id: usize) -> Option<&mut TabularState> {
        self.tabs
            .iter_mut()
            .find(|t| t.id == tab_id)
            .and_then(|t| t.tabular.as_mut())
    }

    pub fn finish_tabular_load(
        &mut self,
        tab_id: usize,
        result: Result<crate::tabular::TabularView, String>,
    ) {
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) else {
            return;
        };
        match result {
            Ok(view) => {
                tab.tabular = Some(TabularState::Ready(view));
                let _ = tab.backend_state.try_transition(BackendState::Ready);
            }
            Err(message) => {
                tab.tabular = None;
                let _ = tab
                    .backend_state
                    .try_transition(BackendState::Failed(PreviewError::new(message, true)));
            }
        }
    }

    /// 取走(清空)`push_tab` 攒下的、还没被 spawn 后台加载的表格 tab 队列。
    /// 调用方(`open_path` 的上层)应在每次调用 `open_path` 之后立即取走,
    /// 不要跨调用攒着——攒着会让后来居上的取用者对着不属于自己这次
    /// `open_path` 调用产生的条目误发 spawn(虽然当前所有调用点都是"取走就
    /// 立刻 spawn",天然不会攒,但方法本身按"一次性取干净"设计更不容易踩)。
    pub fn take_pending_tabular_loads(&mut self) -> Vec<(usize, PathBuf)> {
        std::mem::take(&mut self.pending_tabular_loads)
    }

    /// 排队一个待下发给 CodeMirror editor webview 的命令(`tab_id`, 命令)。
    /// 只有 `uses_codemirror()` 的 tab 会被 `window_events` 真正注入,其余
    /// (老 iced editor / webview)在派发时按 binding 过滤掉。
    pub fn queue_editor_command(&mut self, tab_id: usize, command: EditorCommand) {
        self.pending_editor_commands.push((tab_id, command));
    }

    /// 取走(消费式)待下发的 editor 命令队列。
    #[cfg(test)]
    pub fn take_pending_editor_commands(&mut self) -> Vec<(usize, EditorCommand)> {
        std::mem::take(&mut self.pending_editor_commands)
    }

    /// 只取当前已有 WebView 句柄对应的命令；其余命令保留，等待池完成创建
    /// 或重建后下一帧重试。
    pub fn take_pending_editor_commands_for(
        &mut self,
        available_webview_ids: &std::collections::HashSet<usize>,
        project_id: i64,
        panel: crate::app::PanelKind,
    ) -> Vec<(usize, EditorCommand)> {
        let pending = std::mem::take(&mut self.pending_editor_commands);
        let mut ready = Vec::new();
        for (tab_id, command) in pending {
            let webview_id = crate::preview::EditorHostBinding::new(
                project_id,
                panel,
                tab_id,
                std::path::PathBuf::new(),
            )
            .webview_id();
            if available_webview_ids.contains(&webview_id) {
                ready.push((tab_id, command));
            } else {
                self.pending_editor_commands.push((tab_id, command));
            }
        }
        ready
    }

    /// 按 tab id 取该 tab 的 JSON 树可变引用。tab 不存在、该 tab 不是 JSON、
    /// 或树还在后台加载中(`JsonTreeState::Loading`)都返回 `None`(展开/滚动
    /// 这类交互在数据到位前没有意义)。语义与 `tabular_mut` 完全对齐。
    pub fn json_tree_mut(&mut self, tab_id: usize) -> Option<&mut crate::json_tree::JsonTreeView> {
        self.tabs
            .iter_mut()
            .find(|t| t.id == tab_id)
            .and_then(|t| t.json_tree.as_mut())
            .and_then(|t| match t {
                JsonTreeState::Ready(view) => Some(view.as_mut()),
                JsonTreeState::Loading => None,
            })
    }

    /// 按 tab id 取出该 tab 的 `JsonTreeState` 可变引用,供加载完成回填使用
    /// (要把 `Loading` 变成 `Ready`)。语义与 `tabular_state_mut` 对齐。
    pub fn json_tree_state_mut(&mut self, tab_id: usize) -> Option<&mut JsonTreeState> {
        self.tabs
            .iter_mut()
            .find(|t| t.id == tab_id)
            .and_then(|t| t.json_tree.as_mut())
    }

    pub fn finish_json_tree_load(
        &mut self,
        tab_id: usize,
        result: Result<crate::json_tree::JsonTreeView, String>,
    ) {
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) else {
            return;
        };
        match result {
            Ok(mut view) => {
                if matches!(tab.current_mode(), Some(PreviewMode::Text)) {
                    view.view_mode = crate::json_tree::ViewMode::RawText;
                }
                tab.json_tree = Some(JsonTreeState::Ready(Box::new(view)));
                let _ = tab.backend_state.try_transition(BackendState::Ready);
            }
            Err(message) => {
                tab.json_tree = None;
                match tab.backend.as_mut() {
                    Some(PreviewBackend::Json(json)) => json.mode = JsonMode::Text,
                    Some(PreviewBackend::Streamed(streamed)) => {
                        streamed.mode = PreviewMode::Text;
                    }
                    _ => {}
                }
                if tab.editor.is_some() {
                    let _ = tab.backend_state.try_transition(BackendState::Ready);
                } else if !tab.loading {
                    let _ = tab
                        .backend_state
                        .try_transition(BackendState::Failed(PreviewError::new(message, true)));
                }
            }
        }
    }

    pub fn sync_json_backend_mode(&mut self, tab_id: usize) {
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) else {
            return;
        };
        let Some(JsonTreeState::Ready(view)) = tab.json_tree.as_ref() else {
            return;
        };
        let mode = match view.view_mode {
            crate::json_tree::ViewMode::Tree => PreviewMode::Tree,
            crate::json_tree::ViewMode::RawText => PreviewMode::Text,
        };
        match tab.backend.as_mut() {
            Some(PreviewBackend::Json(json)) => {
                json.mode = if mode == PreviewMode::Text {
                    JsonMode::Text
                } else {
                    JsonMode::Tree
                };
            }
            Some(PreviewBackend::Streamed(streamed)) => {
                streamed.mode = if mode == PreviewMode::Tree {
                    PreviewMode::Streamed
                } else {
                    mode
                };
            }
            _ => {}
        }
    }

    /// 恢复 JSON/Streamed tab 的持久 mode。壳恢复已由 `push_shell_tab` 直接落到
    /// backend 上,这里保留给"已 Ready 的树"场景(暂无调用方)。
    #[allow(dead_code)]
    pub fn restore_json_mode(&mut self, tab_id: usize, mode: PreviewMode) {
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) else {
            return;
        };
        let Some(route) = tab.route.as_ref() else {
            return;
        };
        if !route.supports(mode) {
            return;
        }
        match tab.backend.as_mut() {
            Some(PreviewBackend::Json(json)) => {
                json.mode = match mode {
                    PreviewMode::Text => JsonMode::Text,
                    _ => JsonMode::Tree,
                };
            }
            Some(PreviewBackend::Streamed(streamed)) => streamed.mode = mode,
            _ => {}
        }
        if let Some(JsonTreeState::Ready(view)) = tab.json_tree.as_mut() {
            view.view_mode = match mode {
                PreviewMode::Text => crate::json_tree::ViewMode::RawText,
                _ => crate::json_tree::ViewMode::Tree,
            };
        }
    }

    /// 取走(清空)JSON 树的后台加载队列。语义与 `take_pending_tabular_loads`
    /// 完全对齐(见其文档):每次 `open_path` 后立即取走,不跨调用攒着。
    pub fn take_pending_json_tree_loads(&mut self) -> Vec<(usize, PathBuf)> {
        std::mem::take(&mut self.pending_json_tree_loads)
    }

    /// 编辑保存后调用:按 `PreviewTab.id` 找到对应 tab,推进 reload。原生
    /// (有 `editor`)tab 直接读盘重建编辑器实例(`bump_reload` 路径),wry
    /// tab 走 `reload_nonce` 计数(驱动 `desired_webviews()` 换 URL)。未知
    /// id 是 no-op(tab 可能已被关闭)。
    pub fn bump_reload(&mut self, tab_id: usize) {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) else {
            return;
        };
        if tab.editor.is_some() {
            // 原生 tab:只读态没有光标/undo 历史值得跨重建保留,直接读盘换新
            // 实例比"原地更新缓冲区"更简单可靠。读取失败保留旧 editor 不动
            // (比闪成空白/丢内容更安全的降级)。`Blank` tab 恒 `editor: None`
            // (见 `push_tab`),这个分支实际到不了,`else` 只是满足穷尽性。
            let TabKind::File(path) = &tab.kind else {
                return;
            };
            if let Ok(load) = read_and_build_native_editor(path) {
                tab.editor = Some(load.view);
                tab.loaded_bytes = load.loaded_bytes;
                tab.total_bytes = load.total_bytes;
                tab.truncated = load.truncated;
                // 读盘重建 = 重载/刷新:buffer 回到磁盘态,原先的就地改动(若有)
                // 一并丢弃,脏标记清零(可写后"刷新"会丢未保存改动——右键刷新前
                // 是否弹确认由调用方 handler 决定,清空这里是为了状态自洽)。
                tab.dirty = false;
            }
        } else if tab.tabular.is_none() {
            tab.reload_nonce += 1;
            if tab.uses_codemirror() {
                // 新 WebView 内部 revision 从 1 重新开始；清掉 Rust 镜像，
                // 让下一条 ready/selection 事件不会被旧 revision 拒绝。
                tab.web_revision = 0;
                tab.web_selection = None;
                tab.web_selected_text = None;
                tab.web_viewport = None;
            }
        }
    }

    /// 预览→代码:按下标读盘建一个可写 `CodeView` 挂到该 tab 上(只应对
    /// `wry_toggle_eligible` 的文件 tab 调用,按钮只在这类 tab 上画)。下标
    /// 越界或该 tab 不是 `TabKind::File` 是 no-op;读盘失败把 `io::Error`
    /// 透传给调用方(`Workspace::preview_pane_toggle_render_mode`)写面板
    /// error,这里不生成错误文案。
    pub fn enter_code_mode(&mut self, idx: usize) -> std::io::Result<()> {
        // feature 打开:Markdown/HTML 的 Source 模式由 CodeMirror editor host
        // 承载,不再构造 iced `CodeView`;只翻转 backend mode。
        if codemirror_enabled() {
            if let Some(tab) = self.tabs.get_mut(idx) {
                if let Some(PreviewBackend::Rendered(rendered)) = tab.backend.as_mut() {
                    rendered.mode = RenderedMode::Source;
                }
                let _ = tab.backend_state.try_transition(BackendState::Ready);
                tab.debug_assert_backend_consistent();
            }
            return Ok(());
        }
        let Some(tab) = self.tabs.get(idx) else {
            return Ok(());
        };
        let TabKind::File(path) = &tab.kind else {
            return Ok(());
        };
        let load = read_and_build_native_editor(path)?;
        if let Some(tab) = self.tabs.get_mut(idx) {
            tab.editor = Some(load.view);
            tab.loaded_bytes = load.loaded_bytes;
            tab.total_bytes = load.total_bytes;
            tab.truncated = load.truncated;
            tab.dirty = false;
            if let Some(PreviewBackend::Rendered(rendered)) = tab.backend.as_mut() {
                rendered.mode = RenderedMode::Source;
            }
            tab.debug_assert_backend_consistent();
        }
        Ok(())
    }

    /// 代码→预览:清空该 tab 的原生 editor,转回 wry/flyfish 渲染。调用方
    /// 负责在此之前先把脏改动落盘(`Workspace::preview_pane_toggle_render_mode`
    /// 里先 `preview_pane_save_at` 再调这个)——这里只做状态切换,不碰磁盘。
    /// 下标越界是 no-op。
    pub fn exit_code_mode(&mut self, idx: usize) {
        if let Some(tab) = self.tabs.get_mut(idx) {
            tab.editor = None;
            if let Some(PreviewBackend::Rendered(rendered)) = tab.backend.as_mut() {
                rendered.mode = RenderedMode::Rendered;
            }
            tab.debug_assert_backend_consistent();
        }
    }

    /// 外部文件系统变化后,按变更路径集跟进预览:只重载**走 wry 的 webview**
    /// 文件 tab(其路径命中任一 `changed`),推进 `reload_nonce` 让它 `load_url`
    /// 读到磁盘最新内容。原生 editor tab **不**动——自动重载会重建实例、丢
    /// 滚动/只读态,与"手动点 tab 不重载原生"同一品鉴口径(见 `select`)。
    /// 路径比对先做逐字节精确匹配,匹配不到再对 tab 路径 canonicalize 后比
    /// 一次(`notify` 递的是规范实路径,而 tab 路径可能来自未规范化的点击)。
    /// 空变更集 / 无命中 tab 都是 no-op。
    pub fn reload_webviews_for(&mut self, changed: &[PathBuf]) {
        if changed.is_empty() {
            return;
        }
        // CodeMirror tab:clean 自动重载(推进 `reload_nonce` 换 URL 逼 WebView
        // 重新拉取);**脏** tab 不自动重载,否则会覆盖用户未保存的改动——改为
        // 置冲突状态,由用户决定刷新/另存。
        for tab in self.tabs.iter_mut() {
            if !tab.uses_codemirror() {
                continue;
            }
            let TabKind::File(path) = &tab.kind else {
                continue;
            };
            let hit = changed.iter().any(|c| c == path)
                || std::fs::canonicalize(path)
                    .map(|p| changed.iter().any(|c| c == &p))
                    .unwrap_or(false);
            if !hit {
                continue;
            }
            if tab.dirty {
                tab.web_error = Some("文件已在外部修改,未自动重载以免覆盖你的改动".into());
            } else {
                tab.reload_nonce += 1;
                tab.web_revision = 0;
                tab.web_selection = None;
                tab.web_selected_text = None;
                tab.web_viewport = None;
                tab.web_error = None;
            }
        }
        let mut matched: Vec<usize> = self
            .tabs
            .iter()
            .filter(|t| t.hosts_webview())
            .filter_map(|t| match &t.kind {
                TabKind::File(path)
                    if changed.iter().any(|c| c == path)
                        || std::fs::canonicalize(path)
                            .map(|p| changed.iter().any(|c| c == &p))
                            .unwrap_or(false) =>
                {
                    Some(t.id)
                }
                _ => None,
            })
            .collect();
        matched.sort();
        matched.dedup();
        for id in matched {
            self.bump_reload(id);
        }
    }

    /// 配色方案切换后调用:把所有走 wry 的文件 tab 的 `reload_nonce` 各推一格,
    /// 逼 `desired_webviews()` 换 URL(新 URL 带新的 `&theme=` 参数)重新导航,
    /// flyfish 据此切到新主题。原生编辑器 / Tabular Viewer tab 不受影响(它们
    /// 是 iced 原生渲染、每帧读 `byteui::theme::color::current()`,切主题自然
    /// 跟随);`Blank` 占位 tab 没有 wry 页面,同样跳过。
    pub fn reload_all_webviews_for_theme(&mut self) {
        let ids: Vec<usize> = self
            .tabs
            .iter()
            .filter(|t| t.hosts_webview() || t.uses_codemirror())
            .map(|t| t.id)
            .collect();
        for id in ids {
            self.bump_reload(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    #[test]
    fn large_file_search_open_close_roundtrip() {
        let mut p = PreviewPane::default();
        let id = p.push_tab(
            TabKind::File(std::path::PathBuf::from("/tmp/x.log")),
            "x.log".into(),
        );
        p.open_large_file_search(id);
        assert!(p.large_file_search.as_ref().is_some_and(|s| s.tab_id == id));
        p.close_large_file_search();
        assert!(p.large_file_search.is_none());
    }

    #[test]
    fn large_file_search_results_fill_hits_and_reset_current() {
        let mut p = PreviewPane::default();
        let id = p.push_tab(
            TabKind::File(std::path::PathBuf::from("/tmp/x.log")),
            "x.log".into(),
        );
        p.open_large_file_search(id);
        let hits = vec![crate::extensions::search::SearchHit {
            path: "/tmp/x.log".into(),
            line_no: 42,
            line_text: "needle here".into(),
        }];
        p.set_large_file_search_results(id, Ok(hits.clone()));
        let s = p.large_file_search.as_ref().unwrap();
        assert_eq!(s.hits, hits);
        assert_eq!(s.current, 0);
        assert!(!s.running);
    }

    #[test]
    fn large_file_search_results_ignore_stale_tab() {
        // 结果回来前用户已经关了搜索条/切走了文件:tab_id 不匹配,忽略。
        let mut p = PreviewPane::default();
        let id = p.push_tab(
            TabKind::File(std::path::PathBuf::from("/tmp/x.log")),
            "x.log".into(),
        );
        p.open_large_file_search(id);
        p.close_large_file_search();
        p.set_large_file_search_results(id, Ok(vec![])); // 不应 panic,也不应重新打开条
        assert!(p.large_file_search.is_none());
    }

    #[cfg(not(feature = "codemirror"))]
    #[test]
    fn bump_reload_rebuilds_native_editor_without_bumping_nonce() {
        let path =
            std::env::temp_dir().join(format!("preview_reload_test_{}.rs", std::process::id()));
        std::fs::write(&path, "fn one() {}").unwrap();

        let mut p = PreviewPane::default();
        let id = p.open_path(path.clone());
        assert!(p.tabs()[1].editor.is_some());
        let nonce_before = p.tabs()[1].reload_nonce;

        std::fs::write(&path, "fn two() {}").unwrap();
        p.bump_reload(id);

        assert_eq!(
            p.tabs()[1].reload_nonce,
            nonce_before,
            "原生 tab 的 reload 不该走 reload_nonce 计数(那是 wry URL 换参专用信号)"
        );
        assert!(
            p.tabs()[1].editor.is_some(),
            "reload 后原生 tab 应仍持有(重建后的)editor"
        );
        assert_eq!(
            p.tabs()[1].editor.as_ref().unwrap().text(),
            "fn two() {}",
            "原生 tab reload 应读入磁盘上的新内容"
        );

        std::fs::remove_file(&path).ok();
    }

    #[cfg(not(feature = "codemirror"))]
    #[test]
    fn opening_native_editor_tab_sets_pending_focus() {
        // 官方 `text_editor` 的焦点是真实 iced 焦点树的一部分,构造时不能
        // 直接拿到,改成置一次性 `pending_editor_focus` 位,main.rs 下一帧
        // 用 `operation::focusable::focus` 强制聚焦(见该字段文档)。
        let path =
            std::env::temp_dir().join(format!("preview_focus_test_{}.rs", std::process::id()));
        std::fs::write(&path, "fn main() {}").unwrap();

        let mut p = PreviewPane::default();
        p.open_path(path.clone());
        assert!(
            p.take_pending_editor_focus(),
            "新建原生 tab 应置一次性聚焦位"
        );
        assert!(!p.take_pending_editor_focus(), "消费式:取走后应复位");

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn opening_webview_tab_does_not_set_pending_focus() {
        let mut p = PreviewPane::default();
        p.open_path(PathBuf::from("/tmp/no_focus_test.png"));
        assert!(p.tabs()[1].editor.is_none());
        assert!(
            !p.take_pending_editor_focus(),
            ".png 走 wry,没有原生 editor,不该置聚焦位"
        );
    }

    #[cfg(not(feature = "codemirror"))]
    #[test]
    fn active_tab_is_native_only_when_active_editor_holds_codeview() {
        let mut p = PreviewPane::default();
        assert!(!p.active_tab_is_native(), "空预览不是原生编辑 tab");

        let path =
            std::env::temp_dir().join(format!("preview_is_native_test_{}.rs", std::process::id()));
        std::fs::write(&path, "fn main() {}").unwrap();
        p.open_path(path.clone());
        assert!(
            p.active_tab_is_native(),
            "白名单源码 tab 应是原生可编辑预览"
        );
        let png = PathBuf::from("/tmp/isnative_native_off.png");
        p.open_path(png.clone());
        assert!(
            !p.active_tab_is_native(),
            "切到 wry 渲染 tab 后不再是原生可编辑预览"
        );
        std::fs::remove_file(&path).ok();
    }

    #[cfg(not(feature = "codemirror"))]
    #[test]
    fn open_path_builds_native_editor_for_whitelisted_extension_only() {
        let dir = std::env::temp_dir();
        let rs_path = dir.join(format!("preview_native_test_{}.rs", std::process::id()));
        let png_path = dir.join(format!("preview_native_test_{}.png", std::process::id()));
        std::fs::write(&rs_path, "fn main() {}").unwrap();
        std::fs::write(&png_path, [0u8; 4]).unwrap();

        let mut p = PreviewPane::default();
        p.open_path(rs_path.clone());
        p.open_path(png_path.clone());

        assert!(p.tabs()[1].editor.is_some(), ".rs 扩展名应构造原生 editor");
        assert!(
            p.tabs()[2].editor.is_none(),
            ".png 扩展名不应构造原生 editor,继续走 wry"
        );

        let specs = p.desired_webviews();
        assert_eq!(
            specs.len(),
            1,
            "原生 tab 不应出现在 wry 期望清单里,只剩 .png 那个"
        );
        assert_eq!(
            specs[0].url,
            format!(
                "dozer://flyfish/host.html?p={}&theme=dark",
                encode_component(&png_path.to_string_lossy())
            ),
            "剩下的唯一一条 wry 期望清单条目应该是 .png 那个,URL 编码规则同 flyfish_url"
        );

        std::fs::remove_file(&rs_path).ok();
        std::fs::remove_file(&png_path).ok();
    }

    #[cfg(not(feature = "codemirror"))]
    #[test]
    fn oversized_edit_tier_file_stays_native_but_becomes_read_only() {
        // 2026-09-19 起(大文件编辑器性能优化)不再有"超过阈值就退回 wry 只读
        // 预览"这回事——原生编辑器按内存动态分三档(`native_editor::SizeTier`),
        // 超过 `EDIT_MODE_MAX_BYTES`(20MB)的可编辑扩展名文件仍然构造原生
        // `CodeView`,只是从可写切换成只读(`is_read_only() == true`),不会
        // 出现在 `desired_webviews()` 期望清单里。
        let dir = std::env::temp_dir();
        let big_path = dir.join(format!(
            "preview_oversize_test_{}.json5",
            std::process::id()
        ));
        let small_path = dir.join(format!("preview_small_test_{}.json5", std::process::id()));
        let filler = "x".repeat(1024);
        let mut big = String::new();
        while big.len() <= crate::preview::EDIT_MODE_MAX_BYTES as usize {
            big.push_str(&filler);
            big.push('\n');
        }
        std::fs::write(&big_path, &big).unwrap();
        std::fs::write(&small_path, "[{ id: 1 }]").unwrap();

        let mut p = PreviewPane::default();
        p.open_path(big_path.clone());
        p.open_path(small_path.clone());

        let big_editor = p.tabs()[1]
            .editor
            .as_ref()
            .expect("超大文件仍构造原生 editor");
        assert!(big_editor.is_read_only(), "超过编辑档上限应变只读");
        let small_editor = p.tabs()[2]
            .editor
            .as_ref()
            .expect("小文件照常构造原生 editor");
        assert!(!small_editor.is_read_only(), "小文件仍可写");
        let specs = p.desired_webviews();
        assert_eq!(
            specs.iter().filter(|s| s.id == p.tabs()[1].id).count(),
            0,
            "只读大文件走原生编辑器渲染,不应出现在 wry 期望清单里"
        );

        std::fs::remove_file(&big_path).ok();
        std::fs::remove_file(&small_path).ok();
    }

    #[test]
    fn markdown_renders_via_webview_but_stays_editable() {
        // .md 是白名单扩展名(`is_editable_extension` 仍为 true,右键"编辑"
        // 照常出现),但默认预览要走 flyfish 的 markdown 渲染器而不是原生
        // 只读代码编辑器——跟 .png 这类天然不可编辑的类型走 wry 的原因不同,
        // 这里是"能编辑但默认展示渲染效果",两个判定必须独立验证。
        let dir = std::env::temp_dir();
        let md_path = dir.join(format!("preview_markdown_test_{}.md", std::process::id()));
        std::fs::write(&md_path, "# hello\n\nworld").unwrap();

        let mut p = PreviewPane::default();
        p.open_path(md_path.clone());

        assert!(
            p.tabs()[1].editor.is_none(),
            ".md 默认预览应走 flyfish 渲染,不建原生只读 editor"
        );
        assert!(
            is_editable_extension(&md_path),
            ".md 仍应保留可编辑属性,右键“编辑”入口不受影响"
        );
        let specs = p.desired_webviews();
        assert_eq!(specs.len(), 1, ".md 现在应进 wry 期望清单");
        assert!(
            specs[0].url.contains("&ln=1"),
            "&ln=1 仍按 is_editable_extension 挂上,flyfish 对非文本渲染器会忽略该 option"
        );

        std::fs::remove_file(&md_path).ok();
    }

    #[test]
    fn html_renders_via_webview_but_stays_editable() {
        // 跟 markdown_renders_via_webview_but_stays_editable 同一套断言,
        // 验证 html 现在也走"能编辑但默认展示渲染效果"这条路径。
        let dir = std::env::temp_dir();
        let html_path = dir.join(format!("preview_html_test_{}.html", std::process::id()));
        std::fs::write(&html_path, "<h1>hello</h1>").unwrap();

        let mut p = PreviewPane::default();
        p.open_path(html_path.clone());

        assert!(
            p.tabs()[1].editor.is_none(),
            ".html 默认预览应走 wry 渲染,不建原生只读 editor"
        );
        assert!(
            is_editable_extension(&html_path),
            ".html 仍应保留可编辑属性,右键“编辑”入口不受影响"
        );
        let specs = p.desired_webviews();
        assert_eq!(specs.len(), 1, ".html 现在应进 wry 期望清单");
        assert_eq!(
            specs[0].url,
            format!("file://{}", html_path.to_string_lossy()),
            ".html 应该直接加载 file:// URL,不经 flyfish(flyfish 只会把它当源码显示)"
        );

        std::fs::remove_file(&html_path).ok();
    }

    #[test]
    fn file_url_percent_encodes_each_path_segment_but_keeps_slashes() {
        assert_eq!(
            file_url(std::path::Path::new("/tmp/a b/c.html")),
            "file:///tmp/a%20b/c.html"
        );
    }

    #[test]
    fn preview_url_dispatches_html_to_file_url_and_others_to_flyfish() {
        assert_eq!(
            preview_url(std::path::Path::new("/tmp/page.html")),
            "file:///tmp/page.html"
        );
        assert_eq!(
            preview_url(std::path::Path::new("/tmp/page.HTM")),
            "file:///tmp/page.HTM",
            "扩展名判定大小写不敏感"
        );
        assert_eq!(
            preview_url(std::path::Path::new("/tmp/notes.md")),
            flyfish_url(std::path::Path::new("/tmp/notes.md")),
            "非 html/htm 扩展名不变,仍走 flyfish"
        );
    }

    #[test]
    fn code_class_extensions_route_to_native_text_editor_preview() {
        // 2026-09-05:js/json「等代码类」文件都应由 text editor 预览,而不是
        // 落进 flyfish 当不可预览的兜底。判据单一来源=语法能力(`extension_to_syntax`
        // 非 txt),凡高亮器认得出的源码扩展名都必须能进原生预览。这里挑几个
        // 旧白名单里没有、用户常碰的源码格式逐一断言。(旧列表只有 rs/toml/md/
        // txt/json/yaml/yml/sh/py/js/ts/tsx/jsx/html/css/xml/log/conf。）
        for ext in [
            "go", "java", "kt", "c", "h", "cpp", "cc", "rb", "php", "scss", "mjs", "cjs", "sql",
            "lua", "r", "swift", "zig", "proto", "graphql", "gql", "ex", "hs", "scala", "diff",
        ] {
            assert!(
                is_editable_extension(std::path::Path::new(&format!("/tmp/code.{ext}"))),
                ".{ext} 是有语法的源码扩展名,应走原生 text editor 预览"
            );
        }
    }

    #[test]
    fn image_pdf_and_binary_exts_stay_out_of_native_editor() {
        // 图片/PDF/富媒体/压缩包不属于"代码类",必须继续留给 flyfish(或其兜底),
        // 绝不能因语法判据脱节被误塞进原生文本编辑器。
        for ext in [
            "png", "jpg", "jpeg", "gif", "webp", "avif", "svg", "pdf", "zip", "mp4",
        ] {
            assert!(
                !is_editable_extension(std::path::Path::new(&format!("/tmp/a.{ext}"))),
                ".{ext} 是二进制/媒体,不该进原生文本编辑器"
            );
        }
    }

    #[test]
    fn markdown_and_html_still_editable_but_default_preview_is_rendered() {
        // is_editable_extension 变宽(语法判据)后,md/html 必须仍被
        // prefers_rendered_preview 拉去渲染预览而不是落到原生,避免回归。
        assert!(is_editable_extension(std::path::Path::new("/tmp/a.md")));
        assert!(is_editable_extension(std::path::Path::new("/tmp/a.html")));
        assert!(prefers_rendered_preview(std::path::Path::new("/tmp/a.md")));
        assert!(prefers_rendered_preview(std::path::Path::new(
            "/tmp/a.html"
        )));
    }

    #[test]
    fn bump_reload_uses_question_mark_separator_for_file_url_html() {
        let mut p = PreviewPane::default();
        let id0 = p.open_path(PathBuf::from("/tmp/a.html"));
        p.bump_reload(id0);
        assert_eq!(
            p.desired_webviews()[0].url,
            "file:///tmp/a.html?_r=1",
            "file:// URL 本身没有查询串,重载参数要用 ? 起头而不是 &"
        );
    }

    #[test]
    fn open_select_close_tabs() {
        let mut p = PreviewPane::default();
        // 第 0 项恒为 Blank 占位,文件 tab 从下标 1 起。
        assert_eq!(p.tabs()[0].kind, TabKind::Blank);
        let id0 = p.open_path(PathBuf::from("/tmp/a.md"));
        let id1 = p.open_path(PathBuf::from("/tmp/b.md"));
        assert_eq!(p.tabs().len(), 3);
        assert_eq!(p.active_idx(), 2, "新开 tab 即激活");
        assert_ne!(id0, id1);
        assert_eq!(p.tabs()[1].title, "a.md");
        assert_eq!(p.tabs()[2].title, "b.md");
        p.select(1);
        assert_eq!(p.active_idx(), 1);
        p.close(1);
        assert_eq!(p.tabs().len(), 2);
        assert_eq!(p.active_idx(), 1);
    }

    #[test]
    fn blank_placeholder_is_state_zero_and_always_present() {
        // `Default` 即停在空白占位页(没有任何可预览文件)。
        let p = PreviewPane::default();
        assert_eq!(p.tabs().len(), 1);
        assert_eq!(p.tabs()[0].kind, TabKind::Blank);
        assert_eq!(p.active_idx(), 0);
        // Blank tab 没有 wry 页面,不该进期望清单。
        assert!(p.desired_webviews().is_empty());
    }

    #[test]
    fn closing_last_file_tab_falls_back_to_the_blank_placeholder() {
        let mut p = PreviewPane::default();
        p.open_path(PathBuf::from("/tmp/only.md"));
        assert_eq!(p.tabs().len(), 2);
        // 关掉唯一文件 tab(下标 1,占位恒在 0)。
        p.close(1);
        assert_eq!(p.tabs().len(), 1, "关掉最后一个文件 tab 后只剩 Blank 占位");
        assert_eq!(p.tabs()[0].kind, TabKind::Blank);
        assert_eq!(p.active_idx(), 0, "落点回到空白占位页");
        assert!(p.desired_webviews().is_empty());
    }

    #[test]
    fn blank_placeholder_cannot_be_closed() {
        let mut p = PreviewPane::default();
        p.open_path(PathBuf::from("/tmp/only.md"));
        p.close(1);
        assert_eq!(p.tabs().len(), 1);
        // 点占位 tab 上的 × / 关它都是 no-op:它恒定存在、不可关闭(同
        // SSH/数据库面板的固定"空白"占位)。
        p.close(0);
        assert_eq!(p.tabs().len(), 1);
        assert_eq!(p.tabs()[0].kind, TabKind::Blank);
    }

    #[test]
    fn clear_all_keeps_exactly_one_blank_placeholder() {
        let mut p = PreviewPane::default();
        p.open_path(PathBuf::from("/tmp/a.md"));
        p.open_path(PathBuf::from("/tmp/b.md"));
        p.clear_all();
        assert_eq!(
            p.tabs().len(),
            1,
            "项目切换清理丢弃旧文件 tab,但保留恒定存在的 Blank 占位"
        );
        assert_eq!(p.tabs()[0].kind, TabKind::Blank);
        assert_eq!(p.active_idx(), 0);
    }

    #[test]
    fn reorder_never_moves_or_displaces_the_blank_placeholder() {
        let mut p = PreviewPane::default();
        p.open_path(PathBuf::from("/tmp/a.md"));
        p.open_path(PathBuf::from("/tmp/b.md"));
        // 把某个文件 tab 拖到占位之前:no-op(占位恒在首位)。
        p.reorder(2, 0);
        assert_eq!(p.tabs()[0].kind, TabKind::Blank);
        // 把占位自己拖走:no-op。
        p.reorder(0, 2);
        assert_eq!(p.tabs()[0].kind, TabKind::Blank);
        assert_eq!(p.tabs()[1].title, "a.md");
        assert_eq!(p.tabs()[2].title, "b.md");
    }

    #[test]
    fn open_tabular_file_starts_loading_and_queues_background_load() {
        let p = std::env::temp_dir().join(format!("tabular_route_{}.csv", std::process::id()));
        std::fs::write(&p, "a,b\n1,2\n").unwrap();
        let mut pane = PreviewPane::default();
        let id = pane.open_path(p.clone());
        let tab = &pane.tabs()[pane.active_idx()];
        // 打开这一刻还没跑后台线程,tab 先落在 Loading——真正解析是异步的
        // (见 `crate::tabular` 模块文档的"够数即停"性能策略)。
        assert!(
            matches!(tab.tabular, Some(TabularState::Loading)),
            "csv tab 应先进入 Loading 态,而不是同步构造好 TabularView"
        );
        assert!(tab.editor.is_none(), "csv 不应再进代码编辑器");
        assert!(
            pane.desired_webviews().is_empty(),
            "tabular tab(哪怕还在加载)不该进 webview 池"
        );
        assert!(
            pane.active_tab_is_native(),
            "tabular tab 应是原生 iced 渲染"
        );
        assert_eq!(
            pane.take_pending_tabular_loads(),
            vec![(id, p.clone())],
            "push_tab 应把这个新 tab 登记进待加载队列,供调用方 spawn 后台加载"
        );
        // 队列取过一次即清空,不会被后续调用重复消费。
        assert!(pane.take_pending_tabular_loads().is_empty());
        std::fs::remove_file(p).ok();
    }

    #[test]
    fn reopening_already_open_tabular_file_does_not_requeue_load() {
        let p = std::env::temp_dir().join(format!("tabular_reopen_{}.csv", std::process::id()));
        std::fs::write(&p, "a,b\n1,2\n").unwrap();
        let mut pane = PreviewPane::default();
        let id0 = pane.open_path(p.clone());
        pane.take_pending_tabular_loads();
        // 同一路径再开一次:走"复用已有 tab"分支,不应该对着还在 Loading
        // 的 tab 再触发一次后台加载(否则两次加载结果谁后到谁覆盖就成了
        // 竞态)。
        let id1 = pane.open_path(p.clone());
        assert_eq!(id0, id1);
        assert!(pane.take_pending_tabular_loads().is_empty());
        std::fs::remove_file(p).ok();
    }

    #[test]
    fn tabular_state_transitions_from_loading_to_ready_via_load_result() {
        let p = std::env::temp_dir().join(format!("tabular_ready_{}.csv", std::process::id()));
        std::fs::write(&p, "a,b\n1,2\n").unwrap();
        let mut pane = PreviewPane::default();
        let id = pane.open_path(p.clone());
        assert!(pane.tabular_mut(id).is_none(), "加载完成前 apply 应 no-op");
        assert!(matches!(
            pane.tabs()[pane.active_idx()].backend_state,
            BackendState::Loading
        ));
        let loaded = crate::tabular::load(&p).expect("测试用 csv 应能正常解析");
        pane.finish_tabular_load(id, Ok(loaded));
        assert!(
            pane.tabular_mut(id).is_some(),
            "Ready 之后 tabular_mut 应能拿到可变引用"
        );
        assert!(matches!(
            pane.tabs()[pane.active_idx()].backend_state,
            BackendState::Ready
        ));
        std::fs::remove_file(p).ok();
    }

    #[test]
    fn desired_webviews_builds_urls_and_visibility() {
        let mut p = PreviewPane::default();
        p.open_path(PathBuf::from("/tmp/a b.md"));
        p.open_path(PathBuf::from("/tmp/c.md"));
        let specs = p.desired_webviews();
        assert_eq!(specs.len(), 2);
        assert_eq!(
            specs[0].url,
            "dozer://flyfish/host.html?p=%2Ftmp%2Fa%20b.md&theme=dark&ln=1"
        );
        assert!(!specs[0].visible, "非激活 tab 不可见");
        assert_eq!(
            specs[1].url,
            "dozer://flyfish/host.html?p=%2Ftmp%2Fc.md&theme=dark&ln=1"
        );
        assert!(specs[1].visible);
    }

    #[test]
    fn bump_reload_appends_query_param_and_only_affects_target_tab() {
        let mut p = PreviewPane::default();
        let id0 = p.open_path(PathBuf::from("/tmp/a.md"));
        let _id1 = p.open_path(PathBuf::from("/tmp/b.md"));
        p.bump_reload(id0);
        let specs = p.desired_webviews();
        assert_eq!(
            specs[0].url,
            "dozer://flyfish/host.html?p=%2Ftmp%2Fa.md&theme=dark&ln=1&_r=1"
        );
        assert_eq!(
            specs[1].url,
            "dozer://flyfish/host.html?p=%2Ftmp%2Fb.md&theme=dark&ln=1"
        );
        p.bump_reload(id0);
        assert_eq!(
            p.desired_webviews()[0].url,
            "dozer://flyfish/host.html?p=%2Ftmp%2Fa.md&theme=dark&ln=1&_r=2"
        );
        // 未知 id 是 no-op,不 panic。
        p.bump_reload(9999);
    }

    // feature 开启时 `.rs` tab 走 CodeMirror,外部变化会推进其 reload_nonce
    // (见 `external_change_reloads_clean_...`),这条 iced-原生语义的断言不再成立。
    #[cfg(not(feature = "codemirror"))]
    #[test]
    fn reload_webviews_for_hits_matching_webview_tabs_only() {
        let mut p = PreviewPane::default();
        p.open_path(PathBuf::from("/tmp/a.md")); // webview
        p.open_path(PathBuf::from("/tmp/b.md")); // webview
        // 原生 editor tab(真实 temp .rs 文件)。
        let rs_path =
            std::env::temp_dir().join(format!("preview_webviews_for_{}.rs", std::process::id()));
        std::fs::write(&rs_path, "fn main() {}").unwrap();
        let _ = p.open_path(rs_path.clone());

        // 空变更集:no-op,一个都不推进。下标 0 是 Blank 占位,文件 tab 从 1 起。
        p.reload_webviews_for(&[]);
        assert_eq!(p.tabs()[1].reload_nonce, 0);
        assert_eq!(p.tabs()[2].reload_nonce, 0);
        assert_eq!(p.tabs()[3].reload_nonce, 0);

        // 命中 b.md:只推进 b 的 reload_nonce。
        p.reload_webviews_for(&[PathBuf::from("/tmp/b.md")]);
        assert_eq!(p.tabs()[1].reload_nonce, 0, "a.md 不受影响");
        assert_eq!(p.tabs()[2].reload_nonce, 1, "b.md 命中,webview 推进");
        let specs = p.desired_webviews();
        assert_eq!(
            specs[1].url, "dozer://flyfish/host.html?p=%2Ftmp%2Fb.md&theme=dark&ln=1&_r=1",
            "命中的 webview 换 URL 重载"
        );

        // 命中原生 tab 的路径:不推进(原生 editor 不自动重载)。
        p.reload_webviews_for(std::slice::from_ref(&rs_path));
        assert_eq!(
            p.tabs()[3].reload_nonce,
            0,
            "原生 editor tab 命中也不自动重载,保住滚动/只读态"
        );

        // 未命中的路径:no-op。
        p.reload_webviews_for(&[PathBuf::from("/tmp/other.md")]);
        assert_eq!(p.tabs()[2].reload_nonce, 1, "未命中不改状态");

        std::fs::remove_file(&rs_path).ok();
    }

    /// 主题参数跟随全局配色方案:`set_scheme(Light)` 后 URL 带 `&theme=light`。
    /// 配色方案是进程级共享 static,测完复原成 Dark,避免污染其它测试。
    #[test]
    fn flyfish_url_carries_light_theme_when_scheme_is_light() {
        use byteui::theme::color::{ColorScheme, set_scheme};
        let restore = byteui::theme::color::current_scheme();
        set_scheme(ColorScheme::Light);
        let url = flyfish_url(Path::new("/tmp/notes.md"));
        set_scheme(restore);
        assert!(
            url.contains("&theme=light"),
            "浅色方案下 URL 应带 &theme=light,实际: {url}"
        );
    }

    /// 切主题后 `reload_all_webviews_for_theme` 推进所有 wry 文件 tab 的
    /// nonce(逼它们按新 theme 重新导航),但不碰原生 editor tab。
    #[cfg(not(feature = "codemirror"))]
    #[test]
    fn reload_all_webviews_for_theme_bumps_only_wry_file_tabs() {
        let mut p = PreviewPane::default();
        p.open_path(PathBuf::from("/tmp/a.md")); // webview
        p.open_path(PathBuf::from("/tmp/b.md")); // webview
        let rs_path =
            std::env::temp_dir().join(format!("preview_theme_reload_{}.rs", std::process::id()));
        std::fs::write(&rs_path, "fn main() {}").unwrap();
        let _ = p.open_path(rs_path.clone()); // 原生 editor

        p.reload_all_webviews_for_theme();

        assert_eq!(p.tabs()[1].reload_nonce, 1, "a.md(webview)应被推进");
        assert_eq!(p.tabs()[2].reload_nonce, 1, "b.md(webview)应被推进");
        assert_eq!(
            p.tabs()[3].reload_nonce,
            0,
            "原生 editor tab 不该被主题切换推进(其配色由 iced 主题驱动)"
        );
        assert_eq!(
            p.tabs()[0].reload_nonce,
            0,
            "Blank 占位 tab 没有 wry 页面,不该被推进"
        );

        std::fs::remove_file(&rs_path).ok();
    }

    #[cfg(not(feature = "codemirror"))]
    #[test]
    fn select_reloads_webview_tab_on_switch_but_not_same_or_native() {
        let mut p = PreviewPane::default();
        let id0 = p.open_path(PathBuf::from("/tmp/a.md")); // webview(.md 走渲染预览)
        let id1 = p.open_path(PathBuf::from("/tmp/b.md")); // webview
        // 下标 0 是 Blank 占位,a.md=1、b.md=2。
        assert_eq!(p.active_idx(), 2);

        // 切到另一个 webview tab:推进 reload_nonce,切回时换 URL 重载。
        p.select(1);
        assert_eq!(
            p.desired_webviews()[0].url,
            "dozer://flyfish/host.html?p=%2Ftmp%2Fa.md&theme=dark&ln=1&_r=1",
            "切到异 tab 的 webview 要自动推进 reload_nonce"
        );
        assert_eq!(
            p.desired_webviews()[1].url,
            "dozer://flyfish/host.html?p=%2Ftmp%2Fb.md&theme=dark&ln=1",
            "非目标 tab 不受影响"
        );

        // 点当前已激活的 tab:no-op,不再多推进一次。
        p.select(1);
        assert_eq!(
            p.desired_webviews()[0].url,
            "dozer://flyfish/host.html?p=%2Ftmp%2Fa.md&theme=dark&ln=1&_r=1",
            "重复选同一 tab 不改 reload_nonce"
        );

        // 原生 editor tab 切换不重载:重开一个走原生路径的文件(whitelisted
        // 非渲染扩展,读盘建 editor),其 reload_nonce 保持 0。
        let rs_path =
            std::env::temp_dir().join(format!("preview_select_test_{}.rs", std::process::id()));
        std::fs::write(&rs_path, "fn main() {}").unwrap();
        let _id_rs = p.open_path(rs_path.clone());
        assert_eq!(p.active_idx(), 3);
        assert!(p.tabs()[3].editor.is_some(), "c.rs 应是原生 editor tab");
        p.select(2); // 切回 b.md(webview)
        assert_eq!(
            p.desired_webviews()[0].url,
            "dozer://flyfish/host.html?p=%2Ftmp%2Fa.md&theme=dark&ln=1&_r=1",
            "再切回 webview 推进一次 reload"
        );
        p.select(3); // 切回 c.rs(原生)
        assert_eq!(
            p.tabs()[3].reload_nonce,
            0,
            "原生 editor tab 切回不自动重载,保住滚动/只读态"
        );
        std::fs::remove_file(&rs_path).ok();

        // 越界/原生切片语义:切到越界下标是 no-op。
        let before = p.active_idx();
        p.select(99);
        assert_eq!(p.active_idx(), before);

        // id0/id1 仍在,避免未使用告警。
        let _ = (id0, id1);
    }

    #[test]
    fn is_editable_extension_covers_common_text_types() {
        assert!(is_editable_extension(Path::new("main.rs")));
        assert!(is_editable_extension(Path::new("Cargo.toml")));
        assert!(is_editable_extension(Path::new("README.md")));
        assert!(
            is_editable_extension(Path::new("notes.TXT")),
            "大小写不敏感"
        );
        assert!(is_editable_extension(Path::new("package.json")));
        assert!(is_editable_extension(Path::new("ci.yaml")));
        assert!(is_editable_extension(Path::new("ci.yml")));
        assert!(is_editable_extension(Path::new("run.sh")));
        assert!(is_editable_extension(Path::new("app.py")));
        assert!(is_editable_extension(Path::new("index.js")));
        assert!(is_editable_extension(Path::new("index.ts")));
        assert!(is_editable_extension(Path::new("index.tsx")));
        assert!(is_editable_extension(Path::new("index.jsx")));
        assert!(is_editable_extension(Path::new("page.html")));
        assert!(is_editable_extension(Path::new("style.css")));
        assert!(is_editable_extension(Path::new("data.xml")));
        assert!(is_editable_extension(Path::new("out.log")));
        assert!(is_editable_extension(Path::new("nginx.conf")));
        assert!(
            is_editable_extension(Path::new(".gitignore")),
            "点开头的无扩展名文件要特判"
        );
    }

    #[test]
    fn is_editable_extension_rejects_unknown_and_binary_like() {
        assert!(!is_editable_extension(Path::new("logo.png")));
        assert!(!is_editable_extension(Path::new("archive.zip")));
        assert!(
            !is_editable_extension(Path::new("LICENSE")),
            "无扩展名不在白名单里"
        );
        assert!(!is_editable_extension(Path::new("Makefile")));
    }

    #[test]
    fn wry_toggle_eligible_true_for_rendered_preview_text_types() {
        assert!(wry_toggle_eligible(Path::new("README.md")));
        assert!(wry_toggle_eligible(Path::new("page.html")));
    }

    #[test]
    fn wry_toggle_eligible_false_for_always_native_text_types() {
        assert!(!wry_toggle_eligible(Path::new("main.rs")));
        assert!(!wry_toggle_eligible(Path::new("script.py")));
        assert!(!wry_toggle_eligible(Path::new("data.json")));
    }

    #[test]
    fn wry_toggle_eligible_false_for_binary_types() {
        assert!(!wry_toggle_eligible(Path::new("photo.png")));
        assert!(!wry_toggle_eligible(Path::new("doc.pdf")));
        assert!(!wry_toggle_eligible(Path::new("archive.zip")));
    }

    #[test]
    fn reopening_same_file_reuses_tab() {
        let mut p = PreviewPane::default();
        let id0 = p.open_path(PathBuf::from("/tmp/a.md")); // 下标 1(0 是占位)
        p.open_path(PathBuf::from("/tmp/b.md")); // 中间插一个,把激活挪走
        assert_eq!(p.active_idx(), 2);
        let id_again = p.open_path(PathBuf::from("/tmp/a.md"));
        assert_eq!(id_again, id0, "同文件复用同一 tab");
        assert_eq!(p.tabs().len(), 3, "不新增 tab(含恒定占位)");
        assert_eq!(p.active_idx(), 1, "切回已开的那个 tab");
    }

    #[test]
    fn rendered_source_toggle_keeps_backend_and_webview_state_in_sync() {
        let path =
            std::env::temp_dir().join(format!("preview_backend_mode_{}.md", std::process::id()));
        std::fs::write(&path, "# title\n").unwrap();
        let mut pane = PreviewPane::default();
        pane.open_path(path.clone());

        assert_eq!(pane.tabs()[1].current_mode(), Some(PreviewMode::Rendered));
        assert!(pane.tabs()[1].hosts_webview());
        pane.enter_code_mode(1).unwrap();
        assert_eq!(pane.tabs()[1].current_mode(), Some(PreviewMode::Source));
        assert!(!pane.tabs()[1].hosts_webview());
        pane.exit_code_mode(1);
        assert_eq!(pane.tabs()[1].current_mode(), Some(PreviewMode::Rendered));
        assert!(pane.tabs()[1].hosts_webview());

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn encode_component_is_rfc3986_strict() {
        assert_eq!(encode_component("aZ09-._~"), "aZ09-._~");
        assert_eq!(encode_component("/a b"), "%2Fa%20b");
        assert_eq!(encode_component("你"), "%E4%BD%A0");
    }

    #[cfg(not(feature = "codemirror"))]
    #[test]
    fn dirty_marker_lifecycle_for_native_tab() {
        // 原生可写 tab 就地编辑:编辑事件标脏 → ⌘S 落盘清脏(mark/clear 按 id)。
        let path =
            std::env::temp_dir().join(format!("dirty_marker_test_{}.rs", std::process::id()));
        std::fs::write(&path, "fn main() {}").unwrap();

        let mut p = PreviewPane::default();
        let id = p.open_path(path.clone());
        assert!(p.tabs()[1].editor.is_some(), "夹具应落在原生 editor 分支");
        assert!(!p.tabs()[1].dirty, "新开原生 tab 默认不脏");

        p.mark_dirty_by_id(id);
        assert!(p.tabs()[1].dirty, "收到编辑 Action 后应标脏");

        // 对 webview 形态 / 不存在 id 标脏——都该 no-op。
        let empty_id = p.next_id + 99;
        p.mark_dirty_by_id(empty_id);
        assert!(
            p.tabs()[1].dirty && p.tabs().len() == 2,
            "未知 id 标脏是 no-op,不应误标/误建"
        );

        p.clear_dirty_by_id(id);
        assert!(!p.tabs()[1].dirty, "落盘成功后应清脏");

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn mark_dirty_only_applies_to_native_tabs() {
        // .png 走 wry(editor.is_none());对 id 标脏应被 mark_dirty_by_id 拒掉。
        let mut p = PreviewPane::default();
        let id = p.open_path(PathBuf::from("/tmp/no_dirty_mark.png"));
        assert!(p.tabs()[1].editor.is_none());
        p.mark_dirty_by_id(id);
        assert!(!p.tabs()[1].dirty, "非原生 tab 不该被标脏");
    }

    #[cfg(not(feature = "codemirror"))]
    #[test]
    fn bump_reload_discards_pending_dirty_for_native_tab() {
        // 右键"刷新"重建原生 editor 会丢弃未保存改动 → 脏标记一并清零(保存
        // 态自洽:内存 buffer 都被换掉了,不能再以"有脏"混淆后续 ⌘S)。
        let path =
            std::env::temp_dir().join(format!("dirty_reload_test_{}.rs", std::process::id()));
        std::fs::write(&path, "fn one() {}").unwrap();

        let mut p = PreviewPane::default();
        let id = p.open_path(path.clone());
        p.mark_dirty_by_id(id);
        assert!(p.tabs()[1].dirty);

        std::fs::write(&path, "fn two() {}").unwrap();
        p.bump_reload(id);

        assert!(
            !p.tabs()[1].dirty,
            "原生 tab 刷新重建 editor 后,内存里的旧改动已丢弃,脏标记应复位"
        );

        std::fs::remove_file(&path).ok();
    }

    /// 防漂移锚:编辑器语法高亮的 token 颜色必须锚定到终端 16 色面板与终端
    /// 默认前景,而不是一套独立的十六进制魔数。字符串=Green、关键字=Cyan、
    /// 注释=BrightBlack、类型=Blue、函数=BrightBlue、默认前/后景=终端本色。
    fn syntax_token(theme: &syntect::highlighting::Theme, scope: &str) -> Option<(u8, u8, u8)> {
        use std::str::FromStr;
        let sel =
            syntect::highlighting::ScopeSelectors::from_str(scope).expect("测试 scope 必须合法");
        theme
            .scopes
            .iter()
            .find(|item| item.scope == sel)
            .and_then(|item| item.style.foreground)
            .map(|c| (c.r, c.g, c.b))
    }

    #[test]
    fn dozer_syntax_theme_anchored_to_terminal_palette() {
        // 显式指定 Dark:主题按方案预计算,断言就不受其它测试线程改全局
        // scheme 的影响(term_model 的用例会来回 set_scheme)。
        use byteui::theme::color::ColorScheme;
        let t = dozer_syntax_theme(ColorScheme::Dark);
        assert_eq!(
            syntax_token(&t, "string").expect("未命中 string"),
            crate::term::term_model::ansi16_color_for(ColorScheme::Dark, 2).unwrap(),
            "字符串应锚定终端 Green"
        );
        assert_eq!(
            syntax_token(&t, "keyword").expect("未命中 keyword"),
            crate::term::term_model::ansi16_color_for(ColorScheme::Dark, 6).unwrap(),
            "关键字应锚定终端 Cyan"
        );
        assert_eq!(
            syntax_token(&t, "comment").expect("未命中 comment"),
            crate::term::term_model::ansi16_color_for(ColorScheme::Dark, 8).unwrap(),
            "注释应锚定终端 BrightBlack"
        );
        assert_eq!(
            syntax_token(&t, "entity.name.type").expect("未命中类型"),
            crate::term::term_model::ansi16_color_for(ColorScheme::Dark, 4).unwrap(),
            "类型应锚定终端 Blue"
        );
        assert_eq!(
            syntax_token(&t, "entity.name.function").expect("未命中函数"),
            crate::term::term_model::ansi16_color_for(ColorScheme::Dark, 12).unwrap(),
            "函数应锚定终端 BrightBlue"
        );
        assert_eq!(
            syntax_token(&t, "operator").expect("未命中 operator"),
            crate::term::term_model::default_fg_rgb_for(ColorScheme::Dark),
            "运算符应锚定终端默认前景"
        );
        assert!(
            syntax_token(&t, "string.regexp")
                .map(|(_, g, _)| g)
                .expect("未命中 regexp")
                != 0xd9,
            "regexp 不应使用甲方金 #F2D94E"
        );
    }

    /// 浅色主题必须带来一套**不同**的 token 色,而不是沿用深色那套
    /// (否则「随主题自动切换」名存实亡)。同时两侧都仍锚定到各自方案的终端色板。
    #[test]
    fn dozer_syntax_theme_light_scheme_differs_and_stays_anchored() {
        use byteui::theme::color::ColorScheme;

        let dark = dozer_syntax_theme(ColorScheme::Dark);
        let light = dozer_syntax_theme(ColorScheme::Light);

        assert_ne!(
            syntax_token(&dark, "keyword"),
            syntax_token(&light, "keyword"),
            "关键字颜色必须随深/浅方案变化"
        );
        assert_eq!(
            syntax_token(&light, "string").expect("未命中 string"),
            crate::term::term_model::ansi16_color_for(ColorScheme::Light, 2).unwrap(),
            "浅色下字符串仍须锚定浅色终端 Green"
        );
    }

    /// 语法主题**不持有背景**:编辑器背景已透明(见 `code_editor::editor_style`),
    /// 面板底色透上来。主题再塞一个背景色只会是一份进不了渲染、还容易与面板
    /// 实际底色打架的死数据。
    #[test]
    fn dozer_syntax_theme_has_no_background() {
        use byteui::theme::color::ColorScheme;
        for scheme in [ColorScheme::Dark, ColorScheme::Light] {
            assert!(
                dozer_syntax_theme(scheme).settings.background.is_none(),
                "语法主题不该自带背景色({scheme:?})"
            );
        }
    }

    #[cfg(not(feature = "codemirror"))]
    #[test]
    fn open_find_binds_to_active_tab_native_or_webview() {
        let tmp = |name: &str| {
            let p = std::env::temp_dir().join(format!("{name}_{}.rs", std::process::id()));
            std::fs::write(&p, "fn x() {}").unwrap();
            p
        };
        // 清理(按名字逐个删;即使中途断言 panic 也留到末尾尽量收拾)。
        let mut created: Vec<PathBuf> = Vec::new();
        let a = tmp("find_a");
        let b = tmp("find_b");
        created.extend([a.clone(), b.clone()]);

        let mut p = PreviewPane::default();
        // 非可编辑文件(flyfish webview 档,如 .xyz)的 tab 上 ⌘F 仍开条,但
        // 走 webview 搜索(flyfish 自带 API),`is_webview=true` 且锁到该
        // webview tab——不碰原生 `CodeView::find_matches_all` 那一套。
        let web = std::env::temp_dir().join(format!("find_web_{}.xyz", std::process::id()));
        std::fs::write(&web, "no editor").unwrap();
        created.push(web.clone());
        p.open_path(web.clone());
        assert!(
            p.tabs()[p.active_idx()].editor.is_none(),
            "非原生扩展(.xyz)不该有 editor"
        );
        p.open_find_on_active(false, None);
        assert!(
            p.find_bar_open(),
            "webview 档 ⌘F 应开 find 条(走 flyfish 搜索)"
        );
        let fweb = p.find_state().expect("webview 档应锁到 find 会话");
        assert!(fweb.is_webview, "webview 档 find 应为 is_webview");
        assert_eq!(
            fweb.tab_id,
            p.tabs()[p.active_idx()].id,
            "应锁到 webview tab"
        );

        // 打开原生 A、B:B 为激活,⌘F 锁到 B。
        p.open_path(a.clone());
        p.open_path(b.clone());
        let id_b = p.tabs()[p.active_idx()].id;
        p.open_find_on_active(false, None);
        assert!(p.find_bar_open());
        assert_eq!(p.find_state().map(|f| f.tab_id), Some(id_b));

        // 切回 A(复用已开的 tab,直接切激活)→ 命中别份文件,cull。
        p.open_path(a.clone());
        assert!(
            !p.find_bar_open(),
            "切到正在搜索文件之外的 tab 后,Find 应被清扫"
        );

        // select 在 A、B 间切换同样触发 cull。
        p.open_find_on_active(false, None); // 激活是 A,锁 A
        let id_a = p.tabs()[p.active_idx()].id;
        let idx_b = p.tabs().iter().position(|t| t.id == id_b).unwrap();
        p.select(idx_b);
        assert!(!p.find_bar_open(), "select 切到别的文件后 Find 应被清扫");
        let _ = id_a;

        for f in created {
            std::fs::remove_file(f).ok();
        }
    }

    #[cfg(not(feature = "codemirror"))]
    #[test]
    fn open_find_same_tab_keeps_query_and_cursor_on_nav() {
        let path = std::env::temp_dir().join(format!("find_nav_{}.rs", std::process::id()));
        std::fs::write(&path, "hello world").unwrap();

        let mut p = PreviewPane::default();
        p.open_path(path.clone());
        p.open_find_on_active(false, None);

        // 输入 query:当场清点 count 并把首个命中**整段选中**("lo" 在 "hello" 起于
        // 首行 col3,含 2 个字节,结束时 col5,光标落末缘——正文里能看见词被框住)。
        p.find_type("lo".into());
        let s = p.find_state().unwrap();
        assert_eq!(s.query, "lo");
        assert_eq!(s.count, 1, "输入后应立即把命中数回填为 1");
        assert_eq!(s.current, 0);
        let e = p.tabs()[p.active_idx()].editor.as_ref().unwrap();
        assert_eq!(
            e.selection_range(),
            Some(((0, 3), (0, 5))),
            "find_type 应自动选中首个命中整段"
        );
        assert_eq!(e.cursor_position(), (0, 5), "选中后光标停在命中末缘");
        assert!(e.has_selection());

        // 已锁定同一 tab 再 ⌘F 是 no-op——query/count 保留(供 main 重聚焦);
        // 这时 text 只命中 1 次,find_go(prev) 也仍停在 col3(循环不自增越界)。
        p.open_find_on_active(false, None);
        assert_eq!(p.find_state().unwrap().query, "lo");
        assert_eq!(p.find_state().unwrap().count, 1);
        p.find_go(false);
        assert_eq!(p.find_state().unwrap().current, 0);

        // find_go(next) 单命中循环:0 → 0。caret 落在命中**末尾列**(选区高亮
        // "lo",position 在其末列;锚点在起点)。
        p.find_go(true);
        assert_eq!(p.find_state().unwrap().current, 0);
        assert_eq!(
            p.tabs()[p.active_idx()]
                .editor
                .as_ref()
                .unwrap()
                .cursor_position(),
            (0, 5)
        );

        // close_find → 条消失;再 open 得到全新空 query 会话。
        p.close_find();
        assert!(!p.find_bar_open());
        assert!(p.find_state().is_none());

        p.open_find_on_active(false, None);
        assert!(p.find_bar_open());
        let s = p.find_state().unwrap();
        assert!(s.query.is_empty());
        assert_eq!(s.count, 0);
        assert_eq!(s.current, 0);

        std::fs::remove_file(&path).ok();
    }

    #[cfg(not(feature = "codemirror"))]
    #[test]
    fn find_go_wraps_across_multiple_matches() {
        let path = std::env::temp_dir().join(format!("find_wrap_{}.rs", std::process::id()));
        std::fs::write(&path, "ab\ncd\nab\nab\nef").unwrap();
        let mut p = PreviewPane::default();
        p.open_path(path.clone());
        p.open_find_on_active(false, None);

        // "ab" 命中 3 次:行0 col0 / 行2 col0 / 行3 col0。
        p.find_type("ab".into());
        let s = p.find_state().unwrap();
        assert_eq!(p.find_state().unwrap().count, 3);
        assert_eq!(p.find_state().unwrap().current, 0);
        let _ = s;
        // find_type 已把首个命中整段选中,caret 落命中**末列**(col0 + query 长2)。
        let e0 = p.tabs()[p.active_idx()].editor.as_ref().unwrap();
        assert_eq!(
            e0.selection_range(),
            Some(((0, 0), (0, 2))),
            "find_type 应自动框住第一个命中"
        );
        assert_eq!(e0.cursor_position(), (0, 2));

        // 正向:用户连按「下一个」,每次把光标停在命中**末缘**(col2)单调续接。
        // find_type 之后 caret 已落在命中0 末缘(0,2);cast 归属在 [s,e) 左闭右开下
        // caret 贴末缘不算段内,走“start≤caret”落在命中0(anchor0)→ 步进到命中1;
        // caret(2,2)同理 → 命中2 → wrap 回命中0。
        p.find_go(true); // 命中0 → 1
        assert_eq!(p.find_state().unwrap().current, 1);
        let cursor = |p: &PreviewPane| {
            p.tabs()[p.active_idx()]
                .editor
                .as_ref()
                .unwrap()
                .cursor_position()
        };
        assert_eq!(
            cursor(&p),
            (2, 2),
            "current=1 应选中第 2 处命中(行2),curs@末缘"
        );
        assert_eq!(p.find_state().unwrap().current, 1);

        p.find_go(true); // 1 → 2
        assert_eq!(p.find_state().unwrap().current, 2);
        assert_eq!(cursor(&p), (3, 2), "current=2 应选中行3 命中,curs@末缘");

        p.find_go(true); // 2 → 0(wrap)
        assert_eq!(p.find_state().unwrap().current, 0);
        assert_eq!(
            cursor(&p),
            (0, 2),
            "正向越过最后命中应 wrap 回第 0 命中的末缘"
        );

        // 从此处反向「上一个」:不再逐字回退,而是以光标锚换边——反向跳把光标停
        // 到命中**前缘**(col0),连按「上一个」单调往回走(wrap:命中0 的前缘出发反
        // 向一步回到最后命中 2)。
        p.find_go(false); // 0 → 2(反向 wrap)
        assert_eq!(p.find_state().unwrap().current, 2);
        assert_eq!(cursor(&p), (3, 0), "反向跳出 0 后落在行3 命中前缘");

        p.find_go(false); // 2 → 1
        assert_eq!(p.find_state().unwrap().current, 1);
        assert_eq!(cursor(&p), (2, 0), "再「上一个」回到行2 命中前缘");

        p.find_go(false); // 1 → 0
        assert_eq!(p.find_state().unwrap().current, 0);
        assert_eq!(cursor(&p), (0, 0));

        // 无命中 query:保持 count0、current 无意义但不 panic。
        p.find_type("zz".into());
        assert_eq!(p.find_state().unwrap().count, 0);
        p.find_go(true);
        assert_eq!(p.find_state().unwrap().count, 0);

        std::fs::remove_file(&path).ok();
    }

    #[cfg(not(feature = "codemirror"))]
    #[test]
    fn clear_all_and_close_native_drop_find() {
        let tmp = |name: &str, content: &str| {
            let p = std::env::temp_dir().join(format!("{name}_{}.rs", std::process::id()));
            std::fs::write(&p, content).unwrap();
            p
        };
        let mut created = Vec::new();
        let p_a = tmp("cull_a", "a");
        let p_b = tmp("cull_b", "b");
        created.extend([p_a.clone(), p_b.clone()]);

        let mut p = PreviewPane::default();
        let id = p.open_path(p_a.clone());
        p.open_find_on_active(false, None);
        assert!(p.find_bar_open());

        // 关掉正搜索的文件会清空 vec → 自动补 Blank(push_tab 里的 cull)。
        p.close(id);
        assert!(!p.find_bar_open());
        assert!(
            p.tabs()[p.active_idx()].editor.is_none(),
            "关到空后应回 Blank 占位"
        );

        // clear_all(项目切换路径)同样吐掉 find。
        p.open_path(p_b.clone());
        p.open_find_on_active(false, None);
        assert!(p.find_bar_open());
        p.clear_all();
        assert!(p.find_state().is_none(), "clear_all 后 Find 应一并丢弃");

        for f in created {
            std::fs::remove_file(f).ok();
        }
    }

    #[cfg(not(feature = "codemirror"))]
    #[test]
    fn replace_all_rewrites_buffer_marks_dirty_and_refreshes_count() {
        let tmp = std::env::temp_dir().join(format!("pane_replace_all_{}.rs", std::process::id()));
        std::fs::write(&tmp, "needle 1\nplain\nneedle 2").unwrap();

        let mut p = PreviewPane::default();
        let id = p.open_path(tmp.clone());
        assert!(p.tabs()[p.active_idx()].editor.is_some());
        p.find = Some(FindState {
            tab_id: id,
            query: "needle".into(),
            current: 0,
            count: 2,
            case_sensitive: false,
            replacement: "SEO".into(),
            replace_open: true,
            query_focused: false,
            is_webview: false,
            pending_webview_exec: None,
            webview_pool_id: None,
        });
        assert!(p.replace_all(), "两处命中应全换掉");
        let editor_text = p.tabs()[p.active_idx()].editor.as_ref().unwrap().text();
        assert_eq!(editor_text, "SEO 1\nplain\nSEO 2");
        assert!(p.tabs()[p.active_idx()].dirty, "替换应标脏待 ⌘S 落盘");
        assert_eq!(p.find_state().unwrap().count, 0, "替换后主题串不再命中");
        assert!(!p.replace_all(), "无命中再替换是 no-op");
        std::fs::remove_file(tmp).ok();
    }

    #[cfg(not(feature = "codemirror"))]
    #[test]
    fn replace_current_targets_only_the_locked_occurrence_then_advances() {
        let tmp = std::env::temp_dir().join(format!("pane_replace_cur_{}.rs", std::process::id()));
        std::fs::write(&tmp, "aa bb aa\ncc").unwrap();

        let mut p = PreviewPane::default();
        let id = p.open_path(tmp.clone());
        p.find = Some(FindState {
            tab_id: id,
            query: "aa".into(),
            current: 1, // 窗口序第 1 个命中(0-based)= 第二个 aa。
            count: 2,
            case_sensitive: false,
            replacement: "Y".into(),
            replace_open: true,
            query_focused: false,
            is_webview: false,
            pending_webview_exec: None,
            webview_pool_id: None,
        });
        assert!(p.replace_current());
        let text = p.tabs()[p.active_idx()].editor.as_ref().unwrap().text();
        assert_eq!(text, "aa bb Y\ncc", "current=1 应该只替换第二个 aa");
        assert!(p.tabs()[p.active_idx()].dirty);
        std::fs::remove_file(tmp).ok();
    }

    #[cfg(not(feature = "codemirror"))]
    #[test]
    fn open_json_file_populates_both_editor_and_json_tree() {
        // JSON 是双视图:原生代码编辑器(RawText 半边)与 JSON 树(Tree 半边)
        // 同时存在,切换按钮在两者间选。不同于 tabular 的"独占认领"。
        let p = std::env::temp_dir().join(format!("json_route_{}.json", std::process::id()));
        std::fs::write(&p, "{}").unwrap();
        let mut pane = PreviewPane::default();
        pane.open_path(p.clone());
        let tab = &pane.tabs()[pane.active_idx()];
        assert!(
            tab.editor.is_some(),
            "JSON 应该仍然进原生代码编辑器(双视图之一)"
        );
        assert!(
            matches!(tab.json_tree, Some(JsonTreeState::Loading)),
            "JSON tab 应该同时进入 json_tree 的 Loading 态"
        );
        assert!(matches!(tab.backend_state, BackendState::Loading));
        std::fs::remove_file(p).ok();
    }

    #[test]
    fn json_mode_and_backend_state_stay_synchronized() {
        let p = std::env::temp_dir().join(format!("json_mode_{}.json", std::process::id()));
        std::fs::write(&p, "{\"a\":1}").unwrap();
        let mut pane = PreviewPane::default();
        let id = pane.open_path(p.clone());
        let view = crate::json_tree::load(&p).unwrap();
        pane.finish_json_tree_load(id, Ok(view));
        assert!(matches!(
            pane.tabs()[pane.active_idx()].backend_state,
            BackendState::Ready
        ));

        pane.json_tree_mut(id)
            .unwrap()
            .apply(crate::json_tree::Action::ToggleViewMode);
        pane.sync_json_backend_mode(id);
        assert_eq!(
            pane.tabs()[pane.active_idx()].current_mode(),
            Some(PreviewMode::Text)
        );

        std::fs::remove_file(p).ok();
    }

    #[test]
    fn json_tab_is_excluded_from_webview_pool() {
        let p = std::env::temp_dir().join(format!("json_webview_{}.json", std::process::id()));
        std::fs::write(&p, "{}").unwrap();
        let mut pane = PreviewPane::default();
        pane.open_path(p.clone());
        assert!(
            pane.desired_webviews().is_empty(),
            "json tab 不该进 webview 池"
        );
        std::fs::remove_file(p).ok();
    }

    #[cfg(not(feature = "codemirror"))]
    #[test]
    fn failed_json_tree_load_drops_tree_and_keeps_editor() {
        // 内容不是合法 JSON 时,加载失败分支会清掉 json_tree,让
        // tab 退回纯文本编辑器。editor 独立于树加载,
        // 清树后仍在;tab 因此重新回到"被原生编辑器认领"的形态,不进 webview 池。
        let p = std::env::temp_dir().join(format!("json_bad_{}.json", std::process::id()));
        std::fs::write(&p, "{ not valid json !! }").unwrap();
        let mut pane = PreviewPane::default();
        let id = pane.open_path(p.clone());
        assert!(pane.tabs()[pane.active_idx()].json_tree.is_some());
        pane.finish_json_tree_load(id, Err("invalid json".into()));
        let tab = &pane.tabs()[pane.active_idx()];
        assert!(tab.json_tree.is_none(), "清树后 json_tree 应为 None");
        assert!(tab.editor.is_some(), "清树不该动 editor");
        assert!(
            pane.desired_webviews().is_empty(),
            "退回文本编辑器后不该进 webview 池"
        );
        std::fs::remove_file(p).ok();
    }

    // ========== Blank 占位 tab 关闭 / 自动补回语义(2026-09-21 用户口径)==========
    // Blank 不再是不可关闭的固定锚点——有兄弟 tab 时真关,关完列表空时自动
    // 补回一个新 Blank(`next_id` 续号)。这俩测试守住这个不变量。

    fn push_file(pane: &mut PreviewPane, path: &str) -> usize {
        pane.push_tab(TabKind::File(PathBuf::from(path)), path.into())
    }

    /// 有兄弟 tab 时,`close(0)` 真把 Blank 删掉,列表里只剩文件 tab,
    /// `active` 自然收敛到 0(原来 idx 1 的文件)。
    #[test]
    fn close_blank_with_siblings_drops_blank_and_keeps_active_sane() {
        let mut p = PreviewPane::default();
        // 默认 active=0 (Blank);开两个文件 tab。
        let _ = push_file(&mut p, "/tmp/a.rs");
        let _ = push_file(&mut p, "/tmp/b.rs");
        assert_eq!(p.tabs().len(), 3);
        assert!(matches!(p.tabs()[0].kind, TabKind::Blank));
        p.select(2); // 切到 b.rs,确保 active 不再指着 Blank,关 Blank 不应改 active 指向
        assert_eq!(p.active_idx(), 2);

        p.close(0);
        assert_eq!(p.tabs().len(), 2, "Blank 真关,剩两个文件 tab");
        assert!(
            !matches!(p.tabs()[0].kind, TabKind::Blank),
            "关后 index 0 应该是文件,不再是 Blank"
        );
        // active 原本是 2,关掉 idx=0 后所有索引 -1,变成 1。
        assert_eq!(p.active_idx(), 1, "active 因 idx<active 收敛到 1");
    }

    /// 列表里只剩 Blank 时 `close(0)` 把它删掉,然后**自动补回一个 Blank**
    /// ——`next_id` 续号(新 Blank id != 0),`active = 0`。这是"关到只剩新
    /// 标签页"那种体验的数据层兜底。
    #[test]
    fn close_blank_when_only_blank_recreates_a_fresh_one() {
        let mut p = PreviewPane::default();
        assert_eq!(p.tabs().len(), 1);
        let original_id = p.tabs()[0].id;
        let original_next_id = p.next_id;

        p.close(0);
        assert_eq!(p.tabs().len(), 1, "关完必须自动补回,不能让列表停在空中");
        assert!(
            matches!(p.tabs()[0].kind, TabKind::Blank),
            "补回的仍是 Blank"
        );
        assert_eq!(
            p.tabs()[0].id,
            original_next_id,
            "新 Blank 用 next_id 续号,不复用旧 id"
        );
        assert_ne!(
            p.tabs()[0].id,
            original_id,
            "新 Blank id 必须递增,不能等于被关掉的 Blank id"
        );
        assert_eq!(p.next_id, original_next_id + 1, "next_id 也递增");
        assert_eq!(p.active_idx(), 0);
    }

    /// 关掉列表中间的文件 tab 时 `active` 收敛:被关项左侧 active 不动、
    /// 右侧 -1。覆盖 `close` 的另一条 `else if` 分支。
    #[test]
    fn close_middle_file_tab_shifts_active_left() {
        let mut p = PreviewPane::default();
        let _ = push_file(&mut p, "/tmp/a.rs");
        let _ = push_file(&mut p, "/tmp/b.rs");
        let _ = push_file(&mut p, "/tmp/c.rs");
        p.select(2); // active = 2 (b.rs)
        p.close(1); // 关 a.rs
        // tabs 变成 [Blank, b.rs, c.rs];active 2 > idx=1 ⇒ active -= 1 ⇒ 1。
        assert_eq!(p.tabs().len(), 3);
        assert_eq!(p.active_idx(), 1, "active 从 2 收到 1(原 b.rs)");
    }

    /// 越界关闭是 no-op:`PreviewCloseTab(idx)` 来自右键菜单关闭后位置可能
    /// 漂移,要稳。
    #[test]
    fn close_out_of_range_is_noop() {
        let mut p = PreviewPane::default();
        let _ = push_file(&mut p, "/tmp/a.rs");
        let tabs_before = p.tabs().len();
        let active_before = p.active_idx();
        p.close(99);
        assert_eq!(p.tabs().len(), tabs_before);
        assert_eq!(p.active_idx(), active_before);
    }

    /// Phase A 契约:每个新打开的文件 tab 都带唯一 route/backend/reason;
    /// `Blank` 占位没有 backend。
    #[test]
    fn every_new_file_tab_carries_route_and_backend() {
        let mut p = PreviewPane::default();
        assert!(
            p.tabs()[0].route.is_none() && p.tabs()[0].backend.is_none(),
            "Blank 占位 tab 不应有 route/backend"
        );
        for path in [
            "/tmp/a.rs",
            "/tmp/readme.md",
            "/tmp/data.json",
            "/tmp/t.csv",
        ] {
            let _ = push_file(&mut p, path);
        }
        for tab in p
            .tabs()
            .iter()
            .filter(|t| matches!(t.kind, TabKind::File(_)))
        {
            let route = tab.route.as_ref().expect("文件 tab 必须有 route");
            let backend = tab.backend.as_ref().expect("文件 tab 必须有 backend");
            assert_eq!(route.kind, backend.kind(), "route/backend kind 必须一致");
            assert!(
                !route.reason.to_string().is_empty(),
                "路由必须带可展示 reason"
            );
        }
    }

    /// Phase B 垂直切片:feature 开启时 Code tab 不再构造 iced CodeView，
    /// 而是产出带可信 host binding 的 editor WebView spec。
    #[cfg(feature = "codemirror")]
    #[test]
    fn code_tab_routes_to_bound_editor_webview_when_feature_is_enabled() {
        let path = std::env::temp_dir().join(format!(
            "codemirror_vertical_slice_{}.rs",
            std::process::id()
        ));
        std::fs::write(&path, "fn main() {}\n").unwrap();

        let mut pane = PreviewPane::default();
        let tab_id = pane.open_path(path.clone());
        let tab = &pane.tabs()[pane.active_idx()];
        assert!(matches!(tab.backend, Some(PreviewBackend::Code(_))));
        assert!(
            tab.editor.is_none(),
            "feature 开启后不应再构造 iced CodeView"
        );

        let specs = pane.desired_editor_webviews(42, crate::app::PanelKind::Files);
        assert_eq!(specs.len(), 1);
        let spec = &specs[0];
        assert_eq!(spec.id, tab_id);
        assert!(spec.visible);
        assert!(is_editor_url(&spec.url));
        let binding = spec.editor_binding.as_ref().expect("必须携带 host binding");
        assert_eq!(binding.project_id, 42);
        assert_eq!(binding.panel, crate::app::PanelKind::Files);
        assert_eq!(binding.tab_id, tab_id);
        assert_eq!(binding.path, path);

        std::fs::remove_file(path).ok();
    }

    /// editor 命令队列:排队后一次性取走,再取为空。
    #[test]
    fn editor_command_queue_round_trips() {
        let mut pane = PreviewPane::default();
        pane.queue_editor_command(3, EditorCommand::Focus);
        pane.queue_editor_command(
            7,
            EditorCommand::RevealPosition {
                line: 12,
                column: 2,
            },
        );
        let taken = pane.take_pending_editor_commands();
        assert_eq!(taken.len(), 2);
        assert_eq!(taken[0].0, 3);
        assert!(matches!(
            taken[1].1,
            EditorCommand::RevealPosition { line: 12, .. }
        ));
        assert!(pane.take_pending_editor_commands().is_empty());
    }

    /// 非 UTF-8 / 二进制内容的代码文件强制只读(避免 CodeMirror 解码后保存
    /// 损坏原文);普通 UTF-8 小文件仍可编辑。
    #[test]
    fn non_utf8_code_file_is_read_only() {
        let dir = std::env::temp_dir();
        let bad = dir.join(format!("non_utf8_{}.rs", std::process::id()));
        std::fs::write(&bad, b"fn main() {}\n\xFF\xFE not utf8").unwrap();
        let (_, backend, _) = route_and_backend(&bad, &crate::capabilities::current());
        match backend {
            PreviewBackend::Code(code) => assert_eq!(code.mode, CodeMode::ReadOnly),
            other => panic!("应是 Code,得到 {other:?}"),
        }
        std::fs::remove_file(&bad).ok();

        let good = dir.join(format!("utf8_{}.rs", std::process::id()));
        std::fs::write(&good, "fn main() {}\n").unwrap();
        let (_, backend, _) = route_and_backend(&good, &crate::capabilities::current());
        match backend {
            PreviewBackend::Code(code) => assert_eq!(code.mode, CodeMode::Editable),
            other => panic!("应是 Code,得到 {other:?}"),
        }
        std::fs::remove_file(&good).ok();
    }

    /// 外部文件变化:干净的 CodeMirror tab 自动重载(推进 reload_nonce →
    /// URL 换 `_r=`),脏 tab 不自动重载、置冲突提示。
    #[cfg(feature = "codemirror")]
    #[test]
    fn external_change_reloads_clean_and_flags_dirty_codemirror_tab() {
        let dir = std::env::temp_dir();
        let clean = dir.join(format!("ext_clean_{}.rs", std::process::id()));
        let dirty = dir.join(format!("ext_dirty_{}.rs", std::process::id()));
        std::fs::write(&clean, "fn main() {}\n").unwrap();
        std::fs::write(&dirty, "fn main() {}\n").unwrap();

        let mut pane = PreviewPane::default();
        let clean_id = pane.open_path(clean.clone());
        let dirty_id = pane.open_path(dirty.clone());
        // 模拟用户在脏 tab 上敲过字(生产路径由 DocumentChanged 事件置位)。
        if let Some(tab) = pane.tabs_mut().iter_mut().find(|t| t.id == dirty_id) {
            tab.dirty = true;
        }

        pane.reload_webviews_for(&[clean.clone(), dirty.clone()]);

        let clean_tab = pane.tabs().iter().find(|t| t.id == clean_id).unwrap();
        assert_eq!(clean_tab.reload_nonce, 1, "干净 tab 自动重载");
        assert!(clean_tab.web_error.is_none());
        let dirty_tab = pane.tabs().iter().find(|t| t.id == dirty_id).unwrap();
        assert_eq!(dirty_tab.reload_nonce, 0, "脏 tab 不自动重载");
        assert!(dirty_tab.web_error.is_some(), "脏 tab 进入冲突提示");

        let specs = pane.desired_editor_webviews(1, crate::app::PanelKind::Files);
        let clean_spec = specs.iter().find(|s| s.id == clean_id).unwrap();
        assert!(clean_spec.url.contains("&_r=1"), "重载 URL 带 nonce");

        std::fs::remove_file(&clean).ok();
        std::fs::remove_file(&dirty).ok();
    }

    /// Phase C Task 5:Suspended 壳不进 WebView 池,物化后转 Loading→Ready。
    #[test]
    fn shell_tab_is_suspended_and_hidden_from_webview_pool() {
        let path = std::env::temp_dir().join(format!("shell_{}.rs", std::process::id()));
        std::fs::write(&path, "fn main() {}\n").unwrap();

        let mut pane = PreviewPane::default();
        let id = pane.push_shell_tab(path.clone(), None);
        let tab = pane.tabs().iter().find(|t| t.id == id).unwrap();
        assert!(matches!(tab.backend_state, BackendState::Suspended));
        assert!(!tab.hosts_webview(), "Suspended 壳不建 WebView");
        assert!(pane.desired_webviews().iter().all(|s| s.id != id));
        assert!(
            pane.desired_editor_webviews(1, crate::app::PanelKind::Files)
                .iter()
                .all(|s| s.id != id),
            "未 Ready 的壳不产出 editor spec"
        );
        assert!(pane.is_suspended(id));

        assert!(pane.begin_shell_load(id));
        assert!(!pane.is_suspended(id));
        assert!(!pane.begin_shell_load(id), "非 Suspended 时幂等返回 false");
        pane.finish_shell_load(id);
        assert!(matches!(
            pane.tabs()
                .iter()
                .find(|t| t.id == id)
                .unwrap()
                .backend_state,
            BackendState::Ready
        ));

        std::fs::remove_file(&path).ok();
    }

    /// 壳恢复时,持久化 mode 只在该 route 支持时覆盖默认值。
    #[test]
    fn shell_tab_applies_supported_persisted_mode() {
        let json = std::env::temp_dir().join(format!("shell_mode_{}.json", std::process::id()));
        std::fs::write(&json, "{\"a\":1}\n").unwrap();
        let mut pane = PreviewPane::default();

        let id = pane.push_shell_tab(json.clone(), Some(PreviewMode::Text));
        let tab = pane.tabs().iter().find(|t| t.id == id).unwrap();
        assert_eq!(tab.route.as_ref().unwrap().default_mode, PreviewMode::Text);
        assert!(matches!(
            tab.backend,
            Some(PreviewBackend::Json(JsonBackend {
                mode: JsonMode::Text
            }))
        ));

        // 失效 mode(Code 不被 Json 支持)→ 落回默认 Tree。
        let id2 = pane.push_shell_tab(json.clone(), Some(PreviewMode::Code));
        let tab2 = pane.tabs().iter().find(|t| t.id == id2).unwrap();
        assert_eq!(tab2.route.as_ref().unwrap().default_mode, PreviewMode::Tree);

        std::fs::remove_file(&json).ok();
    }

    /// `file_policy` 窗口化的文件(超长首行)被判只读,且 `path_is_windowed`
    /// 为真。
    #[test]
    fn windowed_sparse_file_is_read_only_and_windowed() {
        let dir = std::env::temp_dir().join(format!("dozer_windowed_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("huge_line.rs");
        let f = std::fs::File::create(&path).unwrap();
        f.set_len(10 * 1024 * 1024).unwrap();
        drop(f);

        let caps = crate::capabilities::current();
        assert!(path_is_windowed(&path, &caps));
        let (_, backend, _) = route_and_backend(&path, &caps);
        match backend {
            PreviewBackend::Code(code) => assert_eq!(code.mode, CodeMode::ReadOnly),
            other => panic!("应是 Code,得到 {other:?}"),
        }
    }

    /// feature 打开时,窗口化 Code tab 走**窗口化 editor host**(URL 带
    /// `windowed=1`),且不再另起 Flyfish webview。
    #[cfg(feature = "codemirror")]
    #[test]
    fn windowed_code_tab_uses_windowed_editor_host() {
        let dir = std::env::temp_dir().join(format!("dozer_windowed_host_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("huge_line.rs");
        let f = std::fs::File::create(&path).unwrap();
        f.set_len(10 * 1024 * 1024).unwrap();
        drop(f);

        let mut pane = PreviewPane::default();
        let id = pane.open_path(path.clone());
        let tab = pane.tabs().iter().find(|t| t.id == id).unwrap();
        assert!(tab.windowed);
        assert!(tab.uses_windowed_editor());
        assert!(!tab.hosts_webview(), "窗口化不吃 Flyfish webview");
        let spec = pane
            .desired_editor_webviews(1, crate::app::PanelKind::Files)
            .into_iter()
            .find(|s| s.id == id)
            .expect("应产出 editor spec");
        assert!(spec.url.contains("windowed=1"));
        assert!(spec.url.contains("ro=1"));
    }

    /// feature 打开时,Markdown 切到 Source 模式由 CodeMirror editor host 承载,
    /// 不再走老 iced 源码视图、也不再另起 Flyfish webview。
    #[cfg(feature = "codemirror")]
    #[test]
    fn markdown_source_mode_uses_editor_host() {
        let path = std::env::temp_dir().join(format!("md_source_{}.md", std::process::id()));
        std::fs::write(&path, "# hi\n").unwrap();
        let mut pane = PreviewPane::default();
        let id = pane.open_path(path.clone());
        let idx = pane.tabs().iter().position(|t| t.id == id).unwrap();
        pane.enter_code_mode(idx).unwrap();

        let tab = pane.tabs().iter().find(|t| t.id == id).unwrap();
        assert!(tab.uses_rendered_source_editor());
        assert!(!tab.hosts_webview(), "Source 模式不吃 Flyfish webview");
        assert!(tab.editor.is_none(), "feature 下不再构造 iced CodeView");
        let spec = pane
            .desired_editor_webviews(1, crate::app::PanelKind::Files)
            .into_iter()
            .find(|s| s.id == id)
            .expect("应产出 editor spec");
        assert!(spec.url.contains("lang=markdown"));
        assert!(!spec.url.contains("windowed=1"));

        std::fs::remove_file(&path).ok();
    }

    /// feature 打开时,JSON 的 Text 模式由 CodeMirror editor host 承载(lang=json)。
    #[cfg(feature = "codemirror")]
    #[test]
    fn json_text_mode_uses_editor_host() {
        let path = std::env::temp_dir().join(format!("json_text_{}.json", std::process::id()));
        std::fs::write(&path, "{\"a\":1}\n").unwrap();
        let mut pane = PreviewPane::default();
        let id = pane.open_path(path.clone());

        // 默认 Tree:不产出 editor spec(由原生 json_tree 承载)。
        assert!(
            pane.desired_editor_webviews(1, crate::app::PanelKind::Files)
                .iter()
                .all(|s| s.id != id)
        );

        // 切到 Text:模拟树就绪 + 模式同步后的状态。
        if let Some(tab) = pane.tabs_mut().iter_mut().find(|t| t.id == id) {
            tab.backend_state = BackendState::Ready;
            tab.json_tree = None;
            if let Some(PreviewBackend::Json(json)) = tab.backend.as_mut() {
                json.mode = JsonMode::Text;
            }
        }
        let tab = pane.tabs().iter().find(|t| t.id == id).unwrap();
        assert!(tab.uses_editor_host());
        let spec = pane
            .desired_editor_webviews(1, crate::app::PanelKind::Files)
            .into_iter()
            .find(|s| s.id == id)
            .expect("Text 模式应产出 editor spec");
        assert!(spec.url.contains("lang=json"));

        std::fs::remove_file(&path).ok();
    }

    /// feature 打开时,CSV 的"原文"模式由 CodeMirror editor host 承载。
    #[cfg(feature = "codemirror")]
    #[test]
    fn csv_text_mode_uses_editor_host() {
        let path = std::env::temp_dir().join(format!("csv_text_{}.csv", std::process::id()));
        std::fs::write(&path, "name,age\nann,3\n").unwrap();
        let mut pane = PreviewPane::default();
        let id = pane.open_path(path.clone());

        // 默认网格:不产出 editor spec。
        assert!(
            pane.desired_editor_webviews(1, crate::app::PanelKind::Files)
                .iter()
                .all(|s| s.id != id)
        );

        // 切原文:翻 backend mode + 就绪。
        if let Some(tab) = pane.tabs_mut().iter_mut().find(|t| t.id == id) {
            tab.backend_state = BackendState::Ready;
            if let Some(PreviewBackend::Tabular(tabular)) = tab.backend.as_mut() {
                tabular.mode = TabularMode::Text;
            }
        }
        let tab = pane.tabs().iter().find(|t| t.id == id).unwrap();
        assert!(tab.uses_editor_host());
        assert!(!tab.hosts_webview(), "表格原文不吃 Flyfish");
        let spec = pane
            .desired_editor_webviews(1, crate::app::PanelKind::Files)
            .into_iter()
            .find(|s| s.id == id)
            .expect("原文模式应产出 editor spec");
        assert!(spec.url.contains("lang=txt"));

        std::fs::remove_file(&path).ok();
    }
}
