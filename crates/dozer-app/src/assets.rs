// crates/dozer-app/src/assets.rs
//! `dozer://` 自定义协议应答(P1d,纯函数,headless 全测)。
//!
//! 布局(设计 §2):
//! - `dozer://flyfish/<path>`          → vendored Flyfish dist 静态字节;
//! - `dozer://flyfish/__file__/<abs>`  → 本地文件字节,仅白名单(用户显式
//!   打开过的路径)。文件端点与 host 页同命名空间——wry 自定义协议在
//!   macOS 的 Origin 是 `dozer://flyfish`,跨命名空间 fetch 被 CORS 拦;
//!   URL 保留原始扩展名,Flyfish 靠它选择渲染管线。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub struct ProtocolReply {
    pub status: u16,
    pub mime: &'static str,
    pub body: Vec<u8>,
}

fn not_found() -> ProtocolReply {
    ProtocolReply {
        status: 404,
        mime: "text/plain",
        body: b"not found".to_vec(),
    }
}

/// 一期 dev 形态:资产直接从仓库源码树读(cargo run 即用,host.html 可
/// 热改)。打包分发(app bundle 内资源目录)是后续任务,不在 P1d。
pub fn assets_root() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/flyfish"))
}

pub fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = bytes.get(i + 1..i + 3)?;
            let hi = (hex[0] as char).to_digit(16)?;
            let lo = (hex[1] as char).to_digit(16)?;
            out.push((hi * 16 + lo) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn mime_for(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "html" => "text/html",
        "js" | "mjs" => "text/javascript",
        "css" => "text/css",
        "wasm" => "application/wasm",
        "json" => "application/json",
        "md" | "markdown" => "text/markdown",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "txt" => "text/plain",
        _ => "application/octet-stream",
    }
}

pub fn handle_protocol(
    assets_root: &Path,
    allowed: &HashSet<PathBuf>,
    review_data: Option<&str>,
    uri: &str,
) -> ProtocolReply {
    // 剥离 scheme 与 query;只服务 flyfish/review-trace 两个命名空间。
    let Some(rest) = uri.strip_prefix("dozer://") else {
        return not_found();
    };
    let rest = rest.split('?').next().unwrap_or(rest);

    // 审阅面板 trace 页面(2026-08-21):页面本身是编译期内嵌的静态资源,
    // 不走磁盘;数据端点回显调用方注入的当前审阅内容快照——没有快照
    // (还没加载过审阅内容)时 404。
    if let Some(path) = rest.strip_prefix("review-trace/") {
        return match path {
            "host.html" => ProtocolReply {
                status: 200,
                mime: "text/html",
                body: include_str!("review_trace.html").as_bytes().to_vec(),
            },
            "data.json" => match review_data {
                Some(json) => ProtocolReply {
                    status: 200,
                    mime: "application/json",
                    body: json.as_bytes().to_vec(),
                },
                None => not_found(),
            },
            _ => not_found(),
        };
    }

    let Some(path) = rest.strip_prefix("flyfish/") else {
        return not_found();
    };

    // 本地文件端点:__file__/<百分号编码的绝对路径>(编码保留 '/')。
    if let Some(encoded) = path.strip_prefix("__file__") {
        let Some(decoded) = percent_decode(encoded) else {
            return not_found();
        };
        let file = PathBuf::from(decoded);
        if !allowed.contains(&file) {
            return not_found();
        }
        return match std::fs::read(&file) {
            Ok(body) => ProtocolReply {
                status: 200,
                mime: mime_for(&file),
                body,
            },
            Err(_) => not_found(),
        };
    }

    // vendored 资产:先逐段解码,再拒绝路径穿越——编码形态也拦得住:
    // %2e%2e 解码成 '..' 后才比较;%2F 解码出的 '/' 直接判拒。
    let mut full = assets_root.to_path_buf();
    for seg in path.split('/') {
        let Some(seg) = percent_decode(seg) else {
            return not_found();
        };
        if seg.is_empty() || seg == ".." || seg == "." || seg.contains('/') || seg.contains('\0') {
            return not_found();
        }
        full.push(seg);
    }
    match std::fs::read(&full) {
        Ok(body) => ProtocolReply {
            status: 200,
            mime: mime_for(&full),
            body,
        },
        Err(_) => not_found(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::fs;
    use std::path::PathBuf;

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dozer-assets-test-{}", std::process::id()));
        let _ = fs::create_dir_all(dir.join("sub"));
        fs::write(dir.join("host.html"), b"<html>").unwrap();
        fs::write(dir.join("sub").join("a.js"), b"js").unwrap();
        dir
    }

    #[test]
    fn review_trace_host_html_serves_embedded_page_regardless_of_review_data() {
        let root = scratch();
        let r = handle_protocol(
            &root,
            &HashSet::new(),
            None,
            "dozer://review-trace/host.html?_r=1",
        );
        assert_eq!(r.status, 200);
        assert_eq!(r.mime, "text/html");
        assert!(!r.body.is_empty());
    }

    #[test]
    fn review_trace_data_json_echoes_injected_snapshot() {
        let root = scratch();
        let r = handle_protocol(
            &root,
            &HashSet::new(),
            Some(r#"[{"Human":{"text":"hi"}}]"#),
            "dozer://review-trace/data.json",
        );
        assert_eq!(r.status, 200);
        assert_eq!(r.mime, "application/json");
        assert_eq!(r.body, br#"[{"Human":{"text":"hi"}}]"#);
    }

    #[test]
    fn review_trace_data_json_404_when_no_snapshot() {
        let root = scratch();
        let r = handle_protocol(
            &root,
            &HashSet::new(),
            None,
            "dozer://review-trace/data.json",
        );
        assert_eq!(r.status, 404);
    }

    #[test]
    fn review_trace_unknown_subpath_404() {
        let root = scratch();
        let r = handle_protocol(&root, &HashSet::new(), None, "dozer://review-trace/nope");
        assert_eq!(r.status, 404);
    }

    #[test]
    fn serves_vendored_asset_with_mime() {
        let root = scratch();
        let r = handle_protocol(&root, &HashSet::new(), None, "dozer://flyfish/host.html");
        assert_eq!((r.status, r.mime), (200, "text/html"));
        assert_eq!(r.body, b"<html>");
        let r = handle_protocol(&root, &HashSet::new(), None, "dozer://flyfish/sub/a.js?v=1");
        assert_eq!(
            (r.status, r.mime),
            (200, "text/javascript"),
            "query 应被剥离"
        );
    }

    #[test]
    fn rejects_traversal_and_unknown() {
        let root = scratch();
        assert_eq!(
            handle_protocol(
                &root,
                &HashSet::new(),
                None,
                "dozer://flyfish/../etc/passwd"
            )
            .status,
            404
        );
        assert_eq!(
            handle_protocol(&root, &HashSet::new(), None, "dozer://other/x").status,
            404
        );
        assert_eq!(
            handle_protocol(&root, &HashSet::new(), None, "dozer://flyfish/nope.js").status,
            404
        );
    }

    #[test]
    fn file_endpoint_requires_allowlist() {
        let root = scratch();
        let f = root.join("victim.md");
        fs::write(&f, b"# hi").unwrap();
        let uri = format!(
            "dozer://flyfish/__file__{}",
            f.to_string_lossy().replace(' ', "%20")
        );
        // 不在白名单 → 404
        assert_eq!(
            handle_protocol(&root, &HashSet::new(), None, &uri).status,
            404
        );
        // 在白名单 → 200 + 按扩展名给 mime
        let mut allowed = HashSet::new();
        allowed.insert(f.clone());
        let r = handle_protocol(&root, &allowed, None, &uri);
        assert_eq!((r.status, r.mime), (200, "text/markdown"));
        assert_eq!(r.body, b"# hi");
    }

    #[test]
    fn rejects_percent_encoded_traversal() {
        let root = scratch();
        for uri in [
            "dozer://flyfish/%2e%2e/etc/passwd",
            "dozer://flyfish/%2e%2e/%2e%2e/etc/passwd",
            "dozer://flyfish/a%2F..%2Fb.js",
            "dozer://flyfish/sub%2F..%2F..%2Fx",
        ] {
            assert_eq!(
                handle_protocol(&root, &HashSet::new(), None, uri).status,
                404,
                "{uri}"
            );
        }
    }

    #[test]
    fn percent_decode_roundtrip() {
        assert_eq!(percent_decode("%2Fa%20b").as_deref(), Some("/a b"));
        assert_eq!(percent_decode("%E4%BD%A0").as_deref(), Some("你"));
        assert_eq!(percent_decode("plain").as_deref(), Some("plain"));
        assert!(percent_decode("%GG").is_none());
    }
}

pub mod clipboard_image;
pub mod fonts;
