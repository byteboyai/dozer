//! 群聊调度器(spec 2026-10-02-group-chat-panel-design §7):每群一条串行队列,
//! 同群单飞、不同群互不阻塞;状态机 `Queued → Running → Done|Failed|Cancelled`
//! 全由 `GroupStore` 的条件更新保证,这里只负责"按顺序一个个跑"。
//!
//! 取消:`Turn` 只停当前那位;`Round` 先把本群 `Queued` 全置 `Cancelled`
//! (worker 取到时 `try_start` 返回 `None` 即跳过),再停当前那位。

use crate::group_adapter::{
    GROUP_TURN_TIMEOUT, GroupAgentRunner, HeadlessGroupRunner, TurnError, TurnRequest,
};
use crate::group_mentions::parse_mentions;
use crate::group_prompt::{PromptInput, build_prompt};
use crate::group_store::GroupStore;
use anyhow::{Result, bail};
use dozer_core::protocol::{GroupCancelScope, GroupMessageInfo};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, watch};

dozer_core::scope!(LOG, module, "group_chat");

/// human 单条消息的字符上限。
pub const MAX_POST_CHARS: usize = 20_000;

pub struct PostOutcome {
    pub human: GroupMessageInfo,
    pub placeholders: Vec<GroupMessageInfo>,
    pub unknown_handles: Vec<String>,
}

type ProjectDirFn = Arc<dyn Fn(i64) -> Option<PathBuf> + Send + Sync>;

/// 正在跑的那一位:取消信号从这里发出。
struct CurrentTurn {
    cancel: watch::Sender<bool>,
}

struct Worker {
    tx: mpsc::UnboundedSender<i64>,
    current: Arc<Mutex<Option<CurrentTurn>>>,
}

pub struct GroupService {
    store: Arc<GroupStore>,
    runner: Arc<dyn GroupAgentRunner>,
    project_dir: ProjectDirFn,
    workers: Mutex<HashMap<i64, Worker>>,
}

impl GroupService {
    pub fn new(
        store: Arc<GroupStore>,
        runner: Arc<dyn GroupAgentRunner>,
        project_dir: ProjectDirFn,
    ) -> Arc<Self> {
        Arc::new(Self {
            store,
            runner,
            project_dir,
            workers: Mutex::new(HashMap::new()),
        })
    }

    /// 给各处 `Stores { .. }` 字面量用的占位实例:临时库 + 立即返回的 runner。
    /// 不会真的起任何子进程。
    #[doc(hidden)]
    pub fn for_tests() -> Arc<Self> {
        struct Noop;
        impl GroupAgentRunner for Noop {
            fn run<'a>(
                &'a self,
                _req: TurnRequest,
                _cancel: watch::Receiver<bool>,
            ) -> crate::group_adapter::BoxFuture<'a, Result<String, TurnError>> {
                Box::pin(async { Ok("ok".to_string()) })
            }
        }
        let db = std::env::temp_dir().join(format!("dozerd-grp-{}.db", uuid::Uuid::new_v4()));
        Self::new(
            Arc::new(GroupStore::new(&db).expect("test group store")),
            Arc::new(Noop),
            Arc::new(|_| Some(std::env::temp_dir())),
        )
    }

    /// 生产构造:真实 runner。
    pub fn with_headless_runner(store: Arc<GroupStore>, project_dir: ProjectDirFn) -> Arc<Self> {
        Self::new(store, Arc::new(HeadlessGroupRunner), project_dir)
    }

    pub fn store(&self) -> &Arc<GroupStore> {
        &self.store
    }

    /// 启动恢复:把上次进程遗留的 `Queued`/`Running` 置 `Failed`。
    pub fn recover_on_startup(&self) {
        match self.store.recover_interrupted() {
            Ok(0) => {}
            Ok(n) => dozer_core::log_warn!(LOG, count = n, "启动恢复:中断的群聊发言已置为失败"),
            Err(e) => dozer_core::log_warn!(LOG, error = %e, "启动恢复群聊发言失败"),
        }
    }

    pub fn post(self: &Arc<Self>, group_id: i64, text: &str) -> Result<PostOutcome> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            bail!("消息不能为空");
        }
        if trimmed.chars().count() > MAX_POST_CHARS {
            bail!("消息过长(上限 {MAX_POST_CHARS} 字)");
        }
        let group = self.store.get_group(group_id)?;
        let roster: Vec<(i64, &str)> = group
            .members
            .iter()
            .map(|m| (m.id, m.handle.as_str()))
            .collect();
        let parsed = parse_mentions(trimmed, &roster);
        let (human, placeholders) =
            self.store
                .post_human_message(group_id, trimmed, &parsed.members)?;
        for p in &placeholders {
            self.enqueue(group_id, p.id);
        }
        Ok(PostOutcome {
            human,
            placeholders,
            unknown_handles: parsed.unknown,
        })
    }

    pub fn cancel(&self, group_id: i64, scope: GroupCancelScope) {
        if scope == GroupCancelScope::Round {
            // 先清排队的,再停当前的:避免当前这位被停后 worker 立刻启动下一位。
            if let Err(e) = self.store.cancel_queued(group_id) {
                dozer_core::log_warn!(LOG, group_id, error = %e, "取消排队发言失败");
            }
        }
        let workers = self.workers.lock().expect("workers lock");
        if let Some(w) = workers.get(&group_id)
            && let Some(cur) = w.current.lock().expect("current lock").as_ref()
        {
            let _ = cur.cancel.send(true);
        }
    }

    pub fn retry(self: &Arc<Self>, message_id: i64) -> Result<GroupMessageInfo> {
        let msg = self.store.requeue(message_id)?;
        self.enqueue(msg.group_id, msg.id);
        Ok(msg)
    }

    /// 删群:先取消在跑的与排队的,再删库,最后撤掉 worker(通道关闭 → 任务退出)。
    pub fn delete_group(&self, group_id: i64) -> Result<()> {
        self.cancel(group_id, GroupCancelScope::Round);
        self.store.delete_group(group_id)?;
        self.workers.lock().expect("workers lock").remove(&group_id);
        Ok(())
    }

    fn enqueue(self: &Arc<Self>, group_id: i64, message_id: i64) {
        let mut workers = self.workers.lock().expect("workers lock");
        let worker = workers.entry(group_id).or_insert_with(|| {
            let (tx, mut rx) = mpsc::unbounded_channel::<i64>();
            let current: Arc<Mutex<Option<CurrentTurn>>> = Arc::new(Mutex::new(None));
            let svc = Arc::clone(self);
            let cur = Arc::clone(&current);
            tokio::spawn(async move {
                while let Some(id) = rx.recv().await {
                    svc.run_turn(group_id, id, &cur).await;
                }
            });
            Worker { tx, current }
        });
        let _ = worker.tx.send(message_id);
    }

    /// 跑一位。取消通道在 `try_start` **之前**登记进 `current`,这样库里一旦是
    /// `Running`,`cancel` 就一定能找到它(没有"已 Running 但取消落空"的窗口)。
    async fn run_turn(
        &self,
        group_id: i64,
        message_id: i64,
        current: &Arc<Mutex<Option<CurrentTurn>>>,
    ) {
        let (cancel_tx, cancel_rx) = watch::channel(false);
        *current.lock().expect("current lock") = Some(CurrentTurn { cancel: cancel_tx });
        let result = self.run_turn_inner(group_id, message_id, cancel_rx).await;
        *current.lock().expect("current lock") = None;
        if let Err(e) = result {
            dozer_core::log_warn!(LOG, group_id, message_id, error = %e, "群聊发言处理出错");
        }
    }

    async fn run_turn_inner(
        &self,
        group_id: i64,
        message_id: i64,
        cancel_rx: watch::Receiver<bool>,
    ) -> Result<()> {
        let Some(msg) = self.store.try_start(message_id)? else {
            return Ok(()); // 已被取消/删除,跳过
        };
        let started = std::time::Instant::now();
        let elapsed = |s: &std::time::Instant| s.elapsed().as_millis() as u64;

        let Some(member_id) = (match msg.author {
            dozer_core::protocol::GroupAuthor::Member { member_id } => Some(member_id),
            _ => None,
        }) else {
            self.store.fail(message_id, "内部错误:不是成员发言", None)?;
            return Ok(());
        };
        let Some(me) = self.store.get_member(member_id)? else {
            self.store.fail(message_id, "成员已移除", None)?;
            return Ok(());
        };
        let group = match self.store.get_group(group_id) {
            Ok(g) => g,
            Err(_) => return Ok(()), // 群已被删除
        };
        let Some(trigger) = self.store.last_human_before(group_id, msg.seq)? else {
            self.store
                .fail(message_id, "找不到触发这次发言的消息", None)?;
            return Ok(());
        };
        let Some(project_dir) = (self.project_dir)(group.project_id) else {
            self.store.fail(message_id, "找不到项目目录", None)?;
            return Ok(());
        };
        let (history, omitted_before) = self.store.history_before(group_id, msg.seq)?;
        let prompt = build_prompt(&PromptInput {
            topic: &group.topic,
            me: &me,
            roster: &group.members,
            history: &history,
            omitted_before,
            trigger: &trigger,
        });

        let outcome = self
            .runner
            .run(
                TurnRequest {
                    agent: me.agent,
                    project_dir,
                    prompt,
                    timeout: GROUP_TURN_TIMEOUT,
                },
                cancel_rx,
            )
            .await;
        let dur = elapsed(&started);
        match outcome {
            Ok(text) => {
                self.store.finish(message_id, &text, dur)?;
            }
            Err(TurnError::Cancelled) => {
                self.store.cancel_running(message_id)?;
            }
            Err(e) => {
                dozer_core::log_info!(LOG, group_id, message_id, kind = ?e, "群聊发言失败");
                self.store.fail(message_id, &e.to_string(), Some(dur))?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::group_adapter::{BoxFuture, TurnError};
    use dozer_core::protocol::{AgentKind, GroupMessageStatus};
    use std::collections::VecDeque;
    use std::sync::Mutex as StdMutex;
    use std::time::Duration;

    struct FakeRunner {
        prompts: StdMutex<Vec<String>>,
        script: StdMutex<VecDeque<Result<String, TurnError>>>,
        /// true:`run` 一直挂起直到被取消(模拟长时间发言)。
        block_until_cancel: bool,
    }

    impl FakeRunner {
        fn new(script: Vec<Result<String, TurnError>>) -> Arc<Self> {
            Arc::new(Self {
                prompts: StdMutex::new(vec![]),
                script: StdMutex::new(script.into()),
                block_until_cancel: false,
            })
        }
        fn blocking() -> Arc<Self> {
            Arc::new(Self {
                prompts: StdMutex::new(vec![]),
                script: StdMutex::new(VecDeque::new()),
                block_until_cancel: true,
            })
        }
        fn prompts(&self) -> Vec<String> {
            self.prompts.lock().unwrap().clone()
        }
    }

    impl GroupAgentRunner for FakeRunner {
        fn run<'a>(
            &'a self,
            req: crate::group_adapter::TurnRequest,
            mut cancel: watch::Receiver<bool>,
        ) -> BoxFuture<'a, Result<String, TurnError>> {
            Box::pin(async move {
                self.prompts.lock().unwrap().push(req.prompt.clone());
                if self.block_until_cancel {
                    loop {
                        if *cancel.borrow() {
                            return Err(TurnError::Cancelled);
                        }
                        if cancel.changed().await.is_err() {
                            std::future::pending::<()>().await;
                        }
                    }
                }
                self.script
                    .lock()
                    .unwrap()
                    .pop_front()
                    .unwrap_or_else(|| Ok("默认回复".into()))
            })
        }
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        svc: Arc<GroupService>,
        runner: Arc<FakeRunner>,
        gid: i64,
        claude: i64,
        codex: i64,
    }

    fn fixture(runner: Arc<FakeRunner>) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(GroupStore::new(&dir.path().join("g.db")).unwrap());
        let proj = dir.path().to_path_buf();
        let svc = GroupService::new(
            store.clone(),
            runner.clone(),
            Arc::new(move |_| Some(proj.clone())),
        );
        let g = store.create_group(1, "评审登录方案").unwrap();
        let g = store
            .add_member(g.id, AgentKind::Claude, "claude", "")
            .unwrap();
        let g = store
            .add_member(g.id, AgentKind::Codex, "codex", "审阅者")
            .unwrap();
        Fixture {
            gid: g.id,
            claude: g.members[0].id,
            codex: g.members[1].id,
            _dir: dir,
            svc,
            runner,
        }
    }

    async fn wait_until_async(mut cond: impl FnMut() -> bool) {
        for _ in 0..250 {
            if cond() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("等待条件超时");
    }

    async fn wait_until(mut cond: impl FnMut() -> bool) {
        for _ in 0..250 {
            if cond() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("等待条件超时");
    }

    fn statuses(f: &Fixture) -> Vec<Option<GroupMessageStatus>> {
        let (msgs, _) = f
            .svc
            .store()
            .list_messages_after_rev(f.gid, 0, 1000)
            .unwrap();
        msgs.into_iter().map(|m| m.status).collect()
    }

    fn all_settled(f: &Fixture) -> bool {
        statuses(f).iter().all(|s| {
            !matches!(
                s,
                Some(GroupMessageStatus::Queued) | Some(GroupMessageStatus::Running)
            )
        })
    }

    #[tokio::test]
    async fn serial_in_mention_order_and_later_speaker_sees_earlier_reply() {
        let f = fixture(FakeRunner::new(vec![
            Ok("CLAUDE-说了这些".into()),
            Ok("CODEX-补充".into()),
        ]));
        f.svc.post(f.gid, "@claude @codex 评审一下").unwrap();
        wait_until(|| all_settled(&f)).await;

        let prompts = f.runner.prompts();
        assert_eq!(prompts.len(), 2);
        assert!(prompts[0].contains("@claude"), "第一位是 claude");
        assert!(!prompts[0].contains("CLAUDE-说了这些"));
        assert!(prompts[1].contains("@codex"));
        assert!(
            prompts[1].contains("CLAUDE-说了这些"),
            "第二位看得到第一位的回复"
        );

        let (msgs, _) = f
            .svc
            .store()
            .list_messages_after_rev(f.gid, 0, 100)
            .unwrap();
        let texts: Vec<_> = msgs.iter().map(|m| m.text.as_str()).collect();
        assert_eq!(
            texts,
            vec!["@claude @codex 评审一下", "CLAUDE-说了这些", "CODEX-补充"]
        );
    }

    #[tokio::test]
    async fn no_mention_runs_nobody() {
        let f = fixture(FakeRunner::new(vec![]));
        let out = f.svc.post(f.gid, "只是随便说说").unwrap();
        assert!(out.placeholders.is_empty());
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(f.runner.prompts().is_empty());
    }

    #[tokio::test]
    async fn unknown_handle_is_reported_but_does_not_block_known_ones() {
        let f = fixture(FakeRunner::new(vec![Ok("好".into())]));
        let out = f.svc.post(f.gid, "@nobody @claude 你来").unwrap();
        assert_eq!(out.unknown_handles, vec!["nobody".to_string()]);
        assert_eq!(out.placeholders.len(), 1);
        wait_until(|| all_settled(&f)).await;
        assert_eq!(f.runner.prompts().len(), 1);
    }

    #[tokio::test]
    async fn failure_does_not_block_the_rest_of_the_round() {
        let f = fixture(FakeRunner::new(vec![
            Err(TurnError::Timeout),
            Ok("我来补".into()),
        ]));
        f.svc.post(f.gid, "@claude @codex hi").unwrap();
        wait_until(|| all_settled(&f)).await;
        let s = statuses(&f);
        assert!(
            matches!(s[1], Some(GroupMessageStatus::Failed { ref reason }) if reason.contains("超时"))
        );
        assert_eq!(s[2], Some(GroupMessageStatus::Done));
    }

    /// Review Focus 5:agent 回复里的 @ 不触发任何人。
    #[tokio::test]
    async fn at_mention_inside_agent_reply_does_not_trigger_anyone() {
        let f = fixture(FakeRunner::new(vec![Ok("我觉得 @codex 应该看看".into())]));
        f.svc.post(f.gid, "@claude 你先说").unwrap();
        wait_until(|| all_settled(&f)).await;
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(f.runner.prompts().len(), 1, "codex 不应被触发");
    }

    /// Review Focus 5:排队期间成员被移除。
    #[tokio::test]
    async fn removed_member_turn_fails_instead_of_hanging() {
        let f = fixture(FakeRunner::blocking());
        f.svc.post(f.gid, "@claude @codex hi").unwrap();
        wait_until(|| f.runner.prompts().len() == 1).await; // claude 在跑
        f.svc.store().remove_member(f.codex).unwrap(); // codex 还在排队
        f.svc.cancel(f.gid, GroupCancelScope::Turn); // 放走 claude,轮到 codex
        wait_until(|| all_settled(&f)).await;
        let s = statuses(&f);
        assert_eq!(s[1], Some(GroupMessageStatus::Cancelled));
        assert!(
            matches!(s[2], Some(GroupMessageStatus::Failed { ref reason }) if reason.contains("已移除")),
            "{:?}",
            s[2]
        );
    }

    #[tokio::test]
    async fn cancel_round_cancels_running_and_queued() {
        let f = fixture(FakeRunner::blocking());
        f.svc.post(f.gid, "@claude @codex hi").unwrap();
        wait_until(|| f.runner.prompts().len() == 1).await;
        f.svc.cancel(f.gid, GroupCancelScope::Round);
        wait_until(|| all_settled(&f)).await;
        let s = statuses(&f);
        assert_eq!(s[1], Some(GroupMessageStatus::Cancelled));
        assert_eq!(s[2], Some(GroupMessageStatus::Cancelled));
        assert_eq!(f.runner.prompts().len(), 1, "codex 从未启动");
    }

    #[tokio::test]
    async fn cancel_turn_only_stops_current_and_next_still_runs() {
        // 第一次调用阻塞(等取消),之后正常返回:验证"取消当前后下一位确实被启动"。
        struct FirstBlocking {
            calls: StdMutex<usize>,
        }
        impl GroupAgentRunner for FirstBlocking {
            fn run<'a>(
                &'a self,
                _req: crate::group_adapter::TurnRequest,
                mut cancel: watch::Receiver<bool>,
            ) -> BoxFuture<'a, Result<String, TurnError>> {
                Box::pin(async move {
                    let n = {
                        let mut c = self.calls.lock().unwrap();
                        *c += 1;
                        *c
                    };
                    if n == 1 {
                        loop {
                            if *cancel.borrow() {
                                return Err(TurnError::Cancelled);
                            }
                            if cancel.changed().await.is_err() {
                                std::future::pending::<()>().await;
                            }
                        }
                    }
                    Ok("第二次成功".into())
                })
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(GroupStore::new(&dir.path().join("g.db")).unwrap());
        let runner: Arc<dyn GroupAgentRunner> = Arc::new(FirstBlocking {
            calls: StdMutex::new(0),
        });
        let proj = dir.path().to_path_buf();
        let svc = GroupService::new(store.clone(), runner, Arc::new(move |_| Some(proj.clone())));
        let g = store.create_group(1, "t").unwrap();
        let g = store
            .add_member(g.id, AgentKind::Claude, "claude", "")
            .unwrap();
        let g = store
            .add_member(g.id, AgentKind::Codex, "codex", "")
            .unwrap();
        svc.post(g.id, "@claude @codex hi").unwrap();

        // 按位置找"claude 的那条"和"codex 的那条",不依赖 list 的返回顺序。
        let statuses = || {
            let (m, _) = store.list_messages_after_rev(g.id, 0, 100).unwrap();
            m.into_iter().map(|x| (x.seq, x.status)).collect::<Vec<_>>()
        };
        let claude_seq = 2;
        let codex_seq = 3;
        let status_of = |seq: i64| {
            statuses()
                .into_iter()
                .find(|(s, _)| *s == seq)
                .and_then(|(_, st)| st)
        };
        wait_until_async(|| status_of(claude_seq) == Some(GroupMessageStatus::Running)).await;
        svc.cancel(g.id, GroupCancelScope::Turn);
        wait_until_async(|| status_of(codex_seq) == Some(GroupMessageStatus::Done)).await;
        assert_eq!(status_of(claude_seq), Some(GroupMessageStatus::Cancelled));
    }

    #[tokio::test]
    async fn retry_reruns_only_that_member_with_history_up_to_its_position() {
        let f = fixture(FakeRunner::new(vec![
            Err(TurnError::Empty),
            Ok("重试成功".into()),
        ]));
        let out = f.svc.post(f.gid, "@claude 说说").unwrap();
        wait_until(|| all_settled(&f)).await;
        assert!(matches!(
            f.svc
                .store()
                .get_message(out.placeholders[0].id)
                .unwrap()
                .status,
            Some(GroupMessageStatus::Failed { .. })
        ));
        f.svc.retry(out.placeholders[0].id).unwrap();
        wait_until(|| {
            f.svc
                .store()
                .get_message(out.placeholders[0].id)
                .unwrap()
                .status
                == Some(GroupMessageStatus::Done)
        })
        .await;
        let m = f.svc.store().get_message(out.placeholders[0].id).unwrap();
        assert_eq!(m.text, "重试成功");
        assert_eq!(f.runner.prompts().len(), 2);
    }

    #[tokio::test]
    async fn retry_rejects_non_failed_message() {
        let f = fixture(FakeRunner::new(vec![Ok("好".into())]));
        let out = f.svc.post(f.gid, "@claude").unwrap();
        wait_until(|| all_settled(&f)).await;
        assert!(f.svc.retry(out.placeholders[0].id).is_err());
        assert!(f.svc.retry(out.human.id).is_err());
    }

    /// Review Focus 1:dozerd 重启后遗留的 Queued/Running 不能永远转圈。
    #[tokio::test]
    async fn recover_on_startup_fails_leftovers() {
        let f = fixture(FakeRunner::new(vec![]));
        let (_, ph) = f
            .svc
            .store()
            .post_human_message(f.gid, "@claude @codex", &[f.claude, f.codex])
            .unwrap();
        f.svc.store().try_start(ph[0].id).unwrap();
        f.svc.recover_on_startup();
        for p in ph {
            assert!(matches!(
                f.svc.store().get_message(p.id).unwrap().status,
                Some(GroupMessageStatus::Failed { .. })
            ));
        }
    }

    /// Review Focus 4:删群时有发言进行中。
    #[tokio::test]
    async fn delete_group_cancels_running_turn_and_leaves_no_rows() {
        let f = fixture(FakeRunner::blocking());
        let out = f.svc.post(f.gid, "@claude hi").unwrap();
        wait_until(|| f.runner.prompts().len() == 1).await;
        f.svc.delete_group(f.gid).unwrap();
        assert!(f.svc.store().get_group(f.gid).is_err());
        assert!(f.svc.store().get_message(out.human.id).is_err());
        // 取消信号送达后 worker 收尾:此后对已删群的 finish/fail 都是无害的空操作
        tokio::time::sleep(Duration::from_millis(150)).await;
    }

    #[tokio::test]
    async fn post_rejects_blank_and_oversized_text_and_unknown_group() {
        let f = fixture(FakeRunner::new(vec![]));
        assert!(f.svc.post(f.gid, "   ").is_err());
        assert!(f.svc.post(f.gid, &"字".repeat(MAX_POST_CHARS + 1)).is_err());
        assert!(f.svc.post(9999, "hi").is_err());
    }

    #[tokio::test]
    async fn missing_project_dir_fails_the_turn() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(GroupStore::new(&dir.path().join("g.db")).unwrap());
        let runner = FakeRunner::new(vec![]);
        let svc = GroupService::new(store.clone(), runner.clone(), Arc::new(|_| None));
        let g = store.create_group(1, "t").unwrap();
        let g = store
            .add_member(g.id, AgentKind::Claude, "claude", "")
            .unwrap();
        svc.post(g.id, "@claude hi").unwrap();
        wait_until(|| {
            let (m, _) = store.list_messages_after_rev(g.id, 0, 10).unwrap();
            matches!(m[1].status, Some(GroupMessageStatus::Failed { ref reason }) if reason.contains("项目"))
        })
        .await;
        assert!(runner.prompts().is_empty());
    }
}
