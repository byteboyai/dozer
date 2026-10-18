//! 预览引擎纯逻辑核心(S5b-2 P1)。见 `Cargo.toml` 的依赖约束:不含界面、不含 webview、不含全局状态。

/// 预览层的 i18n owner(官方 owner 固定 `dozer.<id>`);资源在本 crate 的 `locales/`。
pub const PREVIEW_OWNER: &str = "dozer.preview";

pub mod backend;
pub mod capabilities;
pub mod file_policy;
pub mod file_profile;
pub mod i18n;
pub mod large_text;
pub mod loading;
pub mod native_editor;
pub mod recovery;
pub mod resources;
pub mod router;
pub mod startup;
pub mod tabular_ext;
pub mod text_range;
pub mod text_save;
