//! 同源 Files 面板库(S5b-1):Dozer 与 Digger 共用的文件树数据层、文件系统层与视图层。
//!
//! 依赖闭包**不含** `dozer-app` / `dozer-client` / `dozer-core`。产品专属语义经注入提供,见 [`adapter`];
//! 请求追踪 / 激活上下文 / 提示发件箱来自共享的 `dozer-panel-host`。

pub mod adapter;
pub mod dialogs;
pub mod external_apps;
pub mod fs;
pub mod git_status;
pub mod i18n;
pub mod menu;
pub mod state;
pub mod tree;
pub mod update;

pub use state::{FilesToolbarTarget, GitInfo, Message};

/// 面板 owner id(spec §5):`locales/`(随库)。
pub const FILES_OWNER: &str = "dozer.files";

#[cfg(test)]
mod tests;
