//! 统一 backend 描述与生命周期状态机(文件预览重构 Phase A)。
//!
//! `PreviewTab` 不再靠 `editor/tabular/json_tree` 多个 `Option` 组合来"猜"
//! 当前后端;每个 tab 都带一份 [`PreviewBackend`] 描述与 [`BackendState`]。
//! 迁移期旧的 viewer 字段仍保留为 adapter(见 `preview/state.rs`),本模块
//! 先建立类型与合法转换,供渲染/预算/Agent 逐步改读。

// Phase A 建立**完整**状态机与描述(含 Suspended/Queued/Failed、失败 fallback、
// IsolatedHtml/Source 等),消费方在 B/C/D 阶段接线;此处显式允许这些前瞻性
// API 在迁移期暂未被构造/调用,避免 dead_code 噪声掩盖真实漏点。
#![allow(dead_code)]

use std::path::Path;

use super::router::{PreviewKind, PreviewMode, PreviewRoute, RouteReason};

/// 一个 backend 的生命周期状态。合法转换见 [`BackendState::can_transition_to`]。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendState {
    /// 只恢复了 tab 壳,内容未加载(启动恢复的默认态)。
    Suspended,
    /// 已排队等待加载(受并发/预算限制)。
    Queued,
    /// 正在加载。
    Loading,
    /// 已就绪。
    Ready,
    /// 加载失败,带可解释错误。
    Failed(PreviewError),
}

/// 可解释、可重试的加载失败。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewError {
    pub message: String,
    pub retryable: bool,
}

impl PreviewError {
    pub fn new(message: impl Into<String>, retryable: bool) -> Self {
        Self {
            message: message.into(),
            retryable,
        }
    }
}

impl BackendState {
    /// 状态机合法转换。非法转换必须由调用方显式拒绝,防止出现
    /// "Failed 里还在 Loading""没加载过就 Ready"这类不自洽态。
    pub fn can_transition_to(&self, next: &BackendState) -> bool {
        use BackendState::*;
        match (self, next) {
            (Suspended, Queued | Loading) => true,
            (Queued, Loading | Suspended) => true,
            (Loading, Ready | Failed(_) | Suspended) => true,
            (Ready, Suspended | Queued | Loading) => true,
            (Failed(_), Queued | Loading | Suspended) => true,
            // 同态重入不算迁移,视为 no-op(合法)。
            (a, b) if a == b => true,
            _ => false,
        }
    }

    /// 尝试迁移;不改变自身,只报告是否合法。调用方自行决定如何处理非法态。
    pub fn try_transition(&mut self, next: BackendState) -> bool {
        if self.can_transition_to(&next) {
            *self = next;
            true
        } else {
            false
        }
    }

    pub fn is_ready(&self) -> bool {
        matches!(self, BackendState::Ready)
    }

    pub fn is_failed(&self) -> bool {
        matches!(self, BackendState::Failed(_))
    }
}

/// 代码/文本 backend 描述。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeBackend {
    pub mode: CodeMode,
    pub language: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeMode {
    Editable,
    ReadOnly,
}

/// 渲染型 backend(Markdown/HTML/图片/PDF/媒体)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedBackend {
    pub renderer: RenderedRenderer,
    pub mode: RenderedMode,
    /// 可切源码时的语言(Markdown/HTML);纯媒体为 `None`。
    pub source_language: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderedRenderer {
    /// 走 Flyfish wry 页面(含图片/PDF/媒体/Markdown)。
    Flyfish,
    /// 隔离 host 的 HTML 渲染(Phase D 收敛目标;Phase A 仍是 file://)。
    IsolatedHtml,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderedMode {
    Rendered,
    Source,
}

/// JSON backend 描述。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonBackend {
    pub mode: JsonMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonMode {
    Tree,
    Text,
}

/// 表格 backend 描述。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabularBackend {
    pub format: TabularFormat,
    pub mode: TabularMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabularMode {
    Grid,
    Text,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabularFormat {
    Csv,
    Tsv,
    Workbook,
}

/// 流式/窗口化 backend 描述(JSONL/NDJSON)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamedBackend {
    pub reason: RouteReason,
    pub mode: PreviewMode,
}

/// 外部打开 backend 描述。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalBackend {
    pub reason: RouteReason,
}

/// 无内部 viewer 的 backend 描述。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsupportedBackend {
    pub reason: RouteReason,
}

/// `PreviewTab` 的唯一后端描述。Phase A 只承载"描述",运行时 viewer 句柄
/// 仍由旧的 `editor/tabular/json_tree` 字段持有(adapter)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreviewBackend {
    Code(CodeBackend),
    Rendered(RenderedBackend),
    Json(JsonBackend),
    Tabular(TabularBackend),
    Streamed(StreamedBackend),
    External(ExternalBackend),
    Unsupported(UnsupportedBackend),
}

impl PreviewBackend {
    /// 由路由结果构造 backend 描述。`read_only` 来自文件画像/大小策略,
    /// Phase A 由旧分档给出。
    pub fn from_route(route: &PreviewRoute, path: &Path, read_only: bool) -> Self {
        match route.kind {
            PreviewKind::Code => PreviewBackend::Code(CodeBackend {
                mode: if read_only {
                    CodeMode::ReadOnly
                } else {
                    CodeMode::Editable
                },
                language: super::native_editor::extension_to_syntax(path),
            }),
            PreviewKind::Rendered => {
                let source_language = if route.supports(PreviewMode::Source) {
                    Some(super::native_editor::extension_to_syntax(path))
                } else {
                    None
                };
                PreviewBackend::Rendered(RenderedBackend {
                    renderer: RenderedRenderer::Flyfish,
                    mode: RenderedMode::Rendered,
                    source_language,
                })
            }
            PreviewKind::Json => PreviewBackend::Json(JsonBackend {
                mode: match route.default_mode {
                    PreviewMode::Text => JsonMode::Text,
                    _ => JsonMode::Tree,
                },
            }),
            PreviewKind::Tabular => PreviewBackend::Tabular(TabularBackend {
                format: tabular_format(path),
                mode: match route.default_mode {
                    PreviewMode::Text => TabularMode::Text,
                    _ => TabularMode::Grid,
                },
            }),
            PreviewKind::Streamed => PreviewBackend::Streamed(StreamedBackend {
                reason: route.reason,
                mode: route.default_mode,
            }),
            PreviewKind::External => PreviewBackend::External(ExternalBackend {
                reason: route.reason,
            }),
            PreviewKind::Unsupported => PreviewBackend::Unsupported(UnsupportedBackend {
                reason: route.reason,
            }),
        }
    }

    pub fn kind(&self) -> PreviewKind {
        match self {
            PreviewBackend::Code(_) => PreviewKind::Code,
            PreviewBackend::Rendered(_) => PreviewKind::Rendered,
            PreviewBackend::Json(_) => PreviewKind::Json,
            PreviewBackend::Tabular(_) => PreviewKind::Tabular,
            PreviewBackend::Streamed(_) => PreviewKind::Streamed,
            PreviewBackend::External(_) => PreviewKind::External,
            PreviewBackend::Unsupported(_) => PreviewKind::Unsupported,
        }
    }

    /// 该 backend 在 Phase A 是否需要一个 Flyfish wry webview。与改动前
    /// `desired_webviews` 的判据逐项对齐:渲染类、以及压缩包/未知二进制的
    /// **兜底**都靠 Flyfish 显示;Code/Json/Tabular/Streamed 走原生渲染。
    ///
    /// Phase D 落地"未知/压缩包 -> External/Unsupported 的正式 fallback"后,
    /// External/Unsupported 将不再 host webview。
    pub fn hosts_webview(&self) -> bool {
        matches!(
            self,
            PreviewBackend::Rendered(RenderedBackend {
                mode: RenderedMode::Rendered,
                ..
            }) | PreviewBackend::External(_)
                | PreviewBackend::Unsupported(_)
        )
    }

    /// 当前实际显示的模式。持久化必须写这个值，而不是路由初始默认值。
    pub fn current_mode(&self) -> PreviewMode {
        match self {
            Self::Code(_) => PreviewMode::Code,
            Self::Rendered(rendered) => match rendered.mode {
                RenderedMode::Rendered => PreviewMode::Rendered,
                RenderedMode::Source => PreviewMode::Source,
            },
            Self::Json(json) => match json.mode {
                JsonMode::Tree => PreviewMode::Tree,
                JsonMode::Text => PreviewMode::Text,
            },
            Self::Tabular(tabular) => match tabular.mode {
                TabularMode::Grid => PreviewMode::Tabular,
                TabularMode::Text => PreviewMode::Text,
            },
            Self::Streamed(streamed) => streamed.mode,
            Self::External(_) => PreviewMode::External,
            Self::Unsupported(_) => PreviewMode::Unsupported,
        }
    }

    /// 失败态可用的能力描述(Phase A 仅描述,UI 接入后续 phase):
    /// 可重试、可转纯文本只读、可外部打开。
    pub fn failure_fallbacks(&self) -> FailureFallbacks {
        FailureFallbacks {
            retry: true,
            plain_text_read_only: matches!(
                self,
                PreviewBackend::Rendered(_)
                    | PreviewBackend::External(_)
                    | PreviewBackend::Unsupported(_)
            ),
            external_open: true,
        }
    }
}

/// Failed 状态下的可选动作描述。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FailureFallbacks {
    pub retry: bool,
    pub plain_text_read_only: bool,
    pub external_open: bool,
}

fn tabular_format(path: &Path) -> TabularFormat {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "csv" => TabularFormat::Csv,
        "tsv" => TabularFormat::Tsv,
        _ => TabularFormat::Workbook,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::{HardwareCapabilities, estimate_capabilities};
    use crate::preview::file_profile::analyze;
    use crate::preview::router::classify_preview;
    use std::path::PathBuf;

    fn caps() -> crate::capabilities::ClientCapabilities {
        estimate_capabilities(HardwareCapabilities {
            total_memory_bytes: 16 * 1024 * 1024 * 1024,
            available_memory_at_start_bytes: 8 * 1024 * 1024 * 1024,
            physical_cpu_count: 8,
            logical_cpu_count: 16,
        })
    }

    fn backend(path: &str, bytes: &[u8]) -> PreviewBackend {
        let profile = analyze(bytes, None, bytes.len() as u64, None);
        let p = PathBuf::from(path);
        let route = classify_preview(&p, &profile, &caps(), None);
        PreviewBackend::from_route(&route, &p, false)
    }

    #[test]
    fn backend_kind_matches_route() {
        assert_eq!(backend("main.rs", b"x").kind(), PreviewKind::Code);
        assert_eq!(backend("README.md", b"x").kind(), PreviewKind::Rendered);
        assert_eq!(backend("a.json", b"{}").kind(), PreviewKind::Json);
        assert_eq!(backend("a.csv", b"a,b").kind(), PreviewKind::Tabular);
        assert_eq!(backend("a.jsonl", b"{}").kind(), PreviewKind::Streamed);
        assert_eq!(backend("a.zip", b"PK").kind(), PreviewKind::External);
    }

    #[test]
    fn webview_hosting_matches_legacy_predicate() {
        assert!(!backend("main.rs", b"x").hosts_webview());
        assert!(backend("README.md", b"x").hosts_webview());
        assert!(!backend("a.json", b"{}").hosts_webview());
        assert!(!backend("a.csv", b"a,b").hosts_webview());
        assert!(!backend("a.jsonl", b"{}").hosts_webview());
        // 压缩包与未知二进制在 Phase A 仍靠 Flyfish 兜底。
        assert!(backend("a.zip", b"PK").hosts_webview());
        assert!(backend("mystery.bin", b"\0\0\0").hosts_webview());
    }

    #[test]
    fn rendered_source_mode_does_not_host_webview() {
        let mut backend = backend("README.md", b"x");
        let PreviewBackend::Rendered(rendered) = &mut backend else {
            panic!()
        };
        rendered.mode = RenderedMode::Source;
        assert_eq!(backend.current_mode(), PreviewMode::Source);
        assert!(!backend.hosts_webview());
    }

    #[test]
    fn code_language_and_readonly() {
        let profile = analyze(b"fn main(){}", None, 11, None);
        let p = PathBuf::from("main.rs");
        let route = classify_preview(&p, &profile, &caps(), None);
        let b = PreviewBackend::from_route(&route, &p, true);
        let PreviewBackend::Code(code) = b else {
            panic!("应是 Code");
        };
        assert_eq!(code.language, "rust");
        assert_eq!(code.mode, CodeMode::ReadOnly);
    }

    #[test]
    fn rendered_source_language_only_when_toggleable() {
        let PreviewBackend::Rendered(md) = backend("README.md", b"x") else {
            panic!()
        };
        assert_eq!(md.source_language.as_deref(), Some("markdown"));
        let PreviewBackend::Rendered(png) = backend("a.png", b"\x89PNG\0") else {
            panic!()
        };
        assert!(png.source_language.is_none());
    }

    #[test]
    fn tabular_format_classified() {
        let PreviewBackend::Tabular(csv) = backend("a.csv", b"a,b") else {
            panic!()
        };
        assert_eq!(csv.format, TabularFormat::Csv);
        let PreviewBackend::Tabular(x) = backend("a.xlsx", b"\0") else {
            panic!()
        };
        assert_eq!(x.format, TabularFormat::Workbook);
    }

    #[test]
    fn tabular_text_mode_from_persisted() {
        let profile = analyze(b"a,b\n1,2\n", None, 8, None);
        let p = PathBuf::from("a.csv");
        let route = classify_preview(&p, &profile, &caps(), Some(PreviewMode::Text));
        let b = PreviewBackend::from_route(&route, &p, false);
        assert_eq!(b.current_mode(), PreviewMode::Text);
        // 默认(Grid)仍是 Tabular。
        let route2 = classify_preview(&p, &profile, &caps(), None);
        assert_eq!(
            PreviewBackend::from_route(&route2, &p, false).current_mode(),
            PreviewMode::Tabular
        );
    }

    #[test]
    fn state_transitions_are_legal_only_where_allowed() {
        use BackendState::*;
        assert!(Suspended.can_transition_to(&Queued));
        assert!(Queued.can_transition_to(&Loading));
        assert!(Loading.can_transition_to(&Ready));
        assert!(Loading.can_transition_to(&Failed(PreviewError::new("x", true))));
        assert!(Failed(PreviewError::new("x", true)).can_transition_to(&Loading));
        assert!(Ready.can_transition_to(&Suspended));
        // 非法:没加载过直接 Ready / Loading 里再 Loading 之外的乱序。
        assert!(!Suspended.can_transition_to(&Ready));
        assert!(!Queued.can_transition_to(&Ready));
    }

    #[test]
    fn try_transition_applies_only_legal_moves() {
        let mut s = BackendState::Suspended;
        assert!(!s.try_transition(BackendState::Ready));
        assert_eq!(s, BackendState::Suspended, "非法转换不应改变状态");
        assert!(s.try_transition(BackendState::Queued));
        assert_eq!(s, BackendState::Queued);
    }

    #[test]
    fn failed_state_exposes_external_open_fallback() {
        let b = backend("mystery.bin", b"\0\0");
        let fb = b.failure_fallbacks();
        assert!(fb.retry);
        assert!(fb.external_open);
    }
}
