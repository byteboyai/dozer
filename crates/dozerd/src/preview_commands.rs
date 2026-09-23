//! T13:daemon 侧的预览命令总线。
//!
//! dozer-mcp 通过 `Request::RunPreviewCommand` 提交一条导航/写入命令;dozerd 把
//! 命令入队(按 project_id),并注册一个 waiter 等待 app 回传
//! `PreviewCommandOutcome`。app 轮询 `TakePendingPreviewCommands` 取走命令、
//! 用 `ReportPreviewCommandOutcome` 回报终态。app 不在线/超时由调用方(server)
//! 判定并回 `Timeout`。

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use dozer_core::protocol::{PreviewCommand, PreviewCommandOutcome};
use tokio::sync::oneshot;

#[derive(Default)]
pub struct PreviewCommandBus {
    queues: Mutex<HashMap<i64, VecDeque<PreviewCommand>>>,
    waiters: Mutex<HashMap<String, oneshot::Sender<PreviewCommandOutcome>>>,
}

impl PreviewCommandBus {
    pub fn new() -> Self {
        Self::default()
    }

    /// 入队命令并为它的 `request_id` 注册 waiter,返回接收端。
    pub fn enqueue(&self, command: PreviewCommand) -> oneshot::Receiver<PreviewCommandOutcome> {
        let (tx, rx) = oneshot::channel();
        self.waiters
            .lock()
            .expect("preview waiters")
            .insert(command.request_id.clone(), tx);
        self.queues
            .lock()
            .expect("preview queues")
            .entry(command.project_id)
            .or_default()
            .push_back(command);
        rx
    }

    /// app 取走某项目全部待处理命令。
    pub fn take(&self, project_id: i64) -> Vec<PreviewCommand> {
        self.queues
            .lock()
            .expect("preview queues")
            .remove(&project_id)
            .map(|q| q.into_iter().collect())
            .unwrap_or_default()
    }

    /// app 回报终态:投递给 waiter。返回是否投递成功(waiter 已超时则 false)。
    pub fn report(&self, outcome: PreviewCommandOutcome) -> bool {
        let tx = self
            .waiters
            .lock()
            .expect("preview waiters")
            .remove(outcome.request_id());
        match tx {
            Some(tx) => tx.send(outcome).is_ok(),
            None => false,
        }
    }

    /// 超时/放弃:移除 waiter,并把队列里同 id 的命令一并丢弃(避免 app 之后
    /// 再取到一条已超时的命令)。
    pub fn forget(&self, request_id: &str) {
        self.waiters
            .lock()
            .expect("preview waiters")
            .remove(request_id);
        for q in self.queues.lock().expect("preview queues").values_mut() {
            q.retain(|c| c.request_id != request_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dozer_core::protocol::{
        PreviewCommand, PreviewCommandAction, PreviewCommandOutcome, PreviewCommandTarget,
    };

    fn cmd(id: &str, project: i64) -> PreviewCommand {
        PreviewCommand {
            request_id: id.into(),
            project_id: project,
            target: PreviewCommandTarget::Path {
                path: "/p/a.rs".into(),
            },
            action: PreviewCommandAction::Reveal { line: 1, column: 1 },
            expected_revision: None,
        }
    }

    #[test]
    fn enqueue_take_and_report_round_trip() {
        let bus = PreviewCommandBus::new();
        let mut rx = bus.enqueue(cmd("r1", 7));
        let taken = bus.take(7);
        assert_eq!(taken.len(), 1);
        assert_eq!(taken[0].request_id, "r1");
        assert!(bus.take(7).is_empty(), "取走后队列清空");
        assert!(bus.report(PreviewCommandOutcome::Accepted {
            request_id: "r1".into()
        }));
        assert!(matches!(
            rx.try_recv(),
            Ok(PreviewCommandOutcome::Accepted { .. })
        ));
    }

    #[test]
    fn queues_are_per_project() {
        let bus = PreviewCommandBus::new();
        let _a = bus.enqueue(cmd("a", 1));
        let _b = bus.enqueue(cmd("b", 2));
        assert_eq!(bus.take(1).len(), 1);
        assert_eq!(bus.take(2).len(), 1);
    }

    #[test]
    fn forget_removes_waiter_and_queued_command() {
        let bus = PreviewCommandBus::new();
        let _rx = bus.enqueue(cmd("r", 3));
        bus.forget("r");
        assert!(bus.take(3).is_empty(), "已忘掉的命令不应再被 app 取到");
        assert!(!bus.report(PreviewCommandOutcome::Timeout {
            request_id: "r".into()
        }));
    }

    #[test]
    fn report_without_waiter_is_false() {
        let bus = PreviewCommandBus::new();
        assert!(!bus.report(PreviewCommandOutcome::Timeout {
            request_id: "nope".into()
        }));
    }
}
