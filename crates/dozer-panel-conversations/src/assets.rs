use serde::{Deserialize, Serialize};
pub const MAX_MESSAGE_BYTES: usize = 8 * 1024 * 1024;
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WebviewEnvelope<T> {
    pub protocol_version: u32,
    #[serde(default)]
    pub project_id: i64,
    #[serde(default)]
    pub panel: String,
    #[serde(default)]
    pub tab_id: usize,
    #[serde(default)]
    pub document_id: String,
    #[serde(default)]
    pub revision: u64,
    #[serde(default)]
    pub request_id: Option<String>,
    pub payload: T,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolError {
    TooLarge { bytes: usize },
    BadJson(String),
    UnknownPayload(String),
}

impl std::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge { bytes } => write!(f, "消息过大({bytes} 字节)"),
            Self::BadJson(e) => write!(f, "消息非合法 JSON: {e}"),
            Self::UnknownPayload(k) => write!(f, "未知 payload kind: {k}"),
        }
    }
}

/// review-trace(单槽、非 tab)host 报回的事件。只有一个变体——
/// review-trace 的失败已经是页面内可见的错误文案(见
/// `web/review-trace/src/main.tsx` 的 `RenderErrorBoundary`/`.catch`),
/// Rust 不需要单独知道"是不是失败了"才决定要不要显示,成功/失败/渲染
/// 异常三种终态都报同一个事件。不复用 `FlyfishEvent`(那个带
/// `Title`/`SearchState`/`Failed{recoverable}`,review-trace 没有这些
/// 交互,复用只会引入不必要的字段)。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReviewTraceEvent {
    DocumentLoaded,
}
pub fn parse_review_trace_event(
    raw: &str,
) -> Result<WebviewEnvelope<ReviewTraceEvent>, ProtocolError> {
    if raw.len() > MAX_MESSAGE_BYTES {
        return Err(ProtocolError::TooLarge { bytes: raw.len() });
    }
    let env: WebviewEnvelope<serde_json::Value> =
        serde_json::from_str(raw).map_err(|e| ProtocolError::BadJson(e.to_string()))?;
    let kind = env
        .payload
        .get("kind")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_default();
    let payload: ReviewTraceEvent = serde_json::from_value(env.payload)
        .map_err(|_| ProtocolError::UnknownPayload(kind.clone()))?;
    Ok(WebviewEnvelope {
        protocol_version: env.protocol_version,
        project_id: env.project_id,
        panel: env.panel,
        tab_id: env.tab_id,
        document_id: env.document_id,
        revision: env.revision,
        request_id: env.request_id,
        payload,
    })
}
pub fn serve_review_trace(
    path: &str,
    review_json: Option<&str>,
) -> Option<(&'static str, Vec<u8>)> {
    let (mime, bytes): (&str, &[u8]) = match path {
        "host.html" => (
            "text/html",
            include_bytes!("../assets/review-trace/host.html"),
        ),
        "review-trace.js" => (
            "text/javascript",
            include_bytes!("../assets/review-trace/review-trace.js"),
        ),
        "review-trace.css" => (
            "text/css",
            include_bytes!("../assets/review-trace/review-trace.css"),
        ),
        "data.json" => ("application/json", review_json?.as_bytes()),
        _ => return None,
    };
    Some((mime, bytes.to_vec()))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parse_review_trace_event_accepts_minimal_envelope() {
        let raw = r#"{"protocol_version":1,"payload":{"kind":"document_loaded"}}"#;
        let env = parse_review_trace_event(raw).expect("应解析成功");
        assert_eq!(env.payload, ReviewTraceEvent::DocumentLoaded);
        // 省略的信封字段(project_id/panel/tab_id/document_id/revision/
        // request_id)全部落到 `#[serde(default)]`,不要求 JS 侧提供——
        // review-trace 单槽非 tab,没有这些身份需要携带。
        assert_eq!(env.project_id, 0);
        assert_eq!(env.tab_id, 0);
    }
    #[test]
    fn parse_review_trace_event_rejects_unknown_kind() {
        let raw = r#"{"protocol_version":1,"payload":{"kind":"bogus"}}"#;
        assert!(parse_review_trace_event(raw).is_err());
    }
    #[test]
    fn parse_review_trace_event_rejects_oversized_message() {
        let huge = "x".repeat(MAX_MESSAGE_BYTES + 1);
        let raw = format!(
            r#"{{"protocol_version":1,"payload":{{"kind":"document_loaded","pad":"{huge}"}}}}"#
        );
        assert!(matches!(
            parse_review_trace_event(&raw),
            Err(ProtocolError::TooLarge { .. })
        ));
    }
}

#[cfg(test)]
mod bundle_tests {
    use std::path::Path;
    /// 提交的 review-trace 产物必须齐全(防止忘记 `npm run build` 就提交)。
    #[test]
    fn review_trace_bundle_assets_are_present() {
        let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/review-trace"));
        for f in ["host.html", "review-trace.js", "review-trace.css"] {
            let p = root.join(f);
            assert!(p.is_file(), "缺少 review-trace 产物 {f}: {}", p.display());
            assert!(
                std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0) > 0,
                "review-trace 产物为空: {f}"
            );
        }
    }
    /// 严格 CSP + 无网络:host.html 不得引用任何外部 URL,且带 `default-src 'none'`。
    #[test]
    fn review_trace_host_has_strict_csp_and_no_external_refs() {
        let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/review-trace"));
        let html = std::fs::read_to_string(root.join("host.html")).expect("读 host.html");
        assert!(
            html.contains("Content-Security-Policy"),
            "host.html 必须声明 CSP"
        );
        assert!(
            html.contains("default-src 'none'"),
            "CSP 必须以 default-src 'none' 起步"
        );
        assert!(html.contains("script-src 'self'"), "脚本仅 self");
        assert!(html.contains("connect-src 'self'"), "仅允许同源 fetch");
        assert!(
            !html.contains("http://") && !html.contains("https://"),
            "review-trace host.html 不得引用外部 URL(离线约束)"
        );
        assert!(html.contains("review-trace.js") && html.contains("review-trace.css"));
    }
    /// 打包产物不得泄漏本机绝对路径或 sourcemap 引用。
    #[test]
    fn review_trace_js_has_no_absolute_paths_or_sourcemap() {
        let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/review-trace"));
        let js = std::fs::read_to_string(root.join("review-trace.js")).expect("读 review-trace.js");
        assert!(!js.contains("sourceMappingURL"), "不应有 sourcemap 引用");
        assert!(
            !js.contains(env!("CARGO_MANIFEST_DIR")),
            "不应含源码树绝对路径"
        );
    }
}
