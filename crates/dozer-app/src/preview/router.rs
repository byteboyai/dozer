//! 单一预览路由(文件预览重构 Phase A + T5)。
//!
//! `classify_preview` 是"这个文件该用哪个 backend 看"的唯一决策点。用一份
//! 可解释的路由替换散落在各处的 `is_editable_extension` /
//! `prefers_rendered_preview` / `is_tabular_extension` 组合判断,并让每个新
//! tab 都带上 `PreviewRoute`(含可展示的 reason)。
//!
//! T5 起路由**不再是 Phase A 的逐项对齐**:文件名注册表(`Dockerfile`/
//! `Makefile`/`LICENSE*`/`.env`/`.gitignore`/`.dockerignore`)优先于扩展名
//! fallback(仍受内容安全检查约束);未知 UTF-8 文本进 Code、未知二进制落
//! 安全 fallback、空文件按可编辑纯文本;`.svg` 默认图像渲染并提供 XML 源码
//! 切换。`RouteReason` 区分文件名规则/扩展名规则/内容探测/用户 mode。

use std::fmt;
use std::path::Path;

use super::file_profile::{ContentKind, FileProfile};
use super::native_editor::{
    filename_code_rule, is_editable_extension, prefers_rendered_preview, wry_toggle_eligible,
};

/// 后端大类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewKind {
    /// 代码/纯文本 -> CodeMirror(迁移前为老 iced editor)。
    Code,
    /// 渲染型(Markdown/HTML/SVG/图片/PDF/媒体等)。
    Rendered,
    /// 结构化 JSON(普通 Tree + Text)。
    Json,
    /// 表格(CSV/TSV/XLSX/...)。
    Tabular,
    /// 交给外部应用/系统默认应用打开。
    External,
    /// 无内部 viewer,给出可解释降级。
    Unsupported,
}

/// backend 内的视图模式(可切换),不再通过同时持有两套完整 viewer 表达。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewMode {
    Code,
    Rendered,
    /// 渲染型 backend 里的源码模式(Markdown/HTML)。
    Source,
    /// JSON Tree。
    Tree,
    /// JSON/表格的原文文本模式。
    Text,
    Tabular,
    External,
    Unsupported,
}

impl PreviewMode {
    /// 宽容解析(持久化恢复用):未知字符串回退 `None`,由调用方安全落回
    /// 路由默认值,不 panic。
    pub fn from_persisted(s: &str) -> Option<Self> {
        match s {
            "code" => Some(Self::Code),
            "rendered" => Some(Self::Rendered),
            "source" => Some(Self::Source),
            "tree" => Some(Self::Tree),
            "text" => Some(Self::Text),
            "tabular" => Some(Self::Tabular),
            "external" => Some(Self::External),
            "unsupported" => Some(Self::Unsupported),
            _ => None,
        }
    }
}

/// 路由结果自带的、可展示给用户/诊断的降级说明。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteReason {
    /// 表格扩展名认领。
    TabularExtension,
    /// 普通 JSON 扩展名认领(含 json5/jsonc/jsonl/ndjson)。
    JsonTreeExtension,
    /// Markdown/HTML/SVG 等"可渲染但也可切源码"的扩展名。
    RenderedExtension(&'static str),
    /// 语法高亮器认识的代码扩展名。
    CodeExtension,
    /// 文件名注册表认领(`Dockerfile`/`Makefile`/`LICENSE*`/`.env`/`.gitignore`
    /// 等;T5)。`&str` 是命中的规则名,供诊断。
    FilenameRule(&'static str),
    /// 已知的媒体/文档类型(图片/PDF/Office)。
    KnownMediaExtension,
    /// 压缩包类,T1 起走统一外部打开 fallback 页。
    ArchiveFallback,
    /// 未知扩展名且内容像二进制 → 安全 fallback(T1 页)。
    ContentBinaryFallback,
    /// 未知扩展名但内容探测为文本 → Code(T5)。
    ContentTextProbe,
    /// 空文件(按可编辑纯文本处理,除非扩展名命中专用 viewer)。
    EmptyFile,
    /// 用户持久化的 mode 覆盖了默认值。
    PersistedMode,
}

impl fmt::Display for RouteReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TabularExtension => write!(f, "表格扩展名"),
            Self::JsonTreeExtension => write!(f, "JSON 树扩展名"),
            Self::RenderedExtension(e) => write!(f, "渲染扩展名 .{e}"),
            Self::CodeExtension => write!(f, "代码扩展名"),
            Self::FilenameRule(rule) => write!(f, "文件名规则 {rule}"),
            Self::KnownMediaExtension => write!(f, "媒体/文档扩展名"),
            Self::ArchiveFallback => write!(f, "压缩包"),
            Self::ContentBinaryFallback => write!(f, "未知二进制"),
            Self::ContentTextProbe => write!(f, "未知文本(内容探测)"),
            Self::EmptyFile => write!(f, "空文件"),
            Self::PersistedMode => write!(f, "用户持久化模式"),
        }
    }
}

/// 一个文件的完整路由描述。
#[derive(Debug, Clone, PartialEq)]
pub struct PreviewRoute {
    pub kind: PreviewKind,
    pub default_mode: PreviewMode,
    pub alternate_modes: Vec<PreviewMode>,
    pub reason: RouteReason,
}

impl PreviewRoute {
    /// 该路由是否支持某个模式(默认或可切换)。
    pub fn supports(&self, mode: PreviewMode) -> bool {
        mode == self.default_mode || self.alternate_modes.contains(&mode)
    }
}

/// 唯一路由决策。`persisted_mode` 只在当前 route 支持时才覆盖默认值;失效
/// 的旧 mode 安全落回默认。`capabilities` 预留给后续按预算降级(Phase C),
/// Phase A 不改变既有行为,故不使用。
pub fn classify_preview(
    path: &Path,
    profile: &FileProfile,
    _capabilities: &crate::capabilities::ClientCapabilities,
    persisted_mode: Option<PreviewMode>,
) -> PreviewRoute {
    let (kind, reason) = classify_kind(path, profile);
    let (default_mode, alternate_modes) = default_modes(path, kind);

    let (default_mode, reason) = match persisted_mode {
        Some(m) if m == default_mode || alternate_modes.contains(&m) => {
            (m, RouteReason::PersistedMode)
        }
        _ => (default_mode, reason),
    };

    PreviewRoute {
        kind,
        default_mode,
        alternate_modes,
        reason,
    }
}

/// 路由 kind 判定(T5:文件名注册表优先、未知文本进 Code、未知二进制安全
/// fallback、空文件按可编辑纯文本)。
fn classify_kind(path: &Path, profile: &FileProfile) -> (PreviewKind, RouteReason) {
    // 文件名注册表优先于扩展名 fallback,但仍受内容安全检查约束:命中规则
    // 但内容含 NUL/二进制时不得当作文本打开,落安全 fallback。
    if let Some((rule, _syntax)) = filename_code_rule(path) {
        if profile.content_kind == ContentKind::Binary {
            return (PreviewKind::Unsupported, RouteReason::ContentBinaryFallback);
        }
        return (PreviewKind::Code, RouteReason::FilenameRule(rule));
    }
    if crate::tabular::is_tabular_extension(path) {
        return (PreviewKind::Tabular, RouteReason::TabularExtension);
    }
    if crate::preview::native_editor::is_json_family_extension(path) {
        // JSON 家族统一走 `PreviewKind::Json`:严格 `.json` 给 Tree/Text 双视图,
        // json5/jsonc/jsonl/ndjson 一律只给 CodeMirror 文本(见 `default_modes`)。
        return (PreviewKind::Json, RouteReason::JsonTreeExtension);
    }
    if prefers_rendered_preview(path) {
        return (
            PreviewKind::Rendered,
            RouteReason::RenderedExtension(rendered_ext(path)),
        );
    }
    if is_editable_extension(path) {
        return (PreviewKind::Code, RouteReason::CodeExtension);
    }
    if is_archive_extension(path) {
        // T1:压缩包走统一外部打开 fallback 页。
        return (PreviewKind::External, RouteReason::ArchiveFallback);
    }
    if is_known_media_extension(path) {
        return (PreviewKind::Rendered, RouteReason::KnownMediaExtension);
    }
    // 未知扩展名/无扩展名:按内容探测决定——文本进 Code,二进制落安全 fallback,
    // 空文件按可编辑纯文本(除非扩展名已命中上面的专用 viewer)。
    match profile.content_kind {
        ContentKind::Empty => (PreviewKind::Code, RouteReason::EmptyFile),
        ContentKind::Binary => (PreviewKind::Unsupported, RouteReason::ContentBinaryFallback),
        ContentKind::Text => (PreviewKind::Code, RouteReason::ContentTextProbe),
    }
}

fn default_modes(path: &Path, kind: PreviewKind) -> (PreviewMode, Vec<PreviewMode>) {
    match kind {
        PreviewKind::Code => (PreviewMode::Code, Vec::new()),
        PreviewKind::Rendered => {
            if wry_toggle_eligible(path) {
                (PreviewMode::Rendered, vec![PreviewMode::Source])
            } else {
                (PreviewMode::Rendered, Vec::new())
            }
        }
        PreviewKind::Json => {
            // 严格 `.json` 树/文本双视图;JSONC/JSON5/JSONL/NDJSON 只给 CodeMirror
            // 文本(vanilla-jsoneditor 不解析注释;JSONL 逐行 JSON 也不是单个文档)。
            if json_extension(path).as_str() == "json" {
                (PreviewMode::Tree, vec![PreviewMode::Text])
            } else {
                (PreviewMode::Text, Vec::new())
            }
        }
        PreviewKind::Tabular => {
            if is_csv_like(path) {
                (PreviewMode::Tabular, vec![PreviewMode::Text])
            } else {
                (PreviewMode::Tabular, Vec::new())
            }
        }
        PreviewKind::External => (PreviewMode::External, Vec::new()),
        PreviewKind::Unsupported => (PreviewMode::Unsupported, Vec::new()),
    }
}

fn json_extension(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

fn rendered_ext(path: &Path) -> &'static str {
    match json_extension(path).as_str() {
        "md" => "md",
        "markdown" => "markdown",
        "html" => "html",
        "htm" => "htm",
        _ => "rendered",
    }
}

fn is_csv_like(path: &Path) -> bool {
    matches!(json_extension(path).as_str(), "csv" | "tsv")
}

/// 已知媒体/文档扩展名:图片、PDF、Office、音频、视频、字体。统一走 Flyfish
/// 渲染。`.svg` 不在其列——它由 `prefers_rendered_preview` 认领以提供 XML
/// 源码切换(T5)。
pub fn is_known_media_extension(path: &Path) -> bool {
    matches!(
        json_extension(path).as_str(),
        "png"
            | "jpg"
            | "jpeg"
            | "gif"
            | "webp"
            | "bmp"
            | "ico"
            | "tif"
            | "tiff"
            | "pdf"
            | "doc"
            | "docx"
            | "ppt"
            | "pptx"
            | "mp3"
            | "wav"
            | "ogg"
            | "flac"
            | "m4a"
            | "aac"
            | "mp4"
            | "mov"
            | "webm"
            | "avi"
            | "mkv"
            | "woff"
            | "woff2"
            | "ttf"
            | "otf"
    )
}

/// 压缩包类扩展名。
pub fn is_archive_extension(path: &Path) -> bool {
    matches!(
        json_extension(path).as_str(),
        "zip" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "7z" | "rar"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::{HardwareCapabilities, estimate_capabilities};
    use crate::preview::file_profile::analyze;
    use std::path::PathBuf;

    fn caps() -> crate::capabilities::ClientCapabilities {
        estimate_capabilities(HardwareCapabilities {
            total_memory_bytes: 16 * 1024 * 1024 * 1024,
            available_memory_at_start_bytes: 8 * 1024 * 1024 * 1024,
            physical_cpu_count: 8,
            logical_cpu_count: 16,
        })
    }

    fn route(path: &str, bytes: &[u8]) -> PreviewRoute {
        let profile = analyze(bytes, None, bytes.len() as u64, None);
        classify_preview(&PathBuf::from(path), &profile, &caps(), None)
    }

    #[test]
    fn source_files_route_to_code() {
        for p in ["main.rs", "app.py", "index.ts", "Cargo.toml", "ci.yaml"] {
            let r = route(p, b"hello\n");
            assert_eq!(r.kind, PreviewKind::Code, "{p}");
            assert_eq!(r.default_mode, PreviewMode::Code, "{p}");
            assert!(r.alternate_modes.is_empty(), "{p}");
        }
    }

    #[test]
    fn gitignore_has_no_extension_but_routes_to_code() {
        let r = route(".gitignore", b"target/\n");
        assert_eq!(r.kind, PreviewKind::Code);
        assert_eq!(r.reason, RouteReason::FilenameRule("ignore"));
    }

    #[test]
    fn markdown_and_html_route_to_rendered_with_source_alternate() {
        for p in ["README.md", "notes.markdown", "page.html", "index.htm"] {
            let r = route(p, b"# hi\n");
            assert_eq!(r.kind, PreviewKind::Rendered, "{p}");
            assert_eq!(r.default_mode, PreviewMode::Rendered, "{p}");
            assert_eq!(r.alternate_modes, vec![PreviewMode::Source], "{p}");
        }
    }

    #[test]
    fn plain_text_config_routes_to_code() {
        for p in ["notes.txt", "run.log", "nginx.conf", "a.ini", "b.cfg"] {
            assert_eq!(route(p, b"x\n").kind, PreviewKind::Code, "{p}");
        }
    }

    #[test]
    fn tabular_extensions() {
        for p in ["a.csv", "b.tsv", "c.xlsx", "d.xls", "e.ods"] {
            let r = route(p, b"a,b\n1,2\n");
            assert_eq!(r.kind, PreviewKind::Tabular, "{p}");
            assert_eq!(r.default_mode, PreviewMode::Tabular, "{p}");
        }
        // CSV/TSV 可切原文,XLSX 不可。
        assert_eq!(
            route("a.csv", b"a,b\n").alternate_modes,
            vec![PreviewMode::Text]
        );
        assert!(route("c.xlsx", b"\0\0").alternate_modes.is_empty());
    }

    #[test]
    fn json_family() {
        let r = route("data.json", b"{\"a\":1}\n");
        assert_eq!(r.kind, PreviewKind::Json);
        assert_eq!(r.default_mode, PreviewMode::Tree);
        assert_eq!(r.alternate_modes, vec![PreviewMode::Text]);

        for p in ["config.jsonc", "app.json5", "events.jsonl", "rows.ndjson"] {
            let r = route(p, b"{}\n");
            assert_eq!(r.kind, PreviewKind::Json, "{p}");
            assert_eq!(r.default_mode, PreviewMode::Text, "{p}");
        }
    }

    #[test]
    fn jsonl_and_ndjson_are_text_like_json5() {
        for p in ["events.jsonl", "rows.ndjson"] {
            let r = route(p, b"{}\n{}\n");
            assert_eq!(r.kind, PreviewKind::Json, "{p}");
            assert_eq!(r.default_mode, PreviewMode::Text, "{p}");
            assert!(r.alternate_modes.is_empty(), "{p}");
            assert_eq!(r.reason, RouteReason::JsonTreeExtension, "{p}");
        }
    }

    #[test]
    fn media_and_documents_render() {
        for p in ["photo.PNG", "doc.pdf", "deck.pptx", "clip.mp4", "icon.svg"] {
            let r = route(p, b"\x89PNG\0");
            assert_eq!(r.kind, PreviewKind::Rendered, "{p}");
        }
    }

    #[test]
    fn archives_are_external_fallback() {
        for p in ["a.zip", "b.tar.gz"] {
            let r = route(p, b"PK\x03\x04");
            assert_eq!(r.kind, PreviewKind::External, "{p}");
            assert_eq!(r.reason, RouteReason::ArchiveFallback, "{p}");
        }
    }

    #[test]
    fn unknown_binary_is_unsupported_fallback() {
        let r = route("mystery.bin", b"\x01\x02\0\x03");
        assert_eq!(r.kind, PreviewKind::Unsupported);
        assert_eq!(r.reason, RouteReason::ContentBinaryFallback);
    }

    #[test]
    fn unknown_text_routes_to_code_by_content_probe() {
        let r = route("mystery.weird", b"just some text\n");
        assert_eq!(r.kind, PreviewKind::Code);
        assert_eq!(r.default_mode, PreviewMode::Code);
        assert_eq!(r.reason, RouteReason::ContentTextProbe);
    }

    #[test]
    fn empty_file_routes_to_editable_code() {
        let r = route("empty.xyz", b"");
        assert_eq!(r.kind, PreviewKind::Code);
        assert_eq!(r.reason, RouteReason::EmptyFile);
    }

    #[test]
    fn empty_file_with_dedicated_viewer_extension_keeps_viewer() {
        assert_eq!(route("empty.json", b"").kind, PreviewKind::Json);
        assert_eq!(route("empty.csv", b"").kind, PreviewKind::Tabular);
        assert_eq!(route("empty.md", b"").kind, PreviewKind::Rendered);
    }

    #[test]
    fn filename_registry_routes_to_code() {
        for (p, rule) in [
            ("Dockerfile", "dockerfile"),
            ("Makefile", "makefile"),
            ("GNUmakefile", "makefile"),
            ("LICENSE", "license"),
            ("LICENSE.md", "license"),
            ("NOTICE", "license"),
            (".env", "env"),
            (".env.local", "env"),
            (".gitignore", "ignore"),
            (".dockerignore", "ignore"),
        ] {
            let r = route(p, b"all:\n\t@echo ok\n");
            assert_eq!(r.kind, PreviewKind::Code, "{p}");
            assert_eq!(r.default_mode, PreviewMode::Code, "{p}");
            assert_eq!(r.reason, RouteReason::FilenameRule(rule), "{p}");
        }
    }

    #[test]
    fn filename_registry_still_subject_to_binary_check() {
        // 名叫 Dockerfile 但内容含 NUL:不得当文本打开。
        let r = route("Dockerfile", b"FROM scratch\0binary");
        assert_eq!(r.kind, PreviewKind::Unsupported);
        assert_eq!(r.reason, RouteReason::ContentBinaryFallback);
    }

    #[test]
    fn svg_renders_by_default_with_xml_source_alternate() {
        let r = route("icon.svg", b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>\n");
        assert_eq!(r.kind, PreviewKind::Rendered);
        assert_eq!(r.default_mode, PreviewMode::Rendered);
        assert_eq!(r.alternate_modes, vec![PreviewMode::Source]);
        assert_eq!(r.reason, RouteReason::RenderedExtension("rendered"));
    }

    #[test]
    fn filename_rules_are_case_insensitive() {
        assert_eq!(
            route("DOCKERFILE", b"FROM x\n").reason,
            RouteReason::FilenameRule("dockerfile")
        );
        assert_eq!(
            route("makefile", b"all:\n").reason,
            RouteReason::FilenameRule("makefile")
        );
        assert_eq!(
            route("License", b"text\n").reason,
            RouteReason::FilenameRule("license")
        );
    }

    #[test]
    fn double_extensions_use_last_extension() {
        // 名称含点:归档用最后一段 `.gz`,代码用最后一段 `.js`。
        assert_eq!(
            route("backup.tar.gz", b"\x1f\x8b").kind,
            PreviewKind::External
        );
        assert_eq!(route("app.min.js", b"var x=1;\n").kind, PreviewKind::Code);
    }

    #[test]
    fn persisted_mode_overrides_only_when_supported() {
        let path = PathBuf::from("data.json");
        let profile = analyze(b"{}", None, 2, None);
        let r = classify_preview(&path, &profile, &caps(), Some(PreviewMode::Text));
        assert_eq!(r.default_mode, PreviewMode::Text);
        assert_eq!(r.reason, RouteReason::PersistedMode);

        // 失效旧 mode(Code 不被 Json 支持)→ 安全落回默认 Tree。
        let r = classify_preview(&path, &profile, &caps(), Some(PreviewMode::Code));
        assert_eq!(r.default_mode, PreviewMode::Tree);
        assert_eq!(r.reason, RouteReason::JsonTreeExtension);
    }

    #[test]
    fn every_rule_carries_displayable_reason() {
        let r = route("main.rs", b"x");
        assert!(!r.reason.to_string().is_empty());
        assert!(r.supports(PreviewMode::Code));
    }
}
