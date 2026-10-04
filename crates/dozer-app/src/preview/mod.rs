//! 预览域状态机(P1d):左二 tabs、webview 期望清单。纯数据,不碰
//! wry/iced——webview 副作用由 main.rs 对照 `desired_webviews()` 差集
//! 执行(spike 约束:句柄只活在事件分发环)。
//!
//! 地址栏/URL tab(`TabKind::Web`)、`AddrTarget` 这套逻辑已经随浏览器
//! 面板扩展化(`extensions::browser::Tabs`)搬走——文件预览面板从来没有
//! 地址栏,这里只保留文件/验收两种 tab。

mod backend;
mod code_host;
mod file_policy;
mod file_profile;
mod large_text;
mod loading;
mod native_editor;
// Task 5 建好后,Task 6 才会接线消费;在那之前内部条目视为暂未使用。
#[allow(dead_code)]
mod plantuml;
mod recovery;
mod resources;
mod router;
mod startup;
mod state;
mod text_save;
mod view;
mod webview;
mod webview_protocol;

pub(crate) use backend::*;
pub(crate) use file_profile::*;
pub(crate) use loading::*;
pub(crate) use native_editor::*;
pub(crate) use router::*;
pub(crate) use state::*;
pub(crate) use text_save::*;
pub(crate) use view::*;
pub(crate) use webview::*;
// `code_host` / `webview_protocol`(Phase B 契约)与 `file_policy` /
// `large_text` / `resources`(Phase C 策略/资源)当前只在各自单测与后续接线
// 使用;对外 re-export 待消费方接入。
#[allow(unused_imports)]
pub(crate) use code_host::*;
#[allow(unused_imports)]
pub(crate) use file_policy::*;
#[allow(unused_imports)]
pub(crate) use large_text::*;
// `plantuml`(Task 5 受限 include resolver)当前只在自身单测使用;对外
// re-export 待 Task 6 的加载接线接入。
#[allow(unused_imports)]
pub(crate) use plantuml::*;
#[allow(unused_imports)]
pub(crate) use recovery::*;
#[allow(unused_imports)]
pub(crate) use resources::*;
#[allow(unused_imports)]
pub(crate) use startup::*;
#[allow(unused_imports)]
pub(crate) use webview_protocol::*;
