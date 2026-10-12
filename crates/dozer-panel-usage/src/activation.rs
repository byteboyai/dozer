//! 面板"切入"(被用户切到前台)时宿主交给它的上下文。库自带一份,不依赖
//! 产品的 `panel_host`(计划 §Task 19 第 5 条:依赖闭包不含 dozer-app)。
//! 产品在 `PanelSelect` 分支构造它并转发给 [`crate::state::on_activate`]。

use crate::adapter::GitStatsProvider;
use crate::state::Message;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::runtime::Handle;

/// 面板切入上下文:当前项目 id、项目路径(项目还没有路径时为 `None`)、
/// dozerd 的用量服务句柄、Git 统计提供者、宿主 runtime 句柄,以及"把结果包成
/// 该面板 `Message` 投回事件循环"的发射器。
pub struct ActivationCtx<M> {
    pub project_id: i64,
    pub project_path: Option<PathBuf>,
    pub usage: Arc<dyn dozer_services::UsageService>,
    pub git: Arc<dyn GitStatsProvider>,
    pub handle: Handle,
    emit: Arc<dyn Fn(M) + Send + Sync + 'static>,
}

impl<M> Clone for ActivationCtx<M> {
    fn clone(&self) -> Self {
        Self {
            project_id: self.project_id,
            project_path: self.project_path.clone(),
            usage: Arc::clone(&self.usage),
            git: Arc::clone(&self.git),
            handle: self.handle.clone(),
            emit: Arc::clone(&self.emit),
        }
    }
}

impl ActivationCtx<Message> {
    pub fn new(
        project_id: i64,
        project_path: Option<PathBuf>,
        usage: Arc<dyn dozer_services::UsageService>,
        git: Arc<dyn GitStatsProvider>,
        handle: Handle,
        emit: impl Fn(Message) + Send + Sync + 'static,
    ) -> Self {
        Self {
            project_id,
            project_path,
            usage,
            git,
            handle,
            emit: Arc::new(emit),
        }
    }

    /// 一个可 `clone`、可跨线程的 `emit` 闭包(`spawn_refresh` 需要 `Fn(M) + Send + 'static`)。
    pub fn emitter(&self) -> impl Fn(Message) + Clone + Send + 'static {
        let emit = Arc::clone(&self.emit);
        move |message| emit(message)
    }
}

#[cfg(test)]
pub mod testing {
    use super::*;
    use crate::adapter::NoGitStats;
    use dozer_model::{ConversationSummary, UsagePayload};
    use dozer_services::{ServiceError, ServiceErrorKind, UsageRows};
    use std::sync::{Mutex, mpsc};
    use std::time::Duration;

    pub const TIMEOUT: Duration = Duration::from_secs(5);

    pub fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap()
    }

    /// 一个永远返回 `Unavailable` 的假 `UsageService`(离线夹具)。
    struct OfflineUsage;

    impl dozer_services::UsageService for OfflineUsage {
        fn usage_summary(
            &self,
            _cwd: String,
            _since_ts: Option<u64>,
        ) -> dozer_services::BoxFuture<'_, Result<UsageRows, ServiceError>> {
            Box::pin(async { Err(ServiceError::new(ServiceErrorKind::Unavailable, "offline")) })
        }
    }

    type FakeCalls = Arc<Mutex<Vec<(String, Option<u64>)>>>;

    /// 返回固定行的假 `UsageService`。
    pub struct FakeUsage {
        rows: Vec<(ConversationSummary, UsagePayload)>,
        calls: FakeCalls,
    }

    impl FakeUsage {
        pub fn ok(rows: Vec<(ConversationSummary, UsagePayload)>) -> Self {
            Self {
                rows,
                calls: Arc::new(Mutex::new(Vec::new())),
            }
        }

        pub fn calls(&self) -> Vec<(String, Option<u64>)> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl dozer_services::UsageService for FakeUsage {
        fn usage_summary(
            &self,
            cwd: String,
            since_ts: Option<u64>,
        ) -> dozer_services::BoxFuture<'_, Result<UsageRows, ServiceError>> {
            self.calls.lock().unwrap().push((cwd, since_ts));
            let rows = self.rows.clone();
            Box::pin(async move { Ok(rows) })
        }
    }

    /// 离线 ctx:用量服务恒失败、Git 为 `NoGitStats`,`emit` 收进 channel。
    pub fn offline_ctx<M: Send + 'static>(
        rt: &tokio::runtime::Runtime,
        project_id: i64,
        project_path: Option<PathBuf>,
    ) -> (ActivationCtx<M>, mpsc::Receiver<M>) {
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let ctx = ActivationCtx {
            project_id,
            project_path,
            usage: Arc::new(OfflineUsage),
            git: Arc::new(NoGitStats),
            handle: rt.handle().clone(),
            emit: Arc::new(move |m| {
                let _ = tx.lock().unwrap().send(m);
            }),
        };
        (ctx, rx)
    }

    /// 用假用量服务替换离线 ctx 的 `usage` 字段(其余不变)。
    pub fn with_usage(
        ctx: ActivationCtx<Message>,
        usage: Arc<dyn dozer_services::UsageService>,
    ) -> ActivationCtx<Message> {
        ActivationCtx { usage, ..ctx }
    }
}
