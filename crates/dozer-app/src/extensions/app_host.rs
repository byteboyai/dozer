//! 应用面板的宿主逻辑(bytehost A4b2):dozerd 里已安装应用的列表轮询、每个应用面板的状态机、启动/停止、
//! "该给 webview 的地址"。**纯状态机**:`update`/`poll_if_due` 只返回 [`Effect`],由 `App` 去执行
//! (发请求、同步 rail、写 `AppViews`、弹 Toast)——这样整套逻辑可以不开窗口、不连 dozerd 地测。
//!
//! 约定(规格 A4 与 A4b 评审留下的):
//! - **轮询,不做推送**:应用面板可见时每 [`POLL_INTERVAL`] 拉一次 `List`;没有应用面板可见时只在启动后拉一次,
//!   以及面板切入、启停操作完成后各拉一次;
//! - **启动地址含令牌(秘密)**:每次面板切入都清掉旧地址、重新取(webview 因切面板/切项目被销毁重建时不会拿到
//!   过期地址);地址只经 `Effect::SetUrl` 进内存里的 `AppViews`,不进日志、不落盘;
//! - **瞬时失败走 Toast,持久状态留在面板里**:启停/取地址失败是"刚发生的一件事"→ Toast;宿主不可用
//!   (`AppErrorKind::Unavailable`)是"当前处于某状态"→ 面板里显示原因,不弹 Toast;
//! - 列表请求的传输层失败(连不上 dozerd)不清空已知的应用、不弹 Toast——顶栏的 dozerd 徽标负责告知。

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use bytehost_apps::proto::{AppErrorKind, AppFailure, AppIssue, AppSummary};
use bytehost_apps::state::{DesiredState, ObservedState};

use crate::app::AppSlot;
use crate::extensions::toast::Level;

/// 应用面板可见时拉 `List` 的间隔。
pub const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// 一次请求的失败:宿主带类别的失败,或者传输/协议层的(连不上 dozerd、协议版本不符)。
#[derive(Debug, Clone, PartialEq)]
pub enum Failure {
    Host(AppFailure),
    Transport(String),
}

impl Failure {
    /// 从 `dozer-client` 的错误还原:能 `downcast` 成 `AppFailure` 的是宿主失败,其余是传输层。
    pub fn from_client_error(e: &anyhow::Error) -> Self {
        match e.downcast_ref::<AppFailure>() {
            Some(f) => Self::Host(f.clone()),
            None => Self::Transport(format!("{e:#}")),
        }
    }

    pub(crate) fn text(&self) -> &str {
        match self {
            Self::Host(f) => &f.message,
            Self::Transport(t) => t,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    Start,
    Stop,
}

#[derive(Debug, Clone)]
pub enum Message {
    ListLoaded(Result<Vec<AppSummary>, Failure>),
    LaunchUrlLoaded(AppSlot, Result<String, Failure>),
    ActionDone(AppSlot, Act, Result<(), Failure>),
    /// 用户点了面板里的「启动」/「停止」。
    Start(AppSlot),
    Stop(AppSlot),
    /// 该应用面板被切到前台(图标栏点击或程序化显示)。
    PanelShown(AppSlot),
    /// 别处(设置里的安装/停止/卸载)改了已安装集合或运行状态:立刻拉一次列表。
    Refresh,
    /// 用户点了崩溃页的「查看日志」。
    ShowLogs(AppSlot),
    /// 用户点了「收起日志」。
    HideLogs(AppSlot),
    /// 日志读取的结果。
    LogsLoaded(AppSlot, Result<(String, bool), Failure>),
    /// 用户点了运行时问题页的「去设置安装」。
    OpenRuntimeSettings,
}

/// 状态机要 `App` 去做的事。
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    FetchList,
    FetchLaunchUrl(AppSlot),
    StartApp(AppSlot),
    StopApp(AppSlot),
    /// 已安装集合变了(或第一次拿到):`App::sync_installed_apps`。
    SyncRail(Vec<AppSlot>),
    SetUrl(AppSlot, String),
    ClearUrl(AppSlot),
    /// 读某应用日志末尾(`max_lines` 由状态机定,服务端还会再夹一次)。
    FetchLogs(AppSlot, u32),
    /// 打开 设置 → 应用 页(运行时问题页的「去设置安装」)。
    OpenSettingsApps,
    Toast {
        level: Level,
        text: String,
        key: String,
    },
}

/// 崩溃页下方日志查看器的状态。
#[derive(Debug, Clone, PartialEq)]
pub enum LogsView {
    /// 未展开。
    Hidden,
    /// 正在读。
    Loading,
    /// 已读到:`truncated` 表示只显示了末尾若干行。
    Loaded { text: String, truncated: bool },
    /// 读取失败(原因留在页面里,不弹 Toast)。
    Failed(String),
}

/// 一个应用在 dozerd 里的当前状况(只留面板要用的)。
#[derive(Debug, Clone, PartialEq)]
struct Row {
    id: String,
    name: String,
    #[allow(dead_code)] // 目前面板只看观察态;期望态留给 A4c 的"开机自启"之类展示
    desired: DesiredState,
    observed: ObservedState,
    /// 崩溃/起不来时的"看得懂的问题"(运行时缺失/版本不符);仅 `Failed` 时带出。
    issue: Option<AppIssue>,
}

/// 整个列表的状况(`State::phase` 为 `None` = 还没拿到过列表)。
#[derive(Debug, Clone, PartialEq)]
enum Phase {
    Loaded,
    /// 应用宿主本身不可用(原因给人看)。
    Unavailable(String),
    /// 最近一次请求在传输层失败;已知的应用保留。
    Disconnected,
}

/// 面板该画什么(`State::view_model` 的结果)。
#[derive(Debug, Clone, PartialEq)]
pub enum PanelView {
    /// 应用宿主不可用,带原因。
    HostUnavailable(String),
    /// 还没拿到列表。
    Loading,
    /// 连不上 dozerd,又没有任何已知的应用。
    Disconnected,
    /// 列表里已经没有这个应用(刚被卸载)。
    Missing,
    Stopped,
    /// 过渡态,文案给人看("启动中…"/"停止中…")。
    Busy(&'static str),
    /// 应用在跑,正在取启动地址。
    Opening,
    /// 取启动地址失败了(再点一次面板图标重试)。
    OpenFailed,
    /// 应用在跑且地址已就绪:webview 盖在面板上,这里只是它下面的底。
    Running,
    /// 应用崩溃/启动失败,带原因。
    Crashed(String),
    /// 应用因运行时缺失/版本不符而起不来:显示专门的提示页(规格 §6.3)。
    RuntimeIssue(AppIssue),
}

#[derive(Debug, Default)]
pub struct State {
    rows: HashMap<AppSlot, Row>,
    order: Vec<AppSlot>,
    phase: Option<Phase>,
    in_flight: bool,
    last_poll: Option<Instant>,
    /// 正在取启动地址的应用。
    launching: HashSet<AppSlot>,
    /// 取启动地址失败过的应用(面板切入时清掉才会再试,避免每次轮询都重试)。
    launch_failed: HashSet<AppSlot>,
    /// 已经把启动地址交给 `AppViews` 的应用。
    url_set: HashSet<AppSlot>,
    acting: HashMap<AppSlot, Act>,
    /// 每个应用崩溃页下方日志查看器的状态(A6e)。应用离开 `Failed` 时复位为 `Hidden`。
    logs: HashMap<AppSlot, LogsView>,
    /// 已经成功拉到过一次列表(并因此同步过图标栏)。首次必须无条件同步:磁盘里的布局可能留着已卸载应用的
    /// 条目,而 `order` 的初值也是空,不能靠"集合变了"来触发。
    synced_once: bool,
}

impl State {
    /// 连不上 dozerd(传输层失败):没有应用面板可见时也要按间隔重试,连上后才恢复安静。
    fn disconnected(&self) -> bool {
        matches!(self.phase, Some(Phase::Disconnected))
    }

    /// 要不要排下一拍唤醒:从没拉过(启动后的第一次)、有应用面板可见(持续轮询)、或者连不上 dozerd
    /// (自愈:dozerd 启动时没起、之后才起或被重启,不用等用户操作就能拿到应用列表)。
    pub fn poll_wanted(&self, app_panel_visible: bool) -> bool {
        !self.in_flight && (self.last_poll.is_none() || app_panel_visible || self.disconnected())
    }

    /// `ResumeTimeReached` 时调用:到点就拉一次列表。
    pub fn poll_if_due(&mut self, now: Instant, visible: &[AppSlot]) -> Vec<Effect> {
        if self.in_flight {
            return Vec::new();
        }
        let due = match self.last_poll {
            None => true,
            Some(t) => {
                (!visible.is_empty() || self.disconnected())
                    && now.duration_since(t) >= POLL_INTERVAL
            }
        };
        if !due {
            return Vec::new();
        }
        self.begin_fetch(now);
        vec![Effect::FetchList]
    }

    fn begin_fetch(&mut self, now: Instant) {
        self.in_flight = true;
        self.last_poll = Some(now);
    }

    pub fn update(&mut self, msg: Message, now: Instant, visible: &[AppSlot]) -> Vec<Effect> {
        match msg {
            Message::ListLoaded(result) => self.list_loaded(result, visible),
            Message::LaunchUrlLoaded(slot, result) => self.launch_url_loaded(slot, result),
            Message::Start(slot) => self.act(slot, Act::Start),
            Message::Stop(slot) => self.act(slot, Act::Stop),
            Message::ActionDone(slot, act, result) => self.action_done(slot, act, result, now),
            Message::Refresh => {
                if self.in_flight {
                    return Vec::new();
                }
                self.begin_fetch(now);
                vec![Effect::FetchList]
            }
            Message::PanelShown(slot) => {
                self.launch_failed.remove(&slot);
                let mut effects = Vec::new();
                if self.url_set.remove(&slot) {
                    effects.push(Effect::ClearUrl(slot));
                }
                if !self.in_flight {
                    self.begin_fetch(now);
                    effects.push(Effect::FetchList);
                }
                effects
            }
            Message::ShowLogs(slot) => self.show_logs(slot),
            Message::HideLogs(slot) => {
                self.logs.insert(slot, LogsView::Hidden);
                Vec::new()
            }
            Message::LogsLoaded(slot, result) => {
                let view = match result {
                    Ok((text, truncated)) => LogsView::Loaded { text, truncated },
                    Err(failure) => LogsView::Failed(failure.text().to_owned()),
                };
                self.logs.insert(slot, view);
                Vec::new()
            }
            Message::OpenRuntimeSettings => vec![Effect::OpenSettingsApps],
        }
    }

    /// 读日志:未展开时置 `Loading` 并发一次请求;在途时重复点不重发。
    fn show_logs(&mut self, slot: AppSlot) -> Vec<Effect> {
        match self.logs.get(&slot) {
            Some(LogsView::Loading) | Some(LogsView::Loaded { .. }) => return Vec::new(),
            _ => {}
        }
        self.logs.insert(slot, LogsView::Loading);
        vec![Effect::FetchLogs(slot, 200)]
    }

    fn list_loaded(
        &mut self,
        result: Result<Vec<AppSummary>, Failure>,
        visible: &[AppSlot],
    ) -> Vec<Effect> {
        self.in_flight = false;
        let apps = match result {
            Ok(apps) => apps,
            Err(Failure::Host(f)) if f.kind == AppErrorKind::Unavailable => {
                self.phase = Some(Phase::Unavailable(f.message));
                return Vec::new();
            }
            Err(Failure::Host(f)) => {
                // 其他类别的列表失败(理论上只有 Internal):保留已知应用,弹一条去重的 Toast。
                return vec![Effect::Toast {
                    level: Level::Warning,
                    text: format!("读取应用列表失败:{}", f.message),
                    key: "app_host:list".into(),
                }];
            }
            Err(Failure::Transport(_)) => {
                self.phase = Some(Phase::Disconnected);
                return Vec::new();
            }
        };
        self.phase = Some(Phase::Loaded);

        let mut effects = Vec::new();
        let mut rows = HashMap::new();
        let mut order = Vec::new();
        for app in apps {
            // 形状非法的 id 不会出现(宿主已校验);万一槽表满了就跳过这个应用,不让整个列表失败。
            let Some(slot) = AppSlot::intern(app.id.as_str()) else {
                continue;
            };
            order.push(slot);
            rows.insert(
                slot,
                Row {
                    id: app.id.as_str().to_owned(),
                    name: app.name,
                    desired: app.desired,
                    observed: app.observed,
                    issue: app.issue,
                },
            );
        }
        // 消失的应用:撤掉它的地址与在途状态。
        let gone: Vec<AppSlot> = self
            .rows
            .keys()
            .filter(|s| !rows.contains_key(s))
            .copied()
            .collect();
        for slot in gone {
            self.forget(slot, &mut effects);
        }
        // 不再运行的应用:撤掉地址(webview 随之被池回收)。
        for (slot, row) in &rows {
            if row.observed != ObservedState::Running {
                self.launching.remove(slot);
                self.launch_failed.remove(slot);
                if self.url_set.remove(slot) {
                    effects.push(Effect::ClearUrl(*slot));
                }
            }
            // 离开崩溃态(被重启/运行/停止):旧日志查看器复位,防陈旧。
            if !matches!(row.observed, ObservedState::Failed { .. }) {
                self.logs.remove(slot);
            }
        }
        let set_changed = !self.synced_once || order != self.order;
        self.synced_once = true;
        self.rows = rows;
        self.order = order;
        if set_changed {
            effects.push(Effect::SyncRail(self.order.clone()));
        }
        // 可见的应用在跑、还没有地址:去取一个(左右两栏可以同时显示两个应用面板,每个都要取)。
        for &slot in visible {
            if self
                .rows
                .get(&slot)
                .is_some_and(|r| r.observed == ObservedState::Running)
                && !self.url_set.contains(&slot)
                && !self.launching.contains(&slot)
                && !self.launch_failed.contains(&slot)
            {
                self.launching.insert(slot);
                effects.push(Effect::FetchLaunchUrl(slot));
            }
        }
        effects
    }

    fn forget(&mut self, slot: AppSlot, effects: &mut Vec<Effect>) {
        self.launching.remove(&slot);
        self.launch_failed.remove(&slot);
        self.acting.remove(&slot);
        self.logs.remove(&slot);
        if self.url_set.remove(&slot) {
            effects.push(Effect::ClearUrl(slot));
        }
    }

    fn launch_url_loaded(&mut self, slot: AppSlot, result: Result<String, Failure>) -> Vec<Effect> {
        self.launching.remove(&slot);
        let still_running = self
            .rows
            .get(&slot)
            .is_some_and(|r| r.observed == ObservedState::Running);
        match result {
            Ok(url) if still_running => {
                self.url_set.insert(slot);
                vec![Effect::SetUrl(slot, url)]
            }
            // 应用在取地址期间停了:丢掉这个(已经没用的)地址。
            Ok(_) => Vec::new(),
            Err(failure) => {
                self.launch_failed.insert(slot);
                vec![Effect::Toast {
                    level: Level::Error,
                    text: format!("打开应用 {} 失败:{}", self.name(slot), failure.text()),
                    key: format!("app_host:launch:{}", self.id(slot)),
                }]
            }
        }
    }

    fn act(&mut self, slot: AppSlot, act: Act) -> Vec<Effect> {
        if self.acting.contains_key(&slot) || !self.rows.contains_key(&slot) {
            return Vec::new();
        }
        self.acting.insert(slot, act);
        vec![match act {
            Act::Start => Effect::StartApp(slot),
            Act::Stop => Effect::StopApp(slot),
        }]
    }

    fn action_done(
        &mut self,
        slot: AppSlot,
        act: Act,
        result: Result<(), Failure>,
        now: Instant,
    ) -> Vec<Effect> {
        self.acting.remove(&slot);
        let mut effects = Vec::new();
        if let Err(failure) = result {
            let verb = match act {
                Act::Start => "启动",
                Act::Stop => "停止",
            };
            effects.push(Effect::Toast {
                level: Level::Error,
                text: format!("{verb}应用 {} 失败:{}", self.name(slot), failure.text()),
                key: format!("app_host:act:{}", self.id(slot)),
            });
        }
        // 不管成败都立刻刷新一次,面板马上反映真实状态。
        if !self.in_flight {
            self.begin_fetch(now);
            effects.push(Effect::FetchList);
        }
        effects
    }

    fn name(&self, slot: AppSlot) -> String {
        self.rows
            .get(&slot)
            .map(|r| r.name.clone())
            .unwrap_or_else(|| slot.id().to_owned())
    }

    fn id(&self, slot: AppSlot) -> &str {
        self.rows
            .get(&slot)
            .map(|r| r.id.as_str())
            .unwrap_or_else(|| slot.id())
    }

    /// 面板该画什么。
    pub fn view_model(&self, slot: AppSlot) -> PanelView {
        if let Some(Phase::Unavailable(reason)) = &self.phase {
            return PanelView::HostUnavailable(reason.clone());
        }
        let Some(row) = self.rows.get(&slot) else {
            return match self.phase {
                None => PanelView::Loading,
                // 连不上 dozerd 时什么都不知道——不能说"应用已不在列表里"。
                Some(Phase::Disconnected) => PanelView::Disconnected,
                Some(_) => PanelView::Missing,
            };
        };
        match self.acting.get(&slot) {
            Some(Act::Start) => return PanelView::Busy("启动中…"),
            Some(Act::Stop) => return PanelView::Busy("停止中…"),
            None => {}
        }
        match &row.observed {
            ObservedState::Running => {
                if self.url_set.contains(&slot) {
                    PanelView::Running
                } else if self.launch_failed.contains(&slot) {
                    PanelView::OpenFailed
                } else {
                    PanelView::Opening
                }
            }
            ObservedState::Preparing | ObservedState::Starting | ObservedState::Updating => {
                PanelView::Busy("启动中…")
            }
            ObservedState::Stopping | ObservedState::Uninstalling => PanelView::Busy("停止中…"),
            ObservedState::Failed { reason, .. } => match &row.issue {
                // 有"看得懂的问题"→ 专门的运行时提示页;否则普通崩溃页(可看日志)。
                Some(issue) => PanelView::RuntimeIssue(issue.clone()),
                None => PanelView::Crashed(reason.clone()),
            },
            ObservedState::Installed | ObservedState::Stopped | ObservedState::NotInstalled => {
                PanelView::Stopped
            }
        }
    }

    /// 崩溃页下方日志查看器的当前状态(未知应用按 `Hidden`)。
    pub fn logs_view(&self, slot: AppSlot) -> LogsView {
        self.logs.get(&slot).cloned().unwrap_or(LogsView::Hidden)
    }

    /// 某应用的显示名(面板标题行用);未知时退回 id。
    pub fn display_name(&self, slot: AppSlot) -> String {
        self.name(slot)
    }
}

/// 把解释器名映射成给人看的运行时名。
fn runtime_display_name(runtime: &str) -> &str {
    match runtime {
        "node" | "nodejs" => "Node.js",
        "python" | "python3" => "Python",
        "uv" => "uv",
        other => other,
    }
}

/// 运行时问题页的标题与详情(纯函数,便于表驱动测试)。
pub fn issue_texts(issue: &AppIssue) -> (String, String) {
    match issue {
        AppIssue::RuntimeMissing { runtime } => (
            format!("需要 {}", runtime_display_name(runtime)),
            "尚未安装,可在 设置 → 应用 里一键安装".into(),
        ),
        AppIssue::RuntimeVersion {
            runtime,
            required,
            found,
        } => (
            format!("需要 {}", runtime_display_name(runtime)),
            format!("要求 {required},当前 {found}"),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytehost_apps::id::{AppId, Version};

    fn slot(id: &str) -> AppSlot {
        AppSlot::intern(id).unwrap()
    }

    fn app(id: &str, observed: ObservedState) -> AppSummary {
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

    fn app_with_issue(id: &str, observed: ObservedState, issue: AppIssue) -> AppSummary {
        AppSummary {
            issue: Some(issue),
            ..app(id, observed)
        }
    }

    fn failed(reason: &str) -> ObservedState {
        ObservedState::Failed {
            reason: reason.into(),
            retryable: true,
        }
    }

    fn runtime_version_issue() -> AppIssue {
        AppIssue::RuntimeVersion {
            runtime: "python3".into(),
            required: ">=99".into(),
            found: "3.13.0".into(),
        }
    }

    fn running(id: &str) -> AppSummary {
        app(id, ObservedState::Running)
    }

    fn host_failure(kind: AppErrorKind, message: &str) -> Failure {
        Failure::Host(AppFailure::new(kind, message))
    }

    fn loaded(state: &mut State, apps: Vec<AppSummary>, visible: &[AppSlot]) -> Vec<Effect> {
        state.update(Message::ListLoaded(Ok(apps)), Instant::now(), visible)
    }

    #[test]
    fn the_first_poll_is_due_and_later_ones_only_while_an_app_panel_is_visible() {
        let mut s = State::default();
        let t0 = Instant::now();
        assert!(s.poll_wanted(false), "启动后至少拉一次");
        assert_eq!(s.poll_if_due(t0, &[]), vec![Effect::FetchList]);
        assert!(s.poll_if_due(t0, &[]).is_empty(), "在途时不重复发");
        s.update(Message::ListLoaded(Ok(vec![])), t0, &[]);
        assert!(!s.poll_wanted(false), "没有应用面板可见就不再轮询");
        assert!(s.poll_if_due(t0 + POLL_INTERVAL * 5, &[]).is_empty());
        let a = [slot("hp-a")];
        assert!(s.poll_wanted(true));
        assert!(
            s.poll_if_due(t0 + POLL_INTERVAL / 2, &a).is_empty(),
            "没到点"
        );
        assert_eq!(
            s.poll_if_due(t0 + POLL_INTERVAL, &a),
            vec![Effect::FetchList]
        );
    }

    #[test]
    fn the_rail_is_synced_when_the_installed_set_changes_and_not_otherwise() {
        let mut s = State::default();
        let (a, b) = (slot("rail-sync-a"), slot("rail-sync-b"));
        let first = loaded(
            &mut s,
            vec![app("rail-sync-a", ObservedState::Stopped)],
            &[],
        );
        assert_eq!(first, vec![Effect::SyncRail(vec![a])]);
        assert!(
            loaded(
                &mut s,
                vec![app("rail-sync-a", ObservedState::Stopped)],
                &[]
            )
            .is_empty()
        );
        let both = loaded(
            &mut s,
            vec![
                app("rail-sync-a", ObservedState::Stopped),
                app("rail-sync-b", ObservedState::Stopped),
            ],
            &[],
        );
        assert_eq!(both, vec![Effect::SyncRail(vec![a, b])]);
        let removed = loaded(
            &mut s,
            vec![app("rail-sync-b", ObservedState::Stopped)],
            &[],
        );
        assert_eq!(removed, vec![Effect::SyncRail(vec![b])]);
    }

    #[test]
    fn a_visible_running_app_gets_exactly_one_launch_url_fetch() {
        let mut s = State::default();
        let a = slot("launch-a");
        let first = loaded(&mut s, vec![running("launch-a")], &[a]);
        assert!(first.contains(&Effect::FetchLaunchUrl(a)));
        let again = loaded(&mut s, vec![running("launch-a")], &[a]);
        assert!(
            !again.contains(&Effect::FetchLaunchUrl(a)),
            "在途时不重复取"
        );
        assert_eq!(s.view_model(a), PanelView::Opening);
        let url = "http://launch-a.localhost:20001/?bh_token=t".to_string();
        let done = s.update(
            Message::LaunchUrlLoaded(a, Ok(url.clone())),
            Instant::now(),
            &[a],
        );
        assert_eq!(done, vec![Effect::SetUrl(a, url)]);
        assert_eq!(s.view_model(a), PanelView::Running);
        let later = loaded(&mut s, vec![running("launch-a")], &[a]);
        assert!(later.is_empty(), "已有地址就不再取");
    }

    #[test]
    fn a_running_app_whose_panel_is_not_visible_is_not_opened() {
        let mut s = State::default();
        let effects = loaded(&mut s, vec![running("hidden-a")], &[]);
        assert!(
            !effects
                .iter()
                .any(|e| matches!(e, Effect::FetchLaunchUrl(_)))
        );
    }

    #[test]
    fn a_launch_url_that_arrives_after_the_app_stopped_is_dropped() {
        let mut s = State::default();
        let a = slot("late-a");
        loaded(&mut s, vec![running("late-a")], &[a]);
        loaded(&mut s, vec![app("late-a", ObservedState::Stopped)], &[a]);
        let done = s.update(
            Message::LaunchUrlLoaded(a, Ok("http://late-a.localhost:1/".into())),
            Instant::now(),
            &[a],
        );
        assert!(done.is_empty());
        assert_eq!(s.view_model(a), PanelView::Stopped);
    }

    #[test]
    fn a_failed_launch_toasts_once_and_is_not_retried_until_the_panel_is_shown_again() {
        let mut s = State::default();
        let a = slot("fail-a");
        loaded(&mut s, vec![running("fail-a")], &[a]);
        let failed = s.update(
            Message::LaunchUrlLoaded(a, Err(host_failure(AppErrorKind::Conflict, "不在运行"))),
            Instant::now(),
            &[a],
        );
        assert_eq!(failed.len(), 1);
        match &failed[0] {
            Effect::Toast { level, text, key } => {
                assert_eq!(*level, Level::Error);
                assert!(
                    text.contains("name-fail-a") && text.contains("不在运行"),
                    "{text}"
                );
                assert_eq!(key, "app_host:launch:fail-a");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(s.view_model(a), PanelView::OpenFailed);
        let poll = loaded(&mut s, vec![running("fail-a")], &[a]);
        assert!(
            !poll.iter().any(|e| matches!(e, Effect::FetchLaunchUrl(_))),
            "轮询不重试"
        );
        // 再次切入面板:清状态并重新拉列表,列表回来后再取地址。
        let shown = s.update(Message::PanelShown(a), Instant::now(), &[a]);
        assert_eq!(shown, vec![Effect::FetchList]);
        let after = loaded(&mut s, vec![running("fail-a")], &[a]);
        assert!(after.contains(&Effect::FetchLaunchUrl(a)));
    }

    #[test]
    fn refresh_fetches_the_list_unless_one_is_already_in_flight() {
        let mut s = State::default();
        assert_eq!(
            s.update(Message::Refresh, Instant::now(), &[]),
            vec![Effect::FetchList]
        );
        assert!(s.update(Message::Refresh, Instant::now(), &[]).is_empty());
    }

    #[test]
    fn showing_the_panel_discards_the_old_address_so_a_fresh_one_is_fetched() {
        let mut s = State::default();
        let a = slot("shown-a");
        loaded(&mut s, vec![running("shown-a")], &[a]);
        s.update(
            Message::LaunchUrlLoaded(a, Ok("http://shown-a.localhost:1/?bh_token=old".into())),
            Instant::now(),
            &[a],
        );
        let shown = s.update(Message::PanelShown(a), Instant::now(), &[a]);
        assert_eq!(shown, vec![Effect::ClearUrl(a), Effect::FetchList]);
        assert_eq!(s.view_model(a), PanelView::Opening);
        let again = s.update(Message::PanelShown(a), Instant::now(), &[a]);
        assert!(again.is_empty(), "已有列表请求在途,不重复发");
    }

    #[test]
    fn an_app_that_stops_or_disappears_loses_its_address() {
        let mut s = State::default();
        let (a, b) = (slot("drop-a"), slot("drop-b"));
        loaded(&mut s, vec![running("drop-a"), running("drop-b")], &[a]);
        for x in [a, b] {
            s.update(
                Message::LaunchUrlLoaded(x, Ok(format!("http://{}.localhost:1/", x.id()))),
                Instant::now(),
                &[a],
            );
        }
        let effects = loaded(&mut s, vec![app("drop-a", ObservedState::Stopped)], &[a]);
        assert!(effects.contains(&Effect::ClearUrl(a)), "停了");
        assert!(effects.contains(&Effect::ClearUrl(b)), "没了");
        assert!(effects.contains(&Effect::SyncRail(vec![a])));
        assert_eq!(s.view_model(b), PanelView::Missing);
    }

    #[test]
    fn an_unavailable_host_is_a_persistent_panel_state_not_a_toast() {
        let mut s = State::default();
        let a = slot("unavail-a");
        loaded(&mut s, vec![running("unavail-a")], &[]);
        let effects = s.update(
            Message::ListLoaded(Err(host_failure(AppErrorKind::Unavailable, "端口被占用"))),
            Instant::now(),
            &[],
        );
        assert!(effects.is_empty());
        assert_eq!(
            s.view_model(a),
            PanelView::HostUnavailable("端口被占用".into())
        );
        loaded(&mut s, vec![running("unavail-a")], &[]);
        assert_eq!(s.view_model(a), PanelView::Opening, "恢复后回到正常状态");
    }

    #[test]
    fn a_transport_failure_keeps_the_known_apps_and_stays_quiet() {
        let mut s = State::default();
        let a = slot("transport-a");
        loaded(
            &mut s,
            vec![app("transport-a", ObservedState::Stopped)],
            &[],
        );
        let effects = s.update(
            Message::ListLoaded(Err(Failure::Transport("连不上".into()))),
            Instant::now(),
            &[],
        );
        assert!(effects.is_empty());
        assert_eq!(s.view_model(a), PanelView::Stopped);
    }

    #[test]
    fn start_and_stop_are_deduplicated_and_refresh_the_list_when_done() {
        let mut s = State::default();
        let a = slot("act-a");
        loaded(&mut s, vec![app("act-a", ObservedState::Stopped)], &[]);
        s.update(
            Message::ListLoaded(Ok(vec![app("act-a", ObservedState::Stopped)])),
            Instant::now(),
            &[],
        );
        assert_eq!(
            s.update(Message::Start(a), Instant::now(), &[]),
            vec![Effect::StartApp(a)]
        );
        assert!(s.update(Message::Start(a), Instant::now(), &[]).is_empty());
        assert!(
            s.update(Message::Stop(a), Instant::now(), &[]).is_empty(),
            "在途时也不接受另一个动作"
        );
        assert_eq!(s.view_model(a), PanelView::Busy("启动中…"));
        let done = s.update(
            Message::ActionDone(a, Act::Start, Ok(())),
            Instant::now(),
            &[],
        );
        assert_eq!(done, vec![Effect::FetchList]);
    }

    #[test]
    fn a_failed_action_toasts_with_the_verb_and_still_refreshes() {
        let mut s = State::default();
        let a = slot("actfail-a");
        loaded(&mut s, vec![app("actfail-a", ObservedState::Stopped)], &[]);
        s.update(Message::Stop(a), Instant::now(), &[]);
        let done = s.update(
            Message::ActionDone(
                a,
                Act::Stop,
                Err(host_failure(AppErrorKind::Conflict, "状态冲突")),
            ),
            Instant::now(),
            &[],
        );
        assert_eq!(done.len(), 2);
        match &done[0] {
            Effect::Toast { text, key, .. } => {
                assert!(text.contains("停止") && text.contains("状态冲突"), "{text}");
                assert_eq!(key, "app_host:act:actfail-a");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(done[1], Effect::FetchList);
    }

    #[test]
    fn actions_on_unknown_apps_are_ignored() {
        let mut s = State::default();
        assert!(
            s.update(Message::Start(slot("nobody-a")), Instant::now(), &[])
                .is_empty()
        );
    }

    #[test]
    fn the_view_model_covers_every_observed_state() {
        let mut s = State::default();
        let a = slot("vm-a");
        let cases = [
            (ObservedState::Installed, PanelView::Stopped),
            (ObservedState::Stopped, PanelView::Stopped),
            (ObservedState::Preparing, PanelView::Busy("启动中…")),
            (ObservedState::Starting, PanelView::Busy("启动中…")),
            (ObservedState::Stopping, PanelView::Busy("停止中…")),
            (
                ObservedState::Failed {
                    reason: "崩了".into(),
                    retryable: false,
                },
                PanelView::Crashed("崩了".into()),
            ),
            (ObservedState::Running, PanelView::Opening),
        ];
        for (observed, want) in cases {
            loaded(&mut s, vec![app("vm-a", observed.clone())], &[]);
            assert_eq!(s.view_model(a), want, "{observed:?}");
        }
        assert_eq!(State::default().view_model(a), PanelView::Loading);
    }

    /// 磁盘里留着已卸载应用的 `app:<id>` 条目;第一次拉到的列表是空的也必须同步(把它清掉),
    /// 不能因为 `order` 的初值也是空就当成"没变"。
    #[test]
    fn the_first_successful_list_always_syncs_the_rail_even_when_empty() {
        let mut s = State::default();
        let effects = loaded(&mut s, vec![], &[]);
        assert_eq!(effects, vec![Effect::SyncRail(vec![])]);
        assert!(loaded(&mut s, vec![], &[]).is_empty(), "之后没变就不再同步");
    }

    #[test]
    fn the_first_list_syncs_even_if_an_unavailable_answer_came_first() {
        let mut s = State::default();
        s.update(
            Message::ListLoaded(Err(host_failure(AppErrorKind::Unavailable, "端口被占用"))),
            Instant::now(),
            &[],
        );
        let effects = loaded(&mut s, vec![], &[]);
        assert!(effects.contains(&Effect::SyncRail(vec![])), "{effects:?}");
    }

    /// 连不上 dozerd 而且没有任何已知应用时,面板不能说"这个应用已不在已安装列表里"。
    #[test]
    fn a_disconnected_panel_with_no_known_apps_says_so_instead_of_claiming_the_app_is_gone() {
        let mut s = State::default();
        let a = slot("disc-a");
        s.update(
            Message::ListLoaded(Err(Failure::Transport("连不上".into()))),
            Instant::now(),
            &[],
        );
        assert_eq!(s.view_model(a), PanelView::Disconnected);
        // 成功拉到列表之后才有资格说"已不在列表里"。
        loaded(&mut s, vec![], &[]);
        assert_eq!(s.view_model(a), PanelView::Missing);
    }

    /// 左右两栏可以同时各显示一个应用面板,两个都要取启动地址。
    #[test]
    fn two_visible_app_panels_each_get_their_launch_url_fetch() {
        let mut s = State::default();
        let (a, b) = (slot("two-a"), slot("two-b"));
        let effects = loaded(&mut s, vec![running("two-a"), running("two-b")], &[a, b]);
        assert!(effects.contains(&Effect::FetchLaunchUrl(a)));
        assert!(effects.contains(&Effect::FetchLaunchUrl(b)));
        let again = loaded(&mut s, vec![running("two-a"), running("two-b")], &[a, b]);
        assert!(
            !again.iter().any(|e| matches!(e, Effect::FetchLaunchUrl(_))),
            "各自在途时不重复取"
        );
    }

    /// dozerd 暂时连不上(启动时没起、被重启中)时,即使没有应用面板可见也要继续按间隔重试,
    /// 连上后恢复安静——否则"启动时没起、之后才起"的 dozerd 永远等不到应用列表。
    #[test]
    fn a_disconnected_host_keeps_polling_until_the_daemon_answers() {
        let mut s = State::default();
        let t0 = Instant::now();
        s.poll_if_due(t0, &[]);
        s.update(
            Message::ListLoaded(Err(Failure::Transport("连不上".into()))),
            t0,
            &[],
        );
        assert!(s.poll_wanted(false), "断开状态要继续排唤醒");
        assert!(
            s.poll_if_due(t0 + POLL_INTERVAL / 2, &[]).is_empty(),
            "没到点"
        );
        assert_eq!(
            s.poll_if_due(t0 + POLL_INTERVAL, &[]),
            vec![Effect::FetchList]
        );
        s.update(Message::ListLoaded(Ok(vec![])), t0 + POLL_INTERVAL, &[]);
        assert!(!s.poll_wanted(false), "连上以后恢复到不轮询");
        assert!(s.poll_if_due(t0 + POLL_INTERVAL * 9, &[]).is_empty());
    }

    // ===== A6e Task 5:运行时问题页 / 日志查看器 / 跳设置 =====

    #[test]
    fn a_failed_app_with_a_runtime_issue_shows_the_issue_page_not_the_crash_page() {
        let mut s = State::default();
        let a = slot("issue-a");
        loaded(
            &mut s,
            vec![app_with_issue(
                "issue-a",
                failed("需要 python3 >=99,当前 3.13.0"),
                runtime_version_issue(),
            )],
            &[],
        );
        assert_eq!(
            s.view_model(a),
            PanelView::RuntimeIssue(runtime_version_issue())
        );

        // 无 issue 的 Failed 仍是普通崩溃页。
        let b = slot("issue-b");
        loaded(&mut s, vec![app("issue-b", failed("崩了"))], &[]);
        assert_eq!(s.view_model(b), PanelView::Crashed("崩了".into()));

        // 陈旧问题:应用已经跑起来了,就不该再显示问题页。
        let c = slot("issue-c");
        loaded(
            &mut s,
            vec![app_with_issue(
                "issue-c",
                ObservedState::Running,
                runtime_version_issue(),
            )],
            &[],
        );
        assert_eq!(s.view_model(c), PanelView::Opening);
    }

    #[test]
    fn showing_logs_fetches_once_and_stores_the_result() {
        let mut s = State::default();
        let a = slot("logs-a");
        loaded(&mut s, vec![app("logs-a", failed("崩了"))], &[]);
        assert_eq!(s.logs_view(a), LogsView::Hidden);

        let effects = s.update(Message::ShowLogs(a), Instant::now(), &[]);
        assert_eq!(effects, vec![Effect::FetchLogs(a, 200)]);
        assert_eq!(s.logs_view(a), LogsView::Loading);
        // 在途时重复点不重复发。
        assert!(
            s.update(Message::ShowLogs(a), Instant::now(), &[])
                .is_empty()
        );

        s.update(
            Message::LogsLoaded(a, Ok(("line1\nline2".into(), true))),
            Instant::now(),
            &[],
        );
        assert_eq!(
            s.logs_view(a),
            LogsView::Loaded {
                text: "line1\nline2".into(),
                truncated: true
            }
        );
        assert!(
            s.update(Message::HideLogs(a), Instant::now(), &[])
                .is_empty()
        );
        assert_eq!(s.logs_view(a), LogsView::Hidden);
    }

    #[test]
    fn logs_failure_is_kept_in_place_not_toasted() {
        let mut s = State::default();
        let a = slot("logsfail-a");
        loaded(&mut s, vec![app("logsfail-a", failed("崩了"))], &[]);
        s.update(Message::ShowLogs(a), Instant::now(), &[]);
        let effects = s.update(
            Message::LogsLoaded(a, Err(host_failure(AppErrorKind::NotFound, "没有这个应用"))),
            Instant::now(),
            &[],
        );
        assert!(effects.is_empty(), "日志失败不进 Toast:{effects:?}");
        assert_eq!(s.logs_view(a), LogsView::Failed("没有这个应用".into()));
    }

    #[test]
    fn logs_are_hidden_again_when_the_app_leaves_the_failed_state() {
        let mut s = State::default();
        let a = slot("reset-a");
        loaded(&mut s, vec![app("reset-a", failed("崩了"))], &[]);
        s.update(Message::ShowLogs(a), Instant::now(), &[]);
        s.update(
            Message::LogsLoaded(a, Ok(("x".into(), false))),
            Instant::now(),
            &[],
        );
        assert!(matches!(s.logs_view(a), LogsView::Loaded { .. }));

        // 被重启回 Starting/Running:日志查看器复位。
        loaded(&mut s, vec![app("reset-a", ObservedState::Running)], &[]);
        assert_eq!(s.logs_view(a), LogsView::Hidden);
    }

    #[test]
    fn open_runtime_settings_emits_exactly_one_effect() {
        let mut s = State::default();
        let effects = s.update(Message::OpenRuntimeSettings, Instant::now(), &[]);
        assert_eq!(effects, vec![Effect::OpenSettingsApps]);
    }

    #[test]
    fn retry_from_the_issue_page_starts_the_app() {
        let mut s = State::default();
        let a = slot("retry-a");
        loaded(
            &mut s,
            vec![app_with_issue(
                "retry-a",
                failed("需要 python3"),
                AppIssue::RuntimeMissing {
                    runtime: "python3".into(),
                },
            )],
            &[],
        );
        let effects = s.update(Message::Start(a), Instant::now(), &[]);
        assert_eq!(effects, vec![Effect::StartApp(a)]);
        // issue 页下 acting 优先 → 显示"启动中…"。
        assert_eq!(s.view_model(a), PanelView::Busy("启动中…"));
    }

    #[test]
    fn issue_texts_cover_missing_and_version_mismatch() {
        let cases = [
            (
                AppIssue::RuntimeMissing {
                    runtime: "node".into(),
                },
                ("需要 Node.js", "尚未安装,可在 设置 → 应用 里一键安装"),
            ),
            (
                AppIssue::RuntimeMissing {
                    runtime: "python3".into(),
                },
                ("需要 Python", "尚未安装,可在 设置 → 应用 里一键安装"),
            ),
            (
                AppIssue::RuntimeVersion {
                    runtime: "node".into(),
                    required: ">=18".into(),
                    found: "16.0.0".into(),
                },
                ("需要 Node.js", "要求 >=18,当前 16.0.0"),
            ),
        ];
        for (issue, (title, detail)) in cases {
            let (t, d) = issue_texts(&issue);
            assert_eq!(t, title, "{issue:?}");
            assert_eq!(d, detail, "{issue:?}");
        }
    }
}
