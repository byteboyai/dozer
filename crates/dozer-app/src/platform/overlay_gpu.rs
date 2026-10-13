//! C10 方案 C:wgpu 渲染管线 + 遮罩外壳已上收 `bytehost_desktop::overlay_gpu`
//! (`overlay` feature)。本模块只做**重导出**,让各消费方的
//! `crate::platform::overlay_gpu::*` 路径保持不变。
//!
//! 少数消费方自己手写 `UserInterface::build`/`update`(`search`/`file_history`/
//! `settings`/`project_create`),需要 `apply_cursor`;它随 `OverlayGpu`
//! 一起从宿主来。
//!
//! 产品侧调用 `backdrop_card*` 一律走 [`crate::platform::overlay_window`] 的
//! 薄适配(那边注入主题遮罩色),不要直接经本模块——这里导出的宿主版本要求
//! 调用方自带 `scrim`。

#[allow(unused_imports)]
pub(crate) use bytehost_desktop::overlay_gpu::{OverlayGpu, apply_cursor};
