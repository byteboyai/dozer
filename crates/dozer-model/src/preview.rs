use serde::{Deserialize, Serialize};

/// `PreviewContext.selected_text` 的上限(字符数)。选区可能很大,上下文不
/// 默认发送整文件;超出即截断并保留头部。
pub const PREVIEW_SELECTED_TEXT_MAX_CHARS: usize = 4096;

/// 预览面板当前上下文：文件路径 + 光标/选区（1-indexed，见 spec
/// "1-indexed 行列" 一节）。无选区时 `start == end` 为光标位置。
///
/// Phase B 起新增 `revision`/`mode`/`read_only`/`selected_text`/可见行范围等
/// 可选字段;**全部带 serde 默认值**,旧客户端/旧 JSON 仍可解码(缺字段取默认)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PreviewContext {
    pub path: String,
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
    pub has_selection: bool,
    /// 这份上下文被推送时的 Unix 纪元毫秒。没有它的话，一个陈旧的缓存值
    /// （GUI 已退出但 dozerd 还活着、或用户几小时没碰过预览面板）和刚刚
    /// 更新的值长得一模一样，调 MCP tool 的 agent 无从判断新鲜度。
    pub updated_at_ms: u64,
    /// 编辑器文档 revision(CodeMirror host 的变更计数;非文本后端为 0)。
    #[serde(default)]
    pub revision: u64,
    /// 预览 mode(`code`/`rendered`/`source`/`tree`/`text`/`streamed`/...)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// 当前后端是否只读。
    #[serde(default)]
    pub read_only: bool,
    /// 选区文本(有上限,见 [`PREVIEW_SELECTED_TEXT_MAX_CHARS`]);无选区为 `None`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_text: Option<String>,
    /// 可见起始行(1-based);后端不提供可见范围时为 `None`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible_start_line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible_end_line: Option<u32>,
    /// 表格(Tabular)预览的上下文:当前 sheet 与逻辑滚动锚点,供 Agent 感知
    /// 用户在看哪张表、哪一段。非表格后端为 `None`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tabular: Option<PreviewTabularContext>,
}

/// 表格预览上下文(Phase D Task 6 + T12):当前 sheet、逻辑滚动锚点、选中
/// 单元格/范围与可见行列。新增字段均带 serde 默认,旧客户端可解码。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PreviewTabularContext {
    pub sheet: usize,
    pub scroll_row: u32,
    pub scroll_col: u32,
    /// 可见数据行区间(0-based,含端点);未知为 `None`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible_rows: Option<(u32, u32)>,
    /// 可见数据列区间(0-based,含端点);未知为 `None`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible_cols: Option<(u32, u32)>,
    /// 选中的单个单元格(0-based row/col);无选中为 `None`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_cell: Option<(u32, u32)>,
    /// 选中范围(0-based,含端点 `(r1,c1,r2,c2)`);无选中为 `None`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_range: Option<(u32, u32, u32, u32)>,
}

impl PreviewTabularContext {
    /// 从选中范围推导单元格选中(r1==r2 && c1==c2 时为单元格)。
    pub fn selected_cell_from_range(&self) -> Option<(u32, u32)> {
        self.selected_range
            .filter(|(r1, c1, r2, c2)| r1 == r2 && c1 == c2)
            .map(|(r1, c1, _, _)| (r1, c1))
    }
}

/// T13:一条下行预览命令的等待上限(毫秒)。超时后调用方收到
/// `PreviewCommandOutcome::Timeout`,不无限等待。
pub const PREVIEW_COMMAND_TIMEOUT_MS: u64 = 2000;

/// T13:daemon → app 预览命令的目标定位。先按 tab(含面板),否则按项目内路径。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "by", rename_all = "snake_case")]
pub enum PreviewCommandTarget {
    /// 按 tab 定位;`panel` 为 `"files"` / `"project"`。
    Tab { panel: String, tab_id: usize },
    /// 按项目内文件路径定位(取该路径当前激活的 tab)。
    Path { path: String },
}

/// T13:预览命令动作。reveal/select/reveal_cell 是只读导航(不受写权限阻塞);
/// replace 仅支持可写 CodeMirror 且必须带 `expected_revision`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PreviewCommandAction {
    /// 滚动并定位光标(1-based)。
    Reveal { line: u32, column: u32 },
    /// 选中范围(1-based,含端点)。
    Select {
        start_line: u32,
        start_column: u32,
        end_line: u32,
        end_column: u32,
    },
    /// 表格单元格 reveal(0-based sheet/row/col)。
    RevealCell { sheet: usize, row: u32, col: u32 },
    /// 替换范围(仅可写 CodeMirror;必须带 `expected_revision`)。
    Replace {
        start_line: u32,
        start_column: u32,
        end_line: u32,
        end_column: u32,
        text: String,
    },
    /// 短暂高亮某段范围(纯视觉装饰,定时自动消退;不改变实际的文本选区/光标
    /// 状态——故意跟 `Select` 分开,`Select` 会占用 human 自己后续操作会用到
    /// 的"当前选中范围",高亮不应该有这个副作用)。
    Highlight {
        start_line: u32,
        start_column: u32,
        end_line: u32,
        end_column: u32,
        duration_ms: u32,
    },
}

/// T13:一条下行预览命令。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PreviewCommand {
    pub request_id: String,
    pub project_id: i64,
    pub target: PreviewCommandTarget,
    pub action: PreviewCommandAction,
    /// replace 必填(期望的文档 revision);其余动作忽略。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<u64>,
}

/// T13:命令结果(确定终态,区分所有失败类别,不无限等待)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum PreviewCommandOutcome {
    /// 已接受并执行(导航立即生效;replace 已落 editor buffer 待保存)。
    Accepted { request_id: String },
    /// 目标 tab / 文件不存在(或已关闭)。
    NotFound { request_id: String, detail: String },
    /// replace 的 `expected_revision` 与当前不一致;返回当前 revision。
    StaleRevision {
        request_id: String,
        current_revision: u64,
    },
    /// 目标 backend 不支持该动作(如对图片 reveal、对只读 tab replace)。
    UnsupportedBackend { request_id: String, detail: String },
    /// 资源预算拒绝加载(suspended 唤醒失败)。
    LoadDenied { request_id: String, detail: String },
    /// app 不在线 / 处理超时(由 daemon 判定)。
    Timeout { request_id: String },
    /// 其它内部错误。
    InternalError { request_id: String, detail: String },
}

impl PreviewCommandOutcome {
    pub fn request_id(&self) -> &str {
        match self {
            Self::Accepted { request_id }
            | Self::NotFound { request_id, .. }
            | Self::StaleRevision { request_id, .. }
            | Self::UnsupportedBackend { request_id, .. }
            | Self::LoadDenied { request_id, .. }
            | Self::Timeout { request_id }
            | Self::InternalError { request_id, .. } => request_id,
        }
    }
}

impl PreviewContext {
    /// 截断到上限并写入选区文本(空串视为无选区)。
    pub fn set_selected_text(&mut self, text: Option<String>) {
        self.selected_text = text.and_then(|t| {
            if t.is_empty() {
                return None;
            }
            let mut out = String::new();
            for (i, ch) in t.chars().enumerate() {
                if i >= PREVIEW_SELECTED_TEXT_MAX_CHARS {
                    break;
                }
                out.push(ch);
            }
            Some(out)
        });
    }
}

/// `LocateInFile` 的一处匹配:坐标(1-based)+ 命中处附近的上下文,供 agent
/// 判断是不是自己想要的位置、或缩小 `query` 范围。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocateMatch {
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
    pub context: String,
}

/// `ApplyPreciseEdit` 的结果。`Applied` 里的 `new_*` 坐标是替换后
/// `new_text` 对应的新区间(供自动定位/高亮使用),`history_id` 是这次修改
/// 写入 `file_edit_history` 表的行 id。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MutationOutcome {
    Applied {
        new_start_line: u32,
        new_start_col: u32,
        new_end_line: u32,
        new_end_col: u32,
        history_id: i64,
    },
    /// Conflict Detection:磁盘当前内容跟调用方传的 `expected_text` 不一致。
    Conflict {
        actual_text: String,
    },
    NotFound,
    PathOutOfBounds,
    /// 二进制/非 UTF-8 等不可写的文件。
    Unwritable {
        reason: String,
    },
}
