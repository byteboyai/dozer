//! Dozer 服务构造适配:把 `dozer_client::Client` 适配成 `dozer_services::Services`。
//!
//! 从 `panel_host.rs` 移出(兼容桥 C4):服务构造是 Dozer 作为产品组合自己的事,
//! 不属于宿主只读视图/执行原语。`ensure_running` 要拉起 dozerd 进程(`runtime::ensure_daemon`),
//! 那段在 dozer-app,`dozer-client` 够不着,所以 `ClientDaemon` 适配器放在这一层。

use dozer_client::Client;
use dozer_services::Services;
use std::sync::Arc;

/// 默认的服务束:各能力都由 `Client` 适配。host 的其它入口(`App` 的消息处理/轮询)也用它,
/// 保证"面板拿到的服务"只有一处构造。
pub(crate) fn services_for(client: &Client) -> Services {
    Services {
        usage: Arc::new(client.clone()),
        todo: Arc::new(client.clone()),
        project: Arc::new(client.clone()),
        memory: Arc::new(client.clone()),
        group_chat: Arc::new(client.clone()),
        conversation: Arc::new(client.clone()),
        codehealth: Arc::new(client.clone()),
        context: Arc::new(client.clone()),
        edit_history: Arc::new(client.clone()),
        bookmark: Arc::new(client.clone()),
        locale: Arc::new(client.clone()),
        daemon: Arc::new(ClientDaemon(client.clone())),
    }
}

/// 把 `Client` 适配成 `DaemonService`。`ensure_running` 要拉起 dozerd 进程(`runtime::ensure_daemon`),
/// 那段在 dozer-app,`dozer-client` 够不着,所以适配器放在这里。
pub(crate) struct ClientDaemon(pub(crate) Client);

/// 只测试用的离线服务束:`Client` 指向不存在的 socket,任何请求都失败为 `Unavailable`。
/// 让测试可以直接 `PanelIo::new(offline_services(), ..)`,无需先造 `Client` 再覆盖(兼容桥 C3)。
#[cfg(test)]
pub(crate) fn offline_services() -> Services {
    services_for(&Client::new(std::path::PathBuf::from(
        "/tmp/dozer-panel-io-test-nonexistent.sock",
    )))
}

fn ensure_error_to_service(msg: String) -> dozer_services::ServiceError {
    dozer_services::ServiceError::new(dozer_services::ServiceErrorKind::Unavailable, msg)
}

impl dozer_services::DaemonService for ClientDaemon {
    fn list_sessions(
        &self,
    ) -> dozer_services::BoxFuture<
        '_,
        Result<Vec<dozer_model::SessionInfo>, dozer_services::ServiceError>,
    > {
        Box::pin(async move {
            self.0
                .list()
                .await
                .map_err(dozer_client::into_service_error)
        })
    }
    fn shutdown_daemon(
        &self,
    ) -> dozer_services::BoxFuture<'_, Result<(), dozer_services::ServiceError>> {
        Box::pin(async move {
            self.0
                .shutdown_daemon()
                .await
                .map_err(dozer_client::into_service_error)
        })
    }
    fn ensure_running(
        &self,
    ) -> dozer_services::BoxFuture<'_, Result<(), dozer_services::ServiceError>> {
        Box::pin(async move {
            crate::runtime::ensure_daemon(&self.0)
                .await
                .map_err(ensure_error_to_service)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dozer_services::{DaemonService, ServiceErrorKind};

    fn offline() -> ClientDaemon {
        ClientDaemon(Client::new(std::path::PathBuf::from(
            "/tmp/dozer-settings-daemon-nonexistent.sock",
        )))
    }

    #[test]
    fn client_daemon_reports_unavailable_when_the_socket_is_missing() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let d = offline();
        let e = rt.block_on(d.list_sessions()).unwrap_err();
        assert_eq!(e.kind, ServiceErrorKind::Unavailable);
        let e = rt.block_on(d.shutdown_daemon()).unwrap_err();
        assert_eq!(e.kind, ServiceErrorKind::Unavailable);
    }

    #[test]
    fn ensure_error_text_is_preserved_verbatim() {
        let e = ensure_error_to_service("无法连接 dozerd（已重试 3 次）：x".into());
        assert_eq!(e.kind, ServiceErrorKind::Unavailable);
        assert_eq!(e.message, "无法连接 dozerd（已重试 3 次）：x");
        assert_eq!(e.to_string(), "无法连接 dozerd（已重试 3 次）：x");
    }
}
