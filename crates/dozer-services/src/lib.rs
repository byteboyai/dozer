//! 面板服务 trait 接缝(X0,spec 2026-10-09)。面板只依赖这里的小 trait 和 `dozer-model`,
//! 不知道对面是 dozerd 还是内存假实现。生产实现在 `dozer-client`(为 `Client` 适配)。

use dozer_model::{ConversationSummary, UsagePayload};
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// trait 方法返回的装箱 future(`async fn in trait` 不能做 trait object)。`Send` 以便在
/// `tokio::spawn`/`Handle::spawn` 里使用。
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceErrorKind {
    /// 连不上或连接中途断开。
    Unavailable,
    /// 服务端明确拒绝(dozerd 回了 `Reply::Error`)。
    Rejected,
    /// 意外应答或解码失败。
    Protocol,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceError {
    pub kind: ServiceErrorKind,
    pub message: String,
}

impl ServiceError {
    pub fn new(kind: ServiceErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl fmt::Display for ServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ServiceError {}

pub type UsageRows = Vec<(ConversationSummary, UsagePayload)>;

/// Usage 面板取数。参数按值传,future 因此不借用调用方的局部变量。
pub trait UsageService: Send + Sync {
    fn usage_summary(
        &self,
        cwd: String,
        since_ts: Option<u64>,
    ) -> BoxFuture<'_, Result<UsageRows, ServiceError>>;
}

/// 各能力 trait 的 `Arc<dyn>` 束;host 构造,经 `PanelIo` 交给面板。其余能力 trait 随各面板迁移逐个加入。
#[derive(Clone)]
pub struct Services {
    pub usage: Arc<dyn UsageService>,
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    struct Mem(Result<UsageRows, ServiceError>);
    impl UsageService for Mem {
        fn usage_summary(
            &self,
            _cwd: String,
            _since: Option<u64>,
        ) -> BoxFuture<'_, Result<UsageRows, ServiceError>> {
            let r = self.0.clone();
            Box::pin(async move { r })
        }
    }

    #[test]
    fn display_is_the_message_only() {
        let e = ServiceError::new(ServiceErrorKind::Rejected, "daemon 错误: boom");
        assert_eq!(e.to_string(), "daemon 错误: boom");
        assert_eq!(e.kind, ServiceErrorKind::Rejected);
    }

    /// `handle.spawn` 要 `'static + Send`:持有 `Arc<dyn UsageService>` 的 async 块必须满足。
    #[test]
    fn dyn_service_future_runs_inside_tokio_spawn() {
        let svc: Arc<dyn UsageService> = Arc::new(Mem(Ok(vec![])));
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let out = rt.block_on(async move {
            tokio::spawn(async move { svc.usage_summary("/p".into(), None).await })
                .await
                .unwrap()
        });
        assert_eq!(out, Ok(vec![]));
    }

    #[test]
    fn failure_kinds_are_distinguishable_from_empty_success() {
        let ok: Result<UsageRows, ServiceError> = Ok(vec![]);
        let un: Result<UsageRows, ServiceError> =
            Err(ServiceError::new(ServiceErrorKind::Unavailable, "x"));
        assert!(ok.is_ok() && un.is_err());
        assert_ne!(ServiceErrorKind::Unavailable, ServiceErrorKind::Protocol);
    }

    #[test]
    fn services_is_cloneable_and_shares_the_service() {
        let s = Services {
            usage: Arc::new(Mem(Ok(vec![]))),
        };
        let t = s.clone();
        assert!(Arc::ptr_eq(&s.usage, &t.usage));
    }
}
