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

/// flyfish 静态资源的根目录。
///
/// 两种形态:
/// - 打包分发:可执行文件位于 `<Dozer AI Coder.app>/Contents/MacOS/dozer`,
///   资源随包放在 `Contents/Resources/flyfish`(`scripts/build-macos-app.sh`
///   打包时拷入)。从 `current_exe` 向上找 `.app` 边界定位 Resources。
/// - dev 形态(cargo run):直接从仓库源码树读,host.html 可热改。
pub fn assets_root() -> PathBuf {
    if let Some(resources) = std::env::current_exe()
        .ok()
        .and_then(|exe| bundle_resources_root(&exe))
    {
        return resources.join("flyfish");
    }
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/flyfish"))
}

/// 给定可执行文件路径,若它位于 macOS `.app` 包内(祖先目录名以 `.app`
/// 结尾),返回该包的 `Contents/Resources`;否则 `None`(dev/裸二进制)。
///
/// 纯函数、可测;不依赖 AppKit/objc2,仅用 `std::path::Path` 祖先遍历。
fn bundle_resources_root(exe: &Path) -> Option<PathBuf> {
    for ancestor in exe.ancestors() {
        if ancestor.extension().and_then(|e| e.to_str()) == Some("app") {
            return Some(ancestor.join("Contents").join("Resources"));
        }
    }
    None
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
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "txt" => "text/plain",
        _ => "application/octet-stream",
    }
}

/// vendored 静态资源服务:逐段百分号解码 + 拒绝路径穿越(编码形态也拦得住),
/// 从 `root` 下读文件。flyfish 与 editor 两个命名空间共用。
fn serve_vendored(root: &Path, encoded_path: &str) -> ProtocolReply {
    let mut full = root.to_path_buf();
    for seg in encoded_path.split('/') {
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

/// 本地文件端点 `__file__/<百分号编码的绝对路径>`:仅白名单内路径可读。
fn serve_allowlisted_file(encoded: &str, allowed: &HashSet<PathBuf>) -> ProtocolReply {
    let Some(decoded) = percent_decode(encoded) else {
        return not_found();
    };
    let file = PathBuf::from(decoded);
    if !allowed.contains(&file) {
        return not_found();
    }
    match std::fs::read(&file) {
        Ok(body) => {
            // UTF-16 文本(带 BOM)先转成 UTF-8:WebView 的 `fetch().text()`
            // 固定按 UTF-8 解码,喂原始 UTF-16 字节会整篇乱码(2026-09 用户
            // 报告 utf16le.rs 打开乱码)。转码只影响显示;该文件因 utf8:Invalid
            // 已按只读处理,不涉及保存回写。
            let body = match crate::preview::decode_utf16_to_utf8(&body) {
                Some(text) => text.into_bytes(),
                None => body,
            };
            ProtocolReply {
                status: 200,
                mime: mime_for(&file),
                body,
            }
        }
        Err(_) => not_found(),
    }
}

/// editor host 静态资源根 = flyfish 根的兄弟目录 `editor`(dev 与打包态同构:
/// `<assets>/flyfish` → `<assets>/editor` / `Resources/flyfish` →
/// `Resources/editor`)。
fn editor_root_for(flyfish_root: &Path) -> PathBuf {
    flyfish_root.with_file_name("editor")
}

/// review-trace host(会话审阅 trace 时间线)静态资源根 = flyfish 根的
/// 兄弟目录 `review-trace`。同 `editor_root_for`,dev 与打包态同构。
fn review_trace_root_for(flyfish_root: &Path) -> PathBuf {
    flyfish_root.with_file_name("review-trace")
}

/// usage-content host(用量面板内容侧图表)静态资源根 = flyfish 根的兄弟
/// 目录 `usage-content`。同 `editor_root_for`/`review_trace_root_for`。
fn usage_content_root_for(flyfish_root: &Path) -> PathBuf {
    flyfish_root.with_file_name("usage-content")
}

/// T7:`dozer://html/__file__/<abs>` 的读取闸门。比 editor 的"精确文件白名单"
/// 宽一点:允许**已打开文件所在目录子树**(相对资源 css/js/图片要能加载),
/// 但不允许跨出这些目录,且拒绝任何 `..` 分量 + 再 canonicalize 复核(防符号
/// 链接逃逸)。`allowed` 是应用维护的"用户显式打开过的文件"集合。
fn serve_html_file(encoded: &str, allowed: &HashSet<PathBuf>) -> ProtocolReply {
    let Some(decoded) = percent_decode(encoded) else {
        return not_found();
    };
    if decoded.split('/').any(|c| c == "..") {
        return not_found();
    }
    let file = PathBuf::from(&decoded);
    let exact = allowed.contains(&file);
    let under_root = |target: &Path| -> bool {
        allowed.iter().any(|a| {
            a.parent()
                .and_then(|root| std::fs::canonicalize(root).ok())
                .is_some_and(|root| target.starts_with(&root))
        })
    };
    let canon = match std::fs::canonicalize(&file) {
        Ok(c) => c,
        Err(_) => return not_found(),
    };
    if !exact && !under_root(&canon) {
        return not_found();
    }
    match std::fs::read(&file) {
        Ok(body) => ProtocolReply {
            status: 200,
            mime: mime_for(&file),
            body,
        },
        Err(_) => not_found(),
    }
}

/// JSON tree/text host(vanilla-jsoneditor)静态资源根 = flyfish 根的兄弟目录
/// `json-editor`。
fn json_editor_root_for(flyfish_root: &Path) -> PathBuf {
    flyfish_root.with_file_name("json-editor")
}

pub fn handle_protocol(
    assets_root: &Path,
    allowed: &HashSet<PathBuf>,
    review_data: Option<&str>,
    uri: &str,
) -> ProtocolReply {
    // 剥离 scheme 与 query;只服务 flyfish/review-trace/editor/json-editor/
    // html/usage-content 这些命名空间。
    let Some(rest) = uri.strip_prefix("dozer://") else {
        return not_found();
    };
    let rest = rest.split('?').next().unwrap_or(rest);

    // 审阅面板 trace 页面(2026-09-25 起 Preact 离线打包产物,从磁盘服务,
    // 同 editor/json-editor):数据端点回显调用方注入的当前审阅内容快照
    // ——没有快照(还没加载过审阅内容)时 404,判断顺序在 serve_vendored
    // 之前,不受影响。
    if let Some(path) = rest.strip_prefix("review-trace/") {
        if path == "data.json" {
            return match review_data {
                Some(json) => ProtocolReply {
                    status: 200,
                    mime: "application/json",
                    body: json.as_bytes().to_vec(),
                },
                None => not_found(),
            };
        }
        return serve_vendored(&review_trace_root_for(assets_root), path);
    }

    // usage-content host(用量面板内容侧图表):没有 data.json 特判——数据
    // 全靠 evaluate_script 推送,不走 fetch(见 protocol.rs)。
    if let Some(path) = rest.strip_prefix("usage-content/") {
        return serve_vendored(&usage_content_root_for(assets_root), path);
    }

    // editor host:页面/脚本/样式/字体从 editor 根服务;`__file__/<abs>` 复用
    // 同一份白名单读取当前预览文件。JS 不能自报任意路径(不入白名单即 404)。
    if let Some(path) = rest.strip_prefix("editor/") {
        if let Some(encoded) = path.strip_prefix("__file__") {
            return serve_allowlisted_file(encoded, allowed);
        }
        return serve_vendored(&editor_root_for(assets_root), path);
    }

    // JSON host(vanilla-jsoneditor):同 editor,独立 CSP/命名空间。
    if let Some(path) = rest.strip_prefix("json-editor/") {
        if let Some(encoded) = path.strip_prefix("__file__") {
            return serve_allowlisted_file(encoded, allowed);
        }
        return serve_vendored(&json_editor_root_for(assets_root), path);
    }

    // T7:HTML 隔离 host。页面编译期内嵌;`__file__` 走 `serve_html_file`
    // (允许已打开文件所在目录子树,供相对资源)。
    if let Some(path) = rest.strip_prefix("html/") {
        if path == "host.html" {
            return ProtocolReply {
                status: 200,
                mime: "text/html",
                body: include_str!("html_host.html").as_bytes().to_vec(),
            };
        }
        if let Some(encoded) = path.strip_prefix("__file__") {
            return serve_html_file(encoded, allowed);
        }
        return not_found();
    }

    let Some(path) = rest.strip_prefix("flyfish/") else {
        return not_found();
    };

    // 本地文件端点:__file__/<百分号编码的绝对路径>(编码保留 '/')。
    if let Some(encoded) = path.strip_prefix("__file__") {
        return serve_allowlisted_file(encoded, allowed);
    }

    serve_vendored(assets_root, path)
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

    /// 提交的 usage-content 产物必须齐全(防止忘记 `npm run build` 就提交)。
    #[test]
    fn usage_content_bundle_assets_are_present() {
        let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/usage-content"));
        for f in ["host.html", "usage-content.js", "usage-content.css"] {
            let p = root.join(f);
            assert!(p.is_file(), "缺少 usage-content 产物 {f}: {}", p.display());
            assert!(
                std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0) > 0,
                "usage-content 产物为空: {f}"
            );
        }
    }

    /// 严格 CSP、无 connect-src(没有 fetch 端点,数据全靠推送)、无网络。
    #[test]
    fn usage_content_host_has_strict_csp_and_no_external_refs() {
        let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/usage-content"));
        let html = std::fs::read_to_string(root.join("host.html")).expect("读 host.html");
        assert!(html.contains("default-src 'none'"));
        assert!(html.contains("script-src 'self'"));
        assert!(
            !html.contains("connect-src"),
            "usage-content 没有 fetch 端点,不应声明 connect-src"
        );
        assert!(!html.contains("http://") && !html.contains("https://"));
        assert!(html.contains("usage-content.js") && html.contains("usage-content.css"));
    }

    #[test]
    fn usage_content_serves_vendored_files() {
        let root = scratch();
        std::fs::create_dir_all(root.with_file_name("usage-content")).unwrap();
        std::fs::write(
            root.with_file_name("usage-content").join("host.html"),
            b"<html>u</html>",
        )
        .unwrap();
        let r = handle_protocol(
            &root,
            &HashSet::new(),
            None,
            "dozer://usage-content/host.html",
        );
        assert_eq!((r.status, r.mime), (200, "text/html"));
        assert_eq!(r.body, b"<html>u</html>");
    }

    #[test]
    fn usage_content_unknown_subpath_404() {
        let root = scratch();
        std::fs::create_dir_all(root.with_file_name("usage-content")).unwrap();
        let r = handle_protocol(&root, &HashSet::new(), None, "dozer://usage-content/nope");
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
    fn file_endpoint_transcodes_utf16_to_utf8() {
        let root = scratch();
        let f = root.join("utf16le.rs");
        // UTF-16LE + BOM:"hi\n"
        fs::write(&f, b"\xFF\xFEh\x00i\x00\n\x00").unwrap();
        let uri = format!("dozer://editor/__file__{}", f.to_string_lossy());
        let mut allowed = HashSet::new();
        allowed.insert(f.clone());
        let r = handle_protocol(&root, &allowed, None, &uri);
        assert_eq!(r.status, 200);
        assert_eq!(r.body, b"hi\n", "UTF-16 应转成 UTF-8 文本");
    }

    #[test]
    fn file_endpoint_leaves_utf8_untouched() {
        let root = scratch();
        let f = root.join("plain.rs");
        fs::write(&f, b"fn main() {}\n").unwrap();
        let uri = format!("dozer://editor/__file__{}", f.to_string_lossy());
        let mut allowed = HashSet::new();
        allowed.insert(f.clone());
        let r = handle_protocol(&root, &allowed, None, &uri);
        assert_eq!(r.body, b"fn main() {}\n");
    }

    #[test]
    fn file_endpoint_passes_odd_length_utf16_through() {
        // 奇数长度(落单尾字节)不是干净 UTF-16,原样透传(字节安全留待 T9)。
        let root = scratch();
        let f = root.join("odd.rs");
        fs::write(&f, b"\xFF\xFEh\x00i").unwrap();
        let uri = format!("dozer://editor/__file__{}", f.to_string_lossy());
        let mut allowed = HashSet::new();
        allowed.insert(f.clone());
        let r = handle_protocol(&root, &allowed, None, &uri);
        assert_eq!(r.body, b"\xFF\xFEh\x00i", "非偶数长度 UTF-16 应原样透传");
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
    fn bundle_resources_root_finds_app_ancestor() {
        let exe = Path::new("/Applications/Dozer AI Coder.app/Contents/MacOS/dozer");
        assert_eq!(
            bundle_resources_root(exe).as_deref(),
            Some(Path::new(
                "/Applications/Dozer AI Coder.app/Contents/Resources"
            ))
        );
    }

    #[test]
    fn bundle_resources_root_none_outside_app() {
        // 裸二进制(dev/直接跑 target 里的可执行文件):祖先里没有 `.app`。
        let exe = Path::new("/tmp/dozer/target/aarch64-apple-darwin/release/dozer");
        assert_eq!(bundle_resources_root(exe), None);
    }

    #[test]
    fn percent_decode_roundtrip() {
        assert_eq!(percent_decode("%2Fa%20b").as_deref(), Some("/a b"));
        assert_eq!(percent_decode("%E4%BD%A0").as_deref(), Some("你"));
        assert_eq!(percent_decode("plain").as_deref(), Some("plain"));
        assert!(percent_decode("%GG").is_none());
    }

    /// 构造 `dir/flyfish` 根 + 兄弟 `dir/editor` 根(与真实 dev/打包态同构)。
    fn scratch_editor() -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("dozer-assets-editor-{}", std::process::id()));
        let flyfish = dir.join("flyfish");
        let editor = dir.join("editor");
        let _ = fs::create_dir_all(&editor);
        fs::write(editor.join("index.html"), b"<!doctype html>").unwrap();
        fs::write(editor.join("editor.js"), b"js").unwrap();
        (flyfish, editor)
    }

    #[test]
    fn serves_editor_host_and_assets() {
        let (flyfish, _editor) = scratch_editor();
        let r = handle_protocol(
            &flyfish,
            &HashSet::new(),
            None,
            "dozer://editor/index.html?v=1",
        );
        assert_eq!((r.status, r.mime), (200, "text/html"));
        let r = handle_protocol(&flyfish, &HashSet::new(), None, "dozer://editor/editor.js");
        assert_eq!((r.status, r.mime), (200, "text/javascript"));
    }

    #[test]
    fn editor_file_endpoint_requires_allowlist() {
        let (flyfish, editor) = scratch_editor();
        let f = editor.join("victim.rs");
        fs::write(&f, b"fn main() {}").unwrap();
        let uri = format!("dozer://editor/__file__{}", f.to_string_lossy());
        assert_eq!(
            handle_protocol(&flyfish, &HashSet::new(), None, &uri).status,
            404
        );
        let mut allowed = HashSet::new();
        allowed.insert(f.clone());
        let r = handle_protocol(&flyfish, &allowed, None, &uri);
        assert_eq!(r.status, 200);
        assert_eq!(r.body, b"fn main() {}");
    }

    #[test]
    fn editor_rejects_traversal_and_unknown() {
        let (flyfish, _editor) = scratch_editor();
        for uri in [
            "dozer://editor/../flyfish/host.html",
            "dozer://editor/%2e%2e/etc/passwd",
            "dozer://editor/nope.js",
        ] {
            assert_eq!(
                handle_protocol(&flyfish, &HashSet::new(), None, uri).status,
                404,
                "{uri}"
            );
        }
    }

    /// JSON host 命名空间:从 `json-editor` 兄弟根服务,拒绝穿越。
    #[test]
    fn serves_json_editor_namespace() {
        let dir = std::env::temp_dir().join(format!("dozer-assets-json-{}", std::process::id()));
        let flyfish = dir.join("flyfish");
        let json_editor = dir.join("json-editor");
        let _ = fs::create_dir_all(&json_editor);
        fs::write(json_editor.join("index.html"), b"<!doctype html>").unwrap();
        fs::write(json_editor.join("json-editor.js"), b"js").unwrap();

        let r = handle_protocol(
            &flyfish,
            &HashSet::new(),
            None,
            "dozer://json-editor/index.html",
        );
        assert_eq!((r.status, r.mime), (200, "text/html"));
        let r = handle_protocol(
            &flyfish,
            &HashSet::new(),
            None,
            "dozer://json-editor/json-editor.js",
        );
        assert_eq!(r.status, 200);
        for uri in [
            "dozer://json-editor/../flyfish/host.html",
            "dozer://json-editor/%2e%2e/etc/passwd",
            "dozer://json-editor/nope.js",
        ] {
            assert_eq!(
                handle_protocol(&flyfish, &HashSet::new(), None, uri).status,
                404,
                "{uri}"
            );
        }
    }

    /// 提交的 JSON host 产物必须齐全。
    #[test]
    fn json_editor_bundle_assets_are_present() {
        let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/json-editor"));
        for f in ["index.html", "json-editor.js", "json-editor.css"] {
            let p = root.join(f);
            assert!(p.is_file(), "缺少 json-editor 产物 {f}: {}", p.display());
            assert!(
                std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0) > 0,
                "json-editor 产物为空: {f}"
            );
        }
        let html = std::fs::read_to_string(root.join("index.html")).unwrap();
        assert!(html.contains("default-src 'none'"));
        assert!(!html.contains("http://") && !html.contains("https://"));
    }

    /// 提交的 CodeMirror 产物必须齐全(防止忘记 `npm run build` 就提交)。
    #[test]
    fn editor_bundle_assets_are_present() {
        let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/editor"));
        for f in [
            "index.html",
            "editor.js",
            "editor.css",
            "fonts/JetBrainsMono.ttf",
        ] {
            let p = root.join(f);
            assert!(p.is_file(), "缺少 editor 产物 {f}: {}", p.display());
            assert!(
                std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0) > 0,
                "editor 产物为空: {f}"
            );
        }
    }

    /// 严格 CSP + 无网络:首页不得引用任何外部 URL,且带 `default-src 'none'`。
    #[test]
    fn editor_index_has_strict_csp_and_no_external_refs() {
        let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/editor"));
        let html = std::fs::read_to_string(root.join("index.html")).expect("读 index.html");
        assert!(
            html.contains("Content-Security-Policy"),
            "index.html 必须声明 CSP"
        );
        assert!(
            html.contains("default-src 'none'"),
            "CSP 必须以 default-src 'none' 起步"
        );
        assert!(html.contains("script-src 'self'"), "脚本仅 self");
        assert!(html.contains("connect-src 'self'"), "仅允许同源 fetch");
        assert!(
            !html.contains("http://") && !html.contains("https://"),
            "editor 首页不得引用外部 URL(离线约束)"
        );
        assert!(html.contains("editor.js") && html.contains("editor.css"));
    }

    /// 打包产物不得泄漏本机绝对路径或 sourcemap 引用。
    #[test]
    fn editor_js_has_no_absolute_paths_or_sourcemap() {
        let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/editor"));
        let js = std::fs::read_to_string(root.join("editor.js")).expect("读 editor.js");
        assert!(!js.contains("sourceMappingURL"), "不应有 sourcemap 引用");
        assert!(
            !js.contains(env!("CARGO_MANIFEST_DIR")),
            "不应含源码树绝对路径"
        );
        assert!(!js.contains("/Users/"), "不应含用户绝对路径");
    }

    /// T7:HTML host 页面可服务,且 CSP 为无网络/无任意脚本起步(sandbox iframe)。
    #[test]
    fn serves_html_host_with_sandbox_and_strict_csp() {
        let root = scratch();
        let r = handle_protocol(&root, &HashSet::new(), None, "dozer://html/host.html");
        assert_eq!((r.status, r.mime), (200, "text/html"));
        let html = String::from_utf8(r.body).unwrap();
        assert!(html.contains("default-src 'none'"));
        // 2026-09-27:`sandbox=""`(不带 allow-same-origin)会让 srcdoc 文档拿到
        // 唯一 opaque origin,WebKit 在这个组合下不认注入的 `<base>` 标签,相对
        // css/js/图片全部解析失败(只见文字不见样式,见 whatwg/html#9025)——必须
        // 带 allow-same-origin 才能让 `<base>` 生效。没有 allow-scripts 时
        // allow-same-origin 本身不引入脚本执行风险,不能把这个 token 删掉当作
        // "更严格"。
        // 从 `<iframe ...>` 标签本身取 sandbox 属性值,不从整份 HTML(含解释性
        // 注释)里子串匹配——注释里为了说明历史 bug 也提到了 `sandbox=""` 之类
        // 的反面写法,直接子串匹配会先命中注释而不是真正的属性。
        let iframe_tag = html.split("<iframe").nth(1).unwrap_or_default();
        let sandbox_value = iframe_tag
            .split("sandbox=\"")
            .nth(1)
            .and_then(|rest| rest.split('"').next())
            .unwrap_or_default();
        assert!(
            sandbox_value
                .split_whitespace()
                .any(|t| t == "allow-same-origin"),
            "iframe 必须带 allow-same-origin,否则 srcdoc 里的 <base> 在 WebKit 下不生效,相对 css/js/图片会全部加载失败(sandbox=\"{sandbox_value}\")"
        );
        assert!(
            !sandbox_value
                .split_whitespace()
                .any(|t| t == "allow-scripts"),
            "iframe 禁脚本,不能同时带 allow-scripts + allow-same-origin(经典沙盒逃逸组合)(sandbox=\"{sandbox_value}\")"
        );
        assert!(
            !html.contains("http://") && !html.contains("https://"),
            "host 不得引用外部 URL"
        );
    }

    /// T7:`dozer://html/__file__` 允许已打开文件所在目录子树,但拒绝越界/穿越。
    #[test]
    fn html_file_endpoint_scopes_to_opened_file_dirs() {
        let root = std::env::temp_dir().join(format!("dozer-html-test-{}", std::process::id()));
        let proj = root.join("proj");
        let sub = proj.join("assets");
        std::fs::create_dir_all(&sub).unwrap();
        let index = proj.join("index.html");
        let css = sub.join("site.css");
        let secret = root.join("outside.txt");
        std::fs::write(&index, b"<html>").unwrap();
        std::fs::write(&css, b"body{}").unwrap();
        std::fs::write(&secret, b"secret").unwrap();

        let mut allowed = HashSet::new();
        allowed.insert(index.clone());

        let uri = |p: &Path| format!("dozer://html/__file__{}", p.to_string_lossy());
        // 已打开文件本身:可读。
        assert_eq!(
            handle_protocol(&root, &allowed, None, &uri(&index)).status,
            200
        );
        // 同项目子树内的相对资源:可读。
        assert_eq!(
            handle_protocol(&root, &allowed, None, &uri(&css)).status,
            200
        );
        // 目录之外:404。
        assert_eq!(
            handle_protocol(&root, &allowed, None, &uri(&secret)).status,
            404
        );
        // 穿越:404。
        assert_eq!(
            handle_protocol(
                &root,
                &allowed,
                None,
                "dozer://html/__file__/proj/../outside.txt"
            )
            .status,
            404
        );
        // 未打开任何文件:404。
        assert_eq!(
            handle_protocol(&root, &HashSet::new(), None, &uri(&css)).status,
            404
        );
    }
}

pub mod clipboard_image;
pub mod fonts;
