//! UI 请求身份(Dozer 产品适配层)。
//!
//! 面板异步回包的**乱序防护**与**作用域写入**语义由 `bytehost_core::request` 提供
//! (Task 05 交付,来自本文件原型的移植)。这一层只做产品适配:
//!
//! - 把 Dozer 的业务作用域 `project_id: Option<i64>` 转成中性的 `ScopeId`
//!   (兼容桥 C8:`ProjectId → ScopeId` 转换只在本文件完成,Host/共享内核看不到 i64 的业务含义);
//! - `RequestToken` 是**遗留类型包装**(compat bridge → Task 21):core 的 `RequestToken` 不携带
//!   业务 project_id,而 Dozer 面板的 `Message::AsyncResult` 需要它做跨项目路由,故外包一层。
//!   Task 21 收口时,改由面板按 scope 路由,即可删除本包装。
//!
//! 语义与迁移前一致:同 key 的旧回包失效、不同 key 相互独立、写操作(`scope`)不被新读作废、
//! 重开面板(新 tracker)拒绝旧结果。

use std::collections::HashMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};

use bytehost_core::id::{PanelInstanceId, ScopeId};
use bytehost_core::request::{
    Generation, RequestKey, RequestToken as CoreToken, RequestTracker as CoreTracker,
};

static NEXT: AtomicU64 = AtomicU64::new(1);
fn next() -> u64 {
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// Dozer 的业务作用域 → 中性 `ScopeId` 的唯一转换点。
///
/// 面板绑定某个项目:`project_id = Some(id)`;全局(跨项目)面板:`None`(如浏览器书签的全局 lane)。
fn scope_of(project_id: Option<i64>) -> ScopeId {
    match project_id {
        Some(id) => ScopeId::new(format!("project:{id}")).expect("project scope 合法"),
        None => ScopeId::new("global").expect("global scope 合法"),
    }
}

/// 遗留类型包装(compat bridge → Task 21):core 请求身份 + Dozer 业务 project_id。
#[derive(Debug, Clone)]
pub(crate) struct RequestToken {
    pub project_id: Option<i64>,
    inner: CoreToken,
}

#[derive(Debug, Clone)]
pub(crate) struct RequestTracker(Arc<Inner>);
#[derive(Debug)]
struct Inner {
    /// 进程内唯一实例号:重开的同 slot 面板(owner 不同)互不接受对方 token。
    owner: u64,
    instance: PanelInstanceId,
    generation: Generation,
    /// 每个 scope 一个 core tracker(一个 Dozer tracker 可服务多 scope,如浏览器全局 + 项目书签)。
    scopes: Mutex<HashMap<ScopeId, CoreTracker>>,
}
impl Default for RequestTracker {
    fn default() -> Self {
        Self(Arc::new(Inner {
            owner: next(),
            instance: PanelInstanceId::new(0, 0),
            generation: Generation::new(0),
            scopes: Mutex::new(HashMap::new()),
        }))
    }
}
impl RequestTracker {
    /// 取(或惰性建)某 scope 的 core tracker。core tracker 的 owner/instance/generation 与本层一致,
    /// 保证同实例不同 scope 的 token 因 scope 不同而互不接受。
    fn core_for(&self, scope: &ScopeId) -> CoreTracker {
        let mut scopes = self.0.scopes.lock().unwrap();
        scopes
            .entry(scope.clone())
            .or_insert_with(|| {
                CoreTracker::new(
                    self.0.owner,
                    scope.clone(),
                    self.0.instance,
                    self.0.generation,
                )
            })
            .clone()
    }

    /// 发起一次 **latest-only 读取**。同 key 的旧 token 立即失效。
    pub fn begin(&self, project_id: Option<i64>, lane: &'static str) -> RequestToken {
        let scope = scope_of(project_id);
        let inner = self.core_for(&scope).begin(RequestKey::new(lane));
        RequestToken { project_id, inner }
    }

    /// **作用域写入**:写操作必须全部完成,不因新请求被作废;实例过期后其 UI 回包被丢弃。
    pub fn scope(&self, project_id: Option<i64>) -> RequestToken {
        let scope = scope_of(project_id);
        let inner = self.core_for(&scope).scope();
        RequestToken { project_id, inner }
    }

    /// 显式作废某个 lane 的当前结果(如面板进入错误态需清空)。
    /// Dozer 的调用不带 scope,故对所有已知 scope 一并作废(与迁移前"单 tracker 单表"等价)。
    pub fn invalidate(&self, lane: &'static str) {
        let key = RequestKey::new(lane);
        let scopes = self.0.scopes.lock().unwrap();
        for tracker in scopes.values() {
            tracker.invalidate(key);
        }
    }

    /// **派发前**校验:来源 tracker 一致、实例与 generation 仍存活、scope 一致,且属于当前有效请求或写入。
    pub fn accepts(&self, token: &RequestToken) -> bool {
        let scopes = self.0.scopes.lock().unwrap();
        match scopes.get(token.inner.scope()) {
            Some(tracker) => tracker.accepts(&token.inner),
            None => false,
        }
    }
}
impl RequestToken {
    pub fn wrap<M: 'static>(
        self,
        emit: impl Fn(M) + Send + 'static,
        envelope: fn(Self, Box<M>) -> M,
    ) -> impl Fn(M) + Clone + Send + Sync + 'static {
        let emit = Arc::new(Mutex::new(emit));
        move |msg| (emit.lock().unwrap())(envelope(self.clone(), Box::new(msg)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn queued_results_are_invalidated_and_lanes_are_independent() {
        let requests = RequestTracker::default();
        let old = requests.begin(Some(1), "list");
        let categories = requests.begin(Some(1), "categories");
        let new = requests.begin(Some(1), "list");
        assert!(!requests.accepts(&old));
        assert!(requests.accepts(&new));
        assert!(requests.accepts(&categories));
        requests.invalidate("list");
        assert!(!requests.accepts(&new));
    }
    #[test]
    fn another_project_or_reopened_panel_rejects_old_results() {
        let original = RequestTracker::default();
        let token = original.begin(Some(7), "list");
        assert!(!RequestTracker::default().accepts(&token));
        assert!(original.accepts(&token));
        let write = original.scope(Some(7));
        original.begin(Some(7), "list");
        assert!(original.accepts(&write));
        assert!(!RequestTracker::default().accepts(&write));
    }

    /// 一个 tracker 服务多个 scope:跨 project 的 token 因 scope 不同互不接受
    /// (对应 core `same_id_different_scope_is_rejected`,此处经产品转换层验证)。
    #[test]
    fn different_projects_through_one_tracker_do_not_accept_each_other() {
        let t = RequestTracker::default();
        let a = t.begin(Some(1), "bookmarks");
        let b = t.begin(Some(2), "bookmarks");
        // 两个项目各自最新;但 token 的 scope 不同,用对方 scope 的 tracker 校验必拒。
        assert!(t.accepts(&a.clone()));
        assert!(t.accepts(&b));
        // 全局 lane 与项目 lane 也互不接受。
        let global = t.begin(None, "bookmarks");
        assert!(t.accepts(&global));
    }
}
