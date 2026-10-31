//! 历史弹窗 diff 子视图的**共用创建路径**(Tier2 Task1)。
//!
//! `file_history_overlay` 与 `edit_history_overlay` 是两份结构相同的弹窗宿主:都
//! 在卡片里挂一个 `dozer://` 协议的 diff 子 webview,IPC 都走同一条
//! `parse_event` + `validate` 管线,只有「把已校验事件发成哪个 `Message` 变体」和
//! 日志措辞不同。本模块收掉创建样板;协议处理、IPC 校验、可见性这些安全/行为
//! 细节**只有这一份**。
//!
//! 两个差异点由调用方注入:
//! - `on_event`:已通过归属校验的事件交给谁(两个弹窗各自的 `Message` 变体);
//! - `log_label`:日志前缀(`file-history` / `edit-history`),只影响文案。

use std::sync::{Arc, Mutex};

use winit::window::Window;

use crate::preview::{EditorEvent, EditorHostBinding, HostBinding, WebviewEnvelope, parse_event};

dozer_core::scope!(LOG, module, "platform");

/// 创建一个 diff 子 webview 所需的全部输入。
pub(crate) struct DiffWebviewConfig {
    /// 已拼好的 `dozer://…diff…` 地址(由调用方按各自绑定算出)。
    pub url: String,
    /// 在弹窗客户区坐标系里的矩形。
    pub bounds: wry::Rect,
    /// 预览资源白名单(`dozer://` 协议共用同一份)。
    pub allowed_files: Arc<Mutex<dozer_panel_preview::assets::AllowedFiles>>,
    /// IPC 事件的期望归属;不匹配的事件被丢弃并记日志。
    pub expected_binding: EditorHostBinding,
    /// 日志前缀,如 `file-history` / `edit-history`。
    pub log_label: &'static str,
}

/// 按统一策略创建 diff 子 webview:只认 `dozer://` 协议,页面事件必须通过
/// `expected_binding` 的归属校验后才交给 `on_event`。
pub(crate) fn build_diff_webview(
    window: &Arc<Window>,
    config: DiffWebviewConfig,
    on_event: impl Fn(HostBinding, WebviewEnvelope<EditorEvent>) + Send + 'static,
) -> Result<wry::WebView, String> {
    let DiffWebviewConfig {
        url,
        bounds,
        allowed_files,
        expected_binding,
        log_label,
    } = config;
    let root = crate::assets::assets_root();

    wry::WebViewBuilder::new()
        .with_url(&url)
        .with_bounds(bounds)
        .with_visible(true)
        .with_custom_protocol("dozer".into(), move |_id, request| {
            let allowed = allowed_files.lock().expect("allowed_files 锁");
            let reply =
                crate::assets::handle_protocol(&root, &allowed, None, &request.uri().to_string());
            wry::http::Response::builder()
                .status(reply.status)
                .header("Content-Type", reply.mime)
                .body(std::borrow::Cow::Owned(reply.body))
                .unwrap()
        })
        .with_ipc_handler(move |req| {
            let expected = HostBinding::new(
                expected_binding.project_id,
                expected_binding.panel,
                expected_binding.tab_id,
                expected_binding.document_id(),
            );
            match parse_event(req.body().as_str()) {
                Ok(event) => {
                    if let Err(error) = event.validate(&expected) {
                        dozer_core::log_warn!(LOG, %error, "拒绝无效 {} diff IPC", log_label);
                    } else {
                        on_event(expected, event);
                    }
                }
                Err(error) => {
                    dozer_core::log_warn!(LOG, %error, "无法解析 {} diff IPC", log_label);
                }
            }
        })
        .build_as_child(window)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    /// 两个弹窗都必须走这里创建 diff webview:任何一方重新抄一份 builder 都会让
    /// 协议/IPC 校验出现第二份实现(本 Task 要消除的正是这个)。没有廉价夹具能
    /// 真建 wry 子视图,用源码扫描钉住委派关系。
    #[test]
    fn both_history_overlays_delegate_to_the_shared_builder() {
        for (name, source) in [
            (
                "file_history_overlay.rs",
                include_str!("file_history_overlay.rs"),
            ),
            (
                "edit_history_overlay.rs",
                include_str!("edit_history_overlay.rs"),
            ),
        ] {
            assert!(
                source.contains("diff_webview::build_diff_webview("),
                "{name} 必须调用共用的 build_diff_webview"
            );
            for forbidden in [
                "WebViewBuilder::new()",
                "with_custom_protocol(",
                "with_ipc_handler(",
            ] {
                assert!(
                    !source.contains(forbidden),
                    "{name} 里不得再出现 {forbidden}(应只留在 diff_webview.rs)"
                );
            }
        }
    }
}
