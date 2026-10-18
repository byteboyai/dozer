//! 编辑器 / 预览共用的 1-based 文本位置与选区(纯数据)。从 `webview_protocol` 搬来,
//! 因为恢复快照(`recovery`)需要它们,而核心不能依赖协议模块。

use serde::{Deserialize, Serialize};

/// 1-based 文本位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextPosition {
    pub line: u32,
    pub column: u32,
}

/// 文本选区。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextRange {
    pub start: TextPosition,
    pub end: TextPosition,
}
