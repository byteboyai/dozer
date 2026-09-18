//! 窗口/拖拽/文件选择平台胶水。Phase 1 结构重组时从 `main.rs` 抽出:原
//! 生交通灯居中、顶栏原生拖窗守卫、外部文件拖拽悬停位置追踪、文件/目录
//! 选择。全部逻辑保持原样,只改模块归属。

pub mod file_drag;
pub mod picker;
pub mod search_overlay;
pub mod window;
pub mod window_events;
