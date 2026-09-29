//! 预览域 view:`impl PreviewPane`(渲染/期望清单/Find 焦点捕获)+ 测试。

use iced_widget::core::Rectangle;
use iced_widget::core::widget::operation::Focusable;
use iced_widget::core::widget::{Id, Operation};
use std::path::PathBuf;

use super::*;

/// 窗口化 viewer 每次推给 CodeMirror 的窗口大小(以目标行为中心的前后行数)。
pub const WINDOW_BEFORE: u32 = 1000;
pub const WINDOW_AFTER: u32 = 2000;

/// 读文件的 mtime(取不到则 `None`)。T10 冲突检测/保存前校验共用。
pub(crate) fn disk_mtime(path: &std::path::Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

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
        runtime: PreviewRuntime::None,
        dirty: false,
        pending_jump_line: None,
        route: None,
        backend: None,
        backend_state: BackendState::Ready,
        windowed: false,
        load_state: PreviewLoadState::default(),
        recovery_written: false,
        pending_restore: None,
        load_observe: None,
        pending_view: None,
        pending_tabular: None,
        web_revision: 0,
        web_selection: None,
        web_selected_text: None,
        web_viewport: None,
        web_view_state: None,
        web_error: None,
        conflict: None,
        conflict_reload_armed: false,
        conflict_baseline: None,
        tabular_host_ready: false,
        task_cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        image_annotations: Vec::new(),
    }
}

/// T11:一份全新的后台任务取消信号(未置位)。tab 建壳/重试时用。
fn fresh_task_cancel() -> std::sync::Arc<std::sync::atomic::AtomicBool> {
    std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false))
}

/// 为文件 tab 计算 Phase A 的唯一 route/backend 描述。画像是**有界采样**
/// (头/尾各至多 64KiB);文件读不到(测试里的假路径、权限问题)时退回空画像,
/// 不影响按扩展名路由。
pub(crate) fn route_and_backend(
    path: &std::path::Path,
    capabilities: &crate::capabilities::ClientCapabilities,
) -> (PreviewRoute, PreviewBackend, BackendState) {
    let profile = profile_file(path).unwrap_or_else(|_| analyze(&[], None, 0, None));
    let (route, backend, read_only) = route_and_backend_from_profile(path, &profile, capabilities);
    let _ = read_only;
    // 创建即 Ready;表格的异步解析在 `push_tab`/`load_preview_tab` 里另行置
    // Loading(T2 runtime)。
    (route, backend, BackendState::Ready)
}

/// **无 I/O** 的临时路由:用空画像(仅按扩展名/文件名注册表)算 route/backend。
/// 生产打开路径先据此建壳、立即返回(UI 线程不做文件 I/O),后台 `profile_file`
/// 完成后经 [`route_and_backend_from_profile`] 精修 `read_only`/窗口化/编码
/// (T3)。对绝大多数按扩展名就能定 kind 的文件,临时 route 与最终 route 一致;
/// 只有"未知扩展名/文件名注册表 + 内容探针"这类依赖内容的判定会随后修正。
pub(crate) fn route_and_backend_provisional(
    path: &std::path::Path,
    capabilities: &crate::capabilities::ClientCapabilities,
) -> (PreviewRoute, PreviewBackend, bool) {
    let profile = analyze(&[], None, 0, None);
    route_and_backend_from_profile(path, &profile, capabilities)
}

/// 由画像计算 route/backend/read_only(纯函数,不碰磁盘)。`read_only` 表示
/// 该 Code backend 是否只读(非 UTF-8/二进制或超预算)。
pub(crate) fn route_and_backend_from_profile(
    path: &std::path::Path,
    profile: &FileProfile,
    capabilities: &crate::capabilities::ClientCapabilities,
) -> (PreviewRoute, PreviewBackend, bool) {
    let route = classify_preview(path, profile, capabilities, None);
    // 只读档:非 UTF-8/二进制(CodeMirror fetch().text() 会丢字节)或
    // `file_policy` 判定非 EditableCode(超 30MiB / 超预算)一律只读。
    let non_text = matches!(profile.utf8, Utf8Status::Invalid)
        || matches!(profile.content_kind, ContentKind::Binary);
    let policy = decide_text_policy(profile, &capabilities.budgets);
    let read_only = non_text || policy.policy != TextFilePolicy::EditableCode;
    let mut backend = PreviewBackend::from_route(&route, path, read_only);
    // T6:严格 `.json` 超 `json_tree_bytes` 预算时**不进 vanilla-jsoneditor**——
    // Tree 视图会让主线程/WebView 长时间解析超大文档,反而比文本更慢更不稳。
    // 改走 CodeMirror Text(超大再经上面 policy 判为窗口化只读)。
    if matches!(route.kind, PreviewKind::Json)
        && crate::preview::native_editor::is_strict_json_extension(path)
        && profile.size_bytes > capabilities.budgets.json_tree_bytes
        && let PreviewBackend::Json(json) = &mut backend
    {
        json.mode = JsonMode::Text;
    }
    (route, backend, read_only)
}

/// 该文件是否应按 `file_policy` 走**窗口化**(超预算 / 超 128MiB / 超长行)。
/// 窗口化专用 viewer 尚未落地,Phase C 期间在 feature 打开时仍走老的 iced
/// 分块只读,避免把超大文件整载进 CodeMirror WebView。
pub(crate) fn path_is_windowed(
    path: &std::path::Path,
    capabilities: &crate::capabilities::ClientCapabilities,
) -> bool {
    let profile = profile_file(path).unwrap_or_else(|_| analyze(&[], None, 0, None));
    path_is_windowed_profiled(&profile, capabilities)
}

/// 由画像判定窗口化(纯函数,不碰磁盘)。T3 后台画像回灌后用它重算。
pub(crate) fn path_is_windowed_profiled(
    profile: &FileProfile,
    capabilities: &crate::capabilities::ClientCapabilities,
) -> bool {
    decide_text_policy(profile, &capabilities.budgets).is_windowed()
}

/// 新建文件 tab 时的 route 来源(T3):
/// - [`TabRoute::SyncProfile`]:老的同步 `profile_file`(测试与遗留调用点);
/// - [`TabRoute::Provisional`]:无 I/O 的临时 route,后台画像后再精修。
///
/// 生产打开路径已全部迁到 `Provisional`(见 `App::preview_open_path*`),故
/// `SyncProfile` 分支只在测试中构造;与 `open_path`/`push_tab` 一并保留。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
enum TabRoute {
    SyncProfile,
    Provisional,
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

/// 窗口化大文件整文件搜索条查询框的稳定 `widget::Id`,与 `find_field_id`
/// 同款(两个面板各一,避免 iced 焦点树同 id 撞车)。也复用为「本条打开时是否
/// 需要程序化聚焦」的判据来源。
pub(crate) fn large_file_search_field_id(
    kind: crate::app::PanelKind,
) -> iced_widget::core::widget::Id {
    match kind {
        crate::app::PanelKind::Project => {
            iced_widget::core::widget::Id::new("preview-large-file-search-project")
        }
        _ => iced_widget::core::widget::Id::new("preview-large-file-search-files"),
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
        let (command, start_line, truncated) = {
            let Some(tab) = self.tabs.iter().find(|t| t.id == tab_id) else {
                return false;
            };
            let Some(index) = tab.window_index() else {
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
            (
                crate::preview::EditorCommand::SetWindow {
                    text: window.text,
                    start_line: window.start_line,
                    total_lines: index.total_lines(),
                    revision: tab.web_revision,
                    truncated: window.truncated,
                },
                window.start_line,
                window.truncated,
            )
        };
        self.queue_editor_command(tab_id, command);
        // 记录最近窗口元数据(T2 runtime)。
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id)
            && let Some(rt) = tab.window_runtime_mut()
        {
            rt.window_start_line = start_line;
            rt.truncated = truncated;
        }
        true
    }

    /// 应用后台建好的稀疏索引:仅当索引 revision 与 tab 当前 `web_revision`
    /// 一致时接受(文件已变则旧索引必须失效,不得套到新内容上)。返回是否接受。
    pub fn apply_window_index(
        &mut self,
        tab_id: usize,
        index: std::sync::Arc<crate::preview::LineIndex>,
    ) -> bool {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) else {
            return false;
        };
        if index.revision() != tab.web_revision {
            return false;
        }
        if !matches!(tab.runtime, PreviewRuntime::Windowed(_)) {
            tab.runtime = PreviewRuntime::Windowed(WindowedRuntime::default());
        }
        if let Some(rt) = tab.window_runtime_mut() {
            rt.index = Some(index);
            rt.error = None;
        }
        true
    }

    /// 应用 image-annotate host 事件。`bindings` 层已完成归属校验
    /// (`HostBinding::validate`),这里只按 `tab_id` 找 tab 落状态,语义与
    /// `Message::FlyfishEvent` 的各 arm 对齐:
    /// - `Ready`:host 脚本就绪,不代表图片已加载,清错误、保持 Loading;
    /// - `DocumentLoaded`:图片首帧就绪 → 置 Ready + finish(T8);
    /// - `Failed`:回落统一 Failed 终态(原生子视图随后移除,fallback 页可见);
    /// - `AnnotationsChanged`:整体替换内存镜像,不触加载状态。
    pub fn apply_image_annotate_event(
        &mut self,
        tab_id: usize,
        event: crate::preview::ImageAnnotateEvent,
    ) {
        use crate::preview::{BackendState, ImageAnnotateEvent, PreviewError, PreviewRuntime};
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) else {
            return;
        };
        match event {
            ImageAnnotateEvent::Ready => {
                tab.web_error = None;
            }
            ImageAnnotateEvent::DocumentLoaded => {
                if !tab.load_state.is_active() {
                    return;
                }
                tab.web_error = None;
                let _ = tab.backend_state.try_transition(BackendState::Ready);
                tab.load_state.finish();
            }
            ImageAnnotateEvent::Failed {
                message,
                recoverable,
            } => {
                tab.runtime = PreviewRuntime::None;
                tab.web_error = Some(message.clone());
                let _ = tab
                    .backend_state
                    .try_transition(BackendState::Failed(PreviewError::new(
                        message,
                        recoverable,
                    )));
                if tab.load_state.is_active() {
                    tab.load_state.finish();
                }
            }
            ImageAnnotateEvent::AnnotationsChanged { annotations } => {
                tab.image_annotations = annotations;
            }
        }
    }

    /// T10:「保留我的修改」:清冲突态,以当前磁盘 mtime 作保存基线(下次保存
    /// 覆盖前会再校验磁盘是否又变了)。返回是否有冲突被清除。
    pub fn keep_conflict_changes(&mut self, tab_id: usize) -> bool {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) else {
            return false;
        };
        if tab.conflict.is_none() {
            return false;
        }
        let TabKind::File(path) = &tab.kind else {
            return false;
        };
        tab.conflict_baseline = Some(disk_mtime(path).unwrap_or(std::time::UNIX_EPOCH));
        tab.conflict = None;
        tab.conflict_reload_armed = false;
        tab.web_error = None;
        true
    }

    /// T10:「重载磁盘」第一次点击 → 进入二次确认。返回 true 表示这次只是
    /// 置位(需再点一次),false 表示该 tab 无冲突/不存在。
    pub fn arm_conflict_reload(&mut self, tab_id: usize) -> bool {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) else {
            return false;
        };
        if tab.conflict.is_none() {
            return false;
        }
        tab.conflict_reload_armed = true;
        true
    }

    /// T10:「重载磁盘(丢弃我的修改)」清冲突、清 dirty、增加 reload nonce、
    /// 复位 revision 基线(磁盘内容将重新拉取)。返回被丢弃 tab 的路径,供
    /// 调用方清理 recovery 快照。
    pub fn discard_conflict_and_reload(&mut self, tab_id: usize) -> Option<PathBuf> {
        let tab = self.tabs.iter_mut().find(|t| t.id == tab_id)?;
        if tab.conflict.is_none() {
            return None;
        }
        let path = match &tab.kind {
            TabKind::File(p) => p.clone(),
            _ => return None,
        };
        tab.conflict = None;
        tab.conflict_reload_armed = false;
        tab.conflict_baseline = None;
        tab.dirty = false;
        tab.recovery_written = false;
        // T11:重载前取消在途后台任务(索引/解析),随后 `load_preview_tab` 起新任务。
        tab.cancel_background();
        tab.reload_nonce += 1;
        tab.web_revision = 0;
        tab.web_selection = None;
        tab.web_selected_text = None;
        tab.web_viewport = None;
        tab.web_error = None;
        Some(path)
    }

    /// T10:是否有激活 tab 正处冲突态(供渲染层决定是否画冲突条)。
    pub fn active_conflict(&self) -> Option<(usize, bool)> {
        let tab = self.tabs.get(self.active)?;
        tab.conflict.map(|_| (tab.id, tab.conflict_reload_armed))
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

    /// 老 iced editor 已退役:不再有原生编辑器焦点 id。
    pub fn find_editor_focus_id(&self) -> Option<iced_widget::core::widget::Id> {
        None
    }

    /// Find 条关闭(⌘F 第二下 / × / Esc)后焦点归回其下的代码编辑器——复用
    /// `pending_editor_focus` 机制,次帧 main.rs 用 `active_editor_focus_id`
    /// 把真实焦点拨回编辑器。
    pub fn request_editor_focus(&mut self) {
        self.pending_editor_focus = true;
        self.pending_find_focus = false;
    }

    /// 老 iced editor 已退役:不再有原生编辑器焦点 id。
    pub fn active_editor_focus_id(&self) -> Option<iced_widget::core::widget::Id> {
        None
    }

    /// 当前激活 tab 是否为原生可编辑渲染(仅剩 Tabular 网格)。
    pub fn active_tab_is_native(&self) -> bool {
        self.tabs
            .get(self.active)
            .is_some_and(|t| t.tabular_state().is_some())
    }

    /// 同一文件已开的 tab 下标(供 `open_path` 与 `App::preview_open_path`
    /// 的异步路径共用同一份"已打开则复用"判重逻辑)。
    pub fn find_existing_file_tab(&self, path: &std::path::Path) -> Option<usize> {
        self.tabs
            .iter()
            .position(|t| t.kind == TabKind::File(path.to_path_buf()))
    }

    /// 同步打开(建壳即 Ready,走 `profile_file` 同步画像)。
    ///
    /// T3 起生产打开路径改用 [`PreviewPane::open_path_provisional`](UI 线程
    /// 不读盘);此同步版本保留给测试与"必须立即拿到 Ready tab"的遗留调用点。
    #[allow(dead_code)]
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

    /// T3:**不读盘**地打开一个文件 tab:按扩展名建临时 route 壳、置
    /// `BackendState::Loading` + `PreviewLoadStage::Profiling`,立即返回
    /// `(tab_id, generation)`。调用方随后在后台 `spawn_blocking(profile_file)`,
    /// 结果经 [`PreviewPane::apply_profile`] 精修。UI 线程不做任何文件 I/O。
    ///
    /// 同一文件已开则直接复用、返回其现有 generation(不重新画像),此时调用方
    /// 不应再派发 profile 任务(见 `App::preview_open_path` 的分支)。
    pub fn open_path_provisional(&mut self, path: PathBuf) -> (usize, Option<u64>) {
        if let Some(idx) = self.find_existing_file_tab(&path) {
            let id = self.tabs[idx].id;
            self.active = idx;
            self.cull_stale_find();
            return (id, None);
        }
        let title = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        let id = self.push_tab_with(TabKind::File(path), title, TabRoute::Provisional);
        (id, Some(self.load_generation(id)))
    }

    /// 该 tab 当前 load generation(不存在返回 0)。供调用方 spawn 后台任务时
    /// 记下,结果回灌时做 generation 闸门。
    pub fn load_generation(&self, tab_id: usize) -> u64 {
        self.tabs
            .iter()
            .find(|t| t.id == tab_id)
            .map(|t| t.load_state.generation)
            .unwrap_or(0)
    }

    /// 追加一个真实 tab(占位 `Blank` 恒在 index 0,这里只用来加 `File`)。
    /// 文件 tab 一律追加在末尾,占位 tab 永远留在最前面,形态对齐 SSH/数据库
    /// 面板 tab 条最前面那个固定"空白"占位。这是唯一的新建文件 tab 路径:
    /// 同步建壳(不读盘),内容由 editor/Flyfish host 或后台任务异步加载。
    #[allow(dead_code)]
    fn push_tab(&mut self, kind: TabKind, title: String) -> usize {
        self.push_tab_with(kind, title, TabRoute::SyncProfile)
    }

    /// `push_tab` 的路由来源选择:[`TabRoute::SyncProfile`] 走老的同步
    /// `profile_file`(测试与遗留调用点,建完即 Ready);[`TabRoute::Provisional`]
    /// 走无 I/O 的临时 route 且停在 `Profiling`(生产异步打开路径,T3)。
    fn push_tab_with(&mut self, kind: TabKind, title: String, route_src: TabRoute) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        let (route, backend, backend_state) = match &kind {
            TabKind::File(path) => {
                let (route, backend, state) = match route_src {
                    TabRoute::SyncProfile => route_and_backend(path, &self.capabilities),
                    TabRoute::Provisional => {
                        let (r, b, _ro) = route_and_backend_provisional(path, &self.capabilities);
                        (r, b, BackendState::Loading)
                    }
                };
                (Some(route), Some(backend), state)
            }
            TabKind::Blank => (None, None, BackendState::Ready),
        };
        // 老 iced editor 已退役:不再同步读盘构造 CodeView。
        // 表格类文件委托 Tabular Viewer(与 editor 互斥)。实际解析是异步的
        // (calamine/csv 对大文件可能要跑一阵,不能卡在这个同步方法里,见
        // `crate::tabular` 模块文档的"够数即停"性能策略)——这里只登记
        // "这是个表格 tab、正在加载",把 `(id, path)` 记进
        // `pending_tabular_loads` 队列,交给调用方(手上有 handle/proxy 那层,
        // 即 `app/update.rs` 的 `preview_open_path`/`project_preview_open_path`)
        // 在 `push_tab` 返回之后立即 `take_pending_tabular_loads()` 取走并
        // spawn 后台加载。`open_path` 复用已存在 tab 的分支不经过 `push_tab`,
        // 不会重复入队,不会对一个正在加载的 tab 触发第二次加载。
        let mut is_tabular = false;
        if let TabKind::File(path) = &kind
            && route
                .as_ref()
                .is_some_and(|r| r.kind == PreviewKind::Tabular)
        {
            self.pending_tabular_loads.push((id, path.clone()));
            is_tabular = true;
        }
        // 临时 route 阶段不做窗口化 I/O 探测(尺寸未知 → 先按非窗口化);真正的
        // 窗口化判定在 `apply_profile` 里用画像重算(T3)。
        let windowed = match (&kind, route_src) {
            (TabKind::File(path), TabRoute::SyncProfile) => {
                path_is_windowed(path, &self.capabilities)
            }
            _ => false,
        };
        let runtime = if is_tabular {
            PreviewRuntime::Tabular(TabularState::Loading)
        } else if windowed {
            PreviewRuntime::Windowed(WindowedRuntime::default())
        } else {
            PreviewRuntime::None
        };
        let backend_state = if is_tabular {
            BackendState::Loading
        } else {
            backend_state
        };
        // 生产异步路径:壳一建好就停在 `Profiling`(generation 1),后台画像
        // 结果回灌后推进阶段;同步路径保持 `Idle`(老行为,建完即 Ready)。
        let load_state = match (&kind, route_src) {
            (TabKind::File(_), TabRoute::Provisional) => {
                PreviewLoadState::starting(1, PreviewLoadStage::Profiling)
            }
            _ => PreviewLoadState::default(),
        };
        let tab = PreviewTab {
            id,
            kind,
            title,
            reload_nonce: 0,
            runtime,
            dirty: false,
            pending_jump_line: None,
            route,
            backend,
            backend_state,
            windowed,
            load_state,
            recovery_written: false,
            pending_restore: None,
            load_observe: None,
            pending_view: None,
            pending_tabular: None,
            web_revision: 0,
            web_selection: None,
            web_selected_text: None,
            web_viewport: None,
            web_view_state: None,
            web_error: None,
            conflict: None,
            conflict_reload_armed: false,
            conflict_baseline: None,
            tabular_host_ready: false,
            task_cancel: fresh_task_cancel(),
            image_annotations: Vec::new(),
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
        // 流式 JSONL/NDJSON(T8)的 Streamed/Text 持久模式同样补一次。
        if let PreviewBackend::Streamed(streamed) = &mut backend {
            streamed.mode = match route.default_mode {
                PreviewMode::Text => StreamedMode::Text,
                _ => StreamedMode::Streamed,
            };
        }
        let windowed = path_is_windowed(&path, &self.capabilities);
        let runtime = if windowed {
            PreviewRuntime::Windowed(WindowedRuntime::default())
        } else {
            PreviewRuntime::None
        };
        let tab = PreviewTab {
            id,
            kind: TabKind::File(path),
            title,
            reload_nonce: 0,
            runtime,
            dirty: false,
            pending_jump_line: None,
            route: Some(route),
            backend: Some(backend),
            backend_state: BackendState::Suspended,
            windowed,
            load_state: PreviewLoadState::default(),
            recovery_written: false,
            pending_restore: None,
            load_observe: None,
            pending_view: None,
            pending_tabular: None,
            web_revision: 0,
            web_selection: None,
            web_selected_text: None,
            web_viewport: None,
            web_view_state: None,
            web_error: None,
            conflict: None,
            conflict_reload_armed: false,
            conflict_baseline: None,
            tabular_host_ready: false,
            task_cancel: fresh_task_cancel(),
            image_annotations: Vec::new(),
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

    /// 某个 tab 是否可被物化:未物化的壳(`Suspended`)或加载失败后的重试
    /// (`Failed`)。`load_preview_tab` 用它做幂等闸门。
    pub fn is_pending_load(&self, tab_id: usize) -> bool {
        self.tabs.iter().find(|t| t.id == tab_id).is_some_and(|t| {
            matches!(
                t.backend_state,
                BackendState::Suspended | BackendState::Failed(_)
            )
        })
    }

    /// T3:释放某文件 tab 的运行时 viewer(索引/表格网格/镜像),退回 Suspended
    /// 壳。route/backend 保留,下次选中会重新物化(窗口化索引随之重建)。这是
    /// 资源淘汰闭环里"销毁 runtime/WebView"那一步的 tab 侧原语。返回是否动作。
    /// (T3 闭环的 app 侧接线未完成前,仅测试使用;见 wrap-up T3。)
    #[allow(dead_code)]
    pub fn suspend_tab(&mut self, tab_id: usize) -> bool {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) else {
            return false;
        };
        if !matches!(tab.kind, TabKind::File(_))
            || matches!(tab.backend_state, BackendState::Suspended)
        {
            return false;
        }
        // T11:淘汰即取消在途后台任务(索引/解析),重新物化时再起新任务。
        tab.cancel_background();
        tab.runtime = PreviewRuntime::None;
        // T11:淘汰前没有时间做一次 `SerializeViewState` 往返,直接把最近一次
        // 节流镜像落成 `pending_view`,重新物化时经 `RestoreViewState` 还原
        // (cursor/selection/top_line/folds)。
        if let Some(mirror) = tab.web_view_state.clone() {
            tab.pending_view = Some(mirror);
        }
        tab.web_revision = 0;
        tab.web_selection = None;
        tab.web_selected_text = None;
        tab.web_viewport = None;
        let _ = tab.backend_state.try_transition(BackendState::Suspended);
        true
    }

    /// T3/T10:reserve 被拒时给 tab 一个可解释的终态(Failed,可重试),由统一
    /// fallback 页/错误条呈现,而不是静默空白或无限 loading。
    ///
    /// `generation` 为该 tab 等待预算时的 loading 世代:只有仍匹配当前加载
    /// 才处理(旧世代被拒不得结束新加载),并作废该世代使在途 host 结果失效。
    pub fn mark_reserve_denied(
        &mut self,
        tab_id: usize,
        generation: u64,
        reason: impl Into<String>,
    ) -> bool {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) else {
            return false;
        };
        if !tab.load_state.accepts(generation) {
            return false;
        }
        let reason = reason.into();
        tab.runtime = PreviewRuntime::None;
        tab.web_error = Some(reason.clone());
        let _ = tab.backend_state.try_transition(BackendState::Loading);
        let _ = tab
            .backend_state
            .try_transition(BackendState::Failed(PreviewError::new(reason, true)));
        // 结束本次加载(回到 Idle 并推进世代),避免 fallback 页背后还有在途
        // host 结果能"结束新 loading"(T10 bullet 4)。
        tab.load_state.finish();
        true
    }

    /// T10 bullet 3:后台 recovery 读取结果落回 tab。`generation` 为物化时的
    /// load 世代:
    ///
    /// - 若该次加载仍在途(`accepts(generation)`),记进 `pending_restore`,由
    ///   `DocumentLoaded` 消费(与同步路径一致)。
    /// - 若 host 已就绪(世代已随 finish 推进),且 tab 非窗口化、尚未脏,则**立即**
    ///   返回一份 `(tab_id, text, revision)` 恢复指令,由调用方排队 `SetDocument`
    ///   并标脏——覆盖"recovery 读比 host 就绪还慢"的竞态。
    /// - 其余情况(过期/已脏/窗口化)丢弃,返回 `None`。
    pub fn apply_recovery_restore(
        &mut self,
        tab_id: usize,
        generation: u64,
        text: String,
    ) -> Option<(usize, String, u64)> {
        let tab = self.tabs.iter_mut().find(|t| t.id == tab_id)?;
        if tab.windowed || tab.dirty {
            return None;
        }
        if tab.load_state.accepts(generation) {
            tab.pending_restore = Some(text);
            return None;
        }
        // 世代已推进(finish/cancel)且不是"更晚的新加载"(generation 只差 1 为
        // 本次 finish)。更晚的加载会是 generation+2 及以上,交由新加载处理。
        if tab.load_state.generation != generation.wrapping_add(1) {
            return None;
        }
        tab.dirty = true;
        tab.recovery_written = true;
        Some((tab.id, text, tab.web_revision))
    }

    /// T10:reserve 获批,把仍在 `Reserving` 等待的 host 推进到 `CreatingHost`。
    /// generation 过期(旧请求)返回 `false`。非 `Reserving` 阶段是 no-op 返回
    /// `false`(例如 host 早已就绪),不算错误。
    pub fn grant_reserve(&mut self, tab_id: usize, generation: u64) -> bool {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) else {
            return false;
        };
        if !tab.load_state.accepts(generation)
            || tab.load_state.stage != PreviewLoadStage::Reserving
        {
            return false;
        }
        tab.load_state.advance(PreviewLoadStage::CreatingHost);
        true
    }

    // ── T2:统一 loading 转换 API ─────────────────────────────────────────
    //
    // 约定(见 plan T2):
    // - 只有 generation 匹配的 advance/finish/fail 才能改动 tab,防旧结果串台。
    // - `begin_load` 只做状态转换、立即返回;绝不在内部做 I/O 或等待。
    // - `finish_load` 只能在"首个可用画面已准备好"后调用,不是"任务已 spawn"。
    // - `fail_load` 进入统一 Failed/fallback,并清理运行时 viewer。
    // - `cancel_load` 是正常结束(关闭/刷新/切 mode),不显示错误。

    /// 开始一次加载,返回新 `generation`。仅当 tab 存在且处于可物化态
    /// (`Suspended`/`Failed`/`Ready`→重新加载)时推进状态并返回 `Some(gen)`;
    /// 否则返回 `None`(幂等保护)。调用方拿到 `gen` 后应立即返回事件循环,
    /// 把耗时工作交给后台任务(plan T3)。
    pub fn begin_load(&mut self, tab_id: usize, stage: PreviewLoadStage) -> Option<u64> {
        let tab = self.tabs.iter_mut().find(|t| t.id == tab_id)?;
        // T11:新一次加载换一份全新的取消信号,避免沿用上一轮(可能已被取消)的信号。
        tab.cancel_background();
        // 先推进 BackendState(同态 Loading→Loading 合法,no-op)。
        let _ = tab.backend_state.try_transition(BackendState::Loading);
        let generation = tab.load_state.generation.wrapping_add(1);
        tab.load_state = PreviewLoadState::starting(generation, stage);
        // T11 bullet 4:开一次三段延迟观测(首帧由视图组装时写回)。
        tab.load_observe = Some(crate::preview::LoadObservation::new(generation));
        // T11 bullet 3:结构化日志(只记阶段/世代,不记正文)。
        tracing::debug!(tab_id, generation, stage = ?stage, "预览加载开始");
        Some(generation)
    }

    /// 推进到下一阶段(不改 generation)。generation 过期或阶段为 `Idle` 时
    /// 返回 false,不改动状态。
    ///
    /// T2 建立 API;真正"多阶段异步"的接线在 T3–T11(Windowed/JSON/搜索等)。
    /// 迁移期仅测试使用,故显式允许 dead_code(同 `backend.rs` 做法)。
    #[allow(dead_code)]
    pub fn advance_load(
        &mut self,
        tab_id: usize,
        generation: u64,
        stage: PreviewLoadStage,
    ) -> bool {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) else {
            return false;
        };
        if !tab.load_state.accepts(generation) {
            return false;
        }
        let from = tab.load_state.stage;
        tab.load_state.advance(stage);
        // T11 bullet 3:阶段推进日志(含已等待毫秒,不记正文)。
        tracing::debug!(
            tab_id,
            generation,
            from = ?from,
            to = ?stage,
            elapsed_ms = tab.load_state.elapsed().as_millis() as u64,
            "预览加载阶段推进"
        );
        true
    }

    /// T11 bullet 3:某个已 arm 的看门狗超时是否仍应生效——tab 仍停在**同
    /// generation 且同 stage**。stage 不符说明已推进到下一阶段(新超时接管),
    /// generation 不符说明已重开/被替换,两者都应丢弃迟到的旧超时。
    pub fn load_timeout_matches(
        &self,
        tab_id: usize,
        generation: u64,
        stage: PreviewLoadStage,
    ) -> bool {
        self.tabs
            .iter()
            .find(|t| t.id == tab_id)
            .is_some_and(|t| t.load_state.accepts(generation) && t.load_state.stage == stage)
    }

    /// 结束加载(首个可用画面已就绪)。generation 过期返回 false。
    pub fn finish_load(&mut self, tab_id: usize, generation: u64) -> bool {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) else {
            return false;
        };
        if !tab.load_state.accepts(generation) {
            return false;
        }
        let _ = tab.backend_state.try_transition(BackendState::Ready);
        tab.load_state.finish();
        true
    }

    /// 加载失败:进入统一 Failed 终态(可解释、可重试),清运行时 viewer。
    /// generation 过期返回 false(旧任务的失败不得结束新加载)。
    ///
    /// T2 建立 API;后台失败回灌接线在 T3–T11。迁移期仅测试使用。
    #[allow(dead_code)]
    pub fn fail_load(&mut self, tab_id: usize, generation: u64, error: PreviewError) -> bool {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) else {
            return false;
        };
        if !tab.load_state.accepts(generation) {
            return false;
        }
        tab.runtime = PreviewRuntime::None;
        tab.web_error = Some(error.message.clone());
        // T11 bullet 3:失败原因 + 已等待毫秒(不记正文)。
        tracing::warn!(
            tab_id,
            generation,
            stage = ?tab.load_state.stage,
            elapsed_ms = tab.load_state.elapsed().as_millis() as u64,
            reason = %error.message,
            retryable = error.retryable,
            "预览加载失败"
        );
        let _ = tab
            .backend_state
            .try_transition(BackendState::Failed(error));
        tab.load_state.finish();
        true
    }

    /// 取消在途加载(关闭/刷新/切 mode/项目切换)。取消是正常结束:回到
    /// `Idle` 并作废结果,不置 Failed。若还在 `Loading`,退回 `Suspended` 以便
    /// 下次选中重新物化。
    ///
    /// T2 建立 API;关闭/刷新/切 mode 的接线在 T11。迁移期仅测试使用。
    #[allow(dead_code)]
    pub fn cancel_load(&mut self, tab_id: usize) -> bool {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) else {
            return false;
        };
        // T11:先置位后台任务取消信号(索引/解析/recovery 尽早退出),再走状态机。
        tab.cancel_background();
        if !tab.load_state.is_active() {
            return false;
        }
        // T11 bullet 3:取消原因(阶段 + 已等待毫秒)。
        tracing::debug!(
            tab_id,
            generation = tab.load_state.generation,
            stage = ?tab.load_state.stage,
            elapsed_ms = tab.load_state.elapsed().as_millis() as u64,
            "预览加载取消"
        );
        tab.load_state.cancel();
        if matches!(tab.backend_state, BackendState::Loading) {
            let _ = tab.backend_state.try_transition(BackendState::Suspended);
        }
        true
    }

    /// 该 tab 当前的 loading 阶段(诊断/渲染用)。不存在返回 `Idle`。
    ///
    /// T2 建立 API;诊断页(T11)与局部 loading 接线前迁移期仅测试使用。
    #[allow(dead_code)]
    pub fn load_stage(&self, tab_id: usize) -> PreviewLoadStage {
        self.tabs
            .iter()
            .find(|t| t.id == tab_id)
            .map(|t| t.load_state.stage)
            .unwrap_or(PreviewLoadStage::Idle)
    }

    /// T11 bullet 5:所有 tab 当前 loading 任务的可观测快照(诊断用)。只含
    /// stage/generation/elapsed/cancellation,不含正文;`Idle` 的 tab 跳过。
    ///
    /// 诊断页尚未建 UI(见 plan T11 bullet 5 的轻量落法):当前由测试与
    /// [`Self::log_load_diagnostics`] 消费。
    #[allow(dead_code)]
    pub fn load_diagnostics(&self) -> Vec<crate::preview::LoadDiagnostic> {
        use std::sync::atomic::Ordering;
        self.tabs
            .iter()
            .filter(|t| t.load_state.is_active())
            .map(|t| crate::preview::LoadDiagnostic {
                tab_id: t.id,
                label: t.title.clone(),
                stage: t.load_state.stage,
                generation: t.load_state.generation,
                elapsed_ms: t.load_state.elapsed().as_millis() as u64,
                first_frame_drawn: t
                    .load_observe
                    .as_ref()
                    .is_some_and(|o| o.first_frame_offset_ms().is_some()),
                cancellation_requested: t.task_cancel.load(Ordering::Relaxed),
            })
            .collect()
    }

    /// T11 bullet 5:把当前 loading 诊断快照写成一条结构化日志(无 UI 依赖的
    /// "诊断页"轻量形态;调用方可按需在 debug 场景触发)。
    ///
    /// 当前无 UI 触发入口(见 [`Self::load_diagnostics`] 说明),显式允许
    /// dead_code。
    #[allow(dead_code)]
    pub fn log_load_diagnostics(&self, panel: &str) {
        for diag in self.load_diagnostics() {
            tracing::info!(
                panel,
                tab_id = diag.tab_id,
                label = %diag.label,
                stage = ?diag.stage,
                generation = diag.generation,
                elapsed_ms = diag.elapsed_ms,
                first_frame_drawn = diag.first_frame_drawn,
                cancellation_requested = diag.cancellation_requested,
                "预览加载诊断"
            );
        }
    }

    /// 物化一个 Suspended 壳(或从 Failed 重试):标记为 `Loading`。返回 false
    /// 表示该 tab 不存在或当前不是可物化态(幂等保护)。
    ///
    /// T2 起内部委托 [`PreviewPane::begin_load`](阶段取 `CreatingHost`);保留
    /// 此方法给尚未迁移到显式阶段的调用点(plan:逐步淘汰)。`load_preview_tab`
    /// 已改用显式阶段 API,故当前仅测试引用。
    #[allow(dead_code)]
    pub fn begin_shell_load(&mut self, tab_id: usize) -> bool {
        let materializable = self.tabs.iter().find(|t| t.id == tab_id).is_some_and(|t| {
            matches!(
                t.backend_state,
                BackendState::Suspended | BackendState::Failed(_)
            )
        });
        if !materializable {
            return false;
        }
        self.begin_load(tab_id, PreviewLoadStage::CreatingHost)
            .is_some()
    }

    /// T3:后台画像完成,精修临时 route 的壳。generation 过期(旧任务)返回
    /// false,不改动状态。
    ///
    /// 用画像重算 `(route, backend, read_only)` 与窗口化,覆盖建壳时的临时
    /// 扩展名判定。表格类文件此时才把 `(id, path)` 记进 `pending_tabular_loads`
    /// (临时 route 已按 `.csv`/`.xlsx` 判为表格的,建壳时已入队,这里不重复;
    /// 只处理"临时判非表格、画像后改判表格"的罕见内容探测场景)。
    ///
    /// 注意:本方法只推进 route/backend/窗口化,**不结束加载**——真正的
    /// `finish_load` 由后续 viewer/host 就绪事件(T4 起)或同步就绪分支负责。
    pub fn apply_profile(&mut self, tab_id: usize, generation: u64, profile: &FileProfile) -> bool {
        let capabilities = self.capabilities.clone();
        // 先在不可变借用下取路径,避免与后面对 `self.pending_tabular_loads` 的
        // 可变借用冲突。
        let path = match self.tabs.iter().find(|t| t.id == tab_id) {
            Some(tab)
                if tab.load_state.accepts(generation) && matches!(tab.kind, TabKind::File(_)) =>
            {
                match &tab.kind {
                    TabKind::File(p) => p.clone(),
                    TabKind::Blank => return false,
                }
            }
            _ => return false,
        };
        let (route, backend, _read_only) =
            route_and_backend_from_profile(&path, profile, &capabilities);
        let windowed = path_is_windowed_profiled(profile, &capabilities);
        let is_tabular = route.kind == PreviewKind::Tabular;
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) else {
            return false;
        };
        let was_tabular = tab
            .route
            .as_ref()
            .is_some_and(|r| r.kind == PreviewKind::Tabular);
        tab.route = Some(route);
        tab.backend = Some(backend);
        tab.windowed = windowed;
        // 画像后改判为表格(建壳时未入队)→ 现在入队并置 Loading。
        let newly_tabular = is_tabular && !was_tabular;
        if newly_tabular {
            tab.runtime = PreviewRuntime::Tabular(TabularState::Loading);
        } else if is_tabular {
            // 建壳时已入队并置 Loading,保持。
        } else if windowed {
            tab.runtime = PreviewRuntime::Windowed(WindowedRuntime::default());
        } else {
            tab.runtime = PreviewRuntime::None;
        }
        if newly_tabular {
            self.pending_tabular_loads.push((tab_id, path));
        }
        true
    }

    /// 物化完成(Loading→Ready),或同步可立即就绪的 shell 直接置 Ready。
    ///
    /// T2 起内部委托 [`PreviewPane::finish_load`];因同步路径没有独立后台任务,
    /// 直接用当前 generation 结束。当前仅测试引用(见 `begin_shell_load`)。
    #[allow(dead_code)]
    pub fn finish_shell_load(&mut self, tab_id: usize) {
        let generation = match self.tabs.iter().find(|t| t.id == tab_id) {
            Some(tab) => tab.load_state.generation,
            None => return,
        };
        self.finish_load(tab_id, generation);
    }

    /// 物化 Tabular shell:登记表格后台加载并置 Loading 态。
    pub fn set_tabular_loading(&mut self, tab_id: usize, path: PathBuf) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
            tab.runtime = PreviewRuntime::Tabular(TabularState::Loading);
        }
        self.pending_tabular_loads.push((tab_id, path));
    }

    /// 记录启动恢复的视图状态,editor `ready` 后应用一次。
    pub fn set_pending_view(&mut self, tab_id: usize, state: ViewStateRestore) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
            tab.pending_view = Some(state);
        }
    }

    /// 只读访问器,供渲染层判断要不要画窗口化大文件搜索条(同 `find_state`
    /// 的既有写法)。
    pub fn large_file_search_state(&self) -> Option<&LargeFileSearch> {
        self.large_file_search.as_ref()
    }

    /// 打开只读大文件档搜索条,锁定 `tab_id`。已开着且锁的是同一个 tab 则
    /// no-op(保留已输入的 query,同 `FindState` 既有语义);换 tab 重开会
    /// 丢旧会话(`query`/`hits`/错误清空)并作废在途查询。
    pub fn open_large_file_search(&mut self, tab_id: usize) {
        if self
            .large_file_search
            .as_ref()
            .is_some_and(|s| s.tab_id == tab_id)
        {
            return;
        }
        if let Some(old) = self.large_file_search.as_mut() {
            old.cancel_inflight();
        }
        self.large_file_search = Some(LargeFileSearch {
            tab_id,
            ..Default::default()
        });
    }

    /// 关闭搜索条:作废在途查询(置位共享取消信号)后丢弃会话。
    pub fn close_large_file_search(&mut self) {
        if let Some(session) = self.large_file_search.as_mut() {
            session.cancel_inflight();
        }
        self.large_file_search = None;
    }

    /// 启动一次查询:作废旧 generation,清错误,`running = true`,返回本查询的
    /// generation 与共享取消句柄(调用方捕获进 `spawn_blocking`,回灌时用
    /// generation 校验)。会话已关闭/未锁到该 tab 返回 `None`。
    pub fn start_large_file_search(
        &mut self,
        tab_id: usize,
        query: String,
    ) -> Option<(u64, std::sync::Arc<std::sync::atomic::AtomicBool>)> {
        let session = self.large_file_search.as_mut()?;
        if session.tab_id != tab_id {
            return None;
        }
        session.cancel_inflight();
        session.generation = session.generation.wrapping_add(1);
        session.query = query;
        session.running = true;
        session.hits.clear();
        session.current = 0;
        session.total_matches = 0;
        session.truncated = false;
        session.progress = None;
        Some((session.generation, session.cancel.clone()))
    }

    /// 提交查询时查询为空:立即取消在途查询并清掉 loading(T9 bullet 5),
    /// 会话保留(仍开着条)。返回是否命中会话。
    pub fn clear_large_file_search(&mut self, tab_id: usize) -> bool {
        let Some(session) = self.large_file_search.as_mut() else {
            return false;
        };
        if session.tab_id != tab_id {
            return false;
        }
        session.cancel_inflight();
        session.generation = session.generation.wrapping_add(1);
        session.query.clear();
        session.running = false;
        session.hits.clear();
        session.current = 0;
        session.total_matches = 0;
        session.truncated = false;
        session.progress = None;
        true
    }

    /// 进度回灌(限频已由发送方保证)。generation 不符或已关闭则丢弃。
    pub fn set_large_file_search_progress(
        &mut self,
        tab_id: usize,
        generation: u64,
        progress: crate::preview::PreviewLoadProgress,
    ) {
        if let Some(session) = self.large_file_search.as_mut()
            && session.tab_id == tab_id
            && session.generation == generation
            && session.running
        {
            session.progress = Some(progress);
        }
    }

    /// 异步搜索结果回灌:会话已关闭 / 已换锁别的 tab / generation 过期(用户
    /// 又提交了新查询)时静默丢弃。`Ok` 携带总数与截断位(T9 bullet 4:
    /// 命中列表封顶但总数仍统计)。
    pub fn set_large_file_search_results(
        &mut self,
        tab_id: usize,
        generation: u64,
        result: Result<crate::preview::SearchOutcome, String>,
    ) {
        let Some(session) = self.large_file_search.as_mut() else {
            return;
        };
        if session.tab_id != tab_id || session.generation != generation {
            return;
        }
        session.running = false;
        session.progress = None;
        match result {
            Ok(outcome) => {
                session.hits = outcome
                    .hits
                    .into_iter()
                    .map(|h| crate::extensions::search::SearchHit {
                        path: PathBuf::new(),
                        line_no: h.line as u64,
                        line_text: h.text,
                    })
                    .collect();
                session.total_matches = outcome.total_matches;
                session.truncated = outcome.truncated;
                session.current = 0;
                session.error = None;
            }
            // 局部错误:只标在搜索条上,不把整个 preview 置 Failed(T9 bullet 5)。
            Err(error) => {
                session.error = Some(error);
            }
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
    /// 手动点 tab / 打开时切到已存在 tab。**只切 `active`**:webview 是常驻
    /// 池、切走只 `set_visible(false)`、切回只 `set_visible(true)`,页面不重载,
    /// 因此滚动位置/查找高亮等页内状态得以保留(用户口径:markdown 等渲染型
    /// 预览切回 tab 应停在原滚动位置,不该跳回文件顶部)。磁盘最新内容仍由
    /// 外部变化监听(`reload_webviews_for`)与右键"刷新"(`bump_reload`)负责,
    /// 与原生 editor tab"切回不自动重载、保住滚动"同一品鉴口径。点当前已激活
    /// tab 是 no-op。
    pub fn select(&mut self, idx: usize) {
        if idx >= self.tabs.len() || idx == self.active {
            return;
        }
        self.active = idx;
        self.cull_stale_find();
    }

    /// 项目切换清理专用:只留下一个 `Blank` 占位 tab(§对 `Default` 的同一
    /// 不变式)。调用方(`Workspace::close_all_tabs_for_switch`)拿到的这份
    /// pane 要换主人,旧项目的文件 tab 全丢弃,但"空白占位恒在第 0 项"这条
    /// 不变式不因换项目而破——落回空白页,而不是一个没有占位 tab 的空列表。
    pub fn clear_all(&mut self) {
        // T11:清空(项目切换/关闭)即取消所有 tab 的在途后台任务。
        for tab in self.tabs.iter_mut() {
            tab.cancel_background();
        }
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
        // 清空后旧 tab 全没了,"保存后关闭"等待列表随之作废(其对应的
        // `SaveDocument` 命令也已在清空时失去 host,回不来)。
        self.pending_close.clear();
    }

    /// 按 tab id 关闭(等价 `close` 的下标版本)。找不到该 id 是 no-op。
    /// 用于"保存后再关闭"的回调——那时下标可能已因别的关 tab 而漂移,id 稳定。
    pub fn close_by_id(&mut self, tab_id: usize) {
        if let Some(idx) = self.tabs.iter().position(|t| t.id == tab_id) {
            self.close(idx);
        }
    }

    /// 关闭前的保存分流:目标 tab 是**脏的 CodeMirror 编辑器**时,Rust 侧
    /// 不持有全文,不能像老 iced editor 那样就地写盘。改为向 host 下发
    /// `SaveDocument`(host 用当前 buffer 回 `save_requested`,见
    /// `EditorEvent::SaveRequested` 的落盘 + 完成关闭逻辑),把 tab id 记进
    /// `pending_close`,等保存回来再真正移除。返回是否走了这条异步路径
    /// (`true` 时调用方**不要**再 `close`)。非 CodeMirror / 不脏的 tab 返回
    /// `false`,调用方维持原同步关闭路径。
    pub fn request_save_before_close_if_dirty(&mut self, idx: usize) -> bool {
        let Some(tab) = self.tabs.get(idx) else {
            return false;
        };
        if !tab.dirty || !tab.uses_codemirror() {
            return false;
        }
        let tab_id = tab.id;
        self.pending_editor_commands
            .push((tab_id, EditorCommand::SaveDocument));
        if !self.pending_close.contains(&tab_id) {
            self.pending_close.push(tab_id);
        }
        true
    }

    /// 某 tab 是否在"保存后关闭"等待列表里(不消费)。
    pub fn has_pending_close(&self, tab_id: usize) -> bool {
        self.pending_close.contains(&tab_id)
    }

    /// 某 tab 的保存是否回来并需要完成关闭:取走(消费式)则返回 `true`,
    /// 调用方随后 `close_by_id`。`SaveRequested` 落盘后用。
    pub fn take_pending_close(&mut self, tab_id: usize) -> bool {
        if let Some(pos) = self.pending_close.iter().position(|id| *id == tab_id) {
            self.pending_close.remove(pos);
            true
        } else {
            false
        }
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
        // T11:关闭即取消该 tab 在途后台任务(索引/解析/recovery),避免白算。
        self.tabs[idx].cancel_background();
        let removed_id = self.tabs[idx].id;
        self.tabs.remove(idx);
        // 该 tab 若还在"保存后关闭"等待列表里,一并清掉(重复关闭路径兜底)。
        if let Some(pos) = self.pending_close.iter().position(|id| *id == removed_id) {
            self.pending_close.remove(pos);
        }
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
                    // flyfish 与 html host 的 URL 都已带 `?p=...` 查询串;按 URL
                    // 是否已有查询串决定用 `&` 还是 `?` 起头(防御性)。
                    let sep = if u.contains('?') { '&' } else { '?' };
                    u.push_str(&format!("{sep}_r={}", tab.reload_nonce));
                }
                Some(WebviewSpec {
                    id: tab.id,
                    url: u,
                    // T5/T8:非 Ready(Failed 已排除;此处是 Loading/SwitchingMode)
                    // 的 host 预创建但 **不显示**,等 `ready` 才可见,避免露出空白
                    // 原生子视图盖住 iced loading。
                    visible: tab.backend_state.is_ready() && idx == self.active,
                    editor_binding: None,
                    // Rendered(Flyfish/HTML)host 不走资源 reserve,无需回灌。
                    loading_generation: None,
                    // Flyfish/HTML 渲染器(docx 等)的 `load()` 依赖 rAF,hidden
                    // 视图下 rAF 被 WebKit 挂起会死锁(见字段文档);未就绪时用
                    // 离屏停放代替 hidden。已就绪者交给上层 visible 判断。
                    park_offscreen: !tab.backend_state.is_ready(),
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
                // T4/T5:加载中的 editor host 也**预创建为 hidden**,让它能完成
                // boot → 建索引/读正文,再经 ACK(`window_applied`/`document_loaded`)
                // 才 finish 可见。否则 host 永远不 boot → 死锁(等不到 ready)。
                // Terminal 态(Failed/Suspended/Queued)仍不挂载。
                let loading = matches!(tab.backend_state, BackendState::Loading);
                if !tab.backend_state.is_ready() && !loading {
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
                    // JSON 的 Text 模式(feature 下由 editor host 承载;
                    // Tree 视图走 vanilla-jsoneditor host)。
                    PreviewBackend::Json(json) if json.mode == JsonMode::Text => tab.windowed,
                    // 流式 JSONL/NDJSON:Streamed(默认)只读,Text 回退可编辑。
                    PreviewBackend::Streamed(streamed) => {
                        matches!(streamed.mode, StreamedMode::Streamed) || tab.windowed
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
                if tab.route.as_ref().is_some_and(|r| r.encoding_lossy) {
                    // T6:非 UTF-8 有损文本,host 顶部显示只读提示、禁用保存。
                    url.push_str("&lossy=1");
                } else if tab.route.as_ref().is_some_and(|r| r.encoding_utf16) {
                    // T6:UTF-16(已转码只读展示),提示只读原因。
                    url.push_str("&enc=utf16");
                }
                // 外部变更/右键刷新的重载:换 URL 逼 WebView 重新导航拉取最新
                // 内容(与 flyfish 的 `_r=` 同一手法)。窗口化 tab 的重载会重推窗口。
                if tab.reload_nonce > 0 {
                    url.push_str(&format!("&_r={}", tab.reload_nonce));
                }
                Some(WebviewSpec {
                    id: tab.id,
                    url,
                    // T4:窗口化首窗 ACK 前 host 只预创建不显示(避免空编辑器可见);
                    // 其余 ready 且激活者可见。
                    visible: tab.backend_state.is_ready() && idx == self.active,
                    editor_binding: Some(binding),
                    // T10:在途加载时携带世代,reserve 批准后据此推进到
                    // `CreatingHost`;已就绪/非加载态的 host 无需回灌。
                    loading_generation: loading.then_some(tab.load_state.generation),
                    // CodeMirror host 不依赖 rAF 完成 boot(hidden 下可正常
                    // 建索引/读正文),保持原有的「hidden 预创建」语义。
                    park_offscreen: false,
                })
            })
            .collect()
    }

    /// 严格 JSON 的 Tree 视图(对照期 feature)的 json-editor webview 期望清单。
    pub fn desired_json_webviews(
        &self,
        project_id: i64,
        panel: crate::app::PanelKind,
    ) -> Vec<WebviewSpec> {
        if !json_editor_enabled() {
            return Vec::new();
        }
        self.tabs
            .iter()
            .enumerate()
            .filter_map(|(idx, tab)| {
                // T5/T6:加载中的 JSON Tree host 也预创建(hidden),等 `ready`
                // 才 finish 可见;否则 host 永远不 boot → 死锁。
                let loading = matches!(tab.backend_state, BackendState::Loading);
                if !tab.uses_json_editor() || (!tab.backend_state.is_ready() && !loading) {
                    return None;
                }
                let TabKind::File(path) = &tab.kind else {
                    return None;
                };
                let binding = EditorHostBinding::new(project_id, panel, tab.id, path.clone());
                // Tree 视为查看态(编辑走 Text 模式 CodeMirror)。
                Some(WebviewSpec {
                    id: tab.id,
                    url: binding.json_url(scheme_query_value(), true),
                    // 非 Ready 预创建但 hidden。
                    visible: tab.backend_state.is_ready() && idx == self.active,
                    editor_binding: Some(binding),
                    // T10:同 editor host,加载在途时携带世代供 reserve 回灌。
                    loading_generation: loading.then_some(tab.load_state.generation),
                    // JSON Tree host 同为 hidden 预创建 boot,不依赖 rAF。
                    park_offscreen: false,
                })
            })
            .collect()
    }

    /// Tabular Grid 视图(ag-grid webview host)的期望清单。
    pub fn desired_tabular_webviews(
        &self,
        project_id: i64,
        panel: crate::app::PanelKind,
    ) -> Vec<WebviewSpec> {
        if !tabular_grid_host_enabled() {
            return Vec::new();
        }
        self.tabs
            .iter()
            .enumerate()
            .filter_map(|(idx, tab)| {
                // 数据后台解析中(`TabularState::Loading`)也预创建 hidden
                // host,让它先完成 boot、报 `ready`,不必等数据到位——两路
                // 汇合逻辑在 `finish_tabular_load`/`try_push_initial_tabular_state`。
                let loading = matches!(tab.backend_state, BackendState::Loading);
                if !tab.uses_tabular_grid_host() || (!tab.backend_state.is_ready() && !loading) {
                    return None;
                }
                let TabKind::File(path) = &tab.kind else {
                    return None;
                };
                let binding = EditorHostBinding::new(project_id, panel, tab.id, path.clone());
                let mut url = binding.tabular_url(scheme_query_value());
                // 主题切换(`reload_all_webviews_for_theme`)靠 `_r=` 逼
                // webview 重新导航拿到新 `theme=` 参数,同 `desired_editor_
                // webviews` 的既有手法(tabular host 不支持原地换主题)。
                if tab.reload_nonce > 0 {
                    url.push_str(&format!("&_r={}", tab.reload_nonce));
                }
                Some(WebviewSpec {
                    id: tab.id,
                    url,
                    visible: tab.backend_state.is_ready() && idx == self.active,
                    editor_binding: Some(binding),
                    loading_generation: loading.then_some(tab.load_state.generation),
                    park_offscreen: false,
                })
            })
            .collect()
    }

    /// 老 iced editor 已退役:不再有原生标脏入口(CodeMirror 的脏由 web 事件维护)。
    #[cfg(test)]
    pub fn mark_dirty_by_id(&mut self, _tab_id: usize) {}
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
    pub fn open_find_on_active(&mut self, _replace_open: bool, webview_pool_id: Option<usize>) {
        // 老 iced 原生 Find 会话已退役;只剩 flyfish webview 档。
        // 激活 tab 是 flyfish webview(无原生 editor、是文件、未在读盘
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
        if let Some(s) = self.find.as_mut()
            && s.is_webview
        {
            s.query = query;
            s.pending_webview_exec = Some(WebviewFindAction::Search);
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
        if let Some(s) = self.find.as_mut()
            && s.is_webview
        {
            s.pending_webview_exec = Some(if next {
                WebviewFindAction::Next
            } else {
                WebviewFindAction::Prev
            });
        }
    }

    /// 编辑事件后刷新展示量:当锁定 tab 的 buffer 被就地改过(用户在条开着时回到
    /// 编辑器敲字),把 `state.count` 按当下 buffer 重算,`current` 钳到有效范围。
    /// 不移动光标(改动发生在用户聚焦编辑器处,不该被 yank)。供 Workspace 编辑
    /// 事件转发层每收到一个 Edit Action 调用;非锁定 tab/未开条是 no-op。
    pub fn find_refresh_after_edit(&mut self, edited_tab_id: usize) {
        let _ = edited_tab_id;
        // 老 iced 原生 Find 已退役;webview 档无需按 buffer 重算计数。
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
        // 老 iced 原生替换已退役;webview(flyfish)档没有替换概念。
        false
    }

    /// 「替换当前命中」:把 `find_state().current` 指着的那一处替换掉(第 `nth`
    /// 个命中,窗口序与 `find_matches_all` 一致)。动作与 [`PreviewPane::replace_all`]
    /// 相同——只动未保存 buffer、标脏等 ⌘S。替换成功后把光标拨回被删匹配的起点,
    /// 再由 [`PreviewPane::find_go`] 按当下 buffer 往**下一个**命中走(替换者通常要
    /// 一路往下逐个处理;简单同字符替换时光标就卡在被换处以便继续替换)。空 query /
    /// 无命中 / 目标已是文件尾(替换后不再有该 query)都会安全 no-op 返回 `false`。
    pub fn replace_current(&mut self) -> bool {
        // 老 iced 原生 Find/替换已退役;webview(flyfish)档本就没有替换概念。
        false
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
                // webview 档:激活 tab 必须还是同一块 webview。
                t.id == f.tab_id
                    && t.tabular_state().is_none()
                    && matches!(t.kind, TabKind::File(_))
            } else {
                false
            }
        });
        if !keep {
            if let Some(id) = self.find.as_ref().and_then(|f| f.webview_pool_id) {
                self.pending_webview_find_clear = Some(id);
            }
            self.find = None;
        }
        // T9:窗口化搜索会话锁定的 tab 已关闭/换项目时,作废在途查询并丢弃
        // 会话(不给已消失的 tab 继续扫盘)。
        let stale_search = self
            .large_file_search
            .as_ref()
            .is_some_and(|s| !self.tabs.iter().any(|t| t.id == s.tab_id));
        if stale_search {
            self.close_large_file_search();
        }
    }

    /// 按 tab id 取该 tab 的 Tabular Viewer 可变引用。tab 不存在、该 tab 不是
    /// 表格、或表格还在后台加载中(`TabularState::Loading`)都返回 `None`
    /// (切 sheet 这类交互动作在数据到位前没有意义,直接 no-op)。
    /// `Workspace::preview_pane_tabular_action` 用它定位对应 tab 后调用
    /// `TabularView::select_sheet`(Tabular Grid 已迁移到 ag-grid webview
    /// host,交互经 `Message::TabularHostEvent` 而不是老式 iced `Action`)。
    pub fn tabular_mut(&mut self, tab_id: usize) -> Option<&mut crate::tabular::TabularView> {
        self.tabs
            .iter_mut()
            .find(|t| t.id == tab_id)
            .and_then(|t| t.tabular_view_mut())
    }

    /// T12:Agent 导航——reveal 某表格 tab 的单元格。返回 `Some(SheetLoadRequest)`
    /// 表示目标 sheet 未加载,调用方应物化后重试;非表格/未就绪返回 `None`。
    #[allow(dead_code)] // T12/T13:Agent 导航通道接线的 tab 侧原语(暂由测试使用)。
    pub fn reveal_tabular_cell(
        &mut self,
        tab_id: usize,
        sheet: usize,
        row: usize,
        col: usize,
    ) -> Option<crate::tabular::SheetLoadRequest> {
        self.tabs
            .iter_mut()
            .find(|t| t.id == tab_id)
            .and_then(|t| t.tabular_view_mut())
            .and_then(|v| v.reveal_cell(sheet, row, col))
    }

    /// T12:Agent 导航——reveal 某表格 tab 的一个范围(0-based,含端点)。
    #[allow(dead_code)] // T12/T13:Agent 导航通道接线的 tab 侧原语(暂由测试使用)。
    pub fn reveal_tabular_range(
        &mut self,
        tab_id: usize,
        sheet: usize,
        r1: usize,
        c1: usize,
        r2: usize,
        c2: usize,
    ) -> Option<crate::tabular::SheetLoadRequest> {
        self.tabs
            .iter_mut()
            .find(|t| t.id == tab_id)
            .and_then(|t| t.tabular_view_mut())
            .and_then(|v| v.reveal_range(sheet, r1, c1, r2, c2))
    }

    /// T13:在一个 tab 上应用一条 Agent 下行预览命令,返回确定终态。只读导航
    /// (reveal/select/reveal_cell)不受写权限阻塞;replace 仅可写 CodeMirror 且
    /// 必须带匹配的 `expected_revision`。tab 定位由调用方(Workspace)按
    /// panel/path 解析后传入 `tab_id`。
    #[allow(dead_code)] // T13:T13b/T13c 的下行通道接线前,仅测试使用。
    pub fn apply_preview_command(
        &mut self,
        tab_id: usize,
        cmd: &dozer_core::protocol::PreviewCommand,
    ) -> dozer_core::protocol::PreviewCommandOutcome {
        use dozer_core::protocol::{PreviewCommandAction as A, PreviewCommandOutcome as O};
        let rid = cmd.request_id.clone();
        let Some(tab) = self.tabs.iter().find(|t| t.id == tab_id) else {
            return O::NotFound {
                request_id: rid,
                detail: "tab 不存在".into(),
            };
        };
        let is_editor = tab.uses_editor_host();
        let is_tabular = tab.tabular_view().is_some();
        let not_writable = tab.backend_read_only() || !tab.can_save();
        let revision = tab.web_revision;

        match &cmd.action {
            A::Reveal { line, column } => {
                if !is_editor {
                    return O::UnsupportedBackend {
                        request_id: rid,
                        detail: "该 tab 无文本编辑器".into(),
                    };
                }
                self.queue_editor_command(
                    tab_id,
                    EditorCommand::RevealPosition {
                        line: *line,
                        column: *column,
                    },
                );
                O::Accepted { request_id: rid }
            }
            A::Select {
                start_line,
                start_column,
                end_line,
                end_column,
            } => {
                if !is_editor {
                    return O::UnsupportedBackend {
                        request_id: rid,
                        detail: "该 tab 无文本编辑器".into(),
                    };
                }
                self.queue_editor_command(
                    tab_id,
                    EditorCommand::SelectRange {
                        start: TextPosition {
                            line: *start_line,
                            column: *start_column,
                        },
                        end: TextPosition {
                            line: *end_line,
                            column: *end_column,
                        },
                    },
                );
                O::Accepted { request_id: rid }
            }
            A::Highlight {
                start_line,
                start_column,
                end_line,
                end_column,
                duration_ms,
            } => {
                if !is_editor {
                    return O::UnsupportedBackend {
                        request_id: rid,
                        detail: "该 tab 无文本编辑器".into(),
                    };
                }
                self.queue_editor_command(
                    tab_id,
                    EditorCommand::HighlightRange {
                        start: TextPosition {
                            line: *start_line,
                            column: *start_column,
                        },
                        end: TextPosition {
                            line: *end_line,
                            column: *end_column,
                        },
                        duration_ms: *duration_ms,
                    },
                );
                O::Accepted { request_id: rid }
            }
            A::RevealCell { sheet, row, col } => {
                if !is_tabular {
                    return O::UnsupportedBackend {
                        request_id: rid,
                        detail: "该 tab 不是表格".into(),
                    };
                }
                let request =
                    self.reveal_tabular_cell(tab_id, *sheet, *row as usize, *col as usize);
                if request.is_some() {
                    return O::LoadDenied {
                        request_id: rid,
                        detail: "目标 sheet 尚未加载".into(),
                    };
                }
                self.queue_tabular_command(
                    tab_id,
                    TabularCommand::RevealRange {
                        sheet_index: *sheet,
                        r1: *row,
                        c1: *col,
                        r2: *row,
                        c2: *col,
                    },
                );
                O::Accepted { request_id: rid }
            }
            A::Replace {
                start_line,
                start_column,
                end_line,
                end_column,
                text,
            } => {
                if not_writable {
                    return O::UnsupportedBackend {
                        request_id: rid,
                        detail: "只读或不可写 backend".into(),
                    };
                }
                let Some(expected) = cmd.expected_revision else {
                    return O::InternalError {
                        request_id: rid,
                        detail: "replace 必须携带 expected_revision".into(),
                    };
                };
                if expected != revision {
                    return O::StaleRevision {
                        request_id: rid,
                        current_revision: revision,
                    };
                }
                self.queue_editor_command(
                    tab_id,
                    EditorCommand::ReplaceRange {
                        start: TextPosition {
                            line: *start_line,
                            column: *start_column,
                        },
                        end: TextPosition {
                            line: *end_line,
                            column: *end_column,
                        },
                        text: text.clone(),
                        revision: expected,
                    },
                );
                O::Accepted { request_id: rid }
            }
        }
    }

    /// 按 tab id 取出该 tab 的 `TabularState` 可变引用,供加载完成/懒加载
    /// sheet 完成的回填使用(`Message::TabularLoaded`/`TabularSheetLoaded`
    /// 的处理函数)。与 `tabular_mut` 不同,这个不区分 `Loading`/`Ready`——
    /// 回填就是要把 `Loading` 变成 `Ready`。
    pub fn tabular_state_mut(&mut self, tab_id: usize) -> Option<&mut TabularState> {
        self.tabs
            .iter_mut()
            .find(|t| t.id == tab_id)
            .and_then(|t| t.tabular_state_mut())
    }

    /// `generation` 是启动后台解析时捕获的世代;与当前 `load_state` 不匹配
    /// (tab 已关闭重开 / 重试)时丢弃旧结果,不回填(T7 取消不回填旧 sheet)。
    ///
    /// 迁移到 webview host 后不再在数据到位那一刻立即 `Ready`——还要等
    /// host `ready` + 首窗 `window_applied`(两路异步汇合,见
    /// `try_push_initial_tabular_state`)。若 host 已经先 `ready` 过,本函数
    /// 直接把初始状态推过去;若 host 还没 `ready`,由 `Message::TabularHostEvent`
    /// 的 `Ready` 分支在稍后补推。
    pub fn finish_tabular_load(
        &mut self,
        tab_id: usize,
        generation: u64,
        result: Result<crate::tabular::TabularView, String>,
    ) -> Option<(usize, usize, usize)> {
        // 借用 `self.tabs` 的部分收在这个块里,块结束后借用释放,后面才能
        // 再调用 `self.try_push_initial_tabular_state`(它需要 `&mut self`)。
        let accepted = {
            let tab = self.tabs.iter_mut().find(|tab| tab.id == tab_id)?;
            if !tab.load_state.accepts(generation) {
                return None;
            }
            match result {
                Ok(view) => {
                    tab.runtime = PreviewRuntime::Tabular(TabularState::Ready(view));
                    true
                }
                Err(message) => {
                    tab.runtime = PreviewRuntime::None;
                    let _ = tab
                        .backend_state
                        .try_transition(BackendState::Failed(PreviewError::new(message, true)));
                    tab.load_state.finish();
                    false
                }
            }
        };
        if !accepted {
            return None;
        }
        // 应用持久化的 sheet / 滚动锚点;若目标 sheet 不是当前已加载的,
        // 返回它让调用方触发一次懒加载。没有持久化状态(常见:新打开的
        // tab,不是从会话恢复来的)时 `pending_sheet` 是 `None`,但仍要往下
        // 走两路汇合尝试——`?` 只在这个 `and_then` 闭包内部短路,不会跳过
        // 后面的 `try_push_initial_tabular_state`。
        // 返回 `(sheet, row, col)` 而不只是 `sheet`——调用方稍后会经
        // `select_sheet` 触发懒加载,而 `select_sheet` 会把 `scroll_row`/
        // `scroll_col` 重置为 0(正常用户切 sheet 的预期行为)。把持久化的
        // row/col 一并交给调用方,让它在 `select_sheet` 重置之后重新应用
        // 一次,否则恢复到非首个 sheet 的滚动位置会被静默清零。
        let pending_sheet = self.tabs.iter_mut().find(|tab| tab.id == tab_id).and_then(
            |tab| -> Option<(usize, usize, usize)> {
                let (sheet, row, col) = tab.pending_tabular.take()?;
                let active_sheet = tab.tabular_view_mut().map(|view| {
                    view.scroll_row = row;
                    view.scroll_col = col;
                    view.active_sheet
                })?;
                (active_sheet != sheet).then_some((sheet, row, col))
            },
        );
        // 两路异步汇合:数据已加载(刚发生在上面)+ host 是否已先 `ready`
        // 过(`tabular_host_ready`)。若 host 已就绪,这里立即推初始状态;
        // 否则等 `Message::TabularHostEvent::Ready` 到达时再推
        // (见 `try_push_initial_tabular_state` 文档)。
        self.try_push_initial_tabular_state(tab_id);
        pending_sheet
    }

    /// 两路异步汇合:数据已加载(`runtime` 是 `TabularState::Ready`)且 host
    /// 已 `ready`(`tabular_host_ready`)都为真时,组好 `Init`+`SetSchema`+
    /// `SetWindow`(首 200 行,与 JS 侧 `cacheBlockSize` 对齐)三条命令入队。
    /// 两个条件哪个先满足都可能发生(小文件解析可能比 webview boot 快,
    /// 反之亦然),调用方是 `finish_tabular_load`(数据到位那一刻)与
    /// `Message::TabularHostEvent::Ready` 处理(host 到位那一刻)各调一次,
    /// 只有真正"两个都满足"的那一次会实际入队命令。用 `pub` 而非
    /// `pub(crate)`——同文件里 `queue_editor_command`/`take_pending_editor_
    /// commands_for` 等跨文件调用的兄弟方法都是 `pub fn`,保持一致。
    pub fn try_push_initial_tabular_state(&mut self, tab_id: usize) {
        let Some((sheet_names, active_sheet, ready)) =
            self.tabs.iter().find(|t| t.id == tab_id).and_then(|tab| {
                if !tab.tabular_host_ready {
                    return None;
                }
                let view = tab.tabular_view()?;
                Some((
                    view.sheet_names.clone(),
                    view.active_sheet,
                    view.active_sheet().is_some(),
                ))
            })
        else {
            return;
        };
        if !ready {
            // sheet 0 理论上在打开文件时已同步预加载(见 `tabular::load` 文档
            // "多 sheet 的 xlsx 只在打开时预加载第一个 sheet"),这个分支正常
            // 不会命中,防御性保留(数据尚未就绪时不发半成品 Init)。
            return;
        }
        self.queue_tabular_command(
            tab_id,
            TabularCommand::Init {
                sheet_names,
                active_sheet,
                read_only: true,
            },
        );
        self.push_sheet_schema_and_window(tab_id, active_sheet);
    }

    /// 把"某个 sheet 当前已加载"的 `SetSchema`+`SetWindow`(首 200 行)命令
    /// 入队。供两处共用:`try_push_initial_tabular_state`(首次打开,额外带
    /// `Init`)与 `Message::TabularSheetLoaded` 处理(懒加载完某个 sheet 后,
    /// 见 Task 11)。`sheet_index` 若已不是当前活动 sheet(用户在懒加载完成
    /// 前又切到别处)则 no-op——不推一份不会被显示的窗口。
    pub fn push_sheet_schema_and_window(&mut self, tab_id: usize, sheet_index: usize) {
        const INITIAL_WINDOW_ROWS: usize = 200;
        let Some((col_count, total_rows, truncated, col_widths, window)) = self
            .tabs
            .iter()
            .find(|t| t.id == tab_id)
            .and_then(|tab| tab.tabular_view())
            .and_then(|view| {
                if view.active_sheet != sheet_index {
                    return None;
                }
                let sheet = view.active_sheet()?;
                Some((
                    sheet.col_count,
                    sheet.total_rows,
                    sheet.truncated,
                    sheet.col_widths.clone(),
                    sheet
                        .rows
                        .iter()
                        .take(INITIAL_WINDOW_ROWS)
                        .cloned()
                        .collect::<Vec<Vec<String>>>(),
                ))
            })
        else {
            return;
        };
        self.queue_tabular_command(
            tab_id,
            TabularCommand::SetSchema {
                sheet_index,
                col_count,
                total_rows,
                truncated,
                col_widths,
            },
        );
        self.queue_tabular_command(
            tab_id,
            TabularCommand::SetWindow {
                sheet_index,
                start_row: 0,
                rows: window,
                revision: 0,
            },
        );
    }

    /// 记录启动恢复的表格视图状态,加载完成后应用一次。
    pub fn set_pending_tabular(
        &mut self,
        tab_id: usize,
        sheet: usize,
        scroll_row: usize,
        scroll_col: usize,
    ) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
            tab.pending_tabular = Some((sheet, scroll_row, scroll_col));
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

    /// 排队一个待下发给 Tabular webview host 的命令(`tab_id`, 命令)。
    pub fn queue_tabular_command(&mut self, tab_id: usize, command: TabularCommand) {
        self.pending_tabular_commands.push((tab_id, command));
    }

    /// 取走(消费式)待下发的 tabular 命令队列。测试专用(同
    /// `take_pending_editor_commands` 的既有先例),生产代码走
    /// `take_pending_tabular_commands_for`(按可用 webview id 过滤)。
    #[cfg(test)]
    pub fn take_pending_tabular_commands(&mut self) -> Vec<(usize, TabularCommand)> {
        std::mem::take(&mut self.pending_tabular_commands)
    }

    /// 只取当前已有 WebView 句柄对应的命令;其余保留待下一帧重试(同
    /// `take_pending_editor_commands_for`)。
    pub fn take_pending_tabular_commands_for(
        &mut self,
        available_webview_ids: &std::collections::HashSet<usize>,
        project_id: i64,
        panel: crate::app::PanelKind,
    ) -> Vec<(usize, TabularCommand)> {
        let pending = std::mem::take(&mut self.pending_tabular_commands);
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
                self.pending_tabular_commands.push((tab_id, command));
            }
        }
        ready
    }

    /// 切换/恢复 JSON tab 的 mode(Tree ⇄ Text)。壳恢复已由 `push_shell_tab`
    /// 直接落到 backend 上;运行期切 Tree/Text 走本方法(由 tab 最右侧的
    /// "树/文本"切换按钮触发)。
    ///
    /// T6:发生 mode 变更时走 `SwitchingMode` + `Loading` 并推进 generation——
    /// 目标 host 的首帧(`document_loaded`:Tree→JSON host,Text→editor host)
    /// 确认前不 finish,旧 host 的迟到结果按 generation 丢弃。mode 未变
    /// (或 route 不支持该 mode)返回 `None`。
    ///
    /// 返回 `Some((tab_id, generation))` 供上层 arm [`PreviewLoadStage::SwitchingMode`]
    /// 看门狗。
    pub fn restore_json_mode(&mut self, tab_id: usize, mode: PreviewMode) -> Option<(usize, u64)> {
        let tab = self.tabs.iter_mut().find(|tab| tab.id == tab_id)?;
        let route = tab.route.as_ref()?;
        if !route.supports(mode) {
            return None;
        }
        let target = match mode {
            PreviewMode::Text => JsonMode::Text,
            _ => JsonMode::Tree,
        };
        let changed =
            matches!(&tab.backend, Some(PreviewBackend::Json(json)) if json.mode != target);
        if !changed {
            return None;
        }
        if let Some(PreviewBackend::Json(json)) = tab.backend.as_mut() {
            json.mode = target;
        }
        let generation = tab.load_state.generation.wrapping_add(1);
        tab.load_state = PreviewLoadState::starting(generation, PreviewLoadStage::SwitchingMode);
        let _ = tab.backend_state.try_transition(BackendState::Loading);
        tab.web_revision = 0;
        tab.debug_assert_backend_consistent();
        Some((tab.id, generation))
    }

    /// 编辑保存后调用:按 `PreviewTab.id` 找到对应 tab,推进 reload。原生
    /// (有 `editor`)tab 直接读盘重建编辑器实例(`bump_reload` 路径),wry
    /// tab 走 `reload_nonce` 计数(驱动 `desired_webviews()` 换 URL)。未知
    /// id 是 no-op(tab 可能已被关闭)。
    pub fn bump_reload(&mut self, tab_id: usize) {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) else {
            return;
        };
        // T11:重载前取消在途后台任务。Tabular Grid 迁移到 webview host 后
        // 也需要参与这条路径(主题切换/外部文件变更都要能逼它重新导航),
        // 不再像老 iced canvas 时代那样整体跳过。
        tab.cancel_background();
        tab.reload_nonce += 1;
        if tab.uses_codemirror() {
            // 新 WebView 内部 revision 从 1 重新开始；清掉 Rust 镜像，
            // 让下一条 ready/selection 事件不会被旧 revision 拒绝。
            tab.web_revision = 0;
            tab.web_selection = None;
            tab.web_selected_text = None;
            tab.web_viewport = None;
        }
        if tab.uses_tabular_grid_host() {
            // 新 webview 会重新走一遍 boot,旧的 `tabular_host_ready` 不再
            // 有效——数据仍在 Rust 内存(不重新解析),host 报新 `ready` 后
            // 两路汇合逻辑(`try_push_initial_tabular_state`)会自动补推
            // `Init`+`SetSchema`+`SetWindow`。
            tab.tabular_host_ready = false;
        }
    }

    /// 预览→代码:按下标读盘建一个可写 `CodeView` 挂到该 tab 上(只应对
    /// `wry_toggle_eligible` 的文件 tab 调用,按钮只在这类 tab 上画)。下标
    /// 越界或该 tab 不是 `TabKind::File` 是 no-op;读盘失败把 `io::Error`
    /// 透传给调用方(`Workspace::preview_pane_toggle_render_mode`)写面板
    /// error,这里不生成错误文案。
    pub fn enter_code_mode(&mut self, idx: usize) -> std::io::Result<()> {
        // Markdown/HTML 的 Source 模式由 CodeMirror editor host 承载,翻转
        // backend mode(老 iced CodeView 已退役)。
        //
        // T5:模式切换走 `SwitchingMode` 阶段并保持 Loading —— 目标 host
        // (editor)的首帧(`document_loaded`)确认前不 finish,避免先露出空编辑器。
        // 旧 viewer 在切换瞬间被替换为 loading 动画(允许"显示 loading")。
        if let Some(tab) = self.tabs.get_mut(idx) {
            if let Some(PreviewBackend::Rendered(rendered)) = tab.backend.as_mut() {
                rendered.mode = RenderedMode::Source;
            }
            let generation = tab.load_state.generation.wrapping_add(1);
            tab.load_state =
                PreviewLoadState::starting(generation, PreviewLoadStage::SwitchingMode);
            let _ = tab.backend_state.try_transition(BackendState::Loading);
            tab.web_revision = 0;
            tab.debug_assert_backend_consistent();
        }
        Ok(())
    }

    /// 代码→预览:转回 wry/flyfish 渲染。下标越界是 no-op。
    ///
    /// T5:同样走 `SwitchingMode` + Loading,等 Flyfish host 的 `ready` 才 finish。
    pub fn exit_code_mode(&mut self, idx: usize) {
        if let Some(tab) = self.tabs.get_mut(idx) {
            if let Some(PreviewBackend::Rendered(rendered)) = tab.backend.as_mut() {
                rendered.mode = RenderedMode::Rendered;
            }
            let generation = tab.load_state.generation.wrapping_add(1);
            tab.load_state =
                PreviewLoadState::starting(generation, PreviewLoadStage::SwitchingMode);
            let _ = tab.backend_state.try_transition(BackendState::Loading);
            tab.web_revision = 0;
            tab.debug_assert_backend_consistent();
        }
    }

    /// T1 fallback 页的"以纯文本只读尝试":把该 tab 的 backend 强制改成只读
    /// Code(editor host 承载),清窗口化/错误态并置 `Ready`。下标越界或不是
    /// 文件 tab 返回 false。
    ///
    /// route 保持原样(它记录"为什么落到这里"),只覆盖 backend——这是用户
    /// 显式选择的退路,有意让 route.kind 与 backend.kind 分叉。
    pub fn force_plain_text(&mut self, tab_id: usize) -> bool {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) else {
            return false;
        };
        let TabKind::File(path) = &tab.kind else {
            return false;
        };
        let language = crate::preview::native_editor::extension_to_syntax(path);
        tab.backend = Some(PreviewBackend::Code(CodeBackend {
            mode: CodeMode::ReadOnly,
            language,
        }));
        tab.web_error = None;
        tab.web_revision = 0;
        let _ = tab.backend_state.try_transition(BackendState::Loading);
        let _ = tab.backend_state.try_transition(BackendState::Ready);
        true
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
        // CodeMirror tab:clean 自动重载;**脏** tab 不自动重载,否则会覆盖用户
        // 未保存的改动——改为置冲突状态,由用户决定刷新/另存。
        //
        // T5:clean 重载走 in-place `ReloadDocument`(host 重新 fetch 并替换 doc),
        // 不再推进 `reload_nonce` 换 URL 重新导航——旧正文一直可见到新内容挂上,
        // 不闪空白。仍推进一个 generation 作废在途加载结果。
        let mut reload_ids: Vec<usize> = Vec::new();
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
                // T10:进入显式冲突态(不只是写一条错误),由用户选择保留/重载。
                let _ = tab.mark_disk_conflict();
                tab.web_error =
                    Some("磁盘文件已被外部修改,请选择「保留我的修改」或「重载磁盘」。".into());
            } else {
                tab.web_revision = 0;
                tab.web_selection = None;
                tab.web_selected_text = None;
                tab.web_viewport = None;
                tab.web_error = None;
                reload_ids.push(tab.id);
            }
        }
        for tab_id in reload_ids {
            self.queue_editor_command(tab_id, EditorCommand::ReloadDocument);
        }
        // 窗口化 tab:`uses_codemirror()` 为 false(窗口化排除),单独处理。
        // 文件变了就**失效旧索引**(清 runtime 索引 + `web_revision` 归零,
        // 让在途/旧的 `PreviewWindowIndex` 结果因 revision 不符被拒),并推进
        // `reload_nonce` 让窗口重新导航。旧索引绝不能套到新内容上(T14)。
        for tab in self.tabs.iter_mut() {
            if !tab.uses_windowed_editor() {
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
            if let Some(rt) = tab.window_runtime_mut() {
                rt.index = None;
                rt.window_start_line = 1;
                rt.truncated = false;
                rt.error = None;
            }
            // T11:外部变更 → 取消在途索引构建,重载后再起新任务。
            tab.cancel_background();
            tab.web_revision = 0;
            tab.reload_nonce += 1;
            tab.web_error = None;
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
    /// 逼 `desired_webviews()` 换 URL(新 URL 带新的 `&theme=`/`&_r=` 参数)
    /// 重新导航,flyfish/CodeMirror/vanilla-jsoneditor/Tabular ag-grid 据此
    /// 切到新主题。原生编辑器 tab 不受影响(它是 iced 原生渲染、每帧读
    /// `byteui::theme::color::current()`,切主题自然跟随);`Blank` 占位 tab
    /// 没有 wry 页面,同样跳过。Tabular 重新导航会丢失 JS 侧已挂载的 ag-grid
    /// 实例状态,但数据仍在 Rust 内存(`TabularView` 未清空),host 重新
    /// `ready` 后两路汇合逻辑会自动补推 `Init`+`SetSchema`+`SetWindow`。
    pub fn reload_all_webviews_for_theme(&mut self) {
        let ids: Vec<usize> = self
            .tabs
            .iter()
            .filter(|t| t.hosts_webview() || t.uses_codemirror() || t.uses_tabular_grid_host())
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

    fn outcome(hits: Vec<crate::preview::SearchHit>, total: u64, truncated: bool) -> SearchOutcome {
        SearchOutcome {
            hits,
            total_matches: total,
            truncated,
            lines_scanned: total.max(1),
        }
    }

    #[test]
    fn large_file_search_results_fill_hits_and_reset_current() {
        let mut p = PreviewPane::default();
        let id = p.push_tab(
            TabKind::File(std::path::PathBuf::from("/tmp/x.log")),
            "x.log".into(),
        );
        p.open_large_file_search(id);
        let (generation, _cancel) = p.start_large_file_search(id, "needle".into()).unwrap();
        let hits = vec![crate::preview::SearchHit {
            line: 42,
            column: 3,
            text: "needle here".into(),
        }];
        p.set_large_file_search_results(id, generation, Ok(outcome(hits, 7, true)));
        let s = p.large_file_search.as_ref().unwrap();
        assert_eq!(s.hits.len(), 1);
        assert_eq!(s.hits[0].line_no, 42);
        assert_eq!(s.total_matches, 7);
        assert!(s.truncated);
        assert_eq!(s.current, 0);
        assert!(!s.running);
        assert!(s.error.is_none());
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
        let (generation, _) = p.start_large_file_search(id, "needle".into()).unwrap();
        p.close_large_file_search();
        p.set_large_file_search_results(id, generation, Ok(outcome(vec![], 0, false))); // 不应 panic,也不应重新打开条
        assert!(p.large_file_search.is_none());
    }

    #[test]
    fn large_file_search_stale_generation_is_dropped() {
        // 快速连续输入:旧查询的结果在 generation 已前进后到达,必须丢弃,
        // 不能覆盖新查询。
        let mut p = PreviewPane::default();
        let id = p.push_tab(
            TabKind::File(std::path::PathBuf::from("/tmp/x.log")),
            "x.log".into(),
        );
        p.open_large_file_search(id);
        let (old_gen, _) = p.start_large_file_search(id, "old".into()).unwrap();
        let (new_gen, _) = p.start_large_file_search(id, "new".into()).unwrap();
        assert_ne!(old_gen, new_gen);
        let stale = vec![crate::preview::SearchHit {
            line: 1,
            column: 1,
            text: "old hit".into(),
        }];
        p.set_large_file_search_results(id, old_gen, Ok(outcome(stale, 1, false)));
        let s = p.large_file_search.as_ref().unwrap();
        assert!(s.hits.is_empty(), "过期 generation 的结果必须被丢弃");
        assert!(s.running, "旧结果不应结束新查询的 running");
        p.set_large_file_search_results(id, new_gen, Ok(outcome(vec![], 2, false)));
        assert!(!p.large_file_search.as_ref().unwrap().running);
    }

    #[test]
    fn large_file_search_new_query_cancels_previous_flag() {
        let mut p = PreviewPane::default();
        let id = p.push_tab(
            TabKind::File(std::path::PathBuf::from("/tmp/x.log")),
            "x.log".into(),
        );
        p.open_large_file_search(id);
        let (_, first_cancel) = p.start_large_file_search(id, "a".into()).unwrap();
        assert!(!first_cancel.load(std::sync::atomic::Ordering::Relaxed));
        let (_, second_cancel) = p.start_large_file_search(id, "b".into()).unwrap();
        assert!(
            first_cancel.load(std::sync::atomic::Ordering::Relaxed),
            "新查询必须置位旧查询的取消信号"
        );
        assert!(!second_cancel.load(std::sync::atomic::Ordering::Relaxed));
    }

    #[test]
    fn large_file_search_empty_query_clears_loading() {
        let mut p = PreviewPane::default();
        let id = p.push_tab(
            TabKind::File(std::path::PathBuf::from("/tmp/x.log")),
            "x.log".into(),
        );
        p.open_large_file_search(id);
        let (_, cancel) = p.start_large_file_search(id, "a".into()).unwrap();
        assert!(p.clear_large_file_search(id));
        let s = p.large_file_search.as_ref().unwrap();
        assert!(!s.running);
        assert!(s.hits.is_empty());
        assert!(cancel.load(std::sync::atomic::Ordering::Relaxed));
    }

    #[test]
    fn large_file_search_failure_is_local_not_failed_state() {
        let mut p = PreviewPane::default();
        let id = p.push_tab(
            TabKind::File(std::path::PathBuf::from("/tmp/x.log")),
            "x.log".into(),
        );
        p.open_large_file_search(id);
        let (generation, _) = p.start_large_file_search(id, "a".into()).unwrap();
        p.set_large_file_search_results(id, generation, Err("读盘失败".into()));
        let s = p.large_file_search.as_ref().unwrap();
        assert_eq!(s.error.as_deref(), Some("读盘失败"));
        assert!(!s.running);
    }

    #[cfg(any())] // 老 iced editor 已退役,历史测试停用
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

    #[cfg(any())] // 老 iced editor 已退役,历史测试停用
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
        assert!(!p.tabs()[1].uses_editor_host());
        assert!(
            !p.take_pending_editor_focus(),
            ".png 走 wry,没有原生 editor,不该置聚焦位"
        );
    }

    #[cfg(any())] // 老 iced editor 已退役,历史测试停用
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

    #[cfg(any())] // 老 iced editor 已退役,历史测试停用
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

        assert!(p.tabs()[1].uses_editor_host(), ".rs 扩展名应走 editor host");
        assert!(
            !p.tabs()[2].uses_editor_host(),
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

    #[cfg(any())] // 老 iced editor 已退役,历史测试停用
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
            !p.tabs()[1].uses_editor_host(),
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

    /// T8:JSONL 默认走 Streamed backend(editor host,只读),持久化 Text 回退
    /// 可编辑。
    #[test]
    fn streamed_jsonl_uses_editor_host_and_text_fallback() {
        let path = std::env::temp_dir().join(format!("t8_rows_{}.jsonl", std::process::id()));
        std::fs::write(&path, "{\"a\":1}\n{\"a\":2}\n").unwrap();
        let mut pane = PreviewPane::default();
        let id = pane.open_path(path.clone());
        let tab = pane.tabs().iter().find(|t| t.id == id).unwrap();
        assert!(matches!(tab.backend, Some(PreviewBackend::Streamed(_))));
        assert_eq!(tab.current_mode(), Some(PreviewMode::Streamed));
        assert!(tab.uses_editor_host(), "Streamed 由 editor host 承载");
        assert!(!tab.hosts_webview());
        assert!(tab.backend_read_only(), "Streamed 默认只读");
        let spec = pane
            .desired_editor_webviews(1, crate::app::PanelKind::Files)
            .into_iter()
            .find(|s| s.id == id)
            .expect("Streamed 应产出 editor spec");
        assert!(spec.url.contains("lang=json"));
        assert!(spec.url.contains("ro=1"));

        // 持久化 Text 回退:可编辑。
        let idx = pane.tabs().iter().position(|t| t.id == id).unwrap();
        if let Some(PreviewBackend::Streamed(s)) = pane.tabs_mut()[idx].backend.as_mut() {
            s.mode = StreamedMode::Text;
        }
        let tab = pane.tabs().iter().find(|t| t.id == id).unwrap();
        assert_eq!(tab.current_mode(), Some(PreviewMode::Text));
        assert!(!tab.backend_read_only(), "Text 回退可编辑");
        std::fs::remove_file(&path).ok();
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
            !p.tabs()[1].uses_editor_host(),
            ".html 默认预览应走 wry 渲染,不建原生只读 editor"
        );
        assert!(
            is_editable_extension(&html_path),
            ".html 仍应保留可编辑属性,右键“编辑”入口不受影响"
        );
        let specs = p.desired_webviews();
        assert_eq!(specs.len(), 1, ".html 现在应进 wry 期望清单");
        assert!(
            specs[0].url.starts_with("dozer://html/host.html?"),
            ".html 应走隔离 host(不再直接 file://),got {}",
            specs[0].url
        );

        std::fs::remove_file(&html_path).ok();
    }

    #[test]
    fn preview_url_dispatches_html_to_isolated_host_and_others_to_flyfish() {
        let html = preview_url(std::path::Path::new("/tmp/page.html"));
        assert!(html.starts_with("dozer://html/host.html?"), "got {html}");
        assert!(html.contains("p=%2Ftmp%2Fpage.html"), "got {html}");
        let htm = preview_url(std::path::Path::new("/tmp/page.HTM"));
        assert!(htm.starts_with("dozer://html/host.html?"), "大小写不敏感");
        assert_eq!(
            preview_url(std::path::Path::new("/tmp/notes.md")),
            flyfish_url(std::path::Path::new("/tmp/notes.md")),
            "非 html/htm 扩展名不变,仍走 flyfish"
        );
    }

    #[test]
    fn preview_url_dispatches_annotate_exts_and_keeps_gif_tif_on_flyfish() {
        for p in [
            "/tmp/a.png",
            "/tmp/b.JPG",
            "/tmp/c.jpeg",
            "/tmp/d.webp",
            "/tmp/e.bmp",
            "/tmp/f.ico",
        ] {
            let u = preview_url(std::path::Path::new(p));
            assert!(
                u.starts_with("dozer://image-annotate/host.html?"),
                "{p} → {u}"
            );
        }
        // gif/tif/tiff 明确不迁移:canvas 丢动画 / OSD 无 TIFF 解码。
        for p in ["/tmp/a.gif", "/tmp/b.tif", "/tmp/c.tiff"] {
            assert_eq!(
                preview_url(std::path::Path::new(p)),
                flyfish_url(std::path::Path::new(p)),
                "{p} 必须仍走 flyfish"
            );
        }
    }

    #[test]
    fn flyfish_binding_from_url_parses_query_and_rejects_non_flyfish() {
        let url =
            "dozer://flyfish/host.html?p=%2Fa.md&theme=dark&proj=7&panel=project&tab=3&doc=p7-t3";
        assert_eq!(
            flyfish_binding_from_url(url),
            Some(HostBinding::new(
                7,
                crate::app::PanelKind::Project,
                3,
                "p7-t3".into()
            ))
        );
        // 缺绑定字段 → None。
        assert!(flyfish_binding_from_url("dozer://flyfish/host.html?p=x").is_none());
        // 非 flyfish URL → None。
        assert!(flyfish_binding_from_url("dozer://html/host.html?proj=1").is_none());
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
        // 绝不能因语法判据脱节被误塞进原生文本编辑器。`.svg` 不在此列:它虽被
        // 当作 XML 源码可编辑(T5),但默认仍由 `prefers_rendered_preview` 拉去
        // 图像渲染,不会默认进文本编辑器(见下一测试)。
        for ext in [
            "png", "jpg", "jpeg", "gif", "webp", "avif", "pdf", "zip", "mp4",
        ] {
            assert!(
                !is_editable_extension(std::path::Path::new(&format!("/tmp/a.{ext}"))),
                ".{ext} 是二进制/媒体,不该进原生文本编辑器"
            );
        }
    }

    #[test]
    fn markdown_and_html_still_editable_but_default_preview_is_rendered() {
        // is_editable_extension 变宽(语法判据)后,md/html/svg 必须仍被
        // prefers_rendered_preview 拉去渲染预览而不是落到原生,避免回归。
        for ext in ["md", "html", "svg"] {
            let path = format!("/tmp/a.{ext}");
            let p = std::path::Path::new(&path);
            assert!(is_editable_extension(p), ".{ext} 有源码语法");
            assert!(prefers_rendered_preview(p), ".{ext} 默认渲染");
        }
    }

    #[test]
    fn bump_reload_appends_reload_param_to_html_host_url() {
        let mut p = PreviewPane::default();
        let id0 = p.open_path(PathBuf::from("/tmp/a.html"));
        p.bump_reload(id0);
        let url = &p.desired_webviews()[0].url;
        assert!(url.starts_with("dozer://html/host.html?"), "got {url}");
        assert!(url.ends_with("&_r=1"), "已有查询串,重载参数用 & : {url}");
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
            matches!(tab.runtime, PreviewRuntime::Tabular(TabularState::Loading)),
            "csv tab 应先进入 Loading 态,而不是同步构造好 TabularView"
        );
        assert!(!tab.uses_editor_host(), "csv 不应再进文本编辑器");
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
        let generation = pane.load_generation(id);
        pane.finish_tabular_load(id, generation, Ok(loaded));
        assert!(
            pane.tabular_mut(id).is_some(),
            "数据到位后 tabular_mut 应能拿到可变引用(即便还没 Ready)"
        );
        // 数据到位了,但 host 还没报 `ready`——两路汇合尚未完成,仍是
        // Loading,不应有任何命令被推给还不存在的 webview。
        assert!(matches!(
            pane.tabs()[pane.active_idx()].backend_state,
            BackendState::Loading
        ));
        assert!(pane.take_pending_tabular_commands().is_empty());
        std::fs::remove_file(p).ok();
    }

    /// 数据先于 host 就绪(小文件解析比 webview boot 快的常见情形)。
    #[test]
    fn initial_tabular_state_pushes_once_when_data_ready_first() {
        let p =
            std::env::temp_dir().join(format!("tabular_rendezvous_a_{}.csv", std::process::id()));
        std::fs::write(&p, "a,b\n1,2\n").unwrap();
        let mut pane = PreviewPane::default();
        let id = pane.open_path(p.clone());
        let generation = pane.load_generation(id);
        let loaded = crate::tabular::load(&p).expect("测试用 csv 应能正常解析");
        pane.finish_tabular_load(id, generation, Ok(loaded));
        assert!(
            pane.take_pending_tabular_commands().is_empty(),
            "host 还没 ready,不该推任何命令"
        );
        pane.tabs_mut()
            .iter_mut()
            .find(|t| t.id == id)
            .unwrap()
            .tabular_host_ready = true;
        pane.try_push_initial_tabular_state(id);
        let cmds = pane.take_pending_tabular_commands();
        assert_eq!(cmds.len(), 3, "应恰好推 Init+SetSchema+SetWindow 三条");
        assert!(matches!(cmds[0].1, TabularCommand::Init { .. }));
        assert!(matches!(cmds[1].1, TabularCommand::SetSchema { .. }));
        assert!(matches!(cmds[2].1, TabularCommand::SetWindow { .. }));
        std::fs::remove_file(p).ok();
    }

    /// host 先于数据就绪(webview boot 比后台解析快的情形)。
    #[test]
    fn initial_tabular_state_pushes_once_when_host_ready_first() {
        let p =
            std::env::temp_dir().join(format!("tabular_rendezvous_b_{}.csv", std::process::id()));
        std::fs::write(&p, "a,b\n1,2\n").unwrap();
        let mut pane = PreviewPane::default();
        let id = pane.open_path(p.clone());
        pane.tabs_mut()
            .iter_mut()
            .find(|t| t.id == id)
            .unwrap()
            .tabular_host_ready = true;
        pane.try_push_initial_tabular_state(id);
        assert!(
            pane.take_pending_tabular_commands().is_empty(),
            "数据还没到位,不该推任何命令"
        );
        let generation = pane.load_generation(id);
        let loaded = crate::tabular::load(&p).expect("测试用 csv 应能正常解析");
        pane.finish_tabular_load(id, generation, Ok(loaded));
        let cmds = pane.take_pending_tabular_commands();
        assert_eq!(cmds.len(), 3, "应恰好推 Init+SetSchema+SetWindow 三条");
        std::fs::remove_file(p).ok();
    }

    /// 回归测试:会话恢复到非首个 sheet 时,`finish_tabular_load` 必须把
    /// 持久化的 (sheet, row, col) 完整交还给调用方——调用方随后会经
    /// `select_sheet` 触发懒加载,而 `select_sheet` 会把 scroll 重置为 0
    /// (正常用户切 sheet 的预期行为),所以 row/col 不能只在这里应用一次
    /// 就再也拿不回来,否则恢复的滚动位置会被静默清零(见
    /// `Message::TabularLoaded` 处理:重新应用一次这里返回的 row/col)。
    #[test]
    fn finish_tabular_load_returns_pending_scroll_for_non_default_sheet_restore() {
        let p =
            std::env::temp_dir().join(format!("tabular_restore_scroll_{}.csv", std::process::id()));
        std::fs::write(&p, "a,b\n1,2\n").unwrap();
        let mut pane = PreviewPane::default();
        let id = pane.open_path(p.clone());
        pane.set_pending_tabular(id, 1, 500, 3);
        let sheet0 = crate::tabular::Sheet {
            rows: vec![vec!["a".into(), "b".into()]],
            col_count: 2,
            total_rows: 1,
            truncated: false,
            col_widths: vec![8.0, 8.0],
        };
        let view = crate::tabular::TabularView::new(
            p.clone(),
            vec!["Sheet1".into(), "Sheet2".into()],
            vec![Some(sheet0), None],
        );
        let generation = pane.load_generation(id);
        let restore = pane.finish_tabular_load(id, generation, Ok(view));
        assert_eq!(
            restore,
            Some((1, 500, 3)),
            "目标 sheet 与持久化 row/col 必须一并交还,不能只剩 sheet 下标"
        );
        std::fs::remove_file(p).ok();
    }

    /// T7 bullet 4:后台解析结果携带的 generation 与启动时不一致(期间 tab
    /// 被重开/重试,世代已推进)时必须丢弃,不得把旧 sheet 回填进新世代的 tab。
    #[test]
    fn stale_generation_tabular_load_result_is_dropped() {
        let p = std::env::temp_dir().join(format!("tabular_stale_{}.csv", std::process::id()));
        std::fs::write(&p, "a,b\n1,2\n").unwrap();
        let mut pane = PreviewPane::default();
        let id = pane.open_path(p.clone());
        let stale_generation = pane.load_generation(id);
        // 模拟重试:世代推进,旧后台任务随后才回来。
        let _ = pane.begin_load(id, crate::preview::PreviewLoadStage::Parsing);
        let loaded = crate::tabular::load(&p).expect("测试用 csv 应能正常解析");
        assert!(
            pane.finish_tabular_load(id, stale_generation, Ok(loaded))
                .is_none(),
            "过期世代的结果不得回填"
        );
        let still_loading = pane.tabs().iter().find(|t| t.id == id).is_some_and(|t| {
            matches!(
                t.tabular_state(),
                Some(crate::preview::TabularState::Loading)
            )
        });
        assert!(still_loading, "tab 应仍在加载新世代,不被旧结果置为 Ready");
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
            "dozer://flyfish/host.html?p=%2Ftmp%2Fa%20b.md&theme=dark&ln=1&fs=14"
        );
        assert!(!specs[0].visible, "非激活 tab 不可见");
        assert_eq!(
            specs[1].url,
            "dozer://flyfish/host.html?p=%2Ftmp%2Fc.md&theme=dark&ln=1&fs=14"
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
            "dozer://flyfish/host.html?p=%2Ftmp%2Fa.md&theme=dark&ln=1&fs=14&_r=1"
        );
        assert_eq!(
            specs[1].url,
            "dozer://flyfish/host.html?p=%2Ftmp%2Fb.md&theme=dark&ln=1&fs=14"
        );
        p.bump_reload(id0);
        assert_eq!(
            p.desired_webviews()[0].url,
            "dozer://flyfish/host.html?p=%2Ftmp%2Fa.md&theme=dark&ln=1&fs=14&_r=2"
        );
        // 未知 id 是 no-op,不 panic。
        p.bump_reload(9999);
    }

    // feature 开启时 `.rs` tab 走 CodeMirror,外部变化会推进其 reload_nonce
    // (见 `external_change_reloads_clean_...`),这条 iced-原生语义的断言不再成立。
    #[cfg(any())] // 老 iced editor 已退役,历史测试停用
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
            specs[1].url, "dozer://flyfish/host.html?p=%2Ftmp%2Fb.md&theme=dark&ln=1&fs=14&_r=1",
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
    #[cfg(any())] // 老 iced editor 已退役,历史测试停用
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

    /// 回归测试:Tabular Grid 迁移到 webview host 后,主题切换必须能推进
    /// 它的 reload_nonce(否则切主题时表格网格会停留在旧配色,见 code
    /// review 发现)。`bump_reload` 顺带把 `tabular_host_ready` 重置为
    /// `false`——新 webview 重新导航后会重新走一遍 `ready`。
    #[test]
    fn reload_all_webviews_for_theme_bumps_tabular_grid_tab() {
        let p =
            std::env::temp_dir().join(format!("tabular_theme_reload_{}.csv", std::process::id()));
        std::fs::write(&p, "a,b\n1,2\n").unwrap();
        let mut pane = PreviewPane::default();
        let id = pane.open_path(p.clone());
        pane.tabs_mut()
            .iter_mut()
            .find(|t| t.id == id)
            .unwrap()
            .tabular_host_ready = true;
        pane.reload_all_webviews_for_theme();
        let tab = pane.tabs().iter().find(|t| t.id == id).unwrap();
        assert_eq!(tab.reload_nonce, 1, "tabular tab 应被主题切换推进");
        assert!(
            !tab.tabular_host_ready,
            "重新导航前应把旧 host 的 ready 标记清掉"
        );
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn select_switches_active_without_reloading_webview_tabs() {
        let mut p = PreviewPane::default();
        let _id0 = p.open_path(PathBuf::from("/tmp/a.md")); // webview(.md 走渲染预览)
        let _id1 = p.open_path(PathBuf::from("/tmp/b.md")); // webview
        // 下标 0 是 Blank 占位,a.md=1、b.md=2。
        assert_eq!(p.active_idx(), 2);

        // 切到另一个 webview tab:只改 active,URL 不带 `_r=`(不重载 → 保滚动)。
        p.select(1);
        assert_eq!(p.active_idx(), 1);
        assert_eq!(
            p.desired_webviews()[0].url,
            "dozer://flyfish/host.html?p=%2Ftmp%2Fa.md&theme=dark&ln=1&fs=14",
            "切到异 tab 的 webview 不该推进 reload_nonce(保滚动位置)"
        );
        assert_eq!(
            p.desired_webviews()[1].url,
            "dozer://flyfish/host.html?p=%2Ftmp%2Fb.md&theme=dark&ln=1&fs=14",
            "非目标 tab 不受影响"
        );
        assert_eq!(p.tabs()[1].reload_nonce, 0);
        assert_eq!(p.tabs()[2].reload_nonce, 0);

        // 反复切换彼此仍是 no-op,reload_nonce 始终为 0。
        p.select(2);
        p.select(1);
        assert_eq!(p.tabs()[1].reload_nonce, 0);
        assert_eq!(p.tabs()[2].reload_nonce, 0);

        // 越界下标是 no-op。
        let before = p.active_idx();
        p.select(99);
        assert_eq!(p.active_idx(), before);
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
        // T5:切到 Source 走 `SwitchingMode` + Loading,等 editor host 首帧。
        assert!(matches!(
            pane.tabs()[1].backend_state,
            BackendState::Loading
        ));
        assert_eq!(
            pane.tabs()[1].load_state.stage,
            PreviewLoadStage::SwitchingMode
        );
        pane.exit_code_mode(1);
        assert_eq!(pane.tabs()[1].current_mode(), Some(PreviewMode::Rendered));
        assert!(pane.tabs()[1].hosts_webview());
        // T5:切回 Rendered 同样 SwitchingMode + Loading,等 Flyfish `ready`。
        assert!(matches!(
            pane.tabs()[1].backend_state,
            BackendState::Loading
        ));
        assert_eq!(
            pane.tabs()[1].load_state.stage,
            PreviewLoadStage::SwitchingMode
        );

        std::fs::remove_file(path).ok();
    }

    /// T5:`hosts_any_webview` 覆盖三类 host(Flyfish 渲染 / CodeMirror / JSON
    /// Tree),Unsupported/External/表格为 false。
    #[test]
    fn hosts_any_webview_covers_all_host_kinds() {
        let dir = std::env::temp_dir().join(format!("t5_hosts_any_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mk = |name: &str, content: &str| {
            let p = dir.join(name);
            std::fs::write(&p, content).unwrap();
            p
        };
        let code = mk("a.rs", "fn main() {}\n");
        let md = mk("a.md", "# hi\n");
        let json = mk("a.json", "{\"a\":1}\n");
        let unsupported = mk("a.bin", "\u{0}\u{1}\u{2}");

        let mut pane = PreviewPane::default();
        for p in [&code, &md, &json] {
            let (id, generation) = pane.open_path_provisional(p.clone());
            let generation = generation.unwrap();
            assert!(pane.apply_profile(id, generation, &profile_file(p).unwrap()));
            assert!(
                pane.tabs()
                    .iter()
                    .find(|t| t.id == id)
                    .unwrap()
                    .hosts_any_webview(),
                "{} 应有 host",
                p.display()
            );
        }
        // 二进制未知类型 → Unsupported,无 host。
        let (id, generation) = pane.open_path_provisional(unsupported.clone());
        let generation = generation.unwrap();
        assert!(pane.apply_profile(id, generation, &profile_file(&unsupported).unwrap()));
        assert!(
            !pane
                .tabs()
                .iter()
                .find(|t| t.id == id)
                .unwrap()
                .hosts_any_webview(),
            "Unsupported 不应有 host"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// T5:非窗口化 editor host 在 `Loading` 期预创建但 **hidden**,`document_loaded`
    /// 才 finish 可见;`ready` 阶段本身不 finish(避免"host 起了但正文未到")。
    #[test]
    fn non_windowed_host_hidden_until_document_loaded() {
        let dir = std::env::temp_dir().join(format!("t5_doc_loaded_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("note.txt");
        std::fs::write(&path, "hello\n").unwrap();

        let mut pane = PreviewPane::default();
        let (id, generation) = pane.open_path_provisional(path.clone());
        let generation = generation.unwrap();
        assert_eq!(pane.load_stage(id), PreviewLoadStage::Profiling);
        assert!(pane.apply_profile(id, generation, &profile_file(&path).unwrap()));

        // 画像落定后(app 处理器推进 CreatingHost);host 预创建(hidden),未 finish。
        assert!(pane.advance_load(id, generation, PreviewLoadStage::CreatingHost));
        let specs = pane.desired_editor_webviews(1, crate::app::PanelKind::Files);
        let spec = specs
            .iter()
            .find(|s| s.id == id)
            .expect("Loading 期应预创建 host");
        assert!(!spec.visible, "document_loaded 前必须 hidden");
        assert!(!spec.url.contains("windowed=1"));
        // host `ready`(仍 Loading,Reading)→ 仍 hidden;`document_loaded` 才 finish。
        assert!(pane.advance_load(id, generation, PreviewLoadStage::Reading));
        let specs = pane.desired_editor_webviews(1, crate::app::PanelKind::Files);
        assert!(
            !specs.iter().find(|s| s.id == id).unwrap().visible,
            "reading 阶段仍应 hidden"
        );
        assert!(pane.finish_load(id, generation));
        let specs = pane.desired_editor_webviews(1, crate::app::PanelKind::Files);
        assert!(
            specs.iter().find(|s| s.id == id).unwrap().visible,
            "finish 后可见"
        );

        std::fs::remove_file(&path).ok();
        std::fs::remove_dir_all(&dir).ok();
    }

    /// T12 自动化:Flyfish 渲染 host 的首个可用画面边界——`Loading` 期即便该 tab
    /// 是激活 tab 也 **不可见**(不覆盖 loading 动画),Flyfish `document_loaded`
    /// ACK(`finish_load`)后才可见。对齐 CodeMirror/JSON 的同类边界测试。
    #[test]
    fn flyfish_host_hidden_until_document_loaded() {
        let dir = std::env::temp_dir().join(format!("t8_flyfish_visible_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("doc.md");
        std::fs::write(&path, "# hi\n").unwrap();

        let mut pane = PreviewPane::default();
        let (id, generation) = pane.open_path_provisional(path.clone());
        let generation = generation.expect("Rendered 走加载管线,应有首个世代");
        assert!(pane.apply_profile(id, generation, &profile_file(&path).unwrap()));
        // 推进到 CreatingHost(等待 Flyfish host)。
        assert!(pane.advance_load(id, generation, PreviewLoadStage::CreatingHost));
        let tab = pane.tabs().iter().find(|t| t.id == id).unwrap();
        assert!(tab.hosts_webview(), "Rendered tab 应 host Flyfish webview");

        // Loading 期(激活 tab)Flyfish spec 存在但不可见 —— 不覆盖 loading 动画。
        let specs = pane.desired_webviews();
        let spec = specs
            .iter()
            .find(|s| s.id == id)
            .expect("Rendered 应产出 Flyfish webview spec");
        assert!(!spec.visible, "document_loaded 前 Flyfish 必须 hidden");
        // 但未就绪时必须以「离屏停放」代替真 hidden:WKWebView 对 hidden 视图
        // 挂起 rAF,而 docx 等渲染器的 `load()` 依赖 rAF,会死锁到看门狗超时
        // (2026-09-27 docx「加载超时」根因)。
        assert!(
            spec.park_offscreen,
            "未就绪 Flyfish 必须离屏停放(visible 但仍跑 rAF),不能真 hidden"
        );

        // Flyfish `document_loaded` ACK → finish → Ready,方可可见。
        assert!(pane.finish_load(id, generation));
        let specs = pane.desired_webviews();
        let spec = specs.iter().find(|s| s.id == id).unwrap();
        assert!(spec.visible, "document_loaded 后 Flyfish 可见");
        assert!(!spec.park_offscreen, "就绪后离开离屏停放,回到真实 bounds");
        assert!(
            spec.loading_generation.is_none(),
            "就绪后不再携带 loading 世代"
        );

        std::fs::remove_file(&path).ok();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn encode_component_is_rfc3986_strict() {
        assert_eq!(encode_component("aZ09-._~"), "aZ09-._~");
        assert_eq!(encode_component("/a b"), "%2Fa%20b");
        assert_eq!(encode_component("你"), "%E4%BD%A0");
    }

    #[cfg(any())] // 老 iced editor 已退役,历史测试停用
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
        assert!(!p.tabs()[1].uses_editor_host());
        p.mark_dirty_by_id(id);
        assert!(!p.tabs()[1].dirty, "非原生 tab 不该被标脏");
    }

    #[cfg(any())] // 老 iced editor 已退役,历史测试停用
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

    #[cfg(any())] // 老 iced editor 已退役,历史测试停用
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
            !p.tabs()[p.active_idx()].uses_editor_host(),
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

    #[cfg(any())] // 老 iced editor 已退役,历史测试停用
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

    #[cfg(any())] // 老 iced editor 已退役,历史测试停用
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

    #[cfg(any())] // 老 iced editor 已退役,历史测试停用
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
            !p.tabs()[p.active_idx()].uses_editor_host(),
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

    #[cfg(any())] // 老 iced editor 已退役,历史测试停用
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

    #[cfg(any())] // 老 iced editor 已退役,历史测试停用
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

    /// 关闭脏 CodeMirror tab:Rust 不持有全文,必须走"下发 SaveDocument →
    /// 等 host 回 save_requested → 再关"的异步路径(不能就地关)。
    #[test]
    fn dirty_codemirror_close_defers_and_queues_save_document() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("cm_close_{}.rs", std::process::id()));
        std::fs::write(&path, "fn main() {}\n").unwrap();

        let mut p = PreviewPane::default();
        let id = p.open_path(path.clone());
        if let Some(tab) = p.tabs_mut().iter_mut().find(|t| t.id == id) {
            tab.dirty = true;
        }
        let len_before = p.tabs().len();
        let idx = p.tabs().iter().position(|t| t.id == id).unwrap();

        assert!(
            p.request_save_before_close_if_dirty(idx),
            "脏 CodeMirror tab 应走异步保存路径"
        );
        assert!(p.has_pending_close(id), "应登记等待关闭");
        assert!(
            p.tabs().iter().any(|t| t.id == id),
            "异步路径下调用方不应立刻 close"
        );
        // 队列里应有一条 SaveDocument。
        let cmds = p.take_pending_editor_commands();
        assert!(
            cmds.iter()
                .any(|(tid, c)| { *tid == id && matches!(c, EditorCommand::SaveDocument) })
        );

        // 保存回来 → 消费 pending_close 并真正移除。
        assert!(p.take_pending_close(id));
        assert!(!p.has_pending_close(id), "取走应复位");
        p.close_by_id(id);
        assert!(p.tabs().len() < len_before, "tab 应已被移除");

        std::fs::remove_file(&path).ok();
    }

    /// 不脏 / 非 CodeMirror 的 tab:走原同步关闭路径(返回 `false`)。
    #[test]
    fn clean_tab_close_uses_sync_path() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("cm_clean_{}.rs", std::process::id()));
        std::fs::write(&path, "fn main() {}\n").unwrap();

        let mut p = PreviewPane::default();
        let id = p.open_path(path.clone());
        let idx = p.tabs().iter().position(|t| t.id == id).unwrap();
        assert!(
            !p.request_save_before_close_if_dirty(idx),
            "干净 tab 不该走异步保存路径"
        );
        assert!(!p.has_pending_close(id));

        std::fs::remove_file(&path).ok();
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

    /// T6:内容含 NUL 二进制时,源码扩展名不得凌驾——路由落安全 fallback。
    #[test]
    fn binary_content_overrides_code_extension_to_fallback() {
        let dir = std::env::temp_dir();
        let spoof = dir.join(format!("binary_spoof_{}.rs", std::process::id()));
        std::fs::write(&spoof, b"fn\0main\n").unwrap();
        let (route, backend, _) = route_and_backend(&spoof, &crate::capabilities::current());
        assert_eq!(route.kind, crate::preview::PreviewKind::Unsupported);
        assert_eq!(backend.kind(), crate::preview::PreviewKind::Unsupported);
        std::fs::remove_file(&spoof).ok();
    }

    /// T6:非法 UTF-8(有损)代码 tab 只读、标 `encoding_lossy`、`can_save` 为
    /// false,editor URL 带 `lossy=1`(host 顶部提示 + 禁用保存)。
    #[test]
    fn lossy_utf8_tab_is_read_only_flagged_and_unsaveable() {
        let path = std::env::temp_dir().join(format!("lossy_{}.rs", std::process::id()));
        std::fs::write(&path, b"fn main() {}\n\xFF\xFE not utf8").unwrap();

        let mut pane = PreviewPane::default();
        let id = pane.open_path(path.clone());
        let tab = pane.tabs().iter().find(|t| t.id == id).unwrap();
        assert!(tab.route.as_ref().unwrap().encoding_lossy);
        assert!(tab.backend_read_only());
        assert!(!tab.can_save(), "有损只读 tab 不允许保存");
        let spec = pane
            .desired_editor_webviews(1, crate::app::PanelKind::Files)
            .into_iter()
            .find(|s| s.id == id)
            .expect("应产出 editor spec");
        assert!(spec.url.contains("lossy=1"));
        assert!(spec.url.contains("ro=1"));

        std::fs::remove_file(&path).ok();
    }

    /// T6:UTF-16 tab 只读、URL 提示编码原因。
    #[test]
    fn utf16_tab_is_read_only_with_encoding_notice() {
        let path = std::env::temp_dir().join(format!("u16_{}.rs", std::process::id()));
        std::fs::write(&path, b"\xFF\xFEb\x00a\x00d\x00\n\x00").unwrap();
        let mut pane = PreviewPane::default();
        let id = pane.open_path(path.clone());
        let tab = pane.tabs().iter().find(|t| t.id == id).unwrap();
        assert!(tab.route.as_ref().unwrap().encoding_utf16);
        assert!(!tab.route.as_ref().unwrap().encoding_lossy);
        assert!(!tab.can_save());
        let spec = pane
            .desired_editor_webviews(1, crate::app::PanelKind::Files)
            .into_iter()
            .find(|s| s.id == id)
            .expect("应产出 editor spec");
        assert!(spec.url.contains("enc=utf16"));
        std::fs::remove_file(&path).ok();
    }

    /// T6:普通可编辑 UTF-8 小文件 `can_save` 为 true(对照有损只读)。
    #[test]
    fn editable_utf8_tab_can_save() {
        let path = std::env::temp_dir().join(format!("editable_{}.rs", std::process::id()));
        std::fs::write(&path, "fn main() {}\n").unwrap();
        let mut pane = PreviewPane::default();
        let id = pane.open_path(path.clone());
        let tab = pane.tabs().iter().find(|t| t.id == id).unwrap();
        assert!(tab.can_save());
        std::fs::remove_file(&path).ok();
    }

    fn windowed_fixture(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("{name}_{}.rs", std::process::id()));
        // 7MiB 单行 > FORCE_WINDOWED_LINE_BYTES(5MiB)。
        std::fs::write(&path, "a".repeat(7 * 1024 * 1024)).unwrap();
        path
    }

    /// T14:窗口化 tab 走 editor host(SetWindow 派发判据),索引就绪后
    /// `queue_windowed_view` 真的排入一条 `SetWindow` 命令(正文 + 截断提示
    /// 都由它下发),URL 带 `windowed=1`。
    #[test]
    fn windowed_tab_enqueues_set_window_command() {
        let path = windowed_fixture("t14_windowed_cmd");
        let mut pane = PreviewPane::default();
        let id = pane.open_path(path.clone());
        let tab = pane.tabs().iter().find(|t| t.id == id).unwrap();
        assert!(tab.uses_windowed_editor());
        // 建索引 revision 0(新 tab 的 web_revision 默认 0),应用后推窗口。
        let idx = LineIndex::build(&path, 1000, 0).unwrap();
        assert!(pane.apply_window_index(id, std::sync::Arc::new(idx)));
        assert!(pane.queue_windowed_view(id, 1));
        let cmds = pane.take_pending_editor_commands();
        assert!(
            cmds.iter()
                .any(|(tid, c)| *tid == id && matches!(c, EditorCommand::SetWindow { .. })),
            "窗口化应排出 SetWindow 命令"
        );
        let spec = pane
            .desired_editor_webviews(1, crate::app::PanelKind::Files)
            .into_iter()
            .find(|s| s.id == id)
            .expect("窗口化应产出 editor spec");
        assert!(spec.url.contains("windowed=1"));
        std::fs::remove_file(&path).ok();
    }

    /// T2:runtime 容器反映 viewer 种类——代码 tab `None`、表格 `Tabular`、
    /// 大文件 `Windowed`;且 runtime resident 时不吃 Flyfish webview。
    #[test]
    fn runtime_container_reflects_viewer_kind() {
        let dir = std::env::temp_dir().join(format!("t2_runtime_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let rs = dir.join("a.rs");
        std::fs::write(&rs, "fn main(){}\n").unwrap();
        let csv = dir.join("a.csv");
        std::fs::write(&csv, "a,b\n1,2\n").unwrap();
        let huge = dir.join("huge.rs");
        std::fs::write(&huge, "a".repeat(7 * 1024 * 1024)).unwrap();

        let mut pane = PreviewPane::default();
        let rs_id = pane.open_path(rs.clone());
        assert!(matches!(
            pane.tabs().iter().find(|t| t.id == rs_id).unwrap().runtime,
            PreviewRuntime::None
        ));
        let csv_id = pane.open_path(csv.clone());
        assert!(matches!(
            pane.tabs().iter().find(|t| t.id == csv_id).unwrap().runtime,
            PreviewRuntime::Tabular(TabularState::Loading)
        ));
        let huge_id = pane.open_path(huge.clone());
        let t = pane.tabs().iter().find(|t| t.id == huge_id).unwrap();
        assert!(matches!(t.runtime, PreviewRuntime::Windowed(_)));
        assert!(t.runtime.is_resident());
        assert!(!t.hosts_webview(), "窗口化/表格 resident 时不吃 Flyfish");

        for f in [rs, csv, huge] {
            std::fs::remove_file(f).ok();
        }
    }

    /// T2/T4:backend 描述、runtime 容器、route 三者对每个新 tab 都自洽
    /// (结构化 invariant 测试,替代只靠 debug_assert)。
    #[test]
    fn backend_runtime_route_are_consistent_per_tab() {
        let dir = std::env::temp_dir().join(format!("t4_invariant_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cases = [
            ("a.rs", "fn main(){}\n", PreviewKind::Code),
            ("a.csv", "a,b\n1,2\n", PreviewKind::Tabular),
            ("a.zip", "PK\x03\x04", PreviewKind::External),
            ("mystery.binblob", "\x00\x01\x02", PreviewKind::Unsupported),
            ("data.json", "{\"a\":1}\n", PreviewKind::Json),
        ];
        let mut pane = PreviewPane::default();
        for (name, content, kind) in cases {
            let p = dir.join(name);
            std::fs::write(&p, content).unwrap();
            let id = pane.open_path(p.clone());
            let tab = pane.tabs().iter().find(|t| t.id == id).unwrap();
            assert_eq!(tab.route.as_ref().unwrap().kind, kind, "{name} route");
            assert_eq!(tab.backend.as_ref().unwrap().kind(), kind, "{name} backend");
            // runtime 一致:表格 → Tabular;其余 → None/Windowed(非 Tabular)。
            if kind == PreviewKind::Tabular {
                assert!(matches!(tab.runtime, PreviewRuntime::Tabular(_)), "{name}");
            } else {
                assert!(
                    !matches!(tab.runtime, PreviewRuntime::Tabular(_)),
                    "{name} 不应挂 Tabular runtime"
                );
            }
            // resident runtime 时不得 host webview。
            if tab.runtime.is_resident() {
                assert!(!tab.hosts_webview(), "{name} resident 不应 host webview");
            }
            std::fs::remove_file(&p).ok();
        }
    }

    /// T13:apply_preview_command 的只读导航与 revision 守卫写入。
    #[test]
    fn apply_preview_command_navigation_and_replace_guard() {
        use dozer_core::protocol::{
            PreviewCommand, PreviewCommandAction, PreviewCommandOutcome, PreviewCommandTarget,
        };
        let mk = |action, expected_revision| PreviewCommand {
            request_id: "r".into(),
            project_id: 1,
            target: PreviewCommandTarget::Tab {
                panel: "files".into(),
                tab_id: 0,
            },
            action,
            expected_revision,
        };

        let dir = std::env::temp_dir().join(format!("t13_cmd_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let rs = dir.join("a.rs");
        std::fs::write(&rs, "fn main(){}\n").unwrap();
        let csv = dir.join("a.csv");
        std::fs::write(&csv, "a,b\n1,2\n").unwrap();

        let mut pane = PreviewPane::default();
        let rs_id = pane.open_path(rs.clone());
        let csv_id = pane.open_path(csv.clone());
        // 表格先就绪。
        let view = crate::tabular::load(&csv).unwrap();
        let csv_generation = pane.load_generation(csv_id);
        assert!(
            pane.finish_tabular_load(csv_id, csv_generation, Ok(view))
                .is_none()
        );

        // NotFound。
        assert!(matches!(
            pane.apply_preview_command(
                9999,
                &mk(PreviewCommandAction::Reveal { line: 1, column: 1 }, None)
            ),
            PreviewCommandOutcome::NotFound { .. }
        ));
        // Reveal on Code → Accepted + 排队。
        let out = pane.apply_preview_command(
            rs_id,
            &mk(PreviewCommandAction::Reveal { line: 2, column: 1 }, None),
        );
        assert!(matches!(out, PreviewCommandOutcome::Accepted { .. }));
        assert!(pane.take_pending_editor_commands().iter().any(|(id, c)| {
            *id == rs_id && matches!(c, EditorCommand::RevealPosition { line: 2, .. })
        }));
        // Reveal on 表格网格 → UnsupportedBackend。
        assert!(matches!(
            pane.apply_preview_command(
                csv_id,
                &mk(PreviewCommandAction::Reveal { line: 1, column: 1 }, None)
            ),
            PreviewCommandOutcome::UnsupportedBackend { .. }
        ));
        // RevealCell on 表格 → Accepted。
        assert!(matches!(
            pane.apply_preview_command(
                csv_id,
                &mk(
                    PreviewCommandAction::RevealCell {
                        sheet: 0,
                        row: 1,
                        col: 1
                    },
                    None
                )
            ),
            PreviewCommandOutcome::Accepted { .. }
        ));

        let replace = |expected| {
            mk(
                PreviewCommandAction::Replace {
                    start_line: 1,
                    start_column: 1,
                    end_line: 1,
                    end_column: 4,
                    text: "xxx".into(),
                },
                expected,
            )
        };
        // replace 缺 expected → InternalError。
        assert!(matches!(
            pane.apply_preview_command(rs_id, &replace(None)),
            PreviewCommandOutcome::InternalError { .. }
        ));
        // revision 失配 → StaleRevision(当前 0)。
        assert!(matches!(
            pane.apply_preview_command(rs_id, &replace(Some(7))),
            PreviewCommandOutcome::StaleRevision {
                current_revision: 0,
                ..
            }
        ));
        // revision 匹配 → Accepted + 排队 ReplaceRange。
        assert!(matches!(
            pane.apply_preview_command(rs_id, &replace(Some(0))),
            PreviewCommandOutcome::Accepted { .. }
        ));
        assert!(
            pane.take_pending_editor_commands()
                .iter()
                .any(|(id, c)| { *id == rs_id && matches!(c, EditorCommand::ReplaceRange { .. }) })
        );
        // 只读 tab(窗口化)replace → UnsupportedBackend。
        let huge = dir.join("huge.rs");
        std::fs::write(&huge, "a".repeat(7 * 1024 * 1024)).unwrap();
        let huge_id = pane.open_path(huge.clone());
        assert!(matches!(
            pane.apply_preview_command(huge_id, &replace(Some(0))),
            PreviewCommandOutcome::UnsupportedBackend { .. }
        ));

        std::fs::remove_file(&rs).ok();
        std::fs::remove_file(&csv).ok();
        std::fs::remove_file(&huge).ok();
    }

    #[test]
    fn highlight_on_code_tab_queues_highlight_command() {
        use dozer_core::protocol::{
            PreviewCommand, PreviewCommandAction, PreviewCommandOutcome, PreviewCommandTarget,
        };
        let mk = |action, expected_revision| PreviewCommand {
            request_id: "r".into(),
            project_id: 1,
            target: PreviewCommandTarget::Tab {
                panel: "files".into(),
                tab_id: 0,
            },
            action,
            expected_revision,
        };
        let mut pane = PreviewPane::default();
        let rs_id = pane.open_path(PathBuf::from("/tmp/highlight_test.rs"));
        let cmd = mk(
            PreviewCommandAction::Highlight {
                start_line: 2,
                start_column: 1,
                end_line: 2,
                end_column: 5,
                duration_ms: 1500,
            },
            None,
        );
        let out = pane.apply_preview_command(rs_id, &cmd);
        assert!(matches!(out, PreviewCommandOutcome::Accepted { .. }));
        assert!(pane.take_pending_editor_commands().iter().any(|(id, c)| {
            *id == rs_id
                && matches!(
                    c,
                    EditorCommand::HighlightRange {
                        duration_ms: 1500,
                        ..
                    }
                )
        }));
    }

    #[test]
    fn highlight_on_non_editor_tab_is_unsupported() {
        use dozer_core::protocol::{
            PreviewCommand, PreviewCommandAction, PreviewCommandOutcome, PreviewCommandTarget,
        };
        let mk = |action, expected_revision| PreviewCommand {
            request_id: "r".into(),
            project_id: 1,
            target: PreviewCommandTarget::Tab {
                panel: "files".into(),
                tab_id: 0,
            },
            action,
            expected_revision,
        };
        let mut pane = PreviewPane::default();
        let csv_id = pane.open_path(PathBuf::from("/tmp/highlight_test.csv"));
        let cmd = mk(
            PreviewCommandAction::Highlight {
                start_line: 1,
                start_column: 1,
                end_line: 1,
                end_column: 1,
                duration_ms: 1500,
            },
            None,
        );
        let out = pane.apply_preview_command(csv_id, &cmd);
        assert!(matches!(
            out,
            PreviewCommandOutcome::UnsupportedBackend { .. }
        ));
    }

    /// T12:reveal_tabular_cell 在已就绪的表格 tab 上写入滚动 + 选中。
    #[test]
    fn reveal_tabular_cell_delegates_to_loaded_view() {
        let path = std::env::temp_dir().join(format!("t12_reveal_{}.csv", std::process::id()));
        std::fs::write(&path, "a,b\n1,2\n3,4\n").unwrap();
        let mut pane = PreviewPane::default();
        let id = pane.open_path(path.clone());
        let view = crate::tabular::load(&path).expect("csv 应能解析");
        let generation = pane.load_generation(id);
        assert!(pane.finish_tabular_load(id, generation, Ok(view)).is_none());
        assert!(pane.reveal_tabular_cell(id, 0, 2, 1).is_none());
        let v = pane
            .tabs()
            .iter()
            .find(|t| t.id == id)
            .unwrap()
            .tabular_view()
            .unwrap();
        assert_eq!(v.selection, Some((2, 1, 2, 1)));
        assert_eq!(v.scroll_row, 2);
        std::fs::remove_file(&path).ok();
    }

    /// T11:set_pending_view 保存完整快照(cursor/selection/top_line/folds),
    /// 空快照 is_empty。
    #[test]
    fn pending_view_state_carries_folds_and_top_line() {
        let path = std::env::temp_dir().join(format!("t11_view_{}.rs", std::process::id()));
        std::fs::write(&path, "fn main(){}\n").unwrap();
        let mut pane = PreviewPane::default();
        let id = pane.open_path(path.clone());
        pane.set_pending_view(
            id,
            ViewStateRestore {
                cursor: Some(TextPosition { line: 2, column: 3 }),
                selection: None,
                top_line: Some(5),
                folds: vec![FoldRange {
                    from_line: 1,
                    to_line: 2,
                }],
            },
        );
        let pv = pane
            .tabs()
            .iter()
            .find(|t| t.id == id)
            .unwrap()
            .pending_view
            .as_ref()
            .unwrap();
        assert!(!pv.is_empty());
        assert_eq!(pv.top_line, Some(5));
        assert_eq!(pv.folds.len(), 1);
        assert!(ViewStateRestore::default().is_empty());
        std::fs::remove_file(&path).ok();
    }

    /// T11/T3:淘汰(suspend)时把最近的视图镜像带进 `pending_view`,重新物化时
    /// 经 `RestoreViewState` 还原(同一会话内的淘汰→恢复 round-trip)。
    #[test]
    fn suspend_carries_view_state_mirror_into_pending_view() {
        let path = std::env::temp_dir().join(format!("t11_suspend_{}.rs", std::process::id()));
        std::fs::write(&path, "fn main() {}\n").unwrap();
        let mut pane = PreviewPane::default();
        let id = pane.open_path(path.clone());
        let idx = pane.tabs().iter().position(|t| t.id == id).unwrap();
        pane.tabs_mut()[idx].web_view_state = Some(ViewStateRestore {
            cursor: Some(TextPosition { line: 2, column: 1 }),
            selection: None,
            top_line: Some(9),
            folds: vec![FoldRange {
                from_line: 1,
                to_line: 2,
            }],
        });
        assert!(pane.suspend_tab(id));
        assert!(pane.is_pending_load(id), "淘汰后应回到可物化态");
        let tab = pane.tabs().iter().find(|t| t.id == id).unwrap();
        let pv = tab.pending_view.as_ref().expect("镜像应转成 pending_view");
        assert_eq!(pv.top_line, Some(9));
        assert_eq!(pv.folds.len(), 1);
        std::fs::remove_file(&path).ok();
    }

    /// T3:`suspend_tab` 释放 runtime 并退回 Suspended 壳;`mark_reserve_denied`
    /// 给出可重试 Failed 终态且不再 host webview。
    #[test]
    fn suspend_and_reserve_denied_lifecycle() {
        let path = windowed_fixture("t3_suspend");
        let mut pane = PreviewPane::default();
        let id = pane.open_path(path.clone());
        let idx = LineIndex::build(&path, 1000, 0).unwrap();
        assert!(pane.apply_window_index(id, std::sync::Arc::new(idx)));
        assert!(matches!(
            pane.tabs().iter().find(|t| t.id == id).unwrap().runtime,
            PreviewRuntime::Windowed(_)
        ));
        // 淘汰:释放 runtime → Suspended。
        assert!(pane.suspend_tab(id));
        let tab = pane.tabs().iter().find(|t| t.id == id).unwrap();
        assert!(matches!(tab.runtime, PreviewRuntime::None));
        assert!(tab.window_index().is_none());
        assert!(pane.is_suspended(id));
        assert!(!pane.suspend_tab(id), "已 Suspended 是幂等 no-op");

        // reserve 被拒:可重试 Failed 终态,不吃 webview。世代取 tab 当前值。
        let generation = pane
            .tabs()
            .iter()
            .find(|t| t.id == id)
            .map(|t| t.load_state.generation)
            .unwrap();
        assert!(pane.mark_reserve_denied(id, generation, "预览资源预算不足"));
        let tab = pane.tabs().iter().find(|t| t.id == id).unwrap();
        assert!(tab.backend_state.is_failed());
        assert!(tab.web_error.is_some());
        assert!(!tab.hosts_webview(), "Failed 不再 host Flyfish");
        assert!(pane.is_pending_load(id), "Failed 可重试");

        std::fs::remove_file(&path).ok();
    }

    /// T10:`grant_reserve` 只在 tab 仍处于 `Reserving` 且世代匹配时才推进到
    /// `CreatingHost`;非 `Reserving`(如已就绪)与过期世代都是 no-op。
    #[test]
    fn grant_reserve_advances_only_from_reserving_and_gates_generation() {
        let path = std::env::temp_dir().join(format!("t10_grant_{}.rs", std::process::id()));
        std::fs::write(&path, "fn main() {}\n").unwrap();
        let mut pane = PreviewPane::default();
        let id = pane.push_shell_tab(path.clone(), None);
        let generation = pane
            .begin_load(id, PreviewLoadStage::Reserving)
            .expect("壳可物化");

        // 过期世代:拒绝。
        assert!(!pane.grant_reserve(id, generation + 99));
        assert_eq!(
            pane.load_stage(id),
            PreviewLoadStage::Reserving,
            "过期 grant 不得推进阶段"
        );
        // 当前世代:推进到 CreatingHost。
        assert!(pane.grant_reserve(id, generation));
        assert_eq!(pane.load_stage(id), PreviewLoadStage::CreatingHost);
        // 再次 grant:非 Reserving,no-op。
        assert!(!pane.grant_reserve(id, generation));
        std::fs::remove_file(&path).ok();
    }

    /// T10:reserve 被拒时按**世代**处理:过期世代的 denied 不得改动新加载。
    #[test]
    fn mark_reserve_denied_is_generation_gated() {
        let path = std::env::temp_dir().join(format!("t10_denied_{}.rs", std::process::id()));
        std::fs::write(&path, "fn main() {}\n").unwrap();
        let mut pane = PreviewPane::default();
        let id = pane.open_path(path.clone());
        let generation = pane
            .begin_load(id, PreviewLoadStage::Reserving)
            .expect("可物化");
        // 过期世代:拒绝,不改状态。
        assert!(!pane.mark_reserve_denied(id, generation + 5, "预算不足"));
        assert!(
            pane.tabs()
                .iter()
                .find(|t| t.id == id)
                .unwrap()
                .load_state
                .accepts(generation),
            "过期 denied 不得结束当前加载"
        );
        // 当前世代:进入 Failed(可重试),并作废该世代。
        assert!(pane.mark_reserve_denied(id, generation, "预算不足"));
        let tab = pane.tabs().iter().find(|t| t.id == id).unwrap();
        assert!(tab.backend_state.is_failed());
        assert!(!tab.load_state.is_active(), "denied 不得留下无限 loading");
        assert!(pane.is_pending_load(id), "Failed 可重试");
        std::fs::remove_file(&path).ok();
    }

    /// T10:后台 recovery 结果——加载在途时记入 `pending_restore`;host 已就绪
    /// (世代随 finish 推进)时就地返回恢复指令并标脏;更晚的新加载则丢弃。
    #[test]
    fn apply_recovery_restore_gates_on_loading_generation() {
        let path = std::env::temp_dir().join(format!("t10_recovery_{}.rs", std::process::id()));
        std::fs::write(&path, "fn main() {}\n").unwrap();
        let mut pane = PreviewPane::default();
        let id = pane.push_shell_tab(path.clone(), None);
        let generation = pane
            .begin_load(id, PreviewLoadStage::Reserving)
            .expect("壳可物化");

        // 在途:记入 pending_restore,返回 None(由 DocumentLoaded 消费)。
        assert!(
            pane.apply_recovery_restore(id, generation, "restored".to_string())
                .is_none()
        );
        assert_eq!(
            pane.tabs()
                .iter()
                .find(|t| t.id == id)
                .unwrap()
                .pending_restore
                .as_deref(),
            Some("restored")
        );

        // 模拟 host 就绪:finish 推进世代。
        assert!(pane.finish_load(id, generation));
        // 清掉在途时写下的 pending_restore,验证"慢读"就地恢复分支。
        if let Some(tab) = pane.tabs_mut().iter_mut().find(|t| t.id == id) {
            tab.pending_restore = None;
        }
        let cmd = pane.apply_recovery_restore(id, generation, "late".to_string());
        let (cmd_id, text, _rev) = cmd.expect("finish 后慢读应就地恢复");
        assert_eq!(cmd_id, id);
        assert_eq!(text, "late");
        let tab = pane.tabs().iter().find(|t| t.id == id).unwrap();
        assert!(tab.dirty);
        assert!(tab.recovery_written);

        // 更晚的新加载(世代差 ≥2):丢弃。
        let newer = pane
            .begin_load(id, PreviewLoadStage::Reserving)
            .expect("Ready 可重载");
        assert!(
            pane.apply_recovery_restore(id, newer.wrapping_add(3), "should-drop".to_string())
                .is_none()
        );
        std::fs::remove_file(&path).ok();
    }

    /// T10:加载在途的 editor host 其 spec 携带 loading 世代,供 reserve 回灌。
    #[test]
    fn desired_editor_webviews_carries_loading_generation() {
        let path = std::env::temp_dir().join(format!("t10_spec_gen_{}.rs", std::process::id()));
        std::fs::write(&path, "fn main() {}\n").unwrap();
        let mut pane = PreviewPane::default();
        let id = pane.push_shell_tab(path.clone(), None);
        let generation = pane
            .begin_load(id, PreviewLoadStage::Reserving)
            .expect("壳可物化");
        let spec = pane
            .desired_editor_webviews(1, crate::app::PanelKind::Files)
            .into_iter()
            .find(|s| s.id == id)
            .expect("Loading editor host 应预创建 hidden");
        assert_eq!(spec.loading_generation, Some(generation));
        std::fs::remove_file(&path).ok();
    }

    /// T14:索引 revision 与 tab 当前 revision 不符时必须拒绝(旧索引不得套
    /// 到新内容上)。
    #[test]
    fn apply_window_index_rejects_stale_revision() {
        let path = windowed_fixture("t14_stale_idx");
        let mut pane = PreviewPane::default();
        let id = pane.open_path(path.clone());
        let stale = LineIndex::build(&path, 1000, 99).unwrap();
        assert!(!pane.apply_window_index(id, std::sync::Arc::new(stale)));
        assert!(
            pane.tabs()
                .iter()
                .find(|t| t.id == id)
                .unwrap()
                .window_index()
                .is_none()
        );
        std::fs::remove_file(&path).ok();
    }

    /// T4:tab 已关闭后,迟到的索引结果必须被丢弃(不得凭空重建 runtime)。
    #[test]
    fn apply_window_index_after_tab_close_is_dropped() {
        let path = windowed_fixture("t4_close_drop");
        let mut pane = PreviewPane::default();
        let id = pane.open_path(path.clone());
        let idx = LineIndex::build(&path, 1000, 0).unwrap();
        pane.close_by_id(id);
        assert!(
            !pane.apply_window_index(id, std::sync::Arc::new(idx)),
            "关闭后索引应被丢弃"
        );
        assert!(pane.tabs().iter().all(|t| t.id != id));
        std::fs::remove_file(&path).ok();
    }

    /// T14:外部变更命中窗口化 tab 时,旧索引失效(清空 + revision 归零)、
    /// 推进 reload。
    #[test]
    fn reload_invalidates_windowed_index() {
        let path = windowed_fixture("t14_reload_inval");
        let mut pane = PreviewPane::default();
        let id = pane.open_path(path.clone());
        let idx = LineIndex::build(&path, 1000, 0).unwrap();
        assert!(pane.apply_window_index(id, std::sync::Arc::new(idx)));
        pane.reload_webviews_for(std::slice::from_ref(&path));
        let tab = pane.tabs().iter().find(|t| t.id == id).unwrap();
        assert!(tab.window_index().is_none(), "旧索引应失效");
        assert_eq!(tab.web_revision, 0);
        assert_eq!(tab.reload_nonce, 1, "应推进 reload 重新导航");
        std::fs::remove_file(&path).ok();
    }

    fn tab_mark_conflict(pane: &mut PreviewPane, id: usize) -> bool {
        pane.tabs_mut()
            .iter_mut()
            .find(|t| t.id == id)
            .is_some_and(|t| t.mark_disk_conflict())
    }

    fn dirty_tab_conflict_fixture(name: &str) -> (PreviewPane, usize, PathBuf) {
        let path = std::env::temp_dir().join(format!("{name}_{}.rs", std::process::id()));
        std::fs::write(&path, "fn main() {}\n").unwrap();
        let mut pane = PreviewPane::default();
        let id = pane.open_path(path.clone());
        if let Some(tab) = pane.tabs_mut().iter_mut().find(|t| t.id == id) {
            tab.dirty = true;
        }
        (pane, id, path)
    }

    /// T10:仅脏文件 tab 进入显式冲突态;「保留我的修改」清冲突并允许保存。
    #[test]
    fn conflict_keep_clears_and_allows_save() {
        let (mut pane, id, path) = dirty_tab_conflict_fixture("t10_keep");
        assert!(tab_mark_conflict(&mut pane, id));
        assert!(pane.active_conflict().is_some(), "应进入显式冲突态");
        assert!(pane.keep_conflict_changes(id));
        assert!(pane.active_conflict().is_none());
        let tab = pane.tabs_mut().iter_mut().find(|t| t.id == id).unwrap();
        assert_eq!(tab.save_gate(), SaveGate::Allow);
        assert!(tab.dirty, "保留我的修改后仍保持脏");
        std::fs::remove_file(&path).ok();
    }

    /// T10:冲突未处理 → 保存闸门拒绝;「保留」后磁盘又变 → 重新进入冲突并拒绝。
    #[test]
    fn conflict_save_gate_blocks_and_rechecks_disk() {
        let (mut pane, id, path) = dirty_tab_conflict_fixture("t10_gate");
        assert!(tab_mark_conflict(&mut pane, id));
        {
            let tab = pane.tabs_mut().iter_mut().find(|t| t.id == id).unwrap();
            assert_eq!(tab.save_gate(), SaveGate::Conflict);
        }
        // 保留后把基线设成一个必然不匹配的旧 mtime → 再保存应重新置冲突。
        assert!(pane.keep_conflict_changes(id));
        {
            let tab = pane.tabs_mut().iter_mut().find(|t| t.id == id).unwrap();
            tab.conflict_baseline = Some(std::time::UNIX_EPOCH);
            assert_eq!(tab.save_gate(), SaveGate::Conflict);
            assert!(tab.conflict.is_some(), "磁盘又变应重新进入冲突态");
        }
        std::fs::remove_file(&path).ok();
    }

    /// T10:「重载磁盘」需二次确认;确认后清冲突/清 dirty/推进 reload,并返回路径。
    #[test]
    fn conflict_reload_arms_then_discards() {
        let (mut pane, id, path) = dirty_tab_conflict_fixture("t10_reload");
        assert!(tab_mark_conflict(&mut pane, id));
        assert!(pane.arm_conflict_reload(id), "第一次点击仅置二次确认");
        assert_eq!(pane.active_conflict(), Some((id, true)));
        let returned = pane.discard_conflict_and_reload(id);
        assert_eq!(returned.as_deref(), Some(path.as_path()));
        let tab = pane.tabs().iter().find(|t| t.id == id).unwrap();
        assert!(tab.conflict.is_none());
        assert!(!tab.conflict_reload_armed);
        assert!(!tab.dirty);
        assert_eq!(tab.reload_nonce, 1);
        assert_eq!(tab.web_revision, 0);
        std::fs::remove_file(&path).ok();
    }

    /// T10:干净 tab 不会被标冲突(外部变更自动重载路径不受影响)。
    #[test]
    fn clean_tab_is_not_flagged_conflict() {
        let path = std::env::temp_dir().join(format!("t10_clean_{}.rs", std::process::id()));
        std::fs::write(&path, "fn main() {}\n").unwrap();
        let mut pane = PreviewPane::default();
        let id = pane.open_path(path.clone());
        assert!(!tab_mark_conflict(&mut pane, id));
        assert!(pane.active_conflict().is_none());
        std::fs::remove_file(&path).ok();
    }

    /// 外部文件变化:干净的 CodeMirror tab 自动**就地**重载(T5:下发
    /// `ReloadDocument`,不换 URL 重新导航,旧内容保持可见),脏 tab 不自动
    /// 重载、置冲突提示。
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
        assert_eq!(
            clean_tab.reload_nonce, 0,
            "T5:干净 tab 走 in-place 重载,不换 URL"
        );
        assert!(clean_tab.web_error.is_none());
        let dirty_tab = pane.tabs().iter().find(|t| t.id == dirty_id).unwrap();
        assert_eq!(dirty_tab.reload_nonce, 0, "脏 tab 不自动重载");
        assert!(dirty_tab.web_error.is_some(), "脏 tab 进入冲突提示");

        // 干净 tab 应收到一条 `ReloadDocument` 命令。
        let cmds = pane.take_pending_editor_commands();
        assert!(
            cmds.iter()
                .any(|(id, c)| *id == clean_id && matches!(c, EditorCommand::ReloadDocument)),
            "干净 tab 应下发 ReloadDocument: {cmds:?}"
        );

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

    /// T10 bullet 5(安全启动回归):恢复壳本身**不启动任何加载**——`load_state`
    /// 停在 `Idle`,不产出任何 webview spec。真正加载只能由用户主动打开/选中
    /// (`load_preview_tab`)触发,避免"安全启动又自动跑起来"的循环。
    #[test]
    fn shell_restore_starts_no_load_until_activated() {
        let path = std::env::temp_dir().join(format!("t10_safe_{}.rs", std::process::id()));
        std::fs::write(&path, "fn main() {}\n").unwrap();
        let mut pane = PreviewPane::default();
        let id = pane.push_shell_tab(path.clone(), None);

        let tab = pane.tabs().iter().find(|t| t.id == id).unwrap();
        assert!(matches!(tab.backend_state, BackendState::Suspended));
        assert_eq!(tab.load_state.stage, PreviewLoadStage::Idle);
        assert!(!tab.load_state.is_active());
        assert!(pane.desired_webviews().iter().all(|s| s.id != id));
        assert!(
            pane.desired_editor_webviews(1, crate::app::PanelKind::Files)
                .iter()
                .all(|s| s.id != id),
            "安全启动只恢复壳,不预创建 host"
        );

        // 用户主动物化后才离开 Idle。
        let generation = pane.begin_load(id, PreviewLoadStage::Reserving).unwrap();
        assert!(
            pane.tabs()
                .iter()
                .find(|t| t.id == id)
                .unwrap()
                .load_state
                .is_active()
        );
        assert!(pane.grant_reserve(id, generation));
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
        // 10MiB 单行文本(> FORCE_WINDOWED_LINE_BYTES)。不能用 set_len 的稀疏
        // 全 NUL 文件——T6 起内容二进制会盖过源码扩展名,被当作 fallback。
        std::fs::write(&path, "a".repeat(10 * 1024 * 1024)).unwrap();

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
    #[test]
    fn windowed_code_tab_uses_windowed_editor_host() {
        let dir = std::env::temp_dir().join(format!("dozer_windowed_host_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("huge_line.rs");
        std::fs::write(&path, "a".repeat(10 * 1024 * 1024)).unwrap();

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
    #[test]
    fn json_text_mode_uses_editor_host() {
        let path = std::env::temp_dir().join(format!("json_text_{}.json", std::process::id()));
        std::fs::write(&path, "{\"a\":1}\n").unwrap();
        let mut pane = PreviewPane::default();
        let id = pane.open_path(path.clone());

        // 默认 Tree:不产出 editor spec(由 vanilla-jsoneditor host 承载)。
        assert!(
            pane.desired_editor_webviews(1, crate::app::PanelKind::Files)
                .iter()
                .all(|s| s.id != id)
        );
        assert!(
            !pane
                .tabs()
                .iter()
                .find(|t| t.id == id)
                .unwrap()
                .uses_editor_host(),
            "Tree 模式不算 editor host"
        );

        // 切到 Text。
        if let Some(tab) = pane.tabs_mut().iter_mut().find(|t| t.id == id) {
            tab.backend_state = BackendState::Ready;
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

    /// json-editor feature:严格 JSON 的 Tree 视图产出 `dozer://json-editor/` spec。
    #[test]
    fn json_tree_uses_json_editor_host_when_feature_on() {
        let path = std::env::temp_dir().join(format!("json_host_{}.json", std::process::id()));
        std::fs::write(&path, "{\"a\":1}\n").unwrap();
        let mut pane = PreviewPane::default();
        let id = pane.open_path(path.clone());
        // 默认 Tree + json-editor feature → 使用 json host。
        if let Some(tab) = pane.tabs_mut().iter_mut().find(|t| t.id == id) {
            tab.backend_state = BackendState::Ready;
        }
        let tab = pane.tabs().iter().find(|t| t.id == id).unwrap();
        assert!(tab.uses_json_editor());
        assert!(!tab.uses_editor_host());
        let spec = pane
            .desired_json_webviews(1, crate::app::PanelKind::Files)
            .into_iter()
            .find(|s| s.id == id)
            .expect("Tree 应产出 json-editor spec");
        assert!(crate::preview::is_json_editor_url(&spec.url));
        assert!(spec.url.contains("ro=1"), "Tree 为查看态");

        std::fs::remove_file(&path).ok();
    }

    /// T6:JSON Tree 在 `Loading` 期预创建 json host 但 **hidden**,`document_loaded`
    /// 才可见(避免空白树/半构造 DOM 盖住 loading)。
    #[test]
    fn json_tree_host_hidden_until_document_loaded() {
        let path = std::env::temp_dir().join(format!("t6_tree_hidden_{}.json", std::process::id()));
        std::fs::write(&path, "{\"a\":1}\n").unwrap();
        let mut pane = PreviewPane::default();
        let (id, generation) = pane.open_path_provisional(path.clone());
        let generation = generation.unwrap();
        assert!(pane.apply_profile(id, generation, &profile_file(&path).unwrap()));
        pane.advance_load(id, generation, PreviewLoadStage::CreatingHost);

        // Loading 期:host 已预创建但 hidden。
        let spec = pane
            .desired_json_webviews(1, crate::app::PanelKind::Files)
            .into_iter()
            .find(|s| s.id == id)
            .expect("Loading 期应预创建 Tree host");
        assert!(!spec.visible, "未 ready 的 Tree host 必须 hidden");

        // ready 只推进到 Reading(仍 Loading),document_loaded 才 finish。
        let tab = pane.tabs_mut().iter_mut().find(|t| t.id == id).unwrap();
        tab.web_error = None;
        assert_eq!(pane.load_stage(id), PreviewLoadStage::CreatingHost);
        assert!(pane.finish_load(id, generation));
        assert!(
            pane.tabs()
                .iter()
                .find(|t| t.id == id)
                .unwrap()
                .backend_state
                .is_ready()
        );
        let spec = pane
            .desired_json_webviews(1, crate::app::PanelKind::Files)
            .into_iter()
            .find(|s| s.id == id)
            .expect("ready 后仍应有 Tree host");
        assert!(spec.visible, "ready 后激活 Tree host 应可见");

        std::fs::remove_file(&path).ok();
    }

    /// T6:严格 `.json` 超 `json_tree_bytes` 预算 → 不进 vanilla-jsoneditor,
    /// 降级为 CodeMirror Text。
    #[test]
    fn oversized_json_degrades_to_text_mode() {
        let path = std::env::temp_dir().join(format!("t6_big_{}.json", std::process::id()));
        std::fs::write(&path, "{\"a\":1}\n").unwrap();
        let mut pane = PreviewPane::default();
        // 预算调到 0,任何非空 json 都超预算。
        let mut caps = *pane.capabilities;
        caps.budgets.json_tree_bytes = 0;
        pane.capabilities = std::sync::Arc::new(caps);
        let id = pane.open_path(path.clone());
        let tab = pane.tabs().iter().find(|t| t.id == id).unwrap();
        assert!(
            matches!(
                &tab.backend,
                Some(PreviewBackend::Json(json)) if json.mode == JsonMode::Text
            ),
            "超预算 json 应降级为 Text"
        );
        assert!(!tab.uses_json_editor());
        assert!(tab.uses_editor_host());
        std::fs::remove_file(&path).ok();
    }

    /// T6:Tree↔Text(mode 变更)走 `SwitchingMode` + Loading,generation 推进,
    /// 目标 host 首帧确认前不 finish。mode 未变是 no-op。
    #[test]
    fn restore_json_mode_switches_via_switching_mode() {
        let path = std::env::temp_dir().join(format!("t6_switch_{}.json", std::process::id()));
        std::fs::write(&path, "{\"a\":1}\n").unwrap();
        let mut pane = PreviewPane::default();
        let id = pane.open_path(path.clone());
        if let Some(tab) = pane.tabs_mut().iter_mut().find(|t| t.id == id) {
            tab.backend_state = BackendState::Ready;
            tab.load_state.finish();
        }
        let gen_before = pane
            .tabs()
            .iter()
            .find(|t| t.id == id)
            .unwrap()
            .load_state
            .generation;

        // Tree(default)→ Text:进入 SwitchingMode + Loading,返回 (tab_id, gen)。
        let ret = pane.restore_json_mode(id, PreviewMode::Text);
        assert_eq!(ret, Some((id, gen_before + 1)));
        let tab = pane.tabs().iter().find(|t| t.id == id).unwrap();
        assert!(matches!(
            &tab.backend,
            Some(PreviewBackend::Json(json)) if json.mode == JsonMode::Text
        ));
        assert_eq!(pane.load_stage(id), PreviewLoadStage::SwitchingMode);
        assert!(
            matches!(tab.backend_state, BackendState::Loading),
            "切换期间保持 Loading"
        );
        assert_eq!(
            tab.load_state.generation,
            gen_before + 1,
            "generation 应推进"
        );

        // 旧 generation 的 finish 被丢弃,不结束新 loading。
        assert!(!pane.finish_load(id, gen_before));
        assert_eq!(pane.load_stage(id), PreviewLoadStage::SwitchingMode);

        // mode 未变是 no-op(generation 不动,返回 None)。
        let gen_now = pane
            .tabs()
            .iter()
            .find(|t| t.id == id)
            .unwrap()
            .load_state
            .generation;
        assert_eq!(pane.restore_json_mode(id, PreviewMode::Text), None);
        assert_eq!(
            pane.tabs()
                .iter()
                .find(|t| t.id == id)
                .unwrap()
                .load_state
                .generation,
            gen_now
        );

        // Text → Tree:同样走 SwitchingMode,返回新的 generation。
        let ret = pane.restore_json_mode(id, PreviewMode::Tree);
        assert_eq!(ret, Some((id, gen_now + 1)));
        assert!(matches!(
            &pane.tabs().iter().find(|t| t.id == id).unwrap().backend,
            Some(PreviewBackend::Json(json)) if json.mode == JsonMode::Tree
        ));

        std::fs::remove_file(&path).ok();
    }

    /// JSONC 不支持 Tree 视图(route.alternate_modes 为空)→ 切 Tree 返回
    /// `None`(按钮不会画,这里兜底验证数据层拒绝)。
    #[test]
    fn restore_json_mode_rejects_unsupported_mode() {
        let path = std::env::temp_dir().join(format!("t6_reject_{}.jsonc", std::process::id()));
        std::fs::write(&path, "{ // c\n \"a\":1\n}\n").unwrap();
        let mut pane = PreviewPane::default();
        let id = pane.open_path(path.clone());
        assert_eq!(pane.restore_json_mode(id, PreviewMode::Tree), None);
        std::fs::remove_file(&path).ok();
    }

    /// JSONC/JSON5 含注释,vanilla-jsoneditor 不解析 → 走 CodeMirror 文本,不用 json host。
    #[test]
    fn jsonc_json5_go_to_text_not_json_host() {
        for ext in ["json5", "jsonc"] {
            let path = std::env::temp_dir().join(format!("j_{}_{}.{ext}", std::process::id(), ext));
            std::fs::write(&path, "{ // c\n \"a\":1\n}\n").unwrap();
            let mut pane = PreviewPane::default();
            let id = pane.open_path(path.clone());
            if let Some(tab) = pane.tabs_mut().iter_mut().find(|t| t.id == id) {
                tab.backend_state = BackendState::Ready;
            }
            let tab = pane.tabs().iter().find(|t| t.id == id).unwrap();
            assert!(!tab.uses_json_editor(), "{ext} 不应走 json host");
            assert!(
                pane.desired_json_webviews(1, crate::app::PanelKind::Files)
                    .iter()
                    .all(|s| s.id != id)
            );
            std::fs::remove_file(&path).ok();
        }
    }

    /// feature 打开时,CSV 的"原文"模式由 CodeMirror editor host 承载。
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

    /// T1:压缩包 / 未知二进制 tab 不再 host Flyfish webview(改由 iced fallback
    /// 页承载),因此也不进 `desired_webviews` 期望清单。
    #[test]
    fn external_and_unsupported_tabs_do_not_host_webview() {
        let dir = std::env::temp_dir().join(format!("dozer_fb_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let zip = dir.join("a.zip");
        std::fs::write(&zip, b"PK\x03\x04stub").unwrap();
        let bin = dir.join("unknown.binblob");
        std::fs::write(&bin, b"\x00\x01\x02\xff").unwrap();

        let mut pane = PreviewPane::default();
        let z = pane.open_path(zip.clone());
        let b = pane.open_path(bin.clone());
        for id in [z, b] {
            let tab = pane.tabs().iter().find(|t| t.id == id).unwrap();
            assert!(!tab.hosts_webview(), "fallback 类 tab 不吃 Flyfish");
        }
        assert!(
            pane.desired_webviews()
                .iter()
                .all(|s| s.id != z && s.id != b),
            "fallback 类 tab 不进 webview 期望清单"
        );

        std::fs::remove_file(&zip).ok();
        std::fs::remove_file(&bin).ok();
    }

    /// T1:fallback 页的「以纯文本只读尝试」把 backend 强制成只读 Code。
    #[test]
    fn force_plain_text_switches_backend_to_readonly_code() {
        let dir = std::env::temp_dir().join(format!("dozer_fpt_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("unknown.binblob");
        std::fs::write(&bin, b"\x00\x01\x02\xff").unwrap();

        let mut pane = PreviewPane::default();
        let id = pane.open_path(bin.clone());
        assert!(pane.force_plain_text(id));
        let tab = pane.tabs().iter().find(|t| t.id == id).unwrap();
        assert!(matches!(
            tab.backend,
            Some(PreviewBackend::Code(CodeBackend {
                mode: CodeMode::ReadOnly,
                ..
            }))
        ));
        assert!(tab.uses_editor_host());
        assert!(tab.backend_state.is_ready());
        assert!(!tab.hosts_webview());

        std::fs::remove_file(&bin).ok();
    }

    /// T1:`begin_shell_load` 现在也接受 Failed→Loading(重试),但 Ready 仍拒绝。
    #[test]
    fn begin_shell_load_allows_retry_from_failed_only() {
        let mut pane = PreviewPane::default();
        let id = pane.push_shell_tab(PathBuf::from("/tmp/a.rs"), None);
        assert!(pane.is_pending_load(id));
        assert!(pane.begin_shell_load(id), "Suspended 可物化");
        pane.finish_shell_load(id);
        assert!(!pane.is_pending_load(id));
        assert!(!pane.begin_shell_load(id), "Ready 不可再物化");
        // Failed → 可重试。
        if let Some(tab) = pane.tabs_mut().iter_mut().find(|t| t.id == id) {
            tab.backend_state = BackendState::Failed(PreviewError::new("x", true));
        }
        assert!(pane.is_pending_load(id));
        assert!(pane.begin_shell_load(id), "Failed 可重试");
    }

    /// T2:`begin_load` 推进 generation,`advance_load` 只在世代匹配时改阶段。
    #[test]
    fn begin_and_advance_load_track_generation() {
        let mut pane = PreviewPane::default();
        let id = pane.push_shell_tab(PathBuf::from("/tmp/a.rs"), None);
        let gen1 = pane.begin_load(id, PreviewLoadStage::Profiling).unwrap();
        assert_eq!(pane.load_stage(id), PreviewLoadStage::Profiling);
        assert!(pane.advance_load(id, gen1, PreviewLoadStage::Reading));
        assert_eq!(pane.load_stage(id), PreviewLoadStage::Reading);
        // 旧世代不能推进。
        assert!(!pane.advance_load(id, gen1 + 99, PreviewLoadStage::Indexing));
        assert_eq!(pane.load_stage(id), PreviewLoadStage::Reading);
        // 再次 begin 得到新世代。
        let gen2 = pane.begin_load(id, PreviewLoadStage::CreatingHost).unwrap();
        assert_ne!(gen1, gen2);
        assert!(
            !pane.advance_load(id, gen1, PreviewLoadStage::Parsing),
            "旧世代失效"
        );
        assert!(pane.advance_load(id, gen2, PreviewLoadStage::Parsing));
    }

    /// T2:`finish_load` 只在世代匹配时置 Ready;过期 finish 不改状态。
    #[test]
    fn finish_load_is_generation_gated() {
        let mut pane = PreviewPane::default();
        let id = pane.push_shell_tab(PathBuf::from("/tmp/a.rs"), None);
        let generation = pane.begin_load(id, PreviewLoadStage::Reading).unwrap();
        assert!(!pane.finish_load(id, generation + 1), "过期 finish 被拒");
        assert_eq!(pane.load_stage(id), PreviewLoadStage::Reading);
        assert!(pane.finish_load(id, generation));
        assert_eq!(pane.load_stage(id), PreviewLoadStage::Idle);
        let tab = pane.tabs().iter().find(|t| t.id == id).unwrap();
        assert!(tab.backend_state.is_ready());
        // 重复 finish 不会把 Idle 再改坏。
        assert!(
            !pane.finish_load(id, generation),
            "已结束的世代再次 finish 被拒"
        );
    }

    /// T2:`fail_load` 进入可解释 Failed(带错误信息),过期失败被丢弃。
    #[test]
    fn fail_load_sets_failed_and_rejects_stale() {
        let mut pane = PreviewPane::default();
        let id = pane.push_shell_tab(PathBuf::from("/tmp/a.rs"), None);
        let generation = pane.begin_load(id, PreviewLoadStage::Indexing).unwrap();
        assert!(
            !pane.fail_load(id, generation + 1, PreviewError::new("old", true)),
            "旧失败被拒"
        );
        assert!(pane.fail_load(id, generation, PreviewError::new("boom", true)));
        let tab = pane.tabs().iter().find(|t| t.id == id).unwrap();
        assert!(tab.backend_state.is_failed());
        assert_eq!(tab.web_error.as_deref(), Some("boom"));
        assert_eq!(pane.load_stage(id), PreviewLoadStage::Idle);
        // 失败后可重试:重新 begin 拿到新世代。
        let generation2 = pane.begin_load(id, PreviewLoadStage::Profiling).unwrap();
        assert!(pane.advance_load(id, generation2, PreviewLoadStage::Reading));
    }

    /// T12 自动化:表驱动覆盖**每个在途阶段**在 `PreviewPane` 上的失败路径。
    /// 对每个 stage:失败进入可解释 Failed、回 Idle、过期失败被拒、失败后可重试。
    #[test]
    fn stage_failure_table() {
        let stages: Vec<PreviewLoadStage> = PreviewLoadStage::active_stages().collect();
        assert_eq!(
            stages.len(),
            PreviewLoadStage::ALL.len() - 1,
            "覆盖全部在途阶段"
        );
        for (i, stage) in stages.into_iter().enumerate() {
            let mut pane = PreviewPane::default();
            let id = pane.push_shell_tab(PathBuf::from(format!("/tmp/s{i}.rs")), None);
            let generation = pane.begin_load(id, stage).unwrap();
            assert_eq!(pane.load_stage(id), stage, "{stage:?} 进入阶段");

            // 过期失败(旧世代)被拒,状态不变。
            assert!(
                !pane.fail_load(id, generation + 1, PreviewError::new("stale", true)),
                "{stage:?} 旧世代失败被拒"
            );
            assert_eq!(pane.load_stage(id), stage, "{stage:?} 状态不被过期失败改动");

            // 匹配失败 → Failed + 回 Idle。
            assert!(pane.fail_load(id, generation, PreviewError::new("boom", true)));
            assert_eq!(pane.load_stage(id), PreviewLoadStage::Idle);
            let tab = pane.tabs().iter().find(|t| t.id == id).unwrap();
            assert!(tab.backend_state.is_failed(), "{stage:?} 进入 Failed");
            assert_eq!(tab.web_error.as_deref(), Some("boom"));

            // 失败后可重试:新世代、可继续。
            let generation2 = pane.begin_load(id, PreviewLoadStage::Profiling).unwrap();
            assert_ne!(generation2, generation, "{stage:?} 重试拿到新世代");
            assert!(pane.advance_load(id, generation2, PreviewLoadStage::Reading));
            assert!(pane.cancel_load(id), "{stage:?} 重试后可再次结束");
        }
    }

    /// T12 自动化:表驱动覆盖**每个在途阶段**的取消路径:取消回 Idle、不留错误、
    /// 原世代终止回调被拒、可再次加载。
    #[test]
    fn stage_cancel_table() {
        for (i, stage) in PreviewLoadStage::active_stages().enumerate() {
            let mut pane = PreviewPane::default();
            let id = pane.push_shell_tab(PathBuf::from(format!("/tmp/c{i}.rs")), None);
            let generation = pane.begin_load(id, stage).unwrap();
            assert!(pane.cancel_load(id), "{stage:?} 可取消");
            assert_eq!(pane.load_stage(id), PreviewLoadStage::Idle);
            let tab = pane.tabs().iter().find(|t| t.id == id).unwrap();
            assert!(tab.web_error.is_none(), "{stage:?} 取消无错误");
            assert!(
                !pane.finish_load(id, generation),
                "{stage:?} 原世代 finish 被拒"
            );
            // 取消后可再次加载。
            assert!(pane.begin_load(id, PreviewLoadStage::Profiling).is_some());
        }
    }

    /// T2:取消是正常结束——回 Idle、作废在途结果、不留错误,并退回可物化。
    #[test]
    fn cancel_load_is_normal_end() {
        let mut pane = PreviewPane::default();
        let id = pane.push_shell_tab(PathBuf::from("/tmp/a.rs"), None);
        let generation = pane.begin_load(id, PreviewLoadStage::Searching).unwrap();
        assert!(pane.cancel_load(id));
        assert_eq!(pane.load_stage(id), PreviewLoadStage::Idle);
        let tab = pane.tabs().iter().find(|t| t.id == id).unwrap();
        assert!(tab.web_error.is_none(), "取消不显示错误");
        // 取消后原世代的 finish 被拒(不会误结束后续加载)。
        assert!(!pane.finish_load(id, generation));
        // 无在途加载时 cancel 为 no-op。
        assert!(!pane.cancel_load(id));
    }

    /// T11 bullet 3:超时只在同 generation 且同 stage 时生效;推进阶段或换世代
    /// 后,迟到的旧超时必须被拒(否则会误杀已推进的加载)。
    #[test]
    fn load_timeout_matches_only_same_generation_and_stage() {
        let mut pane = PreviewPane::default();
        let id = pane.push_shell_tab(PathBuf::from("/tmp/a.rs"), None);
        let generation = pane.begin_load(id, PreviewLoadStage::CreatingHost).unwrap();
        assert!(pane.load_timeout_matches(id, generation, PreviewLoadStage::CreatingHost));
        // stage 不符(已推进)→ 拒绝。
        assert!(!pane.load_timeout_matches(id, generation, PreviewLoadStage::Reading));
        // generation 不符(已重开)→ 拒绝。
        assert!(!pane.load_timeout_matches(id, generation + 1, PreviewLoadStage::CreatingHost));
        // 推进到 Reading 后,Reading 的超时匹配,CreatingHost 的不再匹配。
        assert!(pane.advance_load(id, generation, PreviewLoadStage::Reading));
        assert!(pane.load_timeout_matches(id, generation, PreviewLoadStage::Reading));
        assert!(!pane.load_timeout_matches(id, generation, PreviewLoadStage::CreatingHost));
        // 未知 tab_id → 拒绝。
        assert!(!pane.load_timeout_matches(id + 999, generation, PreviewLoadStage::Reading));
    }

    /// T11 bullet 5:诊断快照只含在途 tab,字段反映真实 stage/generation,
    /// 首帧/取消状态可观测。
    #[test]
    fn load_diagnostics_reports_active_loads_only() {
        let mut pane = PreviewPane::default();
        let idle_id = pane.push_shell_tab(PathBuf::from("/tmp/idle.rs"), None);
        let active_id = pane.push_shell_tab(PathBuf::from("/tmp/active.rs"), None);
        let generation = pane
            .begin_load(active_id, PreviewLoadStage::Indexing)
            .unwrap();
        let diags = pane.load_diagnostics();
        assert_eq!(diags.len(), 1, "只有在途 tab 出现在诊断里");
        let d = &diags[0];
        assert_eq!(d.tab_id, active_id);
        assert_eq!(d.stage, PreviewLoadStage::Indexing);
        assert_eq!(d.generation, generation);
        assert!(!d.first_frame_drawn);
        assert!(!d.cancellation_requested);
        // 取消后该 tab 退出在途 → 不再出现在诊断里。
        assert!(pane.cancel_load(active_id));
        assert!(pane.load_diagnostics().is_empty());
        // idle_id 从未加载,也从未出现在诊断里。
        assert!(!pane.load_diagnostics().iter().any(|d| d.tab_id == idle_id));
    }

    /// T11 bullet 4/5:视图组装写回首帧后,诊断如实反映 first_frame_drawn。
    #[test]
    fn mark_first_frame_reflects_in_diagnostics() {
        let mut pane = PreviewPane::default();
        let id = pane.push_shell_tab(PathBuf::from("/tmp/a.rs"), None);
        let generation = pane.begin_load(id, PreviewLoadStage::Reading).unwrap();
        // 模拟视图组装时写回首帧(带当前 generation)。
        pane.tabs()
            .iter()
            .find(|t| t.id == id)
            .and_then(|t| t.load_observe.as_ref())
            .unwrap()
            .mark_first_frame(generation);
        let d = pane
            .load_diagnostics()
            .into_iter()
            .find(|d| d.tab_id == id)
            .unwrap();
        assert!(d.first_frame_drawn);
        // 世代不符的写回被忽略。
        let mut pane2 = PreviewPane::default();
        let id2 = pane2.push_shell_tab(PathBuf::from("/tmp/b.rs"), None);
        let gen2 = pane2.begin_load(id2, PreviewLoadStage::Reading).unwrap();
        pane2
            .tabs()
            .iter()
            .find(|t| t.id == id2)
            .and_then(|t| t.load_observe.as_ref())
            .unwrap()
            .mark_first_frame(gen2 + 99);
        assert!(
            !pane2
                .load_diagnostics()
                .into_iter()
                .find(|d| d.tab_id == id2)
                .unwrap()
                .first_frame_drawn
        );
    }

    /// T3:`open_path_provisional` 建壳即停在 `Profiling` + `Loading`,返回
    /// 首个世代;同一文件复用已开 tab 时不再返回世代(调用方不重复派发画像)。
    #[test]
    fn open_path_provisional_starts_profiling_and_reuses() {
        let mut pane = PreviewPane::default();
        let (id, generation) = pane.open_path_provisional(PathBuf::from("/tmp/t3_a.rs"));
        assert_eq!(generation, Some(1));
        assert_eq!(pane.load_stage(id), PreviewLoadStage::Profiling);
        let tab = pane.tabs().iter().find(|t| t.id == id).unwrap();
        assert!(matches!(tab.backend_state, BackendState::Loading));
        assert!(!tab.hosts_webview(), "Loading 期不得挂 webview");
        // 复用已开文件:同 id,不再返回世代。
        let (id2, generation2) = pane.open_path_provisional(PathBuf::from("/tmp/t3_a.rs"));
        assert_eq!(id2, id);
        assert_eq!(generation2, None);
    }

    /// T3:后台画像回灌精修 route/后端世代闸门——过期画像(世代不符)必须被
    /// 丢弃;匹配的画像把临时壳推进到与画像一致的 route。
    #[test]
    fn apply_profile_is_generation_gated_and_refines_route() {
        let mut pane = PreviewPane::default();
        let (id, generation) = pane.open_path_provisional(PathBuf::from("/tmp/t3_b.rs"));
        let generation = generation.unwrap();
        // 过期世代:拒绝。
        assert!(!pane.apply_profile(
            id,
            generation + 1,
            &analyze(b"fn main(){}\n", None, 11, None)
        ));
        // 匹配世代:用真实画像精修(小可编辑文本 → Code,可编辑)。
        assert!(pane.apply_profile(id, generation, &analyze(b"fn main(){}\n", None, 11, None)));
        let tab = pane.tabs().iter().find(|t| t.id == id).unwrap();
        assert_eq!(tab.route.as_ref().unwrap().kind, PreviewKind::Code);
        assert!(!tab.windowed);
        assert!(
            tab.backend_state.is_ready() || matches!(tab.backend_state, BackendState::Loading),
            "apply_profile 只改 route,不改 Loading(tab 结束由调用方 finish)"
        );
    }

    /// T3:临时 route(尺寸未知)按非窗口化;真实画像回灌后大文件**升级为
    /// 窗口化**,runtime 换成 `Windowed`。
    #[test]
    fn apply_profile_upgrades_to_windowed() {
        let path = windowed_fixture("t3_upgrade_windowed");
        let mut pane = PreviewPane::default();
        let (id, generation) = pane.open_path_provisional(path.clone());
        let generation = generation.unwrap();
        // 建壳阶段:不读盘 → 非窗口化。
        assert!(!pane.tabs().iter().find(|t| t.id == id).unwrap().windowed);
        // 回灌真实画像:超长单行 → 窗口化。
        let profile = profile_file(&path).unwrap();
        assert!(pane.apply_profile(id, generation, &profile));
        let tab = pane.tabs().iter().find(|t| t.id == id).unwrap();
        assert!(tab.windowed);
        assert!(matches!(tab.runtime, PreviewRuntime::Windowed(_)));
        std::fs::remove_file(&path).ok();
    }

    /// T4:窗口化 tab 在 `Loading` 期就**预创建 host(hidden)**,避免 `ready`
    /// 空编辑器先露面;`finish_load` 后才可见。
    #[test]
    fn windowed_tab_precreates_hidden_host_until_ready() {
        let path = windowed_fixture("t4_hidden_host");
        let mut pane = PreviewPane::default();
        let (id, generation) = pane.open_path_provisional(path.clone());
        let generation = generation.unwrap();
        assert!(pane.apply_profile(id, generation, &profile_file(&path).unwrap()));
        assert!(pane.tabs().iter().find(|t| t.id == id).unwrap().windowed);
        // Loading + 窗口化:应出现在 editor 期望清单里,但 **hidden**(不显示)。
        let specs = pane.desired_editor_webviews(1, crate::app::PanelKind::Files);
        let spec = specs
            .iter()
            .find(|s| s.id == id)
            .expect("窗口化应预创建 host");
        assert!(!spec.visible, "首窗 ACK 前保持 hidden");
        assert!(spec.url.contains("windowed=1"));
        // 结束加载(模拟收到 window_applied)→ 可见。
        pane.finish_load(id, generation);
        let specs = pane.desired_editor_webviews(1, crate::app::PanelKind::Files);
        let spec = specs.iter().find(|s| s.id == id).unwrap();
        assert!(spec.visible, "finish 后活跃窗口化 tab 可见");
        std::fs::remove_file(&path).ok();
    }

    /// T4:窗口化加载阶段序列 `Profiling → CreatingHost → Indexing → LoadingWindow`,
    /// 直到 `window_applied`(finish)才回到 Idle。
    #[test]
    fn windowed_load_stage_sequence_reaches_loading_window_before_ready() {
        let path = windowed_fixture("t4_stage_seq");
        let mut pane = PreviewPane::default();
        let (id, generation) = pane.open_path_provisional(path.clone());
        let generation = generation.unwrap();
        assert_eq!(pane.load_stage(id), PreviewLoadStage::Profiling);
        assert!(pane.apply_profile(id, generation, &profile_file(&path).unwrap()));
        // 画像落定:窗口化推进 CreatingHost(仍在 Loading)。
        assert!(pane.advance_load(id, generation, PreviewLoadStage::CreatingHost));
        assert!(matches!(
            pane.tabs()
                .iter()
                .find(|t| t.id == id)
                .unwrap()
                .backend_state,
            BackendState::Loading
        ));
        // host ready → Indexing。
        assert!(pane.advance_load(id, generation, PreviewLoadStage::Indexing));
        // 索引就绪 → LoadingWindow(推首窗,等 ACK)。
        assert!(pane.advance_load(id, generation, PreviewLoadStage::LoadingWindow));
        assert!(
            !pane
                .tabs()
                .iter()
                .find(|t| t.id == id)
                .unwrap()
                .backend_state
                .is_ready()
        );
        // 收到 window_applied → finish。
        assert!(pane.finish_load(id, generation));
        assert_eq!(pane.load_stage(id), PreviewLoadStage::Idle);
        assert!(
            pane.tabs()
                .iter()
                .find(|t| t.id == id)
                .unwrap()
                .backend_state
                .is_ready()
        );
        std::fs::remove_file(&path).ok();
    }

    /// T3:未知扩展名 + 二进制内容——临时 route 无法按扩展名判定(落空画像
    /// `Empty` → Code),真实画像回灌后**改判 Unsupported**(安全 fallback)。
    #[test]
    fn apply_profile_unknown_binary_becomes_unsupported() {
        let mut pane = PreviewPane::default();
        let (id, generation) = pane.open_path_provisional(PathBuf::from("/tmp/t3_unknown.binxyz"));
        let generation = generation.unwrap();
        assert_eq!(
            pane.tabs()
                .iter()
                .find(|t| t.id == id)
                .unwrap()
                .route
                .as_ref()
                .unwrap()
                .kind,
            PreviewKind::Code,
            "临时 route 对未知扩展名按空内容(Empty)判 Code"
        );
        // 带 NUL 的二进制内容。
        let profile = analyze(b"\x00\x01\x02rubbish", None, 10, None);
        assert!(pane.apply_profile(id, generation, &profile));
        assert_eq!(
            pane.tabs()
                .iter()
                .find(|t| t.id == id)
                .unwrap()
                .route
                .as_ref()
                .unwrap()
                .kind,
            PreviewKind::Unsupported
        );
    }

    #[test]
    fn image_annotate_events_apply_to_matching_tab_only() {
        use crate::preview::{ImageAnnotateEvent, PreviewRuntime};
        let mut pane = PreviewPane::default();
        let id = pane.push_tab(
            TabKind::File(std::path::PathBuf::from("/tmp/pic.png")),
            "pic.png".into(),
        );
        // 标注入镜像后不触加载状态。
        pane.apply_image_annotate_event(
            id,
            ImageAnnotateEvent::AnnotationsChanged {
                annotations: vec![serde_json::json!({"id":"a1"})],
            },
        );
        let tab = pane.tabs().iter().find(|t| t.id == id).unwrap();
        assert_eq!(tab.image_annotations.len(), 1);
        // 非本 tab 的事件不污染。
        pane.apply_image_annotate_event(
            id + 999,
            ImageAnnotateEvent::AnnotationsChanged {
                annotations: vec![serde_json::json!({"id":"z"})],
            },
        );
        assert_eq!(
            pane.tabs()
                .iter()
                .find(|t| t.id == id)
                .unwrap()
                .image_annotations
                .len(),
            1
        );
        // Failed 回落统一 Failed 终态并清 runtime(先置 Loading,模拟生产
        // 建壳后的加载态——`try_transition` 只允许 Loading→Failed)。
        pane.tabs_mut()
            .iter_mut()
            .find(|t| t.id == id)
            .unwrap()
            .backend_state = crate::preview::BackendState::Loading;
        pane.apply_image_annotate_event(
            id,
            ImageAnnotateEvent::Failed {
                message: "decode".into(),
                recoverable: true,
            },
        );
        let tab = pane.tabs().iter().find(|t| t.id == id).unwrap();
        assert!(tab.backend_state.is_failed());
        assert!(matches!(tab.runtime, PreviewRuntime::None));
        assert_eq!(tab.web_error.as_deref(), Some("decode"));
    }
}
