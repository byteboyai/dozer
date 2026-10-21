//! OSC 7 / OSC 133 扫描器已搬到共享库 `dozer-panel-term::osc`(S6c-1)。
//! 本文件只保留重新导出,既有 `crate::osc::OscScanner` 调用点不动。

pub use dozer_panel_term::osc::*;
