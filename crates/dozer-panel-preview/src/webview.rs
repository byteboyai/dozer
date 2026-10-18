//! webview 期望清单与 URL 构造:WebviewSpec/encode_component/flyfish_url/
//! file_url/preview_url。

use super::*;
use crate::ViewerCost;

/// 不携带 `EditorHostBinding`(非 editor/JSON/tabular host)但**仍需计入
/// 预览资源预算**的 viewer 的 reserve 线索。PlantUML Rendered host 用它:
/// 它没有 CodeMirror 的 path 绑定,却要占用一个重型 WebView 名额、按
/// `estimate_cost` 的固定引擎开销计费(规格 §7)。
///
/// `ViewerKey` 即 `(project_id, panel, tab_id)`,与 editor host 共用同一套
/// reserve/register/touch/evict/deny 闭环;`cost` 用显式估算,不回落到
/// `fs::metadata` 的编辑器启发式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReserveHint {
    pub project_id: i64,
    pub panel: ViewerPanel,
    pub tab_id: usize,
    pub cost: ViewerCost,
}

impl ReserveHint {
    pub fn key(&self) -> crate::ViewerKey {
        (self.project_id, self.panel, self.tab_id)
    }
}

/// 库产出的 webview 期望清单项(纯数据)。产品(Dozer 的 `WebviewSpec`)在适配层补上
/// 共享宿主 Surface 身份 / 应用外壳 i18n 投影等预览不关心的字段。
#[derive(Debug, Clone, PartialEq)]
pub struct PreviewWebviewSpec {
    pub id: usize,
    pub url: String,
    pub visible: bool,
    pub editor_binding: Option<EditorHostBinding>,
    /// 无 `editor_binding` 但需计入资源预算的 viewer 的 reserve 线索
    /// (目前仅 PlantUML Rendered host)。`None` = 不走 reserve(与
    /// Flyfish/HTML 等 Rendered host 的既有行为一致)。
    pub reserve: Option<ReserveHint>,
    /// T10:该 host 若在 `Reserving` 阶段等待预算,携带其 loading `generation`。
    /// 仅 host 类(editor/JSON)且在途加载时为 `Some`。
    pub loading_generation: Option<u64>,
    /// 「创建但尚未就绪」的 Rendered 宿主(Flyfish/隔离 HTML)用**离屏停放**
    /// 代替 `set_visible(false)`:WKWebView 对 hidden 视图会挂起 `requestAnimationFrame`,
    /// docx 渲染器的分页布局依赖 rAF,hidden 下永不 resolve 会触发「加载超时」(2026-09-27)。
    /// 就绪后再回到真实 bounds;「已就绪但被浮层/非激活隐藏」仍走 `set_visible(false)`。
    pub park_offscreen: bool,
}

/// 百分号解码(`encode_component` 的逆);非法转义或非 UTF-8 返回 `None`。
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
pub fn flyfish_url(path: &std::path::Path, locale: &str, font_size: f32) -> String {
    let mut u = format!(
        "dozer://flyfish/host.html?p={}&theme={}&locale={}",
        encode_component(&path.to_string_lossy()),
        crate::scheme_query_value(),
        locale
    );
    if is_editable_extension(path) {
        u.push_str("&ln=1");
        // 文本/Markdown 预览字号跟随终端,与终端保持一致(`terminal_font::size()`
        // 已是唯一真相源,如 assets/theme/terminal.json 的 14.0)。
        u.push_str(&format!("&fs={}", font_size));
    }
    u
}

/// 当前配色方案 → flyfish `theme` 属性/URL 参数取值(`light`/`dark`),
/// host.html 与 `flyfish_url` 共用一份,避免两处各写一次映射。
pub fn scheme_query_value() -> &'static str {
    match byteui::theme::color::current_scheme() {
        byteui::theme::color::ColorScheme::Light => "light",
        byteui::theme::color::ColorScheme::Dark => "dark",
    }
}

// `is_html_extension` 已搬进 `dozer-preview-core::native_editor`(后端选择要用),经 `preview` 的
// `native_editor::*` 通配导出,路径不变。

/// HTML/HTM 的隔离 host URL:不再直接 `file://` 加载,改走 `dozer://html/host.html`,
/// 由 host 把绑定文件放进**无脚本 sandbox iframe** 渲染,相对资源经
/// `dozer://html/__file__` 白名单(已打开文件所在目录子树)解析。详见
/// `assets::serve_html_file` 与内嵌 `html_host.html`。
pub fn html_url(path: &std::path::Path, locale: &str) -> String {
    format!(
        "dozer://html/host.html?p={}&theme={}&locale={}",
        encode_component(&path.to_string_lossy()),
        scheme_query_value(),
        locale
    )
}

/// image-annotate host 的 URL:与 `flyfish_url` 同为 Rendered host,但走
/// OpenSeadragon + Annotorious 图片查看/标注 host。仅 `png/jpg/jpeg/webp/bmp/ico`
/// 经此(判据 `router::is_image_annotate_extension`),`gif/tif/tiff` 仍走 Flyfish。
pub fn image_annotate_url(path: &std::path::Path, locale: &str) -> String {
    format!(
        "dozer://image-annotate/host.html?p={}&theme={}&locale={}",
        encode_component(&path.to_string_lossy()),
        scheme_query_value(),
        locale
    )
}

/// 是否是需要 T9 envelope 绑定(proj/panel/tab/doc)的 Rendered host URL——
/// Flyfish / 隔离 HTML / 图片标注 三者之一。`app.rs` 据此决定要不要往 URL
/// 追加绑定查询串,`flyfish_binding_from_url` 据此决定要不要解析,避免两处
/// 各写一份前缀列表、迟早漏改其中一处。
pub fn hosts_rendered_binding(url: &str) -> bool {
    url.starts_with("dozer://flyfish/")
        || url.starts_with("dozer://html/")
        || url.starts_with("dozer://image-annotate/")
        || url.starts_with("dozer://plantuml-viewer/")
}

/// T9/T8:从 Rendered host(Flyfish、隔离 HTML 或 image-annotate)URL 的查询串
/// 解析归属绑定(`proj`/`panel`/`tab`/`doc`),供 host 回传 envelope 时校验归属。
/// 非 Rendered host URL 或缺字段返回 `None`。
pub fn flyfish_binding_from_url(url: &str) -> Option<HostBinding> {
    if !hosts_rendered_binding(url) {
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
            "doc" => doc = percent_decode(v),
            _ => {}
        }
    }
    let panel = match panel.as_deref() {
        Some("project") => ViewerPanel::Project,
        Some("files") => ViewerPanel::Files,
        _ => return None,
    };
    Some(HostBinding::new(proj?, panel, tab?, doc?))
}

/// `TabKind::File` → wry 期望加载的 URL。**按 backend 的 renderer 分派**
/// (而非只凭扩展名再猜一次):PlantUML 走 `dozer://plantuml-viewer/index.html`,
/// 隔离 HTML 走 html host,image-annotate 走图片 host,其余(含 gif/tif/tiff)
/// 回落 Flyfish。
///
/// `backend` 为 `None`(backend 尚未判定/已判为不 host)时退化为按扩展名分派,
/// 与 PlantUML 引入前的行为一致——但正常路径下调用方 `desired_webviews` 一定
/// 已持有 `Some(PreviewBackend::Rendered(..))`。扩展名判据仍集中在
/// `router::is_image_annotate_extension`,不再额外新增 PlantUML 扩展名判断。
pub fn preview_url(
    path: &std::path::Path,
    backend: Option<&PreviewBackend>,
    locale: &str,
    font_size: f32,
) -> String {
    if let Some(PreviewBackend::Rendered(rendered)) = backend {
        match rendered.renderer {
            RenderedRenderer::PlantUml => return plantuml_viewer_url(locale),
            RenderedRenderer::IsolatedHtml => return html_url(path, locale),
            RenderedRenderer::Flyfish => {
                // Flyfish 仍然承载 image-annotate 图片(OpenSeadragon)——图片
                // 扩展名由 router 判据决定,不由 renderer 决定。
                if is_image_annotate_extension(path) {
                    return image_annotate_url(path, locale);
                }
                return flyfish_url(path, locale, font_size);
            }
        }
    }
    if is_image_annotate_extension(path) {
        return image_annotate_url(path, locale);
    }
    if is_html_extension(path) {
        html_url(path, locale)
    } else {
        flyfish_url(path, locale, font_size)
    }
}

/// PlantUML viewer host 的 URL。与其它 Rendered host 一样在调用处再追加
/// `proj`/`panel`/`tab`/`doc` 绑定查询串(`hosts_rendered_binding`)。
pub fn plantuml_viewer_url(locale: &str) -> String {
    format!(
        "dozer://plantuml-viewer/index.html?theme={}&locale={}",
        scheme_query_value(),
        locale
    )
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

    #[test]
    fn image_annotate_url_targets_its_namespace_and_carries_theme() {
        let u = image_annotate_url(std::path::Path::new("/tmp/图 a.png"), "en");
        assert!(u.starts_with("dozer://image-annotate/host.html?p="), "{u}");
        assert!(u.contains("&theme="), "{u}");
    }

    #[test]
    fn hosts_rendered_binding_covers_all_four_prefixes() {
        assert!(hosts_rendered_binding("dozer://flyfish/host.html?p=x"));
        assert!(hosts_rendered_binding("dozer://html/host.html?p=x"));
        assert!(hosts_rendered_binding(
            "dozer://image-annotate/host.html?p=x"
        ));
        assert!(hosts_rendered_binding(
            "dozer://plantuml-viewer/index.html?theme=dark"
        ));
        assert!(!hosts_rendered_binding("dozer://editor/index.html"));
    }

    #[test]
    fn plantuml_viewer_url_targets_namespace_and_binds() {
        let u = plantuml_viewer_url("en");
        assert!(u.starts_with("dozer://plantuml-viewer/index.html?"), "{u}");
        // 与其它 rendered host 一样,绑定由调用处(desired_webviews→app.rs)注入;
        // 解析走同一 `flyfish_binding_from_url`。
        let b = flyfish_binding_from_url(&format!("{u}&proj=3&panel=files&tab=7&doc=p3-t7"));
        assert_eq!(
            b.map(|b| (b.tab_id, b.panel)),
            Some((7, ViewerPanel::Files))
        );
    }

    #[test]
    fn preview_url_uses_backend_renderer_for_plantuml() {
        use crate::{PreviewBackend, RenderedBackend, RenderedMode, RenderedRenderer};
        let backend = PreviewBackend::Rendered(RenderedBackend {
            renderer: RenderedRenderer::PlantUml,
            mode: RenderedMode::Rendered,
            source_language: Some("plantuml".to_string()),
        });
        // 即便路径无关(fixture 文件名),只要 backend 说是 PlantUML 就走 viewer。
        let u = preview_url(
            std::path::Path::new("/tmp/diagram.puml"),
            Some(&backend),
            "en",
            14.0,
        );
        assert!(u.starts_with("dozer://plantuml-viewer/index.html?"), "{u}");
        // 无 backend 时退化为扩展名分派:PlantUML 扩展名在没有 backend 判定时
        // 不特殊处理,仍回落 flyfish(ProductInt:backend 是唯一身份来源)。
        let fallback = preview_url(std::path::Path::new("/tmp/diagram.puml"), None, "en", 14.0);
        assert!(fallback.starts_with("dozer://flyfish/"), "{fallback}");
    }

    #[test]
    fn image_annotate_binding_from_url_parses_query() {
        let q = "?p=x&proj=3&panel=files&tab=7&doc=p3-t7";
        let b = flyfish_binding_from_url(&format!("dozer://image-annotate/host.html{q}"));
        assert_eq!(
            b.map(|b| (b.tab_id, b.panel)),
            Some((7, ViewerPanel::Files))
        );
        // 走 image_annotate_url 构造出的 URL 本身不挂 proj/panel/tab/doc(由
        // app.rs 注入),故直接解析应为 None——注入后才可解析(E2E 覆盖)。
        assert!(
            flyfish_binding_from_url(&image_annotate_url(std::path::Path::new("a.png"), "en"))
                .is_none()
        );
    }
}
