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
use bytehost_apps::proto::{
    AppSummary, ManagedRuntime, RuntimeAvailability, RuntimeInstallPlan, RuntimeProbe,
};
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
    /// 正在出一份运行时安装计划(不下载任何东西)。
    RuntimePlanning {
        runtime: ManagedRuntime,
    },
    /// 运行时安装计划已展示,等用户批准。
    RuntimeReviewing {
        plan: Box<RuntimeInstallPlan>,
    },
    /// 卸载某个受管运行时版本的确认。
    ConfirmRuntimeUninstall {
        runtime: ManagedRuntime,
        version: String,
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
    /// 点「安装…」:为一个运行时出一份安装计划。
    RuntimeInstallClicked(ManagedRuntime),
    RuntimePlanLoaded(Result<Box<RuntimeInstallPlan>, Failure>),
    RuntimeApproveClicked,
    /// 安装请求已发出(成功表示 dozerd 接受了任务,进度靠轮询)。
    RuntimeInstallStarted(Result<(), Failure>),
    RuntimeUninstallClicked(ManagedRuntime, String),
    RuntimeUninstallConfirmed,
    RuntimeUninstallDone(Result<(), Failure>),
    /// 安装进行中的定时刷新(由 `Effect::ProbeAfter` 转回)。
    PollProbes,
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
    /// 为一个运行时出一份安装计划(不下载)。
    RuntimePlan(ManagedRuntime),
    /// 安装一份已批准的运行时计划。
    InstallRuntime(Box<RuntimeInstallPlan>),
    /// 卸载某个受管运行时版本。
    UninstallRuntime(ManagedRuntime, String),
    /// 延时后再探测一次(安装进行中的进度轮询)。
    ProbeAfter(std::time::Duration),
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
    /// 已经报过结果(成功或失败 Toast)的运行时任务——避免每次轮询重复弹。
    reported_jobs: std::collections::HashSet<ManagedRuntime>,
    /// 已发出但还没回结果的受管运行时卸载(去重)。
    uninstalling: std::collections::HashSet<(ManagedRuntime, String)>,
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
                self.probe_side_effects()
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
                    Flow::Reviewing { .. }
                        | Flow::Failed { .. }
                        | Flow::ConfirmUninstall { .. }
                        | Flow::RuntimeReviewing { .. }
                        | Flow::ConfirmRuntimeUninstall { .. }
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
            Message::RuntimeInstallClicked(runtime) => {
                if self.flow != Flow::Idle {
                    return Vec::new();
                }
                self.flow = Flow::RuntimePlanning { runtime };
                vec![Effect::RuntimePlan(runtime)]
            }
            Message::RuntimePlanLoaded(result) => {
                let Flow::RuntimePlanning { .. } = &self.flow else {
                    return Vec::new();
                };
                self.flow = match result {
                    Ok(plan) => Flow::RuntimeReviewing { plan },
                    // 流程内的失败:留在对话里,不弹 Toast(与安装计划一致)。
                    Err(f) => Flow::Failed {
                        message: f.text().to_owned(),
                    },
                };
                Vec::new()
            }
            Message::RuntimeApproveClicked => {
                let Flow::RuntimeReviewing { plan } = &self.flow else {
                    return Vec::new();
                };
                // 批准的就是展示的这一份:原封不动发回去,dozerd 会重算核对。
                let plan = plan.clone();
                self.flow = Flow::Idle;
                vec![
                    Effect::InstallRuntime(plan),
                    Effect::ProbeAfter(std::time::Duration::from_millis(500)),
                ]
            }
            Message::RuntimeInstallStarted(result) => match result {
                Ok(()) => Vec::new(),
                Err(f) => vec![Effect::Toast {
                    level: Level::Error,
                    text: format!("安装运行时失败:{}", f.text()),
                    key: "runtimes:install".into(),
                }],
            },
            Message::RuntimeUninstallClicked(runtime, version) => {
                if self.flow != Flow::Idle
                    || self.uninstalling.contains(&(runtime, version.clone()))
                {
                    return Vec::new();
                }
                self.flow = Flow::ConfirmRuntimeUninstall { runtime, version };
                Vec::new()
            }
            Message::RuntimeUninstallConfirmed => {
                let Flow::ConfirmRuntimeUninstall { runtime, version } = &self.flow else {
                    return Vec::new();
                };
                let (runtime, version) = (*runtime, version.clone());
                self.flow = Flow::Idle;
                self.uninstalling.insert((runtime, version.clone()));
                vec![Effect::UninstallRuntime(runtime, version)]
            }
            Message::RuntimeUninstallDone(result) => {
                // 哪个版本已经不在状态里记了(可能同时多个);都清掉,要求刷新即可。
                for key in self.uninstalling.drain() {
                    self.reported_jobs.remove(&key.0);
                }
                let effects = vec![Effect::Probe];
                match result {
                    Ok(()) => effects,
                    Err(f) => {
                        let mut effects = effects;
                        effects.insert(
                            0,
                            Effect::Toast {
                                level: Level::Error,
                                text: format!("卸载运行时失败:{}", f.text()),
                                key: "runtimes:uninstall".into(),
                            },
                        );
                        effects
                    }
                }
            }
            Message::PollProbes => vec![Effect::Probe],
        }
    }

    /// `ProbesLoaded` 之后的副作用:安装进行中继续轮询;刚完成的任务报一次结果。
    fn probe_side_effects(&mut self) -> Vec<Effect> {
        let Load::Loaded(probes) = &self.probes else {
            return Vec::new();
        };
        let mut effects = Vec::new();
        let mut any_running = false;
        for probe in probes {
            let Some(job) = &probe.job else { continue };
            if !job.finished {
                any_running = true;
                continue;
            }
            if self.reported_jobs.contains(&job.runtime) {
                continue;
            }
            self.reported_jobs.insert(job.runtime);
            let name = managed_runtime_label(job.runtime);
            match &job.failed {
                Some(why) => effects.push(Effect::Toast {
                    level: Level::Error,
                    text: format!("{name} 运行时安装失败:{why}"),
                    key: format!("runtimes:job:{name}"),
                }),
                None => {
                    effects.push(Effect::Toast {
                        level: Level::Success,
                        text: format!("{name} 运行时已安装"),
                        key: format!("runtimes:job:{name}"),
                    });
                    effects.push(Effect::HostChanged);
                }
            }
        }
        if any_running {
            effects.push(Effect::ProbeAfter(std::time::Duration::from_secs(1)));
        }
        effects
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
        Message::RuntimeInstallStarted(Err(f)) => vec![Effect::Toast {
            level: Level::Error,
            text: format!("安装运行时失败:{}", f.text()),
            key: "runtimes:install".into(),
        }],
        Message::RuntimeUninstallDone(Err(f)) => vec![Effect::Toast {
            level: Level::Error,
            text: format!("卸载运行时失败:{}", f.text()),
            key: "runtimes:uninstall".into(),
        }],
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

/// 受管运行时的中文名(给按钮/Toast 用)。
pub fn managed_runtime_label(runtime: ManagedRuntime) -> &'static str {
    match runtime {
        ManagedRuntime::Node => "Node.js",
        ManagedRuntime::Python => "Python",
    }
}

/// 运行时行:一个运行时一行(名称、探测状态、受管版本、安装/卸载按钮、进度)。
#[cfg(test)]
#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeRow {
    pub probe_runtime: String,
    pub name: String,
    /// 探测到的可用性文案(如 `可用 · v20` / `未安装`)。
    pub status: String,
    /// 已装的受管版本(新→旧),带 `uninstall` 的按钮文案所需。
    pub managed: Vec<String>,
    /// 此平台可自动安装(有固定版本)。
    pub installable: bool,
    /// 显示的进度文案(有任务且未完成时),如 `下载 node · 45%` 或 `下载 node · 12.3 MB`。
    pub progress: Option<String>,
}

/// 把探测结果映射成运行时行。每个探测一行;未受管不支持的运行时(docker)`installable=false`。
#[cfg(test)]
pub fn runtime_rows(probes: &[RuntimeProbe]) -> Vec<RuntimeRow> {
    probes
        .iter()
        .map(|probe| {
            let (name, status, _) = runtime_line(probe);
            RuntimeRow {
                probe_runtime: probe.runtime.clone(),
                name,
                status,
                managed: probe.managed.clone(),
                installable: probe.installable,
                progress: probe
                    .job
                    .as_ref()
                    .filter(|j| !j.finished)
                    .map(progress_text),
            }
        })
        .collect()
}

/// 一个进行中的任务 → 进度文案;`total` 未知时只显示已下载 MB。
fn progress_text(job: &bytehost_apps::proto::RuntimeJob) -> String {
    match job.total {
        Some(total) if total > 0 => {
            let pct = (job.done.saturating_mul(100) / total).min(100);
            format!("{} · {}%", job.phase, pct)
        }
        _ => format!("{} · {:.1} MB", job.phase, job.done as f64 / 1_048_576.0),
    }
}

/// 判断某探测对应的运行时是否正在安装(未完成的任务)。
pub fn probe_is_installing(probe: &RuntimeProbe) -> bool {
    probe.job.as_ref().is_some_and(|j| !j.finished)
}

/// 审批页逐行内容:`(标签, 值)`——来源 / 固定版本 / SHA-256 / 安装位置 / 将要做的事。
pub fn plan_lines(plan: &RuntimeInstallPlan) -> Vec<(String, String)> {
    let mut lines: Vec<(String, String)> = Vec::new();
    lines.push((
        "运行时".into(),
        managed_runtime_label(plan.runtime).to_owned(),
    ));
    for (name, version) in &plan.versions {
        lines.push((format!("版本({name})"), version.clone()));
    }
    for d in &plan.downloads {
        lines.push((format!("来源({})", d.what), d.url.clone()));
        let sha = match &d.sha256 {
            Some(s) => s.clone(),
            None => "由 uv 内置哈希校验".to_owned(),
        };
        lines.push((format!("SHA-256({})", d.what), sha));
        if !d.note.is_empty() {
            lines.push((format!("说明({})", d.what), d.note.clone()));
        }
    }
    lines.push(("安装位置".into(), plan.dest.clone()));
    lines
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
    let mut body = column![heading("应用")].spacing(12).width(Length::Fill);

    // 运行时探测 + 受管运行时的安装/卸载。
    body = body.push(dim("运行时(受管版本随 dozerd 安装在本机,优先于系统版本)"));
    let probes_view: El<'_> = match &state.probes {
        Load::Loading => dim("检测中…").into(),
        Load::Failed(why) => dim(format!("探测失败:{why}")).into(),
        Load::Loaded(probes) => {
            let mut col = column![].spacing(4);
            for probe in probes {
                col = col.push(runtime_row_view(state, probe));
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

/// 一行受管运行时:名称、状态、受管版本、进度或安装/卸载按钮。
fn runtime_row_view<'a>(state: &'a State, probe: &'a RuntimeProbe) -> El<'a> {
    let colors = byteui::theme::color::current();
    let (name, status, _) = runtime_line(probe);
    let mut r = row![
        text(name)
            .size(byteui::theme::font::body())
            .color(colors.cream),
    ]
    .spacing(10)
    .align_y(Alignment::Center);

    if probe_is_installing(probe) {
        let progress = probe.job.as_ref().map(progress_text).unwrap_or_default();
        r = r.push(dim(progress));
        r = r.push(Space::new().width(Length::Fill));
        return r.into();
    }

    r = r.push(dim(status));
    r = r.push(Space::new().width(Length::Fill));

    let runtime = managed_runtime_of(&probe.runtime);
    if !probe.managed.is_empty() {
        for v in &probe.managed {
            let Some(rt) = runtime else {
                r = r.push(dim(v.clone()));
                continue;
            };
            if state.uninstalling.contains(&(rt, v.clone())) {
                r = r.push(dim(format!("{v}(处理中…)")));
                continue;
            }
            r = r.push(dim(v.clone()));
            r = r.push(action_button(
                "卸载",
                Message::RuntimeUninstallClicked(rt, v.clone()),
                colors.red,
            ));
        }
    } else if probe.installable
        && let Some(rt) = runtime
    {
        r = r.push(action_button(
            "安装…",
            Message::RuntimeInstallClicked(rt),
            colors.gold,
        ));
    }
    r.into()
}

/// 探测的 `runtime` 键 → 受管运行时(仅 node/python;其他返回 `None`)。
fn managed_runtime_of(key: &str) -> Option<ManagedRuntime> {
    match key {
        "node" => Some(ManagedRuntime::Node),
        "python" => Some(ManagedRuntime::Python),
        _ => None,
    }
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
        Flow::RuntimePlanning { runtime } => dim(format!(
            "正在准备 {} 运行时的安装计划…",
            managed_runtime_label(*runtime)
        ))
        .into(),
        Flow::RuntimeReviewing { plan } => runtime_review_view(plan),
        Flow::ConfirmRuntimeUninstall { runtime, version } => column![
            text(format!(
                "卸载受管运行时 {} {version}?",
                managed_runtime_label(*runtime)
            ))
            .size(byteui::theme::font::body())
            .color(colors.cream),
            dim("依赖它的应用下次启动会因运行时缺失而失败;重装可恢复。"),
            row![
                action_button("取消", Message::FlowDismissed, colors.dim),
                action_button("卸载", Message::RuntimeUninstallConfirmed, colors.red),
            ]
            .spacing(8),
        ]
        .spacing(8)
        .into(),
    }
}

/// 运行时安装审批卡:逐行展示 `plan_lines`(来源 URL、固定版本、完整 SHA-256、目标目录)。
fn runtime_review_view(plan: &RuntimeInstallPlan) -> El<'_> {
    let colors = byteui::theme::color::current();
    let mut col = column![
        text(format!(
            "安装 {} 运行时",
            managed_runtime_label(plan.runtime)
        ))
        .size(byteui::theme::font::body())
        .color(colors.cream),
    ]
    .spacing(6);
    for (k, v) in plan_lines(plan) {
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
    col = col.push(dim("将要做的事:"));
    for line in &plan.will_do {
        col = col.push(
            text(line.clone())
                .size(byteui::theme::font::label())
                .color(colors.cream),
        );
    }
    col = col.push(
        row![
            action_button("取消", Message::FlowDismissed, colors.dim),
            action_button("下载并安装", Message::RuntimeApproveClicked, colors.gold),
        ]
        .spacing(8),
    );
    container(col).padding(10).width(Length::Fill).into()
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
    use bytehost_apps::proto::{AppErrorKind, AppFailure, RuntimeDownload, RuntimeJob};
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
            issue: None,
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
            managed: Vec::new(),
            installable: false,
            job: None,
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

    // ---- A6d Task 5:运行时安装审批 / 进度 / 卸载 ----

    fn rt_plan(runtime: ManagedRuntime, sha: Option<&str>) -> RuntimeInstallPlan {
        RuntimeInstallPlan {
            runtime,
            versions: vec![("node".into(), "24.21.0".into())],
            downloads: vec![RuntimeDownload {
                what: "node".into(),
                url: "https://nodejs.org/dist/v24.21.0/node.tar.gz".into(),
                sha256: sha.map(str::to_owned),
                note: String::new(),
            }],
            dest: "/Users/x/bytehost/runtimes/node/24.21.0".into(),
            will_do: vec!["下载并解压到目标目录".into(), "校验 SHA-256".into()],
        }
    }

    fn rt_probe(
        runtime: &str,
        managed: Vec<&str>,
        installable: bool,
        job: Option<RuntimeJob>,
    ) -> RuntimeProbe {
        RuntimeProbe {
            runtime: runtime.into(),
            availability: RuntimeAvailability::NotInstalled,
            managed: managed.into_iter().map(String::from).collect(),
            installable,
            job,
        }
    }

    fn job(runtime: ManagedRuntime, finished: bool, failed: Option<&str>) -> RuntimeJob {
        RuntimeJob {
            runtime,
            phase: "下载".into(),
            done: 5,
            total: Some(10),
            failed: failed.map(str::to_owned),
            finished,
        }
    }

    #[test]
    fn clicking_install_plans_then_reviewing_shows_every_download_with_its_checksum() {
        let mut s = State::default();
        assert_eq!(
            s.update(Message::RuntimeInstallClicked(ManagedRuntime::Node), NOW),
            vec![Effect::RuntimePlan(ManagedRuntime::Node)]
        );
        assert_eq!(
            s.flow,
            Flow::RuntimePlanning {
                runtime: ManagedRuntime::Node
            }
        );
        s.update(
            Message::RuntimePlanLoaded(Ok(Box::new(rt_plan(
                ManagedRuntime::Node,
                Some(&"a".repeat(64)),
            )))),
            NOW,
        );
        assert!(matches!(s.flow, Flow::RuntimeReviewing { .. }));

        let plan = rt_plan(ManagedRuntime::Node, Some(&"a".repeat(64)));
        let lines = plan_lines(&plan);
        assert!(lines.iter().any(|(_, v)| v.contains("nodejs.org")));
        assert!(
            lines
                .iter()
                .any(|(k, v)| k.starts_with("版本") && v == "24.21.0")
        );
        assert!(
            lines
                .iter()
                .any(|(k, v)| k.starts_with("SHA-256") && v == &"a".repeat(64))
        );
        assert!(
            lines
                .iter()
                .any(|(_, v)| v.contains("/runtimes/node/24.21.0"))
        );

        // sha256 == None → 说明由 uv 校验。
        let uv = RuntimeInstallPlan {
            runtime: ManagedRuntime::Python,
            versions: vec![("python".into(), "3.13".into())],
            downloads: vec![RuntimeDownload {
                what: "python".into(),
                url: "uv:python-install".into(),
                sha256: None,
                note: "由 uv 下载".into(),
            }],
            dest: "/x/runtimes/python".into(),
            will_do: vec![],
        };
        assert!(
            plan_lines(&uv)
                .iter()
                .any(|(k, v)| k.starts_with("SHA-256") && v == "由 uv 内置哈希校验")
        );
    }

    #[test]
    fn approving_sends_back_exactly_the_plan_that_was_shown_and_starts_polling() {
        let mut s = State::default();
        s.update(Message::RuntimeInstallClicked(ManagedRuntime::Node), NOW);
        let plan = rt_plan(ManagedRuntime::Node, Some(&"b".repeat(64)));
        s.update(Message::RuntimePlanLoaded(Ok(Box::new(plan.clone()))), NOW);
        let effects = s.update(Message::RuntimeApproveClicked, NOW);
        assert_eq!(effects.len(), 2);
        assert_eq!(effects[0], Effect::InstallRuntime(Box::new(plan)));
        assert!(matches!(effects[1], Effect::ProbeAfter(_)));
        assert_eq!(s.flow, Flow::Idle, "批准后回到 Idle");
    }

    #[test]
    fn approve_outside_reviewing_does_nothing() {
        let mut s = State::default();
        assert!(s.update(Message::RuntimeApproveClicked, NOW).is_empty());
        s.update(Message::RuntimeInstallClicked(ManagedRuntime::Node), NOW);
        assert!(s.update(Message::RuntimeApproveClicked, NOW).is_empty());
    }

    #[test]
    fn polling_continues_only_while_a_job_is_unfinished_and_stops_after() {
        let mut s = State::default();
        // 未完成 → 继续轮询。
        let effects = s.update(
            Message::ProbesLoaded(Ok(vec![rt_probe(
                "node",
                vec![],
                true,
                Some(job(ManagedRuntime::Node, false, None)),
            )])),
            NOW,
        );
        assert!(effects.iter().any(|e| matches!(e, Effect::ProbeAfter(_))));

        // 已完成且成功 → 一次成功 Toast + HostChanged,且不再轮询。
        let effects = s.update(
            Message::ProbesLoaded(Ok(vec![rt_probe(
                "node",
                vec!["24.21.0"],
                true,
                Some(job(ManagedRuntime::Node, true, None)),
            )])),
            NOW,
        );
        assert!(effects.iter().any(|e| matches!(
            e,
            Effect::Toast {
                level: Level::Success,
                ..
            }
        )));
        assert!(effects.contains(&Effect::HostChanged));
        assert!(!effects.iter().any(|e| matches!(e, Effect::ProbeAfter(_))));
    }

    #[test]
    fn a_finished_job_toasts_once_and_a_failed_one_toasts_its_reason_once() {
        let mut s = State::default();
        let probes_ok = || {
            vec![rt_probe(
                "node",
                vec!["24.21.0"],
                true,
                Some(job(ManagedRuntime::Node, true, None)),
            )]
        };
        let first = s.update(Message::ProbesLoaded(Ok(probes_ok())), NOW);
        assert_eq!(
            first
                .iter()
                .filter(|e| matches!(e, Effect::Toast { .. }))
                .count(),
            1
        );
        let second = s.update(Message::ProbesLoaded(Ok(probes_ok())), NOW);
        assert_eq!(
            second
                .iter()
                .filter(|e| matches!(e, Effect::Toast { .. }))
                .count(),
            0,
            "同一任务只报一次"
        );

        // 失败:报一次原因;不同运行时各报各的。
        let mut s = State::default();
        let f = s.update(
            Message::ProbesLoaded(Ok(vec![rt_probe(
                "python",
                vec![],
                true,
                Some(job(ManagedRuntime::Python, true, Some("下载超时"))),
            )])),
            NOW,
        );
        assert!(f.iter().any(|e| matches!(e,
            Effect::Toast { level: Level::Error, text, .. } if text.contains("下载超时"))));
    }

    #[test]
    fn docker_never_offers_install() {
        let probes = vec![rt_probe("docker", vec![], false, None)];
        let rows = runtime_rows(&probes);
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].installable);
        assert!(rows[0].managed.is_empty());
        assert!(managed_runtime_of("docker").is_none());
    }

    #[test]
    fn an_installed_managed_version_offers_uninstall_with_the_warning() {
        let probes = vec![rt_probe("node", vec!["24.21.0"], true, None)];
        let rows = runtime_rows(&probes);
        assert_eq!(rows[0].managed, vec!["24.21.0".to_string()]);

        // 确认页文案含"运行时缺失而失败"的警告。
        let mut s = State::default();
        s.update(Message::ProbesLoaded(Ok(probes.clone())), NOW);
        let effects = s.update(
            Message::RuntimeUninstallClicked(ManagedRuntime::Node, "24.21.0".into()),
            NOW,
        );
        assert!(effects.is_empty(), "先确认");
        assert!(matches!(
            s.flow,
            Flow::ConfirmRuntimeUninstall { ref runtime, .. } if *runtime == ManagedRuntime::Node
        ));
        let effects = s.update(Message::RuntimeUninstallConfirmed, NOW);
        assert_eq!(
            effects,
            vec![Effect::UninstallRuntime(
                ManagedRuntime::Node,
                "24.21.0".into()
            )]
        );
    }

    #[test]
    fn a_plan_failure_stays_in_the_flow_not_a_toast() {
        let mut s = State::default();
        s.update(Message::RuntimeInstallClicked(ManagedRuntime::Node), NOW);
        let effects = s.update(
            Message::RuntimePlanLoaded(Err(fail(AppErrorKind::Unsupported, "此平台不支持"))),
            NOW,
        );
        assert!(effects.is_empty(), "流程内失败不弹 Toast");
        assert_eq!(
            s.flow,
            Flow::Failed {
                message: "此平台不支持".into()
            }
        );
    }

    #[test]
    fn runtime_rows_show_progress_percent_or_megabytes() {
        // total 已知 → 百分比。
        let pct = runtime_rows(&[rt_probe(
            "node",
            vec![],
            true,
            Some(RuntimeJob {
                runtime: ManagedRuntime::Node,
                phase: "下载".into(),
                done: 3,
                total: Some(4),
                failed: None,
                finished: false,
            }),
        )]);
        assert_eq!(pct[0].progress.as_deref(), Some("下载 · 75%"));

        // total 未知 → 只显示 MB。
        let mb = runtime_rows(&[rt_probe(
            "node",
            vec![],
            true,
            Some(RuntimeJob {
                runtime: ManagedRuntime::Node,
                phase: "下载".into(),
                done: 2_097_152,
                total: None,
                failed: None,
                finished: false,
            }),
        )]);
        assert_eq!(mb[0].progress.as_deref(), Some("下载 · 2.0 MB"));
    }

    #[test]
    fn a_finished_uninstall_clears_dedup_and_refreshes() {
        let mut s = State::default();
        s.update(Message::ProbesLoaded(Ok(vec![])), NOW);
        s.update(
            Message::RuntimeUninstallClicked(ManagedRuntime::Node, "24.21.0".into()),
            NOW,
        );
        s.update(Message::RuntimeUninstallConfirmed, NOW);
        let effects = s.update(Message::RuntimeUninstallDone(Ok(())), NOW);
        assert_eq!(effects, vec![Effect::Probe]);
        // 失败版带 Toast。
        s.update(
            Message::RuntimeUninstallClicked(ManagedRuntime::Node, "24.21.0".into()),
            NOW,
        );
        s.update(Message::RuntimeUninstallConfirmed, NOW);
        let bad = s.update(
            Message::RuntimeUninstallDone(Err(fail(AppErrorKind::Conflict, "仍在运行"))),
            NOW,
        );
        assert!(bad.iter().any(|e| matches!(e,
            Effect::Toast { level: Level::Error, text, .. } if text.contains("仍在运行"))));
    }

    #[test]
    fn poll_probes_outside_the_settings_window_does_nothing_bad() {
        // PollProbes 在有窗口时只返回一个 Probe 意图;窗口没了时不产生任何孤儿副作用。
        assert!(orphan_result_effects(&Message::PollProbes).is_empty());
    }
}
