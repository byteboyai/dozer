//! 应用面板的 webview(bytehost A4b1):已安装应用(`PanelKind::App`)在 rail 面板里用 wry 子视图加载
//! `http://<app-id>.localhost:<端口>/`。**应用代码不可信**(第三方或 agent 生成),所以本模块只放三类东西:
//!
//! - 纯判断:这个 URL 是否仍在该应用自己的 origin 内([`AppOrigin::allows_navigation`],导航策略的唯一真相)、
//!   池里的 webview id 段、每应用存储标识;
//! - 状态:每个应用面板"当前要加载的地址"([`AppViews`],A4b2 的状态机往里写);
//! - 期望清单项的构造([`app_webview_spec`])。
//!
//! 真正创建 webview(**不装** `dozer://` 协议、IPC 只留白名单)在 `runtime.rs::build_app_webview`。

use std::collections::HashMap;

use crate::app::{APP_CONTENT_ID_OFFSET, AppSlot};
use crate::preview::WebviewSpec;

/// 池里这个 id 是不是应用 webview(`APP_CONTENT_ID_OFFSET + 槽号`,段内最多 65536 个)。
pub(crate) fn is_app_webview_id(id: usize) -> bool {
    (APP_CONTENT_ID_OFFSET..APP_CONTENT_ID_OFFSET + 65_536).contains(&id)
}

pub(crate) fn webview_id(slot: AppSlot) -> usize {
    APP_CONTENT_ID_OFFSET + slot.index()
}

/// `webview_id` 的反函数;不在应用段内返回 `None`。
pub(crate) fn slot_for_webview_id(id: usize) -> Option<AppSlot> {
    if !is_app_webview_id(id) {
        return None;
    }
    AppSlot::from_index(id - APP_CONTENT_ID_OFFSET)
}

/// 一个应用的 origin(`http://<host>:<port>`)。从应用的**站点地址**解析(启动地址里带的令牌查询串不影响)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AppOrigin {
    host: String,
    port: u16,
}

impl AppOrigin {
    /// 只接受 `http://<id>.localhost:<端口>/…`:主机必须是 `<合法 app id>.localhost`,端口必须显式给出,
    /// 不允许用户名/密码。其余(含 `https`、裸 `localhost`、IP)一律 `None`——应用只会由 gateway 以这个形状发布。
    pub(crate) fn from_url(url: &str) -> Option<Self> {
        let parsed = url::Url::parse(url).ok()?;
        if parsed.scheme() != "http" || !parsed.username().is_empty() || parsed.password().is_some()
        {
            return None;
        }
        let host = parsed.host_str()?.to_ascii_lowercase();
        let id = host.strip_suffix(".localhost")?;
        if !crate::app::valid_app_id(id) {
            return None;
        }
        Some(Self {
            host,
            port: parsed.port()?,
        })
    }

    /// 应用 id(主机名 `<id>.localhost` 去掉后缀)。
    #[allow(dead_code)] // A4b2 的面板状态机与手工冒烟调用
    pub(crate) fn app_id(&self) -> &str {
        self.host.strip_suffix(".localhost").unwrap_or(&self.host)
    }

    /// 导航策略:**只放行本 origin**。wry 的回调拿不到"主框架还是子框架",对每一次导航都问,所以子框架同样适用:
    /// 本 origin 的 `http` 地址、`about:blank`/`about:srcdoc`(应用常用的空 iframe)、
    /// 以及本 origin 创建的 `blob:`。其余一律拒绝(别的站点、`file:`、`dozer:`、`data:`、`javascript:` ……)。
    pub(crate) fn allows_navigation(&self, url: &str) -> bool {
        if url == "about:blank" || url == "about:srcdoc" {
            return true;
        }
        let inner = url.strip_prefix("blob:").unwrap_or(url);
        let Ok(parsed) = url::Url::parse(inner) else {
            return false;
        };
        parsed.scheme() == "http"
            && parsed.username().is_empty()
            && parsed.password().is_none()
            && parsed
                .host_str()
                .is_some_and(|h| h.eq_ignore_ascii_case(&self.host))
            && parsed.port() == Some(self.port)
    }
}

/// 每应用的 WKWebView 数据存储标识(macOS 14+ 的 `WKWebsiteDataStore(forIdentifier:)`):由应用 id 确定性
/// 派生,**不能改算法**——改了所有应用的本地数据都会"消失"(测试钉死了一个向量)。FNV-1a 128 位,无新依赖。
pub(crate) fn data_store_identifier(app_id: &str) -> [u8; 16] {
    const OFFSET: u128 = 0x6c62272e07bb014262b821756295c58d;
    const PRIME: u128 = 0x0000000001000000000000000000013b;
    let mut h = OFFSET;
    for b in b"bytehost-app:".iter().chain(app_id.as_bytes()) {
        h ^= u128::from(*b);
        h = h.wrapping_mul(PRIME);
    }
    h.to_be_bytes()
}

/// 每个应用面板"当前要加载的地址"。地址带一次性令牌(**秘密**):只存在内存里,不写日志、不落盘;
/// 清掉它(应用停了/卸载)就会让池里对应的 webview 被回收。A4b1 里没有写入者,A4b2 的面板状态机写。
#[derive(Debug, Default)]
pub(crate) struct AppViews {
    urls: HashMap<AppSlot, String>,
}

impl AppViews {
    #[allow(dead_code)] // A4b2 的启动流程调用
    pub(crate) fn set_url(&mut self, slot: AppSlot, url: String) {
        self.urls.insert(slot, url);
    }

    #[allow(dead_code)] // A4b2
    pub(crate) fn clear(&mut self, slot: AppSlot) {
        self.urls.remove(&slot);
    }

    pub(crate) fn url(&self, slot: AppSlot) -> Option<&str> {
        self.urls.get(&slot).map(String::as_str)
    }

    /// 该应用面板的 webview 期望项;没有地址(未运行)返回 `None`。
    pub(crate) fn spec(&self, slot: AppSlot, visible: bool) -> Option<WebviewSpec> {
        self.url(slot)
            .map(|url| app_webview_spec(slot, url, visible))
    }
}

/// 应用 webview 允许发给宿主的 IPC 消息(白名单)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AppIpc {
    Focus,
    MouseUp,
    ZoomIn,
    ZoomOut,
    ZoomReset,
}

impl AppIpc {
    /// 消息体必须是 `<nonce>:<动词>`:nonce 是本 webview 创建时随机生成、只写进注入脚本闭包的,页面脚本拿不到,
    /// 所以页面**不能**自己 `window.ipc.postMessage(..)` 伪造焦点/缩放(抢终端键盘、狂刷全局缩放)。
    /// 其余一切(没有 nonce、nonce 不对、未知动词、多余字段)一律 `None`。
    pub(crate) fn parse(body: &str, nonce: &str) -> Option<Self> {
        let verb = body.strip_prefix(nonce)?.strip_prefix(':')?;
        Some(match verb {
            "focus" => Self::Focus,
            "mouseup" => Self::MouseUp,
            "zoom_in" => Self::ZoomIn,
            "zoom_out" => Self::ZoomOut,
            "zoom_reset" => Self::ZoomReset,
            _ => return None,
        })
    }
}

/// 每个应用 webview 一个新 nonce(122 位随机,32 个十六进制字符,可安全嵌进脚本字面量)。
pub(crate) fn new_ipc_nonce() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// 注入脚本(在页面脚本之前运行):先把 `postMessage` 绑定到局部变量——页面之后改写
/// `window.ipc.postMessage` 也截不到 nonce——再只转发 **`isTrusted` 的真实用户事件**
/// (页面脚本 `dispatchEvent` 合成的事件 `isTrusted` 为假)。转发的内容只有焦点/拖拽松开/缩放三类。
pub(crate) fn app_init_script(nonce: &str) -> String {
    format!(
        "(function(){{var post=window.ipc.postMessage.bind(window.ipc);var N='{nonce}';\
function send(v){{post(N+':'+v)}}\
document.addEventListener('mousedown',function(e){{if(e.isTrusted)send('focus')}},true);\
document.addEventListener('mouseup',function(e){{if(e.isTrusted)send('mouseup')}},true);\
document.addEventListener('keydown',function(e){{if(!e.isTrusted||!e.ctrlKey)return;var c=e.code,k=e.key;\
if(c==='Equal'||k==='+'||k==='='){{e.preventDefault();send('zoom_in')}}\
else if(c==='Minus'||k==='-'){{e.preventDefault();send('zoom_out')}}\
else if(c==='Digit1'||k==='1'){{e.preventDefault();send('zoom_reset')}}}},true)}})();"
    )
}

pub(crate) fn app_webview_spec(slot: AppSlot, url: &str, visible: bool) -> WebviewSpec {
    WebviewSpec {
        id: webview_id(slot),
        url: url.to_owned(),
        visible,
        editor_binding: None,
        reserve: None,
        loading_generation: None,
        park_offscreen: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn origin() -> AppOrigin {
        AppOrigin::from_url("http://excalidraw.localhost:20001/?bh_token=secret").unwrap()
    }

    #[test]
    fn origin_is_parsed_only_from_the_gateway_shaped_address() {
        assert_eq!(
            origin(),
            AppOrigin::from_url("http://EXCALIDRAW.localhost:20001/other").unwrap(),
            "大小写与路径/查询串不影响 origin"
        );
        for bad in [
            "https://excalidraw.localhost:20001/",
            "http://excalidraw.localhost/",
            "http://localhost:20001/",
            "http://127.0.0.1:20001/",
            "http://Bad_Id.localhost:20001/",
            "http://-a.localhost:20001/",
            "http://u:p@excalidraw.localhost:20001/",
            "dozer://flyfish/host.html",
            "not a url",
        ] {
            assert!(AppOrigin::from_url(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn the_app_id_is_the_host_without_the_localhost_suffix() {
        assert_eq!(origin().app_id(), "excalidraw");
    }

    #[test]
    fn navigation_stays_inside_the_apps_own_origin() {
        let o = origin();
        for ok in [
            "http://excalidraw.localhost:20001/",
            "http://excalidraw.localhost:20001/a/b?x=1#h",
            "HTTP://Excalidraw.Localhost:20001/up",
            "about:blank",
            "about:srcdoc",
            "blob:http://excalidraw.localhost:20001/6f1c",
        ] {
            assert!(o.allows_navigation(ok), "{ok}");
        }
        for bad in [
            "http://excalidraw.localhost:20002/",
            "http://other.localhost:20001/",
            "http://excalidraw.localhost.evil.com:20001/",
            "http://evil.com/",
            "https://excalidraw.localhost:20001/",
            "http://u:p@excalidraw.localhost:20001/",
            "http://excalidraw.localhost@evil.com:20001/",
            "file:///etc/passwd",
            "dozer://flyfish/host.html",
            "data:text/html,<script>1</script>",
            "javascript:alert(1)",
            "blob:http://evil.com/6f1c",
            "blob:https://excalidraw.localhost:20001/6f1c",
            "about:config",
            "",
        ] {
            assert!(!o.allows_navigation(bad), "{bad}");
        }
    }

    #[test]
    fn webview_ids_live_in_their_own_range_and_invert() {
        let slot = AppSlot::intern("wv-ids").unwrap();
        let id = webview_id(slot);
        assert!(is_app_webview_id(id));
        assert_eq!(slot_for_webview_id(id), Some(slot));
        assert!(!is_app_webview_id(crate::app::GROUP_CHAT_CONTENT_ID_OFFSET));
        assert!(!is_app_webview_id(APP_CONTENT_ID_OFFSET - 1));
        assert!(!is_app_webview_id(APP_CONTENT_ID_OFFSET + 65_536));
        assert_eq!(slot_for_webview_id(0), None);
    }

    /// 钉死:这个标识决定每个应用的持久存储,算法一变所有应用数据都"丢"。
    #[test]
    fn the_data_store_identifier_is_stable_and_per_app() {
        assert_eq!(
            data_store_identifier("excalidraw"),
            // 同一算法的独立实现(Python)算出的值:30cd3bc2160471a7ee03a52d8c2e59db
            [
                0x30, 0xcd, 0x3b, 0xc2, 0x16, 0x04, 0x71, 0xa7, 0xee, 0x03, 0xa5, 0x2d, 0x8c, 0x2e,
                0x59, 0xdb
            ]
        );
        assert_ne!(data_store_identifier("a"), data_store_identifier("b"));
    }

    const NONCE: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn ipc_messages_need_the_per_webview_nonce_and_a_known_verb() {
        for (verb, want) in [
            ("focus", AppIpc::Focus),
            ("mouseup", AppIpc::MouseUp),
            ("zoom_in", AppIpc::ZoomIn),
            ("zoom_out", AppIpc::ZoomOut),
            ("zoom_reset", AppIpc::ZoomReset),
        ] {
            assert_eq!(AppIpc::parse(&format!("{NONCE}:{verb}"), NONCE), Some(want));
        }
        for bad in [
            "focus",
            "mouseup",
            "title:1:x",
            "find_native",
            &format!("{NONCE}:"),
            &format!("{NONCE}:format_disk"),
            &format!("{NONCE}x:focus"),
            &format!("x{NONCE}:focus"),
            ":focus",
            &format!("{NONCE}:focus:extra"),
            "wrong-nonce:focus",
            "",
        ] {
            assert_eq!(AppIpc::parse(bad, NONCE), None, "{bad}");
        }
    }

    #[test]
    fn nonces_are_unguessable_and_safe_to_embed_in_a_script() {
        let a = new_ipc_nonce();
        let b = new_ipc_nonce();
        assert_ne!(a, b);
        assert!(
            a.len() >= 32 && a.bytes().all(|c| c.is_ascii_hexdigit()),
            "{a}"
        );
    }

    /// 页面脚本不能伪造 IPC:消息带只有注入脚本闭包知道的 nonce,注入脚本在页面脚本之前
    /// 绑定好 `postMessage`(页面之后改写 `window.ipc.postMessage` 也截不到),且只转发
    /// `isTrusted` 的真实用户事件。
    #[test]
    fn the_init_script_binds_post_message_first_and_forwards_only_trusted_events() {
        let script = app_init_script(NONCE);
        assert!(script.contains(NONCE));
        assert!(script.contains("window.ipc.postMessage.bind(window.ipc)"));
        assert!(
            !script.contains("window.ipc.postMessage('"),
            "不得再直接调用页面可改写的 postMessage"
        );
        for listener in ["mousedown", "mouseup", "keydown"] {
            let at = script
                .find(&format!("addEventListener('{listener}'"))
                .unwrap();
            assert!(
                script[at..].contains("isTrusted"),
                "{listener} 监听必须校验 isTrusted"
            );
        }
        assert!(script.matches("isTrusted").count() >= 3);
    }

    /// `build_app_webview` 是创建 wry webview 的唯一应用路径,没有廉价夹具能真的建一个;
    /// 这些是它**必须**写出的、且 wry 默认值会反过来的配置,用源码扫描钉住
    /// (wry 0.55 默认:下载放行、debug 下开发者工具开、媒体权限自动批准)。
    #[test]
    fn build_app_webview_pins_the_restrictive_settings() {
        let src = include_str!("runtime.rs");
        let start = src.find("fn build_app_webview(").unwrap();
        let end = start + src[start..].find("\n}\n").unwrap();
        let body = &src[start..end];
        for required in [
            ".with_download_started_handler(|_, _| false)",
            ".with_devtools(false)",
            ".with_new_window_req_handler(",
            ".with_navigation_handler(",
            "AppIpc::parse(",
        ] {
            assert!(body.contains(required), "缺少 {required}");
        }
        for forbidden in [
            "with_custom_protocol",
            "with_download_completed_handler",
            "APP_INIT_SCRIPT",
        ] {
            assert!(!body.contains(forbidden), "不得出现 {forbidden}");
        }
    }

    #[test]
    fn views_produce_a_spec_only_while_an_address_is_set() {
        let slot = AppSlot::intern("wv-views").unwrap();
        let mut views = AppViews::default();
        assert!(views.spec(slot, true).is_none());
        views.set_url(slot, "http://wv-views.localhost:20001/?bh_token=t".into());
        let spec = views.spec(slot, false).unwrap();
        assert_eq!(spec.id, webview_id(slot));
        assert!(!spec.visible);
        assert!(spec.editor_binding.is_none() && spec.reserve.is_none());
        views.clear(slot);
        assert!(views.spec(slot, true).is_none());
    }
}
