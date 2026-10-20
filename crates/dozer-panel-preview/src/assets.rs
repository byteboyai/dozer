//! 预览 webview 的 `dozer://` 协议应答(S5b-2 P3)。
//!
//! 从 `dozer-app::assets` 原样搬来**只服务预览宿主**的那一半:
//!
//! - `dozer://flyfish/<path>`           → vendored Flyfish dist 静态字节;
//! - `dozer://flyfish/__file__/<abs>`   → 本地文件字节,仅白名单(用户显式
//!   打开过的路径)。文件端点与 host 页同命名空间——wry 自定义协议在
//!   macOS 的 Origin 是 `dozer://flyfish`,跨命名空间 fetch 被 CORS 拦;
//!   URL 保留原始扩展名,Flyfish 靠它选择渲染管线。
//! - `dozer://editor|json-editor|tabular|image-annotate/` → 各 host 的静态
//!   根(`__file__` 端点按各自闸门读取已打开文件);
//! - `dozer://html/host.html` 与 `dozer://image-annotate/host.html` 编译期内嵌;
//! - `dozer://plantuml-viewer/` 只服务 vendored 资产(没有 `__file__`)。
//!
//! 应用外壳自己的命名空间(review-trace / usage-content / codehealth-content /
//! todo-content / group-chat-content)留在产品侧,不属于预览面板。纯函数、
//! 无 wry/winit 依赖,headless 全测。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::file_profile::decode_utf16_to_utf8;

/// 协议端点可读文件的**规范化**白名单(S5b-2 P4 Task 3)。
///
/// 产品用 `insert` / `clear` 把「用户显式打开过的文件」登记进来;协议闭包
/// 用 `contains` 判断能否读。
///
/// **判定与读取必须落在同一个路径上**(S5b-2 P4 复审 D-I2)。早先只做纯词法
/// 规范化,而 `serve_allowlisted_file` 拿原始解码路径 `fs::read`,系统会解析
/// 符号链接——`proj/link/../ok.txt` 词法折叠成 `proj/ok.txt` 通过判定,读出来
/// 却是 `link` 指向的另一个目录里的文件。现在 `insert` / `remove` / `contains`
/// 一律 `canonicalize`(解析符号链接、`/tmp` ↔ `/private/tmp` 一类等价),
/// **失败即拒绝**(不降级回词法比较);判定与读取都用 canonical 结果。
///
/// 词法规范化仍然保留在 [`clean_allowed_path`]:它是 `canonicalize` 之前的第一
/// 道折叠(消掉 `.` / `..` / 重复分隔符),也让相对路径先接上当前目录。
#[derive(Debug, Clone, Default)]
pub struct AllowedFiles {
    inner: HashSet<PathBuf>,
}

impl AllowedFiles {
    pub fn new() -> Self {
        Self::default()
    }

    /// 登记一个文件;返回内部实际保存的 canonical 路径。
    ///
    /// `canonicalize` 失败(文件不存在 / 权限不足 / 悬空链接)时**不登记**,
    /// 返回未 canonical 的折叠路径——调用方拿到的值只能用来报错,不能当作
    /// "已授权"。
    pub fn insert(&mut self, path: impl Into<PathBuf>) -> PathBuf {
        let folded = clean_allowed_path(&path.into());
        let Some(canonical) = canonical_allowed_path(&folded) else {
            return folded;
        };
        self.inner.insert(canonical.clone());
        canonical
    }

    /// 撤销登记(同样先 canonical 再删)。
    pub fn remove(&mut self, path: &Path) {
        if let Some(canonical) = canonical_allowed_path(&clean_allowed_path(path)) {
            self.inner.remove(&canonical);
        }
    }

    /// 白名单是否含该文件。查询路径同样 canonical——失败即 `false`
    /// (悬空链接不能被当成"授权过")。
    pub fn contains(&self, path: &Path) -> bool {
        canonical_allowed_path(&clean_allowed_path(path))
            .is_some_and(|canonical| self.inner.contains(&canonical))
    }

    /// 把一次请求解析成**要读的那个 canonical 路径**:既在白名单里、又确实
    /// 是普通文件(不是目录)才返回。`serve_allowlisted_file` 必须用这个结果
    /// 去 `fs::read`,不能再用请求里的原始字符串。
    pub fn resolve(&self, path: &Path) -> Option<PathBuf> {
        let canonical = canonical_allowed_path(&clean_allowed_path(path))?;
        if !self.inner.contains(&canonical) {
            return None;
        }
        let meta = std::fs::metadata(&canonical).ok()?;
        if !meta.is_file() {
            return None;
        }
        Some(canonical)
    }

    pub fn clear(&mut self) {
        self.inner.clear();
    }

    /// 撤销**这个前缀目录下**的所有登记(项目关闭 / 切走时清掉它打开过的文件)。
    /// 白名单只增不减会让加载过的项目永久可读,切走项目后必须回收。
    pub fn remove_under(&mut self, dir: &Path) {
        let folded = clean_allowed_path(dir);
        let Some(base) = canonical_allowed_path(&folded) else {
            return;
        };
        let stale: Vec<PathBuf> = self
            .inner
            .iter()
            .filter(|p| p.starts_with(&base))
            .cloned()
            .collect();
        for p in stale {
            self.inner.remove(&p);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// 遍历**已规范化**的登记项(`assets` 的 `html` 目录闸门要用)。
    pub fn iter(&self) -> impl Iterator<Item = &PathBuf> {
        self.inner.iter()
    }
}

impl FromIterator<PathBuf> for AllowedFiles {
    fn from_iter<I: IntoIterator<Item = PathBuf>>(iter: I) -> Self {
        let mut out = Self::new();
        for path in iter {
            out.insert(path);
        }
        out
    }
}

/// 白名单判定与读取共用的 canonical 形式。
///
/// `canonicalize` 失败就返回 `None`:白名单宁可少放行,也不能把"读不到"当成
/// "可以读"。macOS 上 `/tmp` 会解析成 `/private/tmp`,这一步顺便统一了它们。
fn canonical_allowed_path(path: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(path).ok()
}

/// 把路径整理成白名单比较用的固定形式。
///
/// - 保留「绝对 / 相对」属性(相对路径先接当前目录,产品传入的都是绝对
///   路径,但规范化不应悄悄把它变成别的盘/根);
/// - 去掉 `.` 分量、消解 `..`(不越出根)、合并重复分隔符、去尾斜杠。
///
/// 纯字符串处理:符号链接、大小写、Unicode 归一化都不碰——那些需要碰文件
/// 系统,而白名单是"用户打开过哪些**路径**",不是"哪些 inode"。
pub fn clean_allowed_path(path: &Path) -> PathBuf {
    use std::path::Component;

    let absolute = path.is_absolute();
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::Prefix(_) | Component::RootDir | Component::CurDir => {
                out.push(comp.as_os_str())
            }
            Component::ParentDir => {
                if !out.pop() && !absolute {
                    out.push("..");
                }
            }
            Component::Normal(part) => out.push(part),
        }
    }
    if !absolute {
        let joined = if out.as_os_str().is_empty() {
            PathBuf::from(".")
        } else {
            out
        };
        if let Ok(cwd) = std::env::current_dir() {
            return clean_allowed_path(&cwd.join(joined));
        }
        return joined;
    }
    out
}

/// 一次协议应答(状态码 / MIME / 字节)。产品把它转成自家 webview 的响应类型。
pub struct ProtocolReply {
    pub status: u16,
    pub mime: &'static str,
    pub body: Vec<u8>,
}

/// 一次 404 应答(产品在预览命名空间之外也用同一份文案)。
pub fn not_found() -> ProtocolReply {
    ProtocolReply {
        status: 404,
        mime: "text/plain",
        body: b"not found".to_vec(),
    }
}

/// `dozer://flyfish/...` 静态资源的根目录。
///
/// 两种形态:
/// - 打包分发:可执行文件位于 `<App>.app/Contents/MacOS/<exe>`,资源随包放在
///   `Contents/Resources/flyfish`。从 `current_exe` 向上找 `.app` 边界定位 Resources。
/// - dev 形态(cargo run):调用方传入源码树里的 `assets/flyfish`。
pub fn bundled_assets_root(exe: &Path, dev_root: &Path) -> PathBuf {
    match bundle_resources_root(exe) {
        Some(resources) => resources.join("flyfish"),
        None => dev_root.to_path_buf(),
    }
}

/// 开发态资源根:本 crate 源码树里的 `assets/flyfish`。
///
/// 产品在 dev 形态(cargo run)没有自己的资源副本,直接读这里;打包态请走
/// `bundled_assets_root(exe, from_lib_source())`。编辑这里会同时影响所有
/// 依赖方,但只影响 dev 运行,不进任何产物。
pub fn from_lib_source() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/flyfish"))
}

/// 给定可执行文件路径,若它位于 macOS `.app` 包内(祖先目录名以 `.app`
/// 结尾),返回该包的 `Contents/Resources`;否则 `None`(dev/裸二进制)。
///
/// 纯函数、可测;不依赖 AppKit/objc2,仅用 `std::path::Path` 祖先遍历。
pub fn bundle_resources_root(exe: &Path) -> Option<PathBuf> {
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
fn serve_allowlisted_file(encoded: &str, allowed: &AllowedFiles) -> ProtocolReply {
    let Some(decoded) = percent_decode(encoded) else {
        return not_found();
    };
    let requested = PathBuf::from(&decoded);
    // 请求里出现 `..` 分量一律拒绝:即使 canonical 后落在白名单内,也不接受
    // 这种写法(它正是符号链接逃逸的载体)。
    if requested
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return not_found();
    }
    // 判定与读取用同一个 canonical 路径,目录也在这里被拒掉。
    let Some(file) = allowed.resolve(&requested) else {
        return not_found();
    };
    match std::fs::read(&file) {
        Ok(body) => {
            // UTF-16 文本(带 BOM)先转成 UTF-8:WebView 的 `fetch().text()`
            // 固定按 UTF-8 解码,喂原始 UTF-16 字节会整篇乱码(2026-09 用户
            // 报告 utf16le.rs 打开乱码)。转码只影响显示;该文件因 utf8:Invalid
            // 已按只读处理,不涉及保存回写。
            let body = match decode_utf16_to_utf8(&body) {
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

/// T7:`dozer://html/__file__/<abs>` 的读取闸门。比 editor 的"精确文件白名单"
/// 宽一点:允许**已打开文件所在目录子树**(相对资源 css/js/图片要能加载),
/// 但不允许跨出这些目录,且拒绝任何 `..` 分量 + 再 canonicalize 复核(防符号
/// 链接逃逸)。`allowed` 是应用维护的"用户显式打开过的文件"集合。
fn serve_html_file(encoded: &str, allowed: &AllowedFiles) -> ProtocolReply {
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

/// Tabular host(ag-grid)静态资源根 = flyfish 根的兄弟目录 `tabular-host`。
/// 同 `json_editor_root_for`,dev 与打包态同构。
fn tabular_host_root_for(flyfish_root: &Path) -> PathBuf {
    flyfish_root.with_file_name("tabular-host")
}

/// image-annotate host(OpenSeadragon + Annotorious 图片查看/标注)静态资源根
/// = flyfish 根的兄弟目录 `image-annotate`。同 `editor_root_for`。
fn image_annotate_root_for(flyfish_root: &Path) -> PathBuf {
    flyfish_root.with_file_name("image-annotate")
}

/// PlantUML viewer host 静态资源根 = flyfish 根的兄弟目录 `plantuml-viewer`。
/// 同 `image_annotate_root_for`,dev 与打包态同构。
fn plantuml_viewer_root_for(flyfish_root: &Path) -> PathBuf {
    flyfish_root.with_file_name("plantuml-viewer")
}

/// 预览 webview 的 HTML host 页面(编译期内嵌)。
/// 隔离 HTML 预览:内容在 webview 里加载,沙箱 + 严格 CSP 由页面自己声明。
const HTML_HOST: &str = include_str!("../assets/html_host.html");
/// 图片查看/标注 host:只读取"当前打开的这一张图片"本身。
const IMAGE_ANNOTATE_HOST: &str = include_str!("../assets/image_annotate_host.html");

/// 应答一条 `dozer://` 请求。只处理预览面板自己的七个命名空间
/// (`flyfish/ editor/ json-editor/ tabular/ html/ image-annotate/
/// plantuml-viewer/`);其余 scheme 或命名空间返回 `None`,由产品外壳接手
/// (Dozer 还有 review-trace / usage-content / codehealth-content /
/// todo-content / group-chat-content 等应用级命名空间)。
pub fn handle_protocol(
    assets_root: &Path,
    allowed: &AllowedFiles,
    uri: &str,
) -> Option<ProtocolReply> {
    let rest = uri.strip_prefix("dozer://")?;
    let rest = rest.split('?').next().unwrap_or(rest);
    // 只认预览的七个命名空间:别的 `dozer://` 路径(应用外壳的
    // review-trace / usage-content / …)交给产品,不能被当成 flyfish 根下的相对路径。
    if !rest.is_empty() && !PREVIEW_NAMESPACES.iter().any(|ns| rest.starts_with(ns)) {
        return None;
    }
    Some(serve(assets_root, allowed, rest))
}

/// 预览面板负责的命名空间前缀。
const PREVIEW_NAMESPACES: [&str; 7] = [
    "flyfish/",
    "editor/",
    "json-editor/",
    "tabular/",
    "html/",
    "image-annotate/",
    "plantuml-viewer/",
];

/// `rest` 是去掉 `dozer://` 与 query 之后的路径。非预览命名空间 → 404 之外的
/// 情形由调用方(`handle_protocol`)判 `None`;这里默认按 404 应答。
fn serve(assets_root: &Path, allowed: &AllowedFiles, rest: &str) -> ProtocolReply {
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

    // Tabular host(ag-grid):页面/脚本/样式从 `tabular-host` 兄弟根服务。
    // 与 JSON host 不同,tabular **没有 `__file__` 端点**——表格数据完全由
    // evaluate_script 推送的 `Init`/`SetWindow` 命令承载,host 不 fetch 任何
    // 用户文件。
    if let Some(path) = rest.strip_prefix("tabular/") {
        return serve_vendored(&tabular_host_root_for(assets_root), path);
    }

    // T7:HTML 隔离 host。页面编译期内嵌;`__file__` 走 `serve_html_file`
    // (允许已打开文件所在目录子树,供相对资源)。
    if let Some(path) = rest.strip_prefix("html/") {
        if path == "host.html" {
            return ProtocolReply {
                status: 200,
                mime: "text/html",
                body: HTML_HOST.as_bytes().to_vec(),
            };
        }
        if let Some(encoded) = path.strip_prefix("__file__") {
            return serve_html_file(encoded, allowed);
        }
        return not_found();
    }

    // image-annotate host(OpenSeadragon + Annotorious):页面编译期内嵌;
    // `__file__` 走 `serve_allowlisted_file`——这个 host 只需要精确读取
    // "当前打开的这一张图片"本身,不像 HTML 预览需要相对资源子树访问。
    if let Some(path) = rest.strip_prefix("image-annotate/") {
        if path == "host.html" {
            return ProtocolReply {
                status: 200,
                mime: "text/html",
                body: IMAGE_ANNOTATE_HOST.as_bytes().to_vec(),
            };
        }
        if let Some(encoded) = path.strip_prefix("__file__") {
            return serve_allowlisted_file(encoded, allowed);
        }
        return serve_vendored(&image_annotate_root_for(assets_root), path);
    }

    // PlantUML viewer host:只服务 vendored 静态资产(index.html/bundle/引擎/
    // stdlib)。**没有 `__file__` 端点**——源码与 include 由 Rust 用具名 command
    // 推送,host 页面不得自行 fetch 任意已打开文件,避免页面凭 URL 猜路径读取
    // 项目内文件。
    if let Some(path) = rest.strip_prefix("plantuml-viewer/") {
        return serve_vendored(&plantuml_viewer_root_for(assets_root), path);
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
mod not_found_tests {
    use super::not_found;

    /// 404 回执必须是固定短体:带上请求路径就等于把宿主绝对路径回显给
    /// webview 里跑的页面(审查项 D-I2)。
    #[test]
    fn not_found_body_is_a_constant_and_never_echoes_the_path() {
        let a = not_found();
        let b = not_found();
        assert_eq!(a.status, 404);
        assert_eq!(a.mime, "text/plain");
        assert_eq!(a.body, b"not found");
        assert_eq!(b.status, a.status);
        assert_eq!(b.body, a.body);
        assert!(!String::from_utf8_lossy(&a.body).contains('/'));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    /// S5b-2 P3 Task 1:协议应答黄金。固定夹具 + 固定 URI 表 → 每行
    /// `status|mime|len|fnv1a`。搬迁前后逐行比较,任何"顺手改进"都会让它变红。
    /// 用 FNV-1a 而不是 `DefaultHasher`(后者跨 Rust 版本不保证稳定)。
    #[test]
    fn handle_protocol_matches_the_golden() {
        const GOLDEN: &str = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/preview_protocol_golden.txt"
        );

        fn fnv1a(bytes: &[u8]) -> u64 {
            let mut h: u64 = 0xcbf2_9ce4_8422_2325;
            for b in bytes {
                h ^= u64::from(*b);
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
            h
        }

        let base = std::env::temp_dir().join("dozer-preview-protocol-golden");
        let flyfish = base.join("flyfish");
        let editor = base.join("editor");
        let html = base.join("html");
        let image = base.join("image-annotate");
        let plantuml = base.join("plantuml-viewer");
        for dir in [&flyfish, &editor, &html, &image, &plantuml] {
            fs::create_dir_all(dir).unwrap();
        }
        fs::write(flyfish.join("host.html"), b"<html>flyfish").unwrap();
        fs::write(flyfish.join("app.js"), b"console.log(1)").unwrap();
        fs::write(editor.join("index.html"), b"<html>editor").unwrap();
        fs::write(html.join("host.html"), b"<html>html-host").unwrap();
        fs::write(image.join("host.html"), b"<html>image").unwrap();
        fs::write(plantuml.join("index.html"), b"<html>plantuml").unwrap();

        let allowed_file = base.join("opened file.txt");
        fs::write(&allowed_file, b"hello").unwrap();
        let escaped = base.join("secret.txt");
        fs::write(&escaped, b"nope").unwrap();

        let mut allowed = AllowedFiles::new();
        allowed.insert(allowed_file.clone());
        let enc = |p: &std::path::Path| {
            p.to_string_lossy()
                .replace('%', "%25")
                .replace(' ', "%20")
                .replace('#', "%23")
                .replace('?', "%3F")
                .replace('&', "%26")
        };

        // 黄金里的 URI 必须与机器无关:夹具根用 `<base>` 占位,记录前替换回去。
        let base_s = base.to_string_lossy().to_string();
        let redact = |uri: String| uri.replace(&base_s, "<base>");
        let cases = vec![
            (
                redact(format!("dozer://flyfish/__file__{}", enc(&allowed_file))),
                true,
            ),
            (
                redact(format!("dozer://flyfish/__file__{}", enc(&escaped))),
                false,
            ),
            (
                String::from("dozer://flyfish/__file__/../etc/passwd"),
                false,
            ),
        ];
        let plain = [
            "dozer://flyfish/host.html",
            "dozer://flyfish/app.js",
            "dozer://editor/index.html",
            "dozer://json-editor/index.html",
            "dozer://tabular/index.html",
            "dozer://html/host.html",
            "dozer://image-annotate/host.html",
            "dozer://plantuml-viewer/index.html",
            "dozer://unknown-namespace/x",
            "https://example.com/",
        ];

        let mut out = String::new();
        for uri in plain {
            let (status, mime, len, hash) = match handle_protocol(&flyfish, &allowed, uri) {
                Some(r) => (r.status, r.mime.to_string(), r.body.len(), fnv1a(&r.body)),
                None => (0, String::from("-"), 0, 0),
            };
            out.push_str(&format!("{uri} => {status}|{mime}|{len}|{hash:016x}\n"));
        }
        for (display, _) in &cases {
            // 实际查询要把占位还原成真实路径,记录时再抹掉。
            let real = display.replace("<base>", &base_s);
            let (status, mime, len, hash) = match handle_protocol(&flyfish, &allowed, &real) {
                Some(r) => (r.status, r.mime.to_string(), r.body.len(), fnv1a(&r.body)),
                None => (0, String::from("-"), 0, 0),
            };
            out.push_str(&format!("{display} => {status}|{mime}|{len}|{hash:016x}\n"));
        }

        if std::env::var_os("UPDATE_GOLDEN").is_some() {
            fs::write(GOLDEN, &out).unwrap();
            return;
        }
        let want = fs::read_to_string(GOLDEN).unwrap_or_default();
        assert_eq!(out, want, "协议应答与黄金不一致(检查 URI 表或响应体)");
    }

    fn scratch() -> PathBuf {
        // 每次调用一个独占目录:测试并行时共用同一个文件会互相截断,读到空 body。
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("dozer-assets-test-{}-{n}", std::process::id()));
        let _ = fs::create_dir_all(dir.join("sub"));
        fs::write(dir.join("host.html"), b"<html>").unwrap();
        fs::write(dir.join("sub").join("a.js"), b"js").unwrap();
        dir
    }

    #[test]
    fn mime_for_covers_all_image_annotate_extensions() {
        for (ext, want) in [
            ("png", "image/png"),
            ("jpg", "image/jpeg"),
            ("jpeg", "image/jpeg"),
            ("gif", "image/gif"),
            ("webp", "image/webp"),
            ("bmp", "image/bmp"),
            ("ico", "image/x-icon"),
            ("svg", "image/svg+xml"),
        ] {
            assert_eq!(mime_for(Path::new(&format!("x.{ext}"))), want, "{ext}");
        }
    }

    #[test]
    fn serves_vendored_asset_with_mime() {
        let root = scratch();
        let r = handle_protocol(&root, &AllowedFiles::new(), "dozer://flyfish/host.html")
            .expect("预览命名空间");
        assert_eq!(r.status, 200);
        assert_eq!(r.mime, "text/html");
        assert_eq!(r.body, b"<html>");
        // 查询串被剥离(缓存破坏参数),不参与路径。
        let r = handle_protocol(&root, &AllowedFiles::new(), "dozer://flyfish/sub/a.js?v=1")
            .expect("预览命名空间");
        assert_eq!(r.status, 200);
        assert_eq!(r.mime, "text/javascript");
    }

    #[test]
    fn rejects_traversal_and_unknown() {
        let root = scratch();
        for uri in [
            "dozer://flyfish/../etc/passwd",
            "dozer://flyfish/sub/../../etc/passwd",
        ] {
            assert_eq!(
                handle_protocol(&root, &AllowedFiles::new(), uri)
                    .expect("预览命名空间")
                    .status,
                404,
                "{uri}"
            );
        }
        // 不属于预览的命名空间不再被当作 flyfish 根下的相对路径。
        assert!(handle_protocol(&root, &AllowedFiles::new(), "dozer://other/x").is_none());
        assert_eq!(
            handle_protocol(&root, &AllowedFiles::new(), "dozer://flyfish/nope.js")
                .expect("预览命名空间")
                .status,
            404
        );
    }

    #[test]
    fn file_endpoint_requires_allowlist() {
        let root = scratch();
        let f = root.join("sub").join("a.js");
        let uri = format!("dozer://flyfish/__file__{}", f.to_string_lossy());
        assert_eq!(
            handle_protocol(&root, &AllowedFiles::new(), &uri)
                .expect("预览命名空间")
                .status,
            404
        );
        let allowed: AllowedFiles = [f.clone()].into_iter().collect();
        let r = handle_protocol(&root, &allowed, &uri).expect("预览命名空间");
        assert_eq!(r.status, 200);
        assert_eq!(r.body, b"js");
    }

    #[test]
    fn file_endpoint_transcodes_utf16_to_utf8() {
        let root = scratch();
        let f = root.join("u16.rs");
        let mut bytes = vec![0xFF, 0xFE];
        for u in "fn main() {}\n".encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        fs::write(&f, &bytes).unwrap();
        let allowed: AllowedFiles = [f.clone()].into_iter().collect();
        let uri = format!("dozer://editor/__file__{}", f.to_string_lossy());
        let r = handle_protocol(&root, &allowed, &uri).expect("预览命名空间");
        assert_eq!(r.status, 200);
        assert_eq!(r.body, b"fn main() {}\n");
    }

    #[test]
    fn file_endpoint_leaves_utf8_untouched() {
        let root = scratch();
        let f = root.join("u8.rs");
        fs::write(&f, b"fn main() {}\n").unwrap();
        let allowed: AllowedFiles = [f.clone()].into_iter().collect();
        let uri = format!("dozer://editor/__file__{}", f.to_string_lossy());
        let r = handle_protocol(&root, &allowed, &uri).expect("预览命名空间");
        assert_eq!(r.body, b"fn main() {}\n");
    }

    #[test]
    fn file_endpoint_passes_odd_length_utf16_through() {
        let root = scratch();
        let f = root.join("odd.bin");
        fs::write(&f, [0xFF, 0xFE, 0x41]).unwrap();
        let allowed: AllowedFiles = [f.clone()].into_iter().collect();
        let uri = format!("dozer://editor/__file__{}", f.to_string_lossy());
        let r = handle_protocol(&root, &allowed, &uri).expect("预览命名空间");
        assert_eq!(r.body, [0xFF, 0xFE, 0x41]);
    }

    #[test]
    fn rejects_percent_encoded_traversal() {
        let root = scratch();
        for uri in [
            "dozer://flyfish/%2e%2e/etc/passwd",
            "dozer://flyfish/%2e%2e/%2e%2e/etc/passwd",
            "dozer://flyfish/sub/%2e%2e/%2e%2e/etc/passwd",
        ] {
            assert_eq!(
                handle_protocol(&root, &AllowedFiles::new(), uri)
                    .expect("预览命名空间")
                    .status,
                404,
                "{uri}"
            );
        }
    }

    #[test]
    fn bundle_resources_root_finds_app_ancestor() {
        let exe = Path::new("/tmp/X.app/Contents/MacOS/x");
        assert_eq!(
            bundle_resources_root(exe),
            Some(PathBuf::from("/tmp/X.app/Contents/Resources"))
        );
    }

    #[test]
    fn bundle_resources_root_none_outside_app() {
        assert_eq!(bundle_resources_root(Path::new("/tmp/build/x")), None);
    }

    #[test]
    fn percent_decode_roundtrip() {
        assert_eq!(percent_decode("a%20b").as_deref(), Some("a b"));
        assert_eq!(percent_decode("a%zzb"), None);
        assert_eq!(percent_decode("a%2"), None);
    }

    #[test]
    fn bundled_assets_root_uses_dev_root_outside_app() {
        let dev = Path::new("/src/assets/flyfish");
        assert_eq!(bundled_assets_root(Path::new("/tmp/build/x"), dev), dev);
        assert_eq!(
            bundled_assets_root(Path::new("/tmp/X.app/Contents/MacOS/x"), dev),
            PathBuf::from("/tmp/X.app/Contents/Resources/flyfish")
        );
    }

    fn scratch_editor() -> (PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!("dozer-editor-test-{}", std::process::id()));
        let flyfish = base.join("flyfish");
        let editor = base.join("editor");
        let _ = fs::create_dir_all(&flyfish);
        let _ = fs::create_dir_all(&editor);
        fs::write(flyfish.join("host.html"), b"flyfish").unwrap();
        fs::write(editor.join("index.html"), b"<html>editor").unwrap();
        fs::write(editor.join("main.js"), b"js").unwrap();
        (flyfish, base)
    }

    #[test]
    fn serves_editor_host_and_assets() {
        let (flyfish, _) = scratch_editor();
        let r = handle_protocol(&flyfish, &AllowedFiles::new(), "dozer://editor/index.html")
            .expect("预览命名空间");
        assert_eq!(r.status, 200);
        assert_eq!(r.mime, "text/html");
        assert_eq!(r.body, b"<html>editor");
        let r = handle_protocol(&flyfish, &AllowedFiles::new(), "dozer://editor/main.js?v=2")
            .expect("预览命名空间");
        assert_eq!(r.status, 200);
        assert_eq!(r.mime, "text/javascript");
    }

    #[test]
    fn editor_file_endpoint_requires_allowlist() {
        let (flyfish, base) = scratch_editor();
        let f = base.join("a.rs");
        fs::write(&f, b"fn main() {}").unwrap();
        let uri = format!("dozer://editor/__file__{}", f.to_string_lossy());
        assert_eq!(
            handle_protocol(&flyfish, &AllowedFiles::new(), &uri)
                .expect("预览命名空间")
                .status,
            404
        );
        let allowed: AllowedFiles = [f.clone()].into_iter().collect();
        let r = handle_protocol(&flyfish, &allowed, &uri).expect("预览命名空间");
        assert_eq!(r.status, 200);
        assert_eq!(r.body, b"fn main() {}");
    }

    #[test]
    fn editor_rejects_traversal_and_unknown() {
        let (flyfish, _) = scratch_editor();
        for uri in [
            "dozer://editor/../etc/passwd",
            "dozer://editor/%2e%2e/etc/passwd",
            "dozer://editor/nope.js",
        ] {
            assert_eq!(
                handle_protocol(&flyfish, &AllowedFiles::new(), uri)
                    .expect("预览命名空间")
                    .status,
                404,
                "{uri}"
            );
        }
    }

    #[test]
    fn serves_json_editor_namespace() {
        let (flyfish, base) = scratch_editor();
        let json = base.join("json-editor");
        fs::create_dir_all(&json).unwrap();
        fs::write(json.join("index.html"), b"<html>json").unwrap();
        let r = handle_protocol(
            &flyfish,
            &AllowedFiles::new(),
            "dozer://json-editor/index.html",
        )
        .expect("预览命名空间");
        assert_eq!(r.status, 200);
        assert_eq!(r.body, b"<html>json");
        let f = base.join("d.json");
        fs::write(&f, b"{}").unwrap();
        let uri = format!("dozer://json-editor/__file__{}", f.to_string_lossy());
        assert_eq!(
            handle_protocol(&flyfish, &AllowedFiles::new(), &uri)
                .expect("预览命名空间")
                .status,
            404
        );
        let allowed: AllowedFiles = [f].into_iter().collect();
        assert_eq!(
            handle_protocol(&flyfish, &allowed, &uri)
                .expect("预览命名空间")
                .status,
            200
        );
    }

    #[test]
    fn serves_tabular_namespace() {
        let (flyfish, base) = scratch_editor();
        let tab = base.join("tabular-host");
        fs::create_dir_all(&tab).unwrap();
        fs::write(tab.join("index.html"), b"<html>tabular").unwrap();
        let r = handle_protocol(&flyfish, &AllowedFiles::new(), "dozer://tabular/index.html")
            .expect("预览命名空间");
        assert_eq!(r.status, 200);
        assert_eq!(r.body, b"<html>tabular");
        assert_eq!(
            handle_protocol(&flyfish, &AllowedFiles::new(), "dozer://tabular/nope.js")
                .expect("预览命名空间")
                .status,
            404
        );
        // 没有 `__file__` 端点(数据只走 evaluate_script 推送)。
        let f = base.join("s.csv");
        fs::write(&f, b"a,b").unwrap();
        let allowed: AllowedFiles = [f.clone()].into_iter().collect();
        let uri = format!("dozer://tabular/__file__{}", f.to_string_lossy());
        assert_eq!(
            handle_protocol(&flyfish, &allowed, &uri)
                .expect("预览命名空间")
                .status,
            404
        );
    }

    #[test]
    fn serves_html_host_with_sandbox_and_strict_csp() {
        let (flyfish, _) = scratch_editor();
        let r = handle_protocol(&flyfish, &AllowedFiles::new(), "dozer://html/host.html")
            .expect("预览命名空间");
        assert_eq!(r.status, 200);
        assert_eq!(r.mime, "text/html");
        let html = String::from_utf8(r.body).unwrap();
        assert!(html.contains("Content-Security-Policy"));
        assert!(html.contains("default-src 'none'"));
        assert!(
            !html.contains("http://") && !html.contains("https://"),
            "隔离 HTML host 不得引用外部 URL"
        );
    }

    #[test]
    fn html_file_endpoint_scopes_to_opened_file_dirs() {
        let (flyfish, base) = scratch_editor();
        let opened = base.join("page.html");
        let sibling = base.join("site.css");
        let outside =
            std::env::temp_dir().join(format!("dozer-outside-{}.css", std::process::id()));
        fs::write(&opened, b"<html>").unwrap();
        fs::write(&sibling, b"body{}").unwrap();
        fs::write(&outside, b"x").unwrap();
        let allowed: AllowedFiles = [opened.clone()].into_iter().collect();

        // 已打开文件本身。
        let uri = format!("dozer://html/__file__{}", opened.to_string_lossy());
        assert_eq!(
            handle_protocol(&flyfish, &allowed, &uri)
                .expect("预览命名空间")
                .body,
            b"<html>"
        );
        // 同目录相对资源。
        let uri = format!("dozer://html/__file__{}", sibling.to_string_lossy());
        let r = handle_protocol(&flyfish, &allowed, &uri).expect("预览命名空间");
        assert_eq!(r.status, 200);
        assert_eq!(r.mime, "text/css");
        // 目录外文件拒绝。
        let uri = format!("dozer://html/__file__{}", outside.to_string_lossy());
        assert_eq!(
            handle_protocol(&flyfish, &allowed, &uri)
                .expect("预览命名空间")
                .status,
            404
        );
        // 未打开的未知页面 → 未知子路径。
        assert_eq!(
            handle_protocol(&flyfish, &AllowedFiles::new(), "dozer://html/nope.html")
                .expect("预览命名空间")
                .status,
            404
        );
        let _ = fs::remove_file(&outside);
    }

    #[test]
    fn serves_image_annotate_host() {
        let (flyfish, base) = scratch_editor();
        let img = base.join("image-annotate");
        fs::create_dir_all(&img).unwrap();
        fs::write(img.join("marker.png"), b"img").unwrap();
        let r = handle_protocol(
            &flyfish,
            &AllowedFiles::new(),
            "dozer://image-annotate/host.html",
        )
        .expect("预览命名空间");
        assert_eq!(r.status, 200);
        assert_eq!(r.mime, "text/html");
        assert!(
            String::from_utf8(r.body)
                .unwrap()
                .contains("Content-Security-Policy")
        );
        let r = handle_protocol(
            &flyfish,
            &AllowedFiles::new(),
            "dozer://image-annotate/marker.png",
        )
        .expect("预览命名空间");
        assert_eq!(r.status, 200);
        assert_eq!(r.body, b"img");

        let f = base.join("pic.png");
        fs::write(&f, b"png").unwrap();
        let uri = format!("dozer://image-annotate/__file__{}", f.to_string_lossy());
        assert_eq!(
            handle_protocol(&flyfish, &AllowedFiles::new(), &uri)
                .expect("预览命名空间")
                .status,
            404
        );
        let allowed: AllowedFiles = [f].into_iter().collect();
        assert_eq!(
            handle_protocol(&flyfish, &allowed, &uri)
                .expect("预览命名空间")
                .status,
            200
        );
    }

    #[test]
    fn image_annotate_rejects_traversal_and_unknown() {
        let (flyfish, _) = scratch_editor();
        for uri in [
            "dozer://image-annotate/../etc/passwd",
            "dozer://image-annotate/nope.png",
        ] {
            assert_eq!(
                handle_protocol(&flyfish, &AllowedFiles::new(), uri)
                    .expect("预览命名空间")
                    .status,
                404,
                "{uri}"
            );
        }
    }

    #[test]
    fn plantuml_viewer_serves_vendored_files() {
        let (flyfish, base) = scratch_editor();
        let puml = base.join("plantuml-viewer");
        fs::create_dir_all(puml.join("engine")).unwrap();
        fs::write(puml.join("index.html"), b"<html>puml").unwrap();
        fs::write(puml.join("engine").join("viz.js"), b"js").unwrap();
        let r = handle_protocol(
            &flyfish,
            &AllowedFiles::new(),
            "dozer://plantuml-viewer/index.html",
        )
        .expect("预览命名空间");
        assert_eq!(r.status, 200);
        assert_eq!(r.body, b"<html>puml");
        let r = handle_protocol(
            &flyfish,
            &AllowedFiles::new(),
            "dozer://plantuml-viewer/engine/viz.js",
        )
        .expect("预览命名空间");
        assert_eq!(r.status, 200);
        assert_eq!(r.mime, "text/javascript");
    }

    #[test]
    fn plantuml_viewer_rejects_traversal_and_has_no_file_endpoint() {
        let (flyfish, base) = scratch_editor();
        let puml = base.join("plantuml-viewer");
        fs::create_dir_all(&puml).unwrap();
        for uri in [
            "dozer://plantuml-viewer/../etc/passwd",
            "dozer://plantuml-viewer/%2e%2e/etc/passwd",
        ] {
            assert_eq!(
                handle_protocol(&flyfish, &AllowedFiles::new(), uri)
                    .expect("预览命名空间")
                    .status,
                404,
                "{uri}"
            );
        }
        let f = base.join("a.puml");
        fs::write(&f, b"@startuml").unwrap();
        let allowed: AllowedFiles = [f.clone()].into_iter().collect();
        let uri = format!("dozer://plantuml-viewer/__file__{}", f.to_string_lossy());
        assert_eq!(
            handle_protocol(&flyfish, &allowed, &uri)
                .expect("预览命名空间")
                .status,
            404
        );
    }

    #[test]
    fn unknown_scheme_and_foreign_namespaces_are_left_to_the_shell() {
        let root = scratch();
        assert!(handle_protocol(&root, &AllowedFiles::new(), "https://x/y").is_none());
        // `dozer://` 裸 scheme 也照旧 404(搬迁前的行为,不是"交给外壳")。
        assert_eq!(
            handle_protocol(&root, &AllowedFiles::new(), "dozer://")
                .expect("dozer:// 本身仍由预览应答")
                .status,
            404
        );
        for foreign in [
            "dozer://usage-content/x",
            "dozer://codehealth-content/x",
            "dozer://todo-content/x",
            "dozer://group-chat-content/x",
            "dozer://review-trace/data.json",
        ] {
            assert!(
                handle_protocol(&root, &AllowedFiles::new(), foreign).is_none(),
                "{foreign}"
            );
        }
        // 预览命名空间里的空路径仍然是 404(不是"交给外壳")。
        assert_eq!(
            handle_protocol(&root, &AllowedFiles::new(), "dozer://flyfish/")
                .expect("预览命名空间")
                .status,
            404
        );
    }

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

    #[test]
    fn tabular_host_bundle_assets_are_present() {
        let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/tabular-host"));
        for f in ["index.html", "tabular-host.js", "tabular-host.css"] {
            let p = root.join(f);
            assert!(p.is_file(), "缺少 tabular-host 产物 {f}: {}", p.display());
            assert!(
                std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0) > 0,
                "tabular-host 产物为空: {f}"
            );
        }
        let html = std::fs::read_to_string(root.join("index.html")).unwrap();
        assert!(html.contains("default-src 'none'"));
        assert!(html.contains("script-src 'self'"), "脚本仅 self");
        assert!(
            !html.contains("http://") && !html.contains("https://"),
            "tabular host 不得引用外部 URL(离线约束)"
        );
        assert!(html.contains("tabular-host.js") && html.contains("tabular-host.css"));
        let js = std::fs::read_to_string(root.join("tabular-host.js")).unwrap();
        assert!(!js.contains("sourceMappingURL"), "不应有 sourcemap 引用");
        assert!(
            !js.contains(env!("CARGO_MANIFEST_DIR")),
            "不应含源码树绝对路径"
        );
        assert!(!js.contains("/Users/"), "不应含用户绝对路径");
    }

    #[test]
    fn plantuml_viewer_bundle_assets_are_present() {
        let root = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/plantuml-viewer"
        ));
        for f in [
            "index.html",
            "bundle.js",
            "bundle.css",
            "plantuml.js",
            "viz-global.js",
            "themes.js",
            "emoji.js",
            "openiconic.js",
            "stdlib/c4.min.js",
        ] {
            let p = root.join(f);
            assert!(
                p.is_file(),
                "缺少 plantuml-viewer 产物 {f}: {}",
                p.display()
            );
            assert!(
                std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0) > 0,
                "plantuml-viewer 产物为空: {f}"
            );
        }
    }

    #[test]
    fn plantuml_viewer_bundle_has_no_absolute_paths_or_sourcemap() {
        let root = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/plantuml-viewer"
        ));
        let js = std::fs::read_to_string(root.join("bundle.js")).expect("读 bundle.js");
        assert!(!js.contains("sourceMappingURL"), "不应有 sourcemap 引用");
        assert!(
            !js.contains(env!("CARGO_MANIFEST_DIR")),
            "不应含源码树绝对路径"
        );
    }

    #[test]
    fn plantuml_viewer_host_has_strict_csp_and_no_external_refs() {
        let root = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/plantuml-viewer"
        ));
        let html = std::fs::read_to_string(root.join("index.html")).expect("读 index.html");
        assert!(html.contains("Content-Security-Policy"), "必须声明 CSP");
        assert!(
            html.contains("default-src 'none'"),
            "CSP 必须以 default-src 'none' 起步"
        );
        assert!(html.contains("script-src 'self'"), "脚本仅 self");
        assert!(
            !html.contains("connect-src"),
            "plantuml-viewer 无 fetch 端点,不应声明 connect-src"
        );
        assert!(
            !html.contains("unsafe-eval"),
            "主线程 TeaVM 路径不需要 unsafe-eval(§12.2)"
        );
        assert!(
            !html.contains("http://") && !html.contains("https://"),
            "index.html 不得引用外部 URL(离线约束)"
        );
        assert!(html.contains("bundle.js") && html.contains("viz-global.js"));
    }

    #[test]
    fn image_annotate_host_has_strict_csp_and_no_external_refs() {
        let html = include_str!("../assets/image_annotate_host.html");
        assert!(html.contains("Content-Security-Policy"), "必须声明 CSP");
        assert!(
            html.contains("default-src 'none'"),
            "CSP 必须以 default-src 'none' 起步"
        );
        assert!(html.contains("script-src 'self'"), "脚本仅 self");
        assert!(
            !html.contains("http://") && !html.contains("https://"),
            "host.html 不得引用外部 URL(离线约束)"
        );
        assert!(html.contains("vendor/openseadragon.esm.js"));
        assert!(html.contains("vendor/annotorious-openseadragon.esm.js"));
        assert!(html.contains("__dozerImageAnnotatePost"));
    }

    #[test]
    fn image_annotate_host_csp_allows_inline_and_eval() {
        let html = include_str!("../assets/image_annotate_host.html");
        assert!(html.contains("script-src 'self' 'unsafe-inline' 'unsafe-eval'"));
    }

    #[test]
    fn image_annotate_vendor_assets_are_present() {
        let root = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/image-annotate/vendor"
        ));
        for f in ["openseadragon.esm.js", "annotorious-openseadragon.esm.js"] {
            let p = root.join(f);
            assert!(
                p.is_file(),
                "缺少 image-annotate vendor 产物 {f}: {}",
                p.display()
            );
            assert!(
                std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0) > 0,
                "image-annotate vendor 产物为空: {f}"
            );
        }
    }

    #[test]
    fn image_annotate_vendor_assets_have_no_external_refs() {
        let root = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/image-annotate/vendor"
        ));
        for f in ["openseadragon.esm.js", "annotorious-openseadragon.esm.js"] {
            let js = std::fs::read_to_string(root.join(f)).expect("读 vendor 产物");
            for (needle, what) in [
                ("import(\"http", "动态 import"),
                ("import('http", "动态 import"),
                ("import( \"http", "动态 import"),
                ("import( 'http", "动态 import"),
                ("fetch(\"http", "fetch"),
                ("fetch('http", "fetch"),
                ("src=\"http", "<script src>"),
                ("src='http", "<script src>"),
            ] {
                assert!(
                    !js.contains(needle),
                    "{f} 含外部网络加载形态({what}: {needle}),违反离线约束"
                );
            }
        }
    }

    #[test]
    fn image_annotate_vendor_imports_are_relative() {
        for f in ["openseadragon.esm.js", "annotorious-openseadragon.esm.js"] {
            let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/image-annotate/vendor/");
            let js = std::fs::read_to_string(format!("{path}{f}")).unwrap();
            assert!(
                !js.contains("from\"/") && !js.contains("import\"/"),
                "{f} 含根路径 import,dozer:// 下无法解析"
            );
        }
    }

    #[test]
    fn image_annotate_file_endpoint_requires_allowlist() {
        let root = scratch();
        let f = root.join("photo.png");
        fs::write(&f, b"\x89PNG\r\n").unwrap();
        let uri = format!("dozer://image-annotate/__file__{}", f.to_string_lossy());
        assert_eq!(
            handle_protocol(&root, &AllowedFiles::new(), &uri)
                .expect("预览命名空间")
                .status,
            404
        );
        let mut allowed = AllowedFiles::new();
        allowed.insert(f.clone());
        let r = handle_protocol(&root, &allowed, &uri).expect("预览命名空间");
        assert_eq!((r.status, r.mime), (200, "image/png"));
    }
}

#[cfg(test)]
mod allowed_files_tests {
    use super::*;
    use std::fs;

    #[test]
    fn equivalent_spellings_share_one_entry() {
        // 白名单现在按 canonical 路径比较(D-I2),所以夹具必须是真实文件。
        let base = unique_temp_dir("allowed-equivalent");
        let dir = base.join("a");
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("b.txt");
        fs::write(&file, b"x").unwrap();

        let mut allowed = AllowedFiles::new();
        allowed.insert(base.join("./a//b.txt"));
        allowed.insert(dir.join("b.txt").join(""));
        assert_eq!(allowed.len(), 1);
        assert!(allowed.contains(&file), "canonical 命中");
        let dotted = base.join("x").join("..").join("a").join("b.txt");
        assert!(allowed.contains(&dotted), ".. 被消解后等价");

        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn remove_and_clear() {
        let base = unique_temp_dir("allowed-remove");
        fs::create_dir_all(&base).unwrap();
        let a = base.join("a.txt");
        let b = base.join("b.txt");
        fs::write(&a, b"a").unwrap();
        fs::write(&b, b"b").unwrap();

        let mut allowed: AllowedFiles = [a.clone()].into_iter().collect();
        assert!(allowed.contains(&a));
        allowed.remove(&base.join(".").join("a.txt"));
        assert!(!allowed.contains(&a));
        allowed.insert(&b);
        allowed.clear();
        assert!(allowed.is_empty());

        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn remove_under_drops_only_that_project() {
        let base = unique_temp_dir("allowed-remove-under");
        let p1 = base.join("p1");
        let p1_sub = p1.join("sub");
        let p10 = base.join("p10");
        let p2 = base.join("p2");
        for dir in [&p1, &p1_sub, &p10, &p2] {
            fs::create_dir_all(dir).unwrap();
        }
        let a = p1.join("a.txt");
        let b = p1_sub.join("b.png");
        let c = p10.join("c.txt");
        let d = p2.join("d.md");
        for f in [&a, &b, &c, &d] {
            fs::write(f, b"x").unwrap();
        }

        let mut allowed: AllowedFiles = [a.clone(), b.clone(), c.clone(), d.clone()]
            .into_iter()
            .collect();
        allowed.remove_under(&p1);
        assert!(!allowed.contains(&a));
        assert!(!allowed.contains(&b));
        assert!(allowed.contains(&c), "同名前缀的兄弟项目不能被误删");
        assert!(allowed.contains(&d));

        fs::remove_dir_all(&base).ok();
    }

    /// D-I2:`/tmp` 与 `/private/tmp` 是同一目录的两种写法,白名单按 canonical
    /// 比较,因此登记任一种都能命中另一种。
    #[cfg(target_os = "macos")]
    #[test]
    fn allowlist_unifies_tmp_and_private_tmp() {
        let dir = std::env::temp_dir();
        let link = Path::new("/tmp");
        let (Some(canon_dir), Ok(canon_link)) = (
            std::fs::canonicalize(&dir).ok(),
            std::fs::canonicalize(link),
        ) else {
            return; // 该机器上 /tmp 不是链接,跳过。
        };
        if canon_dir != canon_link {
            return;
        }
        let name = format!("dozer-tmp-{}.txt", std::process::id());
        let file = canon_dir.join(&name);
        fs::write(&file, b"x").unwrap();

        let mut allowed = AllowedFiles::new();
        allowed.insert(link.join(&name));
        assert!(allowed.contains(&file), "/tmp 与 /private/tmp 必须等价");

        fs::remove_file(&file).ok();
    }

    #[test]
    fn no_parent_escape_past_the_root() {
        assert_eq!(
            clean_allowed_path(Path::new("/base/../../etc/passwd")),
            PathBuf::from("/etc/passwd"),
            ".. 不能越过根,但根内的消解照常"
        );
        assert_eq!(
            clean_allowed_path(Path::new("/a/b/../../../c")),
            PathBuf::from("/c")
        );
    }

    #[test]
    fn relative_paths_are_anchored_to_the_current_dir() {
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(
            clean_allowed_path(Path::new("rel/./x")),
            clean_allowed_path(&cwd.join("rel/x"))
        );
    }

    /// D-I2:白名单判定与读取必须落在同一个 canonical 路径上。
    ///
    /// 探针(第二轮复审):白名单只含 `proj/ok.txt`,`proj/link` 是指向
    /// `secret/deeper` 的符号链接,请求 `__file__/.../proj/link/../ok.txt`
    /// 曾经返回 200 和 `secret/ok.txt` 的内容。修复前这条测试必红。
    #[cfg(unix)]
    #[test]
    fn allowlist_rejects_traversal_encoded_slash_nul_and_symlink_escape() {
        use std::os::unix::fs::symlink;

        let base = unique_temp_dir("allowlist-traversal");
        let proj = base.join("proj");
        let secret = base.join("secret");
        let deeper = secret.join("deeper");
        for dir in [&proj, &deeper] {
            fs::create_dir_all(dir).unwrap();
        }
        let ok = proj.join("ok.txt");
        fs::write(&ok, b"public").unwrap();
        // `proj/link` → `secret/deeper`,于是 `proj/link/../ok.txt` 在系统眼里
        // 是 `secret/ok.txt`(词法折叠却仍是 `proj/ok.txt`)。
        fs::write(secret.join("ok.txt"), b"SECRET").unwrap();
        fs::write(deeper.join("ok.txt"), b"SECRET-DEEPER").unwrap();
        symlink(&deeper, proj.join("link")).unwrap();

        let mut allowed = AllowedFiles::new();
        allowed.insert(&ok);

        let encoded = |p: &std::path::Path| p.to_string_lossy().replace('%', "%25");
        let reply = |uri: String| handle_protocol(&base, &allowed, &uri).expect("预览命名空间");

        // 正当读取仍然成功。
        let direct = reply(format!("dozer://editor/__file__{}", encoded(&ok)));
        assert_eq!(direct.status, 200);
        assert_eq!(direct.body, b"public");

        // 符号链接逃逸:词法上落在 proj 里,实质读到 secret 下的同名文件。
        for uri in [
            format!(
                "dozer://editor/__file__{}",
                encoded(&proj.join("link/../ok.txt"))
            ),
            format!(
                "dozer://editor/__file__{}",
                encoded(&proj.join("link/../../secret/ok.txt"))
            ),
        ] {
            let r = reply(uri.clone());
            assert_eq!(r.status, 404, "{uri} 必须 404");
            assert_ne!(r.body, b"SECRET", "{uri} 泄露了白名单外的内容");
        }

        // 编码形态的 `..` 与 NUL 也不能放行。
        let dotdot = proj.join("link").to_string_lossy().to_string() + "/%2e%2e/ok.txt";
        assert_eq!(
            reply(format!("dozer://editor/__file__{dotdot}")).status,
            404
        );
        assert_eq!(
            reply(format!(
                "dozer://editor/__file__{}/ok%00.txt",
                encoded(&proj)
            ))
            .status,
            404
        );
        // 不在白名单里的普通文件照样 404。
        assert_eq!(
            reply(format!(
                "dozer://editor/__file__{}",
                encoded(&secret.join("ok.txt"))
            ))
            .status,
            404
        );

        fs::remove_dir_all(&base).ok();
    }

    /// D-I2:判定按 canonical 路径比较(`/tmp` ↔ `/private/tmp`),目录一律拒绝。
    #[cfg(unix)]
    #[test]
    fn allowlist_compares_canonical_paths_and_rejects_directories() {
        let base = unique_temp_dir("allowlist-canonical");
        fs::create_dir_all(&base).unwrap();
        let file = base.join("doc.txt");
        fs::write(&file, b"hello").unwrap();

        let mut allowed = AllowedFiles::new();
        allowed.insert(&file);
        let canonical = fs::canonicalize(&file).unwrap();
        assert!(allowed.contains(&canonical), "canonical 形式必须命中");

        // macOS 上 /tmp 与 /private/tmp 是同一目录的两种写法。
        let tmp = std::env::temp_dir();
        let via_tmp = tmp.join(
            file.strip_prefix(fs::canonicalize(&tmp).unwrap_or(tmp.clone()))
                .unwrap_or(&file),
        );
        let _ = via_tmp; // 平台无关:上面的 canonical 断言已覆盖语义。

        // 目录即使被登记也不能被当作文件读。
        let mut with_dir = AllowedFiles::new();
        let stored = with_dir.insert(&base);
        assert!(!stored.is_file(), "目录登记后仍是目录");
        assert!(with_dir.contains(&base), "目录本身仍在集合里");
        assert!(with_dir.resolve(&base).is_none(), "目录不能被读");
        assert!(
            with_dir.resolve(&file).is_none(),
            "未登记的子孙文件不能被读"
        );

        // 不存在的路径:登记失败、查询为 false(不降级放行)。
        let missing = base.join("nope.txt");
        let returned = allowed.insert(&missing);
        assert_eq!(returned, missing, "失败时返回折叠路径供报错");
        assert!(!allowed.contains(&missing), "未登记成功就不能命中");

        fs::remove_dir_all(&base).ok();
    }

    /// D-I2(计划点名):带空格 / 中文 / `#` / `?` / `%` / `&` 的文件名能正确往返。
    #[test]
    fn file_url_roundtrips_names_with_space_cjk_hash_question_percent_ampersand() {
        let base = unique_temp_dir("allowlist-roundtrip");
        fs::create_dir_all(&base).unwrap();
        let name = "带 空格#问号?百分之%和&.txt";
        let file = base.join(name);
        fs::write(&file, b"payload").unwrap();

        let mut allowed = AllowedFiles::new();
        allowed.insert(&file);

        let encoded = name
            .replace('%', "%25")
            .replace(' ', "%20")
            .replace('#', "%23")
            .replace('?', "%3F")
            .replace('&', "%26");
        let uri = format!(
            "dozer://editor/__file__{}/{encoded}",
            base.to_string_lossy()
        );
        let reply = handle_protocol(&base, &allowed, &uri).expect("预览命名空间");
        assert_eq!(reply.status, 200, "{uri}");
        assert_eq!(reply.body, b"payload");
        assert_eq!(reply.mime, "text/plain");

        fs::remove_dir_all(&base).ok();
    }

    /// 每个测试用独立目录:协议黄金测试也在 `temp_dir` 下建夹具,不能互相踩。
    fn unique_temp_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!("dozer-{tag}-{}-{nanos}", std::process::id()))
    }
}
