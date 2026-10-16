//! 面板宿主件的共享库:请求追踪、`PanelIo` / `ActivationCtx`、宿主需求发件箱、中性提示发件箱。
//!
//! 依赖闭包刻意**不含** dozer-app / dozer-client / dozer-core:Dozer 与 Digger 的共享面板库(`dozer-panel-todo` 起)
//! 都依赖它,而不是各自复制一份。逐字搬自 `dozer-app/src/panel_host.rs`(只改可见性),Dozer 的 `panel_host`
//! 重新导出;依赖 `App` 的 `PanelHost` 视图契约 trait 仍留在 Dozer。

mod drag;
mod notice;
pub use drag::{
    TREE_DRAG_CONFIRM_THRESHOLD_PX, TREE_DRAG_MIN_HOLD_DURATION, tree_drag_held_long_enough,
    tree_drag_past_threshold,
};
pub use notice::{NOTICE_COOLDOWN, Notice, NoticeLevel, NoticeOutbox};

use bytehost_core::id::ScopeId;
use dozer_services::Services;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::runtime::Handle;

/// Dozer 的业务作用域 → 中性 `ScopeId` 的**唯一转换点**(compat bridge C8 收口)。
///
/// 面板绑定某个项目:`project_id = Some(id)`;全局(跨项目)面板:`None`(如浏览器书签的全局 lane)。
/// Host/共享内核看不到 `ScopeId` 里的业务含义——只有本模块知道 `"project:{id}"` 该怎么解。
pub fn scope_of(project_id: Option<i64>) -> ScopeId {
    match project_id {
        Some(id) => ScopeId::new(format!("project:{id}")).expect("project scope 合法"),
        None => ScopeId::new("global").expect("global scope 合法"),
    }
}

/// [`scope_of`] 的逆:从结果回包的 token scope 还原业务 `project_id`。
///
/// 非 `"project:{N}"`(如全局 lane)返回 `None`。这是**产品路由**层的唯一解析点——
/// 回包 `Message::AsyncResult` 不再携带 `project_id`,改由 scope 反解,故 Host 侧无业务字段。
pub fn project_of(scope: &ScopeId) -> Option<i64> {
    scope.as_str().strip_prefix("project:")?.parse().ok()
}

/// 把一条面板内层消息包成 `envelope(token, Box<M>)`:后台结果回包时自动带上请求身份。
/// (原 `request_tracker::RequestToken::wrap` 的实现移到这里——core `RequestToken` 无此方法,
/// 且它本就属于 host 的 `PanelIo` 收口,而非请求身份本身。)
pub fn wrap_token<M: 'static>(
    token: bytehost_core::request::RequestToken,
    emit: impl Fn(M) + Send + 'static,
    envelope: fn(bytehost_core::request::RequestToken, Box<M>) -> M,
) -> impl Fn(M) + Clone + Send + Sync + 'static {
    let emit = Arc::new(std::sync::Mutex::new(emit));
    move |msg| (emit.lock().unwrap())(envelope(token.clone(), Box::new(msg)))
}

/// `CoreToken::wrap(...)` 的产品侧扩展:让面板沿用 `.begin(..).wrap(emit, Message::AsyncResult)`
/// 的链式写法不变(实现就是 [`wrap_token`])。C8 删除产品 `RequestToken` 包装后,面板不再能直接
/// 调 `.wrap`,改由此扩展 trait 提供。
pub trait RequestTokenWrapExt: Sized + Clone + Send + Sync + 'static {
    fn wrap<M: 'static>(
        self,
        emit: impl Fn(M) + Send + 'static,
        envelope: fn(Self, Box<M>) -> M,
    ) -> impl Fn(M) + Clone + Send + Sync + 'static;
}

impl RequestTokenWrapExt for bytehost_core::request::RequestToken {
    fn wrap<M: 'static>(
        self,
        emit: impl Fn(M) + Send + 'static,
        envelope: fn(Self, Box<M>) -> M,
    ) -> impl Fn(M) + Clone + Send + Sync + 'static {
        wrap_token(self, emit, envelope)
    }
}

/// 面板请求身份的进程内唯一 owner 分配器。
///
/// 每个面板 state 构造一份 `ScopedTrackers` 时取一个新 owner,使**重开的同 slot 面板**
/// (owner 不同)互不接受对方在途结果——语义与迁移前 `request_tracker.rs` 的 `NEXT` 一致。
pub fn next_request_owner() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// 给面板 state 构造一份请求身份集合(多 scope)。这是产品侧对
/// [`bytehost_core::request::ScopedTrackers`] 的唯一构造封装——面板不再自持包装类型。
pub fn new_request_trackers() -> bytehost_core::request::ScopedTrackers {
    use bytehost_core::id::PanelInstanceId;
    use bytehost_core::request::Generation;
    bytehost_core::request::ScopedTrackers::new(
        next_request_owner(),
        PanelInstanceId::new(0, 0),
        Generation::new(0),
    )
}

/// 面板 state 持有的请求身份集合的产品包装。
///
/// 它 `Default`(每次新建取一个新 owner),故面板 state 可以继续 `#[derive(Default)]`;
/// 其余行为直接转发到中立的 [`bytehost_core::request::ScopedTrackers`]。scope 由产品按
/// `project_id` 经 [`scope_of`] 给出,取回时经 [`project_of`] 还原,Host 不见业务字段。
#[derive(Debug, Clone)]
pub struct RequestTrackers(bytehost_core::request::ScopedTrackers);

impl Default for RequestTrackers {
    fn default() -> Self {
        Self(new_request_trackers())
    }
}

impl RequestTrackers {
    pub fn begin(
        &self,
        project_id: Option<i64>,
        lane: &'static str,
    ) -> bytehost_core::request::RequestToken {
        self.0.begin(
            &scope_of(project_id),
            bytehost_core::request::RequestKey::new(lane),
        )
    }

    pub fn scope(&self, project_id: Option<i64>) -> bytehost_core::request::RequestToken {
        self.0.scope(&scope_of(project_id))
    }

    pub fn invalidate(&self, lane: &'static str) {
        self.0
            .invalidate(bytehost_core::request::RequestKey::new(lane));
    }

    pub fn accepts(&self, token: &bytehost_core::request::RequestToken) -> bool {
        self.0.accepts(token)
    }
}

/// 面板"切入"(被用户切到前台)时 host 交给它的上下文:当前项目 id、项目路径(项目还没有路径时为
/// `Handle`(tokio runtime)、"把结果包成该面板的 `Message` 投回事件循环"。收成一个值,由 host
/// 构造、传给面板模块里的执行器(`group_chat::run_effect` 等);host 不再认识面板的具体 Effect。
///
/// 与 [`PanelHost`](只读视图契约)是一对:`PanelHost` 管"读 host 状态来画 view",`PanelIo` 管
/// "让 host 替我跑东西"。是具体结构体而不是 trait(规格 §1:不为未来的第二个宿主预先抽象);
/// 测试里用 `PanelIo::new` 配一个收进 channel 的 `emit` 即可。
pub struct PanelIo<M> {
    services: Services,
    handle: Handle,
    emit: Arc<dyn Fn(M) + Send + Sync + 'static>,
}

impl<M> Clone for PanelIo<M> {
    fn clone(&self) -> Self {
        Self {
            services: self.services.clone(),
            handle: self.handle.clone(),
            emit: Arc::clone(&self.emit),
        }
    }
}

impl<M: Send + 'static> PanelIo<M> {
    /// 用**已构造好的服务束**构造执行原语。产品组合根经
    /// `host_adapters::services::services_for` 把 `Client` 适配成 `Services` 后传入
    /// (兼容桥 C3:fake 直接在测试里给 `Services`,不再"先构造离线 `Client` 再覆盖")。
    pub fn new(
        services: Services,
        handle: Handle,
        emit: impl Fn(M) + Send + Sync + 'static,
    ) -> Self {
        Self {
            services,
            handle,
            emit: Arc::new(emit),
        }
    }

    /// host 给面板的服务束(由产品组合根构造;测试可直接给进程内假实现)。
    pub fn services(&self) -> &Services {
        &self.services
    }

    /// 仅测试替换服务束用。
    #[allow(dead_code)]
    pub fn with_services(mut self, services: Services) -> Self {
        self.services = services;
        self
    }

    pub fn with_request(
        &self,
        token: bytehost_core::request::RequestToken,
        envelope: fn(bytehost_core::request::RequestToken, Box<M>) -> M,
    ) -> Self {
        Self {
            services: self.services.clone(),
            handle: self.handle.clone(),
            emit: Arc::new(wrap_token(token, self.emitter(), envelope)),
        }
    }

    /// 同步投回一条面板消息。
    pub fn emit(&self, message: M) {
        (self.emit)(message);
    }

    /// host 的 tokio runtime 句柄(给仍然吃 `(client, handle, emit)` 三件套的面板 `spawn_*` 函数用)。
    pub fn handle(&self) -> &Handle {
        &self.handle
    }

    /// 一个可 `clone`、可跨线程的 `emit` 闭包(同上,给 `spawn_*(.., emit: impl Fn(M) + Send + 'static)`)。
    pub fn emitter(&self) -> impl Fn(M) + Clone + Send + 'static {
        let emit = Arc::clone(&self.emit);
        move |message| emit(message)
    }

    /// 在 host 的 runtime 上跑一个后台任务。任务拿到 host 的服务束 `Services` 和一份 `PanelIo` 副本,
    /// 想发多少条消息(包括零条:只在失败时才发的命令)由任务自己决定。
    pub fn spawn<F, Fut>(&self, task: F)
    where
        F: FnOnce(Services, PanelIo<M>) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let services = self.services.clone();
        let io = self.clone();
        self.handle.spawn(async move { task(services, io).await });
    }
}

/// 面板"切入"(被用户切到前台)时 host 交给它的上下文:当前项目 id、项目路径(项目还没有路径时为
/// `None`)和执行原语。面板的 `on_activate` 钩子只依赖它,不碰 `App`/`Workspace`。
pub struct ActivationCtx<M> {
    pub project_id: i64,
    pub project_path: Option<PathBuf>,
    pub io: PanelIo<M>,
}

/// 面板之间"我想让另一个面板做点事"的**受限词汇**(设计文档 P1,用户 2026-10-04 裁决:面板不依赖
/// host 的总 `Message`,也不互相引用)。载荷只用通用类型(路径、布尔),不出现任何面板的业务类型;
/// 由 host 把命令翻译成对目标面板/弹窗的具体动作(`App::run_panel_command`)。
#[derive(Debug, Clone, PartialEq)]
pub enum PanelCommand {
    /// 在某个路径下发起搜索(`is_dir` 决定按目录递归还是只搜单文件)。
    SearchIn { path: PathBuf, is_dir: bool },
    /// 打开某个文件(绝对路径)的 git 历史。
    ShowFileHistory { path: PathBuf },
}

/// 面板向 host 提的需求——只有 host 才能做的事(系统对话框、跨面板动作)。`M` 是提需求的那个面板
/// 自己的 `Message` 类型:需要回复的需求带一个 `fn(..) -> M`,host 执行后把回复包成该面板的消息
/// 投回(包装函数由 host 在排空时给,面板不知道自己在 host 里叫什么)。
pub enum HostRequest<M> {
    /// 弹系统"选择文件夹"对话框;选中后用 `on_picked` 造出面板消息。`start` 是起始目录。
    PickDirectory {
        start: Option<PathBuf>,
        on_picked: fn(PathBuf) -> M,
    },
    /// 让 host 把一个跨面板命令派发给目标。
    Command(PanelCommand),
}

/// 面板 state 里待交给 host 的需求队列。与 `toast::Outbox`(待发提示)同一个模式:面板的 `update`
/// 签名各不相同(返回 `()`/`Option`/`Vec<Effect>`),拿不到 `App`;往自己 state 的 outbox 里 `push`,
/// `App::update` 的包装函数每条消息后统一排空执行。纯数据,可单测。
pub struct HostOutbox<M> {
    items: Vec<HostRequest<M>>,
}

impl<M> Default for HostOutbox<M> {
    fn default() -> Self {
        Self { items: Vec::new() }
    }
}

impl<M> HostOutbox<M> {
    pub fn push(&mut self, request: HostRequest<M>) {
        self.items.push(request);
    }

    pub fn take(&mut self) -> Vec<HostRequest<M>> {
        std::mem::take(&mut self.items)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 评审关注点 1:`next_request_owner` 的计数器是进程内唯一一份,两个 trackers 的 owner 不同,
    /// 所以重开的同 slot 面板互不接受对方的在途结果。
    #[test]
    fn owners_are_unique_across_trackers() {
        let a = RequestTrackers::default();
        let b = RequestTrackers::default();
        let token_a = a.begin(Some(1), "todos");
        let token_b = b.begin(Some(1), "todos");
        assert!(a.accepts(&token_a));
        assert!(
            !a.accepts(&token_b),
            "another instance's token is not accepted"
        );
        assert!(!b.accepts(&token_a));
    }

    /// owner 计数器本身:每次取到的都不同(且来自同一份进程内计数器)。token 的 `seq` 另有全局计数,
    /// 所以只看 `accepts` 抓不住 owner 撞车,要直接断言这一点。
    #[test]
    fn next_request_owner_never_repeats() {
        let owners: Vec<u64> = (0..100).map(|_| next_request_owner()).collect();
        let unique: std::collections::HashSet<_> = owners.iter().collect();
        assert_eq!(unique.len(), owners.len());
    }

    #[test]
    fn scope_round_trips_the_project_id() {
        assert_eq!(project_of(&scope_of(Some(7))), Some(7));
        assert_eq!(
            project_of(&scope_of(None)),
            None,
            "the global lane has no project"
        );
    }

    #[test]
    fn a_newer_request_in_the_same_lane_invalidates_the_older_one() {
        let t = RequestTrackers::default();
        let old = t.begin(Some(1), "todos");
        let new = t.begin(Some(1), "todos");
        assert!(!t.accepts(&old));
        assert!(t.accepts(&new));
    }
}
