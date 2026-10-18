//! 预览引擎纯逻辑核心(S5b-2 P1)。见 `Cargo.toml` 的依赖约束:不含界面、不含 webview、不含全局状态。

pub mod capabilities;
pub mod file_profile;
pub mod large_text;
pub mod native_editor;
pub mod recovery;
pub mod startup;
pub mod tabular_ext;
pub mod text_range;
