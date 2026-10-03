//! 群聊面板 webview 推送协议:把 `WorkspaceState` 算成要序列化推给 webview 的
//! `GroupChatViewPayload`,并解析、校验 webview 发回的事件。前端只渲染与保存纯视图
//! 状态(草稿、对话框开关、滚动),**不做任何业务判断**——作者显示名、成员颜色
//! 槽位、状态映射、提示文案都在这里算好。形态仿 `extensions/todo/protocol.rs`
//! (声明式比较 + 固定单槽)。

use super::WorkspaceState;
use dozer_core::protocol::{
    AgentKind, GroupAuthor, GroupCancelScope, GroupInfo, GroupMessageStatus,
};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

pub const GROUP_CHAT_PROTOCOL_VERSION: u32 = 1;

/// webview 加载后超过这个时长还没发 `ready` 就判失败,回落原生占位页。
pub const READY_TIMEOUT: Duration = Duration::from_secs(10);

/// 来自 webview 的文本长度上限(字符数)。webview 内容不可信,防止异常超长文本
/// 落库/进提示词。`MAX_POST_CHARS` 与后端 `group_service::MAX_POST_CHARS` 对齐。
pub const MAX_TOPIC_CHARS: usize = 200;
pub const MAX_ROLE_CHARS: usize = 2_000;
pub const MAX_POST_CHARS: usize = 20_000;
pub const MAX_TODO_CHARS: usize = 10_000;
const MAX_HANDLE_CHARS: usize = 32;

/// 成员颜色槽位数(前端映射到 4 个 CSS 变量)。
const COLOR_SLOTS: usize = 4;

// ---------------------------------------------------------------------------
// Rust → webview
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StatusKey {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuthorKind {
    Human,
    Member,
    System,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AuthorVm {
    pub kind: AuthorKind,
    /// 显示名:"你" / "@handle" / "已移除成员" / "系统"。
    pub label: String,
    /// "Claude"/"Codex",成员消息才有。
    pub agent_label: Option<String>,
    /// 成员颜色槽位(0..COLOR_SLOTS),已移除成员/非成员为 `None`。
    pub color_slot: Option<u8>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct MemberVm {
    pub id: i64,
    pub agent: AgentKind,
    pub agent_label: &'static str,
    pub handle: String,
    pub role_prompt: String,
    pub color_slot: u8,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct GroupVm {
    pub id: i64,
    pub topic: String,
    pub members: Vec<MemberVm>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct MessageVm {
    pub id: i64,
    pub seq: i64,
    pub author: AuthorVm,
    pub text: String,
    pub status: Option<StatusKey>,
    /// 失败原因(`status == Failed` 时)。
    pub reason: Option<String>,
    pub duration_ms: Option<u64>,
    pub todo_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct GroupChatViewPayload {
    pub project_id: i64,
    /// `false` = 还没从 dozerd 加载过(前端显示"加载中"而不是"还没有群聊")。
    pub loaded: bool,
    pub groups: Vec<GroupVm>,
    pub selected_group_id: Option<i64>,
    /// 当前选中群的消息,按 `seq` 升序。
    pub messages: Vec<MessageVm>,
    /// 一次性发送提示("无人被点名…"/"未找到成员 @x"),下次发送/切群时清除。
    pub hint: Option<String>,
    /// 本群有 `Queued`/`Running` 的发言(输入框旁显示"停止本轮")。
    pub running: bool,
}

fn color_slot_of(group: &GroupInfo, member_id: i64) -> Option<u8> {
    group
        .members
        .iter()
        .position(|m| m.id == member_id)
        .map(|i| (i % COLOR_SLOTS) as u8)
}

fn status_of(status: &GroupMessageStatus) -> StatusKey {
    match status {
        GroupMessageStatus::Queued => StatusKey::Queued,
        GroupMessageStatus::Running => StatusKey::Running,
        GroupMessageStatus::Done => StatusKey::Done,
        GroupMessageStatus::Failed { .. } => StatusKey::Failed,
        GroupMessageStatus::Cancelled => StatusKey::Cancelled,
    }
}

pub fn current_view_payload(state: &WorkspaceState, project_id: i64) -> GroupChatViewPayload {
    let groups: Vec<GroupVm> = state
        .groups()
        .iter()
        .map(|g| GroupVm {
            id: g.id,
            topic: g.topic.clone(),
            members: g
                .members
                .iter()
                .enumerate()
                .map(|(i, m)| MemberVm {
                    id: m.id,
                    agent: m.agent,
                    agent_label: m.agent.display_label(),
                    handle: m.handle.clone(),
                    role_prompt: m.role_prompt.clone(),
                    color_slot: (i % COLOR_SLOTS) as u8,
                })
                .collect(),
        })
        .collect();

    let selected_group = state
        .selected()
        .and_then(|id| state.groups().iter().find(|g| g.id == id));

    let messages = state
        .messages()
        .iter()
        .map(|m| {
            let author = match &m.author {
                GroupAuthor::Human => AuthorVm {
                    kind: AuthorKind::Human,
                    label: "你".to_string(),
                    agent_label: None,
                    color_slot: None,
                },
                GroupAuthor::System => AuthorVm {
                    kind: AuthorKind::System,
                    label: "系统".to_string(),
                    agent_label: None,
                    color_slot: None,
                },
                GroupAuthor::Member { member_id } => {
                    let found =
                        selected_group.and_then(|g| g.members.iter().find(|x| x.id == *member_id));
                    match found {
                        Some(mem) => AuthorVm {
                            kind: AuthorKind::Member,
                            label: format!("@{}", mem.handle),
                            agent_label: Some(mem.agent.display_label().to_string()),
                            color_slot: selected_group.and_then(|g| color_slot_of(g, mem.id)),
                        },
                        None => AuthorVm {
                            kind: AuthorKind::Member,
                            label: "已移除成员".to_string(),
                            agent_label: None,
                            color_slot: None,
                        },
                    }
                }
            };
            MessageVm {
                id: m.id,
                seq: m.seq,
                author,
                text: m.text.clone(),
                status: m.status.as_ref().map(status_of),
                reason: match &m.status {
                    Some(GroupMessageStatus::Failed { reason }) => Some(reason.clone()),
                    _ => None,
                },
                duration_ms: m.duration_ms,
                todo_id: m.todo_id,
            }
        })
        .collect();

    GroupChatViewPayload {
        project_id,
        loaded: state.loaded(),
        groups,
        selected_group_id: state.selected(),
        messages,
        hint: state.hint().map(str::to_string),
        running: state.has_active_turn(),
    }
}

#[derive(Debug, Serialize)]
struct GroupChatPushEnvelope {
    protocol_version: u32,
    revision: u64,
    payload: GroupChatViewPayload,
}

pub fn encode_group_chat_push(revision: u64, payload: GroupChatViewPayload) -> String {
    serde_json::to_string(&GroupChatPushEnvelope {
        protocol_version: GROUP_CHAT_PROTOCOL_VERSION,
        revision,
        payload,
    })
    .unwrap_or_else(|_| "null".to_string())
}

// ---------------------------------------------------------------------------
// webview → Rust
// ---------------------------------------------------------------------------

/// webview 发回的事件。内容不可信:长度、id 存在性、agent 种类都在 `route_event`
/// 里校验。
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GroupChatWebviewEvent {
    Ready,
    Failed {
        reason: String,
    },
    CreateGroup {
        topic: String,
    },
    SelectGroup {
        group_id: i64,
    },
    DeleteGroup {
        group_id: i64,
    },
    AddMember {
        group_id: i64,
        agent: AgentKind,
        handle: String,
        role_prompt: String,
    },
    UpdateMember {
        member_id: i64,
        handle: String,
        role_prompt: String,
    },
    RemoveMember {
        member_id: i64,
    },
    Post {
        group_id: i64,
        text: String,
    },
    Cancel {
        group_id: i64,
        scope: GroupCancelScope,
    },
    Retry {
        message_id: i64,
    },
    PushTodo {
        message_id: i64,
        text: String,
    },
    OpenTodo {
        todo_id: i64,
    },
}

pub fn parse_group_chat_event(body: &str) -> Result<GroupChatWebviewEvent, String> {
    serde_json::from_str(body).map_err(|e| e.to_string())
}

/// 校验后交给 `App` 执行的命令。
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    CreateGroup {
        topic: String,
    },
    SelectGroup {
        group_id: i64,
    },
    DeleteGroup {
        group_id: i64,
    },
    AddMember {
        group_id: i64,
        agent: AgentKind,
        handle: String,
        role_prompt: String,
    },
    UpdateMember {
        member_id: i64,
        handle: String,
        role_prompt: String,
    },
    RemoveMember {
        member_id: i64,
    },
    Post {
        group_id: i64,
        text: String,
    },
    Cancel {
        group_id: i64,
        scope: GroupCancelScope,
    },
    Retry {
        message_id: i64,
    },
    PushTodo {
        group_id: i64,
        message_id: i64,
        text: String,
    },
    OpenTodo {
        todo_id: i64,
    },
}

fn clean(text: &str, max_chars: usize) -> Option<String> {
    let t = text.trim();
    (!t.is_empty() && t.chars().count() <= max_chars).then(|| t.to_string())
}

fn group_exists(state: &WorkspaceState, id: i64) -> bool {
    state.groups().iter().any(|g| g.id == id)
}

fn member_exists(state: &WorkspaceState, id: i64) -> bool {
    state
        .groups()
        .iter()
        .any(|g| g.members.iter().any(|m| m.id == id))
}

/// 校验 webview 事件并映射成命令。`id` 已不存在、入参非法一律返回 `None`
/// (空操作),绝不能落到别的对象上。`Ready`/`Failed` 由 `App` 自己处理,这里
/// 返回 `None`。
pub(crate) fn route_event(state: &WorkspaceState, event: GroupChatWebviewEvent) -> Option<Command> {
    use GroupChatWebviewEvent as Ev;
    match event {
        Ev::Ready | Ev::Failed { .. } => None,
        Ev::CreateGroup { topic } => Some(Command::CreateGroup {
            topic: clean(&topic, MAX_TOPIC_CHARS)?,
        }),
        Ev::SelectGroup { group_id } => {
            group_exists(state, group_id).then_some(Command::SelectGroup { group_id })
        }
        Ev::DeleteGroup { group_id } => {
            group_exists(state, group_id).then_some(Command::DeleteGroup { group_id })
        }
        Ev::AddMember {
            group_id,
            agent,
            handle,
            role_prompt,
        } => {
            if !group_exists(state, group_id)
                || !matches!(agent, AgentKind::Claude | AgentKind::Codex)
            {
                return None;
            }
            Some(Command::AddMember {
                group_id,
                agent,
                handle: clean(&handle, MAX_HANDLE_CHARS)?,
                role_prompt: role_prompt_of(&role_prompt)?,
            })
        }
        Ev::UpdateMember {
            member_id,
            handle,
            role_prompt,
        } => {
            if !member_exists(state, member_id) {
                return None;
            }
            Some(Command::UpdateMember {
                member_id,
                handle: clean(&handle, MAX_HANDLE_CHARS)?,
                role_prompt: role_prompt_of(&role_prompt)?,
            })
        }
        Ev::RemoveMember { member_id } => {
            member_exists(state, member_id).then_some(Command::RemoveMember { member_id })
        }
        Ev::Post { group_id, text } => {
            if state.selected() != Some(group_id) {
                return None;
            }
            Some(Command::Post {
                group_id,
                text: clean(&text, MAX_POST_CHARS)?,
            })
        }
        Ev::Cancel { group_id, scope } => {
            (state.selected() == Some(group_id)).then_some(Command::Cancel { group_id, scope })
        }
        Ev::Retry { message_id } => {
            let m = state.messages().iter().find(|m| m.id == message_id)?;
            matches!(
                m.status,
                Some(GroupMessageStatus::Failed { .. }) | Some(GroupMessageStatus::Cancelled)
            )
            .then_some(Command::Retry { message_id })
        }
        Ev::PushTodo { message_id, text } => {
            let m = state.messages().iter().find(|m| m.id == message_id)?;
            // 一条消息只转一次;进行中/排队中的 agent 消息还没有正文。
            if m.todo_id.is_some()
                || matches!(
                    m.status,
                    Some(GroupMessageStatus::Queued) | Some(GroupMessageStatus::Running)
                )
            {
                return None;
            }
            Some(Command::PushTodo {
                // 消息列表只含当前选中群的消息,所以所属群就是当前选中群。
                group_id: state.selected()?,
                message_id,
                text: clean(&text, MAX_TODO_CHARS)?,
            })
        }
        Ev::OpenTodo { todo_id } => (todo_id > 0).then_some(Command::OpenTodo { todo_id }),
    }
}

/// 角色设定允许为空(默认无角色);非空则按长度上限截检,超限整个事件作废。
fn role_prompt_of(text: &str) -> Option<String> {
    let t = text.trim();
    (t.chars().count() <= MAX_ROLE_CHARS).then(|| t.to_string())
}

// ---------------------------------------------------------------------------
// 声明式推送状态(同 todo::WebviewPushState)
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct WebviewPushState {
    ready: bool,
    last_sent: Option<GroupChatViewPayload>,
    revision: u64,
    /// webview 已进池但尚未 ready 的起点,用于判定加载超时。
    waiting_since: Option<Instant>,
    /// 加载失败/超时原因;`Some` 时内容区回落原生占位页(重试清除)。
    failed: Option<String>,
}

impl WebviewPushState {
    pub fn set_ready(&mut self, ready: bool) {
        self.ready = ready;
        if ready {
            // 新实例(或重新确认 ready)一律强制重发当前内容。
            self.last_sent = None;
            self.waiting_since = None;
        }
    }

    /// 每帧由消费点调用:`available` = webview 当前在池里。
    pub fn observe_availability(&mut self, available: bool, now: Instant) {
        if !available {
            self.ready = false;
            self.waiting_since = None;
            return;
        }
        if self.ready {
            self.waiting_since = None;
            return;
        }
        let since = *self.waiting_since.get_or_insert(now);
        if self.failed.is_none() && now.duration_since(since) > READY_TIMEOUT {
            self.failed = Some("面板页面加载超时".to_string());
        }
    }

    pub fn pending_push(&self, desired: &GroupChatViewPayload) -> Option<GroupChatViewPayload> {
        if !self.ready || self.last_sent.as_ref() == Some(desired) {
            return None;
        }
        Some(desired.clone())
    }

    /// 记录已送达并返回本次 revision(从 1 起)。
    pub fn mark_sent(&mut self, payload: GroupChatViewPayload) -> u64 {
        self.revision += 1;
        self.last_sent = Some(payload);
        self.revision
    }

    pub fn failed(&self) -> Option<&str> {
        self.failed.as_deref()
    }

    pub fn set_failed(&mut self, reason: String) {
        self.failed = Some(reason);
        self.ready = false;
        self.last_sent = None;
        self.waiting_since = None;
    }

    pub fn clear_failed(&mut self) {
        self.failed = None;
        self.waiting_since = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::group_chat::{Message, update};
    use dozer_core::protocol::{
        AgentKind, GroupAuthor, GroupCancelScope, GroupInfo, GroupMemberInfo, GroupMessageInfo,
        GroupMessageStatus,
    };
    use serde_json::json;

    fn member(id: i64, agent: AgentKind, handle: &str) -> GroupMemberInfo {
        GroupMemberInfo {
            id,
            group_id: 1,
            agent,
            handle: handle.into(),
            role_prompt: "角色".into(),
        }
    }

    fn group() -> GroupInfo {
        GroupInfo {
            id: 1,
            project_id: 7,
            topic: "评审登录方案".into(),
            created_ms: 0,
            members: vec![
                member(10, AgentKind::Claude, "架构师"),
                member(11, AgentKind::Codex, "审阅者"),
            ],
        }
    }

    fn msg(
        id: i64,
        seq: i64,
        author: GroupAuthor,
        status: Option<GroupMessageStatus>,
    ) -> GroupMessageInfo {
        GroupMessageInfo {
            id,
            group_id: 1,
            seq,
            rev: seq,
            author,
            text: format!("t{id}"),
            mentions: vec![],
            status,
            duration_ms: Some(1500),
            created_ms: 0,
            todo_id: None,
        }
    }

    fn state_with_messages(msgs: Vec<GroupMessageInfo>) -> WorkspaceState {
        let mut s = WorkspaceState::default();
        update(&mut s, Message::GroupsLoaded(7, Ok(vec![group()])));
        update(
            &mut s,
            Message::Polled {
                project_id: 7,
                group_id: 1,
                result: Ok((msgs, 99)),
            },
        );
        s
    }

    // ---- payload ----

    #[test]
    fn payload_for_unloaded_state_is_empty_but_marks_not_loaded() {
        let p = current_view_payload(&WorkspaceState::default(), 7);
        assert!(!p.loaded);
        assert!(p.groups.is_empty());
        assert_eq!(p.selected_group_id, None);
        assert!(!p.running);
    }

    #[test]
    fn payload_resolves_author_labels_agents_and_color_slots() {
        let s = state_with_messages(vec![
            msg(1, 1, GroupAuthor::Human, None),
            msg(
                2,
                2,
                GroupAuthor::Member { member_id: 10 },
                Some(GroupMessageStatus::Done),
            ),
            msg(
                3,
                3,
                GroupAuthor::Member { member_id: 11 },
                Some(GroupMessageStatus::Done),
            ),
            msg(
                4,
                4,
                GroupAuthor::Member { member_id: 99 },
                Some(GroupMessageStatus::Done),
            ),
        ]);
        let p = current_view_payload(&s, 7);
        assert_eq!(p.messages[0].author.kind, AuthorKind::Human);
        assert_eq!(p.messages[0].author.label, "你");
        assert_eq!(p.messages[1].author.label, "@架构师");
        assert_eq!(p.messages[1].author.agent_label.as_deref(), Some("Claude"));
        assert_eq!(p.messages[1].author.color_slot, Some(0));
        assert_eq!(p.messages[2].author.agent_label.as_deref(), Some("Codex"));
        assert_eq!(p.messages[2].author.color_slot, Some(1));
        assert_eq!(p.messages[3].author.label, "已移除成员");
        assert_eq!(p.messages[3].author.color_slot, None);
    }

    #[test]
    fn payload_maps_statuses_and_failure_reason() {
        let s = state_with_messages(vec![
            msg(
                1,
                1,
                GroupAuthor::Member { member_id: 10 },
                Some(GroupMessageStatus::Queued),
            ),
            msg(
                2,
                2,
                GroupAuthor::Member { member_id: 10 },
                Some(GroupMessageStatus::Running),
            ),
            msg(
                3,
                3,
                GroupAuthor::Member { member_id: 10 },
                Some(GroupMessageStatus::Failed {
                    reason: "发言超时".into(),
                }),
            ),
            msg(
                4,
                4,
                GroupAuthor::Member { member_id: 10 },
                Some(GroupMessageStatus::Cancelled),
            ),
            msg(5, 5, GroupAuthor::Human, None),
        ]);
        let p = current_view_payload(&s, 7);
        let states: Vec<_> = p.messages.iter().map(|m| m.status).collect();
        assert_eq!(
            states,
            vec![
                Some(StatusKey::Queued),
                Some(StatusKey::Running),
                Some(StatusKey::Failed),
                Some(StatusKey::Cancelled),
                None
            ]
        );
        assert_eq!(p.messages[2].reason.as_deref(), Some("发言超时"));
        assert!(p.running, "有 Queued/Running → running");
    }

    #[test]
    fn payload_members_carry_agent_label_and_role() {
        let s = state_with_messages(vec![]);
        let p = current_view_payload(&s, 7);
        let g = &p.groups[0];
        assert_eq!(g.topic, "评审登录方案");
        assert_eq!(g.members[0].agent, AgentKind::Claude);
        assert_eq!(g.members[0].agent_label, "Claude");
        assert_eq!(g.members[1].agent_label, "Codex");
        assert_eq!(g.members[1].role_prompt, "角色");
        assert_eq!(p.selected_group_id, Some(1));
    }

    #[test]
    fn payload_serializes_with_snake_case_keys_and_roundtrips_equality() {
        let s = state_with_messages(vec![msg(1, 1, GroupAuthor::Human, None)]);
        let p = current_view_payload(&s, 7);
        let v: serde_json::Value =
            serde_json::from_str(&encode_group_chat_push(3, p.clone())).unwrap();
        assert_eq!(v["protocol_version"], 1);
        assert_eq!(v["revision"], 3);
        assert_eq!(v["payload"]["selected_group_id"], 1);
        assert_eq!(v["payload"]["groups"][0]["members"][0]["agent"], "claude");
        assert_eq!(v["payload"]["messages"][0]["author"]["kind"], "human");
        assert_eq!(
            current_view_payload(&s, 7),
            p,
            "同一状态两次计算相等(声明式去重依赖它)"
        );
    }

    #[test]
    fn payload_includes_hint() {
        let mut s = state_with_messages(vec![]);
        update(
            &mut s,
            Message::Posted {
                project_id: 7,
                group_id: 1,
                human: msg(1, 1, GroupAuthor::Human, None),
                placeholders: vec![],
                unknown: vec![],
            },
        );
        assert_eq!(
            current_view_payload(&s, 7).hint.as_deref(),
            Some("无人被点名,仅作为上下文")
        );
    }

    // ---- 事件解析与校验 ----

    fn ev(v: serde_json::Value) -> GroupChatWebviewEvent {
        parse_group_chat_event(&v.to_string()).expect("parse")
    }

    #[test]
    fn parse_rejects_garbage_and_unknown_kinds() {
        assert!(parse_group_chat_event("nope").is_err());
        assert!(parse_group_chat_event(r#"{"kind":"rm_rf"}"#).is_err());
    }

    #[test]
    fn ready_and_failed_are_not_commands() {
        let s = state_with_messages(vec![]);
        assert_eq!(route_event(&s, ev(json!({"kind":"ready"}))), None);
        assert_eq!(
            route_event(&s, ev(json!({"kind":"failed","reason":"x"}))),
            None
        );
    }

    #[test]
    fn create_group_trims_and_validates_topic() {
        let s = WorkspaceState::default();
        assert_eq!(
            route_event(&s, ev(json!({"kind":"create_group","topic":"  评审  "}))),
            Some(Command::CreateGroup {
                topic: "评审".into()
            })
        );
        assert_eq!(
            route_event(&s, ev(json!({"kind":"create_group","topic":"   "}))),
            None
        );
        let long = "字".repeat(MAX_TOPIC_CHARS + 1);
        assert_eq!(
            route_event(&s, ev(json!({"kind":"create_group","topic":long}))),
            None
        );
    }

    #[test]
    fn select_and_delete_require_an_existing_group() {
        let s = state_with_messages(vec![]);
        assert_eq!(
            route_event(&s, ev(json!({"kind":"select_group","group_id":1}))),
            Some(Command::SelectGroup { group_id: 1 })
        );
        assert_eq!(
            route_event(&s, ev(json!({"kind":"select_group","group_id":9}))),
            None
        );
        assert_eq!(
            route_event(&s, ev(json!({"kind":"delete_group","group_id":1}))),
            Some(Command::DeleteGroup { group_id: 1 })
        );
        assert_eq!(
            route_event(&s, ev(json!({"kind":"delete_group","group_id":9}))),
            None
        );
    }

    #[test]
    fn add_member_accepts_only_claude_and_codex() {
        let s = state_with_messages(vec![]);
        let ok = ev(
            json!({"kind":"add_member","group_id":1,"agent":"codex","handle":" 审阅者2 ","role_prompt":"挑刺"}),
        );
        assert_eq!(
            route_event(&s, ok),
            Some(Command::AddMember {
                group_id: 1,
                agent: AgentKind::Codex,
                handle: "审阅者2".into(),
                role_prompt: "挑刺".into()
            })
        );
        for agent in ["goose", "aider", "unknown", "codebuddy"] {
            let bad = ev(
                json!({"kind":"add_member","group_id":1,"agent":agent,"handle":"x","role_prompt":""}),
            );
            assert_eq!(route_event(&s, bad), None, "{agent}");
        }
        let no_group = ev(
            json!({"kind":"add_member","group_id":9,"agent":"claude","handle":"x","role_prompt":""}),
        );
        assert_eq!(route_event(&s, no_group), None);
        let empty_handle = ev(
            json!({"kind":"add_member","group_id":1,"agent":"claude","handle":"  ","role_prompt":""}),
        );
        assert_eq!(route_event(&s, empty_handle), None);
    }

    #[test]
    fn role_prompt_length_is_capped() {
        let s = state_with_messages(vec![]);
        let long = "字".repeat(MAX_ROLE_CHARS + 1);
        let e = ev(
            json!({"kind":"add_member","group_id":1,"agent":"claude","handle":"x","role_prompt":long}),
        );
        assert_eq!(route_event(&s, e), None);
    }

    #[test]
    fn update_and_remove_member_require_existing_member() {
        let s = state_with_messages(vec![]);
        assert_eq!(
            route_event(
                &s,
                ev(json!({"kind":"update_member","member_id":10,"handle":"新名","role_prompt":""}))
            ),
            Some(Command::UpdateMember {
                member_id: 10,
                handle: "新名".into(),
                role_prompt: String::new()
            })
        );
        assert_eq!(
            route_event(
                &s,
                ev(json!({"kind":"update_member","member_id":77,"handle":"x","role_prompt":""}))
            ),
            None
        );
        assert_eq!(
            route_event(&s, ev(json!({"kind":"remove_member","member_id":11}))),
            Some(Command::RemoveMember { member_id: 11 })
        );
        assert_eq!(
            route_event(&s, ev(json!({"kind":"remove_member","member_id":77}))),
            None
        );
    }

    #[test]
    fn post_requires_current_group_and_valid_text() {
        let s = state_with_messages(vec![]);
        assert_eq!(
            route_event(
                &s,
                ev(json!({"kind":"post","group_id":1,"text":" @架构师 你好 "}))
            ),
            Some(Command::Post {
                group_id: 1,
                text: "@架构师 你好".into()
            })
        );
        assert_eq!(
            route_event(&s, ev(json!({"kind":"post","group_id":2,"text":"hi"}))),
            None
        );
        assert_eq!(
            route_event(&s, ev(json!({"kind":"post","group_id":1,"text":"  "}))),
            None
        );
        let long = "字".repeat(MAX_POST_CHARS + 1);
        assert_eq!(
            route_event(&s, ev(json!({"kind":"post","group_id":1,"text":long}))),
            None
        );
    }

    #[test]
    fn cancel_requires_current_group_and_valid_scope() {
        let s = state_with_messages(vec![]);
        assert_eq!(
            route_event(
                &s,
                ev(json!({"kind":"cancel","group_id":1,"scope":"round"}))
            ),
            Some(Command::Cancel {
                group_id: 1,
                scope: GroupCancelScope::Round
            })
        );
        assert!(
            parse_group_chat_event(r#"{"kind":"cancel","group_id":1,"scope":"everything"}"#)
                .is_err()
        );
        assert_eq!(
            route_event(&s, ev(json!({"kind":"cancel","group_id":2,"scope":"turn"}))),
            None
        );
    }

    #[test]
    fn retry_only_for_failed_or_cancelled_messages_in_view() {
        let s = state_with_messages(vec![
            msg(
                1,
                1,
                GroupAuthor::Member { member_id: 10 },
                Some(GroupMessageStatus::Failed { reason: "x".into() }),
            ),
            msg(
                2,
                2,
                GroupAuthor::Member { member_id: 10 },
                Some(GroupMessageStatus::Done),
            ),
            msg(
                3,
                3,
                GroupAuthor::Member { member_id: 10 },
                Some(GroupMessageStatus::Cancelled),
            ),
            msg(4, 4, GroupAuthor::Human, None),
        ]);
        assert_eq!(
            route_event(&s, ev(json!({"kind":"retry","message_id":1}))),
            Some(Command::Retry { message_id: 1 })
        );
        assert_eq!(
            route_event(&s, ev(json!({"kind":"retry","message_id":3}))),
            Some(Command::Retry { message_id: 3 })
        );
        assert_eq!(
            route_event(&s, ev(json!({"kind":"retry","message_id":2}))),
            None
        );
        assert_eq!(
            route_event(&s, ev(json!({"kind":"retry","message_id":4}))),
            None
        );
        assert_eq!(
            route_event(&s, ev(json!({"kind":"retry","message_id":99}))),
            None
        );
    }

    #[test]
    fn push_todo_requires_message_in_view_without_existing_todo_and_valid_text() {
        let mut linked = msg(
            2,
            2,
            GroupAuthor::Member { member_id: 10 },
            Some(GroupMessageStatus::Done),
        );
        linked.todo_id = Some(5);
        let s = state_with_messages(vec![
            msg(1, 1, GroupAuthor::Human, None),
            linked,
            msg(
                3,
                3,
                GroupAuthor::Member { member_id: 10 },
                Some(GroupMessageStatus::Running),
            ),
        ]);
        assert_eq!(
            route_event(
                &s,
                ev(json!({"kind":"push_todo","message_id":1,"text":" 加验证码 "}))
            ),
            Some(Command::PushTodo {
                group_id: 1,
                message_id: 1,
                text: "加验证码".into()
            })
        );
        assert_eq!(
            route_event(
                &s,
                ev(json!({"kind":"push_todo","message_id":2,"text":"x"}))
            ),
            None,
            "已转过"
        );
        assert_eq!(
            route_event(
                &s,
                ev(json!({"kind":"push_todo","message_id":3,"text":"x"}))
            ),
            None,
            "进行中的消息没有正文可转"
        );
        assert_eq!(
            route_event(
                &s,
                ev(json!({"kind":"push_todo","message_id":1,"text":" "}))
            ),
            None
        );
        assert_eq!(
            route_event(
                &s,
                ev(json!({"kind":"push_todo","message_id":99,"text":"x"}))
            ),
            None
        );
        let long = "字".repeat(MAX_TODO_CHARS + 1);
        assert_eq!(
            route_event(
                &s,
                ev(json!({"kind":"push_todo","message_id":1,"text":long}))
            ),
            None
        );
    }

    #[test]
    fn open_todo_needs_positive_id() {
        let s = WorkspaceState::default();
        assert_eq!(
            route_event(&s, ev(json!({"kind":"open_todo","todo_id":5}))),
            Some(Command::OpenTodo { todo_id: 5 })
        );
        assert_eq!(
            route_event(&s, ev(json!({"kind":"open_todo","todo_id":0}))),
            None
        );
        assert_eq!(
            route_event(&s, ev(json!({"kind":"open_todo","todo_id":-3}))),
            None
        );
    }

    // ---- 声明式推送状态 ----

    #[test]
    fn push_state_only_pushes_when_ready_and_changed() {
        let mut ps = WebviewPushState::default();
        let a = current_view_payload(&WorkspaceState::default(), 7);
        assert!(ps.pending_push(&a).is_none(), "未 ready 不推");
        ps.set_ready(true);
        assert!(ps.pending_push(&a).is_some());
        assert_eq!(ps.mark_sent(a.clone()), 1);
        assert!(ps.pending_push(&a).is_none(), "未变化不重推");
        let b = current_view_payload(&state_with_messages(vec![]), 7);
        assert!(ps.pending_push(&b).is_some());
        ps.set_ready(true);
        assert!(ps.pending_push(&b).is_some(), "重新 ready 强制重发");
    }

    #[test]
    fn push_state_times_out_when_never_ready_and_recovers_on_clear() {
        let mut ps = WebviewPushState::default();
        let t0 = Instant::now();
        ps.observe_availability(true, t0);
        assert!(ps.failed().is_none());
        ps.observe_availability(true, t0 + READY_TIMEOUT + Duration::from_secs(1));
        assert!(ps.failed().is_some());
        ps.clear_failed();
        assert!(ps.failed().is_none());
    }

    #[test]
    fn push_state_failed_event_blocks_pushes() {
        let mut ps = WebviewPushState::default();
        ps.set_ready(true);
        ps.set_failed("渲染异常".into());
        let a = current_view_payload(&WorkspaceState::default(), 7);
        assert!(ps.pending_push(&a).is_none());
        assert_eq!(ps.failed(), Some("渲染异常"));
    }
}
