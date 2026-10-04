//! 面板 view 对宿主的**只读视图契约**:面板需要从宿主问的东西(悬停动画进度、列表列折叠态、
//! 窗口/光标位置、折叠按钮)集中在这一个 trait 里,面板代码写 `app: &impl PanelHost`,
//! 不再 import 宿主的 `App`。
//!
//! 这是 bytehost 面板边界的第一块契约(见 `docs/superpowers/specs/2026-10-03-bytehost-boundary-design.md`
//! §2 的 E1/E2):trait 里不得出现任何面板的业务类型;`HoverId`/`PanelKind` 暂时仍是宿主类型,
//! 它们的命名空间化/注册制是后续切片(`docs/dozer-v2/bytehost-H0/00-summary.md` §4)。
//! 目前只有 `App` 一个实现;不为未来的第二个实现预先抽象更多方法——面板需要什么才加什么。

use crate::app::{App, HoverId, PanelKind};
use dozer_client::Client;
use iced_widget::core::Element;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::runtime::Handle;

/// host 给面板的**执行原语**:面板要在后台跑任务,需要三样东西——`Client`(dozerd 连接)、
/// `Handle`(tokio runtime)、"把结果包成该面板的 `Message` 投回事件循环"。收成一个值,由 host
/// 构造、传给面板模块里的执行器(`group_chat::run_effect` 等);host 不再认识面板的具体 Effect。
///
/// 与 [`PanelHost`](只读视图契约)是一对:`PanelHost` 管"读 host 状态来画 view",`PanelIo` 管
/// "让 host 替我跑东西"。是具体结构体而不是 trait(规格 §1:不为未来的第二个宿主预先抽象);
/// 测试里用 `PanelIo::new` 配一个收进 channel 的 `emit` 即可。
pub(crate) struct PanelIo<M> {
    client: Client,
    handle: Handle,
    emit: Arc<dyn Fn(M) + Send + Sync + 'static>,
}

impl<M> Clone for PanelIo<M> {
    fn clone(&self) -> Self {
        Self {
            client: self.client.clone(),
            handle: self.handle.clone(),
            emit: Arc::clone(&self.emit),
        }
    }
}

impl<M: Send + 'static> PanelIo<M> {
    pub(crate) fn new(
        client: Client,
        handle: Handle,
        emit: impl Fn(M) + Send + Sync + 'static,
    ) -> Self {
        Self {
            client,
            handle,
            emit: Arc::new(emit),
        }
    }

    /// 同步投回一条面板消息。
    pub(crate) fn emit(&self, message: M) {
        (self.emit)(message);
    }

    /// host 的 dozerd 连接(给仍然吃 `(client, handle, emit)` 三件套的面板 `spawn_*` 函数用)。
    pub(crate) fn client(&self) -> &Client {
        &self.client
    }

    /// host 的 tokio runtime 句柄(给仍然吃 `(client, handle, emit)` 三件套的面板 `spawn_*` 函数用)。
    pub(crate) fn handle(&self) -> &Handle {
        &self.handle
    }

    /// 一个可 `clone`、可跨线程的 `emit` 闭包(同上,给 `spawn_*(.., emit: impl Fn(M) + Send + 'static)`)。
    pub(crate) fn emitter(&self) -> impl Fn(M) + Clone + Send + 'static {
        let emit = Arc::clone(&self.emit);
        move |message| emit(message)
    }

    /// 在 host 的 runtime 上跑一个后台任务。任务拿到 host 的 `Client` 和一份 `PanelIo` 副本,
    /// 想发多少条消息(包括零条:只在失败时才发的命令)由任务自己决定。
    pub(crate) fn spawn<F, Fut>(&self, task: F)
    where
        F: FnOnce(Client, PanelIo<M>) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let client = self.client.clone();
        let io = self.clone();
        self.handle.spawn(async move { task(client, io).await });
    }
}

/// 面板"切入"(被用户切到前台)时 host 交给它的上下文:当前项目 id、项目路径(项目还没有路径时为
/// `None`)和执行原语。面板的 `on_activate` 钩子只依赖它,不碰 `App`/`Workspace`。
pub(crate) struct ActivationCtx<M> {
    pub(crate) project_id: i64,
    pub(crate) project_path: Option<PathBuf>,
    pub(crate) io: PanelIo<M>,
}

/// 面板之间"我想让另一个面板做点事"的**受限词汇**(设计文档 P1,用户 2026-10-04 裁决:面板不依赖
/// host 的总 `Message`,也不互相引用)。载荷只用通用类型(路径、布尔),不出现任何面板的业务类型;
/// 由 host 把命令翻译成对目标面板/弹窗的具体动作(`App::run_panel_command`)。
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PanelCommand {
    /// 在某个路径下发起搜索(`is_dir` 决定按目录递归还是只搜单文件)。
    SearchIn { path: PathBuf, is_dir: bool },
    /// 打开某个文件(绝对路径)的 git 历史。
    ShowFileHistory { path: PathBuf },
}

/// 面板向 host 提的需求——只有 host 才能做的事(系统对话框、跨面板动作)。`M` 是提需求的那个面板
/// 自己的 `Message` 类型:需要回复的需求带一个 `fn(..) -> M`,host 执行后把回复包成该面板的消息
/// 投回(包装函数由 host 在排空时给,面板不知道自己在 host 里叫什么)。
pub(crate) enum HostRequest<M> {
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
pub(crate) struct HostOutbox<M> {
    items: Vec<HostRequest<M>>,
}

impl<M> Default for HostOutbox<M> {
    fn default() -> Self {
        Self { items: Vec::new() }
    }
}

impl<M> HostOutbox<M> {
    pub(crate) fn push(&mut self, request: HostRequest<M>) {
        self.items.push(request);
    }

    pub(crate) fn take(&mut self) -> Vec<HostRequest<M>> {
        std::mem::take(&mut self.items)
    }
}

/// 面板内一个可悬停元素的**槽位**——词汇通用,不含任何面板名(规格 E2:宿主公开类型里不出现业务类型)。
/// 与面板(`PanelKind`)一起构成 `HoverId::Panel(panel, slot)`。新增槽位种类前先看能不能用 `Named`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HoverSlot {
    /// 列表列折叠按钮。
    ListCollapse,
    /// 搜索框提交按钮。
    SearchSubmit,
    /// "更多…"翻页按钮。
    More,
    /// 页签标题(下标或稳定 id)。
    TabItem(u64),
    /// 页签关闭按钮。
    TabClose(u64),
    /// 页签溢出菜单按钮。
    TabOverflow,
    /// 列表行/卡片。
    Row(u64),
    /// 一组互斥选项里的一项。
    Choice(u64),
    /// 面板内一次性的具名按钮(同一面板内唯一)。
    Named(&'static str),
}

pub trait PanelHost {
    /// 某个按钮/页签的悬停动画进度 0.0..=1.0。
    fn hover_progress(&self, id: HoverId) -> f32;
    /// 悬停是否已持续满 tooltip 延迟(该弹标题全称 tooltip 了)。
    fn hover_tooltip_ready(&self, id: HoverId) -> bool;
    /// 两栏面板的列表列当前是否收起。
    fn list_collapsed(&self, kind: PanelKind) -> bool;
    /// 列表列的折叠/展开按钮(图标随面板所在栏位镜像)。
    #[allow(clippy::too_many_arguments)]
    fn list_collapse_button<'a, M: Clone + 'a>(
        &self,
        kind: PanelKind,
        collapsed: bool,
        hover_id: HoverId,
        tooltip_collapse: &'a str,
        tooltip_expand: &'a str,
        on_select: M,
        on_hover: impl Fn(bool) -> M + 'a,
    ) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>;
    /// 窗口逻辑尺寸 `(宽, 高)`。
    fn window_size(&self) -> (f32, f32);
    /// 最近一次光标位置 `(x, y)`(窗口坐标)。
    fn last_cursor(&self) -> (f32, f32);
}

impl PanelHost for App {
    fn hover_progress(&self, id: HoverId) -> f32 {
        App::hover_progress(self, id)
    }

    fn hover_tooltip_ready(&self, id: HoverId) -> bool {
        App::hover_tooltip_ready(self, id)
    }

    fn list_collapsed(&self, kind: PanelKind) -> bool {
        App::list_collapsed(self, kind)
    }

    fn list_collapse_button<'a, M: Clone + 'a>(
        &self,
        kind: PanelKind,
        collapsed: bool,
        hover_id: HoverId,
        tooltip_collapse: &'a str,
        tooltip_expand: &'a str,
        on_select: M,
        on_hover: impl Fn(bool) -> M + 'a,
    ) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer> {
        App::list_collapse_button(
            self,
            kind,
            collapsed,
            hover_id,
            tooltip_collapse,
            tooltip_expand,
            on_select,
            on_hover,
        )
    }

    fn window_size(&self) -> (f32, f32) {
        self.window_size
    }

    fn last_cursor(&self) -> (f32, f32) {
        self.last_cursor
    }
}

/// 给面板钩子单测用的离线夹具:`Client` 指向不存在的 socket,`emit` 收进 channel。
#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use std::sync::{Mutex, mpsc};
    use std::time::Duration;

    pub(crate) const TIMEOUT: Duration = Duration::from_secs(5);

    pub(crate) fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap()
    }

    pub(crate) fn offline_ctx<M: Send + 'static>(
        rt: &tokio::runtime::Runtime,
        project_id: i64,
        project_path: Option<PathBuf>,
    ) -> (ActivationCtx<M>, mpsc::Receiver<M>) {
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let client = Client::new(PathBuf::from("/tmp/dozer-activation-test-nonexistent.sock"));
        let io = PanelIo::new(client, rt.handle().clone(), move |m| {
            let _ = tx.lock().unwrap().send(m);
        });
        (
            ActivationCtx {
                project_id,
                project_path,
                io,
            },
            rx,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced_widget::core::{Border, Length};
    use std::cell::Cell;

    /// 不依赖 `App` 的假宿主:证明"一个面板 + 宿主契约"就能构造出 view(规格 §6 第 5 条的第一个探针)。
    #[derive(Default)]
    struct FakeHost {
        collapse_buttons_asked: Cell<u32>,
    }

    impl PanelHost for FakeHost {
        fn hover_progress(&self, _id: HoverId) -> f32 {
            0.0
        }
        fn hover_tooltip_ready(&self, _id: HoverId) -> bool {
            false
        }
        fn list_collapsed(&self, _kind: PanelKind) -> bool {
            false
        }
        fn list_collapse_button<'a, M: Clone + 'a>(
            &self,
            _kind: PanelKind,
            _collapsed: bool,
            _hover_id: HoverId,
            _tooltip_collapse: &'a str,
            _tooltip_expand: &'a str,
            _on_select: M,
            _on_hover: impl Fn(bool) -> M + 'a,
        ) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer> {
            self.collapse_buttons_asked
                .set(self.collapse_buttons_asked.get() + 1);
            iced_widget::Space::new().into()
        }
        fn window_size(&self) -> (f32, f32) {
            (800.0, 600.0)
        }
        fn last_cursor(&self) -> (f32, f32) {
            (0.0, 0.0)
        }
    }

    #[test]
    fn usage_content_pane_builds_against_a_fake_host_and_asks_it_for_the_collapse_button() {
        let host = FakeHost::default();
        let ws_state = crate::extensions::usage::WorkspaceState::default();
        let _view = crate::extensions::usage::content_pane(
            &host,
            &ws_state,
            Length::Fill,
            Border::default(),
        );
        assert_eq!(host.collapse_buttons_asked.get(), 1);
    }
    // ---- H2:HoverId 的面板作用域键 ----

    use crate::extensions::git_log::FileFilter;

    #[test]
    fn same_slot_in_different_panels_is_a_different_key() {
        assert_ne!(
            HoverId::tab_item(PanelKind::Files, 1),
            HoverId::tab_item(PanelKind::Project, 1)
        );
        assert_ne!(
            HoverId::named(PanelKind::Files, "find_prev"),
            HoverId::named(PanelKind::Project, "find_prev")
        );
        assert_ne!(
            HoverId::list_collapse(PanelKind::Todo),
            HoverId::list_collapse(PanelKind::Usage)
        );
    }

    #[test]
    fn different_slots_and_keys_in_one_panel_are_different_keys() {
        let p = PanelKind::Database;
        let all = [
            HoverId::list_collapse(p),
            HoverId::search_submit(p),
            HoverId::more(p),
            HoverId::tab_overflow(p),
            HoverId::tab_item(p, 0),
            HoverId::tab_item(p, 1),
            HoverId::tab_close(p, 0),
            HoverId::row(p, 0),
            HoverId::choice(p, 0),
            HoverId::named(p, "a"),
            HoverId::named(p, "b"),
        ];
        let set: std::collections::HashSet<_> = all.iter().copied().collect();
        assert_eq!(set.len(), all.len());
    }

    #[test]
    fn git_file_filters_map_to_distinct_choice_keys() {
        let keys: std::collections::HashSet<_> = [
            FileFilter::All,
            FileFilter::Modified,
            FileFilter::Added,
            FileFilter::Deleted,
            FileFilter::Renamed,
        ]
        .into_iter()
        .map(|f| HoverId::choice(PanelKind::GitLog, f as u64))
        .collect();
        assert_eq!(keys.len(), 5);
    }

    // ---- H4:PanelIo ----

    fn test_runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap()
    }

    /// 一个把 `emit` 都收进 channel 的 `PanelIo<u32>`(`Client` 指向不存在的 socket,本组测试不用它)。
    fn recording_io(
        rt: &tokio::runtime::Runtime,
    ) -> (PanelIo<u32>, std::sync::mpsc::Receiver<u32>) {
        let (tx, rx) = std::sync::mpsc::channel();
        let tx = std::sync::Mutex::new(tx);
        let client = dozer_client::Client::new(std::path::PathBuf::from(
            "/tmp/dozer-panel-io-test-nonexistent.sock",
        ));
        let io = PanelIo::new(client, rt.handle().clone(), move |m| {
            let _ = tx.lock().unwrap().send(m);
        });
        (io, rx)
    }

    #[test]
    fn emit_delivers_the_message_synchronously() {
        let rt = test_runtime();
        let (io, rx) = recording_io(&rt);
        io.emit(7);
        assert_eq!(rx.try_recv().unwrap(), 7);
    }

    #[test]
    fn clones_share_one_sink() {
        let rt = test_runtime();
        let (io, rx) = recording_io(&rt);
        let other = io.clone();
        io.emit(1);
        other.emit(2);
        assert_eq!(rx.try_iter().collect::<Vec<_>>(), vec![1, 2]);
    }

    #[test]
    fn spawn_runs_on_the_runtime_and_the_task_may_emit_any_number_of_times() {
        let rt = test_runtime();
        let (io, rx) = recording_io(&rt);
        io.spawn(|_client, io| async move {
            io.emit(10);
            io.emit(11);
        });
        io.spawn(
            |_client, _io| async move { /* 零条也合法(例如只在失败时才发消息的命令) */
            },
        );
        let got = [
            rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap(),
            rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap(),
        ];
        assert_eq!(got, [10, 11]);
        assert!(
            rx.recv_timeout(std::time::Duration::from_millis(200))
                .is_err()
        );
    }

    #[test]
    fn spawn_hands_the_task_the_hosts_client() {
        let rt = test_runtime();
        let (io, rx) = recording_io(&rt);
        io.spawn(|client, io| async move {
            // 指向不存在 socket 的 Client:真的发请求必然失败——证明拿到的是 host 的那个 Client。
            let failed = client.list_groups(1).await.is_err();
            io.emit(u32::from(failed));
        });
        assert_eq!(
            rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap(),
            1
        );
    }

    // ---- H5:HostOutbox / HostRequest / PanelCommand ----

    #[test]
    fn host_outbox_returns_requests_in_push_order_and_empties_itself() {
        let mut out: HostOutbox<u32> = HostOutbox::default();
        out.push(HostRequest::Command(PanelCommand::SearchIn {
            path: "/a".into(),
            is_dir: true,
        }));
        out.push(HostRequest::PickDirectory {
            start: None,
            on_picked: |p| p.as_os_str().len() as u32,
        });
        let reqs = out.take();
        assert_eq!(reqs.len(), 2);
        assert!(matches!(
            &reqs[0],
            HostRequest::Command(PanelCommand::SearchIn { is_dir: true, .. })
        ));
        assert!(matches!(
            &reqs[1],
            HostRequest::PickDirectory { start: None, .. }
        ));
        assert!(out.take().is_empty());
    }

    #[test]
    fn pick_directory_reply_maps_the_picked_path_into_the_panels_message() {
        let req: HostRequest<String> = HostRequest::PickDirectory {
            start: Some("/s".into()),
            on_picked: |p| p.display().to_string(),
        };
        let HostRequest::PickDirectory { start, on_picked } = req else {
            panic!("expected PickDirectory");
        };
        assert_eq!(start, Some(std::path::PathBuf::from("/s")));
        assert_eq!(on_picked("/x/y".into()), "/x/y");
    }

    // ---- H6:PanelIo 访问器 ----

    #[test]
    fn emitter_is_a_clone_that_shares_the_sink_and_handle_is_the_hosts_runtime() {
        let rt = test_runtime();
        let (io, rx) = recording_io(&rt);
        let emit = io.emitter();
        let again = emit.clone();
        emit(1);
        again(2);
        assert_eq!(rx.try_iter().collect::<Vec<_>>(), vec![1, 2]);
        // handle() 是 host 的 runtime:能在上面 spawn
        let (tx, done) = std::sync::mpsc::channel();
        io.handle().spawn(async move {
            let _ = tx.send(42u32);
        });
        assert_eq!(
            done.recv_timeout(std::time::Duration::from_secs(5))
                .unwrap(),
            42
        );
    }
}
