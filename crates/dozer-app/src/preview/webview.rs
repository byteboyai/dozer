//! webview 期望清单与 URL 构造:WebviewSpec/encode_component/flyfish_url/
//! file_url/preview_url。

use super::*;

/// main.rs 同步 webview 的期望清单项。
#[derive(Debug, Clone, PartialEq)]
pub struct WebviewSpec {
    pub id: usize,
    pub url: String,
    pub visible: bool,
}

/// RFC3986 严格百分号编码:unreserved(字母/数字/`-._~`)之外全部 %XX。
pub fn encode_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// `TabKind::File` → flyfish 渲染 URL 的唯一决策点。目前只有这一条渲染
/// 路径;把它从内联拼接抽成具名函数,是为将来"某些扩展名不走 flyfish、
/// 走 Acceptance 式 iced 原生 pane"的分叉预留一个函数级插入点——不引入
/// trait/注册表,YAGNI。
///
/// 文本文件(`is_editable_extension`)额外挂 `&ln=1`:host.html 读到后给
/// flyfish 的 text 渲染器开 `options.text.lineNumbers`,预览里显示行号
///(图片/PDF 渲染器不认这个 option,挂了也无副作用)。
pub(crate) fn flyfish_url(path: &std::path::Path) -> String {
    let mut u = format!(
        "dozer://flyfish/host.html?p={}",
        encode_component(&path.to_string_lossy())
    );
    if is_editable_extension(path) {
        u.push_str("&ln=1");
    }
    u
}

/// html/htm 走真实 `file://` URL 直接加载,不经 flyfish——flyfish 的渲染
/// 器把 html/htm 也归进它自己的通用文本/源码管线(不是当网页渲染),给它
/// 加 `prefers_rendered_preview` 只是换个地方显示源码,达不到"像 md 一样
/// 渲染出效果"的目的(核心原则见 CLAUDE.md:预览应该让用户看到 AI 产出的
/// 实际效果)。让 wry 直接加载文件本身的 `file://` URL,WKWebView 按普通
/// 网页处理,相对路径引用的 css/js/图片按文件所在目录自然解析,不用额外
/// 起服务。`p`(每段路径分量分别编码,保留 `/` 分隔符,不能直接套
/// `encode_component` 整段编码——那会把 `/` 也转义掉,破坏 URL 结构)。
pub(crate) fn file_url(path: &std::path::Path) -> String {
    let encoded_segments: Vec<String> = path
        .to_string_lossy()
        .split('/')
        .map(encode_component)
        .collect();
    format!("file://{}", encoded_segments.join("/"))
}

/// `TabKind::File` → wry 期望加载的 URL,按扩展名分派两条渲染路径。
pub(crate) fn preview_url(path: &std::path::Path) -> String {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "html" | "htm" => file_url(path),
        _ => flyfish_url(path),
    }
}
