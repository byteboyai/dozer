//! 所有 webview host 共用的消息 envelope(文件预览重构 Phase B Task 3)。
//!
//! CodeMirror / Flyfish / vanilla-jsoneditor 三种 host 复用同一结构,各自只
//! 扩展命令/事件名。Rust 侧只做**具名命令**解析,禁止执行任意 JS;解析失败
//! 不 panic,返回错误由调用方丢弃。
//!
//! 坐标协议对外统一 1-based line/column(桥接层负责 CodeMirror offset、
//! Unicode 列与 UTF-8 byte offset 的转换)。

// 本模块是 Phase B 建立的通用契约,Scheme/host 与 IPC 派发接线在后续步骤
// 完成前部分 API 暂未被非测试代码调用;显式允许,避免 dead_code 噪声。
#![allow(dead_code)]

use serde::{Deserialize, Serialize};

use crate::app::PanelKind;

/// 当前协议版本。JS/Rust 两侧必须一致,否则消息被拒。
pub const PROTOCOL_VERSION: u32 = 1;

/// 单条消息的字节上限(恶意/异常超大消息直接拒绝,不进解析)。
pub const MAX_MESSAGE_BYTES: usize = 8 * 1024 * 1024;

/// 通用 envelope。`payload` 为具名命令/事件。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WebviewEnvelope<T> {
    pub protocol_version: u32,
    #[serde(default)]
    pub project_id: i64,
    #[serde(default)]
    pub panel: String,
    #[serde(default)]
    pub tab_id: usize,
    #[serde(default)]
    pub document_id: String,
    #[serde(default)]
    pub revision: u64,
    #[serde(default)]
    pub request_id: Option<String>,
    pub payload: T,
}

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

/// 折叠区(行范围)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FoldRange {
    pub from_line: u32,
    pub to_line: u32,
}

/// 编辑器栈 -> Rust 的事件。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EditorEvent {
    Ready {
        read_only: bool,
        language: String,
    },
    SelectionChanged {
        anchor: TextPosition,
        head: TextPosition,
        cursor: TextPosition,
        #[serde(default)]
        selected_text: Option<String>,
    },
    DocumentChanged {
        revision: u64,
        length: u64,
        changes: Vec<TextChange>,
    },
    SaveRequested {
        revision: u64,
        text: String,
    },
    /// 编辑器防抖上报的脏正文快照(用于 dirty recovery)。`text` 有大小上限,
    /// 由 JS 侧只在真正变脏时按节流发送。
    Snapshot {
        revision: u64,
        text: String,
    },
    FocusChanged {
        focused: bool,
    },
    ViewportChanged {
        from_line: u32,
        to_line: u32,
    },
    ViewState {
        cursor: TextPosition,
        selection: Option<TextRange>,
        top_line: u32,
        folds: Vec<FoldRange>,
    },
    /// 窗口化 viewer 请求相邻窗口。`edge` 表示用户靠近
    /// 窗口的哪一端;`anchor_line` 是当前窗口内的全局行号。Rust 用稀疏索引
    /// 读相邻窗口后回 `SetWindow`。
    WindowRequest {
        edge: WindowEdge,
        anchor_line: u32,
    },
    /// 窗口化 viewer 的 ⌘F:请求在**整文件**上做流式搜索(而不是只搜持有
    /// 窗口)。Rust 打开大文件搜索条并跑 `large_text::stream_search`。
    FindRequest,
    Failed {
        message: String,
        recoverable: bool,
    },
}

/// 窗口化 viewer 请求的相邻窗口方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowEdge {
    Top,
    Bottom,
}

/// CodeMirror change set 的单条增量(offset 域,JS 侧坐标)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextChange {
    pub from: u64,
    pub to: u64,
    pub insert: String,
}

/// Rust -> 编辑器的具名命令。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EditorCommand {
    SetDocument {
        text: String,
        revision: u64,
        language: String,
        read_only: bool,
    },
    /// 窗口化只读 viewer:把文档替换为某个全局行区间的内容,`start_line` 是
    /// 窗口首行的全局行号(用于全局行号显示与坐标换算),`total_lines` 供状态栏。
    /// `truncated` 表示窗口因字节上限被截断(超长单行只含前缀),展示端提示。
    SetWindow {
        text: String,
        start_line: u32,
        total_lines: u32,
        revision: u64,
        #[serde(default)]
        truncated: bool,
    },
    /// Git Log diff pane 专用:CodeMirror `unifiedMergeView` 需要旧/新两份
    /// 完整文档自己跑 diff 算法,不是 unified patch 文本。恒只读
    /// (`read_only` 字段仍保留是为了和其它命令的字段形状一致,当前唯一
    /// 调用方 `runtime.rs` 恒传 `true`)。
    SetDiffDocument {
        old_text: String,
        new_text: String,
        language: String,
        revision: u64,
        read_only: bool,
    },
    RevealPosition {
        line: u32,
        column: u32,
    },
    SelectRange {
        start: TextPosition,
        end: TextPosition,
    },
    ReplaceRange {
        start: TextPosition,
        end: TextPosition,
        text: String,
        revision: u64,
    },
    OpenFind {
        query: Option<String>,
        replace: bool,
    },
    SetReadOnly {
        read_only: bool,
    },
    Focus,
    SerializeViewState {
        request_id: String,
    },
    /// T11:一次恢复完整视图状态。顺序固定由 host 保证:先展开/重折 folds,再
    /// 落 selection/cursor,最后滚到 top_line。窗口化只送全局 `top_line`(不送
    /// 局部 offset),`folds` 为空。
    RestoreViewState {
        #[serde(default)]
        cursor: Option<TextPosition>,
        #[serde(default)]
        selection: Option<TextRange>,
        #[serde(default)]
        top_line: Option<u32>,
        #[serde(default)]
        folds: Vec<FoldRange>,
    },
    /// 让 host 用**当前 buffer**发起一次保存(等价用户按 ⌘S):host 收到后
    /// 走 `saveHandler` → 回 `save_requested`。用于关闭 dirty tab 前先把
    /// 磁盘内容补齐(Rust 侧不持有全文,必须经由 host 落盘)。
    SaveDocument,
}

/// 解析/校验错误。调用方只做日志/丢弃,不 panic。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolError {
    TooLarge { bytes: usize },
    BadJson(String),
    Version { got: u32, expected: u32 },
    ProjectId { got: i64, expected: i64 },
    Panel { got: String, expected: &'static str },
    TabId { got: usize, expected: usize },
    DocumentId { got: String, expected: String },
    UnknownPayload(String),
}

impl std::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge { bytes } => write!(f, "消息过大({bytes} 字节)"),
            Self::BadJson(e) => write!(f, "消息非合法 JSON: {e}"),
            Self::Version { got, expected } => {
                write!(f, "协议版本不匹配(收到 {got},期望 {expected})")
            }
            Self::ProjectId { got, expected } => {
                write!(f, "project_id 不匹配(收到 {got},期望 {expected})")
            }
            Self::Panel { got, expected } => {
                write!(f, "panel 不匹配(收到 {got},期望 {expected})")
            }
            Self::TabId { got, expected } => {
                write!(f, "tab_id 不匹配(收到 {got},期望 {expected})")
            }
            Self::DocumentId { got, expected } => {
                write!(f, "document_id 不匹配(收到 {got},期望 {expected})")
            }
            Self::UnknownPayload(k) => write!(f, "未知 payload kind: {k}"),
        }
    }
}

/// 一个 webview 绑定的归属信息,用于校验入站消息不得自报别的 project/tab/doc。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostBinding {
    pub project_id: i64,
    pub panel: PanelKind,
    pub tab_id: usize,
    pub document_id: String,
}

impl HostBinding {
    pub fn new(project_id: i64, panel: PanelKind, tab_id: usize, document_id: String) -> Self {
        Self {
            project_id,
            panel,
            tab_id,
            document_id,
        }
    }

    fn panel_token(&self) -> &'static str {
        match self.panel {
            PanelKind::Project => "project",
            PanelKind::GitLog => "gitlog",
            _ => "files",
        }
    }
}

impl<T> WebviewEnvelope<T> {
    /// 校验 envelope 归属当前 host 绑定。版本/项目/面板/tab/文档 id 全部
    /// 必须一致——JS 不能自报任意路径/归属。
    pub fn validate(&self, expect: &HostBinding) -> Result<(), ProtocolError> {
        if self.protocol_version != PROTOCOL_VERSION {
            return Err(ProtocolError::Version {
                got: self.protocol_version,
                expected: PROTOCOL_VERSION,
            });
        }
        if self.project_id != expect.project_id {
            return Err(ProtocolError::ProjectId {
                got: self.project_id,
                expected: expect.project_id,
            });
        }
        if self.panel != expect.panel_token() {
            return Err(ProtocolError::Panel {
                got: self.panel.clone(),
                expected: expect.panel_token(),
            });
        }
        if self.tab_id != expect.tab_id {
            return Err(ProtocolError::TabId {
                got: self.tab_id,
                expected: expect.tab_id,
            });
        }
        if self.document_id != expect.document_id {
            return Err(ProtocolError::DocumentId {
                got: self.document_id.clone(),
                expected: expect.document_id.clone(),
            });
        }
        Ok(())
    }
}

/// 解析一条入站事件(JSON 字符串)。过大/非法/未知 kind 均返回错误。
pub fn parse_event(raw: &str) -> Result<WebviewEnvelope<EditorEvent>, ProtocolError> {
    if raw.len() > MAX_MESSAGE_BYTES {
        return Err(ProtocolError::TooLarge { bytes: raw.len() });
    }
    let env: WebviewEnvelope<serde_json::Value> =
        serde_json::from_str(raw).map_err(|e| ProtocolError::BadJson(e.to_string()))?;
    // 先按 tagged enum 解析 payload;未知 kind 报可读错误。
    let kind = env
        .payload
        .get("kind")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_default();
    let payload: EditorEvent = serde_json::from_value(env.payload)
        .map_err(|_| ProtocolError::UnknownPayload(kind.clone()))?;
    Ok(WebviewEnvelope {
        protocol_version: env.protocol_version,
        project_id: env.project_id,
        panel: env.panel,
        tab_id: env.tab_id,
        document_id: env.document_id,
        revision: env.revision,
        request_id: env.request_id,
        payload,
    })
}

/// JSON host(vanilla-jsoneditor)的事件。与 `EditorEvent` 共用 envelope,只
/// 扩展自己的命令/事件名。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum JsonEvent {
    Ready {
        read_only: bool,
        language: String,
    },
    /// 树编辑后的当前文本(可能较大,由 JS 侧节流)。
    DocumentChanged {
        revision: u64,
        text: String,
    },
    Failed {
        message: String,
        recoverable: bool,
    },
}

/// 解析一条 JSON host 事件。与 [`parse_event`] 同规则(超大/非法/未知不 panic)。
pub fn parse_json_event(raw: &str) -> Result<WebviewEnvelope<JsonEvent>, ProtocolError> {
    if raw.len() > MAX_MESSAGE_BYTES {
        return Err(ProtocolError::TooLarge { bytes: raw.len() });
    }
    let env: WebviewEnvelope<serde_json::Value> =
        serde_json::from_str(raw).map_err(|e| ProtocolError::BadJson(e.to_string()))?;
    let kind = env
        .payload
        .get("kind")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_default();
    let payload: JsonEvent = serde_json::from_value(env.payload)
        .map_err(|_| ProtocolError::UnknownPayload(kind.clone()))?;
    Ok(WebviewEnvelope {
        protocol_version: env.protocol_version,
        project_id: env.project_id,
        panel: env.panel,
        tab_id: env.tab_id,
        document_id: env.document_id,
        revision: env.revision,
        request_id: env.request_id,
        payload,
    })
}

/// Flyfish 渲染 host 的事件(T9)。与 `EditorEvent`/`JsonEvent` 共用 envelope,
/// 但保留自己的 payload enum——不与 editor 命令混用。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FlyfishEvent {
    /// host 脚本初始化完成(元素已插入)。
    Ready,
    /// 渲染失败(读盘/解码错误、资源 404)。
    Failed { message: String, recoverable: bool },
    /// 文档标题(展示在 tab 上)。
    Title { title: String },
    /// 页内搜索状态(当前序号 / 命中总数)。
    SearchState { current: usize, total: usize },
}

/// 解析一条 Flyfish host 事件。与 [`parse_event`] 同规则(超大/非法/未知不 panic)。
pub fn parse_flyfish_event(raw: &str) -> Result<WebviewEnvelope<FlyfishEvent>, ProtocolError> {
    if raw.len() > MAX_MESSAGE_BYTES {
        return Err(ProtocolError::TooLarge { bytes: raw.len() });
    }
    let env: WebviewEnvelope<serde_json::Value> =
        serde_json::from_str(raw).map_err(|e| ProtocolError::BadJson(e.to_string()))?;
    let kind = env
        .payload
        .get("kind")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_default();
    let payload: FlyfishEvent = serde_json::from_value(env.payload)
        .map_err(|_| ProtocolError::UnknownPayload(kind.clone()))?;
    Ok(WebviewEnvelope {
        protocol_version: env.protocol_version,
        project_id: env.project_id,
        panel: env.panel,
        tab_id: env.tab_id,
        document_id: env.document_id,
        revision: env.revision,
        request_id: env.request_id,
        payload,
    })
}

/// 编码一条 Rust -> 编辑器的命令为 envelope JSON,供 `evaluate_script` 注入。
pub fn encode_command(
    project_id: i64,
    panel: PanelKind,
    tab_id: usize,
    document_id: &str,
    revision: u64,
    request_id: Option<String>,
    command: EditorCommand,
) -> String {
    let env = WebviewEnvelope {
        protocol_version: PROTOCOL_VERSION,
        project_id,
        panel: match panel {
            PanelKind::Project => "project".to_string(),
            PanelKind::GitLog => "gitlog".to_string(),
            _ => "files".to_string(),
        },
        tab_id,
        document_id: document_id.to_string(),
        revision,
        request_id,
        payload: command,
    };
    serde_json::to_string(&env).unwrap_or_else(|_| "{}".to_string())
}

/// 把 `encode_command` 产出的 envelope JSON 包成可 `evaluate_script` 注入的
/// JS 片段。`__dozer.dispatch` 尚未就绪(页面还没 boot 完)时静默跳过,
/// 不抛错、不 panic。
pub fn dispatch_script(envelope_json: &str) -> String {
    let literal = serde_json::to_string(envelope_json).unwrap_or_else(|_| "\"{}\"".to_string());
    format!("window.__dozer&&window.__dozer.dispatch&&window.__dozer.dispatch({literal});")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding() -> HostBinding {
        HostBinding::new(7, PanelKind::Files, 3, "p7-t3".to_string())
    }

    fn raw(payload: &str) -> String {
        format!(
            r#"{{"protocol_version":1,"project_id":7,"panel":"files","tab_id":3,"document_id":"p7-t3","revision":2,"request_id":null,"payload":{payload}}}"#
        )
    }

    #[test]
    fn parses_ready_and_validates() {
        let env = parse_event(&raw(
            r#"{"kind":"ready","read_only":true,"language":"rust"}"#,
        ))
        .unwrap();
        assert_eq!(
            env.payload,
            EditorEvent::Ready {
                read_only: true,
                language: "rust".to_string()
            }
        );
        assert!(env.validate(&binding()).is_ok());
    }

    #[test]
    fn parses_save_requested_with_text() {
        let env = parse_event(&raw(
            r#"{"kind":"save_requested","revision":4,"text":"hello\n"}"#,
        ))
        .unwrap();
        assert!(matches!(
            env.payload,
            EditorEvent::SaveRequested { revision: 4, .. }
        ));
    }

    #[test]
    fn parses_selection_and_view_state() {
        let sel = parse_event(&raw(
            r#"{"kind":"selection_changed","anchor":{"line":1,"column":1},"head":{"line":2,"column":3},"cursor":{"line":2,"column":3}}"#,
        ))
        .unwrap();
        assert!(matches!(sel.payload, EditorEvent::SelectionChanged { .. }));

        let vs = parse_event(&raw(
            r#"{"kind":"view_state","cursor":{"line":5,"column":2},"selection":null,"top_line":1,"folds":[{"from_line":3,"to_line":9}]}"#,
        ))
        .unwrap();
        assert!(matches!(vs.payload, EditorEvent::ViewState { .. }));
        // T11:view_state 必须完整携带 folds,不能丢。
        assert!(matches!(
            vs.payload,
            EditorEvent::ViewState { folds, top_line: 1, .. } if folds.len() == 1
        ));

        let wr = parse_event(&raw(
            r#"{"kind":"window_request","edge":"bottom","anchor_line":900}"#,
        ))
        .unwrap();
        assert_eq!(
            wr.payload,
            EditorEvent::WindowRequest {
                edge: WindowEdge::Bottom,
                anchor_line: 900
            }
        );
    }

    #[test]
    fn restore_view_state_command_round_trips_with_defaults() {
        // 全字段。
        let cmd = EditorCommand::RestoreViewState {
            cursor: Some(TextPosition { line: 5, column: 2 }),
            selection: Some(TextRange {
                start: TextPosition { line: 1, column: 1 },
                end: TextPosition { line: 5, column: 2 },
            }),
            top_line: Some(3),
            folds: vec![FoldRange {
                from_line: 3,
                to_line: 9,
            }],
        };
        let json = serde_json::to_string(&cmd).unwrap();
        let back: EditorCommand = serde_json::from_str(&json).unwrap();
        assert_eq!(back, cmd);

        // 缺省字段(仅 kind + folds)也要能解析为默认。
        let minimal: EditorCommand =
            serde_json::from_str(r#"{"kind":"restore_view_state","folds":[]}"#).unwrap();
        assert_eq!(
            minimal,
            EditorCommand::RestoreViewState {
                cursor: None,
                selection: None,
                top_line: None,
                folds: Vec::new(),
            }
        );
    }

    #[test]
    fn encodes_set_window_command() {
        let s = encode_command(
            1,
            PanelKind::Files,
            2,
            "p1-t2",
            3,
            None,
            EditorCommand::SetWindow {
                text: "a\nb\n".into(),
                start_line: 1001,
                total_lines: 500_000,
                revision: 3,
                truncated: false,
            },
        );
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["payload"]["kind"], "set_window");
        assert_eq!(v["payload"]["start_line"], 1001);
        assert_eq!(v["payload"]["truncated"], false);
    }

    #[test]
    fn encodes_set_diff_document_command() {
        let s = encode_command(
            0,
            PanelKind::GitLog,
            0,
            "gitlog-diff",
            9,
            None,
            EditorCommand::SetDiffDocument {
                old_text: "old\n".into(),
                new_text: "new\n".into(),
                language: "rust".into(),
                revision: 9,
                read_only: true,
            },
        );
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["payload"]["kind"], "set_diff_document");
        assert_eq!(v["payload"]["old_text"], "old\n");
        assert_eq!(v["payload"]["new_text"], "new\n");
        assert_eq!(v["payload"]["read_only"], true);
        assert_eq!(v["panel"], "gitlog");
    }

    #[test]
    fn set_diff_document_round_trips() {
        let cmd = EditorCommand::SetDiffDocument {
            old_text: "a\nb\n".into(),
            new_text: "a\nc\n".into(),
            language: "python".into(),
            revision: 3,
            read_only: true,
        };
        let json = serde_json::to_string(&cmd).unwrap();
        let back: EditorCommand = serde_json::from_str(&json).unwrap();
        assert_eq!(back, cmd);
    }

    /// 关闭脏 tab 前下发的"用当前 buffer 保存一次"命令,payload 只是一个
    /// 无参 `save_document`(正文由 host 自己持有)。
    #[test]
    fn encodes_save_document_command() {
        let s = encode_command(
            1,
            PanelKind::Files,
            2,
            "p1-t2",
            7,
            None,
            EditorCommand::SaveDocument,
        );
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["payload"]["kind"], "save_document");
        assert_eq!(v["revision"], 7);
    }

    #[test]
    fn parses_snapshot_event() {
        let env =
            parse_event(&raw(r#"{"kind":"snapshot","revision":6,"text":"dirty\n"}"#)).unwrap();
        assert_eq!(
            env.payload,
            EditorEvent::Snapshot {
                revision: 6,
                text: "dirty\n".into()
            }
        );
    }

    #[test]
    fn parses_find_request_event() {
        let env = parse_event(&raw(r#"{"kind":"find_request"}"#)).unwrap();
        assert_eq!(env.payload, EditorEvent::FindRequest);
    }

    #[test]
    fn parses_json_host_events() {
        let ready = parse_json_event(&raw(
            r#"{"kind":"ready","read_only":true,"language":"json"}"#,
        ))
        .unwrap();
        assert_eq!(
            ready.payload,
            JsonEvent::Ready {
                read_only: true,
                language: "json".into()
            }
        );
        let changed = parse_json_event(&raw(
            r#"{"kind":"document_changed","revision":2,"text":"{}"}"#,
        ))
        .unwrap();
        assert!(matches!(
            changed.payload,
            JsonEvent::DocumentChanged { revision: 2, .. }
        ));
        assert!(matches!(
            parse_json_event(&raw(r#"{"kind":"evil"}"#)),
            Err(ProtocolError::UnknownPayload(_))
        ));
    }

    #[test]
    fn rejects_wrong_version_project_panel_tab_document() {
        let env = parse_event(&raw(
            r#"{"kind":"ready","read_only":false,"language":"txt"}"#,
        ))
        .unwrap();

        let mut wrong = env.clone();
        wrong.protocol_version = 99;
        assert!(matches!(
            wrong.validate(&binding()),
            Err(ProtocolError::Version { .. })
        ));

        let mut wrong = env.clone();
        wrong.project_id = 8;
        assert!(matches!(
            wrong.validate(&binding()),
            Err(ProtocolError::ProjectId { .. })
        ));

        let mut wrong = env.clone();
        wrong.panel = "project".into();
        assert!(matches!(
            wrong.validate(&binding()),
            Err(ProtocolError::Panel { .. })
        ));

        let mut wrong = env.clone();
        wrong.tab_id = 4;
        assert!(matches!(
            wrong.validate(&binding()),
            Err(ProtocolError::TabId { .. })
        ));

        let mut wrong = env.clone();
        wrong.document_id = "other".into();
        assert!(matches!(
            wrong.validate(&binding()),
            Err(ProtocolError::DocumentId { .. })
        ));
    }

    #[test]
    fn rejects_malformed_json_without_panic() {
        assert!(matches!(
            parse_event("not json"),
            Err(ProtocolError::BadJson(_))
        ));
        assert!(matches!(parse_event("{}"), Err(ProtocolError::BadJson(_))));
    }

    #[test]
    fn rejects_unknown_payload_kind() {
        assert!(matches!(
            parse_event(&raw(r#"{"kind":"evil","x":1}"#)),
            Err(ProtocolError::UnknownPayload(_))
        ));
    }

    #[test]
    fn rejects_oversized_message() {
        let big = "a".repeat(MAX_MESSAGE_BYTES + 1);
        assert!(matches!(
            parse_event(&big),
            Err(ProtocolError::TooLarge { .. })
        ));
    }

    #[test]
    fn encodes_command_envelope() {
        let s = encode_command(
            7,
            PanelKind::Project,
            3,
            "p7-t3",
            5,
            Some("req-1".into()),
            EditorCommand::RevealPosition {
                line: 10,
                column: 2,
            },
        );
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["panel"], "project");
        assert_eq!(v["payload"]["kind"], "reveal_position");
        assert_eq!(v["request_id"], "req-1");
    }

    #[test]
    fn host_binding_panel_token_covers_gitlog() {
        let b = HostBinding::new(0, PanelKind::GitLog, 0, "gitlog-diff".into());
        // 不能直接调用私有 `panel_token()`,靠 `validate()` 间接验证:一条
        // 携带 `panel: "gitlog"` 的 envelope 必须通过校验。
        let env = WebviewEnvelope {
            protocol_version: PROTOCOL_VERSION,
            project_id: 0,
            panel: "gitlog".to_string(),
            tab_id: 0,
            document_id: "gitlog-diff".to_string(),
            revision: 0,
            request_id: None,
            payload: EditorEvent::Ready {
                read_only: true,
                language: "rust".to_string(),
            },
        };
        assert!(env.validate(&b).is_ok());
    }

    #[test]
    fn protocol_error_is_displayable() {
        assert!(!ProtocolError::BadJson("x".into()).to_string().is_empty());
    }

    #[test]
    fn dispatch_script_escapes_envelope_and_is_defensive() {
        let js = dispatch_script(r#"{"a":"b\"c"}"#);
        assert!(
            js.starts_with("window.__dozer&&window.__dozer.dispatch&&window.__dozer.dispatch(")
        );
        assert!(js.ends_with(");"));
        // 内层 JSON 字符串被转义成一个合法 JS 字符串字面量。
        let start = js.find('(').unwrap() + 1;
        let end = js.rfind(')').unwrap();
        let literal = &js[start..end];
        let parsed: serde_json::Value = serde_json::from_str(literal).unwrap();
        assert_eq!(parsed, serde_json::json!(r#"{"a":"b\"c"}"#));
    }

    /// T9:flyfish envelope 解析 + 归属校验(版本/项目/面板/tab/文档)。
    #[test]
    fn parses_and_validates_flyfish_events() {
        let raw = r#"{"protocol_version":1,"project_id":7,"panel":"files","tab_id":3,"document_id":"p7-t3","revision":0,"request_id":null,"payload":{"kind":"failed","message":"boom","recoverable":true}}"#;
        let env = parse_flyfish_event(raw).unwrap();
        assert_eq!(
            env.payload,
            FlyfishEvent::Failed {
                message: "boom".into(),
                recoverable: true
            }
        );
        let good = HostBinding::new(7, PanelKind::Files, 3, "p7-t3".into());
        assert!(env.validate(&good).is_ok());
        // 自报别的 project/tab/doc 一律拒绝。
        assert!(
            env.validate(&HostBinding::new(8, PanelKind::Files, 3, "p7-t3".into()))
                .is_err()
        );
        assert!(
            env.validate(&HostBinding::new(7, PanelKind::Project, 3, "p7-t3".into()))
                .is_err()
        );
        assert!(
            env.validate(&HostBinding::new(7, PanelKind::Files, 4, "p7-t4".into()))
                .is_err()
        );

        // 其它 kind。
        for (raw, want) in [
            (r#"{"kind":"ready"}"#, FlyfishEvent::Ready),
            (
                r#"{"kind":"title","title":"hi"}"#,
                FlyfishEvent::Title { title: "hi".into() },
            ),
            (
                r#"{"kind":"search_state","current":1,"total":9}"#,
                FlyfishEvent::SearchState {
                    current: 1,
                    total: 9,
                },
            ),
        ] {
            let full = format!(
                r#"{{"protocol_version":1,"project_id":1,"panel":"files","tab_id":1,"document_id":"d","payload":{raw}}}"#
            );
            assert_eq!(parse_flyfish_event(&full).unwrap().payload, want);
        }

        // 未知 kind / 非 JSON 报错且不 panic。
        let bad = r#"{"protocol_version":1,"payload":{"kind":"nope"}}"#;
        assert!(matches!(
            parse_flyfish_event(bad),
            Err(ProtocolError::UnknownPayload(_))
        ));
        assert!(parse_flyfish_event("not json").is_err());
    }
}
