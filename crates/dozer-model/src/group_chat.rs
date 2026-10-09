use crate::AgentKind;
use serde::{Deserialize, Serialize};

/// 群聊成员(spec 2026-10-02-group-chat-panel-design §5)。同一种 agent 可
/// 多次入群扮演不同角色,`handle` 在群内唯一(忽略大小写)。第一版 `agent`
/// 只会是 `Claude`/`Codex`,由 `dozerd::group_store` 校验。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroupMemberInfo {
    pub id: i64,
    pub group_id: i64,
    pub agent: AgentKind,
    pub handle: String,
    pub role_prompt: String,
}

/// 一个群聊。`topic` 既是列表里的显示名,也是每次发言提示词里的"讨论目标"。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroupInfo {
    pub id: i64,
    pub project_id: i64,
    pub topic: String,
    pub created_ms: u64,
    pub members: Vec<GroupMemberInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GroupAuthor {
    Human,
    Member { member_id: i64 },
    System,
}

/// agent 消息的状态机:`Queued → Running → Done|Failed|Cancelled`;`Failed`/
/// `Cancelled` 可经重试回到 `Queued`。human/系统消息没有状态(`None`)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum GroupMessageStatus {
    Queued,
    Running,
    Done,
    Failed { reason: String },
    Cancelled,
}

/// 群消息。`seq` 是群内展示顺序(创建时定,不变);`rev` 是群内变更序号
/// (插入或状态/正文变化都会取新的更大值),GUI 用它做增量轮询。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroupMessageInfo {
    pub id: i64,
    pub group_id: i64,
    pub seq: i64,
    pub rev: i64,
    pub author: GroupAuthor,
    pub text: String,
    /// human 消息里被点名的成员 id(按首次出现顺序);其余消息为空。
    pub mentions: Vec<i64>,
    pub status: Option<GroupMessageStatus>,
    pub duration_ms: Option<u64>,
    pub created_ms: u64,
    /// 已转成的待办 id(只在消息侧记录,Todo 表不加反向字段)。
    pub todo_id: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupCancelScope {
    /// 只停当前正在发言的那一位。
    Turn,
    /// 停当前发言并清掉本群排队中的全部发言。
    Round,
}
