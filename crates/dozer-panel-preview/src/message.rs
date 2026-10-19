//! 预览面板的消息。路由信息(项目 / 面板)由产品适配层负责,库里的消息只描述「对这一个面板做什么」。
//! 属于 webview 页面事件 / Agent 命令的消息(`*WebviewEvent`、`PreviewCommandsFetched`)不在这里。

use std::path::PathBuf;
use std::sync::Arc;

use crate::tabular::TabularView;
use crate::{
    FileProfile, LineIndex, PlantUmlDocument, PlantUmlLoadError, PreviewLoadStage, SearchOutcome,
};

#[derive(Debug, Clone)]
pub enum Message {
    // —— 页签 ——
    OpenPath {
        path: PathBuf,
        /// 打开后跳到的行(1-based);仅 CodeMirror 类页签生效。
        target_line: Option<usize>,
    },
    SelectTab(usize),
    CloseTab(usize),
    ToggleRenderMode(usize),
    TabOverflowToggle {
        /// 光标位置,作为溢出下拉的锚点。
        cursor: (f32, f32),
    },
    TabOverflowDismiss,
    // —— 降级页 / 冲突条 / 模式切换 ——
    OpenExternal(usize),
    OpenExternalFailed {
        path: PathBuf,
        error: String,
    },
    Retry(usize),
    PlainTextOpen(usize),
    ConflictKeep(usize),
    ConflictReload(usize),
    TabularTextModeToggle(usize),
    JsonModeToggle(usize),
    // —— 编辑器遗留(原生 iced 编辑器已退役,保留为 no-op 以便映射一一对应)——
    SaveActive,
    InsertTab,
    UndoActive,
    RedoActive,
    // —— 查找 ——
    FindOpen,
    FindOpenWithReplace,
    FindReplaceToggle,
    FindClose,
    FindText(String),
    FindGo(bool),
    FindCase(bool),
    FindReplacement(String),
    FindReplaceCurrent,
    FindReplaceAll,
    FindWebviewState {
        current: usize,
        total: usize,
    },
    // —— 整文件搜索(窗口化大文件)——
    LargeFileSearchClose,
    LargeFileSearchSubmit {
        tab_id: usize,
        query: String,
    },
    LargeFileSearchResults {
        tab_id: usize,
        generation: u64,
        result: Result<SearchOutcome, String>,
    },
    LargeFileSearchProgress {
        tab_id: usize,
        generation: u64,
        completed: u64,
        total: u64,
    },
    LargeFileSearchGo(bool),
    // —— 后台任务结果 ——
    BlankInfoLoaded {
        info: crate::BlankPaneInfo,
    },
    Profiled {
        tab_id: usize,
        generation: u64,
        result: Result<FileProfile, String>,
    },
    LoadTimeout {
        tab_id: usize,
        generation: u64,
        stage: PreviewLoadStage,
    },
    WindowIndex {
        tab_id: usize,
        result: Result<Arc<LineIndex>, String>,
    },
    RecoveryWritten(usize),
    RecoveryRead {
        tab_id: usize,
        generation: u64,
        restore: Option<String>,
    },
    TabularLoaded {
        tab_id: usize,
        generation: u64,
        result: Result<TabularView, String>,
    },
    TabularSheetLoaded {
        tab_id: usize,
        sheet_index: usize,
        result: Result<crate::tabular::Sheet, String>,
    },
    PlantUmlLoaded {
        tab_id: usize,
        generation: u64,
        result: Result<PlantUmlDocument, PlantUmlLoadError>,
    },
    // —— 页面事件(P4 进库)——
    /// 一条已由 `host_ipc::classify_ipc` 解析并校验归属的 webview 事件。
    Host(crate::host_ipc::HostEvent),
    /// `Effect::SaveFile` 的落盘回执。
    SaveFinished {
        tab_id: usize,
        result: Result<(), String>,
    },
}
