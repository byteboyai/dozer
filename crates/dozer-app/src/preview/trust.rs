//! webview 信任类别(规格 §5.4;Task 09 / api-decisions §9)。
//!
//! Dozer 有**六条** wry 构造路径(`runtime.rs` 内联 builder + `build_app_webview`),
//! 但真正的信任分界只有两类:**应用(不可信)** 与 **宿主/预览(可信)**。内容协议、
//! IPC 白名单、CSP、nonce、data store 全是 Dozer 的产品语义,不是 bytehost 的机制,
//! 因此信任类别**建模在本层**(Dozer 的 builder 分派),`bytehost-desktop` 只认
//! core 的 `SurfaceKind{Builtin, Application}`,不认识 editor/preview。
//!
//! [`WebviewTrust`] 由 [`WebviewSpec`] 的既有字段派生(唯一入口 [`WebviewTrust::of`]),
//! 避免新增易漏填的字段。它把"应用 builder 不带 `dozer://`、带 nonce-IPC"变成
//! 类型/构造层的保证,而非散落的字符串判断。

use crate::app_webview::is_app_webview_id;
use crate::preview::WebviewSpec;

/// 一个 webview 的信任类别。产品枚举,非 bytehost 类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WebviewTrust {
    /// 第三方/agent 生成的应用:无 `dozer://` 自定义协议、IPC 只走 `AppIpc`
    /// nonce 白名单、devtools/下载/新窗口严格关闭。走 `build_app_webview`。
    Application,
    /// 编辑器宿主(CodeMirror/JSON/tabular/git-log diff):`dozer://` 协议 +
    /// envelope IPC(`editor_binding` / `HostBinding`)。
    Editor,
    /// 预览渲染宿主(Flyfish/隔离 HTML/image-annotate/PlantUML/review-trace/browser):
    /// `dozer://` 协议 + envelope IPC。
    Preview,
    /// 固定单槽内置面板(usage-content/codehealth-content/todo-content/
    /// group-chat-content):`dozer://` 协议 + 固定槽 id + `parse_*_event`。
    BuiltinPanel,
}

impl WebviewTrust {
    /// 由 `spec` 的既有字段派生信任类别(唯一入口,避免新增易漏填字段)。
    ///
    /// 判据顺序:
    /// 1. 应用 id 区间 → `Application`(最外层:应用绝不能被当成宿主);
    /// 2. 固定单槽内置面板 → `BuiltinPanel`(已迁移带 `surface`;未迁移按固定槽 id);
    /// 3. 带 `editor_binding` → `Editor`;
    /// 4. 其余 → `Preview`。
    pub(crate) fn of(spec: &WebviewSpec) -> Self {
        if is_app_webview_id(spec.id) {
            return WebviewTrust::Application;
        }
        if spec.surface.is_some() || is_builtin_panel_slot(spec.id) {
            return WebviewTrust::BuiltinPanel;
        }
        if spec.editor_binding.is_some() {
            return WebviewTrust::Editor;
        }
        WebviewTrust::Preview
    }

    /// 是否走 `build_app_webview`(受限构建路径)。
    pub(crate) fn is_application(self) -> bool {
        matches!(self, WebviewTrust::Application)
    }

    /// 是否装载 `dozer://` 自定义协议。只有应用不装。
    pub(crate) fn allows_custom_protocol(self) -> bool {
        !self.is_application()
    }
}

/// 未迁移到共享 Surface、但仍按固定槽 id 识别 IPC 的内置面板槽位。
/// 已迁移的 Usage/CodeHealth 由 `spec.surface.is_some()` 覆盖。
fn is_builtin_panel_slot(id: usize) -> bool {
    id == crate::app::TODO_CONTENT_ID_OFFSET
        || id == crate::app::GROUP_CHAT_CONTENT_ID_OFFSET
        // 迁移完成后这两条会被 `surface.is_some()` 取代;保留以覆盖过渡态
        // (spec 构造到 Surface 接线之间的瞬间)与直接构造 spec 的测试。
        || id == crate::app::USAGE_CONTENT_ID_OFFSET
        || id == crate::app::CODEHEALTH_CONTENT_ID_OFFSET
}

/// 供分派点断言:应用 never 走宿主的 `dozer://` 构建路径。
#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::PanelKind;
    use crate::preview::WebviewSpec;

    fn spec(id: usize) -> WebviewSpec {
        WebviewSpec {
            id,
            url: "about:blank".into(),
            visible: true,
            editor_binding: None,
            reserve: None,
            loading_generation: None,
            surface: None,
            park_offscreen: false,
            display_i18n: None,
        }
    }

    #[test]
    fn app_id_derives_application() {
        let s = spec(crate::app::APP_CONTENT_ID_OFFSET);
        assert_eq!(WebviewTrust::of(&s), WebviewTrust::Application);
        assert!(WebviewTrust::of(&s).is_application());
        assert!(!WebviewTrust::of(&s).allows_custom_protocol());
    }

    #[test]
    fn fixed_slot_derives_builtin_panel() {
        let s = spec(crate::app::TODO_CONTENT_ID_OFFSET);
        assert_eq!(WebviewTrust::of(&s), WebviewTrust::BuiltinPanel);
        assert!(WebviewTrust::of(&s).allows_custom_protocol());
    }

    #[test]
    fn surface_marks_builtin_panel_even_without_offset() {
        let mut s = spec(12345);
        s.surface = Some(bytehost_core::SurfaceId::new(
            bytehost_core::PanelInstanceId::new(0, 0),
            bytehost_core::SurfaceSlot(12345),
            bytehost_core::Generation::new(0),
        ));
        assert_eq!(WebviewTrust::of(&s), WebviewTrust::BuiltinPanel);
    }

    #[test]
    fn editor_binding_derives_editor() {
        let mut s = spec(7);
        s.editor_binding = Some(crate::preview::EditorHostBinding::new(
            1,
            PanelKind::Files,
            1,
            std::path::PathBuf::from("/tmp/x.rs"),
        ));
        assert_eq!(WebviewTrust::of(&s), WebviewTrust::Editor);
    }

    #[test]
    fn plain_host_derives_preview() {
        let s = spec(7);
        assert_eq!(WebviewTrust::of(&s), WebviewTrust::Preview);
    }
}
