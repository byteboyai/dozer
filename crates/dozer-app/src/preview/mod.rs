//! 预览域:状态机、webview 期望清单、协议契约已搬进同源预览面板库
//! (`dozer-panel-preview`,S5b-2 P2);这里保留 Dozer 自己的部分(信任判定、webview 宿主清单项、
//! 全局单例与落盘位置绑定),并用 `pub(crate) use` 保持原有 `crate::preview::` 路径不变。

mod paths;
mod resources_global;
mod trust;
mod webview;
#[cfg(test)]
mod webview_spec_golden;

// 已搬进 `dozer-preview-core` 的模块:保留 `crate::preview::<mod>::` 路径。

pub(crate) use dozer_preview_core::backend::*;
#[allow(unused_imports)]
pub(crate) use dozer_preview_core::file_policy::*;
pub(crate) use dozer_preview_core::file_profile::*;
#[allow(unused_imports)]
pub(crate) use dozer_preview_core::large_text::*;
pub(crate) use dozer_preview_core::loading::*;
pub(crate) use dozer_preview_core::native_editor::*;
#[allow(unused_imports)]
pub(crate) use dozer_preview_core::recovery::*;
#[allow(unused_imports)]
pub(crate) use dozer_preview_core::resources::*;
pub(crate) use dozer_preview_core::router::*;
#[allow(unused_imports)]
pub(crate) use dozer_preview_core::startup::*;
pub(crate) use dozer_preview_core::text_save::*;

// 搬进 `dozer-panel-preview` 的状态机 / URL / 协议契约。核心已有同名项(`SearchHit` 等)时以核心为准。
pub(crate) use dozer_panel_preview::code_host::*;
pub(crate) use dozer_panel_preview::pane::*;
pub(crate) use dozer_panel_preview::plantuml::*;
pub(crate) use dozer_panel_preview::state::*;
pub(crate) use dozer_panel_preview::webview::*;
pub(crate) use dozer_panel_preview::webview_protocol::*;
pub(crate) use dozer_panel_preview::{BlankPaneInfo, EditorFont, PreviewPane, PreviewWebviewSpec};

pub(crate) use paths::*;
pub(crate) use resources_global::*;
pub(crate) use trust::WebviewTrust;
pub(crate) use webview::*;

/// Dozer 终端字体 → 预览 editor host 的字号 / 行高倍数(库不读任何全局,产品注入)。
pub(crate) fn editor_font() -> EditorFont {
    EditorFont {
        size: crate::theme::terminal_font::size(),
        line_height_factor: crate::theme::terminal_font::editor_line_height_factor(),
    }
}

/// 新建预览窗格:带上 Dozer 进程级的能力快照与终端字体(库的 `Default` 用固定参考值)。
pub(crate) fn new_preview_pane() -> PreviewPane {
    let mut pane = PreviewPane::default();
    pane.capabilities = crate::capabilities::current();
    pane.editor_font = editor_font();
    pane
}
