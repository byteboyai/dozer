//! webview 期望清单与 URL 构造:WebviewSpec/encode_component/flyfish_url/
//! file_url/preview_url。

use super::*;

/// main.rs 同步 webview 的期望清单项。
#[derive(Debug, Clone, PartialEq)]
pub struct WebviewSpec {
    pub id: usize,
    pub url: String,
    pub visible: bool,
    pub editor_binding: Option<EditorHostBinding>,
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
///
/// `&theme=light|dark` 让预览渲染跟随 dozer 当前配色方案(单一真相源
/// `byteui::theme::color::current_scheme()`)——host.html 读它设置
/// flyfish 的 `theme` 属性,不再硬编码深色。切主题时由 `App` 推进所有
/// wry 文件 tab 的 `reload_nonce`,让 webview 带着新 theme 参数重新导航。
pub(crate) fn flyfish_url(path: &std::path::Path) -> String {
    let mut u = format!(
        "dozer://flyfish/host.html?p={}&theme={}",
        encode_component(&path.to_string_lossy()),
        crate::preview::scheme_query_value()
    );
    if is_editable_extension(path) {
        u.push_str("&ln=1");
    }
    u
}

/// 当前配色方案 → flyfish `theme` 属性/URL 参数取值(`light`/`dark`),
/// host.html 与 `flyfish_url` 共用一份,避免两处各写一次映射。
pub(crate) fn scheme_query_value() -> &'static str {
    match byteui::theme::color::current_scheme() {
        byteui::theme::color::ColorScheme::Light => "light",
        byteui::theme::color::ColorScheme::Dark => "dark",
    }
}

/// HTML/HTM 的隔离 host URL:不再直接 `file://` 加载,改走 `dozer://html/host.html`,
/// 由 host 把绑定文件放进**无脚本 sandbox iframe** 渲染,相对资源经
/// `dozer://html/__file__` 白名单(已打开文件所在目录子树)解析。详见
/// `assets::serve_html_file` 与内嵌 `html_host.html`。
pub(crate) fn html_url(path: &std::path::Path) -> String {
    format!(
        "dozer://html/host.html?p={}&theme={}",
        encode_component(&path.to_string_lossy()),
        scheme_query_value()
    )
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
        "html" | "htm" => html_url(path),
        _ => flyfish_url(path),
    }
}
