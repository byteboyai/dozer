//! 一个预览面板(Files 或 Project)的全部可变状态:窗格 + 面板级错误 + 页签条翻页起点 + 溢出菜单锚点。
//! 产品持两份(原先 `Workspace` 里 `preview` / `project_preview` 两组并列字段)。

use dozer_model::LocalizedText;

use crate::PreviewPane;

#[derive(Default)]
pub struct PreviewPanelState {
    pub pane: PreviewPane,
    /// 面板级错误文案(打开不存在的文件、切代码模式失败等),视图渲染在内容区。
    pub error: Option<LocalizedText>,
    /// 页签条翻页窗口的起点下标。
    pub tab_first: usize,
    /// 页签溢出下拉的锚点(光标位置);`Some` 表示菜单已展开。
    pub overflow_anchor: Option<(f32, f32)>,
}

impl PreviewPanelState {
    pub fn new(pane: PreviewPane) -> Self {
        Self {
            pane,
            ..Default::default()
        }
    }
}
