//! webview 期望清单与 URL 构造:WebviewSpec/encode_component/flyfish_url/
//! file_url/preview_url。

use super::*;
use crate::app::PanelKind;

/// main.rs 同步 webview 的期望清单项。
#[derive(Debug, Clone, PartialEq)]
pub struct WebviewSpec {
    pub id: usize,
    pub url: String,
    pub visible: bool,
    pub editor_binding: Option<EditorHostBinding>,
    /// T10:该 host 若在 `Reserving` 阶段等待预算,携带其 loading `generation`。
    /// `sync_webview_pool` 在 reserve 被批准时把 `(key, generation)` 回灌给
    /// 调用方,由 `apply_preview_pool_outcome` 世代校验后推进到 `CreatingHost`;
    /// 仅 host 类(editor/JSON)且在途加载时为 `Some`。
    pub loading_generation: Option<u64>,
    /// 「创建但尚未就绪」的 Rendered 宿主(Flyfish/隔离 HTML)用**离屏停放**
    /// 代替 `set_visible(false)`。
    ///
    /// 起因(2026-09-27,docx 预览「加载超时」):Flyfish 宿主为避免空白子视图
    /// 露出,一律先以 hidden 创建、等 `document_loaded` 才 `set_visible(true)`。
    /// 但 WKWebView 对 **hidden** 视图会挂起 `requestAnimationFrame`(与尺寸
    /// 无关,`isHidden=true` 即停);而 docx 的 word 渲染器 `load()` 依赖
    /// 「字体就绪 + 双 rAF」做分页布局(见 `awaitLayout`),于是 hidden 下
    /// `load()` 永不 resolve → 15s `CreatingHost` 看门狗超时 → 用户看到
    /// 「预览加载超时,请重试」。image/pdf/text 渲染器在 resolve 前不 await
    /// rAF,故不受影响(这也是该 bug 只砸 docx/office 的原因)。
    ///
    /// 处置:对「未就绪」的 Rendered 宿主,把子视图停到窗口外(负 x),维持
    /// `visible=true`——WebKit 因此照常跑 rAF,渲染能完成;而离屏矩形被窗口
    /// 裁剪,用户不可见,与 `set_visible(false)` 的观感等价。就绪后再回到真实
    /// bounds。真正「已就绪但被浮层/非激活隐藏」的场景仍走 `set_visible(false)`
    /// (`park_offscreen=false`),以免后台 tab 常驻渲染空耗。
    pub park_offscreen: bool,
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
/// 文本文件(`is_editable_extension`)额外挂两个参数:
/// - `&ln=1`:host.html 读到后给 flyfish 的 text 渲染器开
///   `options.text.lineNumbers`,预览里显示行号(图片/PDF 渲染器不认这个
///   option,挂了也无副作用)。
/// - `&fs=<px>`:预览字号跟随终端(单一真相源 `terminal_font::size()`),
///   host.html 读到后用注入的 CSS 把 `.markdown-body` 缩到一致大小,使
///   Markdown 预览与终端字号统一。缺省/异常值不挂,保持 flyfish 默认。
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
        // 文本/Markdown 预览字号跟随终端,与终端保持一致(`terminal_font::size()`
        // 已是唯一真相源,如 assets/theme/terminal.json 的 14.0)。
        u.push_str(&format!("&fs={}", crate::theme::terminal_font::size()));
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

/// T9/T8:从 Rendered host(Flyfish 或隔离 HTML)URL 的查询串解析归属绑定
/// (`proj`/`panel`/`tab`/`doc`),供 host 回传 envelope 时校验归属。非 Rendered
/// host URL 或缺字段返回 `None`。
pub(crate) fn flyfish_binding_from_url(url: &str) -> Option<HostBinding> {
    if !url.starts_with("dozer://flyfish/") && !url.starts_with("dozer://html/") {
        return None;
    }
    let query = url.split_once('?')?.1;
    let mut proj = None;
    let mut panel = None;
    let mut tab = None;
    let mut doc = None;
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        match k {
            "proj" => proj = v.parse::<i64>().ok(),
            "panel" => panel = Some(v.to_string()),
            "tab" => tab = v.parse::<usize>().ok(),
            "doc" => doc = crate::assets::percent_decode(v),
            _ => {}
        }
    }
    let panel = match panel.as_deref() {
        Some("project") => PanelKind::Project,
        Some("files") => PanelKind::Files,
        _ => return None,
    };
    Some(HostBinding::new(proj?, panel, tab?, doc?))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binding_parses_both_rendered_hosts() {
        let q = "?p=x&proj=3&panel=files&tab=7&doc=p3-t7";
        let flyfish = flyfish_binding_from_url(&format!("dozer://flyfish/host.html{q}"));
        assert!(flyfish.is_some());
        // T8:隔离 HTML host 与 Flyfish 共用同一 envelope 绑定解析。
        let html = flyfish_binding_from_url(&format!("dozer://html/host.html{q}"));
        assert_eq!(html.map(|b| b.tab_id), Some(7));
    }

    #[test]
    fn binding_ignores_other_hosts_and_incomplete_queries() {
        assert!(
            flyfish_binding_from_url("dozer://editor/x?proj=1&panel=files&tab=2&doc=d").is_none()
        );
        assert!(flyfish_binding_from_url("dozer://flyfish/host.html?p=x").is_none());
    }
}
