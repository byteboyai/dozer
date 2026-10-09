//! 进程内 `UsageService` 假实现:不起 UDS,返回固定结果并记录调用。
use dozer_services::{
    BoxFuture, ServiceError, ServiceErrorKind, Services, UsageRows, UsageService,
};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub(crate) struct FakeUsage {
    reply: Result<UsageRows, ServiceError>,
    calls: Arc<Mutex<Vec<(String, Option<u64>)>>>,
}

impl FakeUsage {
    pub(crate) fn ok(rows: UsageRows) -> Self {
        Self {
            reply: Ok(rows),
            calls: Default::default(),
        }
    }
    #[allow(dead_code)] // Task 5 起被 Usage 失败场景使用
    pub(crate) fn failing(kind: ServiceErrorKind, message: &str) -> Self {
        Self {
            reply: Err(ServiceError::new(kind, message)),
            calls: Default::default(),
        }
    }
    pub(crate) fn calls(&self) -> Vec<(String, Option<u64>)> {
        self.calls.lock().unwrap().clone()
    }
    pub(crate) fn into_services(self) -> Services {
        Services {
            usage: Arc::new(self),
        }
    }
}

impl UsageService for FakeUsage {
    fn usage_summary(
        &self,
        cwd: String,
        since_ts: Option<u64>,
    ) -> BoxFuture<'_, Result<UsageRows, ServiceError>> {
        self.calls.lock().unwrap().push((cwd, since_ts));
        let r = self.reply.clone();
        Box::pin(async move { r })
    }
}
