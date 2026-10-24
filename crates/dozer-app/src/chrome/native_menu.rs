//! macOS 原生右键菜单(NSMenu)。实现已搬进共享库 `byteui::native_menu`(与 Digger 共用),
//! 这里只保留原路径的重导出,调用点不用改。设计见
//! `docs/superpowers/specs/2026-09-16-native-context-menu-design.md`。

#![cfg(target_os = "macos")]

pub use byteui::native_menu::*;
