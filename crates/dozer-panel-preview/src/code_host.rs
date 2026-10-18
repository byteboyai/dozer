//! CodeMirror editor host 的绑定与 URL(文件预览重构 Phase B Task 2)。
//!
//! 一个 editor webview 绑定 `(project_id, panel, tab_id, path)`;`document_id`
//! 由 project/tab 派生且**不来自 JS**——JS 不能自报任意路径,所有读写都由
//! Rust 侧按绑定校验(见 `webview_protocol::HostBinding`)。

// host 的运行时接线(desired_webviews/runtime IPC 分派)在后续步骤完成前,
// 本模块的部分 API 暂未被非测试代码调用;显式允许。
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use dozer_preview_core::resources::ViewerPanel;

/// `ws.preview`(Files)与 `ws.project_preview`(Project)是两个独立 `PreviewPane`,各自 `next_id`
/// 从 0 起数——两者的 webview 一旦同时进同一个 `webviews` 池,原始 id 会撞。给 `Project` 那一侧的
/// id 统一加这个偏移,`Files` 侧不动;按 id 反查 Project webview 的地方都要用同一个偏移量加/减。
pub const PROJECT_PREVIEW_ID_OFFSET: usize = 1_000_000;

/// Git Log 面板 diff webview 的固定单槽位 id(没有 tab 概念,任意时刻最多一个 diff webview)。
pub const GIT_LOG_DIFF_ID_OFFSET: usize = 3_000_000;

/// editor host 的等宽字号与行高倍数(产品的终端字体设定,基准值,不含全局 scale)。
/// 库不读任何全局:Dozer 取 `theme::terminal_font`,Digger 取自己的。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EditorFont {
    pub size: f32,
    pub line_height_factor: f32,
}

impl Default for EditorFont {
    fn default() -> Self {
        Self {
            size: 14.0,
            line_height_factor: 1.6,
        }
    }
}

/// editor host 页面 URL 前缀。`sync_webview_pool` 据此把 editor webview 与
/// flyfish webview 分流(不同注入脚本/IPC 路由)。
pub const EDITOR_URL_PREFIX: &str = "dozer://editor/";

/// JSON host(vanilla-jsoneditor)页面 URL 前缀。
pub const JSON_EDITOR_URL_PREFIX: &str = "dozer://json-editor/";

/// Tabular 预览 host(ag-grid)页面 URL 前缀。
pub const TABULAR_URL_PREFIX: &str = "dozer://tabular/";

/// 一个 editor webview 的归属绑定。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorHostBinding {
    pub project_id: i64,
    pub panel: ViewerPanel,
    pub tab_id: usize,
    pub path: PathBuf,
}

/// `ViewerPanel` → webview URL 里的 `panel=` token(Files 与 Project 两个预览面板
/// 共用同一个编辑器 host,靠它区分)。资源淘汰按 URL 里的 `panel=`/`tab=` 定位。
pub fn panel_token(panel: ViewerPanel) -> &'static str {
    match panel {
        ViewerPanel::Project => "project",
        ViewerPanel::GitLog => "gitlog",
        _ => "files",
    }
}

impl EditorHostBinding {
    pub fn new(project_id: i64, panel: ViewerPanel, tab_id: usize, path: PathBuf) -> Self {
        Self {
            project_id,
            panel,
            tab_id,
            path,
        }
    }

    /// 稳定 document id(project + tab 派生;tab 内容换文件时 tab_id 不变,
    /// 但 tab 会被重建/换绑,因此不需要含路径)。
    pub fn document_id(&self) -> String {
        format!("p{}-t{}", self.project_id, self.tab_id)
    }

    pub fn panel_token(&self) -> &'static str {
        panel_token(self.panel)
    }

    /// webview 池 key(= 本地 id + 面板偏移,与 flyfish 预览一致)。
    pub fn webview_id(&self) -> usize {
        match self.panel {
            ViewerPanel::Project => PROJECT_PREVIEW_ID_OFFSET + self.tab_id,
            ViewerPanel::GitLog => GIT_LOG_DIFF_ID_OFFSET,
            _ => self.tab_id,
        }
    }

    /// editor host 页面 URL。路径逐段百分号编码(保留 `/`);`theme` 跟随全局
    /// 配色方案,`ro` / `lang` 给首屏直接渲染用,Rust 侧仍会以绑定为准校验。
    pub fn url(&self, theme: &str, read_only: bool, locale: &str, font: EditorFont) -> String {
        // `fs`/`lh`:终端同款等宽字号与行高倍数(基准值,不含全局 scale——
        // WebView 的 pageZoom 已承担缩放),让编辑器与 PTY 观感一致。
        format!(
            "{EDITOR_URL_PREFIX}index.html?p={}&theme={}&locale={}&ro={}&doc={}&proj={}&panel={}&tab={}&lang={}&fs={}&lh={}",
            encode_path(&self.path),
            theme,
            locale,
            if read_only { 1 } else { 0 },
            super::encode_component(&self.document_id()),
            self.project_id,
            self.panel_token(),
            self.tab_id,
            super::extension_to_syntax(&self.path),
            font.size,
            font.line_height_factor,
        )
    }

    /// editor host 页面 URL,diff 模式:不带 `p=`(不触发文件读取——正文由
    /// `SetDiffDocument` 命令推送),恒 `ro=1`。`path` 仍参与 `document_id`/
    /// 资源预算估算(见 `runtime.rs` 的 `fs::metadata` 兜底),只是不会被
    /// JS 端 `fetch()`。
    pub fn diff_url(&self, theme: &str, locale: &str, font: EditorFont) -> String {
        format!(
            "{EDITOR_URL_PREFIX}index.html?mode=diff&theme={}&locale={}&ro=1&doc={}&proj={}&panel={}&tab={}&lang={}&fs={}&lh={}",
            theme,
            locale,
            super::encode_component(&self.document_id()),
            self.project_id,
            self.panel_token(),
            self.tab_id,
            super::extension_to_syntax(&self.path),
            font.size,
            font.line_height_factor,
        )
    }

    /// JSON host(vanilla-jsoneditor)URL:tree/text 由命令驱动,主题经 class,
    /// 不需要 lang/fs。
    pub fn json_url(&self, theme: &str, read_only: bool, locale: &str) -> String {
        format!(
            "{JSON_EDITOR_URL_PREFIX}index.html?p={}&theme={}&locale={}&ro={}&doc={}&proj={}&panel={}&tab={}",
            encode_path(&self.path),
            theme,
            locale,
            if read_only { 1 } else { 0 },
            super::encode_component(&self.document_id()),
            self.project_id,
            self.panel_token(),
            self.tab_id,
        )
    }

    /// Tabular host(ag-grid)URL:正文完全由命令推送(`Init`/`SetSchema`/
    /// `SetWindow`),host 不 fetch 任何文件,故不带 `fs=`/`lh=`(那是
    /// CodeMirror 专用的等宽字号参数——tabular 走系统默认字体,见
    /// CLAUDE.md"非代码/终端场景禁用等宽字体"裁决)。恒只读,不带 `ro=`
    /// 参数(host 本身不支持编辑,无需首屏协商)。
    pub fn tabular_url(&self, theme: &str, locale: &str) -> String {
        format!(
            "{TABULAR_URL_PREFIX}index.html?p={}&theme={}&locale={}&doc={}&proj={}&panel={}&tab={}",
            encode_path(&self.path),
            theme,
            locale,
            super::encode_component(&self.document_id()),
            self.project_id,
            self.panel_token(),
            self.tab_id,
        )
    }
}

/// 把 webview 池 key 反解成 `(panel, tab_id)`(IPC 回来时按 id 找绑定用)。
/// 越界/负数按 Files 处理(不可能出现,防御)。
pub fn panel_and_tab_from_webview_id(id: usize) -> (ViewerPanel, usize) {
    if id == GIT_LOG_DIFF_ID_OFFSET {
        (ViewerPanel::GitLog, 0)
    } else if id >= PROJECT_PREVIEW_ID_OFFSET {
        (ViewerPanel::Project, id - PROJECT_PREVIEW_ID_OFFSET)
    } else {
        (ViewerPanel::Files, id)
    }
}

/// URL 是否是 editor host(而不是 flyfish)。
pub fn is_editor_url(url: &str) -> bool {
    url.starts_with(EDITOR_URL_PREFIX)
}

/// URL 是否是 JSON tree/text host。
pub fn is_json_editor_url(url: &str) -> bool {
    url.starts_with(JSON_EDITOR_URL_PREFIX)
}

/// URL 是否是 Tabular(ag-grid)host。
pub fn is_tabular_url(url: &str) -> bool {
    url.starts_with(TABULAR_URL_PREFIX)
}

/// URL 是否是任一内部 host(CodeMirror / JSON / Tabular),用于运行期区分
/// "host WebView"与 Flyfish WebView(注入脚本/IPC 路由不同)。
pub fn is_host_url(url: &str) -> bool {
    is_editor_url(url) || is_json_editor_url(url) || is_tabular_url(url)
}

/// 严格 JSON 的 Tree 视图由 vanilla-jsoneditor host 承载(常开)。
pub fn json_editor_enabled() -> bool {
    true
}

/// CodeMirror editor host 已转默认常开(老 iced `CodeView` 已退役)。
pub fn codemirror_enabled() -> bool {
    true
}

/// Tabular Grid 视图已转 webview host(ag-grid),不留对照期,恒常开
/// (与 `codemirror_enabled()`/`json_editor_enabled()` 同一"始终开"风格)。
pub fn tabular_grid_host_enabled() -> bool {
    true
}

/// 路径逐段编码:保留 `/` 分隔符,其余按 RFC3986 严格编码(与 flyfish
/// `file_url` 同一手法)。
fn encode_path(path: &Path) -> String {
    path.to_string_lossy()
        .split('/')
        .map(super::encode_component)
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(panel: ViewerPanel, tab_id: usize) -> EditorHostBinding {
        EditorHostBinding::new(7, panel, tab_id, PathBuf::from("/repo/src/main.rs"))
    }

    #[test]
    fn document_id_is_derived_and_stable() {
        let b = binding(ViewerPanel::Files, 3);
        assert_eq!(b.document_id(), "p7-t3");
    }

    #[test]
    fn webview_id_applies_project_offset() {
        assert_eq!(binding(ViewerPanel::Files, 3).webview_id(), 3);
        assert_eq!(
            binding(ViewerPanel::Project, 3).webview_id(),
            PROJECT_PREVIEW_ID_OFFSET + 3
        );
    }

    #[test]
    fn webview_id_inverts_back_to_panel_and_tab() {
        assert_eq!(panel_and_tab_from_webview_id(3), (ViewerPanel::Files, 3));
        assert_eq!(
            panel_and_tab_from_webview_id(PROJECT_PREVIEW_ID_OFFSET + 9),
            (ViewerPanel::Project, 9)
        );
    }

    #[test]
    fn url_encodes_path_and_carries_binding() {
        let b = EditorHostBinding::new(
            7,
            ViewerPanel::Project,
            3,
            PathBuf::from("/repo/a b/main.rs"),
        );
        let url = b.url("dark", true, "en", EditorFont::default());
        assert!(is_editor_url(&url));
        assert!(url.contains("panel=project"));
        assert!(url.contains("tab=3"));
        assert!(url.contains("ro=1"));
        assert!(url.contains("theme=dark"));
        assert!(url.contains("lang=rust"));
        assert!(url.contains("main.rs"));
        // 空格段被编码、'/' 保留。
        assert!(url.contains("a%20b/main.rs"));
        assert!(!url.contains("a b/main.rs"));
    }

    #[test]
    fn flyfish_url_is_not_editor_url() {
        assert!(!is_editor_url("dozer://flyfish/host.html?p=/x"));
    }

    #[test]
    fn json_editor_url_is_distinct() {
        let b = binding(ViewerPanel::Files, 3);
        let url = b.json_url("dark", true, "en");
        assert!(is_json_editor_url(&url));
        assert!(!is_editor_url(&url));
        assert!(url.contains("dozer://json-editor/index.html"));
        assert!(url.contains("ro=1"));
        assert!(url.contains("doc=p7-t3"));
    }

    #[test]
    fn tabular_url_is_distinct_and_always_read_only() {
        let b = binding(ViewerPanel::Files, 3);
        let url = b.tabular_url("dark", "en");
        assert!(is_tabular_url(&url));
        assert!(!is_editor_url(&url));
        assert!(!is_json_editor_url(&url));
        assert!(url.contains("dozer://tabular/index.html"));
        assert!(url.contains("doc=p7-t3"));
        assert!(url.contains("theme=dark"));
        // 不带 CodeMirror 专用的字号/行高参数。
        assert!(!url.contains("fs="));
        assert!(!url.contains("lh="));
    }

    #[test]
    fn is_host_url_covers_tabular() {
        let b = binding(ViewerPanel::Files, 3);
        assert!(is_host_url(&b.tabular_url("dark", "en")));
    }

    #[test]
    fn tabular_grid_host_always_enabled() {
        assert!(tabular_grid_host_enabled());
    }

    #[test]
    fn gitlog_panel_token_is_distinct() {
        assert_eq!(panel_token(ViewerPanel::GitLog), "gitlog");
        assert_eq!(panel_token(ViewerPanel::Files), "files");
        assert_eq!(panel_token(ViewerPanel::Project), "project");
    }

    #[test]
    fn gitlog_webview_id_applies_own_offset() {
        let b = EditorHostBinding::new(0, ViewerPanel::GitLog, 0, PathBuf::from("a.txt"));
        assert_eq!(b.webview_id(), GIT_LOG_DIFF_ID_OFFSET);
    }

    #[test]
    fn gitlog_webview_id_inverts_back() {
        assert_eq!(
            panel_and_tab_from_webview_id(GIT_LOG_DIFF_ID_OFFSET),
            (ViewerPanel::GitLog, 0)
        );
    }

    #[test]
    fn diff_url_carries_mode_and_gitlog_panel_token() {
        let b = EditorHostBinding::new(0, ViewerPanel::GitLog, 0, PathBuf::from("src/lib.rs"));
        let url = b.diff_url("dark", "en", EditorFont::default());
        assert!(is_editor_url(&url));
        assert!(url.contains("mode=diff"));
        assert!(url.contains("panel=gitlog"));
        assert!(url.contains("ro=1"));
        assert!(url.contains("theme=dark"));
        assert!(url.contains("lang=rust"));
    }
}
