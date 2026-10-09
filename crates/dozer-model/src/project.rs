use serde::{Deserialize, Serialize};

/// 项目（甲方资产域的根；P1g）。id 为 dozerd SQLite 主键。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectInfo {
    pub id: i64,
    pub path: String,
    pub name: String,
    /// 用户在 Dozer 里最后一次打开/激活该项目的时间（严格单调活跃戳，
    /// 见 `dozerd::projects`）。仅用于排序/判定"最近用过"，不再作为列表
    /// 展示的更新时间。
    pub last_active_ms: u64,
    /// 创建时间：项目在 Dozer 中首次被新建（首次 `open`）时的时间。
    pub created_ms: u64,
    /// 更新时间（git 感知）：优先取项目仓库最新 commit 时间；若该仓库没有
    /// commit，或 commit 时间早于 `last_active_ms`，则回落为 `last_active_ms`。
    /// 由 dozerd 在 `list()` 时实时计算（不入库），保证每次打开或新提交后
    /// 都能拿到最新值。
    pub updated_ms: u64,
}

/// 一条共享记忆(`dozerd` 的 `memories` 表一行)。`(project_id, title)`
/// 唯一,`write_memory` 按这个键做 upsert(2026-09-23 agent 记忆共享设计)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryInfo {
    pub id: i64,
    pub project_id: i64,
    pub title: String,
    /// 自由字符串,不做枚举约束;默认建议集合 `user`/`feedback`/`project`/
    /// `reference`,但其他 agent 塞别的字符串也不报错。
    pub kind: String,
    pub description: String,
    pub updated_ms: u64,
    /// `AgentKind::as_str()` 的结果,或人工操作时的字面量 `"user"`。
    pub updated_by: String,
}

/// `GetMemory`/`WriteMemory` 应答里的完整记录,比 `MemoryInfo` 多正文和
/// 最近历史。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryDetail {
    pub id: i64,
    pub project_id: i64,
    pub title: String,
    pub kind: String,
    pub description: String,
    pub body: String,
    pub created_ms: u64,
    pub created_by: String,
    pub updated_ms: u64,
    pub updated_by: String,
    /// `write`/`delete` 返回的 `MemoryDetail` 里这个字段恒为空(写操作不
    /// 顺带查历史);要看历史需要单独调 `MemoryStore::history`/走
    /// `GetMemory` 请求。
    pub history: Vec<MemoryHistoryEntry>,
}

/// `memory_history` 表一行:某次改动后的完整快照。`change_kind` 是
/// `"created"`/`"updated"`/`"deleted"` 三选一字符串(不用 Rust enum 是因为
/// 这条记录被删除后仍要能反序列化出历史,不希望未来加新 kind 时旧数据的
/// enum 解不出来——字符串更宽容)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryHistoryEntry {
    pub id: i64,
    pub memory_id: i64,
    pub changed_ms: u64,
    pub changed_by: String,
    pub change_kind: String,
    pub title: String,
    pub kind: String,
    pub description: String,
    pub body: String,
}
