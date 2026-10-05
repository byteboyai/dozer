# bytehost A4b2:应用面板的宿主逻辑(列表轮询 + 面板状态机 + 启停) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让应用面板真的"活起来":启动后拉一次已安装应用列表并同步到图标栏(`sync_installed_apps`);应用面板可见时持续轮询;每个应用面板有状态机(未运行/启动中/打开中/运行/崩溃/宿主不可用);应用在跑时取启动地址交给 A4b1 的 webview;面板里能启动应用;失败按"瞬时 → Toast、持久 → 留在面板里"分流。

**Architecture:** 新的纯状态机 `extensions/app_host.rs`(`State::update`/`poll_if_due` 只返回 `Effect`,不碰窗口/网络)+ `App` 里一层薄的效果执行器(`run_app_host_effects`:发请求、同步 rail、写/清 `AppViews`、弹 Toast)。新消息 `Message::AppHost`;轮询接在既有的 `new_events`/`about_to_wait` 唤醒机制上(同群聊/Todo 的做法)。面板内容由 `State::view_model` 决定。

**Tech Stack:** Rust、iced 0.14、`dozer-client::app_*`、`bytehost-apps`(类型,默认 feature 只有 serde)。**dozer-app 新增对 `bytehost-apps` 的依赖**(线上类型 `AppSummary`/`AppFailure`/`ObservedState`/`AppId`;`Cargo.lock` 只多一行)。

**Spec:** `docs/superpowers/specs/2026-10-04-bytehost-app-host-design.md`(A4 行);前置:A3(rail 条目)、A4a(类别化失败)、A4b1(应用 webview 机制,含评审后修订)。

## Global Constraints

- **轮询,不做推送**(A4a 的裁决);应用面板可见时 `POLL_INTERVAL = 2s`,否则只在启动后拉一次、面板切入与启停完成后各拉一次。
- **启动地址是秘密**:只经 `Effect::SetUrl` 进内存里的 `AppViews`,不写日志、不落盘、不进 Toast 文案。
- **每次面板切入清掉旧地址并重取**(webview 因切面板/切项目被销毁重建时不会拿到过期地址)。
- **Toast 规则**(CLAUDE.md):瞬时失败(启停失败、取地址失败、列表读取的非不可用类失败)走 `push_toast_keyed`;"宿主不可用"是持久状态,只在面板里显示原因,**不弹 Toast**;传输层失败(连不上 dozerd)不清空已知应用、不弹 Toast(顶栏徽标负责)。
- **日志**:`App` 里沿用 `shell` 来源;`app_host.rs` 是纯状态机不写日志。
- **字体/按钮**:面板按钮用 `byteui::feedback::dialog::action_button_style`(现有面板同款),文本用系统默认字体。
- 不新增面向用户的安装/卸载/设置界面——那是 A4c。

## Review Focus

- 轮询不空转:没有应用面板可见时,启动后只拉一次(`poll_wanted` 为假,`about_to_wait` 不再为它排唤醒)。
- 不重复请求:列表在途时不再发;取启动地址在途时不再发;启停在途时不接受第二个动作。
- 取地址失败后**不**在每次轮询里重试(会刷屏 Toast),只在面板再次切入时重试。
- 应用在取地址期间停掉:迟到的地址被丢弃,不写进 `AppViews`。
- 应用停止/消失:立刻清掉地址(A4b1 的池随之回收 webview);消失的应用同步出图标栏。
- 宿主不可用恢复后,面板回到正常状态,不残留旧的不可用提示。
- 只有 `AppFailure` 才分类;`Transport` 错误不被当成"宿主不可用"。

---

### Task 1: 纯状态机 `extensions/app_host.rs`

**Files:**
- Create: `crates/dozer-app/src/extensions/app_host.rs`
- Modify: `crates/dozer-app/src/extensions.rs`(`pub mod app_host;`)、`crates/dozer-app/Cargo.toml` + `Cargo.lock`(`bytehost-apps` 依赖)

**Interfaces:**
- Produces: `State`(`poll_wanted(bool)`、`poll_if_due(now, visible)`、`update(msg, now, visible)`、`view_model(slot)`、`display_name(slot)`)、`Message{ListLoaded, LaunchUrlLoaded, ActionDone, Start, Stop, PanelShown}`、`Effect{FetchList, FetchLaunchUrl, StartApp, StopApp, SyncRail, SetUrl, ClearUrl, Toast}`、`PanelView`、`Failure::from_client_error(&anyhow::Error)`、`Act`、`POLL_INTERVAL`。

- [ ] **Step 1: 写失败测试。** 新文件末尾的 `tests` 模块(14 个)先写,`State` 的方法先 `todo!()`。
- [ ] **Step 2: RED。** `cargo test -p dozer-app app_host` → 失败。
- [ ] **Step 3: 实现。**

```diff
diff --git a/crates/dozer-app/Cargo.toml b/crates/dozer-app/Cargo.toml
index 707ee7e5..c1b3732c 100644
--- a/crates/dozer-app/Cargo.toml
+++ b/crates/dozer-app/Cargo.toml
@@ -15,4 +15,5 @@ default = []
 dozer-core = { path = "../dozer-core", features = ["logging"] }
 dozer-client = { path = "../dozer-client" }
+bytehost-apps = { path = "../bytehost-apps" }
 dozer-codehealth = { path = "../dozer-codehealth" }
 # 复用 dozer-hook 的 hook 安装逻辑(install::run_at/opencode_install::run_at)，
diff --git a/crates/dozer-app/src/extensions.rs b/crates/dozer-app/src/extensions.rs
index b88f72ea..bae27f6e 100644
--- a/crates/dozer-app/src/extensions.rs
+++ b/crates/dozer-app/src/extensions.rs
@@ -5,4 +5,5 @@
 
 pub mod agent_context;
+pub mod app_host;
 pub mod browser;
 pub mod codehealth;
diff --git a/crates/dozer-app/src/extensions/app_host.rs b/crates/dozer-app/src/extensions/app_host.rs
new file mode 100644
index 00000000..c1262320
--- /dev/null
+++ b/crates/dozer-app/src/extensions/app_host.rs
@@ -0,0 +1,745 @@
+//! 应用面板的宿主逻辑(bytehost A4b2):dozerd 里已安装应用的列表轮询、每个应用面板的状态机、启动/停止、
+//! "该给 webview 的地址"。**纯状态机**:`update`/`poll_if_due` 只返回 [`Effect`],由 `App` 去执行
+//! (发请求、同步 rail、写 `AppViews`、弹 Toast)——这样整套逻辑可以不开窗口、不连 dozerd 地测。
+//!
+//! 约定(规格 A4 与 A4b 评审留下的):
+//! - **轮询,不做推送**:应用面板可见时每 [`POLL_INTERVAL`] 拉一次 `List`;没有应用面板可见时只在启动后拉一次,
+//!   以及面板切入、启停操作完成后各拉一次;
+//! - **启动地址含令牌(秘密)**:每次面板切入都清掉旧地址、重新取(webview 因切面板/切项目被销毁重建时不会拿到
+//!   过期地址);地址只经 `Effect::SetUrl` 进内存里的 `AppViews`,不进日志、不落盘;
+//! - **瞬时失败走 Toast,持久状态留在面板里**:启停/取地址失败是"刚发生的一件事"→ Toast;宿主不可用
+//!   (`AppErrorKind::Unavailable`)是"当前处于某状态"→ 面板里显示原因,不弹 Toast;
+//! - 列表请求的传输层失败(连不上 dozerd)不清空已知的应用、不弹 Toast——顶栏的 dozerd 徽标负责告知。
+
+use std::collections::{HashMap, HashSet};
+use std::time::{Duration, Instant};
+
+use bytehost_apps::proto::{AppErrorKind, AppFailure, AppSummary};
+use bytehost_apps::state::{DesiredState, ObservedState};
+
+use crate::app::AppSlot;
+use crate::extensions::toast::Level;
+
+/// 应用面板可见时拉 `List` 的间隔。
+pub const POLL_INTERVAL: Duration = Duration::from_secs(2);
+
+/// 一次请求的失败:宿主带类别的失败,或者传输/协议层的(连不上 dozerd、协议版本不符)。
+#[derive(Debug, Clone, PartialEq)]
+pub enum Failure {
+    Host(AppFailure),
+    Transport(String),
+}
+
+impl Failure {
+    /// 从 `dozer-client` 的错误还原:能 `downcast` 成 `AppFailure` 的是宿主失败,其余是传输层。
+    pub fn from_client_error(e: &anyhow::Error) -> Self {
+        match e.downcast_ref::<AppFailure>() {
+            Some(f) => Self::Host(f.clone()),
+            None => Self::Transport(format!("{e:#}")),
+        }
+    }
+
+    fn text(&self) -> &str {
+        match self {
+            Self::Host(f) => &f.message,
+            Self::Transport(t) => t,
+        }
+    }
+}
+
+#[derive(Debug, Clone, Copy, PartialEq, Eq)]
+pub enum Act {
+    Start,
+    Stop,
+}
+
+#[derive(Debug, Clone)]
+pub enum Message {
+    ListLoaded(Result<Vec<AppSummary>, Failure>),
+    LaunchUrlLoaded(AppSlot, Result<String, Failure>),
+    ActionDone(AppSlot, Act, Result<(), Failure>),
+    /// 用户点了面板里的「启动」/「停止」。
+    Start(AppSlot),
+    Stop(AppSlot),
+    /// 该应用面板被切到前台(图标栏点击或程序化显示)。
+    PanelShown(AppSlot),
+}
+
+/// 状态机要 `App` 去做的事。
+#[derive(Debug, Clone, PartialEq)]
+pub enum Effect {
+    FetchList,
+    FetchLaunchUrl(AppSlot),
+    StartApp(AppSlot),
+    StopApp(AppSlot),
+    /// 已安装集合变了(或第一次拿到):`App::sync_installed_apps`。
+    SyncRail(Vec<AppSlot>),
+    SetUrl(AppSlot, String),
+    ClearUrl(AppSlot),
+    Toast {
+        level: Level,
+        text: String,
+        key: String,
+    },
+}
+
+/// 一个应用在 dozerd 里的当前状况(只留面板要用的)。
+#[derive(Debug, Clone, PartialEq)]
+struct Row {
+    id: String,
+    name: String,
+    #[allow(dead_code)] // 目前面板只看观察态;期望态留给 A4c 的"开机自启"之类展示
+    desired: DesiredState,
+    observed: ObservedState,
+}
+
+/// 整个列表的状况(`State::phase` 为 `None` = 还没拿到过列表)。
+#[derive(Debug, Clone, PartialEq)]
+enum Phase {
+    Loaded,
+    /// 应用宿主本身不可用(原因给人看)。
+    Unavailable(String),
+    /// 最近一次请求在传输层失败;已知的应用保留。
+    Disconnected,
+}
+
+/// 面板该画什么(`State::view_model` 的结果)。
+#[derive(Debug, Clone, PartialEq)]
+pub enum PanelView {
+    /// 应用宿主不可用,带原因。
+    HostUnavailable(String),
+    /// 还没拿到列表。
+    Loading,
+    /// 列表里已经没有这个应用(刚被卸载)。
+    Missing,
+    Stopped,
+    /// 过渡态,文案给人看("启动中…"/"停止中…")。
+    Busy(&'static str),
+    /// 应用在跑,正在取启动地址。
+    Opening,
+    /// 取启动地址失败了(再点一次面板图标重试)。
+    OpenFailed,
+    /// 应用在跑且地址已就绪:webview 盖在面板上,这里只是它下面的底。
+    Running,
+    /// 应用崩溃/启动失败,带原因。
+    Crashed(String),
+}
+
+#[derive(Debug, Default)]
+pub struct State {
+    rows: HashMap<AppSlot, Row>,
+    order: Vec<AppSlot>,
+    phase: Option<Phase>,
+    in_flight: bool,
+    last_poll: Option<Instant>,
+    /// 正在取启动地址的应用。
+    launching: HashSet<AppSlot>,
+    /// 取启动地址失败过的应用(面板切入时清掉才会再试,避免每次轮询都重试)。
+    launch_failed: HashSet<AppSlot>,
+    /// 已经把启动地址交给 `AppViews` 的应用。
+    url_set: HashSet<AppSlot>,
+    acting: HashMap<AppSlot, Act>,
+}
+
+impl State {
+    /// 要不要排下一拍唤醒:从没拉过(启动后的第一次),或者有应用面板可见(持续轮询)。
+    pub fn poll_wanted(&self, app_panel_visible: bool) -> bool {
+        !self.in_flight && (self.last_poll.is_none() || app_panel_visible)
+    }
+
+    /// `ResumeTimeReached` 时调用:到点就拉一次列表。
+    pub fn poll_if_due(&mut self, now: Instant, visible: Option<AppSlot>) -> Vec<Effect> {
+        if self.in_flight {
+            return Vec::new();
+        }
+        let due = match self.last_poll {
+            None => true,
+            Some(t) => visible.is_some() && now.duration_since(t) >= POLL_INTERVAL,
+        };
+        if !due {
+            return Vec::new();
+        }
+        self.begin_fetch(now);
+        vec![Effect::FetchList]
+    }
+
+    fn begin_fetch(&mut self, now: Instant) {
+        self.in_flight = true;
+        self.last_poll = Some(now);
+    }
+
+    pub fn update(&mut self, msg: Message, now: Instant, visible: Option<AppSlot>) -> Vec<Effect> {
+        match msg {
+            Message::ListLoaded(result) => self.list_loaded(result, visible),
+            Message::LaunchUrlLoaded(slot, result) => self.launch_url_loaded(slot, result),
+            Message::Start(slot) => self.act(slot, Act::Start),
+            Message::Stop(slot) => self.act(slot, Act::Stop),
+            Message::ActionDone(slot, act, result) => self.action_done(slot, act, result, now),
+            Message::PanelShown(slot) => {
+                self.launch_failed.remove(&slot);
+                let mut effects = Vec::new();
+                if self.url_set.remove(&slot) {
+                    effects.push(Effect::ClearUrl(slot));
+                }
+                if !self.in_flight {
+                    self.begin_fetch(now);
+                    effects.push(Effect::FetchList);
+                }
+                effects
+            }
+        }
+    }
+
+    fn list_loaded(
+        &mut self,
+        result: Result<Vec<AppSummary>, Failure>,
+        visible: Option<AppSlot>,
+    ) -> Vec<Effect> {
+        self.in_flight = false;
+        let apps = match result {
+            Ok(apps) => apps,
+            Err(Failure::Host(f)) if f.kind == AppErrorKind::Unavailable => {
+                self.phase = Some(Phase::Unavailable(f.message));
+                return Vec::new();
+            }
+            Err(Failure::Host(f)) => {
+                // 其他类别的列表失败(理论上只有 Internal):保留已知应用,弹一条去重的 Toast。
+                return vec![Effect::Toast {
+                    level: Level::Warning,
+                    text: format!("读取应用列表失败:{}", f.message),
+                    key: "app_host:list".into(),
+                }];
+            }
+            Err(Failure::Transport(_)) => {
+                self.phase = Some(Phase::Disconnected);
+                return Vec::new();
+            }
+        };
+        self.phase = Some(Phase::Loaded);
+
+        let mut effects = Vec::new();
+        let mut rows = HashMap::new();
+        let mut order = Vec::new();
+        for app in apps {
+            // 形状非法的 id 不会出现(宿主已校验);万一槽表满了就跳过这个应用,不让整个列表失败。
+            let Some(slot) = AppSlot::intern(app.id.as_str()) else {
+                continue;
+            };
+            order.push(slot);
+            rows.insert(
+                slot,
+                Row {
+                    id: app.id.as_str().to_owned(),
+                    name: app.name,
+                    desired: app.desired,
+                    observed: app.observed,
+                },
+            );
+        }
+        // 消失的应用:撤掉它的地址与在途状态。
+        let gone: Vec<AppSlot> = self
+            .rows
+            .keys()
+            .filter(|s| !rows.contains_key(s))
+            .copied()
+            .collect();
+        for slot in gone {
+            self.forget(slot, &mut effects);
+        }
+        // 不再运行的应用:撤掉地址(webview 随之被池回收)。
+        for (slot, row) in &rows {
+            if row.observed != ObservedState::Running {
+                self.launching.remove(slot);
+                self.launch_failed.remove(slot);
+                if self.url_set.remove(slot) {
+                    effects.push(Effect::ClearUrl(*slot));
+                }
+            }
+        }
+        let set_changed = order != self.order;
+        self.rows = rows;
+        self.order = order;
+        if set_changed {
+            effects.push(Effect::SyncRail(self.order.clone()));
+        }
+        // 可见的应用在跑、还没有地址:去取一个。
+        if let Some(slot) = visible
+            && self
+                .rows
+                .get(&slot)
+                .is_some_and(|r| r.observed == ObservedState::Running)
+            && !self.url_set.contains(&slot)
+            && !self.launching.contains(&slot)
+            && !self.launch_failed.contains(&slot)
+        {
+            self.launching.insert(slot);
+            effects.push(Effect::FetchLaunchUrl(slot));
+        }
+        effects
+    }
+
+    fn forget(&mut self, slot: AppSlot, effects: &mut Vec<Effect>) {
+        self.launching.remove(&slot);
+        self.launch_failed.remove(&slot);
+        self.acting.remove(&slot);
+        if self.url_set.remove(&slot) {
+            effects.push(Effect::ClearUrl(slot));
+        }
+    }
+
+    fn launch_url_loaded(&mut self, slot: AppSlot, result: Result<String, Failure>) -> Vec<Effect> {
+        self.launching.remove(&slot);
+        let still_running = self
+            .rows
+            .get(&slot)
+            .is_some_and(|r| r.observed == ObservedState::Running);
+        match result {
+            Ok(url) if still_running => {
+                self.url_set.insert(slot);
+                vec![Effect::SetUrl(slot, url)]
+            }
+            // 应用在取地址期间停了:丢掉这个(已经没用的)地址。
+            Ok(_) => Vec::new(),
+            Err(failure) => {
+                self.launch_failed.insert(slot);
+                vec![Effect::Toast {
+                    level: Level::Error,
+                    text: format!("打开应用 {} 失败:{}", self.name(slot), failure.text()),
+                    key: format!("app_host:launch:{}", self.id(slot)),
+                }]
+            }
+        }
+    }
+
+    fn act(&mut self, slot: AppSlot, act: Act) -> Vec<Effect> {
+        if self.acting.contains_key(&slot) || !self.rows.contains_key(&slot) {
+            return Vec::new();
+        }
+        self.acting.insert(slot, act);
+        vec![match act {
+            Act::Start => Effect::StartApp(slot),
+            Act::Stop => Effect::StopApp(slot),
+        }]
+    }
+
+    fn action_done(
+        &mut self,
+        slot: AppSlot,
+        act: Act,
+        result: Result<(), Failure>,
+        now: Instant,
+    ) -> Vec<Effect> {
+        self.acting.remove(&slot);
+        let mut effects = Vec::new();
+        if let Err(failure) = result {
+            let verb = match act {
+                Act::Start => "启动",
+                Act::Stop => "停止",
+            };
+            effects.push(Effect::Toast {
+                level: Level::Error,
+                text: format!("{verb}应用 {} 失败:{}", self.name(slot), failure.text()),
+                key: format!("app_host:act:{}", self.id(slot)),
+            });
+        }
+        // 不管成败都立刻刷新一次,面板马上反映真实状态。
+        if !self.in_flight {
+            self.begin_fetch(now);
+            effects.push(Effect::FetchList);
+        }
+        effects
+    }
+
+    fn name(&self, slot: AppSlot) -> String {
+        self.rows
+            .get(&slot)
+            .map(|r| r.name.clone())
+            .unwrap_or_else(|| slot.id().to_owned())
+    }
+
+    fn id(&self, slot: AppSlot) -> &str {
+        self.rows
+            .get(&slot)
+            .map(|r| r.id.as_str())
+            .unwrap_or_else(|| slot.id())
+    }
+
+    /// 面板该画什么。
+    pub fn view_model(&self, slot: AppSlot) -> PanelView {
+        if let Some(Phase::Unavailable(reason)) = &self.phase {
+            return PanelView::HostUnavailable(reason.clone());
+        }
+        let Some(row) = self.rows.get(&slot) else {
+            return match self.phase {
+                None => PanelView::Loading,
+                Some(_) => PanelView::Missing,
+            };
+        };
+        match self.acting.get(&slot) {
+            Some(Act::Start) => return PanelView::Busy("启动中…"),
+            Some(Act::Stop) => return PanelView::Busy("停止中…"),
+            None => {}
+        }
+        match &row.observed {
+            ObservedState::Running => {
+                if self.url_set.contains(&slot) {
+                    PanelView::Running
+                } else if self.launch_failed.contains(&slot) {
+                    PanelView::OpenFailed
+                } else {
+                    PanelView::Opening
+                }
+            }
+            ObservedState::Preparing | ObservedState::Starting | ObservedState::Updating => {
+                PanelView::Busy("启动中…")
+            }
+            ObservedState::Stopping | ObservedState::Uninstalling => PanelView::Busy("停止中…"),
+            ObservedState::Failed { reason, .. } => PanelView::Crashed(reason.clone()),
+            ObservedState::Installed | ObservedState::Stopped | ObservedState::NotInstalled => {
+                PanelView::Stopped
+            }
+        }
+    }
+
+    /// 某应用的显示名(面板标题行用);未知时退回 id。
+    pub fn display_name(&self, slot: AppSlot) -> String {
+        self.name(slot)
+    }
+}
+
+#[cfg(test)]
+mod tests {
+    use super::*;
+    use bytehost_apps::id::{AppId, Version};
+
+    fn slot(id: &str) -> AppSlot {
+        AppSlot::intern(id).unwrap()
+    }
+
+    fn app(id: &str, observed: ObservedState) -> AppSummary {
+        AppSummary {
+            id: AppId::new(id).unwrap(),
+            name: format!("name-{id}"),
+            version: Version::new(1, 0, 0),
+            desired: DesiredState::Running,
+            observed,
+            url: None,
+        }
+    }
+
+    fn running(id: &str) -> AppSummary {
+        app(id, ObservedState::Running)
+    }
+
+    fn host_failure(kind: AppErrorKind, message: &str) -> Failure {
+        Failure::Host(AppFailure::new(kind, message))
+    }
+
+    fn loaded(state: &mut State, apps: Vec<AppSummary>, visible: Option<AppSlot>) -> Vec<Effect> {
+        state.update(Message::ListLoaded(Ok(apps)), Instant::now(), visible)
+    }
+
+    #[test]
+    fn the_first_poll_is_due_and_later_ones_only_while_an_app_panel_is_visible() {
+        let mut s = State::default();
+        let t0 = Instant::now();
+        assert!(s.poll_wanted(false), "启动后至少拉一次");
+        assert_eq!(s.poll_if_due(t0, None), vec![Effect::FetchList]);
+        assert!(s.poll_if_due(t0, None).is_empty(), "在途时不重复发");
+        s.update(Message::ListLoaded(Ok(vec![])), t0, None);
+        assert!(!s.poll_wanted(false), "没有应用面板可见就不再轮询");
+        assert!(s.poll_if_due(t0 + POLL_INTERVAL * 5, None).is_empty());
+        let a = Some(slot("hp-a"));
+        assert!(s.poll_wanted(true));
+        assert!(
+            s.poll_if_due(t0 + POLL_INTERVAL / 2, a).is_empty(),
+            "没到点"
+        );
+        assert_eq!(
+            s.poll_if_due(t0 + POLL_INTERVAL, a),
+            vec![Effect::FetchList]
+        );
+    }
+
+    #[test]
+    fn the_rail_is_synced_when_the_installed_set_changes_and_not_otherwise() {
+        let mut s = State::default();
+        let (a, b) = (slot("rail-sync-a"), slot("rail-sync-b"));
+        let first = loaded(
+            &mut s,
+            vec![app("rail-sync-a", ObservedState::Stopped)],
+            None,
+        );
+        assert_eq!(first, vec![Effect::SyncRail(vec![a])]);
+        assert!(
+            loaded(
+                &mut s,
+                vec![app("rail-sync-a", ObservedState::Stopped)],
+                None
+            )
+            .is_empty()
+        );
+        let both = loaded(
+            &mut s,
+            vec![
+                app("rail-sync-a", ObservedState::Stopped),
+                app("rail-sync-b", ObservedState::Stopped),
+            ],
+            None,
+        );
+        assert_eq!(both, vec![Effect::SyncRail(vec![a, b])]);
+        let removed = loaded(
+            &mut s,
+            vec![app("rail-sync-b", ObservedState::Stopped)],
+            None,
+        );
+        assert_eq!(removed, vec![Effect::SyncRail(vec![b])]);
+    }
+
+    #[test]
+    fn a_visible_running_app_gets_exactly_one_launch_url_fetch() {
+        let mut s = State::default();
+        let a = slot("launch-a");
+        let first = loaded(&mut s, vec![running("launch-a")], Some(a));
+        assert!(first.contains(&Effect::FetchLaunchUrl(a)));
+        let again = loaded(&mut s, vec![running("launch-a")], Some(a));
+        assert!(
+            !again.contains(&Effect::FetchLaunchUrl(a)),
+            "在途时不重复取"
+        );
+        assert_eq!(s.view_model(a), PanelView::Opening);
+        let url = "http://launch-a.localhost:20001/?bh_token=t".to_string();
+        let done = s.update(
+            Message::LaunchUrlLoaded(a, Ok(url.clone())),
+            Instant::now(),
+            Some(a),
+        );
+        assert_eq!(done, vec![Effect::SetUrl(a, url)]);
+        assert_eq!(s.view_model(a), PanelView::Running);
+        let later = loaded(&mut s, vec![running("launch-a")], Some(a));
+        assert!(later.is_empty(), "已有地址就不再取");
+    }
+
+    #[test]
+    fn a_running_app_whose_panel_is_not_visible_is_not_opened() {
+        let mut s = State::default();
+        let effects = loaded(&mut s, vec![running("hidden-a")], None);
+        assert!(
+            !effects
+                .iter()
+                .any(|e| matches!(e, Effect::FetchLaunchUrl(_)))
+        );
+    }
+
+    #[test]
+    fn a_launch_url_that_arrives_after_the_app_stopped_is_dropped() {
+        let mut s = State::default();
+        let a = slot("late-a");
+        loaded(&mut s, vec![running("late-a")], Some(a));
+        loaded(&mut s, vec![app("late-a", ObservedState::Stopped)], Some(a));
+        let done = s.update(
+            Message::LaunchUrlLoaded(a, Ok("http://late-a.localhost:1/".into())),
+            Instant::now(),
+            Some(a),
+        );
+        assert!(done.is_empty());
+        assert_eq!(s.view_model(a), PanelView::Stopped);
+    }
+
+    #[test]
+    fn a_failed_launch_toasts_once_and_is_not_retried_until_the_panel_is_shown_again() {
+        let mut s = State::default();
+        let a = slot("fail-a");
+        loaded(&mut s, vec![running("fail-a")], Some(a));
+        let failed = s.update(
+            Message::LaunchUrlLoaded(a, Err(host_failure(AppErrorKind::Conflict, "不在运行"))),
+            Instant::now(),
+            Some(a),
+        );
+        assert_eq!(failed.len(), 1);
+        match &failed[0] {
+            Effect::Toast { level, text, key } => {
+                assert_eq!(*level, Level::Error);
+                assert!(
+                    text.contains("name-fail-a") && text.contains("不在运行"),
+                    "{text}"
+                );
+                assert_eq!(key, "app_host:launch:fail-a");
+            }
+            other => panic!("{other:?}"),
+        }
+        assert_eq!(s.view_model(a), PanelView::OpenFailed);
+        let poll = loaded(&mut s, vec![running("fail-a")], Some(a));
+        assert!(
+            !poll.iter().any(|e| matches!(e, Effect::FetchLaunchUrl(_))),
+            "轮询不重试"
+        );
+        // 再次切入面板:清状态并重新拉列表,列表回来后再取地址。
+        let shown = s.update(Message::PanelShown(a), Instant::now(), Some(a));
+        assert_eq!(shown, vec![Effect::FetchList]);
+        let after = loaded(&mut s, vec![running("fail-a")], Some(a));
+        assert!(after.contains(&Effect::FetchLaunchUrl(a)));
+    }
+
+    #[test]
+    fn showing_the_panel_discards_the_old_address_so_a_fresh_one_is_fetched() {
+        let mut s = State::default();
+        let a = slot("shown-a");
+        loaded(&mut s, vec![running("shown-a")], Some(a));
+        s.update(
+            Message::LaunchUrlLoaded(a, Ok("http://shown-a.localhost:1/?bh_token=old".into())),
+            Instant::now(),
+            Some(a),
+        );
+        let shown = s.update(Message::PanelShown(a), Instant::now(), Some(a));
+        assert_eq!(shown, vec![Effect::ClearUrl(a), Effect::FetchList]);
+        assert_eq!(s.view_model(a), PanelView::Opening);
+        let again = s.update(Message::PanelShown(a), Instant::now(), Some(a));
+        assert!(again.is_empty(), "已有列表请求在途,不重复发");
+    }
+
+    #[test]
+    fn an_app_that_stops_or_disappears_loses_its_address() {
+        let mut s = State::default();
+        let (a, b) = (slot("drop-a"), slot("drop-b"));
+        loaded(&mut s, vec![running("drop-a"), running("drop-b")], Some(a));
+        for x in [a, b] {
+            s.update(
+                Message::LaunchUrlLoaded(x, Ok(format!("http://{}.localhost:1/", x.id()))),
+                Instant::now(),
+                Some(a),
+            );
+        }
+        let effects = loaded(&mut s, vec![app("drop-a", ObservedState::Stopped)], Some(a));
+        assert!(effects.contains(&Effect::ClearUrl(a)), "停了");
+        assert!(effects.contains(&Effect::ClearUrl(b)), "没了");
+        assert!(effects.contains(&Effect::SyncRail(vec![a])));
+        assert_eq!(s.view_model(b), PanelView::Missing);
+    }
+
+    #[test]
+    fn an_unavailable_host_is_a_persistent_panel_state_not_a_toast() {
+        let mut s = State::default();
+        let a = slot("unavail-a");
+        loaded(&mut s, vec![running("unavail-a")], None);
+        let effects = s.update(
+            Message::ListLoaded(Err(host_failure(AppErrorKind::Unavailable, "端口被占用"))),
+            Instant::now(),
+            None,
+        );
+        assert!(effects.is_empty());
+        assert_eq!(
+            s.view_model(a),
+            PanelView::HostUnavailable("端口被占用".into())
+        );
+        loaded(&mut s, vec![running("unavail-a")], None);
+        assert_eq!(s.view_model(a), PanelView::Opening, "恢复后回到正常状态");
+    }
+
+    #[test]
+    fn a_transport_failure_keeps_the_known_apps_and_stays_quiet() {
+        let mut s = State::default();
+        let a = slot("transport-a");
+        loaded(
+            &mut s,
+            vec![app("transport-a", ObservedState::Stopped)],
+            None,
+        );
+        let effects = s.update(
+            Message::ListLoaded(Err(Failure::Transport("连不上".into()))),
+            Instant::now(),
+            None,
+        );
+        assert!(effects.is_empty());
+        assert_eq!(s.view_model(a), PanelView::Stopped);
+    }
+
+    #[test]
+    fn start_and_stop_are_deduplicated_and_refresh_the_list_when_done() {
+        let mut s = State::default();
+        let a = slot("act-a");
+        loaded(&mut s, vec![app("act-a", ObservedState::Stopped)], None);
+        s.update(
+            Message::ListLoaded(Ok(vec![app("act-a", ObservedState::Stopped)])),
+            Instant::now(),
+            None,
+        );
+        assert_eq!(
+            s.update(Message::Start(a), Instant::now(), None),
+            vec![Effect::StartApp(a)]
+        );
+        assert!(s.update(Message::Start(a), Instant::now(), None).is_empty());
+        assert!(
+            s.update(Message::Stop(a), Instant::now(), None).is_empty(),
+            "在途时也不接受另一个动作"
+        );
+        assert_eq!(s.view_model(a), PanelView::Busy("启动中…"));
+        let done = s.update(
+            Message::ActionDone(a, Act::Start, Ok(())),
+            Instant::now(),
+            None,
+        );
+        assert_eq!(done, vec![Effect::FetchList]);
+    }
+
+    #[test]
+    fn a_failed_action_toasts_with_the_verb_and_still_refreshes() {
+        let mut s = State::default();
+        let a = slot("actfail-a");
+        loaded(&mut s, vec![app("actfail-a", ObservedState::Stopped)], None);
+        s.update(Message::Stop(a), Instant::now(), None);
+        let done = s.update(
+            Message::ActionDone(
+                a,
+                Act::Stop,
+                Err(host_failure(AppErrorKind::Conflict, "状态冲突")),
+            ),
+            Instant::now(),
+            None,
+        );
+        assert_eq!(done.len(), 2);
+        match &done[0] {
+            Effect::Toast { text, key, .. } => {
+                assert!(text.contains("停止") && text.contains("状态冲突"), "{text}");
+                assert_eq!(key, "app_host:act:actfail-a");
+            }
+            other => panic!("{other:?}"),
+        }
+        assert_eq!(done[1], Effect::FetchList);
+    }
+
+    #[test]
+    fn actions_on_unknown_apps_are_ignored() {
+        let mut s = State::default();
+        assert!(
+            s.update(Message::Start(slot("nobody-a")), Instant::now(), None)
+                .is_empty()
+        );
+    }
+
+    #[test]
+    fn the_view_model_covers_every_observed_state() {
+        let mut s = State::default();
+        let a = slot("vm-a");
+        let cases = [
+            (ObservedState::Installed, PanelView::Stopped),
+            (ObservedState::Stopped, PanelView::Stopped),
+            (ObservedState::Preparing, PanelView::Busy("启动中…")),
+            (ObservedState::Starting, PanelView::Busy("启动中…")),
+            (ObservedState::Stopping, PanelView::Busy("停止中…")),
+            (
+                ObservedState::Failed {
+                    reason: "崩了".into(),
+                    retryable: false,
+                },
+                PanelView::Crashed("崩了".into()),
+            ),
+            (ObservedState::Running, PanelView::Opening),
+        ];
+        for (observed, want) in cases {
+            loaded(&mut s, vec![app("vm-a", observed.clone())], None);
+            assert_eq!(s.view_model(a), want, "{observed:?}");
+        }
+        assert_eq!(State::default().view_model(a), PanelView::Loading);
+    }
+}
```

`Cargo.lock` 只多一行:

```diff
--- a/Cargo.lock
+++ b/Cargo.lock
@@ -1914,2 +1914,3 @@ dependencies = [
  "bytegit",
+ "bytehost-apps",
  "byteui",
```

- [ ] **Step 4: GREEN。** `cargo fmt -p dozer-app && cargo test -p dozer-app app_host` → 14 个全过。
- [ ] **Step 5: 变异检查(每条必须让对应测试 FAILED,再还原):** (a) `if set_changed {` 改成 `if true {`;(b) 删掉 `PanelShown` 里的 `ClearUrl`;(c) 删掉 `FetchLaunchUrl` 判断里的 `&& !self.launch_failed.contains(&slot)`;(d) `Unavailable` 分支改成同时返回一条 `Toast`。
- [ ] **Step 6: Commit。** `git commit -am "feat(dozer-app): app_host state machine for the app panel (A4b2 task 1)"`

### Task 2: `App` 接线(消息、效果执行、轮询、面板切入)

**Files:** Modify `crates/dozer-app/src/app/message.rs`、`app/app.rs`、`app/update.rs`、`platform/window_events.rs`

**Interfaces:**
- Consumes: Task 1 的 `State`/`Message`/`Effect`/`Failure`;A3 的 `App::sync_installed_apps`(去掉 `#[allow(dead_code)]`);A4b1 的 `AppViews::{set_url, clear}`(两处 `#[allow(dead_code)]` 现在有调用者了,A4b1 里留的 `allow` 一并去掉)。
- Produces: `Message::AppHost(app_host::Message)`、`App.app_host`、`App::{visible_app_slot, app_host_poll_wanted, poll_app_host_if_due, app_host_update}`、`run_app_host_effects`(私有)。

- [ ] **Step 1: 实现。**(这些是接线,逻辑都在 Task 1 已测的状态机里;`App` 需要 `Client`/事件循环,没有廉价夹具。)

```diff
diff --git a/crates/dozer-app/src/app/message.rs b/crates/dozer-app/src/app/message.rs
index 581ba9b1..9caba2de 100644
--- a/crates/dozer-app/src/app/message.rs
+++ b/crates/dozer-app/src/app/message.rs
@@ -128,4 +128,6 @@ pub enum Message {
     /// `extensions::todo::Message`。
     Todo(todo::Message),
+    /// 应用面板的宿主消息(见 `extensions::app_host`):列表/启动地址的异步结果与启停点击。
+    AppHost(crate::extensions::app_host::Message),
     /// 群聊面板的消息(见 `extensions::group_chat::Message`)。
     GroupChat(crate::extensions::group_chat::Message),
diff --git a/crates/dozer-app/src/app/app.rs b/crates/dozer-app/src/app/app.rs
index c2674e86..c742ccd6 100644
--- a/crates/dozer-app/src/app/app.rs
+++ b/crates/dozer-app/src/app/app.rs
@@ -468,4 +468,6 @@ pub struct App {
     /// 应用面板当前要加载的地址(bytehost A4b1,见 `app_webview`)。
     pub(crate) app_views: crate::app_webview::AppViews,
+    /// 已安装应用的列表轮询与每个应用面板的状态机(bytehost A4b2,见 `extensions::app_host`)。
+    pub(crate) app_host: crate::extensions::app_host::State,
     /// 数据库面板 App 级状态(哪些驱动类型在"新增数据源"下拉里可选,
     /// 启动时读盘)——见 `extensions::database::AppState`。
@@ -857,4 +859,5 @@ impl App {
             group_chat_webview: crate::extensions::group_chat::WebviewPushState::default(),
             app_views: crate::app_webview::AppViews::default(),
+            app_host: crate::extensions::app_host::State::default(),
             database: database::AppState::load(),
             footbar: footbar::AppState::default(),
@@ -1705,4 +1708,102 @@ impl App {
     }
 
+    /// 当前可见(未收起、未被另一侧放大盖住)的应用面板;没有返回 `None`。
+    pub(crate) fn visible_app_slot(&self) -> Option<AppSlot> {
+        let left = match self.left_view {
+            PanelKind::App(slot) if !self.left_collapsed => Some(slot),
+            _ => None,
+        };
+        let right = match self.right_view {
+            PanelKind::App(slot) if !self.right_collapsed => Some(slot),
+            _ => None,
+        };
+        match self.maximized {
+            Some(MaximizedPane::Left) => left,
+            Some(MaximizedPane::Right) => right,
+            None => left.or(right),
+        }
+    }
+
+    /// `about_to_wait` 是否要为应用宿主排下一拍唤醒(见 `app_host::State::poll_wanted`)。
+    pub fn app_host_poll_wanted(&self) -> bool {
+        self.app_host.poll_wanted(self.visible_app_slot().is_some())
+    }
+
+    /// `ResumeTimeReached` 时调用:到点就拉一次已安装应用列表。
+    pub fn poll_app_host_if_due(&mut self) {
+        let effects = self
+            .app_host
+            .poll_if_due(std::time::Instant::now(), self.visible_app_slot());
+        self.run_app_host_effects(effects);
+    }
+
+    /// 应用宿主状态机的消息入口(异步结果、点击、面板切入都走这里)。
+    pub(crate) fn app_host_update(&mut self, msg: crate::extensions::app_host::Message) {
+        let effects = self
+            .app_host
+            .update(msg, std::time::Instant::now(), self.visible_app_slot());
+        self.run_app_host_effects(effects);
+    }
+
+    /// 执行状态机吐出的副作用:发请求(结果经 `proxy` 回到 `Message::AppHost`)、同步 rail、写/清
+    /// 启动地址、弹 Toast。**启动地址是秘密,不写日志。**
+    fn run_app_host_effects(&mut self, effects: Vec<crate::extensions::app_host::Effect>) {
+        use crate::extensions::app_host::{Act, Effect, Failure, Message as M};
+        for effect in effects {
+            match effect {
+                Effect::FetchList => {
+                    let (client, proxy) = (self.client.clone(), self.proxy.clone());
+                    self.handle.spawn(async move {
+                        let result = client
+                            .app_list()
+                            .await
+                            .map_err(|e| Failure::from_client_error(&e));
+                        let _ = proxy.send_event(Message::AppHost(M::ListLoaded(result)));
+                    });
+                }
+                Effect::FetchLaunchUrl(slot) => {
+                    let Ok(id) = bytehost_apps::id::AppId::new(slot.id()) else {
+                        continue;
+                    };
+                    let (client, proxy) = (self.client.clone(), self.proxy.clone());
+                    self.handle.spawn(async move {
+                        let result = client
+                            .app_launch_url(id)
+                            .await
+                            .map_err(|e| Failure::from_client_error(&e));
+                        let _ =
+                            proxy.send_event(Message::AppHost(M::LaunchUrlLoaded(slot, result)));
+                    });
+                }
+                Effect::StartApp(slot) | Effect::StopApp(slot) => {
+                    let act = if matches!(effect, Effect::StartApp(_)) {
+                        Act::Start
+                    } else {
+                        Act::Stop
+                    };
+                    let Ok(id) = bytehost_apps::id::AppId::new(slot.id()) else {
+                        continue;
+                    };
+                    let (client, proxy) = (self.client.clone(), self.proxy.clone());
+                    self.handle.spawn(async move {
+                        let result = match act {
+                            Act::Start => client.app_start(id).await.map(|_url| ()),
+                            Act::Stop => client.app_stop(id).await,
+                        }
+                        .map_err(|e| Failure::from_client_error(&e));
+                        let _ =
+                            proxy.send_event(Message::AppHost(M::ActionDone(slot, act, result)));
+                    });
+                }
+                Effect::SyncRail(slots) => self.sync_installed_apps(&slots),
+                Effect::SetUrl(slot, url) => self.app_views.set_url(slot, url),
+                Effect::ClearUrl(slot) => self.app_views.clear(slot),
+                Effect::Toast { level, text, key } => {
+                    self.push_toast_keyed(LOG, level, text, &key);
+                }
+            }
+        }
+    }
+
     /// 设置某按钮的悬停目标（`true`=进入,`false`=离开）；动画由
     /// `advance_hover_anims` 循环把它指数逼近（见 `HoverAnim`）。
@@ -2565,5 +2666,4 @@ impl App {
     /// 当前正显示着已消失应用的那一侧退到该栏第一个面板。有改动才存盘。A3 里还没有调用者——A4 在拿到
     /// dozerd 的应用列表后调它。
-    #[allow(dead_code)]
     pub(crate) fn sync_installed_apps(&mut self, installed: &[AppSlot]) {
         let changed = self
diff --git a/crates/dozer-app/src/app/update.rs b/crates/dozer-app/src/app/update.rs
index 942a3afa..6bf21faf 100644
--- a/crates/dozer-app/src/app/update.rs
+++ b/crates/dozer-app/src/app/update.rs
@@ -1918,4 +1918,5 @@ impl App {
                 self.group_chat_webview.clear_failed();
             }
+            Message::AppHost(msg) => self.app_host_update(msg),
             Message::GroupChat(msg) => {
                 let project_id = msg.project_id();
@@ -5210,5 +5211,8 @@ impl App {
                 crate::extensions::group_chat::on_activate(&mut ws.group_chat);
             }),
-            PanelKind::Files | PanelKind::Web | PanelKind::Agent | PanelKind::App(_) => {}
+            PanelKind::App(slot) => {
+                self.app_host_update(crate::extensions::app_host::Message::PanelShown(slot));
+            }
+            PanelKind::Files | PanelKind::Web | PanelKind::Agent => {}
         }
     }
diff --git a/crates/dozer-app/src/platform/window_events.rs b/crates/dozer-app/src/platform/window_events.rs
index a24b346f..ef023677 100644
--- a/crates/dozer-app/src/platform/window_events.rs
+++ b/crates/dozer-app/src/platform/window_events.rs
@@ -2648,4 +2648,6 @@ impl winit::application::ApplicationHandler<Message> for Runner {
             // 轮询 `dozerd`;没有发言时不空转(`group_chat_poll_wanted`)。
             app.poll_group_chat_if_active();
+            // 已安装应用列表:启动后拉一次;应用面板可见时按 `app_host::POLL_INTERVAL` 持续拉。
+            app.poll_app_host_if_due();
             // 新增任务闪光倒计时:到点且用户未手动改选就自动清除选中高亮
             // (每次都调,内部按 `until` 自己短路,不再显式判断 `flash_active`)。
@@ -2688,5 +2690,5 @@ impl winit::application::ApplicationHandler<Message> for Runner {
             let next_drag_expand = app.next_drag_hover_expand_wake();
             let next_toast = app.next_toast_wake();
-            let wakes: [(bool, Duration); 8] = [
+            let wakes: [(bool, Duration); 9] = [
                 (
                     app.any_hover_anim_active(),
@@ -2698,4 +2700,8 @@ impl winit::application::ApplicationHandler<Message> for Runner {
                     crate::extensions::group_chat::POLL_INTERVAL,
                 ),
+                (
+                    app.app_host_poll_wanted(),
+                    crate::extensions::app_host::POLL_INTERVAL,
+                ),
                 (app.dragging_tab().is_some(), DRAG_REDRAW_INTERVAL),
                 (
```

注意 `window_events.rs` 里 `wakes` 数组长度由 8 改 9。
- [ ] **Step 2: 静态核对。** `FetchLaunchUrl`/`SetUrl` 的地址没有出现在任何 `log_*!`/`push_toast*` 的参数里;`Effect::Toast` 的文案不含地址;`FetchList` 的任务只经 `proxy` 回 `Message::AppHost`。
- [ ] **Step 3: 编译 + 既有测试。** `cargo fmt -p dozer-app && cargo test -p dozer-app` → 仅 `extensions::files::tests::delete_confirm_spec_reflects_pending_target`(已在 main 上失败)红。
- [ ] **Step 4: Commit。** `git commit -am "feat(dozer-app): wire the app host state machine into App (A4b2 task 2)"`

### Task 3: 面板内容 + 文档 + 手工验收

**Files:** Modify `crates/dozer-app/src/app/view.rs`;文档(规格 A4 行、`CLAUDE.md`、本计划的"执行后修订")

**Interfaces:**
- Consumes: Task 1 的 `view_model`/`display_name`/`PanelView`;Task 2 的 `Message::AppHost`。
- Produces: `app_panel_pane`(替换 A3 的 `app_placeholder_pane`)。

- [ ] **Step 1: 实现。**

```diff
diff --git a/crates/dozer-app/src/app/view.rs b/crates/dozer-app/src/app/view.rs
index f74fa2f4..7ab52907 100644
--- a/crates/dozer-app/src/app/view.rs
+++ b/crates/dozer-app/src/app/view.rs
@@ -1134,26 +1134,72 @@ pub(crate) fn panel_body<'a>(
             }
         }
-        PanelKind::App(slot) => app_placeholder_pane(slot, zone_pane_border(zone, PaneCorner::All)),
+        PanelKind::App(slot) => app_panel_pane(app, slot, zone_pane_border(zone, PaneCorner::All)),
     }
 }
 
-/// 应用面板的占位内容(bytehost A3):rail 条目与切换已经通了,应用自己的 wry 视图由 A4 接入。
-/// 只显示应用 id,不挂任何 webview(矩形恒为空,见 `webview_geometry`)。
-fn app_placeholder_pane<'a>(
+/// 应用面板的内容(bytehost A4b2):按 `app_host::State::view_model` 画应用当前状况与可做的操作。
+/// 应用在跑且地址就绪时 wry webview 盖在整个面板上(见 `webview_geometry`),这里只是它下面的底;
+/// 其余状态(未运行/启动中/打开失败/宿主不可用/崩溃)靠这里告诉用户发生了什么。
+fn app_panel_pane<'a>(
+    app: &'a App,
     slot: AppSlot,
     border: Border,
 ) -> Element<'a, Message, iced_widget::Theme, iced_renderer::Renderer> {
-    container(
-        text(format!("应用 {}", slot.id()))
+    use crate::extensions::app_host::{Message as M, PanelView as V};
+    let colors = byteui::theme::color::current();
+    let on_press = |m: M| Message::AppHost(m);
+    let (headline, detail, action): (String, Option<String>, Option<(&'static str, M)>) =
+        match app.app_host.view_model(slot) {
+            V::HostUnavailable(reason) => ("应用宿主不可用".into(), Some(reason), None),
+            V::Loading => ("正在读取应用状态…".into(), None, None),
+            V::Missing => ("这个应用已不在已安装列表里".into(), None, None),
+            V::Stopped => ("应用未运行".into(), None, Some(("启动", M::Start(slot)))),
+            V::Busy(text) => (text.into(), None, None),
+            V::Opening => ("正在打开…".into(), None, None),
+            V::OpenFailed => (
+                "打开失败".into(),
+                Some("再次点击侧栏图标重试,或先停止应用".into()),
+                Some(("停止", M::Stop(slot))),
+            ),
+            V::Running => ("应用运行中".into(), None, None),
+            V::Crashed(reason) => (
+                "应用已退出".into(),
+                Some(reason),
+                Some(("重新启动", M::Start(slot))),
+            ),
+        };
+    let mut body = column![
+        text(app.app_host.display_name(slot))
             .size(byteui::theme::font::body())
-            .color(byteui::theme::color::current().dim),
-    )
-    .center(Length::Fill)
-    .style(move |_t: &iced_widget::Theme| container::Style {
-        background: Some(byteui::theme::color::current().panel.into()),
-        border,
-        ..container::Style::default()
-    })
-    .into()
+            .color(colors.cream),
+        text(headline)
+            .size(byteui::theme::font::body())
+            .color(colors.dim),
+    ]
+    .spacing(8)
+    .align_x(iced_widget::core::alignment::Horizontal::Center);
+    if let Some(detail) = detail {
+        body = body.push(
+            text(detail)
+                .size(byteui::theme::font::body())
+                .color(colors.dim),
+        );
+    }
+    if let Some((label, msg)) = action {
+        body = body.push(
+            iced_widget::button(text(label).size(byteui::theme::font::body()))
+                .padding([4, 12])
+                .on_press(on_press(msg))
+                .style(byteui::feedback::dialog::action_button_style(colors.gold)),
+        );
+    }
+    container(body)
+        .center(Length::Fill)
+        .style(move |_t: &iced_widget::Theme| container::Style {
+            background: Some(byteui::theme::color::current().panel.into()),
+            border,
+            ..container::Style::default()
+        })
+        .into()
 }
```

- [ ] **Step 2: 全量门禁。** `cargo fmt --check -p dozer-app && bash scripts/check-log-scope.sh && bash scripts/check-bytehost-apps-deps.sh && cargo test -p dozer-app && cargo clippy -p dozer-app --all-targets` → 测试同上(1896 通过 + 那 1 个既有失败);clippy 里没有 `app_host.rs`/`app/view.rs`/`app/app.rs` 的新警告(其余为存量)。
- [ ] **Step 3: 手工验收(需要真实 GUI + dozerd,**不能**自动化;结果写进最终汇报)。** 用 `dozer-client` 或一个临时脚本装一个静态应用(`manifest.toml` + 页面,含外链、`window.open`、`<iframe src="about:blank">`),然后:
  1. 启动 Dozer:图标栏出现该应用条目(左栏末尾),tooltip 是应用 id;
  2. 点它:应用未运行 → 面板显示「应用未运行」+「启动」,点「启动」→ 「启动中…」→ 「正在打开…」→ 页面加载,键盘焦点在页面里;
  3. A4b1 的冒烟项:点外链被拒(日志有"应用尝试离开自己的 origin")、`window.open` 无反应、`about:blank` iframe 正常、页面里连发 `ipc.postMessage('focus')` **不会**抢走终端键盘;
  4. 切到别的面板再切回:页面重新加载(状态丢失是已知局限),不会停在过期地址;
  5. 外部 `app_stop`:面板回到「应用未运行」;`app_uninstall`:图标栏条目在 2 秒内消失,左右栏的当前视图退到该栏第一个面板;
  6. 停掉 dozerd:应用面板显示"应用宿主不可用"并带原因(如端口占用);顶栏 dozerd 徽标照常;
  7. 重启 dozerd:应用按 `desired` 恢复,面板自动回到可用。
- [ ] **Step 4: Commit + 文档。** `git commit -am "feat(dozer-app): app panel content driven by the app_host view model (A4b2 task 3)"`;同一提交里:规格 A4 行补"A4b2 已完成:宿主逻辑(轮询/状态机/启停),见 `plans/2026-10-05-bytehost-a4b2-app-host-logic.md`;**A4c 剩:安装/卸载/停止运行中应用的界面、Settings 运行时探测展示**";`CLAUDE.md` 的 bytehost-apps 行补一句"应用面板宿主逻辑在 `extensions/app_host.rs`(纯状态机 + `App::run_app_host_effects`)"。

## 已知局限

- **运行中的应用在 GUI 里没有"停止"入口**(webview 盖住整个面板,按钮在下面):只有「打开失败」状态才露出「停止」。停止/卸载/安装属于 A4c 的应用管理界面。
- **切面板/收起/回首页/切项目/无活动项目时 webview 被销毁重载,页面内状态丢失**(评审 I2,有意保留 A4b1 的池语义)。每次切入都重新取启动地址,所以不会拿到过期地址;要保状态就得让应用 webview 隐藏常驻——另做。
- 同一个槽的 origin 变化(用户改了固定端口)时旧 webview 持有旧 origin 的导航策略会拒绝新地址(评审 M1);端口一期固定,暂不处理。
- 应用面板在首页(`AppPage::Home`)与没有活动工作区时不显示 webview(沿用 `preview_desired` 的入口判断);但列表轮询与图标栏同步不受影响。
- 图标栏图标仍是 `LayoutList`,tooltip 是应用 id 而不是显示名(`panel_meta` 返回 `&'static str`);显示名只在面板标题行。
- 取启动地址失败的应用,面板显示「打开失败」,需用户再次点图标栏才会重试(避免轮询刷屏)。
