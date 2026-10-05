//! 设置弹窗的「应用」页(bytehost A4c):运行时探测、已安装应用的停止/卸载、安装流程
//! (选目录 → 出安装计划 → **展示给用户审批** → 安装)。**纯状态机**:`update` 只返回 [`Effect`],
//! 由 `settings::update` 去执行(发请求、弹 Toast),所以不开窗口、不连 dozerd 也能测。
//!
//! 要点:
//! - **审批的就是展示的**:用户看到的 `InstallPlan` 原封不动地 `approve` 并交给 dozerd;dozerd 对 staging
//!   副本重新计算并核对(摘要/整份计划),不一致就拒绝——这里不重算也不改写计划;
//! - **强制等级原样展示**(`Enforced`/`Advisory`/`Unsupported`),不替权限"美化":一条只是声明、没被强制的权限
//!   必须看得出来;
//! - 流程里的校验类失败(清单不合法、版本已装、审批后源码变了)**留在对话流程内**(同"移动对话框内的校验
//!   错误"的先例),不弹 Toast;"刚发生的一件事"(安装成功、停止/卸载失败)走 Toast;
//! - 来源只有"用户在本机选的目录"(`Provenance::Local`、`TrustLevel::Trusted`);Agent 生成/第三方来源的
//!   安装不经这个界面。

use std::path::PathBuf;

use bytehost_apps::permissions::{Enforcement, PermissionKey};
use bytehost_apps::plan::{Approval, ApprovedInstallPlan, InstallPlan, Provenance, TrustLevel};
use bytehost_apps::proto::{AppSummary, RuntimeAvailability, RuntimeProbe};
use bytehost_apps::registry::UninstallMode;
use bytehost_apps::state::ObservedState;

use crate::extensions::app_host::Failure;
use crate::extensions::toast::Level;

/// 批准记录里写的"谁批准的"(审计用,不防伪——见 `ApprovedInstallPlan` 文档)。
pub const APPROVER: &str = "dozer-gui";

#[derive(Debug, Clone, PartialEq, Default)]
pub enum Load<T> {
    #[default]
    Loading,
    Loaded(T),
    Failed(String),
}

/// 安装/卸载对话流程(同一时刻只有一个)。
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Flow {
    #[default]
    Idle,
    /// 原生选目录对话框开着(由窗口层执行,结果经 `SourcePicked` 回来)。
    Picking,
    Planning {
        source: PathBuf,
    },
    /// 计划已展示,等用户批准。
    Reviewing {
        plan: Box<InstallPlan>,
        source: PathBuf,
    },
    Installing {
        plan: Box<InstallPlan>,
    },
    /// 流程内的失败(原因给人看);「知道了」回到 `Idle`。
    Failed {
        message: String,
    },
    /// 卸载确认:选"保留数据"还是"连数据一起删"。
    ConfirmUninstall {
        id: String,
        name: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActKind {
    Stop,
    Uninstall,
}

#[derive(Debug, Clone)]
pub enum Message {
    /// 切到「应用」页(或设置弹窗以它打开)。
    Opened,
    ProbesLoaded(Result<Vec<RuntimeProbe>, Failure>),
    ListLoaded(Result<Vec<AppSummary>, Failure>),
    InstallClicked,
    /// 选目录对话框的结果;`None` = 用户取消。
    SourcePicked(Option<PathBuf>),
    PlanLoaded(Result<Box<InstallPlan>, Failure>),
    ApproveClicked,
    /// 取消/关闭当前对话步骤(审批中取消、失败页「知道了」、卸载确认取消)。
    FlowDismissed,
    InstallDone(Result<(), Failure>),
    StopClicked(String),
    UninstallClicked(String),
    UninstallConfirmed(UninstallMode),
    ActionDone(String, ActKind, Result<(), Failure>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    Probe,
    List,
    /// 窗口层弹原生选目录对话框(`window_events` 拦截 `InstallClicked` 执行,这里只是记录意图)。
    PickSource,
    Plan(PathBuf),
    Install {
        approved: Box<ApprovedInstallPlan>,
        source: PathBuf,
    },
    Stop(String),
    Uninstall(String, UninstallMode),
    /// 已安装集合/运行状态变了:通知主窗口的应用宿主立刻刷新列表(同步图标栏)。
    HostChanged,
    Toast {
        level: Level,
        text: String,
        key: String,
    },
}

#[derive(Debug, Default)]
pub struct State {
    pub probes: Load<Vec<RuntimeProbe>>,
    pub apps: Load<Vec<AppSummary>>,
    pub flow: Flow,
    acting: std::collections::HashMap<String, ActKind>,
}

impl State {
    pub fn update(&mut self, msg: Message, now_ms: u64) -> Vec<Effect> {
        match msg {
            Message::Opened => {
                self.probes = Load::Loading;
                self.apps = Load::Loading;
                vec![Effect::Probe, Effect::List]
            }
            Message::ProbesLoaded(result) => {
                self.probes = match result {
                    Ok(p) => Load::Loaded(p),
                    Err(f) => Load::Failed(f.text().to_owned()),
                };
                Vec::new()
            }
            Message::ListLoaded(result) => {
                self.apps = match result {
                    Ok(a) => Load::Loaded(a),
                    Err(f) => Load::Failed(f.text().to_owned()),
                };
                Vec::new()
            }
            Message::InstallClicked => {
                if self.flow != Flow::Idle {
                    return Vec::new();
                }
                self.flow = Flow::Picking;
                vec![Effect::PickSource]
            }
            Message::SourcePicked(picked) => {
                if self.flow != Flow::Picking {
                    return Vec::new();
                }
                match picked {
                    None => {
                        self.flow = Flow::Idle;
                        Vec::new()
                    }
                    Some(source) => {
                        self.flow = Flow::Planning {
                            source: source.clone(),
                        };
                        vec![Effect::Plan(source)]
                    }
                }
            }
            Message::PlanLoaded(result) => {
                let Flow::Planning { source } = &self.flow else {
                    return Vec::new();
                };
                let source = source.clone();
                self.flow = match result {
                    Ok(plan) => Flow::Reviewing { plan, source },
                    Err(f) => Flow::Failed {
                        message: f.text().to_owned(),
                    },
                };
                Vec::new()
            }
            Message::ApproveClicked => {
                let Flow::Reviewing { plan, source } = &self.flow else {
                    return Vec::new();
                };
                let (plan, source) = (plan.clone(), source.clone());
                // 批准的就是展示的这一份:不重算、不改写。
                let approved = plan.as_ref().clone().approve(Approval {
                    approver: APPROVER.into(),
                    approved_ms: now_ms,
                });
                self.flow = Flow::Installing { plan };
                vec![Effect::Install {
                    approved: Box::new(approved),
                    source,
                }]
            }
            Message::FlowDismissed => {
                // 在途的规划/安装不能取消(请求已发出);其余步骤都可以关。
                if matches!(
                    self.flow,
                    Flow::Reviewing { .. } | Flow::Failed { .. } | Flow::ConfirmUninstall { .. }
                ) {
                    self.flow = Flow::Idle;
                }
                Vec::new()
            }
            Message::InstallDone(result) => {
                let Flow::Installing { plan } = &self.flow else {
                    return Vec::new();
                };
                match result {
                    Ok(()) => {
                        let text = format!("已安装应用 {}", plan.name);
                        let key = format!("apps:install:{}", plan.app_id.as_str());
                        self.flow = Flow::Idle;
                        vec![
                            Effect::Toast {
                                level: Level::Success,
                                text,
                                key,
                            },
                            Effect::List,
                            Effect::HostChanged,
                        ]
                    }
                    Err(f) => {
                        self.flow = Flow::Failed {
                            message: format!("安装失败:{}", f.text()),
                        };
                        Vec::new()
                    }
                }
            }
            Message::StopClicked(id) => {
                if self.acting.contains_key(&id) || self.flow != Flow::Idle {
                    return Vec::new();
                }
                self.acting.insert(id.clone(), ActKind::Stop);
                vec![Effect::Stop(id)]
            }
            Message::UninstallClicked(id) => {
                if self.acting.contains_key(&id) || self.flow != Flow::Idle {
                    return Vec::new();
                }
                let name = self.display_name(&id);
                self.flow = Flow::ConfirmUninstall { id, name };
                Vec::new()
            }
            Message::UninstallConfirmed(mode) => {
                let Flow::ConfirmUninstall { id, .. } = &self.flow else {
                    return Vec::new();
                };
                let id = id.clone();
                self.flow = Flow::Idle;
                self.acting.insert(id.clone(), ActKind::Uninstall);
                vec![Effect::Uninstall(id, mode)]
            }
            Message::ActionDone(id, kind, result) => {
                self.acting.remove(&id);
                let mut effects = Vec::new();
                if let Err(f) = result {
                    let verb = match kind {
                        ActKind::Stop => "停止",
                        ActKind::Uninstall => "卸载",
                    };
                    effects.push(Effect::Toast {
                        level: Level::Error,
                        text: format!("{verb}应用 {} 失败:{}", self.display_name(&id), f.text()),
                        key: format!("apps:act:{id}"),
                    });
                }
                // 成败都刷新:列表反映真相,图标栏同步。
                effects.push(Effect::List);
                effects.push(Effect::HostChanged);
                effects
            }
        }
    }

    pub fn is_acting(&self, id: &str) -> Option<ActKind> {
        self.acting.get(id).copied()
    }

    fn display_name(&self, id: &str) -> String {
        match &self.apps {
            Load::Loaded(apps) => apps
                .iter()
                .find(|a| a.id.as_str() == id)
                .map(|a| a.name.clone())
                .unwrap_or_else(|| id.to_owned()),
            _ => id.to_owned(),
        }
    }
}

/// 设置窗口已经关掉(状态没了)时到达的安装/停止/卸载结果:状态机已不存在,但事情照样发生了——
/// 仍要通知主窗口刷新应用列表(同步图标栏),并把失败/成功告诉用户(Toast)。其余消息在没有窗口时无意义,忽略。
pub fn orphan_result_effects(msg: &Message) -> Vec<Effect> {
    match msg {
        Message::InstallDone(Ok(())) => vec![
            Effect::Toast {
                level: Level::Success,
                text: "应用已安装".into(),
                key: "apps:install".into(),
            },
            Effect::HostChanged,
        ],
        Message::InstallDone(Err(f)) => vec![
            Effect::Toast {
                level: Level::Error,
                text: format!("安装失败:{}", f.text()),
                key: "apps:install".into(),
            },
            Effect::HostChanged,
        ],
        Message::ActionDone(id, kind, result) => {
            let mut effects = Vec::new();
            if let Err(f) = result {
                let verb = match kind {
                    ActKind::Stop => "停止",
                    ActKind::Uninstall => "卸载",
                };
                effects.push(Effect::Toast {
                    level: Level::Error,
                    text: format!("{verb}应用 {id} 失败:{}", f.text()),
                    key: format!("apps:act:{id}"),
                });
            }
            effects.push(Effect::HostChanged);
            effects
        }
        _ => Vec::new(),
    }
}

// ---------------------------------------------------------------------------------------------
// 展示用的纯函数(可测):计划 → 要给用户看的行;运行时探测 → 一行文案;观察态 → 文案。
// ---------------------------------------------------------------------------------------------

pub fn provenance_label(p: Provenance) -> &'static str {
    match p {
        Provenance::Local => "本地目录",
        Provenance::AgentGenerated => "Agent 生成",
        Provenance::ThirdParty => "第三方来源",
    }
}

pub fn trust_label(t: TrustLevel) -> &'static str {
    match t {
        TrustLevel::Trusted => "受信任",
        TrustLevel::Untrusted => "不受信任",
    }
}

pub fn enforcement_label(e: Option<Enforcement>) -> &'static str {
    match e {
        Some(Enforcement::Enforced) => "由宿主强制",
        Some(Enforcement::Advisory) => "仅声明,不强制",
        Some(Enforcement::Unsupported) => "此运行方式不支持",
        None => "未知(按不强制处理)",
    }
}

fn permission_label(key: PermissionKey) -> &'static str {
    match key {
        PermissionKey::NetworkOutbound => "出站网络",
        PermissionKey::FilesystemData => "应用数据目录",
        PermissionKey::Clipboard => "剪贴板",
        PermissionKey::Downloads => "下载",
        PermissionKey::Popups => "弹窗",
    }
}

/// 权限取值的中文(来自 `Permissions::level` 的稳定英文标签;未知标签原样显示,不吞)。
fn level_label(raw: &str) -> String {
    match raw {
        "none" => "无",
        "any" => "任意",
        "read" => "只读",
        "read_write" => "读写",
        "deny" => "禁止",
        "user_confirm" => "需确认",
        "allow" => "允许",
        other => return other.to_owned(),
    }
    .to_owned()
}

#[derive(Debug, Clone, PartialEq)]
pub struct PermLine {
    pub label: &'static str,
    /// 例如 `无 → 任意`。
    pub change: String,
    /// 申请比当前授予更多(升级时需要重新确认)。
    pub escalation: bool,
    pub enforcement: &'static str,
}

/// 安装计划里要展示给用户的全部内容。
#[derive(Debug, Clone, PartialEq)]
pub struct PlanView {
    pub title: String,
    /// `(项, 值)` 事实行:来源、信任、运行方式、版本变化。
    pub facts: Vec<(&'static str, String)>,
    /// 安装/运行时会执行什么(原样展示)。
    pub will_run: Vec<String>,
    pub permissions: Vec<PermLine>,
}

pub fn plan_view(plan: &InstallPlan) -> PlanView {
    let version = match &plan.upgrading_from {
        Some(from) => format!("{from} → {}", plan.version),
        None => plan.version.to_string(),
    };
    let permissions = plan
        .permission_diff
        .iter()
        .map(|c| PermLine {
            label: permission_label(c.key),
            change: format!("{} → {}", level_label(&c.from), level_label(&c.to)),
            escalation: c.escalation,
            enforcement: enforcement_label(
                plan.enforcement
                    .iter()
                    .find(|e| e.key == c.key)
                    .map(|e| e.enforcement),
            ),
        })
        .collect();
    PlanView {
        title: format!("{}({})", plan.name, plan.app_id.as_str()),
        facts: vec![
            ("版本", version),
            ("来源", provenance_label(plan.provenance).to_owned()),
            ("信任", trust_label(plan.trust).to_owned()),
            ("运行方式", plan.runtime_kind.clone()),
        ],
        will_run: plan.will_run.clone(),
        permissions,
    }
}

/// 一行运行时探测文案 `(名称, 状态文字, 是否可用)`。
pub fn runtime_line(probe: &RuntimeProbe) -> (String, String, bool) {
    let name = match probe.runtime.as_str() {
        "docker" => "Docker",
        "node" => "Node.js",
        "python" => "Python",
        other => other,
    }
    .to_owned();
    match &probe.availability {
        RuntimeAvailability::Available { detail } => (name, format!("可用 · {detail}"), true),
        RuntimeAvailability::Unavailable { detail } => {
            (name, format!("已安装但当前不可用 · {detail}"), false)
        }
        RuntimeAvailability::NotInstalled => (name, "未安装".to_owned(), false),
    }
}

pub fn observed_label(state: &ObservedState) -> String {
    match state {
        ObservedState::Running => "运行中".into(),
        ObservedState::Stopped | ObservedState::Installed => "已停止".into(),
        ObservedState::Preparing | ObservedState::Starting | ObservedState::Updating => {
            "启动中".into()
        }
        ObservedState::Stopping | ObservedState::Uninstalling => "停止中".into(),
        ObservedState::NotInstalled => "未安装".into(),
        ObservedState::Failed { reason, .. } => format!("已退出 · {reason}"),
    }
}

// ---------------------------------------------------------------------------------------------
// 视图(只画 `State` 与上面的纯展示函数,不含逻辑)。
// ---------------------------------------------------------------------------------------------

use iced_widget::core::{Alignment, Element, Length};
use iced_widget::{Space, button, column, container, row, scrollable, text};

type El<'a> = Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer>;

fn action_button<'a>(label: &'static str, msg: Message, color: iced_widget::core::Color) -> El<'a> {
    button(text(label).size(byteui::theme::font::body()))
        .padding([4, 12])
        .on_press(msg)
        .style(byteui::feedback::dialog::action_button_style(color))
        .into()
}

fn heading<'a>(label: &'static str) -> El<'a> {
    text(label)
        .size(byteui::theme::font::subtitle())
        .color(byteui::theme::color::current().cream)
        .into()
}

fn dim<'a>(
    s: impl text::IntoFragment<'a>,
) -> iced_widget::Text<'a, iced_widget::Theme, iced_renderer::Renderer> {
    text(s)
        .size(byteui::theme::font::label())
        .color(byteui::theme::color::current().dim)
}

pub fn view(state: &State) -> El<'_> {
    let colors = byteui::theme::color::current();
    let mut body = column![heading("应用")].spacing(12).width(Length::Fill);

    // 运行时探测(只展示;一期只有静态 Web 应用,不依赖它们)。
    body = body.push(dim("运行时(Python/Node/容器类应用以后才支持,这里只探测)"));
    let probes_view: El<'_> = match &state.probes {
        Load::Loading => dim("检测中…").into(),
        Load::Failed(why) => dim(format!("探测失败:{why}")).into(),
        Load::Loaded(probes) => {
            let mut col = column![].spacing(4);
            for probe in probes {
                let (name, status, ok) = runtime_line(probe);
                col = col.push(
                    row![
                        text(name)
                            .size(byteui::theme::font::body())
                            .color(colors.cream),
                        Space::new().width(Length::Fill),
                        text(status)
                            .size(byteui::theme::font::label())
                            .color(if ok { colors.gold } else { colors.dim }),
                    ]
                    .spacing(10),
                );
            }
            col.into()
        }
    };
    body = body.push(probes_view);

    // 已安装应用。
    body = body.push(heading("已安装"));
    let apps_view: El<'_> = match &state.apps {
        Load::Loading => dim("读取中…").into(),
        Load::Failed(why) => dim(format!("读取失败:{why}")).into(),
        Load::Loaded(apps) if apps.is_empty() => dim("还没有安装任何应用").into(),
        Load::Loaded(apps) => {
            let mut col = column![].spacing(6);
            for app in apps {
                col = col.push(app_row(state, app));
            }
            col.into()
        }
    };
    body = body.push(apps_view);

    // 安装/卸载流程。
    body = body.push(flow_view(&state.flow));

    scrollable(body)
        .direction(scrollable::Direction::Vertical(
            byteui::interaction::scrollbar::scrollbar(),
        ))
        .style(|_t, _s| byteui::interaction::scrollbar::scrollbar_style())
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn app_row<'a>(state: &'a State, app: &'a AppSummary) -> El<'a> {
    let colors = byteui::theme::color::current();
    let id = app.id.as_str().to_owned();
    let mut r = row![
        text(app.name.as_str())
            .size(byteui::theme::font::body())
            .color(colors.cream),
        dim(format!(
            "{} · {}",
            app.version,
            observed_label(&app.observed)
        )),
        Space::new().width(Length::Fill),
    ]
    .spacing(10)
    .align_y(Alignment::Center);
    match state.is_acting(&id) {
        Some(_) => r = r.push(dim("处理中…")),
        None => {
            if matches!(
                app.observed,
                ObservedState::Running | ObservedState::Starting
            ) {
                r = r.push(action_button(
                    "停止",
                    Message::StopClicked(id.clone()),
                    colors.dim,
                ));
            }
            if !app.observed.is_transient() {
                r = r.push(action_button(
                    "卸载",
                    Message::UninstallClicked(id),
                    colors.red,
                ));
            }
        }
    }
    r.into()
}

fn flow_view(flow: &Flow) -> El<'_> {
    let colors = byteui::theme::color::current();
    match flow {
        Flow::Idle => action_button("安装应用…", Message::InstallClicked, colors.gold),
        Flow::Picking => dim("请在弹出的对话框里选择应用目录(目录里要有 manifest.toml)…").into(),
        Flow::Planning { .. } => dim("正在检查应用…").into(),
        Flow::Installing { .. } => dim("正在安装…").into(),
        Flow::Failed { message } => column![
            text(message.as_str())
                .size(byteui::theme::font::body())
                .color(colors.red),
            action_button("知道了", Message::FlowDismissed, colors.dim),
        ]
        .spacing(8)
        .into(),
        Flow::ConfirmUninstall { name, .. } => column![
            text(format!("卸载应用「{name}」?"))
                .size(byteui::theme::font::body())
                .color(colors.cream),
            dim("「保留数据」只删程序,重装后数据还在;「连数据一起删」不可恢复。"),
            row![
                action_button("取消", Message::FlowDismissed, colors.dim),
                action_button(
                    "保留数据",
                    Message::UninstallConfirmed(UninstallMode::Program),
                    colors.gold
                ),
                action_button(
                    "连数据一起删",
                    Message::UninstallConfirmed(UninstallMode::ProgramAndData),
                    colors.red
                ),
            ]
            .spacing(8),
        ]
        .spacing(8)
        .into(),
        Flow::Reviewing { plan, .. } => review_view(plan),
    }
}

/// 审批卡:把 `plan_view` 的每一行如实画出来;升级(申请比已授予更多)的权限用金色标 `↑`。
fn review_view(plan: &InstallPlan) -> El<'_> {
    let colors = byteui::theme::color::current();
    let view = plan_view(plan);
    let mut col = column![
        text(format!("安装 {}", view.title))
            .size(byteui::theme::font::body())
            .color(colors.cream),
    ]
    .spacing(6);
    for (k, v) in view.facts {
        col = col.push(
            row![
                dim(k),
                text(v)
                    .size(byteui::theme::font::label())
                    .color(colors.cream)
            ]
            .spacing(10),
        );
    }
    col = col.push(dim("安装与运行时会执行:"));
    for line in view.will_run {
        col = col.push(
            text(line)
                .size(byteui::theme::font::label())
                .color(colors.cream),
        );
    }
    col = col.push(dim("申请的权限:"));
    if view.permissions.is_empty() {
        col = col.push(
            text("不申请任何权限")
                .size(byteui::theme::font::label())
                .color(colors.cream),
        );
    }
    for p in view.permissions {
        col = col.push(
            row![
                text(if p.escalation { "↑" } else { " " })
                    .size(byteui::theme::font::label())
                    .color(colors.gold),
                text(p.label)
                    .size(byteui::theme::font::label())
                    .color(colors.cream),
                text(p.change)
                    .size(byteui::theme::font::label())
                    .color(colors.cream),
                Space::new().width(Length::Fill),
                dim(p.enforcement),
            ]
            .spacing(10),
        );
    }
    col = col.push(
        row![
            action_button("取消", Message::FlowDismissed, colors.dim),
            action_button("批准并安装", Message::ApproveClicked, colors.gold),
        ]
        .spacing(8),
    );
    container(col).padding(10).width(Length::Fill).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytehost_apps::id::{AppId, Version};
    use bytehost_apps::permissions::{Access, Gate, Outbound, Permissions, diff_permissions};
    use bytehost_apps::plan::EnforcementEntry;
    use bytehost_apps::proto::{AppErrorKind, AppFailure};
    use bytehost_apps::state::DesiredState;

    const NOW: u64 = 1_700_000_000_000;

    fn plan() -> InstallPlan {
        let requested = Permissions {
            network: bytehost_apps::permissions::NetworkPerm {
                outbound: Outbound::Any,
            },
            downloads: Gate::UserConfirm,
            ..Permissions::default()
        };
        InstallPlan {
            app_id: AppId::new("excalidraw").unwrap(),
            name: "Excalidraw".into(),
            version: Version::new(0, 17, 0),
            upgrading_from: None,
            provenance: Provenance::Local,
            trust: TrustLevel::Trusted,
            runtime_kind: "static_web".into(),
            manifest_digest: "m".into(),
            source_digest: "s".into(),
            requested,
            enforcement: vec![
                EnforcementEntry {
                    key: PermissionKey::NetworkOutbound,
                    enforcement: Enforcement::Advisory,
                },
                EnforcementEntry {
                    key: PermissionKey::Downloads,
                    enforcement: Enforcement::Unsupported,
                },
            ],
            permission_diff: diff_permissions(&Permissions::default(), &requested),
            will_run: vec!["不执行任何命令".into()],
        }
    }

    fn fail(kind: AppErrorKind, msg: &str) -> Failure {
        Failure::Host(AppFailure::new(kind, msg))
    }

    fn summary(id: &str, observed: ObservedState) -> AppSummary {
        AppSummary {
            id: AppId::new(id).unwrap(),
            name: format!("name-{id}"),
            version: Version::new(1, 0, 0),
            desired: DesiredState::Running,
            observed,
            url: None,
        }
    }

    fn reviewing() -> State {
        let mut s = State::default();
        s.update(Message::InstallClicked, NOW);
        s.update(Message::SourcePicked(Some("/src/app".into())), NOW);
        s.update(Message::PlanLoaded(Ok(Box::new(plan()))), NOW);
        assert!(matches!(s.flow, Flow::Reviewing { .. }));
        s
    }

    #[test]
    fn opening_the_page_probes_runtimes_and_lists_apps() {
        let mut s = State::default();
        assert_eq!(
            s.update(Message::Opened, NOW),
            vec![Effect::Probe, Effect::List]
        );
        assert_eq!(s.probes, Load::Loading);
    }

    #[test]
    fn the_install_flow_goes_pick_plan_review_and_cancelling_the_picker_returns_to_idle() {
        let mut s = State::default();
        assert_eq!(
            s.update(Message::InstallClicked, NOW),
            vec![Effect::PickSource]
        );
        assert_eq!(s.flow, Flow::Picking);
        assert!(
            s.update(Message::InstallClicked, NOW).is_empty(),
            "流程中不接受第二次安装"
        );
        s.update(Message::SourcePicked(None), NOW);
        assert_eq!(s.flow, Flow::Idle);
        s.update(Message::InstallClicked, NOW);
        let effects = s.update(Message::SourcePicked(Some("/src/app".into())), NOW);
        assert_eq!(effects, vec![Effect::Plan("/src/app".into())]);
        s.update(Message::PlanLoaded(Ok(Box::new(plan()))), NOW);
        assert!(matches!(s.flow, Flow::Reviewing { .. }));
    }

    #[test]
    fn a_source_picked_outside_the_picking_step_is_ignored() {
        let mut s = State::default();
        assert!(
            s.update(Message::SourcePicked(Some("/x".into())), NOW)
                .is_empty()
        );
        assert_eq!(s.flow, Flow::Idle);
    }

    #[test]
    fn a_rejected_plan_stays_inside_the_flow_as_an_inline_failure() {
        let mut s = State::default();
        s.update(Message::InstallClicked, NOW);
        s.update(Message::SourcePicked(Some("/src/app".into())), NOW);
        let effects = s.update(
            Message::PlanLoaded(Err(fail(AppErrorKind::Rejected, "manifest 缺少 id"))),
            NOW,
        );
        assert!(effects.is_empty(), "校验错误不弹 Toast");
        assert_eq!(
            s.flow,
            Flow::Failed {
                message: "manifest 缺少 id".into()
            }
        );
        s.update(Message::FlowDismissed, NOW);
        assert_eq!(s.flow, Flow::Idle);
    }

    /// 批准的必须**就是展示的那份**:不重算、不改写,批准人/时间如实记录。
    #[test]
    fn approving_sends_exactly_the_plan_that_was_shown() {
        let mut s = reviewing();
        let effects = s.update(Message::ApproveClicked, NOW);
        assert_eq!(effects.len(), 1);
        let Effect::Install { approved, source } = &effects[0] else {
            panic!("{effects:?}")
        };
        assert_eq!(approved.plan(), &plan());
        assert_eq!(approved.approval().approver, APPROVER);
        assert_eq!(approved.approval().approved_ms, NOW);
        assert_eq!(source, &PathBuf::from("/src/app"));
        assert!(matches!(s.flow, Flow::Installing { .. }));
        assert!(
            s.update(Message::ApproveClicked, NOW).is_empty(),
            "不能重复批准"
        );
    }

    #[test]
    fn approving_is_only_possible_while_reviewing() {
        let mut s = State::default();
        assert!(s.update(Message::ApproveClicked, NOW).is_empty());
        s.update(Message::InstallClicked, NOW);
        assert!(s.update(Message::ApproveClicked, NOW).is_empty());
    }

    #[test]
    fn dismissing_reviews_cancels_but_in_flight_steps_cannot_be_cancelled() {
        let mut s = reviewing();
        s.update(Message::FlowDismissed, NOW);
        assert_eq!(s.flow, Flow::Idle);
        let mut s = reviewing();
        s.update(Message::ApproveClicked, NOW);
        s.update(Message::FlowDismissed, NOW);
        assert!(
            matches!(s.flow, Flow::Installing { .. }),
            "安装请求已发出,不能取消"
        );
    }

    #[test]
    fn a_successful_install_toasts_refreshes_and_tells_the_host() {
        let mut s = reviewing();
        s.update(Message::ApproveClicked, NOW);
        let effects = s.update(Message::InstallDone(Ok(())), NOW);
        assert_eq!(s.flow, Flow::Idle);
        assert!(
            matches!(&effects[0], Effect::Toast { level: Level::Success, text, key }
            if text.contains("Excalidraw") && key == "apps:install:excalidraw")
        );
        assert!(effects.contains(&Effect::List));
        assert!(effects.contains(&Effect::HostChanged));
    }

    #[test]
    fn a_failed_install_is_shown_in_the_flow_and_does_not_touch_the_host() {
        let mut s = reviewing();
        s.update(Message::ApproveClicked, NOW);
        let effects = s.update(
            Message::InstallDone(Err(fail(
                AppErrorKind::Rejected,
                "安装被拒绝: 应用源码在审批之后发生了变化",
            ))),
            NOW,
        );
        assert!(effects.is_empty());
        match &s.flow {
            Flow::Failed { message } => assert!(message.contains("源码在审批之后"), "{message}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn stop_is_deduplicated_and_always_refreshes_with_a_toast_only_on_failure() {
        let mut s = State::default();
        s.update(
            Message::ListLoaded(Ok(vec![summary("stop-a", ObservedState::Running)])),
            NOW,
        );
        assert_eq!(
            s.update(Message::StopClicked("stop-a".into()), NOW),
            vec![Effect::Stop("stop-a".into())]
        );
        assert!(
            s.update(Message::StopClicked("stop-a".into()), NOW)
                .is_empty()
        );
        assert_eq!(s.is_acting("stop-a"), Some(ActKind::Stop));
        let ok = s.update(
            Message::ActionDone("stop-a".into(), ActKind::Stop, Ok(())),
            NOW,
        );
        assert_eq!(ok, vec![Effect::List, Effect::HostChanged]);
        s.update(Message::StopClicked("stop-a".into()), NOW);
        let bad = s.update(
            Message::ActionDone(
                "stop-a".into(),
                ActKind::Stop,
                Err(fail(AppErrorKind::Conflict, "状态冲突")),
            ),
            NOW,
        );
        assert!(
            matches!(&bad[0], Effect::Toast { level: Level::Error, text, .. }
            if text.contains("停止") && text.contains("name-stop-a") && text.contains("状态冲突"))
        );
        assert_eq!(&bad[1..], &[Effect::List, Effect::HostChanged]);
    }

    #[test]
    fn uninstall_asks_which_mode_first_and_can_be_cancelled() {
        let mut s = State::default();
        s.update(
            Message::ListLoaded(Ok(vec![summary("un-a", ObservedState::Stopped)])),
            NOW,
        );
        assert!(
            s.update(Message::UninstallClicked("un-a".into()), NOW)
                .is_empty()
        );
        assert_eq!(
            s.flow,
            Flow::ConfirmUninstall {
                id: "un-a".into(),
                name: "name-un-a".into()
            }
        );
        s.update(Message::FlowDismissed, NOW);
        assert_eq!(s.flow, Flow::Idle);
        s.update(Message::UninstallClicked("un-a".into()), NOW);
        let effects = s.update(Message::UninstallConfirmed(UninstallMode::Program), NOW);
        assert_eq!(
            effects,
            vec![Effect::Uninstall("un-a".into(), UninstallMode::Program)]
        );
        assert_eq!(s.flow, Flow::Idle);
        assert_eq!(s.is_acting("un-a"), Some(ActKind::Uninstall));
    }

    #[test]
    fn uninstall_confirmation_is_ignored_outside_its_step() {
        let mut s = State::default();
        assert!(
            s.update(
                Message::UninstallConfirmed(UninstallMode::ProgramAndData),
                NOW
            )
            .is_empty()
        );
    }

    #[test]
    fn load_failures_are_kept_as_text_not_raised() {
        let mut s = State::default();
        s.update(
            Message::ListLoaded(Err(fail(AppErrorKind::Unavailable, "端口被占用"))),
            NOW,
        );
        assert_eq!(s.apps, Load::Failed("端口被占用".into()));
        s.update(
            Message::ProbesLoaded(Err(Failure::Transport("连不上".into()))),
            NOW,
        );
        assert_eq!(s.probes, Load::Failed("连不上".into()));
    }

    #[test]
    fn the_plan_view_shows_every_requested_change_with_its_real_enforcement() {
        let v = plan_view(&plan());
        assert_eq!(v.title, "Excalidraw(excalidraw)");
        assert!(v.facts.contains(&("版本", "0.17.0".to_string())));
        assert!(v.facts.contains(&("来源", "本地目录".to_string())));
        assert!(v.facts.contains(&("信任", "受信任".to_string())));
        assert_eq!(v.will_run, vec!["不执行任何命令".to_string()]);
        assert_eq!(
            v.permissions,
            vec![
                PermLine {
                    label: "出站网络",
                    change: "无 → 任意".into(),
                    escalation: true,
                    enforcement: "仅声明,不强制"
                },
                PermLine {
                    label: "下载",
                    change: "禁止 → 需确认".into(),
                    escalation: true,
                    enforcement: "此运行方式不支持"
                },
            ]
        );
    }

    #[test]
    fn the_plan_view_marks_a_missing_enforcement_entry_as_unknown_and_shows_upgrades() {
        let mut p = plan();
        p.enforcement.clear();
        p.upgrading_from = Some(Version::new(0, 16, 0));
        p.permission_diff = diff_permissions(
            &Permissions {
                clipboard: Access::ReadWrite,
                ..Permissions::default()
            },
            &Permissions {
                clipboard: Access::Read,
                ..Permissions::default()
            },
        );
        let v = plan_view(&p);
        assert!(v.facts.contains(&("版本", "0.16.0 → 0.17.0".to_string())));
        assert_eq!(
            v.permissions,
            vec![PermLine {
                label: "剪贴板",
                change: "读写 → 只读".into(),
                escalation: false,
                enforcement: "未知(按不强制处理)"
            }]
        );
    }

    #[test]
    fn runtime_lines_distinguish_available_unavailable_and_missing() {
        let probe = |runtime: &str, availability| RuntimeProbe {
            runtime: runtime.into(),
            availability,
        };
        assert_eq!(
            runtime_line(&probe(
                "node",
                RuntimeAvailability::Available {
                    detail: "v20.1.0".into()
                }
            )),
            ("Node.js".into(), "可用 · v20.1.0".into(), true)
        );
        assert_eq!(
            runtime_line(&probe(
                "docker",
                RuntimeAvailability::Unavailable {
                    detail: "守护进程未启动".into()
                }
            )),
            (
                "Docker".into(),
                "已安装但当前不可用 · 守护进程未启动".into(),
                false
            )
        );
        assert_eq!(
            runtime_line(&probe("python", RuntimeAvailability::NotInstalled)),
            ("Python".into(), "未安装".into(), false)
        );
    }

    #[test]
    fn observed_labels_cover_every_state() {
        assert_eq!(observed_label(&ObservedState::Running), "运行中");
        assert_eq!(observed_label(&ObservedState::Stopped), "已停止");
        assert_eq!(observed_label(&ObservedState::Starting), "启动中");
        assert_eq!(observed_label(&ObservedState::Stopping), "停止中");
        assert_eq!(
            observed_label(&ObservedState::Failed {
                reason: "崩了".into(),
                retryable: false
            }),
            "已退出 · 崩了"
        );
    }

    // ---- 设置窗口已关闭时到达的结果 ----

    #[test]
    fn results_that_arrive_after_the_settings_window_closed_still_refresh_the_host_and_toast() {
        let ok = orphan_result_effects(&Message::InstallDone(Ok(())));
        assert!(ok.contains(&Effect::HostChanged));
        assert!(ok.iter().any(|e| matches!(
            e,
            Effect::Toast {
                level: Level::Success,
                ..
            }
        )));

        let bad = orphan_result_effects(&Message::InstallDone(Err(fail(
            AppErrorKind::Rejected,
            "应用源码在审批之后发生了变化",
        ))));
        assert!(bad.iter().any(|e| matches!(e,
            Effect::Toast { level: Level::Error, text, .. } if text.contains("源码在审批之后"))));
        assert!(
            bad.contains(&Effect::HostChanged),
            "失败也刷新,列表反映真相"
        );

        let stop_ok =
            orphan_result_effects(&Message::ActionDone("x-a".into(), ActKind::Stop, Ok(())));
        assert_eq!(stop_ok, vec![Effect::HostChanged]);
        let un_bad = orphan_result_effects(&Message::ActionDone(
            "x-a".into(),
            ActKind::Uninstall,
            Err(fail(AppErrorKind::Conflict, "状态冲突")),
        ));
        assert!(un_bad.iter().any(|e| matches!(e,
            Effect::Toast { level: Level::Error, text, .. } if text.contains("卸载") && text.contains("x-a"))));
        assert!(un_bad.contains(&Effect::HostChanged));
    }

    #[test]
    fn only_install_and_action_results_are_handled_without_a_window() {
        for msg in [
            Message::Opened,
            Message::InstallClicked,
            Message::ApproveClicked,
            Message::FlowDismissed,
            Message::ListLoaded(Ok(vec![])),
            Message::ProbesLoaded(Ok(vec![])),
        ] {
            assert!(orphan_result_effects(&msg).is_empty(), "{msg:?}");
        }
    }
}
