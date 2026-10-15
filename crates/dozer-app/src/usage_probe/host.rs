//! `ProbeHost`:扮演 `App` 对 Usage 面板做的那部分事。每一件"面板自己做不了、由 host 代做"的事都记进
//! `duties`,这份日志就是缺口报告的原始数据。**逻辑从真实 host 手抄而来**,见各方法注释里的出处;
//! `drift_guard` 测试负责在出处改动时报警。

use super::fake_dozerd::FakeDozerd;
use super::surface::FakeSurface;
use crate::extensions::usage::{self, Message, UsageSurface, WorkspaceState};
use crate::panel_host::{ActivationCtx, PanelIo};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Mutex, mpsc};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Duty {
    BuildActivationCtx,
    RouteLoaded,
    DropOtherProject,
    InterceptCollapse,
    InterceptHover,
    FramePush,
    DowngradeReady,
    SkipWhileLoading,
    ParseIpcAndMarkReady,
    DrainToastOutbox,
}

impl Duty {
    /// 真实 host 里承担这件事的位置(读代码得到,2026-10-08)。
    pub(crate) fn host_site(self) -> &'static str {
        match self {
            Duty::BuildActivationCtx => "app/update.rs:5484",
            Duty::RouteLoaded => "app/update.rs:1673 (with_project)",
            Duty::DropOtherProject => "app/update.rs:1673 (with_project 路由到别的 Workspace)",
            Duty::InterceptCollapse => "host_adapters/usage_panel.rs:92",
            Duty::InterceptHover => "host_adapters/usage_panel.rs:88",
            Duty::FramePush => "app/app.rs:1281 take_usage_content_script",
            Duty::DowngradeReady => "app/app.rs:1291",
            Duty::SkipWhileLoading => "app/app.rs:1298",
            Duty::ParseIpcAndMarkReady => "runtime.rs:746 + app/update.rs:265",
            Duty::DrainToastOutbox => "app/update.rs:223 (drain_outboxes)",
        }
    }
}

pub(crate) struct ProbeHost {
    pub ws: WorkspaceState,
    pub push: UsageSurface,
    pub surface: FakeSurface,
    pub duties: Vec<Duty>,
    project_id: i64,
}

impl ProbeHost {
    pub(crate) fn new(project_id: i64) -> Self {
        Self {
            ws: WorkspaceState::default(),
            push: UsageSurface::new(
                crate::app::USAGE_CONTENT_ID_OFFSET as u32,
                crate::preview::dispatch_script,
            ),
            surface: FakeSurface::default(),
            duties: Vec::new(),
            project_id,
        }
    }

    /// 对应 `app/update.rs:5317`:host 构造 `ActivationCtx` 再调面板的 `on_activate`。
    pub(crate) fn activate(
        &mut self,
        rt: &tokio::runtime::Runtime,
        dozerd: &FakeDozerd,
        path: Option<PathBuf>,
    ) -> mpsc::Receiver<Message> {
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let client = dozer_client::Client::new(dozerd.socket().to_path_buf());
        let io = PanelIo::new(
            crate::host_adapters::services::services_for(&client),
            rt.handle().clone(),
            move |m| {
                let _ = tx.lock().unwrap().send(m);
            },
        );
        let ctx = ActivationCtx {
            project_id: self.project_id,
            project_path: path,
            io,
        };
        self.duties.push(Duty::BuildActivationCtx);
        usage::activate(&mut self.ws, Some(ctx));
        rx
    }

    /// 与 `activate` 同,但服务由调用方给(进程内假实现),不经 UDS。
    pub(crate) fn activate_with(
        &mut self,
        rt: &tokio::runtime::Runtime,
        services: dozer_services::Services,
        path: Option<PathBuf>,
    ) -> mpsc::Receiver<Message> {
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let io = PanelIo::new(services, rt.handle().clone(), move |m| {
            let _ = tx.lock().unwrap().send(m);
        });
        let ctx = ActivationCtx {
            project_id: self.project_id,
            project_path: path,
            io,
        };
        self.duties.push(Duty::BuildActivationCtx);
        usage::activate(&mut self.ws, Some(ctx));
        rx
    }

    /// 对应 `App::drain_outboxes`(`app/update.rs`)里 `pending.extend(ws.usage.take_outbox())`。
    pub(crate) fn drain_toasts(&mut self) -> Vec<crate::extensions::toast::Pending> {
        self.duties.push(Duty::DrainToastOutbox);
        usage::notices_to_pending(self.ws.take_outbox())
    }

    /// 对应 `app/update.rs:1640-1651` 的路由:`Loaded`/`LoadFailed` 按 `project_id` 路由(这里只有一个 Workspace,
    /// 非本项目的等价于"路由到别的 Workspace",对本面板即丢弃);`ToggleListCollapse`/`Hover` 由 host
    /// 拦截,不进面板的 `update`。
    pub(crate) fn deliver(&mut self, msg: Message) {
        match msg {
            Message::Loaded(project_id, ..) if project_id != self.project_id => {
                self.duties.push(Duty::DropOtherProject);
            }
            msg @ Message::Loaded(..) => {
                self.duties.push(Duty::RouteLoaded);
                usage::update(&mut self.ws, msg);
            }
            Message::LoadFailed(project_id, ..) if project_id != self.project_id => {
                self.duties.push(Duty::DropOtherProject);
            }
            msg @ Message::LoadFailed(..) => {
                self.duties.push(Duty::RouteLoaded);
                usage::update(&mut self.ws, msg);
            }
            Message::ToggleListCollapse => self.duties.push(Duty::InterceptCollapse),
            Message::CollapseHover(..) => self.duties.push(Duty::InterceptHover),
            msg => usage::update(&mut self.ws, msg),
        }
    }

    /// 对应 `App::take_usage_content_script`(`app/app.rs`):每帧先用"原生实例
    /// 是否在池中"驱动 Surface 生命周期,再声明式比较"该显示什么"与"上次送达
    /// 什么"。
    pub(crate) fn tick(&mut self) {
        self.duties.push(Duty::FramePush);
        // 进池/离池 → Created / Destroyed(离池时递增代数,旧实例在途 IPC 作废);
        // 原生实例同帧被替换(replaced)→ 先判死旧身份再重建。
        self.push
            .observe_instance(self.surface.is_attached(), self.surface.take_replaced());
        if !self.surface.is_attached() {
            self.duties.push(Duty::DowngradeReady);
            return;
        }
        if self.ws.loading() {
            self.duties.push(Duty::SkipWhileLoading);
            return;
        }
        let desired = usage::current_view_payload(&self.ws);
        self.push
            .set_desired(dozer_model::Stamped::new(Self::probe_stamp(), desired));
        let Some((_, script)) = self.push.take_pending_script() else {
            return;
        };
        self.surface.receive_script(script);
    }

    fn probe_stamp() -> dozer_model::LocaleStamp {
        dozer_model::LocaleStamp {
            ui_locale: dozer_model::Locale::default_locale(),
            locale_revision: 0,
        }
    }

    /// 对应 `runtime.rs` 的 IPC 识别 + `app/update.rs` 的 `Ready` 处理:
    /// 先经路由层校验归属,再交 Surface 状态机推进到 Ready。
    pub(crate) fn webview_ready(&mut self) {
        self.duties.push(Duty::ParseIpcAndMarkReady);
        let event = usage::parse_usage_event(r#"{"kind":"ready"}"#).unwrap();
        if !self.push.accepts(Some(self.push.id())) {
            return;
        }
        if matches!(event, usage::UsageWebviewEvent::Ready) {
            self.push.apply_ready();
        }
    }

    pub(crate) fn duty_set(&self) -> HashSet<Duty> {
        self.duties.iter().copied().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_without_a_surface_downgrades_ready_and_pushes_nothing() {
        // 没有原生实例时:tick 把 Surface 推入/维持"无实例"态,不推内容。
        let mut h = ProbeHost::new(1);
        h.tick();
        assert_eq!(h.surface.count(), 0);
        assert!(h.duty_set().contains(&Duty::DowngradeReady));
        assert!(h.push.take_pending_script().is_none(), "无实例时没有待推送");
    }

    #[test]
    fn webview_ready_then_tick_pushes_once_and_dedups() {
        let mut h = ProbeHost::new(1);
        h.surface.attach();
        h.tick();
        h.webview_ready();
        h.tick();
        h.tick();
        assert_eq!(h.surface.kinds(), vec!["empty".to_string()]);
    }

    /// 漂移守卫:`tick` 是 `App::take_usage_content_script` 的手抄版。真实函数里这几步的先后顺序一变,
    /// 本测试就失败,提醒同步夹具(并在报告里记一笔"host 行为变了")。
    #[test]
    fn drift_guard_tick_mirrors_take_usage_content_script() {
        let src = include_str!("../app/app.rs");
        let start = src
            .find("pub fn take_usage_content_script")
            .expect("app.rs 里找不到 take_usage_content_script");
        let body = &src[start..];
        let body = &body[..body.find("pub fn take_codehealth_content_script").unwrap()];
        let order = [
            "observe_instance",
            "active_workspace()",
            "usage.loading()",
            "current_view_payload",
            "set_desired",
            "take_pending_script",
        ];
        let mut at = 0;
        for needle in order {
            let found = body[at..].find(needle).unwrap_or_else(|| {
                panic!("take_usage_content_script 里 `{needle}` 不在预期位置之后,夹具需同步")
            });
            at += found;
        }
    }

    /// `host_site()` 的每个引用都必须落在含预期内容的真实行上(行号漂了就失败)。
    #[test]
    fn host_site_citations_point_at_the_code_they_name() {
        let update = include_str!("../app/update.rs");
        let app = include_str!("../app/app.rs");
        let runtime = include_str!("../runtime.rs");
        let usage = include_str!("../host_adapters/usage_panel.rs");
        let table: &[(Duty, &[&str])] = &[
            (
                Duty::BuildActivationCtx,
                &["workspace_activation_ctx(ws, io, Message::Usage)"],
            ),
            (
                Duty::RouteLoaded,
                &["msg @ (usage::Message::Loaded(project_id, ..)"],
            ),
            (
                Duty::DropOtherProject,
                &["msg @ (usage::Message::Loaded(project_id, ..)"],
            ),
            (
                Duty::InterceptCollapse,
                &["AppMessage::TogglePanelListCollapse(PanelKind::Usage)"],
            ),
            (
                Duty::InterceptHover,
                &["Message::Hover(HoverId::list_collapse(PanelKind::Usage), hovered)"],
            ),
            (Duty::FramePush, &["pub fn take_usage_content_script("]),
            (Duty::DowngradeReady, &["self.usage_surface"]),
            (Duty::SkipWhileLoading, &["if ws.usage.loading()"]),
            (
                Duty::DrainToastOutbox,
                &["pending.extend(usage::notices_to_pending(ws.usage.take_outbox()))"],
            ),
            (
                Duty::ParseIpcAndMarkReady,
                &[
                    "webview_id == crate::app::USAGE_CONTENT_ID_OFFSET",
                    "Message::UsageContentWebviewEvent(event, surface)",
                ],
            ),
        ];
        for (duty, needles) in table {
            let sites: Vec<&str> = duty.host_site().split(" + ").collect();
            assert_eq!(sites.len(), needles.len(), "{duty:?}: 引用个数与预期不符");
            for (site, needle) in sites.iter().zip(needles.iter()) {
                let (file, rest) = site.split_once(':').expect("host_site 形如 file:line");
                let line: usize = rest
                    .split(|c: char| !c.is_ascii_digit())
                    .next()
                    .unwrap()
                    .parse()
                    .expect("host_site 的行号");
                let src = match file {
                    "app/update.rs" => update,
                    "app/app.rs" => app,
                    "runtime.rs" => runtime,
                    "extensions/usage.rs" | "host_adapters/usage_panel.rs" => usage,
                    other => panic!("{duty:?}: 未知文件 {other}"),
                };
                let text = src.lines().nth(line - 1).unwrap_or("");
                assert!(
                    text.contains(needle),
                    "{duty:?}: {site} 实际是 `{}`,应含 `{needle}`",
                    text.trim()
                );
            }
        }
    }

    /// 漂移守卫(补全):`deliver`/`webview_ready` 手抄的 update.rs 路由、`tick` 手抄的 app.rs 函数。
    /// 真实路由分支的出现顺序、`take_usage_content_script` 的提前返回个数一变就失败。
    #[test]
    fn drift_guard_covers_routing_ready_handling_and_both_sides_of_tick() {
        fn in_order(label: &str, src: &str, needles: &[&str]) {
            let mut at = 0;
            for needle in needles {
                let found = src[at..]
                    .find(needle)
                    .unwrap_or_else(|| panic!("{label}: `{needle}` 不在预期位置之后,夹具需同步"));
                at += found;
            }
        }
        let update = include_str!("../app/update.rs");
        in_order(
            "update.rs 路由",
            update,
            &[
                "msg @ (usage::Message::Loaded(project_id, ..)",
                "| usage::Message::LoadFailed(project_id, ..)",
                "self.with_project(project_id",
                "Message::Usage(msg) =>",
                "self.with_focused_project",
            ],
        );
        // 折叠/悬停拦截已从 update.rs 的 `Message::Usage` 分支迁到产品展示适配
        // (`host_adapters/usage_panel.rs` 的 `PanelDisplay` 把库内 `ToggleListCollapse`/
        // `CollapseHover` 直接映射成顶层 `Message::TogglePanelListCollapse`/`Message::Hover`)。
        in_order(
            "host_adapters/usage_panel.rs 折叠/悬停映射",
            include_str!("../host_adapters/usage_panel.rs"),
            &[
                "AppMessage::Hover(HoverId::list_collapse(PanelKind::Usage), hovered)",
                "AppMessage::TogglePanelListCollapse(PanelKind::Usage)",
            ],
        );
        in_order(
            "update.rs Ready 处理",
            update,
            &[
                "Message::UsageContentWebviewEvent(event, surface)",
                "self.usage_surface.accepts(surface)",
                "UsageWebviewEvent::Ready",
                "self.usage_surface.apply_ready()",
            ],
        );
        in_order(
            "runtime.rs IPC",
            include_str!("../runtime.rs"),
            &["USAGE_CONTENT_ID_OFFSET", "parse_usage_event"],
        );
        // 真实函数的提前返回个数被钉住:新增一个提前返回(如 `if failed`)就失败。
        let app = include_str!("../app/app.rs");
        let start = app.find("pub fn take_usage_content_script").unwrap();
        let real = &app[start
            ..start
                + app[start..]
                    .find("pub fn take_codehealth_content_script")
                    .unwrap()];
        assert_eq!(
            real.matches("return Vec::new()").count(),
            2,
            "真实函数的提前返回个数变了,夹具 tick 需同步"
        );
        // 夹具侧同样钉住:tick 的步骤顺序与提前返回个数。
        let me = include_str!("host.rs");
        let s = me.find("pub(crate) fn tick").unwrap();
        let ours = &me[s..s + me[s..].find("/// 对应 `runtime.rs` 的 IPC 识别").unwrap()];
        in_order(
            "ProbeHost::tick",
            ours,
            &[
                "observe_instance",
                "is_attached()",
                "loading()",
                "current_view_payload",
                "set_desired",
                "take_pending_script",
            ],
        );
        assert_eq!(
            ours.matches("return;").count(),
            3,
            "夹具 tick 的提前返回个数变了(离池 + 加载中 + 无待发脚本)"
        );
    }
}
