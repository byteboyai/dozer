//! 群聊面板扩展(spec `docs/superpowers/specs/2026-10-02-group-chat-panel-design.md`)。
//!
//! 状态按项目挂在 `Workspace.group_chat`。`update` 是**纯函数**(不碰 `Client`、
//! 不 spawn),返回 `Effect` 由 `App` 去执行——这样"乱序/重复/换群后的旧结果"
//! 这类竞态能在单测里直接覆盖。后端更新靠 `rev` 增量轮询(见 `POLL_INTERVAL`
//! 与 `App::poll_group_chat_if_active`)。

use crate::consts::{POLL_BACKOFF, POLL_INTERVAL, POLL_LIMIT};
use crate::protocol::Command;
use dozer_model::{AgentKind, GroupInfo, GroupMessageInfo, GroupMessageStatus, LocalizedText};
use dozer_panel_host::{Notice, NoticeLevel, NoticeOutbox, PanelIo};
use dozer_services::ServiceError;
use std::collections::{BTreeMap, HashMap};
use std::time::Instant;

#[derive(Debug, Default)]
pub struct WorkspaceState {
    pub(crate) requests: dozer_panel_host::RequestTrackers,
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
    hint: Option<LocalizedText>,
    /// 原生群列表侧"新建群聊"弹窗主题框草稿(多行 text_editor,约 3 行定高)。
    new_group_topic: iced_widget::text_editor::Content,
    /// 群聊内容页(webview)消息输入框草稿,按 group_id 存。webview 是单固定
    /// 槽、切面板即销毁重建,前端 `useRef` 里的草稿会随页面一起丢;这里持有
    /// host 侧权威副本(`set_draft` 事件逐键同步),页面加载完再整体下发恢复。
    composer_drafts: HashMap<i64, String>,
    /// 原生群列表侧"新建群聊"弹窗是否打开。
    new_group_dialog_open: bool,
    /// 新建群弹窗里已勾选要加入的 agent(后端支持的成员种类,见 `CREATABLE_AGENTS`)。
    /// 提交创建群后由 `GroupCreated` 消费为 `AddMembers` effect。
    new_group_selected_agents: Vec<AgentKind>,
    pub(crate) outbox: NoticeOutbox,
}

#[derive(Debug, Clone)]
#[allow(clippy::enum_variant_names)]
pub enum Message {
    AsyncResult(bytehost_core::request::RequestToken, Box<Message>),
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
    /// 原生群列表:点一行选中该群(项目 id, 群 id)。项目 id 用于消息路由,
    /// 群 id 才是 `select` 的目标——两者不可混用。
    SelectRequested(i64, i64),
    /// 原生群列表"新建群聊"弹窗主题文本框的编辑动作(text_editor)。
    NewGroupTopicAction(i64, iced_widget::text_editor::Action),
    /// 打开/关闭原生群列表"新建群聊"弹窗。
    NewGroupDialogToggle(i64),
    /// 取消并关闭"新建群聊"弹窗。
    NewGroupDialogCancel(i64),
    /// 新建群弹窗内勾选/取消某个参与 agent。
    NewGroupAgentToggled(i64, AgentKind),
    /// 原生群列表提交"新建群聊"(空主题忽略)。
    NewGroupSubmit(i64),
    /// 原生列表交互的悬停/无操作占位。
    Noop(i64),
    /// 一次性失败(新建群失败、发送失败…),进 Toast。后台任务(拿不到 locale)
    /// 构造 keyed 文案,渲染端解析。
    Failed(i64, LocalizedText),
    /// 弹窗提交的创建群失败:带回提交时的主题,让弹窗带着草稿重新打开(同时进 Toast)。
    CreateFailed(i64, String, LocalizedText),
}

impl Message {
    /// Strip the async-result envelope; used by host-side tests.
    #[doc(hidden)]
    pub fn result_payload(self) -> Self {
        match self {
            Self::AsyncResult(_, inner) => inner.result_payload(),
            other => other,
        }
    }

    pub fn project_id(&self) -> i64 {
        match self {
            Message::AsyncResult(token, _) => {
                dozer_panel_host::project_of(token.scope()).expect("group project")
            }
            Message::GroupsLoaded(p, _)
            | Message::GroupCreated(p, _)
            | Message::GroupChanged(p, _)
            | Message::GroupDeleted(p, _)
            | Message::MessageUpdated(p, _)
            | Message::NewGroupTopicAction(p, _)
            | Message::NewGroupDialogToggle(p)
            | Message::NewGroupDialogCancel(p)
            | Message::NewGroupSubmit(p)
            | Message::NewGroupAgentToggled(p, _)
            | Message::Noop(p)
            | Message::Failed(p, _)
            | Message::CreateFailed(p, _, _) => *p,
            Message::SelectRequested(p, _)
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
    /// 新建群(主题由弹窗输入提供)。
    CreateGroup { topic: String },
    /// 群创建成功后批量加入已勾选的 agent(handle 默认取品牌名,角色留空)。
    AddMembers {
        group_id: i64,
        members: Vec<AgentKind>,
    },
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
    pub fn hint(&self) -> Option<&LocalizedText> {
        self.hint.as_ref()
    }
    /// 原生群列表"新建群聊"弹窗主题框的当前草稿。
    pub fn new_group_topic(&self) -> &iced_widget::text_editor::Content {
        &self.new_group_topic
    }
    /// 原生群列表"新建群聊"弹窗是否打开。
    pub fn new_group_dialog_open(&self) -> bool {
        self.new_group_dialog_open
    }
    /// 新建群弹窗里已勾选要加入的 agent。
    pub fn new_group_selected_agents(&self) -> &[AgentKind] {
        &self.new_group_selected_agents
    }

    /// webview 发来的消息输入框草稿(群必须存在;空/纯空白即清除;超长丢弃)。
    /// 校验收敛在这里,调用方(`App` 的 `SetDraft` 分支)只管透传。
    pub fn set_composer_draft(&mut self, group_id: i64, text: &str) {
        if !self.groups.iter().any(|g| g.id == group_id) {
            return;
        }
        if text.chars().count() > crate::protocol::MAX_POST_CHARS {
            return;
        }
        if text.trim().is_empty() {
            self.composer_drafts.remove(&group_id);
        } else {
            self.composer_drafts.insert(group_id, text.to_string());
        }
    }

    /// 整份草稿快照序列化成 JSON(`{"<group_id>":"<text>"}`),供 webview
    /// (重)加载后一次性恢复前端 `useRef`。键序稳定(BTreeMap),便于测试。
    pub fn composer_drafts_restore_json(&self) -> String {
        let map: BTreeMap<i64, &String> =
            self.composer_drafts.iter().map(|(k, v)| (*k, v)).collect();
        serde_json::to_string(&map).unwrap_or_else(|_| "{}".into())
    }

    pub fn has_active_turn(&self) -> bool {
        self.messages.iter().any(|m| is_active(&m.status))
    }

    pub fn take_outbox(&mut self) -> Vec<Notice> {
        self.outbox.take()
    }

    /// Request tracker, exposed for hosts/tests that must start or invalidate requests.
    pub fn requests(&self) -> &dozer_panel_host::RequestTrackers {
        &self.requests
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
        self.requests.invalidate("messages");
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
    pub fn load_due(&mut self) -> bool {
        if !self.load_pending() {
            return false;
        }
        self.load_in_flight = true;
        true
    }

    /// 切入面板时调用:下次 `load_due` 重新加载群列表。不清已有的群与消息,
    /// 避免切入瞬间闪成"没有群聊"。
    pub fn mark_stale(&mut self) {
        self.requests.invalidate("groups");
        self.load_in_flight = false;
        self.fresh = false;
    }

    /// 是否该发起一次轮询:没有在途、已过退避、距上次 ≥ `POLL_INTERVAL`。
    /// 返回 `true` 时已记下"在途 + 本次时间"。
    pub fn poll_due(&mut self, now: Instant) -> bool {
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
    let mut msg = msg;
    while let Message::AsyncResult(token, inner) = msg {
        if !state.requests.accepts(&token) {
            return Vec::new();
        }
        msg = *inner;
    }
    match msg {
        Message::AsyncResult(..) => unreachable!("nested async result"),
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
                    state.outbox.push(
                        NoticeLevel::Error,
                        crate::i18n::keyed_args(
                            "error.load_failed",
                            "Failed to load group chats: {e}",
                            &[("e", &e)],
                        ),
                    );
                    Vec::new()
                }
            }
        }
        Message::GroupCreated(_, group) => {
            state.mark_stale();
            let id = group.id;
            state.groups.retain(|g| g.id != id);
            state.groups.insert(0, group);
            state.selected = Some(id);
            state.reset_messages();
            let mut fx = vec![Effect::FetchMessages { group_id: id }];
            // 弹窗里勾选的 agent 在群创建成功后加入(不清草稿之外的状态,这里 drain)。
            if !state.new_group_selected_agents.is_empty() {
                let members = std::mem::take(&mut state.new_group_selected_agents);
                fx.push(Effect::AddMembers {
                    group_id: id,
                    members,
                });
            }
            fx
        }
        Message::GroupChanged(_, group) => {
            state.mark_stale();
            if let Some(slot) = state.groups.iter_mut().find(|g| g.id == group.id) {
                *slot = group;
            }
            Vec::new()
        }
        Message::GroupDeleted(_, id) => {
            state.mark_stale();
            state.groups.retain(|g| g.id != id);
            // 草稿随群一起清,避免删群后重建同名 id 的群时捞出旧草稿。
            state.composer_drafts.remove(&id);
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
                    state.outbox.push(
                        NoticeLevel::Error,
                        crate::i18n::keyed_args(
                            "error.refresh_failed",
                            "Failed to refresh group chats: {e}",
                            &[("e", &e)],
                        ),
                    );
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
                Some(crate::i18n::keyed_args(
                    "hint.unknown_members",
                    "Members not found: {names}",
                    &[("names", &names)],
                ))
            } else if placeholders.is_empty() {
                Some(crate::i18n::keyed(
                    "hint.nobody_mentioned",
                    "No one was mentioned; posted as context only",
                ))
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
        Message::SelectRequested(_, group_id) => state.select(group_id),
        Message::NewGroupTopicAction(_, action) => {
            state.new_group_topic.perform(action.clone());
            Vec::new()
        }
        Message::NewGroupDialogToggle(_) => {
            // 仅打开(幂等):关闭走 Cancel / Esc / 创建后的 `NewGroupDialogCancel`,
            // 避免误触把已填主题与已选 agent 丢掉。
            state.new_group_dialog_open = true;
            Vec::new()
        }
        Message::NewGroupAgentToggled(_, agent) => {
            if let Some(pos) = state
                .new_group_selected_agents
                .iter()
                .position(|a| *a == agent)
            {
                state.new_group_selected_agents.remove(pos);
            } else {
                state.new_group_selected_agents.push(agent);
            }
            Vec::new()
        }
        Message::NewGroupDialogCancel(_) => {
            state.new_group_dialog_open = false;
            state.new_group_topic = iced_widget::text_editor::Content::new();
            Vec::new()
        }
        Message::NewGroupSubmit(_) => {
            let topic = state.new_group_topic.text().trim().to_string();
            if topic.is_empty() {
                // 空主题忽略(与后端校验一致),不清草稿以免用户误触丢输入。
                return Vec::new();
            }
            state.new_group_topic = iced_widget::text_editor::Content::new();
            state.new_group_dialog_open = false;
            vec![Effect::CreateGroup { topic }]
        }
        Message::Noop(_) => Vec::new(),
        Message::Failed(_, text) => {
            state.outbox.push(NoticeLevel::Error, text);
            Vec::new()
        }
        Message::CreateFailed(_, topic, text) => {
            state.outbox.push(NoticeLevel::Error, text);
            // 提交时已关弹窗并清了主题框;失败后带回草稿。已选 agent 在成功前不清,
            // 无需恢复。用户若在等待期间重新打开并写了新内容,不覆盖它。
            if state.new_group_topic.text().trim().is_empty() {
                state.new_group_topic = iced_widget::text_editor::Content::with_text(&topic);
            }
            state.new_group_dialog_open = true;
            Vec::new()
        }
    }
}

// ---------------------------------------------------------------------------
// 异步 spawn:只负责"调 Client → 把结果包成 Message 经 emit 发回",不含业务逻辑
// (校验已在 `protocol::route_event` 完成)。
// ---------------------------------------------------------------------------

/// 弹窗提交的创建:与页面发起的 `Command::CreateGroup` 区别在失败时要带回主题恢复草稿。
fn spawn_create_from_dialog(
    requests: &dozer_panel_host::RequestTrackers,
    project_id: i64,
    topic: String,
    io: &PanelIo<Message>,
) {
    let io = io.with_request(requests.scope(Some(project_id)), Message::AsyncResult);
    io.spawn(move |services, io| async move {
        match services
            .group_chat
            .create_group(project_id, topic.clone())
            .await
        {
            Ok(group) => io.emit(Message::GroupCreated(project_id, group)),
            Err(e) => io.emit(Message::CreateFailed(
                project_id,
                topic,
                crate::i18n::keyed_args(
                    "error.create_failed",
                    "Failed to create group chat: {e}",
                    &[("e", &e.to_string())],
                ),
            )),
        }
    });
}

/// 加载某项目的群聊列表。
pub fn spawn_load_groups(
    requests: &dozer_panel_host::RequestTrackers,
    project_id: i64,
    io: &PanelIo<Message>,
) {
    let io = io.with_request(
        requests.begin(Some(project_id), "groups"),
        Message::AsyncResult,
    );
    io.spawn(move |services, io| async move {
        let result = services
            .group_chat
            .list_groups(project_id)
            .await
            .map_err(|e| e.message);
        io.emit(Message::GroupsLoaded(project_id, result));
    });
}

pub fn spawn_fetch_messages(
    requests: &dozer_panel_host::RequestTrackers,
    project_id: i64,
    group_id: i64,
    after_rev: i64,
    io: &PanelIo<Message>,
) {
    let io = io.with_request(
        requests.begin(Some(project_id), "messages"),
        Message::AsyncResult,
    );
    io.spawn(move |services, io| async move {
        let result = services
            .group_chat
            .list_group_messages(group_id, after_rev, POLL_LIMIT)
            .await
            .map_err(|e| e.message);
        io.emit(Message::Polled {
            project_id,
            group_id,
            result,
        });
    });
}

/// 执行一条经 `route_event` 校验过的命令。`SelectGroup`/`OpenTodo` 不走这里
/// (前者是同步状态变更,见 [`select_group`];后者是切面板,由 host 处理)。
pub fn spawn_command(
    requests: &dozer_panel_host::RequestTrackers,
    project_id: i64,
    cmd: Command,
    io: &PanelIo<Message>,
) {
    let io = io.with_request(requests.scope(Some(project_id)), Message::AsyncResult);
    io.spawn(move |services, io| async move {
        let chat = services.group_chat;
        let fail = |key: &str, fallback: &str, e: ServiceError| {
            Message::Failed(
                project_id,
                crate::i18n::keyed_args(key, fallback, &[("e", &e.to_string())]),
            )
        };
        match cmd {
            Command::CreateGroup { topic } => match chat.create_group(project_id, topic).await {
                Ok(group) => io.emit(Message::GroupCreated(project_id, group)),
                Err(e) => io.emit(fail(
                    "error.create_failed",
                    "Failed to create group chat: {e}",
                    e,
                )),
            },
            Command::DeleteGroup { group_id } => match chat.delete_group(group_id).await {
                Ok(()) => io.emit(Message::GroupDeleted(project_id, group_id)),
                Err(e) => io.emit(fail(
                    "error.delete_failed",
                    "Failed to delete group chat: {e}",
                    e,
                )),
            },
            Command::AddMember {
                group_id,
                agent,
                handle,
                role_prompt,
            } => match chat
                .add_group_member(group_id, agent, handle, role_prompt)
                .await
            {
                Ok(group) => io.emit(Message::GroupChanged(project_id, group)),
                Err(e) => io.emit(fail(
                    "error.add_member_failed",
                    "Failed to add member: {e}",
                    e,
                )),
            },
            Command::UpdateMember {
                member_id,
                handle,
                role_prompt,
            } => match chat
                .update_group_member(member_id, handle, role_prompt)
                .await
            {
                Ok(group) => io.emit(Message::GroupChanged(project_id, group)),
                Err(e) => io.emit(fail(
                    "error.update_member_failed",
                    "Failed to update member: {e}",
                    e,
                )),
            },
            Command::RemoveMember { member_id } => {
                match chat.remove_group_member(member_id).await {
                    Ok(group) => io.emit(Message::GroupChanged(project_id, group)),
                    Err(e) => io.emit(fail(
                        "error.remove_member_failed",
                        "Failed to remove member: {e}",
                        e,
                    )),
                }
            }
            Command::Post { group_id, text } => {
                match chat.post_group_message(group_id, text).await {
                    Ok((human, placeholders, unknown)) => io.emit(Message::Posted {
                        project_id,
                        group_id,
                        human,
                        placeholders,
                        unknown,
                    }),
                    Err(e) => io.emit(fail("error.post_failed", "Failed to send: {e}", e)),
                }
            }
            Command::Cancel { group_id, scope } => {
                if let Err(e) = chat.cancel_group(group_id, scope).await {
                    io.emit(fail("error.cancel_failed", "Failed to stop: {e}", e));
                }
            }
            Command::Retry { message_id } => match chat.retry_group_message(message_id).await {
                Ok(message) => io.emit(Message::MessageUpdated(project_id, message)),
                Err(e) => io.emit(fail("error.retry_failed", "Failed to retry: {e}", e)),
            },
            Command::PushTodo {
                group_id,
                message_id,
                text,
            } => match chat.push_group_message_to_todo(message_id, text).await {
                Ok(todo) => io.emit(Message::TodoPushed {
                    project_id,
                    group_id,
                    message_id,
                    todo_id: todo.id,
                }),
                Err(e) => io.emit(fail(
                    "error.push_todo_failed",
                    "Failed to add to to-do: {e}",
                    e,
                )),
            },
            Command::SelectGroup { .. } | Command::OpenTodo { .. } => {}
        }
    });
}

/// 面板 `Effect` 的执行器:host 不认识这些变体,只负责给 `PanelIo`。
pub fn run_effect(
    requests: &dozer_panel_host::RequestTrackers,
    effect: Effect,
    project_id: i64,
    io: &PanelIo<Message>,
) {
    match effect {
        Effect::FetchMessages { group_id } => {
            spawn_fetch_messages(requests, project_id, group_id, 0, io)
        }
        Effect::CreateGroup { topic } => spawn_create_from_dialog(requests, project_id, topic, io),
        Effect::AddMembers { group_id, members } => {
            // handle 默认取品牌名(均属合法 handle 字符),角色留空。每个 agent
            // 一条独立的 `AddMember`,后端逐一回 `GroupChanged`。
            for agent in members {
                spawn_command(
                    requests,
                    project_id,
                    Command::AddMember {
                        group_id,
                        agent,
                        handle: agent.display_label().into(),
                        role_prompt: String::new(),
                    },
                    io,
                );
            }
        }
    }
}

/// 选中某群(`Command::SelectGroup`:同步状态变更),再执行它产生的 Effect(清空后从头拉消息)。
/// 其余命令走异步 [`spawn_command`];`OpenTodo`(切面板)是跨面板动作,由 host 处理。
pub fn select_group(
    state: &mut WorkspaceState,
    project_id: i64,
    group_id: i64,
    io: &PanelIo<Message>,
) {
    for effect in state.select(group_id) {
        run_effect(&state.requests, effect, project_id, io);
    }
}

/// `ResumeTimeReached` 时调用:该加载群列表就加载,该轮询当前群的消息就轮询
/// (限速、在途、退避都在 `WorkspaceState` 里)。"面板是否可见"由 host 在调用前判断。
pub fn poll_if_due(
    state: &mut WorkspaceState,
    project_id: i64,
    now: Instant,
    io: &PanelIo<Message>,
) {
    if state.load_due() {
        spawn_load_groups(&state.requests, project_id, io);
        return;
    }
    let (Some(group_id), true) = (state.selected(), state.has_active_turn()) else {
        return;
    };
    if state.poll_due(now) {
        let after_rev = state.latest_rev();
        spawn_fetch_messages(&state.requests, project_id, group_id, after_rev, io);
    }
}

/// 面板切入:把群列表标成过期(下次轮询重新加载)。`mark_stale` 不清已有内容,切入瞬间不会闪成
/// "没有群聊";实际加载由 host 的轮询(`poll_if_due`)发起。
pub fn on_activate(state: &mut WorkspaceState) {
    state.mark_stale();
}

#[cfg(test)]
mod review_focus_tests {
    use super::*;
    use dozer_model::{GroupInfo, GroupMessageInfo, GroupMessageStatus};

    fn group(id: i64) -> GroupInfo {
        GroupInfo {
            id,
            project_id: 7,
            topic: format!("g{id}"),
            created_ms: 0,
            members: vec![],
        }
    }

    fn msg(id: i64, seq: i64, rev: i64, status: Option<GroupMessageStatus>) -> GroupMessageInfo {
        GroupMessageInfo {
            id,
            group_id: 1,
            seq,
            rev,
            author: dozer_model::GroupAuthor::Human,
            text: format!("m{id}"),
            mentions: vec![],
            status,
            duration_ms: None,
            created_ms: 0,
            todo_id: None,
        }
    }

    #[test]
    fn a_stale_poll_after_switching_group_is_dropped() {
        let mut state = WorkspaceState::default();
        update(
            &mut state,
            Message::GroupsLoaded(7, Ok(vec![group(1), group(2)])),
        );
        state.select(2);
        update(
            &mut state,
            Message::Polled {
                project_id: 7,
                group_id: 1,
                result: Ok((vec![msg(1, 1, 1, None)], 1)),
            },
        );
        assert!(state.messages().is_empty(), "旧群的迟到结果必须丢弃");
        assert_eq!(state.latest_rev(), 0);
    }

    #[test]
    fn rev_never_goes_backwards_on_out_of_order_polls() {
        let mut state = WorkspaceState::default();
        update(&mut state, Message::GroupsLoaded(7, Ok(vec![group(1)])));
        update(
            &mut state,
            Message::Polled {
                project_id: 7,
                group_id: 1,
                result: Ok((vec![], 10)),
            },
        );
        update(
            &mut state,
            Message::Polled {
                project_id: 7,
                group_id: 1,
                result: Ok((vec![msg(1, 1, 5, None)], 5)),
            },
        );
        assert_eq!(state.latest_rev(), 10);
    }

    #[test]
    fn an_empty_group_state_builds_without_panicking() {
        let state = WorkspaceState::default();
        assert!(state.groups().is_empty());
        assert!(state.messages().is_empty());
        assert!(state.selected().is_none());
        assert!(!state.has_active_turn());
    }
}
