use serde::{Deserialize, Serialize};

/// Agent 上下文列表的一项(v0.1 意向文档 §10 Context Scope)。`entity_kind`
/// 本期取值 `"file"`/`"dir"`,日后要纳入 todo/会话/ssh/数据库时直接新增取值,
/// 不需要迁移表结构,所以传输与落库都用字符串而非枚举;读到不认识的取值时
/// 调用方应原样保留、显示为"未知类型"。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextItemInfo {
    pub id: i64,
    pub project_id: i64,
    pub entity_kind: String,
    /// 本期是项目内相对路径,目录不带尾部 `/`。
    pub entity_ref: String,
    pub created_ms: u64,
}

/// `file_edit_history` 一行,给 GUI 的修改历史弹窗用。`start_*`/`end_*` 是
/// **修改前**的坐标(1-based,结束坐标是最后一个字符之后的位置);
/// `new_end_*` 是**修改后** `new_text` 的结束坐标(起点不变),由 dozerd
/// 在查询时按 `new_text` 推算,Revert 反向调用 `apply_precise_edit` 需要它。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileEditHistoryInfo {
    pub id: i64,
    pub project_id: i64,
    pub target_path: String,
    pub actor: String,
    pub session_id: String,
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
    pub new_end_line: u32,
    pub new_end_col: u32,
    pub old_text: String,
    pub new_text: String,
    pub summary: String,
    pub created_ms: u64,
}

/// 收藏夹范围:全局(跨项目共享)或挂靠某个项目(`project_id` 必填)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BookmarkScope {
    Global,
    Project,
}

/// 一条收藏记录。`project_id`:`scope=Global` 时恒为 `None`,
/// `scope=Project` 时是该项目在 `projects` 表里的 `id`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BookmarkInfo {
    pub id: i64,
    pub scope: BookmarkScope,
    pub project_id: Option<i64>,
    pub url: String,
    pub title: String,
    pub created_ms: u64,
}

/// 一次代码健康度扫描的落盘结果。`report_json` 是
/// `dozer_codehealth::ProjectReport` 的完整序列化（含 `functions` 明细列表），
/// 顶部几个标量字段冗余存一份是为了 dozerd 侧不需要反序列化整个 JSON
/// 就能回答"这个项目健康度是什么档位"这类粗粒度查询（目前没有这类查询，
/// 但同 `session_summaries` 表"标量列 + 大文本列"并存的既有设计一致，
/// 不额外增加复杂度）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodeHealthReportInfo {
    pub total_loc: u64,
    pub total_functions: u64,
    pub critical_functions: u64,
    pub overall_tier: String,
    pub report_json: String,
    pub scanned_at_ms: u64,
    /// 报告 schema 版本（`dozer_codehealth::SCHEMA_VERSION`）。旧协议帧缺字段
    /// 回落 0，UI 据此判定“旧版报告”。
    #[serde(default)]
    pub schema_version: u32,
    /// 扫描时的 Git HEAD。旧协议帧缺字段回落 `None`。
    #[serde(default)]
    pub git_head: Option<String>,
    /// 扫描时的 Git 分支。旧协议帧缺字段回落 `None`。
    #[serde(default)]
    pub git_branch: Option<String>,
    /// 扫描时工作区是否 dirty。旧协议帧缺字段回落 `false`。
    #[serde(default)]
    pub git_dirty: bool,
}
