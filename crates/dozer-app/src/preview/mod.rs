//! 预览域状态机(P1d):左二 tabs、webview 期望清单。纯数据,不碰
//! wry/iced——webview 副作用由 main.rs 对照 `desired_webviews()` 差集
//! 执行(spike 约束:句柄只活在事件分发环)。
//!
//! 地址栏/URL tab(`TabKind::Web`)、`AddrTarget` 这套逻辑已经随浏览器
//! 面板扩展化(`extensions::browser::Tabs`)搬走——文件预览面板从来没有
//! 地址栏,这里只保留文件/验收两种 tab。

mod code_host;
mod paths;
mod resources_global;
// Task 5 建好后,Task 6 才会接线消费;在那之前内部条目视为暂未使用。
#[allow(dead_code)]
mod plantuml;
mod state;
mod trust;
mod view;
mod webview;
mod webview_protocol;

/// 预览层的 i18n owner(spec §6.1:官方 owner 固定 `dozer.<id>`)。
/// 预览不是 `PanelKind` 面板,资源在 `locales/panels/preview/`,但归
/// `dozer.preview` 命名空间,由 `extensions::PANEL_OWNERS` 一并登记以便解析。
pub(crate) use dozer_preview_core::PREVIEW_OWNER;

// 已搬进 `dozer-preview-core` 的模块:保留 `crate::preview::<mod>::` 与 `super::<mod>::` 路径,
// 没搬走的文件(`state.rs` / `view.rs` / `webview*.rs` 等)不用改 `use`。
#[cfg(test)]
pub(crate) use dozer_preview_core::file_profile;
pub(crate) use dozer_preview_core::native_editor;
// 产品相关的默认落盘位置(核心只提供 `*_in(config_dir)`):Dozer 在这里绑定自己的配置目录。
pub(crate) use dozer_preview_core::backend::*;
pub(crate) use dozer_preview_core::file_profile::*;
pub(crate) use dozer_preview_core::loading::*;
pub(crate) use dozer_preview_core::native_editor::*;
pub(crate) use dozer_preview_core::router::*;
pub(crate) use dozer_preview_core::text_save::*;
pub(crate) use paths::*;
pub(crate) use state::*;
pub(crate) use trust::WebviewTrust;
pub(crate) use view::*;
pub(crate) use webview::*;
// `code_host` / `webview_protocol`(Phase B 契约)与 `file_policy` /
// `large_text` / `resources`(Phase C 策略/资源)当前只在各自单测与后续接线
// 使用;对外 re-export 待消费方接入。
#[allow(unused_imports)]
pub(crate) use code_host::*;
#[allow(unused_imports)]
pub(crate) use dozer_preview_core::file_policy::*;
#[allow(unused_imports)]
pub(crate) use dozer_preview_core::large_text::*;
// `plantuml`(Task 5 受限 include resolver)当前只在自身单测使用;对外
// re-export 待 Task 6 的加载接线接入。
#[allow(unused_imports)]
pub(crate) use dozer_preview_core::recovery::*;
#[allow(unused_imports)]
pub(crate) use dozer_preview_core::resources::*;
#[allow(unused_imports)]
pub(crate) use dozer_preview_core::startup::*;
#[allow(unused_imports)]
pub(crate) use plantuml::*;
pub(crate) use resources_global::*;
#[allow(unused_imports)]
pub(crate) use webview_protocol::*;
