//! `ProbeHost`:扮演 `App` 对 Usage 面板做的那部分事。每一件"面板自己做不了、由 host 代做"的事都记进
//! `duties`,这份日志就是缺口报告的原始数据。**逻辑从真实 host 手抄而来**,见各方法注释里的出处;
//! `drift_guard` 测试负责在出处改动时报警。

use super::fake_dozerd::FakeDozerd;
use super::surface::FakeSurface;
use crate::extensions::usage::{self, Message, WebviewPushState, WorkspaceState};
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
}

impl Duty {
    /// 真实 host 里承担这件事的位置(读代码得到,2026-10-08)。
    pub(crate) fn host_site(self) -> &'static str {
        match self {
            Duty::BuildActivationCtx => "app/update.rs:5251",
            Duty::RouteLoaded => "app/update.rs:1621 (with_project)",
            Duty::DropOtherProject => "app/update.rs:1621 (with_project 路由到别的 Workspace)",
            Duty::InterceptCollapse => "app/update.rs:1626",
            Duty::InterceptHover => "app/update.rs:1629",
            Duty::FramePush => "app/app.rs:1242 take_usage_content_script",
            Duty::DowngradeReady => "app/app.rs:1250",
            Duty::SkipWhileLoading => "app/app.rs:1255",
            Duty::ParseIpcAndMarkReady => "runtime.rs:705 + app/update.rs:261",
        }
    }
}

pub(crate) struct ProbeHost {
    pub ws: WorkspaceState,
    pub push: WebviewPushState,
    pub surface: FakeSurface,
    pub duties: Vec<Duty>,
    project_id: i64,
}

impl ProbeHost {
    pub(crate) fn new(project_id: i64) -> Self {
        Self {
            ws: WorkspaceState::default(),
            push: WebviewPushState::default(),
            surface: FakeSurface::default(),
            duties: Vec::new(),
            project_id,
        }
    }

    /// 对应 `app/update.rs:5251`:host 构造 `ActivationCtx` 再调面板的 `on_activate`。
    pub(crate) fn activate(
        &mut self,
        rt: &tokio::runtime::Runtime,
        dozerd: &FakeDozerd,
        path: Option<PathBuf>,
    ) -> mpsc::Receiver<Message> {
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let client = dozer_client::Client::new(dozerd.socket().to_path_buf());
        let io = PanelIo::new(client, rt.handle().clone(), move |m| {
            let _ = tx.lock().unwrap().send(m);
        });
        let ctx = ActivationCtx {
            project_id: self.project_id,
            project_path: path,
            io,
        };
        self.duties.push(Duty::BuildActivationCtx);
        usage::on_activate(&mut self.ws, Some(&ctx));
        rx
    }

    /// 对应 `app/update.rs:1621-1633` 的路由:`Loaded` 按 `project_id` 路由(这里只有一个 Workspace,
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
            Message::ToggleListCollapse => self.duties.push(Duty::InterceptCollapse),
            Message::Hover(..) => self.duties.push(Duty::InterceptHover),
            msg => usage::update(&mut self.ws, msg),
        }
    }

    /// 对应 `App::take_usage_content_script`(`app/app.rs:1242`):每帧声明式比较"该显示什么"与"上次送达什么"。
    pub(crate) fn tick(&mut self) {
        self.duties.push(Duty::FramePush);
        if !self.surface.is_attached() {
            self.push.set_ready(false);
            self.duties.push(Duty::DowngradeReady);
            return;
        }
        if self.ws.loading() {
            self.duties.push(Duty::SkipWhileLoading);
            return;
        }
        let desired = usage::current_view_payload(&self.ws);
        let Some(payload) = self.push.pending_push(&desired) else {
            return;
        };
        let envelope = usage::encode_usage_push(payload.clone());
        self.push.mark_sent(payload);
        self.surface.receive(envelope);
    }

    /// 对应 `runtime.rs:705` 的 IPC 识别 + `app/update.rs:261` 的 `Ready` 处理。
    pub(crate) fn webview_ready(&mut self) {
        self.duties.push(Duty::ParseIpcAndMarkReady);
        let event = usage::parse_usage_event(r#"{"kind":"ready"}"#).unwrap();
        if matches!(event, usage::UsageWebviewEvent::Ready) {
            self.push.set_ready(true);
        }
    }

    pub(crate) fn duty_set(&self) -> HashSet<Duty> {
        self.duties.iter().copied().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::usage::UsageViewPayload;

    #[test]
    fn tick_without_a_surface_downgrades_ready_and_pushes_nothing() {
        let mut h = ProbeHost::new(1);
        h.push.set_ready(true);
        h.tick();
        assert_eq!(h.surface.count(), 0);
        assert!(h.duty_set().contains(&Duty::DowngradeReady));
        assert!(
            h.push.pending_push(&UsageViewPayload::Empty).is_none(),
            "ready 已被降级"
        );
    }

    #[test]
    fn webview_ready_then_tick_pushes_once_and_dedups() {
        let mut h = ProbeHost::new(1);
        h.surface.attach();
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
            "available_webview_ids.contains",
            "set_ready(false)",
            "active_workspace()",
            "usage.loading()",
            "current_view_payload",
            "pending_push",
            "encode_usage_push",
            "mark_sent",
        ];
        let mut at = 0;
        for needle in order {
            let found = body[at..].find(needle).unwrap_or_else(|| {
                panic!("take_usage_content_script 里 `{needle}` 不在预期位置之后,夹具需同步")
            });
            at += found;
        }
    }
}
