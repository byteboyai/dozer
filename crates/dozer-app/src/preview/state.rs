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
    /// 该 tab 的**运行时状态容器**(T2):backend 只回答"该用什么看",本字段
    /// 回答"当前加载到什么状态"(表格加载/就绪、窗口化索引/窗口/截断)。
    /// 与 backend 描述分开,是 viewer 运行时对象的唯一归属。
    pub runtime: PreviewRuntime,
    /// 原生可编辑 tab 的"buffer 与磁盘不一致"标记:用户就地改过、还没 ⌘S 保存
    /// (或右键"刷新"/项目切换丢弃归零)为 `true`。`Blank`/`webview` tab 恒
    /// `false`。2026-09-06 原生预览不再只读,有了就地编辑就必须能显式挂脏并兜底,
    /// 否则用户会无声丢改动(见 workspace.rs 关闭/切换前的确认)。
    pub dirty: bool,
    /// 由外部面板（目前只有代码健康度面板）请求的"打开后立即跳转到这一
    /// 行"——`Some` 只在"这个 tab 刚被新建、还没就绪"的窗口期内有意义，收到
    /// editor `ready`、窗口/正文推给 host 的那一刻立刻 `take()` 消费掉。
    /// 已经打开且已就绪的 tab不走这个字段，直接排队 `RevealPosition`。
    pub pending_jump_line: Option<usize>,
    /// Phase A 统一路由结果(含可展示 reason)。`Blank` 占位 tab 没有文件,
    /// 为 `None`;其余文件 tab 在**创建那一刻**就带上,是唯一的路由真相源。
    pub route: Option<PreviewRoute>,
    /// 统一 backend 描述(只回答"该用什么看",无运行时句柄)。与 `route`
    /// 同生同灭;运行时加载状态在 `runtime` + `backend_state`。
    pub backend: Option<PreviewBackend>,
    /// backend 生命周期状态(加载态/就绪/失败的唯一真相)。
    pub backend_state: BackendState,
    /// `file_policy` 判定该文件应窗口化(超预算/超 128MiB/超长行)。窗口化
    /// 专用 viewer 未落地前,这类文件不吃 CodeMirror 整载。
    pub windowed: bool,
    /// T2:统一 loading 生命周期(阶段 + 世代 + 起始时刻 + 可选进度)。取代
    /// 零散地把 `BackendState` 在 Loading/Ready 间弹,以及 `load_started` 只记
    /// 时间不表达阶段的旧做法。`BackendState::Loading` 仍表示"尚无首个可用画面",
    /// 具体文案由 `load_state.stage` 决定。
    pub load_state: PreviewLoadState,
    /// 脏内容的 recovery snapshot 是否已落盘(允许休眠脏 tab 的前提)。
    pub recovery_written: bool,
    /// 启动恢复时从 recovery 读回的正文;editor `ready` 后经 `SetDocument` 推回,
    /// 并重新标脏。
    pub pending_restore: Option<String>,
    /// 本次物化开始时刻(用于 ready latency 观测;不记文件内容)。
    pub load_started: Option<std::time::Instant>,
    /// 启动恢复/淘汰恢复的完整视图状态(cursor/selection/top_line/folds),
    /// editor `ready` 后经一次 `RestoreViewState` 应用。
    pub pending_view: Option<ViewStateRestore>,
    /// 启动恢复的表格视图状态 `(sheet, scroll_row, scroll_col)`,表格加载完成
    /// 后应用一次。
    pub pending_tabular: Option<(usize, usize, usize)>,
    /// CodeMirror host 的轻量镜像；正文仍由 WebView 持有，Rust 只保留 Agent、
    /// 保存和过期事件校验所需状态。
    pub web_revision: u64,
    pub web_selection: Option<crate::preview::TextRange>,
    pub web_selected_text: Option<String>,
    pub web_viewport: Option<(u32, u32)>,
    /// T11:最近一次 `view_state` 事件的完整镜像(cursor/selection/top_line/
    /// folds),用于持久化与资源淘汰前序列化;比 `web_selection`/`web_viewport`
    /// 更全(含 top_line/folds)。
    pub web_view_state: Option<ViewStateRestore>,
    pub web_error: Option<String>,
    /// T10:脏 tab 的磁盘文件被外部修改——`Some(mtime)` 是检测到冲突时的磁盘
    /// 修改时间。为 `Some` 时该 tab 进入显式冲突态,保存被拒,直到用户选择
    /// 「保留我的修改」或「重载磁盘」。
    pub conflict: Option<SystemTime>,
    /// T10:「重载磁盘」的二次确认;第一次点击置真,再点一次才真正丢弃改动。
    pub conflict_reload_armed: bool,
    /// T10:用户「保留我的修改」后记下的磁盘 mtime 基线;下次保存覆盖前再次
    /// 校验磁盘是否又变了(变了则重新进入冲突态)。
    pub conflict_baseline: Option<SystemTime>,
}

impl PreviewTab {
    /// 该 tab 在 Phase A 是否需要 Flyfish wry webview。
    ///
    /// WebView 生命周期只读统一 backend；旧 viewer `Option` 不再参与正常
    /// 路由。唯一例外是原生加载进入 `Failed` 后，为保持 Phase A 用户行为不变，
    /// 状态机明确选择 Flyfish fallback。Phase D 落地正式 External/Unsupported
    /// 页面后再移除该兼容分支。
    pub fn hosts_webview(&self) -> bool {
        if matches!(self.backend_state, BackendState::Suspended)
            // T3/Failed:加载失败不再 host Flyfish,让统一 fallback 页(或
            // 错误条)真正可见,不被残留的原生子视图盖住。
            || self.backend_state.is_failed()
            || !matches!(self.kind, TabKind::File(_))
        {
            return false;
        }
        // 窗口化只读走 editor host(与 CodeMirror 同一 WebView 通道),不再另起
        // Flyfish webview。Markdown/HTML 的 Source 模式同理。
        if self.uses_windowed_editor() || self.uses_rendered_source_editor() {
            return false;
        }
        self.backend
            .as_ref()
            .is_some_and(|backend| backend.hosts_webview())
    }

    /// 该 tab 是否由**任一** WebView host 承载( Flyfish 渲染 / CodeMirror editor /
    /// vanilla-jsoneditor Tree )。用于"加载是否需要等 host 信号"的判定:凡有
    /// host 就不该在画像后立即 finish,须等 host 的 `ready`/`document_loaded`
    /// (T5/T6/T8)。纯 iced fallback(Unsupported/External/表格)返回 false。
    pub fn hosts_any_webview(&self) -> bool {
        self.hosts_webview() || self.uses_editor_host() || self.uses_json_editor()
    }

    pub fn current_mode(&self) -> Option<PreviewMode> {
        self.backend.as_ref().map(PreviewBackend::current_mode)
    }

    /// 后端是否只读(Code backend 的 `ReadOnly` 档;其余后端如 Tree/渲染按
    /// 各自语义,这里对非 Code 返回 false)。
    pub fn backend_read_only(&self) -> bool {
        match self.backend {
            Some(PreviewBackend::Code(CodeBackend {
                mode: CodeMode::ReadOnly,
                ..
            })) => true,
            // 流式视图只读(逐行虚拟化,不做就地编辑);Text 回退可编辑。
            Some(PreviewBackend::Streamed(StreamedBackend {
                mode: StreamedMode::Streamed,
                ..
            })) => true,
            _ => false,
        }
    }

    /// T6:该 tab 是否允许把 editor host 的正文写回磁盘。只读档 / 窗口化 /
    /// 有损编码(非法 UTF-8,`fetch().text()` 已丢字节)恒拒绝,防止把关
    /// 替换字符的文本覆盖原文件。
    pub fn can_save(&self) -> bool {
        !self.windowed
            && !self.backend_read_only()
            && !self.route.as_ref().is_some_and(|r| r.encoding_lossy)
    }

    /// T10 保存闸门:综合只读/有损(T6)与磁盘冲突判定。
    pub fn save_gate(&mut self) -> SaveGate {
        if !self.can_save() {
            return SaveGate::ReadOnly;
        }
        let TabKind::File(path) = &self.kind else {
            return SaveGate::Allow;
        };
        if self.conflict.is_some() {
            return SaveGate::Conflict;
        }
        // 已「保留我的修改」但磁盘自那以后又变了:重新进入冲突态并拒绝保存,
        // 避免在提示之后的新外部修改被静默覆盖(T10)。
        if let Some(baseline) = self.conflict_baseline {
            let current = super::disk_mtime(path).unwrap_or(std::time::UNIX_EPOCH);
            if current != baseline {
                self.conflict = Some(current);
                self.conflict_reload_armed = false;
                return SaveGate::Conflict;
            }
        }
        SaveGate::Allow
    }

    /// T10:把该 tab 标记为"脏 + 磁盘已变"的显式冲突态,记录当前磁盘 mtime 供
    /// 保存前再次校验。非文件 tab 或干净 tab 不动作。
    pub fn mark_disk_conflict(&mut self) -> bool {
        if !self.dirty {
            return false;
        }
        let mtime = match &self.kind {
            TabKind::File(p) => super::disk_mtime(p),
            _ => return false,
        };
        self.conflict = Some(mtime.unwrap_or(std::time::UNIX_EPOCH));
        self.conflict_reload_armed = false;
        true
    }

    pub fn uses_codemirror(&self) -> bool {
        codemirror_enabled()
            && !self.windowed
            && matches!(self.backend, Some(PreviewBackend::Code(_)))
    }

    /// 是否走 CodeMirror 编辑 host(含窗口化只读)。Rendered 的 **Source** 模式、
    /// JSON(含 JSONC/JSON5/JSONL/NDJSON)的 **Text** 模式在 feature 打开时走
    /// editor host;JSON 的 Tree 视图由 vanilla-jsoneditor host 承载(此时返回
    /// false)。
    pub fn uses_editor_host(&self) -> bool {
        if !codemirror_enabled() {
            return false;
        }
        match &self.backend {
            Some(PreviewBackend::Code(_)) => true,
            Some(PreviewBackend::Json(json)) => json.mode == JsonMode::Text,
            // 流式 JSONL/NDJSON(两种 mode)都由 code editor host 承载(T8):
            // Streamed 走窗口化只读,Text 走普通文本。
            Some(PreviewBackend::Streamed(_)) => true,
            Some(PreviewBackend::Rendered(r)) => r.mode == RenderedMode::Source,
            // CSV/TSV 的"原文"模式由 editor host 承载(网格仍原生)。
            Some(PreviewBackend::Tabular(t)) => t.mode == TabularMode::Text,
            _ => false,
        }
    }

    /// 是否走窗口化只读 editor host(大文件)。
    pub fn uses_windowed_editor(&self) -> bool {
        self.uses_editor_host() && self.windowed
    }

    /// Rendered(Markdown/HTML)在 feature 下切到 Source 源码模式 → 由 editor
    /// host 承载,Flyfish webview 须让位。
    pub fn uses_rendered_source_editor(&self) -> bool {
        self.uses_editor_host() && matches!(self.backend, Some(PreviewBackend::Rendered(_)))
    }

    /// 严格 JSON 的 Tree 视图是否改用 vanilla-jsoneditor host(对照期 feature)。
    /// **只限 `.json`**:JSONC/JSON5 含注释,vanilla-jsoneditor 不解析,仍走原生树。
    pub fn uses_json_editor(&self) -> bool {
        if !json_editor_enabled() {
            return false;
        }
        if !matches!(
            self.backend,
            Some(PreviewBackend::Json(JsonBackend {
                mode: JsonMode::Tree,
                ..
            }))
        ) {
            return false;
        }
        matches!(&self.kind, TabKind::File(p)
            if p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("json")))
    }

    /// 运行时容器种类(仅用于 Debug 输出,避免打印整个 TabularView)。
    pub fn runtime_kind(&self) -> &'static str {
        match self.runtime {
            PreviewRuntime::None => "none",
            PreviewRuntime::Tabular(TabularState::Loading) => "tabular:loading",
            PreviewRuntime::Tabular(TabularState::Ready(_)) => "tabular:ready",
            PreviewRuntime::Windowed(_) => "windowed",
        }
    }

    /// 取表格加载态/就绪态(`TabularState`)。
    pub fn tabular_state(&self) -> Option<&TabularState> {
        match &self.runtime {
            PreviewRuntime::Tabular(state) => Some(state),
            _ => None,
        }
    }

    pub fn tabular_state_mut(&mut self) -> Option<&mut TabularState> {
        match &mut self.runtime {
            PreviewRuntime::Tabular(state) => Some(state),
            _ => None,
        }
    }

    /// 取已就绪的表格网格视图。
    pub fn tabular_view(&self) -> Option<&crate::tabular::TabularView> {
        match &self.runtime {
            PreviewRuntime::Tabular(TabularState::Ready(view)) => Some(view),
            _ => None,
        }
    }

    pub fn tabular_view_mut(&mut self) -> Option<&mut crate::tabular::TabularView> {
        match &mut self.runtime {
            PreviewRuntime::Tabular(TabularState::Ready(view)) => Some(view),
            _ => None,
        }
    }

    /// 取窗口化运行时元数据。
    pub fn window_runtime(&self) -> Option<&WindowedRuntime> {
        match &self.runtime {
            PreviewRuntime::Windowed(w) => Some(w),
            _ => None,
        }
    }

    pub fn window_runtime_mut(&mut self) -> Option<&mut WindowedRuntime> {
        match &mut self.runtime {
            PreviewRuntime::Windowed(w) => Some(w),
            _ => None,
        }
    }

    /// 窗口化稀疏行索引(就绪后才有)。
    pub fn window_index(&self) -> Option<&Arc<crate::preview::LineIndex>> {
        self.window_runtime().and_then(|w| w.index.as_ref())
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
            let has_native_viewer = self.runtime.is_resident();
            if has_native_viewer {
                debug_assert!(
                    !self.hosts_webview(),
                    "持有原生 viewer 的 backend 不应再 host webview (tab {:?})",
                    self.title
                );
            }
            // T2:backend 描述与 runtime 容器必须自洽——表格 backend 必须挂
            // Tabular runtime;Code backend 不得挂 Tabular runtime。
            match backend.kind() {
                PreviewKind::Tabular => debug_assert!(
                    matches!(self.runtime, PreviewRuntime::Tabular(_)),
                    "Tabular backend 必须挂 Tabular runtime (tab {:?})",
                    self.title
                ),
                PreviewKind::Code => debug_assert!(
                    !matches!(self.runtime, PreviewRuntime::Tabular(_)),
                    "Code backend 不应挂 Tabular runtime (tab {:?})",
                    self.title
                ),
                _ => {}
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
            .field("runtime", &self.runtime_kind())
            .field("pending_jump_line", &self.pending_jump_line)
            .field("route", &self.route)
            .field("backend", &self.backend)
            .field("backend_state", &self.backend_state)
            .field("load_stage", &self.load_state.stage)
            .field("web_revision", &self.web_revision)
            .field("web_selection", &self.web_selection)
            .field("web_selected_text", &self.web_selected_text)
            .field("web_viewport", &self.web_viewport)
            .field("web_error", &self.web_error)
            .field("conflict", &self.conflict.is_some())
            .finish()
    }
}

/// T10 保存闸门判定结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveGate {
    /// 可以写盘。
    Allow,
    /// 只读 / 窗口化 / 有损编码(T6),拒绝。
    ReadOnly,
    /// 存在未处理的磁盘冲突,拒绝并提示用户选择。
    Conflict,
}

/// 表格 tab 的加载态。`Loading` = 首次打开该文件、或懒加载某个 sheet 期间
/// 在后台线程跑(见 `crate::tabular::load`/`load_sheet`),完成后经
/// `Message::TabularLoaded` 落回 `Ready`；失败由统一 `BackendState::Failed`
/// 承载并提供重试/外部打开降级，不会无限停留在 Loading。
pub enum TabularState {
    Loading,
    Ready(crate::tabular::TabularView),
}

/// 一个 tab 的**运行时状态容器**(T2)。
///
/// 职责边界:backend(见 `backend.rs`)只回答"该用什么看"(路由描述,无句柄);
/// runtime 只回答"当前加载到什么状态"。两者是唯一真相,不得再用平行 `Option`
/// 字段猜 viewer 类型。
///
/// 真实 `wry::WebView` 句柄仍由平台 pool 持有,不进业务状态;需要 resident
/// metadata(host 类型/可见性/document revision)时读 `backend` +
/// `backend_state` + tab 上的 `web_*` 镜像字段。
pub enum PreviewRuntime {
    /// 没有运行时对象:CodeMirror / vanilla-jsoneditor / Flyfish / External /
    /// Unsupported 的"加载到什么状态"由 `backend_state` 与 `web_*` 镜像表达。
    None,
    /// 表格:首次解析 / 懒加载 sheet 的加载态,或已就绪的网格视图。
    Tabular(TabularState),
    /// 窗口化只读大文件:稀疏行索引与窗口元数据。
    Windowed(WindowedRuntime),
}

impl PreviewRuntime {
    /// 是否持有一个"原生 viewer"(表格网格 / 窗口化索引)。持有时不应再 host
    /// Flyfish webview(见 `PreviewTab::hosts_webview` 与一致性断言)。
    pub fn is_resident(&self) -> bool {
        matches!(
            self,
            PreviewRuntime::Tabular(_) | PreviewRuntime::Windowed(_)
        )
    }
}

/// 窗口化只读 viewer 的运行时元数据(T2 从 `PreviewTab` 的散落字段归入)。
#[derive(Debug, Clone, Default)]
pub struct WindowedRuntime {
    /// 稀疏行索引(后台建好后回填);外部变更/重载会清空使其失效。
    pub index: Option<Arc<crate::preview::LineIndex>>,
    /// 最近一次读取窗口的全局起始行(诊断/恢复用)。
    pub window_start_line: u32,
    /// 最近一次读取的窗口是否因字节上限被截断(超长单行)。
    pub truncated: bool,
    /// 索引/窗口加载错误(局部,不等同 backend `Failed`)。
    pub error: Option<String>,
}

/// T11:一次完整视图状态快照(cursor / selection / top_line / folds)。持久化、
/// 资源淘汰前序列化、物化后恢复共用同一形状,保证三者字段一致。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ViewStateRestore {
    pub cursor: Option<crate::preview::TextPosition>,
    pub selection: Option<crate::preview::TextRange>,
    pub top_line: Option<u32>,
    pub folds: Vec<crate::preview::FoldRange>,
}

impl ViewStateRestore {
    /// 是否为空(无 cursor/selection/top_line/folds)→ 无需恢复命令。
    pub fn is_empty(&self) -> bool {
        self.cursor.is_none()
            && self.selection.is_none()
            && self.top_line.is_none()
            && self.folds.is_empty()
    }
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

/// T10 冲突条占的纵向高度(逻辑像素)。与 Find 条同款:webview 是原生子视图、
/// 不听 iced 绘制顺序,画冲突条时必须显式把 webview 矩形下推这一高度。
pub(crate) const PREVIEW_CONFLICT_BAR_HEIGHT: f32 = 44.0;

/// 只读大文件档的搜索会话——⌘F 在这类 tab 上不打开 `FindState`(内存线性
/// 扫描,大文件上代价不可接受),而是打开这个,走
/// `preview::large_text::stream_search` 的磁盘流式扫描。`line_no` 是 1-based
/// (见 `SearchHit` 文档)。
///
/// T9:一次查询携带 `generation`;新提交/关闭会置位 `cancel`(共享给后台
/// `spawn_blocking` 任务)并作废旧 generation,只有匹配的结果才允许回灌,
/// 避免"快速连续输入时旧结果覆盖新结果"。
#[derive(Debug, Clone)]
pub struct LargeFileSearch {
    pub tab_id: usize,
    pub query: String,
    pub hits: Vec<crate::extensions::search::SearchHit>,
    pub current: usize,
    /// 是否有查询在途(搜索条据此显示小尺寸 math_curve loading)。
    pub running: bool,
    /// 本次查询的 generation;异步结果必须与当前值一致才被接受。
    pub generation: u64,
    /// 整个文件的匹配总数(可能远大于 `hits.len()`;≥ 上限即被截断)。
    pub total_matches: u64,
    /// 命中列表是否因上限被截断。
    pub truncated: bool,
    /// 搜索失败信息(局部错误,不把整个 preview 置 Failed)。
    pub error: Option<String>,
    /// 已扫描字节/总字节(展示用;`total == 0` 表示未知)。
    pub progress: Option<crate::preview::PreviewLoadProgress>,
    /// 取消信号:置位后后台任务在下一个分段检查点退出。`Arc` 与任务共享。
    pub cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Default for LargeFileSearch {
    fn default() -> Self {
        Self {
            tab_id: 0,
            query: String::new(),
            hits: Vec::new(),
            current: 0,
            running: false,
            generation: 0,
            total_matches: 0,
            truncated: false,
            error: None,
            progress: None,
            cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
}

impl LargeFileSearch {
    /// 作废在途查询:置位取消并清掉错误(新查询即将开始时调用)。
    pub fn cancel_inflight(&mut self) {
        self.cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
        self.cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.error = None;
    }
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
    /// 待下发给 CodeMirror editor webview 的命令队列(`tab_id`, 命令)。
    /// `window_events` 每帧(同 `apply_pending_preview_find` 节奏)取走并
    /// `evaluate_script` 注入;Agent reveal/select 与外部 reload 用它。
    pub(crate) pending_editor_commands: Vec<(usize, EditorCommand)>,
    /// "保存后再关闭"的 CodeMirror tab id 列表。Rust 侧不持有编辑器全文,
    /// 关闭 dirty tab 不能直接落盘,得先向 host 下发 `SaveDocument`,待
    /// host 回 `save_requested` 真正落盘后再移除 tab。`pending_editor_commands`
    /// 里对应的那条 `SaveDocument` 已在关 tab 那一刻排队,本列表只记"这条
    /// tab 的保存回来后要把 tab 关掉"。
    pub(crate) pending_close: Vec<usize>,
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
            pending_editor_commands: Vec::new(),
            pending_close: Vec::new(),
            blank_info: None,
            blank_info_in_flight: false,
        }
    }
}
