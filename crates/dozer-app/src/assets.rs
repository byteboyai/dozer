// crates/dozer-app/src/assets.rs
//! `dozer://` 自定义协议应答(P1d,纯函数,headless 全测)。
//!
//! 布局(设计 §2):
//! - `dozer://flyfish/<path>`          → vendored Flyfish dist 静态字节;
//! - `dozer://flyfish/__file__/<abs>`  → 本地文件字节,仅白名单(用户显式
//!   打开过的路径)。文件端点与 host 页同命名空间——wry 自定义协议在
//!   macOS 的 Origin 是 `dozer://flyfish`,跨命名空间 fetch 被 CORS 拦;
//!   URL 保留原始扩展名,Flyfish 靠它选择渲染管线。

use std::path::{Path, PathBuf};

pub use dozer_panel_preview::assets::ProtocolReply;

fn not_found() -> ProtocolReply {
    dozer_panel_preview::assets::not_found()
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
    dozer_panel_preview::assets::from_lib_source()
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

/// usage-content host(用量面板内容侧图表)静态资源根。产物唯一来源在
/// `dozer-panel-usage` crate(Task 19 起);打包分发时 `build-macos-app.sh`
/// 把它拷进 `Contents/Resources/usage-content`,此时 `flyfish_root` 的兄弟
/// 目录 `usage-content` 存在,优先用它;dev 态(兄弟目录不存在)回退到
/// crate 自带资源根。同 `editor_root_for`/`review_trace_root_for`。
fn usage_content_root_for(flyfish_root: &Path) -> PathBuf {
    let bundled = flyfish_root.with_file_name("usage-content");
    if bundled.is_dir() {
        bundled
    } else {
        dozer_panel_usage::web_assets_root()
    }
}

/// codehealth-content host(代码健康度内容侧)静态资源根 = flyfish 根的兄弟
/// 目录 `codehealth-content`。同 `usage_content_root_for`。
fn codehealth_content_root_for(flyfish_root: &Path) -> PathBuf {
    flyfish_root.with_file_name("codehealth-content")
}

/// todo-content host(Todo 面板内容区)静态资源根。产物唯一来源在 `dozer-panel-todo` crate(S3a 起);
/// 打包分发时 `build-macos-app.sh` 把它拷进 `Contents/Resources/todo-content`,此时 `flyfish_root` 的兄弟
/// 目录 `todo-content` 存在,优先用它;dev 态(兄弟目录不存在)回退到 crate 自带资源根。同 `usage_content_root_for`。
fn todo_content_root_for(flyfish_root: &Path) -> PathBuf {
    let bundled = flyfish_root.with_file_name("todo-content");
    if bundled.is_dir() {
        bundled
    } else {
        dozer_panel_todo::web_assets_root()
    }
}

/// group-chat-content host(群聊面板)静态资源根 = flyfish 根的兄弟目录
/// `group-chat-content`。同 `todo_content_root_for`。
fn group_chat_content_root_for(flyfish_root: &Path) -> PathBuf {
    flyfish_root.with_file_name("group-chat-content")
}

/// 应用外壳自己的命名空间(与预览库的七个互斥)。
fn is_app_namespace(uri: &str) -> bool {
    let Some(rest) = uri.strip_prefix("dozer://") else {
        return false;
    };
    let rest = rest.split('?').next().unwrap_or(rest);
    [
        "review-trace/",
        "usage-content/",
        "codehealth-content/",
        "todo-content/",
        "group-chat-content/",
    ]
    .iter()
    .any(|ns| rest.starts_with(ns))
}

pub fn handle_protocol(
    assets_root: &Path,
    allowed: &dozer_panel_preview::assets::AllowedFiles,
    review_data: Option<&str>,
    uri: &str,
) -> ProtocolReply {
    // 预览宿主的七个命名空间(flyfish/editor/json-editor/tabular/html/
    // image-annotate/plantuml-viewer)由共享库应答;其余是应用外壳自己的。
    if let Some(reply) = dozer_panel_preview::assets::handle_protocol(assets_root, allowed, uri)
        && !is_app_namespace(uri)
    {
        return reply;
    }
    let Some(rest) = uri.strip_prefix("dozer://") else {
        return not_found();
    };
    let rest = rest.split('?').next().unwrap_or(rest);

    // 审阅 trace 的离线资源由共享库嵌入;数据端点回显当前快照,缺省时 404。
    if let Some(path) = rest.strip_prefix("review-trace/") {
        return match dozer_panel_conversations::assets::serve_review_trace(path, review_data) {
            Some((mime, body)) => ProtocolReply {
                status: 200,
                mime,
                body,
            },
            None => not_found(),
        };
    }

    // usage-content host(用量面板内容侧图表):没有 data.json 特判——数据
    // 全靠 evaluate_script 推送,不走 fetch(见 protocol.rs)。
    if let Some(path) = rest.strip_prefix("usage-content/") {
        return serve_vendored(&usage_content_root_for(assets_root), path);
    }

    // codehealth-content host(代码健康度内容侧):同 usage-content,没有 data.json
    // 特判,数据全靠 evaluate_script 推送。
    if let Some(path) = rest.strip_prefix("codehealth-content/") {
        return serve_vendored(&codehealth_content_root_for(assets_root), path);
    }

    // todo-content host(Todo 面板内容区):同 codehealth-content,没有 data.json
    // 特判,数据全靠 evaluate_script 推送。
    if let Some(path) = rest.strip_prefix("todo-content/") {
        return serve_vendored(&todo_content_root_for(assets_root), path);
    }

    // group-chat-content host(群聊面板):同 todo-content,没有 data.json 特判,
    // 数据全靠 evaluate_script 推送。
    if let Some(path) = rest.strip_prefix("group-chat-content/") {
        return serve_vendored(&group_chat_content_root_for(assets_root), path);
    }

    not_found()
}

/// vendored 静态资源服务(应用外壳自己的命名空间)。
fn serve_vendored(root: &Path, encoded_path: &str) -> ProtocolReply {
    let mut full = root.to_path_buf();
    for seg in encoded_path.split('/') {
        let Some(seg) = dozer_panel_preview::assets::percent_decode(seg) else {
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
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
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

#[cfg(test)]
mod tests {
    use super::*;
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
    fn review_trace_data_json_echoes_injected_snapshot() {
        let root = scratch();
        let r = handle_protocol(
            &root,
            &dozer_panel_preview::assets::AllowedFiles::new(),
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
            &dozer_panel_preview::assets::AllowedFiles::new(),
            None,
            "dozer://review-trace/data.json",
        );
        assert_eq!(r.status, 404);
    }

    #[test]
    fn review_trace_unknown_subpath_404() {
        let root = scratch();
        let r = handle_protocol(
            &root,
            &dozer_panel_preview::assets::AllowedFiles::new(),
            None,
            "dozer://review-trace/nope",
        );
        assert_eq!(r.status, 404);
    }

    /// 提交的 usage-content 产物必须齐全(防止忘记 `npm run build` 就提交)。
    #[test]
    fn usage_content_bundle_assets_are_present() {
        let root = dozer_panel_usage::web_assets_root();
        for f in ["host.html", "usage-content.js", "usage-content.css"] {
            let p = root.join(f);
            assert!(p.is_file(), "缺少 usage-content 产物 {f}: {}", p.display());
            assert!(
                std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0) > 0,
                "usage-content 产物为空: {f}"
            );
        }
    }

    /// 评审关注点 5:todo-content 产物的**唯一来源**在 `dozer-panel-todo`;`dozer-app` 里不再留副本。
    #[test]
    fn todo_content_has_one_source_and_dozer_app_keeps_no_copy() {
        assert!(
            dozer_panel_todo::web_assets_root()
                .join("host.html")
                .is_file()
        );
        let old = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/todo-content");
        assert!(
            !old.exists(),
            "dozer-app 里不应再留 todo-content 副本: {}",
            old.display()
        );
    }

    /// 严格 CSP、无 connect-src(没有 fetch 端点,数据全靠推送)、无网络。
    #[test]
    fn usage_content_host_has_strict_csp_and_no_external_refs() {
        let root = dozer_panel_usage::web_assets_root();
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
            &dozer_panel_preview::assets::AllowedFiles::new(),
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
        let r = handle_protocol(
            &root,
            &dozer_panel_preview::assets::AllowedFiles::new(),
            None,
            "dozer://usage-content/nope",
        );
        assert_eq!(r.status, 404);
    }

    #[test]
    fn codehealth_content_serves_vendored_files() {
        let root = scratch();
        std::fs::create_dir_all(root.with_file_name("codehealth-content")).unwrap();
        std::fs::write(
            root.with_file_name("codehealth-content").join("host.html"),
            b"<html>c</html>",
        )
        .unwrap();
        let r = handle_protocol(
            &root,
            &dozer_panel_preview::assets::AllowedFiles::new(),
            None,
            "dozer://codehealth-content/host.html",
        );
        assert_eq!((r.status, r.mime), (200, "text/html"));
        assert_eq!(r.body, b"<html>c</html>");
    }

    #[test]
    fn codehealth_content_unknown_subpath_404() {
        let root = scratch();
        std::fs::create_dir_all(root.with_file_name("codehealth-content")).unwrap();
        let r = handle_protocol(
            &root,
            &dozer_panel_preview::assets::AllowedFiles::new(),
            None,
            "dozer://codehealth-content/nope",
        );
        assert_eq!(r.status, 404);
    }

    /// 路径穿越不得逃出 codehealth-content 根。
    #[test]
    fn codehealth_content_rejects_path_traversal() {
        let root = scratch();
        std::fs::create_dir_all(root.with_file_name("codehealth-content")).unwrap();
        let r = handle_protocol(
            &root,
            &dozer_panel_preview::assets::AllowedFiles::new(),
            None,
            "dozer://codehealth-content/../usage-content/host.html",
        );
        assert_eq!(r.status, 404);
    }

    /// 提交的 codehealth-content 产物必须齐全(防止忘记 `npm run build` 就提交)。
    #[test]
    fn codehealth_content_bundle_assets_are_present() {
        let root = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/codehealth-content"
        ));
        for f in [
            "host.html",
            "codehealth-content.js",
            "codehealth-content.css",
        ] {
            let p = root.join(f);
            assert!(
                p.is_file(),
                "缺少 codehealth-content 产物 {f}: {}",
                p.display()
            );
            assert!(
                std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0) > 0,
                "codehealth-content 产物为空: {f}"
            );
        }
    }

    /// 严格 CSP、无 connect-src、无网络引用。
    #[test]
    fn codehealth_content_host_has_strict_csp_and_no_external_refs() {
        let root = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/codehealth-content"
        ));
        let html = std::fs::read_to_string(root.join("host.html")).expect("读 host.html");
        assert!(html.contains("default-src 'none'"));
        assert!(html.contains("script-src 'self'"));
        assert!(!html.contains("connect-src"));
        assert!(!html.contains("http://") && !html.contains("https://"));
        assert!(html.contains("codehealth-content.js") && html.contains("codehealth-content.css"));
    }

    /// 提交的 group-chat-content 产物必须齐全(防止忘记 `npm run build` 就提交)。
    #[test]
    fn group_chat_content_bundle_assets_are_present() {
        let root = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/group-chat-content"
        ));
        for f in [
            "host.html",
            "group-chat-content.js",
            "group-chat-content.css",
        ] {
            let p = root.join(f);
            assert!(
                p.is_file(),
                "缺少 group-chat-content 产物 {f}: {}",
                p.display()
            );
            assert!(
                std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0) > 0,
                "group-chat-content 产物为空: {f}"
            );
        }
    }

    /// 严格 CSP、无 connect-src、无网络引用。
    #[test]
    fn group_chat_content_host_has_strict_csp_and_no_external_refs() {
        let root = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/group-chat-content"
        ));
        let html = std::fs::read_to_string(root.join("host.html")).expect("读 host.html");
        assert!(html.contains("default-src 'none'"));
        assert!(html.contains("script-src 'self'"));
        assert!(!html.contains("connect-src"));
        assert!(!html.contains("http://") && !html.contains("https://"));
        assert!(html.contains("group-chat-content.js") && html.contains("group-chat-content.css"));
    }

    #[test]
    fn todo_content_serves_vendored_files() {
        let root = scratch();
        std::fs::create_dir_all(root.with_file_name("todo-content")).unwrap();
        std::fs::write(
            root.with_file_name("todo-content").join("host.html"),
            b"<html>t</html>",
        )
        .unwrap();
        let r = handle_protocol(
            &root,
            &dozer_panel_preview::assets::AllowedFiles::new(),
            None,
            "dozer://todo-content/host.html",
        );
        assert_eq!((r.status, r.mime), (200, "text/html"));
        assert_eq!(r.body, b"<html>t</html>");
    }

    #[test]
    fn todo_content_unknown_subpath_404() {
        let root = scratch();
        std::fs::create_dir_all(root.with_file_name("todo-content")).unwrap();
        let r = handle_protocol(
            &root,
            &dozer_panel_preview::assets::AllowedFiles::new(),
            None,
            "dozer://todo-content/nope",
        );
        assert_eq!(r.status, 404);
    }

    /// 路径穿越不得逃出 todo-content 根。
    #[test]
    fn todo_content_rejects_path_traversal() {
        let root = scratch();
        std::fs::create_dir_all(root.with_file_name("todo-content")).unwrap();
        let r = handle_protocol(
            &root,
            &dozer_panel_preview::assets::AllowedFiles::new(),
            None,
            "dozer://todo-content/../usage-content/host.html",
        );
        assert_eq!(r.status, 404);
    }

    #[test]
    fn group_chat_content_serves_vendored_files() {
        let root = scratch();
        std::fs::create_dir_all(root.with_file_name("group-chat-content")).unwrap();
        std::fs::write(
            root.with_file_name("group-chat-content").join("host.html"),
            b"<html>g</html>",
        )
        .unwrap();
        let r = handle_protocol(
            &root,
            &dozer_panel_preview::assets::AllowedFiles::new(),
            None,
            "dozer://group-chat-content/host.html",
        );
        assert_eq!((r.status, r.mime), (200, "text/html"));
        assert_eq!(r.body, b"<html>g</html>");
    }

    #[test]
    fn group_chat_content_unknown_subpath_404() {
        let root = scratch();
        std::fs::create_dir_all(root.with_file_name("group-chat-content")).unwrap();
        let r = handle_protocol(
            &root,
            &dozer_panel_preview::assets::AllowedFiles::new(),
            None,
            "dozer://group-chat-content/nope",
        );
        assert_eq!(r.status, 404);
    }

    /// 路径穿越不得逃出 group-chat-content 根。
    #[test]
    fn group_chat_content_rejects_path_traversal() {
        let root = scratch();
        std::fs::create_dir_all(root.with_file_name("group-chat-content")).unwrap();
        let r = handle_protocol(
            &root,
            &dozer_panel_preview::assets::AllowedFiles::new(),
            None,
            "dozer://group-chat-content/../usage-content/host.html",
        );
        assert_eq!(r.status, 404);
    }

    /// 提交的 todo-content 产物必须齐全(防止忘记 `npm run build` 就提交)。
    #[test]
    fn todo_content_bundle_assets_are_present() {
        let root = dozer_panel_todo::web_assets_root();
        for f in ["host.html", "todo-content.js", "todo-content.css"] {
            let p = root.join(f);
            assert!(p.is_file(), "缺少 todo-content 产物 {f}: {}", p.display());
            assert!(
                std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0) > 0,
                "todo-content 产物为空: {f}"
            );
        }
    }

    /// 严格 CSP、无 connect-src、无网络引用。
    #[test]
    fn todo_content_host_has_strict_csp_and_no_external_refs() {
        let root = dozer_panel_todo::web_assets_root();
        let html = std::fs::read_to_string(root.join("host.html")).expect("读 host.html");
        assert!(html.contains("default-src 'none'"));
        assert!(html.contains("script-src 'self'"));
        assert!(!html.contains("connect-src"));
        assert!(!html.contains("http://") && !html.contains("https://"));
        assert!(html.contains("todo-content.js") && html.contains("todo-content.css"));
    }
}

pub mod clipboard_image;
pub mod fonts;

#[cfg(test)]
mod review_serve_golden_tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn review_trace_serve_matches_the_golden() {
        let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/flyfish"));
        let snapshot = include_str!("../tests/fixtures/review_snapshot_golden.json");
        let mut actual = String::new();
        for name in [
            "host.html",
            "review-trace.js",
            "review-trace.css",
            "data.json",
        ] {
            let reply = handle_protocol(
                root,
                &dozer_panel_preview::assets::AllowedFiles::new(),
                Some(snapshot),
                &format!("dozer://review-trace/{name}"),
            );
            assert_eq!(reply.status, 200, "{name}");
            actual.push_str(&format!(
                "{name}\t{}\t{}\t{:x}\n",
                reply.mime,
                reply.body.len(),
                Sha256::digest(&reply.body)
            ));
        }
        let missing = handle_protocol(
            root,
            &dozer_panel_preview::assets::AllowedFiles::new(),
            None,
            "dozer://review-trace/data.json",
        );
        assert_eq!(missing.status, 404);
        actual.push_str("data.json without snapshot\t404\n");
        actual.push_str(snapshot);
        assert_eq!(
            actual,
            include_str!("../tests/fixtures/review_trace_serve_golden.txt")
        );
    }
}
