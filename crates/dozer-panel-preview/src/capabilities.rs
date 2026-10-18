//! 产品按后端开关预览能力(spec §5.3)。Digger 第一批只开一部分后端,其余页签降级。

use std::path::Path;

use dozer_preview_core::backend::{PreviewBackend, RenderedRenderer};
use dozer_preview_core::router::is_known_media_extension;

/// 各后端是否启用。`Default` 全关,`all()` 全开。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PreviewCapabilities {
    pub code: bool,
    pub rendered: bool,
    pub json: bool,
    pub streamed: bool,
    pub tabular: bool,
    pub plantuml: bool,
    pub pdf: bool,
    pub media: bool,
}

impl PreviewCapabilities {
    pub fn all() -> Self {
        Self {
            code: true,
            rendered: true,
            json: true,
            streamed: true,
            tabular: true,
            plantuml: true,
            pdf: true,
            media: true,
        }
    }
}

fn is_pdf(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
}

/// 该后端在当前能力下是否可用。`External` / `Unsupported` 本身就是降级态,恒为可用。
/// PDF / 媒体 / PlantUML 是 `Rendered` 的子类型,需要路径或渲染器才能判定。
pub fn backend_enabled(backend: &PreviewBackend, path: &Path, caps: &PreviewCapabilities) -> bool {
    match backend {
        PreviewBackend::Code(_) => caps.code,
        PreviewBackend::Rendered(r) => {
            if !caps.rendered {
                return false;
            }
            if r.renderer == RenderedRenderer::PlantUml {
                return caps.plantuml;
            }
            if is_pdf(path) {
                return caps.pdf;
            }
            if is_known_media_extension(path) {
                return caps.media;
            }
            true
        }
        PreviewBackend::Json(_) => caps.json,
        PreviewBackend::Streamed(_) => caps.streamed,
        PreviewBackend::Tabular(_) => caps.tabular,
        PreviewBackend::External(_) | PreviewBackend::Unsupported(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dozer_preview_core::router::{PreviewKind, PreviewMode, PreviewRoute, RouteReason};

    #[test]
    fn capabilities_default_is_all_off_and_all_is_all_on() {
        let d = PreviewCapabilities::default();
        assert!(!(d.code || d.rendered || d.json || d.streamed));
        assert!(!(d.tabular || d.plantuml || d.pdf || d.media));
        let a = PreviewCapabilities::all();
        assert!(a.code && a.rendered && a.json && a.streamed);
        assert!(a.tabular && a.plantuml && a.pdf && a.media);
    }

    fn route(kind: PreviewKind) -> PreviewRoute {
        PreviewRoute {
            kind,
            default_mode: PreviewMode::Code,
            alternate_modes: Vec::new(),
            reason: RouteReason::TabularExtension,
            content_is_text: true,
            encoding_lossy: false,
            encoding_utf16: false,
        }
    }

    /// 穷举 `match`:`PreviewKind` 新增变体时这里编译即报错,提醒补 `backend_enabled`。
    fn only_enabled(kind: PreviewKind) -> PreviewCapabilities {
        let mut c = PreviewCapabilities::default();
        match kind {
            PreviewKind::Code => c.code = true,
            PreviewKind::Rendered => c.rendered = true,
            PreviewKind::Json => c.json = true,
            PreviewKind::Streamed => c.streamed = true,
            PreviewKind::Tabular => c.tabular = true,
            PreviewKind::External | PreviewKind::Unsupported => {}
        }
        c
    }

    #[test]
    fn backend_enabled_covers_every_preview_kind_variant() {
        let kinds = [
            (PreviewKind::Code, "a.rs"),
            (PreviewKind::Rendered, "a.md"),
            (PreviewKind::Json, "a.json"),
            (PreviewKind::Streamed, "a.jsonl"),
            (PreviewKind::Tabular, "a.csv"),
            (PreviewKind::External, "a.bin"),
            (PreviewKind::Unsupported, "a.bin"),
        ];
        for (kind, name) in kinds {
            let path = Path::new(name);
            let backend = PreviewBackend::from_route(&route(kind), path, false);
            assert_eq!(backend.kind(), kind);
            let off = PreviewCapabilities::default();
            let degraded = matches!(kind, PreviewKind::External | PreviewKind::Unsupported);
            assert_eq!(
                backend_enabled(&backend, path, &off),
                degraded,
                "{kind:?} off"
            );
            assert!(
                backend_enabled(&backend, path, &only_enabled(kind)),
                "{kind:?} on"
            );
            assert!(backend_enabled(&backend, path, &PreviewCapabilities::all()));
        }
    }

    #[test]
    fn rendered_subtypes_need_their_own_switch() {
        let mut caps = PreviewCapabilities {
            rendered: true,
            ..Default::default()
        };
        let rendered = |name: &str| {
            PreviewBackend::from_route(&route(PreviewKind::Rendered), Path::new(name), false)
        };
        for (name, ok) in [
            ("a.md", true),
            ("a.pdf", false),
            ("a.png", false),
            ("a.puml", false),
        ] {
            assert_eq!(
                backend_enabled(&rendered(name), Path::new(name), &caps),
                ok,
                "{name}"
            );
        }
        caps.pdf = true;
        caps.media = true;
        caps.plantuml = true;
        for name in ["a.pdf", "a.png", "a.puml"] {
            assert!(
                backend_enabled(&rendered(name), Path::new(name), &caps),
                "{name}"
            );
        }
    }
}
