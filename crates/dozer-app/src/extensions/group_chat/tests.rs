
use super::*;
use dozer_model::{
    AgentKind, GroupAuthor, GroupInfo, GroupMemberInfo, GroupMessageInfo, GroupMessageStatus,
};
use std::time::{Duration, Instant};

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
    assert!(crate::extensions::group_chat::i18n::render(&pending[0].text, "en").contains("boom"));
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
fn new_group_agent_toggled_adds_and_removes() {
    let mut s = WorkspaceState::default();
    assert!(s.new_group_selected_agents().is_empty());
    update(&mut s, Message::NewGroupAgentToggled(1, AgentKind::Claude));
    assert_eq!(s.new_group_selected_agents(), &[AgentKind::Claude]);
    update(&mut s, Message::NewGroupAgentToggled(1, AgentKind::Codex));
    assert_eq!(
        s.new_group_selected_agents(),
        &[AgentKind::Claude, AgentKind::Codex]
    );
    // 再点一次同一个 → 取消勾选(去重,不重复)。
    update(&mut s, Message::NewGroupAgentToggled(1, AgentKind::Claude));
    assert_eq!(s.new_group_selected_agents(), &[AgentKind::Codex]);
}

#[test]
fn new_group_submit_keeps_selected_agents_and_creates() {
    let mut s = WorkspaceState::default();
    update(&mut s, Message::NewGroupAgentToggled(1, AgentKind::Claude));
    update(&mut s, Message::NewGroupDialogToggle(1)); // 仅打开
    assert!(s.new_group_dialog_open());
    update(
        &mut s,
        Message::NewGroupTopicAction(
            1,
            iced_widget::text_editor::Action::Edit(iced_widget::text_editor::Edit::Paste(
                std::sync::Arc::new("评审方案".into()),
            )),
        ),
    );
    let fx = update(&mut s, Message::NewGroupSubmit(1));
    assert_eq!(
        fx,
        vec![Effect::CreateGroup {
            topic: "评审方案".to_string()
        }]
    );
    // 选中 agent 不清(留到 GroupCreated 消费)。
    assert_eq!(s.new_group_selected_agents(), &[AgentKind::Claude]);
    assert!(!s.new_group_dialog_open(), "提交后关闭弹窗");
    assert!(s.new_group_topic().text().is_empty(), "提交后清空草稿");
}

#[test]
fn new_group_submit_ignores_empty_topic() {
    let mut s = WorkspaceState::default();
    update(&mut s, Message::NewGroupAgentToggled(1, AgentKind::Claude));
    let fx = update(&mut s, Message::NewGroupSubmit(1));
    assert!(fx.is_empty(), "空主题不创建群(无副作用)");
    // 主题草稿与已选 agent 都保留,避免误触空提交丢掉用户已做的选择。
    assert_eq!(s.new_group_selected_agents(), &[AgentKind::Claude]);
}

// ---- 输入框草稿(webview → host 同步,切面板后恢复) ----

#[test]
fn composer_draft_set_update_clear_and_rejects() {
    let mut s = loaded(vec![group(1, "A")]);
    // 存草稿。
    s.set_composer_draft(1, "评审一下这个方案");
    assert_eq!(
        s.composer_drafts_restore_json(),
        r#"{"1":"评审一下这个方案"}"#
    );
    // 覆盖更新。
    s.set_composer_draft(1, "第二版");
    assert_eq!(s.composer_drafts_restore_json(), r#"{"1":"第二版"}"#);
    // 空白文本 = 清除(与前端"发空清草稿"一致)。
    s.set_composer_draft(1, "   ");
    assert_eq!(s.composer_drafts_restore_json(), "{}");
    // 超长(超过 Post 上限)丢弃,不落 host 副本。
    let long = "字".repeat(crate::extensions::group_chat::protocol::MAX_POST_CHARS + 1);
    s.set_composer_draft(1, &long);
    assert_eq!(s.composer_drafts_restore_json(), "{}");
    // 群不存在不落(换群后迟到的 set_draft 不许写脏)。
    s.set_composer_draft(99, "x");
    assert_eq!(s.composer_drafts_restore_json(), "{}");
}

#[test]
fn group_deleted_prunes_composer_draft() {
    let mut s = loaded(vec![group(1, "A")]);
    s.set_composer_draft(1, "草稿");
    update(&mut s, Message::GroupDeleted(7, 1));
    assert_eq!(s.composer_drafts_restore_json(), "{}");
}

#[test]
fn group_created_emits_add_members_for_selected() {
    let mut s = loaded(vec![group(1, "A")]);
    update(&mut s, Message::NewGroupAgentToggled(1, AgentKind::Claude));
    update(&mut s, Message::NewGroupAgentToggled(1, AgentKind::Codex));
    update(
        &mut s,
        Message::NewGroupTopicAction(
            1,
            iced_widget::text_editor::Action::Edit(iced_widget::text_editor::Edit::Paste(
                std::sync::Arc::new("评审".into()),
            )),
        ),
    );
    let fx = update(&mut s, Message::GroupCreated(1, group(2, "新群")));
    // 先 FetchMessages(选中新群)再 AddMembers(已选 agent)。
    assert_eq!(fx[0], Effect::FetchMessages { group_id: 2 });
    assert_eq!(
        fx[1],
        Effect::AddMembers {
            group_id: 2,
            members: vec![AgentKind::Claude, AgentKind::Codex],
        }
    );
    // 消费后清空,避免下次 GroupCreated 再次加入。
    assert!(s.new_group_selected_agents().is_empty());
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
    assert_eq!(
        s.hint()
            .map(|t| crate::extensions::group_chat::i18n::render(t, "en")),
        Some("No one was mentioned; posted as context only".to_string())
    );
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
    assert_eq!(
        s.hint()
            .map(|t| crate::extensions::group_chat::i18n::render(t, "en")),
        Some("Members not found: @nobody、@Override".to_string())
    );
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
        Message::Failed(
            1,
            keyed_args(
                "error.create_failed",
                "Failed to create group chat: {e}",
                &[("e", "group topic cannot be empty")],
            ),
        ),
    );
    let p = s.take_outbox();
    assert_eq!(p.len(), 1);
    assert!(
        crate::extensions::group_chat::i18n::render(&p[0].text, "en")
            .contains("group topic cannot be empty")
    );
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
        Message::SelectRequested(3, 1),
        Message::NewGroupTopicAction(
            3,
            iced_widget::text_editor::Action::Edit(iced_widget::text_editor::Edit::Enter),
        ),
        Message::NewGroupDialogToggle(3),
        Message::NewGroupDialogCancel(3),
        Message::NewGroupSubmit(3),
        Message::NewGroupAgentToggled(3, AgentKind::Claude),
        Message::Noop(3),
        Message::Failed(3, "x".into()),
    ];
    for msg in all {
        assert_eq!(msg.project_id(), 3, "{msg:?}");
    }
}

// ---- H4:Effect/Command 的执行器随面板走(host 只给 PanelIo) ----

use crate::panel_host::PanelIo;
use crate::service_fakes::FakeGroupChat;
use dozer_services::ServiceErrorKind;
use std::sync::mpsc::Receiver;

const TIMEOUT: Duration = Duration::from_secs(5);

/// 默认服务束由指向不存在 socket 的 `Client` 适配:所有请求都会快速失败,于是每个任务都恰好发出一条"失败"
/// 形态的消息——不需要 dozerd 就能断言"哪个 Effect/Command 触发了哪个任务、报什么错"。
fn offline_io(rt: &tokio::runtime::Runtime) -> (PanelIo<Message>, Receiver<Message>) {
    // 默认服务束由指向不存在 socket 的 Client 适配:所有请求快速失败,语义同迁移前。
    let (ctx, rx) = crate::panel_host::testing::offline_ctx::<Message>(rt, 7, None);
    (ctx.io, rx)
}

/// 用进程内 `FakeGroupChat` 构造 io(不起任何 socket)。
fn fake_io(
    rt: &tokio::runtime::Runtime,
    fake: &FakeGroupChat,
) -> (PanelIo<Message>, Receiver<Message>) {
    let (ctx, rx) = crate::panel_host::testing::offline_ctx::<Message>(rt, 7, None);
    (ctx.io.with_services(fake.clone().into_services()), rx)
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap()
}

#[test]
fn load_groups_against_an_unreachable_daemon_reports_an_error_for_that_project() {
    let rt = runtime();
    let (io, rx) = offline_io(&rt);
    spawn_load_groups(&Default::default(), 7, &io);
    match rx.recv_timeout(TIMEOUT).unwrap().result_payload() {
        Message::GroupsLoaded(7, Err(_)) => {}
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn fetch_messages_against_an_unreachable_daemon_reports_an_error_poll_for_that_group() {
    let rt = runtime();
    let (io, rx) = offline_io(&rt);
    spawn_fetch_messages(&Default::default(), 7, 5, 3, &io);
    match rx.recv_timeout(TIMEOUT).unwrap().result_payload() {
        Message::Polled {
            project_id: 7,
            group_id: 5,
            result: Err(_),
        } => {}
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn every_async_command_reports_failure_with_its_own_wording() {
    let cases: Vec<(Command, &str)> = vec![
        (
            Command::CreateGroup { topic: "t".into() },
            "Failed to create group chat",
        ),
        (
            Command::DeleteGroup { group_id: 5 },
            "Failed to delete group chat",
        ),
        (
            Command::AddMember {
                group_id: 5,
                agent: dozer_model::AgentKind::Claude,
                handle: "h".into(),
                role_prompt: String::new(),
            },
            "Failed to add member",
        ),
        (
            Command::UpdateMember {
                member_id: 1,
                handle: "h".into(),
                role_prompt: String::new(),
            },
            "Failed to update member",
        ),
        (
            Command::RemoveMember { member_id: 1 },
            "Failed to remove member",
        ),
        (
            Command::Post {
                group_id: 5,
                text: "hi".into(),
            },
            "Failed to send",
        ),
        (
            Command::Cancel {
                group_id: 5,
                scope: dozer_model::GroupCancelScope::Turn,
            },
            "Failed to stop",
        ),
        (Command::Retry { message_id: 9 }, "Failed to retry"),
        (
            Command::PushTodo {
                group_id: 5,
                message_id: 9,
                text: "x".into(),
            },
            "Failed to add to to-do",
        ),
    ];
    let rt = runtime();
    for (cmd, wording) in cases {
        let (io, rx) = offline_io(&rt);
        spawn_command(&Default::default(), 7, cmd.clone(), &io);
        match rx.recv_timeout(TIMEOUT).unwrap().result_payload() {
            Message::Failed(7, text) => {
                let rendered = crate::extensions::group_chat::i18n::render(&text, "en");
                assert!(rendered.starts_with(wording), "{cmd:?}: {rendered}");
            }
            other => panic!("{cmd:?}: unexpected {other:?}"),
        }
    }
}

#[test]
fn select_and_open_todo_commands_do_not_spawn_anything() {
    let rt = runtime();
    let (io, rx) = offline_io(&rt);
    spawn_command(
        &Default::default(),
        7,
        Command::SelectGroup { group_id: 5 },
        &io,
    );
    spawn_command(
        &Default::default(),
        7,
        Command::OpenTodo { todo_id: 1 },
        &io,
    );
    assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());
}

#[test]
fn run_effect_maps_each_effect_to_its_task() {
    let rt = runtime();
    let (io, rx) = offline_io(&rt);
    run_effect(
        &Default::default(),
        Effect::FetchMessages { group_id: 5 },
        7,
        &io,
    );
    match rx.recv_timeout(TIMEOUT).unwrap().result_payload() {
        Message::Polled {
            project_id: 7,
            group_id: 5,
            result: Err(_),
        } => {}
        other => panic!("unexpected: {other:?}"),
    }
    run_effect(
        &Default::default(),
        Effect::CreateGroup { topic: "t".into() },
        7,
        &io,
    );
    assert!(matches!(
        rx.recv_timeout(TIMEOUT).unwrap().result_payload(),
        Message::Failed(7, ref t)
            if crate::extensions::group_chat::i18n::render(t, "en").starts_with("Failed to create group chat")
    ));
}

#[test]
fn select_group_changes_selection_and_fetches_its_messages() {
    let rt = runtime();
    let (io, rx) = offline_io(&rt);
    let mut s = loaded(vec![group(5, "A"), group(6, "B")]);
    select_group(&mut s, 7, 6, &io);
    assert_eq!(s.selected(), Some(6));
    match rx.recv_timeout(TIMEOUT).unwrap().result_payload() {
        Message::Polled {
            project_id: 7,
            group_id: 6,
            ..
        } => {}
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn select_of_the_current_or_an_unknown_group_does_nothing() {
    let rt = runtime();
    let (io, rx) = offline_io(&rt);
    let mut s = loaded(vec![group(5, "A")]);
    select_group(&mut s, 7, 5, &io);
    select_group(&mut s, 7, 999, &io);
    assert_eq!(s.selected(), Some(5));
    assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());
}

#[test]
fn select_requested_uses_the_group_id_not_the_project_id() {
    // 回归:原生列表曾把项目 id 当群 id 传给 `select`,只有「群 id 恰好等于
    // 项目 id」的那一行能选中,其余行点了没反应。
    let mut s = loaded(vec![group(5, "A"), group(6, "B")]);
    let fx = update(&mut s, Message::SelectRequested(1, 6));
    assert_eq!(s.selected(), Some(6));
    assert_eq!(fx, vec![Effect::FetchMessages { group_id: 6 }]);
}

#[test]
fn poll_if_due_loads_the_group_list_first() {
    let rt = runtime();
    let (io, rx) = offline_io(&rt);
    let mut s = WorkspaceState::default(); // 从未加载:load_due
    poll_if_due(&mut s, 7, Instant::now(), &io);
    assert!(matches!(
        rx.recv_timeout(TIMEOUT).unwrap().result_payload(),
        Message::GroupsLoaded(7, Err(_))
    ));
    // 在途期间不重复发起
    poll_if_due(&mut s, 7, Instant::now(), &io);
    assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());
}

#[test]
fn poll_if_due_polls_only_a_selected_group_with_an_active_turn() {
    let rt = runtime();
    let (io, rx) = offline_io(&rt);
    let mut s = loaded(vec![group(5, "A")]);
    // 没有进行中的发言:不轮询
    poll_if_due(&mut s, 7, Instant::now(), &io);
    assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());
    // 有一条 Running 的消息:轮询,after_rev 取当前 latest_rev
    update(
        &mut s,
        Message::Polled {
            project_id: 7,
            group_id: 5,
            result: Ok((vec![msg(1, 1, 4, Some(GroupMessageStatus::Running))], 4)),
        },
    );
    poll_if_due(&mut s, 7, Instant::now(), &io);
    match rx.recv_timeout(TIMEOUT).unwrap().result_payload() {
        Message::Polled {
            project_id: 7,
            group_id: 5,
            result: Err(_),
        } => {}
        other => panic!("unexpected: {other:?}"),
    }
}

// ---- H6:切入钩子 ----

#[test]
fn on_activate_marks_the_group_list_stale_without_clearing_it() {
    let mut s = loaded(vec![group(5, "A")]);
    assert!(!s.load_pending(), "刚加载完:列表是新的");
    on_activate(&mut s);
    assert!(s.load_pending(), "切入后下次轮询要重新加载群列表");
    assert_eq!(s.groups().len(), 1, "不清已有内容,免得切入瞬间闪成没有群聊");
    assert!(s.loaded(), "ever_loaded 不回退,前端不闪回加载中");
}

#[test]
fn successful_commands_emit_their_messages() {
    let rt = runtime();
    let fake = FakeGroupChat::default();
    let (io, rx) = fake_io(&rt, &fake);
    spawn_command(
        &Default::default(),
        7,
        Command::CreateGroup {
            topic: "主题".into(),
        },
        &io,
    );
    match rx.recv_timeout(TIMEOUT).unwrap().result_payload() {
        Message::GroupCreated(7, g) => assert_eq!((g.id, g.topic.as_str()), (900, "主题")),
        other => panic!("unexpected: {other:?}"),
    }
    spawn_command(
        &Default::default(),
        7,
        Command::DeleteGroup { group_id: 5 },
        &io,
    );
    assert!(matches!(
        rx.recv_timeout(TIMEOUT).unwrap().result_payload(),
        Message::GroupDeleted(7, 5)
    ));
    for cmd in [
        Command::AddMember {
            group_id: 5,
            agent: dozer_model::AgentKind::Claude,
            handle: "h".into(),
            role_prompt: String::new(),
        },
        Command::UpdateMember {
            member_id: 1,
            handle: "h".into(),
            role_prompt: String::new(),
        },
        Command::RemoveMember { member_id: 1 },
    ] {
        spawn_command(&Default::default(), 7, cmd.clone(), &io);
        assert!(
            matches!(
                rx.recv_timeout(TIMEOUT).unwrap().result_payload(),
                Message::GroupChanged(7, _)
            ),
            "{cmd:?}"
        );
    }
    spawn_command(
        &Default::default(),
        7,
        Command::Post {
            group_id: 5,
            text: "hi".into(),
        },
        &io,
    );
    match rx.recv_timeout(TIMEOUT).unwrap().result_payload() {
        Message::Posted {
            project_id: 7,
            group_id: 5,
            human,
            placeholders,
            unknown,
        } => {
            assert_eq!(human.id, 901);
            assert_eq!(placeholders.len(), 1);
            assert_eq!(unknown, vec!["unknown".to_string()]);
        }
        other => panic!("unexpected: {other:?}"),
    }
    spawn_command(
        &Default::default(),
        7,
        Command::Retry { message_id: 9 },
        &io,
    );
    match rx.recv_timeout(TIMEOUT).unwrap().result_payload() {
        Message::MessageUpdated(7, m) => assert_eq!(m.id, 9),
        other => panic!("unexpected: {other:?}"),
    }
    spawn_command(
        &Default::default(),
        7,
        Command::PushTodo {
            group_id: 5,
            message_id: 9,
            text: "x".into(),
        },
        &io,
    );
    assert!(matches!(
        rx.recv_timeout(TIMEOUT).unwrap().result_payload(),
        Message::TodoPushed {
            project_id: 7,
            group_id: 5,
            message_id: 9,
            todo_id: 77
        }
    ));
}

#[test]
fn a_successful_cancel_emits_nothing() {
    let rt = runtime();
    let fake = FakeGroupChat::default();
    let (io, rx) = fake_io(&rt, &fake);
    spawn_command(
        &Default::default(),
        7,
        Command::Cancel {
            group_id: 5,
            scope: dozer_model::GroupCancelScope::Turn,
        },
        &io,
    );
    assert!(
        rx.recv_timeout(Duration::from_millis(300)).is_err(),
        "成功的停止不发任何消息"
    );
    assert_eq!(fake.calls(), vec!["cancel_group(5,Turn)".to_string()]);
}

#[test]
fn command_failure_text_is_prefix_plus_the_service_message_unchanged() {
    let cases: Vec<(Command, &str)> = vec![
        (
            Command::CreateGroup { topic: "t".into() },
            "Failed to create group chat",
        ),
        (
            Command::DeleteGroup { group_id: 5 },
            "Failed to delete group chat",
        ),
        (
            Command::AddMember {
                group_id: 5,
                agent: dozer_model::AgentKind::Claude,
                handle: "h".into(),
                role_prompt: String::new(),
            },
            "Failed to add member",
        ),
        (
            Command::UpdateMember {
                member_id: 1,
                handle: "h".into(),
                role_prompt: String::new(),
            },
            "Failed to update member",
        ),
        (
            Command::RemoveMember { member_id: 1 },
            "Failed to remove member",
        ),
        (
            Command::Post {
                group_id: 5,
                text: "hi".into(),
            },
            "Failed to send",
        ),
        (
            Command::Cancel {
                group_id: 5,
                scope: dozer_model::GroupCancelScope::Turn,
            },
            "Failed to stop",
        ),
        (Command::Retry { message_id: 9 }, "Failed to retry"),
        (
            Command::PushTodo {
                group_id: 5,
                message_id: 9,
                text: "x".into(),
            },
            "Failed to add to to-do",
        ),
    ];
    let rt = runtime();
    for (cmd, wording) in cases {
        let fake = FakeGroupChat::failing_writes(ServiceErrorKind::Rejected, "daemon 错误: boom");
        let (io, rx) = fake_io(&rt, &fake);
        spawn_command(&Default::default(), 7, cmd.clone(), &io);
        match rx.recv_timeout(TIMEOUT).unwrap().result_payload() {
            Message::Failed(7, text) => {
                let rendered = crate::extensions::group_chat::i18n::render(&text, "en");
                assert_eq!(rendered, format!("{wording}: daemon 错误: boom"), "{cmd:?}");
            }
            other => panic!("{cmd:?}: unexpected {other:?}"),
        }
    }
}

#[test]
fn fetch_messages_passes_after_rev_and_the_poll_limit_to_the_service() {
    let rt = runtime();
    let fake = FakeGroupChat::default().with_messages(vec![], 42);
    let (io, rx) = fake_io(&rt, &fake);
    spawn_fetch_messages(&Default::default(), 7, 5, 3, &io);
    match rx.recv_timeout(TIMEOUT).unwrap().result_payload() {
        Message::Polled {
            project_id: 7,
            group_id: 5,
            result: Ok((msgs, latest)),
        } => {
            assert!(msgs.is_empty());
            assert_eq!(latest, 42);
        }
        other => panic!("unexpected: {other:?}"),
    }
    assert_eq!(
        fake.calls(),
        vec![format!("list_group_messages(5,3,{POLL_LIMIT})")]
    );
}

#[test]
fn load_groups_failure_text_is_the_service_message_unchanged() {
    let rt = runtime();
    let fake = FakeGroupChat::failing_reads(ServiceErrorKind::Unavailable, "daemon 断开");
    let (io, rx) = fake_io(&rt, &fake);
    spawn_load_groups(&Default::default(), 7, &io);
    match rx.recv_timeout(TIMEOUT).unwrap().result_payload() {
        Message::GroupsLoaded(7, Err(text)) => assert_eq!(text, "daemon 断开"),
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn stale_groups_do_not_undo_create_and_old_poll_cannot_reenter_selected_group() {
    let mut state = loaded(vec![group(5, "A"), group(6, "B")]);
    let old = state.requests().begin(Some(1), "groups");
    update(&mut state, Message::GroupCreated(1, group(7, "new")));
    update(
        &mut state,
        Message::AsyncResult(old, Box::new(Message::GroupsLoaded(1, Ok(vec![])))),
    );
    assert_eq!(state.groups()[0].id, 7);
    let old_poll = state.requests().begin(Some(1), "messages");
    state.select(5);
    state.select(7);
    update(
        &mut state,
        Message::AsyncResult(
            old_poll,
            Box::new(Message::Polled {
                project_id: 1,
                group_id: 7,
                result: Err("old poll".into()),
            }),
        ),
    );
    assert!(state.take_outbox().is_empty());
    let old_list = state.requests().begin(Some(1), "groups");
    let new_list = state.requests().begin(Some(1), "groups");
    update(
        &mut state,
        Message::AsyncResult(
            new_list,
            Box::new(Message::GroupsLoaded(1, Ok(vec![group(8, "latest")]))),
        ),
    );
    update(
        &mut state,
        Message::AsyncResult(
            old_list,
            Box::new(Message::GroupsLoaded(1, Err("old list".into()))),
        ),
    );
    assert_eq!(state.groups()[0].id, 8);
    assert!(state.take_outbox().is_empty());
}
