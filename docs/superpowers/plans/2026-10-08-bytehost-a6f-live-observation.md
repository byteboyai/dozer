# bytehost A6f:状态推送、依赖安装失败页、实时日志 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 GUI 即时、准确地"看见"进程型应用:dozerd 把应用状态变化推给 GUI(取代 2s 轮询),依赖安装失败有专门的问题页并带安装输出,日志查看器自动刷新(崩溃页与设置页里正在运行的应用都能用)。

**Architecture:** 推送走 dozerd 现有的"同一连接上先应答、再流式推送"模式(`Attach` 的做法):新 `AppRequest::Subscribe` 之后该连接只往外写 `AppReply::Changed{app}`/`Resync`,**只当"失效信号"**,GUI 收到就按现有 `List` 重拉(合并在途请求),不在推送里复制状态——状态机仍是 `List` 结果的纯函数。轮询保留为兜底(订阅正常时降到 30s,订阅断开时回到 2s 并按退避重订阅)。依赖安装失败经新增 `Transitions::install_failed` 记 `AppIssue::DependencyInstall`,问题页复用 A6e 的 `RuntimeIssue` 通路。日志查看器抽成共享小状态机 `app_logs.rs`,`app_host` 与 `settings_apps` 共用。

**Tech Stack:** Rust、tokio `broadcast`、iced 0.14(纯状态机 + `Effect`)、serde(wire 只追加)。

**Spec:** `docs/superpowers/specs/2026-10-04-bytehost-app-host-design.md`(§6.3 提示页由 host 提供;§7 A2 备注"推送事件本切片没做,GUI 靠轮询")。前序:`plans/2026-10-08-bytehost-a6e-process-app-experience.md`、验收报告 `specs/2026-10-08-bytehost-a6e-acceptance-report.md`。

**范围说明:** 用户为"A6f"选了四项——推送、依赖失败页 + 实时日志、应用升级与回滚、非本机来源——其中后两项与前两项互相独立、各自要先做设计决策(升级的审批语义、来源的信任等级),合进一份计划无法分任务评审。**本计划只含前两项;升级与回滚 = A6g、非本机来源 = A6h,各自单独成计划。容器运行时按用户裁决继续推迟。**

## Global Constraints

- `bytehost-apps` 默认 feature 依赖只能是 serde 家族;`scripts/check-bytehost-apps-deps.sh` 与 `scripts/check-log-scope.sh` 必须通过。
- wire 协议**只追加**:旧形状 JSON 解析不得失败(新字段 `#[serde(default)]`,新变体只加不改)。
- **持久状态不进 Toast**(问题页、日志查看器的错误都留在页面里);瞬时失败(如订阅重连失败)只写日志,不弹 Toast。
- 日志统一走 `dozer_core::log_*!`;`dozerd`/`dozer-app` 里禁止裸 `tracing::*!`/`eprintln!`。面板日志来源名必须是 `PanelKind` 对应名(应用宿主逻辑属 `module` 来源)。
- **应用日志内容、安装输出不写进 dozerd 日志**,也不经推送事件传输(推送只带 `AppId`)。
- 字体:日志查看器是普通文本场景,用系统默认字体(`Font::default()`),不得用等宽代码字体。
- 监管线程回写状态必须持锁并核对取消标志、拿锁用尝试循环(见 `supervisor.rs`/`Core::try_guard`);`AppTransitions` 的新方法照此写。
- 新增函数参数 ≥7 个且有相邻同类型参数时用具名字段结构体,不要 `#[allow(clippy::too_many_arguments)]`。
- 提交前 `git diff --cached --stat` 看全貌,只 `git add` 指定路径;不动工作区里已有的 `Cargo.lock` 改动。
- 变异验证:每个修复/新行为的测试,都要临时去掉实现确认测试会失败,再恢复。

## Review Focus

(spec 与 A6e 隐含、但没有任务测试覆盖的输入或失败模式,最可能咬到真实用户的排前面。每条在对应任务里有测试。)

1. **订阅连接断开/dozerd 重启**:GUI 必须回到 2s 轮询并自动重订阅,不能停在"已订阅"的假象里看着陈旧状态(Task 2)。
2. **推送风暴**:一个应用在重启退避里每秒多次改状态,GUI 的 `List` 请求必须合并(在途时不重发),不能每个事件发一次(Task 2)。
3. **广播落后(`Lagged`)**:慢消费者丢了事件时必须收到 `Resync` 并整体重拉,不是静默漏掉(Task 1)。
4. **应用已 `Failed` 但旧日志查看器还开着**:应用重启成功后查看器必须复位;迟到的日志结果不得复活它(A6e 已修一处,Task 4 的刷新路径不得把它改坏)。
5. **依赖安装输出里有 ANSI 进度条与超长行**:问题页和日志里不得出现转义字符,单行封顶(复用 `logs::sanitize`,Task 3/4)。
6. **日志文件被轮转(`app.log` → `app.log.1`)的瞬间自动刷新**:不得报错或显示空白闪烁(Task 4)。

## File Structure

| 文件 | 变更 | 职责 |
|---|---|---|
| `crates/bytehost-apps/src/proto.rs` | 改 | `AppRequest::Subscribe`、`AppReply::{Subscribed, Changed{app}, Resync}`、`AppIssue::DependencyInstall` |
| `crates/bytehost-apps/src/supervisor.rs` | 改 | `Transitions::install_failed`;`run` 在依赖安装失败时调它 |
| `crates/bytehost-apps/src/manager.rs` | 改 | `AppTransitions::install_failed` 记 issue + 置 `Failed`;`Core::subscribe_changes()` |
| `crates/dozerd/src/app_service.rs` | 改 | `AppService::subscribe()` |
| `crates/dozerd/src/server.rs` | 改 | 连接级 `app_sub`:`Subscribe` 后把事件写成 `Changed`/`Resync` |
| `crates/dozer-client/src/lib.rs` | 改 | `Client::app_subscribe() -> mpsc::UnboundedReceiver<AppChange>` |
| `crates/dozer-app/src/extensions/app_logs.rs` | **新** | 共享日志查看器状态机(`LogsState`):展开/收起/加载/自动刷新/结果接收规则 |
| `crates/dozer-app/src/extensions/app_host.rs` | 改 | 订阅状态、变更合并、兜底轮询间隔;依赖失败问题页;改用 `app_logs` |
| `crates/dozer-app/src/extensions/settings_apps.rs` | 改 | 每行"日志"入口(正在运行的应用也能看),用 `app_logs` |
| `crates/dozer-app/src/app/app.rs`、`app/view.rs` | 改 | 执行 `Subscribe`/`FetchLogs` 副作用;画依赖失败页与设置页日志区 |
| `docs/superpowers/specs/2026-10-08-bytehost-a6f-acceptance-report.md` | **新** | 验收报告(只填实际运行结果) |

---

### Task 1: wire 与 dozerd 侧推送

**Files:** Modify `proto.rs`、`manager.rs`(仅 `subscribe_changes`)、`app_service.rs`、`server.rs`;Test: `proto.rs` 的 `mod tests`、`crates/dozerd/tests/app_requests.rs`。

**Interfaces:**
- Produces(`proto.rs`):
  ```rust
  // AppRequest 追加
  Subscribe,
  // AppReply 追加
  /// 订阅成功;此后该连接只会收到 Changed / Resync。
  Subscribed,
  /// 某应用的状态/端点/问题变了。**只是失效信号**,不带状态——收到后重拉 `List`。
  Changed { app: AppId },
  /// 广播落后丢了事件,或服务刚恢复:整体重拉。
  Resync,
  ```
- Produces(`AppService`):`pub fn subscribe(&self) -> Option<tokio::sync::broadcast::Receiver<AppEvent>>`(`Unavailable` 状态返回 `None`)。
- Consumes:`AppManager::events()`(已有)。

- [x] **Step 1: 写失败测试**
  - `proto.rs`:`Subscribe` → `{"op":"subscribe"}`;`Subscribed`/`Resync`/`Changed{app}` → `{"reply":"subscribed"}`、`{"reply":"resync"}`、`{"reply":"changed","app":"excalidraw"}`;旧 `AppReply` JSON 仍能解析(已有用例不改)。
  - `app_requests.rs`(沿用 `start_daemon(apps)`):直接用 `UnixStream` 连 daemon 发 `Request::App{Subscribe}`,第一行是 `Reply::App{Subscribed}`;之后安装一个静态应用 → 同一连接收到 `Changed{app}`(超时 5s 失败);**同一连接**之后再发普通 `List` 请求仍得到应答(订阅不独占连接的读侧)。
  - 不可用的 `AppService::unavailable("x")`:`Subscribe` 得到 `Failed{kind: Unavailable}`,连接不挂。
  - 落后:用 `AppService` 级单测,把广播容量灌满后消费者收到 `Lagged`;服务端转发函数(见 Step 3)对 `Lagged` 产出 `Resync`——把"事件 → 应答行"的转换抽成纯函数 `fn change_reply(Result<AppEvent, RecvError>) -> Option<AppReply>` 单测:`Ok(e)` → `Changed{e.app()}`;`Lagged(_)` → `Resync`;`Closed` → `None`。
- [x] **Step 2: 确认失败 → Step 3: 实现**:
  - `server.rs` 连接循环加 `let mut app_sub: Option<broadcast::Receiver<AppEvent>> = None;`,`Request::App{request: AppRequest::Subscribe}` 在进入 `apps.handle` **之前**拦截(`handle` 保持"一问一答",不要把流塞进去):`apps.subscribe()` 为 `Some(rx)` → `app_sub = Some(rx)` 并回 `Subscribed`;`None` → 回 `Failed{Unavailable}`。`select!` 增加一个 `if app_sub.is_some()` 的分支,调 `change_reply` 写出;`Closed` 时 `app_sub = None`。重复 `Subscribe` 覆盖旧订阅(不累加)。
  - `Progress` 事件也转成 `Changed`(GUI 重拉很便宜;不在推送里区分事件种类,避免两套真相)。
- [x] **Step 4: 通过** — `cargo test -p bytehost-apps --all-features proto`、`cargo test -p dozerd --test app_requests`、`cargo test -p dozerd app_service`。变异:删掉 `Lagged → Resync` 分支,纯函数单测失败。
- [x] **Step 5: Commit** — `feat(bytehost-apps,dozerd): push app changes as invalidation signals over a subscribed connection (A6f task 1)`。

---

### Task 2: Client 订阅 + GUI 消费(推送取代轮询,轮询降为兜底)

**Files:** Modify `crates/dozer-client/src/lib.rs`、`extensions/app_host.rs`、`app/app.rs`;Test: `app_host.rs` 的 `mod tests`、`dozer-client` 的测试。

**Interfaces:**
- Produces(`dozer-client`):
  ```rust
  pub enum AppChange { Changed(AppId), Resync, Disconnected }
  impl Client {
      /// 订阅应用变更。第一行应答确认后返回事件流;连接断开时流里最后一条是 `Disconnected`,随后关闭。
      pub async fn app_subscribe(&self) -> Result<mpsc::UnboundedReceiver<AppChange>>;
  }
  ```
  读循环模仿 `attach`:`tx.closed()` 时退出并关闭连接,避免任务与 fd 滞留。
- Produces(`app_host`):
  ```rust
  pub const SAFETY_POLL_INTERVAL: Duration = Duration::from_secs(30);   // 已订阅时的兜底
  pub const RESUBSCRIBE_BACKOFF: [Duration; 4] = [2s, 5s, 15s, 30s];     // 重订阅退避,封顶 30s
  pub enum Message { /* 追加 */ Subscribed, Changed, SubscriptionLost }
  pub enum Effect  { /* 追加 */ Subscribe }
  ```
- `State` 追加私有字段 `subscription: Subscription { Idle, Pending, Live, Backoff{ attempt: usize, retry_at: Instant } }`。

**行为(状态机,纯函数,全部可测):**
- 首次/需要时发 `Effect::Subscribe` 一次(`Idle` → `Pending`);`Subscribed` → `Live`;`SubscriptionLost` → `Backoff{attempt: 0, ..}`,每次重订阅失败 attempt+1(封顶)。
- `Live` 时:`Changed` → 若无在途 `List` 则发 `FetchList`,有则记 `dirty = true`,在途 `List` 回来后若 `dirty` 再发**一次**(合并风暴);`poll_if_due` 间隔用 `SAFETY_POLL_INTERVAL`。
- 非 `Live`(`Idle`/`Pending`/`Backoff`):`poll_if_due` 用原来的 `POLL_INTERVAL`(2s),行为与 A6e 完全一致(现有测试不改一行就应通过)。
- `Resync` 等同 `Changed` 但不依赖具体应用。
- `disconnected()`(传输层失败)与订阅无关,保持原逻辑。

- [x] **Step 1: 写失败测试**(`app_host.rs`,沿用 `loaded(...)` fixture 风格)
  - `a_live_subscription_slows_polling_to_the_safety_interval`:`Subscribed` 后 `poll_if_due(t0+POLL_INTERVAL)` 为空,`t0+SAFETY_POLL_INTERVAL` 才发 `FetchList`。
  - `a_change_triggers_exactly_one_refetch_even_in_a_storm`:`Live`、无在途;连发 10 次 `Changed` → 只有第 1 次产出 `FetchList`;`ListLoaded` 回来后恰好再产出 1 次(因 `dirty`),再回来不再发。
  - `losing_the_subscription_goes_back_to_fast_polling_and_retries_with_backoff`:`SubscriptionLost` 后 `poll_if_due(t0+POLL_INTERVAL)` 发 `FetchList`;`retry_at` 前不发 `Subscribe`,之后发一次;连续 5 次失败后退避停在 30s。
  - `subscribe_is_requested_once_until_it_resolves`:`Pending` 时重复调用不重发。
  - `dozerd_restart_is_recovered`:`Live` → `SubscriptionLost` → 退避到期 → `Subscribe` → `Subscribed` → 恢复 `Live`,期间没有丢失 `Changed` 后的重拉(`Resync` 被 `Subscribed` 之后立刻补发一次 `FetchList`,因为断线期间可能漏了事件)。
  - `dozer-client`:对一个本地 `UnixListener` 假服务端,发 `Subscribed` + 两条 `Changed` + 关闭 → 收到 `Changed, Changed, Disconnected`;丢弃接收端后读任务退出(用 `tx.closed()` 的同款断言)。
- [x] **Step 2: 确认失败 → Step 3: 实现**;`app.rs::run_app_host_effects` 增 `Effect::Subscribe`:`client.app_subscribe().await`,成功则发 `M::Subscribed` 并 `spawn` 转发循环把 `AppChange` 映射成 `M::Changed`/`M::SubscriptionLost`(经 `proxy.send_event(Message::AppHost(..))`);失败直接 `M::SubscriptionLost`。失败只写日志(`module` 来源),**不弹 Toast**。
- [x] **Step 4: 通过** — `cargo test -p dozer-app app_host`(34 passed)、`cargo test -p dozer-client`(7 passed);`cargo clippy -p dozer-app -p dozer-client --all-targets` 无新警告。变异:去掉 `dirty` 合并,风暴用例失败;让 `Live` 仍用 2s,第一个用例失败(均已实测确认)。
- [ ] **Step 5: 手动验收**(需要 GUI;结果如实写进报告 §2,未做的不勾):装 `py-notes` → 点它的 `/crash` → 面板应在 <1s 内进入"启动中…"而不是等下一次 2s 轮询;`kill -9 $(pgrep dozerd)` 后 2s 内显示 dozerd 不可用,重启 dozerd 后自动恢复订阅(日志里能看到重订阅)。
- [x] **Step 6: Commit** — `feat(dozer-client,dozer-app): subscribe to app changes; polling becomes a safety net (A6f task 2)`。

---

### Task 3: 依赖安装失败的问题页

**Files:** Modify `proto.rs`(`AppIssue`)、`supervisor.rs`、`manager.rs`、`app_host.rs`(`issue_texts`)、`app/view.rs`;Test: 同文件 `mod tests`、`crates/dozerd/tests/process_apps_live.rs`(新增一个 `#[ignore]` 用例)。

**Interfaces:**
- Produces:
  ```rust
  // proto.rs  AppIssue 追加
  /// 依赖安装(npm ci / uv sync 等)失败;`summary` 是一行人类可读的原因(不含安装输出,输出在日志里)。
  DependencyInstall { summary: String },
  // supervisor.rs  Transitions 追加(默认实现不提供,所有实现者都要写)
  fn install_failed(&self, summary: String) -> bool;
  ```
- `run` 里 `InstallOutcome::Failed(why)` 改调 `tr.install_failed(why)`(原来是 `tr.failed(why, true)`);`install_failed` 在 `AppTransitions` 里:持锁 + 核对取消 → `set_issue(DependencyInstall{summary})` → 与 `failed(reason, true)` 同样撤站点、置 `Failed{retryable: true}`。**顺序:先 `set_issue` 再 `set_observed`**,让 GUI 因 `Changed` 重拉时一定看得到 issue(推送与 issue 的竞态)。
- `issue_texts(DependencyInstall{summary})` → 标题"依赖安装失败",详情 = `summary` + "查看日志了解安装输出"。问题页对该 issue 的动作:「查看日志」(展开日志,Task 4 的查看器)、「重试」。**没有"去设置安装"**——这不是运行时缺失。

- [ ] **Step 1: 写失败测试**
  - `proto.rs`:`DependencyInstall` 序列化形状 `{"issue":"dependency_install","summary":"..."}`;旧 JSON 解析不受影响。
  - `supervisor.rs`:安装命令退出码非 0 → 记录到的 `Transitions` 调用是 `install_failed`,**不是** `failed`;超时、起不来、写标记失败同样走 `install_failed`(四个分支各一个断言,用 `python3 -c "import sys; sys.exit(3)"` 之类的真命令,缺 python3 则 `return`)。
  - `manager.rs`:依赖安装失败的应用 → `list()` 里 `observed == Failed{retryable:true}` 且 `issue == Some(DependencyInstall{..})`;重试成功后(换掉会失败的命令)`issue` 清除;`issue` 在应用 `Running` 时不带出(A6e 规则不变)。
  - `app_host.rs`:`Failed` + `DependencyInstall` → `PanelView::RuntimeIssue(..)`;`issue_texts` 表驱动。
  - Review Focus 5:安装命令输出含 `\x1b[32m` 进度条与 6000 字符的单行 → 通过 `Logs` 读回时已清洗、封顶(这条放进 Task 4 的用例,这里只确认 `summary` 本身不含输出)。
- [ ] **Step 2: 确认失败 → Step 3: 实现 → Step 4: 通过**(`cargo test -p bytehost-apps --all-features`;所有 `Transitions` 实现者——生产的 `AppTransitions` 与测试里的 `Rec`——都要补方法,编译器会指出)。变异:把 `install_failed` 改回只调 `failed`,manager 用例失败。
- [ ] **Step 5: live 用例**(`process_apps_live.rs`,`#[ignore]`):装一个 `command = ["python3","server.py"]` 且声明了一个**必然失败的依赖安装**的应用(node 样例:`package-lock.json` 引用不存在的包;python 样例用 `uv.lock` 需要 uv,若无 uv 则只跑 node 版)→ `Start` 后 `list` 里出现 `Failed` + `DependencyInstall`,`Logs` 含安装器的错误输出。如实记录运行结果。
- [ ] **Step 6: Commit** — `feat(bytehost-apps,dozer-app): dependency-install failures get their own issue page (A6f task 3)`。

---

### Task 4: 实时日志(共享查看器 + 设置页入口)

**Files:** Create `crates/dozer-app/src/extensions/app_logs.rs`;Modify `app_host.rs`(迁移到共享状态机)、`settings_apps.rs`、`app/app.rs`、`app/view.rs`;Test: `app_logs.rs` 的 `mod tests`、`app_host.rs`/`settings_apps.rs` 的既有与新增用例。

**Interfaces:**
- Produces(`app_logs.rs`,纯状态机):
  ```rust
  pub const REFRESH_INTERVAL: Duration = Duration::from_secs(1);
  pub const FETCH_LINES: u32 = 200;
  #[derive(Debug, Clone, PartialEq)]
  pub enum LogsView { Hidden, Loading, Loaded { text: String, truncated: bool }, Failed(String) }
  #[derive(Debug, Clone, PartialEq)]
  pub enum LogsEffect { Fetch }          // 调用方把它映射成自己的 Effect(带 slot / id)
  #[derive(Default)]
  pub struct LogsState { /* view, in_flight, last_fetch */ }
  impl LogsState {
      pub fn view(&self) -> LogsView;
      pub fn show(&mut self, now: Instant) -> Vec<LogsEffect>;
      pub fn hide(&mut self);
      pub fn reset(&mut self);                                   // 应用离开 Failed / 查看器的宿主不可见了
      pub fn loaded(&mut self, now: Instant, r: Result<(String, bool), String>) -> Vec<LogsEffect>;
      /// 展开着且距上次 >= REFRESH_INTERVAL 且无在途 → Fetch;Hidden 永远空。
      pub fn tick(&mut self, now: Instant) -> Vec<LogsEffect>;
  }
  ```
- 规则(全部由 `LogsState` 保证,两个宿主不各写一遍):
  - `loaded` 只接收**在途的那次**(沿用 A6e 的修复:Hidden 或被 `reset` 后迟到的结果丢弃)。
  - 刷新时**保留旧文本**直到新文本到(不回到 `Loading` 闪烁);刷新失败 → 保持旧文本并在其上标"刷新失败"(`Failed` 只用于首次加载失败)。
  - 文本没变就不触发多余重绘:`loaded` 返回值相同的 `text`/`truncated` 时 `view()` 值不变(调用方据此无需额外判断)。
- 设置页(`settings_apps.rs`):每个已安装的进程型应用行加「日志」按钮,展开后在该行下方显示查看器(同一时刻只展开一个应用),`Running` 与 `Failed` 都能看。新 `Effect::FetchLogs(String)`(应用 id)与 `Message::{ShowLogs(String), HideLogs, LogsLoaded(String, Result<..>)}`。
- 刷新驱动:`app_host` 与 `settings_apps` 各在已有的"下一拍唤醒"里加 `tick`;**查看器不可见(面板被隐藏、设置页关闭)时不刷新**——`reset`/不调 `tick` 即可,不要后台空转。

- [ ] **Step 1: 写失败测试**(`app_logs.rs`)
  - `show` 发一次 `Fetch` 并进入 `Loading`;在途时重复 `show`/`tick` 不重发。
  - `loaded` 之后 `tick` 在 `REFRESH_INTERVAL` 前为空、之后发一次 `Fetch`;刷新期间 `view()` 仍是旧 `Loaded`(不回 `Loading`)。
  - `hide` 后 `tick` 永远为空;`hide` 后迟到的 `loaded` 被丢弃(`view()` 仍 `Hidden`);`reset` 同理且下次 `show` 重新 `Fetch`(不显示旧内容)。
  - 刷新失败保持旧文本;首次失败是 `Failed`。
  - 相同结果不改变 `view()`。
  - 迁移后 `app_host.rs` 现有 6 个日志相关用例(`showing_logs_fetches_once...`、`a_late_logs_result_*`、`logs_are_hidden_again_when_the_app_leaves_the_failed_state` 等)**不改断言**也应通过——这是迁移正确性的证据。
  - `settings_apps.rs`:展开 A 再展开 B → A 收起;应用被卸载 → 查看器复位;`Running` 的应用可展开。
  - **服务端 Review Focus 6**(`logs.rs`):读取期间 `app.log` 被轮转(用测试在读之前把 `app.log` 改名为 `app.log.1` 并新建空 `app.log`)→ 返回 `.1` 的内容,不报错;`app.log` 消失而 `.1` 在 → 同上;两者都没有 → 空文本(已有用例保持通过)。
  - **服务端 Review Focus 5**(`process_apps_live.rs` 或 `logs.rs` 单测):含 `\x1b[32m` 与 6000 字符单行的日志读回后无 `\x1b`、该行长度 ≤ `MAX_LINE_CHARS + 1`(`logs.rs` 已有 `an_overlong_line_is_cut_and_marked`,补一条含转义 + 超长的组合用例)。
- [ ] **Step 2: 确认失败 → Step 3: 实现 → Step 4: 通过**(`cargo test -p dozer-app app_logs app_host settings_apps`、`cargo test -p bytehost-apps --all-features logs`;`cargo clippy -p dozer-app --all-targets` 无新警告)。变异:去掉 `loaded` 的"只接收在途"守卫,迟到用例失败;让刷新回到 `Loading`,保留旧文本用例失败。
- [ ] **Step 5: 视图**:查看器用系统默认字体,`truncated` 时顶部一行"只显示末尾若干行";自动滚到底(iced `scrollable` 的 `snap_to`,**仅当用户没有手动上滚时**——用 `scrollable::on_scroll` 记录是否贴底,不贴底则保持位置)。
- [ ] **Step 6: 手动验收**(需要 GUI,如实记录):`py-notes` 运行中,在设置→应用里点「日志」,另开终端 `curl` 触发应用打印,日志在 ~1s 内出现新行;关闭设置页后用 `lsof`/日志确认不再有 `Logs` 请求;上滚后新行到来不被拉回底部。
- [ ] **Step 7: Commit** — `feat(dozer-app): live-refreshing log viewer shared by the app panel and the settings page (A6f task 4)`。

---

### Task 5: 验收、文档收尾

**Files:** Create `docs/superpowers/specs/2026-10-08-bytehost-a6f-acceptance-report.md`;Modify `CLAUDE.md`(`bytehost-apps` 一行)、规格 §7 表。

报告结构(**§1/§3 由执行时按实际运行填写,不得预填;测试通过也要给出命令与实测耗时**):§0 环境;§1 自动化(`cargo test -p bytehost-apps --all-features`、`-p dozerd`(含 `process_apps_live -- --ignored`)、`-p dozer-client`、`-p dozer-app` 的结果与已知无关失败逐个注明);§2 GUI 手工项(Task 2 Step 5、Task 4 Step 6,未执行保持未勾选);§3 发现的缺陷与已知局限。

- [ ] **Step 1: live 回归**:`cargo test -p dozerd --test process_apps_live -- --ignored --nocapture` 连跑 3 遍;A6e 修正后的 11 步必须仍通过(推送不得改变其行为);**审查报告里每个耗时是否合理**——A6e 曾因接受网关错误页正文而假通过,任何"快得不可能"的步骤都要追到根因。
- [ ] **Step 2: CLAUDE.md** `bytehost-apps` 一行补:A6f 落地——`AppRequest::Subscribe` 推送"失效信号"(`Changed{app}`/`Resync`,不带状态,GUI 收到即重拉 `List`;订阅连接独立,轮询降为 30s 兜底,断开回 2s 并退避重订阅)、`AppIssue::DependencyInstall`(经 `Transitions::install_failed`,先记 issue 再改状态)、共享日志查看器 `extensions/app_logs.rs`(1s 刷新、保留旧文本、只接收在途结果,设置页每行可看)。规格 §7 表加 A6f 一行,并写明 A6g(升级与回滚)、A6h(非本机来源)待写计划,容器运行时继续推迟。
- [ ] **Step 3: 全量**:`cargo test -p bytehost-apps -p dozerd -p dozer-client -p dozer-app`(已知无关的偶发/确定失败:`files::tests::delete_confirm_spec_reflects_pending_target`、`memory::tests::list_orders_by_updated_ms_desc`、`app_service::tests::the_first_run_is_unavailable_when_the_port_cannot_be_saved`——如仍存在逐个注明,不要顺手改)、`cargo clippy --all-targets`(A6f 触碰的文件无新警告)、`cargo fmt --check`、两个门禁脚本。
- [ ] **Step 4: Commit** — `docs(bytehost): A6f landed`。

---

## 已知局限(写在这里,不是缺陷)

- **推送只是失效信号**:每次变化 GUI 仍要 `List` 一次(本机 UDS,开销可忽略);换来的是只有一份状态真相。
- **日志刷新是整段重读**:每秒读末尾 ≤256 KiB;没做偏移增量。日志很大且频繁刷新时会多读,但封顶有界。
- **推送不带日志内容与安装输出**,查看器只能靠自己的 `Logs` 请求。
- **订阅按连接**:GUI 一个窗口一条订阅;多窗口/多 GUI 各订各的,互不影响。
- **容器运行时、应用升级与回滚(A6g)、非本机来源(A6h)不在本计划**。

## Self-Review(写计划时已核对)

- **范围覆盖**:用户选的四项中本计划覆盖"依赖安装失败页 + 实时日志"与"状态推送"(Task 1–4);"升级与回滚""非本机来源"明确拆出为 A6g/A6h 并说明理由;容器按裁决推迟。
- **占位符**:无;验收报告 §1/§3 要求执行时据实填写。
- **类型一致**:`AppChange`/`Subscription`/`Message::{Subscribed,Changed,SubscriptionLost}`/`Effect::Subscribe`(Task 2)、`AppIssue::DependencyInstall`/`Transitions::install_failed`(Task 3)、`LogsState`/`LogsView`/`LogsEffect`/`REFRESH_INTERVAL`(Task 4)在定义它们的任务里给出签名,后续任务按同名使用。注意 `app_host.rs` 现有的 `LogsView` 在 Task 4 迁移到 `app_logs.rs`,`view.rs` 的引用路径要同步改。
- **顺序依赖**:Task 3 的问题页"查看日志"动作依赖 Task 4 的查看器——Task 3 先接到 A6e 现有的 `ShowLogs`,Task 4 迁移时保持消息名不变,不会在 Task 3/4 之间留下断点。
