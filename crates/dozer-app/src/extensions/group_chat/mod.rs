//! 群聊面板扩展(spec `docs/superpowers/specs/2026-10-02-group-chat-panel-design.md`)。
//!
//! 状态按项目挂在 `Workspace.group_chat`。`update` 是**纯函数**(不碰 `Client`、
//! 不 spawn),返回 `Effect` 由 `App` 去执行——这样"乱序/重复/换群后的旧结果"
//! 这类竞态能在单测里直接覆盖。后端更新靠 `rev` 增量轮询(见 `POLL_INTERVAL`
//! 与 `App::poll_group_chat_if_active`)。

use crate::extensions::toast::{Level, Outbox, Pending};
use dozer_client::Client;
use dozer_core::protocol::{GroupInfo, GroupMessageInfo, GroupMessageStatus};
use std::time::{Duration, Instant};
use tokio::runtime::Handle;

mod protocol;
mod view;
pub(crate) use protocol::route_event;
pub use protocol::{
    Command, GroupChatWebviewEvent, WebviewPushState, current_view_payload, encode_group_chat_push,
    parse_group_chat_event,
};
pub use view::{ShellMessage, content_pane, list_pane};

dozer_core::scope!(pub(crate) LOG, panel, "group_chat");

/// 有发言进行中时,两次轮询的最小间隔。本地 UDS 往返亚毫秒级,这个间隔只是为了
/// 让流式感觉足够跟手又不空转。
pub const POLL_INTERVAL: Duration = Duration::from_millis(500);
/// 轮询出错(dozerd 不在等)后的退避。
pub const POLL_BACKOFF: Duration = Duration::from_secs(5);
/// 一次轮询最多取回的消息条数(`ListGroupMessages.limit`)。
pub const POLL_LIMIT: u32 = 200;

#[derive(Debug, Default)]
pub struct WorkspaceState {
    groups: Vec<GroupInfo>,
    selected: Option<i64>,
    /// 当前选中群的消息,按 `seq` 升序。
    messages: Vec<GroupMessageInfo>,
    /// 只由 `Polled` 推进(见 `Message::Posted` 的说明)。
    latest_rev: i64,
    /// 是否曾经成功/失败地加载过一次(payload 的 `loaded`;`mark_stale` 不会把它
    /// 改回 `false`,否则切入面板时前端会闪回"加载中")。
    ever_loaded: bool,
    /// 群列表是否是新的;`false` = 需要(重新)加载。默认 `false`(没加载过)。
    fresh: bool,
    load_in_flight: bool,
    poll_in_flight: bool,
    last_poll: Option<Instant>,
    backoff_until: Option<Instant>,
    hint: Option<String>,
    /// 原生群列表侧"新建群聊"内联输入框的草稿。空串 = 未在新建。
    new_group_topic: String,
    /// 原生群列表侧待确认删除的群 id(内联二次确认,不弹独立窗口)。
    delete_confirm: Option<i64>,
    pub(crate) outbox: Outbox,
}

#[derive(Debug, Clone)]
#[allow(clippy::enum_variant_names)]
pub enum Message {
    GroupsLoaded(i64, Result<Vec<GroupInfo>, String>),
    /// 新建群成功:排最前并自动选中。
    GroupCreated(i64, GroupInfo),
    /// 成员增删改后后端返回刷新过的整个群。
    GroupChanged(i64, GroupInfo),
    GroupDeleted(i64, i64),
    Polled {
        project_id: i64,
        group_id: i64,
        result: Result<(Vec<GroupMessageInfo>, i64), String>,
    },
    /// 发送成功:upsert human 消息与排队占位。**不推进 `latest_rev`**——占位的
    /// rev 可能大于某条尚未取回的、rev 更小的变更,推进会让它永久丢失;下一次
    /// 轮询会幂等地带回这些消息。
    Posted {
        project_id: i64,
        group_id: i64,
        human: GroupMessageInfo,
        placeholders: Vec<GroupMessageInfo>,
        unknown: Vec<String>,
    },
    /// 重试成功后后端返回的 `Queued` 消息。
    MessageUpdated(i64, GroupMessageInfo),
    TodoPushed {
        project_id: i64,
        group_id: i64,
        message_id: i64,
        todo_id: i64,
    },
    /// 原生群列表:点一行选中该群。
    SelectRequested(i64),
    /// 原生群列表"新建群聊"内联输入框内容变化。
    NewGroupDraftChanged(i64, String),
    /// 原生群列表提交"新建群聊"(空主题忽略)。
    NewGroupSubmit(i64),
    /// 原生群列表点删除,进入内联二次确认态。
    DeleteRequested(i64, i64),
    /// 内联确认删除。
    DeleteConfirmed(i64),
    /// 取消内联二次确认。
    DeleteCancelled(i64),
    /// 原生列表交互的悬停/无操作占位。
    Noop(i64),
    /// 一次性失败(新建群失败、发送失败…),进 Toast。
    Failed(i64, String),
}

impl Message {
    pub fn project_id(&self) -> i64 {
        match self {
            Message::GroupsLoaded(p, _)
            | Message::GroupCreated(p, _)
            | Message::GroupChanged(p, _)
            | Message::GroupDeleted(p, _)
            | Message::MessageUpdated(p, _)
            | Message::NewGroupDraftChanged(p, _)
            | Message::NewGroupSubmit(p)
            | Message::DeleteRequested(p, _)
            | Message::DeleteConfirmed(p)
            | Message::DeleteCancelled(p)
            | Message::Noop(p)
            | Message::Failed(p, _) => *p,
            Message::SelectRequested(p)
            | Message::Polled { project_id: p, .. }
            | Message::Posted { project_id: p, .. }
            | Message::TodoPushed { project_id: p, .. } => *p,
        }
    }
}

/// `update` 要 `App` 去做的异步副作用。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// 从头拉某群的消息(`after_rev = 0`)。
    FetchMessages { group_id: i64 },
    /// 新建群(主题由原生列表内联输入提供)。
    CreateGroup { topic: String },
    /// 删除某群(原生列表内联二次确认通过后)。
    DeleteGroup { group_id: i64 },
}

fn is_active(status: &Option<GroupMessageStatus>) -> bool {
    matches!(
        status,
        Some(GroupMessageStatus::Queued) | Some(GroupMessageStatus::Running)
    )
}

/// 按 id upsert:已有的只有新 `rev >= 旧 rev` 才替换(乱序到达的旧结果不退回旧
/// 状态);没有的插入,保持 `seq` 升序。
fn upsert(messages: &mut Vec<GroupMessageInfo>, incoming: GroupMessageInfo) {
    if let Some(slot) = messages.iter_mut().find(|m| m.id == incoming.id) {
        if incoming.rev >= slot.rev {
            *slot = incoming;
        }
        return;
    }
    let at = messages.partition_point(|m| m.seq < incoming.seq);
    messages.insert(at, incoming);
}

impl WorkspaceState {
    pub fn groups(&self) -> &[GroupInfo] {
        &self.groups
    }
    pub fn selected(&self) -> Option<i64> {
        self.selected
    }
    pub fn messages(&self) -> &[GroupMessageInfo] {
        &self.messages
    }
    pub fn latest_rev(&self) -> i64 {
        self.latest_rev
    }
    pub fn loaded(&self) -> bool {
        self.ever_loaded
    }
    /// 需要(重新)加载群列表且当前没有在途加载。
    pub fn load_pending(&self) -> bool {
        !self.fresh && !self.load_in_flight
    }
    pub fn hint(&self) -> Option<&str> {
        self.hint.as_deref()
    }
    /// 原生群列表"新建群聊"内联输入框的当前草稿。
    pub fn new_group_topic(&self) -> &str {
        &self.new_group_topic
    }
    /// 原生群列表内联二次确认待删除的群 id。
    pub fn delete_confirm(&self) -> Option<i64> {
        self.delete_confirm
    }

    pub fn has_active_turn(&self) -> bool {
        self.messages.iter().any(|m| is_active(&m.status))
    }

    pub fn take_outbox(&mut self) -> Vec<Pending> {
        self.outbox.take()
    }

    /// 选中某群:清空消息与 `latest_rev`、清提示,并请求从头拉取。重复选中同一个
    /// 群、选中不存在的群都是空操作。
    pub fn select(&mut self, group_id: i64) -> Vec<Effect> {
        if self.selected == Some(group_id) || !self.groups.iter().any(|g| g.id == group_id) {
            return Vec::new();
        }
        self.selected = Some(group_id);
        self.reset_messages();
        vec![Effect::FetchMessages { group_id }]
    }

    fn reset_messages(&mut self) {
        self.messages.clear();
        self.latest_rev = 0;
        self.hint = None;
        self.poll_in_flight = false;
    }

    /// 切到一个"该选的群"(首次加载 / 当前群消失 / 删除后回落),返回需要的副作用。
    fn ensure_selection(&mut self) -> Vec<Effect> {
        if let Some(id) = self.selected
            && self.groups.iter().any(|g| g.id == id)
        {
            return Vec::new();
        }
        self.reset_messages();
        match self.groups.first().map(|g| g.id) {
            Some(id) => {
                self.selected = Some(id);
                vec![Effect::FetchMessages { group_id: id }]
            }
            None => {
                self.selected = None;
                Vec::new()
            }
        }
    }

    /// `load_pending()` 为真时标记在途并返回 `true`(调用方随后去 spawn 加载)。
    pub(crate) fn load_due(&mut self) -> bool {
        if !self.load_pending() {
            return false;
        }
        self.load_in_flight = true;
        true
    }

    /// 切入面板时调用:下次 `load_due` 重新加载群列表。不清已有的群与消息,
    /// 避免切入瞬间闪成"没有群聊"。
    pub(crate) fn mark_stale(&mut self) {
        self.fresh = false;
    }

    /// 是否该发起一次轮询:没有在途、已过退避、距上次 ≥ `POLL_INTERVAL`。
    /// 返回 `true` 时已记下"在途 + 本次时间"。
    pub(crate) fn poll_due(&mut self, now: Instant) -> bool {
        if self.poll_in_flight {
            return false;
        }
        if self.backoff_until.is_some_and(|t| now < t) {
            return false;
        }
        if self
            .last_poll
            .is_some_and(|t| now.duration_since(t) < POLL_INTERVAL)
        {
            return false;
        }
        self.poll_in_flight = true;
        self.last_poll = Some(now);
        true
    }
}

pub fn update(state: &mut WorkspaceState, msg: Message) -> Vec<Effect> {
    match msg {
        Message::GroupsLoaded(_, result) => {
            state.load_in_flight = false;
            state.ever_loaded = true;
            state.fresh = true;
            match result {
                Ok(groups) => {
                    state.groups = groups;
                    state.ensure_selection()
                }
                Err(e) => {
                    state
                        .outbox
                        .push(LOG, Level::Error, format!("加载群聊失败: {e}"));
                    Vec::new()
                }
            }
        }
        Message::GroupCreated(_, group) => {
            let id = group.id;
            state.groups.retain(|g| g.id != id);
            state.groups.insert(0, group);
            state.selected = Some(id);
            state.reset_messages();
            vec![Effect::FetchMessages { group_id: id }]
        }
        Message::GroupChanged(_, group) => {
            if let Some(slot) = state.groups.iter_mut().find(|g| g.id == group.id) {
                *slot = group;
            }
            Vec::new()
        }
        Message::GroupDeleted(_, id) => {
            state.groups.retain(|g| g.id != id);
            if state.delete_confirm == Some(id) {
                state.delete_confirm = None;
            }
            if state.selected == Some(id) {
                state.selected = None;
            }
            state.ensure_selection()
        }
        Message::Polled {
            group_id, result, ..
        } => {
            state.poll_in_flight = false;
            if state.selected != Some(group_id) {
                return Vec::new();
            }
            match result {
                Ok((messages, latest)) => {
                    state.backoff_until = None;
                    for m in messages {
                        upsert(&mut state.messages, m);
                    }
                    state.latest_rev = state.latest_rev.max(latest);
                }
                Err(e) => {
                    state.backoff_until = Some(Instant::now() + POLL_BACKOFF);
                    state
                        .outbox
                        .push(LOG, Level::Error, format!("刷新群聊失败: {e}"));
                }
            }
            Vec::new()
        }
        Message::Posted {
            group_id,
            human,
            placeholders,
            unknown,
            ..
        } => {
            if state.selected != Some(group_id) {
                return Vec::new();
            }
            state.hint = if !unknown.is_empty() {
                let names = unknown
                    .iter()
                    .map(|n| format!("@{n}"))
                    .collect::<Vec<_>>()
                    .join("、");
                Some(format!("未找到成员 {names}"))
            } else if placeholders.is_empty() {
                Some("无人被点名,仅作为上下文".to_string())
            } else {
                None
            };
            upsert(&mut state.messages, human);
            for p in placeholders {
                upsert(&mut state.messages, p);
            }
            Vec::new()
        }
        Message::MessageUpdated(_, message) => {
            if state.selected == Some(message.group_id) {
                upsert(&mut state.messages, message);
            }
            Vec::new()
        }
        Message::TodoPushed {
            group_id,
            message_id,
            todo_id,
            ..
        } => {
            if state.selected == Some(group_id)
                && let Some(m) = state.messages.iter_mut().find(|m| m.id == message_id)
            {
                m.todo_id = Some(todo_id);
            }
            Vec::new()
        }
        Message::SelectRequested(id) => state.select(id),
        Message::NewGroupDraftChanged(_, text) => {
            state.new_group_topic = text;
            Vec::new()
        }
        Message::NewGroupSubmit(_) => {
            let topic = state.new_group_topic.trim().to_string();
            if topic.is_empty() {
                // 空主题忽略(与后端校验一致),不清草稿以免用户误触丢输入。
                return Vec::new();
            }
            state.new_group_topic.clear();
            vec![Effect::CreateGroup { topic }]
        }
        Message::DeleteRequested(_, id) => {
            state.delete_confirm = Some(id);
            Vec::new()
        }
        Message::DeleteConfirmed(_) => match state.delete_confirm.take() {
            Some(id) => vec![Effect::DeleteGroup { group_id: id }],
            None => Vec::new(),
        },
        Message::DeleteCancelled(_) => {
            state.delete_confirm = None;
            Vec::new()
        }
        Message::Noop(_) => Vec::new(),
        Message::Failed(_, text) => {
            state.outbox.push(LOG, Level::Error, text);
            Vec::new()
        }
    }
}

// ---------------------------------------------------------------------------
// 异步 spawn:只负责"调 Client → 把结果包成 Message 经 emit 发回",不含业务逻辑
// (校验已在 `protocol::route_event` 完成)。
// ---------------------------------------------------------------------------

/// 加载某项目的群聊列表。
pub fn spawn_load_groups(
    project_id: i64,
    client: &Client,
    handle: &Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    let client = client.clone();
    handle.spawn(async move {
        let result = client
            .list_groups(project_id)
            .await
            .map_err(|e| e.to_string());
        emit(Message::GroupsLoaded(project_id, result));
    });
}

/// 取某群 `rev > after_rev` 的消息(`after_rev = 0` 即全量)。
pub fn spawn_fetch_messages(
    project_id: i64,
    group_id: i64,
    after_rev: i64,
    client: &Client,
    handle: &Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    let client = client.clone();
    handle.spawn(async move {
        let result = client
            .list_group_messages(group_id, after_rev, POLL_LIMIT)
            .await
            .map_err(|e| e.to_string());
        emit(Message::Polled {
            project_id,
            group_id,
            result,
        });
    });
}

/// 执行一条经 `route_event` 校验过的命令。`SelectGroup`/`OpenTodo` 不走这里
/// (前者是同步状态变更,后者是切面板),由 `App` 直接处理。
pub fn spawn_command(
    project_id: i64,
    cmd: Command,
    client: &Client,
    handle: &Handle,
    emit: impl Fn(Message) + Send + 'static,
) {
    let client = client.clone();
    handle.spawn(async move {
        let fail =
            |what: &str, e: anyhow::Error| Message::Failed(project_id, format!("{what}: {e}"));
        match cmd {
            Command::CreateGroup { topic } => match client.create_group(project_id, &topic).await {
                Ok(group) => emit(Message::GroupCreated(project_id, group)),
                Err(e) => emit(fail("新建群聊失败", e)),
            },
            Command::DeleteGroup { group_id } => match client.delete_group(group_id).await {
                Ok(()) => emit(Message::GroupDeleted(project_id, group_id)),
                Err(e) => emit(fail("删除群聊失败", e)),
            },
            Command::AddMember {
                group_id,
                agent,
                handle,
                role_prompt,
            } => match client
                .add_group_member(group_id, agent, &handle, &role_prompt)
                .await
            {
                Ok(group) => emit(Message::GroupChanged(project_id, group)),
                Err(e) => emit(fail("添加成员失败", e)),
            },
            Command::UpdateMember {
                member_id,
                handle,
                role_prompt,
            } => match client
                .update_group_member(member_id, &handle, &role_prompt)
                .await
            {
                Ok(group) => emit(Message::GroupChanged(project_id, group)),
                Err(e) => emit(fail("修改成员失败", e)),
            },
            Command::RemoveMember { member_id } => {
                match client.remove_group_member(member_id).await {
                    Ok(group) => emit(Message::GroupChanged(project_id, group)),
                    Err(e) => emit(fail("移除成员失败", e)),
                }
            }
            Command::Post { group_id, text } => {
                match client.post_group_message(group_id, &text).await {
                    Ok((human, placeholders, unknown)) => emit(Message::Posted {
                        project_id,
                        group_id,
                        human,
                        placeholders,
                        unknown,
                    }),
                    Err(e) => emit(fail("发送失败", e)),
                }
            }
            Command::Cancel { group_id, scope } => {
                if let Err(e) = client.cancel_group(group_id, scope).await {
                    emit(fail("停止失败", e));
                }
            }
            Command::Retry { message_id } => match client.retry_group_message(message_id).await {
                Ok(message) => emit(Message::MessageUpdated(project_id, message)),
                Err(e) => emit(fail("重试失败", e)),
            },
            Command::PushTodo {
                group_id,
                message_id,
                text,
            } => match client.push_group_message_to_todo(message_id, &text).await {
                Ok(todo) => emit(Message::TodoPushed {
                    project_id,
                    group_id,
                    message_id,
                    todo_id: todo.id,
                }),
                Err(e) => emit(fail("转为待办失败", e)),
            },
            Command::SelectGroup { .. } | Command::OpenTodo { .. } => {}
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use dozer_core::protocol::{AgentKind, GroupAuthor, GroupMemberInfo};

    fn member(id: i64, handle: &str) -> GroupMemberInfo {
        GroupMemberInfo {
            id,
            group_id: 1,
            agent: AgentKind::Claude,
            handle: handle.into(),
            role_prompt: String::new(),
        }
    }

    fn group(id: i64, topic: &str) -> GroupInfo {
        GroupInfo {
            id,
            project_id: 1,
            topic: topic.into(),
            created_ms: 0,
            members: vec![member(id * 10, "claude")],
        }
    }

    fn msg(id: i64, seq: i64, rev: i64, status: Option<GroupMessageStatus>) -> GroupMessageInfo {
        GroupMessageInfo {
            id,
            group_id: 1,
            seq,
            rev,
            author: if status.is_some() {
                GroupAuthor::Member { member_id: 10 }
            } else {
                GroupAuthor::Human
            },
            text: format!("m{id}"),
            mentions: vec![],
            status,
            duration_ms: None,
            created_ms: 0,
            todo_id: None,
        }
    }

    fn loaded(groups: Vec<GroupInfo>) -> WorkspaceState {
        let mut s = WorkspaceState::default();
        update(&mut s, Message::GroupsLoaded(1, Ok(groups)));
        s
    }

    #[test]
    fn groups_loaded_selects_first_group_and_requests_messages() {
        let mut s = WorkspaceState::default();
        let fx = update(
            &mut s,
            Message::GroupsLoaded(1, Ok(vec![group(5, "A"), group(6, "B")])),
        );
        assert!(s.loaded());
        assert_eq!(s.selected(), Some(5));
        assert_eq!(fx, vec![Effect::FetchMessages { group_id: 5 }]);
    }

    #[test]
    fn groups_loaded_keeps_selection_when_group_still_exists_and_does_not_refetch() {
        let mut s = loaded(vec![group(5, "A"), group(6, "B")]);
        s.select(6);
        let fx = update(
            &mut s,
            Message::GroupsLoaded(1, Ok(vec![group(5, "A"), group(6, "B2")])),
        );
        assert_eq!(s.selected(), Some(6));
        assert!(fx.is_empty());
        assert_eq!(s.groups()[1].topic, "B2");
    }

    #[test]
    fn groups_loaded_falls_back_when_selected_group_vanished() {
        let mut s = loaded(vec![group(5, "A"), group(6, "B")]);
        s.select(6);
        let fx = update(&mut s, Message::GroupsLoaded(1, Ok(vec![group(5, "A")])));
        assert_eq!(s.selected(), Some(5));
        assert_eq!(fx, vec![Effect::FetchMessages { group_id: 5 }]);
    }

    #[test]
    fn groups_loaded_with_no_groups_clears_selection_and_messages() {
        let mut s = loaded(vec![group(5, "A")]);
        update(
            &mut s,
            Message::Polled {
                project_id: 1,
                group_id: 5,
                result: Ok((vec![msg(1, 1, 1, None)], 1)),
            },
        );
        let fx = update(&mut s, Message::GroupsLoaded(1, Ok(vec![])));
        assert_eq!(s.selected(), None);
        assert!(s.messages().is_empty());
        assert!(fx.is_empty());
    }

    #[test]
    fn groups_loaded_error_goes_to_outbox_and_does_not_retry_storm() {
        let mut s = WorkspaceState::default();
        update(&mut s, Message::GroupsLoaded(1, Err("boom".into())));
        assert!(s.loaded());
        assert!(
            !s.load_pending(),
            "失败也不立刻重试,避免每个 tick 一次重试风暴"
        );
        let pending = s.take_outbox();
        assert_eq!(pending.len(), 1);
        assert!(pending[0].text.contains("boom"));
    }

    #[test]
    fn select_resets_messages_and_requests_fetch() {
        let mut s = loaded(vec![group(5, "A"), group(6, "B")]);
        update(
            &mut s,
            Message::Polled {
                project_id: 1,
                group_id: 5,
                result: Ok((vec![msg(1, 1, 3, None)], 3)),
            },
        );
        let fx = s.select(6);
        assert_eq!(s.selected(), Some(6));
        assert!(s.messages().is_empty());
        assert_eq!(s.latest_rev(), 0);
        assert_eq!(fx, vec![Effect::FetchMessages { group_id: 6 }]);
        assert!(s.select(6).is_empty(), "重复选中同一个群不重复拉取");
        assert!(s.select(999).is_empty(), "不存在的群忽略");
    }

    #[test]
    fn polled_merges_by_id_and_sorts_by_seq() {
        let mut s = loaded(vec![group(1, "A")]);
        update(
            &mut s,
            Message::Polled {
                project_id: 1,
                group_id: 1,
                result: Ok((
                    vec![
                        msg(2, 2, 2, Some(GroupMessageStatus::Running)),
                        msg(1, 1, 1, None),
                    ],
                    2,
                )),
            },
        );
        let ids: Vec<_> = s.messages().iter().map(|m| m.id).collect();
        assert_eq!(ids, vec![1, 2]);
        assert_eq!(s.latest_rev(), 2);
        update(
            &mut s,
            Message::Polled {
                project_id: 1,
                group_id: 1,
                result: Ok((vec![msg(2, 2, 3, Some(GroupMessageStatus::Done))], 3)),
            },
        );
        assert_eq!(s.messages().len(), 2);
        assert_eq!(s.messages()[1].status, Some(GroupMessageStatus::Done));
        assert_eq!(s.latest_rev(), 3);
    }

    /// Review Focus 3:后到的旧结果不得把消息退回旧状态。
    #[test]
    fn stale_poll_result_does_not_regress_a_newer_message() {
        let mut s = loaded(vec![group(1, "A")]);
        update(
            &mut s,
            Message::Polled {
                project_id: 1,
                group_id: 1,
                result: Ok((vec![msg(2, 2, 5, Some(GroupMessageStatus::Done))], 5)),
            },
        );
        update(
            &mut s,
            Message::Polled {
                project_id: 1,
                group_id: 1,
                result: Ok((vec![msg(2, 2, 3, Some(GroupMessageStatus::Running))], 3)),
            },
        );
        assert_eq!(s.messages()[0].status, Some(GroupMessageStatus::Done));
        assert_eq!(s.latest_rev(), 5, "latest_rev 不回退");
    }

    /// Review Focus 3:`Posted` 只 upsert,不推进 `latest_rev`(否则会永久漏掉
    /// rev 更小、尚未取回的变更)。
    #[test]
    fn posted_upserts_messages_but_does_not_advance_latest_rev() {
        let mut s = loaded(vec![group(1, "A")]);
        update(
            &mut s,
            Message::Posted {
                project_id: 1,
                group_id: 1,
                human: msg(1, 1, 10, None),
                placeholders: vec![msg(2, 2, 11, Some(GroupMessageStatus::Queued))],
                unknown: vec![],
            },
        );
        assert_eq!(s.messages().len(), 2);
        assert_eq!(s.latest_rev(), 0);
        assert!(s.has_active_turn());
    }

    /// Review Focus 4:换群后,上一个群的异步结果必须被丢弃。
    #[test]
    fn results_for_a_different_group_are_dropped() {
        let mut s = loaded(vec![group(1, "A"), group(2, "B")]);
        s.select(2);
        update(
            &mut s,
            Message::Polled {
                project_id: 1,
                group_id: 1,
                result: Ok((vec![msg(9, 1, 7, None)], 7)),
            },
        );
        assert!(s.messages().is_empty());
        assert_eq!(s.latest_rev(), 0);
        update(
            &mut s,
            Message::Posted {
                project_id: 1,
                group_id: 1,
                human: msg(9, 1, 7, None),
                placeholders: vec![],
                unknown: vec![],
            },
        );
        assert!(s.messages().is_empty());
    }

    #[test]
    fn poll_error_goes_to_outbox_and_backs_off() {
        let mut s = loaded(vec![group(1, "A")]);
        let t0 = Instant::now();
        assert!(s.poll_due(t0));
        update(
            &mut s,
            Message::Polled {
                project_id: 1,
                group_id: 1,
                result: Err("连不上".into()),
            },
        );
        assert_eq!(s.take_outbox().len(), 1);
        assert!(
            !s.poll_due(t0 + POLL_INTERVAL),
            "出错后退避,不是下个 tick 就重试"
        );
        // 退避起点是出错那一刻的真实时钟(略晚于 t0),留 1s 余量避免慢机器上偶发失败。
        assert!(s.poll_due(t0 + POLL_BACKOFF + Duration::from_secs(1)));
    }

    #[test]
    fn poll_due_rate_limits_and_blocks_while_in_flight() {
        let mut s = loaded(vec![group(1, "A")]);
        let t0 = Instant::now();
        assert!(s.poll_due(t0));
        assert!(!s.poll_due(t0 + POLL_INTERVAL * 2), "上一次还在途");
        update(
            &mut s,
            Message::Polled {
                project_id: 1,
                group_id: 1,
                result: Ok((vec![], 0)),
            },
        );
        assert!(!s.poll_due(t0 + Duration::from_millis(10)), "未到间隔");
        assert!(s.poll_due(t0 + POLL_INTERVAL + Duration::from_millis(10)));
    }

    #[test]
    fn load_due_fires_once_until_result_arrives() {
        let mut s = WorkspaceState::default();
        assert!(s.load_pending());
        assert!(s.load_due());
        assert!(!s.load_due(), "在途期间不重复");
        update(&mut s, Message::GroupsLoaded(1, Ok(vec![])));
        assert!(!s.load_due(), "已加载不再触发");
    }

    #[test]
    fn mark_stale_makes_load_due_fire_again_without_clearing_anything() {
        let mut s = loaded(vec![group(1, "A")]);
        update(
            &mut s,
            Message::Polled {
                project_id: 1,
                group_id: 1,
                result: Ok((vec![msg(1, 1, 1, None)], 1)),
            },
        );
        s.mark_stale();
        assert_eq!(s.groups().len(), 1, "不清空已有的群");
        assert_eq!(s.messages().len(), 1, "不清空已有的消息");
        assert!(s.loaded(), "曾经加载过:payload 不会闪回\"加载中\"");
        assert!(s.load_pending());
        assert!(s.load_due());
    }

    #[test]
    fn group_created_is_selected_and_prepended() {
        let mut s = loaded(vec![group(1, "A")]);
        let fx = update(&mut s, Message::GroupCreated(1, group(2, "新群")));
        assert_eq!(s.selected(), Some(2));
        assert_eq!(s.groups()[0].id, 2, "新群排最前(后端 list 按 id 倒序)");
        assert_eq!(fx, vec![Effect::FetchMessages { group_id: 2 }]);
    }

    #[test]
    fn group_changed_replaces_in_place() {
        let mut s = loaded(vec![group(1, "A"), group(2, "B")]);
        let mut g = group(2, "B");
        g.members.push(member(99, "codex"));
        update(&mut s, Message::GroupChanged(1, g));
        assert_eq!(s.groups()[1].members.len(), 2);
        assert_eq!(s.groups()[0].members.len(), 1);
    }

    #[test]
    fn group_deleted_falls_back_to_next_group() {
        let mut s = loaded(vec![group(1, "A"), group(2, "B")]);
        s.select(2);
        let fx = update(&mut s, Message::GroupDeleted(1, 2));
        assert_eq!(s.groups().len(), 1);
        assert_eq!(s.selected(), Some(1));
        assert_eq!(fx, vec![Effect::FetchMessages { group_id: 1 }]);
        // 删的不是当前选中的群:选中与消息保持不动
        let mut s2 = loaded(vec![group(1, "A"), group(2, "B")]);
        let fx2 = update(&mut s2, Message::GroupDeleted(1, 2));
        assert_eq!(s2.selected(), Some(1));
        assert!(fx2.is_empty());
    }

    #[test]
    fn hint_reports_unknown_handles_and_nobody_mentioned() {
        let mut s = loaded(vec![group(1, "A")]);
        update(
            &mut s,
            Message::Posted {
                project_id: 1,
                group_id: 1,
                human: msg(1, 1, 1, None),
                placeholders: vec![],
                unknown: vec![],
            },
        );
        assert_eq!(s.hint(), Some("无人被点名,仅作为上下文"));
        update(
            &mut s,
            Message::Posted {
                project_id: 1,
                group_id: 1,
                human: msg(2, 2, 2, None),
                placeholders: vec![msg(3, 3, 3, Some(GroupMessageStatus::Queued))],
                unknown: vec!["nobody".into(), "Override".into()],
            },
        );
        assert_eq!(s.hint(), Some("未找到成员 @nobody、@Override"));
        update(
            &mut s,
            Message::Posted {
                project_id: 1,
                group_id: 1,
                human: msg(4, 4, 4, None),
                placeholders: vec![msg(5, 5, 5, Some(GroupMessageStatus::Queued))],
                unknown: vec![],
            },
        );
        assert_eq!(s.hint(), None, "正常点名清掉上次提示");
    }

    #[test]
    fn todo_pushed_marks_the_message_locally() {
        let mut s = loaded(vec![group(1, "A")]);
        update(
            &mut s,
            Message::Polled {
                project_id: 1,
                group_id: 1,
                result: Ok((vec![msg(7, 1, 1, Some(GroupMessageStatus::Done))], 1)),
            },
        );
        update(
            &mut s,
            Message::TodoPushed {
                project_id: 1,
                group_id: 1,
                message_id: 7,
                todo_id: 42,
            },
        );
        assert_eq!(s.messages()[0].todo_id, Some(42));
    }

    #[test]
    fn message_updated_upserts_retry_reply() {
        let mut s = loaded(vec![group(1, "A")]);
        update(
            &mut s,
            Message::Polled {
                project_id: 1,
                group_id: 1,
                result: Ok((
                    vec![msg(
                        2,
                        2,
                        2,
                        Some(GroupMessageStatus::Failed { reason: "x".into() }),
                    )],
                    2,
                )),
            },
        );
        update(
            &mut s,
            Message::MessageUpdated(1, msg(2, 2, 4, Some(GroupMessageStatus::Queued))),
        );
        assert_eq!(s.messages()[0].status, Some(GroupMessageStatus::Queued));
        assert!(s.has_active_turn());
    }

    #[test]
    fn failed_message_goes_to_outbox() {
        let mut s = WorkspaceState::default();
        update(
            &mut s,
            Message::Failed(1, "新建群聊失败: 群主题不能为空".into()),
        );
        let p = s.take_outbox();
        assert_eq!(p.len(), 1);
        assert!(p[0].text.contains("群主题不能为空"));
        assert!(s.take_outbox().is_empty(), "取走后清空");
    }

    #[test]
    fn project_id_is_exposed_for_every_message_kind() {
        let g = group(1, "A");
        let m = msg(1, 1, 1, None);
        let all = [
            Message::GroupsLoaded(3, Ok(vec![])),
            Message::GroupCreated(3, g.clone()),
            Message::GroupChanged(3, g),
            Message::GroupDeleted(3, 1),
            Message::Polled {
                project_id: 3,
                group_id: 1,
                result: Ok((vec![], 0)),
            },
            Message::Posted {
                project_id: 3,
                group_id: 1,
                human: m.clone(),
                placeholders: vec![],
                unknown: vec![],
            },
            Message::MessageUpdated(3, m),
            Message::TodoPushed {
                project_id: 3,
                group_id: 1,
                message_id: 1,
                todo_id: 1,
            },
            Message::Failed(3, "x".into()),
        ];
        for msg in all {
            assert_eq!(msg.project_id(), 3, "{msg:?}");
        }
    }
}
