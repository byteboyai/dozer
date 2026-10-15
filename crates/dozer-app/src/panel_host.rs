//! 面板 view 对宿主的**只读视图契约**:面板需要从宿主问的东西(悬停动画进度、列表列折叠态、
//! 窗口/光标位置、折叠按钮)集中在这一个 trait 里,面板代码写 `app: &impl PanelHost`,
//! 不再 import 宿主的 `App`。
//!
//! 这是 bytehost 面板边界的第一块契约(见 `docs/superpowers/specs/2026-10-03-bytehost-boundary-design.md`
//! §2 的 E1/E2):trait 里不得出现任何面板的业务类型;`HoverId`/`PanelKind` 暂时仍是宿主类型,
//! 它们的命名空间化/注册制是后续切片(`docs/dozer-v2/bytehost-H0/00-summary.md` §4)。
//! 目前只有 `App` 一个实现;不为未来的第二个实现预先抽象更多方法——面板需要什么才加什么。

use crate::app::{App, HoverId, PanelKind};
use iced_widget::core::Element;
use std::path::PathBuf;
// 请求追踪 / `PanelIo` / `ActivationCtx` / 宿主需求发件箱已抽到共享库 `dozer-panel-host`
// (todo 起,Digger 与以后的共享面板库共用);这里重新导出,面板和 `App` 的 `crate::panel_host::…` 路径不变。
pub(crate) use dozer_panel_host::{
    ActivationCtx, HostOutbox, HostRequest, PanelCommand, PanelIo, RequestTokenWrapExt, RequestTrackers, project_of, scope_of,
};

/// 面板内一个可悬停元素的**槽位**——词汇通用,不含任何面板名(规格 E2:宿主公开类型里不出现业务类型)。
/// 与面板(`PanelKind`)一起构成 `HoverId::Panel(panel, slot)`。新增槽位种类前先看能不能用 `Named`。
///
/// C9:槽位词汇已移入宿主 `bytehost_desktop::HoverSlot`(产品中立、注册制收敛),
/// 这里只重导出,产品侧不再各留一份。
pub use bytehost_desktop::HoverSlot;

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
        tooltip_collapse: impl Into<std::borrow::Cow<'a, str>>,
        tooltip_expand: impl Into<std::borrow::Cow<'a, str>>,
        on_select: M,
        on_hover: impl Fn(bool) -> M + 'a,
    ) -> Element<'a, M, iced_widget::Theme, iced_renderer::Renderer>;
    /// 窗口逻辑尺寸 `(宽, 高)`。
    fn window_size(&self) -> (f32, f32);
    /// 最近一次光标位置 `(x, y)`(窗口坐标)。
    fn last_cursor(&self) -> (f32, f32);
    /// 当前生效语言(BCP-47,如 `en`/`zh-CN`),供面板渲染自有文案时查表。
    /// 默认 `en`:测试桩不需要实现就能渲染。
    fn locale(&self) -> &str {
        "en"
    }
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

    fn locale(&self) -> &str {
        self.locale.active_locale()
    }

    fn list_collapse_button<'a, M: Clone + 'a>(
        &self,
        kind: PanelKind,
        collapsed: bool,
        hover_id: HoverId,
        tooltip_collapse: impl Into<std::borrow::Cow<'a, str>>,
        tooltip_expand: impl Into<std::borrow::Cow<'a, str>>,
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
        let io = PanelIo::new(
            crate::host_adapters::services::offline_services(),
            rt.handle().clone(),
            move |m| {
                let _ = tx.lock().unwrap().send(m);
            },
        );
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
            _tooltip_collapse: impl Into<std::borrow::Cow<'a, str>>,
            _tooltip_expand: impl Into<std::borrow::Cow<'a, str>>,
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

    /// 面板展示适配(`PanelDisplay`)在假宿主上的最小实现:证明"面板 view + 宿主
    /// 展示适配契约"就能构造出 view,无需真的 `App`(库化后 `content_pane` 的第一
    /// 参数是 `impl PanelDisplay<Message>` 而非 `impl PanelHost`)。
    impl crate::extensions::usage::adapter::PanelDisplay<crate::app::Message> for FakeHost {
        fn wrap(&self, message: crate::extensions::usage::Message) -> crate::app::Message {
            crate::app::Message::Usage(message)
        }
        fn agent_icon_color(&self, _agent: dozer_model::AgentKind) -> iced_widget::core::Color {
            iced_widget::core::Color::BLACK
        }
        fn agent_icon(
            &self,
            _agent: dozer_model::AgentKind,
            _color: iced_widget::core::Color,
        ) -> Element<'static, crate::app::Message, iced_widget::Theme, iced_renderer::Renderer>
        {
            iced_widget::Space::new().into()
        }
        fn panel_head<'a>(
            &self,
            _kind: dozer_panel_usage::adapter::PanelHeadKind,
            _title: &'a str,
            _actions: Option<
                Element<'a, crate::app::Message, iced_widget::Theme, iced_renderer::Renderer>,
            >,
        ) -> Element<'a, crate::app::Message, iced_widget::Theme, iced_renderer::Renderer> {
            iced_widget::Space::new().into()
        }
        fn list_collapse_button<'a>(
            &self,
            _collapsed: bool,
            _tooltip_collapse: impl Into<std::borrow::Cow<'a, str>>,
            _tooltip_expand: impl Into<std::borrow::Cow<'a, str>>,
            _on_select: crate::app::Message,
            _on_hover: impl Fn(bool) -> crate::app::Message + 'a,
        ) -> Element<'a, crate::app::Message, iced_widget::Theme, iced_renderer::Renderer> {
            self.collapse_buttons_asked
                .set(self.collapse_buttons_asked.get() + 1);
            iced_widget::Space::new().into()
        }
        fn collapse_hover(&self, _hovered: bool) -> crate::app::Message {
            crate::app::Message::Noop
        }
        fn toggle_list_collapse(&self) -> crate::app::Message {
            crate::app::Message::Noop
        }
    }

    #[test]
    fn usage_content_pane_builds_against_a_fake_host_and_asks_it_for_the_collapse_button() {
        let host = FakeHost::default();
        let ws_state = crate::extensions::usage::WorkspaceState::default();
        let _view = crate::extensions::usage::content_pane(
            &host,
            false,
            &ws_state,
            Length::Fill,
            Border::default(),
            "en",
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

    /// C9:面板内槽位词汇即宿主的 `bytehost_desktop::HoverSlot`,产品不各留一份。
    #[test]
    fn hover_slot_vocabulary_is_the_host_type() {
        let slot: bytehost_desktop::HoverSlot = HoverSlot::ListCollapse;
        assert_eq!(slot, HoverSlot::ListCollapse);
        // 具名兜底仍是收敛新槽位的首选。
        assert_eq!(
            HoverId::named(PanelKind::Files, "find_prev"),
            HoverId::Panel(
                PanelKind::Files,
                bytehost_desktop::HoverSlot::Named("find_prev")
            )
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

    /// 一个把 `emit` 都收进 channel 的 `PanelIo<u32>`(服务指向不存在的 socket,本组测试不用它)。
    fn recording_io(
        rt: &tokio::runtime::Runtime,
    ) -> (PanelIo<u32>, std::sync::mpsc::Receiver<u32>) {
        let (tx, rx) = std::sync::mpsc::channel();
        let tx = std::sync::Mutex::new(tx);
        let io = PanelIo::new(
            crate::host_adapters::services::offline_services(),
            rt.handle().clone(),
            move |m| {
                let _ = tx.lock().unwrap().send(m);
            },
        );
        (io, rx)
    }

    #[test]
    fn with_services_replaces_the_offline_adapter() {
        use crate::usage_probe::fake_usage::FakeUsage;
        let rt = test_runtime();
        let (io, _rx) = recording_io(&rt);
        // 默认服务指向不存在的 socket → Unavailable
        let default_err = rt
            .block_on(io.services().usage.usage_summary("/p".into(), None))
            .unwrap_err();
        assert_eq!(
            default_err.kind,
            dozer_services::ServiceErrorKind::Unavailable
        );
        // 替换后走假实现
        let fake = FakeUsage::ok(vec![]);
        let io = io.with_services(fake.clone().into_services());
        assert_eq!(
            rt.block_on(io.services().usage.usage_summary("/p".into(), Some(3))),
            Ok(vec![])
        );
        assert_eq!(fake.calls(), vec![("/p".to_string(), Some(3))]);
        // clone 之后仍用替换后的服务
        let again = io.clone();
        rt.block_on(again.services().usage.usage_summary("/q".into(), None))
            .unwrap();
        assert_eq!(fake.calls().len(), 2);
    }

    #[test]
    fn default_services_route_todo_through_the_client_adapter() {
        let rt = test_runtime();
        let (io, _rx) = recording_io(&rt);
        // recording_io 的 Client 指向不存在的 socket → Unavailable
        let e = rt.block_on(io.services().todo.list_todos(1)).unwrap_err();
        assert_eq!(e.kind, dozer_services::ServiceErrorKind::Unavailable);
    }

    #[test]
    fn default_services_route_project_and_memory_through_the_client_adapter() {
        let rt = test_runtime();
        let (io, _rx) = recording_io(&rt);
        // recording_io 的 Client 指向不存在的 socket → Unavailable
        let e = rt
            .block_on(io.services().memory.list_memories(1))
            .unwrap_err();
        assert_eq!(e.kind, dozer_services::ServiceErrorKind::Unavailable);
        let e = rt
            .block_on(io.services().project.remove_project(1))
            .unwrap_err();
        assert_eq!(e.kind, dozer_services::ServiceErrorKind::Unavailable);
    }

    #[test]
    fn default_services_route_group_chat_through_the_client_adapter() {
        let rt = test_runtime();
        let (io, _rx) = recording_io(&rt);
        // recording_io 的 Client 指向不存在的 socket → Unavailable
        let e = rt
            .block_on(io.services().group_chat.list_groups(1))
            .unwrap_err();
        assert_eq!(e.kind, dozer_services::ServiceErrorKind::Unavailable);
    }

    #[test]
    fn default_services_route_conversation_through_the_client_adapter() {
        let rt = test_runtime();
        let (io, _rx) = recording_io(&rt);
        // recording_io 的 Client 指向不存在的 socket → Unavailable
        let e = rt
            .block_on(
                io.services()
                    .conversation
                    .list_conversations_with_summaries("/p".into(), None, 500, 0),
            )
            .unwrap_err();
        assert_eq!(e.kind, dozer_services::ServiceErrorKind::Unavailable);
    }

    #[test]
    fn default_services_route_codehealth_through_the_client_adapter() {
        let rt = test_runtime();
        let (io, _rx) = recording_io(&rt);
        // recording_io 的 Client 指向不存在的 socket → Unavailable
        let e = rt
            .block_on(io.services().codehealth.get_code_health_report(1))
            .unwrap_err();
        assert_eq!(e.kind, dozer_services::ServiceErrorKind::Unavailable);
    }

    #[test]
    fn default_services_route_context_through_the_client_adapter() {
        let rt = test_runtime();
        let (io, _rx) = recording_io(&rt);
        // recording_io 的 Client 指向不存在的 socket → Unavailable
        let e = rt
            .block_on(io.services().context.list_context_items(1))
            .unwrap_err();
        assert_eq!(e.kind, dozer_services::ServiceErrorKind::Unavailable);
    }

    #[test]
    fn default_services_route_edit_history_through_the_client_adapter() {
        let rt = test_runtime();
        let (io, _rx) = recording_io(&rt);
        // recording_io 的 Client 指向不存在的 socket → Unavailable
        let e = rt
            .block_on(
                io.services()
                    .edit_history
                    .list_file_edit_history(1, None, 200),
            )
            .unwrap_err();
        assert_eq!(e.kind, dozer_services::ServiceErrorKind::Unavailable);
    }

    #[test]
    fn default_services_route_bookmark_through_the_client_adapter() {
        let rt = test_runtime();
        let (io, _rx) = recording_io(&rt);
        // recording_io 的 Client 指向不存在的 socket → Unavailable
        let e = rt
            .block_on(io.services().bookmark.list_bookmarks(None))
            .unwrap_err();
        assert_eq!(e.kind, dozer_services::ServiceErrorKind::Unavailable);
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
        io.spawn(|_services, io| async move {
            io.emit(10);
            io.emit(11);
        });
        io.spawn(
            |_services, _io| async move { /* 零条也合法(例如只在失败时才发消息的命令) */
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
    fn spawn_hands_the_task_the_hosts_services() {
        let rt = test_runtime();
        let (io, rx) = recording_io(&rt);
        io.spawn(|services, io| async move {
            // 指向不存在 socket 的 Client 适配出的服务:真的发请求必然失败——证明拿到的是 host 的服务束。
            let failed = services.group_chat.list_groups(1).await.is_err();
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
