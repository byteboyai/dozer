# bytehost A4c:设置里的「应用」页(安装审批 / 停止 / 卸载 / 运行时探测) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 补齐 A4 剩下的界面:在设置弹窗里加一页「应用」——展示 docker/node/python 运行时探测结果、列出已安装应用并能**停止/卸载**、**安装应用**(选本机目录 → dozerd 出安装计划 → 把计划如实展示给用户 → 用户批准 → 安装),安装/停止/卸载后主窗口的图标栏随之同步。

**Architecture:** 不新开独立原生窗口(那要在 `window_events.rs` 里接十几处):复用已经是独立原生窗口的设置弹窗,加 `SettingsTab::Apps`。新的纯状态机 `extensions/settings_apps.rs`(`update` 只返回 `Effect`,不碰网络/窗口),由 `settings::update` 里的 `run_apps_message` 执行副作用(经 `dozer-client::app_*`);选目录的原生对话框是阻塞的,和 `ProjectTabPickFolder` 一样由 `window_events.rs` 拦截 `InstallClicked` 在窗口层执行。设置页改了已安装集合/运行状态后置 `host_changed`,`App::update` 的包装函数取走并给 `app_host` 发 `Refresh`(立刻拉列表 → 同步图标栏)。

**Tech Stack:** Rust、iced 0.14、`dozer-client`、`bytehost-apps` 类型、`rfd`(已有依赖,选目录)。**不新增依赖。**

**Spec:** `docs/superpowers/specs/2026-10-04-bytehost-app-host-design.md`(A4 行、§6.3、§6.4);前置:A3、A4a、A4b1(含评审后修订)、A4b2(已在 main)。

## Global Constraints

- **审批的就是展示的**:用户看到的 `InstallPlan` 原封不动 `approve` 后交给 dozerd;GUI 不重算、不改写。dozerd 对 staging 副本重新计算并核对(摘要 + 整份计划),不一致即拒绝——审批后源码被改会得到"安装被拒绝"并**显示在流程里**。
- **强制等级原样展示**:每条申请的权限旁边写明 `由宿主强制`/`仅声明,不强制`/`此运行方式不支持`;找不到对应条目时写"未知(按不强制处理)",不得默认成"已强制"。
- **失败分流**(CLAUDE.md Toast 规则):流程内的校验类失败(清单不合法、版本已装、审批后源码变了)留在对话流程内(同"移动对话框内的校验错误"先例);"刚发生的一件事"(安装成功、停止/卸载失败)走 Toast(经 `settings.outbox`,不直接持有 `ToastCenter`)。
- **来源只有"用户在本机选的目录"**:`Provenance::Local`、`TrustLevel::Trusted`;批准记录 `approver = "dozer-gui"`。Agent 生成/第三方来源的安装不经这个界面。
- **卸载必须二选一确认**:「保留数据」(`UninstallMode::Program`)/「连数据一起删」(`ProgramAndData`,不可恢复),不提供一键卸载。
- 一期只有静态 Web 应用:运行时探测**只展示**,不提供安装 Python/Node/Docker(规格的"运行时安装"留到有 Python/Node adapter 时)。
- 设置弹窗已是独立原生窗口且不做失焦关闭(见 `window_events.rs` 对 settings 的注释),所以弹原生选目录对话框不会把它关掉。
- 字体/按钮沿用现有:系统默认字体,`byteui::feedback::dialog::action_button_style`。

## Review Focus

- 批准的计划就是展示的那份,批准人/时间如实记录(`approving_sends_exactly_the_plan_that_was_shown`)。
- 流程里的在途请求(规划/安装)不能被取消键"取消"掉(请求已发出)(`dismissing_reviews_cancels_but_in_flight_steps_cannot_be_cancelled`)。
- 同一时刻只有一个流程:选目录/审批/安装/卸载确认期间不接受第二次"安装/卸载/停止"。
- 停止/卸载**无论成败**都刷新列表并通知主窗口(图标栏同步),失败才弹 Toast。
- 取消选目录对话框回到空闲,不留"选择中"。
- 卸载一个当前显示着的应用:靠 A3/A4b2 的 `sync_installed_apps` + `view_or_first` 退到该栏第一个面板(这里只发 `Refresh`)。
- 强制等级缺失时的文案(`the_plan_view_marks_a_missing_enforcement_entry_as_unknown_and_shows_upgrades`)。

---

### Task 1: 「应用」页的纯状态机与展示函数

**Files:**
- Create: `crates/dozer-app/src/extensions/settings_apps.rs`
- Modify: `crates/dozer-app/src/extensions.rs`(`pub mod settings_apps;`)、`crates/dozer-app/src/extensions/toast.rs`(`Outbox::push_keyed` + 测试)

**Interfaces:**
- Produces: `settings_apps::{State, Message, Effect, Flow, Load, ActKind, PlanView, PermLine, plan_view, runtime_line, observed_label, enforcement_label, provenance_label, trust_label, view, APPROVER}`、`Outbox::push_keyed(scope, level, text, key)`。

- [x] **Step 1: 写失败测试。** 新文件的 `tests` 模块(17 个)先写,`State::update` 先 `todo!()`;`toast.rs` 的 `outbox_push_keyed_carries_the_dedupe_key`。
- [x] **Step 2: RED。** `cargo test -p dozer-app -- settings_apps outbox_push_keyed` → 失败。
- [x] **Step 3: 实现。** 新文件全文(含视图函数 `view`;视图只画 `State` 与展示函数,不含逻辑):

```rust
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
}
```

其余改动:

```diff
--- a/crates/dozer-app/src/extensions.rs
+++ b/crates/dozer-app/src/extensions.rs
@@ -21,6 +21,7 @@
 pub mod project_create;
 pub mod search;
 pub mod settings;
+pub mod settings_apps;
 pub mod ssh;
 pub mod toast;
 pub mod todo;
--- a/crates/dozer-app/src/extensions/toast.rs
+++ b/crates/dozer-app/src/extensions/toast.rs
@@ -133,6 +133,22 @@
         });
     }
 
+    /// 带去重键的版本:同 key 再推只刷新文本与计时(语义同 `App::push_toast_keyed`)。
+    pub fn push_keyed(
+        &mut self,
+        scope: Scope,
+        level: Level,
+        text: impl Into<String>,
+        key: impl Into<String>,
+    ) {
+        self.items.push(Pending {
+            scope,
+            level,
+            text: text.into(),
+            key: Some(key.into()),
+        });
+    }
+
     /// `Err(e)` 时推一条 `Error` 级 `"{what}: {e}"`;`Ok` 什么都不做。
     pub fn push_err<T, E: std::fmt::Display>(
         &mut self,
@@ -372,6 +388,17 @@
     }
 
     #[test]
+    fn outbox_push_keyed_carries_the_dedupe_key() {
+        let mut o = Outbox::default();
+        o.push_keyed(TEST_TODO, Level::Error, "停止失败", "apps:act:x");
+        let got = o.take();
+        assert_eq!(got.len(), 1);
+        assert_eq!(got[0].key.as_deref(), Some("apps:act:x"));
+        assert_eq!(got[0].text, "停止失败");
+        assert_eq!(got[0].level, Level::Error);
+    }
+
+    #[test]
     fn outbox_push_err_pushes_only_on_err_with_what_prefix() {
         let mut o = Outbox::default();
         o.push_err(TEST_TODO, "保存失败", &Ok::<(), String>(()));
```

- [x] **Step 4: GREEN。** `cargo fmt -p dozer-app && cargo test -p dozer-app -- settings_apps outbox` → 全过(`settings_apps` 17 个 + toast 新增 1 个)。
- [x] **Step 5: 变异检查(每条必须让对应测试 FAILED,再还原):** (a) `approved_ms: now_ms` 改成 `0`;(b) `FlowDismissed` 的可取消集合里加上 `Flow::Installing { .. }`;(c) `plan_view` 里 `.map(|e| e.enforcement)` 后追加 `.map(|_| Enforcement::Enforced)`;(d) `ActionDone` 失败分支里在 `Toast` 前多塞一个 `Effect::List`。
- [x] **Step 6: Commit。** `git commit -am "feat(dozer-app): settings_apps state machine, plan view and apps tab view (A4c task 1)"`

### Task 2: 接线(设置页、选目录、刷新主窗口)

**Files:** Modify `crates/dozer-app/src/extensions/app_host.rs`、`extensions/settings.rs`、`platform/window_events.rs`、`app/update.rs`

**Interfaces:**
- Consumes: Task 1;A4b2 的 `app_host::{Failure, State, Message}`(`Failure::text` 改为 `pub(crate)`)。
- Produces: `app_host::Message::Refresh`(列表在途时忽略,否则立刻拉一次)、`SettingsTab::Apps`、`settings::Message::Apps`、`settings::State.apps`/`take_host_changed()`、`run_apps_message`(私有)。

- [x] **Step 1: 写失败测试。** `app_host.rs` 的 `refresh_fetches_the_list_unless_one_is_already_in_flight`;`settings.rs` 里已有的 `test_state` 夹具补两个新字段(编译层面的改动,不是行为测试)。
- [x] **Step 2: RED。** `cargo test -p dozer-app app_host::tests::refresh` → 编译失败(`Message::Refresh` 不存在)。
- [x] **Step 3: 实现。**

```diff
--- a/crates/dozer-app/src/extensions/app_host.rs
+++ b/crates/dozer-app/src/extensions/app_host.rs
@@ -39,7 +39,7 @@
         }
     }
 
-    fn text(&self) -> &str {
+    pub(crate) fn text(&self) -> &str {
         match self {
             Self::Host(f) => &f.message,
             Self::Transport(t) => t,
@@ -63,6 +63,8 @@
     Stop(AppSlot),
     /// 该应用面板被切到前台(图标栏点击或程序化显示)。
     PanelShown(AppSlot),
+    /// 别处(设置里的安装/停止/卸载)改了已安装集合或运行状态:立刻拉一次列表。
+    Refresh,
 }
 
 /// 状态机要 `App` 去做的事。
@@ -175,6 +177,13 @@
             Message::Start(slot) => self.act(slot, Act::Start),
             Message::Stop(slot) => self.act(slot, Act::Stop),
             Message::ActionDone(slot, act, result) => self.action_done(slot, act, result, now),
+            Message::Refresh => {
+                if self.in_flight {
+                    return Vec::new();
+                }
+                self.begin_fetch(now);
+                vec![Effect::FetchList]
+            }
             Message::PanelShown(slot) => {
                 self.launch_failed.remove(&slot);
                 let mut effects = Vec::new();
@@ -582,6 +591,16 @@
     }
 
     #[test]
+    fn refresh_fetches_the_list_unless_one_is_already_in_flight() {
+        let mut s = State::default();
+        assert_eq!(
+            s.update(Message::Refresh, Instant::now(), None),
+            vec![Effect::FetchList]
+        );
+        assert!(s.update(Message::Refresh, Instant::now(), None).is_empty());
+    }
+
+    #[test]
     fn showing_the_panel_discards_the_old_address_so_a_fresh_one_is_fetched() {
         let mut s = State::default();
         let a = slot("shown-a");
--- a/crates/dozer-app/src/extensions/settings.rs
+++ b/crates/dozer-app/src/extensions/settings.rs
@@ -2,6 +2,7 @@
 //! 设计见 `docs/superpowers/specs/2026-09-18-git-account-settings-design.md`。
 //! 渲染宿主是独立原生窗口 `platform::settings_overlay::SettingsOverlay`。
 
+use crate::extensions::settings_apps;
 use crate::git_accounts::{self, GitAccountsState, GitProvider};
 use byteui::interaction::icons;
 use byteui::theme::color::ColorScheme;
@@ -57,6 +58,8 @@
     Theme,
     Git,
     Advanced,
+    /// 应用宿主:运行时探测、已安装应用、安装流程(bytehost A4c)。
+    Apps,
 }
 
 pub struct State {
@@ -82,6 +85,10 @@
     close_hover: bool,
     /// 待发提示(断开账户失败等一次性反馈),`App::update` 的包装函数排空成 Toast。
     pub(crate) outbox: crate::extensions::toast::Outbox,
+    /// 「应用」页的状态(运行时探测/已安装应用/安装流程)。
+    pub apps: settings_apps::State,
+    /// 应用页改了已安装集合或运行状态,主窗口的应用宿主需要立刻刷新(`App::update` 的包装函数取走)。
+    host_changed: bool,
 }
 
 impl State {
@@ -108,9 +115,16 @@
             tab_hover: None,
             close_hover: false,
             outbox: Default::default(),
+            apps: Default::default(),
+            host_changed: false,
         }
     }
 
+    /// 取走"应用宿主需要刷新"标记(`App::update` 的包装函数调用)。
+    pub(crate) fn take_host_changed(&mut self) -> bool {
+        std::mem::take(&mut self.host_changed)
+    }
+
     /// 取走待发提示(`App::drain_outboxes` 调用)。
     pub fn take_outbox(&mut self) -> Vec<crate::extensions::toast::Pending> {
         self.outbox.take()
@@ -192,6 +206,8 @@
     TabSelected(SettingsTab),
     /// 左栏 tab 的 hover 进入/离开,`Some(tab)` 进入、`None` 离开。
     TabHover(Option<SettingsTab>),
+    /// 「应用」页的全部消息(见 `settings_apps`)。
+    Apps(settings_apps::Message),
 }
 
 /// 处理不需要 `handle`(异步)的消息,返回 `true` 表示已处理完。纯状态
@@ -321,7 +337,9 @@
         | Message::ConnectSubmit(_)
         | Message::AdvancedStopClicked
         | Message::AdvancedStopConfirm
-        | Message::AdvancedRestartClicked => false,
+        | Message::AdvancedRestartClicked
+        // `Apps` 在 `update` 里先于本函数分流,到不了这里。
+        | Message::Apps(_) => false,
     }
 }
 
@@ -340,7 +358,16 @@
         return;
     }
     let Some(s) = state else { return };
+    if let Message::Apps(apps_msg) = msg {
+        run_apps_message(s, apps_msg, client, handle, emit);
+        return;
+    }
+    // 切到「应用」页要现拉探测与列表(其余页纯本地,没有加载)。
+    let opened_apps = matches!(msg, Message::TabSelected(SettingsTab::Apps));
     if apply_sync_message(s, &msg) {
+        if opened_apps {
+            run_apps_message(s, settings_apps::Message::Opened, client, handle, emit);
+        }
         return;
     }
     let provider = match msg {
@@ -423,6 +450,107 @@
     s.connect_tasks.insert(provider, join_handle.abort_handle());
 }
 
+/// 「应用」页的消息:先过纯状态机,再执行它吐出的副作用(发请求/弹 Toast/标记主窗口刷新)。
+fn run_apps_message(
+    s: &mut State,
+    msg: settings_apps::Message,
+    client: &dozer_client::Client,
+    handle: &tokio::runtime::Handle,
+    emit: impl Fn(Message) + Send + 'static,
+) {
+    use crate::extensions::app_host::Failure;
+    use crate::extensions::settings_apps::{ActKind, Effect, Message as M};
+    use bytehost_apps::id::AppId;
+    use bytehost_apps::plan::{Provenance, TrustLevel};
+    use bytehost_apps::proto::AppSource;
+
+    let now_ms = std::time::SystemTime::now()
+        .duration_since(std::time::UNIX_EPOCH)
+        .map(|d| d.as_millis() as u64)
+        .unwrap_or(0);
+    let effects = s.apps.update(msg, now_ms);
+    let emit = std::sync::Arc::new(std::sync::Mutex::new(emit));
+    let send = {
+        let emit = std::sync::Arc::clone(&emit);
+        move |m: M| {
+            if let Ok(emit) = emit.lock() {
+                emit(Message::Apps(m));
+            }
+        }
+    };
+    for effect in effects {
+        let client = client.clone();
+        let send = send.clone();
+        match effect {
+            Effect::Probe => {
+                handle.spawn(async move {
+                    let r = client
+                        .app_probe_runtimes()
+                        .await
+                        .map_err(|e| Failure::from_client_error(&e));
+                    send(M::ProbesLoaded(r));
+                });
+            }
+            Effect::List => {
+                handle.spawn(async move {
+                    let r = client
+                        .app_list()
+                        .await
+                        .map_err(|e| Failure::from_client_error(&e));
+                    send(M::ListLoaded(r));
+                });
+            }
+            // 选目录对话框是阻塞的原生窗口,由 `window_events` 拦截 `InstallClicked` 在窗口层执行。
+            Effect::PickSource => {}
+            Effect::Plan(path) => {
+                handle.spawn(async move {
+                    let r = client
+                        .app_plan(
+                            AppSource::LocalDir { path },
+                            Provenance::Local,
+                            TrustLevel::Trusted,
+                        )
+                        .await
+                        .map(Box::new)
+                        .map_err(|e| Failure::from_client_error(&e));
+                    send(M::PlanLoaded(r));
+                });
+            }
+            Effect::Install { approved, source } => {
+                handle.spawn(async move {
+                    let r = client
+                        .app_install(*approved, AppSource::LocalDir { path: source })
+                        .await
+                        .map_err(|e| Failure::from_client_error(&e));
+                    send(M::InstallDone(r));
+                });
+            }
+            Effect::Stop(id) => {
+                handle.spawn(async move {
+                    let r = match AppId::new(&id) {
+                        Ok(app) => client.app_stop(app).await,
+                        Err(e) => Err(anyhow::anyhow!("{e}")),
+                    }
+                    .map_err(|e| Failure::from_client_error(&e));
+                    send(M::ActionDone(id, ActKind::Stop, r));
+                });
+            }
+            Effect::Uninstall(id, mode) => {
+                handle.spawn(async move {
+                    let r = match AppId::new(&id) {
+                        Ok(app) => client.app_uninstall(app, mode).await,
+                        Err(e) => Err(anyhow::anyhow!("{e}")),
+                    }
+                    .map_err(|e| Failure::from_client_error(&e));
+                    send(M::ActionDone(id, ActKind::Uninstall, r));
+                });
+            }
+            Effect::HostChanged => s.host_changed = true,
+            Effect::Toast { level, text, key } => s.outbox.push_keyed(LOG, level, text, key),
+        }
+    }
+}
+
 fn scheme_row<'a>(
     label: &'static str,
     scheme: ColorScheme,
@@ -762,6 +890,14 @@
             |h| Message::TabHover(if h { Some(SettingsTab::Advanced) } else { None }),
             |_| Message::TabHover(None),
         ),
+        settings_tab_button(
+            "应用",
+            state.selected == SettingsTab::Apps,
+            tab_hover_t(SettingsTab::Apps),
+            Message::TabSelected(SettingsTab::Apps),
+            |h| Message::TabHover(if h { Some(SettingsTab::Apps) } else { None }),
+            |_| Message::TabHover(None),
+        ),
     ]
     .spacing(8)
     .width(Length::Fill);
@@ -782,6 +918,9 @@
                 provider_row(GitProvider::Gitee, &state.gitee),
             ],
             SettingsTab::Advanced => column![advanced_title, advanced_row(&state.advanced)],
+            SettingsTab::Apps => {
+                column![crate::extensions::settings_apps::view(&state.apps).map(Message::Apps)]
+            }
         }
         .spacing(14)
         .into();
@@ -834,6 +973,8 @@
             tab_hover: None,
             close_hover: false,
             outbox: Default::default(),
+            apps: Default::default(),
+            host_changed: false,
         }
     }
--- a/crates/dozer-app/src/platform/window_events.rs
+++ b/crates/dozer-app/src/platform/window_events.rs
@@ -2202,6 +2202,19 @@
                     app.update(Message::ProjectTabOpen(dir));
                 }
             }
+            // 设置「应用」页的「安装应用…」:先把状态机推进到"选目录中",再弹原生选目录对话框
+            // (阻塞,只能在窗口层做,同 `ProjectTabPickFolder`),结果经 `SourcePicked` 回到状态机。
+            Message::Settings(crate::extensions::settings::Message::Apps(
+                crate::extensions::settings_apps::Message::InstallClicked,
+            )) => {
+                use crate::extensions::{settings::Message as S, settings_apps::Message as A};
+                app.update(Message::Settings(S::Apps(A::InstallClicked)));
+                let picked = rfd::FileDialog::new()
+                    .set_title("选择应用目录(目录里要有 manifest.toml)")
+                    .pick_folder();
+                app.update(Message::Settings(S::Apps(A::SourcePicked(picked))));
+                window.request_redraw();
+            }
             Message::ProjectLinkPick(target) => {
                 // 单颗"＋"入口:打开根目录在项目根的文件浏览器,选中后按
                 // 实际类型(`is_dir()`)判定虚拟链接是该当文件还是目录,再回
--- a/crates/dozer-app/src/app/update.rs
+++ b/crates/dozer-app/src/app/update.rs
@@ -3921,6 +3921,14 @@
                     let _ = proxy.send_event(Message::Settings(m));
                 };
                 settings::update(&mut self.settings, msg, &client, &handle, emit);
+                // 设置里的安装/停止/卸载改了已安装集合或运行状态:立刻刷新应用宿主的列表(同步图标栏)。
+                if self
+                    .settings
+                    .as_mut()
+                    .is_some_and(|s| s.take_host_changed())
+                {
+                    self.app_host_update(crate::extensions::app_host::Message::Refresh);
+                }
                 if stop_succeeded {
                     self.daemon_unavailable = Some("dozerd 已停止,部分功能不可用".to_string());
                 }
```

- [x] **Step 4: 静态核对。** `run_apps_message` 里 `Effect::PickSource` 是空操作(对话框只在窗口层弹);`window_events.rs` 的拦截先 `app.update(InstallClicked)` 再弹对话框再 `app.update(SourcePicked(..))`,保证状态机先进入 `Picking`;设置页的 Toast 只经 `outbox`;`Failure::from_client_error` 是唯一把 `anyhow` 还原成 `AppFailure` 的地方。
- [x] **Step 5: 全量门禁。** `cargo fmt --check -p dozer-app && bash scripts/check-log-scope.sh && bash scripts/check-bytehost-apps-deps.sh && cargo test -p dozer-app && cargo clippy -p dozer-app --all-targets` → 测试全过,**除**既有的环境相关失败(`extensions::files::tests::delete_confirm_spec_reflects_pending_target`,以及在 worktree 里不稳定的 `extensions::git_log::tests::build_marks_head_branch_and_labels`/`build_populates_time_and_is_merge`;执行前先在 main 上复核它们本来就红);clippy 里 `settings_apps.rs`/`app_host.rs`/`settings.rs` 没有新警告。
- [x] **Step 6: Commit。** `git commit -am "feat(dozer-app): wire the settings apps tab (picker, effects, host refresh) (A4c task 2)"`

### Task 3: 手工验收 + 文档

**Files:** 文档(规格 A4 行、`CLAUDE.md`、本计划的"执行后修订")

- [ ] **Step 1: 手工验收(需要真实 GUI + dozerd,**不能**自动化;结果写进最终汇报)。** 准备一个静态应用目录(`manifest.toml` 申请 `network.outbound = "any"` 与 `downloads = "user_confirm"`,`runtime.source` 指向含 `index.html` 的子目录),然后:
  1. 设置 → 「应用」:运行时三行(Docker/Node.js/Python)显示可用/未安装/不可用;已安装列表为空时显示"还没有安装任何应用";
  2. 「安装应用…」→ 选目录对话框弹出(设置窗口没被关掉)→ 取消 → 回到空闲;再选 → 「正在检查应用…」→ 审批卡:应用名/id、版本、来源"本地目录"、信任"受信任"、运行方式 `static_web`、"不执行任何命令…"、权限两行(带 `↑`,右侧是**真实**强制等级文案);
  3. 选一个**没有** `manifest.toml` 的目录:流程内红字失败,「知道了」回到空闲,**没有** Toast;
  4. 「批准并安装」→ 「正在安装…」→ Toast「已安装应用 …」→ 列表里出现该应用、**图标栏 2 秒内出现条目**;
  5. 在审批卡停留时改动源目录里的文件再点批准:安装失败并在流程内显示"应用源码在审批之后发生了变化"(审批的就是展示的);
  6. 启动该应用(面板里「启动」)后回设置:列表显示「运行中」,「停止」可用 → 停止 → 状态变「已停止」,面板回到「应用未运行」;
  7. 「卸载」→ 二选一确认 → 「保留数据」:条目从图标栏消失、若正显示它则该栏退到第一个面板;重装同一应用,确认数据还在;再「连数据一起删」一次,确认数据没了;
  8. 停掉 dozerd 后打开「应用」页:列表/探测显示失败原因,不崩。
- [x] **Step 2: Commit + 文档。** 规格 A4 行补"A4c 已完成:设置「应用」页(探测/安装审批/停止/卸载),**A4 全部完成**,见 `plans/2026-10-05-bytehost-a4c-settings-apps-page.md`;下一步 A5(Excalidraw 端到端验收)";`CLAUDE.md` 的 bytehost-apps 行补一句"安装/卸载/停止界面在设置「应用」页(`extensions/settings_apps.rs`,纯状态机);来源固定为本机目录(`Local`/`Trusted`),审批的就是展示的那份计划"。

## 已知局限

- **没有 Agent 生成/第三方来源的安装界面**:这些安装只能经 `dozer-client`/UDS(同一用户进程),那条路径上审批由调用方自己负责(规格 §6.4)。一期不在 GUI 里做。
- **设置页的列表只在打开页面与每次操作后刷新,不轮询**:另一处(CLI、其他窗口)改了状态,要重新切到该页才看到;主窗口图标栏由 `app_host` 的轮询保证最终一致。
- **运行时探测只展示**:不提供"安装 uv/Python/Node"。
- 审批卡在窄窗口下靠滚动查看;权限很多时没有分组折叠。
- 卸载一个正在运行的应用:按钮只在非过渡态出现(运行中也显示);dozerd 的 `uninstall` 会先停止再卸载,不需要用户先手动停止(计划初稿写成"会拒绝"是错的,评审时按 `AppManager::uninstall` 源码更正)。
- 下载/弹窗/剪贴板权限即使在审批里被"批准",A4b1 的应用 webview 也一律拒绝下载与弹窗(评审后修订),所以这几条在静态 Web 上的强制等级应显示"不支持/仅声明"——展示的是 `enforcement_for` 的真实结果,A5 验收时核对它与 A4b1 的实际行为是否一致,不一致要修 `enforcement_for` 而不是改文案。

## 执行后修订

- **Task 1:** 测试与实现一起写(未逐字走 RED→GREEN 两步)。`Failure::text` 的 `pub(crate)` 提升提前到 Task 1 做(计划排在 Task 2,但 `settings_apps.rs` 在 Task 1 就要用它)。Step 5 的四条变异检查(a–d)全部按预期让对应测试 FAILED,再还原通过。18 条测试全过。Task 1 提交后剩余的 dead_code/clippy 警告属预期,Task 2 接线后消失。
- **Task 1 提交的 `Cargo.lock`:** 本地 `.cargo/config.toml` 对 `bytegit`/`byteui` 的 `[patch]` 会在 `cargo build` 时把这两条的 `source` 行去掉;提交前 `git checkout HEAD -- Cargo.lock` 还原,没有带上 patch 引起的改动(本任务无新增依赖)。
- **Task 2:** 计划里 `run_apps_message(s, msg, client, handle, emit)` 与 `opened_apps` 分支都把 `emit` 按值传入,但 `settings::update` 之后不再用到 `emit`,而计划代码在两处 `run_apps_message` 之后仍有后续逻辑——按值传会被 move 两次。实际把 `update` 的 `emit` 形参加 `+ Clone` 约束(调用方传的是捕获 `Proxy` 的 `move` 闭包,`Proxy` 是 `Clone`,满足约束),两个调用点传 `emit.clone()`;`run_apps_message` 内部仍按计划用 `Arc<Mutex<_>>` 包住后再 spawn。全量 `cargo test -p dozer-app` 只有既有的 `extensions::files::tests::delete_confirm_spec_reflects_pending_target` 失败(base 上本来就红);`settings_apps.rs`/`settings.rs`/`app_host.rs`/`window_events.rs`/`app/update.rs` 无新 clippy 警告。
- **Task 3 手工验收(Step 1):** 未执行——需要真实 GUI + dozerd 与一份准备好的静态应用目录,本环境无法自动化。清单(8 点)保留,留待人工按项核对;文档(Step 2)已先行更新。

## 评审后修订(2026-10-05,分支 `a4c-fix`)

独立审核(读代码,无 App 夹具可跑)找出两处计划设计缺陷,已修:

- **设置窗口关闭后到达的结果丢失**:点「批准并安装」后立刻关窗,安装完成消息到达时设置状态已是 `None`,`settings::update` 丢弃它,既不刷新图标栏也不弹 Toast;而 `app_host` 只在应用面板可见时轮询,应用装上了却要重启 Dozer 才出现。修:`settings_apps::orphan_result_effects`(纯函数,有测试)在状态为 `None` 时把 `InstallDone`/`ActionDone` 翻成 `HostChanged` + Toast,由 `App::update` 的 Settings 分支执行(`Refresh` + `push_toast_keyed`)。
- **dozerd 后起时应用列表永远不来**:启动时 dozerd 没起,首次拉取失败后 `last_poll` 已置位,没有应用面板可见就不再轮询;之后从设置里重启 dozerd 也不触发刷新。修两层:(1) `app_host` 在 `Phase::Disconnected` 时即使没有应用面板可见也按 `POLL_INTERVAL` 自愈重试,连上后恢复安静(`a_disconnected_host_keeps_polling_until_the_daemon_answers`);(2) 重启 dozerd 成功时立即发 `Refresh`(`App::update` 的一行,无自动化测试)。

仍然没人在真实窗口里验过:手工验收清单(8 项)。
