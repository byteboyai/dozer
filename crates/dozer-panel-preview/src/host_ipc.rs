//! 预览 webview 的 IPC 分类(S5b-2 P3)。
//!
//! 从 `dozer-app::runtime` 的 `with_ipc_handler` 闭包搬来的「解析 + 校验」那一半。
//! 产品仍持有 wry 句柄、自己决定往事件环发哪条消息;这里只回答两个纯问题:
//!
//! 1. 一段原始 IPC 文本属于哪一类([`classify_ipc`]):焦点/拖拽/缩放/粘贴请求
//!    等免解析的页内命令,还是某个 host 的已校验事件([`HostEvent`]);
//! 2. 一段 `dozer://` 协议 URL 里带着哪种 host 身份([`WebviewKind::of_url`]),
//!    用来决定装哪套注入脚本、用哪个解析器、发哪条产品消息。
//!
//! 归属校验的规则与搬迁前逐字一致:信封自报的 project/panel/tab/document 必须
//! 与 Rust 创建 webview 时捕获的绑定相同,否则整条事件丢弃并返回原因(产品照旧
//! 记 warn)。本模块不含 wry/winit,Dozer 的 wry 池与 Digger 的 wry 宿主共用。

use crate::{
    EditorEvent, EditorHostBinding, FlyfishEvent, HostBinding, ImageAnnotateEvent, JsonEvent,
    PlantUmlEvent, ReviewTraceEvent, TabularEvent, ViewerPanel, WebviewEnvelope,
};

/// 被丢弃的 IPC 及其原因(产品照旧 `log_warn`,但不影响控制流)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IpcRejection {
    /// 不是 JSON 信封/超出长度上限。
    Parse(String),
    /// 信封自报归属与 Rust 侧绑定不符(或被判为不安全),整条丢弃。
    Foreign(String),
}

impl std::fmt::Display for IpcRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(e) => write!(f, "{e}"),
            Self::Foreign(e) => write!(f, "拒绝无效 IPC: {e}"),
        }
    }
}

/// 一次 `dozer://` 页面属于哪种宿主。决定注入脚本段落、IPC 解析器与产品消息。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebviewKind {
    /// CodeMirror 编辑器 host(`dozer://editor/`):自带编辑行为,不装文本编辑段落。
    Editor,
    /// JSON host(`dozer://json-editor/`)。
    Json,
    /// Tabular ag-grid host(`dozer://tabular/`)。
    Tabular,
    /// Flyfish 渲染 host(`dozer://flyfish/`)。
    Flyfish,
    /// 隔离 HTML 预览 host(`dozer://html/`)。
    Html,
    /// 图片查看/标注 host(`dozer://image-annotate/`)。
    ImageAnnotate,
    /// PlantUML viewer host(`dozer://plantuml-viewer/`)。
    PlantUml,
    /// 会话审阅 trace host(`dozer://review-trace/`)。
    ReviewTrace,
    /// 不在预览面板职责内的页面(产品外壳/浏览器/其它 scheme)。
    Other,
}

impl WebviewKind {
    pub fn of_url(url: &str) -> Self {
        // 与搬迁前的判定顺序一致:各 host 的 envelope 载荷不同,解析器不能混用。
        if url.starts_with("dozer://flyfish/") {
            Self::Flyfish
        } else if url.starts_with("dozer://plantuml-viewer/") {
            Self::PlantUml
        } else if url.starts_with("dozer://image-annotate/") {
            Self::ImageAnnotate
        } else if url.starts_with("dozer://review-trace/") {
            Self::ReviewTrace
        } else if crate::is_editor_url(url) {
            Self::Editor
        } else if crate::is_json_editor_url(url) {
            Self::Json
        } else if crate::is_tabular_url(url) {
            Self::Tabular
        } else if url.starts_with("dozer://html/") {
            Self::Html
        } else {
            Self::Other
        }
    }

    /// 该 host 的常驻脚本要不要装文本光标/复制/剪切粘贴段落。
    ///
    /// 与搬迁前 `editor_binding.is_none()` 的判定等价:有 editor 绑定就一定是
    /// editor/JSON/tabular host(自带编辑行为),其余全部装。
    pub fn wants_text_editing(self) -> bool {
        !matches!(self, Self::Editor)
    }

    /// 从 URL 解析 host 归属绑定(Flyfish / 隔离 HTML / image-annotate /
    /// PlantUML 共用同一查询串格式)。editor/JSON/tabular 有 `EditorHostBinding`。
    pub fn host_binding(url: &str) -> Option<HostBinding> {
        crate::flyfish_binding_from_url(url)
    }
}

/// 从 Rust 侧捕获的 editor 绑定派生校验基准。
pub fn expected_binding(binding: &EditorHostBinding) -> HostBinding {
    HostBinding::new(
        binding.project_id,
        binding.panel,
        binding.tab_id,
        binding.document_id(),
    )
}

/// 创建 webview 时由 Rust 捕获的归属,IPC 回传据此校验;不采信 JS 自报路径。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IpcContext {
    pub kind: WebviewKind,
    /// webview 池 id(产品用来找回 wry 句柄 / 识别固定单槽面板)。
    pub webview_id: usize,
    /// editor/JSON/tabular host 的绑定;其它 host 为 `None`。
    pub editor_binding: Option<EditorHostBinding>,
    /// Flyfish / 隔离 HTML / image-annotate 的绑定(从 URL 解析)。
    pub host_binding: Option<HostBinding>,
}

impl IpcContext {
    /// 从期望清单里的 URL 与绑定构造。
    pub fn new(webview_id: usize, url: &str, editor_binding: Option<EditorHostBinding>) -> Self {
        let kind = WebviewKind::of_url(url);
        let host_binding = match kind {
            WebviewKind::Flyfish
            | WebviewKind::Html
            | WebviewKind::ImageAnnotate
            | WebviewKind::PlantUml => WebviewKind::host_binding(url),
            _ => None,
        };
        Self {
            kind,
            webview_id,
            editor_binding,
            host_binding,
        }
    }

    /// 该 webview 是否用某个宿主专用解析器/校验基准;不是则 IPC 只当"聚焦"处理。
    pub fn routes_events(&self) -> bool {
        matches!(
            self.kind,
            WebviewKind::Editor
                | WebviewKind::Json
                | WebviewKind::Tabular
                | WebviewKind::Flyfish
                | WebviewKind::Html
                | WebviewKind::ImageAnnotate
                | WebviewKind::PlantUml
                | WebviewKind::ReviewTrace
        )
    }
}

/// 一次已解析、已校验的预览 host 事件。
#[derive(Debug, Clone, PartialEq)]
pub enum HostEvent {
    Editor {
        binding: EditorHostBinding,
        envelope: WebviewEnvelope<EditorEvent>,
    },
    /// git log 面板的 diff host(无 tab,轻量绑定)。产品据此走独立分支。
    GitLogDiff {
        binding: HostBinding,
        envelope: WebviewEnvelope<EditorEvent>,
    },
    Json {
        binding: EditorHostBinding,
        envelope: WebviewEnvelope<JsonEvent>,
    },
    Tabular {
        binding: EditorHostBinding,
        envelope: WebviewEnvelope<TabularEvent>,
    },
    Flyfish {
        binding: HostBinding,
        envelope: WebviewEnvelope<FlyfishEvent>,
    },
    ImageAnnotate {
        binding: HostBinding,
        envelope: WebviewEnvelope<ImageAnnotateEvent>,
    },
    PlantUml {
        binding: HostBinding,
        envelope: WebviewEnvelope<PlantUmlEvent>,
    },
    /// 会话审阅 trace host(目前只有 `document_loaded`),固定单槽、不带绑定。
    ReviewTrace(ReviewTraceEvent),
}

impl HostEvent {
    /// 事件归属的项目。审阅 trace 是"当前聚焦项目"的直接字段,没有项目身份。
    pub fn project_id(&self) -> Option<i64> {
        match self {
            Self::Editor { binding, .. }
            | Self::Json { binding, .. }
            | Self::Tabular { binding, .. } => Some(binding.project_id),
            Self::GitLogDiff { binding, .. }
            | Self::Flyfish { binding, .. }
            | Self::ImageAnnotate { binding, .. }
            | Self::PlantUml { binding, .. } => Some(binding.project_id),
            Self::ReviewTrace(_) => None,
        }
    }

    pub fn panel(&self) -> Option<ViewerPanel> {
        match self {
            Self::Editor { binding, .. }
            | Self::Json { binding, .. }
            | Self::Tabular { binding, .. } => Some(binding.panel),
            Self::GitLogDiff { binding, .. }
            | Self::Flyfish { binding, .. }
            | Self::ImageAnnotate { binding, .. }
            | Self::PlantUml { binding, .. } => Some(binding.panel),
            Self::ReviewTrace(_) => None,
        }
    }

    pub fn tab_id(&self) -> Option<usize> {
        match self {
            Self::Editor { binding, .. }
            | Self::Json { binding, .. }
            | Self::Tabular { binding, .. } => Some(binding.tab_id),
            Self::GitLogDiff { binding, .. }
            | Self::Flyfish { binding, .. }
            | Self::ImageAnnotate { binding, .. }
            | Self::PlantUml { binding, .. } => Some(binding.tab_id),
            Self::ReviewTrace(_) => None,
        }
    }
}

/// 一条原始 IPC 文本的归类。免解析的页内命令直接给出,其余由产品翻译成事件。
#[derive(Debug, Clone, PartialEq)]
pub enum IpcMessage {
    /// 子 webview 拿到键盘焦点(首次点击后 WKWebView 成为 first responder)。
    Focus,
    /// 页内松开鼠标:页签拖拽拖进 webview 后在这里松开,winit 收不到 `Released`。
    MouseUp,
    /// 子 webview 抢到键盘焦点时的全局缩放快捷键。
    ZoomIn,
    ZoomOut,
    ZoomReset,
    /// Flyfish host 把 ⌘F 交还给产品 Find 状态机。
    FindPreview,
    /// ⌘F:JS 已找到并聚焦页面自带搜索框。
    FindPage,
    /// ⌘F:页面没有自带搜索框,退回平台原生查找条。
    FindNative,
    /// ⌘V:页面输入框里粘贴(WKWebView 没有宿主 Edit 菜单,读剪贴板必须宿主代劳)。
    PasteRequest,
    /// 浏览器面板的页面标题回报 `title:<webview_id>:<title>`。不属于预览面板,
    /// 由外壳消费(这里只做解析,不改变行为)。
    BrowserTitle {
        webview_id: usize,
        title: String,
    },
    /// 已解析、已校验的 host 事件。
    Host(Box<HostEvent>),
    /// JSON 载荷但当前 webview 不是可路由的 host(搬迁前:
    /// `Message::WebViewFocused`,把焦点还给产品)。
    FocusedEnvelope,
    /// 无法识别/被拒:整条丢弃,`reason` 供产品记日志。
    Rejected(IpcRejection),
}

fn looks_like_envelope(body: &str) -> bool {
    body.starts_with('{')
}

fn parse_and_validate<T: serde::de::DeserializeOwned>(
    body: &str,
    parse: impl Fn(&str) -> Result<WebviewEnvelope<T>, crate::ProtocolError>,
    expected: Option<&HostBinding>,
) -> Result<WebviewEnvelope<T>, IpcRejection> {
    let envelope = parse(body).map_err(|e| IpcRejection::Parse(e.to_string()))?;
    if let Some(expected) = expected
        && let Err(e) = envelope.validate(expected)
    {
        return Err(IpcRejection::Foreign(e.to_string()));
    }
    Ok(envelope)
}

/// 分类一条入站 IPC 文本(经 `with_ipc_handler` 的 `body`)。
///
/// 判定顺序与搬迁前的闭包逐分支一致:先认免解析的页内命令,再认浏览器标题回报,
/// 最后按 host 身份解析并校验事件。`context` 描述创建该 webview 时捕获的归属。
pub fn classify_ipc(context: &IpcContext, body: &str) -> IpcMessage {
    match body {
        "mouseup" => return IpcMessage::MouseUp,
        "zoom_in" => return IpcMessage::ZoomIn,
        "zoom_out" => return IpcMessage::ZoomOut,
        "zoom_reset" => return IpcMessage::ZoomReset,
        "focus" => return IpcMessage::Focus,
        "find_preview" if context.kind == WebviewKind::Flyfish => {
            return IpcMessage::FindPreview;
        }
        "find_page" => return IpcMessage::FindPage,
        "find_native" => return IpcMessage::FindNative,
        "paste_request" => return IpcMessage::PasteRequest,
        title_msg if title_msg.starts_with("title:") => {
            let rest = &title_msg["title:".len()..];
            let mut it = rest.splitn(2, ':');
            if let (Some(id_s), Some(title)) = (it.next(), it.next())
                && let Ok(id) = id_s.parse::<usize>()
            {
                return IpcMessage::BrowserTitle {
                    webview_id: id,
                    title: title.to_string(),
                };
            }
            return IpcMessage::Rejected(IpcRejection::Parse("标题回报格式错误".into()));
        }
        _ => {}
    }

    if context.kind == WebviewKind::ReviewTrace && looks_like_envelope(body) {
        return match crate::parse_review_trace_event(body) {
            Ok(env) => IpcMessage::Host(Box::new(HostEvent::ReviewTrace(env.payload))),
            Err(e) => IpcMessage::Rejected(IpcRejection::Parse(e.to_string())),
        };
    }

    if context.kind == WebviewKind::PlantUml {
        let Some(binding) = context.host_binding.clone() else {
            return IpcMessage::Rejected(IpcRejection::Foreign("host URL 缺少归属查询串".into()));
        };
        return match parse_and_validate(body, crate::parse_plantuml_event, Some(&binding)) {
            Ok(envelope) => IpcMessage::Host(Box::new(HostEvent::PlantUml { binding, envelope })),
            Err(rejection) => IpcMessage::Rejected(rejection),
        };
    }

    if matches!(
        context.kind,
        WebviewKind::Flyfish | WebviewKind::Html | WebviewKind::ImageAnnotate
    ) && looks_like_envelope(body)
    {
        let Some(expected) = context.host_binding.clone() else {
            return IpcMessage::Rejected(IpcRejection::Foreign("host URL 缺少归属查询串".into()));
        };
        return match context.kind {
            WebviewKind::ImageAnnotate => {
                match parse_and_validate(body, crate::parse_image_annotate_event, Some(&expected)) {
                    Ok(envelope) => IpcMessage::Host(Box::new(HostEvent::ImageAnnotate {
                        binding: expected,
                        envelope,
                    })),
                    Err(rejection) => IpcMessage::Rejected(rejection),
                }
            }
            _ => match parse_and_validate(body, crate::parse_flyfish_event, Some(&expected)) {
                Ok(envelope) => IpcMessage::Host(Box::new(HostEvent::Flyfish {
                    binding: expected,
                    envelope,
                })),
                Err(rejection) => IpcMessage::Rejected(rejection),
            },
        };
    }

    if let Some(binding) = context.editor_binding.as_ref() {
        let expected = expected_binding(binding);
        return match context.kind {
            WebviewKind::Tabular => {
                match parse_and_validate(body, crate::parse_tabular_event, Some(&expected)) {
                    Ok(envelope) => IpcMessage::Host(Box::new(HostEvent::Tabular {
                        binding: binding.clone(),
                        envelope,
                    })),
                    Err(rejection) => IpcMessage::Rejected(rejection),
                }
            }
            WebviewKind::Json => {
                match parse_and_validate(body, crate::parse_json_event, Some(&expected)) {
                    Ok(envelope) => IpcMessage::Host(Box::new(HostEvent::Json {
                        binding: binding.clone(),
                        envelope,
                    })),
                    Err(rejection) => IpcMessage::Rejected(rejection),
                }
            }
            _ => match parse_and_validate(body, crate::parse_event, Some(&expected)) {
                Ok(envelope) => {
                    // git log 的 diff host 共用 editor 解析器,但没有 tab 模型。
                    if binding.panel == ViewerPanel::GitLog {
                        IpcMessage::Host(Box::new(HostEvent::GitLogDiff {
                            binding: expected,
                            envelope,
                        }))
                    } else {
                        IpcMessage::Host(Box::new(HostEvent::Editor {
                            binding: binding.clone(),
                            envelope,
                        }))
                    }
                }
                Err(rejection) => IpcMessage::Rejected(rejection),
            },
        };
    }

    if looks_like_envelope(body) {
        IpcMessage::FocusedEnvelope
    } else {
        IpcMessage::Rejected(IpcRejection::Parse(format!(
            "无法识别的 IPC: {}",
            body.chars().take(64).collect::<String>()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn editor_binding(panel: ViewerPanel) -> EditorHostBinding {
        EditorHostBinding::new(7, panel, 3, PathBuf::from("/tmp/nope.rs"))
    }

    fn editor_ctx(panel: ViewerPanel) -> IpcContext {
        IpcContext::new(
            1,
            "dozer://editor/index.html?p=/tmp/nope.rs&proj=7&panel=files&tab=3&doc=p7-t3",
            Some(editor_binding(panel)),
        )
    }

    fn flyfish_ctx() -> IpcContext {
        IpcContext::new(
            2,
            "dozer://flyfish/host.html?p=/tmp/a.md&proj=7&panel=files&tab=4&doc=p7-t4",
            None,
        )
    }

    /// 一次 `ready` 信封:editor / json / tabular 的 `Ready` 都带必填字段,
    /// 只有 Flyfish 的 `Ready` 是单元变体。
    fn envelope(panel: &str, tab: usize) -> String {
        format!(
            r#"{{"protocol_version":1,"project_id":7,"panel":"{panel}","tab_id":{tab},"document_id":"p7-t{tab}","revision":1,"payload":{{"kind":"ready","read_only":false,"language":"rust"}}}}"#
        )
    }

    #[test]
    fn page_commands_are_classified_without_parsing() {
        let ctx = editor_ctx(ViewerPanel::Files);
        assert_eq!(classify_ipc(&ctx, "focus"), IpcMessage::Focus);
        assert_eq!(classify_ipc(&ctx, "mouseup"), IpcMessage::MouseUp);
        assert_eq!(classify_ipc(&ctx, "zoom_in"), IpcMessage::ZoomIn);
        assert_eq!(classify_ipc(&ctx, "zoom_out"), IpcMessage::ZoomOut);
        assert_eq!(classify_ipc(&ctx, "zoom_reset"), IpcMessage::ZoomReset);
        assert_eq!(
            classify_ipc(&ctx, "paste_request"),
            IpcMessage::PasteRequest
        );
        assert_eq!(classify_ipc(&ctx, "find_page"), IpcMessage::FindPage);
        assert_eq!(classify_ipc(&ctx, "find_native"), IpcMessage::FindNative);
    }

    #[test]
    fn find_preview_only_belongs_to_the_flyfish_host() {
        assert_eq!(
            classify_ipc(&flyfish_ctx(), "find_preview"),
            IpcMessage::FindPreview
        );
        // 其它宿主收到同一条文本时不再命中该分支,落到"无法识别"。
        assert!(matches!(
            classify_ipc(&editor_ctx(ViewerPanel::Files), "find_preview"),
            IpcMessage::Rejected(_)
        ));
    }

    #[test]
    fn browser_title_carries_the_webview_id() {
        assert_eq!(
            classify_ipc(&editor_ctx(ViewerPanel::Files), "title:9:Hello: world"),
            IpcMessage::BrowserTitle {
                webview_id: 9,
                title: "Hello: world".to_string()
            }
        );
        assert!(matches!(
            classify_ipc(&editor_ctx(ViewerPanel::Files), "title:notanumber:x"),
            IpcMessage::Rejected(IpcRejection::Parse(_))
        ));
    }

    #[test]
    fn editor_events_validate_against_the_rust_binding() {
        let ctx = editor_ctx(ViewerPanel::Files);
        let good = envelope("files", 3);
        match classify_ipc(&ctx, &good) {
            IpcMessage::Host(event) => match *event {
                HostEvent::Editor { binding, .. } => {
                    assert_eq!(binding.path, PathBuf::from("/tmp/nope.rs"));
                    assert_eq!(binding.tab_id, 3);
                }
                other => panic!("期望 Editor 事件,得到 {other:?}"),
            },
            other => panic!("期望 Host 事件,得到 {other:?}"),
        }
        // 自报别的 tab:整条丢弃。
        let foreign = envelope("files", 99);
        assert!(matches!(
            classify_ipc(&ctx, &foreign),
            IpcMessage::Rejected(IpcRejection::Foreign(_))
        ));
    }

    #[test]
    fn git_log_diff_events_use_the_lightweight_binding() {
        let ctx = editor_ctx(ViewerPanel::GitLog);
        let body = envelope("gitlog", 3);
        match classify_ipc(&ctx, &body) {
            IpcMessage::Host(event) => {
                assert!(matches!(*event, HostEvent::GitLogDiff { .. }));
                assert_eq!(event.panel(), Some(ViewerPanel::GitLog));
            }
            other => panic!("期望 GitLogDiff,得到 {other:?}"),
        }
    }

    #[test]
    fn flyfish_and_html_events_parse_with_their_binding() {
        let ctx = flyfish_ctx();
        let body = envelope("files", 4);
        match classify_ipc(&ctx, &body) {
            IpcMessage::Host(event) => match *event {
                HostEvent::Flyfish { binding, envelope } => {
                    assert_eq!(binding.tab_id, 4);
                    assert!(matches!(envelope.payload, FlyfishEvent::Ready));
                }
                other => panic!("期望 Flyfish,得到 {other:?}"),
            },
            other => panic!("期望 Host,得到 {other:?}"),
        }
        let foreign = envelope("files", 5);
        assert!(matches!(
            classify_ipc(&ctx, &foreign),
            IpcMessage::Rejected(IpcRejection::Foreign(_))
        ));
    }

    #[test]
    fn image_annotate_uses_its_own_payload_parser() {
        let ctx = IpcContext::new(
            3,
            "dozer://image-annotate/host.html?p=/tmp/p.png&proj=7&panel=files&tab=5&doc=p7-t5",
            None,
        );
        let body = envelope("files", 5);
        assert!(matches!(
            classify_ipc(&ctx, &body),
            IpcMessage::Host(event) if matches!(*event, HostEvent::ImageAnnotate { .. })
        ));
    }

    #[test]
    fn plantuml_and_tabular_route_to_their_parsers() {
        let plantuml = IpcContext::new(
            4,
            "dozer://plantuml-viewer/index.html?theme=dark&locale=en&proj=7&panel=project&tab=6&doc=p7-t6",
            None,
        );
        let body = r#"{"protocol_version":1,"project_id":7,"panel":"project","tab_id":6,"document_id":"p7-t6","revision":1,"payload":{"kind":"ready"}}"#.to_string();
        assert!(matches!(
            classify_ipc(&plantuml, &body),
            IpcMessage::Host(event) if matches!(*event, HostEvent::PlantUml { .. })
        ));

        let tabular = IpcContext::new(
            5,
            "dozer://tabular/index.html?proj=7&panel=files&tab=8&doc=p7-t8",
            Some(EditorHostBinding::new(
                7,
                ViewerPanel::Files,
                8,
                PathBuf::from("/tmp/s.csv"),
            )),
        );
        let body = envelope("files", 8);
        assert!(matches!(
            classify_ipc(&tabular, &body),
            IpcMessage::Host(event) if matches!(*event, HostEvent::Tabular { .. })
        ));
    }

    #[test]
    fn json_payload_without_a_host_binding_is_handed_back_as_focus() {
        // 搬迁前:没有 editor 绑定、又不是可解析 host 的 JSON 载荷 → `WebViewFocused`。
        let ctx = IpcContext::new(6, "dozer://review-trace/data.json", None);
        let ctx = IpcContext {
            kind: WebviewKind::Other,
            ..ctx
        };
        assert_eq!(
            classify_ipc(&ctx, &envelope("files", 1)),
            IpcMessage::FocusedEnvelope
        );
        assert!(matches!(
            classify_ipc(&ctx, "garbage"),
            IpcMessage::Rejected(IpcRejection::Parse(_))
        ));
    }

    #[test]
    fn review_trace_parses_without_a_binding() {
        let ctx = IpcContext::new(7, "dozer://review-trace/host.html", None);
        let body = r#"{"protocol_version":1,"payload":{"kind":"document_loaded"}}"#;
        assert!(matches!(
            classify_ipc(&ctx, body),
            IpcMessage::Host(event)
                if matches!(*event, HostEvent::ReviewTrace(ReviewTraceEvent::DocumentLoaded))
        ));
    }

    #[test]
    fn urls_map_to_their_host_kind() {
        for (url, want) in [
            ("dozer://flyfish/host.html?x", WebviewKind::Flyfish),
            ("dozer://plantuml-viewer/index.html", WebviewKind::PlantUml),
            (
                "dozer://image-annotate/host.html",
                WebviewKind::ImageAnnotate,
            ),
            ("dozer://review-trace/data.json", WebviewKind::ReviewTrace),
            ("dozer://editor/index.html", WebviewKind::Editor),
            ("dozer://json-editor/index.html", WebviewKind::Json),
            ("dozer://tabular/index.html", WebviewKind::Tabular),
            ("dozer://html/host.html", WebviewKind::Html),
            ("https://example.com", WebviewKind::Other),
        ] {
            assert_eq!(WebviewKind::of_url(url), want, "{url}");
        }
        assert!(!WebviewKind::Editor.wants_text_editing());
        for kind in [
            WebviewKind::Json,
            WebviewKind::Tabular,
            WebviewKind::Flyfish,
            WebviewKind::Html,
            WebviewKind::ImageAnnotate,
            WebviewKind::PlantUml,
            WebviewKind::ReviewTrace,
            WebviewKind::Other,
        ] {
            assert!(kind.wants_text_editing(), "{kind:?}");
        }
    }

    #[test]
    fn host_event_carries_routing_identity() {
        let ctx = IpcContext::new(
            8,
            "dozer://html/host.html?p=/tmp/page.html&proj=11&panel=project&tab=2&doc=p11-t2",
            None,
        );
        let body = envelope("project", 2)
            .replace("project_id\":7", "project_id\":11")
            .replace("p7-t2", "p11-t2");
        match classify_ipc(&ctx, &body) {
            IpcMessage::Host(event) => {
                assert_eq!(event.project_id(), Some(11));
                assert_eq!(event.panel(), Some(ViewerPanel::Project));
                assert_eq!(event.tab_id(), Some(2));
            }
            other => panic!("期望 Host,得到 {other:?}"),
        }
    }
}
