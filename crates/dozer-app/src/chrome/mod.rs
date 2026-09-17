//! 界面"chrome":顶栏、图标栏、页签组件、菜单(iced 弹层 + mac 原生 NSMenu)、
//! 首页落地页。Phase 4.4 把 `topbar.rs`/`rail.rs`/`tab_widget.rs`/`menu.rs`/
//! `native_menu.rs`/`homespace.rs` 六个平铺文件聚合进本目录。

pub mod homespace;
pub mod menu;
pub mod native_menu;
pub mod rail;
pub mod tab_widget;
pub mod topbar;
