//! Dozer 的 webview 期望清单项。预览相关的 URL 构造与 `ReserveHint` 已随同源预览面板库搬走
//! (`dozer-panel-preview::webview`);这里只留 Dozer 自己多出来的宿主字段与转换。

use super::*;

/// main.rs 同步 webview 的期望清单项。
#[derive(Debug, Clone, PartialEq)]
pub struct WebviewSpec {
    pub id: usize,
    pub url: String,
    pub visible: bool,
    pub editor_binding: Option<EditorHostBinding>,
    /// 无 `editor_binding` 但需计入资源预算的 viewer 的 reserve 线索
    /// (目前仅 PlantUML Rendered host)。`None` = 不走 reserve(与
    /// Flyfish/HTML 等 Rendered host 的既有行为一致)。
    pub reserve: Option<ReserveHint>,
    /// T10:该 host 若在 `Reserving` 阶段等待预算,携带其 loading `generation`。
    /// `sync_webview_pool` 在 reserve 被批准时把 `(key, generation)` 回灌给
    /// 调用方,由 `apply_preview_pool_outcome` 世代校验后推进到 `CreatingHost`;
    /// 仅 host 类(editor/JSON)且在途加载时为 `Some`。
    pub loading_generation: Option<u64>,
    /// 该 webview 归属的共享宿主 Surface 身份(instance/slot/generation)。
    ///
    /// 走 `bytehost-desktop` Surface 管理的固定单槽面板(目前仅 Usage)在这里
    /// 带上身份,IPC 回传时据此校验归属;旧代数的在途事件被路由层拒绝。
    /// 其余 webview 为 `None`(沿用各自的 binding/URL 归属机制)。
    pub surface: Option<bytehost_core::SurfaceId>,
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
    /// T6:应用外壳标题 / WebView 语言桥的 i18n 投影(`AppSummary.display_i18n`)。
    /// 仅应用 webview 由 `AppViews` 填充;其余(预览/浏览器/共享 Surface)一律 `None`,
    /// 不装桥、标题回落 `app.name`。
    pub display_i18n: Option<bytehost_apps::proto::AppDisplayI18n>,
}

impl From<PreviewWebviewSpec> for WebviewSpec {
    /// 预览面板库产出的清单项补上 Dozer 独有的宿主字段(共享 Surface 身份 / 应用外壳 i18n)。
    fn from(spec: PreviewWebviewSpec) -> Self {
        Self {
            id: spec.id,
            url: spec.url,
            visible: spec.visible,
            editor_binding: spec.editor_binding,
            reserve: spec.reserve,
            loading_generation: spec.loading_generation,
            surface: None,
            park_offscreen: spec.park_offscreen,
            display_i18n: None,
        }
    }
}
