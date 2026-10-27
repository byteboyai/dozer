//! 仅测试用:最小 `BookmarkService` 假实现与小型 tokio 运行时(行为同 `dozer-app::service_fakes::FakeBookmark`)。

use dozer_model::{BookmarkInfo, BookmarkScope};
use dozer_services::{BookmarkService, BoxFuture, ServiceError, ServiceErrorKind};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub(crate) const TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap()
}

#[derive(Default)]
struct BookmarkState {
    items: Vec<BookmarkInfo>,
    list_error: Option<ServiceError>,
    write_error: Option<ServiceError>,
    calls: Vec<String>,
}

/// 克隆共享同一份状态。`Default` = 全部成功、列表为空。
#[derive(Clone, Default)]
pub(crate) struct FakeBookmark {
    state: Arc<Mutex<BookmarkState>>,
}

impl FakeBookmark {
    pub(crate) fn with_items(self, items: Vec<BookmarkInfo>) -> Self {
        self.state.lock().unwrap().items = items;
        self
    }
    pub(crate) fn with_list_error(self, kind: ServiceErrorKind, message: &str) -> Self {
        self.state.lock().unwrap().list_error = Some(ServiceError::new(kind, message));
        self
    }
    /// add 与 remove 共用。
    #[allow(dead_code)]
    pub(crate) fn with_write_error(self, kind: ServiceErrorKind, message: &str) -> Self {
        self.state.lock().unwrap().write_error = Some(ServiceError::new(kind, message));
        self
    }
    pub(crate) fn calls(&self) -> Vec<String> {
        self.state.lock().unwrap().calls.clone()
    }
    pub(crate) fn into_service(self) -> Arc<dyn BookmarkService> {
        Arc::new(self)
    }
}

impl BookmarkService for FakeBookmark {
    fn list_bookmarks(
        &self,
        project_id: Option<i64>,
    ) -> BoxFuture<'_, Result<Vec<BookmarkInfo>, ServiceError>> {
        let mut s = self.state.lock().unwrap();
        s.calls.push(format!("list_bookmarks({project_id:?})"));
        let r = match &s.list_error {
            Some(e) => Err(e.clone()),
            None => Ok(s.items.clone()),
        };
        Box::pin(async move { r })
    }
    fn add_bookmark(
        &self,
        scope: BookmarkScope,
        project_id: Option<i64>,
        url: String,
        title: String,
    ) -> BoxFuture<'_, Result<(), ServiceError>> {
        let mut s = self.state.lock().unwrap();
        s.calls.push(format!(
            "add_bookmark({scope:?},{project_id:?},{url},{title})"
        ));
        let r = match &s.write_error {
            Some(e) => Err(e.clone()),
            None => Ok(()),
        };
        Box::pin(async move { r })
    }
    fn remove_bookmark(&self, id: i64) -> BoxFuture<'_, Result<(), ServiceError>> {
        let mut s = self.state.lock().unwrap();
        s.calls.push(format!("remove_bookmark({id})"));
        let r = match &s.write_error {
            Some(e) => Err(e.clone()),
            None => Ok(()),
        };
        Box::pin(async move { r })
    }
}
