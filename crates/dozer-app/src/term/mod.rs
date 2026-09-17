//! 终端域:终端模型(PTY 网格/回显)、终端视图(自绘 canvas)、终端消息目标
//! 与 tab 栏。Phase 4.3 把 `term_model.rs`/`term_view.rs`/`terminal.rs` 三个
//! 平铺文件聚合进本目录。

pub mod term_model;
pub mod term_view;
pub mod terminal;
