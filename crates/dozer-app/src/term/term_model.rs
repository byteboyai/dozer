//! 终端模型已抽到共享库 `dozer-panel-term`(S6b)。本文件只保留转调,
//! 让既有 `crate::term::term_model::*` 调用点不需要改动。

pub use dozer_panel_term::model::*;
